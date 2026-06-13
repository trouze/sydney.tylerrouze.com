use askama::Template;
use axum::response::Html;

#[derive(Template)]
#[template(path = "story.html")]
struct StoryTemplate {}

pub async fn story() -> Html<String> {
    Html(StoryTemplate {}.render().unwrap())
}
