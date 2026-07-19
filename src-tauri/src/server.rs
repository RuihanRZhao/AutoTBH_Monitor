//! Embedded axum HTTP server on 127.0.0.1:5260. Mirrors the `/api/*` contract of the original
//! Node backend. Serves the bundled Nuxt SPA as the static root (SPA fallback → 200.html).

use crate::{currency, farm, meter::Meter, pricing, save, steam, wiki};
use axum::{
    extract::{Query, State},
    http::StatusCode,
    response::IntoResponse,
    routing::{get, post},
    Json, Router,
};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;
use tower_http::cors::CorsLayer;
use tower_http::services::{ServeDir, ServeFile};

/// Listen port. `TSM_PORT` overrides the default (matches the original backend).
pub fn port() -> u16 {
    std::env::var("TSM_PORT").ok().and_then(|v| v.parse().ok()).unwrap_or(5260)
}

#[derive(Clone)]
pub struct AppState {
    pub data_dir: PathBuf,     // bundled seeds + engine snapshots
    pub frontend_dir: PathBuf, // Nuxt .output/public
    pub currency: Arc<AtomicU32>,
    pub meter: Meter,          // built-in live DPS/gold/EXP + run tracker
    pub scan: Arc<std::sync::Mutex<ScanState>>, // Deep Scan progress
}

/// Background "Deep Scan" state: probes every stash entry's order book.
#[derive(Default)]
pub struct ScanState {
    pub status: String, // idle | running | done | error
    pub total: usize,
    pub done: usize,
    pub items: Vec<Value>,
    pub total_instant_cents: i64,
    pub total_suggested_cents: i64,
    pub error: Option<String>,
}

type Q = HashMap<String, String>;

fn read_bundled(state: &AppState, rel: &str) -> Option<Value> {
    let p = state.data_dir.join(rel);
    std::fs::read_to_string(p).ok().and_then(|s| serde_json::from_str(&s).ok())
}

pub fn router(state: AppState) -> Router {
    let frontend = state.frontend_dir.clone();
    let spa_fallback = ServeFile::new(frontend.join("200.html"));
    let static_svc = ServeDir::new(&frontend).fallback(spa_fallback);

    Router::new()
        .route("/__tsm-ping", get(|| async { "tbh-steam-market" }))
        .route("/api/version", get(h_version))
        .route("/api/currency", get(h_currency))
        .route("/api/codex", get(h_codex))
        .route("/api/crafting", get(h_crafting))
        .route("/api/rune-tree", get(h_rune_tree))
        .route("/api/farm-stages", get(h_farm_stages))
        .route("/api/save-mtime", get(h_save_mtime))
        .route("/api/stash", get(h_stash))
        .route("/api/stash-tabs", get(h_stash_tabs))
        .route("/api/stash-orders", get(h_stash_orders))
        .route("/api/stash-scan", get(h_stash_scan))
        .route("/api/wiki-item", get(h_wiki_item))
        .route("/api/farm-calibration", get(h_farm_calibration))
        .route("/api/runs", get(h_runs))
        .route("/api/runs/reset", post(h_runs_reset))
        .route("/api/insights", get(h_insights))
        .route("/api/upgrades", get(h_upgrades))
        .route("/api/meter", get(h_meter))
        .route("/api/meter/status", get(h_meter_status))
        .route("/api/meter/enable", post(h_meter_enable))
        .route("/api/items", get(h_items))
        .route("/api/orderbook", get(h_orderbook))
        .route("/api/market-depth", get(h_market_depth))
        .route("/api/pricehistory", get(h_pricehistory))
        .route("/api/hover", get(h_hover))
        .route("/api/resolve-hash", get(h_resolve_hash))
        .route("/api/updates", get(h_updates))
        .fallback_service(static_svc)
        .layer(CorsLayer::permissive())
        .with_state(state)
}

// ── Bundled / offline data ──────────────────────────────────────────────────
async fn h_version(State(s): State<AppState>) -> impl IntoResponse {
    Json(read_bundled(&s, "engine/version.json").unwrap_or(json!({ "version": "1.22.4" })))
}
async fn h_codex(State(s): State<AppState>) -> impl IntoResponse {
    Json(read_bundled(&s, "engine/codex.json").unwrap_or(json!({ "ok": false, "monsters": [], "stages": [] })))
}
async fn h_crafting(State(s): State<AppState>) -> impl IntoResponse {
    Json(read_bundled(&s, "engine/crafting.json").unwrap_or(json!({ "ok": false, "craft": [] })))
}
async fn h_rune_tree(State(s): State<AppState>) -> impl IntoResponse {
    Json(read_bundled(&s, "engine/rune_tree.json").unwrap_or(json!([])))
}
async fn h_farm_stages(State(s): State<AppState>) -> impl IntoResponse {
    Json(read_bundled(&s, "engine/farm_stages.json").unwrap_or(json!([])))
}

// ── Currency (in-memory state) ──────────────────────────────────────────────
async fn h_currency(State(s): State<AppState>, Query(q): Query<Q>) -> impl IntoResponse {
    if let Some(set) = q.get("set") {
        if set == "auto" {
            // keep current; a full impl would clear the override + re-detect.
        } else if let Ok(code) = set.parse::<u32>() {
            if currency::get(code).is_some() {
                s.currency.store(code, Ordering::Relaxed);
            } else {
                return Json(json!({ "ok": false, "error": "unknown currency code" }));
            }
        }
    }
    let code = s.currency.load(Ordering::Relaxed);
    Json(json!({
        "ok": true, "code": code, "info": currency::info_json(code),
        "auto": false, "list": currency::list_json(),
    }))
}

// ── Game-save backed (offline) ──────────────────────────────────────────────
async fn h_save_mtime() -> impl IntoResponse {
    Json(json!({ "mtime": save::save_mtime() }))
}
async fn h_stash(State(s): State<AppState>, Query(q): Query<Q>) -> impl IntoResponse {
    let appid: i64 = q.get("appid").and_then(|v| v.parse().ok()).unwrap_or(save::TBH_APPID);
    if appid != save::TBH_APPID {
        return Json(json!({ "supported": false }));
    }
    if !save::save_exists() {
        return Json(json!({ "supported": true, "found": false }));
    }
    // Cross-reference the last cached market list if present.
    let market = read_bundled(&s, "cache/items.json")
        .and_then(|v| v.get("items").cloned())
        .and_then(|v| v.as_array().cloned())
        .unwrap_or_default();
    match save::read_stash(&market) {
        Ok(mut stash) => {
            let code = s.currency.load(Ordering::Relaxed);
            // Localized names / icons / rarity colours from the wiki catalog.
            let lang = q.get("lang").map(|s| s.as_str()).unwrap_or("en-US");
            if let Some(cat) = wiki::ensure_catalog(&s.data_dir).await {
                if let Some(items) = stash.get_mut("items").and_then(|v| v.as_array_mut()) {
                    wiki::enrich_items(&cat, items, lang);
                }
            }
            let mut out = json!({ "supported": true, "found": true, "currency": currency::info_json(code) });
            if let (Some(o), Some(st)) = (out.as_object_mut(), stash.as_object()) {
                for (k, v) in st { o.insert(k.clone(), v.clone()); }
            }
            Json(out)
        }
        Err(e) => Json(json!({ "supported": true, "found": true, "error": e.to_string() })),
    }
}
async fn h_stash_tabs() -> impl IntoResponse {
    if !save::save_exists() { return Json(json!({ "found": false })); }
    match save::read_tabs() {
        Ok(t) => {
            let mut out = json!({ "found": true });
            if let (Some(o), Some(st)) = (out.as_object_mut(), t.as_object()) {
                for (k, v) in st { o.insert(k.clone(), v.clone()); }
            }
            Json(out)
        }
        Err(e) => Json(json!({ "found": true, "error": e.to_string() })),
    }
}
/// Farm calibration from the built-in meter's recorded runs.
async fn h_farm_calibration(State(s): State<AppState>, Query(q): Query<Q>) -> impl IntoResponse {
    let runs = s.meter.inner.lock().unwrap().runs.clone();
    let mut opts = farm::AggregateOpts::default();
    if let Some(days) = q.get("days").and_then(|v| v.parse::<f64>().ok()) {
        if days > 0.0 { opts.max_age_ms = days * 86_400_000.0; }
    }
    Json(farm::aggregate_runs_for_farm(&runs, &opts))
}
async fn h_runs(State(s): State<AppState>) -> impl IntoResponse {
    Json(s.meter.runs_json())
}
async fn h_runs_reset(State(s): State<AppState>, Query(q): Query<Q>) -> impl IntoResponse {
    // Guard: require ?confirm=1 (archives rather than deletes).
    if q.get("confirm").map(|v| v.as_str()) != Some("1") {
        return (StatusCode::BAD_REQUEST, Json(json!({ "ok": false, "error": "POST with ?confirm=1 required" }))).into_response();
    }
    Json(s.meter.reset_runs()).into_response()
}

// ── Engine-dependent (the 2 MB JS game engine is not yet ported to Rust) ─────
fn engine_pending(feature: &str) -> Value {
    json!({
        "found": save::save_exists(),
        "enginePending": true,
        "note": format!("{feature} requires the TBH simulation engine (port in progress)"),
        "insights": Value::Null,
    })
}
async fn h_insights() -> impl IntoResponse { Json(engine_pending("insights")) }
async fn h_upgrades() -> impl IntoResponse {
    Json(json!({ "found": save::save_exists(), "enginePending": true, "slots": [] }))
}
// ── Sell desk: value the stash against live BUY ORDERS ──────────────────────
/// Fetch order books for every owned sellable item and rank them.
/// Throttled (650 ms) and 429-aware, mirroring the original's cadence.
async fn h_stash_orders(State(s): State<AppState>, Query(q): Query<Q>) -> impl IntoResponse {
    let appid: i64 = q.get("appid").and_then(|v| v.parse().ok()).unwrap_or(save::TBH_APPID);
    let lang = q.get("lang").map(|s| s.as_str()).unwrap_or("en-US");
    if !save::save_exists() { return Json(json!({ "found": false })); }

    let market = read_bundled(&s, "cache/items.json")
        .and_then(|v| v.get("items").cloned())
        .and_then(|v| v.as_array().cloned())
        .unwrap_or_default();
    let stash = match save::read_stash(&market) {
        Ok(v) => v,
        Err(e) => return Json(json!({ "found": true, "error": e.to_string() })),
    };
    let code = s.currency.load(Ordering::Relaxed);
    let info = currency::get(code).unwrap();
    let cat = wiki::ensure_catalog(&s.data_dir).await;

    let empty = vec![];
    let owned = stash.get("items").and_then(|v| v.as_array()).unwrap_or(&empty);
    let mut items: Vec<Value> = Vec::new();
    let (mut total_instant, mut total_suggested) = (0i64, 0i64);

    for it in owned {
        let hash = it.get("hash").and_then(|v| v.as_str()).unwrap_or("").to_string();
        let qty = it.get("qty").and_then(|v| v.as_i64()).unwrap_or(1);
        if hash.is_empty() { continue; }
        match steam::fetch_orderbook(appid, &hash, code).await {
            Ok(ob) => {
                let max_buy = ob["maxBuyCents"].as_i64().unwrap_or(0);
                let min_sell = ob["minSellCents"].as_i64().unwrap_or(0);
                let buy_count = ob["buyCount"].as_i64().unwrap_or(0);
                let suggested = pricing::suggested_list_cents(min_sell, max_buy);
                total_instant += max_buy * qty;
                total_suggested += suggested * qty;
                let mut row = json!({
                    "name": it.get("name"), "hash": hash, "qty": qty,
                    "maxBuyCents": max_buy, "minSellCents": min_sell,
                    "buyCount": buy_count, "liquidity": ob["liquidity"],
                    "subtotalCents": max_buy * qty,
                    "suggestedCents": suggested,
                    "suggestedSubtotalCents": suggested * qty,
                    "netAfterFeeCents": pricing::net_after_fee_cents(suggested),
                    "spreadPct": pricing::spread_pct_of(min_sell, max_buy),
                });
                if let Some(c) = &cat {
                    let mut one = vec![row.clone()];
                    wiki::enrich_items(c, &mut one, lang);
                    row = one.remove(0);
                }
                items.push(row);
            }
            Err(e) => {
                let msg = e.to_string();
                items.push(json!({ "name": it.get("name"), "hash": hash, "qty": qty, "error": msg }));
                if e.to_string().contains("429") { break; } // rate limited — stop early
            }
        }
        tokio::time::sleep(std::time::Duration::from_millis(650)).await;
    }

    // most liquid first, then richest buy order
    items.sort_by(|a, b| {
        let ka = (a["buyCount"].as_i64().unwrap_or(-1), a["maxBuyCents"].as_i64().unwrap_or(0));
        let kb = (b["buyCount"].as_i64().unwrap_or(-1), b["maxBuyCents"].as_i64().unwrap_or(0));
        kb.cmp(&ka)
    });

    Json(json!({
        "found": true, "currency": code, "symbol": info.symbol,
        "totalInstantCents": total_instant, "totalSuggestedCents": total_suggested,
        "count": items.len(), "items": items,
    }))
}

/// Deep Scan: background probe of every stash entry's order book.
async fn h_stash_scan(State(s): State<AppState>, Query(q): Query<Q>) -> impl IntoResponse {
    let action = q.get("action").map(|s| s.as_str()).unwrap_or("status");
    if !save::save_exists() { return Json(json!({ "found": false })); }
    let code = s.currency.load(Ordering::Relaxed);
    let info = currency::get(code).unwrap();

    if action == "start" {
        {
            let mut st = s.scan.lock().unwrap();
            if st.status == "running" {
                return Json(json!({ "found": true, "status": "running", "total": st.total, "done": st.done }));
            }
            *st = ScanState { status: "running".into(), ..Default::default() };
        }
        let s2 = s.clone();
        tokio::spawn(async move { run_stash_scan(s2, code).await });
        return Json(json!({ "found": true, "status": "running", "total": 0, "done": 0 }));
    }

    let st = s.scan.lock().unwrap();
    let mut out = json!({
        "found": true, "status": st.status, "total": st.total, "done": st.done,
        "currency": code, "symbol": info.symbol,
        "totalInstantCents": st.total_instant_cents,
        "totalSuggestedCents": st.total_suggested_cents,
        "error": st.error,
    });
    if st.status != "running" {
        out["items"] = json!(st.items);
    }
    Json(out)
}

async fn run_stash_scan(s: AppState, code: u32) {
    let market = read_bundled(&s, "cache/items.json")
        .and_then(|v| v.get("items").cloned())
        .and_then(|v| v.as_array().cloned())
        .unwrap_or_default();
    let stash = match save::read_stash(&market) {
        Ok(v) => v,
        Err(e) => {
            let mut st = s.scan.lock().unwrap();
            st.status = "error".into();
            st.error = Some(e.to_string());
            return;
        }
    };
    // allEntries covers items the market cache never matched (materials with buy orders only)
    let entries: Vec<Value> = stash
        .get("items").and_then(|v| v.as_array()).cloned().unwrap_or_default();
    s.scan.lock().unwrap().total = entries.len();

    let (mut instant, mut suggested_total) = (0i64, 0i64);
    let mut out: Vec<Value> = Vec::new();
    for e in entries {
        let hash = e.get("hash").and_then(|v| v.as_str()).unwrap_or("").to_string();
        let qty = e.get("qty").and_then(|v| v.as_i64()).unwrap_or(1);
        if !hash.is_empty() {
            if let Ok(ob) = steam::fetch_orderbook(save::TBH_APPID, &hash, code).await {
                let max_buy = ob["maxBuyCents"].as_i64().unwrap_or(0);
                let min_sell = ob["minSellCents"].as_i64().unwrap_or(0);
                let sug = pricing::suggested_list_cents(min_sell, max_buy);
                instant += max_buy * qty;
                suggested_total += sug * qty;
                out.push(json!({
                    "name": e.get("name"), "hash": hash, "qty": qty, "kind": e.get("kind"),
                    "matched": true, "maxBuyCents": max_buy, "minSellCents": min_sell,
                    "buyCount": ob["buyCount"], "liquidity": ob["liquidity"],
                    "subtotalCents": max_buy * qty, "suggestedCents": sug,
                }));
            }
        }
        // Scope the guard: a std MutexGuard held across an await makes the future non-Send.
        {
            let mut st = s.scan.lock().unwrap();
            st.done += 1;
            st.total_instant_cents = instant;
            st.total_suggested_cents = suggested_total;
        }
        tokio::time::sleep(std::time::Duration::from_millis(700)).await;
    }
    let mut st = s.scan.lock().unwrap();
    st.items = out;
    st.status = "done".into();
}

async fn h_wiki_item(State(s): State<AppState>, Query(q): Query<Q>) -> impl IntoResponse {
    let hash = match q.get("hash") { Some(h) => h, None => return Json(json!({ "ok": false })) };
    let lang = q.get("lang").map(|s| s.as_str()).unwrap_or("en-US");
    match wiki::ensure_catalog(&s.data_dir).await {
        Some(c) => Json(json!({ "ok": true, "hash": hash, "wiki": c.enrich_hash(hash, lang) })),
        None => Json(json!({ "ok": false, "hash": hash })),
    }
}

// ── Built-in live meter (absorbed from mad-labs-org/tbh-meter, MIT) ─────────
async fn h_meter(State(s): State<AppState>) -> impl IntoResponse {
    Json(s.meter.live_json())
}
async fn h_meter_status(State(s): State<AppState>) -> impl IntoResponse {
    Json(s.meter.status())
}
async fn h_meter_enable(State(s): State<AppState>, Query(q): Query<Q>) -> impl IntoResponse {
    let on = q.get("on").map(|v| v == "1" || v == "true").unwrap_or(true);
    s.meter.set_enabled(on);
    Json(s.meter.status())
}

// ── Steam network endpoints ─────────────────────────────────────────────────
async fn h_items(State(s): State<AppState>, Query(q): Query<Q>) -> impl IntoResponse {
    let appid: i64 = q.get("appid").and_then(|v| v.parse().ok()).unwrap_or(save::TBH_APPID);
    let code = s.currency.load(Ordering::Relaxed);
    match steam::fetch_all_items(appid, code).await {
        Ok(mut v) => {
            // best-effort cache for /api/stash cross-reference (pre-enrichment)
            let _ = std::fs::create_dir_all(s.data_dir.join("cache"));
            let _ = std::fs::write(s.data_dir.join("cache/items.json"), v.to_string());
            let lang = q.get("lang").map(|s| s.as_str()).unwrap_or("en-US");
            if let Some(cat) = wiki::ensure_catalog(&s.data_dir).await {
                if let Some(items) = v.get_mut("items").and_then(|x| x.as_array_mut()) {
                    wiki::enrich_items(&cat, items, lang);
                }
            }
            if let Some(o) = v.as_object_mut() { o.insert("stale".into(), json!(false)); }
            Json(v)
        }
        Err(e) => Json(json!({ "error": e.to_string() })),
    }
}
async fn h_orderbook(State(s): State<AppState>, Query(q): Query<Q>) -> impl IntoResponse {
    let appid: i64 = q.get("appid").and_then(|v| v.parse().ok()).unwrap_or(save::TBH_APPID);
    let name = match q.get("name") { Some(n) => n, None => return err500("name is required") };
    let code = s.currency.load(Ordering::Relaxed);
    match steam::fetch_orderbook(appid, name, code).await {
        Ok(v) => Json(v).into_response(),
        Err(e) => err500(&e.to_string()),
    }
}
async fn h_market_depth(Query(q): Query<Q>) -> impl IntoResponse {
    let appid: i64 = q.get("appid").and_then(|v| v.parse().ok()).unwrap_or(save::TBH_APPID);
    let hash = match q.get("hash") { Some(h) => h, None => return Json(json!({ "ok": false })) };
    match steam::fetch_orderbook(appid, hash, 1).await {
        Ok(v) => Json(json!({ "ok": true, "hash": hash, "buyCount": v["buyCount"], "sellCount": v["sellCount"], "dailyVolume": Value::Null })),
        Err(e) => Json(json!({ "ok": false, "hash": hash, "error": e.to_string() })),
    }
}
async fn h_pricehistory(Query(q): Query<Q>) -> impl IntoResponse {
    let appid: i64 = q.get("appid").and_then(|v| v.parse().ok()).unwrap_or(save::TBH_APPID);
    let hash = match q.get("hash") { Some(h) => h, None => return Json(json!({ "found": false })) };
    match steam::fetch_price_history(appid, hash).await {
        Ok(points) => {
            let pts: Vec<Value> = points.iter().map(|p| json!({ "t": p.t, "priceCents": (p.price * 100.0).round() as i64, "vol": p.vol })).collect();
            let metrics = pricing::history_metrics(&points).map(|m| json!({
                "trend": m.trend, "dailyVolume": m.daily_volume, "pointCount": m.point_count,
            }));
            Json(json!({ "found": true, "hash": hash, "count": pts.len(), "points": pts, "metrics": metrics }))
        }
        Err(e) => Json(json!({ "found": true, "hash": hash, "error": e.to_string() })),
    }
}
async fn h_hover(Query(q): Query<Q>) -> impl IntoResponse {
    let appid: i64 = q.get("appid").and_then(|v| v.parse().ok()).unwrap_or(save::TBH_APPID);
    let hash = match q.get("hash") { Some(h) => h, None => return Json(json!({ "found": false })) };
    match steam::fetch_orderbook(appid, hash, 1).await {
        Ok(ob) => Json(json!({
            "found": true, "hash": hash,
            "lowestSellCents": ob["minSellCents"], "highestBuyCents": ob["maxBuyCents"],
            "buyCount": ob["buyCount"], "sellCount": ob["sellCount"], "liquidity": ob["liquidity"],
            "suggestedCents": pricing::suggested_list_cents(ob["minSellCents"].as_i64().unwrap_or(0), ob["maxBuyCents"].as_i64().unwrap_or(0)),
        })),
        Err(e) => Json(json!({ "found": true, "hash": hash, "error": e.to_string() })),
    }
}
async fn h_resolve_hash(Query(q): Query<Q>) -> impl IntoResponse {
    let appid: i64 = q.get("appid").and_then(|v| v.parse().ok()).unwrap_or(save::TBH_APPID);
    let name = match q.get("name") { Some(n) => n, None => return Json(json!({ "ok": false, "error": "name required" })) };
    match steam::resolve_hash_by_name(appid, name).await {
        Ok(h) => Json(json!({ "ok": true, "name": name, "hash": h })),
        Err(e) => Json(json!({ "ok": false, "error": e.to_string() })),
    }
}
async fn h_updates(Query(q): Query<Q>) -> impl IntoResponse {
    let lang = q.get("lang").map(|s| s.as_str()).unwrap_or("en-US");
    let (cc, l) = steam_feed_params(lang);
    match steam::fetch_updates(cc, l).await {
        Ok(v) => Json(v),
        Err(e) => Json(json!({ "ok": false, "error": e.to_string(), "patchnotes": [], "news": [] })),
    }
}

fn steam_feed_params(locale: &str) -> (&'static str, &'static str) {
    match locale {
        "id-ID" => ("ID", "indonesian"), "ja-JP" => ("JP", "japanese"), "ko-KR" => ("KR", "koreana"),
        "zh-Hans" => ("CN", "schinese"), "zh-Hant" => ("TW", "tchinese"), "de-DE" => ("DE", "german"),
        "es-ES" => ("ES", "spanish"), "fr-FR" => ("FR", "french"), "pt-BR" => ("BR", "brazilian"),
        "ru-RU" => ("RU", "russian"), "th-TH" => ("TH", "thai"), "tr-TR" => ("TR", "turkish"),
        "vi-VN" => ("VN", "vietnamese"), "pl-PL" => ("PL", "polish"), _ => ("US", "english"),
    }
}

fn err500(msg: &str) -> axum::response::Response {
    (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({ "error": msg }))).into_response()
}

/// Bind + serve. Returns once the listener is bound (serving continues in a spawned task).
pub async fn serve(state: AppState) -> anyhow::Result<()> {
    let app = router(state);
    let addr = std::net::SocketAddr::from(([127, 0, 0, 1], port()));
    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, app).await?;
    Ok(())
}
