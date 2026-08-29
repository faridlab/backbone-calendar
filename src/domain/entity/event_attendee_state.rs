use serde::{Deserialize, Serialize};
use sqlx::Type;
use std::str::FromStr;
#[cfg(feature = "openapi")]
use utoipa::ToSchema;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Type)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
#[serde(rename_all = "snake_case")]
#[sqlx(type_name = "event_attendee_state", rename_all = "snake_case")]
pub enum EventAttendeeState {
    NeedsAction,
    Accepted,
    Declined,
    Tentative,
}

impl std::fmt::Display for EventAttendeeState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NeedsAction => write!(f, "needs_action"),
            Self::Accepted => write!(f, "accepted"),
            Self::Declined => write!(f, "declined"),
            Self::Tentative => write!(f, "tentative"),
        }
    }
}

impl FromStr for EventAttendeeState {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "needs_action" => Ok(Self::NeedsAction),
            "accepted" => Ok(Self::Accepted),
            "declined" => Ok(Self::Declined),
            "tentative" => Ok(Self::Tentative),
            _ => Err(format!("Unknown EventAttendeeState variant: {}", s)),
        }
    }
}

impl Default for EventAttendeeState {
    fn default() -> Self {
        Self::NeedsAction
    }
}
