//! Read-only scrape of the couple's public MyRegistry wedding registry.
//!
//! We do NOT use MyRegistry's developer API (it's gated behind merchant/partner
//! approval). Instead we fetch the public giftlist page and pull two things out:
//!
//!   1. A `<script type="application/ld+json">` schema.org `CollectionPage`
//!      block — the primary source. It carries each gift's name, price, image,
//!      store, availability, and a purchase URL. It's MyRegistry's SEO output,
//!      so it's the most stable target (less likely to churn than CSS classes).
//!   2. The `<div class="itemGiftVisitorList" giftid="..." ispurchased="...">`
//!      attributes in the HTML — used only to mark a gift as already purchased
//!      (the JSON-LD quantity fields are ambiguous about that).
//!
//! MyRegistry still owns the actual purchase / mark-as-purchased flow: each
//! gift's button links out to their site. We only display.
//!
//! Results are cached for 10 minutes. A refresh that fails (MyRegistry slow,
//! down, markup changed) never blanks the page — we serve the last good cache,
//! and the handler falls back to a plain "View Our Registry" link when there
//! are no gifts at all.
//!
//! THIS IS A SCRAPE — if MyRegistry changes their markup it can break. See
//! `.claude/skills/registry-scrape.md` for how to re-derive the parse.
//!
//! Config (env, all optional):
//!   REGISTRY_URL  full public giftlist URL; defaults to the couple's registry.

use std::collections::HashMap;
use std::sync::OnceLock;
use std::time::{Duration, Instant};

use serde::Serialize;
use tokio::sync::RwLock;

/// Public giftlist page used when `REGISTRY_URL` isn't set.
const DEFAULT_URL: &str = "https://www.myregistry.com/wedding-registry/tyler-rouze-and-sydney-norcross-austin-tx/5485797/giftlist";

/// How long a successful scrape is reused before we refetch.
const TTL: Duration = Duration::from_secs(600);

/// One registry item, shaped for the template.
#[derive(Clone, Debug, Serialize, PartialEq)]
pub struct Gift {
    pub name: String,
    /// Preformatted for display, e.g. "$795" or "$1,295.50"; empty if unknown.
    pub price_display: String,
    pub image: String,
    /// Store/seller name, e.g. "theessential.com".
    pub store: String,
    /// Outbound link to MyRegistry where the gift is actually purchased.
    pub url: String,
    pub purchased: bool,
    pub in_stock: bool,
}

struct Cache {
    at: Instant,
    gifts: Vec<Gift>,
}

fn cache() -> &'static RwLock<Option<Cache>> {
    static CACHE: OnceLock<RwLock<Option<Cache>>> = OnceLock::new();
    CACHE.get_or_init(|| RwLock::new(None))
}

/// Current registry gifts: a fresh cached copy when available, otherwise a
/// refetch. On a failed refetch returns the last good cache (or empty).
pub async fn gifts() -> Vec<Gift> {
    {
        let guard = cache().read().await;
        if let Some(c) = guard.as_ref() {
            if c.at.elapsed() < TTL {
                return c.gifts.clone();
            }
        }
    }
    match fetch_and_parse().await {
        Ok(gifts) => {
            let mut guard = cache().write().await;
            *guard = Some(Cache {
                at: Instant::now(),
                gifts: gifts.clone(),
            });
            gifts
        }
        Err(e) => {
            tracing::warn!("registry: refresh failed, serving stale cache if any: {e:#}");
            let guard = cache().read().await;
            guard.as_ref().map(|c| c.gifts.clone()).unwrap_or_default()
        }
    }
}

async fn fetch_and_parse() -> anyhow::Result<Vec<Gift>> {
    let url = std::env::var("REGISTRY_URL")
        .ok()
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(|| DEFAULT_URL.to_string());
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        // MyRegistry serves a stripped page to obvious bots; look like a browser.
        .user_agent("Mozilla/5.0 (compatible; wedding-site/1.0; +registry display)")
        .build()?;
    let html = client
        .get(&url)
        .send()
        .await?
        .error_for_status()?
        .text()
        .await?;
    parse_gifts(&html)
}

/// Pure parse so it can be unit-tested against a saved page. Errors only when no
/// JSON-LD ItemList is present at all (the signal that the markup changed).
fn parse_gifts(html: &str) -> anyhow::Result<Vec<Gift>> {
    let ld = extract_jsonld_itemlist(html)
        .ok_or_else(|| anyhow::anyhow!("no JSON-LD ItemList found (markup may have changed)"))?;
    let purchased = purchased_map(html);

    let mut gifts = Vec::new();
    let items = ld["mainEntity"]["itemListElement"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    for entry in items {
        let item = &entry["item"];
        let offered = &item["itemOffered"];
        let offers = &offered["offers"];

        let name = item["name"].as_str().unwrap_or("").trim().to_string();
        if name.is_empty() {
            continue;
        }
        let price = offers["price"].as_f64();
        let currency = offers["priceCurrency"].as_str().unwrap_or("USD");
        let store = offers["seller"]["name"].as_str().unwrap_or("").to_string();
        let image = offered["image"].as_str().unwrap_or("").to_string();
        let in_stock = offers["availability"]
            .as_str()
            .map(|s| s.contains("InStock"))
            .unwrap_or(true);
        // Prefer the offer's GetLink URL (cleaner outbound), fall back to item url.
        let url = offers["url"]
            .as_str()
            .or_else(|| item["url"].as_str())
            .unwrap_or("")
            .to_string();
        // Cross-reference purchase status by gift id parsed from the item url.
        let purchased = item["url"]
            .as_str()
            .and_then(extract_giftid)
            .and_then(|gid| purchased.get(&gid).copied())
            .unwrap_or(false);

        gifts.push(Gift {
            name,
            price_display: format_price(price, currency),
            image,
            store,
            url,
            purchased,
            in_stock,
        });
    }
    Ok(gifts)
}

/// Scan all `application/ld+json` blocks, returning the first whose
/// `mainEntity.itemListElement` is an array (the registry item list).
fn extract_jsonld_itemlist(html: &str) -> Option<serde_json::Value> {
    let mut search = html;
    loop {
        let idx = search.find("application/ld+json")?;
        let after = &search[idx..];
        let gt = after.find('>')?;
        let body_start = idx + gt + 1;
        let rest = &search[body_start..];
        let end = rest.find("</script>")?;
        let json_str = rest[..end].trim();
        if let Ok(v) = serde_json::from_str::<serde_json::Value>(json_str) {
            if v["mainEntity"]["itemListElement"].is_array() {
                return Some(v);
            }
        }
        search = &search[body_start + end..];
    }
}

/// Map of `giftid` -> purchased, read from the `itemGiftVisitorList` div tags.
fn purchased_map(html: &str) -> HashMap<u64, bool> {
    let mut map = HashMap::new();
    let mut search = html;
    while let Some(idx) = search.find("itemGiftVisitorList") {
        let rest = &search[idx..];
        let tag_end = rest.find('>').unwrap_or(rest.len());
        let tag = &rest[..tag_end];
        if let Some(gid) = attr_value(tag, "giftid").and_then(|s| s.parse::<u64>().ok()) {
            let purchased = attr_value(tag, "ispurchased")
                .map(|s| s.eq_ignore_ascii_case("true"))
                .unwrap_or(false);
            map.insert(gid, purchased);
        }
        // Advance past this tag (guard against a tag with no '>').
        let step = (tag_end + 1).min(rest.len()).max(1);
        search = &rest[step..];
    }
    map
}

/// Read `attr="value"` out of an HTML opening-tag slice.
fn attr_value(tag: &str, attr: &str) -> Option<String> {
    let needle = format!("{attr}=\"");
    let i = tag.find(&needle)? + needle.len();
    let rest = &tag[i..];
    let end = rest.find('"')?;
    Some(rest[..end].to_string())
}

/// Pull the numeric `giftid`/`giftId` out of a MyRegistry URL.
fn extract_giftid(url: &str) -> Option<u64> {
    let lower = url.to_ascii_lowercase();
    let i = lower.find("giftid=")? + "giftid=".len();
    let rest = &url[i..];
    let end = rest
        .find(|c: char| !c.is_ascii_digit())
        .unwrap_or(rest.len());
    rest[..end].parse().ok()
}

/// "$795", "$1,295.50", "EUR 40" — empty string when price is unknown.
fn format_price(price: Option<f64>, currency: &str) -> String {
    let Some(p) = price else {
        return String::new();
    };
    let whole = p.trunc().abs() as u64;
    let cents = (p.fract().abs() * 100.0).round() as u64;
    let grouped = group_thousands(whole);
    let prefix = if currency.eq_ignore_ascii_case("USD") {
        "$".to_string()
    } else {
        format!("{currency} ")
    };
    if cents == 0 {
        format!("{prefix}{grouped}")
    } else {
        format!("{prefix}{grouped}.{cents:02}")
    }
}

fn group_thousands(n: u64) -> String {
    let digits = n.to_string();
    let bytes = digits.as_bytes();
    let mut out = String::with_capacity(digits.len() + digits.len() / 3);
    for (i, b) in bytes.iter().enumerate() {
        if i > 0 && (bytes.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(*b as char);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    // A trimmed-down copy of the real MyRegistry giftlist page: one JSON-LD
    // ItemList + the matching itemGiftVisitorList div. If a future MyRegistry
    // change breaks the parse, update this fixture from a fresh page/HAR and
    // re-derive the parser (see .claude/skills/registry-scrape.md).
    const PAGE: &str = r##"
<html><head>
<script type="application/ld+json">
{ "@context":"https://schema.org", "@type":"WebSite" }
</script>
<script type="application/ld+json">
{
  "@context": "https://schema.org",
  "@type": "CollectionPage",
  "mainEntity": {
    "@type": "ItemList",
    "identifier": "5485797",
    "itemListElement": [
      {
        "@type": "ListItem",
        "position": 1,
        "item": {
          "@type": "Demand",
          "name": "Lounge Chair | The Essential Store",
          "url": "https://www.myregistry.com/Visitors/Giftlist/PurchaseAssistant.aspx?registryId=5485797&giftid=173677524",
          "itemOffered": {
            "@type": "Product",
            "name": "Lounge Chair | The Essential Store",
            "offers": {
              "@type": "Offer",
              "seller": { "@type": "OnlineStore", "name": "theessential.com" },
              "price": 795.0,
              "priceCurrency": "USD",
              "url": "https://www.myregistry.com/GetLink.ashx?giftId=173677524&mr_apsa=1",
              "availability": "https://schema.org/InStock"
            },
            "image": "https://stmr.blob.core.windows.net/users/x/GiftImages/abc_Large.jpg"
          }
        }
      },
      {
        "@type": "ListItem",
        "position": 2,
        "item": {
          "@type": "Demand",
          "name": "Stand Mixer",
          "url": "https://www.myregistry.com/Visitors/Giftlist/PurchaseAssistant.aspx?registryId=5485797&giftid=99",
          "itemOffered": {
            "@type": "Product",
            "name": "Stand Mixer",
            "offers": {
              "@type": "Offer",
              "seller": { "@type": "OnlineStore", "name": "williams-sonoma.com" },
              "price": 1295.5,
              "priceCurrency": "USD",
              "url": "https://www.myregistry.com/GetLink.ashx?giftId=99",
              "availability": "https://schema.org/OutOfStock"
            },
            "image": "https://example.com/mixer.jpg"
          }
        }
      }
    ]
  }
}
</script>
</head><body>
<div class="itemGiftVisitorList   " ispurchased="false"
    giftid="173677524" isoffline="false" giftsurprisegroupid="">
</div>
<div class="itemGiftVisitorList   " ispurchased="true"
    giftid="99" isoffline="false" giftsurprisegroupid="">
</div>
</body></html>
"##;

    #[test]
    fn parses_gifts_from_jsonld() {
        let gifts = parse_gifts(PAGE).unwrap();
        assert_eq!(gifts.len(), 2);

        let chair = &gifts[0];
        assert_eq!(chair.name, "Lounge Chair | The Essential Store");
        assert_eq!(chair.price_display, "$795");
        assert_eq!(chair.store, "theessential.com");
        assert_eq!(
            chair.url,
            "https://www.myregistry.com/GetLink.ashx?giftId=173677524&mr_apsa=1"
        );
        assert!(chair.in_stock);
        assert!(!chair.purchased, "chair has ispurchased=false");

        let mixer = &gifts[1];
        assert_eq!(mixer.price_display, "$1,295.50", "cents + thousands sep");
        assert!(!mixer.in_stock, "mixer is OutOfStock");
        assert!(mixer.purchased, "mixer has ispurchased=true");
    }

    #[test]
    fn skips_the_non_itemlist_jsonld_block() {
        // The page has a WebSite JSON-LD block first; we must pick the ItemList.
        let ld = extract_jsonld_itemlist(PAGE).unwrap();
        assert_eq!(ld["mainEntity"]["identifier"], "5485797");
    }

    #[test]
    fn missing_jsonld_is_an_error_not_a_panic() {
        assert!(parse_gifts("<html>no ld json here</html>").is_err());
    }

    #[test]
    fn empty_itemlist_yields_no_gifts() {
        let page = r#"<script type="application/ld+json">
            {"mainEntity":{"itemListElement":[]}}</script>"#;
        assert_eq!(parse_gifts(page).unwrap().len(), 0);
    }

    #[test]
    fn giftid_parsed_case_insensitively() {
        assert_eq!(extract_giftid("a?giftId=123&x=1"), Some(123));
        assert_eq!(extract_giftid("a?giftid=456"), Some(456));
        assert_eq!(extract_giftid("a?nope=1"), None);
    }

    #[test]
    fn giftid_attr_not_confused_with_groupid() {
        let tag =
            r#"itemGiftVisitorList" ispurchased="false" giftid="173677524" giftsurprisegroupid="""#;
        assert_eq!(attr_value(tag, "giftid").as_deref(), Some("173677524"));
    }

    #[test]
    fn price_formats() {
        assert_eq!(format_price(Some(795.0), "USD"), "$795");
        assert_eq!(format_price(Some(1295.5), "USD"), "$1,295.50");
        assert_eq!(format_price(Some(40.0), "EUR"), "EUR 40");
        assert_eq!(format_price(None, "USD"), "");
        assert_eq!(format_price(Some(1000000.0), "USD"), "$1,000,000");
    }
}
