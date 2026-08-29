use chrono::{DateTime, Utc, NaiveDate};
use serde::{Deserialize, Serialize};
use sqlx::FromRow;
use uuid::Uuid;

use super::EventRecurrenceFreq;
use super::AuditMetadata;

/// Strongly-typed ID for CalendarEventSeries
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct CalendarEventSeriesId(pub Uuid);

impl CalendarEventSeriesId {
    pub fn new(id: Uuid) -> Self { Self(id) }
    pub fn generate() -> Self { Self(Uuid::new_v4()) }
    pub fn into_inner(self) -> Uuid { self.0 }
}

impl std::fmt::Display for CalendarEventSeriesId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl std::str::FromStr for CalendarEventSeriesId {
    type Err = uuid::Error;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(Self(Uuid::parse_str(s)?))
    }
}

impl From<Uuid> for CalendarEventSeriesId {
    fn from(id: Uuid) -> Self { Self(id) }
}

impl From<CalendarEventSeriesId> for Uuid {
    fn from(id: CalendarEventSeriesId) -> Self { id.0 }
}

impl AsRef<Uuid> for CalendarEventSeriesId {
    fn as_ref(&self) -> &Uuid { &self.0 }
}

impl std::ops::Deref for CalendarEventSeriesId {
    type Target = Uuid;
    fn deref(&self) -> &Self::Target { &self.0 }
}

#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct CalendarEventSeries {
    pub id: Uuid,
    pub company_id: Uuid,
    pub name: Option<String>,
    pub freq: EventRecurrenceFreq,
    pub interval: i32,
    pub by_weekday: Option<String>,
    pub by_monthday: Option<String>,
    pub until: Option<NaiveDate>,
    pub count: Option<i32>,
    pub base_event_id: Uuid,
    #[serde(default)]
    #[sqlx(json)]
    pub metadata: AuditMetadata,
}

impl CalendarEventSeries {
    /// Create a builder for CalendarEventSeries
    pub fn builder() -> CalendarEventSeriesBuilder {
        <CalendarEventSeriesBuilder as Default>::default()
    }

    /// Create a new CalendarEventSeries with required fields
    pub fn new(company_id: Uuid, freq: EventRecurrenceFreq, interval: i32, base_event_id: Uuid) -> Self {
        Self {
            id: Uuid::new_v4(),
            company_id,
            name: None,
            freq,
            interval,
            by_weekday: None,
            by_monthday: None,
            until: None,
            count: None,
            base_event_id,
            metadata: AuditMetadata::default(),
        }
    }

    /// Get the entity's unique identifier
    pub fn id(&self) -> &Uuid {
        &self.id
    }

    /// Get a strongly-typed ID for this entity
    pub fn typed_id(&self) -> CalendarEventSeriesId {
        CalendarEventSeriesId(self.id)
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

    /// Set the name field (chainable)
    pub fn with_name(mut self, value: String) -> Self {
        self.name = Some(value);
        self
    }

    /// Set the by_weekday field (chainable)
    pub fn with_by_weekday(mut self, value: String) -> Self {
        self.by_weekday = Some(value);
        self
    }

    /// Set the by_monthday field (chainable)
    pub fn with_by_monthday(mut self, value: String) -> Self {
        self.by_monthday = Some(value);
        self
    }

    /// Set the until field (chainable)
    pub fn with_until(mut self, value: NaiveDate) -> Self {
        self.until = Some(value);
        self
    }

    /// Set the count field (chainable)
    pub fn with_count(mut self, value: i32) -> Self {
        self.count = Some(value);
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
                "name" => {
                    if let Ok(v) = serde_json::from_value(value) { self.name = v; }
                }
                "freq" => {
                    if let Ok(v) = serde_json::from_value(value) { self.freq = v; }
                }
                "interval" => {
                    if let Ok(v) = serde_json::from_value(value) { self.interval = v; }
                }
                "by_weekday" => {
                    if let Ok(v) = serde_json::from_value(value) { self.by_weekday = v; }
                }
                "by_monthday" => {
                    if let Ok(v) = serde_json::from_value(value) { self.by_monthday = v; }
                }
                "until" => {
                    if let Ok(v) = serde_json::from_value(value) { self.until = v; }
                }
                "count" => {
                    if let Ok(v) = serde_json::from_value(value) { self.count = v; }
                }
                "base_event_id" => {
                    if let Ok(v) = serde_json::from_value(value) { self.base_event_id = v; }
                }
                _ => {} // ignore unknown fields
            }
        }
    }

    // <<< CUSTOM METHODS START >>>
    // <<< CUSTOM METHODS END >>>
}

impl super::Entity for CalendarEventSeries {
    type Id = Uuid;

    fn entity_id(&self) -> &Self::Id {
        &self.id
    }

    fn entity_type() -> &'static str {
        "CalendarEventSeries"
    }
}

impl backbone_core::PersistentEntity for CalendarEventSeries {
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

impl backbone_orm::EntityRepoMeta for CalendarEventSeries {
    fn column_types() -> std::collections::HashMap<String, String> {
        let mut m = std::collections::HashMap::new();
        m.insert("id".to_string(), "uuid".to_string());
        m.insert("company_id".to_string(), "uuid".to_string());
        m.insert("base_event_id".to_string(), "uuid".to_string());
        m.insert("freq".to_string(), "event_recurrence_freq".to_string());
        m
    }
    fn search_fields() -> &'static [&'static str] {
        &[]
    }
    fn company_field() -> Option<&'static str> {
        Some("company_id")
    }
}

/// Builder for CalendarEventSeries entity
///
/// Provides a fluent API for constructing CalendarEventSeries instances.
/// System fields (id, metadata, timestamps) are auto-initialized.
#[derive(Debug, Clone, Default)]
pub struct CalendarEventSeriesBuilder {
    company_id: Option<Uuid>,
    name: Option<String>,
    freq: Option<EventRecurrenceFreq>,
    interval: Option<i32>,
    by_weekday: Option<String>,
    by_monthday: Option<String>,
    until: Option<NaiveDate>,
    count: Option<i32>,
    base_event_id: Option<Uuid>,
}

impl CalendarEventSeriesBuilder {
    /// Set the company_id field (required)
    pub fn company_id(mut self, value: Uuid) -> Self {
        self.company_id = Some(value);
        self
    }

    /// Set the name field (optional)
    pub fn name(mut self, value: String) -> Self {
        self.name = Some(value);
        self
    }

    /// Set the freq field (required)
    pub fn freq(mut self, value: EventRecurrenceFreq) -> Self {
        self.freq = Some(value);
        self
    }

    /// Set the interval field (default: `1`)
    pub fn interval(mut self, value: i32) -> Self {
        self.interval = Some(value);
        self
    }

    /// Set the by_weekday field (optional)
    pub fn by_weekday(mut self, value: String) -> Self {
        self.by_weekday = Some(value);
        self
    }

    /// Set the by_monthday field (optional)
    pub fn by_monthday(mut self, value: String) -> Self {
        self.by_monthday = Some(value);
        self
    }

    /// Set the until field (optional)
    pub fn until(mut self, value: NaiveDate) -> Self {
        self.until = Some(value);
        self
    }

    /// Set the count field (optional)
    pub fn count(mut self, value: i32) -> Self {
        self.count = Some(value);
        self
    }

    /// Set the base_event_id field (required)
    pub fn base_event_id(mut self, value: Uuid) -> Self {
        self.base_event_id = Some(value);
        self
    }

    /// Build the CalendarEventSeries entity
    ///
    /// Returns Err if any required field without a default is missing.
    pub fn build(self) -> Result<CalendarEventSeries, String> {
        let company_id = self.company_id.ok_or_else(|| "company_id is required".to_string())?;
        let freq = self.freq.ok_or_else(|| "freq is required".to_string())?;
        let base_event_id = self.base_event_id.ok_or_else(|| "base_event_id is required".to_string())?;

        Ok(CalendarEventSeries {
            id: Uuid::new_v4(),
            company_id,
            name: self.name,
            freq,
            interval: self.interval.unwrap_or(1),
            by_weekday: self.by_weekday,
            by_monthday: self.by_monthday,
            until: self.until,
            count: self.count,
            base_event_id,
            metadata: AuditMetadata::default(),
        })
    }
}
