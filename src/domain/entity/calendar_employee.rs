use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::FromRow;
use uuid::Uuid;
use super::AuditMetadata;

/// Strongly-typed ID for CalendarEmployee
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct CalendarEmployeeId(pub Uuid);

impl CalendarEmployeeId {
    pub fn new(id: Uuid) -> Self { Self(id) }
    pub fn generate() -> Self { Self(Uuid::new_v4()) }
    pub fn into_inner(self) -> Uuid { self.0 }
}

impl std::fmt::Display for CalendarEmployeeId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl std::str::FromStr for CalendarEmployeeId {
    type Err = uuid::Error;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(Self(Uuid::parse_str(s)?))
    }
}

impl From<Uuid> for CalendarEmployeeId {
    fn from(id: Uuid) -> Self { Self(id) }
}

impl From<CalendarEmployeeId> for Uuid {
    fn from(id: CalendarEmployeeId) -> Self { id.0 }
}

impl AsRef<Uuid> for CalendarEmployeeId {
    fn as_ref(&self) -> &Uuid { &self.0 }
}

impl std::ops::Deref for CalendarEmployeeId {
    type Target = Uuid;
    fn deref(&self) -> &Self::Target { &self.0 }
}

#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct CalendarEmployee {
    pub id: Uuid,
    pub calendar_id: Uuid,
    pub employee_id: Uuid,
    #[serde(default)]
    #[sqlx(json)]
    pub metadata: AuditMetadata,
}

impl CalendarEmployee {
    /// Create a builder for CalendarEmployee
    pub fn builder() -> CalendarEmployeeBuilder {
        <CalendarEmployeeBuilder as Default>::default()
    }

    /// Create a new CalendarEmployee with required fields
    pub fn new(calendar_id: Uuid, employee_id: Uuid) -> Self {
        Self {
            id: Uuid::new_v4(),
            calendar_id,
            employee_id,
            metadata: AuditMetadata::default(),
        }
    }

    /// Get the entity's unique identifier
    pub fn id(&self) -> &Uuid {
        &self.id
    }

    /// Get a strongly-typed ID for this entity
    pub fn typed_id(&self) -> CalendarEmployeeId {
        CalendarEmployeeId(self.id)
    }

    /// Get when this entity was created
    pub fn created_at(&self) -> Option<&DateTime<Utc>> {
        self.metadata.created_at.as_ref()
    }

    /// Get when this entity was last updated
    pub fn updated_at(&self) -> Option<&DateTime<Utc>> {
        self.metadata.updated_at.as_ref()
    }

    /// Check if this entity is soft deleted
    pub fn is_deleted(&self) -> bool {
        self.metadata.deleted_at.is_some()
    }

    /// Check if this entity is active (not deleted)
    pub fn is_active(&self) -> bool {
        self.metadata.deleted_at.is_none()
    }

    /// Get when this entity was deleted
    pub fn deleted_at(&self) -> Option<&DateTime<Utc>> {
        self.metadata.deleted_at.as_ref()
    }

    /// Get who created this entity
    pub fn created_by(&self) -> Option<&Uuid> {
        self.metadata.created_by.as_ref()
    }

    /// Get who last updated this entity
    pub fn updated_by(&self) -> Option<&Uuid> {
        self.metadata.updated_by.as_ref()
    }

    /// Get who deleted this entity
    pub fn deleted_by(&self) -> Option<&Uuid> {
        self.metadata.deleted_by.as_ref()
    }


    // ==========================================================
    // Partial Update
    // ==========================================================

    /// Apply partial updates from a map of field name to JSON value
    pub fn apply_patch(&mut self, fields: std::collections::HashMap<String, serde_json::Value>) {
        for (key, value) in fields {
            match key.as_str() {
                "calendar_id" => {
                    if let Ok(v) = serde_json::from_value(value) { self.calendar_id = v; }
                }
                "employee_id" => {
                    if let Ok(v) = serde_json::from_value(value) { self.employee_id = v; }
                }
                _ => {} // ignore unknown fields
            }
        }
    }

    // <<< CUSTOM METHODS START >>>
    // <<< CUSTOM METHODS END >>>
}

impl super::Entity for CalendarEmployee {
    type Id = Uuid;

    fn entity_id(&self) -> &Self::Id {
        &self.id
    }

    fn entity_type() -> &'static str {
        "CalendarEmployee"
    }
}

impl backbone_core::PersistentEntity for CalendarEmployee {
    fn entity_id(&self) -> String {
        self.id.to_string()
    }
    fn set_entity_id(&mut self, id: String) {
        if let Ok(uuid) = uuid::Uuid::parse_str(&id) {
            self.id = uuid;
        }
    }
    fn created_at(&self) -> Option<chrono::DateTime<chrono::Utc>> {
        self.metadata.created_at
    }
    fn set_created_at(&mut self, ts: chrono::DateTime<chrono::Utc>) {
        self.metadata.created_at = Some(ts);
    }
    fn updated_at(&self) -> Option<chrono::DateTime<chrono::Utc>> {
        self.metadata.updated_at
    }
    fn set_updated_at(&mut self, ts: chrono::DateTime<chrono::Utc>) {
        self.metadata.updated_at = Some(ts);
    }
    fn deleted_at(&self) -> Option<chrono::DateTime<chrono::Utc>> {
        self.metadata.deleted_at
    }
    fn set_deleted_at(&mut self, ts: Option<chrono::DateTime<chrono::Utc>>) {
        self.metadata.deleted_at = ts;
    }
}

impl backbone_orm::EntityRepoMeta for CalendarEmployee {
    fn column_types() -> std::collections::HashMap<String, String> {
        let mut m = std::collections::HashMap::new();
        m.insert("id".to_string(), "uuid".to_string());
        m.insert("calendar_id".to_string(), "uuid".to_string());
        m.insert("employee_id".to_string(), "uuid".to_string());
        m
    }
    fn search_fields() -> &'static [&'static str] {
        &[]
    }
}

/// Builder for CalendarEmployee entity
///
/// Provides a fluent API for constructing CalendarEmployee instances.
/// System fields (id, metadata, timestamps) are auto-initialized.
#[derive(Debug, Clone, Default)]
pub struct CalendarEmployeeBuilder {
    calendar_id: Option<Uuid>,
    employee_id: Option<Uuid>,
}

impl CalendarEmployeeBuilder {
    /// Set the calendar_id field (required)
    pub fn calendar_id(mut self, value: Uuid) -> Self {
        self.calendar_id = Some(value);
        self
    }

    /// Set the employee_id field (required)
    pub fn employee_id(mut self, value: Uuid) -> Self {
        self.employee_id = Some(value);
        self
    }

    /// Build the CalendarEmployee entity
    ///
    /// Returns Err if any required field without a default is missing.
    pub fn build(self) -> Result<CalendarEmployee, String> {
        let calendar_id = self.calendar_id.ok_or_else(|| "calendar_id is required".to_string())?;
        let employee_id = self.employee_id.ok_or_else(|| "employee_id is required".to_string())?;

        Ok(CalendarEmployee {
            id: Uuid::new_v4(),
            calendar_id,
            employee_id,
            metadata: AuditMetadata::default(),
        })
    }
}
