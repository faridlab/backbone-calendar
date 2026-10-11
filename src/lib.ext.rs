//! The module's extension (hand-written; ADR-0031): what the module adds beside its
//! generated services. The generated module and builder carry `ModuleExt` and
//! `ModuleBuilderExt` and dereference to them, so their fields read as `module.field`.

#[allow(unused_imports)]
use super::*;

/// State the module adds; read through the generated module's `Deref`.
pub struct ModuleExt {
    // Held so the `CalendarQueryService` impl can delegate `working_days` to the repo's
    // hand-written SQL, and standard lookups to the CRUD services above. `db_pool` is the same pool
    // the repo was constructed with (the repo's `working_days` takes it per the backbone-hr read-port
    // convention; when the composing service bound an org request scope, the repo relays it onto a
    // read transaction so the decorator's row-level fence applies).
    pub(crate) calendar_repository: Arc<CalendarRepository>,
    pub(crate) db_pool: sqlx::PgPool,
    // Event family: the series engine (eager materialization, (start,stop)-identity
    // rewrites, exception ledger, attendee writes), which holds the hand-owned
    // repositories it drives. Held here so `calendar_event_routes()` can hand it to the
    // guarded HTTP composition; every engine query runs on a transaction that relays the composing
    // service's request org scope and pins `app.user_id`, which is what makes the
    // org fence (as installed by the decorator) and the domain privacy read fence real.
    // Read only by `calendar_event_routes()`, which exists with the `auth` feature.
    #[cfg_attr(not(feature = "auth"), allow(dead_code))]
    pub(crate) calendar_event_series_engine: Arc<application::service::CalendarEventSeriesEngine>,
}

/// State the builder adds.
pub struct ModuleBuilderExt {
}

impl Default for ModuleBuilderExt {
    fn default() -> Self {
        Self {
        }
    }
}

impl ModuleBuilderExt {
    /// Build the extension's state from what the generated build made.
    #[allow(unused_variables, clippy::redundant_clone)]
    pub(crate) fn build(self, parts: &ModuleParts<'_>) -> anyhow::Result<ModuleExt> {
        let db_pool = parts.db_pool.clone();
        let calendar_repository = parts.calendar_repository.clone();
        let calendar_event_attendee_repository = parts.calendar_event_attendee_repository.clone();
        let calendar_event_exception_repository = parts.calendar_event_exception_repository.clone();
        let calendar_event_repository = parts.calendar_event_repository.clone();
        let calendar_event_series_repository = parts.calendar_event_series_repository.clone();
        // Event family engine: the eagerly-materialized series operations run on
        // scoped transactions over the same pool, driving the hand-owned event-family
        // repositories plus the generated series repository.
        let calendar_event_series_engine =
            Arc::new(application::service::CalendarEventSeriesEngine::new(
                db_pool.clone(),
                calendar_event_repository.clone(),
                calendar_event_series_repository.clone(),
                calendar_event_exception_repository.clone(),
                calendar_event_attendee_repository.clone(),
            ));
        Ok(ModuleExt {
            calendar_repository: calendar_repository.clone(),
            db_pool,
            calendar_event_series_engine,
        })
    }
}

impl crate::ThisModule {
    /// The guarded event-family HTTP surface (schema-named mounts under
    /// `/events`, `/event-series`, `/event-attendees`; every route behind an
    /// explicit permission guard — fail-closed). Exists only with the `auth`
    /// feature: without it, no event-family route mounts at all. The generated
    /// 12-endpoint CRUD for the family is deliberately NOT mounted here; this
    /// is the sole public surface.
    #[cfg(feature = "auth")]
    pub fn calendar_event_routes(&self) -> Router {
        use presentation::http::create_calendar_event_guarded_routes;
        create_calendar_event_guarded_routes(
            self.calendar_event_series_engine.clone(),
            self.calendar_event_service.clone(),
            self.db_pool.clone(),
        )
    }
}
