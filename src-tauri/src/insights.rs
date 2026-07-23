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
            "engine": {
                "pending": true,
                "missing": ["skillDps", "gearScoring", "farmClearTimeModel", "upgradeFinder"],
                "note": "hero dps/ehp/power require the game running; the save carries no resolved stats",
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
