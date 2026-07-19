//! Pure pricing / sell-intelligence helpers. Port of `pricing.mjs`.
//! Order-book formulas adapted from Task Bar Trade Center (MIT); listing-page parsers
//! adapted from Allyans3/steam-market-api-v2 (MIT).

use once_cell::sync::Lazy;
use regex::Regex;
use serde_json::{json, Value};

pub fn classify_liquidity(buy_count: i64) -> &'static str {
    if buy_count <= 0 {
        "none"
    } else if buy_count > 500 {
        "high"
    } else if buy_count >= 50 {
        "medium"
    } else {
        "low"
    }
}

/// Shave the smallest unit off cheap items, 1% off pricier ones. Port of `undercutCents`.
pub fn undercut_cents(c: i64) -> i64 {
    if c <= 0 {
        return 0;
    }
    if c <= 1000 {
        (c - 1).max(1)
    } else {
        (c as f64 * 0.99).round() as i64
    }
}

/// Suggested LIST price: undercut lowest listing, never below highest-buy × 1.03.
pub fn suggested_list_cents(min_sell: i64, max_buy: i64) -> i64 {
    let sell = min_sell.max(0);
    let buy = max_buy.max(0);
    if sell > 0 {
        let mut t = undercut_cents(sell);
        let floor = if buy > 0 { (buy as f64 * 1.03).round() as i64 } else { 0 };
        if floor > 0 {
            t = t.max(floor);
        }
        let ceil = undercut_cents(sell);
        if t > ceil {
            t = ceil;
        }
        return if t > 0 { t } else { sell };
    }
    if buy > 0 {
        return (buy as f64 * 1.03).round() as i64;
    }
    0
}

pub fn spread_pct_of(min_sell: i64, max_buy: i64) -> Option<f64> {
    if min_sell > 0 && max_buy > 0 {
        Some((min_sell - max_buy) as f64 / min_sell as f64 * 100.0)
    } else {
        None
    }
}

pub const STEAM_FEE_DIVISOR: f64 = 1.15;

pub fn net_after_fee_cents(c: i64) -> i64 {
    if c > 0 {
        (c as f64 / STEAM_FEE_DIVISOR).round() as i64
    } else {
        0
    }
}

static NAMEID_RE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"Market_LoadOrderSpread\(\s*(\d+)\s*\)").unwrap());
static LINE1_RE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"(?s)var\s+line1\s*=\s*(\[.*?\]\s*\])\s*;").unwrap());
static HOUR_FIX_RE: Lazy<Regex> = Lazy::new(|| Regex::new(r"(\d{1,2}):\s*\+0$").unwrap());

/// Extract `item_nameid` from a listing page. Port of `parseNameId`.
pub fn parse_name_id(html: &str) -> Option<i64> {
    NAMEID_RE
        .captures(html)
        .and_then(|c| c.get(1))
        .and_then(|m| m.as_str().parse::<i64>().ok())
}

#[derive(Clone)]
pub struct PricePoint {
    pub t: Option<i64>, // unix ms
    pub price: f64,
    pub vol: i64,
}

/// Parse the `var line1=[...]` price-history array. Port of `parsePriceHistory`.
pub fn parse_price_history(html: &str) -> Option<Vec<PricePoint>> {
    let caps = LINE1_RE.captures(html)?;
    let raw: Value = serde_json::from_str(caps.get(1)?.as_str()).ok()?;
    let arr = raw.as_array()?;
    let mut pts = Vec::new();
    for r in arr {
        let row = match r.as_array() {
            Some(a) if a.len() >= 3 => a,
            _ => continue,
        };
        let price = match row[1].as_f64() {
            Some(p) if p.is_finite() => p,
            _ => continue,
        };
        let date_str = row[0].as_str().unwrap_or("");
        let fixed = HOUR_FIX_RE.replace(date_str, "$1:00:00 +0000").to_string();
        let t = parse_steam_date(&fixed);
        let vol = row[2]
            .as_str()
            .and_then(|s| s.trim().parse::<i64>().ok())
            .or_else(|| row[2].as_i64())
            .unwrap_or(0);
        pts.push(PricePoint { t, price, vol });
    }
    if pts.is_empty() {
        None
    } else {
        Some(pts)
    }
}

/// Best-effort parse of Steam's history timestamps ("Jul 01 2026 01:00:00 +0000").
fn parse_steam_date(s: &str) -> Option<i64> {
    use chrono::{NaiveDateTime, TimeZone, Utc};
    // Offset-aware first ("Jul 01 2026 01:00:00 +0000"), then naive (assume UTC).
    if let Ok(dt) = chrono::DateTime::parse_from_str(s, "%b %d %Y %H:%M:%S %z") {
        return Some(dt.timestamp_millis());
    }
    if let Ok(ndt) = NaiveDateTime::parse_from_str(s, "%b %d %Y %H:%M:%S") {
        return Some(Utc.from_utc_datetime(&ndt).timestamp_millis());
    }
    None
}

/// Steam search/render → resolve a display name to its market_hash_name. Port of `parseSearchRender`.
pub fn parse_search_render(json: &Value, want_name: Option<&str>) -> Option<String> {
    let results = json.get("results")?.as_array()?;
    if results.is_empty() {
        return None;
    }
    if let Some(w) = want_name {
        let w = w.to_lowercase();
        for r in results {
            let name = r.get("name").and_then(|v| v.as_str()).unwrap_or("").to_lowercase();
            let hash = r.get("hash_name").and_then(|v| v.as_str()).unwrap_or("").to_lowercase();
            if name == w || hash == w {
                return r.get("hash_name").and_then(|v| v.as_str()).map(String::from);
            }
        }
    }
    results[0].get("hash_name").and_then(|v| v.as_str()).map(String::from)
}

pub fn percentile(sorted_asc: &[f64], p: f64) -> Option<f64> {
    if sorted_asc.is_empty() {
        return None;
    }
    let idx = (((p / 100.0) * sorted_asc.len() as f64).floor() as usize)
        .min(sorted_asc.len() - 1);
    Some(sorted_asc[idx])
}

pub struct HistoryMetrics {
    pub weekly_avg: Option<f64>,
    pub recent_p75: Option<f64>,
    pub daily_volume: i64,
    pub weekly_daily_avg: f64,
    pub trend: String,
    pub last_price: f64,
    pub point_count: usize,
}

/// Derive trade signals from raw history points. Port of `historyMetrics`.
pub fn history_metrics(points: &[PricePoint]) -> Option<HistoryMetrics> {
    if points.is_empty() {
        return None;
    }
    let day = 86_400_000i64;
    let with_t: Vec<&PricePoint> = points.iter().filter(|p| p.t.is_some()).collect();
    let last_t = with_t.last().and_then(|p| p.t);

    let recent7: Vec<&PricePoint> = match last_t {
        Some(lt) => with_t.iter().filter(|p| p.t.unwrap() >= lt - 7 * day).cloned().collect(),
        None => points.iter().rev().take(168).collect::<Vec<_>>().into_iter().rev().collect(),
    };
    let recent1: Vec<&PricePoint> = match last_t {
        Some(lt) => with_t.iter().filter(|p| p.t.unwrap() >= lt - day).cloned().collect(),
        None => points.iter().rev().take(24).collect::<Vec<_>>().into_iter().rev().collect(),
    };

    let mut prices7: Vec<f64> = recent7.iter().map(|p| p.price).collect();
    prices7.sort_by(|a, b| a.partial_cmp(b).unwrap());

    let mut wsum = 0.0;
    let mut wvol = 0.0;
    for p in &recent7 {
        let w = if p.vol > 0 { p.vol as f64 } else { 1.0 };
        wsum += p.price * w;
        wvol += w;
    }
    let weekly_avg = if wvol > 0.0 {
        Some(wsum / wvol)
    } else if !prices7.is_empty() {
        Some(prices7.iter().sum::<f64>() / prices7.len() as f64)
    } else {
        None
    };

    let recent_p75 = percentile(&prices7, 75.0);
    let daily_volume: i64 = recent1.iter().map(|p| p.vol).sum();
    let weekly_daily_avg = recent7.iter().map(|p| p.vol).sum::<i64>() as f64 / 7.0;

    let mut trend = "flat".to_string();
    if let Some(lt) = last_t {
        let mean = |pred: &dyn Fn(i64) -> bool| -> Option<f64> {
            let sel: Vec<f64> = with_t.iter().filter(|p| pred(p.t.unwrap())).map(|p| p.price).collect();
            if sel.is_empty() {
                None
            } else {
                Some(sel.iter().sum::<f64>() / sel.len() as f64)
            }
        };
        let ma = mean(&|t| t >= lt - 3 * day);
        let mb = mean(&|t| t < lt - 3 * day && t >= lt - 6 * day);
        if let (Some(ma), Some(mb)) = (ma, mb) {
            if mb > 0.0 {
                let ch = (ma - mb) / mb;
                trend = if ch > 0.05 { "up" } else if ch < -0.05 { "down" } else { "flat" }.to_string();
            }
        }
    }

    Some(HistoryMetrics {
        weekly_avg,
        recent_p75,
        daily_volume,
        weekly_daily_avg,
        trend,
        last_price: points[points.len() - 1].price,
        point_count: points.len(),
    })
}

pub struct AnalyseInput {
    pub min_sell_cents: i64,
    pub max_buy_cents: i64,
    pub buy_count: i64,
    pub daily_volume: Option<i64>,
    pub metrics: Option<HistoryMetrics>,
    pub usd_to_local: f64,
}

/// Combine order book + history into trade signals. Port of `analysePrice`.
pub fn analyse_price(inp: &AnalyseInput) -> Value {
    let conv = |usd: Option<f64>| -> Option<i64> {
        match usd {
            Some(u) if inp.usd_to_local > 0.0 => Some((u * 100.0 * inp.usd_to_local).round() as i64),
            _ => None,
        }
    };
    let have_orderbook = inp.min_sell_cents > 0 || inp.max_buy_cents > 0;
    let have_history = inp.metrics.as_ref().map(|m| m.point_count > 0).unwrap_or(false);

    let mut weekly_avg_cents = None;
    let mut p75_cents = None;
    let mut trend = None;
    let mut deal_tag: Option<&str> = None;
    if let Some(m) = &inp.metrics {
        if have_history {
            weekly_avg_cents = conv(m.weekly_avg);
            p75_cents = conv(m.recent_p75);
            trend = Some(m.trend.clone());
        }
    }
    let ref_cents = p75_cents.or(weekly_avg_cents);
    if let Some(rc) = ref_cents {
        if inp.min_sell_cents > 0 {
            deal_tag = Some(if (inp.min_sell_cents as f64) < rc as f64 * 0.85 {
                "undervalued"
            } else if (inp.min_sell_cents as f64) > rc as f64 * 1.20 {
                "overpriced"
            } else {
                "fair"
            });
        }
    }

    let mut cs = 0;
    if have_orderbook { cs += 2; }
    if inp.min_sell_cents > 0 && inp.max_buy_cents > 0 { cs += 1; }
    if inp.buy_count >= 50 { cs += 1; }
    if have_history { cs += 2; }
    if weekly_avg_cents.is_some() { cs += 1; }
    if p75_cents.is_some() { cs += 1; }
    let confidence = if cs >= 5 { "verified" } else if cs >= 3 { "estimated" } else { "speculative" };

    let mut volume_activity: Option<&str> = None;
    if have_history {
        if let Some(m) = &inp.metrics {
            if m.weekly_daily_avg > 0.0 {
                if let Some(dv) = inp.daily_volume {
                    let r = dv as f64 / m.weekly_daily_avg;
                    volume_activity = Some(if r >= 1.5 { "active" } else if r <= 0.5 { "slow" } else { "normal" });
                }
            }
        }
    }

    let spread = spread_pct_of(inp.min_sell_cents, inp.max_buy_cents);
    let mut score = 0.0;
    if let Some(dv) = inp.daily_volume {
        score += (dv as f64 / 100.0 * 25.0).min(25.0);
    }
    if let Some(sp) = spread {
        score += (25.0 - (sp - 8.0).max(0.0) * (25.0 / 32.0)).max(0.0);
    }
    score += ((inp.buy_count as f64) / 50.0 * 20.0).min(20.0);
    score += match confidence {
        "verified" => 20.0,
        "estimated" => 12.0,
        _ => 5.0,
    };
    if let Some(wa) = weekly_avg_cents {
        if inp.max_buy_cents > 0 && inp.max_buy_cents > wa {
            score += 15.0;
        }
    }
    let sell_now_score = score.min(100.0).round() as i64;

    json!({
        "dealTag": deal_tag,
        "confidence": confidence,
        "volumeActivity": volume_activity,
        "sellNowScore": sell_now_score,
        "sellNowRecommended": sell_now_score >= 45,
        "weeklyAvgCents": weekly_avg_cents,
        "recentP75Cents": p75_cents,
        "trend": trend,
    })
}
