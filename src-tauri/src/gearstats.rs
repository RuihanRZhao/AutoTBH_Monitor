//! Equipped-gear → stat contributions, computed from the save (offline, no game process needed).
//!
//! Pipeline: `heroSaveDatas[].equippedItemIds` → `itemSaveDatas` (by UniqueId) → `ItemKey`
//! → wiki `items_detail.json` → the item's stat lines, plus any enchant affixes held in the save.
//!
//! Where the per-item stat lines come from, and why:
//!   * NOT the bundled item table — it only carries ItemKey/GRADE/GEARTYPE/Level/icon/marketable.
//!   * NOT the game's assets — the CSV tables an older build exposed are gone in current builds
//!     (searched every .assets file; even the previously-used `ItemKey,ITEMTYPE,` header is absent).
//!   * The wiki's `items_detail.json`, keyed by ItemKey, which does carry them.
//!
//! `BaseStat1_Value` / `BaseStat2_Value` are bare numbers — their StatType/ModType is fixed per
//! gear type and stored nowhere, so it comes from `data/gear-base-stats.json`. Unmapped gear
//! types are reported in `unmappedGearTypes` rather than being assigned a guessed stat type.

use crate::engine::{ModType, StatContrib};
use serde_json::{json, Map, Value};
use std::collections::{HashMap, HashSet};
use std::path::Path;

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

fn mod_type(s: &str) -> ModType {
    match s {
        "ADDITIVE" => ModType::Additive,
        "MULTIPLICATIVE" => ModType::Multiplicative,
        _ => ModType::Flat,
    }
}

/// One resolved stat line: `[StatType, ModType, Value]`.
#[derive(Clone, Debug)]
pub struct Line {
    pub stat: String,
    pub mode: String,
    pub value: f64,
}

impl Line {
    fn to_json(&self) -> Value {
        json!([self.stat, self.mode, self.value])
    }
}

/// GEARTYPE → base-stat typing, loaded from `data/gear-base-stats.json`.
pub struct BaseStatMap {
    map: HashMap<String, (Option<(String, String)>, Option<(String, String)>)>,
}

impl BaseStatMap {
    pub fn load(data_dir: &Path) -> Self {
        let mut map = HashMap::new();
        if let Ok(txt) = std::fs::read_to_string(data_dir.join("gear-base-stats.json")) {
            if let Ok(v) = serde_json::from_str::<Value>(&txt) {
                if let Some(obj) = v.get("gearTypes").and_then(|g| g.as_object()) {
                    for (gear, spec) in obj {
                        let pick = |k: &str| -> Option<(String, String)> {
                            let a = spec.get(k)?.as_array()?;
                            Some((a.first()?.as_str()?.to_string(), a.get(1)?.as_str()?.to_string()))
                        };
                        map.insert(gear.to_uppercase(), (pick("base1"), pick("base2")));
                    }
                }
            }
        }
        Self { map }
    }
    fn get(&self, gear: &str) -> Option<&(Option<(String, String)>, Option<(String, String)>)> {
        self.map.get(&gear.to_uppercase())
    }
    pub fn is_empty(&self) -> bool { self.map.is_empty() }
}

/// Resolve one item's full stat lines from `items_detail`.
/// Returns `None` when the item isn't in the catalogue at all.
pub fn item_lines(
    detail: &Value,
    item_key: i64,
    gear_type: &str,
    bases: &BaseStatMap,
    unmapped: &mut HashSet<String>,
) -> Option<Vec<Line>> {
    let st = detail.get(item_key.to_string())?.get("stats")?;
    let mut out = Vec::new();

    let num = |k: &str| -> f64 { st.get(k).and_then(|v| v.as_f64()).unwrap_or(0.0) };

    // Base stats: value from items_detail, type from the gear-type map.
    let b1 = num("BaseStat1_Value");
    let b2 = num("BaseStat2_Value");
    match bases.get(gear_type) {
        Some((s1, s2)) => {
            if b1 != 0.0 {
                match s1 {
                    Some((stat, mode)) => out.push(Line { stat: stat.clone(), mode: mode.clone(), value: b1 }),
                    // Has a base value but the map says this type has none — surface it.
                    None => { unmapped.insert(gear_type.to_uppercase()); }
                }
            }
            if b2 != 0.0 {
                match s2 {
                    Some((stat, mode)) => out.push(Line { stat: stat.clone(), mode: mode.clone(), value: b2 }),
                    None => { unmapped.insert(gear_type.to_uppercase()); }
                }
            }
        }
        None => {
            if b1 != 0.0 || b2 != 0.0 {
                unmapped.insert(gear_type.to_uppercase());
            }
        }
    }

    // Inherent stats carry their own type and mode.
    for i in 1..=3 {
        let stat = st.get(format!("InherentStat{i}_STATTYPE")).and_then(|v| v.as_str()).unwrap_or("NONE");
        let value = num(&format!("InherentStat{i}_Value"));
        if stat == "NONE" || stat.is_empty() || value == 0.0 { continue; }
        let mode = st
            .get(format!("InherentStat{i}_MODTYPE"))
            .and_then(|v| v.as_str())
            .unwrap_or("FLAT")
            .to_string();
        out.push(Line { stat: stat.to_string(), mode, value });
    }

    Some(out)
}

/// Enchant affixes rolled onto a specific item instance, held in the save.
fn enchant_lines(item: &Value) -> Vec<Line> {
    let mut out = Vec::new();
    for e in as_arr(&item["EnchantData"]) {
        let stat = e.get("StatType").and_then(|v| v.as_i64()).unwrap_or(0);
        let value = e.get("Value").and_then(|v| v.as_f64()).unwrap_or(0.0);
        if stat == 0 || value == 0.0 { continue; }
        let mode = match e.get("ModType").and_then(|v| v.as_i64()).unwrap_or(0) {
            1 => "ADDITIVE",
            2 => "MULTIPLICATIVE",
            _ => "FLAT",
        };
        out.push(Line {
            stat: crate::engine::stat_name(stat).to_string(),
            mode: mode.to_string(),
            value,
        });
    }
    out
}

pub struct HeroGear {
    pub hero_key: i64,
    pub slots: Vec<Value>,
    pub contrib: HashMap<String, StatContrib>,
}

/// Build per-hero equipped-gear stat lines and aggregated contributions from the save.
pub async fn build(data_dir: &Path) -> anyhow::Result<Value> {
    let raw = crate::save::player_save_data_string()?;
    let psd: Value = serde_json::from_str(&raw)?;
    let detail = crate::wiki::ensure_items_detail(data_dir)
        .await
        .ok_or_else(|| anyhow::anyhow!("items_detail.json unavailable"))?;
    let bases = BaseStatMap::load(data_dir);
    if bases.is_empty() {
        anyhow::bail!("data/gear-base-stats.json missing or empty");
    }
    let table = crate::save::item_table_snapshot();

    // UniqueId → itemSaveData
    let mut by_uid: HashMap<String, Value> = HashMap::new();
    for it in as_arr(&psd["itemSaveDatas"]) {
        let uid = match &it["UniqueId"] {
            Value::Number(n) => n.to_string(),
            Value::String(s) => s.clone(),
            _ => continue,
        };
        by_uid.insert(uid, it);
    }

    let mut unmapped: HashSet<String> = HashSet::new();
    let mut missing_detail = 0usize;
    let mut heroes = Vec::new();

    for h in as_arr(&psd["heroSaveDatas"]) {
        let hero_key = h.get("heroKey").and_then(|v| v.as_i64()).unwrap_or(0);
        let mut slots = Vec::new();
        let mut contrib: HashMap<String, StatContrib> = HashMap::new();

        for (slot, id) in h
            .get("equippedItemIds")
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default()
            .iter()
            .enumerate()
        {
            let uid = match id {
                Value::Number(n) => n.to_string(),
                Value::String(s) => s.clone(),
                _ => continue,
            };
            if uid == "0" { continue; }
            let item = match by_uid.get(&uid) { Some(i) => i, None => continue };
            let item_key = item.get("ItemKey").and_then(|v| v.as_i64()).unwrap_or(0);
            let row = table.get(&item_key.to_string());
            let gear_type = row
                .and_then(|r| r.get("GEARTYPE"))
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();

            // `lines` = base + inherent (the item's intrinsic stats, identical for every copy).
            // Enchants are rolled per instance and are kept separate: the reference's
            // `gear.slots[].current.lines` covers only the intrinsic ones, so mixing the two in
            // would make an apples-to-apples diff impossible. Both feed the contribution totals.
            let lines = match item_lines(&detail, item_key, &gear_type, &bases, &mut unmapped) {
                Some(l) => l,
                None => { missing_detail += 1; continue; }
            };
            let enchants = enchant_lines(item);

            for l in lines.iter().chain(enchants.iter()) {
                contrib.entry(l.stat.clone()).or_default().push(mod_type(&l.mode), l.value);
            }
            slots.push(json!({
                "slot": slot,
                "itemKey": item_key,
                "uniqueId": uid,
                "gearType": gear_type,
                "grade": row.and_then(|r| r.get("GRADE")).cloned().unwrap_or(Value::Null),
                "level": row.and_then(|r| r.get("Level")).cloned().unwrap_or(Value::Null),
                "lines": lines.iter().map(|l| l.to_json()).collect::<Vec<_>>(),
                "enchantLines": enchants.iter().map(|l| l.to_json()).collect::<Vec<_>>(),
            }));
        }

        // Aggregate each stat with the verified formula.
        let mut totals = Map::new();
        for (stat, c) in &contrib {
            totals.insert(stat.clone(), json!(crate::engine::aggregate_stat(c)));
        }
        heroes.push(json!({
            "heroKey": hero_key,
            "slots": slots,
            "gearStats": totals,
        }));
    }

    Ok(json!({
        "ok": true,
        "source": "save+items_detail",
        "heroes": heroes,
        "unmappedGearTypes": unmapped.into_iter().collect::<Vec<_>>(),
        "itemsMissingDetail": missing_detail,
    }))
}
