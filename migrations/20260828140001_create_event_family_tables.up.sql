-- Event family tables: series, events, exception ledger, attendees
-- Hand-written in the 20260426220001 house shape (bare logical-FK columns +
-- indexes, per-table audit trigger functions), plus the integrity layer the
-- schema DSL cannot express (CHECK + partial unique indexes).

CREATE SCHEMA IF NOT EXISTS calendar;

-- ---------------------------------------------------------------------------
-- calendar.event_series
-- ---------------------------------------------------------------------------
CREATE TABLE IF NOT EXISTS calendar.event_series (
    id UUID NOT NULL DEFAULT gen_random_uuid(),
    company_id UUID NOT NULL,
    name TEXT,
    freq event_recurrence_freq NOT NULL,
    interval INT NOT NULL DEFAULT 1,
    by_weekday TEXT,
    by_monthday TEXT,
    until DATE,
    count INT,
    base_event_id UUID NOT NULL,
    metadata JSONB NOT NULL DEFAULT '{"created_at":null,"updated_at":null,"deleted_at":null,"created_by":null,"updated_by":null,"deleted_by":null}'::jsonb,
    PRIMARY KEY (id)
);

CREATE INDEX IF NOT EXISTS idx_event_series_company_id ON calendar.event_series (company_id);
CREATE INDEX IF NOT EXISTS idx_event_series_base_event_id ON calendar.event_series (base_event_id);

CREATE INDEX IF NOT EXISTS idx_event_series_metadata_gin ON calendar.event_series USING GIN (metadata);
CREATE INDEX IF NOT EXISTS idx_event_series_metadata_deleted_at ON calendar.event_series ((metadata->>'deleted_at'));
CREATE INDEX IF NOT EXISTS idx_event_series_metadata_created_at ON calendar.event_series ((metadata->>'created_at'));
CREATE INDEX IF NOT EXISTS idx_event_series_metadata_updated_at ON calendar.event_series ((metadata->>'updated_at'));

CREATE OR REPLACE FUNCTION calendar.event_series_audit_timestamp() RETURNS trigger AS $$
BEGIN
    IF TG_OP = 'INSERT' THEN
        NEW.metadata = jsonb_set(NEW.metadata::jsonb, '{created_at}', to_jsonb(NOW()));
        NEW.metadata = jsonb_set(NEW.metadata::jsonb, '{updated_at}', to_jsonb(NOW()));
    ELSIF TG_OP = 'UPDATE' THEN
        NEW.metadata = jsonb_set(NEW.metadata::jsonb, '{updated_at}', to_jsonb(NOW()));
    END IF;
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

DROP TRIGGER IF EXISTS event_series_insert_audit ON calendar.event_series;
CREATE TRIGGER event_series_insert_audit BEFORE INSERT ON calendar.event_series
    FOR EACH ROW EXECUTE FUNCTION calendar.event_series_audit_timestamp();

DROP TRIGGER IF EXISTS event_series_update_audit ON calendar.event_series;
CREATE TRIGGER event_series_update_audit BEFORE UPDATE ON calendar.event_series
    FOR EACH ROW EXECUTE FUNCTION calendar.event_series_audit_timestamp();

-- ---------------------------------------------------------------------------
-- calendar.events
-- ---------------------------------------------------------------------------
CREATE TABLE IF NOT EXISTS calendar.events (
    id UUID NOT NULL DEFAULT gen_random_uuid(),
    company_id UUID NOT NULL,
    series_id UUID,
    title TEXT NOT NULL,
    description TEXT,
    start_at TIMESTAMPTZ NOT NULL,
    stop_at TIMESTAMPTZ NOT NULL,
    privacy event_privacy NOT NULL DEFAULT 'public',
    organizer_user_id UUID NOT NULL,
    location TEXT,
    metadata JSONB NOT NULL DEFAULT '{"created_at":null,"updated_at":null,"deleted_at":null,"created_by":null,"updated_by":null,"deleted_by":null}'::jsonb,
    PRIMARY KEY (id)
);

CREATE INDEX IF NOT EXISTS idx_events_company_id_start_at ON calendar.events (company_id, start_at);
CREATE INDEX IF NOT EXISTS idx_events_series_id_start_at ON calendar.events (series_id, start_at);
CREATE INDEX IF NOT EXISTS idx_events_organizer_user_id ON calendar.events (organizer_user_id);

CREATE INDEX IF NOT EXISTS idx_events_metadata_gin ON calendar.events USING GIN (metadata);
CREATE INDEX IF NOT EXISTS idx_events_metadata_deleted_at ON calendar.events ((metadata->>'deleted_at'));
CREATE INDEX IF NOT EXISTS idx_events_metadata_created_at ON calendar.events ((metadata->>'created_at'));
CREATE INDEX IF NOT EXISTS idx_events_metadata_updated_at ON calendar.events ((metadata->>'updated_at'));

CREATE OR REPLACE FUNCTION calendar.events_audit_timestamp() RETURNS trigger AS $$
BEGIN
    IF TG_OP = 'INSERT' THEN
        NEW.metadata = jsonb_set(NEW.metadata::jsonb, '{created_at}', to_jsonb(NOW()));
        NEW.metadata = jsonb_set(NEW.metadata::jsonb, '{updated_at}', to_jsonb(NOW()));
    ELSIF TG_OP = 'UPDATE' THEN
        NEW.metadata = jsonb_set(NEW.metadata::jsonb, '{updated_at}', to_jsonb(NOW()));
    END IF;
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

DROP TRIGGER IF EXISTS events_insert_audit ON calendar.events;
CREATE TRIGGER events_insert_audit BEFORE INSERT ON calendar.events
    FOR EACH ROW EXECUTE FUNCTION calendar.events_audit_timestamp();

DROP TRIGGER IF EXISTS events_update_audit ON calendar.events;
CREATE TRIGGER events_update_audit BEFORE UPDATE ON calendar.events
    FOR EACH ROW EXECUTE FUNCTION calendar.events_audit_timestamp();

-- ---------------------------------------------------------------------------
-- calendar.event_exceptions (the split/cancel ledger)
-- ---------------------------------------------------------------------------
CREATE TABLE IF NOT EXISTS calendar.event_exceptions (
    id UUID NOT NULL DEFAULT gen_random_uuid(),
    company_id UUID NOT NULL,
    series_id UUID NOT NULL,
    event_id UUID NOT NULL,
    slot_start_at TIMESTAMPTZ NOT NULL,
    slot_stop_at TIMESTAMPTZ NOT NULL,
    kind event_exception_kind NOT NULL,
    metadata JSONB NOT NULL DEFAULT '{"created_at":null,"updated_at":null,"deleted_at":null,"created_by":null,"updated_by":null,"deleted_by":null}'::jsonb,
    PRIMARY KEY (id)
);

CREATE INDEX IF NOT EXISTS idx_event_exceptions_series_slot ON calendar.event_exceptions (series_id, slot_start_at, slot_stop_at);
CREATE INDEX IF NOT EXISTS idx_event_exceptions_event_id ON calendar.event_exceptions (event_id);

CREATE INDEX IF NOT EXISTS idx_event_exceptions_metadata_gin ON calendar.event_exceptions USING GIN (metadata);
CREATE INDEX IF NOT EXISTS idx_event_exceptions_metadata_deleted_at ON calendar.event_exceptions ((metadata->>'deleted_at'));
CREATE INDEX IF NOT EXISTS idx_event_exceptions_metadata_created_at ON calendar.event_exceptions ((metadata->>'created_at'));
CREATE INDEX IF NOT EXISTS idx_event_exceptions_metadata_updated_at ON calendar.event_exceptions ((metadata->>'updated_at'));

CREATE OR REPLACE FUNCTION calendar.event_exceptions_audit_timestamp() RETURNS trigger AS $$
BEGIN
    IF TG_OP = 'INSERT' THEN
        NEW.metadata = jsonb_set(NEW.metadata::jsonb, '{created_at}', to_jsonb(NOW()));
        NEW.metadata = jsonb_set(NEW.metadata::jsonb, '{updated_at}', to_jsonb(NOW()));
    ELSIF TG_OP = 'UPDATE' THEN
        NEW.metadata = jsonb_set(NEW.metadata::jsonb, '{updated_at}', to_jsonb(NOW()));
    END IF;
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

DROP TRIGGER IF EXISTS event_exceptions_insert_audit ON calendar.event_exceptions;
CREATE TRIGGER event_exceptions_insert_audit BEFORE INSERT ON calendar.event_exceptions
    FOR EACH ROW EXECUTE FUNCTION calendar.event_exceptions_audit_timestamp();

DROP TRIGGER IF EXISTS event_exceptions_update_audit ON calendar.event_exceptions;
CREATE TRIGGER event_exceptions_update_audit BEFORE UPDATE ON calendar.event_exceptions
    FOR EACH ROW EXECUTE FUNCTION calendar.event_exceptions_audit_timestamp();

-- ---------------------------------------------------------------------------
-- calendar.event_attendees
-- ---------------------------------------------------------------------------
CREATE TABLE IF NOT EXISTS calendar.event_attendees (
    id UUID NOT NULL DEFAULT gen_random_uuid(),
    company_id UUID NOT NULL,
    event_id UUID NOT NULL,
    user_id UUID NOT NULL,
    state event_attendee_state NOT NULL DEFAULT 'needs_action',
    access_token UUID NOT NULL DEFAULT gen_random_uuid(),
    metadata JSONB NOT NULL DEFAULT '{"created_at":null,"updated_at":null,"deleted_at":null,"created_by":null,"updated_by":null,"deleted_by":null}'::jsonb,
    PRIMARY KEY (id)
);

CREATE INDEX IF NOT EXISTS idx_event_attendees_event_id ON calendar.event_attendees (event_id);
CREATE INDEX IF NOT EXISTS idx_event_attendees_user_id ON calendar.event_attendees (user_id);

CREATE INDEX IF NOT EXISTS idx_event_attendees_metadata_gin ON calendar.event_attendees USING GIN (metadata);
CREATE INDEX IF NOT EXISTS idx_event_attendees_metadata_deleted_at ON calendar.event_attendees ((metadata->>'deleted_at'));
CREATE INDEX IF NOT EXISTS idx_event_attendees_metadata_created_at ON calendar.event_attendees ((metadata->>'created_at'));
CREATE INDEX IF NOT EXISTS idx_event_attendees_metadata_updated_at ON calendar.event_attendees ((metadata->>'updated_at'));

CREATE OR REPLACE FUNCTION calendar.event_attendees_audit_timestamp() RETURNS trigger AS $$
BEGIN
    IF TG_OP = 'INSERT' THEN
        NEW.metadata = jsonb_set(NEW.metadata::jsonb, '{created_at}', to_jsonb(NOW()));
        NEW.metadata = jsonb_set(NEW.metadata::jsonb, '{updated_at}', to_jsonb(NOW()));
    ELSIF TG_OP = 'UPDATE' THEN
        NEW.metadata = jsonb_set(NEW.metadata::jsonb, '{updated_at}', to_jsonb(NOW()));
    END IF;
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

DROP TRIGGER IF EXISTS event_attendees_insert_audit ON calendar.event_attendees;
CREATE TRIGGER event_attendees_insert_audit BEFORE INSERT ON calendar.event_attendees
    FOR EACH ROW EXECUTE FUNCTION calendar.event_attendees_audit_timestamp();

DROP TRIGGER IF EXISTS event_attendees_update_audit ON calendar.event_attendees;
CREATE TRIGGER event_attendees_update_audit BEFORE UPDATE ON calendar.event_attendees
    FOR EACH ROW EXECUTE FUNCTION calendar.event_attendees_audit_timestamp();

-- ---------------------------------------------------------------------------
-- Integrity layer (not expressible in the schema DSL)
-- ---------------------------------------------------------------------------

-- An event always stops after it starts (app-side AND DB-enforced).
ALTER TABLE calendar.events DROP CONSTRAINT IF EXISTS events_stop_after_start;
ALTER TABLE calendar.events ADD CONSTRAINT events_stop_after_start CHECK (stop_at > start_at);

-- (start, stop) identity as a DB-enforced invariant: no double materialization
-- of one series slot; a soft-deleted row frees its slot for re-materialization.
CREATE UNIQUE INDEX IF NOT EXISTS uq_calendar_events_series_slot
    ON calendar.events (series_id, start_at, stop_at)
    WHERE series_id IS NOT NULL AND (metadata->>'deleted_at') IS NULL;

-- Attendee dedup backstop: one LIVE attendee per (event, user). Partial so
-- soft-delete + re-add works. The application also dedups at write time with a
-- distinct conflict error; the DB constraint is the guarantee that survives
-- every write path.
CREATE UNIQUE INDEX IF NOT EXISTS uq_calendar_event_attendees_event_user
    ON calendar.event_attendees (event_id, user_id)
    WHERE (metadata->>'deleted_at') IS NULL;

-- One slot claim per series: the exception ledger keys reconciliation by
-- (series, slot) identity, so a slot can be claimed at most once while alive.
CREATE UNIQUE INDEX IF NOT EXISTS uq_calendar_event_exceptions_slot
    ON calendar.event_exceptions (series_id, slot_start_at, slot_stop_at)
    WHERE (metadata->>'deleted_at') IS NULL;

-- Invitation token seam (W7 /ics + answer flows): one token per attendee row,
-- forever — no partial filter, tokens never get reissued for a live row.
CREATE UNIQUE INDEX IF NOT EXISTS uq_calendar_event_attendees_token
    ON calendar.event_attendees (access_token);
