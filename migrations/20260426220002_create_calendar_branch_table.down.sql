-- Down: drop calendar.calendar_branches table
DROP TABLE IF EXISTS calendar.calendar_branches CASCADE;
DROP FUNCTION IF EXISTS calendar.calendar_branches_audit_timestamp() CASCADE;
