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
    // Deterministic: HashMap iteration order varies run to run, so ties on `lastAt` used to
    // pick a different tier each time — and the whole clear-time calibration keys on this.
    // Break ties on the higher difficulty id.
    let mut primary_difficulty: Option<i64> = None;
    let mut best = (-1.0f64, i64::MIN);
    for v in by_diff.values() {
        if (v.3, v.0) > best { best = (v.3, v.0); primary_difficulty = Some(v.0); }
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

/// Calibration constants from `data/farm-calibration.json`, needed to rank stages that have
/// never been farmed. See that file's `_comment`s for how each was derived.
pub struct RankCalib {
    /// `farm_stages.json`'s `totalHP` is this many times the real stage HP.
    pub stage_hp_scale: f64,
    pub per_wave_sec: f64,
    pub per_monster_sec: f64,
    pub fitted_dps: f64,
    /// The movement speed the above three constants were fitted at (~10.4). `perWaveSec` bakes
    /// in this speed silently — see `movementSpeedConfound` — so a party far from it should not
    /// trust the modelled clear time without a caveat attached.
    pub baseline_movement_speed: f64,
}

/// clearSec for a stage that has never been measured, from table-derived HP and the fitted
/// per-wave/per-monster/DPS constants. NOT to be presented next to a measured clearSec without
/// both being labelled — see the module-level ranking constraint this exists to satisfy.
fn modelled_clear_sec(total_hp_raw: f64, waves: f64, monsters: f64, c: &RankCalib) -> f64 {
    let real_hp = total_hp_raw / c.stage_hp_scale;
    c.per_wave_sec * waves + c.per_monster_sec * monsters + real_hp / c.fitted_dps
}

/// Rank every stage in `farm_stages.json` by gold/hr and exp/hr, WITHOUT ever comparing a
/// measured clear time to a modelled one directly. `measured` is the `"stages"` array from
/// [`aggregate_runs_for_farm`] — real player data, already clean (no `totalHP` in it at all).
/// Everything else is priced with the game's own `expectedGold`/`expectedEXP` (unaffected by the
/// HP-scale bug — that bug is specifically in `totalHP`) divided by whichever clear time applies.
///
/// Returns `{ measured: [...], modelled: [...] }` as two separate, separately-sorted lists. This
/// is the fix for the confirmed bug: the reference app puts both into one ranking, so whichever
/// stages the player happened to farm (accurate short clear times) get buried under unfarmed
/// stages whose modelled clear time is ~10x too fast (from the same HP bug), producing an
/// EXP/gold-per-hour figure inflated by up to ~78x. Keeping the lists apart makes that
/// impossible: a modelled figure can only ever be compared against other modelled figures.
pub fn rank_stages(stages: &[Value], measured: &[Value], calib: &RankCalib) -> Value {
    let by_key: HashMap<i64, &Value> = measured
        .iter()
        .filter_map(|m| Some((m.get("stageKey")?.as_i64()?, m)))
        .collect();

    let mut measured_out = Vec::new();
    let mut modelled_out = Vec::new();

    for s in stages {
        let Some(key) = s.get("key").and_then(|v| v.as_i64()) else { continue };
        let expected_gold = s.get("expectedGold").and_then(|v| v.as_f64()).unwrap_or(0.0);
        let expected_exp = s.get("expectedEXP").and_then(|v| v.as_f64()).unwrap_or(0.0);
        let label = s.get("label").cloned().unwrap_or(Value::Null);

        if let Some(m) = by_key.get(&key) {
            let clear_sec = m.get("clearSec").and_then(|v| v.as_f64()).unwrap_or(0.0);
            if clear_sec <= 0.0 { continue; }
            // Cross-check the table's reward constants against what was actually measured —
            // the same "surface disagreements instead of trusting one source" pattern used for
            // gear lines. A real mismatch here would mean expectedGold/expectedEXP themselves
            // need recalibrating, same class of bug as stageHpScale.
            let measured_gold_per_sec = m.get("goldPerSec").and_then(|v| v.as_f64());
            let table_gold_per_sec = if expected_gold > 0.0 { Some(expected_gold / clear_sec) } else { None };
            let gold_disagrees = match (measured_gold_per_sec, table_gold_per_sec) {
                (Some(a), Some(b)) if b > 0.0 => ((a - b).abs() / b) > 0.25,
                _ => false,
            };
            // Our own meter never fills RunRecord.xp (no game-authoritative source hooked up
            // yet), so `m.get("expPerSec")` is present but JSON `null` — NOT absent. `.unwrap_or`
            // only falls back on a missing KEY, not a null VALUE, so a naive
            // `.cloned().unwrap_or(fallback)` here would silently keep serialising `null`
            // forever instead of ever reaching the table fallback below.
            let measured_exp_per_sec = m.get("expPerSec").and_then(|v| v.as_f64());
            let table_exp_per_sec = if expected_exp > 0.0 { Some(expected_exp / clear_sec) } else { None };
            let gold_per_sec = measured_gold_per_sec.or(table_gold_per_sec);
            let exp_per_sec = measured_exp_per_sec.or(table_exp_per_sec);
            measured_out.push(json!({
                "stageKey": key, "label": label,
                "source": "measured", "n": m.get("n").cloned().unwrap_or(Value::Null),
                "clearSec": clear_sec,
                "goldPerSec": gold_per_sec, "expPerSec": exp_per_sec,
                "goldPerHour": gold_per_sec.map(|g| g * 3600.0),
                "expPerHour": exp_per_sec.map(|e| e * 3600.0),
                "tableGoldDisagreesWithMeasured": gold_disagrees,
            }));
        } else {
            let waves = s.get("waves").and_then(|v| v.as_f64()).unwrap_or(0.0);
            let monsters = s.get("count").and_then(|v| v.as_f64()).unwrap_or(0.0);
            let total_hp = s.get("totalHP").and_then(|v| v.as_f64()).unwrap_or(0.0);
            if waves <= 0.0 || total_hp <= 0.0 { continue; }
            let clear_sec = modelled_clear_sec(total_hp, waves, monsters, calib);
            if clear_sec <= 0.0 { continue; }
            let gold_per_sec = if expected_gold > 0.0 { expected_gold / clear_sec } else { 0.0 };
            let exp_per_sec = if expected_exp > 0.0 { expected_exp / clear_sec } else { 0.0 };
            modelled_out.push(json!({
                "stageKey": key, "label": label,
                "source": "modelled",
                "clearSec": clear_sec,
                "goldPerSec": gold_per_sec, "expPerSec": exp_per_sec,
                "goldPerHour": gold_per_sec * 3600.0, "expPerHour": exp_per_sec * 3600.0,
            }));
        }
    }

    let sort_desc = |v: &mut Vec<Value>, field: &str| {
        v.sort_by(|a, b| {
            b[field].as_f64().unwrap_or(0.0).partial_cmp(&a[field].as_f64().unwrap_or(0.0)).unwrap()
        });
    };
    sort_desc(&mut measured_out, "expPerHour");
    sort_desc(&mut modelled_out, "expPerHour");

    json!({
        "ok": true,
        "measured": measured_out,
        "modelled": modelled_out,
        "modelledCaveat": "Clear time for these stages is a MODEL, not a measurement — accurate \
            only near the calibration's baseline party movement speed. Never compare its \
            goldPerHour/expPerHour directly against the measured list.",
        "calibBaselineMovementSpeed": calib.baseline_movement_speed,
    })
}

/// Should the player keep farming `current_key` or switch? The decision ONLY ever compares
/// measured entries against other measured entries — never against the modelled list, for the
/// same reason `rank_stages` keeps the two apart. A separate, clearly-labelled "if you want to
/// explore" hint points at the modelled frontier, but never feeds the stay/switch verdict itself.
///
/// Sorts both lists itself rather than trusting the caller to have passed them in exp/hr order —
/// `rank_stages` already returns them sorted, but "best" silently meaning "first" is the kind of
/// implicit precondition that breaks quietly the moment either list is built a different way.
pub fn stay_vs_switch(measured: &[Value], modelled: &[Value], current_key: Option<i64>) -> Value {
    let exp_hr = |v: &&Value| v["expPerHour"].as_f64().unwrap_or(0.0);
    let best_measured = measured.iter().max_by(|a, b| exp_hr(a).partial_cmp(&exp_hr(b)).unwrap());
    let current_measured = current_key.and_then(|k| measured.iter().find(|r| r["stageKey"].as_i64() == Some(k)));
    let best_modelled = modelled.iter().max_by(|a, b| exp_hr(a).partial_cmp(&exp_hr(b)).unwrap());

    let verdict = match (current_measured, best_measured) {
        (Some(cur), Some(best)) if cur["stageKey"] == best["stageKey"] => "stay",
        (Some(_), Some(_)) => "switch",
        // No measured data at all (current stage, or anywhere) — nothing to judge yet.
        (None, _) | (_, None) => "unmeasured",
    };

    json!({
        "ok": true,
        "verdict": verdict,
        "current": current_measured,
        "bestMeasured": best_measured,
        "exploreHint": best_modelled.map(|m| json!({
            "stageKey": m["stageKey"], "label": m["label"], "expPerHour": m["expPerHour"],
            "caveat": "Modelled, not measured — farm it a few times before trusting this number.",
        })),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn calib() -> RankCalib {
        RankCalib {
            stage_hp_scale: 10.0978,
            per_wave_sec: 3.342,
            per_monster_sec: 0.475,
            fitted_dps: 1321.0,
            baseline_movement_speed: 10.38,
        }
    }

    /// Ground-truth check against the real stage-1101 numbers recorded in
    /// data/farm-calibration.json: waves=10, count=10, totalHP=560 measured ~41s live.
    #[test]
    fn modelled_clear_sec_matches_ground_truth_stage_1101() {
        let sec = modelled_clear_sec(560.0, 10.0, 10.0, &calib());
        assert!((sec - 38.7).abs() < 2.0, "got {sec}, expected close to the measured ~41s");
    }

    /// The whole point of this function: a stage present in `measured` must never end up priced
    /// by the model, and vice versa — that mixing is the confirmed bug this replaces.
    #[test]
    fn measured_and_modelled_stay_partitioned() {
        let stages = vec![
            json!({ "key": 1101, "label": "1-1", "waves": 10, "count": 10, "totalHP": 560.0, "expectedGold": 14.0, "expectedEXP": 16.0 }),
            json!({ "key": 1102, "label": "1-2", "waves": 11, "count": 22, "totalHP": 2040.0, "expectedGold": 30.0, "expectedEXP": 40.0 }),
        ];
        let measured = vec![
            json!({ "stageKey": 1101, "n": 5, "clearSec": 41.0, "goldPerSec": 2.0, "expPerSec": 3.0 }),
        ];
        let out = rank_stages(&stages, &measured, &calib());
        let m: Vec<i64> = out["measured"].as_array().unwrap().iter().map(|r| r["stageKey"].as_i64().unwrap()).collect();
        let d: Vec<i64> = out["modelled"].as_array().unwrap().iter().map(|r| r["stageKey"].as_i64().unwrap()).collect();
        assert_eq!(m, vec![1101]);
        assert_eq!(d, vec![1102]);
        // No stage may appear in both.
        assert!(m.iter().all(|k| !d.contains(k)));
    }

    #[test]
    fn stay_vs_switch_never_compares_across_lists() {
        let measured = vec![
            json!({ "stageKey": 1101, "expPerHour": 1000.0 }),
            json!({ "stageKey": 1102, "expPerHour": 2000.0 }),
        ];
        let modelled = vec![json!({ "stageKey": 1103, "label": "1-3", "expPerHour": 999_999.0 })];

        // Currently farming the WORSE measured stage: must recommend switching to the better
        // MEASURED one, never to the modelled stage even though its number looks huge.
        let out = stay_vs_switch(&measured, &modelled, Some(1101));
        assert_eq!(out["verdict"], json!("switch"));
        assert_eq!(out["bestMeasured"]["stageKey"], json!(1102));
        assert_eq!(out["exploreHint"]["stageKey"], json!(1103)); // present, but separate

        // Currently on the best measured stage: stay.
        let out2 = stay_vs_switch(&measured, &modelled, Some(1102));
        assert_eq!(out2["verdict"], json!("stay"));

        // No measured data for the current stage at all: don't pretend to judge it.
        let out3 = stay_vs_switch(&measured, &modelled, Some(9999));
        assert_eq!(out3["verdict"], json!("unmeasured"));
    }
}
