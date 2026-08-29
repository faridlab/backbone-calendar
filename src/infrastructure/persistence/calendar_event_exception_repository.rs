//! Repository for CalendarEventException entities
//!
//! Hand-written (user-owned). Starts as the same thin newtype the generator
//! emits for the other entities; extended with the exception-ledger queries the
//! event-family engine needs (slot claims by series, alive-only reads on
//! scoped transactions — see calendar_event_repository.rs for the scoping
//! contract).
//!
//! The ledger is what makes per-occurrence edits and deletes STICK across
//! series rewrites: a claimed `(series_id, slot_start_at, slot_stop_at)` slot
//! is never re-materialized by a rewrite, no matter how the rule changes. The
//! partial unique index `uq_calendar_event_exceptions_slot` guarantees one
//! live claim per slot; the INSERT below is idempotent against it (a
//! defensively re-claimed slot is a no-op, never an error and never a second
//! row).

use chrono::{DateTime, Utc};
use sqlx::{Postgres};
use sqlx::PgPool;
use uuid::Uuid;

use crate::domain::entity::CalendarEventException;

/// Table name for CalendarEventException entities
pub const TABLE_NAME: &str = "calendar.event_exceptions";

/// Repository for CalendarEventException entities.
///
/// All standard CRUD, soft-delete, pagination, and bulk methods are
/// provided automatically via `Deref` to `backbone_orm::GenericCrudRepository`.
pub struct CalendarEventExceptionRepository(
    backbone_orm::GenericCrudRepository<CalendarEventException, backbone_orm::SoftDelete>,
);

impl std::ops::Deref for CalendarEventExceptionRepository {
    type Target = backbone_orm::GenericCrudRepository<CalendarEventException, backbone_orm::SoftDelete>;
    fn deref(&self) -> &Self::Target { &self.0 }
}

impl CalendarEventExceptionRepository {
    /// Create a new repository instance.
    pub fn new(pool: PgPool) -> Self {
        Self(backbone_orm::GenericCrudRepository::new(pool, TABLE_NAME))
    }
}

/// Hand-written exception-ledger SQL (scoped-executor contract as above).
impl CalendarEventExceptionRepository {
    /// Claim one slot for a series: record that the occurrence that used to sit
    /// at `(slot_start_at, slot_stop_at)` was edited away or cancelled, so a
    /// future series rewrite must skip the slot.
    ///
    /// Idempotent against the partial unique index
    /// `uq_calendar_event_exceptions_slot`: re-claiming an already-claimed slot
    /// does nothing (ON CONFLICT ... DO NOTHING with the index predicate). The
    /// `kind` of the first claim wins — an edited slot stays edited; callers
    /// that need to observe the distinction read the ledger back.
    pub async fn claim_slot_scoped(
        &self,
        exec: impl sqlx::Executor<'_, Database = Postgres>,
        company_id: Uuid,
        series_id: Uuid,
        event_id: Uuid,
        slot_start_at: DateTime<Utc>,
        slot_stop_at: DateTime<Utc>,
        kind: &str,
        acting_user_id: Uuid,
    ) -> Result<(), sqlx::Error> {
        sqlx::query(
            r#"INSERT INTO calendar.event_exceptions
                   (company_id, series_id, event_id, slot_start_at, slot_stop_at,
                    kind, metadata)
               VALUES ($1, $2, $3, $4, $5, $6::event_exception_kind,
                       jsonb_build_object('created_by', $7::text))
               ON CONFLICT (series_id, slot_start_at, slot_stop_at)
                   WHERE (metadata->>'deleted_at') IS NULL
               DO NOTHING"#,
        )
        .bind(company_id)
        .bind(series_id)
        .bind(event_id)
        .bind(slot_start_at)
        .bind(slot_stop_at)
        .bind(kind)
        .bind(acting_user_id)
        .execute(exec)
        .await?;
        Ok(())
    }

    /// All alive slot claims for a series on a scoped executor — the
    /// reconciliation input for a series rewrite: every returned
    /// `(slot_start_at, slot_stop_at)` is exempt from re-materialization.
    pub async fn alive_slots_scoped(
        &self,
        exec: impl sqlx::Executor<'_, Database = Postgres>,
        series_id: Uuid,
    ) -> Result<Vec<(DateTime<Utc>, DateTime<Utc>)>, sqlx::Error> {
        sqlx::query_as::<_, (DateTime<Utc>, DateTime<Utc>)>(
            r#"SELECT slot_start_at, slot_stop_at
               FROM calendar.event_exceptions
               WHERE series_id = $1
                 AND (metadata->>'deleted_at') IS NULL"#,
        )
        .bind(series_id)
        .fetch_all(exec)
        .await
    }
}

backbone_core::impl_crud_repository!(CalendarEventExceptionRepository, CalendarEventException, soft_delete);
