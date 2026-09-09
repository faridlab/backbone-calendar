-- Hand-authored (user-owned). Not regenerated.
--
-- Best-effort restore sketch for the tenancy strip (ADR-0029). This is a breaking module
-- release against dev-stage databases: the down re-adds the company_id column as nullable
-- with its plain index and the company isolation policy shape, but restores NO data —
-- rows written after the strip (or after the decorator re-keyed them) carry org_unit_id
-- only. The composing service's tenancy decorator remains the live fence; treat this
-- down as a schema-shape sketch for archaeology, not a usable rollback.

ALTER TABLE calendar.calendars         ADD COLUMN IF NOT EXISTS company_id uuid;
ALTER TABLE calendar.calendar_branches ADD COLUMN IF NOT EXISTS company_id uuid;
ALTER TABLE calendar.events            ADD COLUMN IF NOT EXISTS company_id uuid;
ALTER TABLE calendar.event_series      ADD COLUMN IF NOT EXISTS company_id uuid;
ALTER TABLE calendar.event_exceptions  ADD COLUMN IF NOT EXISTS company_id uuid;
ALTER TABLE calendar.event_attendees   ADD COLUMN IF NOT EXISTS company_id uuid;

CREATE INDEX IF NOT EXISTS idx_calendars_company_id_date_start ON calendar.calendars (company_id, date_start);
CREATE INDEX IF NOT EXISTS idx_events_company_id_start_at      ON calendar.events (company_id, start_at);
CREATE INDEX IF NOT EXISTS idx_event_series_company_id         ON calendar.event_series (company_id);

CREATE POLICY calendars_company_isolation ON calendar.calendars
    FOR ALL
    USING      (company_id = NULLIF(current_setting('app.company_id', true), '')::uuid)
    WITH CHECK (company_id = NULLIF(current_setting('app.company_id', true), '')::uuid);
CREATE POLICY calendar_branches_company_isolation ON calendar.calendar_branches
    FOR ALL
    USING      (company_id = NULLIF(current_setting('app.company_id', true), '')::uuid)
    WITH CHECK (company_id = NULLIF(current_setting('app.company_id', true), '')::uuid);
CREATE POLICY events_company_isolation ON calendar.events
    FOR ALL
    USING      (company_id = NULLIF(current_setting('app.company_id', true), '')::uuid)
    WITH CHECK (company_id = NULLIF(current_setting('app.company_id', true), '')::uuid);
CREATE POLICY event_series_company_isolation ON calendar.event_series
    FOR ALL
    USING      (company_id = NULLIF(current_setting('app.company_id', true), '')::uuid)
    WITH CHECK (company_id = NULLIF(current_setting('app.company_id', true), '')::uuid);
CREATE POLICY event_exceptions_company_isolation ON calendar.event_exceptions
    FOR ALL
    USING      (company_id = NULLIF(current_setting('app.company_id', true), '')::uuid)
    WITH CHECK (company_id = NULLIF(current_setting('app.company_id', true), '')::uuid);
CREATE POLICY event_attendees_company_isolation ON calendar.event_attendees
    FOR ALL
    USING      (company_id = NULLIF(current_setting('app.company_id', true), '')::uuid)
    WITH CHECK (company_id = NULLIF(current_setting('app.company_id', true), '')::uuid);
