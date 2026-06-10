use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, sqlx::FromRow)]
pub struct Rsvp {
    pub id: String,
    pub name: String,
    pub email: String,
    pub attending: bool,
    pub guest_count: i64,
    pub meal_choice: Option<String>,
    pub dietary_restrictions: Option<String>,
    pub message: Option<String>,
    pub created_at: String,
}

#[derive(Debug, Deserialize)]
pub struct RsvpForm {
    pub name: String,
    pub email: String,
    pub attending: String,
    pub guest_count: Option<i64>,
    pub meal_choice: Option<String>,
    pub dietary_restrictions: Option<String>,
    pub message: Option<String>,
}
