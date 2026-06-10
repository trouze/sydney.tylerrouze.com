CREATE TABLE IF NOT EXISTS rsvps (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL,
    email TEXT NOT NULL,
    attending INTEGER NOT NULL DEFAULT 1,
    guest_count INTEGER NOT NULL DEFAULT 1,
    meal_choice TEXT,
    dietary_restrictions TEXT,
    message TEXT,
    created_at TEXT NOT NULL DEFAULT (datetime('now'))
);
