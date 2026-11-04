//! Runtime feature flags, driven by environment variables ("1"/"true"/"yes"/"on").
//!
//! Pre-launch staging: the landing page and /admin are always served; the
//! pages we haven't committed to yet (story, details, registry, rsvp) are
//! only mounted when `all_pages_enabled()` is true.

/// Whether the full suite of public pages is served. Driven by `ALL_PAGES`.
/// Unset (production) keeps the site pared down to the landing page; set it
/// locally (`ALL_PAGES=1 cargo run`) to work on the rest of the pages.
pub fn all_pages_enabled() -> bool {
    truthy("ALL_PAGES")
}

fn truthy(name: &str) -> bool {
    matches!(
        std::env::var(name).map(|v| v.to_ascii_lowercase()),
        Ok(ref v) if matches!(v.as_str(), "1" | "true" | "yes" | "on")
    )
}
