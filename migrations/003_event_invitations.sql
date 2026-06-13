-- Per-guest event invitations. A row means "this guest is invited to this
-- event". The RSVP page shows a guest only the events they're invited to, and
-- submissions are accepted only for invited (guest, event) pairs.
--
-- No backfill: the app hadn't launched when this shipped, so there's no live
-- guest data to preserve. New guests created via the admin UI or CSV import
-- default to "invited to all events"; the CSV gains one column per event so the
-- set can be edited in a spreadsheet, and the per-party edit page has a matrix.
CREATE TABLE IF NOT EXISTS event_invitations (
    guest_id TEXT NOT NULL REFERENCES guests(id),
    event_id TEXT NOT NULL REFERENCES events(id),
    PRIMARY KEY (guest_id, event_id)
);
CREATE INDEX IF NOT EXISTS idx_event_inv_event ON event_invitations(event_id);
