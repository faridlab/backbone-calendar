//! Auth configuration for CalendarEvent handlers
//!
//! Hand-written in the exact generated shape (the module does not enable the
//! generator's opt-in auth layer, so the file is user-owned — see
//! metaphor.codegen.yaml). Implements `ResourcePolicy<CalendarEvent>` with
//! entity-specific permission string constants. Guard middleware logic lives
//! in backbone-auth.

use backbone_auth::{ResourcePolicy, ResourceAction, PermissionGuard, ServicePermissionGuard};
use async_trait::async_trait;

use crate::application::service::CalendarEventService;
use crate::domain::entity::CalendarEvent;

/// Permission string constants for CalendarEvent operations.
pub mod permissions {
    pub const LIST:        &str = "calendar_event:list";
    pub const READ:        &str = "calendar_event:read";
    pub const CREATE:      &str = "calendar_event:create";
    pub const UPDATE:      &str = "calendar_event:update";
    pub const DELETE:      &str = "calendar_event:delete";
    pub const RESTORE:     &str = "calendar_event:restore";
    pub const TRASH:       &str = "calendar_event:trash";
    pub const EMPTY_TRASH: &str = "calendar_event:empty_trash";
    pub const BULK_CREATE: &str = "calendar_event:bulk_create";
    pub const UPSERT:      &str = "calendar_event:upsert";
}

/// Resource policy for CalendarEvent — maps CRUD actions to permission strings.
pub struct CalendarEventPolicy;

#[async_trait]
impl ResourcePolicy<CalendarEvent> for CalendarEventPolicy {
    async fn can(
        &self,
        action: ResourceAction,
        _entity: &CalendarEvent,
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

/// Permission guard for CalendarEvent handlers.
///
/// Wraps `CalendarEventService` and checks `CalendarEventPolicy` before delegating.
pub type CalendarEventGuard = PermissionGuard<CalendarEvent>;

/// Service-integrated permission guard for CalendarEvent (for use in generated handlers).
pub type CalendarEventServiceGuard = ServicePermissionGuard<CalendarEvent, CalendarEventService, CalendarEventPolicy>;

// <<< CUSTOM
// END CUSTOM
