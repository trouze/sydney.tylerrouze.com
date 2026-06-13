//! Best-effort sync of RSVP contact emails into a listmonk mailing list.
//!
//! The instance is shared with other sites, so we must never replace a
//! subscriber's full list membership (that's what `PATCH .../subscribers` with a
//! `lists` array does). Instead we create new subscribers with the wedding list
//! pre-confirmed, and for already-existing subscribers we add them to the list
//! via `PUT /api/subscribers/lists` (`action: "add"`), which preserves their
//! other subscriptions.
//!
//! Configured entirely through env vars; if any of URL/USER/TOKEN is missing the
//! integration is disabled and every call is a no-op (handy for dev and tests):
//!   LISTMONK_URL      e.g. https://mailing.tylerrouze.com
//!   LISTMONK_USER     API username
//!   LISTMONK_TOKEN    API access token
//!   LISTMONK_LIST_ID  target list id (defaults to 4)

use std::time::Duration;

use serde_json::json;

#[derive(Clone)]
pub struct Listmonk {
    base_url: String,
    user: String,
    token: String,
    list_id: i64,
    client: reqwest::Client,
}

/// A contact to sync: an email, the name to store, and the events that guest is
/// currently attending (surfaced to listmonk as the `events_attending` attrib).
#[derive(Clone, Debug)]
pub struct Contact {
    pub name: String,
    pub email: String,
    pub events_attending: Vec<String>,
}

/// Build the SQL `query` expression listmonk uses to find a subscriber by email,
/// escaping single quotes so an address can't break out of the literal.
fn email_query(email: &str) -> String {
    format!("subscribers.email = '{}'", email.replace('\'', "''"))
}

impl Listmonk {
    /// Construct from env vars, or `None` if the integration isn't configured.
    pub fn from_env() -> Option<Self> {
        let nonempty = |k: &str| std::env::var(k).ok().filter(|v| !v.trim().is_empty());
        let base_url = nonempty("LISTMONK_URL")?;
        let user = nonempty("LISTMONK_USER")?;
        let token = nonempty("LISTMONK_TOKEN")?;
        let list_id = nonempty("LISTMONK_LIST_ID")
            .and_then(|v| v.trim().parse().ok())
            .unwrap_or(4);
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(10))
            .build()
            .ok()?;
        Some(Self {
            base_url: base_url.trim_end_matches('/').to_string(),
            user,
            token,
            list_id,
            client,
        })
    }

    /// Ensure the contact is a confirmed subscriber on the wedding list with an
    /// up-to-date `events_attending` attribute, creating the subscriber first if
    /// listmonk doesn't already know them. Never disturbs the subscriber's
    /// membership in other lists, nor their other attributes.
    pub async fn add_to_list(&self, contact: &Contact) -> anyhow::Result<()> {
        let resp = self
            .client
            .post(format!("{}/api/subscribers", self.base_url))
            .basic_auth(&self.user, Some(&self.token))
            .json(&json!({
                "email": contact.email,
                "name": contact.name,
                "status": "enabled",
                "lists": [self.list_id],
                "preconfirm_subscriptions": true,
                "attribs": { "events_attending": contact.events_attending },
            }))
            .send()
            .await?;

        if resp.status().is_success() {
            return Ok(());
        }
        // Anything other than "already exists" is a real failure.
        if resp.status() != reqwest::StatusCode::CONFLICT {
            let code = resp.status();
            let body = resp.text().await.unwrap_or_default();
            anyhow::bail!("listmonk create subscriber failed ({code}): {body}");
        }

        // Already a subscriber (from a prior RSVP, or another site's list). Fetch
        // them so we can preserve their other attributes when updating.
        let (id, mut attribs) = self.find_subscriber(&contact.email).await?.ok_or_else(|| {
            anyhow::anyhow!("listmonk says {} exists but lookup returned nothing", contact.email)
        })?;

        // Add to the wedding list without touching existing memberships.
        let resp = self
            .client
            .put(format!("{}/api/subscribers/lists", self.base_url))
            .basic_auth(&self.user, Some(&self.token))
            .json(&json!({
                "ids": [id],
                "action": "add",
                "target_list_ids": [self.list_id],
                "status": "confirmed",
            }))
            .send()
            .await?;
        if !resp.status().is_success() {
            let code = resp.status();
            let body = resp.text().await.unwrap_or_default();
            anyhow::bail!("listmonk add-to-list failed ({code}): {body}");
        }

        // Refresh events_attending, leaving any other attribs (e.g. from another
        // site) intact. A bare PATCH updates only the fields we send.
        if !attribs.is_object() {
            attribs = json!({});
        }
        attribs["events_attending"] = json!(contact.events_attending);
        let resp = self
            .client
            .patch(format!("{}/api/subscribers/{}", self.base_url, id))
            .basic_auth(&self.user, Some(&self.token))
            .json(&json!({ "attribs": attribs }))
            .send()
            .await?;
        if !resp.status().is_success() {
            let code = resp.status();
            let body = resp.text().await.unwrap_or_default();
            anyhow::bail!("listmonk attrib update failed ({code}): {body}");
        }
        Ok(())
    }

    /// Look up a subscriber by email, returning `(id, attribs)`; `None` if not
    /// found. `attribs` is whatever listmonk currently stores (possibly null).
    async fn find_subscriber(&self, email: &str) -> anyhow::Result<Option<(i64, serde_json::Value)>> {
        let body: serde_json::Value = self
            .client
            .get(format!("{}/api/subscribers", self.base_url))
            .basic_auth(&self.user, Some(&self.token))
            .query(&[("query", email_query(email).as_str()), ("per_page", "1")])
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?;
        let Some(first) = body["data"]["results"].get(0) else {
            return Ok(None);
        };
        let Some(id) = first["id"].as_i64() else {
            return Ok(None);
        };
        Ok(Some((id, first["attribs"].clone())))
    }
}

/// Fire-and-forget sync of contacts onto the wedding list.
///
/// Best-effort by design: it returns immediately, runs in the background, and
/// only logs failures — a slow or down listmonk must never block or fail an
/// RSVP. A no-op when the integration isn't configured.
pub fn sync_contacts(contacts: Vec<Contact>) {
    let Some(lm) = Listmonk::from_env() else {
        return;
    };
    tokio::spawn(async move {
        for contact in contacts {
            match lm.add_to_list(&contact).await {
                Ok(()) => tracing::debug!("listmonk: synced {} to list {}", contact.email, lm.list_id),
                Err(e) => tracing::warn!("listmonk: failed to sync {}: {e:#}", contact.email),
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{
        extract::State,
        http::StatusCode,
        routing::{patch, post, put},
        Json, Router,
    };
    use std::sync::{Arc, Mutex};

    fn contact(name: &str, email: &str, events: &[&str]) -> Contact {
        Contact {
            name: name.into(),
            email: email.into(),
            events_attending: events.iter().map(|s| s.to_string()).collect(),
        }
    }

    #[test]
    fn email_query_escapes_single_quotes() {
        assert_eq!(email_query("a@b.com"), "subscribers.email = 'a@b.com'");
        // An apostrophe in the local part must be doubled, not left to break the
        // SQL string literal.
        assert_eq!(
            email_query("o'brien@x.com"),
            "subscribers.email = 'o''brien@x.com'"
        );
    }

    // ---------- contract tests against an in-process mock listmonk ----------

    #[derive(Clone)]
    struct MockState {
        calls: Arc<Mutex<Calls>>,
        post_status: u16,
        existing_id: Option<i64>,
    }

    #[derive(Default)]
    struct Calls {
        posts: Vec<serde_json::Value>,
        puts: Vec<serde_json::Value>,
        patches: Vec<serde_json::Value>,
    }

    async fn mock_post(
        State(s): State<MockState>,
        Json(body): Json<serde_json::Value>,
    ) -> StatusCode {
        s.calls.lock().unwrap().posts.push(body);
        StatusCode::from_u16(s.post_status).unwrap()
    }

    async fn mock_get(State(s): State<MockState>) -> Json<serde_json::Value> {
        // Existing subscriber carries a pre-existing attrib from "another site"
        // so tests can prove the merge preserves it.
        let results = match s.existing_id {
            Some(id) => json!([{ "id": id, "attribs": { "site": "personal" } }]),
            None => json!([]),
        };
        Json(json!({ "data": { "results": results } }))
    }

    async fn mock_put(
        State(s): State<MockState>,
        Json(body): Json<serde_json::Value>,
    ) -> StatusCode {
        s.calls.lock().unwrap().puts.push(body);
        StatusCode::OK
    }

    async fn mock_patch(
        State(s): State<MockState>,
        Json(body): Json<serde_json::Value>,
    ) -> StatusCode {
        s.calls.lock().unwrap().patches.push(body);
        StatusCode::OK
    }

    /// Spin up a mock listmonk and return a `Listmonk` client pointed at it.
    async fn mock_listmonk(post_status: u16, existing_id: Option<i64>) -> (Listmonk, Arc<Mutex<Calls>>) {
        let calls = Arc::new(Mutex::new(Calls::default()));
        let state = MockState {
            calls: calls.clone(),
            post_status,
            existing_id,
        };
        let app = Router::new()
            .route("/api/subscribers", post(mock_post).get(mock_get))
            .route("/api/subscribers/lists", put(mock_put))
            .route("/api/subscribers/:id", patch(mock_patch))
            .with_state(state);
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        let lm = Listmonk {
            base_url: format!("http://{addr}"),
            user: "u".into(),
            token: "t".into(),
            list_id: 4,
            client: reqwest::Client::new(),
        };
        (lm, calls)
    }

    #[tokio::test]
    async fn new_subscriber_is_created_on_the_list_with_events() {
        let (lm, calls) = mock_listmonk(201, None).await;
        lm.add_to_list(&contact("Jane Smith", "jane@x.com", &["Ceremony", "Reception"]))
            .await
            .unwrap();

        let calls = calls.lock().unwrap();
        assert_eq!(calls.posts.len(), 1, "should POST once");
        assert!(calls.puts.is_empty(), "no list-add needed for a new subscriber");
        assert!(calls.patches.is_empty(), "no attrib merge needed for a new subscriber");
        let body = &calls.posts[0];
        assert_eq!(body["email"], "jane@x.com");
        assert_eq!(body["name"], "Jane Smith");
        assert_eq!(body["lists"], json!([4]));
        assert_eq!(body["preconfirm_subscriptions"], json!(true));
        assert_eq!(body["attribs"]["events_attending"], json!(["Ceremony", "Reception"]));
    }

    #[tokio::test]
    async fn existing_subscriber_is_added_and_events_merged_without_clobbering() {
        // POST 409 -> look up id+attribs -> PUT add to list -> PATCH merged attribs.
        let (lm, calls) = mock_listmonk(409, Some(7)).await;
        lm.add_to_list(&contact("Bob Jones", "bob@x.com", &["Ceremony"]))
            .await
            .unwrap();

        let calls = calls.lock().unwrap();
        assert_eq!(calls.posts.len(), 1, "still attempts the create first");
        assert_eq!(calls.puts.len(), 1, "adds to the list without replacing memberships");
        let put = &calls.puts[0];
        assert_eq!(put["ids"], json!([7]));
        assert_eq!(put["action"], "add");
        assert_eq!(put["target_list_ids"], json!([4]));
        assert_eq!(put["status"], "confirmed");

        assert_eq!(calls.patches.len(), 1, "refreshes events_attending");
        let attribs = &calls.patches[0]["attribs"];
        assert_eq!(attribs["events_attending"], json!(["Ceremony"]));
        // The unrelated attrib from "another site" must survive the update.
        assert_eq!(attribs["site"], "personal");
    }

    #[tokio::test]
    async fn server_error_propagates() {
        // A non-409 failure must surface as an error (so it's logged, not silent).
        let (lm, _calls) = mock_listmonk(500, None).await;
        assert!(lm.add_to_list(&contact("X", "x@x.com", &[])).await.is_err());
    }
}
