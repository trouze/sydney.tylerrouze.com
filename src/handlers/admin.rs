use askama::Template;
use axum::{
    extract::{Form, State},
    response::{Html, IntoResponse, Redirect, Response},
};
use axum_extra::extract::cookie::{Cookie, CookieJar, SameSite};
use uuid::Uuid;

use crate::{
    error::AppError,
    models::{
        AdminEvent, AdminGuest, AdminLoginForm, AdminMealOption, AdminParty, EventForm, MealForm,
    },
    AppState,
};

const ADMIN_COOKIE: &str = "admin";

/// The configured admin secret, or `None` if `ADMIN_TOKEN` is unset.
fn admin_token() -> Option<String> {
    std::env::var("ADMIN_TOKEN").ok().filter(|t| !t.is_empty())
}

/// Re-validate the admin cookie against `ADMIN_TOKEN` on every request.
/// Never trust the client: a missing/mismatched cookie (or unset secret)
/// always fails.
fn require_admin(jar: &CookieJar) -> bool {
    let Some(expected) = admin_token() else {
        return false;
    };
    jar.get(ADMIN_COOKIE)
        .map(|c| c.value() == expected)
        .unwrap_or(false)
}

// ---------- templates ----------

#[derive(Template)]
#[template(path = "admin_login.html")]
struct LoginTemplate {
    error: Option<String>,
}

#[derive(Template)]
#[template(path = "admin.html")]
struct AdminTemplate {
    parties: Vec<PartyView>,
    meal_options: Vec<AdminMealOption>,
    events: Vec<AdminEvent>,
}

struct PartyView {
    party: AdminParty,
    guests: Vec<AdminGuest>,
}

// ---------- auth routes ----------

pub async fn login_page(jar: CookieJar) -> Result<Response, AppError> {
    if require_admin(&jar) {
        return Ok(Redirect::to("/admin").into_response());
    }
    Ok(Html(LoginTemplate { error: None }.render()?).into_response())
}

pub async fn login_submit(
    jar: CookieJar,
    Form(form): Form<AdminLoginForm>,
) -> Result<Response, AppError> {
    match admin_token() {
        Some(expected) if form.token == expected => {
            let cookie = Cookie::build((ADMIN_COOKIE, expected))
                .http_only(true)
                .same_site(SameSite::Lax)
                .path("/")
                .build();
            Ok((jar.add(cookie), Redirect::to("/admin")).into_response())
        }
        _ => Ok(Html(
            LoginTemplate {
                error: Some("Incorrect password.".into()),
            }
            .render()?,
        )
        .into_response()),
    }
}

// ---------- dashboard ----------

pub async fn dashboard(State(pool): State<AppState>, jar: CookieJar) -> Result<Response, AppError> {
    if !require_admin(&jar) {
        return Ok(Redirect::to("/admin/login").into_response());
    }

    let parties = sqlx::query_as::<_, AdminParty>(
        "SELECT id, invite_code, label FROM parties ORDER BY created_at",
    )
    .fetch_all(&pool)
    .await?;

    let guests = sqlx::query_as::<_, AdminGuest>(
        "SELECT id, party_id, first_name, last_name, email, is_plus_one
         FROM guests ORDER BY is_plus_one, created_at",
    )
    .fetch_all(&pool)
    .await?;

    let party_views = parties
        .into_iter()
        .map(|party| {
            let guests = guests
                .iter()
                .filter(|g| g.party_id == party.id)
                .map(|g| AdminGuest {
                    id: g.id.clone(),
                    party_id: g.party_id.clone(),
                    first_name: g.first_name.clone(),
                    last_name: g.last_name.clone(),
                    email: g.email.clone(),
                    is_plus_one: g.is_plus_one,
                })
                .collect();
            PartyView { party, guests }
        })
        .collect();

    let meal_options = sqlx::query_as::<_, AdminMealOption>(
        "SELECT id, label, is_active, sort_order FROM meal_options ORDER BY sort_order",
    )
    .fetch_all(&pool)
    .await?;

    let events = sqlx::query_as::<_, AdminEvent>(
        "SELECT id, name, starts_at, location, serves_meal, sort_order
         FROM events ORDER BY sort_order",
    )
    .fetch_all(&pool)
    .await?;

    Ok(Html(
        AdminTemplate {
            parties: party_views,
            meal_options,
            events,
        }
        .render()?,
    )
    .into_response())
}

// ---------- POST /admin/parties ----------

pub async fn create_party(
    State(pool): State<AppState>,
    jar: CookieJar,
    Form(fields): Form<Vec<(String, String)>>,
) -> Result<Response, AppError> {
    if !require_admin(&jar) {
        return Ok(Redirect::to("/admin/login").into_response());
    }

    // Pull the party label + parallel guest-row arrays from the raw form.
    let mut label = String::new();
    let mut last_name = String::new();
    let mut first_names: Vec<String> = Vec::new();
    let mut last_names: Vec<String> = Vec::new();
    let mut emails: Vec<String> = Vec::new();
    let mut plus_ones: Vec<bool> = Vec::new();
    for (key, value) in fields {
        match key.as_str() {
            "label" => label = value,
            "last_name" => last_name = value,
            "first_names" => first_names.push(value),
            "last_names" => last_names.push(value),
            "emails" => emails.push(value),
            // checkbox value is the row index it belongs to
            "plus_ones" => {
                if let Ok(idx) = value.parse::<usize>() {
                    while plus_ones.len() <= idx {
                        plus_ones.push(false);
                    }
                    plus_ones[idx] = true;
                }
            }
            _ => {}
        }
    }

    let label = label.trim();
    let last_name = last_name.trim();
    if label.is_empty() || last_name.is_empty() {
        return Ok(Redirect::to("/admin").into_response());
    }

    let invite_code = generate_invite_code(&pool, last_name).await?;
    let party_id = Uuid::new_v4().to_string();

    let mut tx = pool.begin().await?;
    sqlx::query("INSERT INTO parties (id, invite_code, label) VALUES (?, ?, ?)")
        .bind(&party_id)
        .bind(&invite_code)
        .bind(label)
        .execute(&mut *tx)
        .await?;

    for (i, first) in first_names.iter().enumerate() {
        let first = first.trim();
        if first.is_empty() {
            continue; // skip empty guest rows
        }
        // Per-guest last name falls back to the party last name.
        let glast = last_names
            .get(i)
            .map(|s| s.trim())
            .filter(|s| !s.is_empty())
            .unwrap_or(last_name);
        let email = emails
            .get(i)
            .map(|s| s.trim())
            .filter(|s| !s.is_empty())
            .map(|s| s.to_string());
        let is_plus_one = plus_ones.get(i).copied().unwrap_or(false);
        sqlx::query(
            "INSERT INTO guests (id, party_id, first_name, last_name, email, is_plus_one)
             VALUES (?, ?, ?, ?, ?, ?)",
        )
        .bind(Uuid::new_v4().to_string())
        .bind(&party_id)
        .bind(first)
        .bind(glast)
        .bind(email)
        .bind(is_plus_one)
        .execute(&mut *tx)
        .await?;
    }
    tx.commit().await?;

    Ok(Redirect::to("/admin").into_response())
}

/// Generate a unique invite code like `SMITH-7Q2`, retrying on collision.
async fn generate_invite_code(pool: &AppState, last_name: &str) -> Result<String, AppError> {
    let prefix: String = last_name
        .chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .map(|c| c.to_ascii_uppercase())
        .collect();
    let prefix = if prefix.is_empty() {
        "GUEST".to_string()
    } else {
        prefix
    };

    for _ in 0..50 {
        let suffix = random_suffix(3 + (rand_byte() % 2) as usize); // 3 or 4 chars
        let code = format!("{prefix}-{suffix}");
        let existing =
            sqlx::query_as::<_, (i64,)>("SELECT COUNT(*) FROM parties WHERE invite_code = ?")
                .bind(&code)
                .fetch_one(pool)
                .await?;
        if existing.0 == 0 {
            return Ok(code);
        }
    }
    Err(AppError::Other(anyhow::anyhow!(
        "could not generate a unique invite code after 50 attempts"
    )))
}

const ALPHABET: &[u8] = b"ABCDEFGHJKLMNPQRSTUVWXYZ23456789"; // no easily-confused chars

fn random_suffix(len: usize) -> String {
    (0..len)
        .map(|_| ALPHABET[(rand_byte() as usize) % ALPHABET.len()] as char)
        .collect()
}

/// One pseudo-random byte. We don't have the `rand` crate, and these codes are
/// not security tokens (uniqueness is enforced by the UNIQUE constraint + retry),
/// so derive entropy from a fresh UUID v4.
fn rand_byte() -> u8 {
    Uuid::new_v4().as_bytes()[0]
}

// ---------- POST /admin/meals ----------

pub async fn save_meal(
    State(pool): State<AppState>,
    jar: CookieJar,
    Form(form): Form<MealForm>,
) -> Result<Response, AppError> {
    if !require_admin(&jar) {
        return Ok(Redirect::to("/admin/login").into_response());
    }

    let is_active = form.is_active.is_some();
    let sort_order = form.sort_order.unwrap_or(0);

    if form.action == "update" {
        if let Some(id) = form.id.as_deref().filter(|s| !s.is_empty()) {
            sqlx::query("UPDATE meal_options SET is_active = ?, sort_order = ? WHERE id = ?")
                .bind(is_active)
                .bind(sort_order)
                .bind(id)
                .execute(&pool)
                .await?;
        }
    } else {
        // add
        let label = form.label.unwrap_or_default();
        let label = label.trim();
        if !label.is_empty() {
            sqlx::query(
                "INSERT INTO meal_options (id, label, is_active, sort_order) VALUES (?, ?, ?, ?)",
            )
            .bind(Uuid::new_v4().to_string())
            .bind(label)
            .bind(is_active)
            .bind(sort_order)
            .execute(&pool)
            .await?;
        }
    }

    Ok(Redirect::to("/admin").into_response())
}

// ---------- POST /admin/events ----------

pub async fn save_event(
    State(pool): State<AppState>,
    jar: CookieJar,
    Form(form): Form<EventForm>,
) -> Result<Response, AppError> {
    if !require_admin(&jar) {
        return Ok(Redirect::to("/admin/login").into_response());
    }

    let name = form.name.trim();
    if name.is_empty() {
        return Ok(Redirect::to("/admin").into_response());
    }
    let serves_meal = form.serves_meal.is_some();
    let sort_order = form.sort_order.unwrap_or(0);
    let starts_at = form
        .starts_at
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());
    let location = form
        .location
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());

    if form.action == "update" {
        if let Some(id) = form.id.as_deref().filter(|s| !s.is_empty()) {
            sqlx::query(
                "UPDATE events
                 SET name = ?, starts_at = ?, location = ?, serves_meal = ?, sort_order = ?
                 WHERE id = ?",
            )
            .bind(name)
            .bind(&starts_at)
            .bind(&location)
            .bind(serves_meal)
            .bind(sort_order)
            .bind(id)
            .execute(&pool)
            .await?;
        }
    } else {
        sqlx::query(
            "INSERT INTO events (id, name, starts_at, location, serves_meal, sort_order)
             VALUES (?, ?, ?, ?, ?, ?)",
        )
        .bind(Uuid::new_v4().to_string())
        .bind(name)
        .bind(&starts_at)
        .bind(&location)
        .bind(serves_meal)
        .bind(sort_order)
        .execute(&pool)
        .await?;
    }

    Ok(Redirect::to("/admin").into_response())
}
