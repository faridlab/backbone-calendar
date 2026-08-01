-- Down: drop calendar.calendar_employees table
DROP TABLE IF EXISTS calendar.calendar_employees CASCADE;
DROP FUNCTION IF EXISTS calendar.calendar_employees_audit_timestamp() CASCADE;
