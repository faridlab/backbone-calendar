use serde::{Deserialize, Serialize};
use sqlx::Type;
use std::str::FromStr;
#[cfg(feature = "openapi")]
use utoipa::ToSchema;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Type)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
#[serde(rename_all = "snake_case")]
#[sqlx(type_name = "employment_status", rename_all = "snake_case")]
pub enum EmploymentStatus {
    Permanent,
    Contract,
    Probation,
    Associate,
}

impl std::fmt::Display for EmploymentStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Permanent => write!(f, "permanent"),
            Self::Contract => write!(f, "contract"),
            Self::Probation => write!(f, "probation"),
            Self::Associate => write!(f, "associate"),
        }
    }
}

impl FromStr for EmploymentStatus {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "permanent" => Ok(Self::Permanent),
            "contract" => Ok(Self::Contract),
            "probation" => Ok(Self::Probation),
            "associate" => Ok(Self::Associate),
            _ => Err(format!("Unknown EmploymentStatus variant: {}", s)),
        }
    }
}

impl Default for EmploymentStatus {
    fn default() -> Self {
        Self::Permanent
    }
}
