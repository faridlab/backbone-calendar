//! Auth configuration for CalendarEventAttendee handlers
//!
//! Hand-written in the exact generated shape (the module does not enable the
//! generator's opt-in auth layer, so the file is user-owned — see
//! metaphor.codegen.yaml). Implements `ResourcePolicy<CalendarEventAttendee>`
//! with entity-specific permission string constants. Guard middleware logic
//! lives in backbone-auth.

use backbone_auth::{ResourcePolicy, ResourceAction, PermissionGuard, ServicePermissionGuard};
use async_trait::async_trait;

use crate::application::service::CalendarEventAttendeeService;
use crate::domain::entity::CalendarEventAttendee;

/// Permission string constants for CalendarEventAttendee operations.
pub mod permissions {
    pub const LIST:        &str = "calendar_event_attendee:list";
    pub const READ:        &str = "calendar_event_attendee:read";
    pub const CREATE:      &str = "calendar_event_attendee:create";
    pub const UPDATE:      &str = "calendar_event_attendee:update";
    pub const DELETE:      &str = "calendar_event_attendee:delete";
    pub const RESTORE:     &str = "calendar_event_attendee:restore";
    pub const TRASH:       &str = "calendar_event_attendee:trash";
    pub const EMPTY_TRASH: &str = "calendar_event_attendee:empty_trash";
    pub const BULK_CREATE: &str = "calendar_event_attendee:bulk_create";
    pub const UPSERT:      &str = "calendar_event_attendee:upsert";
}

/// Resource policy for CalendarEventAttendee — maps CRUD actions to permission strings.
pub struct CalendarEventAttendeePolicy;

#[async_trait]
impl ResourcePolicy<CalendarEventAttendee> for CalendarEventAttendeePolicy {
    async fn can(
        &self,
        action: ResourceAction,
        _entity: &CalendarEventAttendee,
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

/// Permission guard for CalendarEventAttendee handlers.
///
/// Wraps `CalendarEventAttendeeService` and checks `CalendarEventAttendeePolicy` before delegating.
pub type CalendarEventAttendeeGuard = PermissionGuard<CalendarEventAttendee>;

/// Service-integrated permission guard for CalendarEventAttendee (for use in generated handlers).
pub type CalendarEventAttendeeServiceGuard = ServicePermissionGuard<CalendarEventAttendee, CalendarEventAttendeeService, CalendarEventAttendeePolicy>;

// <<< CUSTOM
// END CUSTOM
