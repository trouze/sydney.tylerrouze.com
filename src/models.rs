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

/// Address-collection form payload from the public `/address` page.
/// All fields are `Option<String>` (stored NOT NULL in the db where required)
/// so a partial POST — e.g. a bot with a truncated form — deserializes and
/// gets the friendly re-rendered error instead of an empty 422.
#[derive(Debug, Deserialize)]
pub struct AddressForm {
    pub name: Option<String>,
    pub email: Option<String>,
    pub street1: Option<String>,
    pub street2: Option<String>,
    pub city: Option<String>,
    pub state: Option<String>,
    pub postal_code: Option<String>,
    pub country: Option<String>,
    /// Hidden honeypot field; real users never fill this in.
    pub website: Option<String>,
}

/// A collected address, as stored in the `addresses` table.
#[derive(Debug, FromRow)]
pub struct AddressRow {
    pub id: String,
    pub name: String,
    pub email: Option<String>,
    pub street1: String,
    pub street2: Option<String>,
    pub city: String,
    pub state: Option<String>,
    pub postal_code: Option<String>,
    pub country: String,
    pub created_at: String,
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

/// Admin settings form.
#[derive(Debug, Deserialize)]
pub struct SettingsForm {
    pub rsvp_cutoff: String,
}
