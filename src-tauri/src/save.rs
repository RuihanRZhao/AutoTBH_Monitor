//! Read-only TBH: Task Bar Hero save reader. Port of `tbh-save.mjs`.
//! Decrypts an in-memory copy of SaveFile_Live.es3 (Easy Save 3: AES-128-CBC + PBKDF2-HMAC-SHA1,
//! optional gzip), parses PlayerSaveData, and cross-references the game item table. Never writes.

use aes::Aes128;
use cbc::cipher::{block_padding::Pkcs7, BlockDecryptMut, KeyIvInit};
use flate2::read::GzDecoder;
use once_cell::sync::OnceCell;
use regex::Regex;
use serde_json::{json, Map, Value};
use std::collections::{HashMap, HashSet};
use std::io::Read;
use std::path::{Path, PathBuf};

type Aes128CbcDec = cbc::Decryptor<Aes128>;

pub const TBH_APPID: i64 = 3678970;
const ES3_FALLBACK_PW: &str = "emuMqG3bLYJ938ZDCfieWJ";

fn grade_map(g: &str) -> String {
    let mapped = match g.to_uppercase().as_str() {
        "COSMIC" => "Cosmic", "DIVINE" => "Divine", "CELESTIAL" => "Celestial", "ARCANA" => "Arcana",
        "IMMORTAL" => "Immortal", "LEGENDARY" => "Legendary", "BEYOND" => "Beyond", "EPIC" => "Epic",
        "RARE" => "Rare", "UNCOMMON" => "Uncommon", "COMMON" => "Common",
        _ => "",
    };
    if mapped.is_empty() { title_token(g) } else { mapped.to_string() }
}

fn title_token(s: &str) -> String {
    let t = s.trim();
    if t.is_empty() { return String::new(); }
    let mut c = t.chars();
    let first = c.next().unwrap().to_uppercase().to_string();
    first + &c.as_str().to_lowercase()
}

fn bool_true(v: &Value) -> bool {
    match v {
        Value::Bool(b) => *b,
        Value::String(s) => s.to_lowercase() == "true",
        _ => false,
    }
}
fn vstr(v: &Value, k: &str) -> String {
    v.get(k).map(val_to_string).unwrap_or_default()
}
fn val_to_string(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Number(n) => n.to_string(),
        Value::Bool(b) => b.to_string(),
        _ => String::new(),
    }
}

/// Save file location (%LOCALAPPDATA%Low/TesseractStudio/TaskbarHero/SaveFile_Live.es3).
fn save_file() -> PathBuf {
    let home = dirs::home_dir().unwrap_or_default();
    home.join("AppData/LocalLow/TesseractStudio/TaskbarHero/SaveFile_Live.es3")
}

pub fn save_exists() -> bool { save_file().exists() }
pub fn save_mtime() -> f64 {
    std::fs::metadata(save_file())
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_millis() as f64)
        .unwrap_or(0.0)
}

fn find_game_data_dir() -> Option<PathBuf> {
    if let Ok(d) = std::env::var("TBH_GAME_DIR") {
        let p = PathBuf::from(&d);
        if p.exists() { return Some(p); }
    }
    let rel = "steamapps/common/TaskbarHero/TaskBarHero_Data";
    for drive in ["C", "D", "E", "F", "G", "H"] {
        for root in [
            format!("{drive}:/Steam"),
            format!("{drive}:/SteamLibrary"),
            format!("{drive}:/Program Files (x86)/Steam"),
            format!("{drive}:/Games/Steam"),
        ] {
            let p = Path::new(&root).join(rel);
            if p.exists() { return Some(p); }
        }
    }
    None
}

fn es3_password() -> String {
    if let Ok(p) = std::env::var("TBH_ES3_PASSWORD") { return p; }
    if let Some(dir) = find_game_data_dir() {
        let re = Regex::new(r"(?s)ES3Defaults.{0,80}?SaveFile_Live\.es3[^\x21-\x7e]+([\x21-\x7e]{8,40})").unwrap();
        for f in ["resources.assets", "sharedassets0.assets", "globalgamemanagers.assets"] {
            if let Ok(bytes) = std::fs::read(dir.join(f)) {
                // latin1 view
                let text: String = bytes.iter().map(|&b| b as char).collect();
                if let Some(c) = re.captures(&text) {
                    return c[1].to_string();
                }
            }
        }
    }
    ES3_FALLBACK_PW.to_string()
}

fn decrypt_es3(buf: &[u8], password: &str) -> anyhow::Result<Vec<u8>> {
    if buf.len() < 32 { anyhow::bail!("save too small"); }
    let iv = &buf[..16];
    let data = &buf[16..];
    let mut key = [0u8; 16];
    pbkdf2::pbkdf2_hmac::<sha1::Sha1>(password.as_bytes(), iv, 100, &mut key);
    let mut buf2 = data.to_vec();
    let out = Aes128CbcDec::new(key.as_ref().into(), iv.into())
        .decrypt_padded_mut::<Pkcs7>(&mut buf2)
        .map_err(|e| anyhow::anyhow!("aes decrypt: {e}"))?
        .to_vec();
    if out.len() >= 2 && out[0] == 0x1f && out[1] == 0x8b {
        let mut gz = GzDecoder::new(&out[..]);
        let mut s = Vec::new();
        gz.read_to_end(&mut s)?;
        Ok(s)
    } else {
        Ok(out)
    }
}

/// Parse PlayerSaveData JSON. serde_json keeps int64 ids exact (u64), so no pre-quoting needed.
fn parse_player_save_data(raw: &str) -> anyhow::Result<Value> {
    Ok(serde_json::from_str(raw)?)
}

fn as_arr(v: &Value) -> Vec<Value> {
    match v {
        Value::Array(a) => a.clone(),
        Value::String(s) => serde_json::from_str::<Value>(s).ok().and_then(|x| x.as_array().cloned()).unwrap_or_default(),
        _ => Vec::new(),
    }
}

// ── Item table + names (bundled seeds) ──────────────────────────────────────────────
static ITEM_TABLE: OnceCell<HashMap<String, Value>> = OnceCell::new();
static ITEM_NAMES: OnceCell<HashMap<String, String>> = OnceCell::new();
static DATA_DIR: OnceCell<PathBuf> = OnceCell::new();

pub fn set_data_dir(p: PathBuf) { let _ = DATA_DIR.set(p); }
fn data_dir() -> PathBuf { DATA_DIR.get().cloned().unwrap_or_else(|| PathBuf::from("data")) }

fn item_table() -> &'static HashMap<String, Value> {
    ITEM_TABLE.get_or_init(|| {
        let mut map = HashMap::new();
        let path = data_dir().join("tbh-itemtable.seed.json");
        if let Ok(txt) = std::fs::read_to_string(&path) {
            if let Ok(Value::Array(arr)) = serde_json::from_str::<Value>(&txt) {
                for r in arr {
                    if let Some(k) = r.get("ItemKey").map(val_to_string) {
                        if !k.is_empty() { map.insert(k, r); }
                    }
                }
            }
        }
        map
    })
}

fn item_names() -> &'static HashMap<String, String> {
    ITEM_NAMES.get_or_init(|| {
        let mut map = HashMap::new();
        let path = data_dir().join("tbh-itemnames.seed.json");
        if let Ok(txt) = std::fs::read_to_string(&path) {
            if let Ok(Value::Object(obj)) = serde_json::from_str::<Value>(&txt) {
                for (k, v) in obj {
                    if let Some(s) = v.as_str() { map.insert(k, s.to_string()); }
                }
            }
        }
        map
    })
}

fn name_from_name_key(row: &Value, names: &HashMap<String, String>) -> Option<String> {
    let key = vstr(row, "ItemKey");
    if let Some(n) = names.get(&key) { return Some(n.clone()); }
    let nk = vstr(row, "NameKey");
    let digits: String = nk.chars().filter(|c| c.is_ascii_digit()).collect();
    if !digits.is_empty() {
        if let Some(n) = names.get(&digits) { return Some(n.clone()); }
    }
    None
}

fn gear_type_text(row: &Value) -> String {
    format!("{} - Lv. {}", title_token(&vstr(row, "GEARTYPE")), vstr(row, "Level"))
}

fn gear_market_hash(row: &Value, names: &HashMap<String, String>) -> Option<String> {
    if vstr(row, "GEARTYPE").is_empty() || vstr(row, "GRADE").is_empty() || vstr(row, "Level").is_empty() {
        return None;
    }
    if !bool_true(row.get("IsCanExchangeMarketable").unwrap_or(&Value::Null)) { return None; }
    let base = name_from_name_key(row, names)?;
    let grade = grade_map(&vstr(row, "GRADE"));
    Some(format!("{base} ({grade}) A"))
}

fn is_gear(row: &Value) -> bool {
    !vstr(row, "GEARTYPE").is_empty() && !vstr(row, "Level").is_empty()
}

/// Map each marketable gear item's Steam hash -> game ItemKey (for the Upgrade Finder).
pub fn gear_key_by_market_hash() -> HashMap<String, i64> {
    let table = item_table();
    let names = item_names();
    let mut m = HashMap::new();
    for (key, row) in table {
        if let Some(hash) = gear_market_hash(row, names) {
            m.entry(hash).or_insert_with(|| key.parse::<i64>().unwrap_or(0));
        }
    }
    m
}

fn synthetic_gear_market_item(row: &Value, names: &HashMap<String, String>) -> Option<Value> {
    let hash = gear_market_hash(row, names)?;
    Some(json!({
        "name": hash, "hash": hash, "priceCents": 0, "priceText": "no listing", "listings": 0,
        "type": gear_type_text(row), "color": "", "icon": "",
        "url": format!("https://steamcommunity.com/market/listings/{TBH_APPID}/{}", enc(&hash)),
        "hasMarketListing": false,
    }))
}

fn enc(s: &str) -> String {
    percent_encoding::utf8_percent_encode(s, percent_encoding::NON_ALPHANUMERIC).to_string()
}

// Market indexes from the passed-in market items cache.
fn build_market_index(items: &[Value]) -> HashMap<String, Value> {
    let re = Regex::new(r"^(\w+)\s*-\s*Lv\.?\s*(\d+)").unwrap();
    let gre = Regex::new(r"\((\w+)\)").unwrap();
    let mut idx = HashMap::new();
    for m in items {
        let ty = vstr(m, "type");
        let name = vstr(m, "name");
        if let (Some(tm), Some(gm)) = (re.captures(&ty), gre.captures(&name)) {
            let key = format!("{}|{}|{}", &tm[1], &gm[1], &tm[2]).to_uppercase();
            idx.insert(key, m.clone());
        }
    }
    idx
}
fn build_market_by_name(items: &[Value]) -> HashMap<String, Value> {
    let mut idx = HashMap::new();
    for m in items { idx.insert(vstr(m, "name").to_lowercase(), m.clone()); }
    idx
}

fn pending_market_item(hash: &str, ty: &str, kind: &str) -> Value {
    json!({
        "name": hash, "hash": hash, "priceCents": 0, "priceText": "price pending", "listings": Value::Null,
        "type": ty, "color": "", "icon": "",
        "url": format!("https://steamcommunity.com/market/listings/{TBH_APPID}/{}", enc(hash)),
        "hasMarketListing": true, "pricePending": true, "kind": kind,
    })
}

/// Read the stash + cross-reference market items. Port of `readStash`. `market_items` = /api/items cache.
pub fn read_stash(market_items: &[Value]) -> anyhow::Result<Value> {
    if !save_exists() { anyhow::bail!("TBH save not found"); }
    let buf = std::fs::read(save_file())?;
    let dec = decrypt_es3(&buf, &es3_password())?;
    let root: Value = serde_json::from_slice(&dec)?;
    let psd_raw = root.get("PlayerSaveData").and_then(|v| v.get("value"))
        .ok_or_else(|| anyhow::anyhow!("PlayerSaveData missing"))?;
    let psd = parse_player_save_data(&val_to_string(psd_raw))?;

    let items = as_arr(&psd["itemSaveDatas"]);
    let mut by_id: HashMap<String, Value> = HashMap::new();
    for it in &items { by_id.insert(val_to_string(&it["UniqueId"]), it.clone()); }

    let mut slot_refs: Vec<Value> = Vec::new();
    for s in as_arr(&psd["stashSaveDatas"]) {
        let mut o = s.clone();
        o["where"] = json!("stash");
        slot_refs.push(o);
    }
    for s in as_arr(&psd["inventorySaveDatas"]) {
        let mut o = s.clone();
        o["where"] = json!("inventory");
        slot_refs.push(o);
    }
    slot_refs.retain(|s| {
        let id = val_to_string(&s["ItemUniqueId"]);
        !id.is_empty() && id != "0"
    });

    let mut seen: HashSet<String> = HashSet::new();
    let mut duplicate_slot_refs_ignored = 0i64;
    let mut slots: Vec<Value> = Vec::new();
    for slot in &slot_refs {
        let id = val_to_string(&slot["ItemUniqueId"]);
        if seen.contains(&id) { duplicate_slot_refs_ignored += 1; continue; }
        seen.insert(id);
        slots.push(slot.clone());
    }

    let table = item_table();
    let names = item_names();
    let mkidx = build_market_index(market_items);
    let mk_by_name = build_market_by_name(market_items);
    // canonical marketable-hash set (accumulated from the current snapshot).
    let mut canon: HashSet<String> = HashSet::new();
    for m in market_items {
        let h = vstr(m, "hash");
        let h = if h.is_empty() { vstr(m, "name") } else { h };
        if !h.is_empty() { canon.insert(h.to_lowercase()); }
    }

    let mut agg: Map<String, Value> = Map::new();
    let mut total_cents = 0i64; let mut gear_cents = 0i64; let mut mat_cents = 0i64;
    let mut priced = 0i64; let mut unpriced = 0i64; let mut unlisted = 0i64; let mut pending = 0i64;
    let mut owned_gear = 0i64; let mut owned_mat = 0i64; let mut owned_other = 0i64;
    let mut unknown: HashMap<String, i64> = HashMap::new();
    let mut unlisted_summary: HashMap<String, i64> = HashMap::new();
    // EVERY stash entry keyed by the market hash we'd search with, matched or not. Deep Scan
    // needs this: many materials have no sell listing (so they never appear in the market
    // cache) yet do have hundreds of BUY orders. Aggregating only matched items hides them.
    let mut entries: Map<String, Value> = Map::new();

    for slot in &slots {
        let iid = val_to_string(&slot["ItemUniqueId"]);
        let it = match by_id.get(&iid) { Some(x) => x, None => continue };
        let item_key = val_to_string(&it["ItemKey"]);
        let row = table.get(&item_key);
        let localized = names.get(&item_key).cloned();

        let gear = row.map(|r| is_gear(r)).unwrap_or(false);
        if gear { owned_gear += 1; }
        else if localized.is_some() { owned_mat += 1; }
        else { owned_other += 1; }

        let mut m: Option<Value> = None;
        let mut kind = "";

        if let Some(r) = row {
            if is_gear(r) {
                let gear_hash = gear_market_hash(r, names);
                let key = format!("{}|{}|{}", vstr(r, "GEARTYPE"), vstr(r, "GRADE"), vstr(r, "Level")).to_uppercase();
                m = mkidx.get(&key).cloned()
                    .or_else(|| gear_hash.as_ref().and_then(|h| mk_by_name.get(&h.to_lowercase()).cloned()));
                if m.is_none() {
                    if let Some(h) = &gear_hash {
                        if canon.contains(&h.to_lowercase()) {
                            m = Some(pending_market_item(h, &gear_type_text(r), "gear"));
                        }
                    }
                }
                if m.is_none() { m = synthetic_gear_market_item(r, names); }
                kind = "gear";
            }
        }
        if m.is_none() {
            if let Some(nm) = &localized {
                let low = nm.to_lowercase();
                if let Some(x) = mk_by_name.get(&low) { m = Some(x.clone()); kind = "material"; }
                else if canon.contains(&low) { m = Some(pending_market_item(nm, "", "material")); kind = "material"; }
            }
        }

        // Borrow (don't move) — `m` is still needed below to build the Deep Scan entry.
        if let Some(mi) = m.as_ref() {
            let hash = vstr(mi, "hash");
            let has_listing = mi.get("hasMarketListing").map(|v| v.as_bool() != Some(false)).unwrap_or(true);
            let price_pending = mi.get("pricePending").and_then(|v| v.as_bool()).unwrap_or(false);
            let price_cents = mi.get("priceCents").and_then(|v| v.as_i64()).unwrap_or(0);
            let entry = agg.entry(hash.clone()).or_insert_with(|| json!({
                "name": vstr(mi, "name"), "hash": hash, "priceCents": price_cents,
                "priceText": mi.get("priceText").cloned().unwrap_or(Value::Null),
                "type": vstr(mi, "type"), "icon": vstr(mi, "icon"), "color": vstr(mi, "color"),
                "url": vstr(mi, "url"), "qty": 0, "kind": kind,
                "hasMarketListing": has_listing, "pricePending": price_pending,
            }));
            entry["qty"] = json!(entry["qty"].as_i64().unwrap_or(0) + 1);
            if !has_listing {
                unlisted += 1;
                *unlisted_summary.entry(vstr(mi, "name")).or_insert(0) += 1;
            } else if price_pending {
                pending += 1;
            } else {
                total_cents += price_cents; priced += 1;
                if kind == "material" { mat_cents += price_cents; } else { gear_cents += price_cents; }
            }
        } else {
            unpriced += 1;
            let label = localized.clone().unwrap_or_else(|| match row {
                Some(r) => format!("{} {} Lv{}", vstr(r, "GEARTYPE"), vstr(r, "GRADE"), vstr(r, "Level")).trim().to_string(),
                None => format!("ItemKey {item_key}"),
            });
            *unknown.entry(label).or_insert(0) += 1;
        }

        // Record the Deep Scan entry for EVERY slot, matched or not.
        let is_g = row.map(|r| is_gear(r)).unwrap_or(false);
        let gear_base = row.filter(|r| is_gear(r)).and_then(|r| name_from_name_key(r, names));
        let gear_hash = row.filter(|r| is_gear(r)).and_then(|r| gear_market_hash(r, names));
        let need_grade_probe = is_g && gear_hash.is_none() && gear_base.is_some();
        let entry_kind = if is_g { "gear" } else if localized.is_some() { "material" } else { "other" };
        let search_name = match (&m, &gear_hash, is_g, &gear_base, &localized) {
            (Some(mi), _, _, _, _) => vstr(mi, "hash"),
            (None, Some(h), _, _, _) => h.clone(),
            (None, None, true, Some(b), _) => b.clone(),
            (None, None, false, _, Some(l)) => l.clone(),
            _ => String::new(),
        };
        if !search_name.is_empty() {
            let key = search_name.to_lowercase();
            let tab = slot.get("Index").and_then(|v| v.as_i64()).map(|i| i / 49);
            let e = entries.entry(key).or_insert_with(|| json!({
                "searchName": search_name,
                "name": m.as_ref().map(|mi| vstr(mi, "name")).or(localized.clone()).unwrap_or_default(),
                "qty": 0, "kind": entry_kind, "matched": false,
                "needGradeProbe": need_grade_probe,
                "baseName": gear_base, "tabs": {},
            }));
            e["qty"] = json!(e["qty"].as_i64().unwrap_or(0) + 1);
            if m.is_some() { e["matched"] = json!(true); }
            if let Some(t) = tab {
                let tk = t.to_string();
                let cur = e["tabs"].get(&tk).and_then(|v| v.as_i64()).unwrap_or(0);
                e["tabs"][tk] = json!(cur + 1);
            }
        }
    }

    let mut list: Vec<Value> = agg.into_iter().map(|(_, v)| v).collect();
    list.sort_by(|a, b| {
        let av = a["priceCents"].as_i64().unwrap_or(0) * a["qty"].as_i64().unwrap_or(1);
        let bv = b["priceCents"].as_i64().unwrap_or(0) * b["qty"].as_i64().unwrap_or(1);
        bv.cmp(&av)
    });

    // Deep Scan entries, most-owned first.
    let mut all_entries: Vec<Value> = entries.into_iter().map(|(_, v)| v).collect();
    all_entries.sort_by(|a, b| b["qty"].as_i64().unwrap_or(0).cmp(&a["qty"].as_i64().unwrap_or(0)));

    let mut unlisted_vec: Vec<(String, i64)> = unlisted_summary.into_iter().collect();
    unlisted_vec.sort_by(|a, b| b.1.cmp(&a.1));
    let mut unknown_vec: Vec<(String, i64)> = unknown.into_iter().collect();
    unknown_vec.sort_by(|a, b| b.1.cmp(&a.1));

    Ok(json!({
        "fetchedAt": now_ms(),
        "saveMtime": save_mtime(),
        "totalCents": total_cents, "gearCents": gear_cents, "matCents": mat_cents,
        "totalItems": slots.len(), "slotRefs": slot_refs.len(),
        "duplicateSlotRefsIgnored": duplicate_slot_refs_ignored,
        "ownedGearItems": owned_gear, "ownedMaterialItems": owned_mat, "ownedOtherItems": owned_other,
        "pricedItems": priced, "pendingItems": pending, "unlistedItems": unlisted, "unpricedItems": unpriced,
        "types": list.len(),
        "dataSources": {
            "itemTable": if item_table().is_empty() { "missing" } else { "seed" },
            "itemTableCount": item_table().len(),
            "itemNames": if item_names().is_empty() { "missing" } else { "seed" },
            "itemNamesCount": item_names().len(),
            "wikiNamesAdded": 0,
        },
        "items": list,
        "allEntries": all_entries,
        "unlistedSummary": unlisted_vec.into_iter().take(30).map(|(k, n)| json!({"label": k, "qty": n})).collect::<Vec<_>>(),
        "unknownSummary": unknown_vec.into_iter().take(30).map(|(k, n)| json!({"label": k, "qty": n})).collect::<Vec<_>>(),
    }))
}

/// 7 tabs × 49 slots stash map. Port of `readTabs`.
pub fn read_tabs() -> anyhow::Result<Value> {
    if !save_exists() { anyhow::bail!("TBH save not found"); }
    let buf = std::fs::read(save_file())?;
    let dec = decrypt_es3(&buf, &es3_password())?;
    let root: Value = serde_json::from_slice(&dec)?;
    let psd_raw = root.get("PlayerSaveData").and_then(|v| v.get("value"))
        .ok_or_else(|| anyhow::anyhow!("PlayerSaveData missing"))?;
    let psd = parse_player_save_data(&val_to_string(psd_raw))?;
    let items = as_arr(&psd["itemSaveDatas"]);
    let mut by_id: HashMap<String, Value> = HashMap::new();
    for it in &items { by_id.insert(val_to_string(&it["UniqueId"]), it.clone()); }
    let table = item_table();
    let names = item_names();
    const TAB_SIZE: i64 = 49;
    let mut tabs: HashMap<i64, Vec<Value>> = HashMap::new();
    for s in as_arr(&psd["stashSaveDatas"]) {
        let iid = val_to_string(&s["ItemUniqueId"]);
        if iid.is_empty() || iid == "0" { continue; }
        let index = s.get("Index").and_then(|v| v.as_i64()).unwrap_or(0);
        let tab = index / TAB_SIZE;
        let cell = index % TAB_SIZE;
        let mut label = "(?)".to_string();
        if let Some(it) = by_id.get(&iid) {
            let ik = val_to_string(&it["ItemKey"]);
            if let Some(r) = table.get(&ik) {
                if is_gear(r) {
                    if let Some(base) = name_from_name_key(r, names) {
                        label = format!("{base} ({}) Lv{}", grade_map(&vstr(r, "GRADE")), vstr(r, "Level"));
                    } else {
                        label = format!("{} {}", gear_type_text(r), vstr(r, "GRADE"));
                    }
                } else {
                    label = names.get(&ik).cloned().unwrap_or_else(|| format!("ItemKey {ik}"));
                }
            } else {
                label = names.get(&ik).cloned().unwrap_or_else(|| format!("ItemKey {ik}"));
            }
        }
        tabs.entry(tab).or_default().push(json!({ "cell": cell, "index": index, "name": label }));
    }
    let mut keys: Vec<i64> = tabs.keys().cloned().collect();
    keys.sort();
    let out: Vec<Value> = keys.iter().map(|t| {
        let mut cells = tabs[t].clone();
        cells.sort_by(|a, b| a["cell"].as_i64().unwrap_or(0).cmp(&b["cell"].as_i64().unwrap_or(0)));
        json!({ "tab": t, "label": format!("Tab {}", t + 1), "occupied": cells.len(), "slots": cells })
    }).collect();
    Ok(json!({ "saveMtime": save_mtime(), "tabSize": TAB_SIZE, "tabCount": out.len(), "tabs": out }))
}

/// Player save as a raw JSON string (for a future engine bridge). Read-only.
pub fn player_save_data_string() -> anyhow::Result<String> {
    if !save_exists() { anyhow::bail!("TBH save not found"); }
    let buf = std::fs::read(save_file())?;
    let dec = decrypt_es3(&buf, &es3_password())?;
    let root: Value = serde_json::from_slice(&dec)?;
    let v = root.get("PlayerSaveData").and_then(|v| v.get("value"))
        .ok_or_else(|| anyhow::anyhow!("PlayerSaveData missing"))?;
    Ok(val_to_string(v))
}

fn now_ms() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as i64).unwrap_or(0)
}
