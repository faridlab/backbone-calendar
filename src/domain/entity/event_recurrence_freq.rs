use serde::{Deserialize, Serialize};
use sqlx::Type;
use std::str::FromStr;
#[cfg(feature = "openapi")]
use utoipa::ToSchema;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Type)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
#[serde(rename_all = "snake_case")]
#[sqlx(type_name = "event_recurrence_freq", rename_all = "snake_case")]
pub enum EventRecurrenceFreq {
    Daily,
    Weekly,
    Monthly,
    Yearly,
}

impl std::fmt::Display for EventRecurrenceFreq {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Daily => write!(f, "daily"),
            Self::Weekly => write!(f, "weekly"),
            Self::Monthly => write!(f, "monthly"),
            Self::Yearly => write!(f, "yearly"),
        }
    }
}

impl FromStr for EventRecurrenceFreq {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "daily" => Ok(Self::Daily),
            "weekly" => Ok(Self::Weekly),
            "monthly" => Ok(Self::Monthly),
            "yearly" => Ok(Self::Yearly),
            _ => Err(format!("Unknown EventRecurrenceFreq variant: {}", s)),
        }
    }
}

impl Default for EventRecurrenceFreq {
    fn default() -> Self {
        Self::Weekly
    }
}
