// This directory's extension (hand-written; ADR-0031). The generated mod.rs (lib.rs for
// src/) pulls it in with include!, so its lines resolve against this directory.

// The guarded event-family composition. Auth-gated at the module level: without
// the `auth` feature the module (and therefore every event-family route) does
// not exist — nothing event-family mounts, fail-closed.
#[cfg(feature = "auth")]
pub mod calendar_event_guarded_routes;
#[cfg(feature = "auth")]
pub use calendar_event_guarded_routes::create_calendar_event_guarded_routes;
