//! Auth configuration for CalendarEventException handlers
//!
//! Hand-written in the exact generated shape (the module does not enable the
//! generator's opt-in auth layer, so the file is user-owned — see
//! metaphor.codegen.yaml). The exception ledger itself gets NO route — the
//! policy exists so internal callers can still permission-check ledger reads.

use backbone_auth::{ResourcePolicy, ResourceAction, PermissionGuard, ServicePermissionGuard};
use async_trait::async_trait;

use crate::application::service::CalendarEventExceptionService;
use crate::domain::entity::CalendarEventException;

/// Permission string constants for CalendarEventException operations.
pub mod permissions {
    pub const LIST:        &str = "calendar_event_exception:list";
    pub const READ:        &str = "calendar_event_exception:read";
    pub const CREATE:      &str = "calendar_event_exception:create";
    pub const UPDATE:      &str = "calendar_event_exception:update";
    pub const DELETE:      &str = "calendar_event_exception:delete";
    pub const RESTORE:     &str = "calendar_event_exception:restore";
    pub const TRASH:       &str = "calendar_event_exception:trash";
    pub const EMPTY_TRASH: &str = "calendar_event_exception:empty_trash";
    pub const BULK_CREATE: &str = "calendar_event_exception:bulk_create";
    pub const UPSERT:      &str = "calendar_event_exception:upsert";
}

/// Resource policy for CalendarEventException — maps CRUD actions to permission strings.
pub struct CalendarEventExceptionPolicy;

#[async_trait]
impl ResourcePolicy<CalendarEventException> for CalendarEventExceptionPolicy {
    async fn can(
        &self,
        action: ResourceAction,
        _entity: &CalendarEventException,
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

/// Permission guard for CalendarEventException handlers.
///
/// Wraps `CalendarEventExceptionService` and checks `CalendarEventExceptionPolicy` before delegating.
pub type CalendarEventExceptionGuard = PermissionGuard<CalendarEventException>;

/// Service-integrated permission guard for CalendarEventException (for use in generated handlers).
pub type CalendarEventExceptionServiceGuard = ServicePermissionGuard<CalendarEventException, CalendarEventExceptionService, CalendarEventExceptionPolicy>;

// <<< CUSTOM
// END CUSTOM
