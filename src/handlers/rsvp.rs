use askama::Template;
use axum::{
    extract::{Form, State},
    response::Html,
};
use uuid::Uuid;

use crate::{error::AppError, models::RsvpForm, AppState};

#[derive(Template)]
#[template(path = "rsvp.html")]
struct RsvpTemplate {}

pub async fn rsvp_page() -> Html<String> {
    Html(RsvpTemplate {}.render().unwrap())
}

pub async fn rsvp_submit(
    State(pool): State<AppState>,
    Form(form): Form<RsvpForm>,
) -> Result<Html<String>, AppError> {
    let id = Uuid::new_v4().to_string();
    let attending = form.attending == "yes";
    let guest_count = form.guest_count.unwrap_or(1);
    let meal_choice = form.meal_choice.filter(|s| !s.is_empty());

    sqlx::query(
        "INSERT INTO rsvps (id, name, email, attending, guest_count, meal_choice, dietary_restrictions, message)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(&id)
    .bind(&form.name)
    .bind(&form.email)
    .bind(attending)
    .bind(guest_count)
    .bind(&meal_choice)
    .bind(&form.dietary_restrictions)
    .bind(&form.message)
    .execute(&pool)
    .await?;

    let safe_name = html_escape(&form.name);
    let html = if attending {
        format!(
            r#"<div class="text-center py-12">
  <p class="text-5xl mb-6">&#127881;</p>
  <h3 class="font-display text-3xl text-stone-800 mb-3">We can&apos;t wait to see you!</h3>
  <p class="text-stone-500">Thank you, {safe_name}. Your RSVP has been received.</p>
</div>"#
        )
    } else {
        format!(
            r#"<div class="text-center py-12">
  <p class="text-5xl mb-6">&#128140;</p>
  <h3 class="font-display text-3xl text-stone-800 mb-3">We&apos;ll miss you!</h3>
  <p class="text-stone-500">Thank you for letting us know, {safe_name}.</p>
</div>"#
        )
    };

    Ok(Html(html))
}

fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}
