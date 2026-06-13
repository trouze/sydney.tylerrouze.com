use askama::Template;
use axum::{
    extract::{Form, Query, State},
    response::{Html, IntoResponse, Redirect, Response},
};
use axum_extra::extract::cookie::{Cookie, CookieJar, SameSite};
use std::collections::{HashMap, HashSet};
use uuid::Uuid;

use crate::{
    error::AppError,
    models::{Event, Guest, MealOption, Party, RsvpQuery},
    AppState,
};

const INVITE_COOKIE: &str = "invite";

/// A row of current RSVP state: (guest_id, event_id, attending, meal_option_id, dietary_notes).
type CurrentRow = (String, String, Option<bool>, Option<String>, Option<String>);

// ---------- templates ----------

#[derive(Template)]
#[template(path = "rsvp_gate.html")]
struct GateTemplate {
    error: Option<String>,
}

#[derive(Template)]
#[template(path = "rsvp.html")]
struct RsvpTemplate {
    party_label: String,
    guests: Vec<GuestRow>,
    meal_options: Vec<MealOption>,
    serves_meal: bool,
}

struct GuestRow {
    id: String,
    name: String,
    meal_choice: String, // selected meal_option_id, "" if none
    dietary: String,
    events: Vec<EventCell>,
}

struct EventCell {
    event_id: String,
    event_name: String,
    attending: bool,
}

// ---------- GET /rsvp ----------

pub async fn rsvp_page(
    State(pool): State<AppState>,
    jar: CookieJar,
    Query(params): Query<RsvpQuery>,
) -> Result<Response, AppError> {
    // A code in the URL (scanned QR or gate form) authenticates the party,
    // sets the session cookie, and redirects to the clean /rsvp URL.
    if let Some(code) = params.code {
        let code = code.trim();
        return Ok(match find_party(&pool, code).await? {
            Some(_) => {
                let cookie = Cookie::build((INVITE_COOKIE, code.to_string()))
                    .http_only(true)
                    .same_site(SameSite::Lax)
                    .path("/")
                    .build();
                (jar.add(cookie), Redirect::to("/rsvp")).into_response()
            }
            None => Html(
                GateTemplate {
                    error: Some(
                        "We couldn't find that invite code. Check your invitation and try again."
                            .into(),
                    ),
                }
                .render()?,
            )
            .into_response(),
        });
    }

    // Otherwise the cookie decides: valid party -> form, else the gate.
    if let Some(code) = jar.get(INVITE_COOKIE).map(|c| c.value().to_string()) {
        if let Some(party) = find_party(&pool, &code).await? {
            return Ok(Html(build_form(&pool, &party).await?).into_response());
        }
    }

    Ok(Html(GateTemplate { error: None }.render()?).into_response())
}

// ---------- POST /rsvp ----------

pub async fn rsvp_submit(
    State(pool): State<AppState>,
    jar: CookieJar,
    Form(fields): Form<Vec<(String, String)>>,
) -> Result<Html<String>, AppError> {
    // Re-validate the party from the cookie on every write -- never trust the
    // client to tell us which party it is.
    let party = match jar.get(INVITE_COOKIE).map(|c| c.value().to_string()) {
        Some(code) => find_party(&pool, &code).await?,
        None => None,
    };
    let Some(party) = party else {
        return Ok(Html(session_expired_html()));
    };

    let guests = fetch_guests(&pool, &party.id).await?;
    let events = fetch_events(&pool).await?;

    // Parse the dynamic form. Checkboxes use name="attend" value="guest:event";
    // selects/inputs use "meal:<guest>" / "diet:<guest>"; plus one "message".
    let mut attend: HashSet<(String, String)> = HashSet::new();
    let mut meal: HashMap<String, String> = HashMap::new();
    let mut diet: HashMap<String, String> = HashMap::new();
    let mut message = String::new();
    for (key, value) in fields {
        if key == "attend" {
            if let Some((g, e)) = value.split_once(':') {
                attend.insert((g.to_string(), e.to_string()));
            }
        } else if let Some(g) = key.strip_prefix("meal:") {
            if !value.is_empty() {
                meal.insert(g.to_string(), value);
            }
        } else if let Some(g) = key.strip_prefix("diet:") {
            if !value.trim().is_empty() {
                diet.insert(g.to_string(), value);
            }
        } else if key == "message" {
            message = value;
        }
    }
    let message = message.trim();
    let message = (!message.is_empty()).then(|| message.to_string());

    // Append one row per guest x event. Meal/dietary/message ride on the
    // meal-serving event rows where they're meaningful.
    let mut tx = pool.begin().await?;
    for g in &guests {
        for e in &events {
            let attending = attend.contains(&(g.id.clone(), e.id.clone()));
            let (meal_id, dietary, msg) = if e.serves_meal {
                (
                    meal.get(&g.id).cloned(),
                    diet.get(&g.id).cloned(),
                    message.clone(),
                )
            } else {
                (None, None, None)
            };
            sqlx::query(
                "INSERT INTO rsvp_history
                   (id, guest_id, event_id, submitted_by, attending, meal_option_id, dietary_notes, message)
                 VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
            )
            .bind(Uuid::new_v4().to_string())
            .bind(&g.id)
            .bind(&e.id)
            .bind(Option::<String>::None) // party-login: individual editor unknown
            .bind(attending)
            .bind(meal_id)
            .bind(dietary)
            .bind(msg)
            .execute(&mut *tx)
            .await?;
        }
    }
    tx.commit().await?;

    Ok(Html(confirmation_html(&party.label, !attend.is_empty())))
}

// ---------- data access ----------

async fn find_party(pool: &AppState, code: &str) -> Result<Option<Party>, AppError> {
    Ok(
        sqlx::query_as::<_, Party>("SELECT id, label FROM parties WHERE invite_code = ?")
            .bind(code)
            .fetch_optional(pool)
            .await?,
    )
}

async fn fetch_guests(pool: &AppState, party_id: &str) -> Result<Vec<Guest>, AppError> {
    Ok(sqlx::query_as::<_, Guest>(
        "SELECT id, first_name, last_name FROM guests
         WHERE party_id = ? ORDER BY is_plus_one, created_at",
    )
    .bind(party_id)
    .fetch_all(pool)
    .await?)
}

async fn fetch_events(pool: &AppState) -> Result<Vec<Event>, AppError> {
    Ok(
        sqlx::query_as::<_, Event>("SELECT id, name, serves_meal FROM events ORDER BY sort_order")
            .fetch_all(pool)
            .await?,
    )
}

/// Render the matrix form, pre-filled from each guest's current RSVP.
async fn build_form(pool: &AppState, party: &Party) -> Result<String, AppError> {
    let guests = fetch_guests(pool, &party.id).await?;
    let events = fetch_events(pool).await?;
    let meal_options = sqlx::query_as::<_, MealOption>(
        "SELECT id, label FROM meal_options WHERE is_active = 1 ORDER BY sort_order",
    )
    .fetch_all(pool)
    .await?;

    // Current state = latest row per (guest, event) for this party.
    let current: Vec<CurrentRow> = sqlx::query_as(
        "SELECT cr.guest_id, cr.event_id, cr.attending, cr.meal_option_id, cr.dietary_notes
             FROM current_rsvp cr
             JOIN guests g ON g.id = cr.guest_id
             WHERE g.party_id = ?",
    )
    .bind(&party.id)
    .fetch_all(pool)
    .await?;

    let mut attend: HashSet<(String, String)> = HashSet::new();
    let mut meal: HashMap<String, String> = HashMap::new();
    let mut diet: HashMap<String, String> = HashMap::new();
    for (guest_id, event_id, attending, meal_id, dietary) in current {
        if attending == Some(true) {
            attend.insert((guest_id.clone(), event_id));
        }
        if let Some(m) = meal_id {
            meal.entry(guest_id.clone()).or_insert(m);
        }
        if let Some(d) = dietary {
            diet.entry(guest_id).or_insert(d);
        }
    }

    let serves_meal = events.iter().any(|e| e.serves_meal);
    let guest_rows = guests
        .iter()
        .map(|g| GuestRow {
            id: g.id.clone(),
            name: format!("{} {}", g.first_name, g.last_name),
            meal_choice: meal.get(&g.id).cloned().unwrap_or_default(),
            dietary: diet.get(&g.id).cloned().unwrap_or_default(),
            events: events
                .iter()
                .map(|e| EventCell {
                    event_id: e.id.clone(),
                    event_name: e.name.clone(),
                    attending: attend.contains(&(g.id.clone(), e.id.clone())),
                })
                .collect(),
        })
        .collect();

    Ok(RsvpTemplate {
        party_label: party.label.clone(),
        guests: guest_rows,
        meal_options,
        serves_meal,
    }
    .render()?)
}

// ---------- inline HTML fragments (htmx swaps #rsvp-form) ----------

fn confirmation_html(party_label: &str, attending: bool) -> String {
    let label = html_escape(party_label);
    if attending {
        format!(
            r#"<div class="text-center py-12">
  <p class="text-5xl mb-6">&#127881;</p>
  <h3 class="font-display text-3xl text-stone-800 mb-3">We can&apos;t wait to celebrate with you!</h3>
  <p class="text-stone-500">Thank you, {label}. Your RSVP has been saved. You can return any time before the deadline to update it.</p>
</div>"#
        )
    } else {
        format!(
            r#"<div class="text-center py-12">
  <p class="text-5xl mb-6">&#128140;</p>
  <h3 class="font-display text-3xl text-stone-800 mb-3">We&apos;ll miss you!</h3>
  <p class="text-stone-500">Thank you for letting us know, {label}.</p>
</div>"#
        )
    }
}

fn session_expired_html() -> String {
    r#"<div class="text-center py-12">
  <p class="text-5xl mb-6">&#128274;</p>
  <h3 class="font-display text-3xl text-stone-800 mb-3">Your session expired</h3>
  <p class="text-stone-500 mb-6">Please re-open your invite link or enter your code again.</p>
  <a href="/rsvp" class="inline-block py-3 px-8 text-xs tracking-widest uppercase bg-stone-800 text-white hover:bg-stone-700 transition-colors">Back to RSVP</a>
</div>"#
        .to_string()
}

fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}
