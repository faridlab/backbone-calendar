-- Hand-authored (user-owned). Not regenerated.
--
-- Strip every company-fence artifact from the calendar tables (ADR-0029): the module is
-- tenant-agnostic; org scoping is installed by the COMPOSING service's tenancy decorator,
-- never by the module. Dropped here, per table: the company-leading indexes, the
-- <table>_company_isolation RLS policy, and the company_id column itself.
--
-- Tables: calendars, calendar_branches, and the four event-family tables
-- (events, event_series, event_exceptions, event_attendees).
--
-- Ordering guard (the decorator must run FIRST on any database with data): the module
-- never moves tenancy data. A table is safe to strip when EITHER
--   a) it carries org_unit_id with no NULLs — the decorator backfilled it from company_id —
--      or b) it is empty (a fresh database: the earlier chain files created it empty).
-- Otherwise the strip RAISEs, naming the decorator step, rather than dropping a column
-- that still holds the only tenancy key. The file is re-runnable (every drop is IF EXISTS
-- and the tracker has no checksums), so a failed run retries cleanly after the decorator
-- lands.
--
-- RLS enable/force flags are deliberately NOT touched: the decorator owns those now.
-- The event privacy read fence (calendar_events_privacy_read, a RESTRICTIVE policy on
-- calendar.events) is a DOMAIN read fence — privacy, not tenancy — and stays.
-- The event-family domain partial uniques are tenant-free and untouched.

DO $$
DECLARE
    t text;
    has_org boolean;
    org_nulls bigint;
    total bigint;
    offenders text := '';
BEGIN
    FOREACH t IN ARRAY ARRAY['calendars', 'calendar_branches', 'events', 'event_series', 'event_exceptions', 'event_attendees']
    LOOP
        IF to_regclass(format('calendar.%I', t)) IS NULL THEN
            CONTINUE; -- chain not fully applied on this database; nothing to strip
        END IF;

        SELECT EXISTS (
                   SELECT 1 FROM information_schema.columns
                   WHERE table_schema = 'calendar' AND table_name = t AND column_name = 'org_unit_id'
               )
        INTO has_org;

        EXECUTE format('SELECT count(*) FROM calendar.%I', t) INTO total;

        IF has_org THEN
            EXECUTE format(
                'SELECT count(*) FROM calendar.%I WHERE org_unit_id IS NULL', t)
            INTO org_nulls;
        ELSE
            org_nulls := total; -- no org column: every row's only tenancy key is company_id
        END IF;

        IF has_org AND org_nulls = 0 THEN
            CONTINUE; -- decorator backfilled: safe
        END IF;
        IF total = 0 THEN
            CONTINUE; -- empty table (fresh database): safe
        END IF;
        offenders := offenders || format(' calendar.%s (%s rows, %s rows not covered by org_unit_id);', t, total, org_nulls);
    END LOOP;

    IF offenders <> '' THEN
        RAISE EXCEPTION 'refusing to strip company_id — these tables are not yet covered by the tenancy decorator:%. Apply the composing service''s tenancy decorator (it backfills org_unit_id from company_id) and re-run; it is the only step that moves tenancy data.', offenders;
    END IF;
END $$;

-- ── calendars ─────────────────────────────────────────────────────────────────
DROP INDEX IF EXISTS calendar.idx_calendars_company_id_date_start;
DROP POLICY IF EXISTS calendars_company_isolation ON calendar.calendars;
ALTER TABLE calendar.calendars DROP COLUMN IF EXISTS company_id;

-- ── calendar_branches ─────────────────────────────────────────────────────────
DROP POLICY IF EXISTS calendar_branches_company_isolation ON calendar.calendar_branches;
ALTER TABLE calendar.calendar_branches DROP COLUMN IF EXISTS company_id;

-- ── events ────────────────────────────────────────────────────────────────────
DROP INDEX IF EXISTS calendar.idx_events_company_id_start_at;
DROP POLICY IF EXISTS events_company_isolation ON calendar.events;
ALTER TABLE calendar.events DROP COLUMN IF EXISTS company_id;

-- ── event_series ──────────────────────────────────────────────────────────────
DROP INDEX IF EXISTS calendar.idx_event_series_company_id;
DROP POLICY IF EXISTS event_series_company_isolation ON calendar.event_series;
ALTER TABLE calendar.event_series DROP COLUMN IF EXISTS company_id;

-- ── event_exceptions ──────────────────────────────────────────────────────────
DROP POLICY IF EXISTS event_exceptions_company_isolation ON calendar.event_exceptions;
ALTER TABLE calendar.event_exceptions DROP COLUMN IF EXISTS company_id;

-- ── event_attendees ───────────────────────────────────────────────────────────
DROP POLICY IF EXISTS event_attendees_company_isolation ON calendar.event_attendees;
ALTER TABLE calendar.event_attendees DROP COLUMN IF EXISTS company_id;
