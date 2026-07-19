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

/// Full market item list via search/render (USD). Simplified single-pass fetch.
pub async fn fetch_all_items(appid: i64, currency_code: u32) -> anyhow::Result<Value> {
    let info = currency::get(currency_code).unwrap_or(currency::Currency {
        iso: "USD", symbol: "$", decimals: 2, name: "US Dollar", country: "US",
    });
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
            let price_cents = currency::sell_price_to_cents(sell_price, info.decimals as i32);
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

/// Order book for one hash.
pub async fn fetch_orderbook(appid: i64, hash: &str, currency_code: u32) -> anyhow::Result<Value> {
    let qp = enc(&format!("[{appid},\"{hash}\"]"));
    let url = format!("{STEAM_ORIGIN}/market/orderbook?q=Load&qp={qp}&currency={currency_code}");
    let referer = format!("{STEAM_ORIGIN}/market/listings/{appid}/{}", enc(hash));
    let j = get_json(&url, Some(&referer)).await?;
    let max_buy = j.get("highest_buy_order").and_then(parse_cents).unwrap_or(0);
    let min_sell = j.get("lowest_sell_order").and_then(parse_cents).unwrap_or(0);
    let buy_count = j.get("buy_order_count").and_then(parse_int).unwrap_or(0);
    let sell_count = j.get("sell_order_count").and_then(parse_int).unwrap_or(0);
    let info = currency::get(currency_code).unwrap();
    Ok(json!({
        "hash": hash, "maxBuyCents": max_buy, "minSellCents": min_sell,
        "buyCount": buy_count, "sellCount": sell_count,
        "currency": currency_code, "symbol": info.symbol,
        "liquidity": pricing::classify_liquidity(buy_count),
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
