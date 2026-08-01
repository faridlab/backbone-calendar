-- Down: drop calendar.calendar_levels table
DROP TABLE IF EXISTS calendar.calendar_levels CASCADE;
DROP FUNCTION IF EXISTS calendar.calendar_levels_audit_timestamp() CASCADE;
