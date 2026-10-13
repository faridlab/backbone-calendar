-- Down: drop deferred foreign keys for calendar module
ALTER TABLE calendar.event_series DROP CONSTRAINT IF EXISTS fk_event_series_base_event_id;
