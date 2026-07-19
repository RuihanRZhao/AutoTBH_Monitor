//! taskbarhero.wiki catalog. Port of `wiki.mjs`.
//! Supplies localized item names (16 locales), rarity colours, icons and deep links for
//! Steam market hashes. Cached to disk and refreshed when the wiki manifest version changes.

use once_cell::sync::Lazy;
use regex::Regex;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

pub const WIKI_BASE: &str = "https://taskbarhero.wiki";
const TTL_MS: u128 = 6 * 3600 * 1000;

/// 10 rarity tiers, ascending.
pub const GRADE_ORDER: &[&str] = &[
    "COMMON", "UNCOMMON", "RARE", "LEGENDARY", "IMMORTAL", "ARCANA", "BEYOND", "CELESTIAL",
    "DIVINE", "COSMIC",
];

pub fn grade_color(grade: &str) -> Option<&'static str> {
    Some(match grade.to_uppercase().as_str() {
        "COMMON" => "#b8c0d0",
        "UNCOMMON" => "#5fcf6b",
        "RARE" => "#5aa0ff",
        "LEGENDARY" => "#c77dff",
        "IMMORTAL" => "#ff7b54",
        "ARCANA" => "#ff5d8f",
        "BEYOND" => "#2ad4c8",
        "CELESTIAL" => "#ffd24a",
        "DIVINE" => "#ff4d4d",
        "COSMIC" => "#a78bfa",
        _ => return None,
    })
}

pub fn grade_rank(grade: &str) -> i32 {
    GRADE_ORDER
        .iter()
        .position(|g| *g == grade.to_uppercase())
        .map(|i| i as i32)
        .unwrap_or(-1)
}

// Strip a trailing " (Grade) X" suffix that Steam market hashes carry, then normalise.
static GRADE_RE: Lazy<Regex> = Lazy::new(|| {
    Regex::new(&format!(
        r"(?i)\s*\(({})\)\s*[a-z]?\s*$",
        GRADE_ORDER.join("|")
    ))
    .unwrap()
});
static NON_ALNUM: Lazy<Regex> = Lazy::new(|| Regex::new(r"[^a-z0-9]+").unwrap());

/// Extract the grade a Steam market hash carries: "Long Sword (Immortal) A" -> "IMMORTAL".
pub fn grade_from_hash(hash: &str) -> Option<String> {
    GRADE_RE
        .captures(hash)
        .and_then(|c| c.get(1))
        .map(|m| m.as_str().to_uppercase())
}

/// "Long Sword (Immortal) A" -> "long sword"
pub fn norm_name(s: &str) -> String {
    let stripped = GRADE_RE.replace(s, "").to_lowercase();
    NON_ALNUM.replace_all(&stripped, " ").trim().to_string()
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SlimItem {
    pub id: i64,
    /// locale -> name
    pub name: HashMap<String, String>,
    #[serde(default)]
    pub grade: Option<String>,
    #[serde(default, rename = "type")]
    pub ty: Option<String>,
    #[serde(default)]
    pub gear: Option<String>,
    #[serde(default)]
    pub icon: Option<String>,
    #[serde(default)]
    pub slug: Option<String>,
    #[serde(default)]
    pub marketable: bool,
}

#[derive(Deserialize)]
struct RawItem {
    id: i64,
    /// Option, not `#[serde(default)] HashMap`: some rows carry an explicit `"name": null`,
    /// and `default` only covers a *missing* field — a null would fail the whole parse.
    #[serde(default)]
    name: Option<HashMap<String, String>>,
    #[serde(default)]
    grade: Option<String>,
    #[serde(default, rename = "type")]
    ty: Option<String>,
    #[serde(default)]
    gear: Option<String>,
    #[serde(default)]
    icon: Option<String>,
    #[serde(default)]
    slug: Option<String>,
    #[serde(default)]
    marketable: bool,
    #[serde(default)]
    deleted: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct Catalog {
    pub version: String,
    pub fetched_at: u128,
    /// normalised English name -> item
    pub by_norm: HashMap<String, SlimItem>,
    /// item id -> item
    pub by_id: HashMap<i64, SlimItem>,
}

impl Catalog {
    pub fn len(&self) -> usize { self.by_norm.len() }

    /// Localized name with English then any-value fallback.
    pub fn localized(map: &HashMap<String, String>, lang: &str) -> String {
        map.get(lang)
            .or_else(|| map.get("en-US"))
            .or_else(|| map.get("en"))
            .or_else(|| map.values().next())
            .cloned()
            .unwrap_or_default()
    }

    /// Resolve enrichment for a Steam market hash. Returns null when no confident match.
    pub fn enrich_hash(&self, hash: &str, lang: &str) -> Value {
        match self.by_norm.get(&norm_name(hash)) {
            Some(it) => self.view(it, lang),
            None => Value::Null,
        }
    }

    pub fn by_id(&self, id: i64, lang: &str) -> Value {
        match self.by_id.get(&id) {
            Some(it) => self.view(it, lang),
            None => Value::Null,
        }
    }

    fn view(&self, it: &SlimItem, lang: &str) -> Value {
        let grade = it.grade.clone().unwrap_or_default();
        json!({
            "id": it.id,
            "name": Self::localized(&it.name, lang),
            "grade": it.grade,
            "gradeColor": grade_color(&grade),
            "gradeRank": grade_rank(&grade),
            "icon": it.icon.as_ref().map(|i| format!("{WIKI_BASE}{i}")),
            "slug": it.slug,
            "wikiUrl": it.slug.as_ref().map(|s| format!("{WIKI_BASE}/items/{s}")),
            "marketable": it.marketable,
        })
    }

    /// id -> English name map, used to fill gaps in the bundled item-name seed.
    pub fn id_name_map(&self) -> HashMap<String, String> {
        self.by_id
            .iter()
            .filter_map(|(id, it)| {
                it.name.get("en-US").map(|n| (id.to_string(), n.clone()))
            })
            .collect()
    }
}

static CATALOG: Lazy<Mutex<Option<Arc<Catalog>>>> = Lazy::new(|| Mutex::new(None));

pub fn cached() -> Option<Arc<Catalog>> {
    CATALOG.lock().unwrap().clone()
}

fn cache_path(data_dir: &Path) -> PathBuf { data_dir.join("wiki/items-slim.json") }

fn now_ms() -> u128 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0)
}

const UA: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/124.0.0.0 Safari/537.36";

async fn fetch_json(url: &str) -> anyhow::Result<Value> {
    let c = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(45))
        .gzip(true)
        .build()?;
    let txt = c.get(url).header("User-Agent", UA).send().await?.text().await?;
    Ok(serde_json::from_str(&txt)?)
}

fn build_catalog(version: String, rows: Vec<RawItem>) -> Catalog {
    let mut by_norm: HashMap<String, SlimItem> = HashMap::new();
    let mut by_id: HashMap<i64, SlimItem> = HashMap::new();
    for r in rows {
        if r.deleted { continue; }
        let names = match r.name { Some(n) => n, None => continue };
        let en = match names.get("en-US").or_else(|| names.get("en")) {
            Some(v) if !v.is_empty() => v.clone(),
            _ => continue,
        };
        let slim = SlimItem {
            id: r.id, name: names, grade: r.grade, ty: r.ty, gear: r.gear,
            icon: r.icon, slug: r.slug, marketable: r.marketable,
        };
        by_id.insert(slim.id, slim.clone());
        // keep the FIRST (lowest-id / live) row per name
        by_norm.entry(norm_name(&en)).or_insert(slim);
    }
    Catalog { version, fetched_at: now_ms(), by_norm, by_id }
}

/// Load from disk, else fetch from the wiki and persist. Safe to call repeatedly.
pub async fn ensure_catalog(data_dir: &Path) -> Option<Arc<Catalog>> {
    if let Some(c) = cached() {
        if now_ms().saturating_sub(c.fetched_at) < TTL_MS {
            return Some(c);
        }
    }
    // disk cache — keep it even when stale, as the offline fallback below.
    let path = cache_path(data_dir);
    let mut stale: Option<Arc<Catalog>> = cached();
    if let Ok(txt) = std::fs::read_to_string(&path) {
        if let Ok(c) = serde_json::from_str::<Catalog>(&txt) {
            if !c.by_norm.is_empty() {
                let fresh = now_ms().saturating_sub(c.fetched_at) < TTL_MS;
                let arc = Arc::new(c);
                *CATALOG.lock().unwrap() = Some(arc.clone());
                if fresh { return Some(arc); }
                stale = Some(arc);
            }
        }
    }
    // network — on failure, fall back to the stale catalog rather than returning None.
    // Returning None made every caller silently drop localized names, icons, rarity colours
    // and wiki links for offline users.
    macro_rules! or_stale {
        ($e:expr) => {
            match $e { Some(v) => v, None => return stale }
        };
    }
    let version = fetch_json(&format!("{WIKI_BASE}/data/manifest.json"))
        .await
        .ok()
        .and_then(|m| m.get("version").and_then(|v| v.as_str()).map(String::from))
        .unwrap_or_default();
    let items = or_stale!(fetch_json(&format!("{WIKI_BASE}/data/items.json")).await.ok());
    let rows: Vec<RawItem> = or_stale!(serde_json::from_value(items).ok());
    let cat = build_catalog(version, rows);
    if cat.by_norm.is_empty() { return stale; }
    if let Some(p) = path.parent() { let _ = std::fs::create_dir_all(p); }
    if let Ok(s) = serde_json::to_string(&cat) { let _ = std::fs::write(&path, s); }
    let arc = Arc::new(cat);
    *CATALOG.lock().unwrap() = Some(arc.clone());
    Some(arc)
}

/// Merge wiki enrichment into a list of items that carry a `hash` (or `name`) field.
/// Bundled names win; the wiki only fills gaps and adds icons/colours/links.
pub fn enrich_items(cat: &Catalog, items: &mut [Value], lang: &str) {
    for it in items.iter_mut() {
        let hash = it
            .get("hash")
            .and_then(|v| v.as_str())
            .or_else(|| it.get("name").and_then(|v| v.as_str()))
            .unwrap_or("")
            .to_string();
        if hash.is_empty() { continue; }
        let w = cat.enrich_hash(&hash, lang);
        if w.is_null() { continue; }
        // The wiki name-index collapses grades onto one base row, so its `grade` is the BASE
        // item's (usually COMMON). The market hash carries the item's ACTUAL grade — prefer it,
        // otherwise a "Witch Staff (Legendary) A" renders as COMMON with the wrong colour.
        let hash_grade = grade_from_hash(&hash);
        let grade = hash_grade
            .clone()
            .or_else(|| w.get("grade").and_then(|g| g.as_str()).map(String::from));
        if let Some(o) = it.as_object_mut() {
            if let Some(g) = &grade {
                o.insert("grade".into(), json!(g));
                o.insert("gradeColor".into(), json!(grade_color(g)));
                o.insert("gradeRank".into(), json!(grade_rank(g)));
            }
            if let Some(u) = w.get("wikiUrl") { o.insert("wikiUrl".into(), u.clone()); }
            if let Some(i) = w.get("icon") { o.insert("wikiIcon".into(), i.clone()); }
            if let Some(n) = w.get("name") { o.insert("wikiName".into(), n.clone()); }
        }
    }
}
