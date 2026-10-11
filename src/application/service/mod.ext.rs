// This directory's extension (hand-written; ADR-0031). The generated mod.rs (lib.rs for
// src/) pulls it in with include!, so its lines resolve against this directory.

pub mod calendar_event_series_service_custom;
pub mod calendar_query_service;
// Event family engine surface (eager materialization, (start,stop)-identity
// rewrites, exception ledger, attendee writes) — the cross-track interface.
pub use calendar_event_series_service_custom::{
    CalendarEventSeriesEngine, EventFamilyError, ScopeCtx, MAX_OCCURRENCES, UNBOUNDED_HORIZON_YEARS,
};
