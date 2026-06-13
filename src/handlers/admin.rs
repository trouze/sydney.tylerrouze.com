use askama::Template;
use axum::{
    extract::{Form, Multipart, Path, State},
    response::{Html, IntoResponse, Redirect, Response},
};
use axum_extra::extract::cookie::{Cookie, CookieJar, SameSite};
use serde::Deserialize;
use std::collections::{HashMap, HashSet};
use uuid::Uuid;

use crate::{
    error::AppError,
    models::{
        AdminEvent, AdminGuest, AdminLoginForm, AdminMealOption, AdminParty, EventForm, MealForm,
    },
    AppState,
};

pub const ADMIN_COOKIE: &str = "admin";

/// The configured admin secret, or `None` if `ADMIN_TOKEN` is unset.
fn admin_token() -> Option<String> {
    std::env::var("ADMIN_TOKEN").ok().filter(|t| !t.is_empty())
}

/// Whether auth cookies should be marked `Secure` (sent only over HTTPS).
/// Defaults to true; set `COOKIE_INSECURE=1` for local http:// development.
fn secure_cookies() -> bool {
    std::env::var("COOKIE_INSECURE").is_err()
}

/// Build the admin session cookie carrying `value`. httponly + SameSite=Lax +
/// Secure (in prod) so it can't be read by JS, sent cross-site, or leaked over
/// plain http.
fn admin_cookie<'a>(value: String) -> Cookie<'a> {
    Cookie::build((ADMIN_COOKIE, value))
        .http_only(true)
        .same_site(SameSite::Lax)
        .secure(secure_cookies())
        .path("/")
        .build()
}

/// Re-validate the admin cookie against `ADMIN_TOKEN` on every request.
/// Never trust the client: a missing/mismatched cookie (or unset secret)
/// always fails. Shared across all admin handlers (incl. `qr`).
pub fn require_admin(jar: &CookieJar) -> bool {
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
    /// Events offered as per-guest invitation checkboxes in the add-party form.
    events: Vec<EventOption>,
}

/// An event id+name pair for invitation checkboxes.
struct EventOption {
    id: String,
    name: String,
}

#[derive(Template)]
#[template(path = "admin_meals.html")]
struct MealsTemplate {
    meal_options: Vec<AdminMealOption>,
}

#[derive(Template)]
#[template(path = "admin_events.html")]
struct EventsTemplate {
    events: Vec<AdminEvent>,
}

struct PartyView {
    party: AdminParty,
    guests: Vec<AdminGuest>,
}

#[derive(Template)]
#[template(path = "admin_edit_party.html")]
struct EditPartyTemplate {
    party: AdminParty,
    guests: Vec<EditGuestRow>,
    /// Event names for the matrix header row.
    event_names: Vec<String>,
}

/// A guest row in the edit-party invitation matrix.
struct EditGuestRow {
    g: AdminGuest,
    /// One cell per event (aligned with `event_names`), checked = invited.
    cells: Vec<EditEventCell>,
}

struct EditEventCell {
    event_id: String,
    checked: bool,
}

#[derive(Template)]
#[template(path = "admin_analytics.html")]
struct AnalyticsTemplate {
    total_parties: i64,
    responded_parties: i64,
    pending_parties: i64,
    total_guests: i64,
    attending_guests: i64,
    events: Vec<EventStat>,
    meals: Vec<MealStat>,
    parties: Vec<PartyStat>,
}

struct EventStat {
    name: String,
    attending: i64,
}

struct MealStat {
    label: String,
    count: i64,
}

struct PartyStat {
    label: String,
    invite_code: String,
    responded: bool,
    guest_count: i64,
    attending_count: i64,
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
            Ok((jar.add(admin_cookie(expected)), Redirect::to("/admin")).into_response())
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

/// POST /admin/logout — clear the admin cookie and bounce to the login page.
/// The removal cookie must match the original path ("/") for the browser to
/// drop it.
pub async fn logout(jar: CookieJar) -> Result<Response, AppError> {
    let removal = Cookie::build((ADMIN_COOKIE, "")).path("/").build();
    Ok((jar.remove(removal), Redirect::to("/admin/login")).into_response())
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

    let events = sqlx::query_as::<_, (String, String)>(
        "SELECT id, name FROM events ORDER BY sort_order",
    )
    .fetch_all(&pool)
    .await?
    .into_iter()
    .map(|(id, name)| EventOption { id, name })
    .collect();

    Ok(Html(
        AdminTemplate {
            parties: party_views,
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
    // Invitation checkboxes are positional (rows have no ids yet):
    // name="inv:<row_index>:<event_id>". They default to checked in the form.
    let mut invites: HashMap<usize, Vec<String>> = HashMap::new();
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
            _ => {
                if let Some(rest) = key.strip_prefix("inv:") {
                    if let Some((idx, eid)) = rest.split_once(':') {
                        if let Ok(idx) = idx.parse::<usize>() {
                            invites.entry(idx).or_default().push(eid.to_string());
                        }
                    }
                }
            }
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
        let guest_id = Uuid::new_v4().to_string();
        sqlx::query(
            "INSERT INTO guests (id, party_id, first_name, last_name, email, is_plus_one)
             VALUES (?, ?, ?, ?, ?, ?)",
        )
        .bind(&guest_id)
        .bind(&party_id)
        .bind(first)
        .bind(glast)
        .bind(email)
        .bind(is_plus_one)
        .execute(&mut *tx)
        .await?;
        // Invite to exactly the events checked for this row.
        if let Some(eids) = invites.get(&i) {
            insert_invitations(&mut tx, &guest_id, eids).await?;
        }
    }
    tx.commit().await?;

    Ok(Redirect::to("/admin").into_response())
}

// ---------- GET /admin/parties/:id/edit ----------

pub async fn edit_party_page(
    State(pool): State<AppState>,
    jar: CookieJar,
    Path(id): Path<String>,
) -> Result<Response, AppError> {
    if !require_admin(&jar) {
        return Ok(Redirect::to("/admin/login").into_response());
    }

    let party =
        sqlx::query_as::<_, AdminParty>("SELECT id, invite_code, label FROM parties WHERE id = ?")
            .bind(&id)
            .fetch_optional(&pool)
            .await?;
    let Some(party) = party else {
        return Ok(Redirect::to("/admin").into_response());
    };

    let guests = sqlx::query_as::<_, AdminGuest>(
        "SELECT id, party_id, first_name, last_name, email, is_plus_one
         FROM guests WHERE party_id = ? ORDER BY is_plus_one, created_at",
    )
    .bind(&id)
    .fetch_all(&pool)
    .await?;

    // Events (matrix columns) + the current invitation set for this party.
    let events = sqlx::query_as::<_, (String, String)>(
        "SELECT id, name FROM events ORDER BY sort_order",
    )
    .fetch_all(&pool)
    .await?;
    let invited = fetch_invitation_set(&pool, &id).await?;

    let event_names = events.iter().map(|(_, name)| name.clone()).collect();
    let guest_rows = guests
        .into_iter()
        .map(|g| {
            let cells = events
                .iter()
                .map(|(eid, _)| EditEventCell {
                    event_id: eid.clone(),
                    checked: invited.contains(&(g.id.clone(), eid.clone())),
                })
                .collect();
            EditGuestRow { g, cells }
        })
        .collect();

    Ok(Html(
        EditPartyTemplate {
            party,
            guests: guest_rows,
            event_names,
        }
        .render()?,
    )
    .into_response())
}

// ---------- POST /admin/parties/:id ----------

pub async fn update_party(
    State(pool): State<AppState>,
    jar: CookieJar,
    Path(id): Path<String>,
    Form(fields): Form<Vec<(String, String)>>,
) -> Result<Response, AppError> {
    if !require_admin(&jar) {
        return Ok(Redirect::to("/admin/login").into_response());
    }

    // Existing guests arrive as parallel arrays keyed by row order; new guests
    // reuse the create-party field names. Plus-one checkboxes carry their row
    // index as the value.
    let mut label = String::new();
    let mut existing_ids: Vec<String> = Vec::new();
    let mut existing_first: Vec<String> = Vec::new();
    let mut existing_last: Vec<String> = Vec::new();
    let mut existing_email: Vec<String> = Vec::new();
    let mut existing_plus: HashSet<usize> = HashSet::new();
    let mut new_first: Vec<String> = Vec::new();
    let mut new_last: Vec<String> = Vec::new();
    let mut new_email: Vec<String> = Vec::new();
    let mut new_plus: HashSet<usize> = HashSet::new();
    // Invitation matrix checkboxes: name="inv:<guest_id>:<event_id>".
    let mut invites: HashMap<String, Vec<String>> = HashMap::new();
    for (key, value) in fields {
        match key.as_str() {
            "label" => label = value,
            "existing_id" => existing_ids.push(value),
            "existing_first" => existing_first.push(value),
            "existing_last" => existing_last.push(value),
            "existing_email" => existing_email.push(value),
            "existing_plus" => {
                if let Ok(i) = value.parse::<usize>() {
                    existing_plus.insert(i);
                }
            }
            "first_names" => new_first.push(value),
            "last_names" => new_last.push(value),
            "emails" => new_email.push(value),
            "plus_ones" => {
                if let Ok(i) = value.parse::<usize>() {
                    new_plus.insert(i);
                }
            }
            _ => {
                if let Some(rest) = key.strip_prefix("inv:") {
                    if let Some((gid, eid)) = rest.split_once(':') {
                        invites
                            .entry(gid.to_string())
                            .or_default()
                            .push(eid.to_string());
                    }
                }
            }
        }
    }

    let label = label.trim();
    if label.is_empty() {
        return Ok(Redirect::to(&format!("/admin/parties/{id}/edit")).into_response());
    }

    // New guests default to every event; trim them in the matrix afterward.
    let event_ids = all_event_ids(&pool).await?;

    let mut tx = pool.begin().await?;
    sqlx::query("UPDATE parties SET label = ? WHERE id = ?")
        .bind(label)
        .bind(&id)
        .execute(&mut *tx)
        .await?;

    // Update each existing guest in place (scoped to this party), and replace
    // their invitations with whatever the matrix has checked.
    for (i, gid) in existing_ids.iter().enumerate() {
        let first = existing_first.get(i).map(|s| s.trim()).unwrap_or("");
        if first.is_empty() {
            continue; // never blank out a name (and leave invitations untouched)
        }
        let last = existing_last.get(i).map(|s| s.trim()).unwrap_or("");
        let email = existing_email
            .get(i)
            .map(|s| s.trim())
            .filter(|s| !s.is_empty())
            .map(|s| s.to_string());
        let is_plus_one = existing_plus.contains(&i);
        sqlx::query(
            "UPDATE guests SET first_name = ?, last_name = ?, email = ?, is_plus_one = ?
             WHERE id = ? AND party_id = ?",
        )
        .bind(first)
        .bind(last)
        .bind(email)
        .bind(is_plus_one)
        .bind(gid)
        .bind(&id)
        .execute(&mut *tx)
        .await?;

        // Replace this guest's invitation set from the matrix checkboxes.
        sqlx::query("DELETE FROM event_invitations WHERE guest_id = ?")
            .bind(gid)
            .execute(&mut *tx)
            .await?;
        if let Some(eids) = invites.get(gid) {
            insert_invitations(&mut tx, gid, eids).await?;
        }
    }

    // Insert any newly entered guests (blank first name = skip the row).
    for (i, first) in new_first.iter().enumerate() {
        let first = first.trim();
        if first.is_empty() {
            continue;
        }
        let last = new_last.get(i).map(|s| s.trim()).unwrap_or("");
        let email = new_email
            .get(i)
            .map(|s| s.trim())
            .filter(|s| !s.is_empty())
            .map(|s| s.to_string());
        let is_plus_one = new_plus.contains(&i);
        let guest_id = Uuid::new_v4().to_string();
        sqlx::query(
            "INSERT INTO guests (id, party_id, first_name, last_name, email, is_plus_one)
             VALUES (?, ?, ?, ?, ?, ?)",
        )
        .bind(&guest_id)
        .bind(&id)
        .bind(first)
        .bind(last)
        .bind(email)
        .bind(is_plus_one)
        .execute(&mut *tx)
        .await?;
        insert_invitations(&mut tx, &guest_id, &event_ids).await?;
    }
    tx.commit().await?;

    Ok(Redirect::to("/admin").into_response())
}

// ---------- GET /admin/analytics ----------

pub async fn analytics(State(pool): State<AppState>, jar: CookieJar) -> Result<Response, AppError> {
    if !require_admin(&jar) {
        return Ok(Redirect::to("/admin/login").into_response());
    }

    let total_parties: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM parties")
        .fetch_one(&pool)
        .await?;
    let total_guests: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM guests")
        .fetch_one(&pool)
        .await?;
    let responded_parties: i64 = sqlx::query_scalar(
        "SELECT COUNT(DISTINCT g.party_id)
         FROM current_rsvp cr JOIN guests g ON g.id = cr.guest_id
         WHERE cr.attending IS NOT NULL",
    )
    .fetch_one(&pool)
    .await?;
    let attending_guests: i64 = sqlx::query_scalar(
        "SELECT COUNT(DISTINCT cr.guest_id) FROM current_rsvp cr WHERE cr.attending = 1",
    )
    .fetch_one(&pool)
    .await?;

    // Attendance per event.
    let events = sqlx::query_as::<_, (String, i64)>(
        "SELECT e.name,
                COALESCE(SUM(CASE WHEN cr.attending = 1 THEN 1 ELSE 0 END), 0) AS attending
         FROM events e
         LEFT JOIN current_rsvp cr ON cr.event_id = e.id
         GROUP BY e.id ORDER BY e.sort_order",
    )
    .fetch_all(&pool)
    .await?
    .into_iter()
    .map(|(name, attending)| EventStat { name, attending })
    .collect();

    // Meal choices among attending guests.
    let meals = sqlx::query_as::<_, (String, i64)>(
        "SELECT m.label, COUNT(*) AS c
         FROM meal_options m
         JOIN current_rsvp cr ON cr.meal_option_id = m.id AND cr.attending = 1
         GROUP BY m.id ORDER BY m.sort_order",
    )
    .fetch_all(&pool)
    .await?
    .into_iter()
    .map(|(label, count)| MealStat { label, count })
    .collect();

    // Per-party status.
    let parties = sqlx::query_as::<_, (String, String, String)>(
        "SELECT id, invite_code, label FROM parties ORDER BY label",
    )
    .fetch_all(&pool)
    .await?;
    let guest_counts: HashMap<String, i64> = sqlx::query_as::<_, (String, i64)>(
        "SELECT party_id, COUNT(*) FROM guests GROUP BY party_id",
    )
    .fetch_all(&pool)
    .await?
    .into_iter()
    .collect();
    let attending_counts: HashMap<String, i64> = sqlx::query_as::<_, (String, i64)>(
        "SELECT g.party_id, COUNT(DISTINCT cr.guest_id)
         FROM current_rsvp cr JOIN guests g ON g.id = cr.guest_id
         WHERE cr.attending = 1 GROUP BY g.party_id",
    )
    .fetch_all(&pool)
    .await?
    .into_iter()
    .collect();
    let responded_set: HashSet<String> = sqlx::query_as::<_, (String,)>(
        "SELECT DISTINCT g.party_id
         FROM current_rsvp cr JOIN guests g ON g.id = cr.guest_id
         WHERE cr.attending IS NOT NULL",
    )
    .fetch_all(&pool)
    .await?
    .into_iter()
    .map(|(id,)| id)
    .collect();

    let party_stats = parties
        .into_iter()
        .map(|(id, invite_code, label)| PartyStat {
            label,
            invite_code,
            responded: responded_set.contains(&id),
            guest_count: guest_counts.get(&id).copied().unwrap_or(0),
            attending_count: attending_counts.get(&id).copied().unwrap_or(0),
        })
        .collect();

    Ok(Html(
        AnalyticsTemplate {
            total_parties,
            responded_parties,
            pending_parties: total_parties - responded_parties,
            total_guests,
            attending_guests,
            events,
            meals,
            parties: party_stats,
        }
        .render()?,
    )
    .into_response())
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

// ---------- GET /admin/meals ----------

pub async fn meals_page(State(pool): State<AppState>, jar: CookieJar) -> Result<Response, AppError> {
    if !require_admin(&jar) {
        return Ok(Redirect::to("/admin/login").into_response());
    }

    let meal_options = sqlx::query_as::<_, AdminMealOption>(
        "SELECT id, label, is_active, sort_order FROM meal_options ORDER BY sort_order",
    )
    .fetch_all(&pool)
    .await?;

    Ok(Html(MealsTemplate { meal_options }.render()?).into_response())
}

// ---------- GET /admin/events ----------

pub async fn events_page(State(pool): State<AppState>, jar: CookieJar) -> Result<Response, AppError> {
    if !require_admin(&jar) {
        return Ok(Redirect::to("/admin/login").into_response());
    }

    let events = sqlx::query_as::<_, AdminEvent>(
        "SELECT id, name, starts_at, location, serves_meal, sort_order
         FROM events ORDER BY sort_order",
    )
    .fetch_all(&pool)
    .await?;

    Ok(Html(EventsTemplate { events }.render()?).into_response())
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

    Ok(Redirect::to("/admin/meals").into_response())
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
        return Ok(Redirect::to("/admin/events").into_response());
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

    Ok(Redirect::to("/admin/events").into_response())
}

// ---------- bulk import (export → edit → re-upload, keyed on invite_code) ----------
//
// The uploaded sheet is treated as the complete guest list ("source of truth"):
//   • row's invite_code matches an existing party  → update it in place
//     (its code, and therefore every QR/magic link, is preserved)
//   • invite_code blank                            → create a new party + code
//   • existing party absent from the sheet         → remove it
// Re-uploading an untouched export is therefore a no-op (idempotent). Within a
// kept party, guests are replaced wholesale — see the discussion in CLAUDE/PR:
// individual RSVPs are NOT preserved across an edit to that party, which is fine
// while we're still building the list. Surgical post-invite tweaks belong in the
// per-party edit UI.

#[derive(Template)]
#[template(path = "admin_import.html")]
struct ImportTemplate {
    /// Set after a successful parse: what *would* change on confirm.
    preview: Option<ImportPreview>,
    /// The raw CSV carried forward into the confirm form (re-parsed on commit).
    raw_csv: String,
    /// A parse/usage error to show instead of a preview.
    error: Option<String>,
}

struct ImportPreview {
    creates: Vec<PreviewParty>,
    updates: Vec<PreviewParty>,
    removes: Vec<PreviewRemove>,
    /// Non-fatal notes (skipped rows, unknown codes) surfaced before committing.
    warnings: Vec<String>,
}

struct PreviewParty {
    label: String,
    /// Existing code for updates; a placeholder note for creates.
    code: String,
    guests: Vec<ParsedGuest>,
}

struct PreviewRemove {
    label: String,
    invite_code: String,
}

#[derive(Clone, Debug)]
struct ParsedGuest {
    first_name: String,
    last_name: String,
    email: Option<String>,
    is_plus_one: bool,
    /// Event names this guest is invited to. `None` when the sheet has no event
    /// columns at all (→ default to every event); `Some` (possibly empty) when
    /// it does, naming exactly the events whose cell was truthy.
    invited_events: Option<Vec<String>>,
}

/// A party assembled from one or more CSV rows (grouped by invite_code, else by
/// party_label). `invite_code` is `Some` only when the sheet carried one.
#[derive(Debug)]
struct ParsedGroup {
    invite_code: Option<String>,
    label: String,
    /// Drives the invite-code prefix for new parties (first guest's last name,
    /// else the label).
    invite_last_name: String,
    guests: Vec<ParsedGuest>,
}

/// Submitted by the "Confirm import" form (hidden textarea round-trips the CSV).
#[derive(Debug, Deserialize)]
pub struct ImportCommitForm {
    csv: String,
}

/// An existing party, loaded to diff the upload against the live list.
struct ExistingParty {
    id: String,
    invite_code: String,
    label: String,
}

/// One party in the reconcile plan. `existing_id`/`invite_code` are `Some` for
/// updates (reuse that row + code); both `None` for creates.
struct PlanParty {
    existing_id: Option<String>,
    invite_code: Option<String>,
    label: String,
    invite_last_name: String,
    guests: Vec<ParsedGuest>,
}

/// What committing the upload would do, computed from the parsed sheet + the
/// current DB state.
struct ReconcilePlan {
    creates: Vec<PlanParty>,
    updates: Vec<PlanParty>,
    /// Existing parties absent from the sheet: (id, invite_code, label).
    removes: Vec<(String, String, String)>,
    warnings: Vec<String>,
}

fn parse_truthy(s: &str) -> bool {
    matches!(
        s.trim().to_ascii_lowercase().as_str(),
        "1" | "true" | "yes" | "y" | "x" | "+1"
    )
}

/// Parse the CSV into parties grouped by invite_code (falling back to
/// party_label for code-less rows), preserving first-seen order. `Err` only for
/// problems that prevent parsing at all (missing column, malformed CSV).
fn parse_import_csv(text: &str) -> Result<(Vec<ParsedGroup>, Vec<String>), String> {
    if text.trim().is_empty() {
        return Err("No CSV data provided.".into());
    }

    let mut rdr = csv::ReaderBuilder::new()
        .has_headers(true)
        .flexible(true)
        .trim(csv::Trim::All)
        .from_reader(text.as_bytes());

    // Header-driven so unknown columns can be treated as per-event invitation
    // columns (one column per event, header = event name, truthy cell = invited).
    let headers = rdr
        .headers()
        .map_err(|e| format!("Couldn't read the header row. ({e})"))?
        .clone();
    let mut col: HashMap<String, usize> = HashMap::new();
    for (i, h) in headers.iter().enumerate() {
        col.entry(h.trim().to_ascii_lowercase()).or_insert(i);
    }
    const KNOWN: [&str; 6] = [
        "invite_code",
        "party_label",
        "first_name",
        "last_name",
        "email",
        "is_plus_one",
    ];
    if !col.contains_key("party_label") {
        return Err("The sheet needs a \"party_label\" column.".into());
    }
    // Event-invitation columns: any non-empty header that isn't a known field.
    let event_cols: Vec<(usize, String)> = headers
        .iter()
        .enumerate()
        .filter_map(|(i, h)| {
            let name = h.trim();
            let lname = name.to_ascii_lowercase();
            (!name.is_empty() && !KNOWN.contains(&lname.as_str())).then(|| (i, name.to_string()))
        })
        .collect();

    let mut order: Vec<String> = Vec::new();
    let mut groups: HashMap<String, ParsedGroup> = HashMap::new();
    let mut warnings: Vec<String> = Vec::new();

    for (i, result) in rdr.records().enumerate() {
        // +2: row 1 is the header, and humans count from 1.
        let line = i + 2;
        let rec = result.map_err(|e| {
            format!("Couldn't read row {line}. Check the columns match the header. ({e})")
        })?;
        let field = |name: &str| -> &str {
            col.get(name)
                .and_then(|&ci| rec.get(ci))
                .unwrap_or("")
                .trim()
        };

        let code = {
            let c = field("invite_code");
            (!c.is_empty()).then(|| c.to_string())
        };
        let label = field("party_label").to_string();
        let first = field("first_name").to_string();

        // Fully blank line: ignore silently.
        if code.is_none() && label.is_empty() && first.is_empty() {
            continue;
        }
        // A guest row with nothing to attach it to.
        if code.is_none() && label.is_empty() {
            warnings.push(format!("Row {line}: no party_label or invite_code — skipped."));
            continue;
        }

        // Rows sharing a code are one party even if the label was edited; code-less
        // rows group by (case-insensitive) label.
        let key = match &code {
            Some(c) => format!("code:{c}"),
            None => format!("label:{}", label.to_ascii_lowercase()),
        };
        let group = groups.entry(key.clone()).or_insert_with(|| {
            order.push(key.clone());
            ParsedGroup {
                invite_code: code.clone(),
                label: label.clone(),
                invite_last_name: String::new(),
                guests: Vec::new(),
            }
        });
        // A non-empty label on any row updates the party label.
        if !label.is_empty() {
            group.label = label.clone();
        }
        // A code-only row (no first name) just declares the party exists, so it's
        // kept rather than removed. Nothing to add as a guest.
        if first.is_empty() {
            continue;
        }

        let last = field("last_name").to_string();
        let email = {
            let e = field("email");
            (!e.is_empty()).then(|| e.to_string())
        };
        // With event columns present, invited = the truthy ones (possibly none);
        // without any event columns, None means "default to every event".
        let invited_events = (!event_cols.is_empty()).then(|| {
            event_cols
                .iter()
                .filter(|(ci, _)| parse_truthy(rec.get(*ci).unwrap_or("")))
                .map(|(_, name)| name.clone())
                .collect::<Vec<_>>()
        });
        let is_plus_one = parse_truthy(field("is_plus_one"));
        if group.invite_last_name.is_empty() && !last.is_empty() {
            group.invite_last_name = last.clone();
        }
        group.guests.push(ParsedGuest {
            first_name: first,
            last_name: last,
            email,
            is_plus_one,
            invited_events,
        });
    }

    let groups: Vec<ParsedGroup> = order
        .into_iter()
        .map(|key| {
            let mut g = groups.remove(&key).expect("key was inserted");
            // No last name anywhere in the group: derive the code prefix from the label.
            if g.invite_last_name.is_empty() {
                g.invite_last_name = g.label.clone();
            }
            g
        })
        .collect();

    if groups.is_empty() {
        return Err("No usable rows found. Need at least a party_label.".into());
    }

    Ok((groups, warnings))
}

/// Diff the parsed sheet against the existing parties into a create/update/remove
/// plan. Pure (no DB) so preview and commit share identical logic.
fn reconcile(
    groups: Vec<ParsedGroup>,
    existing: Vec<ExistingParty>,
    mut warnings: Vec<String>,
) -> ReconcilePlan {
    let by_code: HashMap<String, ExistingParty> =
        existing.into_iter().map(|p| (p.invite_code.clone(), p)).collect();
    let mut referenced: HashSet<String> = HashSet::new();
    let mut creates = Vec::new();
    let mut updates = Vec::new();

    for g in groups {
        match g.invite_code.clone() {
            Some(code) if by_code.contains_key(&code) => {
                referenced.insert(code.clone());
                let ex = by_code.get(&code).expect("checked");
                // An empty label cell shouldn't blank out the party name.
                let label = if g.label.is_empty() {
                    ex.label.clone()
                } else {
                    g.label
                };
                updates.push(PlanParty {
                    existing_id: Some(ex.id.clone()),
                    invite_code: Some(code),
                    label,
                    invite_last_name: g.invite_last_name,
                    guests: g.guests,
                });
            }
            Some(code) => {
                warnings.push(format!(
                    "Invite code \"{code}\" (\"{}\") isn't in the database — importing as a brand-new party with a fresh code.",
                    g.label
                ));
                creates.push(PlanParty {
                    existing_id: None,
                    invite_code: None,
                    label: g.label,
                    invite_last_name: g.invite_last_name,
                    guests: g.guests,
                });
            }
            None => creates.push(PlanParty {
                existing_id: None,
                invite_code: None,
                label: g.label,
                invite_last_name: g.invite_last_name,
                guests: g.guests,
            }),
        }
    }

    let mut removes: Vec<(String, String, String)> = by_code
        .into_iter()
        .filter(|(code, _)| !referenced.contains(code))
        .map(|(code, p)| (p.id, code, p.label))
        .collect();
    removes.sort_by(|a, b| a.2.to_ascii_lowercase().cmp(&b.2.to_ascii_lowercase()));

    ReconcilePlan {
        creates,
        updates,
        removes,
        warnings,
    }
}

/// Warn about event-column headers that don't match any event (only those a
/// guest was actually marked invited under — an unused stray column is harmless).
fn unknown_event_warnings(groups: &[ParsedGroup], known: &HashMap<String, String>) -> Vec<String> {
    let mut seen: HashSet<String> = HashSet::new();
    let mut out: Vec<String> = Vec::new();
    for g in groups {
        for guest in &g.guests {
            if let Some(names) = &guest.invited_events {
                for n in names {
                    let key = n.trim().to_ascii_lowercase();
                    if !known.contains_key(&key) && seen.insert(key) {
                        out.push(format!(
                            "Column \"{n}\" doesn't match any event — those invitations were ignored."
                        ));
                    }
                }
            }
        }
    }
    out
}

/// Load the current parties for diffing.
async fn load_existing_parties(pool: &AppState) -> Result<Vec<ExistingParty>, AppError> {
    let rows = sqlx::query_as::<_, (String, String, String)>(
        "SELECT id, invite_code, label FROM parties",
    )
    .fetch_all(pool)
    .await?;
    Ok(rows
        .into_iter()
        .map(|(id, invite_code, label)| ExistingParty {
            id,
            invite_code,
            label,
        })
        .collect())
}

/// Delete a party's guests plus any RSVP history and event invitations pointing
/// at them. Used both when removing a party and when replacing a kept party's
/// guest list. Children go before guests because SQLite enforces FKs here.
async fn clear_party_guests(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    party_id: &str,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "DELETE FROM rsvp_history
         WHERE guest_id IN (SELECT id FROM guests WHERE party_id = ?)
            OR submitted_by IN (SELECT id FROM guests WHERE party_id = ?)",
    )
    .bind(party_id)
    .bind(party_id)
    .execute(&mut **tx)
    .await?;
    sqlx::query(
        "DELETE FROM event_invitations
         WHERE guest_id IN (SELECT id FROM guests WHERE party_id = ?)",
    )
    .bind(party_id)
    .execute(&mut **tx)
    .await?;
    sqlx::query("DELETE FROM guests WHERE party_id = ?")
        .bind(party_id)
        .execute(&mut **tx)
        .await?;
    Ok(())
}

/// All event ids in display order (the default "invited to everything" set).
async fn all_event_ids(pool: &AppState) -> Result<Vec<String>, AppError> {
    Ok(
        sqlx::query_scalar::<_, String>("SELECT id FROM events ORDER BY sort_order")
            .fetch_all(pool)
            .await?,
    )
}

/// Map of lowercased event name -> id, for resolving CSV event-column headers.
async fn event_name_index(pool: &AppState) -> Result<HashMap<String, String>, AppError> {
    let rows = sqlx::query_as::<_, (String, String)>("SELECT id, name FROM events")
        .fetch_all(pool)
        .await?;
    Ok(rows
        .into_iter()
        .map(|(id, name)| (name.trim().to_ascii_lowercase(), id))
        .collect())
}

/// Current (guest_id, event_id) invitations for one party.
async fn fetch_invitation_set(
    pool: &AppState,
    party_id: &str,
) -> Result<HashSet<(String, String)>, AppError> {
    let rows = sqlx::query_as::<_, (String, String)>(
        "SELECT ei.guest_id, ei.event_id
         FROM event_invitations ei
         JOIN guests g ON g.id = ei.guest_id
         WHERE g.party_id = ?",
    )
    .bind(party_id)
    .fetch_all(pool)
    .await?;
    Ok(rows.into_iter().collect())
}

/// Insert invitation rows for one guest (idempotent).
async fn insert_invitations(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    guest_id: &str,
    event_ids: &[String],
) -> Result<(), sqlx::Error> {
    for eid in event_ids {
        sqlx::query("INSERT OR IGNORE INTO event_invitations (guest_id, event_id) VALUES (?, ?)")
            .bind(guest_id)
            .bind(eid)
            .execute(&mut **tx)
            .await?;
    }
    Ok(())
}

/// Resolve a parsed guest's invited-event names to event ids. `None` (no event
/// columns in the sheet) defaults to every event; unknown names are dropped
/// (they're surfaced as warnings during preview/commit).
fn resolved_invites(
    g: &ParsedGuest,
    name_to_id: &HashMap<String, String>,
    all_event_ids: &[String],
) -> Vec<String> {
    match &g.invited_events {
        None => all_event_ids.to_vec(),
        Some(names) => names
            .iter()
            .filter_map(|n| name_to_id.get(&n.trim().to_ascii_lowercase()).cloned())
            .collect(),
    }
}

/// Insert the parsed guests of a party, along with their event invitations.
async fn insert_guests(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    party_id: &str,
    guests: &[ParsedGuest],
    name_to_id: &HashMap<String, String>,
    all_event_ids: &[String],
) -> Result<(), sqlx::Error> {
    for g in guests {
        let guest_id = Uuid::new_v4().to_string();
        sqlx::query(
            "INSERT INTO guests (id, party_id, first_name, last_name, email, is_plus_one)
             VALUES (?, ?, ?, ?, ?, ?)",
        )
        .bind(&guest_id)
        .bind(party_id)
        .bind(&g.first_name)
        .bind(&g.last_name)
        .bind(&g.email)
        .bind(g.is_plus_one)
        .execute(&mut **tx)
        .await?;
        let invited = resolved_invites(g, name_to_id, all_event_ids);
        insert_invitations(tx, &guest_id, &invited).await?;
    }
    Ok(())
}

/// GET /admin/import — the upload/paste page (empty state).
pub async fn import_page(jar: CookieJar) -> Result<Response, AppError> {
    if !require_admin(&jar) {
        return Ok(Redirect::to("/admin/login").into_response());
    }
    Ok(Html(
        ImportTemplate {
            preview: None,
            raw_csv: String::new(),
            error: None,
        }
        .render()?,
    )
    .into_response())
}

/// GET /admin/import/export — download the current guest list as a CSV in the
/// exact shape the importer expects (invite_code included as the stable key).
pub async fn import_export(
    State(pool): State<AppState>,
    jar: CookieJar,
) -> Result<Response, AppError> {
    if !require_admin(&jar) {
        return Ok(Redirect::to("/admin/login").into_response());
    }

    // One column per event (header = event name), so invitations round-trip
    // through the spreadsheet. "x" in a cell = that guest is invited.
    let events = sqlx::query_as::<_, (String, String)>(
        "SELECT id, name FROM events ORDER BY sort_order",
    )
    .fetch_all(&pool)
    .await?;
    let invited: HashSet<(String, String)> =
        sqlx::query_as::<_, (String, String)>("SELECT guest_id, event_id FROM event_invitations")
            .fetch_all(&pool)
            .await?
            .into_iter()
            .collect();

    // LEFT JOIN so a party with no guests still emits one (code-only) row, which
    // keeps it referenced on a round-trip instead of being treated as removed.
    let rows = sqlx::query_as::<_, (String, String, Option<String>, Option<String>, Option<String>, Option<String>, Option<bool>)>(
        "SELECT p.invite_code, p.label, g.id, g.first_name, g.last_name, g.email, g.is_plus_one
         FROM parties p
         LEFT JOIN guests g ON g.party_id = p.id
         ORDER BY p.label, g.is_plus_one, g.created_at",
    )
    .fetch_all(&pool)
    .await?;

    let mut wtr = csv::Writer::from_writer(Vec::new());
    let mut header: Vec<String> = vec![
        "invite_code".into(),
        "party_label".into(),
        "first_name".into(),
        "last_name".into(),
        "email".into(),
        "is_plus_one".into(),
    ];
    header.extend(events.iter().map(|(_, name)| name.clone()));
    wtr.write_record(&header)
        .map_err(|e| AppError::Other(anyhow::anyhow!("csv write error: {e}")))?;
    for (code, label, gid, first, last, email, plus) in rows {
        let mut record: Vec<String> = vec![
            code,
            label,
            first.unwrap_or_default(),
            last.unwrap_or_default(),
            email.unwrap_or_default(),
            if plus.unwrap_or(false) {
                "yes".into()
            } else {
                String::new()
            },
        ];
        for (eid, _) in &events {
            let cell = match &gid {
                Some(g) if invited.contains(&(g.clone(), eid.clone())) => "x",
                _ => "",
            };
            record.push(cell.into());
        }
        wtr.write_record(&record)
            .map_err(|e| AppError::Other(anyhow::anyhow!("csv write error: {e}")))?;
    }
    let bytes = wtr
        .into_inner()
        .map_err(|e| AppError::Other(anyhow::anyhow!("csv flush error: {e}")))?;
    let body = String::from_utf8(bytes)
        .map_err(|e| AppError::Other(anyhow::anyhow!("csv utf8 error: {e}")))?;

    Ok((
        [
            ("content-type", "text/csv; charset=utf-8"),
            (
                "content-disposition",
                "attachment; filename=\"guest-list.csv\"",
            ),
        ],
        body,
    )
        .into_response())
}

/// Read the uploaded file (preferred) or pasted textarea from a multipart form.
async fn read_uploaded_csv(multipart: &mut Multipart) -> Result<String, AppError> {
    let mut file_text = String::new();
    let mut pasted = String::new();
    while let Some(field) = multipart
        .next_field()
        .await
        .map_err(|e| AppError::Other(anyhow::anyhow!("upload error: {e}")))?
    {
        match field.name() {
            Some("file") => {
                file_text = field
                    .text()
                    .await
                    .map_err(|e| AppError::Other(anyhow::anyhow!("could not read file: {e}")))?
            }
            Some("pasted") => {
                pasted = field
                    .text()
                    .await
                    .map_err(|e| AppError::Other(anyhow::anyhow!("could not read text: {e}")))?
            }
            _ => {}
        }
    }
    // An uploaded file wins over the textarea when both are present.
    Ok(if !file_text.trim().is_empty() {
        file_text
    } else {
        pasted
    })
}

/// POST /admin/import/preview — diff the uploaded sheet against the live list and
/// show what would be created, updated, and removed. Nothing is written here.
pub async fn import_preview(
    State(pool): State<AppState>,
    jar: CookieJar,
    mut multipart: Multipart,
) -> Result<Response, AppError> {
    if !require_admin(&jar) {
        return Ok(Redirect::to("/admin/login").into_response());
    }

    let raw_csv = read_uploaded_csv(&mut multipart).await?;

    let template = match parse_import_csv(&raw_csv) {
        Ok((groups, mut warnings)) => {
            let existing = load_existing_parties(&pool).await?;
            let name_to_id = event_name_index(&pool).await?;
            warnings.extend(unknown_event_warnings(&groups, &name_to_id));
            let plan = reconcile(groups, existing, warnings);
            let creates = plan
                .creates
                .into_iter()
                .map(|p| PreviewParty {
                    label: p.label,
                    code: "new code on import".to_string(),
                    guests: p.guests,
                })
                .collect();
            let updates = plan
                .updates
                .into_iter()
                .map(|p| PreviewParty {
                    label: p.label,
                    code: p.invite_code.unwrap_or_default(),
                    guests: p.guests,
                })
                .collect();
            let removes = plan
                .removes
                .into_iter()
                .map(|(_, invite_code, label)| PreviewRemove { label, invite_code })
                .collect();
            ImportTemplate {
                preview: Some(ImportPreview {
                    creates,
                    updates,
                    removes,
                    warnings: plan.warnings,
                }),
                raw_csv,
                error: None,
            }
        }
        Err(msg) => ImportTemplate {
            preview: None,
            raw_csv,
            error: Some(msg),
        },
    };

    Ok(Html(template.render()?).into_response())
}

/// POST /admin/import — re-parse the confirmed CSV, diff it, and apply the plan
/// (remove dropped parties, replace kept parties' guests, create new ones) in a
/// single transaction.
pub async fn import_commit(
    State(pool): State<AppState>,
    jar: CookieJar,
    Form(form): Form<ImportCommitForm>,
) -> Result<Response, AppError> {
    if !require_admin(&jar) {
        return Ok(Redirect::to("/admin/login").into_response());
    }

    let (groups, warnings) = match parse_import_csv(&form.csv) {
        Ok(v) => v,
        // Shouldn't happen (preview already parsed it), but never write garbage.
        Err(msg) => {
            return Ok(Html(
                ImportTemplate {
                    preview: None,
                    raw_csv: form.csv,
                    error: Some(msg),
                }
                .render()?,
            )
            .into_response())
        }
    };

    let existing = load_existing_parties(&pool).await?;
    let plan = reconcile(groups, existing, warnings);
    apply_plan(&pool, plan).await?;

    Ok(Redirect::to("/admin").into_response())
}

/// Apply a reconcile plan in a single transaction: remove dropped parties,
/// replace kept parties' guests, and create new ones.
async fn apply_plan(pool: &AppState, plan: ReconcilePlan) -> Result<(), AppError> {
    // Pre-generate codes for new parties before opening the write transaction
    // (matches create_party). generate_invite_code reads committed rows, so it
    // already avoids kept parties' codes; the in-memory set guards against two
    // new same-last-name parties colliding in one run.
    let mut used: HashSet<String> = HashSet::new();
    let mut create_codes: Vec<String> = Vec::with_capacity(plan.creates.len());
    for c in &plan.creates {
        let mut code = generate_invite_code(pool, &c.invite_last_name).await?;
        while used.contains(&code) {
            code = generate_invite_code(pool, &c.invite_last_name).await?;
        }
        used.insert(code.clone());
        create_codes.push(code);
    }

    // Event lookups for translating the sheet's event columns into invitations.
    let name_to_id = event_name_index(pool).await?;
    let all_ids = all_event_ids(pool).await?;

    let mut tx = pool.begin().await?;

    // Remove parties dropped from the sheet (guests + RSVP history first for FKs).
    for (id, _code, _label) in &plan.removes {
        clear_party_guests(&mut tx, id).await?;
        sqlx::query("DELETE FROM parties WHERE id = ?")
            .bind(id)
            .execute(&mut *tx)
            .await?;
    }

    // Update kept parties: rename + replace their guest list.
    for u in &plan.updates {
        let id = u.existing_id.as_ref().expect("update has existing_id");
        sqlx::query("UPDATE parties SET label = ? WHERE id = ?")
            .bind(&u.label)
            .bind(id)
            .execute(&mut *tx)
            .await?;
        clear_party_guests(&mut tx, id).await?;
        insert_guests(&mut tx, id, &u.guests, &name_to_id, &all_ids).await?;
    }

    // Create new parties with their pre-generated codes.
    for (c, code) in plan.creates.iter().zip(create_codes) {
        let party_id = Uuid::new_v4().to_string();
        sqlx::query("INSERT INTO parties (id, invite_code, label) VALUES (?, ?, ?)")
            .bind(&party_id)
            .bind(&code)
            .bind(&c.label)
            .execute(&mut *tx)
            .await?;
        insert_guests(&mut tx, &party_id, &c.guests, &name_to_id, &all_ids).await?;
    }

    tx.commit().await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
    use std::str::FromStr;

    // ---------- parse_truthy ----------

    #[test]
    fn truthy_accepts_common_yes_values() {
        for s in ["1", "true", "TRUE", "yes", "Yes", "y", "x", "+1", " yes "] {
            assert!(parse_truthy(s), "{s:?} should be truthy");
        }
    }

    #[test]
    fn truthy_rejects_everything_else() {
        for s in ["", "0", "no", "false", "n", "maybe"] {
            assert!(!parse_truthy(s), "{s:?} should be falsy");
        }
    }

    // ---------- parse_import_csv ----------

    /// Find a parsed group by label (parse order is otherwise first-seen).
    fn group<'a>(groups: &'a [ParsedGroup], label: &str) -> &'a ParsedGroup {
        groups
            .iter()
            .find(|g| g.label == label)
            .unwrap_or_else(|| panic!("no group labelled {label:?}"))
    }

    #[test]
    fn empty_input_is_an_error() {
        assert!(parse_import_csv("   \n  ").is_err());
    }

    #[test]
    fn header_only_yields_no_groups() {
        // A header with no data rows parses but has nothing usable.
        let err = parse_import_csv("invite_code,party_label,first_name").unwrap_err();
        assert!(err.contains("No usable rows"), "got: {err}");
    }

    #[test]
    fn rows_group_by_label_and_collect_guests() {
        let csv = "invite_code,party_label,first_name,last_name,email,is_plus_one\n\
                   ,Smith Family,John,Smith,john@x.com,yes\n\
                   ,Smith Family,Jane,Smith,,\n\
                   ,Jones,Bob,Jones,,1\n";
        let (groups, warnings) = parse_import_csv(csv).unwrap();
        assert_eq!(groups.len(), 2);
        assert!(warnings.is_empty());

        let smith = group(&groups, "Smith Family");
        assert!(smith.invite_code.is_none());
        assert_eq!(smith.guests.len(), 2);
        assert_eq!(smith.guests[0].first_name, "John");
        assert!(smith.guests[0].is_plus_one);
        assert_eq!(smith.guests[0].email.as_deref(), Some("john@x.com"));
        assert!(!smith.guests[1].is_plus_one);
        assert!(smith.guests[1].email.is_none());
        // Invite-code prefix seeds from the first non-empty last name.
        assert_eq!(smith.invite_last_name, "Smith");

        let jones = group(&groups, "Jones");
        assert!(jones.guests[0].is_plus_one);
    }

    #[test]
    fn rows_sharing_a_code_merge_even_when_label_edited() {
        // Same code, different label text on the two rows: one group, latest
        // non-empty label wins.
        let csv = "invite_code,party_label,first_name\n\
                   SMITH-7Q2,Smith Family,John\n\
                   SMITH-7Q2,The Smiths,Jane\n";
        let (groups, _) = parse_import_csv(csv).unwrap();
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].invite_code.as_deref(), Some("SMITH-7Q2"));
        assert_eq!(groups[0].label, "The Smiths");
        assert_eq!(groups[0].guests.len(), 2);
    }

    #[test]
    fn code_only_row_keeps_an_empty_party() {
        // A party with no guests still registers (so it isn't treated as removed).
        let csv = "invite_code,party_label,first_name\nEMPTY-1,Lonely Party,\n";
        let (groups, _) = parse_import_csv(csv).unwrap();
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].invite_code.as_deref(), Some("EMPTY-1"));
        assert!(groups[0].guests.is_empty());
    }

    #[test]
    fn blank_and_anchorless_rows_are_skipped_with_warnings() {
        let csv = "invite_code,party_label,first_name\n\
                   ,,\n\
                   ,,Orphan\n\
                   ,Real,Alice\n";
        let (groups, warnings) = parse_import_csv(csv).unwrap();
        // Fully blank row: silent. Anchorless guest row: one warning.
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].label, "Real");
        assert_eq!(warnings.len(), 1);
        assert!(warnings[0].contains("no party_label or invite_code"));
    }

    #[test]
    fn missing_required_column_errors() {
        // No party_label column at all.
        let err = parse_import_csv("invite_code,first_name\nX,John\n").unwrap_err();
        assert!(err.contains("party_label"), "got: {err}");
    }

    #[test]
    fn no_event_columns_leaves_invited_events_none() {
        // Without event columns, guests default to "all events" (None).
        let (groups, _) = parse_import_csv(
            "party_label,first_name\nSmith,John\n",
        )
        .unwrap();
        assert!(groups[0].guests[0].invited_events.is_none());
    }

    #[test]
    fn event_columns_parse_invitations_by_truthy_cell() {
        let csv = "party_label,first_name,Welcome Drinks,Ceremony,Reception\n\
                   Smith,John,x,yes,\n\
                   Smith,Kid,,1,\n";
        let (groups, _) = parse_import_csv(csv).unwrap();
        let g = group(&groups, "Smith");
        // John: Welcome Drinks + Ceremony (Reception blank → not invited).
        assert_eq!(
            g.guests[0].invited_events.as_deref(),
            Some(&["Welcome Drinks".to_string(), "Ceremony".to_string()][..])
        );
        // Kid: Ceremony only.
        assert_eq!(
            g.guests[1].invited_events.as_deref(),
            Some(&["Ceremony".to_string()][..])
        );
    }

    // ---------- reconcile ----------

    fn existing(id: &str, code: &str, label: &str) -> ExistingParty {
        ExistingParty {
            id: id.into(),
            invite_code: code.into(),
            label: label.into(),
        }
    }

    fn parse_only(csv: &str) -> Vec<ParsedGroup> {
        parse_import_csv(csv).unwrap().0
    }

    #[test]
    fn reconcile_sorts_creates_updates_removes() {
        let groups = parse_only(
            "invite_code,party_label,first_name\n\
             SMITH-7Q2,Smith Family,John\n\
             ,The Lees,Amy\n",
        );
        let existing = vec![
            existing("p1", "SMITH-7Q2", "Smith Family"),
            existing("p2", "JONES-QQ", "Jones"), // absent from sheet -> remove
        ];
        let plan = reconcile(groups, existing, vec![]);

        assert_eq!(plan.creates.len(), 1);
        assert_eq!(plan.creates[0].label, "The Lees");
        assert!(plan.creates[0].invite_code.is_none());

        assert_eq!(plan.updates.len(), 1);
        assert_eq!(plan.updates[0].existing_id.as_deref(), Some("p1"));
        assert_eq!(plan.updates[0].invite_code.as_deref(), Some("SMITH-7Q2"));

        assert_eq!(plan.removes.len(), 1);
        assert_eq!(plan.removes[0].0, "p2");
        assert_eq!(plan.removes[0].2, "Jones");
    }

    #[test]
    fn reconcile_unknown_code_becomes_a_create_with_warning() {
        let groups = parse_only("invite_code,party_label,first_name\nGHOST-9,Nobody,Sue\n");
        let plan = reconcile(groups, vec![], vec![]);
        assert_eq!(plan.creates.len(), 1);
        assert!(plan.creates[0].invite_code.is_none()); // fresh code, not the typed one
        assert!(plan.warnings.iter().any(|w| w.contains("GHOST-9")));
    }

    #[test]
    fn reconcile_blank_label_keeps_existing_name() {
        // A code-only row (empty label) must not blank out the party name.
        let groups = parse_only("invite_code,party_label,first_name\nSMITH-7Q2,,\n");
        let plan = reconcile(groups, vec![existing("p1", "SMITH-7Q2", "Smith Family")], vec![]);
        assert_eq!(plan.updates.len(), 1);
        assert_eq!(plan.updates[0].label, "Smith Family");
        assert!(plan.removes.is_empty());
    }

    // ---------- apply_plan (in-memory DB) ----------

    async fn test_pool() -> AppState {
        // foreign_keys(true) mirrors production so the FK-safe deletion path is
        // actually exercised; max_connections(1) keeps the in-memory DB stable.
        let opts = SqliteConnectOptions::from_str("sqlite::memory:")
            .unwrap()
            .foreign_keys(true);
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(opts)
            .await
            .unwrap();
        sqlx::migrate!("./migrations").run(&pool).await.unwrap();
        pool
    }

    /// Full parse → reconcile → apply against the live pool, as import_commit does.
    async fn apply_csv(pool: &AppState, csv: &str) {
        let (groups, warnings) = parse_import_csv(csv).unwrap();
        let existing = load_existing_parties(pool).await.unwrap();
        let plan = reconcile(groups, existing, warnings);
        apply_plan(pool, plan).await.unwrap();
    }

    async fn code_for(pool: &AppState, label: &str) -> String {
        sqlx::query_scalar::<_, String>("SELECT invite_code FROM parties WHERE label = ?")
            .bind(label)
            .fetch_one(pool)
            .await
            .unwrap()
    }

    async fn count(pool: &AppState, sql: &str) -> i64 {
        sqlx::query_scalar::<_, i64>(sql).fetch_one(pool).await.unwrap()
    }

    #[tokio::test]
    async fn commit_creates_then_roundtrip_updates_creates_and_removes() {
        let pool = test_pool().await;

        // Seed two parties (all creates).
        apply_csv(
            &pool,
            "invite_code,party_label,first_name,last_name,is_plus_one\n\
             ,Smith Family,John,Smith,yes\n\
             ,Smith Family,Jane,Smith,\n\
             ,Jones,Bob,Jones,1\n",
        )
        .await;
        assert_eq!(count(&pool, "SELECT COUNT(*) FROM parties").await, 2);
        let smith_code = code_for(&pool, "Smith Family").await;

        // Attach an RSVP to a Jones guest so removal must clear rsvp_history (FK).
        let jones_guest: String = sqlx::query_scalar(
            "SELECT g.id FROM guests g JOIN parties p ON p.id = g.party_id WHERE p.label = 'Jones'",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        let event: String = sqlx::query_scalar("SELECT id FROM events LIMIT 1")
            .fetch_one(&pool)
            .await
            .unwrap();
        sqlx::query(
            "INSERT INTO rsvp_history (id, guest_id, event_id, attending, submitted_by)
             VALUES (?, ?, ?, 1, ?)",
        )
        .bind(Uuid::new_v4().to_string())
        .bind(&jones_guest)
        .bind(&event)
        .bind(&jones_guest)
        .execute(&pool)
        .await
        .unwrap();
        assert_eq!(count(&pool, "SELECT COUNT(*) FROM rsvp_history").await, 1);

        // Edited sheet: keep Smith (by code, Jane->Janet + add Tim), add The Lees,
        // drop Jones.
        let edited = format!(
            "invite_code,party_label,first_name,last_name,email,is_plus_one\n\
             {smith_code},Smith Family,John,Smith,,yes\n\
             {smith_code},Smith Family,Janet,Smith,janet@x.com,\n\
             {smith_code},Smith Family,Tim,Smith,,1\n\
             ,The Lees,Amy,Lee,,\n"
        );
        apply_csv(&pool, &edited).await;

        // Smith updated in place: same code, new guest list.
        assert_eq!(code_for(&pool, "Smith Family").await, smith_code);
        let smith_firsts: Vec<String> = sqlx::query_scalar(
            "SELECT g.first_name FROM guests g JOIN parties p ON p.id = g.party_id
             WHERE p.label = 'Smith Family' ORDER BY g.first_name",
        )
        .fetch_all(&pool)
        .await
        .unwrap();
        assert_eq!(smith_firsts, vec!["Janet", "John", "Tim"]);

        // The Lees created; Jones gone with its RSVP; no orphaned guests.
        assert_eq!(count(&pool, "SELECT COUNT(*) FROM parties").await, 2);
        assert_eq!(count(&pool, "SELECT COUNT(*) FROM parties WHERE label = 'The Lees'").await, 1);
        assert_eq!(count(&pool, "SELECT COUNT(*) FROM parties WHERE label = 'Jones'").await, 0);
        assert_eq!(count(&pool, "SELECT COUNT(*) FROM rsvp_history").await, 0);
        assert_eq!(
            count(
                &pool,
                "SELECT COUNT(*) FROM guests WHERE party_id NOT IN (SELECT id FROM parties)"
            )
            .await,
            0
        );
    }

    #[tokio::test]
    async fn reuploading_an_unchanged_export_is_a_noop() {
        let pool = test_pool().await;
        apply_csv(
            &pool,
            "invite_code,party_label,first_name,last_name\n,Smith Family,John,Smith\n",
        )
        .await;
        let code = code_for(&pool, "Smith Family").await;

        // Re-upload the export verbatim, twice.
        let export = format!(
            "invite_code,party_label,first_name,last_name,email,is_plus_one\n\
             {code},Smith Family,John,Smith,,\n"
        );
        apply_csv(&pool, &export).await;
        apply_csv(&pool, &export).await;

        assert_eq!(count(&pool, "SELECT COUNT(*) FROM parties").await, 1);
        assert_eq!(count(&pool, "SELECT COUNT(*) FROM guests").await, 1);
        assert_eq!(code_for(&pool, "Smith Family").await, code);
    }

    #[tokio::test]
    async fn empty_party_survives_a_roundtrip() {
        let pool = test_pool().await;
        // Create a party with a guest, then export-shaped re-upload as code-only
        // (guest removed in the sheet) keeps the party but empties it — and a
        // second identical upload is stable.
        apply_csv(
            &pool,
            "invite_code,party_label,first_name,last_name\n,Solo,Pat,Solo\n",
        )
        .await;
        let code = code_for(&pool, "Solo").await;

        let code_only = format!("invite_code,party_label,first_name\n{code},Solo,\n");
        apply_csv(&pool, &code_only).await;
        assert_eq!(count(&pool, "SELECT COUNT(*) FROM parties WHERE label = 'Solo'").await, 1);
        assert_eq!(count(&pool, "SELECT COUNT(*) FROM guests").await, 0);

        apply_csv(&pool, &code_only).await;
        assert_eq!(count(&pool, "SELECT COUNT(*) FROM parties WHERE label = 'Solo'").await, 1);
        assert_eq!(code_for(&pool, "Solo").await, code);
    }

    async fn invited_events_of(pool: &AppState, first: &str) -> Vec<String> {
        sqlx::query_scalar::<_, String>(
            "SELECT e.name FROM event_invitations ei
             JOIN events e ON e.id = ei.event_id
             JOIN guests g ON g.id = ei.guest_id
             WHERE g.first_name = ? ORDER BY e.sort_order",
        )
        .bind(first)
        .fetch_all(pool)
        .await
        .unwrap()
    }

    #[tokio::test]
    async fn import_without_event_columns_invites_everyone_to_all_events() {
        let pool = test_pool().await;
        apply_csv(&pool, "party_label,first_name,last_name\nSmith,John,Smith\n").await;
        let total_events = count(&pool, "SELECT COUNT(*) FROM events").await as usize;
        assert_eq!(invited_events_of(&pool, "John").await.len(), total_events);
    }

    #[tokio::test]
    async fn import_event_columns_set_per_guest_invitations() {
        let pool = test_pool().await;
        // Event columns by name; John gets two events, Kid only the Ceremony.
        apply_csv(
            &pool,
            "party_label,first_name,last_name,Welcome Drinks,Ceremony,Reception\n\
             Smith,John,Smith,x,x,\n\
             Smith,Kid,Smith,,x,\n",
        )
        .await;
        assert_eq!(
            invited_events_of(&pool, "John").await,
            vec!["Welcome Drinks", "Ceremony"]
        );
        assert_eq!(invited_events_of(&pool, "Kid").await, vec!["Ceremony"]);

        // Re-importing the same sheet replaces invitations cleanly (no dupes).
        apply_csv(
            &pool,
            "party_label,first_name,last_name,Welcome Drinks,Ceremony,Reception\n\
             Smith,John,Smith,,x,x\n\
             Smith,Kid,Smith,,x,\n",
        )
        .await;
        assert_eq!(
            invited_events_of(&pool, "John").await,
            vec!["Ceremony", "Reception"]
        );
    }
}
