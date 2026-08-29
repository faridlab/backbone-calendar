use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::FromRow;
use uuid::Uuid;

use super::EventExceptionKind;
use super::AuditMetadata;

/// Strongly-typed ID for CalendarEventException
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct CalendarEventExceptionId(pub Uuid);

impl CalendarEventExceptionId {
    pub fn new(id: Uuid) -> Self { Self(id) }
    pub fn generate() -> Self { Self(Uuid::new_v4()) }
    pub fn into_inner(self) -> Uuid { self.0 }
}

impl std::fmt::Display for CalendarEventExceptionId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl std::str::FromStr for CalendarEventExceptionId {
    type Err = uuid::Error;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(Self(Uuid::parse_str(s)?))
    }
}

impl From<Uuid> for CalendarEventExceptionId {
    fn from(id: Uuid) -> Self { Self(id) }
}

impl From<CalendarEventExceptionId> for Uuid {
    fn from(id: CalendarEventExceptionId) -> Self { id.0 }
}

impl AsRef<Uuid> for CalendarEventExceptionId {
    fn as_ref(&self) -> &Uuid { &self.0 }
}

impl std::ops::Deref for CalendarEventExceptionId {
    type Target = Uuid;
    fn deref(&self) -> &Self::Target { &self.0 }
}

#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct CalendarEventException {
    pub id: Uuid,
    pub company_id: Uuid,
    pub series_id: Uuid,
    pub event_id: Uuid,
    pub slot_start_at: DateTime<Utc>,
    pub slot_stop_at: DateTime<Utc>,
    pub kind: EventExceptionKind,
    #[serde(default)]
    #[sqlx(json)]
    pub metadata: AuditMetadata,
}

impl CalendarEventException {
    /// Create a builder for CalendarEventException
    pub fn builder() -> CalendarEventExceptionBuilder {
        <CalendarEventExceptionBuilder as Default>::default()
    }

    /// Create a new CalendarEventException with required fields
    pub fn new(company_id: Uuid, series_id: Uuid, event_id: Uuid, slot_start_at: DateTime<Utc>, slot_stop_at: DateTime<Utc>, kind: EventExceptionKind) -> Self {
        Self {
            id: Uuid::new_v4(),
            company_id,
            series_id,
            event_id,
            slot_start_at,
            slot_stop_at,
            kind,
            metadata: AuditMetadata::default(),
        }
    }

    /// Get the entity's unique identifier
    pub fn id(&self) -> &Uuid {
        &self.id
    }

    /// Get a strongly-typed ID for this entity
    pub fn typed_id(&self) -> CalendarEventExceptionId {
        CalendarEventExceptionId(self.id)
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
                "company_id" => {
                    if let Ok(v) = serde_json::from_value(value) { self.company_id = v; }
                }
                "series_id" => {
                    if let Ok(v) = serde_json::from_value(value) { self.series_id = v; }
                }
                "event_id" => {
                    if let Ok(v) = serde_json::from_value(value) { self.event_id = v; }
                }
                "slot_start_at" => {
                    if let Ok(v) = serde_json::from_value(value) { self.slot_start_at = v; }
                }
                "slot_stop_at" => {
                    if let Ok(v) = serde_json::from_value(value) { self.slot_stop_at = v; }
                }
                "kind" => {
                    if let Ok(v) = serde_json::from_value(value) { self.kind = v; }
                }
                _ => {} // ignore unknown fields
            }
        }
    }

    // <<< CUSTOM METHODS START >>>
    // <<< CUSTOM METHODS END >>>
}

impl super::Entity for CalendarEventException {
    type Id = Uuid;

    fn entity_id(&self) -> &Self::Id {
        &self.id
    }

    fn entity_type() -> &'static str {
        "CalendarEventException"
    }
}

impl backbone_core::PersistentEntity for CalendarEventException {
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

impl backbone_orm::EntityRepoMeta for CalendarEventException {
    fn column_types() -> std::collections::HashMap<String, String> {
        let mut m = std::collections::HashMap::new();
        m.insert("id".to_string(), "uuid".to_string());
        m.insert("company_id".to_string(), "uuid".to_string());
        m.insert("series_id".to_string(), "uuid".to_string());
        m.insert("event_id".to_string(), "uuid".to_string());
        m.insert("kind".to_string(), "event_exception_kind".to_string());
        m
    }
    fn search_fields() -> &'static [&'static str] {
        &[]
    }
    fn company_field() -> Option<&'static str> {
        Some("company_id")
    }
}

/// Builder for CalendarEventException entity
///
/// Provides a fluent API for constructing CalendarEventException instances.
/// System fields (id, metadata, timestamps) are auto-initialized.
#[derive(Debug, Clone, Default)]
pub struct CalendarEventExceptionBuilder {
    company_id: Option<Uuid>,
    series_id: Option<Uuid>,
    event_id: Option<Uuid>,
    slot_start_at: Option<DateTime<Utc>>,
    slot_stop_at: Option<DateTime<Utc>>,
    kind: Option<EventExceptionKind>,
}

impl CalendarEventExceptionBuilder {
    /// Set the company_id field (required)
    pub fn company_id(mut self, value: Uuid) -> Self {
        self.company_id = Some(value);
        self
    }

    /// Set the series_id field (required)
    pub fn series_id(mut self, value: Uuid) -> Self {
        self.series_id = Some(value);
        self
    }

    /// Set the event_id field (required)
    pub fn event_id(mut self, value: Uuid) -> Self {
        self.event_id = Some(value);
        self
    }

    /// Set the slot_start_at field (required)
    pub fn slot_start_at(mut self, value: DateTime<Utc>) -> Self {
        self.slot_start_at = Some(value);
        self
    }

    /// Set the slot_stop_at field (required)
    pub fn slot_stop_at(mut self, value: DateTime<Utc>) -> Self {
        self.slot_stop_at = Some(value);
        self
    }

    /// Set the kind field (required)
    pub fn kind(mut self, value: EventExceptionKind) -> Self {
        self.kind = Some(value);
        self
    }

    /// Build the CalendarEventException entity
    ///
    /// Returns Err if any required field without a default is missing.
    pub fn build(self) -> Result<CalendarEventException, String> {
        let company_id = self.company_id.ok_or_else(|| "company_id is required".to_string())?;
        let series_id = self.series_id.ok_or_else(|| "series_id is required".to_string())?;
        let event_id = self.event_id.ok_or_else(|| "event_id is required".to_string())?;
        let slot_start_at = self.slot_start_at.ok_or_else(|| "slot_start_at is required".to_string())?;
        let slot_stop_at = self.slot_stop_at.ok_or_else(|| "slot_stop_at is required".to_string())?;
        let kind = self.kind.ok_or_else(|| "kind is required".to_string())?;

        Ok(CalendarEventException {
            id: Uuid::new_v4(),
            company_id,
            series_id,
            event_id,
            slot_start_at,
            slot_stop_at,
            kind,
            metadata: AuditMetadata::default(),
        })
    }
}
