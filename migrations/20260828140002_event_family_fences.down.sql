-- Down: drop the privacy read fence first, then the company fences.
DROP POLICY IF EXISTS calendar_events_privacy_read ON calendar.events;

DROP POLICY IF EXISTS event_attendees_company_isolation ON calendar.event_attendees;
DROP POLICY IF EXISTS event_exceptions_company_isolation ON calendar.event_exceptions;
DROP POLICY IF EXISTS events_company_isolation ON calendar.events;
DROP POLICY IF EXISTS event_series_company_isolation ON calendar.event_series;
