-- Down: drop calendar.calendar_religions table
DROP TABLE IF EXISTS calendar.calendar_religions CASCADE;
DROP FUNCTION IF EXISTS calendar.calendar_religions_audit_timestamp() CASCADE;
