//! Embedded axum HTTP server on 127.0.0.1:5260. Mirrors the `/api/*` contract of the original
//! Node backend. Serves the bundled Nuxt SPA as the static root (SPA fallback → 200.html).

use crate::{currency, pricing, save, steam};
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
        .route("/api/farm-calibration", get(h_farm_calibration))
        .route("/api/runs", get(h_runs))
        .route("/api/runs/reset", post(h_runs_reset))
        .route("/api/insights", get(h_insights))
        .route("/api/upgrades", get(h_upgrades))
        .route("/api/meter", get(h_meter))
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
        Ok(stash) => {
            let code = s.currency.load(Ordering::Relaxed);
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
async fn h_farm_calibration() -> impl IntoResponse {
    // Run logs come from the external meter reader; empty until it writes them.
    Json(json!({ "ok": true, "stages": [], "primaryDifficulty": Value::Null, "difficulties": [], "totalRuns": 0, "generatedAt": 0 }))
}
async fn h_runs() -> impl IntoResponse {
    Json(json!({ "ok": true, "runs": [] }))
}
async fn h_runs_reset() -> impl IntoResponse {
    Json(json!({ "ok": true, "archived": 0, "total": 0 }))
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
async fn h_meter(State(s): State<AppState>) -> impl IntoResponse {
    let live = s.data_dir.join("meter/live.json");
    match std::fs::read_to_string(live) {
        Ok(t) => Json(json!({ "ok": true, "live": serde_json::from_str::<Value>(&t).unwrap_or(Value::Null) })),
        Err(_) => Json(json!({ "ok": false })),
    }
}

// ── Steam network endpoints ─────────────────────────────────────────────────
async fn h_items(State(s): State<AppState>, Query(q): Query<Q>) -> impl IntoResponse {
    let appid: i64 = q.get("appid").and_then(|v| v.parse().ok()).unwrap_or(save::TBH_APPID);
    let code = s.currency.load(Ordering::Relaxed);
    match steam::fetch_all_items(appid, code).await {
        Ok(mut v) => {
            // best-effort cache for /api/stash cross-reference
            let _ = std::fs::create_dir_all(s.data_dir.join("cache"));
            let _ = std::fs::write(s.data_dir.join("cache/items.json"), v.to_string());
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
