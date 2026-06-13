use askama::Template;
use axum::response::Html;

use crate::registry::Gift;

#[derive(Template)]
#[template(path = "registry.html")]
struct RegistryTemplate {
    gifts: Vec<Gift>,
}

pub async fn registry() -> Html<String> {
    let gifts = crate::registry::gifts().await;
    Html(RegistryTemplate { gifts }.render().unwrap())
}
