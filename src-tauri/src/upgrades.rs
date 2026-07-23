//! Gear-swap simulation.
//!
//! ## Why this needs the modifier decomposition
//!
//! Stats aggregate as `ΣFLAT × (1 + ΣADDITIVE) × (1 + ΣMULTIPLICATIVE)`. Knowing a hero's final
//! stat and one item's contribution is not enough to predict what happens when that item is
//! replaced: removing a FLAT term rescales differently than removing an ADDITIVE one, and the
//! final value alone cannot tell you which bucket a contribution sat in. `read_party_modifiers`
//! recovers the buckets from the game itself, so a swap can be simulated by subtracting the
//! outgoing item's lines and adding the incoming item's, then re-aggregating.
//!
//! ## Why reconciliation gates the whole feature
//!
//! That subtraction is only valid if the ITEM-sourced modifiers the game holds are exactly the
//! lines we resolve for the equipped items. If they disagree — a missing enchant, an unmapped
//! gear type, a stat whose display scale is unverified — then subtracting our lines removes the
//! wrong amount and every delta downstream is quietly wrong. So `reconcile` runs first and the
//! simulation refuses to emit numbers for any stat that does not balance. A blank field is
//! recoverable; a confident wrong number is not.
//!
//! ## Units
//!
//! Modifier values are game-native; gear lines are in display units. The conversion is
//! PER-BUCKET, not per-stat:
//!
//! * FLAT scales by the stat's own factor (`engine::game_to_display_scale`) — AttackDamage 1x,
//!   AttackSpeed 100x, DodgeChance 1000x. Unverified stats are reported as `unverifiedScale`
//!   rather than guessed at.
//! * ADDITIVE and MULTIPLICATIVE are per-mille for every stat alike: AttackDamage ADDITIVE reads
//!   0.605 in the game against 605 in the gear table, even though its FLAT factor is 1x.
//!
//! Applying the stat factor to all three buckets is wrong and mostly invisible, because it is a
//! no-op for the 1x stats that dominate a hero sheet. This is the fourth unit-scaling bug found
//! in this codebase, which is why the reconciliation gate below exists at all.

use crate::engine::{self, ModType, StatContrib};
use serde_json::{json, Map, Value};
use std::collections::HashMap;

/// Absolute tolerance for reconciliation, in game-native units. Modifier values are read as f32,
/// so exact equality is not available; this is well below the smallest real stat line.
const EPS: f64 = 1e-4;

fn stat_id_by_name() -> HashMap<String, i64> {
    (0..64).map(|i| (engine::stat_name(i).to_string(), i)).collect()
}

fn mode_index(mode: &str) -> usize {
    match mode {
        "ADDITIVE" => 1,
        "MULTIPLICATIVE" => 2,
        _ => 0,
    }
}

fn mode_name(i: usize) -> &'static str {
    ["FLAT", "ADDITIVE", "MULTIPLICATIVE"][i]
}

/// Display-unit -> game-native divisor for one bucket of one stat.
/// `None` when the stat's FLAT factor has not been verified against a live process.
fn bucket_divisor(stat: &str, bucket: usize, ids: &HashMap<String, i64>) -> Option<f64> {
    match bucket {
        0 => ids.get(stat).and_then(|id| engine::game_to_display_scale(*id)),
        _ => Some(1000.0), // ADDITIVE / MULTIPLICATIVE are per-mille regardless of stat
    }
}

/// Per-stat, per-bucket sums keyed by stat name.
type Buckets = HashMap<String, [f64; 3]>;

/// Sum an equipped hero's gear lines (intrinsic + enchant) into game-native buckets.
///
/// Returns the buckets plus the set of stats that had to be skipped because their display scale
/// is unverified — the caller must not pretend those reconcile.
fn gear_buckets(slots: &[Value], ids: &HashMap<String, i64>) -> (Buckets, Vec<String>) {
    let mut out: Buckets = HashMap::new();
    let mut unverified = Vec::new();
    for s in slots {
        for key in ["lines", "enchantLines"] {
            for l in s.get(key).and_then(|v| v.as_array()).map(|a| a.as_slice()).unwrap_or(&[]) {
                let (Some(stat), Some(mode), Some(val)) = (
                    l.get(0).and_then(|v| v.as_str()),
                    l.get(1).and_then(|v| v.as_str()),
                    l.get(2).and_then(|v| v.as_f64()),
                ) else {
                    continue;
                };
                let bucket = mode_index(mode);
                let Some(div) = bucket_divisor(stat, bucket, ids) else {
                    if !unverified.iter().any(|u| u == stat) {
                        unverified.push(stat.to_string());
                    }
                    continue;
                };
                out.entry(stat.to_string()).or_insert([0.0; 3])[bucket] += val / div;
            }
        }
    }
    (out, unverified)
}

/// Sum the game's own ITEM-sourced (`MOD_SOURCE == 1`) modifiers into buckets.
fn item_source_buckets(stats: &Value) -> Buckets {
    let mut out: Buckets = HashMap::new();
    for (stat, entry) in stats.as_object().map(|m| m.iter().collect::<Vec<_>>()).unwrap_or_default() {
        for m in entry.get("mods").and_then(|v| v.as_array()).map(|a| a.as_slice()).unwrap_or(&[]) {
            if m.get("source").and_then(|v| v.as_i64()) != Some(1) {
                continue;
            }
            let mode = m.get("mode").and_then(|v| v.as_i64()).unwrap_or(-1);
            let v = m.get("value").and_then(|v| v.as_f64()).unwrap_or(0.0);
            if !(0..=2).contains(&mode) {
                continue;
            }
            out.entry(stat.clone()).or_insert([0.0; 3])[mode as usize] += v;
        }
    }
    out
}

/// Compare the game's ITEM-sourced modifiers against our resolved gear lines, per hero.
///
/// The result is the gate for gear-swap simulation: only stats reported as balanced may be
/// simulated. Everything else is named explicitly so the gap is visible rather than absorbed.
pub fn reconcile(gear: &Value, modifiers: &[Value]) -> Value {
    let ids = stat_id_by_name();
    let mods_by_hero: HashMap<i64, &Value> = modifiers
        .iter()
        .filter_map(|h| Some((h.get("heroKey")?.as_i64()?, h.get("stats")?)))
        .collect();

    let mut heroes = Vec::new();
    for h in gear.get("heroes").and_then(|v| v.as_array()).map(|a| a.as_slice()).unwrap_or(&[]) {
        let hero_key = h.get("heroKey").and_then(|v| v.as_i64()).unwrap_or(0);
        // Heroes not on the field have no runtime modifiers to compare against; that is expected,
        // not a mismatch.
        let Some(stats) = mods_by_hero.get(&hero_key) else { continue };
        let slots = h.get("slots").and_then(|v| v.as_array()).cloned().unwrap_or_default();
        let (ours, unverified) = gear_buckets(&slots, &ids);
        let theirs = item_source_buckets(stats);

        let mut all: Vec<String> = ours.keys().chain(theirs.keys()).cloned().collect();
        all.sort();
        all.dedup();

        let (mut balanced, mut mismatched) = (Vec::new(), Vec::new());
        for stat in all {
            if unverified.contains(&stat) {
                continue;
            }
            let a = ours.get(&stat).copied().unwrap_or([0.0; 3]);
            let b = theirs.get(&stat).copied().unwrap_or([0.0; 3]);
            let bad: Vec<Value> = (0..3)
                .filter(|i| (a[*i] - b[*i]).abs() > EPS)
                .map(|i| json!({ "mode": mode_name(i), "ours": a[i], "game": b[i] }))
                .collect();
            if bad.is_empty() {
                balanced.push(stat);
            } else {
                mismatched.push(json!({ "stat": stat, "buckets": bad }));
            }
        }
        heroes.push(json!({
            "heroKey": hero_key,
            "balanced": balanced,
            "mismatched": mismatched,
            "unverifiedScale": unverified,
        }));
    }
    let total_bad: usize = heroes
        .iter()
        .map(|h| h["mismatched"].as_array().map(|a| a.len()).unwrap_or(0))
        .sum();
    json!({ "ok": total_bad == 0, "mismatchCount": total_bad, "heroes": heroes })
}

/// Buckets for every stat of one hero, in game-native units, from the full modifier list.
fn all_buckets(stats: &Value) -> Buckets {
    let mut out: Buckets = HashMap::new();
    for (stat, e) in stats.as_object().map(|m| m.iter().collect::<Vec<_>>()).unwrap_or_default() {
        let sum = |k: &str| {
            e.get(k)
                .and_then(|v| v.as_array())
                .map(|a| a.iter().filter_map(|x| x.as_f64()).sum::<f64>())
                .unwrap_or(0.0)
        };
        out.insert(stat.clone(), [sum("flat"), sum("additive"), sum("multiplicative")]);
    }
    out
}

fn to_contrib(b: &[f64; 3]) -> StatContrib {
    let mut c = StatContrib::default();
    c.push(ModType::Flat, b[0]);
    c.push(ModType::Additive, b[1] * 1000.0);
    c.push(ModType::Multiplicative, b[2] * 100.0);
    c
}

/// Aggregate buckets into resolved stat values, staying in GAME-NATIVE units.
///
/// Deliberately not converted to display units here: `engine::auto_dps_game` and
/// `ehp_from_stats` are fed game-native values in `insights.rs`, and the two call sites must
/// agree or a swap delta will not be comparable to the hero's own displayed DPS. AttackSpeed is
/// the trap — display is 100x game-native, so converting first inflates DPS by 100x.
fn resolve(buckets: &Buckets) -> HashMap<String, f64> {
    buckets
        .iter()
        .map(|(stat, b)| (stat.clone(), engine::aggregate_stat(&to_contrib(b))))
        .collect()
}

/// DPS / EHP / POWER from game-native stats. Mirrors `insights::live_combat` exactly, including
/// the dodge conversion (game-native fraction -> per-mille display -> percent, which
/// `damage_taken_fraction` expects).
fn score(s: &HashMap<String, f64>, stage_level: f64) -> (f64, f64, f64) {
    let p = engine::Params::default();
    let g = |k: &str, d: f64| s.get(k).copied().unwrap_or(d);
    let dps = engine::auto_dps_game(
        g("AttackDamage", 0.0),
        g("AttackSpeed", 0.0),
        g("CriticalChance", 0.0),
        g("CriticalDamage", 1.0),
        &p,
    );
    let dodge = g("DodgeChance", 0.0) * engine::game_to_display_scale(16).unwrap_or(1000.0);
    let ehp = engine::ehp_from_stats(g("MaxHp", 0.0), g("Armor", 0.0), stage_level, dodge, &p);
    (dps, ehp, engine::power(dps, ehp))
}

/// Simulate replacing one equipped item with a candidate, returning the deltas.
///
/// `remove` / `add` are the outgoing and incoming items' lines in display units.
fn simulate(
    base: &Buckets,
    remove: &[Value],
    add: &[Value],
    ids: &HashMap<String, i64>,
    stage_level: f64,
) -> (f64, f64, f64, Vec<String>) {
    let mut b = base.clone();
    let mut skipped: Vec<String> = Vec::new();
    for (lines, sign) in [(remove, -1.0), (add, 1.0)] {
        for l in lines {
            let (Some(stat), Some(mode), Some(val)) = (
                l.get(0).and_then(|v| v.as_str()),
                l.get(1).and_then(|v| v.as_str()),
                l.get(2).and_then(|v| v.as_f64()),
            ) else {
                continue;
            };
            let bucket = mode_index(mode);
            // A line whose scale is unverified is dropped from the maths but NAMED on the result.
            // Dropping the whole candidate instead would remove an item from the ranking with no
            // trace, which reads as "you own nothing better" — the one wrong answer here.
            let Some(div) = bucket_divisor(stat, bucket, ids) else {
                if !skipped.iter().any(|x| x == stat) {
                    skipped.push(stat.to_string());
                }
                continue;
            };
            b.entry(stat.to_string()).or_insert([0.0; 3])[bucket] += sign * val / div;
        }
    }
    let (d, e, p) = score(&resolve(&b), stage_level);
    (d, e, p, skipped)
}

/// Per-slot upgrade candidates for each fielded hero.
///
/// Candidates are drawn from the player's own stash: items of the same gear type that are not
/// currently equipped. This deliberately does not rank market items — buying advice needs price
/// data joined per candidate, which is a separate concern from whether the swap is an upgrade.
pub fn build(gear: &Value, modifiers: &[Value], stash: &Value, stage_level: f64) -> Value {
    let ids = stat_id_by_name();
    let recon = reconcile(gear, modifiers);
    // Per-hero gate, not all-or-nothing: a single hero whose gear lines don't reconcile must not
    // suppress swap advice for the others. Heroes that don't balance are marked `blocked` with
    // the offending stats; heroes that do get full deltas. (An earlier version bailed the whole
    // response on any mismatch, so swapping in one hero with unmapped gear silently killed the
    // feature for the entire party.)
    let blocked: HashMap<i64, Value> = recon.get("heroes").and_then(|v| v.as_array())
        .map(|a| a.iter().filter_map(|h| {
            let key = h.get("heroKey")?.as_i64()?;
            let bad = h.get("mismatched")?.as_array()?;
            (!bad.is_empty()).then(|| (key, json!(bad)))
        }).collect())
        .unwrap_or_default();

    let mods_by_hero: HashMap<i64, &Value> = modifiers
        .iter()
        .filter_map(|h| Some((h.get("heroKey")?.as_i64()?, h.get("stats")?)))
        .collect();

    // Stash items grouped by gear type, so a slot only sees plausible replacements.
    let mut by_type: HashMap<String, Vec<&Value>> = HashMap::new();
    for it in stash.get("items").and_then(|v| v.as_array()).map(|a| a.as_slice()).unwrap_or(&[]) {
        if let Some(t) = it.get("gearType").and_then(|v| v.as_str()) {
            by_type.entry(t.to_uppercase()).or_default().push(it);
        }
    }

    let mut heroes = Vec::new();
    for h in gear.get("heroes").and_then(|v| v.as_array()).map(|a| a.as_slice()).unwrap_or(&[]) {
        let hero_key = h.get("heroKey").and_then(|v| v.as_i64()).unwrap_or(0);
        let Some(stats) = mods_by_hero.get(&hero_key) else { continue };
        let base = all_buckets(stats);
        let (base_dps, base_ehp, base_power) = score(&resolve(&base), stage_level);

        // This hero's gear didn't reconcile — report its current stats but no swap deltas, since
        // subtracting lines we can't account for would remove the wrong amount.
        if let Some(bad) = blocked.get(&hero_key) {
            let mut cur_stats = Map::new();
            for (k, v) in resolve(&base) {
                if let Some(sc) = ids.get(&k).and_then(|id| engine::game_to_display_scale(*id)) {
                    cur_stats.insert(k, json!(v * sc));
                }
            }
            heroes.push(json!({
                "heroKey": hero_key, "blocked": true, "mismatched": bad,
                "dps": base_dps, "ehp": base_ehp, "power": base_power,
                "stats": cur_stats, "slots": [],
            }));
            continue;
        }

        let mut slots = Vec::new();
        for s in h.get("slots").and_then(|v| v.as_array()).map(|a| a.as_slice()).unwrap_or(&[]) {
            let gear_type = s.get("gearType").and_then(|v| v.as_str()).unwrap_or("").to_uppercase();
            let equipped_uid = s.get("uniqueId").and_then(|v| v.as_str()).unwrap_or("");
            let mut cur: Vec<Value> = Vec::new();
            for k in ["lines", "enchantLines"] {
                cur.extend(s.get(k).and_then(|v| v.as_array()).cloned().unwrap_or_default());
            }

            let mut cands = Vec::new();
            for it in by_type.get(&gear_type).map(|v| v.as_slice()).unwrap_or(&[]) {
                if it.get("uniqueId").and_then(|v| v.as_str()) == Some(equipped_uid) {
                    continue;
                }
                let mut lines: Vec<Value> = Vec::new();
                for k in ["lines", "enchantLines"] {
                    lines.extend(it.get(k).and_then(|v| v.as_array()).cloned().unwrap_or_default());
                }
                let (dps, ehp, power, skipped) =
                    simulate(&base, &cur, &lines, &ids, stage_level);
                cands.push(json!({
                    "uniqueId": it.get("uniqueId").cloned().unwrap_or(Value::Null),
                    "itemKey": it.get("itemKey").cloned().unwrap_or(Value::Null),
                    "name": it.get("name").cloned().unwrap_or(Value::Null),
                    "dps": dps, "ehp": ehp, "power": power,
                    "dDps": dps - base_dps, "dEhp": ehp - base_ehp, "dPower": power - base_power,
                    // Non-empty means this comparison ignored some of the item's lines.
                    "ignoredStats": skipped,
                }));
            }
            cands.sort_by(|a, b| {
                b["dPower"].as_f64().unwrap_or(0.0).total_cmp(&a["dPower"].as_f64().unwrap_or(0.0))
            });
            let best = cands.first().cloned().unwrap_or(Value::Null);
            cands.truncate(5);
            slots.push(json!({
                "slot": s.get("slot").cloned().unwrap_or(Value::Null),
                "gearType": gear_type,
                "equipped": s.get("itemKey").cloned().unwrap_or(Value::Null),
                "best": best,
                "candidates": cands,
            }));
        }

        // Reported in display units so the UI matches the in-game numbers; the maths above
        // stays game-native. Stats without a verified scale are omitted, not guessed.
        let mut cur_stats = Map::new();
        for (k, v) in resolve(&base) {
            if let Some(sc) = ids.get(&k).and_then(|id| engine::game_to_display_scale(*id)) {
                cur_stats.insert(k, json!(v * sc));
            }
        }
        heroes.push(json!({
            "heroKey": hero_key,
            "dps": base_dps, "ehp": base_ehp, "power": base_power,
            "stats": cur_stats,
            "slots": slots,
        }));
    }

    json!({
        "ok": true,
        "stageLevel": stage_level,
        "candidateSource": "stash",
        "blockedHeroes": blocked.keys().collect::<Vec<_>>(),
        "reconciliation": recon,
        "heroes": heroes,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line(stat: &str, mode: &str, v: f64) -> Value {
        json!([stat, mode, v])
    }

    /// A swap that changes nothing must produce exactly zero deltas — the simplest way to catch
    /// an asymmetry between the remove and add paths.
    #[test]
    fn identical_swap_is_neutral() {
        let ids = stat_id_by_name();
        let mut base: Buckets = HashMap::new();
        base.insert("AttackDamage".into(), [31.0, 0.605, 0.0]);
        base.insert("AttackSpeed".into(), [1.05, 0.183, 0.0]);
        base.insert("MaxHp".into(), [380.0, 0.081, 0.0]);
        let item = vec![line("AttackDamage", "FLAT", 12.0), line("AttackDamage", "ADDITIVE", 40.0)];
        let (d0, e0, p0) = score(&resolve(&base), 100.0);
        let (d1, e1, p1, _) = simulate(&base, &item, &item, &ids, 100.0);
        assert!((d0 - d1).abs() < 1e-9, "dps {d0} vs {d1}");
        assert!((e0 - e1).abs() < 1e-9);
        assert!((p0 - p1).abs() < 1e-9);
    }

    /// Removing an item and adding a strictly better one must raise DPS, and the ADDITIVE bucket
    /// must scale the FLAT one (not add to it) — the whole reason buckets are needed.
    #[test]
    fn additive_scales_flat() {
        let ids = stat_id_by_name();
        let mut base: Buckets = HashMap::new();
        base.insert("AttackDamage".into(), [100.0, 0.0, 0.0]);
        base.insert("AttackSpeed".into(), [1.0, 0.0, 0.0]);
        let flat = vec![line("AttackDamage", "FLAT", 100.0)];
        let addv = vec![line("AttackDamage", "ADDITIVE", 1000.0)]; // +100% in game units
        let (fd, _, _, _) = simulate(&base, &[], &flat, &ids, 100.0);
        let (ad, _, _, _) = simulate(&base, &[], &addv, &ids, 100.0);
        // Both double a 100-point base, so they must agree; if ADDITIVE were treated as flat the
        // second would come out at 1100 instead of 200.
        assert!((fd - ad).abs() < 1e-6, "flat {fd} vs additive {ad}");
    }

    /// A line whose scale is unverified must not delete the candidate from the ranking — it must
    /// still be scored on its remaining lines and say what it ignored.
    #[test]
    fn unverified_line_is_reported_not_dropped() {
        let ids = stat_id_by_name();
        let mut base: Buckets = HashMap::new();
        base.insert("AttackDamage".into(), [100.0, 0.0, 0.0]);
        base.insert("AttackSpeed".into(), [1.0, 0.0, 0.0]);
        let item = vec![line("AttackDamage", "FLAT", 50.0), line("BlockChance", "FLAT", 30.0)];
        let (dps, _, _, skipped) = simulate(&base, &[], &item, &ids, 100.0);
        assert_eq!(skipped, vec!["BlockChance".to_string()]);
        assert!(dps > 0.0, "the verified AttackDamage line must still count");
    }

    /// Reconciliation must FAIL loudly when our lines disagree with the game's ITEM modifiers,
    /// because that is exactly the case where swap deltas are silently wrong.
    #[test]
    fn reconcile_detects_disagreement() {
        let gear = json!({ "heroes": [{
            "heroKey": 401,
            "slots": [{ "lines": [["AttackDamage", "FLAT", 17.0]], "enchantLines": [] }],
        }]});
        let agree = vec![json!({ "heroKey": 401, "stats": {
            "AttackDamage": { "flat": [17.0], "additive": [], "multiplicative": [],
                              "mods": [{ "mode": 0, "value": 17.0, "source": 1 }] } } })];
        assert_eq!(reconcile(&gear, &agree)["ok"], json!(true));

        let disagree = vec![json!({ "heroKey": 401, "stats": {
            "AttackDamage": { "flat": [12.0], "additive": [], "multiplicative": [],
                              "mods": [{ "mode": 0, "value": 12.0, "source": 1 }] } } })];
        let r = reconcile(&gear, &disagree);
        assert_eq!(r["ok"], json!(false));
        assert_eq!(r["mismatchCount"], json!(1));
    }
}
