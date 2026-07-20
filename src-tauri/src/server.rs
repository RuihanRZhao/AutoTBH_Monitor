//! Embedded axum HTTP server on 127.0.0.1:5260. Mirrors the `/api/*` contract of the original
//! Node backend. Serves the bundled Nuxt SPA as the static root (SPA fallback → 200.html).

use crate::{currency, engine, farm, gearstats, insights, meter::Meter, pricing, save, steam, upgrades, wiki};
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
    pub items_progress: Arc<std::sync::Mutex<steam::ItemsProgress>>,
}

const ITEMS_TTL_MS: i64 = 10 * 60 * 1000;

/// Never panic on an out-of-table currency code (e.g. a bogus TSM_CURRENCY): fall back to USD.
fn cur_or_usd(code: u32) -> currency::Currency {
    currency::get(code).unwrap_or(currency::Currency {
        iso: "USD", symbol: "$", decimals: 2, name: "US Dollar", country: "US",
    })
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
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
        .route("/api/hero-stats", get(h_hero_stats))
        .route("/api/hero-modifiers", get(h_hero_modifiers))
        .route("/api/gear-lines", get(h_gear_lines))
        .route("/api/debug/iteminfo", get(h_debug_iteminfo))
        .route("/api/debug/findrecord", get(h_debug_findrecord))
        .route("/api/debug/modifiers", get(h_debug_modifiers))
        .route("/api/debug/monsterhp", get(h_debug_monsterhp))
        .route("/api/debug/classscan", get(h_debug_classscan))
        .route("/api/debug/instancedump", get(h_debug_instancedump))
        .route("/api/debug/stagelogs", get(h_debug_stagelogs))
        .route("/api/items", get(h_items))
        .route("/api/items-progress", get(h_items_progress))
        .route("/api/orderbook", get(h_orderbook))
        .route("/api/market-depth", get(h_market_depth))
        .route("/api/pricehistory", get(h_pricehistory))
        .route("/api/hover", get(h_hover))
        .route("/api/resolve-hash", get(h_resolve_hash))
        .route("/api/updates", get(h_updates))
        .fallback_service(static_svc)
        // Only our own window may call this API. `permissive()` let ANY page the user had open
        // read their save-derived stash and POST /api/runs/reset.
        .layer(
            CorsLayer::new()
                .allow_origin(
                    [
                        format!("http://localhost:{}", port()).parse().unwrap(),
                        format!("http://127.0.0.1:{}", port()).parse().unwrap(),
                    ]
                    .to_vec(),
                )
                .allow_methods(tower_http::cors::Any)
                .allow_headers(tower_http::cors::Any),
        )
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

/// Save-derived insights (native Rust; simulation-dependent parts are flagged, not guessed).
async fn h_insights(State(s): State<AppState>) -> impl IntoResponse {
    if !save::save_exists() {
        return Json(json!({ "found": false }));
    }
    match crate::insights::build(&s.data_dir, &s.meter) {
        Ok(v) => Json(v),
        Err(e) => Json(json!({ "found": true, "error": e.to_string() })),
    }
}
/// Per-slot gear-swap ranking. Requires the game running: the swap maths needs the modifier
/// buckets, which only exist in the live process.
async fn h_upgrades(State(s): State<AppState>) -> impl IntoResponse {
    if !save::save_exists() {
        return Json(json!({ "found": false }));
    }
    let gear = match gearstats::build(&s.data_dir, &s.meter).await {
        Ok(g) => g,
        Err(e) => return Json(json!({ "found": true, "ok": false, "error": e.to_string() })),
    };
    let modifiers = match s.meter.read_party_modifiers() {
        Ok(m) => m,
        Err(e) => {
            return Json(json!({
                "found": true, "ok": false, "needsGame": true, "error": e,
            }))
        }
    };
    let stage_level = insights::current_stage_level(&s.data_dir).unwrap_or(1.0);
    let pool = json!({ "items": gear.get("pool").cloned().unwrap_or(json!([])) });
    Json(upgrades::build(&gear, &modifiers, &pool, stage_level))
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
    let info = cur_or_usd(code);
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
    let info = cur_or_usd(code);

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
    // allEntries (not `items`) is the point of Deep Scan: it includes entries the market cache
    // never matched — materials with no sell listing but plenty of buy orders.
    let entries: Vec<Value> = stash
        .get("allEntries").and_then(|v| v.as_array()).cloned()
        .or_else(|| stash.get("items").and_then(|v| v.as_array()).cloned())
        .unwrap_or_default();
    s.scan.lock().unwrap().total = entries.len();

    let (mut instant, mut suggested_total) = (0i64, 0i64);
    let mut out: Vec<Value> = Vec::new();
    for e in entries {
        // allEntries uses `searchName`; fall back to `hash` when scanning the matched list.
        let hash = e.get("searchName").and_then(|v| v.as_str())
            .or_else(|| e.get("hash").and_then(|v| v.as_str()))
            .unwrap_or("").to_string();
        let qty = e.get("qty").and_then(|v| v.as_i64()).unwrap_or(1);
        let mut rate_limited = false;
        if !hash.is_empty() {
            match steam::fetch_orderbook(save::TBH_APPID, &hash, code).await {
                Ok(ob) => {
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
                Err(err) => {
                    // Record the failure instead of dropping the row: otherwise the UI reports
                    // "done, N/N" with a total that silently excludes every failed item.
                    let msg = err.to_string();
                    rate_limited = msg.contains("429");
                    out.push(json!({
                        "name": e.get("name"), "hash": hash, "qty": qty, "kind": e.get("kind"),
                        "matched": false, "error": msg,
                    }));
                }
            }
        }
        if rate_limited {
            let mut st = s.scan.lock().unwrap();
            st.error = Some("rate-limited by Steam — partial results".into());
            st.items = out;
            st.status = "error".into();
            return;
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
/// Equipped-gear stat lines rebuilt from the save + wiki items_detail (offline path).
async fn h_gear_lines(State(s): State<AppState>) -> impl IntoResponse {
    if !save::save_exists() {
        return Json(json!({ "ok": false, "error": "save not found" }));
    }
    match crate::gearstats::build(&s.data_dir, &s.meter).await {
        Ok(v) => Json(v),
        Err(e) => Json(json!({ "ok": false, "error": e.to_string() })),
    }
}

/// Probe the game's own ItemInfoData object for an ItemKey (game-authoritative layout discovery).
async fn h_debug_iteminfo(State(s): State<AppState>, Query(q): Query<Q>) -> impl IntoResponse {
    let key: i64 = q.get("key").and_then(|v| v.parse().ok()).unwrap_or(334031);
    let words: usize = q.get("words").and_then(|v| v.parse().ok()).unwrap_or(64);
    let deref = q.get("deref").and_then(|v| {
        let t = v.trim_start_matches("0x");
        usize::from_str_radix(t, 16).ok()
    });
    match s.meter.probe_item_info(key, words.min(512), deref) {
        Ok(v) => Json(v),
        Err(e) => Json(json!({ "ok": false, "error": e })),
    }
}

async fn h_debug_findrecord(State(s): State<AppState>, Query(q): Query<Q>) -> impl IntoResponse {
    let key: i32 = q.get("key").and_then(|v| v.parse().ok()).unwrap_or(334031);
    let expect: Vec<i32> = q.get("expect").map(|v| v.split(',').filter_map(|x| x.trim().parse().ok()).collect()).unwrap_or_default();
    let window: usize = q.get("window").and_then(|v| v.parse().ok()).unwrap_or(32);
    match s.meter.find_record_with(key, expect, window.min(128)) {
        Ok(v) => Json(v),
        Err(e) => Json(json!({ "ok": false, "error": e })),
    }
}

async fn h_debug_modifiers(State(s): State<AppState>, Query(q): Query<Q>) -> impl IntoResponse {
    let words: usize = q.get("words").and_then(|v| v.parse().ok()).unwrap_or(32);
    match s.meter.probe_modifier_mgr(words.min(256)) {
        Ok(v) => Json(v),
        Err(e) => Json(json!({ "ok": false, "error": e })),
    }
}

async fn h_debug_monsterhp(State(s): State<AppState>) -> impl IntoResponse {
    match s.meter.probe_monster_hp() {
        Ok(v) => Json(v),
        Err(e) => Json(json!({ "ok": false, "error": e })),
    }
}

/// Live TypeInfoTable scan for classes whose name contains any of `q=needle1,needle2,...`.
/// Used to go from a global-metadata.dat string-scan hit to a live, resolvable class.
async fn h_debug_classscan(State(s): State<AppState>, Query(q): Query<Q>) -> impl IntoResponse {
    let needles: Vec<String> = q.get("q").map(|v| v.split(',').map(|s| s.trim().to_string()).collect()).unwrap_or_default();
    if needles.is_empty() {
        return Json(json!({ "ok": false, "error": "missing ?q=needle1,needle2" }));
    }
    let max_index: usize = q.get("max").and_then(|v| v.parse().ok()).unwrap_or(60_000);
    match s.meter.scan_classes(&needles, max_index) {
        Ok(v) => Json(v),
        Err(e) => Json(json!({ "ok": false, "error": e })),
    }
}

/// Dump live instances of `?type=<typeIndex>` (from /api/debug/classscan) as raw f32/i32 fields,
/// for eyeballing an unknown class's layout by value magnitude.
async fn h_debug_instancedump(State(s): State<AppState>, Query(q): Query<Q>) -> impl IntoResponse {
    let Some(ty) = q.get("type").and_then(|v| v.parse::<usize>().ok()) else {
        return Json(json!({ "ok": false, "error": "missing ?type=<typeIndex>" }));
    };
    let limit: usize = q.get("limit").and_then(|v| v.parse().ok()).unwrap_or(5);
    let window: usize = q.get("window").and_then(|v| v.parse().ok()).unwrap_or(128);
    match s.meter.dump_instances(ty, limit, window) {
        Ok(v) => Json(v),
        Err(e) => Json(json!({ "ok": false, "error": e })),
    }
}

/// Game-authoritative stage-clear/fail log, straight from LogManager.LOG_LIST.
async fn h_debug_stagelogs(State(s): State<AppState>, Query(q): Query<Q>) -> impl IntoResponse {
    let limit: usize = q.get("limit").and_then(|v| v.parse().ok()).unwrap_or(2000);
    match s.meter.read_stage_logs(limit) {
        Ok(v) => Json(v),
        Err(e) => Json(json!({ "ok": false, "error": e })),
    }
}

/// Stat-modifier decomposition per hero, with a self-check: aggregating the buckets must
/// reproduce the game's own FINAL_STATS. Only if it does is gear-swap simulation trustworthy,
/// because a swap works by editing those buckets and re-aggregating.
async fn h_hero_modifiers(State(s): State<AppState>) -> impl IntoResponse {
    let mods = match s.meter.read_party_modifiers() {
        Ok(m) => m,
        Err(e) => return Json(json!({ "ok": false, "error": e })),
    };
    let finals = s.meter.read_party_stats().unwrap_or_default();
    let mut heroes = Vec::new();
    for m in &mods {
        let key = m["heroKey"].as_i64().unwrap_or(0);
        let fin_stats = finals
            .iter()
            .find(|f| f["heroKey"].as_i64() == Some(key))
            .and_then(|f| f.get("stats"))
            .and_then(|v| v.as_object())
            .cloned()
            .unwrap_or_default();
        let mut checks = Vec::new();
        if let Some(stats) = m["stats"].as_object() {
            for (name, b) in stats {
                let sum = |k: &str| -> f64 {
                    b.get(k)
                        .and_then(|v| v.as_array())
                        .map(|a| a.iter().filter_map(|x| x.as_f64()).sum())
                        .unwrap_or(0.0)
                };
                let (f, a, mu) = (sum("flat"), sum("additive"), sum("multiplicative"));
                // Game-native units: these are already fractions, so no divisors here.
                let computed = f * (1.0 + a) * (1.0 + mu);
                let actual = fin_stats
                    .iter()
                    .find(|(k, _)| engine::stat_name(k.parse::<i64>().unwrap_or(-1)) == name)
                    .and_then(|(_, v)| v.as_f64());
                if let Some(act) = actual {
                    let diff = (computed - act).abs();
                    let ok = diff < 1e-3 || (act != 0.0 && diff / act.abs() < 1e-4);
                    checks.push(json!({
                        "stat": name, "computed": computed, "final": act, "match": ok,
                        "flatSum": f, "addSum": a, "mulSum": mu, "modCount": b.get("count"),
                    }));
                }
            }
        }
        checks.sort_by(|a, b| a["stat"].as_str().unwrap_or("").cmp(b["stat"].as_str().unwrap_or("")));
        let matched = checks.iter().filter(|c| c["match"].as_bool() == Some(true)).count();
        heroes.push(json!({
            "heroKey": key, "checked": checks.len(), "matched": matched, "checks": checks,
        }));
    }
    Json(json!({ "ok": true, "heroes": heroes }))
}

async fn h_meter_status(State(s): State<AppState>) -> impl IntoResponse {
    Json(s.meter.status())
}

/// Live per-hero FINAL_STATS + engine-derived offence numbers.
///
/// Stats are read from the running game (the save carries no resolved stats), labelled by
/// StatType name, and fed through the verified engine formulas. `ehp`/`power` are only emitted
/// once the armour→mitigation curve is fitted — until then they stay null rather than guessed.
async fn h_hero_stats(State(s): State<AppState>) -> impl IntoResponse {
    let p = engine::Params::default();
    // Content level our survivability metric is measured against (save + bundled stage table).
    let stage_level = crate::insights::current_stage_level(&s.data_dir).unwrap_or(1.0);
    match s.meter.read_party_stats() {
        Ok(list) => {
            let heroes: Vec<Value> = list
                .iter()
                .map(|h| {
                    let raw = h.get("stats").and_then(|v| v.as_object()).cloned().unwrap_or_default();
                    let get = |id: i64| -> f64 {
                        raw.get(&id.to_string()).and_then(|v| v.as_f64()).unwrap_or(0.0)
                    };
                    // Two views: the game's native FINAL_STATS, and those rescaled into the
                    // reference engine's display units (only where the factor is confirmed).
                    let mut game_named = serde_json::Map::new();
                    let mut display_named = serde_json::Map::new();
                    for (k, v) in raw.iter() {
                        let id: i64 = k.parse().unwrap_or(-1);
                        let name = engine::stat_name(id).to_string();
                        let val = v.as_f64().unwrap_or(0.0);
                        game_named.insert(name.clone(), json!(val));
                        if let Some(scale) = engine::game_to_display_scale(id) {
                            display_named.insert(name, json!(val * scale));
                        }
                    }
                    let (ad, as_, cc, cd) = (get(1), get(2), get(3), get(4));
                    // Game units are already normalised — no divisors.
                    let auto = engine::auto_dps_game(ad, as_, cc, cd, &p);
                    let (max_hp, armor) = (get(5), get(6));
                    // DodgeChance: game stores 0.031, which is 31% (not 3.1%). Go through the
                    // verified scale table rather than a literal — the display value IS the
                    // percent, so game 0.031 x 1000 = 31. Using x100 here silently credits only
                    // a tenth of the hero's dodge, which is exactly the class of 10x unit slip
                    // this codebase keeps getting bitten by.
                    let dodge = get(16) * engine::game_to_display_scale(16).unwrap_or(1000.0);
                    let ehp = engine::ehp_from_stats(max_hp, armor, stage_level, dodge, &p);
                    json!({
                        "heroKey": h.get("heroKey"),
                        "slot": h.get("slot"),
                        "stats": game_named,
                        "statsDisplay": display_named,
                        "autoDps": auto,
                        "critMultiplier": 1.0 + cc * (cd - 1.0),
                        "maxHp": max_hp,
                        "armor": armor,
                        "dodgePercent": dodge,
                        "stageLevel": stage_level,
                        "armorMitigation": engine::armor_mitigation(armor, stage_level, &p),
                        "ehp": ehp,
                        "power": engine::power(auto, ehp),
                        "pending": ["skillDps"],
                        "notes": {
                            "units": engine::MULTIPLICATIVE_DIVISOR_NOTE,
                            "ehp": "own metric from game-authoritative stats; does not match the reference implementation by design",
                        },
                    })
                })
                .collect();
            Json(json!({ "ok": true, "source": "memory", "heroes": heroes }))
        }
        Err(e) => Json(json!({ "ok": false, "error": e })),
    }
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
    let lang = q.get("lang").map(|s| s.as_str()).unwrap_or("en-US").to_string();
    let force = q.get("refresh").map(|v| v == "1").unwrap_or(false);
    let cached = read_bundled(&s, "cache/items.json");
    let age_ms = cached
        .as_ref()
        .and_then(|c| c.get("fetchedAt").and_then(|v| v.as_i64()))
        .map(|t| now_ms() - t)
        .unwrap_or(i64::MAX);
    let running = s.items_progress.lock().unwrap().status == "running";

    // A full sweep is ~750 items at 10/page with a 1.8 s cadence (minutes), so it must never
    // block the request. Serve what we have and refresh in the background; the UI polls
    // /api/items-progress. Matches the original's refresh-dedup behaviour.
    let need_refresh = force || age_ms > ITEMS_TTL_MS;
    if need_refresh && !running {
        let s2 = s.clone();
        let prog = s.items_progress.clone();
        {
            let mut g = prog.lock().unwrap();
            *g = steam::ItemsProgress { status: "running".into(), started_at: now_ms(), ..Default::default() };
        }
        tokio::spawn(async move {
            let res = steam::fetch_all_items(appid, code, Some(prog.clone())).await;
            let mut g = prog.lock().unwrap();
            match res {
                Ok(v) => {
                    let _ = std::fs::create_dir_all(s2.data_dir.join("cache"));
                    let _ = std::fs::write(s2.data_dir.join("cache/items.json"), v.to_string());
                    g.got = v.get("items").and_then(|i| i.as_array()).map(|a| a.len()).unwrap_or(0);
                    g.status = "done".into();
                }
                Err(e) => { g.status = "error".into(); g.error = Some(e.to_string()); }
            }
            g.ended_at = now_ms();
        });
    }

    match cached {
        Some(mut v) => {
            if let Some(cat) = wiki::ensure_catalog(&s.data_dir).await {
                if let Some(items) = v.get_mut("items").and_then(|x| x.as_array_mut()) {
                    wiki::enrich_items(&cat, items, &lang);
                }
            }
            if let Some(o) = v.as_object_mut() {
                o.insert("stale".into(), json!(age_ms > ITEMS_TTL_MS));
                o.insert("refreshing".into(), json!(need_refresh || running));
            }
            Json(v)
        }
        None => Json(json!({
            "appid": appid, "fetchedAt": 0, "total": 0, "items": [],
            "currency": currency::info_json(code),
            "stale": true, "refreshing": true,
        })),
    }
}

async fn h_items_progress(State(s): State<AppState>) -> impl IntoResponse {
    Json(json!(*s.items_progress.lock().unwrap()))
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
async fn h_market_depth(State(s): State<AppState>, Query(q): Query<Q>) -> impl IntoResponse {
    let appid: i64 = q.get("appid").and_then(|v| v.parse().ok()).unwrap_or(save::TBH_APPID);
    let hash = match q.get("hash") { Some(h) => h, None => return Json(json!({ "ok": false })) };
    let code = s.currency.load(Ordering::Relaxed); // was hardcoded to USD
    match steam::fetch_orderbook(appid, hash, code).await {
        Ok(v) => Json(json!({ "ok": true, "hash": hash, "buyCount": v["buyCount"], "sellCount": v["sellCount"], "dailyVolume": Value::Null })),
        Err(e) => Json(json!({ "ok": false, "hash": hash, "error": e.to_string() })),
    }
}
async fn h_pricehistory(State(s): State<AppState>, Query(q): Query<Q>) -> impl IntoResponse {
    let appid: i64 = q.get("appid").and_then(|v| v.parse().ok()).unwrap_or(save::TBH_APPID);
    let hash = match q.get("hash") { Some(h) => h, None => return Json(json!({ "found": false })) };
    let code = s.currency.load(Ordering::Relaxed);
    let info = cur_or_usd(code);
    // Steam's embedded history is always USD. Emitting it raw while labelling the response
    // with the display currency put the chart and the order book on different scales
    // (e.g. an IDR user saw Rp 34,171 vs Rp 2.15 for the same item).
    let rate = if info.iso == "USD" { 1.0 } else { steam::usd_to_local(info.iso).await };
    let conv = |usd: f64| -> i64 { (usd * 100.0 * rate).round() as i64 };

    match steam::fetch_price_history(appid, hash).await {
        Ok(points) => {
            let pts: Vec<Value> = points
                .iter()
                .map(|p| json!({ "t": p.t, "priceCents": conv(p.price), "vol": p.vol }))
                .collect();
            let metrics = pricing::history_metrics(&points).map(|m| json!({
                "trend": m.trend,
                "dailyVolume": m.daily_volume,
                "pointCount": m.point_count,
                "weeklyAvgCents": m.weekly_avg.map(conv),
                "recentP75Cents": m.recent_p75.map(conv),
            }));
            Json(json!({
                "found": true, "hash": hash, "symbol": info.symbol, "currency": code,
                "count": pts.len(), "points": pts, "metrics": metrics,
            }))
        }
        Err(e) => Json(json!({ "found": true, "hash": hash, "error": e.to_string() })),
    }
}
/// Full hover intelligence: order book + price history metrics + trade signals + wiki.
async fn h_hover(State(s): State<AppState>, Query(q): Query<Q>) -> impl IntoResponse {
    let appid: i64 = q.get("appid").and_then(|v| v.parse().ok()).unwrap_or(save::TBH_APPID);
    let hash = match q.get("hash") { Some(h) => h.clone(), None => return Json(json!({ "found": false })) };
    let lang = q.get("lang").map(|s| s.as_str()).unwrap_or("en-US");
    let code = s.currency.load(Ordering::Relaxed); // was hardcoded to USD
    let info = cur_or_usd(code);

    let ob = match steam::fetch_orderbook(appid, &hash, code).await {
        Ok(v) => v,
        Err(e) => return Json(json!({ "found": true, "hash": hash, "error": e.to_string() })),
    };
    let min_sell = ob["minSellCents"].as_i64().unwrap_or(0);
    let max_buy = ob["maxBuyCents"].as_i64().unwrap_or(0);
    let buy_count = ob["buyCount"].as_i64().unwrap_or(0);

    // Price history is best-effort: it needs the listing page, which Steam may withhold.
    let metrics = steam::fetch_price_history(appid, &hash)
        .await
        .ok()
        .and_then(|pts| pricing::history_metrics(&pts));
    let daily_volume = metrics.as_ref().map(|m| m.daily_volume);
    let usd_to_local = if info.iso == "USD" { 1.0 } else { steam::usd_to_local(info.iso).await };

    let signals = pricing::analyse_price(&pricing::AnalyseInput {
        min_sell_cents: min_sell, max_buy_cents: max_buy, buy_count,
        daily_volume, metrics, usd_to_local,
    });

    let suggested = pricing::suggested_list_cents(min_sell, max_buy);
    // Match the reference thresholds: `liquid` requires a TIGHT spread AND real depth, and the
    // middle band is deliberately untagged rather than optimistically called liquid.
    let spread = pricing::spread_pct_of(min_sell, max_buy);
    let tag: Value = if max_buy <= 0 {
        json!("no-buyers")
    } else if spread.map(|s| s <= 8.0).unwrap_or(false) && buy_count >= 50 {
        json!("liquid")
    } else if spread.map(|s| s > 25.0).unwrap_or(false) {
        json!("wide-spread")
    } else {
        Value::Null
    };

    let wiki_v = match wiki::ensure_catalog(&s.data_dir).await {
        Some(c) => c.enrich_hash(&hash, lang),
        None => Value::Null,
    };

    let mut out = json!({
        "found": true, "hash": hash, "symbol": info.symbol, "currency": code,
        "lowestSellCents": min_sell, "highestBuyCents": max_buy,
        "suggestedCents": suggested,
        "netAfterFeeCents": pricing::net_after_fee_cents(suggested),
        "spreadPct": spread,
        "buyCount": buy_count, "sellCount": ob["sellCount"], "liquidity": ob["liquidity"],
        "dailyVolume": daily_volume, "tag": tag,
        "hasHistory": daily_volume.is_some(),
        "wiki": wiki_v,
    });
    if let (Some(o), Some(sig)) = (out.as_object_mut(), signals.as_object()) {
        for (k, v) in sig { o.insert(k.clone(), v.clone()); }
    }
    Json(out)
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
