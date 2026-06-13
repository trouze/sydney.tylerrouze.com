---
name: registry-scrape
description: Diagnose and repair the MyRegistry public-page scrape (the /registry page) when MyRegistry changes their markup and gifts stop showing. Use when the registry page is empty, missing items, or showing wrong prices/images/purchase state.
---

# Fixing the registry scrape

The `/registry` page scrapes the couple's **public** MyRegistry giftlist page (no
official API — that's partner-gated). When MyRegistry changes their HTML, the
parse can silently degrade: the page falls back to a plain "View Our Registry"
link, shows stale items, or shows wrong prices/images/purchase badges.

**All scrape logic lives in `src/registry.rs`.** The handler is
`src/handlers/registry.rs`; the markup is `templates/registry.html`. Nothing else
is involved.

## How the scrape works (what to re-derive)

The fetcher pulls two things out of the giftlist HTML:

1. **JSON-LD (primary)** — a `<script type="application/ld+json">` block of
   `@type: CollectionPage`. Per gift, under
   `mainEntity.itemListElement[].item`:
   - `name` — gift title
   - `itemOffered.offers.price` + `.priceCurrency`
   - `itemOffered.offers.seller.name` — store
   - `itemOffered.offers.url` — outbound purchase link (GetLink.ashx)
   - `itemOffered.offers.availability` — contains `InStock` / `OutOfStock`
   - `itemOffered.image` — image URL
   - `url` — contains `giftid=NNN` (used to join to purchase state)
   Parsed by `extract_jsonld_itemlist()` + `parse_gifts()`.
2. **HTML attributes (purchase state only)** — each gift is a
   `<div class="itemGiftVisitorList" giftid="NNN" ispurchased="true|false">`.
   Parsed by `purchased_map()`, joined to gifts by `giftid`.

## Repair procedure

### 1. Reproduce and confirm it's the scrape
Run the unit tests and a live fetch:
```bash
cargo test registry
DATABASE_URL="sqlite::memory:" cargo run &  # then: curl -s localhost:8080/registry | grep -i 'price\|View &amp; Buy\|More to come'
```
- Tests fail → the parser logic / fixture is stale. Go to step 3.
- Tests pass but live page is empty/wrong → MyRegistry's live markup changed
  vs. the test fixture. Capture a fresh page (step 2) and compare.

### 2. Capture the current live page
Ask the user to open the public registry in a browser and either **Save Page As
> HTML** or export a **HAR** (DevTools > Network > right-click > Save all as
HAR), then point you at the file. Or fetch it yourself:
```bash
curl -s -A "Mozilla/5.0 (compatible; wedding-site/1.0)" \
  "https://www.myregistry.com/wedding-registry/tyler-rouze-and-sydney-norcross-austin-tx/5485797/giftlist" > /tmp/giftlist.html
```
(The registry URL is `DEFAULT_URL` in `src/registry.rs`, overridable via the
`REGISTRY_URL` env var. If the couple's registry slug/ID changed, that's the fix
— update `DEFAULT_URL` and the two hard-coded links in `templates/registry.html`.)

If working from a HAR, extract the giftlist HTML response body:
```bash
jq -r '.log.entries[] | select(.request.url | endswith("/giftlist")) | .response.content.text' yourfile.har > /tmp/giftlist.html
```

### 3. Inspect what changed
Check the JSON-LD still exists and still has the expected shape:
```bash
python3 - <<'PY'
import re, json
html = open('/tmp/giftlist.html', encoding='utf-8', errors='replace').read()
blocks = re.findall(r'application/ld\+json[^>]*>(.*?)</script>', html, re.S)
print("ld+json blocks:", len(blocks))
for b in blocks:
    try:
        v = json.loads(b.strip())
    except Exception as e:
        print("  unparseable:", e); continue
    items = v.get('mainEntity', {}).get('itemListElement')
    if isinstance(items, list):
        print("  ItemList with", len(items), "items; first item:")
        print(json.dumps(items[0], indent=2)[:1200])
PY
# purchase-state divs:
grep -o 'itemGiftVisitorList[^>]*' /tmp/giftlist.html | head
```
Compare the field paths printed here against what `parse_gifts()` reads. Common
breakages:
- JSON-LD block removed entirely → switch primary parse to the HTML
  `itemGiftVisitorList` divs (title in `.gift-title`, price in `.gift-price`,
  image in `.gift-image-container` background-image url, link in `.gift-viewOrBuy`).
- Field renamed/moved (e.g. `seller.name` → `seller.legalName`) → update the
  path in `parse_gifts()`.
- `ispurchased`/`giftid` attribute renamed → update `purchased_map()`.

### 4. Update parser + fixture + tests
- Fix the field paths in `src/registry.rs::parse_gifts()` (and helpers).
- Update the `PAGE` fixture constant in the `#[cfg(test)] mod tests` to a
  trimmed copy of the new real markup (one or two gifts is enough — include one
  purchased and one not, one in-stock and one out).
- Adjust assertions in the tests to match.

### 5. Verify
```bash
cargo test registry          # all parser tests green
DATABASE_URL="sqlite::memory:" cargo run &   # then curl /registry and eyeball cards
```
Confirm: gift names, prices (with `$`/thousands separators), images, store
names, "Purchased" badges, and "View & Buy" links all render correctly.

## Notes / invariants
- **Never block or fail the page on a scrape error.** `gifts()` serves the
  last-good 10-min cache on a failed refetch; an empty result must fall back to
  the "View Our Registry" link in the template. Keep that behavior.
- `parse_gifts()` is pure (HTML string in, `Vec<Gift>` out) so it's unit-testable
  against a saved page — keep it that way.
- We deliberately do NOT mark gifts as purchased or hit any write endpoint;
  MyRegistry owns the purchase flow. We only display.
- TTL is `TTL` (10 min) in `src/registry.rs`. Cache is per-process, in-memory.
