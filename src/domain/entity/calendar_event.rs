use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::FromRow;
use uuid::Uuid;

use super::EventPrivacy;
use super::AuditMetadata;

/// Strongly-typed ID for CalendarEvent
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct CalendarEventId(pub Uuid);

impl CalendarEventId {
    pub fn new(id: Uuid) -> Self { Self(id) }
    pub fn generate() -> Self { Self(Uuid::new_v4()) }
    pub fn into_inner(self) -> Uuid { self.0 }
}

impl std::fmt::Display for CalendarEventId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl std::str::FromStr for CalendarEventId {
    type Err = uuid::Error;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(Self(Uuid::parse_str(s)?))
    }
}

impl From<Uuid> for CalendarEventId {
    fn from(id: Uuid) -> Self { Self(id) }
}

impl From<CalendarEventId> for Uuid {
    fn from(id: CalendarEventId) -> Self { id.0 }
}

impl AsRef<Uuid> for CalendarEventId {
    fn as_ref(&self) -> &Uuid { &self.0 }
}

impl std::ops::Deref for CalendarEventId {
    type Target = Uuid;
    fn deref(&self) -> &Self::Target { &self.0 }
}

#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct CalendarEvent {
    pub id: Uuid,
    pub company_id: Uuid,
    pub series_id: Option<Uuid>,
    pub title: String,
    pub description: Option<String>,
    pub start_at: DateTime<Utc>,
    pub stop_at: DateTime<Utc>,
    pub privacy: EventPrivacy,
    pub organizer_user_id: Uuid,
    pub location: Option<String>,
    #[serde(default)]
    #[sqlx(json)]
    pub metadata: AuditMetadata,
}

impl CalendarEvent {
    /// Create a builder for CalendarEvent
    pub fn builder() -> CalendarEventBuilder {
        <CalendarEventBuilder as Default>::default()
    }

    /// Create a new CalendarEvent with required fields
    pub fn new(company_id: Uuid, title: String, start_at: DateTime<Utc>, stop_at: DateTime<Utc>, privacy: EventPrivacy, organizer_user_id: Uuid) -> Self {
        Self {
            id: Uuid::new_v4(),
            company_id,
            series_id: None,
            title,
            description: None,
            start_at,
            stop_at,
            privacy,
            organizer_user_id,
            location: None,
            metadata: AuditMetadata::default(),
        }
    }

    /// Get the entity's unique identifier
    pub fn id(&self) -> &Uuid {
        &self.id
    }

    /// Get a strongly-typed ID for this entity
    pub fn typed_id(&self) -> CalendarEventId {
        CalendarEventId(self.id)
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
    // Fluent Setters (with_* for optional fields)
    // ==========================================================

    /// Set the series_id field (chainable)
    pub fn with_series_id(mut self, value: Uuid) -> Self {
        self.series_id = Some(value);
        self
    }

    /// Set the description field (chainable)
    pub fn with_description(mut self, value: String) -> Self {
        self.description = Some(value);
        self
    }

    /// Set the location field (chainable)
    pub fn with_location(mut self, value: String) -> Self {
        self.location = Some(value);
        self
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
                "title" => {
                    if let Ok(v) = serde_json::from_value(value) { self.title = v; }
                }
                "description" => {
                    if let Ok(v) = serde_json::from_value(value) { self.description = v; }
                }
                "start_at" => {
                    if let Ok(v) = serde_json::from_value(value) { self.start_at = v; }
                }
                "stop_at" => {
                    if let Ok(v) = serde_json::from_value(value) { self.stop_at = v; }
                }
                "privacy" => {
                    if let Ok(v) = serde_json::from_value(value) { self.privacy = v; }
                }
                "organizer_user_id" => {
                    if let Ok(v) = serde_json::from_value(value) { self.organizer_user_id = v; }
                }
                "location" => {
                    if let Ok(v) = serde_json::from_value(value) { self.location = v; }
                }
                _ => {} // ignore unknown fields
            }
        }
    }

    // <<< CUSTOM METHODS START >>>
    // <<< CUSTOM METHODS END >>>
}

impl super::Entity for CalendarEvent {
    type Id = Uuid;

    fn entity_id(&self) -> &Self::Id {
        &self.id
    }

    fn entity_type() -> &'static str {
        "CalendarEvent"
    }
}

impl backbone_core::PersistentEntity for CalendarEvent {
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

impl backbone_orm::EntityRepoMeta for CalendarEvent {
    fn column_types() -> std::collections::HashMap<String, String> {
        let mut m = std::collections::HashMap::new();
        m.insert("id".to_string(), "uuid".to_string());
        m.insert("company_id".to_string(), "uuid".to_string());
        m.insert("series_id".to_string(), "uuid".to_string());
        m.insert("organizer_user_id".to_string(), "uuid".to_string());
        m.insert("privacy".to_string(), "event_privacy".to_string());
        m
    }
    fn search_fields() -> &'static [&'static str] {
        &["title"]
    }
    fn company_field() -> Option<&'static str> {
        Some("company_id")
    }
}

/// Builder for CalendarEvent entity
///
/// Provides a fluent API for constructing CalendarEvent instances.
/// System fields (id, metadata, timestamps) are auto-initialized.
#[derive(Debug, Clone, Default)]
pub struct CalendarEventBuilder {
    company_id: Option<Uuid>,
    series_id: Option<Uuid>,
    title: Option<String>,
    description: Option<String>,
    start_at: Option<DateTime<Utc>>,
    stop_at: Option<DateTime<Utc>>,
    privacy: Option<EventPrivacy>,
    organizer_user_id: Option<Uuid>,
    location: Option<String>,
}

impl CalendarEventBuilder {
    /// Set the company_id field (required)
    pub fn company_id(mut self, value: Uuid) -> Self {
        self.company_id = Some(value);
        self
    }

    /// Set the series_id field (optional)
    pub fn series_id(mut self, value: Uuid) -> Self {
        self.series_id = Some(value);
        self
    }

    /// Set the title field (required)
    pub fn title(mut self, value: String) -> Self {
        self.title = Some(value);
        self
    }

    /// Set the description field (optional)
    pub fn description(mut self, value: String) -> Self {
        self.description = Some(value);
        self
    }

    /// Set the start_at field (required)
    pub fn start_at(mut self, value: DateTime<Utc>) -> Self {
        self.start_at = Some(value);
        self
    }

    /// Set the stop_at field (required)
    pub fn stop_at(mut self, value: DateTime<Utc>) -> Self {
        self.stop_at = Some(value);
        self
    }

    /// Set the privacy field (default: `EventPrivacy::default()`)
    pub fn privacy(mut self, value: EventPrivacy) -> Self {
        self.privacy = Some(value);
        self
    }

    /// Set the organizer_user_id field (required)
    pub fn organizer_user_id(mut self, value: Uuid) -> Self {
        self.organizer_user_id = Some(value);
        self
    }

    /// Set the location field (optional)
    pub fn location(mut self, value: String) -> Self {
        self.location = Some(value);
        self
    }

    /// Build the CalendarEvent entity
    ///
    /// Returns Err if any required field without a default is missing.
    pub fn build(self) -> Result<CalendarEvent, String> {
        let company_id = self.company_id.ok_or_else(|| "company_id is required".to_string())?;
        let title = self.title.ok_or_else(|| "title is required".to_string())?;
        let start_at = self.start_at.ok_or_else(|| "start_at is required".to_string())?;
        let stop_at = self.stop_at.ok_or_else(|| "stop_at is required".to_string())?;
        let organizer_user_id = self.organizer_user_id.ok_or_else(|| "organizer_user_id is required".to_string())?;

        Ok(CalendarEvent {
            id: Uuid::new_v4(),
            company_id,
            series_id: self.series_id,
            title,
            description: self.description,
            start_at,
            stop_at,
            privacy: self.privacy.unwrap_or_default(),
            organizer_user_id,
            location: self.location,
            metadata: AuditMetadata::default(),
        })
    }
}
