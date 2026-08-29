use serde::{Deserialize, Serialize};
use sqlx::Type;
use std::str::FromStr;
#[cfg(feature = "openapi")]
use utoipa::ToSchema;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Type)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
#[serde(rename_all = "snake_case")]
#[sqlx(type_name = "event_privacy", rename_all = "snake_case")]
pub enum EventPrivacy {
    Public,
    Private,
    Confidential,
}

impl std::fmt::Display for EventPrivacy {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Public => write!(f, "public"),
            Self::Private => write!(f, "private"),
            Self::Confidential => write!(f, "confidential"),
        }
    }
}

impl FromStr for EventPrivacy {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "public" => Ok(Self::Public),
            "private" => Ok(Self::Private),
            "confidential" => Ok(Self::Confidential),
            _ => Err(format!("Unknown EventPrivacy variant: {}", s)),
        }
    }
}

impl Default for EventPrivacy {
    fn default() -> Self {
        Self::Public
    }
}
