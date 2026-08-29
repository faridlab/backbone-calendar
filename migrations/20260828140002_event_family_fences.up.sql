-- Event family fences: company isolation (strict) + event privacy read fence.
--
-- Company fence: the 20260816130000 pattern on each of the four event-family
-- tables — a session sees only rows whose company_id equals the request-scoped
-- company (`set_config('app.company_id', <uuid>, true)`); an unset var sees
-- zero rows (fail-closed).
--
-- Privacy read fence: a RESTRICTIVE SELECT policy on calendar.events. The
-- permissive company policy ORs; a restrictive policy ANDs — so a read must
-- satisfy BOTH company-match AND privacy-pass, while writes stay governed by
-- the company policy alone. An unset `app.user_id` passes the restrictive
-- policy only for public rows: jobs that need private reads must pin an
-- acting user. Both `private` AND `confidential` are row-invisible to
-- non-participants this wave (display-layer "Busy" masking is a webapp concern
-- deferred by declaration; the DB fence errs invisible).

-- Migration: company row-level-security fence for calendar.event_series
ALTER TABLE calendar.event_series ENABLE ROW LEVEL SECURITY;
ALTER TABLE calendar.event_series FORCE  ROW LEVEL SECURITY;
DROP POLICY IF EXISTS event_series_company_isolation ON calendar.event_series;
CREATE POLICY event_series_company_isolation ON calendar.event_series
    FOR ALL
    USING      (company_id = NULLIF(current_setting('app.company_id', true), '')::uuid)
    WITH CHECK (company_id = NULLIF(current_setting('app.company_id', true), '')::uuid);

-- Migration: company row-level-security fence for calendar.events
ALTER TABLE calendar.events ENABLE ROW LEVEL SECURITY;
ALTER TABLE calendar.events FORCE  ROW LEVEL SECURITY;
DROP POLICY IF EXISTS events_company_isolation ON calendar.events;
CREATE POLICY events_company_isolation ON calendar.events
    FOR ALL
    USING      (company_id = NULLIF(current_setting('app.company_id', true), '')::uuid)
    WITH CHECK (company_id = NULLIF(current_setting('app.company_id', true), '')::uuid);

-- Migration: company row-level-security fence for calendar.event_exceptions
ALTER TABLE calendar.event_exceptions ENABLE ROW LEVEL SECURITY;
ALTER TABLE calendar.event_exceptions FORCE  ROW LEVEL SECURITY;
DROP POLICY IF EXISTS event_exceptions_company_isolation ON calendar.event_exceptions;
CREATE POLICY event_exceptions_company_isolation ON calendar.event_exceptions
    FOR ALL
    USING      (company_id = NULLIF(current_setting('app.company_id', true), '')::uuid)
    WITH CHECK (company_id = NULLIF(current_setting('app.company_id', true), '')::uuid);

-- Migration: company row-level-security fence for calendar.event_attendees
ALTER TABLE calendar.event_attendees ENABLE ROW LEVEL SECURITY;
ALTER TABLE calendar.event_attendees FORCE  ROW LEVEL SECURITY;
DROP POLICY IF EXISTS event_attendees_company_isolation ON calendar.event_attendees;
CREATE POLICY event_attendees_company_isolation ON calendar.event_attendees
    FOR ALL
    USING      (company_id = NULLIF(current_setting('app.company_id', true), '')::uuid)
    WITH CHECK (company_id = NULLIF(current_setting('app.company_id', true), '')::uuid);

-- Event privacy read fence: public, or organizer, or a live attendee.
DROP POLICY IF EXISTS calendar_events_privacy_read ON calendar.events;
CREATE POLICY calendar_events_privacy_read ON calendar.events
    AS RESTRICTIVE FOR SELECT USING (
        privacy = 'public'
        OR organizer_user_id = NULLIF(current_setting('app.user_id', true), '')::uuid
        OR EXISTS (SELECT 1 FROM calendar.event_attendees a
                   WHERE a.event_id = events.id
                     AND a.user_id = NULLIF(current_setting('app.user_id', true), '')::uuid
                     AND (a.metadata->>'deleted_at') IS NULL));
