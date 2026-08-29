use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::FromRow;
use uuid::Uuid;

use super::EventAttendeeState;
use super::AuditMetadata;

/// Strongly-typed ID for CalendarEventAttendee
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct CalendarEventAttendeeId(pub Uuid);

impl CalendarEventAttendeeId {
    pub fn new(id: Uuid) -> Self { Self(id) }
    pub fn generate() -> Self { Self(Uuid::new_v4()) }
    pub fn into_inner(self) -> Uuid { self.0 }
}

impl std::fmt::Display for CalendarEventAttendeeId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl std::str::FromStr for CalendarEventAttendeeId {
    type Err = uuid::Error;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(Self(Uuid::parse_str(s)?))
    }
}

impl From<Uuid> for CalendarEventAttendeeId {
    fn from(id: Uuid) -> Self { Self(id) }
}

impl From<CalendarEventAttendeeId> for Uuid {
    fn from(id: CalendarEventAttendeeId) -> Self { id.0 }
}

impl AsRef<Uuid> for CalendarEventAttendeeId {
    fn as_ref(&self) -> &Uuid { &self.0 }
}

impl std::ops::Deref for CalendarEventAttendeeId {
    type Target = Uuid;
    fn deref(&self) -> &Self::Target { &self.0 }
}

#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct CalendarEventAttendee {
    pub id: Uuid,
    pub company_id: Uuid,
    pub event_id: Uuid,
    pub user_id: Uuid,
    pub state: EventAttendeeState,
    pub access_token: Uuid,
    #[serde(default)]
    #[sqlx(json)]
    pub metadata: AuditMetadata,
}

impl CalendarEventAttendee {
    /// Create a builder for CalendarEventAttendee
    pub fn builder() -> CalendarEventAttendeeBuilder {
        <CalendarEventAttendeeBuilder as Default>::default()
    }

    /// Create a new CalendarEventAttendee with required fields
    pub fn new(company_id: Uuid, event_id: Uuid, user_id: Uuid, state: EventAttendeeState, access_token: Uuid) -> Self {
        Self {
            id: Uuid::new_v4(),
            company_id,
            event_id,
            user_id,
            state,
            access_token,
            metadata: AuditMetadata::default(),
        }
    }

    /// Get the entity's unique identifier
    pub fn id(&self) -> &Uuid {
        &self.id
    }

    /// Get a strongly-typed ID for this entity
    pub fn typed_id(&self) -> CalendarEventAttendeeId {
        CalendarEventAttendeeId(self.id)
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
                "event_id" => {
                    if let Ok(v) = serde_json::from_value(value) { self.event_id = v; }
                }
                "user_id" => {
                    if let Ok(v) = serde_json::from_value(value) { self.user_id = v; }
                }
                "state" => {
                    if let Ok(v) = serde_json::from_value(value) { self.state = v; }
                }
                "access_token" => {
                    if let Ok(v) = serde_json::from_value(value) { self.access_token = v; }
                }
                _ => {} // ignore unknown fields
            }
        }
    }

    // <<< CUSTOM METHODS START >>>
    // <<< CUSTOM METHODS END >>>
}

impl super::Entity for CalendarEventAttendee {
    type Id = Uuid;

    fn entity_id(&self) -> &Self::Id {
        &self.id
    }

    fn entity_type() -> &'static str {
        "CalendarEventAttendee"
    }
}

impl backbone_core::PersistentEntity for CalendarEventAttendee {
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

impl backbone_orm::EntityRepoMeta for CalendarEventAttendee {
    fn column_types() -> std::collections::HashMap<String, String> {
        let mut m = std::collections::HashMap::new();
        m.insert("id".to_string(), "uuid".to_string());
        m.insert("company_id".to_string(), "uuid".to_string());
        m.insert("event_id".to_string(), "uuid".to_string());
        m.insert("user_id".to_string(), "uuid".to_string());
        m.insert("state".to_string(), "event_attendee_state".to_string());
        m
    }
    fn search_fields() -> &'static [&'static str] {
        &[]
    }
    fn company_field() -> Option<&'static str> {
        Some("company_id")
    }
}

/// Builder for CalendarEventAttendee entity
///
/// Provides a fluent API for constructing CalendarEventAttendee instances.
/// System fields (id, metadata, timestamps) are auto-initialized.
#[derive(Debug, Clone, Default)]
pub struct CalendarEventAttendeeBuilder {
    company_id: Option<Uuid>,
    event_id: Option<Uuid>,
    user_id: Option<Uuid>,
    state: Option<EventAttendeeState>,
    access_token: Option<Uuid>,
}

impl CalendarEventAttendeeBuilder {
    /// Set the company_id field (required)
    pub fn company_id(mut self, value: Uuid) -> Self {
        self.company_id = Some(value);
        self
    }

    /// Set the event_id field (required)
    pub fn event_id(mut self, value: Uuid) -> Self {
        self.event_id = Some(value);
        self
    }

    /// Set the user_id field (required)
    pub fn user_id(mut self, value: Uuid) -> Self {
        self.user_id = Some(value);
        self
    }

    /// Set the state field (default: `EventAttendeeState::default()`)
    pub fn state(mut self, value: EventAttendeeState) -> Self {
        self.state = Some(value);
        self
    }

    /// Set the access_token field (default: `Uuid::new_v4()`)
    pub fn access_token(mut self, value: Uuid) -> Self {
        self.access_token = Some(value);
        self
    }

    /// Build the CalendarEventAttendee entity
    ///
    /// Returns Err if any required field without a default is missing.
    pub fn build(self) -> Result<CalendarEventAttendee, String> {
        let company_id = self.company_id.ok_or_else(|| "company_id is required".to_string())?;
        let event_id = self.event_id.ok_or_else(|| "event_id is required".to_string())?;
        let user_id = self.user_id.ok_or_else(|| "user_id is required".to_string())?;

        Ok(CalendarEventAttendee {
            id: Uuid::new_v4(),
            company_id,
            event_id,
            user_id,
            state: self.state.unwrap_or_default(),
            access_token: self.access_token.unwrap_or(Uuid::new_v4()),
            metadata: AuditMetadata::default(),
        })
    }
}
