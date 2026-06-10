use serde::Deserialize;

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
