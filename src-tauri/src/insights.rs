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

/// Level of the content the player is currently on — the reference point our survivability
/// metric is measured against. Stage key comes from the save (game-authoritative); the
/// key→level mapping comes from the bundled stage table.
pub fn current_stage_level(data_dir: &std::path::Path) -> Option<f64> {
    stage_level_for(data_dir, current_stage_key()?)
}

/// The stage key the save says the player is currently on. Game-authoritative (comes straight
/// from `commonSaveData`), used to decide which farm-rank entry is "current" for stay-vs-switch.
pub fn current_stage_key() -> Option<i64> {
    let raw = save::player_save_data_string().ok()?;
    let psd: Value = serde_json::from_str(&raw).ok()?;
    let common = as_obj(&psd["commonSaveData"]);
    common.get("currentStageKey").and_then(|v| v.as_i64())
}

/// Player's current gold from the parsed save (currency key 100001).
pub fn current_gold(psd: &Value) -> i64 {
    for c in as_arr(&psd["currenySaveDatas"]) { // NB: the game misspells "curreny"
        if i(&c, "Key") == GOLD_KEY {
            return i(&c, "Quantity");
        }
    }
    0
}

/// Look up a stage's level in the bundled stage table.
pub fn stage_level_for(data_dir: &std::path::Path, stage_key: i64) -> Option<f64> {
    let txt = std::fs::read_to_string(data_dir.join("engine/codex.json")).ok()?;
    let v: Value = serde_json::from_str(&txt).ok()?;
    v.get("stages")?
        .as_array()?
        .iter()
        .find(|s| s.get("key").and_then(|k| k.as_i64()) == Some(stage_key))
        .and_then(|s| s.get("level"))
        .and_then(|l| l.as_f64())
}

/// Per-hero combat numbers from the running game, keyed by heroKey.
/// Empty when the game isn't running — the save carries no resolved stats, so these are the
/// only trustworthy source for them.
fn live_combat(data_dir: &std::path::Path, meter: &crate::meter::Meter) -> HashMap<i64, Value> {
    let p = crate::engine::Params::default();
    let stage_level = current_stage_level(data_dir).unwrap_or(1.0);
    let mut out = HashMap::new();
    let list = match meter.read_party_stats() { Ok(l) => l, Err(_) => return out };
    for h in list {
        let key = match h.get("heroKey").and_then(|v| v.as_i64()) { Some(k) => k, None => continue };
        let raw = h.get("stats").and_then(|v| v.as_object()).cloned().unwrap_or_default();
        let get = |id: i64| raw.get(&id.to_string()).and_then(|v| v.as_f64()).unwrap_or(0.0);
        let auto = crate::engine::auto_dps_game(get(1), get(2), get(3), get(4), &p);
        let (max_hp, armor) = (get(5), get(6));
        let dodge = get(16) * crate::engine::game_to_display_scale(16).unwrap_or(1000.0);
        let ehp = crate::engine::ehp_from_stats(max_hp, armor, stage_level, dodge, &p);
        out.insert(key, json!({
            "autoDps": auto,
            "dps": auto,                 // == autoDps until skill DPS lands
            "ehp": ehp,
            "power": crate::engine::power(auto, ehp),
            "maxHp": max_hp,
            "armor": armor,
            "dodgePercent": dodge,
            "armorMitigation": crate::engine::armor_mitigation(armor, stage_level, &p),
            "stageLevel": stage_level,
        }));
    }
    out
}

pub fn build(data_dir: &std::path::Path, meter: &crate::meter::Meter) -> anyhow::Result<Value> {
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

    // Merge live combat numbers (game-authoritative) onto the save-derived roster.
    let combat = live_combat(data_dir, meter);
    for h in heroes.iter_mut() {
        let key = h["heroKey"].as_i64().unwrap_or(0);
        if let (Some(obj), Some(c)) = (h.as_object_mut(), combat.get(&key)) {
            if let Some(cm) = c.as_object() {
                for (k, v) in cm { obj.insert(k.clone(), v.clone()); }
            }
        }
    }

    // Party aggregates. Only heroes with live numbers contribute; EHP of a party is the
    // weakest link (whoever dies first), not the sum.
    let fielded: Vec<&Value> = heroes.iter().filter(|h| h["inParty"].as_bool() == Some(true)).collect();
    let party_dps: f64 = fielded.iter().filter_map(|h| h["dps"].as_f64()).sum();
    let party_ehp = fielded
        .iter()
        .filter_map(|h| h["ehp"].as_f64())
        .fold(f64::INFINITY, f64::min);
    let carry = fielded
        .iter()
        .filter(|h| h["dps"].is_number())
        .max_by(|a, b| {
            a["dps"].as_f64().unwrap_or(0.0).partial_cmp(&b["dps"].as_f64().unwrap_or(0.0)).unwrap()
        });
    let meta = json!({
        "party": arranged,
        "partyDPS": if party_dps > 0.0 { json!(party_dps) } else { Value::Null },
        "partyEHP": if party_ehp.is_finite() { json!(party_ehp) } else { Value::Null },
        "carryHero": carry.map(|h| h["heroKey"].clone()).unwrap_or(Value::Null),
        "carryShare": carry
            .and_then(|h| h["dps"].as_f64())
            .filter(|_| party_dps > 0.0)
            .map(|d| json!(d / party_dps))
            .unwrap_or(Value::Null),
        "combatSource": if combat.is_empty() { "unavailable (game not running)" } else { "live game memory" },
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
            "meta": meta,
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
            // dps/ehp/power now land on each hero (live game) and in `meta`.
            // farmClearTimeModel (/api/farm-rank), upgradeFinder (/api/upgrades) and gear scoring
            // (/api/gear-lines + /api/upgrades) are DONE — only skill DPS remains unmodelled.
            "engine": {
                "pending": true,
                "missing": ["skillDps"],
                "note": "skill damage is not yet modelled (no game-verified skill table); it is why                          the fitted farm DPS runs ~1.8x the auto-attack DPS. The clear-time model,                          gear scoring and upgrade finder the original gated behind its JS engine are                          now ported and game-grounded.",
            },
        }
    }))
}

/// XP needed to reach each milestone level (20/30/50/100), and an ETA at the given exp/sec.
///
/// `levels_table[L-1]` is the XP needed to go from level L to L+1 (see `data/hero-level-xp.json`
/// — wiki-sourced, not yet verified against the game; `HeroLevelUpLog` exists in
/// `LogManager.LOG_LIST` and would give a game-authoritative version of this curve if its field
/// layout were mapped out, same as `StageClearLog`).
///
/// `eps` should be a MEASURED exp/sec (from `farm::rank_stages`'s `measured` list), never a
/// modelled one — an ETA built on a ~10x-optimistic modelled rate would be exactly the kind of
/// confidently-wrong number the farm-ranking split was built to avoid.
pub fn xp_forecast(psd: &Value, levels_table: &[f64], eps: f64) -> Value {
    const TARGETS: [i64; 4] = [20, 30, 50, 100];
    let common = as_obj(&psd["commonSaveData"]);
    let arranged: Vec<i64> = common
        .get("arrangedHeroKey")
        .and_then(|v| v.as_array())
        .map(|a| a.iter().filter_map(|x| x.as_i64()).filter(|k| *k > 0).collect())
        .unwrap_or_default();

    let xp_to = |level: i64, prog: f64, target: i64| -> f64 {
        if target <= level { return 0.0; }
        let get = |l: i64| levels_table.get((l - 1).max(0) as usize).copied().unwrap_or(0.0);
        let mut xp = get(level) - prog;
        for l in (level + 1)..target { xp += get(l); }
        xp.max(0.0)
    };

    let mut heroes = Vec::new();
    for h in as_arr(&psd["heroSaveDatas"]) {
        let key = i(&h, "heroKey");
        if !arranged.contains(&key) { continue; } // benched heroes don't earn combat exp
        let level = i(&h, "HeroLevel");
        let prog = f(&h, "HeroExp");
        let targets: Vec<Value> = TARGETS
            .iter()
            .filter(|&&t| t > level)
            .map(|&t| {
                let xp = xp_to(level, prog, t);
                json!({ "level": t, "xp": xp, "etaSec": if eps > 0.0 { Some(xp / eps) } else { None } })
            })
            .collect();
        heroes.push(json!({ "heroKey": key, "level": level, "exp": prog, "targets": targets }));
    }
    json!({
        "ok": true, "heroes": heroes, "expPerSecUsed": eps,
        "levelsTableSource": "TBH wiki — not yet verified against the game",
    })
}

/// Offline/idle reward projection: how much gold/exp has accrued while away, and how long until
/// the cap. `rewards_table` and `rune_docs` are `data/offline-rewards.json` /
/// `data/offline-reward-runes.json` (both TBH wiki, not yet verified against the game — see their
/// `_comment`s). `elapsed_sec` should come from the save FILE's mtime, not the in-game
/// `lastSavedTime` field: that field is local-time .NET ticks but naive parsing reads it as UTC,
/// which skews accrual by the machine's UTC offset. mtime and "now" share one clock, so it can't.
pub fn idle_info(psd: &Value, elapsed_sec: Option<f64>, stage_level: Option<f64>, rewards_table: &Value, rune_docs: &Value) -> Value {
    const OFFLINE_CAP_SECONDS: f64 = 28800.0; // 8h, matches the reference app's PARAMS
    let runes = as_arr(&psd["RuneSaveData"]);
    let owned_level = |key: i64| -> i64 {
        runes.iter().find(|r| i(r, "RuneKey") == key).map(|r| i(r, "Level")).unwrap_or(0)
    };
    let unlock_key = rune_docs.get("unlockRuneKey").and_then(|v| v.as_i64()).unwrap_or(11001);
    let unlocked = owned_level(unlock_key) > 0;
    let sl = stage_level.unwrap_or(0.0).floor().max(0.0) as i64;
    let cap = OFFLINE_CAP_SECONDS;

    if !unlocked {
        return json!({ "unlocked": false, "stageLevel": sl, "cap": cap, "fullGold": 0, "fullExp": 0 });
    }
    let Some(row) = rewards_table.get(sl.to_string()) else {
        return json!({
            "unlocked": true, "stageLevel": sl, "cap": cap, "fullGold": 0, "fullExp": 0,
            "note": "no offline-reward row for this stage level in the wiki table",
        });
    };
    let row_gold = row.get("gold").and_then(|v| v.as_f64()).unwrap_or(0.0);
    let row_exp = row.get("exp").and_then(|v| v.as_f64()).unwrap_or(0.0);
    let row_kills = row.get("kills").and_then(|v| v.as_f64()).unwrap_or(0.0);

    // Sum the OfflineReward*Percent stat rows across every OWNED level (1..=lv) of each rune —
    // these stack per-level, they are not a single "value at current level" lookup.
    let bonus_sum = |keys: &[i64], stat: &str| -> f64 {
        let mut v = 0.0;
        for k in keys {
            let lv = owned_level(*k);
            if lv <= 0 { continue; }
            let Some(levels) = rune_docs.get("runes").and_then(|m| m.get(k.to_string())).and_then(|rd| rd.get("levels")) else { continue };
            for l in 1..=lv {
                if let Some(row) = levels.get(l.to_string()) {
                    if row.get("st").and_then(|v| v.as_str()) == Some(stat) {
                        v += row.get("v").and_then(|v| v.as_f64()).unwrap_or(0.0);
                    }
                }
            }
        }
        v
    };
    let keys_of = |field: &str| -> Vec<i64> {
        rune_docs.get(field).and_then(|v| v.as_array())
            .map(|a| a.iter().filter_map(|x| x.as_i64()).collect())
            .unwrap_or_default()
    };
    let gold_bonus = bonus_sum(&keys_of("goldRuneKeys"), "OfflineRewardGoldPercent") / 100.0;
    let exp_bonus = bonus_sum(&keys_of("expRuneKeys"), "OfflineRewardExpPercent") / 100.0;

    let full_gold = (row_gold * row_kills * (1.0 + gold_bonus)).round();
    let full_exp = (row_exp * row_kills * (1.0 + exp_bonus)).round();
    let frac = elapsed_sec.map(|e| (e.max(0.0) / cap).min(1.0));

    json!({
        "unlocked": true, "stageLevel": sl, "cap": cap, "capHours": cap / 3600.0,
        "goldBonus": gold_bonus, "expBonus": exp_bonus,
        "fullGold": full_gold, "fullExp": full_exp,
        "goldPerSec": full_gold / cap, "expPerSec": full_exp / cap,
        "accruedGold": frac.map(|f| (full_gold * f).round()),
        "accruedExp": frac.map(|f| (full_exp * f).round()),
        "frac": frac,
        "secsToCap": elapsed_sec.map(|e| (cap - e.max(0.0).min(cap)).max(0.0)),
        "rewardsTableSource": "TBH wiki — not yet verified against the game",
    })
}

/// Thin summary combining idle accrual and current farm income into a couple of headline ETAs.
/// Deliberately does not repeat `xpForecast`'s per-hero milestones — this is just gold100k + the
/// idle cap, the two numbers small enough to want at a glance without opening either sub-view.
pub fn forecast(gold_now: i64, gold_per_sec: f64, idle: &Value) -> Value {
    let idle_cap_sec = if idle.get("unlocked").and_then(|v| v.as_bool()) == Some(true) {
        idle.get("cap").cloned().unwrap_or(Value::Null)
    } else {
        Value::Null
    };
    let gold100k_sec = if gold_per_sec > 0.0 {
        Some(((100_000.0 - gold_now as f64).max(0.0)) / gold_per_sec)
    } else {
        None
    };
    json!({
        "idleCapSec": idle_cap_sec,
        "goldPerSec": if gold_per_sec > 0.0 { Some(gold_per_sec) } else { None },
        "gold100kSec": gold100k_sec,
    })
}

/// Highest hero level among the fielded party (falls back to the overall max if the arranged
/// list is empty — e.g. between stages).
pub fn max_party_level(psd: &Value) -> i64 {
    let common = as_obj(&psd["commonSaveData"]);
    let arranged: Vec<i64> = common.get("arrangedHeroKey").and_then(|v| v.as_array())
        .map(|a| a.iter().filter_map(|x| x.as_i64()).filter(|k| *k > 0).collect())
        .unwrap_or_default();
    let heroes = as_arr(&psd["heroSaveDatas"]);
    let level_of = |h: &Value| i(h, "HeroLevel");
    let fielded_max = heroes.iter()
        .filter(|h| arranged.contains(&i(h, "heroKey")))
        .map(level_of).max();
    fielded_max.or_else(|| heroes.iter().map(level_of).max()).unwrap_or(0)
}

/// Look up a stage's `{level, label}` in the bundled stage table by key. The codex stores the
/// human name under `name` and the position as `act`/`no` (not a ready-made `label`), so the
/// label is built as `act-no` with the name appended when present.
fn stage_meta(data_dir: &std::path::Path, key: i64) -> Option<(f64, String)> {
    let txt = std::fs::read_to_string(data_dir.join("engine/codex.json")).ok()?;
    let v: Value = serde_json::from_str(&txt).ok()?;
    let s = v.get("stages")?.as_array()?.iter().find(|s| s.get("key").and_then(|k| k.as_i64()) == Some(key))?;
    let lvl = s.get("level").and_then(|l| l.as_f64())?;
    let (act, no) = (i(s, "act"), i(s, "no"));
    let name = s.get("name").and_then(|n| n.as_str()).unwrap_or("");
    let label = if name.is_empty() { format!("{act}-{no}") } else { format!("{act}-{no} {name}") };
    Some((lvl, label))
}

/// Push-readiness for the next uncleared stage — how much of the party's survivability survives
/// the jump to that stage's content level.
///
/// ## What this is, and deliberately is NOT
///
/// This answers "does pushing to the frontier gut my survivability?" using ONLY game-authoritative
/// inputs: live hero stats (HP/armour/dodge from the running game) and the stage LEVEL (a reliable
/// table index). Our EHP metric already scales armour mitigation against the content level, so
/// recomputing party EHP at the target level vs the current level gives an honest *retention* ratio.
///
/// It is NOT an absolute "can you survive N monster hits" verdict. That needs monster attack, and
/// the only monster-attack numbers available (codex/wiki `atk`) carry the SAME unverified ~10x
/// inflation as the stage-HP table (the Slime reads `life` 50 in the codex against a measured ~5.6
/// per-monster HP live). Feeding that into a danger model would produce confidently-wrong verdicts,
/// so absolute danger is omitted until monster attack is read from memory and its scale verified —
/// the same bar every other numeric parameter in this project is held to.
///
/// Target selection: stage keys encode `tier*1000 + act*100 + stageNo`, so ascending key order IS
/// progression order; the push target is the smallest key greater than `maxCompletedStage`.
pub fn push_goal(data_dir: &std::path::Path, meter: &crate::meter::Meter, psd: &Value) -> Value {
    let common = as_obj(&psd["commonSaveData"]);
    let max_completed = common.get("maxCompletedStage").and_then(|v| v.as_i64()).unwrap_or(0);
    let cur_key = common.get("currentStageKey").and_then(|v| v.as_i64()).unwrap_or(0);

    // Smallest stage key strictly greater than the highest completed — the natural next push.
    let target_key = std::fs::read_to_string(data_dir.join("engine/codex.json")).ok()
        .and_then(|t| serde_json::from_str::<Value>(&t).ok())
        .and_then(|v| v.get("stages").and_then(|s| s.as_array()).map(|a| {
            a.iter().filter_map(|s| s.get("key").and_then(|k| k.as_i64()))
                .filter(|k| *k > max_completed).min()
        }))
        .flatten();

    let Some(target_key) = target_key else {
        return json!({ "ok": false, "reason": "no stage above maxCompletedStage — already at the end of the table" });
    };
    let Some((target_lvl, target_label)) = stage_meta(data_dir, target_key) else {
        return json!({ "ok": false, "reason": "target stage missing from the stage table" });
    };
    let cur_lvl = stage_meta(data_dir, cur_key).map(|(l, _)| l).unwrap_or(target_lvl);

    // Party EHP at a given content level, from the LIVE game (weakest link, matching how party EHP
    // is defined everywhere else in this file). Empty when the game isn't running.
    let p = crate::engine::Params::default();
    let list = match meter.read_party_stats() { Ok(l) => l, Err(_) => Vec::new() };
    if list.is_empty() {
        return json!({
            "ok": false, "needsGame": true,
            "target": { "stageKey": target_key, "label": target_label, "level": target_lvl },
            "reason": "push readiness needs the game running — the save carries no resolved stats",
        });
    }
    let party_ehp_at = |level: f64| -> Option<f64> {
        let mut min_ehp = f64::INFINITY;
        for h in &list {
            let raw = h.get("stats").and_then(|v| v.as_object())?;
            let get = |id: i64| raw.get(&id.to_string()).and_then(|v| v.as_f64()).unwrap_or(0.0);
            let dodge = get(16) * crate::engine::game_to_display_scale(16).unwrap_or(1000.0);
            let ehp = crate::engine::ehp_from_stats(get(5), get(6), level, dodge, &p);
            if ehp < min_ehp { min_ehp = ehp; }
        }
        min_ehp.is_finite().then_some(min_ehp)
    };

    let ehp_target = party_ehp_at(target_lvl);
    let ehp_current = party_ehp_at(cur_lvl.max(1.0));
    let retention = match (ehp_target, ehp_current) {
        (Some(t), Some(c)) if c > 0.0 => Some(t / c),
        _ => None,
    };
    let party_level = crate::insights::max_party_level(psd);
    let rating = retention.map(|r| if r >= 0.7 { "comfortable" } else if r >= 0.4 { "tight" } else { "risky" });

    json!({
        "ok": true,
        "target": { "stageKey": target_key, "label": target_label, "level": target_lvl },
        "current": { "stageKey": cur_key, "level": cur_lvl },
        "partyMaxLevel": party_level,
        "levelGap": (target_lvl - party_level as f64).max(0.0),
        "partyEhpAtTarget": ehp_target,
        "partyEhpAtCurrent": ehp_current,
        "survivabilityRetention": retention,
        "rating": rating,
        "metric": "survivability-retention",
        "note": "Relative EHP retention from armour-mitigation scaling only, from live hero stats. \
                 NOT an absolute survival verdict — monster attack is not modelled (the only \
                 available monster-attack data is wiki/codex, which carries the same unverified \
                 ~10x inflation as the stage-HP table).",
    })
}

/// The game's 10 item grades in ascending order. Synthesis promotes to the NEXT one up.
const GRADE_ORDER: [&str; 10] = [
    "COMMON", "UNCOMMON", "RARE", "LEGENDARY", "IMMORTAL",
    "ARCANA", "BEYOND", "CELESTIAL", "DIVINE", "COSMIC",
];

fn grade_rank(g: &str) -> Option<usize> {
    GRADE_ORDER.iter().position(|x| x.eq_ignore_ascii_case(g))
}

/// Synthesis planner: the game fuses 9 same-grade items into one of the next grade up
/// (`crafting.json` cubeInfo: "Synthesize 9 items of the same grade into one of a higher grade").
///
/// This is fully save-derived — item grade comes from the game's own item table, and the 9→1 rule
/// is the game's stated mechanic, so no wiki numeric parameter is guessed at. Equipped items are
/// excluded (you would not fuse away what you are wearing). It also projects a full CASCADE: the
/// items you'd get from fusing a grade can themselves be fused, so "27 COMMON" surfaces as 3
/// UNCOMMON now and notes the cascade potential without pretending the higher-grade drops are free.
pub fn synthesis_plan(psd: &Value) -> Value {
    let table = crate::save::item_table_snapshot();

    // UniqueIds currently equipped by any hero — never counted as fusible.
    let mut equipped: std::collections::HashSet<String> = std::collections::HashSet::new();
    for h in as_arr(&psd["heroSaveDatas"]) {
        for id in h.get("equippedItemIds").and_then(|v| v.as_array()).map(|a| a.as_slice()).unwrap_or(&[]) {
            match id {
                Value::Number(n) => { equipped.insert(n.to_string()); }
                Value::String(s) => { equipped.insert(s.clone()); }
                _ => {}
            }
        }
    }

    // Count unequipped items per grade (only items the table knows a grade for).
    let mut by_grade: HashMap<usize, i64> = HashMap::new();
    let mut ungraded = 0i64;
    for it in as_arr(&psd["itemSaveDatas"]) {
        let uid = match &it["UniqueId"] {
            Value::Number(n) => n.to_string(),
            Value::String(s) => s.clone(),
            _ => continue,
        };
        if uid == "0" || equipped.contains(&uid) { continue; }
        let item_key = it.get("ItemKey").and_then(|v| v.as_i64()).unwrap_or(0);
        let grade = table.get(&item_key.to_string())
            .and_then(|r| r.get("GRADE")).and_then(|v| v.as_str());
        match grade.and_then(grade_rank) {
            Some(r) => *by_grade.entry(r).or_insert(0) += 1,
            None => ungraded += 1,
        }
    }

    // Immediate fusions per grade (floor(count / 9)); the top grade can't promote.
    let mut rows = Vec::new();
    for r in 0..GRADE_ORDER.len() {
        let count = by_grade.get(&r).copied().unwrap_or(0);
        if count == 0 { continue; }
        let fuses = if r + 1 < GRADE_ORDER.len() { count / 9 } else { 0 };
        rows.push(json!({
            "grade": GRADE_ORDER[r],
            "have": count,
            "fusable": r + 1 < GRADE_ORDER.len(),
            "fusesNow": fuses,
            "producesGrade": (r + 1 < GRADE_ORDER.len()).then(|| GRADE_ORDER[r + 1]),
            "leftover": count % 9,
        }));
    }

    // Cascade: simulate repeatedly fusing 9→1 up the chain, so the player sees the eventual top
    // grade reachable purely from what they already own, without conflating it with drops.
    let mut cascade = by_grade.clone();
    let mut cascade_moves = 0i64;
    loop {
        let mut promoted = false;
        for r in 0..GRADE_ORDER.len() - 1 {
            let c = cascade.get(&r).copied().unwrap_or(0);
            if c >= 9 {
                let f = c / 9;
                *cascade.entry(r).or_insert(0) -= f * 9;
                *cascade.entry(r + 1).or_insert(0) += f;
                cascade_moves += f;
                promoted = true;
            }
        }
        if !promoted { break; }
    }
    let cascade_top = cascade.iter().filter(|(_, &c)| c > 0).map(|(&r, _)| r).max();

    json!({
        "ok": true,
        "rule": "9 same-grade items fuse into 1 of the next grade",
        "rows": rows,
        "ungradedItems": ungraded,
        "cascade": {
            "totalFuses": cascade_moves,
            "topGradeReachable": cascade_top.map(|r| GRADE_ORDER[r]),
            "note": "If you fused everything upward as far as it goes (each 9→1, repeatedly). \
                     Higher-grade results are NOT free drops — this only counts what you already own.",
        },
    })
}

/// Alchemy value of unequipped items: total sell gold + total cube EXP if you converted every
/// loose item. `alchemy_table` is `data/item-alchemy.json` (wiki, not game-verified — see that
/// file). Equipped items are excluded. Also breaks the sell gold down by grade so a player can
/// see whether it's worth alchemising low grades vs saving them to synthesise.
pub fn alchemy_value(psd: &Value, alchemy_table: &Value) -> Value {
    let table = crate::save::item_table_snapshot();
    let sell = alchemy_table.get("sell");
    let cube = alchemy_table.get("cubeExp");

    let mut equipped: std::collections::HashSet<String> = std::collections::HashSet::new();
    for h in as_arr(&psd["heroSaveDatas"]) {
        for id in h.get("equippedItemIds").and_then(|v| v.as_array()).map(|a| a.as_slice()).unwrap_or(&[]) {
            match id {
                Value::Number(n) => { equipped.insert(n.to_string()); }
                Value::String(s) => { equipped.insert(s.clone()); }
                _ => {}
            }
        }
    }

    let (mut sell_gold, mut cube_exp, mut count, mut priced) = (0i64, 0i64, 0i64, 0i64);
    let mut by_grade: HashMap<String, i64> = HashMap::new();
    for it in as_arr(&psd["itemSaveDatas"]) {
        let uid = match &it["UniqueId"] {
            Value::Number(n) => n.to_string(),
            Value::String(s) => s.clone(),
            _ => continue,
        };
        if uid == "0" || equipped.contains(&uid) { continue; }
        let key = it.get("ItemKey").and_then(|v| v.as_i64()).unwrap_or(0).to_string();
        count += 1;
        if let Some(g) = sell.and_then(|s| s.get(&key)).and_then(|v| v.as_i64()) {
            sell_gold += g;
            priced += 1;
            let grade = table.get(&key).and_then(|r| r.get("GRADE")).and_then(|v| v.as_str()).unwrap_or("?");
            *by_grade.entry(grade.to_string()).or_insert(0) += g;
        }
        if let Some(e) = cube.and_then(|c| c.get(&key)).and_then(|v| v.as_i64()) {
            cube_exp += e;
        }
    }

    json!({
        "ok": true,
        "looseItems": count,
        "pricedItems": priced,
        "unpricedItems": count - priced,
        "sellGold": sell_gold,
        "cubeExp": cube_exp,
        "sellGoldByGrade": by_grade,
        "source": "wiki — not game-verified",
    })
}

/// Attribute the game's aggregated stats to their SOURCE (base / gear / attributes / passives /
/// account), straight from the live modifier manager — fully game-authoritative, no wiki data.
///
/// The game tags every `StatModifier` with a `MOD_SOURCE`; `read_party_modifiers` preserves it.
/// Stats aggregate multiplicatively, so a source's contribution is its MARGINAL effect: the stat
/// with all sources, minus the stat recomputed with that one source's mods removed. Marginals
/// don't sum to the total under multiplication (that's inherent, not a bug), so the raw per-source
/// FLAT/ADDITIVE/MULT buckets are returned alongside for full transparency.
///
/// `SOURCE_NAMES` is verified live below (see the endpoint's cross-checks): ITEM must equal the
/// gear-line reconciliation, and ATTRIBUTE must be non-empty exactly when the hero has allocated
/// ability points.
pub fn stat_sources(modifiers: &[Value]) -> Value {
    // Verified live: 0/1/3/4 are base/item/passive/account (source 1 = ITEM matches the gear-line
    // reconciliation exactly). Source 2 has never been observed for any hero, so it is NOT labelled
    // "attribute" on a guess — attribute allocations grant PASSIVESKILL passives and therefore show
    // up under "passive" (3), which is why there is no separate attribute line.
    const SOURCE_NAMES: [&str; 5] = ["base", "item", "source2", "passive", "account"];
    let src_name = |s: i64| -> String {
        SOURCE_NAMES.get(s as usize).map(|x| x.to_string()).unwrap_or_else(|| format!("source{s}"))
    };
    // Aggregate a set of (mode, value) mods into a game-native stat value.
    let agg = |mods: &[(i64, f64)]| -> f64 {
        let (mut flat, mut add, mut mul) = (0.0, 0.0, 0.0);
        for (mode, v) in mods {
            match mode { 0 => flat += v, 1 => add += v, 2 => mul += v, _ => {} }
        }
        flat * (1.0 + add) * (1.0 + mul)
    };

    let mut heroes = Vec::new();
    for h in modifiers {
        let hero_key = h.get("heroKey").and_then(|v| v.as_i64()).unwrap_or(0);
        let stats_obj = match h.get("stats").and_then(|v| v.as_object()) { Some(o) => o, None => continue };
        let mut stats = Map::new();
        for (stat, entry) in stats_obj {
            let mods: Vec<(i64, f64, i64)> = entry.get("mods").and_then(|v| v.as_array())
                .map(|a| a.iter().filter_map(|m| Some((
                    m.get("mode")?.as_i64()?, m.get("value")?.as_f64()?, m.get("source")?.as_i64()?,
                ))).collect())
                .unwrap_or_default();
            if mods.is_empty() { continue; }

            let all: Vec<(i64, f64)> = mods.iter().map(|(m, v, _)| (*m, *v)).collect();
            let total = agg(&all);

            // Per-source raw buckets + marginal contribution.
            let mut sources_present: Vec<i64> = mods.iter().map(|(_, _, s)| *s).collect();
            sources_present.sort_unstable();
            sources_present.dedup();

            let mut per_source = Map::new();
            for s in sources_present {
                let without: Vec<(i64, f64)> = mods.iter().filter(|(_, _, ms)| *ms != s).map(|(m, v, _)| (*m, *v)).collect();
                let marginal = total - agg(&without);
                let (mut flat, mut add, mut mul) = (0.0, 0.0, 0.0);
                for (m, v, ms) in &mods {
                    if *ms != s { continue; }
                    match m { 0 => flat += v, 1 => add += v, 2 => mul += v, _ => {} }
                }
                per_source.insert(src_name(s), json!({
                    "flat": flat, "additive": add, "multiplicative": mul, "marginal": marginal,
                }));
            }
            stats.insert(stat.clone(), json!({ "total": total, "bySource": per_source }));
        }
        heroes.push(json!({ "heroKey": hero_key, "stats": stats }));
    }
    json!({ "ok": true, "heroes": heroes, "source": "live game modifier manager (authoritative)" })
}

/// Pet advisor: which pets you own, which is active, and the best owned pet for each of gold /
/// exp / drop-rate. Unlocked/arranged state is game-authoritative (save); the pet STAT values are
/// wiki (`data/pets.json`, not game-verified). The next locked pet's unlock requirement is
/// resolved to a monster name via the codex when it's a KillMonster unlock.
pub fn pet_advisor(psd: &Value, pets_table: &Value, codex: &Value) -> Value {
    let defs = pets_table.get("pets").and_then(|v| v.as_object());
    let stats = pets_table.get("petStats");
    let Some(defs) = defs else { return json!({ "ok": false, "error": "pets.json missing pets" }); };

    let save_pets = as_arr(&psd["PetSaveData"]);
    let unlocked: std::collections::HashSet<i64> = save_pets.iter()
        .filter(|p| b(p, "IsUnlock")).map(|p| i(p, "PetKey")).collect();
    let arranged = i(&as_obj(&psd["commonSaveData"]), "ArrangedPetKey");

    // Best owned pet per stat.
    let stat_val = |pet_key: i64, stat: &str| -> f64 {
        let sk = defs.get(&pet_key.to_string()).and_then(|d| d.get("statKey")).and_then(|v| v.as_i64());
        let Some(sk) = sk else { return 0.0 };
        stats.and_then(|s| s.get(sk.to_string())).and_then(|v| v.as_array())
            .map(|rows| rows.iter().filter(|r| r.get("st").and_then(|x| x.as_str()) == Some(stat))
                .filter_map(|r| r.get("v").and_then(|x| x.as_f64())).sum())
            .unwrap_or(0.0)
    };
    let best_for = |stat: &str| -> Value {
        unlocked.iter().map(|&k| (k, stat_val(k, stat)))
            .filter(|(_, v)| *v > 0.0)
            .max_by(|a, b| a.1.partial_cmp(&b.1).unwrap())
            .map(|(k, v)| json!({ "petKey": k, "value": v,
                "name": defs.get(&k.to_string()).and_then(|d| d.get("name")).cloned().unwrap_or(Value::Null) }))
            .unwrap_or(Value::Null)
    };

    // Next locked pet + its unlock requirement (Bat first if not owned, then any locked).
    let monster_name = |mk: i64| -> Option<String> {
        codex.get("monsters").and_then(|m| m.as_array()).and_then(|arr| {
            arr.iter().find(|x| x.get("key").and_then(|k| k.as_i64()) == Some(mk))
                .and_then(|x| x.get("name").and_then(|n| n.as_str()).map(|s| s.to_string()))
        })
    };
    let locked: Vec<i64> = defs.keys().filter_map(|k| k.parse::<i64>().ok())
        .filter(|k| !unlocked.contains(k)).collect();
    let next = if !unlocked.contains(&1001) { Some(1001) } else { locked.iter().min().copied() };
    let next_info = next.and_then(|k| defs.get(&k.to_string()).map(|d| {
        let unlock = d.get("unlock").and_then(|v| v.as_str()).unwrap_or("");
        let param1 = d.get("param1").and_then(|v| v.as_i64());
        json!({
            "petKey": k, "name": d.get("name").cloned().unwrap_or(Value::Null),
            "unlock": unlock, "param1": param1,
            "requirement": if unlock == "KillMonster" {
                param1.and_then(monster_name).map(|n| format!("Kill {n}")).unwrap_or_else(|| "Kill a specific monster".into())
            } else if unlock == "DLC" {
                "Steam DLC".into()
            } else { unlock.to_string() },
        })
    }));

    json!({
        "ok": true,
        "total": defs.len(), "unlocked": unlocked.len(), "arranged": arranged,
        "bestGold": best_for("IncreaseGoldAmount"),
        "bestExp": best_for("IncreaseExpAmount"),
        "bestDrop": best_for("DropChanceNormalChestPercent"),
        "nextUnlock": next_info,
        "statSource": "wiki — not game-verified",
    })
}

/// Rune status advisor: owned level, next-level cost, affordability, and unlock state per rune.
///
/// Grounding: owned levels + gold are game-authoritative (save); costs, effects, max levels, and
/// the unlock tree are wiki (`data/runes.json`). Deliberately does NOT compute a power-delta ROI:
/// a rune's stat VALUE units are unverified against the game, and turning an unverified value into
/// a confident "+X power per gold" is exactly the kind of laundering this project avoids. It shows
/// what each rune grants (stat + raw value) and what you can afford, and leaves the power judgement
/// to the player until the rune value scale is anchored to a live modifier read.
pub fn rune_status(psd: &Value, runes_table: &Value, gold: i64) -> Value {
    let defs = match runes_table.get("runes").and_then(|v| v.as_object()) {
        Some(d) => d, None => return json!({ "ok": false, "error": "runes.json missing runes" }),
    };
    let levels = runes_table.get("runeLevels");
    let edges = runes_table.get("tree").and_then(|t| t.get("edges")).and_then(|v| v.as_array());
    let starts: std::collections::HashSet<i64> = runes_table.get("tree")
        .and_then(|t| t.get("starts")).and_then(|v| v.as_array())
        .map(|a| a.iter().filter_map(|x| x.as_i64()).collect()).unwrap_or_default();

    // parent map from tree edges: parent[to] = from
    let mut parent: HashMap<i64, i64> = HashMap::new();
    for e in edges.map(|a| a.as_slice()).unwrap_or(&[]) {
        if let Some(a) = e.as_array() {
            if let (Some(from), Some(to)) = (a.first().and_then(|v| v.as_i64()), a.get(1).and_then(|v| v.as_i64())) {
                parent.insert(to, from);
            }
        }
    }

    let owned: HashMap<i64, i64> = as_arr(&psd["RuneSaveData"]).iter()
        .map(|r| (i(r, "RuneKey"), i(r, "Level"))).collect();
    let owned_level = |k: i64| owned.get(&k).copied().unwrap_or(0);
    let req_of = |k: i64| defs.get(&k.to_string()).and_then(|d| d.get("prevReq")).and_then(|v| v.as_i64()).unwrap_or(1);
    let unlocked = |k: i64| -> bool {
        if starts.contains(&k) || !parent.contains_key(&k) { return true; }
        owned_level(parent[&k]) >= req_of(k)
    };

    let is_combat = |st: &str| st.starts_with("AllHero");
    let mut affordable_next = Vec::new();
    let (mut total_levels, mut maxed, mut owned_count) = (0i64, 0i64, 0i64);

    for (key_s, d) in defs {
        let key: i64 = match key_s.parse() { Ok(k) => k, Err(_) => continue };
        let lv = owned_level(key);
        let max = d.get("max").and_then(|v| v.as_i64()).unwrap_or(0);
        if lv > 0 { owned_count += 1; total_levels += lv; }
        if max > 0 && lv >= max { maxed += 1; }
        if lv >= max || max == 0 { continue; }
        if lv == 0 && !unlocked(key) { continue; } // can't start a locked rune
        let ldk = d.get("ldk").and_then(|v| v.as_i64()).unwrap_or(key);
        let next_row = levels.and_then(|l| l.get(ldk.to_string())).and_then(|t| t.get((lv + 1).to_string()));
        let Some(row) = next_row else { continue };
        let cost = row.get("cost").and_then(|v| v.as_i64()).unwrap_or(0);
        let cost_item = row.get("costItem").and_then(|v| v.as_i64()).unwrap_or(0);
        let st = row.get("st").and_then(|v| v.as_str()).unwrap_or("");
        // Only gold-cost upgrades (costItem 100001) get an affordability verdict; others need mats.
        let gold_cost = cost_item == 100001;
        affordable_next.push(json!({
            "runeKey": key,
            "name": d.get("name").cloned().unwrap_or(Value::Null),
            "level": lv, "nextLevel": lv + 1, "max": max,
            "cost": cost, "costItem": cost_item, "goldCost": gold_cost,
            "affordable": gold_cost && cost <= gold,
            "stat": st, "value": row.get("v").cloned().unwrap_or(Value::Null),
            "combat": is_combat(st),
            "isNew": lv == 0,
        }));
    }

    // Cheapest affordable gold upgrades first — the actionable list.
    affordable_next.sort_by(|a, b| {
        let (aa, ba) = (a["affordable"].as_bool().unwrap_or(false), b["affordable"].as_bool().unwrap_or(false));
        ba.cmp(&aa).then(a["cost"].as_i64().unwrap_or(i64::MAX).cmp(&b["cost"].as_i64().unwrap_or(i64::MAX)))
    });
    let affordable_count = affordable_next.iter().filter(|r| r["affordable"].as_bool() == Some(true)).count();

    json!({
        "ok": true,
        "gold": gold,
        "ownedRunes": owned_count, "totalLevels": total_levels, "maxed": maxed, "total": defs.len(),
        "affordableCount": affordable_count,
        "upgrades": affordable_next,
        "note": "Costs/effects are wiki data. No power-delta ROI is shown: rune value units are not \
                 yet verified against the game, so ranking by 'power per gold' would be a guess. \
                 Cheapest affordable upgrades are listed first; combat runes are flagged.",
    })
}

/// Loot finder: which gear a stage of the given level can drop, and at what chance.
///
/// A stage drops the "box" of its 5-level band. `drops_table` is `data/drops.json`
/// (`boxDrops[band]` = weighted `[groupKey, weight, subType]` rows; `dropGroups[group]` = item
/// keys). An item's chance in a band = Σ over groups containing it of (weight / groupSize),
/// divided by the band's total weight.
///
/// Drop chances are inherently wiki data — they are RNG tables the game never exposes, so unlike
/// stat/HP values there is nothing to read from memory to verify them. Flagged accordingly. Item
/// grade/type/name come from the game item table (authoritative for those).
pub fn loot_finder(stage_level: f64, drops_table: &Value, top_n: usize) -> Value {
    let box_drops = drops_table.get("boxDrops").and_then(|v| v.as_object());
    let groups = drops_table.get("dropGroups").and_then(|v| v.as_object());
    let (Some(box_drops), Some(groups)) = (box_drops, groups) else {
        return json!({ "ok": false, "error": "drops.json malformed" });
    };

    // Band = largest band key <= stage level.
    let lvl = stage_level.floor().max(1.0) as i64;
    let band = box_drops.keys().filter_map(|k| k.parse::<i64>().ok())
        .filter(|b| *b <= lvl).max();
    let Some(band) = band else { return json!({ "ok": false, "error": "no drop band for this level" }); };

    let rows = box_drops.get(&band.to_string()).and_then(|v| v.as_array());
    let Some(rows) = rows else { return json!({ "ok": false, "error": "band has no rows" }); };

    let total_weight: f64 = rows.iter().filter_map(|r| r.as_array()?.get(1)?.as_f64()).sum();
    if total_weight <= 0.0 { return json!({ "ok": false, "error": "band total weight is zero" }); }

    // Accumulate each item's weighted share.
    let mut chance: HashMap<i64, f64> = HashMap::new();
    for r in rows {
        let Some(a) = r.as_array() else { continue };
        let (Some(g), Some(w)) = (a.first().and_then(|v| v.as_i64()), a.get(1).and_then(|v| v.as_f64())) else { continue };
        let Some(items) = groups.get(&g.to_string()).and_then(|v| v.as_array()) else { continue };
        let n = items.len();
        if n == 0 { continue; }
        for it in items {
            if let Some(k) = it.as_i64() {
                *chance.entry(k).or_insert(0.0) += (w / n as f64) / total_weight;
            }
        }
    }

    let table = crate::save::item_table_snapshot();
    let mut items: Vec<Value> = chance.into_iter().filter_map(|(k, c)| {
        let row = table.get(&k.to_string())?;
        // Only gear (accessories/materials drop too but the finder is for gear upgrades).
        if row.get("ITEMTYPE").and_then(|v| v.as_str()) != Some("GEAR") { return None; }
        Some(json!({
            "itemKey": k, "chance": c,
            "grade": row.get("GRADE").cloned().unwrap_or(Value::Null),
            "gearType": row.get("GEARTYPE").cloned().unwrap_or(Value::Null),
            "level": row.get("Level").cloned().unwrap_or(Value::Null),
        }))
    }).collect();
    items.sort_by(|a, b| b["chance"].as_f64().unwrap_or(0.0).total_cmp(&a["chance"].as_f64().unwrap_or(0.0)));
    let shown = items.len().min(top_n);
    items.truncate(top_n);

    json!({
        "ok": true, "stageLevel": lvl, "band": band,
        "gearDropCount": shown,
        "items": items,
        "source": "wiki drop tables — chances not game-verifiable (RNG tables the game never exposes)",
    })
}
