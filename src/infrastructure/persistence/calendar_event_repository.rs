//! Repository for CalendarEvent entities
//!
//! Hand-written (user-owned). Starts as the same thin newtype the generator
//! emits for the other entities; extended with the RLS-scoped transaction
//! reads/writes the event-family engine needs (every query must run on a
//! connection that has pinned `app.company_id` / `app.user_id` via
//! `set_config(..., true)` so the company fence and the privacy read fence
//! evaluate per request).
//!
//! NOTE (event-family fence): nothing in the event family may consult
//! `CalendarRepository::working_days` — that read-port answers with a
//! company-wide Mon–Fri-minus-holidays simplification (with known unresolved
//! scope junctions) that is working-time-family only. Event availability math
//! is out of scope this wave; when it arrives it must not silently inherit
//! that simplification.
//!
//! Scoping contract: the custom methods below all take an `impl Executor`
//! that the caller obtained from [`CalendarEventRepository::begin_scope`] — a
//! transaction whose connection has `app.company_id` and `app.user_id` pinned
//! transaction-locally. Because the pin is transaction-local, it can never
//! leak onto a pooled connection reused by the next request; because every
//! statement runs inside that transaction, row-level security (company
//! isolation + the event privacy read fence) evaluates every row the engine
//! touches.

use chrono::{DateTime, Utc};
use sqlx::{PgPool, Postgres};
use uuid::Uuid;

use crate::domain::entity::CalendarEvent;

use super::CalendarEventSeriesRepository;

/// Table name for CalendarEvent entities
pub const TABLE_NAME: &str = "calendar.events";

/// Repository for CalendarEvent entities.
///
/// All standard CRUD, soft-delete, pagination, and bulk methods are
/// provided automatically via `Deref` to `backbone_orm::GenericCrudRepository`.
pub struct CalendarEventRepository(
    backbone_orm::GenericCrudRepository<CalendarEvent, backbone_orm::SoftDelete>,
);

impl std::ops::Deref for CalendarEventRepository {
    type Target = backbone_orm::GenericCrudRepository<CalendarEvent, backbone_orm::SoftDelete>;
    fn deref(&self) -> &Self::Target { &self.0 }
}

impl CalendarEventRepository {
    /// Create a new repository instance.
    pub fn new(pool: PgPool) -> Self {
        Self(backbone_orm::GenericCrudRepository::new(pool, TABLE_NAME))
    }
}

/// Hand-written event-family SQL. Lives here (not in the series engine) per the
/// module's 4-layer rule: services orchestrate, repositories hold the SQL.
impl CalendarEventRepository {
    /// Open a scoped transaction: a pooled connection with `app.company_id`
    /// and `app.user_id` pinned via `set_config(..., is_local = true)`.
    ///
    /// The transaction-local pin is what makes both RLS fences real for every
    /// statement the engine runs on the returned transaction: the permissive
    /// company-isolation policies on all four event-family tables, and the
    /// restrictive privacy read policy on `calendar.events`. A transaction-local
    /// setting is reset automatically at COMMIT/ROLLBACK, so a pooled connection
    /// can never carry one request's scope into the next.
    ///
    /// An unset `app.user_id` (which this helper never leaves — it always pins
    /// both variables) would fail the privacy fence closed to public rows only;
    /// callers that need private reads must pass the acting user they act for.
    pub async fn begin_scope(
        &self,
        pool: &PgPool,
        company_id: Uuid,
        acting_user_id: Uuid,
    ) -> Result<sqlx::Transaction<'static, Postgres>, sqlx::Error> {
        let mut tx = pool.begin().await?;
        sqlx::query(
            "SELECT set_config('app.company_id', $1, true), set_config('app.user_id', $2, true)",
        )
        .bind(company_id.to_string())
        .bind(acting_user_id.to_string())
        .execute(&mut *tx)
        .await?;
        Ok(tx)
    }

    /// Fetch one event by id on a scoped executor. Invisible rows (wrong
    /// company per the strict fence, or non-public without the acting user as
    /// organizer/attendee per the privacy fence) simply do not exist to this
    /// query — the caller maps `None` to not-found.
    pub async fn find_by_id_scoped(
        &self,
        exec: impl sqlx::Executor<'_, Database = Postgres>,
        id: Uuid,
    ) -> Result<Option<CalendarEvent>, sqlx::Error> {
        sqlx::query_as::<_, CalendarEvent>(
            r#"SELECT id, company_id, series_id, title, description,
                      start_at, stop_at, privacy, organizer_user_id, location, metadata
               FROM calendar.events
               WHERE id = $1"#,
        )
        .bind(id)
        .fetch_optional(exec)
        .await
    }

    /// Alive member rows of a series on a scoped executor, ordered by start:
    /// `(id, start_at, stop_at, organizer_user_id)` per occurrence. This is the
    /// read side of the (start, stop) reconcile — the tuple identity the series
    /// rewrite algorithm matches rows by.
    pub async fn member_rows_scoped(
        &self,
        exec: impl sqlx::Executor<'_, Database = Postgres>,
        series_id: Uuid,
    ) -> Result<Vec<(Uuid, DateTime<Utc>, DateTime<Utc>, Uuid)>, sqlx::Error> {
        sqlx::query_as::<_, (Uuid, DateTime<Utc>, DateTime<Utc>, Uuid)>(
            r#"SELECT id, start_at, stop_at, organizer_user_id
               FROM calendar.events
               WHERE series_id = $1
                 AND (metadata->>'deleted_at') IS NULL
               ORDER BY start_at"#,
        )
        .bind(series_id)
        .fetch_all(exec)
        .await
    }

    /// Insert one event on a scoped executor, returning its id. The privacy
    /// value is bound as text and cast explicitly so no enum OID negotiation is
    /// needed at bind time; `created_by` is stamped into the audit metadata
    /// alongside the insert trigger's `created_at`/`updated_at`.
    #[allow(clippy::too_many_arguments)]
    pub async fn insert_event_scoped(
        &self,
        exec: impl sqlx::Executor<'_, Database = Postgres>,
        company_id: Uuid,
        series_id: Option<Uuid>,
        title: &str,
        description: Option<&str>,
        start_at: DateTime<Utc>,
        stop_at: DateTime<Utc>,
        privacy: &str,
        organizer_user_id: Uuid,
        location: Option<&str>,
    ) -> Result<Uuid, sqlx::Error> {
        let id: Uuid = sqlx::query_scalar(
            r#"INSERT INTO calendar.events
                   (company_id, series_id, title, description, start_at, stop_at,
                    privacy, organizer_user_id, location, metadata)
               VALUES ($1, $2, $3, $4, $5, $6,
                       $7::event_privacy, $8, $9,
                       jsonb_build_object('created_by', $8::text))
               RETURNING id"#,
        )
        .bind(company_id)
        .bind(series_id)
        .bind(title)
        .bind(description)
        .bind(start_at)
        .bind(stop_at)
        .bind(privacy)
        .bind(organizer_user_id)
        .bind(location)
        .fetch_one(exec)
        .await?;
        Ok(id)
    }

    /// Bulk-insert occurrence rows for one series on a scoped executor — the
    /// eager half of the eager-materialization posture: every recurrence slot
    /// becomes a real `calendar.events` row. Column values ride UNNEST arrays
    /// (one round trip regardless of slot count); ids come back so attendees
    /// can be attached to every materialized row in the same transaction.
    ///
    /// The partial unique index `uq_calendar_events_series_slot`
    /// `(series_id, start_at, stop_at)` backstops the (start, stop) identity:
    /// a double-materialization of one slot aborts the statement (and the
    /// caller's transaction) with a unique violation — never a silent second
    /// row.
    #[allow(clippy::too_many_arguments)]
    pub async fn bulk_insert_members_scoped(
        &self,
        exec: impl sqlx::Executor<'_, Database = Postgres>,
        company_id: Uuid,
        series_id: Uuid,
        title: &str,
        description: Option<&str>,
        starts: &[DateTime<Utc>],
        stops: &[DateTime<Utc>],
        privacy: &str,
        organizer_user_id: Uuid,
        location: Option<&str>,
    ) -> Result<Vec<Uuid>, sqlx::Error> {
        let n = starts.len();
        let titles: Vec<String> = vec![title.to_string(); n];
        let descriptions: Vec<Option<String>> = vec![description.map(str::to_string); n];
        let privacies: Vec<String> = vec![privacy.to_string(); n];
        let organizers: Vec<Uuid> = vec![organizer_user_id; n];
        let locations: Vec<Option<String>> = vec![location.map(str::to_string); n];

        let ids: Vec<Uuid> = sqlx::query_scalar(
            r#"INSERT INTO calendar.events
                   (company_id, series_id, title, description, start_at, stop_at,
                    privacy, organizer_user_id, location, metadata)
               SELECT company_id, series_id, title, description, start_at, stop_at,
                      privacy::event_privacy, organizer_user_id, location,
                      jsonb_build_object('created_by', organizer_user_id::text)
               FROM UNNEST($1::uuid[], $2::uuid[], $3::text[], $4::text[],
                           $5::timestamptz[], $6::timestamptz[], $7::text[],
                           $8::uuid[], $9::text[])
                    AS u(company_id, series_id, title, description, start_at,
                         stop_at, privacy, organizer_user_id, location)
               RETURNING id"#,
        )
        .bind(vec![company_id; n])
        .bind(vec![series_id; n])
        .bind(titles)
        .bind(descriptions)
        .bind(starts.to_vec())
        .bind(stops.to_vec())
        .bind(privacies)
        .bind(organizers)
        .bind(locations)
        .fetch_all(exec)
        .await?;
        Ok(ids)
    }

    /// Apply a field patch to one event on a scoped executor. `None` means
    /// "leave unchanged" (COALESCE); a provided start/stop pair must satisfy
    /// `stop_at > start_at` (the caller validates — the DB CHECK backstops).
    /// The row's `updated_by` audit stamp records the acting user.
    #[allow(clippy::too_many_arguments)]
    pub async fn apply_edits_scoped(
        &self,
        exec: impl sqlx::Executor<'_, Database = Postgres>,
        id: Uuid,
        title: Option<&str>,
        description: Option<&str>,
        start_at: Option<DateTime<Utc>>,
        stop_at: Option<DateTime<Utc>>,
        privacy: Option<&str>,
        location: Option<&str>,
        acting_user_id: Uuid,
    ) -> Result<u64, sqlx::Error> {
        let rows = sqlx::query(
            r#"UPDATE calendar.events SET
                   title       = COALESCE($2, title),
                   description = COALESCE($3, description),
                   start_at    = COALESCE($4, start_at),
                   stop_at     = COALESCE($5, stop_at),
                   privacy     = COALESCE($6::event_privacy, privacy),
                   location    = COALESCE($7, location),
                   metadata    = jsonb_set(metadata, '{updated_by}', to_jsonb($8::text))
               WHERE id = $1"#,
        )
        .bind(id)
        .bind(title)
        .bind(description)
        .bind(start_at)
        .bind(stop_at)
        .bind(privacy)
        .bind(location)
        .bind(acting_user_id)
        .execute(exec)
        .await?
        .rows_affected();
        Ok(rows)
    }

    /// Overwrite the content fields of an aligned row during a series rewrite
    /// (the (start, stop) tuple already matches the new rule grid, so times and
    /// the id stay put — only what the series template carries is rewritten).
    pub async fn apply_series_template_scoped(
        &self,
        exec: impl sqlx::Executor<'_, Database = Postgres>,
        id: Uuid,
        title: &str,
        description: Option<&str>,
        privacy: &str,
        location: Option<&str>,
        acting_user_id: Uuid,
    ) -> Result<u64, sqlx::Error> {
        let rows = sqlx::query(
            r#"UPDATE calendar.events SET
                   title       = $2,
                   description = $3,
                   privacy     = $4::event_privacy,
                   location    = $5,
                   metadata    = jsonb_set(metadata, '{updated_by}', to_jsonb($6::text))
               WHERE id = $1"#,
        )
        .bind(id)
        .bind(title)
        .bind(description)
        .bind(privacy)
        .bind(location)
        .bind(acting_user_id)
        .execute(exec)
        .await?
        .rows_affected();
        Ok(rows)
    }

    /// Detach one event from its series (`series_id = NULL`). The row survives
    /// standalone — this is the "editing an occurrence splits it from the
    /// series" half of the exception mechanism; the matching exception-ledger
    /// claim (same transaction) is what keeps the freed slot from ever being
    /// re-materialized. The audit stamp records who split it.
    pub async fn detach_from_series_scoped(
        &self,
        exec: impl sqlx::Executor<'_, Database = Postgres>,
        id: Uuid,
        acting_user_id: Uuid,
    ) -> Result<u64, sqlx::Error> {
        let rows = sqlx::query(
            r#"UPDATE calendar.events SET
                   series_id = NULL,
                   metadata  = jsonb_set(metadata, '{updated_by}', to_jsonb($2::text))
               WHERE id = $1"#,
        )
        .bind(id)
        .bind(acting_user_id)
        .execute(exec)
        .await?
        .rows_affected();
        Ok(rows)
    }

    /// Soft-delete one event (audit metadata `deleted_at`/`deleted_by` stamps —
    /// the house soft-delete shape). A soft-deleted series member frees its
    /// `(series_id, start_at, stop_at)` slot for the partial unique index, and
    /// the caller's exception claim (`cancelled`) keeps rewrites from
    /// resurrecting the occurrence.
    pub async fn soft_delete_scoped(
        &self,
        exec: impl sqlx::Executor<'_, Database = Postgres>,
        id: Uuid,
        acting_user_id: Uuid,
    ) -> Result<u64, sqlx::Error> {
        let rows = sqlx::query(
            r#"UPDATE calendar.events SET
                   metadata = jsonb_set(
                       jsonb_set(metadata, '{deleted_at}', to_jsonb(NOW())),
                       '{deleted_by}', to_jsonb($2::text))
               WHERE id = $1
                 AND (metadata->>'deleted_at') IS NULL"#,
        )
        .bind(id)
        .bind(acting_user_id)
        .execute(exec)
        .await?
        .rows_affected();
        Ok(rows)
    }
}

/// Series-table scoped SQL. The generated `CalendarEventSeriesRepository` file
/// is not user-owned, so its scoped extensions live in this user-owned sibling
/// (Rust permits the impl block anywhere in the same crate; the generated file
/// itself stays untouched). Same scoping contract as the event methods above.
impl CalendarEventSeriesRepository {
    /// Fetch one series row by id on a scoped executor (company fence applies).
    pub async fn find_series_scoped(
        &self,
        exec: impl sqlx::Executor<'_, Database = Postgres>,
        id: Uuid,
    ) -> Result<Option<crate::domain::entity::CalendarEventSeries>, sqlx::Error> {
        sqlx::query_as::<_, crate::domain::entity::CalendarEventSeries>(
            r#"SELECT id, company_id, name, freq, interval, by_weekday, by_monthday,
                      until, count, base_event_id, metadata
               FROM calendar.event_series
               WHERE id = $1"#,
        )
        .bind(id)
        .fetch_optional(exec)
        .await
    }

    /// Insert one series row on a scoped executor with a CALLER-MINTED id.
    ///
    /// The id must be supplied (not DB-minted) because the engine mints the
    /// series id BEFORE inserting the member rows that reference it — the
    /// base event and every occurrence carry `series_id` from the first
    /// statement of the transaction, so the series row must land under the
    /// exact same id or the series-scoped reads (occurrences, rewrite,
    /// trim) can never find it.
    #[allow(clippy::too_many_arguments)]
    pub async fn insert_series_scoped(
        &self,
        exec: impl sqlx::Executor<'_, Database = Postgres>,
        id: Uuid,
        company_id: Uuid,
        name: Option<&str>,
        freq: &str,
        interval: i32,
        by_weekday: Option<&str>,
        by_monthday: Option<&str>,
        until: Option<chrono::NaiveDate>,
        count: Option<i32>,
        base_event_id: Uuid,
        acting_user_id: Uuid,
    ) -> Result<Uuid, sqlx::Error> {
        let id: Uuid = sqlx::query_scalar(
            r#"INSERT INTO calendar.event_series
                   (id, company_id, name, freq, interval, by_weekday, by_monthday,
                    until, count, base_event_id, metadata)
               VALUES ($1, $2, $3, $4::event_recurrence_freq, $5, $6, $7,
                       $8, $9, $10, jsonb_build_object('created_by', $11::text))
               RETURNING id"#,
        )
        .bind(id)
        .bind(company_id)
        .bind(name)
        .bind(freq)
        .bind(interval)
        .bind(by_weekday)
        .bind(by_monthday)
        .bind(until)
        .bind(count)
        .bind(base_event_id)
        .bind(acting_user_id)
        .fetch_one(exec)
        .await?;
        Ok(id)
    }

    /// Overwrite a series' rule (rewrite/edit-scope-all) on a scoped executor.
    #[allow(clippy::too_many_arguments)]
    pub async fn update_series_rule_scoped(
        &self,
        exec: impl sqlx::Executor<'_, Database = Postgres>,
        id: Uuid,
        name: Option<&str>,
        freq: &str,
        interval: i32,
        by_weekday: Option<&str>,
        by_monthday: Option<&str>,
        until: Option<chrono::NaiveDate>,
        count: Option<i32>,
        base_event_id: Uuid,
    ) -> Result<u64, sqlx::Error> {
        let rows = sqlx::query(
            r#"UPDATE calendar.event_series SET
                   name         = $2,
                   freq         = $3::event_recurrence_freq,
                   interval     = $4,
                   by_weekday   = $5,
                   by_monthday  = $6,
                   until        = $7,
                   count        = $8,
                   base_event_id = $9,
                   metadata     = jsonb_set(metadata, '{updated_at}', to_jsonb(NOW()))
               WHERE id = $1"#,
        )
        .bind(id)
        .bind(name)
        .bind(freq)
        .bind(interval)
        .bind(by_weekday)
        .bind(by_monthday)
        .bind(until)
        .bind(count)
        .bind(base_event_id)
        .execute(exec)
        .await?
        .rows_affected();
        Ok(rows)
    }

    /// Trim a series' `until` to an inclusive date (the edit-scope-following
    /// split: the day before the occurrence the tail detached at).
    pub async fn trim_series_until_scoped(
        &self,
        exec: impl sqlx::Executor<'_, Database = Postgres>,
        id: Uuid,
        until: chrono::NaiveDate,
    ) -> Result<u64, sqlx::Error> {
        let rows = sqlx::query(
            r#"UPDATE calendar.event_series SET
                   until    = $2,
                   metadata = jsonb_set(metadata, '{updated_at}', to_jsonb(NOW()))
               WHERE id = $1"#,
        )
        .bind(id)
        .bind(until)
        .execute(exec)
        .await?
        .rows_affected();
        Ok(rows)
    }
}

backbone_core::impl_crud_repository!(CalendarEventRepository, CalendarEvent, soft_delete);
