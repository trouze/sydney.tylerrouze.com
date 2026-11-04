use askama::Template;
use axum::{
    extract::{Form, State},
    response::{Html, IntoResponse, Response},
};
use uuid::Uuid;

use crate::{error::AppError, models::AddressForm, AppState};

// ---------- templates ----------

/// The form partial, used two ways: included by `address.html` on the initial
/// page load, and rendered standalone as the htmx POST response (validation
/// errors with sticky values). Fields must stay identical to `AddressPage`.
#[derive(Template)]
#[template(path = "address_form.html")]
struct FormPartial {
    error: Option<String>,
    name: String,
    email: String,
    street1: String,
    street2: String,
    city: String,
    state: String,
    postal_code: String,
    country: String,
}

/// The full `/address` page (navigation + header + the form partial).
#[derive(Template)]
#[template(path = "address.html")]
struct AddressPage {
    error: Option<String>,
    name: String,
    email: String,
    street1: String,
    street2: String,
    city: String,
    state: String,
    postal_code: String,
    country: String,
}

/// Success fragment: htmx swaps this in place of the form container.
#[derive(Template)]
#[template(path = "address_thanks.html")]
struct ThanksPage {
    name: String,
}

// ---------- handlers ----------

/// GET /address — the blank form.
pub async fn address() -> Html<String> {
    Html(
        AddressPage {
            error: None,
            name: String::new(),
            email: String::new(),
            street1: String::new(),
            street2: String::new(),
            city: String::new(),
            state: String::new(),
            postal_code: String::new(),
            country: "USA".to_string(),
        }
        .render()
        .unwrap(),
    )
}

/// POST /address — validate, store, and hand htmx back a fragment to swap in.
pub async fn submit(
    State(pool): State<AppState>,
    Form(f): Form<AddressForm>,
) -> Result<Response, AppError> {
    // Honeypot: the `website` field is hidden from humans, so anything in it
    // means a bot. Show a plausible success while discarding the row.
    if f.website
        .as_deref()
        .map(|w| !w.trim().is_empty())
        .unwrap_or(false)
    {
        tracing::warn!("address honeypot triggered — submission discarded");
        return thanks_html("friend");
    }

    let name = text(&f.name);
    let email = text(&f.email);
    let street1 = text(&f.street1);
    let street2 = text(&f.street2);
    let city = text(&f.city);
    let state = text(&f.state);
    let postal_code = text(&f.postal_code);
    let country = text(&f.country);

    // Required: name, street1, city, country. Email is optional but must be
    // plausible when present; everything else is international-friendly.
    let mut missing = Vec::new();
    if name.is_empty() {
        missing.push("name");
    }
    if street1.is_empty() {
        missing.push("street address");
    }
    if city.is_empty() {
        missing.push("city");
    }
    if country.is_empty() {
        missing.push("country");
    }
    if !missing.is_empty() {
        return form_error(
            Some(format!(
                "Please add your {}.",
                missing
                    .join(", ")
                    .rsplit_once(", ")
                    .map_or_else(|| missing.join(", "), |(a, b)| format!("{a} and {b}"),)
            )),
            &name,
            &email,
            &street1,
            &street2,
            &city,
            &state,
            &postal_code,
            &country,
        );
    }
    if !email.is_empty() && !email.contains('@') {
        return form_error(
            Some("That email address doesn't look right — please check it.".to_string()),
            &name,
            &email,
            &street1,
            &street2,
            &city,
            &state,
            &postal_code,
            &country,
        );
    }

    sqlx::query(
        "INSERT INTO addresses (id, name, email, street1, street2, city, state, postal_code, country)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
    )
    .bind(Uuid::new_v4().to_string())
    .bind(&name)
    .bind(empty_is_none(&email))
    .bind(&street1)
    .bind(empty_is_none(&street2))
    .bind(&city)
    .bind(empty_is_none(&state))
    .bind(empty_is_none(&postal_code))
    .bind(&country)
    .execute(&pool)
    .await?;

    thanks_html(&name)
}

/// An empty/missing field becomes NULL in the database; anything else is
/// stored trimmed.
fn text(s: &Option<String>) -> String {
    s.as_deref().unwrap_or_default().trim().to_string()
}

/// NULL out empty strings so the admin view/CSV compressed handles them
/// consistently.
fn empty_is_none(s: &str) -> Option<String> {
    (!s.is_empty()).then(|| s.to_string())
}

/// Re-render the form with an error banner and the entered values kept.
#[allow(clippy::too_many_arguments)]
fn form_error(
    error: Option<String>,
    name: &str,
    email: &str,
    street1: &str,
    street2: &str,
    city: &str,
    state: &str,
    postal_code: &str,
    country: &str,
) -> Result<Response, AppError> {
    Ok(Html(
        FormPartial {
            error,
            name: name.to_string(),
            email: email.to_string(),
            street1: street1.to_string(),
            street2: street2.to_string(),
            city: city.to_string(),
            state: state.to_string(),
            postal_code: postal_code.to_string(),
            country: country.to_string(),
        }
        .render()
        .map_err(AppError::Template)?,
    )
    .into_response())
}

/// Success fragment with the first name on the envelope (last name when there
/// is only one name given).
fn thanks_html(name: &str) -> Result<Response, AppError> {
    let first = name
        .split_whitespace()
        .next()
        .unwrap_or("friend")
        .to_string();
    Ok(Html(
        ThanksPage { name: first }
            .render()
            .map_err(AppError::Template)?,
    )
    .into_response())
}
