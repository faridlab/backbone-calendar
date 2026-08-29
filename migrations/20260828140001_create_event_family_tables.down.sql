-- Down: drop event family tables (indexes, constraints, and trigger
-- functions ride the tables / schema).

DROP TABLE IF EXISTS calendar.event_attendees CASCADE;
DROP FUNCTION IF EXISTS calendar.event_attendees_audit_timestamp() CASCADE;

DROP TABLE IF EXISTS calendar.event_exceptions CASCADE;
DROP FUNCTION IF EXISTS calendar.event_exceptions_audit_timestamp() CASCADE;

DROP TABLE IF EXISTS calendar.events CASCADE;
DROP FUNCTION IF EXISTS calendar.events_audit_timestamp() CASCADE;

DROP TABLE IF EXISTS calendar.event_series CASCADE;
DROP FUNCTION IF EXISTS calendar.event_series_audit_timestamp() CASCADE;
