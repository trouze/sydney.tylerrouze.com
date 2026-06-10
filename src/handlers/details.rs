use askama::Template;
use axum::response::Html;

#[derive(Template)]
#[template(path = "details.html")]
struct DetailsTemplate {}

pub async fn details() -> Html<String> {
    Html(DetailsTemplate {}.render().unwrap())
}
