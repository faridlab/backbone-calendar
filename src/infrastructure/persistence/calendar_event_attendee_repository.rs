//! Repository for CalendarEventAttendee entities
//!
//! Hand-written (user-owned). Starts as the same thin newtype the generator
//! emits for the other entities; extended with the attendee writes the
//! event-family engine needs. Dedup contract: the application dedups at write
//! time (first-wins) and translates the DB unique-violation on
//! (event_id, user_id) into `EventFamilyError::DuplicateAttendee`; the
//! PARTIAL unique index `uq_calendar_event_attendees_event_user` is the
//! backstop that survives every other write path.
//!
//! The plain unique index `uq_calendar_event_attendees_token` on
//! `access_token` is the invitation/ics token seam: the DB mints tokens via
//! `gen_random_uuid()` and the column is unique forever — no transport reads it
//! yet (the invitation flows are a later wave).

use sqlx::{PgPool, Postgres};
use uuid::Uuid;

use crate::domain::entity::CalendarEventAttendee;

/// Table name for CalendarEventAttendee entities
pub const TABLE_NAME: &str = "calendar.event_attendees";

/// Repository for CalendarEventAttendee entities.
///
/// All standard CRUD, soft-delete, pagination, and bulk methods are
/// provided automatically via `Deref` to `backbone_orm::GenericCrudRepository`.
pub struct CalendarEventAttendeeRepository(
    backbone_orm::GenericCrudRepository<CalendarEventAttendee, backbone_orm::SoftDelete>,
);

impl std::ops::Deref for CalendarEventAttendeeRepository {
    type Target = backbone_orm::GenericCrudRepository<CalendarEventAttendee, backbone_orm::SoftDelete>;
    fn deref(&self) -> &Self::Target { &self.0 }
}

impl CalendarEventAttendeeRepository {
    /// Create a new repository instance.
    pub fn new(pool: PgPool) -> Self {
        Self(backbone_orm::GenericCrudRepository::new(pool, TABLE_NAME))
    }
}

/// Hand-written attendee SQL (scoped-executor contract as above).
impl CalendarEventAttendeeRepository {
    /// Bulk-insert attendee rows on a scoped executor — the cartesian product
    /// of `event_ids × user_states` rides UNNEST arrays in one round trip.
    /// Each row gets a DB-minted `access_token` (the invitation seam).
    ///
    /// Errors with SQLSTATE 23505 on `uq_calendar_event_attendees_event_user`
    /// surface as `Err` — the engine maps them to
    /// `EventFamilyError::DuplicateAttendee` (the 409 the HTTP surface
    /// reports); the pre-check there catches the common case with the exact
    /// user id, this constraint catches everything else.
    pub async fn bulk_insert_scoped(
        &self,
        exec: impl sqlx::Executor<'_, Database = Postgres>,
        company_id: Uuid,
        event_ids: &[Uuid],
        user_states: &[(Uuid, &str)],
        acting_user_id: Uuid,
    ) -> Result<(), sqlx::Error> {
        let mut companies = Vec::with_capacity(event_ids.len() * user_states.len());
        let mut events = Vec::with_capacity(event_ids.len() * user_states.len());
        let mut users = Vec::with_capacity(event_ids.len() * user_states.len());
        let mut states = Vec::with_capacity(event_ids.len() * user_states.len());
        for event_id in event_ids {
            for (user_id, state) in user_states {
                companies.push(company_id);
                events.push(*event_id);
                users.push(*user_id);
                states.push(state.to_string());
            }
        }

        sqlx::query(
            r#"INSERT INTO calendar.event_attendees
                   (company_id, event_id, user_id, state, metadata)
               SELECT company_id, event_id, user_id,
                      state::event_attendee_state,
                      jsonb_build_object('created_by', $5::text)
               FROM UNNEST($1::uuid[], $2::uuid[], $3::uuid[], $4::text[])
                    AS u(company_id, event_id, user_id, state)"#,
        )
        .bind(&companies)
        .bind(&events)
        .bind(&users)
        .bind(&states)
        .bind(acting_user_id)
        .execute(exec)
        .await?;
        Ok(())
    }

    /// All live attendee rows of one event on a scoped executor — the
    /// pre-check input for duplicate rejection, and the attendee set a series
    /// rewrite copies onto newly materialized occurrences.
    pub async fn alive_attendees_of_event_scoped(
        &self,
        exec: impl sqlx::Executor<'_, Database = Postgres>,
        event_id: Uuid,
    ) -> Result<Vec<CalendarEventAttendee>, sqlx::Error> {
        sqlx::query_as::<_, CalendarEventAttendee>(
            r#"SELECT id, company_id, event_id, user_id, state, access_token, metadata
               FROM calendar.event_attendees
               WHERE event_id = $1
                 AND (metadata->>'deleted_at') IS NULL"#,
        )
        .bind(event_id)
        .fetch_all(exec)
        .await
    }

    /// Hand-set an attendee's response state on a scoped executor (no
    /// validation gate beyond the enum — the states are hand-set by design).
    /// Returns the rows affected: 0 maps to not-found.
    pub async fn set_state_scoped(
        &self,
        exec: impl sqlx::Executor<'_, Database = Postgres>,
        attendee_id: Uuid,
        state: &str,
        acting_user_id: Uuid,
    ) -> Result<u64, sqlx::Error> {
        let rows = sqlx::query(
            r#"UPDATE calendar.event_attendees SET
                   state    = $2::event_attendee_state,
                   metadata = jsonb_set(metadata, '{updated_by}', to_jsonb($3::text))
               WHERE id = $1
                 AND (metadata->>'deleted_at') IS NULL"#,
        )
        .bind(attendee_id)
        .bind(state)
        .bind(acting_user_id)
        .execute(exec)
        .await?
        .rows_affected();
        Ok(rows)
    }
}

backbone_core::impl_crud_repository!(CalendarEventAttendeeRepository, CalendarEventAttendee, soft_delete);
