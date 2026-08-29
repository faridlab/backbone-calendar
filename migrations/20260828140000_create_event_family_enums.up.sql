-- Event family enum types for calendar module
-- Unqualified, matching the module's existing enum convention (employment_status
-- in 20260426220000) and the sqlx entity derives, which declare bare type names
-- (`#[sqlx(type_name = "event_privacy")]`) — a schema-qualified CREATE TYPE makes
-- row decoding fail with a type-name mismatch.

-- Recurrence frequency for calendar.event_series
DO $$
BEGIN
    IF NOT EXISTS (SELECT 1 FROM pg_type WHERE typname = 'event_recurrence_freq') THEN
        CREATE TYPE event_recurrence_freq AS ENUM ('daily', 'weekly', 'monthly', 'yearly');
    END IF;
END
$$;

-- Visibility fence for calendar.events
DO $$
BEGIN
    IF NOT EXISTS (SELECT 1 FROM pg_type WHERE typname = 'event_privacy') THEN
        CREATE TYPE event_privacy AS ENUM ('public', 'private', 'confidential');
    END IF;
END
$$;

-- Response state for calendar.event_attendees
DO $$
BEGIN
    IF NOT EXISTS (SELECT 1 FROM pg_type WHERE typname = 'event_attendee_state') THEN
        CREATE TYPE event_attendee_state AS ENUM ('needs_action', 'accepted', 'declined', 'tentative');
    END IF;
END
$$;

-- Slot-claim kind for calendar.event_exceptions
DO $$
BEGIN
    IF NOT EXISTS (SELECT 1 FROM pg_type WHERE typname = 'event_exception_kind') THEN
        CREATE TYPE event_exception_kind AS ENUM ('edited', 'cancelled');
    END IF;
END
$$;
