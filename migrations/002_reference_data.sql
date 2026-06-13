-- Reference data: the weekend's events and the meal options.
-- These ship to every environment as sensible defaults; the /admin UI edits
-- them later. (No parties/guests here -- those are real PII loaded via admin,
-- and a test invite must never end up in production. See seeds/dev_party.sql
-- for local testing.)

INSERT INTO events (id, name, starts_at, location, serves_meal, sort_order) VALUES
    ('evt-welcome',   'Welcome Drinks',  '2026-09-18T18:00:00', 'Schrute Farms', 0, 1),
    ('evt-ceremony',  'Ceremony',        '2026-09-19T16:00:00', 'Schrute Farms', 0, 2),
    ('evt-reception', 'Reception',       '2026-09-19T18:00:00', 'Schrute Farms', 1, 3),
    ('evt-brunch',    'Farewell Brunch', '2026-09-20T10:00:00', 'Schrute Farms', 0, 4);

INSERT INTO meal_options (id, label, is_active, sort_order) VALUES
    ('meal-filet',  'Filet Mignon', 1, 1),
    ('meal-salmon', 'Salmon',       1, 2),
    ('meal-veg',    'Vegetarian',   1, 3),
    ('meal-kids',   'Kids Meal',    1, 4);
