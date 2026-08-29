use serde::{Deserialize, Serialize};
use sqlx::Type;
use std::str::FromStr;
#[cfg(feature = "openapi")]
use utoipa::ToSchema;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Type)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
#[serde(rename_all = "snake_case")]
#[sqlx(type_name = "event_exception_kind", rename_all = "snake_case")]
pub enum EventExceptionKind {
    Edited,
    Cancelled,
}

impl std::fmt::Display for EventExceptionKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Edited => write!(f, "edited"),
            Self::Cancelled => write!(f, "cancelled"),
        }
    }
}

impl FromStr for EventExceptionKind {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "edited" => Ok(Self::Edited),
            "cancelled" => Ok(Self::Cancelled),
            _ => Err(format!("Unknown EventExceptionKind variant: {}", s)),
        }
    }
}

impl Default for EventExceptionKind {
    fn default() -> Self {
        Self::Edited
    }
}
