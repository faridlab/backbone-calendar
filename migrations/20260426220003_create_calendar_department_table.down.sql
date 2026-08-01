-- Down: drop calendar.calendar_departments table
DROP TABLE IF EXISTS calendar.calendar_departments CASCADE;
DROP FUNCTION IF EXISTS calendar.calendar_departments_audit_timestamp() CASCADE;
