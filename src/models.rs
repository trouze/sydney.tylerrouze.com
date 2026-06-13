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
