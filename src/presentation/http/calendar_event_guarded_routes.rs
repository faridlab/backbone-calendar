//! Guarded event-family HTTP composition — the module's sole public event surface.
//!
//! Hand-written (user-owned) and auth-gated at its declaration in
//! `presentation/http/mod.rs`: without the `auth` feature this module (and with
//! it every event-family route) does not exist — fail-closed by construction.
//!
//! # What mounts here
//!
//! The host service nests this router under `/api/v1/calendar` (schema-name
//! mounting); the bases below are module-relative:
//!
//! | Method | Path | Permission |
//! |--------|------|------------|
//! | GET    | `/events` (+ `from`/`to` range filters) | `calendar_event:read` |
//! | POST   | `/events` (standalone event, optional attendees) | `calendar_event:create` |
//! | GET    | `/events/:id` | `calendar_event:read` |
//! | PUT    | `/events/:id` (edit scope defaults to `all`) | `calendar_event:update` |
//! | PATCH  | `/events/:id` (`edit_scope: this\|following\|all`, default `this`) | `calendar_event:update` |
//! | DELETE | `/events/:id` (occurrence delete sticks via the exception ledger) | `calendar_event:delete` |
//! | POST   | `/events/:id/attendees` (deduped; 409 on the DB backstop) | `calendar_event:update` |
//! | GET    | `/event-series`, POST `/event-series` | read / create |
//! | GET/PUT/DELETE | `/event-series/:id` (PUT rewrites the materialization) | read / update / delete |
//! | GET    | `/event-series/:id/occurrences` | `calendar_event:read` |
//! | PUT    | `/event-attendees/:id/state` | `calendar_event:update` |
//!
//! The exception ledger (`calendar.event_exceptions`) is deliberately UNMOUNTED:
//! it is the internal record that makes single edits and deletes stick across
//! series rewrites, not a client resource. The generated 12-endpoint CRUD for
//! all four family entities is likewise NOT mounted here — `all_crud_routes()`
//! remains the explicit trusted/admin surface for it.
//!
//! # How a request is scoped (the fences made real)
//!
//! Every handler derives its scope from two request extensions the HOST auth
//! stack provides — never from the request body:
//!
//! * [`CompanyContext`] (from `backbone_auth::company`, inserted by the host's
//!   `company_auth` layer over a signed Bearer token): the company AND the
//!   acting user. A request without it is rejected 401 by the extractor.
//! * `AuthContext` (permissions): the RBAC vocabulary check. A request without
//!   it is rejected 401 here; a caller lacking the route's permission gets 403.
//!
//! The two are combined into the engine's `ScopeCtx`, and every statement this
//! file issues runs inside a transaction that pins `app.company_id` and
//! `app.user_id` via `set_config(..., true)`. Visibility therefore comes from
//! the ROW-LEVEL-SECURITY fences (company isolation + the restrictive
//! `calendar_events_privacy_read` policy), NOT from application filtering: the
//! SQL here carries no privacy predicate at all.
//!
//! # Error map
//!
//! | Engine/domain condition | Status | Code |
//! |--------------------------|--------|------|
//! | recurrence cap exceeded (loud, whole tx rolled back) | 422 | `CALENDAR_EVENT_RECURRENCE_CAP_EXCEEDED` |
//! | duplicate attendee (DB unique backstop or app dedup) | 409 | `CALENDAR_ATTENDEE_DUPLICATE` |
//! | validation error | 400 | `CALENDAR_EVENT_VALIDATION_ERROR` |
//! | not found (or fenced out — indistinguishable by design) | 404 | `CALENDAR_EVENT_NOT_FOUND` |
//! | database error | 500 | `CALENDAR_EVENT_DATABASE_ERROR` |
//!
//! # Declared deferrals (see docs/event-family.md)
//!
//! * `/ics` export is deferred: when it lands it must be token-gated through
//!   the attendee `access_token` seam (the W7 events ruling).
//! * No availability endpoint exists here, and nothing in the event family may
//!   consult `CalendarRepository::working_days` — that port's company-wide
//!   Mon–Fri-minus-holidays simplification is working-time-family only.
//! * `calendar_sms` is a flag-only channel overlay marker in `Cargo.toml`; no
//!   transport code ships under it.

use std::sync::Arc;

use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::IntoResponse,
    routing::{get, post, put},
    Json, Router,
};
use backbone_auth::company::CompanyContext;
use backbone_auth::middleware::AuthContext;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use uuid::Uuid;

use crate::application::service::{
    calendar_event_series_service_custom as engine_surface, CalendarEventSeriesEngine,
    CalendarEventService, EventFamilyError, ScopeCtx,
};
use crate::domain::entity::{AuditMetadata, EventAttendeeState, EventPrivacy, EventRecurrenceFreq};
use crate::presentation::dto::{CalendarEventResponseDto, CalendarEventSeriesResponseDto};

// =============================================================================
// Permission vocabulary
// =============================================================================

/// The RBAC permission strings this surface gates on. Canonical source:
/// `src/application/auth/calendar_event_auth.rs` — that tree is currently NOT
/// declared in `src/application/mod.rs`, so it is unreachable from the module
/// tree and the constants are restated here (identical strings) rather than
/// imported from dead code.
mod event_permissions {
    pub const READ: &str = "calendar_event:read";
    pub const CREATE: &str = "calendar_event:create";
    pub const UPDATE: &str = "calendar_event:update";
    pub const DELETE: &str = "calendar_event:delete";
}

// =============================================================================
// State
// =============================================================================

/// Shared handler state: the series engine (all validated writes), the pool
/// (RLS-scoped reads and the few composition-level updates below), and the
/// generated CRUD service.
///
/// The generated `CalendarEventService` is held per the fixed construction
/// surface but is intentionally NOT used to serve reads here: the generic CRUD
/// path does not pin the request GUCs (`app.company_id` / `app.user_id`), so
/// under the enabled RLS fences it would see zero rows. It remains available to
/// the composing host for the trusted/admin surface (`all_crud_routes`).
#[derive(Clone)]
struct EventFamilyState {
    engine: Arc<CalendarEventSeriesEngine>,
    #[allow(dead_code)]
    events_svc: Arc<CalendarEventService>,
    pool: PgPool,
}

/// Build the guarded event-family router. Signature is fixed (the module
/// wiring in `lib.rs` calls this); the surface it mounts is documented on the
/// module.
pub fn create_calendar_event_guarded_routes(
    engine: Arc<CalendarEventSeriesEngine>,
    events_svc: Arc<CalendarEventService>,
    pool: PgPool,
) -> Router {
    let state = EventFamilyState { engine, events_svc, pool };
    Router::new()
        .route("/events", get(list_events).post(create_standalone_event))
        .route(
            "/events/:id",
            get(get_event)
                .put(replace_event)
                .patch(edit_event)
                .delete(delete_event),
        )
        .route("/events/:id/attendees", post(attach_attendees))
        .route("/event-series", get(list_series).post(create_series_handler))
        .route(
            "/event-series/:id",
            get(get_series)
                .put(rewrite_series_handler)
                .delete(delete_series),
        )
        .route("/event-series/:id/occurrences", get(series_occurrences))
        .route("/event-attendees/:id/state", put(set_attendee_state_handler))
        .with_state(state)
}

// =============================================================================
// Scope extraction + permission gate
// =============================================================================

/// Fail-closed gate: check the route's permission on the `AuthContext`, then
/// derive the engine `ScopeCtx` from the signed `CompanyContext`.
///
/// The acting user is the token's `sub` (the same signed source as the
/// company). A `sub` that is not a UUID cannot own or attend rows in this
/// model (`organizer_user_id` / `user_id` are UUIDs), so such a principal is
/// rejected outright rather than mapped to a zero-visibility reader.
fn gate(
    tenant: &CompanyContext,
    auth: &Option<axum::Extension<AuthContext>>,
    permission: &str,
) -> Result<ScopeCtx, axum::response::Response> {
    let Some(axum::Extension(auth)) = auth else {
        return Err((
            StatusCode::UNAUTHORIZED,
            Json(ErrorBody {
                error: "CALENDAR_EVENT_UNAUTHENTICATED".to_string(),
                message: "authentication context missing".to_string(),
            }),
        )
            .into_response());
    };
    if !auth.permissions.iter().any(|p| p == permission) {
        return Err((
            StatusCode::FORBIDDEN,
            Json(ErrorBody {
                error: "CALENDAR_EVENT_FORBIDDEN".to_string(),
                message: format!("permission '{permission}' required"),
            }),
        )
            .into_response());
    }
    let acting_user_id = Uuid::parse_str(&tenant.user_id).map_err(|_| {
        (
            StatusCode::UNAUTHORIZED,
            Json(ErrorBody {
                error: "CALENDAR_EVENT_PRINCIPAL_INVALID".to_string(),
                message: "the authenticated principal id is not a user uuid".to_string(),
            }),
        )
            .into_response()
    })?;
    Ok(ScopeCtx {
        company_id: tenant.company_id,
        acting_user_id,
    })
}

// =============================================================================
// Error mapping
// =============================================================================

#[derive(Debug, Serialize)]
struct ErrorBody {
    error: String,
    message: String,
}

/// Map an `EventFamilyError` to the declared HTTP contract. This is the LOUD
/// channel for the recurrence cap: 422 + `CALENDAR_EVENT_RECURRENCE_CAP_EXCEEDED`
/// carrying the projected and cap numbers — never a silent truncation.
fn event_family_error_response(e: EventFamilyError) -> axum::response::Response {
    let (status, code) = match &e {
        EventFamilyError::RecurrenceCap { .. } => (
            StatusCode::UNPROCESSABLE_ENTITY,
            "CALENDAR_EVENT_RECURRENCE_CAP_EXCEEDED",
        ),
        EventFamilyError::DuplicateAttendee { .. } => {
            (StatusCode::CONFLICT, "CALENDAR_ATTENDEE_DUPLICATE")
        }
        EventFamilyError::Validation(_) => (StatusCode::BAD_REQUEST, "CALENDAR_EVENT_VALIDATION_ERROR"),
        EventFamilyError::NotFound => (StatusCode::NOT_FOUND, "CALENDAR_EVENT_NOT_FOUND"),
        EventFamilyError::Db(_) => {
            (StatusCode::INTERNAL_SERVER_ERROR, "CALENDAR_EVENT_DATABASE_ERROR")
        }
    };
    (
        status,
        Json(ErrorBody {
            error: code.to_string(),
            message: e.to_string(),
        }),
    )
        .into_response()
}

fn validation_response(message: impl Into<String>) -> axum::response::Response {
    (
        StatusCode::BAD_REQUEST,
        Json(ErrorBody {
            error: "CALENDAR_EVENT_VALIDATION_ERROR".to_string(),
            message: message.into(),
        }),
    )
        .into_response()
}

fn not_found_response() -> axum::response::Response {
    (
        StatusCode::NOT_FOUND,
        Json(ErrorBody {
            error: "CALENDAR_EVENT_NOT_FOUND".to_string(),
            message: "not found".to_string(),
        }),
    )
        .into_response()
}

fn db_error_response(e: sqlx::Error) -> axum::response::Response {
    event_family_error_response(EventFamilyError::Db(e))
}

// =============================================================================
// RLS-scoped statement helpers
// =============================================================================

/// Pin the request GUCs on a transaction-local basis so the strict company
/// fence and the privacy read fence evaluate every statement of this caller.
/// `set_config(..., true)` is transaction-scoped: the values cannot leak onto
/// a pooled connection reused by the next request.
async fn pin_scope(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    scope: &ScopeCtx,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "SELECT set_config('app.company_id', $1, true), \
                set_config('app.user_id', $2, true)",
    )
    .bind(scope.company_id.to_string())
    .bind(scope.acting_user_id.to_string())
    .execute(&mut **tx)
    .await?;
    Ok(())
}

/// Row shape for scoped event reads. Enum columns are read as text and parsed
/// with the domain `FromStr` impls so decoding never depends on the
/// connection's `search_path` resolving the schema-qualified PG enum types.
#[derive(Debug, sqlx::FromRow)]
struct EventRow {
    id: Uuid,
    series_id: Option<Uuid>,
    title: String,
    description: Option<String>,
    start_at: DateTime<Utc>,
    stop_at: DateTime<Utc>,
    privacy: String,
    organizer_user_id: Uuid,
    location: Option<String>,
    #[sqlx(json)]
    metadata: AuditMetadata,
}

impl EventRow {
    fn privacy(&self) -> EventPrivacy {
        // A row can only carry one of the three enum literals; the fallback is
        // display-only (the fences already decided the row is visible).
        self.privacy.parse().unwrap_or(EventPrivacy::Public)
    }

    /// The response DTO carries the row's company. Rows are only visible to a
    /// caller whose own company matches (the RLS fence decided that), so the
    /// caller's fence value IS the row's value here.
    fn into_dto(self, company_id: Uuid) -> CalendarEventResponseDto {
        let privacy = self.privacy();
        CalendarEventResponseDto {
            id: self.id,
            company_id,
            series_id: self.series_id,
            title: self.title,
            description: self.description,
            start_at: self.start_at,
            stop_at: self.stop_at,
            privacy,
            organizer_user_id: self.organizer_user_id,
            location: self.location,
            metadata: self.metadata,
        }
    }
}

/// Row shape for scoped series reads (same text-cast policy as `EventRow`).
#[derive(Debug, sqlx::FromRow)]
struct SeriesRow {
    id: Uuid,
    name: Option<String>,
    freq: String,
    interval: i32,
    by_weekday: Option<String>,
    by_monthday: Option<String>,
    until: Option<chrono::NaiveDate>,
    count: Option<i32>,
    base_event_id: Uuid,
    #[sqlx(json)]
    metadata: AuditMetadata,
}

impl SeriesRow {
    fn into_dto(self, company_id: Uuid) -> CalendarEventSeriesResponseDto {
        CalendarEventSeriesResponseDto {
            id: self.id,
            company_id,
            name: self.name,
            freq: self.freq.parse().unwrap_or(EventRecurrenceFreq::Weekly),
            interval: self.interval,
            by_weekday: self.by_weekday,
            by_monthday: self.by_monthday,
            until: self.until,
            count: self.count,
            base_event_id: self.base_event_id,
            metadata: self.metadata,
        }
    }

    /// The recurrence rule as the engine's rewrite command needs it (used when
    /// an `edit_scope: all` PATCH rewrites a series from an occurrence).
    fn freq(&self) -> EventRecurrenceFreq {
        self.freq.parse().unwrap_or(EventRecurrenceFreq::Weekly)
    }
}

// =============================================================================
// Request / response bodies
// =============================================================================

fn default_privacy() -> EventPrivacy {
    EventPrivacy::Public
}

fn default_edit_scope() -> String {
    "this".to_string()
}

fn default_replace_scope() -> String {
    "all".to_string()
}

fn default_interval() -> i32 {
    1
}

/// `GET /events` range filters. `from`/`to` are RFC 3339 timestamps bounding
/// `start_at` inclusively; both are optional. `limit` bounds the response
/// (default 1000, max 5000) — a paging convenience, unrelated to the
/// recurrence cap, which the engine enforces loudly long before here.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ListEventsQuery {
    from: Option<DateTime<Utc>>,
    to: Option<DateTime<Utc>>,
    limit: Option<i64>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CreateStandaloneBody {
    title: String,
    #[serde(default)]
    description: Option<String>,
    start_at: DateTime<Utc>,
    stop_at: DateTime<Utc>,
    #[serde(default = "default_privacy")]
    privacy: EventPrivacy,
    #[serde(default)]
    location: Option<String>,
    #[serde(default)]
    attendee_user_ids: Vec<Uuid>,
}

/// Body for PUT/PATCH `/events/:id`. `edit_scope` discriminates:
/// `this` (split this occurrence out as an exception), `following` (detach the
/// tail and trim the series rule), `all` (rewrite the series materialization).
/// PATCH defaults to `this`; PUT defaults to `all` (replace semantics).
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct EventEditBody {
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    description: Option<String>,
    #[serde(default)]
    start_at: Option<DateTime<Utc>>,
    #[serde(default)]
    stop_at: Option<DateTime<Utc>>,
    #[serde(default)]
    privacy: Option<EventPrivacy>,
    #[serde(default)]
    location: Option<String>,
    #[serde(default)]
    edit_scope: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CreateSeriesBody {
    #[serde(default)]
    name: Option<String>,
    freq: EventRecurrenceFreq,
    #[serde(default = "default_interval")]
    interval: i32,
    #[serde(default)]
    by_weekday: Option<String>,
    #[serde(default)]
    by_monthday: Option<String>,
    #[serde(default)]
    until: Option<chrono::NaiveDate>,
    #[serde(default)]
    count: Option<i32>,
    title: String,
    #[serde(default)]
    description: Option<String>,
    first_start_at: DateTime<Utc>,
    first_stop_at: DateTime<Utc>,
    #[serde(default = "default_privacy")]
    privacy: EventPrivacy,
    #[serde(default)]
    location: Option<String>,
    #[serde(default)]
    attendee_user_ids: Vec<Uuid>,
}

/// Body for PUT `/event-series/:id` — the full new rule the materialization is
/// rewritten under (reconciled by (start, stop) identity; exception-claimed
/// slots are never re-materialized).
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RewriteSeriesBody {
    #[serde(default)]
    name: Option<String>,
    freq: EventRecurrenceFreq,
    #[serde(default = "default_interval")]
    interval: i32,
    #[serde(default)]
    by_weekday: Option<String>,
    #[serde(default)]
    by_monthday: Option<String>,
    #[serde(default)]
    until: Option<chrono::NaiveDate>,
    #[serde(default)]
    count: Option<i32>,
    title: String,
    #[serde(default)]
    description: Option<String>,
    first_start_at: DateTime<Utc>,
    first_stop_at: DateTime<Utc>,
    #[serde(default = "default_privacy")]
    privacy: EventPrivacy,
    #[serde(default)]
    location: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct AttachAttendeesBody {
    attendee_user_ids: Vec<Uuid>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct AttendeeStateBody {
    state: EventAttendeeState,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct IdResponse {
    id: Uuid,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct EventListResponse {
    items: Vec<CalendarEventResponseDto>,
    total: usize,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct SeriesListResponse {
    items: Vec<CalendarEventSeriesResponseDto>,
    total: usize,
}

// =============================================================================
// Shared validation
// =============================================================================

fn validate_title(title: &str) -> Result<(), axum::response::Response> {
    let trimmed = title.trim();
    if trimmed.is_empty() {
        return Err(validation_response("title must not be empty"));
    }
    if trimmed.len() > 200 {
        return Err(validation_response("title exceeds 200 characters"));
    }
    Ok(())
}

fn validate_window(start: DateTime<Utc>, stop: DateTime<Utc>) -> Result<(), axum::response::Response> {
    if stop <= start {
        return Err(validation_response("stop_at must be after start_at"));
    }
    Ok(())
}

fn validate_rule(
    interval: i32,
    by_weekday: &Option<String>,
    by_monthday: &Option<String>,
    count: Option<i32>,
) -> Result<(), axum::response::Response> {
    if interval < 1 {
        return Err(validation_response("interval must be >= 1"));
    }
    if let Some(list) = by_weekday {
        // Comma list of ISO weekday numbers, 1 = Monday .. 7 = Sunday.
        for part in list.split(',') {
            match part.trim().parse::<u8>() {
                Ok(d) if (1..=7).contains(&d) => {}
                _ => {
                    return Err(validation_response(
                        "by_weekday must be a comma list of ISO weekday numbers 1..=7",
                    ))
                }
            }
        }
    }
    if let Some(list) = by_monthday {
        for part in list.split(',') {
            match part.trim().parse::<u8>() {
                Ok(d) if (1..=31).contains(&d) => {}
                _ => {
                    return Err(validation_response(
                        "by_monthday must be a comma list of day numbers 1..=31",
                    ))
                }
            }
        }
    }
    if let Some(c) = count {
        if c < 1 {
            return Err(validation_response("count must be >= 1"));
        }
    }
    Ok(())
}

/// The HTTP-level edit-scope discriminator. `all` is a surface concern only:
/// it delegates to the series rewrite, not to the engine's per-occurrence
/// `EditScope` (which knows `this` / `following`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum EditScopeToken {
    This,
    Following,
    All,
}

fn edit_scope_token(raw: &str) -> Result<EditScopeToken, axum::response::Response> {
    match raw.trim() {
        "this" => Ok(EditScopeToken::This),
        "following" => Ok(EditScopeToken::Following),
        "all" => Ok(EditScopeToken::All),
        other => Err(validation_response(format!(
            "edit_scope must be one of 'this', 'following', 'all' (got '{other}')"
        ))),
    }
}

// =============================================================================
// Handlers — events
// =============================================================================

/// `GET /events` — list the caller's visible events. Visibility is decided by
/// the RLS fences (company + privacy); the SQL adds only range, liveness, and
/// ordering.
async fn list_events(
    State(st): State<EventFamilyState>,
    tenant: CompanyContext,
    auth: Option<axum::Extension<AuthContext>>,
    Query(q): Query<ListEventsQuery>,
) -> axum::response::Response {
    let scope = match gate(&tenant, &auth, event_permissions::READ) {
        Ok(s) => s,
        Err(resp) => return resp,
    };
    let limit = q.limit.unwrap_or(1000).clamp(1, 5000);

    let mut tx = match st.pool.begin().await {
        Ok(tx) => tx,
        Err(e) => return db_error_response(e),
    };
    if let Err(e) = pin_scope(&mut tx, &scope).await {
        return db_error_response(e);
    }
    let rows = match sqlx::query_as::<_, EventRow>(
        "SELECT id, series_id, title, description, start_at, stop_at, \
                privacy::text AS privacy, organizer_user_id, location, metadata \
         FROM calendar.events \
         WHERE (metadata->>'deleted_at') IS NULL \
           AND ($1::timestamptz IS NULL OR start_at >= $1::timestamptz) \
           AND ($2::timestamptz IS NULL OR start_at <= $2::timestamptz) \
         ORDER BY start_at, id \
         LIMIT $3",
    )
    .bind(q.from)
    .bind(q.to)
    .bind(limit)
    .fetch_all(&mut *tx)
    .await
    {
        Ok(rows) => rows,
        Err(e) => return db_error_response(e),
    };
    let total = rows.len();
    let items = rows
        .into_iter()
        .map(|row| row.into_dto(scope.company_id))
        .collect::<Vec<_>>();
    (StatusCode::OK, Json(EventListResponse { items, total })).into_response()
}

/// `POST /events` — create a standalone (non-series) event with optional
/// attendees. The acting user is the organizer and is auto-added as an
/// accepted attendee by the engine.
async fn create_standalone_event(
    State(st): State<EventFamilyState>,
    tenant: CompanyContext,
    auth: Option<axum::Extension<AuthContext>>,
    axum::Json(b): axum::Json<CreateStandaloneBody>,
) -> axum::response::Response {
    let scope = match gate(&tenant, &auth, event_permissions::CREATE) {
        Ok(s) => s,
        Err(resp) => return resp,
    };
    if let Err(resp) = validate_title(&b.title) {
        return resp;
    }
    if let Err(resp) = validate_window(b.start_at, b.stop_at) {
        return resp;
    }
    match st
        .engine
        .create_standalone(
            engine_surface::CreateStandaloneCmd {
                title: b.title,
                description: b.description,
                start_at: b.start_at,
                stop_at: b.stop_at,
                privacy: b.privacy,
                location: b.location,
                attendee_user_ids: b.attendee_user_ids,
            },
            scope,
        )
        .await
    {
        Ok(id) => (StatusCode::CREATED, Json(IdResponse { id })).into_response(),
        Err(e) => event_family_error_response(e),
    }
}

/// `GET /events/:id` — fetch one visible event. A fenced-out id is
/// indistinguishable from a missing one (404) by design.
async fn get_event(
    State(st): State<EventFamilyState>,
    tenant: CompanyContext,
    auth: Option<axum::Extension<AuthContext>>,
    Path(id): Path<Uuid>,
) -> axum::response::Response {
    let scope = match gate(&tenant, &auth, event_permissions::READ) {
        Ok(s) => s,
        Err(resp) => return resp,
    };
    let mut tx = match st.pool.begin().await {
        Ok(tx) => tx,
        Err(e) => return db_error_response(e),
    };
    if let Err(e) = pin_scope(&mut tx, &scope).await {
        return db_error_response(e);
    }
    match fetch_event(&mut tx, id).await {
        Ok(Some(row)) => (StatusCode::OK, Json(row.into_dto(scope.company_id))).into_response(),
        Ok(None) => not_found_response(),
        Err(e) => db_error_response(e),
    }
}

async fn fetch_event(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    id: Uuid,
) -> Result<Option<EventRow>, sqlx::Error> {
    sqlx::query_as::<_, EventRow>(
        "SELECT id, series_id, title, description, start_at, stop_at, \
                privacy::text AS privacy, organizer_user_id, location, metadata \
         FROM calendar.events \
         WHERE id = $1::uuid AND (metadata->>'deleted_at') IS NULL",
    )
    .bind(id)
    .fetch_optional(&mut **tx)
    .await
}

async fn fetch_series(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    id: Uuid,
) -> Result<Option<SeriesRow>, sqlx::Error> {
    sqlx::query_as::<_, SeriesRow>(
        "SELECT id, name, freq::text AS freq, interval, by_weekday, by_monthday, \
                until, count, base_event_id, metadata \
         FROM calendar.event_series \
         WHERE id = $1::uuid AND (metadata->>'deleted_at') IS NULL",
    )
    .bind(id)
    .fetch_optional(&mut **tx)
    .await
}

/// Shared body of PUT/PATCH `/events/:id`.
///
/// * scope `this`/`following` on a series member delegates to the engine's
///   `edit_occurrence` (split / tail-detach, exception ledger updated).
/// * scope `all` on a series member delegates to `rewrite_series`, rebuilding
///   the rewrite command from the stored rule with the patched fields applied
///   over the base event's slot.
/// * a standalone row is patched in place (it belongs to no series, so no
///   series invariant applies); `following` on a standalone row is a
///   validation error.
async fn apply_event_edit(
    st: &EventFamilyState,
    scope: &ScopeCtx,
    id: Uuid,
    b: EventEditBody,
    default_scope: fn() -> String,
) -> axum::response::Response {
    let raw_scope = b.edit_scope.clone().unwrap_or_else(default_scope);
    let token = match edit_scope_token(&raw_scope) {
        Ok(t) => t,
        Err(resp) => return resp,
    };
    if let Some(title) = &b.title {
        if let Err(resp) = validate_title(title) {
            return resp;
        }
    }
    if let (Some(start), Some(stop)) = (b.start_at, b.stop_at) {
        if let Err(resp) = validate_window(start, stop) {
            return resp;
        }
    }

    let mut tx = match st.pool.begin().await {
        Ok(tx) => tx,
        Err(e) => return db_error_response(e),
    };
    if let Err(e) = pin_scope(&mut tx, scope).await {
        return db_error_response(e);
    }
    let row = match fetch_event(&mut tx, id).await {
        Ok(Some(row)) => row,
        Ok(None) => return not_found_response(),
        Err(e) => return db_error_response(e),
    };

    match (row.series_id, token) {
        (Some(_series_id), EditScopeToken::This) => {
            // Read-only tx; the engine opens its own scoped transaction.
            drop(tx);
            let cmd = engine_surface::EditOccurrenceCmd {
                event_id: id,
                scope: engine_surface::EditScope::This,
                title: b.title,
                description: b.description,
                start_at: b.start_at,
                stop_at: b.stop_at,
                privacy: b.privacy,
                location: b.location,
            };
            match st.engine.edit_occurrence(cmd, *scope).await {
                Ok(()) => StatusCode::NO_CONTENT.into_response(),
                Err(e) => event_family_error_response(e),
            }
        }
        (Some(_series_id), EditScopeToken::Following) => {
            drop(tx);
            let cmd = engine_surface::EditOccurrenceCmd {
                event_id: id,
                scope: engine_surface::EditScope::Following,
                title: b.title,
                description: b.description,
                start_at: b.start_at,
                stop_at: b.stop_at,
                privacy: b.privacy,
                location: b.location,
            };
            match st.engine.edit_occurrence(cmd, *scope).await {
                Ok(()) => StatusCode::NO_CONTENT.into_response(),
                Err(e) => event_family_error_response(e),
            }
        }
        (Some(series_id), EditScopeToken::All) => {
            let series = match fetch_series(&mut tx, series_id).await {
                Ok(Some(s)) => s,
                Ok(None) => return not_found_response(),
                Err(e) => return db_error_response(e),
            };
            let base = match fetch_event(&mut tx, series.base_event_id).await {
                Ok(Some(base)) => base,
                Ok(None) => return not_found_response(),
                Err(e) => return db_error_response(e),
            };
            // Read-only tx; the rewrite runs in the engine's own transaction.
            drop(tx);
            let series_freq = series.freq();
            let base_privacy = base.privacy();
            let cmd = engine_surface::RewriteSeriesCmd {
                series_id,
                name: series.name,
                freq: series_freq,
                interval: series.interval,
                by_weekday: series.by_weekday,
                by_monthday: series.by_monthday,
                until: series.until,
                count: series.count,
                title: b.title.unwrap_or(base.title),
                description: b.description.or(base.description),
                first_start_at: b.start_at.unwrap_or(base.start_at),
                first_stop_at: b.stop_at.unwrap_or(base.stop_at),
                privacy: b.privacy.unwrap_or(base_privacy),
                location: b.location.or(base.location),
            };
            match st.engine.rewrite_series(cmd, *scope).await {
                Ok(()) => StatusCode::NO_CONTENT.into_response(),
                Err(e) => event_family_error_response(e),
            }
        }
        (None, EditScopeToken::This) | (None, EditScopeToken::All) => {
            // Standalone row: apply the patch in place under the same scoped
            // transaction. The stop-after-start CHECK constraint backstops the
            // window validation above at the database level.
            let result = sqlx::query(
                "UPDATE calendar.events SET \
                    title = COALESCE($2, title), \
                    description = COALESCE($3, description), \
                    start_at = COALESCE($4, start_at), \
                    stop_at  = COALESCE($5, stop_at), \
                    privacy  = COALESCE($6::event_privacy, privacy), \
                    location = COALESCE($7, location) \
                 WHERE id = $1::uuid AND (metadata->>'deleted_at') IS NULL",
            )
            .bind(id)
            .bind(b.title)
            .bind(b.description)
            .bind(b.start_at)
            .bind(b.stop_at)
            .bind(b.privacy.map(|p| p.to_string()))
            .bind(b.location)
            .execute(&mut *tx)
            .await;
            match result {
                Ok(res) if res.rows_affected() > 0 => {
                    if let Err(e) = tx.commit().await {
                        return db_error_response(e);
                    }
                    StatusCode::NO_CONTENT.into_response()
                }
                Ok(_) => not_found_response(),
                Err(e) => db_error_response(e),
            }
        }
        (None, EditScopeToken::Following) => validation_response(
            "edit_scope 'following' applies to a series occurrence; this event is standalone",
        ),
    }
}

/// `PATCH /events/:id` — partial edit, `edit_scope` defaults to `this`.
async fn edit_event(
    State(st): State<EventFamilyState>,
    tenant: CompanyContext,
    auth: Option<axum::Extension<AuthContext>>,
    Path(id): Path<Uuid>,
    axum::Json(b): axum::Json<EventEditBody>,
) -> axum::response::Response {
    let scope = match gate(&tenant, &auth, event_permissions::UPDATE) {
        Ok(s) => s,
        Err(resp) => return resp,
    };
    apply_event_edit(&st, &scope, id, b, default_edit_scope).await
}

/// `PUT /events/:id` — replace edit, `edit_scope` defaults to `all`.
async fn replace_event(
    State(st): State<EventFamilyState>,
    tenant: CompanyContext,
    auth: Option<axum::Extension<AuthContext>>,
    Path(id): Path<Uuid>,
    axum::Json(b): axum::Json<EventEditBody>,
) -> axum::response::Response {
    let scope = match gate(&tenant, &auth, event_permissions::UPDATE) {
        Ok(s) => s,
        Err(resp) => return resp,
    };
    apply_event_edit(&st, &scope, id, b, default_replace_scope).await
}

/// `DELETE /events/:id` — soft-delete. On a series member this runs through
/// the engine so the slot is claimed as `cancelled` in the exception ledger
/// (a later series rewrite must not resurrect it); a standalone row is
/// soft-deleted in place with the house metadata shape.
async fn delete_event(
    State(st): State<EventFamilyState>,
    tenant: CompanyContext,
    auth: Option<axum::Extension<AuthContext>>,
    Path(id): Path<Uuid>,
) -> axum::response::Response {
    let scope = match gate(&tenant, &auth, event_permissions::DELETE) {
        Ok(s) => s,
        Err(resp) => return resp,
    };
    let mut tx = match st.pool.begin().await {
        Ok(tx) => tx,
        Err(e) => return db_error_response(e),
    };
    if let Err(e) = pin_scope(&mut tx, &scope).await {
        return db_error_response(e);
    }
    let row = match fetch_event(&mut tx, id).await {
        Ok(Some(row)) => row,
        Ok(None) => return not_found_response(),
        Err(e) => return db_error_response(e),
    };
    if row.series_id.is_some() {
        // The engine claims the slot as `cancelled` in its own transaction.
        drop(tx);
        return match st.engine.delete_occurrence(id, scope).await {
            Ok(()) => StatusCode::NO_CONTENT.into_response(),
            Err(e) => event_family_error_response(e),
        };
    }
    let result = sqlx::query(
        "UPDATE calendar.events \
         SET metadata = jsonb_set(COALESCE(metadata, '{}'), '{deleted_at}', to_jsonb(NOW())) \
         WHERE id = $1::uuid AND (metadata->>'deleted_at') IS NULL",
    )
    .bind(id)
    .execute(&mut *tx)
    .await;
    match result {
        Ok(res) if res.rows_affected() > 0 => {
            if let Err(e) = tx.commit().await {
                return db_error_response(e);
            }
            StatusCode::NO_CONTENT.into_response()
        }
        Ok(_) => not_found_response(),
        Err(e) => db_error_response(e),
    }
}

/// `POST /events/:id/attendees` — attach attendees; the engine dedups
/// app-side (first-wins) and the partial unique index
/// `uq_calendar_event_attendees_event_user` backstops it (409 on conflict).
async fn attach_attendees(
    State(st): State<EventFamilyState>,
    tenant: CompanyContext,
    auth: Option<axum::Extension<AuthContext>>,
    Path(id): Path<Uuid>,
    axum::Json(b): axum::Json<AttachAttendeesBody>,
) -> axum::response::Response {
    let scope = match gate(&tenant, &auth, event_permissions::UPDATE) {
        Ok(s) => s,
        Err(resp) => return resp,
    };
    match st
        .engine
        .attach_attendees(
            engine_surface::AttachAttendeesCmd {
                event_id: id,
                attendee_user_ids: b.attendee_user_ids,
            },
            scope,
        )
        .await
    {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => event_family_error_response(e),
    }
}

// =============================================================================
// Handlers — series
// =============================================================================

/// `GET /event-series` — list the caller's visible series (company fence only;
/// the privacy fence is declared on `calendar.events`).
async fn list_series(
    State(st): State<EventFamilyState>,
    tenant: CompanyContext,
    auth: Option<axum::Extension<AuthContext>>,
) -> axum::response::Response {
    let scope = match gate(&tenant, &auth, event_permissions::READ) {
        Ok(s) => s,
        Err(resp) => return resp,
    };
    let mut tx = match st.pool.begin().await {
        Ok(tx) => tx,
        Err(e) => return db_error_response(e),
    };
    if let Err(e) = pin_scope(&mut tx, &scope).await {
        return db_error_response(e);
    }
    let rows = match sqlx::query_as::<_, SeriesRow>(
        "SELECT id, name, freq::text AS freq, interval, by_weekday, by_monthday, \
                until, count, base_event_id, metadata \
         FROM calendar.event_series \
         WHERE (metadata->>'deleted_at') IS NULL \
         ORDER BY id \
         LIMIT 1000",
    )
    .fetch_all(&mut *tx)
    .await
    {
        Ok(rows) => rows,
        Err(e) => return db_error_response(e),
    };
    let total = rows.len();
    let items = rows
        .into_iter()
        .map(|row| row.into_dto(scope.company_id))
        .collect::<Vec<_>>();
    (StatusCode::OK, Json(SeriesListResponse { items, total })).into_response()
}

/// `POST /event-series` — create a series: the engine eagerly materializes the
/// occurrences (real rows, base event = slot 0), enforces the 720-occurrence
/// cap LOUDLY (422, whole transaction rolled back, zero rows), and adds the
/// organizer as an accepted attendee.
///
/// A rule with neither `until` nor `count` is NOT rejected here: the declared
/// posture is that the engine bounds it to the 15-year horizon from the first
/// slot's start and then applies the SAME loud cap — a daily-forever rule hits
/// 422 `CALENDAR_EVENT_RECURRENCE_CAP_EXCEEDED` with zero rows written, while a
/// horizon-bounded rule that fits under the cap (e.g. yearly) is honored. The
/// cap channel is the single loud bound for every unbounded rule; this surface
/// adds no second, quieter one.
async fn create_series_handler(
    State(st): State<EventFamilyState>,
    tenant: CompanyContext,
    auth: Option<axum::Extension<AuthContext>>,
    axum::Json(b): axum::Json<CreateSeriesBody>,
) -> axum::response::Response {
    let scope = match gate(&tenant, &auth, event_permissions::CREATE) {
        Ok(s) => s,
        Err(resp) => return resp,
    };
    if let Err(resp) = validate_title(&b.title) {
        return resp;
    }
    if let Err(resp) = validate_window(b.first_start_at, b.first_stop_at) {
        return resp;
    }
    if let Err(resp) = validate_rule(b.interval, &b.by_weekday, &b.by_monthday, b.count) {
        return resp;
    }
    match st
        .engine
        .create_series(
            engine_surface::CreateSeriesCmd {
                name: b.name,
                freq: b.freq,
                interval: b.interval,
                by_weekday: b.by_weekday,
                by_monthday: b.by_monthday,
                until: b.until,
                count: b.count,
                title: b.title,
                description: b.description,
                first_start_at: b.first_start_at,
                first_stop_at: b.first_stop_at,
                privacy: b.privacy,
                location: b.location,
                attendee_user_ids: b.attendee_user_ids,
            },
            scope,
        )
        .await
    {
        Ok(id) => (StatusCode::CREATED, Json(IdResponse { id })).into_response(),
        Err(e) => event_family_error_response(e),
    }
}

/// `GET /event-series/:id`.
async fn get_series(
    State(st): State<EventFamilyState>,
    tenant: CompanyContext,
    auth: Option<axum::Extension<AuthContext>>,
    Path(id): Path<Uuid>,
) -> axum::response::Response {
    let scope = match gate(&tenant, &auth, event_permissions::READ) {
        Ok(s) => s,
        Err(resp) => return resp,
    };
    let mut tx = match st.pool.begin().await {
        Ok(tx) => tx,
        Err(e) => return db_error_response(e),
    };
    if let Err(e) = pin_scope(&mut tx, &scope).await {
        return db_error_response(e);
    }
    match fetch_series(&mut tx, id).await {
        Ok(Some(row)) => (StatusCode::OK, Json(row.into_dto(scope.company_id))).into_response(),
        Ok(None) => not_found_response(),
        Err(e) => db_error_response(e),
    }
}

/// `PUT /event-series/:id` — rewrite the series under a new rule. The engine
/// re-expands (cap re-checked, loudly), reconciles by (start, stop) identity
/// (aligned rows keep their ids), and never re-materializes a slot claimed in
/// the exception ledger — which is what makes single edits and deletes stick.
async fn rewrite_series_handler(
    State(st): State<EventFamilyState>,
    tenant: CompanyContext,
    auth: Option<axum::Extension<AuthContext>>,
    Path(id): Path<Uuid>,
    axum::Json(b): axum::Json<RewriteSeriesBody>,
) -> axum::response::Response {
    let scope = match gate(&tenant, &auth, event_permissions::UPDATE) {
        Ok(s) => s,
        Err(resp) => return resp,
    };
    if let Err(resp) = validate_title(&b.title) {
        return resp;
    }
    if let Err(resp) = validate_window(b.first_start_at, b.first_stop_at) {
        return resp;
    }
    if let Err(resp) = validate_rule(b.interval, &b.by_weekday, &b.by_monthday, b.count) {
        return resp;
    }
    match st
        .engine
        .rewrite_series(
            engine_surface::RewriteSeriesCmd {
                series_id: id,
                name: b.name,
                freq: b.freq,
                interval: b.interval,
                by_weekday: b.by_weekday,
                by_monthday: b.by_monthday,
                until: b.until,
                count: b.count,
                title: b.title,
                description: b.description,
                first_start_at: b.first_start_at,
                first_stop_at: b.first_stop_at,
                privacy: b.privacy,
                location: b.location,
            },
            scope,
        )
        .await
    {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => event_family_error_response(e),
    }
}

/// `DELETE /event-series/:id` — withdraw the series: soft-delete every live
/// member occurrence, then the series row. Attendee rows ride their (now
/// invisible) events. The engine has no series-delete verb (its ledger only
/// records per-occurrence decisions), so this composition-level withdrawal
/// runs as one scoped transaction here.
async fn delete_series(
    State(st): State<EventFamilyState>,
    tenant: CompanyContext,
    auth: Option<axum::Extension<AuthContext>>,
    Path(id): Path<Uuid>,
) -> axum::response::Response {
    let scope = match gate(&tenant, &auth, event_permissions::DELETE) {
        Ok(s) => s,
        Err(resp) => return resp,
    };
    let mut tx = match st.pool.begin().await {
        Ok(tx) => tx,
        Err(e) => return db_error_response(e),
    };
    if let Err(e) = pin_scope(&mut tx, &scope).await {
        return db_error_response(e);
    };
    let exists = match sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*) FROM calendar.event_series \
         WHERE id = $1::uuid AND (metadata->>'deleted_at') IS NULL",
    )
    .bind(id)
    .fetch_one(&mut *tx)
    .await
    {
        Ok(n) => n > 0,
        Err(e) => return db_error_response(e),
    };
    if !exists {
        return not_found_response();
    }
    let members = sqlx::query(
        "UPDATE calendar.events \
         SET metadata = jsonb_set(COALESCE(metadata, '{}'), '{deleted_at}', to_jsonb(NOW())) \
         WHERE series_id = $1::uuid AND (metadata->>'deleted_at') IS NULL",
    )
    .bind(id)
    .execute(&mut *tx)
    .await;
    if let Err(e) = members {
        return db_error_response(e);
    }
    let series = sqlx::query(
        "UPDATE calendar.event_series \
         SET metadata = jsonb_set(COALESCE(metadata, '{}'), '{deleted_at}', to_jsonb(NOW())) \
         WHERE id = $1::uuid AND (metadata->>'deleted_at') IS NULL",
    )
    .bind(id)
    .execute(&mut *tx)
    .await;
    match series {
        Ok(_) => {
            if let Err(e) = tx.commit().await {
                return db_error_response(e);
            }
            StatusCode::NO_CONTENT.into_response()
        }
        Err(e) => db_error_response(e),
    }
}

/// `GET /event-series/:id/occurrences` — the live member rows of the series
/// (the eagerly materialized occurrences), ordered by start. Detached
/// (edited) occurrences no longer appear here: they became standalone events.
async fn series_occurrences(
    State(st): State<EventFamilyState>,
    tenant: CompanyContext,
    auth: Option<axum::Extension<AuthContext>>,
    Path(id): Path<Uuid>,
) -> axum::response::Response {
    let scope = match gate(&tenant, &auth, event_permissions::READ) {
        Ok(s) => s,
        Err(resp) => return resp,
    };
    let mut tx = match st.pool.begin().await {
        Ok(tx) => tx,
        Err(e) => return db_error_response(e),
    };
    if let Err(e) = pin_scope(&mut tx, &scope).await {
        return db_error_response(e);
    }
    let series_exists = match sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*) FROM calendar.event_series \
         WHERE id = $1::uuid AND (metadata->>'deleted_at') IS NULL",
    )
    .bind(id)
    .fetch_one(&mut *tx)
    .await
    {
        Ok(n) => n > 0,
        Err(e) => return db_error_response(e),
    };
    if !series_exists {
        return not_found_response();
    }
    let rows = match sqlx::query_as::<_, EventRow>(
        "SELECT id, series_id, title, description, start_at, stop_at, \
                privacy::text AS privacy, organizer_user_id, location, metadata \
         FROM calendar.events \
         WHERE series_id = $1::uuid AND (metadata->>'deleted_at') IS NULL \
         ORDER BY start_at, id",
    )
    .bind(id)
    .fetch_all(&mut *tx)
    .await
    {
        Ok(rows) => rows,
        Err(e) => return db_error_response(e),
    };
    let total = rows.len();
    let items = rows
        .into_iter()
        .map(|row| row.into_dto(scope.company_id))
        .collect::<Vec<_>>();
    (StatusCode::OK, Json(EventListResponse { items, total })).into_response()
}

// =============================================================================
// Handlers — attendees
// =============================================================================

/// `PUT /event-attendees/:id/state` — hand-set an attendee response state
/// (needs_action/accepted/declined/tentative). No transition gate beyond the
/// enum, faithful to the ported state machine.
async fn set_attendee_state_handler(
    State(st): State<EventFamilyState>,
    tenant: CompanyContext,
    auth: Option<axum::Extension<AuthContext>>,
    Path(id): Path<Uuid>,
    axum::Json(b): axum::Json<AttendeeStateBody>,
) -> axum::response::Response {
    let scope = match gate(&tenant, &auth, event_permissions::UPDATE) {
        Ok(s) => s,
        Err(resp) => return resp,
    };
    match st
        .engine
        .set_attendee_state(
            engine_surface::SetAttendeeStateCmd {
                attendee_id: id,
                state: b.state,
            },
            scope,
        )
        .await
    {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => event_family_error_response(e),
    }
}

// =============================================================================
// Probes
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    use axum::body::Body;
    use axum::http::{Method, Request, StatusCode};
    use axum::middleware::{self, Next};
    use backbone_auth::company::{company_auth, CompanyVerifier};
    use sqlx::postgres::PgPoolOptions;
    use tower::ServiceExt;

    // ── harness ─────────────────────────────────────────────────────────────

    /// Build the state against a lazy pool: routing and guards are decided
    /// before any handler touches the database, so router-shape probes need no
    /// live connection.
    fn lazy_state() -> EventFamilyState {
        let pool = PgPoolOptions::new()
            .connect_lazy("postgres://nobody:nobody@127.0.0.1:5432/_")
            .expect("lazy pool options parse");
        let engine = Arc::new(CalendarEventSeriesEngine::new(
            pool.clone(),
            Arc::new(crate::infrastructure::persistence::CalendarEventRepository::new(
                pool.clone(),
            )),
            Arc::new(crate::infrastructure::persistence::CalendarEventSeriesRepository::new(
                pool.clone(),
            )),
            Arc::new(crate::infrastructure::persistence::CalendarEventExceptionRepository::new(
                pool.clone(),
            )),
            Arc::new(crate::infrastructure::persistence::CalendarEventAttendeeRepository::new(
                pool.clone(),
            )),
        ));
        let events_svc = Arc::new(
            CalendarEventService::with_repository(Arc::new(
                crate::infrastructure::persistence::CalendarEventRepository::new(pool.clone()),
            )),
        );
        EventFamilyState { engine, events_svc, pool }
    }

    fn lazy_router() -> Router {
        let st = lazy_state();
        create_calendar_event_guarded_routes(st.engine.clone(), st.events_svc.clone(), st.pool)
    }

    fn tenant_of(company_id: Uuid, user_id: Uuid) -> CompanyContext {
        CompanyContext {
            company_id,
            branch_id: None,
            user_id: user_id.to_string(),
        }
    }

    fn auth_with(permissions: &[&str]) -> AuthContext {
        AuthContext {
            user_id: "permissions-only-principal".to_string(),
            roles: vec![],
            permissions: permissions.iter().map(|p| p.to_string()).collect(),
        }
    }

    /// Wrap the router with the two extensions the host auth stack provides in
    /// production: the signed `CompanyContext` and the RBAC `AuthContext`.
    fn as_caller(router: Router, tenant: &CompanyContext, auth: Option<AuthContext>) -> Router {
        let tenant = tenant.clone();
        router.layer(middleware::from_fn(
            move |mut req: axum::extract::Request, next: Next| {
                let tenant = tenant.clone();
                let auth = auth.clone();
                async move {
                    req.extensions_mut().insert(tenant);
                    if let Some(auth) = auth {
                        req.extensions_mut().insert(auth);
                    }
                    next.run(req).await
                }
            },
        ))
    }

    async fn body_json(resp: axum::response::Response) -> serde_json::Value {
        let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .expect("body readable");
        serde_json::from_slice(&bytes).expect("body is JSON")
    }

    const COMPANY: Uuid = Uuid::nil();
    const USER_A: Uuid = Uuid::nil();

    // ── router shape: fail-closed surface ───────────────────────────────────

    /// The exception ledger deliberately has NO route: it is internal state.
    #[tokio::test]
    async fn exception_ledger_is_not_mounted() {
        let app = lazy_router();
        for (method, uri) in [
            (Method::GET, "/event-exceptions"),
            (Method::POST, "/event-exceptions"),
            (Method::GET, "/event-exceptions/00000000-0000-0000-0000-000000000000"),
        ] {
            let resp = app
                .clone()
                .oneshot(
                    Request::builder()
                        .method(method.clone())
                        .uri(uri)
                        .body(Body::empty())
                        .unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(
                resp.status(),
                StatusCode::NOT_FOUND,
                "{method} {uri} must not be mounted"
            );
        }
    }

    /// No company context → 401 before any handler logic runs.
    #[tokio::test]
    async fn request_without_company_context_is_unauthenticated() {
        let app = lazy_router();
        let resp = app
            .oneshot(
                Request::builder()
                    .method(Method::GET)
                    .uri("/events")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    }

    /// Company context but no RBAC context → 401 (fail-closed: the permission
    /// check cannot even be attempted).
    #[tokio::test]
    async fn request_without_auth_context_is_unauthenticated() {
        let app = as_caller(lazy_router(), &tenant_of(COMPANY, USER_A), None);
        let resp = app
            .oneshot(
                Request::builder()
                    .method(Method::GET)
                    .uri("/events")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
        let body = body_json(resp).await;
        assert_eq!(body["error"], "CALENDAR_EVENT_UNAUTHENTICATED");
    }

    /// Authenticated caller with none of the route's permissions → 403, and a
    /// read-only caller cannot reach the write verbs.
    #[tokio::test]
    async fn permissionless_caller_is_forbidden() {
        let app = as_caller(lazy_router(), &tenant_of(COMPANY, USER_A), Some(auth_with(&[])));
        let resp = app
            .oneshot(
                Request::builder()
                    .method(Method::GET)
                    .uri("/events")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::FORBIDDEN);
        let body = body_json(resp).await;
        assert_eq!(body["error"], "CALENDAR_EVENT_FORBIDDEN");

        let app = as_caller(
            lazy_router(),
            &tenant_of(COMPANY, USER_A),
            Some(auth_with(&[event_permissions::READ])),
        );
        // A well-formed body matters here: axum runs the Json extractor before
        // the handler, and an invalid body would answer 422 before the
        // permission gate could speak. The gate must be what refuses.
        let resp = app
            .oneshot(
                Request::builder()
                    .method(Method::POST)
                    .uri("/events")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        serde_json::json!({
                            "title": "read-only caller write attempt",
                            "startAt": "2027-01-04T09:00:00Z",
                            "stopAt": "2027-01-04T10:00:00Z"
                        })
                        .to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::FORBIDDEN);
    }

    /// A caller whose principal is not a user uuid cannot own or attend rows
    /// → rejected 401 (fail-closed), not mapped to a zero-visibility reader.
    #[tokio::test]
    async fn non_uuid_principal_is_rejected() {
        let tenant = CompanyContext {
            company_id: COMPANY,
            branch_id: None,
            user_id: "not-a-uuid".to_string(),
        };
        let app = as_caller(
            lazy_router(),
            &tenant,
            Some(auth_with(&[event_permissions::READ])),
        );
        let resp = app
            .oneshot(
                Request::builder()
                    .method(Method::GET)
                    .uri("/events")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
        let body = body_json(resp).await;
        assert_eq!(body["error"], "CALENDAR_EVENT_PRINCIPAL_INVALID");
    }

    /// A permitted caller reaches the database (the lazy pool cannot connect,
    /// so the response is the mapped 500 database error — proof the guard
    /// passed and the scoped read was attempted).
    #[tokio::test]
    async fn permitted_caller_reaches_the_scoped_read() {
        let app = as_caller(
            lazy_router(),
            &tenant_of(COMPANY, USER_A),
            Some(auth_with(&[event_permissions::READ])),
        );
        let resp = app
            .oneshot(
                Request::builder()
                    .method(Method::GET)
                    .uri("/events")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::INTERNAL_SERVER_ERROR);
        let body = body_json(resp).await;
        assert_eq!(body["error"], "CALENDAR_EVENT_DATABASE_ERROR");
    }

    /// An unknown edit_scope is a 400 validation error, not a silent default.
    #[tokio::test]
    async fn unknown_edit_scope_is_a_validation_error() {
        let app = as_caller(
            lazy_router(),
            &tenant_of(COMPANY, USER_A),
            Some(auth_with(&[event_permissions::UPDATE])),
        );
        let resp = app
            .oneshot(
                Request::builder()
                    .method(Method::PATCH)
                    .uri("/events/00000000-0000-0000-0000-000000000000")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        serde_json::json!({"editScope": "sometimes"}).to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
        let body = body_json(resp).await;
        assert_eq!(body["error"], "CALENDAR_EVENT_VALIDATION_ERROR");
    }

    /// Stop-before-start windows are rejected 400 at the surface (the DB CHECK
    /// constraint backstops any path that skips this).
    #[tokio::test]
    async fn inverted_window_is_rejected() {
        let app = as_caller(
            lazy_router(),
            &tenant_of(COMPANY, USER_A),
            Some(auth_with(&[event_permissions::CREATE])),
        );
        let resp = app
            .oneshot(
                Request::builder()
                    .method(Method::POST)
                    .uri("/events")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        serde_json::json!({
                            "title": "inverted",
                            "startAt": "2027-01-04T10:00:00Z",
                            "stopAt": "2027-01-04T09:00:00Z"
                        })
                        .to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
        let body = body_json(resp).await;
        assert_eq!(body["error"], "CALENDAR_EVENT_VALIDATION_ERROR");
    }

    /// An unbounded series rule (neither `until` nor `count`) passes the
    /// surface and reaches the engine, which bounds it to the 15-year horizon
    /// and applies the SAME loud cap. Expansion is pure (before any database
    /// touch), so even on the lazy pool a daily-forever rule answers with the
    /// declared 422 cap — the surface must never shadow it with a quieter 400.
    #[tokio::test]
    async fn unbounded_series_hits_the_loud_cap_not_a_surface_400() {
        let app = as_caller(
            lazy_router(),
            &tenant_of(COMPANY, USER_A),
            Some(auth_with(&[event_permissions::CREATE])),
        );
        let resp = app
            .oneshot(
                Request::builder()
                    .method(Method::POST)
                    .uri("/event-series")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        serde_json::json!({
                            "freq": "daily",
                            "title": "unbounded",
                            "firstStartAt": "2027-01-04T09:00:00Z",
                            "firstStopAt": "2027-01-04T10:00:00Z"
                        })
                        .to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(
            resp.status(),
            StatusCode::UNPROCESSABLE_ENTITY,
            "an unbounded daily rule must hit the engine's horizon+cap (422), not a surface 400"
        );
        let body = body_json(resp).await;
        assert_eq!(body["error"], "CALENDAR_EVENT_RECURRENCE_CAP_EXCEEDED");
    }

    // ── error map: the loud channels ────────────────────────────────────────

    /// The 720-occurrence cap must fire LOUD: 422 with the declared error code
    /// and both numbers in the message — never a truncation.
    #[test]
    fn recurrence_cap_maps_to_422_with_code() {
        assert_eq!(engine_surface::MAX_OCCURRENCES, 720);
        let resp = event_family_error_response(EventFamilyError::RecurrenceCap {
            projected: 721,
            cap: engine_surface::MAX_OCCURRENCES,
        });
        assert_eq!(resp.status(), StatusCode::UNPROCESSABLE_ENTITY);
        let body = tokio_test::block_on(body_json(resp));
        assert_eq!(body["error"], "CALENDAR_EVENT_RECURRENCE_CAP_EXCEEDED");
        let message = body["message"].as_str().unwrap();
        assert!(
            message.contains("721"),
            "message carries the projection: {message}"
        );
        assert!(message.contains("720"), "message carries the cap: {message}");
    }

    /// Duplicate attendee → 409 `CALENDAR_ATTENDEE_DUPLICATE`.
    #[test]
    fn duplicate_attendee_maps_to_409_with_code() {
        let resp = event_family_error_response(EventFamilyError::DuplicateAttendee {
            user_id: Uuid::new_v4(),
        });
        assert_eq!(resp.status(), StatusCode::CONFLICT);
        let body = tokio_test::block_on(body_json(resp));
        assert_eq!(body["error"], "CALENDAR_ATTENDEE_DUPLICATE");
    }

    /// Validation → 400, not-found → 404, database → 500.
    #[test]
    fn validation_not_found_and_db_map_correctly() {
        let resp = event_family_error_response(EventFamilyError::Validation("bad".into()));
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
        let resp = event_family_error_response(EventFamilyError::NotFound);
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
        let resp = event_family_error_response(EventFamilyError::Db(sqlx::Error::RowNotFound));
        assert_eq!(resp.status(), StatusCode::INTERNAL_SERVER_ERROR);
    }

    // ── DB-backed fence probes ──────────────────────────────────────────────
    //
    // Skipped LOUD when the probe database is not configured. Setup (outside
    // the test binary):
    //
    //   * a scratch database with the module's migrations applied in filename
    //     order;
    //   * a NOSUPERUSER LOGIN role that owns nothing (so the RLS fences apply
    //     to it) with USAGE on schema `calendar` and SELECT / INSERT / UPDATE
    //     / DELETE on the four family tables.
    //
    // Point `CALENDAR_PROBE_DATABASE_URL` at that role's connection.

    async fn probe_pool() -> Option<PgPool> {
        let url = match std::env::var("CALENDAR_PROBE_DATABASE_URL") {
            Ok(url) => url,
            Err(_) => {
                eprintln!(
                    "SKIP (loud): CALENDAR_PROBE_DATABASE_URL is not set — \
                     the DB-backed fence probes did not run"
                );
                return None;
            }
        };
        match PgPoolOptions::new().max_connections(4).connect(&url).await {
            Ok(pool) => Some(pool),
            Err(e) => {
                eprintln!(
                    "SKIP (loud): could not connect the probe pool ({url}): {e} — \
                     the DB-backed fence probes did not run"
                );
                None
            }
        }
    }

    fn probe_router(pool: &PgPool) -> Router {
        let engine = Arc::new(CalendarEventSeriesEngine::new(
            pool.clone(),
            Arc::new(crate::infrastructure::persistence::CalendarEventRepository::new(
                pool.clone(),
            )),
            Arc::new(crate::infrastructure::persistence::CalendarEventSeriesRepository::new(
                pool.clone(),
            )),
            Arc::new(crate::infrastructure::persistence::CalendarEventExceptionRepository::new(
                pool.clone(),
            )),
            Arc::new(crate::infrastructure::persistence::CalendarEventAttendeeRepository::new(
                pool.clone(),
            )),
        ));
        let events_svc = Arc::new(CalendarEventService::with_repository(Arc::new(
            crate::infrastructure::persistence::CalendarEventRepository::new(pool.clone()),
        )));
        create_calendar_event_guarded_routes(engine, events_svc, pool.clone())
    }

    /// Seed one event row as the organizer under the probe role, with the
    /// request GUCs pinned (the RLS WITH CHECK must pass).
    async fn seed_event(
        pool: &PgPool,
        scope: &ScopeCtx,
        title: &str,
        privacy: EventPrivacy,
    ) -> Uuid {
        let id = Uuid::new_v4();
        let mut tx = pool.begin().await.expect("seed tx");
        pin_scope(&mut tx, scope).await.expect("seed scope");
        let start = chrono::Utc::now() + chrono::TimeDelta::hours(1);
        let stop = start + chrono::TimeDelta::hours(1);
        sqlx::query(
            "INSERT INTO calendar.events \
             (id, company_id, series_id, title, start_at, stop_at, privacy, organizer_user_id) \
             VALUES ($1, $2, NULL, $3, $4, $5, $6::event_privacy, $7)",
        )
        .bind(id)
        .bind(scope.company_id)
        .bind(title)
        .bind(start)
        .bind(stop)
        .bind(privacy.to_string())
        .bind(scope.acting_user_id)
        .execute(&mut *tx)
        .await
        .expect("seed event");
        tx.commit().await.expect("seed commit");
        id
    }

    async fn seed_attendee(pool: &PgPool, scope: &ScopeCtx, event_id: Uuid, user: Uuid) {
        let mut tx = pool.begin().await.expect("seed tx");
        pin_scope(&mut tx, scope).await.expect("seed scope");
        sqlx::query(
            "INSERT INTO calendar.event_attendees (id, company_id, event_id, user_id, state) \
             VALUES ($1, $2, $3, $4, 'needs_action'::event_attendee_state)",
        )
        .bind(Uuid::new_v4())
        .bind(scope.company_id)
        .bind(event_id)
        .bind(user)
        .execute(&mut *tx)
        .await
        .expect("seed attendee");
        tx.commit().await.expect("seed commit");
    }

    async fn list_titles_as(
        pool: &PgPool,
        company: Uuid,
        user: Uuid,
        permissions: &[&str],
    ) -> Vec<String> {
        let app = as_caller(
            probe_router(pool),
            &tenant_of(company, user),
            Some(auth_with(permissions)),
        );
        let resp = app
            .oneshot(
                Request::builder()
                    .method(Method::GET)
                    .uri("/events")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(
            resp.status(),
            StatusCode::OK,
            "list must succeed for a permitted caller"
        );
        let body = body_json(resp).await;
        body["items"]
            .as_array()
            .expect("items array")
            .iter()
            .map(|item| item["title"].as_str().unwrap().to_string())
            .collect()
    }

    /// Cross-fence invisibility (the wave's DoD leg), through the HTTP
    /// surface: user B in the SAME company sees user A's public event but
    /// neither the private nor the confidential one; the organizer sees all
    /// three; an attendee sees the private event they attend. Visibility is
    /// decided by the RLS policy — this file's SQL carries no privacy
    /// predicate.
    #[tokio::test]
    async fn cross_fence_invisibility_through_http() {
        let Some(pool) = probe_pool().await else { return };
        let company = Uuid::new_v4();
        let user_a = Uuid::new_v4();
        let user_b = Uuid::new_v4();
        let user_c = Uuid::new_v4();
        let scope_a = ScopeCtx { company_id: company, acting_user_id: user_a };

        seed_event(&pool, &scope_a, "probe-public", EventPrivacy::Public).await;
        let private_id = seed_event(&pool, &scope_a, "probe-private", EventPrivacy::Private).await;
        seed_event(&pool, &scope_a, "probe-confidential", EventPrivacy::Confidential).await;
        seed_attendee(&pool, &scope_a, private_id, user_c).await;

        // Fenced-out role in the same company: public only.
        let seen_by_b = list_titles_as(&pool, company, user_b, &[event_permissions::READ]).await;
        assert_eq!(
            seen_by_b,
            vec!["probe-public".to_string()],
            "user B must see ONLY the public event (privacy fence), got {seen_by_b:?}"
        );

        // Organizer sees everything they organized.
        let seen_by_a = list_titles_as(&pool, company, user_a, &[event_permissions::READ]).await;
        assert_eq!(seen_by_a.len(), 3, "organizer sees all three: {seen_by_a:?}");

        // Attendee sees public + the private event they attend.
        let seen_by_c = list_titles_as(&pool, company, user_c, &[event_permissions::READ]).await;
        assert_eq!(
            seen_by_c.len(),
            2,
            "attendee sees public + attended private: {seen_by_c:?}"
        );
        assert!(seen_by_c.contains(&"probe-private".to_string()));
        assert!(!seen_by_c.contains(&"probe-confidential".to_string()));
    }

    /// Company-fence regression: another company's caller sees zero rows.
    #[tokio::test]
    async fn company_fence_hides_other_companies() {
        let Some(pool) = probe_pool().await else { return };
        let company = Uuid::new_v4();
        let scope_a = ScopeCtx { company_id: company, acting_user_id: Uuid::new_v4() };
        seed_event(&pool, &scope_a, "other-company-event", EventPrivacy::Public).await;

        let other_company = Uuid::new_v4();
        let seen =
            list_titles_as(&pool, other_company, Uuid::new_v4(), &[event_permissions::READ]).await;
        assert!(
            seen.is_empty(),
            "a different app.company_id must see zero event-family rows, got {seen:?}"
        );
    }

    /// The same privacy fence at SQL level, on separate connections with
    /// different pinned users — and the fail-closed case: `app.user_id` unset
    /// ⇒ only public rows are readable, even inside the right company.
    #[tokio::test]
    async fn privacy_fence_at_sql_level() {
        let Some(pool) = probe_pool().await else { return };
        let company = Uuid::new_v4();
        let user_a = Uuid::new_v4();
        let scope_a = ScopeCtx { company_id: company, acting_user_id: user_a };
        seed_event(&pool, &scope_a, "sql-public", EventPrivacy::Public).await;
        seed_event(&pool, &scope_a, "sql-private", EventPrivacy::Private).await;

        let count_visible = |pool: &PgPool, company: Uuid, user: Option<Uuid>| {
            let pool = pool.clone();
            async move {
                let mut tx = pool.begin().await.expect("tx");
                sqlx::query("SELECT set_config('app.company_id', $1, true)")
                    .bind(company.to_string())
                    .execute(&mut *tx)
                    .await
                    .expect("pin company");
                if let Some(user) = user {
                    sqlx::query("SELECT set_config('app.user_id', $1, true)")
                        .bind(user.to_string())
                        .execute(&mut *tx)
                        .await
                        .expect("pin user");
                }
                let n: i64 = sqlx::query_scalar(
                    "SELECT COUNT(*) FROM calendar.events \
                     WHERE (metadata->>'deleted_at') IS NULL",
                )
                .fetch_one(&mut *tx)
                .await
                .expect("count");
                tx.rollback().await.expect("rollback resets the LOCAL settings");
                n
            }
        };

        let as_organizer = count_visible(&pool, company, Some(user_a)).await;
        assert_eq!(as_organizer, 2, "organizer sees both rows");
        let as_stranger = count_visible(&pool, company, Some(Uuid::new_v4())).await;
        assert_eq!(as_stranger, 1, "non-participant sees only the public row");
        let unscoped = count_visible(&pool, company, None).await;
        assert_eq!(
            unscoped, 1,
            "unset app.user_id fails closed: only public rows are readable"
        );
    }

    /// The full `company_auth` token path: a request with a valid signed token
    /// carrying the company claim reaches the handler; a token without the
    /// claim is rejected 401 before any event-family logic.
    #[tokio::test]
    async fn signed_company_token_path() {
        let verifier = CompanyVerifier::hs256(b"probe-secret");
        let company = Uuid::new_v4();
        let user = Uuid::new_v4();

        let claims = serde_json::json!({
            "sub": user.to_string(),
            "exp": 4102444800i64,
            "company_id": company.to_string(),
        });
        let token = jsonwebtoken::encode(
            &jsonwebtoken::Header::default(),
            &claims,
            &jsonwebtoken::EncodingKey::from_secret(b"probe-secret"),
        )
        .expect("mint token");

        // Valid token + host-inserted AuthContext → guard passes, scoped read
        // runs (lazy pool ⇒ mapped 500, proving the route was reached).
        let app = lazy_router()
            .route_layer(middleware::from_fn_with_state(
                verifier.clone(),
                company_auth,
            ))
            .layer(middleware::from_fn(
                move |mut req: axum::extract::Request, next: Next| {
                    let auth = AuthContext {
                        user_id: user.to_string(),
                        roles: vec![],
                        permissions: vec![event_permissions::READ.to_string()],
                    };
                    async move {
                        req.extensions_mut().insert(auth);
                        next.run(req).await
                    }
                },
            ));
        let resp = app
            .oneshot(
                Request::builder()
                    .method(Method::GET)
                    .uri("/events")
                    .header("authorization", format!("Bearer {token}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(
            resp.status(),
            StatusCode::INTERNAL_SERVER_ERROR,
            "valid token reaches the scoped read (lazy pool maps to 500)"
        );

        // A token WITHOUT the company claim is rejected 401 by the middleware.
        let claims_no_company = serde_json::json!({
            "sub": user.to_string(),
            "exp": 4102444800i64,
        });
        let token_no_company = jsonwebtoken::encode(
            &jsonwebtoken::Header::default(),
            &claims_no_company,
            &jsonwebtoken::EncodingKey::from_secret(b"probe-secret"),
        )
        .expect("mint token");
        let app =
            lazy_router().route_layer(middleware::from_fn_with_state(verifier, company_auth));
        let resp = app
            .oneshot(
                Request::builder()
                    .method(Method::GET)
                    .uri("/events")
                    .header("authorization", format!("Bearer {token_no_company}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    }

    /// Standalone create through the HTTP surface. This reports the exact
    /// engine state on the branch: with the engine implemented it must be a
    /// 201 with a persisted row and the organizer auto-attached as an
    /// accepted attendee; while the engine is still the cross-track stub it
    /// surfaces the stub's honest validation error. The result is printed
    /// LOUDLY so a reviewer sees which state held.
    #[tokio::test]
    async fn standalone_create_through_http_maps_engine_state() {
        let Some(pool) = probe_pool().await else { return };
        let company = Uuid::new_v4();
        let user_a = Uuid::new_v4();
        let app = as_caller(
            probe_router(&pool),
            &tenant_of(company, user_a),
            Some(auth_with(&[event_permissions::CREATE, event_permissions::READ])),
        );
        let resp = app
            .clone()
            .oneshot(
                Request::builder()
                    .method(Method::POST)
                    .uri("/events")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        serde_json::json!({
                            "title": "standalone-probe",
                            "startAt": "2027-01-04T09:00:00Z",
                            "stopAt": "2027-01-04T10:00:00Z",
                            "privacy": "private"
                        })
                        .to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        let status = resp.status();
        let body = body_json(resp).await;
        eprintln!(
            "PROBE standalone_create_through_http: status={status}, body={body} \
             (201 ⇒ engine live; 400 CALENDAR_EVENT_VALIDATION_ERROR ⇒ engine still the \
             cross-track stub — the HTTP contract itself is proven by the map probes)"
        );
        match status {
            StatusCode::CREATED => {
                let id: Uuid = body["id"].as_str().unwrap().parse().unwrap();
                // Row visible to the organizer (private + organizer).
                let titles =
                    list_titles_as(&pool, company, user_a, &[event_permissions::READ]).await;
                assert!(titles.contains(&"standalone-probe".to_string()));
                // Organizer auto-attendee, accepted. The verification read must
                // pin the request GUCs too: under the probe role the RLS
                // fences (correctly) hide every row from an unpinned query.
                let mut tx = pool.begin().await.expect("verify tx");
                pin_scope(&mut tx, &ScopeCtx {
                    company_id: company,
                    acting_user_id: user_a,
                })
                .await
                .expect("verify scope");
                let state: String = sqlx::query_scalar(
                    "SELECT state::text FROM calendar.event_attendees \
                     WHERE event_id = $1 AND user_id = $2",
                )
                .bind(id)
                .bind(user_a)
                .fetch_one(&mut *tx)
                .await
                .expect("organizer auto-attendee row");
                assert_eq!(state, "accepted");
            }
            StatusCode::BAD_REQUEST => {
                assert_eq!(
                    body["error"], "CALENDAR_EVENT_VALIDATION_ERROR",
                    "a 400 here must be the engine's validation surface, not a surface bug"
                );
            }
            other => panic!("unexpected status {other} for standalone create: {body}"),
        }
    }
}
