//! Steam Community Market + news client. Port of the network parts of `server.mjs`.
//! Anonymous, read-only. All prices in the main-unit×100 convention.

use crate::currency;
use crate::pricing;
use crate::save::TBH_APPID;
use once_cell::sync::Lazy;
use serde_json::{json, Value};
use std::time::Duration;

const STEAM_ORIGIN: &str = "https://steamcommunity.com";
const UA_POOL: &[&str] = &[
    "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/124.0.0.0 Safari/537.36",
    "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/17.4 Safari/605.1.15",
    "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/122.0.0.0 Safari/537.36",
    "Mozilla/5.0 (Windows NT 10.0; Win64; x64; rv:125.0) Gecko/20100101 Firefox/125.0",
];

static CLIENT: Lazy<reqwest::Client> = Lazy::new(|| {
    reqwest::Client::builder()
        .timeout(Duration::from_secs(15))
        .gzip(true)
        .build()
        .unwrap()
});

fn ua() -> &'static str {
    // Deterministic rotation without RNG (Math.random is unavailable in the reference anyway).
    let n = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis()).unwrap_or(0);
    UA_POOL[(n as usize) % UA_POOL.len()]
}

fn enc(s: &str) -> String {
    percent_encoding::utf8_percent_encode(s, percent_encoding::NON_ALPHANUMERIC).to_string()
}

async fn get_text(url: &str, referer: Option<&str>) -> anyhow::Result<String> {
    let mut req = CLIENT
        .get(url)
        .header("User-Agent", ua())
        .header("Accept", "*/*")
        .header("Origin", STEAM_ORIGIN)
        .header("Referer", referer.unwrap_or(STEAM_ORIGIN));
    let _ = &mut req;
    let resp = req.send().await?;
    if resp.status().as_u16() == 429 {
        anyhow::bail!("429");
    }
    Ok(resp.text().await?)
}

async fn get_json(url: &str, referer: Option<&str>) -> anyhow::Result<Value> {
    let txt = get_text(url, referer).await?;
    Ok(serde_json::from_str(&txt)?)
}

/// Full market item list via search/render.
///
/// NOTE: search/render **ignores** the currency parameter and always answers in USD, where
/// `sell_price` is USD minor units (cents). Our internal convention is always main-unit x 100,
/// so converting to the display currency is a single multiply by the USD->local rate —
/// independent of that currency's decimal count. Scaling by the *display* currency's decimals
/// (as an earlier revision did) silently produced wrong prices for every non-USD user.
pub async fn fetch_all_items(appid: i64, currency_code: u32) -> anyhow::Result<Value> {
    let info = currency::get(currency_code).unwrap_or(currency::Currency {
        iso: "USD", symbol: "$", decimals: 2, name: "US Dollar", country: "US",
    });
    let rate = if info.iso == "USD" { 1.0 } else { usd_to_local(info.iso).await };
    let mut items: Vec<Value> = Vec::new();
    let mut start = 0i64;
    for _page in 0..40 {
        let url = format!(
            "{STEAM_ORIGIN}/market/search/render/?appid={appid}&norender=1&count=100&start={start}&sort_column=price&sort_dir=desc&currency=1"
        );
        let j = match get_json(&url, None).await {
            Ok(v) => v,
            Err(_) => break,
        };
        let results = j.get("results").and_then(|v| v.as_array()).cloned().unwrap_or_default();
        if results.is_empty() { break; }
        for r in &results {
            let name = r.get("hash_name").and_then(|v| v.as_str()).unwrap_or("").to_string();
            let sell_price = r.get("sell_price").and_then(|v| v.as_f64()).unwrap_or(0.0);
            let listings = r.get("sell_listings").and_then(|v| v.as_i64()).unwrap_or(0);
            let icon = r.get("asset_description").and_then(|a| a.get("icon_url")).and_then(|v| v.as_str())
                .map(|u| format!("https://community.fastly.steamstatic.com/economy/image/{u}/96fx96f"))
                .unwrap_or_default();
            let ty = r.get("asset_description").and_then(|a| a.get("type")).and_then(|v| v.as_str()).unwrap_or("").to_string();
            // sell_price is USD cents (already main x 100); convert straight to the display currency.
            let price_cents = ((sell_price * rate).round() as i64).max(1);
            items.push(json!({
                "name": name, "hash": name, "priceCents": price_cents,
                "priceText": Value::Null, "listings": listings, "type": ty, "color": "", "icon": icon,
                "url": format!("{STEAM_ORIGIN}/market/listings/{appid}/{}", enc(&name)),
                "hasMarketListing": true,
            }));
        }
        let got = results.len() as i64;
        start += got;
        if got < 10 { break; }
        tokio::time::sleep(Duration::from_millis(1800)).await;
    }
    items.sort_by(|a, b| b["priceCents"].as_i64().unwrap_or(0).cmp(&a["priceCents"].as_i64().unwrap_or(0)));
    Ok(json!({
        "appid": appid, "fetchedAt": now_ms(), "total": items.len(),
        "currency": { "code": currency_code, "symbol": info.symbol, "decimals": info.decimals, "iso": info.iso, "name": info.name, "country": info.country },
        "items": items, "stale": false,
    }))
}

// ── item_nameid resolution ──────────────────────────────────────────────────
// Steam's order book is only addressable by `item_nameid`, which is embedded in the listing
// page as `Market_LoadOrderSpread(<id>)`. The mapping never changes for an item, so it is
// cached in memory and persisted to disk.
static NAMEID_CACHE: Lazy<std::sync::Mutex<std::collections::HashMap<String, i64>>> =
    Lazy::new(|| std::sync::Mutex::new(std::collections::HashMap::new()));
static DATA_DIR: once_cell::sync::OnceCell<std::path::PathBuf> = once_cell::sync::OnceCell::new();

pub fn set_data_dir(p: std::path::PathBuf) {
    let _ = DATA_DIR.set(p);
    // warm the persisted cache
    if let Some(dir) = DATA_DIR.get() {
        if let Ok(txt) = std::fs::read_to_string(dir.join("cache/nameid-cache.json")) {
            if let Ok(m) = serde_json::from_str::<std::collections::HashMap<String, i64>>(&txt) {
                *NAMEID_CACHE.lock().unwrap() = m;
            }
        }
    }
}

fn persist_nameids() {
    if let Some(dir) = DATA_DIR.get() {
        let _ = std::fs::create_dir_all(dir.join("cache"));
        let map = NAMEID_CACHE.lock().unwrap().clone();
        if let Ok(s) = serde_json::to_string(&map) {
            let _ = std::fs::write(dir.join("cache/nameid-cache.json"), s);
        }
    }
}

pub async fn item_name_id(appid: i64, hash: &str) -> anyhow::Result<i64> {
    let key = format!("{appid}|{hash}");
    if let Some(v) = NAMEID_CACHE.lock().unwrap().get(&key) { return Ok(*v); }
    let url = format!("{STEAM_ORIGIN}/market/listings/{appid}/{}", enc(hash));
    let html = get_text(&url, None).await?;
    let id = pricing::parse_name_id(&html)
        .ok_or_else(|| anyhow::anyhow!("item_nameid not found for {hash}"))?;
    NAMEID_CACHE.lock().unwrap().insert(key, id);
    persist_nameids();
    Ok(id)
}

/// Order book for one hash.
///
/// Uses the market's compact order-book endpoint, which works anonymously and needs no
/// `item_nameid`. The payload nests everything under `data`:
///   { success, data: { amtMaxBuyOrder, amtMinSellOrder, eCurrency, cBuyOrders, cSellOrders,
///                      rgCompactBuyOrders: [price, qty, ...], rgCompactSellOrders: [...] } }
/// Amounts are minor units of `eCurrency`; rescale into our main-unit x 100 convention.
pub async fn fetch_orderbook(appid: i64, hash: &str, currency_code: u32) -> anyhow::Result<Value> {
    let qp = enc(&serde_json::to_string(&json!([appid, hash]))?);
    let url = format!("{STEAM_ORIGIN}/market/orderbook?q=Load&qp={qp}&currency={currency_code}");
    let referer = format!("{STEAM_ORIGIN}/market/listings/{appid}/{}", enc(hash));
    let j = get_json(&url, Some(&referer)).await?;
    let d = j.get("data").unwrap_or(&Value::Null);

    // Trust the currency Steam actually answered in, not the one we asked for.
    let eff_code = d.get("eCurrency").and_then(|v| v.as_u64()).unwrap_or(currency_code as u64) as u32;
    let info = currency::get(eff_code).unwrap_or(currency::Currency {
        iso: "USD", symbol: "$", decimals: 2, name: "US Dollar", country: "US",
    });
    let to_cents = |v: Option<i64>| -> i64 {
        v.map(|n| currency::sell_price_to_cents(n as f64, info.decimals as i32)).unwrap_or(0)
    };
    let max_buy = to_cents(d.get("amtMaxBuyOrder").and_then(parse_cents));
    let min_sell = to_cents(d.get("amtMinSellOrder").and_then(parse_cents));
    let buy_count = d.get("cBuyOrders").and_then(parse_int).unwrap_or(0);
    let sell_count = d.get("cSellOrders").and_then(parse_int).unwrap_or(0);

    // Flat [price, qty, price, qty, ...] depth arrays.
    let depth = |k: &str| -> Vec<Value> {
        d.get(k)
            .and_then(|v| v.as_array())
            .map(|a| {
                a.chunks(2)
                    .filter(|c| c.len() == 2)
                    .map(|c| {
                        json!({
                            "priceCents": to_cents(c[0].as_i64()),
                            "qty": c[1].as_i64().unwrap_or(0),
                        })
                    })
                    .collect()
            })
            .unwrap_or_default()
    };

    Ok(json!({
        "hash": hash, "maxBuyCents": max_buy, "minSellCents": min_sell,
        "buyCount": buy_count, "sellCount": sell_count,
        "currency": eff_code, "symbol": info.symbol,
        "liquidity": pricing::classify_liquidity(buy_count),
        "buyDepth": depth("rgCompactBuyOrders"),
        "sellDepth": depth("rgCompactSellOrders"),
    }))
}

/// Price history for a hash (from the listing HTML).
pub async fn fetch_price_history(appid: i64, hash: &str) -> anyhow::Result<Vec<pricing::PricePoint>> {
    let url = format!("{STEAM_ORIGIN}/market/listings/{appid}/{}", enc(hash));
    let html = get_text(&url, None).await?;
    pricing::parse_price_history(&html).ok_or_else(|| anyhow::anyhow!("no history"))
}

/// Resolve a display name to its market_hash_name.
pub async fn resolve_hash_by_name(appid: i64, name: &str) -> anyhow::Result<Option<String>> {
    let url = format!("{STEAM_ORIGIN}/market/search/render/?appid={appid}&norender=1&count=10&start=0&query={}", enc(name));
    let j = get_json(&url, None).await?;
    Ok(pricing::parse_search_render(&j, Some(name)))
}

/// Live FX (Frankfurter) → USD→local rate for a currency.
pub async fn usd_to_local(iso: &str) -> f64 {
    let url = "https://api.frankfurter.dev/v1/latest?base=USD";
    if let Ok(j) = get_json(url, None).await {
        if let Some(map) = currency::parse_frankfurter(&j) {
            if let Some(r) = currency::rate_for_iso(Some(&map), iso) {
                return r;
            }
        }
    }
    currency::rate_for_iso(None, iso).unwrap_or(1.0)
}

/// Updates: SteamDB patch notes RSS + Steam store news.
pub async fn fetch_updates(lang_cc: &str, lang_l: &str) -> anyhow::Result<Value> {
    let pn_url = format!("https://steamdb.info/api/PatchnotesRSS/?appid={TBH_APPID}");
    let patchnotes = match get_text(&pn_url, None).await {
        Ok(xml) => crate::news::parse_patchnotes(&xml),
        Err(_) => Vec::new(),
    };
    let news_url = format!("https://store.steampowered.com/feeds/news/app/{TBH_APPID}/?cc={lang_cc}&l={lang_l}");
    let news = match get_text(&news_url, None).await {
        Ok(xml) => crate::news::parse_store_news_rss(&xml),
        Err(_) => {
            let alt = format!("https://api.steampowered.com/ISteamNews/GetNewsForApp/v2/?appid={TBH_APPID}&count=20&maxlength=0");
            match get_json(&alt, None).await {
                Ok(j) => crate::news::parse_steam_news(&j),
                Err(_) => Vec::new(),
            }
        }
    };
    Ok(json!({
        "ok": !patchnotes.is_empty() || !news.is_empty(),
        "lang": format!("{lang_cc}"), "cc": lang_cc,
        "patchnotes": patchnotes, "news": news, "stale": false,
    }))
}

fn parse_cents(v: &Value) -> Option<i64> {
    match v {
        Value::Number(n) => n.as_i64(),
        Value::String(s) => {
            let digits: String = s.chars().filter(|c| c.is_ascii_digit()).collect();
            digits.parse::<i64>().ok()
        }
        _ => None,
    }
}
fn parse_int(v: &Value) -> Option<i64> {
    match v {
        Value::Number(n) => n.as_i64(),
        Value::String(s) => s.replace(',', "").parse::<i64>().ok(),
        _ => None,
    }
}
fn now_ms() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as i64).unwrap_or(0)
}
