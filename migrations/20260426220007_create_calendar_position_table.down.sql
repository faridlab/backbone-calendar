-- Down: drop calendar.calendar_positions table
DROP TABLE IF EXISTS calendar.calendar_positions CASCADE;
DROP FUNCTION IF EXISTS calendar.calendar_positions_audit_timestamp() CASCADE;
