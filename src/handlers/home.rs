use askama::Template;
use axum::response::Html;

#[derive(Template)]
#[template(path = "home.html")]
struct HomeTemplate {}

pub async fn home() -> Html<String> {
    Html(HomeTemplate {}.render().unwrap())
}
