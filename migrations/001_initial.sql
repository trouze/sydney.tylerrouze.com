-- Wedding RSVP schema.
-- Philosophy: plain entity tables + an append-only RSVP log. No UPDATEs to
-- history; "current" state is always the latest row per (guest, event).
-- SQLite enforces FKs only when `PRAGMA foreign_keys = ON` is set per connection
-- (do this in the app on connect).

-- An invitation: the unit you mail / hand a magic link to. A household or couple.
CREATE TABLE IF NOT EXISTS parties (
    id          TEXT PRIMARY KEY,
    invite_code TEXT NOT NULL UNIQUE,           -- magic-link / login token like SMITH-7Q2
    label       TEXT NOT NULL,                  -- "The Smith Family"
    created_at  TEXT NOT NULL DEFAULT (datetime('now'))
);

-- A person within a party. Admin loads these (or bulk-imports) in /admin.
CREATE TABLE IF NOT EXISTS guests (
    id          TEXT PRIMARY KEY,
    party_id    TEXT NOT NULL REFERENCES parties(id),
    first_name  TEXT NOT NULL,
    last_name   TEXT NOT NULL,
    email       TEXT,                           -- nullable: a +1 may have none
    phone       TEXT,                           -- nullable: digits only e.g. 5551234567
    is_plus_one INTEGER NOT NULL DEFAULT 0,
    created_at  TEXT NOT NULL DEFAULT (datetime('now'))
);
CREATE INDEX IF NOT EXISTS idx_guests_party ON guests(party_id);

-- Weekend events (welcome drinks, ceremony, reception, brunch). Admin-managed.
CREATE TABLE IF NOT EXISTS events (
    id          TEXT PRIMARY KEY,
    name        TEXT NOT NULL,                  -- "Welcome Drinks", "Reception"
    starts_at   TEXT,                           -- ISO8601
    location    TEXT,
    serves_meal INTEGER NOT NULL DEFAULT 0,     -- whether meal_option applies here
    sort_order  INTEGER NOT NULL DEFAULT 0,
    created_at  TEXT NOT NULL DEFAULT (datetime('now'))
);

-- Admin-configured meal choices, managed in /admin. Soft-disable via is_active
-- so historical RSVPs that referenced a now-retired option still resolve.
CREATE TABLE IF NOT EXISTS meal_options (
    id         TEXT PRIMARY KEY,
    label      TEXT NOT NULL,                   -- "Filet", "Salmon", "Vegetarian"
    is_active  INTEGER NOT NULL DEFAULT 1,
    sort_order INTEGER NOT NULL DEFAULT 0
);

-- Append-only RSVP log. Grain = one row per guest x event per edit.
-- Never UPDATE; only INSERT. Full audit history falls out for free, and any
-- party member can edit (recorded in submitted_by). Enforce the cutoff date in
-- the app by rejecting inserts past it -- closed responses stay intact.
CREATE TABLE IF NOT EXISTS rsvp_history (
    id             TEXT PRIMARY KEY,
    guest_id       TEXT NOT NULL REFERENCES guests(id),       -- whose RSVP
    event_id       TEXT NOT NULL REFERENCES events(id),       -- which event
    submitted_by   TEXT REFERENCES guests(id),                -- who edited; NULL = party-login, individual unknown
    attending      INTEGER,                                   -- NULL = pending, 0 = no, 1 = yes
    meal_option_id TEXT REFERENCES meal_options(id),          -- only for serves_meal events
    dietary_notes  TEXT,
    message        TEXT,
    response_ts    TEXT NOT NULL DEFAULT (datetime('now'))
);
CREATE INDEX IF NOT EXISTS idx_rsvp_guest_event_ts
    ON rsvp_history(guest_id, event_id, response_ts DESC);

-- Current RSVP per guest per event = newest row in the log.
-- rowid tiebreaks edits made within the same second.
CREATE VIEW IF NOT EXISTS current_rsvp AS
SELECT h.*
FROM rsvp_history h
WHERE h.rowid = (
    SELECT h2.rowid
    FROM rsvp_history h2
    WHERE h2.guest_id = h.guest_id
      AND h2.event_id = h.event_id
    ORDER BY h2.response_ts DESC, h2.rowid DESC
    LIMIT 1
);
