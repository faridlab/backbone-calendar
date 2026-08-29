# The calendar event family — declared behavior

> Scope: the personal/event calendar (meetings, recurrence, attendees) that lives in this
> module alongside the pre-existing working-time and religion calendars. Everything here is
> a **declaration**: it names the posture, the invariants, and the deferrals so a reader
> can tell designed behavior from an accident. Probes live in
> `tests/calendar_event_family_probes.rs`; the engine in
> `src/application/service/calendar_event_series_service_custom.rs`; the fences in
> `migrations/20260828140002_event_family_fences.up.sql`.
>
> Upstream grounding: the Odoo 19 community `calendar` addon as catalogued by the
> services/extensions cycle-40 study (flags CAL-1..CAL-6, CALM-1 — see
> `docs/odoo/services/extensions/README.md` in the framework repo). Where this module
> departs from Odoo, the departure is declared and the reason given.

## The four entities

| Table | What it is |
|---|---|
| `calendar.event_series` | The recurrence rule (`freq`, `interval`, `by_weekday`, `by_monthday`, `until`, `count`) plus a pointer to its base event. |
| `calendar.events` | Real event rows — one per materialized occurrence (and all standalone events). |
| `calendar.event_exceptions` | The split/cancel ledger: which (series, slot) pairs have been claimed by an edit or a delete. No HTTP route; internal by design. |
| `calendar.event_attendees` | One row per (event, user) with a hand-set response state and a unique `access_token` (the invitation seam). |

All four carry `company_id NOT NULL` under the module-wide `company_fence: strict`
declaration, audit metadata in `metadata` JSONB, and logical (bare-column) foreign keys —
the module's established table shape. No DB-level FK constraints.

The four family enum types (`event_recurrence_freq`, `event_privacy`,
`event_attendee_state`, `event_exception_kind`) are created **unqualified** (they land in
`public`), matching the module's pre-existing enum convention (`employment_status`) and
the generated sqlx entity derives, which declare bare `type_name`s. A schema-qualified
`CREATE TYPE calendar.*` makes row decoding fail with a type-name mismatch — do not
"tidy" them into the `calendar` schema.

## Posture: recurrence is EAGERLY materialized (CAL-1)

A series is **real rows**, not a virtual expansion. Creating a series with N occurrences
writes N `calendar.events` rows (the base event is slot 0) inside one transaction. This is
the declared opposite pole of maintenance's lazy clone-on-done recurrence: both postures
exist in the codebase on purpose, declared per family (the ADR-0016 principle — posture is
a declared property of the family, not something to normalize away).

Two bounds:

- **The cap is 720 occurrences** (`MAX_OCCURRENCES`, named for Odoo's
  `MAX_RECURRENT_EVENT`). A rule that projects more than 720 slots fails **loudly**: the
  whole transaction rolls back, zero rows are written, and the caller receives
  HTTP 422 with error code `CALENDAR_EVENT_RECURRENCE_CAP_EXCEEDED`. A silent truncation
  would be a data-integrity lie — the caller asked for a series and would silently own a
  different, shorter one.
- **A rule with neither `until` nor `count` is bounded to a horizon** of first-slot start
  + 15 years (`UNBOUNDED_HORIZON_YEARS`, Odoo's `calendar.max_recurrence_years`
  default). "Forever" is a finite number of rows; the cap still applies on top.
  Deliberate posture decision: an unbounded rule is **not** rejected as a 400
  validation error — it is expanded against the horizon and then cap-checked, so a
  daily-forever rule answers the same loud 422 `CALENDAR_EVENT_RECURRENCE_CAP_EXCEEDED`
  (zero rows) while a sparse unbounded rule that fits under the cap (e.g. yearly) is
  honored. The cap channel is the single loud bound for every rule; the surface adds no
  second, quieter one.

## Series edits rewrite by (start, stop) identity (CAL-2)

Reconciliation between a new rule expansion and the surviving member rows keys on the
exact `(start_at, stop_at)` tuple — **an occurrence whose times drifted is by definition
an exception**, never a row to be silently re-dated. Three edit scopes:

- **`this`** — the occurrence splits from the series: its current `(start, stop)` is
  claimed in the ledger as `kind = edited`, the row's `series_id` is set to NULL, and the
  field edits apply to the now-standalone row. The row's identity (its `id`) survives the
  split.
- **`following`** — every member at or after the target's start detaches the same way,
  and the series rule is trimmed (`until` := the day before the split slot). The tail
  becomes standalone rows; the head is untouched.
- **`all`** (series rewrite, PUT) — the new rule is expanded (cap re-checked, loud):
  slots claimed by the exception ledger are **never re-materialized** — this is what
  makes single edits and single deletes stick across rewrites; a member whose `(start, stop)`
  matches a new-grid slot is UPDATEd in place (its `id` is stable); a new-grid slot with
  no matching row is INSERTed; a surviving member whose `(start, stop)` is off-grid and
  unclaimed is detached defensively with an `edited` exception — drifted data is never
  destroyed.

The DB backstops the identity contract:
`uq_calendar_events_series_slot UNIQUE (series_id, start_at, stop_at)` over live rows —
a series can never hold two rows for one slot, and a soft-deleted row frees its slot.
`uq_calendar_event_exceptions_slot UNIQUE (series_id, slot_start_at, slot_stop_at)` over
live rows — a slot can be claimed at most once. Deleting one occurrence soft-deletes the
row and claims its slot as `kind = cancelled`; a later series rewrite will not resurrect it.

## Attendee dedup is a DB constraint (CAL-3)

Odoo keeps the one-attendee-per-partner invariant procedurally (a command-diff in
`write()`; there is **no SQL unique**). Raw SQL bypasses that entirely, which is exactly
the class of hole ADR-0015 exists to close: an invariant reachable by raw SQL must carry
a database backstop. This module declares the `both` shape:

- **application-side**: attendee lists are deduplicated before insert (first-wins); a
  duplicate surfaces as HTTP 409 `CALENDAR_ATTENDEE_DUPLICATE`;
- **DB backstop**: `uq_calendar_event_attendees_event_user UNIQUE (event_id, user_id)` over
  live rows (partial on `metadata->>'deleted_at' IS NULL`, so soft-delete + re-add works).
  Any write path — including a hand-run `INSERT` — hits 23505.

The attendee state machine is faithful to Odoo: `needs_action → accepted / declined /
tentative`, **hand-set with no validation gate**, and the organizer is auto-attendee with
`state = accepted` on create (mirroring Odoo's organizer auto-accept in `create()`).

Every attendee row carries a unique `access_token` (plain unique index). It is the seam
for the future invitation-answer and `/ics` flows — **no transport exists this wave**.

## Privacy is a declared read fence (CAL-4)

Odoo enforces `privacy` in five ORM layers (ir.rules, read-cache field masking where a
private event's name becomes `"Busy"`, display-name masking, group-by domain injection,
a write-side manual check) — and raw SQL bypasses all five. The port re-declares the
fence where it cannot be bypassed: **the database**.

`calendar.events` carries, alongside the strict company policy every table in this module
has, a **RESTRICTIVE SELECT policy** (`calendar_events_privacy_read`): a row is readable
when it is `public`, or the reader is the organizer, or the reader is a live attendee.
Because permissive policies OR and restrictive policies AND, a read must satisfy
**company-match AND privacy-pass**; writes remain governed by the company policy alone.

Declared consequences, both on purpose:

- **Fail-closed on unset user**: a session without `app.user_id` pinned sees only public
  rows. Background jobs that legitimately need private reads must pin an acting user.
- **`private` AND `confidential` are both row-invisible to non-participants this wave.**
  Odoo's private→"Busy" masking is a display-layer behavior; the DB fence errs invisible,
  and the masking decision is deferred to the webapp where display belongs.

## The working_days non-inheritance fence

The working-time family's read port (`CalendarRepository::working_days`) answers with a
company-wide Monday–Friday-minus-holidays simplification and carries unresolved scope
junctions — it is **working-time-family only**. The event family calls nothing that reads
it, and no availability endpoint exists this wave. This fence is declared here and in the
engine source: when availability math is eventually built it must either scope away from
that port entirely or re-declare its own working-time source — never inherit the
simplification silently.

## HTTP surface

One guarded composition (`src/presentation/http/calendar_event_guarded_routes.rs`), mounted
by the host under the module's schema-named base (`/api/v1/calendar`): `/events`,
`/event-series`, `/event-attendees`. The generated 12-endpoint CRUD for the four entities
is deliberately **not** mounted. Every route requires its permission from the
`calendar_event:*` vocabulary — no route mounts without a guard, and without the `auth`
cargo feature nothing mounts at all (fail-closed). Identity comes from the host's
`company_auth` middleware (signed token → `CompanyContext`); the company never crosses the
wire in a request body.

Error map: recurrence cap → 422 `CALENDAR_EVENT_RECURRENCE_CAP_EXCEEDED`; duplicate
attendee → 409 `CALENDAR_ATTENDEE_DUPLICATE`; validation → 400; not found → 404.

## Declared deferrals

| What | Status | Grounding |
|---|---|---|
| Alarms (dual-channel: windowed email cron + bus notification + one rolling cron trigger per recurrence) | **Deferred** — CAL-5. Not in this wave's named surface; when the alarm family lands it must carry `posture:` and route through the notification module. |
| `allday` events + DST localization | **Deferred** — CAL-6. No `allday` field exists this wave (start/stop are UTC timestamptz, the CALM-1 storage truth); the allday local-date convention ports as an additive field when needed. |
| Duration mutual-awareness, per-occurrence videocall tokens, meeting-activity two-way sync, partner↔attendee command-diff duality | **Deferred** — CALM-1 extras beyond the start/stop storage truth. |
| Availability math of any kind | **Scoped away** — see the working_days fence above. |
| Invitation-token answer flows + `/ics` export | **Deferred to the events wave** — any `/ics` surface must carry the `access_token` gate; the token seam (unique per attendee) already exists. |
| `calendar_sms` | **Flag only** — an empty `sms = []` cargo feature marks the channel-overlay seam. No transport code ships under it; the transport lands with the notification-channel wave. Hooks already declare `sms_enabled: false`. |

## Running the probes

`tests/calendar_event_family_probes.rs` is DB-backed: it skips loudly when `DATABASE_URL`
is unset (the module is a library; the DB is the review's responsibility). The recipe:

```bash
# scratch container on 5433 (postgres/postgres); fresh DB, module migrations in filename order
psql 'postgresql://postgres:postgres@127.0.0.1:5433/postgres' -c 'CREATE DATABASE calendar_sv2_probe'
for f in migrations/*.up.sql; do      # filename order
  psql -v ON_ERROR_STOP=1 -q -f "$f" 'postgresql://postgres:postgres@127.0.0.1:5433/calendar_sv2_probe'
done
DATABASE_URL='postgresql://postgres:postgres@127.0.0.1:5433/calendar_sv2_probe' \
  cargo test --features auth --test calendar_event_family_probes
```

**Migration hygiene (probe P10, run around the suite):** the event-family migration
pairs must be fully reversible — down in reverse filename order, then re-up in filename
order, both with `ON_ERROR_STOP=1`:

```bash
for f in $(ls migrations/2026082814000*.down.sql | sort -r); do   # reverse order
  psql -v ON_ERROR_STOP=1 -q -f "$f" "$DATABASE_URL"
done   # then verify: zero calendar.event* tables and zero event_* enum types remain
for f in $(ls migrations/2026082814000*.up.sql | sort); do        # filename order
  psql -v ON_ERROR_STOP=1 -q -f "$f" "$DATABASE_URL"
done   # then verify: 4 company policies + the RESTRICTIVE privacy policy, the
       # events_stop_after_start CHECK, and the 4 uq_calendar_* indexes are all back
```

A half-applied state after either direction is a red result: the down pass must leave
zero event-family tables and enums, and the re-up must restore every fence, the CHECK
constraint, and all four unique indexes (verify against `pg_policies`, `pg_constraint`,
`pg_indexes`). The full up→down→up cycle plus the probe suite was last run green
together on a fresh database.

The suite is self-contained: it mints its own probe role (non-superuser, NOBYPASSRLS —
the only session posture under which RLS actually binds) and uses fresh random company ids
per test so parallel runs never collide. The embedded router-shape and error-map tests in
`src/presentation/http/calendar_event_guarded_routes.rs` take a second variable,
`CALENDAR_PROBE_DATABASE_URL`, pointing at a pre-minted restricted role
(`NOSUPERUSER NOBYPASSRLS`, `USAGE` on schema `calendar`, `SELECT/INSERT/UPDATE/DELETE`
on the four family tables); without it those DB-backed legs skip loudly.

The **generated** API tests (`tests/integration_tests.rs`, one file per entity) are a
different harness: they drive a running service over HTTP via `API_BASE_URL` (default
`http://127.0.0.1:3000`) and take a skip-as-success posture when nothing answers. Two
caveats when running them on a dev machine: point `API_BASE_URL` at a dead port to get
that skip posture explicitly — a locally running service that happens to own the
generated (un-namespaced) `/api/v1/events` path will answer 401 and fail those legs as a
pure environment artifact; and note the generated paths do not carry the module's
schema-named mount (`/api/v1/calendar/...`), so even a live service only exercises them
through its own route table. Never pipe the migration `psql` runs through `head` — the
closed pipe SIGPIPE-kills psql mid-file and leaves a half-applied schema.
