-- Down: remove the company RLS fence for calendar module

-- Reverse the company RLS fence for calendar.calendars
DROP POLICY IF EXISTS calendars_company_isolation ON calendar.calendars;
ALTER TABLE calendar.calendars NO FORCE ROW LEVEL SECURITY;
ALTER TABLE calendar.calendars DISABLE ROW LEVEL SECURITY;

-- Reverse the company RLS fence for calendar.calendar_branches
DROP POLICY IF EXISTS calendar_branches_company_isolation ON calendar.calendar_branches;
ALTER TABLE calendar.calendar_branches NO FORCE ROW LEVEL SECURITY;
ALTER TABLE calendar.calendar_branches DISABLE ROW LEVEL SECURITY;

