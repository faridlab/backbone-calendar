//! Event-family probes — DB-backed, in-process, against the guarded router.
//!
//! Proves the declared behavior of the calendar event family (docs/event-family.md):
//!
//! P1  cap-loud            — a rule projecting past the 720-occurrence cap fails LOUD
//!                           (422 + CALENDAR_EVENT_RECURRENCE_CAP_EXCEEDED, whole-tx
//!                           rollback, zero rows); boundary exact: 721 errors, 720
//!                           succeeds with 720 real member rows.
//! P2  eager-materialize   — weekly x10 writes exactly 10 real rows (base = slot 0)
//!                           and the occurrences endpoint returns 10.
//! P3  edit-one-splits     — PATCH scope `this` detaches the row (series_id NULL),
//!                           claims its old (start, stop) as an `edited` exception,
//!                           keeps the row id stable; 9 members, 10 rows total.
//! P4  rewrite-by-identity — PUT reconciles by (start, stop): aligned rows keep ids,
//!                           missing slots materialize, claimed slots stay skipped,
//!                           the detached row is untouched; a time shift never
//!                           destroys data (old members update in place or detach —
//!                           never vanish).
//! P5  delete-sticks       — DELETE one occurrence + a later series rewrite does NOT
//!                           resurrect it.
//! P6  attendee-db-dedup   — API duplicate → 409 CALENDAR_ATTENDEE_DUPLICATE, AND a
//!                           raw SQL insert hits unique index 23505 (the DB
//!                           constraint, not application filtering).
//! P7  cross-fence-inv.    — privacy as a declared read fence: same company, another
//!                           user sees ONLY public rows; the organizer sees all; an
//!                           attendee sees the attended private event; unset
//!                           app.user_id ⇒ public-only (fail-closed). Proven at HTTP
//!                           level AND at SQL level on two scoped connections.
//! P8  company-fence       — a different app.company_id sees zero event-family rows
//!                           and cannot write cross-company (strict RLS on all four
//!                           tables).
//! P9  this+following      — the tail detaches standalone, the series rule trims its
//!                           `until` to the day before the split, the head is intact.
//! P10 migration hygiene   — full up→down→up reversibility is a RECIPE step (see
//!                           docs/event-family.md), not an in-process test; it runs
//!                           in the review pass around this suite.
//!
//! Skipping: without DATABASE_URL the suite skips LOUD (prints SKIP + reason per
//! test) — the module is a library; providing the migrated DB is the reviewer's job.
//!
//! Recipe (fresh scratch DB, module migrations in filename order, then this suite):
//!
//! ```text
//! psql 'postgresql://postgres:postgres@127.0.0.1:5433/postgres' -c 'CREATE DATABASE calendar_sv2_probe'
//! for f in migrations/*.up.sql; do
//!   psql -v ON_ERROR_STOP=1 -q -f "$f" 'postgresql://postgres:postgres@127.0.0.1:5433/calendar_sv2_probe'
//! done
//! DATABASE_URL='postgresql://postgres:postgres@127.0.0.1:5433/calendar_sv2_probe' \
//!   cargo test --features auth --test calendar_event_family_probes -- --nocapture
//! ```
//!
//! Every test mints a fresh random company id so parallel runs never collide. The
//! fence probes (SQL- AND HTTP-level) run as a minted non-superuser, NOBYPASSRLS
//! probe role — the only session posture under which Row-Level Security actually
//! binds (superusers bypass it), mirroring the fence suites of backbone-selling /
//! backbone-attendance.

use axum::body::Body;
use axum::http::{Request, StatusCode};
use chrono::Utc;
use sqlx::{Acquire, PgPool};
use tower::ServiceExt;
use uuid::Uuid;

// ─────────────────────────────────────────────────────────────────────────────
// DB harness
// ─────────────────────────────────────────────────────────────────────────────

/// DATABASE_URL or a loud skip. Return `None` after printing why.
fn database_url() -> Option<String> {
    match std::env::var("DATABASE_URL") {
        Ok(url) if !url.trim().is_empty() => Some(url),
        _ => {
            eprintln!(
                "SKIP (loud): DATABASE_URL is not set — {} needs a migrated database. \
                 Recipe: see the module doc comment of this file / docs/event-family.md.",
                module_path!()
            );
            None
        }
    }
}

/// Admin pool (superuser): used for seeding assertions and role minting only —
/// fence probes deliberately run on the restricted probe role instead.
async fn admin_pool() -> PgPool {
    let url = database_url().expect("caller checked");
    PgPool::connect(&url).await.expect("connect admin pool")
}

/// Count live (not soft-deleted) member rows of a series.
async fn member_count(admin: &PgPool, series: Uuid) -> i64 {
    sqlx::query_scalar(
        "SELECT COUNT(*) FROM calendar.events \
         WHERE series_id = $1 AND (metadata->>'deleted_at') IS NULL",
    )
    .bind(series)
    .fetch_one(admin)
    .await
    .expect("count members")
}

/// Count ALL live event rows of a company (members + standalone).
async fn company_event_count(admin: &PgPool, company: Uuid) -> i64 {
    sqlx::query_scalar(
        "SELECT COUNT(*) FROM calendar.events \
         WHERE company_id = $1 AND (metadata->>'deleted_at') IS NULL",
    )
    .bind(company)
    .fetch_one(admin)
    .await
    .expect("count company events")
}

// ─────────────────────────────────────────────────────────────────────────────
// Restricted probe role (non-superuser, NOBYPASSRLS — RLS binds only here)
// ─────────────────────────────────────────────────────────────────────────────

const PROBE_ROLE: &str = "calendar_fence_probe";
const PROBE_PASSWORD: &str = "probe";

/// Rebuild DATABASE_URL aimed at the probe role, keeping host/port/database.
fn restricted_url(admin_url: &str) -> String {
    let rest = admin_url
        .trim_start_matches("postgresql://")
        .trim_start_matches("postgres://");
    let (authority, path) = rest.split_once('/').expect("DATABASE_URL must name a database");
    let hostport = authority.rsplit_once('@').map(|(_, h)| h).unwrap_or(authority);
    let db = path.split('?').next().unwrap_or("postgres");
    format!("postgresql://{PROBE_ROLE}:{PROBE_PASSWORD}@{hostport}/{db}")
}

/// Mint (idempotently) and connect the restricted probe role. RLS only binds for a
/// non-BYPASSRLS session, so the SQL-level fence probes must run as this role — under
/// the admin superuser connection every fence below would pass vacuously.
async fn fenced_pool(admin: &PgPool) -> PgPool {
    let url = database_url().expect("caller checked");
    let db = url
        .trim_start_matches("postgresql://")
        .trim_start_matches("postgres://")
        .split_once('/')
        .and_then(|(_, path)| path.split('?').next())
        .unwrap_or("postgres")
        .to_string();

    // Serialize mint + grants across parallel tests (shared-catalog DDL does not
    // tolerate concurrent GRANTs); tolerate losing the race — same role, same shape.
    sqlx::query(&format!(
        "SELECT pg_advisory_lock(hashtext('{PROBE_ROLE}'))"
    ))
    .execute(admin)
    .await
    .expect("take probe mint lock");
    let _ = sqlx::query(&format!(
        "CREATE ROLE {PROBE_ROLE} LOGIN PASSWORD '{PROBE_PASSWORD}' \
           NOSUPERUSER NOBYPASSRLS NOCREATEDB NOCREATEROLE"
    ))
    .execute(admin)
    .await;
    for grant in [
        format!(r#"GRANT CONNECT ON DATABASE "{db}" TO {PROBE_ROLE}"#),
        format!("GRANT USAGE ON SCHEMA calendar TO {PROBE_ROLE}"),
        format!("GRANT SELECT, INSERT, UPDATE ON TABLE calendar.events TO {PROBE_ROLE}"),
        format!("GRANT SELECT, INSERT, UPDATE ON TABLE calendar.event_series TO {PROBE_ROLE}"),
        format!("GRANT SELECT, INSERT, UPDATE ON TABLE calendar.event_exceptions TO {PROBE_ROLE}"),
        format!("GRANT SELECT, INSERT, UPDATE ON TABLE calendar.event_attendees TO {PROBE_ROLE}"),
    ] {
        sqlx::query(&grant).execute(admin).await.expect("grant probe role");
    }
    sqlx::query(&format!(
        "SELECT pg_advisory_unlock(hashtext('{PROBE_ROLE}'))"
    ))
    .execute(admin)
    .await
    .expect("release probe mint lock");

    PgPool::connect(&restricted_url(&url))
        .await
        .expect("connect as restricted probe")
}

/// Count `calendar.events` on one scoped transaction: acquires a connection, begins a
/// tx, pins `app.company_id` (and `app.user_id` when given) via `set_config(..., true)`
/// INSIDE it — the same pinning the engine uses — then counts under the RLS fences.
/// `user = None` models a session with no acting user (the fail-closed posture).
async fn scoped_event_count(pool: &PgPool, company: Uuid, user: Option<Uuid>) -> i64 {
    let mut conn = pool.acquire().await.expect("acquire scoped connection");
    let mut tx = conn.begin().await.expect("begin scoped tx");
    if let Some(u) = user {
        sqlx::query(
            "SELECT set_config('app.company_id', $1, true), set_config('app.user_id', $2, true)",
        )
        .bind(company.to_string())
        .bind(u.to_string())
        .execute(&mut *tx)
        .await
        .expect("pin company + user");
    } else {
        sqlx::query("SELECT set_config('app.company_id', $1, true)")
            .bind(company.to_string())
            .execute(&mut *tx)
            .await
            .expect("pin company");
    }
    sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM calendar.events")
        .fetch_one(&mut *tx)
        .await
        .expect("scoped count")
}

// ─────────────────────────────────────────────────────────────────────────────
// HTTP harness (guarded router, in-process oneshot)
// ─────────────────────────────────────────────────────────────────────────────

#[cfg(feature = "auth")]
mod http_probe {
    use super::*;
    use axum::middleware::{from_fn, from_fn_with_state, Next};
    use axum::response::Response;
    use backbone_auth::company::{company_auth, CompanyVerifier};
    use backbone_auth::middleware::AuthContext;
    use backbone_calendar::CalendarModule;

    const SECRET: &[u8] = b"calendar-event-family-probe-secret";

    /// The acting principal: company + user from the signed token; permissions from
    /// the probe header layer below (the composing service supplies these in
    /// production from its session; a library module must not invent transport).
    #[derive(Clone, Copy)]
    pub struct Actor {
        pub company_id: Uuid,
        pub user_id: Uuid,
    }

    fn token(a: Actor) -> String {
        let exp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs() as usize
            + 3600;
        let claims =
            serde_json::json!({"sub": a.user_id.to_string(), "company_id": a.company_id, "exp": exp});
        jsonwebtoken::encode(
            &jsonwebtoken::Header::default(),
            &claims,
            &jsonwebtoken::EncodingKey::from_secret(SECRET),
        )
        .unwrap()
    }

    /// Insert an `AuthContext` (the permission currency of backbone-auth) built from
    /// probe headers, standing in for the composing service's session middleware.
    /// Grants the full calendar_event vocabulary when the header is absent.
    async fn probe_auth_context(mut req: Request<Body>, next: Next) -> Response {
        let user = req
            .headers()
            .get("x-probe-user")
            .and_then(|v| v.to_str().ok())
            .map(str::to_string)
            .unwrap_or_default();
        let permissions: Vec<String> = req
            .headers()
            .get("x-probe-permissions")
            .and_then(|v| v.to_str().ok())
            .map(|s| {
                s.split(',')
                    .map(|p| p.trim().to_string())
                    .filter(|p| !p.is_empty())
                    .collect()
            })
            .unwrap_or_else(|| {
                ["list", "read", "create", "update", "delete", "restore", "trash", "bulk_create", "upsert"]
                    .iter()
                    .map(|a| format!("calendar_event:{a}"))
                    .collect()
            });
        req.extensions_mut().insert(AuthContext { user_id: user, roles: vec![], permissions });
        next.run(req).await
    }

    /// The guarded event-family router mounted the way a composing service mounts it:
    /// company identity from a signed token via the REAL `company_auth` middleware.
    pub fn app(pool: &PgPool) -> axum::Router {
        let module = CalendarModule::builder()
            .with_database(pool.clone())
            .build()
            .expect("build calendar module");
        module
            .calendar_event_routes()
            .route_layer(from_fn_with_state(CompanyVerifier::hs256(SECRET), company_auth))
            .layer(from_fn(probe_auth_context))
    }

    /// Send one request as `actor`; returns (status, body).
    pub async fn send(
        app: axum::Router,
        actor: Actor,
        method: &str,
        uri: &str,
        body: Option<String>,
    ) -> (StatusCode, String) {
        send_as(app, actor, method, uri, body, &[]).await
    }

    /// `send` with extra probe headers (used to model principals with explicit,
    /// possibly empty, permission sets).
    pub async fn send_as(
        app: axum::Router,
        actor: Actor,
        method: &str,
        uri: &str,
        body: Option<String>,
        extra_headers: &[(&str, &str)],
    ) -> (StatusCode, String) {
        let b = body.map(Body::from).unwrap_or(Body::empty());
        let mut builder = Request::builder()
            .method(method)
            .uri(uri)
            .header("content-type", "application/json")
            .header("authorization", format!("Bearer {}", token(actor)))
            .header("x-probe-user", actor.user_id.to_string());
        for (k, v) in extra_headers {
            builder = builder.header(*k, *v);
        }
        let req = builder.body(b).expect("build probe request");
        let resp = app.oneshot(req).await.expect("oneshot probe request");
        let status = resp.status();
        let bytes = axum::body::to_bytes(resp.into_body(), 1024 * 1024).await.expect("read body");
        (status, String::from_utf8_lossy(&bytes).to_string())
    }

    /// Ids out of a JSON response that is either `{items:[...]}` or a bare array.
    pub fn ids_of(body: &str) -> Vec<Uuid> {
        let v: serde_json::Value = serde_json::from_str(body).expect("json body");
        let arr = v
            .get("items")
            .and_then(|i| i.as_array())
            .cloned()
            .or_else(|| v.as_array().cloned())
            .expect("items array");
        arr.iter()
            .filter_map(|i| i.get("id").and_then(|i| i.as_str()))
            .map(|s| s.parse().expect("uuid id"))
            .collect()
    }

    pub fn id_of(body: &str) -> Uuid {
        serde_json::from_str::<serde_json::Value>(body).expect("json body")
            .get("id")
            .and_then(|i| i.as_str())
            .expect("id field")
            .parse()
            .expect("uuid")
    }
}

#[cfg(feature = "auth")]
use http_probe::{app, id_of, ids_of, send, send_as, Actor};

// ─────────────────────────────────────────────────────────────────────────────
// Shared JSON bodies for the guarded surface (camelCase; the module's DTOs accept
// snake_case aliases too). Centralized so a body-shape change is a one-place edit.
// ─────────────────────────────────────────────────────────────────────────────

#[cfg(feature = "auth")]
fn series_body(title: &str, first_start: &str, first_stop: &str, freq: &str, count: Option<i32>) -> String {
    let count_json = match count {
        Some(c) => format!(r#""count":{c},"#),
        None => String::new(),
    };
    format!(
        "{{\"name\":\"probe series\",\"title\":\"{title}\",\"firstStartAt\":\"{first_start}\",\
\"firstStopAt\":\"{first_stop}\",\"privacy\":\"public\",\"freq\":\"{freq}\",\"interval\":1,{count_json}\
\"attendeeUserIds\":[]}}"
    )
}

#[cfg(feature = "auth")]
fn standalone_body(title: &str, start: &str, stop: &str, privacy: &str, attendees: &[Uuid]) -> String {
    let att = attendees.iter().map(|u| format!("\"{u}\"")).collect::<Vec<_>>().join(",");
    format!(
        "{{\"title\":\"{title}\",\"startAt\":\"{start}\",\"stopAt\":\"{stop}\",\"privacy\":\"{privacy}\",\
\"attendeeUserIds\":[{att}]}}"
    )
}

// The fixed weekly grid all series probes ride: Mondays 09:00–09:30 UTC from
// 2026-09-07 (a Monday). Ten weeks run 2026-09-07 .. 2026-11-09.
const BASE_START: &str = "2026-09-07T09:00:00Z";
const BASE_STOP: &str = "2026-09-07T09:30:00Z";

// ─────────────────────────────────────────────────────────────────────────────
// P0 — the surface is fail-closed: no token ⇒ 401, no permission ⇒ 403
// ─────────────────────────────────────────────────────────────────────────────

#[cfg(feature = "auth")]
#[tokio::test]
async fn p0_guarded_surface_fails_closed() {
    let Some(_) = database_url() else { return };
    let pool = admin_pool().await;
    let app = app(&pool);

    // No Bearer token at all: the company_auth layer must reject before any
    // handler runs.
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/events")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED, "unauthenticated ⇒ 401");

    // A valid token whose principal holds ZERO permissions: the gate must refuse
    // (x-probe-permissions set to empty ⇒ AuthContext with no permissions).
    let actor = Actor { company_id: Uuid::new_v4(), user_id: Uuid::new_v4() };
    let (status, body) = send_as(
        app, actor, "GET", "/events", None,
        &[("x-probe-permissions", "")],
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN, "permissionless principal ⇒ 403: {body}");
}

// ─────────────────────────────────────────────────────────────────────────────
// P1 — the cap fires LOUD; boundary exact at 720
// ─────────────────────────────────────────────────────────────────────────────

#[cfg(feature = "auth")]
#[tokio::test]
async fn p1_unbounded_daily_series_hits_the_cap_loud_with_zero_rows() {
    let Some(_) = database_url() else { return };
    let pool = admin_pool().await;
    let company = Uuid::new_v4();
    let organizer = Uuid::new_v4();
    let actor = Actor { company_id: company, user_id: organizer };
    let app = app(&pool);

    // Daily with neither `until` nor `count`: the 15-year horizon projects thousands
    // of slots — far past the 720 cap.
    let body = series_body("daily forever", BASE_START, BASE_STOP, "daily", None);
    let (status, resp) = send(app.clone(), actor, "POST", "/event-series", Some(body)).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "cap must be 422, got {status}: {resp}");
    assert!(
        resp.contains("CALENDAR_EVENT_RECURRENCE_CAP_EXCEEDED"),
        "error code must name the cap loudly: {resp}"
    );

    // Whole transaction rolled back: zero rows of ANY event-family kind persisted.
    let events = company_event_count(&pool, company).await;
    let series: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM calendar.event_series WHERE company_id = $1",
    )
    .bind(company)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(events, 0, "cap breach must persist zero event rows");
    assert_eq!(series, 0, "cap breach must persist zero series rows");
}

#[cfg(feature = "auth")]
#[tokio::test]
async fn p1_boundary_721_errors_and_720_materializes() {
    let Some(_) = database_url() else { return };
    let pool = admin_pool().await;
    let company = Uuid::new_v4();
    let actor = Actor { company_id: company, user_id: Uuid::new_v4() };
    let app = app(&pool);

    let (status, resp) = send(
        app.clone(), actor, "POST", "/event-series",
        Some(series_body("721", BASE_START, BASE_STOP, "daily", Some(721))),
    ).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "count=721 must fail: {resp}");

    let (status, resp) = send(
        app.clone(), actor, "POST", "/event-series",
        Some(series_body("720", BASE_START, BASE_STOP, "daily", Some(720))),
    ).await;
    assert!(status.is_success(), "count=720 must succeed: {status} {resp}");
    let series = id_of(&resp);
    let members = member_count(&pool, series).await;
    assert_eq!(members, 720, "exactly 720 member rows including the base event");
}

// ─────────────────────────────────────────────────────────────────────────────
// P2 — eager materialization writes real rows
// ─────────────────────────────────────────────────────────────────────────────

#[cfg(feature = "auth")]
#[tokio::test]
async fn p2_weekly_ten_materializes_ten_real_rows() {
    let Some(_) = database_url() else { return };
    let pool = admin_pool().await;
    let actor = Actor { company_id: Uuid::new_v4(), user_id: Uuid::new_v4() };
    let app = app(&pool);

    let (status, resp) = send(
        app.clone(), actor, "POST", "/event-series",
        Some(series_body("weekly ten", BASE_START, BASE_STOP, "weekly", Some(10))),
    ).await;
    assert!(status.is_success(), "series create: {status} {resp}");
    let series = id_of(&resp);

    let members = member_count(&pool, series).await;
    assert_eq!(members, 10, "ten occurrences = ten real calendar.events rows");

    let (status, resp) = send(
        app.clone(), actor, "GET", &format!("/event-series/{series}/occurrences"), None,
    ).await;
    assert!(status.is_success(), "occurrences endpoint: {status} {resp}");
    assert_eq!(ids_of(&resp).len(), 10, "occurrences endpoint returns 10 items");
}

// ─────────────────────────────────────────────────────────────────────────────
// P3 — editing one occurrence splits it from the series
// ─────────────────────────────────────────────────────────────────────────────

#[cfg(feature = "auth")]
#[tokio::test]
async fn p3_edit_one_occurrence_splits_into_exception_row() {
    let Some(_) = database_url() else { return };
    let pool = admin_pool().await;
    let actor = Actor { company_id: Uuid::new_v4(), user_id: Uuid::new_v4() };
    let app = app(&pool);

    let (status, resp) = send(
        app.clone(), actor, "POST", "/event-series",
        Some(series_body("split probe", BASE_START, BASE_STOP, "weekly", Some(10))),
    ).await;
    assert!(status.is_success(), "{resp}");
    let series = id_of(&resp);

    // Occurrence 3 by start_at (0-based index 2) = 2026-09-21 09:00.
    let rows: Vec<(Uuid, chrono::DateTime<Utc>)> = sqlx::query_as(
        "SELECT id, start_at FROM calendar.events \
         WHERE series_id = $1 AND (metadata->>'deleted_at') IS NULL ORDER BY start_at",
    )
    .bind(series)
    .fetch_all(&pool)
    .await
    .unwrap();
    let (occ3_id, occ3_start) = rows[2].clone();
    assert_eq!(occ3_start.to_rfc3339(), "2026-09-21T09:00:00+00:00");

    let (status, resp) = send(
        app.clone(), actor, "PATCH", &format!("/events/{occ3_id}"),
        Some(r#"{"editScope":"this","title":"detached edit","startAt":"2026-09-21T11:00:00Z","stopAt":"2026-09-21T11:30:00Z"}"#.into()),
    ).await;
    assert!(status.is_success(), "PATCH this: {status} {resp}");

    // The row survived standalone, id stable, edits applied.
    let row: (Option<Uuid>, String, chrono::DateTime<Utc>) = sqlx::query_as(
        "SELECT series_id, title, start_at FROM calendar.events WHERE id = $1",
    )
    .bind(occ3_id)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert!(row.0.is_none(), "edited occurrence must be detached (series_id NULL)");
    assert_eq!(row.1, "detached edit");
    assert_eq!(row.2.to_rfc3339(), "2026-09-21T11:00:00+00:00");

    // The ledger claimed the OLD (start, stop) slot as `edited`.
    let exc: Vec<(chrono::DateTime<Utc>, chrono::DateTime<Utc>, String)> = sqlx::query_as(
        "SELECT slot_start_at, slot_stop_at, kind::text FROM calendar.event_exceptions \
         WHERE series_id = $1 AND (metadata->>'deleted_at') IS NULL",
    )
    .bind(series)
    .fetch_all(&pool)
    .await
    .unwrap();
    assert_eq!(exc.len(), 1, "exactly one exception: {exc:?}");
    assert_eq!(exc[0].0.to_rfc3339(), "2026-09-21T09:00:00+00:00", "slot_start = the pre-edit start");
    assert_eq!(exc[0].1.to_rfc3339(), "2026-09-21T09:30:00+00:00", "slot_stop = the pre-edit stop");
    assert_eq!(exc[0].2, "edited");

    assert_eq!(member_count(&pool, series).await, 9, "9 members remain");
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM calendar.events WHERE company_id = $1 AND (metadata->>'deleted_at') IS NULL"
        )
        .bind(actor.company_id)
        .fetch_one(&pool)
        .await
        .unwrap(),
        10,
        "10 rows total: nothing destroyed by the split"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// P4 — rewrite reconciles by (start, stop) identity
// ─────────────────────────────────────────────────────────────────────────────

#[cfg(feature = "auth")]
#[tokio::test]
async fn p4_series_rewrite_reconciles_by_identity() {
    let Some(_) = database_url() else { return };
    let pool = admin_pool().await;
    let actor = Actor { company_id: Uuid::new_v4(), user_id: Uuid::new_v4() };
    let app = app(&pool);

    let (status, resp) = send(
        app.clone(), actor, "POST", "/event-series",
        Some(series_body("rewrite probe", BASE_START, BASE_STOP, "weekly", Some(10))),
    ).await;
    assert!(status.is_success(), "{resp}");
    let series = id_of(&resp);

    // Detach occurrence 3 first: its slot becomes a claimed exception.
    let rows: Vec<(Uuid, chrono::DateTime<Utc>)> = sqlx::query_as(
        "SELECT id, start_at FROM calendar.events \
         WHERE series_id = $1 AND (metadata->>'deleted_at') IS NULL ORDER BY start_at",
    )
    .bind(series)
    .fetch_all(&pool)
    .await
    .unwrap();
    let occ3_id = rows[2].0;

    let (status, resp) = send(
        app.clone(), actor, "PATCH", &format!("/events/{occ3_id}"),
        Some(r#"{"editScope":"this","title":"detached"}"#.into()),
    ).await;
    assert!(status.is_success(), "{resp}");

    let ids_before: Vec<Uuid> = sqlx::query_scalar(
        "SELECT id FROM calendar.events \
         WHERE series_id = $1 AND (metadata->>'deleted_at') IS NULL ORDER BY start_at",
    )
    .bind(series)
    .fetch_all(&pool)
    .await
    .unwrap();

    // Rewrite: SAME grid (aligned), new title, count 10 -> 12 (missing slots).
    let rewrite = r#"{"name":"probe series","title":"rewritten","firstStartAt":"2026-09-07T09:00:00Z",
        "firstStopAt":"2026-09-07T09:30:00Z","privacy":"public","freq":"weekly","interval":1,
        "count":12,"attendees":[]}"#;
    let (status, resp) = send(
        app.clone(), actor, "PUT", &format!("/event-series/{series}"), Some(rewrite.into()),
    ).await;
    assert!(status.is_success(), "series rewrite: {status} {resp}");

    let ids_after: Vec<Uuid> = sqlx::query_scalar(
        "SELECT id FROM calendar.events \
         WHERE series_id = $1 AND (metadata->>'deleted_at') IS NULL ORDER BY start_at",
    )
    .bind(series)
    .fetch_all(&pool)
    .await
    .unwrap();

    // Aligned rows keep ids; 2 missing slots materialize; 1 claimed slot skipped:
    // 9 old members + 2 new = 11 members; every pre-rewrite id survived.
    assert_eq!(ids_after.len(), 11, "11 members after rewrite: {ids_after:?}");
    for id in &ids_before {
        assert!(ids_after.contains(id), "aligned member {id} must keep its id");
    }
    // The claimed slot (2026-09-21 09:00) is NOT re-materialized.
    let claimed: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM calendar.events \
         WHERE series_id = $1 AND start_at = '2026-09-21T09:00:00Z' \
           AND (metadata->>'deleted_at') IS NULL",
    )
    .bind(series)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(claimed, 0, "a claimed slot must never be re-materialized");

    // The detached row is untouched by the rewrite.
    let det: (Option<Uuid>, String) = sqlx::query_as(
        "SELECT series_id, title FROM calendar.events WHERE id = $1",
    )
    .bind(occ3_id)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert!(det.0.is_none() && det.1 == "detached", "detached row untouched: {det:?}");

    // Phase 2 — a TIME SHIFT rewrite: by (start,stop) identity a drifted time IS an
    // exception, so old members either update in place or detach — but are never
    // destroyed, and no live member remains at the old time.
    let shift = r#"{"name":"probe series","title":"shifted","firstStartAt":"2026-09-07T10:00:00Z",
        "firstStopAt":"2026-09-07T10:30:00Z","privacy":"public","freq":"weekly","interval":1,
        "count":12,"attendees":[]}"#;
    let (status, resp) = send(
        app.clone(), actor, "PUT", &format!("/event-series/{series}"), Some(shift.into()),
    ).await;
    assert!(status.is_success(), "shifted rewrite: {status} {resp}");

    let members_at_old_time: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM calendar.events \
         WHERE series_id = $1 AND start_at::time = time '09:00' \
           AND (metadata->>'deleted_at') IS NULL",
    )
    .bind(series)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(members_at_old_time, 0, "no live member remains on the old time grid");

    // Data never destroyed: every id from before the shift is still a live row
    // (member at the new time, or standalone exception), and the series is fully
    // populated on the new grid (12 slots, none claimed on the 10:00 grid).
    for id in &ids_after {
        let alive: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM calendar.events \
             WHERE id = $1 AND (metadata->>'deleted_at') IS NULL",
        )
        .bind(id)
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(alive, 1, "row {id} must survive the shift rewrite");
    }
    let new_grid: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM calendar.events \
         WHERE series_id = $1 AND start_at::time = time '10:00' \
           AND (metadata->>'deleted_at') IS NULL",
    )
    .bind(series)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(new_grid, 12, "the new grid is fully materialized");
}

// ─────────────────────────────────────────────────────────────────────────────
// P5 — a deleted occurrence stays deleted across a series rewrite
// ─────────────────────────────────────────────────────────────────────────────

#[cfg(feature = "auth")]
#[tokio::test]
async fn p5_deleted_occurrence_is_not_resurrected_by_rewrite() {
    let Some(_) = database_url() else { return };
    let pool = admin_pool().await;
    let actor = Actor { company_id: Uuid::new_v4(), user_id: Uuid::new_v4() };
    let app = app(&pool);

    let (status, resp) = send(
        app.clone(), actor, "POST", "/event-series",
        Some(series_body("delete probe", BASE_START, BASE_STOP, "weekly", Some(10))),
    ).await;
    assert!(status.is_success(), "{resp}");
    let series = id_of(&resp);

    let occ5_id: Uuid = sqlx::query_scalar(
        "SELECT id FROM calendar.events \
         WHERE series_id = $1 AND (metadata->>'deleted_at') IS NULL ORDER BY start_at OFFSET 4 LIMIT 1",
    )
    .bind(series)
    .fetch_one(&pool)
    .await
    .unwrap();

    let (status, resp) = send(app.clone(), actor, "DELETE", &format!("/events/{occ5_id}"), None).await;
    assert!(status.is_success(), "occurrence delete: {status} {resp}");

    let deleted_at: Option<String> = sqlx::query_scalar(
        "SELECT metadata->>'deleted_at' FROM calendar.events WHERE id = $1",
    )
    .bind(occ5_id)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert!(deleted_at.is_some(), "occurrence is soft-deleted");

    let exc_kind: Option<String> = sqlx::query_scalar(
        "SELECT kind::text FROM calendar.event_exceptions \
         WHERE series_id = $1 AND event_id = $2 AND (metadata->>'deleted_at') IS NULL",
    )
    .bind(series)
    .bind(occ5_id)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(exc_kind.as_deref(), Some("cancelled"), "the slot is claimed as cancelled");

    // Rewrite touching the series (same grid, new title): the cancelled slot must
    // NOT come back.
    let touch = r#"{"name":"probe series","title":"touched","firstStartAt":"2026-09-07T09:00:00Z",
        "firstStopAt":"2026-09-07T09:30:00Z","privacy":"public","freq":"weekly","interval":1,
        "count":10,"attendees":[]}"#;
    let (status, resp) = send(
        app.clone(), actor, "PUT", &format!("/event-series/{series}"), Some(touch.into()),
    ).await;
    assert!(status.is_success(), "series touch: {status} {resp}");

    assert_eq!(member_count(&pool, series).await, 9, "the deleted occurrence is not resurrected");
    let still_deleted: Option<String> = sqlx::query_scalar(
        "SELECT metadata->>'deleted_at' FROM calendar.events WHERE id = $1",
    )
    .bind(occ5_id)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert!(still_deleted.is_some(), "the soft-deleted row stays soft-deleted");
}

// ─────────────────────────────────────────────────────────────────────────────
// P6 — attendee dedup is a DB constraint, not application filtering
// ─────────────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn p6_raw_sql_duplicate_attendee_hits_unique_23505() {
    let Some(_) = database_url() else { return };
    let pool = admin_pool().await;
    let company = Uuid::new_v4();
    let organizer = Uuid::new_v4();
    let attendee = Uuid::new_v4();

    // Seed one event + one attendee row directly (superuser bypasses RLS; unique
    // indexes do NOT care who you are — that is the point of this probe).
    let event: Uuid = sqlx::query_scalar(
        "INSERT INTO calendar.events (company_id, title, start_at, stop_at, privacy, organizer_user_id) \
         VALUES ($1, 'dedup probe', '2026-09-07T09:00:00Z', '2026-09-07T09:30:00Z', 'public', $2) \
         RETURNING id",
    )
    .bind(company)
    .bind(organizer)
    .fetch_one(&pool)
    .await
    .unwrap();

    sqlx::query(
        "INSERT INTO calendar.event_attendees (company_id, event_id, user_id) VALUES ($1, $2, $3)",
    )
    .bind(company)
    .bind(event)
    .bind(attendee)
    .execute(&pool)
    .await
    .unwrap();

    // The RAW duplicate — the exact statement a hand-run migration/script would make.
    let err = sqlx::query(
        "INSERT INTO calendar.event_attendees (company_id, event_id, user_id) VALUES ($1, $2, $3)",
    )
    .bind(company)
    .bind(event)
    .bind(attendee)
    .execute(&pool)
    .await
    .expect_err("duplicate (event, user) must be rejected by the DB");
    let code = err
        .as_database_error()
        .and_then(|d| d.code().map(|c| c.to_string()))
        .unwrap_or_default();
    assert_eq!(code, "23505", "unique-violation 23505, got {err}");
}

#[cfg(feature = "auth")]
#[tokio::test]
async fn p6_api_duplicate_attendee_is_409_with_code() {
    let Some(_) = database_url() else { return };
    let pool = admin_pool().await;
    let organizer = Uuid::new_v4();
    let guest = Uuid::new_v4();
    let actor = Actor { company_id: Uuid::new_v4(), user_id: organizer };
    let app = app(&pool);

    let (status, resp) = send(
        app.clone(), actor, "POST", "/events",
        Some(standalone_body("dedup api", BASE_START, BASE_STOP, "public", &[organizer, guest])),
    ).await;
    assert!(status.is_success(), "standalone create: {status} {resp}");
    let event = id_of(&resp);

    let (status, resp) = send(
        app.clone(), actor, "POST", &format!("/events/{event}/attendees"),
        Some(format!(r#"{{"attendeeUserIds":["{guest}"]}}"#)),
    ).await;
    assert_eq!(status, StatusCode::CONFLICT, "duplicate attendee must 409: {status} {resp}");
    assert!(resp.contains("CALENDAR_ATTENDEE_DUPLICATE"), "error code required: {resp}");
}

// ─────────────────────────────────────────────────────────────────────────────
// P7 — cross-fence invisibility (privacy read fence, DoD leg)
// ─────────────────────────────────────────────────────────────────────────────

#[cfg(feature = "auth")]
#[tokio::test]
async fn p7_privacy_fence_cross_user_invisibility_http() {
    let Some(_) = database_url() else { return };
    let admin = admin_pool().await;
    // RLS only binds for a non-BYPASSRLS session, so the HTTP-level privacy
    // proof must run the whole app on the restricted probe role — on the admin
    // (superuser) pool every privacy assertion below would pass vacuously and
    // cross-company rows would leak into the counts.
    let pool = fenced_pool(&admin).await;
    let company = Uuid::new_v4();
    let a = Uuid::new_v4(); // organizer
    let c = Uuid::new_v4(); // attendee of the private event
    let b = Uuid::new_v4(); // same company, not a participant
    let app = app(&pool);

    let as_a = Actor { company_id: company, user_id: a };
    for (title, privacy) in [("pub", "public"), ("priv", "private"), ("conf", "confidential")] {
        let attendees: Vec<Uuid> = if privacy == "private" { vec![c] } else { vec![] };
        let body = standalone_body(title, BASE_START, BASE_STOP, privacy, &attendees);
        let (status, resp) = send(app.clone(), as_a, "POST", "/events", Some(body)).await;
        assert!(status.is_success(), "create {title}: {status} {resp}");
    }

    let range = "?from=2026-09-01T00:00:00Z&to=2027-01-01T00:00:00Z";

    // B (non-participant): only the public event is visible.
    let (status, resp) =
        send(app.clone(), Actor { company_id: company, user_id: b }, "GET", &format!("/events{range}"), None).await;
    assert!(status.is_success(), "{resp}");
    assert_eq!(ids_of(&resp).len(), 1, "user B must see ONLY the public event: {resp}");

    // A (organizer): sees all three.
    let (status, resp) = send(app.clone(), as_a, "GET", &format!("/events{range}"), None).await;
    assert!(status.is_success(), "{resp}");
    assert_eq!(ids_of(&resp).len(), 3, "the organizer sees all own events: {resp}");

    // C (attendee of the private event): public + private; confidential stays invisible.
    let (status, resp) =
        send(app.clone(), Actor { company_id: company, user_id: c }, "GET", &format!("/events{range}"), None).await;
    assert!(status.is_success(), "{resp}");
    assert_eq!(ids_of(&resp).len(), 2, "attendee sees public + attended private, not confidential: {resp}");
}

#[tokio::test]
async fn p7_privacy_fence_cross_user_invisibility_sql() {
    let Some(_) = database_url() else { return };
    let admin = admin_pool().await;
    let fenced = fenced_pool(&admin).await;
    let company = Uuid::new_v4();
    let a = Uuid::new_v4();
    let b = Uuid::new_v4();
    let c = Uuid::new_v4();

    // Seed as superuser: 3 events, one per privacy class; C attends the private one.
    let mut events = vec![];
    for (privacy, attendee) in [("public", None), ("private", Some(c)), ("confidential", None)] {
        let id: Uuid = sqlx::query_scalar(
            "INSERT INTO calendar.events (company_id, title, start_at, stop_at, privacy, organizer_user_id) \
             VALUES ($1, $2, '2026-09-07T09:00:00Z', '2026-09-07T09:30:00Z', $3::event_privacy, $4) RETURNING id",
        )
        .bind(company)
        .bind(format!("sql-fence-{privacy}"))
        .bind(privacy)
        .bind(a)
        .fetch_one(&admin)
        .await
        .unwrap();
        if let Some(u) = attendee {
            sqlx::query("INSERT INTO calendar.event_attendees (company_id, event_id, user_id) VALUES ($1,$2,$3)")
                .bind(company)
                .bind(id)
                .bind(u)
                .execute(&admin)
                .await
                .unwrap();
        }
        events.push(id);
    }

    // Two scoped connections, same company, different acting users — the RLS
    // privacy fence answers differently per connection.
    assert_eq!(scoped_event_count(&fenced, company, Some(a)).await, 3, "organizer sees all three");
    assert_eq!(
        scoped_event_count(&fenced, company, Some(b)).await, 1,
        "non-participant sees only the public row"
    );
    assert_eq!(
        scoped_event_count(&fenced, company, Some(c)).await, 2,
        "attendee sees public + attended private"
    );
    // Fail-closed: no acting user pinned ⇒ only public rows readable.
    assert_eq!(
        scoped_event_count(&fenced, company, None).await, 1,
        "unset app.user_id must fail closed to public-only"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// P8 — company fence regression: a foreign company sees zero rows
// ─────────────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn p8_company_fence_zero_rows_and_no_cross_company_write() {
    let Some(_) = database_url() else { return };
    let admin = admin_pool().await;
    let fenced = fenced_pool(&admin).await;
    let company_x = Uuid::new_v4();
    let company_y = Uuid::new_v4();
    let user_x = Uuid::new_v4();

    // Seed one of each family table in company X (superuser seeding).
    let event: Uuid = sqlx::query_scalar(
        "INSERT INTO calendar.events (company_id, title, start_at, stop_at, privacy, organizer_user_id) \
         VALUES ($1, 'fence probe', '2026-09-07T09:00:00Z', '2026-09-07T09:30:00Z', 'public', $2) RETURNING id",
    )
    .bind(company_x)
    .bind(user_x)
    .fetch_one(&admin)
    .await
    .unwrap();
    let series: Uuid = sqlx::query_scalar(
        "INSERT INTO calendar.event_series (company_id, freq, base_event_id) \
         VALUES ($1, 'weekly', $2) RETURNING id",
    )
    .bind(company_x)
    .bind(event)
    .fetch_one(&admin)
    .await
    .unwrap();
    sqlx::query("UPDATE calendar.events SET series_id = $1 WHERE id = $2")
        .bind(series)
        .bind(event)
        .execute(&admin)
        .await
        .unwrap();
    sqlx::query(
        "INSERT INTO calendar.event_exceptions (company_id, series_id, event_id, slot_start_at, slot_stop_at, kind) \
         VALUES ($1, $2, $3, '2026-09-07T09:00:00Z', '2026-09-07T09:30:00Z', 'edited')",
    )
    .bind(company_x)
    .bind(series)
    .bind(event)
    .execute(&admin)
    .await
    .unwrap();
    sqlx::query("INSERT INTO calendar.event_attendees (company_id, event_id, user_id) VALUES ($1, $2, $3)")
        .bind(company_x)
        .bind(event)
        .bind(user_x)
        .execute(&admin)
        .await
        .unwrap();

    // Scoped to company Y: ZERO rows of every event-family table, and the write side
    // refuses a cross-company insert. One scoped transaction for the whole probe.
    let mut conn = fenced.acquire().await.expect("acquire fenced connection");
    let mut tx = conn.begin().await.expect("begin scoped tx");
    sqlx::query("SELECT set_config('app.company_id', $1, true), set_config('app.user_id', $2, true)")
        .bind(company_y.to_string())
        .bind(user_x.to_string())
        .execute(&mut *tx)
        .await
        .expect("pin company Y");
    for table in ["calendar.events", "calendar.event_series", "calendar.event_exceptions", "calendar.event_attendees"] {
        let n: i64 = sqlx::query_scalar(&format!("SELECT COUNT(*) FROM {table}"))
            .fetch_one(&mut *tx)
            .await
            .unwrap_or_else(|e| panic!("count {table}: {e}"));
        assert_eq!(n, 0, "company Y must see zero rows of {table}");
    }

    // And the write side: a Y-scoped session cannot insert an X-company row.
    let err = sqlx::query(
        "INSERT INTO calendar.events (company_id, title, start_at, stop_at, privacy, organizer_user_id) \
         VALUES ($1, 'cross-company attempt', '2026-09-07T09:00:00Z', '2026-09-07T09:30:00Z', 'public', $2)",
    )
    .bind(company_x)
    .bind(user_x)
    .execute(&mut *tx)
    .await
    .expect_err("cross-company insert must be refused by RLS");
    let code = err
        .as_database_error()
        .and_then(|d| d.code().map(|c| c.to_string()))
        .unwrap_or_default();
    assert_eq!(code, "42501", "RLS violation, got {err}");
}

// ─────────────────────────────────────────────────────────────────────────────
// P9 — scope `following`: the tail detaches, the rule trims
// ─────────────────────────────────────────────────────────────────────────────

#[cfg(feature = "auth")]
#[tokio::test]
async fn p9_edit_following_detaches_tail_and_trims_until() {
    let Some(_) = database_url() else { return };
    let pool = admin_pool().await;
    let actor = Actor { company_id: Uuid::new_v4(), user_id: Uuid::new_v4() };
    let app = app(&pool);

    let (status, resp) = send(
        app.clone(), actor, "POST", "/event-series",
        Some(series_body("following probe", BASE_START, BASE_STOP, "weekly", Some(10))),
    ).await;
    assert!(status.is_success(), "{resp}");
    let series = id_of(&resp);

    // Occurrence 4 (2026-09-28) — everything at or after it becomes standalone.
    let occ4_id: Uuid = sqlx::query_scalar(
        "SELECT id FROM calendar.events \
         WHERE series_id = $1 AND (metadata->>'deleted_at') IS NULL ORDER BY start_at OFFSET 3 LIMIT 1",
    )
    .bind(series)
    .fetch_one(&pool)
    .await
    .unwrap();

    let (status, resp) = send(
        app.clone(), actor, "PATCH", &format!("/events/{occ4_id}"),
        Some(r#"{"editScope":"following","title":"tail edit"}"#.into()),
    ).await;
    assert!(status.is_success(), "PATCH following: {status} {resp}");

    assert_eq!(member_count(&pool, series).await, 3, "the head (3 occurrences) stays in the series");

    let until: Option<chrono::NaiveDate> = sqlx::query_scalar(
        "SELECT until FROM calendar.event_series WHERE id = $1",
    )
    .bind(series)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(
        until.map(|d| d.to_string()),
        Some("2026-09-27".to_string()),
        "until trims to the day before the split slot (2026-09-28)"
    );

    let standalone: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM calendar.events \
         WHERE company_id = $1 AND series_id IS NULL AND (metadata->>'deleted_at') IS NULL",
    )
    .bind(actor.company_id)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(standalone, 7, "the 7-occurrence tail is standalone, data intact");

    let exc: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM calendar.event_exceptions \
         WHERE series_id = $1 AND kind = 'edited' AND (metadata->>'deleted_at') IS NULL",
    )
    .bind(series)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(exc, 7, "each detached slot is claimed in the ledger");
}
