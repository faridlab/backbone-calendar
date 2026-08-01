-- Down: drop calendar.calendar_employee_statuses table
DROP TABLE IF EXISTS calendar.calendar_employee_statuses CASCADE;
DROP FUNCTION IF EXISTS calendar.calendar_employee_statuses_audit_timestamp() CASCADE;
