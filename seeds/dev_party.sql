-- Local/dev only. NOT a migration -- never runs in production.
-- Loads one test invitation so you can exercise the magic-link flow:
--   visit /rsvp?code=SMITH-TEST  (or enter SMITH-TEST at the gate)
--
-- Apply with:
--   sqlite3 data/wedding.db < seeds/dev_party.sql
-- Re-runnable: it clears the test party first.

DELETE FROM rsvp_history WHERE guest_id IN (SELECT id FROM guests WHERE party_id = 'party-smith');
DELETE FROM guests WHERE party_id = 'party-smith';
DELETE FROM parties WHERE id = 'party-smith';

INSERT INTO parties (id, invite_code, label) VALUES
    ('party-smith', 'SMITH-TEST', 'The Smith Family');

INSERT INTO guests (id, party_id, first_name, last_name, email, is_plus_one) VALUES
    ('guest-sarah', 'party-smith', 'Sarah', 'Smith', 'sarah@example.com', 0),
    ('guest-tom',   'party-smith', 'Tom',   'Smith', NULL,                0);
