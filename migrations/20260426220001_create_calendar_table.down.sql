-- Down: drop calendar.calendars table
DROP TABLE IF EXISTS calendar.calendars CASCADE;
DROP FUNCTION IF EXISTS calendar.calendars_audit_timestamp() CASCADE;
