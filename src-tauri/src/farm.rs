//! Turn recorded run logs into per-stage farming calibration. Port of `farm.mjs`.
//! Pure functions over run records produced by the built-in meter (see `meter.rs`).

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashMap;

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct RunRecord {
    pub ts: f64,
    pub outcome: String,
    #[serde(rename = "stageKey")]
    pub stage_key: Option<i64>,
    pub difficulty: Option<i64>,
    #[serde(rename = "clearTime")]
    pub clear_time: Option<f64>,
    #[serde(rename = "totalDamage")]
    pub total_damage: Option<f64>,
    pub gold: Option<f64>,
    pub xp: Option<f64>,
}

pub fn median(xs: &[f64]) -> f64 {
    let mut a: Vec<f64> = xs.iter().cloned().filter(|x| x.is_finite()).collect();
    if a.is_empty() { return 0.0; }
    a.sort_by(|p, q| p.partial_cmp(q).unwrap());
    let m = a.len() / 2;
    if a.len() % 2 == 1 { a[m] } else { (a[m - 1] + a[m]) / 2.0 }
}

pub struct AggregateOpts {
    pub now: f64,
    pub max_age_ms: f64,
    pub min_clear_sec: f64,
    pub min_damage: f64,
}
impl Default for AggregateOpts {
    fn default() -> Self {
        Self {
            now: now_ms(),
            max_age_ms: 14.0 * 24.0 * 3600.0 * 1000.0,
            min_clear_sec: 3.0,
            min_damage: 1.0,
        }
    }
}

/// Aggregate run records into per-(stage) measured stats. Port of `aggregateRunsForFarm`.
pub fn aggregate_runs_for_farm(runs: &[RunRecord], opts: &AggregateOpts) -> Value {
    let usable: Vec<&RunRecord> = runs
        .iter()
        .filter(|r| {
            r.outcome == "success"
                && r.clear_time.unwrap_or(0.0) >= opts.min_clear_sec
                && r.total_damage.unwrap_or(0.0) >= opts.min_damage
                && r.stage_key.is_some()
                && (opts.now - r.ts) <= opts.max_age_ms
        })
        .collect();

    // Group by stageKey (a stageKey already determines the difficulty tier).
    let mut groups: HashMap<i64, Vec<&RunRecord>> = HashMap::new();
    for r in &usable {
        groups.entry(r.stage_key.unwrap()).or_default().push(r);
    }

    let mut stages: Vec<Value> = Vec::new();
    for (_k, rs) in groups.iter() {
        let clear_sec = median(&rs.iter().filter_map(|r| r.clear_time).collect::<Vec<_>>());
        let hp = median(&rs.iter().filter_map(|r| r.total_damage).collect::<Vec<_>>());
        let gold_vals: Vec<f64> = rs.iter().filter_map(|r| r.gold).collect();
        let xp_vals: Vec<f64> = rs.iter().filter_map(|r| r.xp).collect();
        let gold = if gold_vals.is_empty() { None } else { Some(median(&gold_vals)) };
        let xp = if xp_vals.is_empty() { None } else { Some(median(&xp_vals)) };
        let last_at = rs.iter().map(|r| r.ts).fold(0.0f64, f64::max);
        let difficulty = rs.iter().find_map(|r| r.difficulty);
        stages.push(json!({
            "stageKey": rs[0].stage_key,
            "difficulty": difficulty,
            "n": rs.len(),
            "clearSec": clear_sec,
            "hp": hp,
            "gold": gold,
            "xp": xp,
            "goldPerSec": gold.and_then(|g| if clear_sec > 0.0 { Some(g / clear_sec) } else { None }),
            "expPerSec": xp.and_then(|x| if clear_sec > 0.0 { Some(x / clear_sec) } else { None }),
            "dps": if clear_sec > 0.0 { hp / clear_sec } else { 0.0 },
            "lastAt": last_at,
        }));
    }
    stages.sort_by(|a, b| {
        b["lastAt"].as_f64().unwrap_or(0.0).partial_cmp(&a["lastAt"].as_f64().unwrap_or(0.0)).unwrap()
    });

    // The difficulty tier the player is actually farming now (most recently seen).
    let mut by_diff: HashMap<i64, (i64, usize, usize, f64)> = HashMap::new(); // diff -> (diff, runs, stages, lastAt)
    for s in &stages {
        let d = match s["difficulty"].as_i64() { Some(d) => d, None => continue };
        let e = by_diff.entry(d).or_insert((d, 0, 0, 0.0));
        e.1 += s["n"].as_u64().unwrap_or(0) as usize;
        e.2 += 1;
        e.3 = e.3.max(s["lastAt"].as_f64().unwrap_or(0.0));
    }
    let mut primary_difficulty: Option<i64> = None;
    let mut best = -1.0f64;
    for (_d, v) in by_diff.iter() {
        if v.3 > best { best = v.3; primary_difficulty = Some(v.0); }
    }
    let mut diffs: Vec<Value> = by_diff
        .values()
        .map(|v| json!({ "difficulty": v.0, "runs": v.1, "stages": v.2, "lastAt": v.3 }))
        .collect();
    diffs.sort_by(|a, b| b["lastAt"].as_f64().unwrap_or(0.0).partial_cmp(&a["lastAt"].as_f64().unwrap_or(0.0)).unwrap());

    json!({
        "ok": true,
        "stages": stages,
        "primaryDifficulty": primary_difficulty,
        "difficulties": diffs,
        "totalRuns": usable.len(),
        "generatedAt": opts.now,
    })
}

pub struct ClearSample { pub stage_key: i64, pub hp: f64, pub waves: f64, pub clear_sec: f64 }

/// Least-squares fit of clearSec - fixed = waveTax·waves + (1/D)·hp. Port of `fitClearModel`.
pub fn fit_clear_model(samples: &[ClearSample], fixed: f64) -> Option<(f64, f64, usize)> {
    let pts: Vec<&ClearSample> = samples.iter().filter(|s| s.clear_sec > 0.0 && s.hp > 0.0 && s.waves > 0.0).collect();
    if pts.len() < 2 { return None; }
    let (mut sww, mut swh, mut shh, mut swy, mut shy) = (0.0, 0.0, 0.0, 0.0, 0.0);
    for s in &pts {
        let (w, h, y) = (s.waves, s.hp, s.clear_sec - fixed);
        sww += w * w; swh += w * h; shh += h * h; swy += w * y; shy += h * y;
    }
    let det = sww * shh - swh * swh;
    if det.abs() < 1e-9 { return None; }
    let t_wave = (swy * shh - shy * swh) / det;
    let inv_d = (sww * shy - swh * swy) / det;
    if !(inv_d > 1e-12) || t_wave < 0.0 { return None; }
    Some((1.0 / inv_d, t_wave, pts.len()))
}

pub fn now_ms() -> f64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as f64)
        .unwrap_or(0.0)
}
