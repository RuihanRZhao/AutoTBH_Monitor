//! Save-derived insights: party, progression, currency, runes, pets, attributes, lifetime stats.
//!
//! This is native Rust computed directly from `PlayerSaveData` — it does NOT use the original
//! app's JavaScript simulation engine. Anything that genuinely needs that engine (party
//! DPS/EHP/POWER modelling, gear scoring, farm clear-time prediction) is reported under
//! `engine: { pending: true }` rather than guessed at.

use crate::save;
use serde_json::{json, Map, Value};
use std::collections::HashMap;

/// Steam/game currency key for gold.
const GOLD_KEY: i64 = 100001;

fn as_arr(v: &Value) -> Vec<Value> {
    match v {
        Value::Array(a) => a.clone(),
        Value::String(s) => serde_json::from_str::<Value>(s)
            .ok()
            .and_then(|x| x.as_array().cloned())
            .unwrap_or_default(),
        _ => Vec::new(),
    }
}

fn as_obj(v: &Value) -> Value {
    match v {
        Value::String(s) => serde_json::from_str::<Value>(s).unwrap_or(Value::Null),
        other => other.clone(),
    }
}

fn i(v: &Value, k: &str) -> i64 { v.get(k).and_then(|x| x.as_i64()).unwrap_or(0) }
fn f(v: &Value, k: &str) -> f64 { v.get(k).and_then(|x| x.as_f64()).unwrap_or(0.0) }
fn b(v: &Value, k: &str) -> bool { v.get(k).and_then(|x| x.as_bool()).unwrap_or(false) }

/// Aggregate type ids we surface (see data/meter-offsets.json → enums.EAggregateType).
fn aggregate_label(t: i64) -> &'static str {
    match t {
        0 => "monsterKill", 1 => "heroDeath", 2 => "goldEarn", 3 => "boxObtain",
        4 => "itemObtain", 5 => "synthesis", 6 => "alchemy", 7 => "crafting",
        8 => "offering", 9 => "extraction", 10 => "decoration", 11 => "engraving",
        12 => "inscription", 13 => "stageClear", 14 => "stageFail", 15 => "playTime",
        16 => "boxOpen", _ => "other",
    }
}

pub fn build() -> anyhow::Result<Value> {
    let raw = save::player_save_data_string()?;
    let psd: Value = serde_json::from_str(&raw)?;

    let common = as_obj(&psd["commonSaveData"]);
    let heroes_raw = as_arr(&psd["heroSaveDatas"]);
    let currencies = as_arr(&psd["currenySaveDatas"]); // NB: the game misspells "curreny"
    let runes = as_arr(&psd["RuneSaveData"]);
    let pets = as_arr(&psd["PetSaveData"]);
    let attributes = as_arr(&psd["attributeSaveDatas"]);
    let aggregates = as_arr(&psd["aggregateSaveDatas"]);
    let inventory = as_arr(&psd["inventorySaveDatas"]);
    let stash = as_arr(&psd["stashSaveDatas"]);

    // ── party / heroes ──────────────────────────────────────────────────────
    let arranged: Vec<i64> = common
        .get("arrangedHeroKey")
        .and_then(|v| v.as_array())
        .map(|a| a.iter().filter_map(|x| x.as_i64()).filter(|k| *k > 0).collect())
        .unwrap_or_default();

    let mut heroes: Vec<Value> = heroes_raw
        .iter()
        .map(|h| {
            let key = i(h, "heroKey");
            let equipped: Vec<i64> = h
                .get("equippedItemIds")
                .and_then(|v| v.as_array())
                .map(|a| a.iter().filter_map(|x| x.as_i64()).collect())
                .unwrap_or_default();
            json!({
                "heroKey": key,
                "level": i(h, "HeroLevel"),
                "exp": f(h, "HeroExp"),
                "unlocked": b(h, "IsUnLock"),
                "abilityPoint": i(h, "AbilityPoint"),
                "allocatedAbilityPoint": i(h, "AllocatedHeroAbilityPoint"),
                "equippedCount": equipped.iter().filter(|x| **x > 0).count(),
                "equippedItemIds": equipped,
                "skills": h.get("equippedSKillKey").cloned().unwrap_or(json!([])),
                "inParty": arranged.contains(&key),
                "partySlot": arranged.iter().position(|k| *k == key),
            })
        })
        .collect();
    // party first (in formation order), then by level
    heroes.sort_by(|a, b| {
        let pa = a["partySlot"].as_i64().unwrap_or(99);
        let pb = b["partySlot"].as_i64().unwrap_or(99);
        pa.cmp(&pb).then(b["level"].as_i64().unwrap_or(0).cmp(&a["level"].as_i64().unwrap_or(0)))
    });

    let unlocked_heroes = heroes.iter().filter(|h| h["unlocked"].as_bool() == Some(true)).count();
    let unspent_points: i64 = heroes.iter().map(|h| h["abilityPoint"].as_i64().unwrap_or(0)).sum();
    let lowest_party_hero = heroes
        .iter()
        .filter(|h| h["inParty"].as_bool() == Some(true))
        .min_by_key(|h| h["level"].as_i64().unwrap_or(i64::MAX))
        .cloned();

    // ── currency ────────────────────────────────────────────────────────────
    let mut currency_map = Map::new();
    let mut gold = 0i64;
    for c in &currencies {
        let key = i(c, "Key");
        let qty = i(c, "Quantity");
        if key == GOLD_KEY { gold = qty; }
        currency_map.insert(key.to_string(), json!(qty));
    }

    // ── runes ───────────────────────────────────────────────────────────────
    let rune_total = runes.len();
    let leveled: Vec<&Value> = runes.iter().filter(|r| i(r, "Level") > 0).collect();
    let rune_levels: i64 = leveled.iter().map(|r| i(r, "Level")).sum();

    // ── pets ────────────────────────────────────────────────────────────────
    let pets_unlocked = pets.iter().filter(|p| b(p, "IsUnlock")).count();

    // ── attributes ──────────────────────────────────────────────────────────
    let attr_levels: i64 = attributes.iter().map(|a| i(a, "Level")).sum();

    // ── lifetime aggregates (Type -> summed Value across subkeys) ───────────
    let mut agg_map: HashMap<&'static str, i64> = HashMap::new();
    for a in &aggregates {
        // SubKey 0 is the rollup for a type; prefer it, else sum the parts.
        if i(a, "SubKey") == 0 {
            agg_map.insert(aggregate_label(i(a, "Type")), i(a, "Value"));
        } else {
            agg_map.entry(aggregate_label(i(a, "Type"))).or_insert(0);
        }
    }
    let agg_json: Map<String, Value> =
        agg_map.into_iter().map(|(k, v)| (k.to_string(), json!(v))).collect();

    // ── storage ─────────────────────────────────────────────────────────────
    let used = |slots: &[Value]| -> usize {
        slots
            .iter()
            .filter(|s| {
                s.get("ItemUniqueId")
                    .map(|v| match v {
                        Value::Number(n) => n.as_i64().unwrap_or(0) != 0,
                        Value::String(t) => !t.is_empty() && t != "0",
                        _ => false,
                    })
                    .unwrap_or(false)
            })
            .count()
    };
    let stash_used = used(&stash);
    let inv_used = used(&inventory);

    // ── progression ─────────────────────────────────────────────────────────
    let play_time = f(&common, "playTime");
    let progression = json!({
        "gameVersion": common.get("version"),
        "currentStageKey": i(&common, "currentStageKey"),
        "currentStageWave": i(&common, "currentStageWave"),
        "maxCompletedStage": i(&common, "maxCompletedStage"),
        "playTimeSec": play_time,
        "playTimeHours": (play_time / 3600.0 * 10.0).round() / 10.0,
        "arrangedPetKey": i(&common, "ArrangedPetKey"),
    });

    // ── "next best move" heuristics (save-derived only, no simulation) ──────
    let mut todo: Vec<Value> = Vec::new();
    if unspent_points > 0 {
        todo.push(json!({
            "kind": "abilityPoints",
            "text": format!("{unspent_points} unspent ability point(s)"),
            "priority": 1
        }));
    }
    if let Some(h) = &lowest_party_hero {
        todo.push(json!({
            "kind": "levelLaggard",
            "heroKey": h["heroKey"],
            "text": format!("Hero {} is your lowest fielded hero (Lv {})",
                            h["heroKey"].as_i64().unwrap_or(0), h["level"].as_i64().unwrap_or(0)),
            "priority": 2
        }));
    }
    if stash_used as f64 >= 0.9 * (stash.len().max(1) as f64) {
        todo.push(json!({ "kind": "stashFull", "text": "Stash is nearly full", "priority": 1 }));
    }
    todo.sort_by_key(|t| t["priority"].as_i64().unwrap_or(9));
    let headline = todo.first().and_then(|t| t["text"].as_str()).map(String::from);

    Ok(json!({
        "found": true,
        "saveMtime": save::save_mtime(),
        "insights": {
            "headline": headline,
            "todo": todo,
            "progression": progression,
            "party": arranged,
            "heroes": heroes,
            "heroSummary": {
                "total": heroes_raw.len(),
                "unlocked": unlocked_heroes,
                "unspentAbilityPoints": unspent_points,
            },
            "gold": gold,
            "currencies": currency_map,
            "runes": {
                "total": rune_total,
                "leveled": leveled.len(),
                "totalLevels": rune_levels,
            },
            "pets": { "total": pets.len(), "unlocked": pets_unlocked },
            "attributes": { "total": attributes.len(), "totalLevels": attr_levels },
            "lifetime": agg_json,
            "storage": {
                "stash": { "used": stash_used, "slots": stash.len() },
                "inventory": { "used": inv_used, "slots": inventory.len() },
            },
            // Explicitly NOT guessed: these need the game's simulation engine.
            "engine": {
                "pending": true,
                "missing": ["partyDps", "partyEhp", "power", "gearScoring", "farmClearTimeModel", "upgradeFinder"],
            },
        }
    }))
}
