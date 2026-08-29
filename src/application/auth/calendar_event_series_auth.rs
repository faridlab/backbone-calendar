//! Auth configuration for CalendarEventSeries handlers
//!
//! Hand-written in the exact generated shape (the module does not enable the
//! generator's opt-in auth layer, so the file is user-owned — see
//! metaphor.codegen.yaml). Implements `ResourcePolicy<CalendarEventSeries>`
//! with entity-specific permission string constants. Guard middleware logic
//! lives in backbone-auth.

use backbone_auth::{ResourcePolicy, ResourceAction, PermissionGuard, ServicePermissionGuard};
use async_trait::async_trait;

use crate::application::service::CalendarEventSeriesService;
use crate::domain::entity::CalendarEventSeries;

/// Permission string constants for CalendarEventSeries operations.
pub mod permissions {
    pub const LIST:        &str = "calendar_event_series:list";
    pub const READ:        &str = "calendar_event_series:read";
    pub const CREATE:      &str = "calendar_event_series:create";
    pub const UPDATE:      &str = "calendar_event_series:update";
    pub const DELETE:      &str = "calendar_event_series:delete";
    pub const RESTORE:     &str = "calendar_event_series:restore";
    pub const TRASH:       &str = "calendar_event_series:trash";
    pub const EMPTY_TRASH: &str = "calendar_event_series:empty_trash";
    pub const BULK_CREATE: &str = "calendar_event_series:bulk_create";
    pub const UPSERT:      &str = "calendar_event_series:upsert";
}

/// Resource policy for CalendarEventSeries — maps CRUD actions to permission strings.
pub struct CalendarEventSeriesPolicy;

#[async_trait]
impl ResourcePolicy<CalendarEventSeries> for CalendarEventSeriesPolicy {
    async fn can(
        &self,
        action: ResourceAction,
        _entity: &CalendarEventSeries,
        ctx: &backbone_auth::middleware::AuthContext,
    ) -> bool {
        let required = match action {
            ResourceAction::Read | ResourceAction::List => permissions::READ,
            ResourceAction::Create => permissions::CREATE,
            ResourceAction::Update | ResourceAction::Patch => permissions::UPDATE,
            ResourceAction::Delete | ResourceAction::HardDelete => permissions::DELETE,
            ResourceAction::Restore => permissions::RESTORE,
            ResourceAction::Custom(_) => return false,
        };
        ctx.permissions.iter().any(|p| p == required)
    }
}

/// Permission guard for CalendarEventSeries handlers.
///
/// Wraps `CalendarEventSeriesService` and checks `CalendarEventSeriesPolicy` before delegating.
pub type CalendarEventSeriesGuard = PermissionGuard<CalendarEventSeries>;

/// Service-integrated permission guard for CalendarEventSeries (for use in generated handlers).
pub type CalendarEventSeriesServiceGuard = ServicePermissionGuard<CalendarEventSeries, CalendarEventSeriesService, CalendarEventSeriesPolicy>;

// <<< CUSTOM
// END CUSTOM
