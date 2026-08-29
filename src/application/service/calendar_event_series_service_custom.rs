//! Calendar event series engine — the event-family core.
//!
//! Hand-written (user-owned). Public surface (fixed for the guarded HTTP
//! composition and the probe suite):
//!
//! - `CalendarEventSeriesEngine::new(pool, events, series, exceptions, attendees)`
//! - operations `create_series` / `rewrite_series` / `edit_occurrence` /
//!   `delete_occurrence` / `attach_attendees` / `set_attendee_state` /
//!   `create_standalone`, each `(&self, cmd, ScopeCtx) -> Result<_, EventFamilyError>`
//! - `EventFamilyError::{RecurrenceCap{projected, cap}, DuplicateAttendee{user_id},
//!   NotFound, Validation(String), Db(sqlx::Error)}`
//! - consts `MAX_OCCURRENCES = 720`, `UNBOUNDED_HORIZON_YEARS = 15`
//!
//! ## Posture: recurrence is EAGERLY materialized (the declared pole)
//!
//! A series is real rows, never a lazy virtual series: creating one writes one
//! `calendar.events` row per occurrence slot (the base event is slot 0), the
//! exact opposite pole of the clone-on-done families elsewhere in the system
//! (both postures are declared per family, not normalized). The cap is LOUD:
//! a rule that would materialize more than [`MAX_OCCURRENCES`] occurrences
//! (named for Odoo's `MAX_RECURRENT_EVENT = 720`) fails with
//! [`EventFamilyError::RecurrenceCap`] — HTTP 422 — and the whole transaction
//! rolls back with ZERO rows written. Never a silent truncation. A rule with
//! neither `until` nor `count` is bounded to [`UNBOUNDED_HORIZON_YEARS`] from
//! the first slot's start (Odoo's `max_recurrence_years` default).
//!
//! ## Identity: series edits REWRITE by (start, stop) tuple
//!
//! A series rewrite (edit scope `all`) re-expands the new rule and reconciles
//! against surviving member rows by exact `(start_at, stop_at)` identity: a
//! row whose times match a new slot is UPDATED in place (id stable); a slot
//! with no row is INSERTed; a row whose times match no new slot has drifted,
//! and a drifted time IS an exception by definition — the row is detached
//! (`series_id = NULL`) with a defensive `edited` exception claim so no data
//! is ever destroyed. Slots claimed in the exception ledger (either kind) are
//! NEVER re-materialized — that is what makes single-occurrence edits and
//! deletes stick across rewrites. The partial unique index
//! `uq_calendar_events_series_slot (series_id, start_at, stop_at)` backstops
//! the identity at the DB level.
//!
//! ## FENCE — working_days non-inheritance (loud, by declaration)
//!
//! This engine MUST NOT consult `CalendarRepository::working_days` (or
//! `holiday_dates`) for ANY computation. That read-port answers with a
//! company-wide Mon–Fri-minus-holidays simplification carrying known
//! unresolved scope junctions (branch/department/level/position/employee/
//! religion/employment-status), and it belongs to the WORKING-TIME family
//! only. The event family does NO availability math this wave; when
//! availability is built later it must either scope itself away from that
//! port entirely or re-declare and fix its own working-time semantics first
//! — never silently inherit the simplification. This fence is declared here,
//! in the repository headers, and in docs/event-family.md.
//!
//! ## Scoping: one transaction per operation, pinned before any query
//!
//! Every operation opens ONE transaction whose connection pins
//! `app.company_id` and `app.user_id` via `set_config(..., true)` (see
//! [`CalendarEventRepository::begin_scope`]), then runs all its statements on
//! that transaction. Row-level security therefore evaluates every row: the
//! strict company fence on all four event-family tables, and the restrictive
//! privacy read fence on `calendar.events` (public, or organizer, or a live
//! attendee). An acting user that cannot see a row cannot edit, delete, or
//! attach to it — it maps to `NotFound`. The transaction-local pin resets at
//! COMMIT/ROLLBACK, so a pooled connection never carries one request's scope
//! into the next.

use std::collections::{HashMap, HashSet};

use chrono::{DateTime, Datelike, Days, Months, NaiveDate, NaiveDateTime, Utc};
use sqlx::PgPool;
use uuid::Uuid;

use crate::domain::entity::{EventAttendeeState, EventPrivacy, EventRecurrenceFreq};
use crate::infrastructure::persistence::{
    CalendarEventAttendeeRepository, CalendarEventExceptionRepository, CalendarEventRepository,
    CalendarEventSeriesRepository,
};

/// Hard cap on materialized occurrences per series. Named for Odoo's
/// `MAX_RECURRENT_EVENT`; hitting it is a loud error, never a truncation.
pub const MAX_OCCURRENCES: usize = 720;

/// Horizon applied when a rule has neither `until` nor `count`: slots are
/// generated up to first-slot start + this many years, still subject to
/// `MAX_OCCURRENCES`.
pub const UNBOUNDED_HORIZON_YEARS: i32 = 15;

/// Safety bound on recurrence period scanning. The DSL cannot express a rule
/// that never emits (daily/weekly are unconditional; monthly day-31 and
/// yearly Feb-29 emit on the years that have them), so real rules terminate
/// long before this bound; exhausting it with a `count` still unmet is a loud
/// validation error, never a silent truncation.
const MAX_SCAN_PERIODS: u64 = 200_000;

/// Request scoping for the RLS fences: the company the series lives in and
/// the user the engine acts as (privacy reads fail closed without one).
#[derive(Debug, Clone, Copy)]
pub struct ScopeCtx {
    pub company_id: Uuid,
    pub acting_user_id: Uuid,
}

/// Which occurrences a PATCH edit applies to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EditScope {
    /// This occurrence only: it splits from the series into an exception row.
    This,
    /// This occurrence and every later one: the tail detaches standalone and
    /// the series rule is trimmed to the day before this occurrence's slot.
    Following,
}

/// Event-family error surface. `RecurrenceCap` maps to HTTP 422
/// `CALENDAR_EVENT_RECURRENCE_CAP_EXCEEDED`; `DuplicateAttendee` to 409
/// `CALENDAR_ATTENDEE_DUPLICATE`.
#[derive(Debug, thiserror::Error)]
pub enum EventFamilyError {
    #[error("recurrence cap exceeded: projected {projected} occurrences, cap {cap}")]
    RecurrenceCap { projected: usize, cap: usize },

    #[error("attendee already invited: {user_id}")]
    DuplicateAttendee { user_id: Uuid },

    #[error("not found")]
    NotFound,

    #[error("validation error: {0}")]
    Validation(String),

    #[error("database error: {0}")]
    Db(#[from] sqlx::Error),
}

/// Command: create a series (rule + first-occurrence template + attendees).
#[derive(Debug, Clone)]
pub struct CreateSeriesCmd {
    pub name: Option<String>,
    pub freq: EventRecurrenceFreq,
    pub interval: i32,
    pub by_weekday: Option<String>,
    pub by_monthday: Option<String>,
    pub until: Option<NaiveDate>,
    pub count: Option<i32>,
    pub title: String,
    pub description: Option<String>,
    pub first_start_at: DateTime<Utc>,
    pub first_stop_at: DateTime<Utc>,
    pub privacy: EventPrivacy,
    pub location: Option<String>,
    pub attendee_user_ids: Vec<Uuid>,
}

/// Command: rewrite a series rule (edit scope `all`); the new rule is
/// re-expanded and reconciled against surviving rows by (start, stop) identity.
#[derive(Debug, Clone)]
pub struct RewriteSeriesCmd {
    pub series_id: Uuid,
    pub name: Option<String>,
    pub freq: EventRecurrenceFreq,
    pub interval: i32,
    pub by_weekday: Option<String>,
    pub by_monthday: Option<String>,
    pub until: Option<NaiveDate>,
    pub count: Option<i32>,
    pub title: String,
    pub description: Option<String>,
    pub first_start_at: DateTime<Utc>,
    pub first_stop_at: DateTime<Utc>,
    pub privacy: EventPrivacy,
    pub location: Option<String>,
}

/// Command: edit one occurrence (field patch + scope).
#[derive(Debug, Clone)]
pub struct EditOccurrenceCmd {
    pub event_id: Uuid,
    pub scope: EditScope,
    pub title: Option<String>,
    pub description: Option<String>,
    pub start_at: Option<DateTime<Utc>>,
    pub stop_at: Option<DateTime<Utc>>,
    pub privacy: Option<EventPrivacy>,
    pub location: Option<String>,
}

/// Command: attach attendees to an existing event (deduped; 409 on backstop).
#[derive(Debug, Clone)]
pub struct AttachAttendeesCmd {
    pub event_id: Uuid,
    pub attendee_user_ids: Vec<Uuid>,
}

/// Command: hand-set an attendee response state (no validation gate).
#[derive(Debug, Clone)]
pub struct SetAttendeeStateCmd {
    pub attendee_id: Uuid,
    pub state: EventAttendeeState,
}

/// Command: create a standalone (non-series) event.
#[derive(Debug, Clone)]
pub struct CreateStandaloneCmd {
    pub title: String,
    pub description: Option<String>,
    pub start_at: DateTime<Utc>,
    pub stop_at: DateTime<Utc>,
    pub privacy: EventPrivacy,
    pub location: Option<String>,
    pub attendee_user_ids: Vec<Uuid>,
}

// ---------------------------------------------------------------------------
// Pure recurrence expander
// ---------------------------------------------------------------------------

/// A parsed recurrence rule (the series row's expansion inputs). `by_weekday`
/// is ISO numbering 1 = Monday .. 7 = Sunday; `by_monthday` is 1..=31 (months
/// lacking the day are skipped, per the RRULE convention). `until` is
/// inclusive and bounds by the slot's start DATE; `count` counts the FIRST
/// occurrence too (slot 0). A rule with neither bound expands to
/// [`UNBOUNDED_HORIZON_YEARS`] from the first slot.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecurrenceRule {
    pub freq: EventRecurrenceFreq,
    pub interval: i32,
    pub by_weekday: Vec<u8>,
    pub by_monthday: Vec<u8>,
    pub until: Option<NaiveDate>,
    pub count: Option<i32>,
}

/// One materialization slot: the (start, stop) tuple that identifies an
/// occurrence of a series.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OccurrenceSlot {
    pub start_at: DateTime<Utc>,
    pub stop_at: DateTime<Utc>,
}

/// Expand a rule into ordered occurrence slots, starting from the given first
/// occurrence (the base event is slot 0, verbatim — its exact timestamps are
/// the template). Pure: chrono arithmetic on UTC timestamps only, no DST or
/// timezone localization (start/stop are stored UTC; all-day events and local
/// timezone semantics are a declared later-wave addition).
///
/// The duration of every slot equals the first occurrence's duration; the
/// time-of-day of every slot equals the first occurrence's UTC time-of-day.
///
/// Loud failure modes (never truncation):
/// - more than [`MAX_OCCURRENCES`] projected slots → [`EventFamilyError::RecurrenceCap`];
/// - a `count` that cannot be met within [`MAX_SCAN_PERIODS`] periods →
///   [`EventFamilyError::Validation`].
pub fn expand_occurrences(
    rule: &RecurrenceRule,
    first_start: DateTime<Utc>,
    first_stop: DateTime<Utc>,
) -> Result<Vec<OccurrenceSlot>, EventFamilyError> {
    let duration = first_stop - first_start;
    let anchor_date = first_start.date_naive();
    let anchor_time = first_start.time();
    let horizon = first_start
        .checked_add_months(Months::new(UNBOUNDED_HORIZON_YEARS as u32 * 12))
        .ok_or_else(|| EventFamilyError::Validation("first occurrence out of range".into()))?;

    let count = rule.count.map(|c| c as usize);
    let mut slots: Vec<OccurrenceSlot> = Vec::new();
    slots.push(OccurrenceSlot { start_at: first_start, stop_at: first_stop });

    let push_slot = |slots: &mut Vec<OccurrenceSlot>, date: NaiveDate| {
        let start = NaiveDateTime::new(date, anchor_time).and_utc();
        let Some(stop) = start.checked_add_signed(duration) else {
            return None;
        };
        slots.push(OccurrenceSlot { start_at: start, stop_at: stop });
        Some(())
    };

    // Periods scan from k = 0 so the anchor's own week/month can still emit
    // its later candidates (e.g. a Monday anchor with by_weekday = Mon,Wed
    // produces the Wednesday of week 0); the anchor date itself is filtered
    // below because slot 0 is already the first occurrence, verbatim.
    // `terminated_by_bound` distinguishes a rule that ended because `until`
    // or the horizon said so (a count legitimately unmet by an earlier bound
    // is fine) from one that exhausted the scan bound without meeting its
    // count (a loud validation error, never a truncation).
    let mut terminated_by_bound = false;
    'periods: for k in 0..=MAX_SCAN_PERIODS {
        // Candidate start DATES for period k.
        let mut candidates: Vec<NaiveDate> = match rule.freq {
            EventRecurrenceFreq::Daily => vec![anchor_date
                .checked_add_days(Days::new(k.saturating_mul(rule.interval as u64)))
                .ok_or_else(|| EventFamilyError::Validation("recurrence date out of range".into()))?],
            EventRecurrenceFreq::Weekly => {
                if rule.by_weekday.is_empty() {
                    vec![anchor_date
                        .checked_add_days(Days::new(k.saturating_mul((rule.interval * 7) as u64)))
                        .ok_or_else(|| EventFamilyError::Validation("recurrence date out of range".into()))?]
                } else {
                    // Week 0 is the ISO week containing the anchor; weekday
                    // offsets are 1 = Monday .. 7 = Sunday.
                    let iso_wd = (anchor_date.weekday().num_days_from_monday() + 1) as u64;
                    let week_start = anchor_date
                        .checked_sub_days(Days::new(iso_wd - 1))
                        .ok_or_else(|| EventFamilyError::Validation("recurrence date out of range".into()))?;
                    let week_k = week_start
                        .checked_add_days(Days::new(k.saturating_mul((rule.interval * 7) as u64)))
                        .ok_or_else(|| EventFamilyError::Validation("recurrence date out of range".into()))?;
                    let mut days: Vec<NaiveDate> = rule
                        .by_weekday
                        .iter()
                        .filter_map(|wd| week_k.checked_add_days(Days::new((*wd as u64) - 1)))
                        .collect();
                    days.sort_unstable();
                    days.dedup();
                    days
                }
            }
            EventRecurrenceFreq::Monthly => {
                let total = anchor_date.year() as i64 * 12 + (anchor_date.month() as i64 - 1)
                    + (k as i64) * (rule.interval as i64);
                let year = total.div_euclid(12) as i32;
                let month = (total.rem_euclid(12) + 1) as u32;
                let days_of_month: Vec<u8> = if rule.by_monthday.is_empty() {
                    vec![anchor_date.day() as u8]
                } else {
                    rule.by_monthday.clone()
                };
                let mut days: Vec<NaiveDate> = days_of_month
                    .iter()
                    .filter_map(|md| NaiveDate::from_ymd_opt(year, month, *md as u32))
                    .collect();
                days.sort_unstable();
                days
            }
            EventRecurrenceFreq::Yearly => {
                // The anchor's month/day in year anchor+k*interval; a date
                // that does not exist (Feb 29 on a non-leap year) is SKIPPED,
                // per the RRULE convention — the occurrence simply does not
                // happen that year.
                match anchor_date
                    .year()
                    .checked_add((k as i32).checked_mul(rule.interval).unwrap_or(i32::MAX))
                    .and_then(|y| NaiveDate::from_ymd_opt(y, anchor_date.month(), anchor_date.day()))
                {
                    Some(d) => vec![d],
                    None => Vec::new(),
                }
            }
        };
        candidates.sort_unstable();
        candidates.dedup();

        for date in candidates {
            if date <= anchor_date {
                continue; // slot 0 is the anchor itself, already emitted
            }
            if let Some(until) = rule.until {
                if date > until {
                    terminated_by_bound = true;
                    break 'periods; // dates are ascending — nothing more can qualify
                }
            }
            if rule.until.is_none() && rule.count.is_none() {
                let start = NaiveDateTime::new(date, anchor_time).and_utc();
                if start > horizon {
                    terminated_by_bound = true;
                    break 'periods;
                }
            }
            if let Some(want) = count {
                if slots.len() >= want {
                    terminated_by_bound = true;
                    break 'periods;
                }
            }
            if push_slot(&mut slots, date).is_none() {
                terminated_by_bound = true;
                break 'periods;
            }
        }
    }

    if let Some(want) = count {
        if slots.len() < want && !terminated_by_bound {
            return Err(EventFamilyError::Validation(format!(
                "recurrence rule cannot produce {want} occurrences within the supported scan bound"
            )));
        }
    }
    if slots.len() > MAX_OCCURRENCES {
        return Err(EventFamilyError::RecurrenceCap {
            projected: slots.len(),
            cap: MAX_OCCURRENCES,
        });
    }
    Ok(slots)
}

// ---------------------------------------------------------------------------
// Engine
// ---------------------------------------------------------------------------

/// The series engine. Constructed once by the module builder; holds the pool
/// and the four family repositories. See the module doc for the posture.
pub struct CalendarEventSeriesEngine {
    pool: PgPool,
    events: std::sync::Arc<CalendarEventRepository>,
    series: std::sync::Arc<CalendarEventSeriesRepository>,
    exceptions: std::sync::Arc<CalendarEventExceptionRepository>,
    attendees: std::sync::Arc<CalendarEventAttendeeRepository>,
}

impl CalendarEventSeriesEngine {
    /// Fixed construction surface: pool + events/series/exceptions/attendees
    /// repositories.
    pub fn new(
        pool: PgPool,
        events: std::sync::Arc<CalendarEventRepository>,
        series: std::sync::Arc<CalendarEventSeriesRepository>,
        exceptions: std::sync::Arc<CalendarEventExceptionRepository>,
        attendees: std::sync::Arc<CalendarEventAttendeeRepository>,
    ) -> Self {
        Self { pool, events, series, exceptions, attendees }
    }

    /// Create a series: expand + cap-check the rule (BEFORE any database
    /// touch — a cap hit writes zero rows), insert the base event (slot 0)
    /// plus one real row per remaining occurrence, the series row pointing at
    /// the base, and the attendee set on every materialized row (organizer
    /// auto-attendee with state `accepted`, mirroring the reference create()
    /// path; everyone else `needs_action`). Attendees are deduped app-side
    /// (first-wins); the DB partial unique index on (event_id, user_id)
    /// backstops every other write path with 23505 → `DuplicateAttendee`.
    pub async fn create_series(
        &self,
        cmd: CreateSeriesCmd,
        scope: ScopeCtx,
    ) -> Result<Uuid, EventFamilyError> {
        let rule = Self::validate_rule(
            cmd.freq,
            cmd.interval,
            cmd.by_weekday.as_deref(),
            cmd.by_monthday.as_deref(),
            cmd.until,
            cmd.count,
            cmd.first_start_at,
            cmd.first_stop_at,
        )?;
        Self::validate_title(&cmd.title)?;

        // LOUD cap: expansion happens before the transaction opens, so a cap
        // violation cannot leave a half-written series behind.
        let slots = expand_occurrences(&rule, cmd.first_start_at, cmd.first_stop_at)?;

        let mut tx = self
            .events
            .begin_scope(&self.pool, scope.company_id, scope.acting_user_id)
            .await?;

        let series_id = Uuid::new_v4();
        let base_id = self
            .events
            .insert_event_scoped(
                &mut *tx,
                scope.company_id,
                Some(series_id),
                &cmd.title,
                cmd.description.as_deref(),
                slots[0].start_at,
                slots[0].stop_at,
                &cmd.privacy.to_string(),
                scope.acting_user_id,
                cmd.location.as_deref(),
            )
            .await?;

        let mut event_ids = vec![base_id];
        if slots.len() > 1 {
            let starts: Vec<DateTime<Utc>> = slots[1..].iter().map(|s| s.start_at).collect();
            let stops: Vec<DateTime<Utc>> = slots[1..].iter().map(|s| s.stop_at).collect();
            let member_ids = self
                .events
                .bulk_insert_members_scoped(
                    &mut *tx,
                    scope.company_id,
                    series_id,
                    &cmd.title,
                    cmd.description.as_deref(),
                    &starts,
                    &stops,
                    &cmd.privacy.to_string(),
                    scope.acting_user_id,
                    cmd.location.as_deref(),
                )
                .await?;
            event_ids.extend(member_ids);
        }

        self.series
            .insert_series_scoped(
                &mut *tx,
                series_id,
                scope.company_id,
                cmd.name.as_deref(),
                &cmd.freq.to_string(),
                cmd.interval,
                cmd.by_weekday.as_deref(),
                cmd.by_monthday.as_deref(),
                cmd.until,
                cmd.count,
                base_id,
                scope.acting_user_id,
            )
            .await?;

        let attendee_set = Self::attendee_set_with_organizer(
            scope.acting_user_id,
            &cmd.attendee_user_ids,
        );
        if !attendee_set.is_empty() {
            self.attendees
                .bulk_insert_scoped(
                    &mut *tx,
                    scope.company_id,
                    &event_ids,
                    &attendee_set,
                    scope.acting_user_id,
                )
                .await
                .map_err(Self::map_attendee_violation)?;
        }

        tx.commit().await?;
        Ok(series_id)
    }

    /// Rewrite a series (edit scope `all`). The new rule is expanded with a
    /// fresh LOUD cap check BEFORE any write; then, on one scoped transaction:
    /// slots claimed in the exception ledger are NEVER re-materialized;
    /// aligned rows (exact (start, stop) match) are UPDATED in place with
    /// stable ids; missing unclaimed slots are INSERTed (carrying the template
    /// attendee set so the privacy fence keeps working for participants);
    /// drifted survivors are defensively claimed as `edited` and detached —
    /// data is never destroyed. Finally the series row carries the new rule
    /// and points `base_event_id` at whatever row now sits on slot 0.
    pub async fn rewrite_series(
        &self,
        cmd: RewriteSeriesCmd,
        scope: ScopeCtx,
    ) -> Result<(), EventFamilyError> {
        let rule = Self::validate_rule(
            cmd.freq,
            cmd.interval,
            cmd.by_weekday.as_deref(),
            cmd.by_monthday.as_deref(),
            cmd.until,
            cmd.count,
            cmd.first_start_at,
            cmd.first_stop_at,
        )?;
        Self::validate_title(&cmd.title)?;

        // LOUD cap, pre-write: a rule too big to hold aborts with the series
        // untouched (the expansion errors before the transaction even opens).
        let slots = expand_occurrences(&rule, cmd.first_start_at, cmd.first_stop_at)?;

        let mut tx = self
            .events
            .begin_scope(&self.pool, scope.company_id, scope.acting_user_id)
            .await?;

        let Some(series) = self.series.find_series_scoped(&mut *tx, cmd.series_id).await? else {
            return Err(EventFamilyError::NotFound);
        };

        // Template inputs for newly materialized rows: the current base row's
        // organizer and attendee set (states preserved), falling back to
        // organizer-only accepted when the base row is gone (e.g. the base
        // occurrence was cancelled earlier) so new rows stay visible to the
        // organizer under the privacy fence.
        let base_row = self
            .events
            .find_by_id_scoped(&mut *tx, series.base_event_id)
            .await?;
        let members = self.events.member_rows_scoped(&mut *tx, series.id).await?;
        let organizer = base_row
            .as_ref()
            .map(|b| b.organizer_user_id)
            .or_else(|| members.first().map(|m| m.3))
            .unwrap_or(scope.acting_user_id);
        let mut template_attendees: Vec<(Uuid, String)> = Vec::new();
        if let Some(base) = &base_row {
            template_attendees = self
                .attendees
                .alive_attendees_of_event_scoped(&mut *tx, base.id)
                .await?
                .into_iter()
                .map(|a| (a.user_id, a.state.to_string()))
                .collect();
        }

        let claimed: HashSet<(DateTime<Utc>, DateTime<Utc>)> =
            self.exceptions.alive_slots_scoped(&mut *tx, series.id).await?.into_iter().collect();
        let by_slot: HashMap<(DateTime<Utc>, DateTime<Utc>), Uuid> =
            members.iter().map(|m| ((m.1, m.2), m.0)).collect();
        let grid: HashSet<(DateTime<Utc>, DateTime<Utc>)> =
            slots.iter().map(|s| (s.start_at, s.stop_at)).collect();

        // Reconcile: update aligned, collect missing.
        let mut missing: Vec<&OccurrenceSlot> = Vec::new();
        for slot in &slots {
            if claimed.contains(&(slot.start_at, slot.stop_at)) {
                continue; // a claimed slot is never re-materialized
            }
            match by_slot.get(&(slot.start_at, slot.stop_at)) {
                Some(id) => {
                    self.events
                        .apply_series_template_scoped(
                            &mut *tx,
                            *id,
                            &cmd.title,
                            cmd.description.as_deref(),
                            &cmd.privacy.to_string(),
                            cmd.location.as_deref(),
                            scope.acting_user_id,
                        )
                        .await?;
                }
                None => missing.push(slot),
            }
        }

        let mut new_ids: Vec<Uuid> = Vec::with_capacity(missing.len());
        if !missing.is_empty() {
            let starts: Vec<DateTime<Utc>> = missing.iter().map(|s| s.start_at).collect();
            let stops: Vec<DateTime<Utc>> = missing.iter().map(|s| s.stop_at).collect();
            new_ids = self
                .events
                .bulk_insert_members_scoped(
                    &mut *tx,
                    scope.company_id,
                    series.id,
                    &cmd.title,
                    cmd.description.as_deref(),
                    &starts,
                    &stops,
                    &cmd.privacy.to_string(),
                    organizer,
                    cmd.location.as_deref(),
                )
                .await?;
        }

        // Copy the attendee set onto every newly materialized row.
        if !new_ids.is_empty() {
            let pairs: Vec<(Uuid, String)> = if template_attendees.is_empty() {
                vec![(organizer, "accepted".to_string())]
            } else {
                template_attendees.clone()
            };
            let borrowed: Vec<(Uuid, &str)> =
                pairs.iter().map(|(u, s)| (*u, s.as_str())).collect();
            self.attendees
                .bulk_insert_scoped(
                    &mut *tx,
                    scope.company_id,
                    &new_ids,
                    &borrowed,
                    scope.acting_user_id,
                )
                .await?;
        }

        // Drifted survivors: claim + detach (never destroy).
        for m in &members {
            let key = (m.1, m.2);
            if !grid.contains(&key) && !claimed.contains(&key) {
                self.exceptions
                    .claim_slot_scoped(
                        &mut *tx,
                        scope.company_id,
                        series.id,
                        m.0,
                        m.1,
                        m.2,
                        "edited",
                        scope.acting_user_id,
                    )
                    .await?;
                self.events
                    .detach_from_series_scoped(&mut *tx, m.0, scope.acting_user_id)
                    .await?;
            }
        }

        // base_event_id: whichever row now sits on slot 0 (aligned or newly
        // inserted); if slot 0 is claimed (its occurrence was edited away or
        // cancelled), the pointer stays where it was.
        let mut new_base = series.base_event_id;
        let slot0 = (slots[0].start_at, slots[0].stop_at);
        if !claimed.contains(&slot0) {
            if let Some(id) = by_slot.get(&slot0) {
                new_base = *id;
            } else if let Some(pos) = missing.iter().position(|s| (s.start_at, s.stop_at) == slot0)
            {
                new_base = new_ids[pos];
            }
        }

        self.series
            .update_series_rule_scoped(
                &mut *tx,
                series.id,
                cmd.name.as_deref(),
                &cmd.freq.to_string(),
                cmd.interval,
                cmd.by_weekday.as_deref(),
                cmd.by_monthday.as_deref(),
                cmd.until,
                cmd.count,
                new_base,
            )
            .await?;

        tx.commit().await?;
        Ok(())
    }

    /// Edit a single occurrence. Scope `this`: claim the occurrence's current
    /// (start, stop) slot as `edited`, detach the row (series_id = NULL),
    /// apply the field edits — the row survives standalone ("editing one
    /// occurrence splits it from the series into an exception row"). Scope
    /// `following`: every member at or after the target start detaches the
    /// same way and the series rule trims `until` to the day before the
    /// split; field edits apply to the target occurrence only, and nothing is
    /// regenerated (the tail became standalone rows).
    pub async fn edit_occurrence(
        &self,
        cmd: EditOccurrenceCmd,
        scope: ScopeCtx,
    ) -> Result<(), EventFamilyError> {
        if let Some(title) = cmd.title.as_deref() {
            Self::validate_title(title)?;
        }

        let mut tx = self
            .events
            .begin_scope(&self.pool, scope.company_id, scope.acting_user_id)
            .await?;

        let Some(event) = self.events.find_by_id_scoped(&mut *tx, cmd.event_id).await? else {
            return Err(EventFamilyError::NotFound);
        };

        match cmd.scope {
            EditScope::This => {
                if let Some(series_id) = event.series_id {
                    self.exceptions
                        .claim_slot_scoped(
                            &mut *tx,
                            scope.company_id,
                            series_id,
                            event.id,
                            event.start_at,
                            event.stop_at,
                            "edited",
                            scope.acting_user_id,
                        )
                        .await?;
                    self.events
                        .detach_from_series_scoped(&mut *tx, event.id, scope.acting_user_id)
                        .await?;
                }
                let new_start = cmd.start_at.unwrap_or(event.start_at);
                let new_stop = cmd.stop_at.unwrap_or(event.stop_at);
                if new_stop <= new_start {
                    return Err(EventFamilyError::Validation(
                        "stop_at must be after start_at".into(),
                    ));
                }
                self.events
                    .apply_edits_scoped(
                        &mut *tx,
                        event.id,
                        cmd.title.as_deref(),
                        cmd.description.as_deref(),
                        cmd.start_at,
                        cmd.stop_at,
                        cmd.privacy.as_ref().map(|p| p.to_string()).as_deref(),
                        cmd.location.as_deref(),
                        scope.acting_user_id,
                    )
                    .await?;
            }
            EditScope::Following => {
                let Some(series_id) = event.series_id else {
                    return Err(EventFamilyError::Validation(
                        "edit scope `following` applies to a series member; this event is \
                         standalone — use scope `this`"
                            .into(),
                    ));
                };
                let members = self.events.member_rows_scoped(&mut *tx, series_id).await?;
                for m in &members {
                    if m.1 >= event.start_at {
                        self.exceptions
                            .claim_slot_scoped(
                                &mut *tx,
                                scope.company_id,
                                series_id,
                                m.0,
                                m.1,
                                m.2,
                                "edited",
                                scope.acting_user_id,
                            )
                            .await?;
                        self.events
                            .detach_from_series_scoped(&mut *tx, m.0, scope.acting_user_id)
                            .await?;
                    }
                }
                let new_start = cmd.start_at.unwrap_or(event.start_at);
                let new_stop = cmd.stop_at.unwrap_or(event.stop_at);
                if new_stop <= new_start {
                    return Err(EventFamilyError::Validation(
                        "stop_at must be after start_at".into(),
                    ));
                }
                self.events
                    .apply_edits_scoped(
                        &mut *tx,
                        event.id,
                        cmd.title.as_deref(),
                        cmd.description.as_deref(),
                        cmd.start_at,
                        cmd.stop_at,
                        cmd.privacy.as_ref().map(|p| p.to_string()).as_deref(),
                        cmd.location.as_deref(),
                        scope.acting_user_id,
                    )
                    .await?;
                // Trim the rule to the day BEFORE the split slot (inclusive
                // `until` semantics: the day before is in, the split day out).
                let split_date = event.start_at.date_naive();
                let trimmed_until = split_date
                    .checked_sub_days(Days::new(1))
                    .ok_or_else(|| EventFamilyError::Validation("split date out of range".into()))?;
                self.series
                    .trim_series_until_scoped(&mut *tx, series_id, trimmed_until)
                    .await?;
            }
        }

        tx.commit().await?;
        Ok(())
    }

    /// Delete a single occurrence: soft-delete the row and claim its slot as
    /// `cancelled` so rewrites never resurrect it. A standalone event simply
    /// soft-deletes (no series to claim against).
    pub async fn delete_occurrence(
        &self,
        event_id: Uuid,
        scope: ScopeCtx,
    ) -> Result<(), EventFamilyError> {
        let mut tx = self
            .events
            .begin_scope(&self.pool, scope.company_id, scope.acting_user_id)
            .await?;

        let Some(event) = self.events.find_by_id_scoped(&mut *tx, event_id).await? else {
            return Err(EventFamilyError::NotFound);
        };

        if let Some(series_id) = event.series_id {
            self.exceptions
                .claim_slot_scoped(
                    &mut *tx,
                    scope.company_id,
                    series_id,
                    event.id,
                    event.start_at,
                    event.stop_at,
                    "cancelled",
                    scope.acting_user_id,
                )
                .await?;
        }

        let rows = self
            .events
            .soft_delete_scoped(&mut *tx, event.id, scope.acting_user_id)
            .await?;
        if rows == 0 {
            return Err(EventFamilyError::NotFound);
        }

        tx.commit().await?;
        Ok(())
    }

    /// Attach attendees to an event: dedup app-side (first-wins within the
    /// request; users already attending are rejected up front with the exact
    /// user id), then insert with DB-minted access tokens. The DB partial
    /// unique index on (event_id, user_id) is the backstop that survives every
    /// other write path — a 23505 on it surfaces as `DuplicateAttendee` (the
    /// 409 the HTTP layer reports). Attaching to a series member attaches to
    /// every live member of the series, so the privacy fence treats the new
    /// participant as an attendee of each occurrence they can now see.
    pub async fn attach_attendees(
        &self,
        cmd: AttachAttendeesCmd,
        scope: ScopeCtx,
    ) -> Result<(), EventFamilyError> {
        let mut tx = self
            .events
            .begin_scope(&self.pool, scope.company_id, scope.acting_user_id)
            .await?;

        let Some(event) = self.events.find_by_id_scoped(&mut *tx, cmd.event_id).await? else {
            return Err(EventFamilyError::NotFound);
        };

        let existing: HashSet<Uuid> = self
            .attendees
            .alive_attendees_of_event_scoped(&mut *tx, event.id)
            .await?
            .into_iter()
            .map(|a| a.user_id)
            .collect();
        let invited = Self::dedup_user_ids(&cmd.attendee_user_ids);
        if let Some(user_id) = invited.iter().find(|u| existing.contains(*u)) {
            return Err(EventFamilyError::DuplicateAttendee { user_id: *user_id });
        }

        let target_event_ids: Vec<Uuid> = match event.series_id {
            Some(series_id) => {
                self.events.member_rows_scoped(&mut *tx, series_id).await?.into_iter().map(|m| m.0).collect()
            }
            None => vec![event.id],
        };

        if !invited.is_empty() && !target_event_ids.is_empty() {
            let states: Vec<(Uuid, &str)> =
                invited.iter().map(|u| (*u, "needs_action")).collect();
            self.attendees
                .bulk_insert_scoped(
                    &mut *tx,
                    scope.company_id,
                    &target_event_ids,
                    &states,
                    scope.acting_user_id,
                )
                .await
                .map_err(Self::map_attendee_violation)?;
        }

        tx.commit().await?;
        Ok(())
    }

    /// Hand-set an attendee response state (needs_action / accepted /
    /// declined / tentative). Faithful to the reference state machine: no
    /// validation gate beyond the enum — the states are hand-set by design.
    /// The addressed attendee row is the one updated; invitation answer flows
    /// that fan a response across a series are a later wave.
    pub async fn set_attendee_state(
        &self,
        cmd: SetAttendeeStateCmd,
        scope: ScopeCtx,
    ) -> Result<(), EventFamilyError> {
        let mut tx = self
            .events
            .begin_scope(&self.pool, scope.company_id, scope.acting_user_id)
            .await?;

        let rows = self
            .attendees
            .set_state_scoped(&mut *tx, cmd.attendee_id, &cmd.state.to_string(), scope.acting_user_id)
            .await?;
        if rows == 0 {
            return Err(EventFamilyError::NotFound);
        }

        tx.commit().await?;
        Ok(())
    }

    /// Create a standalone event (no series), organizer auto-attendee
    /// accepted, optional attendees deduped.
    pub async fn create_standalone(
        &self,
        cmd: CreateStandaloneCmd,
        scope: ScopeCtx,
    ) -> Result<Uuid, EventFamilyError> {
        Self::validate_title(&cmd.title)?;
        if cmd.stop_at <= cmd.start_at {
            return Err(EventFamilyError::Validation("stop_at must be after start_at".into()));
        }

        let mut tx = self
            .events
            .begin_scope(&self.pool, scope.company_id, scope.acting_user_id)
            .await?;

        let event_id = self
            .events
            .insert_event_scoped(
                &mut *tx,
                scope.company_id,
                None,
                &cmd.title,
                cmd.description.as_deref(),
                cmd.start_at,
                cmd.stop_at,
                &cmd.privacy.to_string(),
                scope.acting_user_id,
                cmd.location.as_deref(),
            )
            .await?;

        let attendee_set =
            Self::attendee_set_with_organizer(scope.acting_user_id, &cmd.attendee_user_ids);
        if !attendee_set.is_empty() {
            self.attendees
                .bulk_insert_scoped(
                    &mut *tx,
                    scope.company_id,
                    &[event_id],
                    &attendee_set,
                    scope.acting_user_id,
                )
                .await
                .map_err(Self::map_attendee_violation)?;
        }

        tx.commit().await?;
        Ok(event_id)
    }
}

// ---------------------------------------------------------------------------
// Engine internals: validation, dedup, error mapping
// ---------------------------------------------------------------------------

impl CalendarEventSeriesEngine {
    /// Validate a rule against its first occurrence. Returns the parsed rule.
    /// Every check is loud (400-shaped `Validation`); the DB CHECK on
    /// `stop_at > start_at` backstops the time order that reaches SQL.
    #[allow(clippy::too_many_arguments)]
    fn validate_rule(
        freq: EventRecurrenceFreq,
        interval: i32,
        by_weekday: Option<&str>,
        by_monthday: Option<&str>,
        until: Option<NaiveDate>,
        count: Option<i32>,
        first_start: DateTime<Utc>,
        first_stop: DateTime<Utc>,
    ) -> Result<RecurrenceRule, EventFamilyError> {
        if first_stop <= first_start {
            return Err(EventFamilyError::Validation(
                "stop_at must be after start_at".into(),
            ));
        }
        if interval < 1 {
            return Err(EventFamilyError::Validation(
                "interval must be at least 1".into(),
            ));
        }
        if let Some(c) = count {
            if c < 1 {
                return Err(EventFamilyError::Validation(
                    "count must be at least 1".into(),
                ));
            }
        }
        if let Some(u) = until {
            if u < first_start.date_naive() {
                return Err(EventFamilyError::Validation(
                    "until must not be before the first occurrence's date".into(),
                ));
            }
        }
        let weekdays = parse_day_list(by_weekday, 1, 7, "by_weekday (ISO 1=Mon..7=Sun)")?;
        let monthdays = parse_day_list(by_monthday, 1, 31, "by_monthday (1..=31)")?;
        Ok(RecurrenceRule { freq, interval, by_weekday: weekdays, by_monthday: monthdays, until, count })
    }

    fn validate_title(title: &str) -> Result<(), EventFamilyError> {
        let trimmed = title.trim();
        if trimmed.is_empty() {
            return Err(EventFamilyError::Validation("title must not be empty".into()));
        }
        if trimmed.len() > 200 {
            return Err(EventFamilyError::Validation(
                "title must be at most 200 characters".into(),
            ));
        }
        Ok(())
    }

    /// First-wins order-preserving dedup of user ids (the application half of
    /// the attendee dedup contract; the DB partial unique index is the
    /// backstop).
    fn dedup_user_ids(ids: &[Uuid]) -> Vec<Uuid> {
        let mut seen = HashSet::new();
        ids.iter().filter(|u| seen.insert(**u)).copied().collect()
    }

    /// The attendee set for a fresh event: the organizer first with state
    /// `accepted` (the reference create() auto-accepts the organizer), every
    /// invited user once with `needs_action`. First-wins dedup.
    fn attendee_set_with_organizer(
        organizer: Uuid,
        invited: &[Uuid],
    ) -> Vec<(Uuid, &'static str)> {
        let mut seen = HashSet::from([organizer]);
        let mut set = vec![(organizer, "accepted")];
        for user_id in invited {
            if seen.insert(*user_id) {
                set.push((*user_id, "needs_action"));
            }
        }
        set
    }

    /// Map a SQL unique violation on `uq_calendar_event_attendees_event_user`
    /// to [`EventFamilyError::DuplicateAttendee`], parsing the offending
    /// user_id out of the violation detail when possible. Anything else is
    /// surfaced as the raw database error.
    fn map_attendee_violation(err: sqlx::Error) -> EventFamilyError {
        if let Some(db) = err.as_database_error() {
            let attendee_backstop =
                db.constraint().is_some_and(|c| c == "uq_calendar_event_attendees_event_user");
            if attendee_backstop && db.code().as_deref() == Some("23505") {
                let user_id = db
                    .as_error()
                    .downcast_ref::<sqlx::postgres::PgDatabaseError>()
                    .and_then(|pg| pg.detail())
                    .and_then(second_uuid_in_key_detail)
                    .unwrap_or(Uuid::nil());
                return EventFamilyError::DuplicateAttendee { user_id };
            }
        }
        EventFamilyError::Db(err)
    }
}

/// Parse a comma list of day numbers (`"1,3"`) into validated bytes within
/// `min..=max`. Empty/blank input yields an empty vector (field unset).
fn parse_day_list(
    raw: Option<&str>,
    min: u8,
    max: u8,
    what: &str,
) -> Result<Vec<u8>, EventFamilyError> {
    let Some(raw) = raw else { return Ok(Vec::new()) };
    if raw.trim().is_empty() {
        return Ok(Vec::new());
    }
    let mut days = Vec::new();
    for token in raw.split(',') {
        let token = token.trim();
        if token.is_empty() {
            return Err(EventFamilyError::Validation(format!(
                "{what} must be a comma list of numbers, got `{raw}`"
            )));
        }
        let value: u8 = token.parse().map_err(|_| {
            EventFamilyError::Validation(format!("{what} must be a comma list of numbers, got `{raw}`"))
        })?;
        if !(min..=max).contains(&value) {
            return Err(EventFamilyError::Validation(format!(
                "{what} values must be between {min} and {max}, got `{value}`"
            )));
        }
        days.push(value);
    }
    days.sort_unstable();
    days.dedup();
    Ok(days)
}

/// Extract the second UUID from a Postgres unique-violation detail of the form
/// `Key (event_id, user_id)=(<uuid>, <uuid>) already exists.` — the user_id of
/// the duplicate attendee.
fn second_uuid_in_key_detail(detail: &str) -> Option<Uuid> {
    let open = detail.find(")=(")?;
    let close = detail.rfind(") already exists")?;
    let inner = detail.get(open + 3..close)?;
    let second = inner.split(',').nth(1)?.trim();
    Uuid::parse_str(second).ok()
}

// ---------------------------------------------------------------------------
// Expander unit tests (pure — no database)
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn slot(nth: usize, rule: &RecurrenceRule, first_start: DateTime<Utc>) -> OccurrenceSlot {
        expand_occurrences(rule, first_start, first_start + chrono::Duration::hours(1))
            .expect("expansion succeeds")
            .get(nth)
            .copied()
            .expect("slot exists")
    }

    fn utc(y: i32, m: u32, d: u32, h: u32, mi: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(y, m, d, h, mi, 0).unwrap()
    }

    fn rule(freq: EventRecurrenceFreq) -> RecurrenceRule {
        RecurrenceRule { freq, interval: 1, by_weekday: Vec::new(), by_monthday: Vec::new(), until: None, count: None }
    }

    #[test]
    fn daily_count_720_is_exactly_at_the_cap() {
        // 720 projected slots: at the cap, NOT over it — succeeds.
        let start = utc(2026, 1, 5, 9, 0);
        let r = RecurrenceRule { count: Some(720), ..rule(EventRecurrenceFreq::Daily) };
        let slots = expand_occurrences(&r, start, start + chrono::Duration::hours(1)).unwrap();
        assert_eq!(slots.len(), MAX_OCCURRENCES);
        // Base event is slot 0 verbatim; the last is day 719.
        assert_eq!(slots[0].start_at, start);
        assert_eq!(
            slots[719].start_at.date_naive(),
            NaiveDate::from_ymd_opt(2027, 12, 25).unwrap()
        );
    }

    #[test]
    fn daily_count_721_hits_the_cap_loudly() {
        let start = utc(2026, 1, 5, 9, 0);
        let r = RecurrenceRule { count: Some(721), ..rule(EventRecurrenceFreq::Daily) };
        match expand_occurrences(&r, start, start + chrono::Duration::hours(1)) {
            Err(EventFamilyError::RecurrenceCap { projected, cap }) => {
                assert_eq!(projected, 721);
                assert_eq!(cap, 720);
            }
            other => panic!("expected RecurrenceCap, got {other:?}"),
        }
    }

    #[test]
    fn daily_forever_is_bounded_by_the_15_year_horizon_then_caps_loudly() {
        let start = utc(2026, 1, 5, 9, 0);
        let r = rule(EventRecurrenceFreq::Daily); // neither until nor count
        match expand_occurrences(&r, start, start + chrono::Duration::hours(1)) {
            Err(EventFamilyError::RecurrenceCap { projected, cap }) => {
                assert_eq!(cap, 720);
                // 15 years of daily slots: 2026..2041 incl. leap days.
                assert!(projected > 5400, "projected {projected} should be ~15y of days");
            }
            other => panic!("expected RecurrenceCap, got {other:?}"),
        }
    }

    #[test]
    fn yearly_unbounded_stops_at_the_horizon_without_capping() {
        // 15 years of a yearly rule fits under the cap: offsets 0..=15.
        let start = utc(2026, 6, 1, 9, 0);
        let r = rule(EventRecurrenceFreq::Yearly);
        let slots = expand_occurrences(&r, start, start + chrono::Duration::hours(2)).unwrap();
        assert_eq!(slots.len(), 16);
        assert_eq!(slots[15].start_at, utc(2041, 6, 1, 9, 0));
        assert_eq!(slots[15].stop_at - slots[15].start_at, chrono::Duration::hours(2));
    }

    #[test]
    fn weekly_ten_materializes_ten_ordered_slots() {
        let start = utc(2026, 1, 5, 9, 30); // a Monday
        let r = RecurrenceRule { count: Some(10), ..rule(EventRecurrenceFreq::Weekly) };
        let slots = expand_occurrences(&r, start, start + chrono::Duration::minutes(90)).unwrap();
        assert_eq!(slots.len(), 10);
        for (i, s) in slots.iter().enumerate() {
            assert_eq!(s.start_at, start + chrono::Duration::weeks(i as i64));
            assert_eq!(s.stop_at - s.start_at, chrono::Duration::minutes(90));
        }
        // Strictly ordered by start.
        assert!(slots.windows(2).all(|w| w[0].start_at < w[1].start_at));
    }

    #[test]
    fn weekly_by_weekday_expands_within_each_week() {
        // Anchor Monday; by_weekday = Monday(1), Wednesday(3).
        let start = utc(2026, 1, 5, 9, 0);
        let r = RecurrenceRule {
            by_weekday: vec![1, 3],
            count: Some(5),
            ..rule(EventRecurrenceFreq::Weekly)
        };
        let slots = expand_occurrences(&r, start, start + chrono::Duration::hours(1)).unwrap();
        assert_eq!(slots.len(), 5);
        // Slot 0 is the anchor Monday itself; then Wed same week, Mon next
        // week, Wed next week, Mon week after.
        assert_eq!(slots[1].start_at, utc(2026, 1, 7, 9, 0));
        assert_eq!(slots[2].start_at, utc(2026, 1, 12, 9, 0));
        assert_eq!(slots[3].start_at, utc(2026, 1, 14, 9, 0));
        assert_eq!(slots[4].start_at, utc(2026, 1, 19, 9, 0));
    }

    #[test]
    fn monthly_by_monthday_skips_months_without_the_day() {
        // Anchor Jan 31; day-31 monthly skips February entirely.
        let start = utc(2026, 1, 31, 10, 0);
        let r = RecurrenceRule {
            by_monthday: vec![31],
            count: Some(3),
            ..rule(EventRecurrenceFreq::Monthly)
        };
        let slots = expand_occurrences(&r, start, start + chrono::Duration::hours(1)).unwrap();
        assert_eq!(slots.len(), 3);
        assert_eq!(slots[0].start_at.date_naive(), NaiveDate::from_ymd_opt(2026, 1, 31).unwrap());
        assert_eq!(slots[1].start_at.date_naive(), NaiveDate::from_ymd_opt(2026, 3, 31).unwrap());
        assert_eq!(slots[2].start_at.date_naive(), NaiveDate::from_ymd_opt(2026, 5, 31).unwrap());
    }

    #[test]
    fn monthly_multiple_monthdays_is_ordered_within_each_month() {
        let start = utc(2026, 1, 1, 8, 0);
        let r = RecurrenceRule {
            by_monthday: vec![15, 1],
            count: Some(4),
            ..rule(EventRecurrenceFreq::Monthly)
        };
        let slots = expand_occurrences(&r, start, start + chrono::Duration::hours(1)).unwrap();
        assert_eq!(slots.len(), 4);
        assert_eq!(slots[1].start_at.date_naive(), NaiveDate::from_ymd_opt(2026, 1, 15).unwrap());
        assert_eq!(slots[2].start_at.date_naive(), NaiveDate::from_ymd_opt(2026, 2, 1).unwrap());
        assert_eq!(slots[3].start_at.date_naive(), NaiveDate::from_ymd_opt(2026, 2, 15).unwrap());
    }

    #[test]
    fn yearly_feb29_skips_non_leap_years() {
        let start = utc(2028, 2, 29, 9, 0);
        let r = RecurrenceRule { count: Some(3), ..rule(EventRecurrenceFreq::Yearly) };
        let slots = expand_occurrences(&r, start, start + chrono::Duration::hours(1)).unwrap();
        assert_eq!(slots.len(), 3);
        assert_eq!(slots[1].start_at.date_naive(), NaiveDate::from_ymd_opt(2032, 2, 29).unwrap());
        assert_eq!(slots[2].start_at.date_naive(), NaiveDate::from_ymd_opt(2036, 2, 29).unwrap());
    }

    #[test]
    fn until_is_inclusive_on_the_slot_start_date() {
        let start = utc(2026, 1, 5, 9, 0);
        let r = RecurrenceRule {
            until: Some(NaiveDate::from_ymd_opt(2026, 1, 19).unwrap()),
            ..rule(EventRecurrenceFreq::Weekly)
        };
        let slots = expand_occurrences(&r, start, start + chrono::Duration::hours(1)).unwrap();
        // Jan 5, 12, 19 — the 19th is included (inclusive until).
        assert_eq!(slots.len(), 3);
        assert_eq!(slots[2].start_at.date_naive(), NaiveDate::from_ymd_opt(2026, 1, 19).unwrap());
    }

    #[test]
    fn interval_two_weeks_doubles_the_spacing() {
        let start = utc(2026, 1, 5, 9, 0);
        let r = RecurrenceRule {
            interval: 2,
            count: Some(3),
            ..rule(EventRecurrenceFreq::Weekly)
        };
        let slots = expand_occurrences(&r, start, start + chrono::Duration::hours(1)).unwrap();
        assert_eq!(slots.len(), 3);
        assert_eq!(slots[1].start_at, start + chrono::Duration::weeks(2));
        assert_eq!(slots[2].start_at, start + chrono::Duration::weeks(4));
    }

    #[test]
    fn count_one_is_the_base_event_alone() {
        let start = utc(2026, 1, 5, 9, 0);
        let r = RecurrenceRule { count: Some(1), ..rule(EventRecurrenceFreq::Daily) };
        let slots = expand_occurrences(&r, start, start + chrono::Duration::hours(1)).unwrap();
        assert_eq!(slots.len(), 1);
        assert_eq!(slots[0].start_at, start);
    }

    #[test]
    fn daily_forever_respects_interval_in_horizon_math() {
        // Every-other-day forever still caps loudly, with roughly half the
        // daily projection.
        let start = utc(2026, 1, 5, 9, 0);
        let r = RecurrenceRule { interval: 2, ..rule(EventRecurrenceFreq::Daily) };
        match expand_occurrences(&r, start, start + chrono::Duration::hours(1)) {
            Err(EventFamilyError::RecurrenceCap { projected, .. }) => {
                assert!(projected > 2700 && projected < 2800, "projected {projected}");
            }
            other => panic!("expected RecurrenceCap, got {other:?}"),
        }
    }

    #[test]
    fn validation_rejects_bad_rules_before_expansion() {
        let start = utc(2026, 1, 5, 9, 0);
        assert!(matches!(
            CalendarEventSeriesEngine::validate_rule(
                EventRecurrenceFreq::Daily,
                0,
                None,
                None,
                None,
                None,
                start,
                start + chrono::Duration::hours(1),
            ),
            Err(EventFamilyError::Validation(_))
        ));
        assert!(matches!(
            CalendarEventSeriesEngine::validate_rule(
                EventRecurrenceFreq::Weekly,
                1,
                Some("1,8"),
                None,
                None,
                None,
                start,
                start + chrono::Duration::hours(1),
            ),
            Err(EventFamilyError::Validation(_))
        ));
        assert!(matches!(
            CalendarEventSeriesEngine::validate_rule(
                EventRecurrenceFreq::Monthly,
                1,
                None,
                Some("32"),
                None,
                None,
                start,
                start + chrono::Duration::hours(1),
            ),
            Err(EventFamilyError::Validation(_))
        ));
        assert!(matches!(
            CalendarEventSeriesEngine::validate_rule(
                EventRecurrenceFreq::Daily,
                1,
                None,
                None,
                Some(NaiveDate::from_ymd_opt(2025, 12, 31).unwrap()),
                None,
                start,
                start + chrono::Duration::hours(1),
            ),
            Err(EventFamilyError::Validation(_))
        ));
        // stop_at <= start_at is rejected.
        assert!(matches!(
            CalendarEventSeriesEngine::validate_rule(
                EventRecurrenceFreq::Daily,
                1,
                None,
                None,
                None,
                None,
                start,
                start,
            ),
            Err(EventFamilyError::Validation(_))
        ));
    }

    #[test]
    fn both_bounds_bind_by_whichever_comes_first() {
        // until lands before count is exhausted → until wins.
        let start = utc(2026, 1, 5, 9, 0);
        let r = RecurrenceRule {
            until: Some(NaiveDate::from_ymd_opt(2026, 1, 12).unwrap()),
            count: Some(10),
            ..rule(EventRecurrenceFreq::Weekly)
        };
        let slots = expand_occurrences(&r, start, start + chrono::Duration::hours(1)).unwrap();
        assert_eq!(slots.len(), 2);
        // count exhausted before until → count wins.
        let r = RecurrenceRule {
            until: Some(NaiveDate::from_ymd_opt(2036, 1, 1).unwrap()),
            count: Some(3),
            ..rule(EventRecurrenceFreq::Weekly)
        };
        let slots = expand_occurrences(&r, start, start + chrono::Duration::hours(1)).unwrap();
        assert_eq!(slots.len(), 3);
    }

    #[test]
    fn slot_helper_and_duration_preservation() {
        let start = utc(2026, 3, 30, 14, 15);
        let r = RecurrenceRule { count: Some(4), ..rule(EventRecurrenceFreq::Weekly) };
        let s = slot(2, &r, start);
        assert_eq!(s.start_at, start + chrono::Duration::weeks(2));
        assert_eq!(s.stop_at, s.start_at + chrono::Duration::hours(1));
    }
}
