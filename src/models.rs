use serde::Deserialize;
use sqlx::FromRow;

/// An invitation (household/couple), identified by its magic-link invite code.
#[derive(Debug, FromRow)]
pub struct Party {
    pub id: String,
    pub label: String,
}

/// A person within a party.
#[derive(Debug, FromRow)]
pub struct Guest {
    pub id: String,
    pub first_name: String,
    pub last_name: String,
    pub email: Option<String>,
}

/// A weekend event guests can RSVP to individually.
#[derive(Debug, FromRow)]
pub struct Event {
    pub id: String,
    pub name: String,
    pub serves_meal: bool,
}

/// An admin-configured meal choice.
#[derive(Debug, FromRow)]
pub struct MealOption {
    pub id: String,
    pub label: String,
}

/// Optional `?code=` on the RSVP page (from a scanned QR or the gate form).
#[derive(Debug, Deserialize)]
pub struct RsvpQuery {
    pub code: Option<String>,
}

// ---------- admin views ----------

/// A party row as shown on the admin dashboard (includes the invite code).
#[derive(Debug, FromRow)]
pub struct AdminParty {
    pub id: String,
    pub invite_code: String,
    pub label: String,
}

/// A guest row as shown on the admin dashboard (full detail).
#[derive(Debug, FromRow)]
pub struct AdminGuest {
    pub id: String,
    pub party_id: String,
    pub first_name: String,
    pub last_name: String,
    pub email: Option<String>,
    pub is_plus_one: bool,
}

/// A full meal option row for the admin dashboard.
#[derive(Debug, FromRow)]
pub struct AdminMealOption {
    pub id: String,
    pub label: String,
    pub is_active: bool,
    pub sort_order: i64,
}

/// A full event row for the admin dashboard.
#[derive(Debug, FromRow)]
pub struct AdminEvent {
    pub id: String,
    pub name: String,
    pub starts_at: Option<String>,
    pub location: Option<String>,
    pub serves_meal: bool,
    pub sort_order: i64,
}

// ---------- admin form payloads ----------

/// Submitted by the admin login form.
#[derive(Debug, Deserialize)]
pub struct AdminLoginForm {
    pub token: String,
}

/// Add a meal option, or toggle/reorder an existing one.
#[derive(Debug, Deserialize)]
pub struct MealForm {
    pub action: String, // "add" | "update"
    pub id: Option<String>,
    pub label: Option<String>,
    pub is_active: Option<String>, // checkbox: present == on
    pub sort_order: Option<i64>,
}

/// Add or edit an event.
#[derive(Debug, Deserialize)]
pub struct EventForm {
    pub action: String, // "add" | "update"
    pub id: Option<String>,
    pub name: String,
    pub starts_at: Option<String>,
    pub location: Option<String>,
    pub serves_meal: Option<String>, // checkbox: present == on
    pub sort_order: Option<i64>,
}
