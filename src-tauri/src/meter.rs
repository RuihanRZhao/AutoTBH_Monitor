//! Built-in live meter: DPS / gold / EXP / run tracker.
//!
//! Functionality absorbed from the MIT-licensed <https://github.com/mad-labs-org/tbh-meter>
//! (originally a Python `tbh-reader.exe` sidecar + Electron overlay) and reimplemented natively
//! in Rust as a first-class sub-feature — no Python, no external process. See NOTICE.
//!
//! Strictly read-only (see `memory.rs`). Class resolution follows the upstream design:
//! a build-pinned RVA anchor holds the IL2CPP TypeInfoTable; classes are picked BY INDEX and
//! only *validated* by name. Offsets live in `data/meter-offsets.json` (data, not code) because
//! they shift between game builds.
//!
//! Invariants carried over from upstream:
//!   * never conflate "unread" with "zero" — a failed read stays `None`, never 0.
//!   * combat gold is `GoldEarn[SubKey 1]`; SubKey 0 is a rollup and must never be used.
//!   * damage is inferred from monster HP deltas (the game exposes no damage counter).
//!   * re-deref singletons every read; the GC relocates instances.

use crate::farm::{now_ms, RunRecord};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::{HashMap, VecDeque};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

#[derive(Clone, Debug, Deserialize, Default)]
pub struct Offsets {
    #[serde(default)]
    pub process: ProcessCfg,
    /// Raw values so `_comment`/`_stride`-style annotation keys anywhere inside a class block
    /// can't break offset loading (they did once — a doc string made the whole file unparseable).
    #[serde(default)]
    pub game: HashMap<String, HashMap<String, Value>>,
    #[serde(default)]
    pub tuning: Tuning,
    /// Raw so `_comment`-style keys anywhere in the file can't break offset loading.
    #[serde(default)]
    pub calibration: HashMap<String, Value>,
    #[serde(default)]
    pub gold: HashMap<String, Value>,
}

impl Offsets {
    /// Parse the calibration entries, skipping comment/non-object keys.
    pub fn calibrations(&self) -> Vec<(String, Calibration)> {
        self.calibration
            .iter()
            .filter(|(k, _)| !k.starts_with('_'))
            .filter_map(|(k, v)| serde_json::from_value::<Calibration>(v.clone()).ok().map(|c| (k.clone(), c)))
            .collect()
    }
    /// Field offset for `class.field`, ignoring any annotation keys.
    pub fn game_off(&self, class: &str, field: &str) -> usize {
        self.game
            .get(class)
            .and_then(|m| m.get(field))
            .and_then(|v| v.as_i64())
            .unwrap_or(0) as usize
    }
    pub fn gold_i64(&self, key: &str, default: i64) -> i64 {
        self.gold.get(key).and_then(|v| v.as_i64()).unwrap_or(default)
    }
}

#[derive(Clone, Debug, Deserialize)]
pub struct ProcessCfg {
    #[serde(default = "def_proc")]
    pub process_name: String,
    #[serde(default = "def_mod")]
    pub module_name: String,
}
impl Default for ProcessCfg {
    fn default() -> Self { Self { process_name: def_proc(), module_name: def_mod() } }
}
fn def_proc() -> String { "TaskBarHero.exe".into() }
fn def_mod() -> String { "GameAssembly.dll".into() }

#[derive(Clone, Debug, Deserialize)]
pub struct Tuning {
    #[serde(default = "d_dps_window")] pub dps_window_sec: f64,
    #[serde(default = "d_scan_cap")] pub monster_scan_cap: i32,
}
impl Default for Tuning {
    fn default() -> Self { Self { dps_window_sec: d_dps_window(), monster_scan_cap: d_scan_cap() } }
}
fn d_dps_window() -> f64 { 5.0 }
fn d_scan_cap() -> i32 { 600 }

#[derive(Clone, Debug, Deserialize, Default)]
pub struct Calibration {
    pub anchor_rva: usize,
    #[serde(default)]
    pub idx_aggregate_manager: Option<usize>,
    #[serde(default)]
    pub indices: HashMap<String, usize>,
}

#[derive(Clone, Debug, Default, Serialize)]
pub struct LiveSample {
    pub ts: f64,
    pub attached: bool,
    pub build: Option<String>,
    pub stage_key: Option<i64>,
    pub gold: Option<i64>,
    pub total_damage: Option<f64>,
    pub dps: Option<f64>,
    pub kills: i64,
    pub alive: i64,
    pub party: Vec<i64>,
    pub elapsed_sec: Option<f64>,
}

#[derive(Default)]
pub struct MeterInner {
    pub enabled: bool,
    pub attached: bool,
    pub error: Option<String>,
    pub live: Option<LiveSample>,
    pub runs: Vec<RunRecord>,
    /// addr -> last seen HP, for damage inference
    last_hp: HashMap<usize, f32>,
    /// rolling (ts_ms, damage) window
    window: VecDeque<(f64, f64)>,
    total_damage: f64,
    kills: i64,
    last_alive: i64,
    run_start_ts: Option<f64>,
    run_start_gold: Option<i64>,
    /// Object addresses of the last-seen tail of LogManager.LOG_LIST, oldest first. Identity
    /// (the heap address of each log entry object), NOT array length or index, is what marks an
    /// entry as "already processed" — the list is capped at 2000 by the game itself (confirmed
    /// live: length stays flat at exactly 2000 even as new entries keep appearing), which the
    /// game maintains by dropping old entries as new ones append. A length/index comparison goes
    /// silently blind forever once a save crosses that cap; comparing which addresses are new to
    /// the tail keeps working regardless of how the underlying array is shuffled.
    log_tail: Vec<usize>,
    /// True once `log_tail` has been seeded from a live read. Guards against treating the game's
    /// entire log HISTORY as "new" on the first tick after attach — only entries that appear
    /// after we start watching become RunRecords.
    log_seeded: bool,
}

#[derive(Clone)]
pub struct Meter {
    pub inner: Arc<Mutex<MeterInner>>,
    pub data_dir: PathBuf,
    /// Memoised gear stat table, keyed by PE build fingerprint. The scan that produces it walks
    /// the whole heap and takes ~80s — far too slow to sit behind a page load — but the table is
    /// static game data, so it only has to be rebuilt when the game binary changes.
    gear_table: Arc<Mutex<Option<(String, HashMap<i64, Vec<i32>>)>>>,
}

impl Meter {
    pub fn new(data_dir: PathBuf) -> Self {
        let m = Self {
            inner: Arc::new(Mutex::new(MeterInner::default())),
            data_dir,
            gear_table: Arc::new(Mutex::new(None)),
        };
        m.load_runs();
        m
    }

    fn runs_path(&self) -> PathBuf { self.data_dir.join("meter/runs.json") }
    fn live_path(&self) -> PathBuf { self.data_dir.join("meter/live.json") }
    fn offsets_path(&self) -> PathBuf { self.data_dir.join("meter-offsets.json") }

    fn load_runs(&self) {
        if let Ok(txt) = std::fs::read_to_string(self.runs_path()) {
            if let Ok(v) = serde_json::from_str::<Vec<RunRecord>>(&txt) {
                self.inner.lock().unwrap().runs = v;
            }
        }
    }

    /// Atomic write (tmp + rename) so a consumer never sees a half-written file.
    fn write_atomic(path: &PathBuf, data: &str) {
        if let Some(p) = path.parent() { let _ = std::fs::create_dir_all(p); }
        let tmp = path.with_extension("tmp");
        if std::fs::write(&tmp, data).is_ok() { let _ = std::fs::rename(&tmp, path); }
    }

    /// `live.json` is small and written every tick. `runs.json` is only rewritten when a run
    /// actually closed — it used to be re-serialized in full 10x/second.
    fn persist(&self, runs_changed: bool) {
        let g = self.inner.lock().unwrap();
        if runs_changed {
            if let Ok(s) = serde_json::to_string(&g.runs) { Self::write_atomic(&self.runs_path(), &s); }
        }
        if let Some(live) = &g.live {
            if let Ok(s) = serde_json::to_string(live) { Self::write_atomic(&self.live_path(), &s); }
        }
    }

    pub fn status(&self) -> Value {
        let g = self.inner.lock().unwrap();
        json!({
            "enabled": g.enabled, "attached": g.attached, "error": g.error,
            "runCount": g.runs.len(),
            "build": g.live.as_ref().and_then(|l| l.build.clone()),
        })
    }

    pub fn live_json(&self) -> Value {
        let g = self.inner.lock().unwrap();
        match &g.live {
            Some(l) => json!({ "ok": true, "at": l.ts, "live": l }),
            None => json!({ "ok": false, "enabled": g.enabled, "attached": g.attached, "error": g.error }),
        }
    }

    pub fn runs_json(&self) -> Value {
        let g = self.inner.lock().unwrap();
        let mut runs = g.runs.clone();
        runs.sort_by(|a, b| b.ts.partial_cmp(&a.ts).unwrap());
        json!({ "ok": true, "runs": runs })
    }

    pub fn reset_runs(&self) -> Value {
        let mut g = self.inner.lock().unwrap();
        let total = g.runs.len();
        let ts = now_ms() as i64;
        let dir = self.data_dir.join(format!("meter/archive/{ts}"));
        let _ = std::fs::create_dir_all(&dir);
        if let Ok(s) = serde_json::to_string(&g.runs) { let _ = std::fs::write(dir.join("runs.json"), s); }
        g.runs.clear();
        drop(g);
        self.persist(true);
        json!({ "ok": true, "archived": total, "total": 0, "archive": ts.to_string() })
    }


    /// Find records containing `key` as an i32 that also carry all of `expect` nearby.
    /// Used to locate the gear-stats table, which is keyed by GearKey but lives in a class
    /// whose index isn't known yet.
    #[cfg(windows)]
    pub fn find_record_with(&self, key: i32, expect: Vec<i32>, window: usize) -> Result<Value, String> {
        use crate::memory::GameProcess;
        let cfg = self.offsets()?;
        let proc = GameProcess::attach(&cfg.process.process_name, &cfg.process.module_name)
            .map_err(|e| e.to_string())?;
        let hits = proc.scan_i32(key, 100_000);
        let mut matches = Vec::new();
        for a in &hits {
            let start = a.saturating_sub(window);
            let bytes = match proc.read_bytes(start, window * 2 + 4) { Ok(b) => b, Err(_) => continue };
            let ints: Vec<i32> = bytes.chunks_exact(4)
                .map(|c| i32::from_le_bytes([c[0], c[1], c[2], c[3]])).collect();
            if expect.iter().all(|e| ints.contains(e)) {
                matches.push(json!({
                    "address": format!("0x{a:x}"),
                    "windowStart": format!("0x{start:x}"),
                    "i32": ints,
                }));
                if matches.len() >= 5 { break; }
            }
        }
        Ok(json!({ "ok": true, "key": key, "totalHits": hits.len(), "matches": matches }))
    }

    #[cfg(not(windows))]
    pub fn find_record_with(&self, _key: i32, _expect: Vec<i32>, _window: usize) -> Result<Value, String> {
        Err("memory reading is Windows-only".into())
    }

    /// Extract the game's whole gear-stat table in one memory pass.
    ///
    /// The records are NOT a contiguous array — four confirmed records sit at unrelated
    /// addresses — so there is no stride to walk. Instead this scans once and accepts a
    /// 9-word window as a record only if it is structurally valid AND its key is a real gear
    /// item in the bundled item table. That key check is what keeps the false-positive rate
    /// at zero; raw structural matching alone would happily accept unrelated integer runs.
    ///
    /// Returns `{ gearKey: [key, b1, b2, s1, m1, v1, s2, m2, v2] }`.
    #[cfg(windows)]
    pub fn read_gear_stat_table(&self) -> Result<HashMap<i64, Vec<i32>>, String> {
        use crate::memory::GameProcess;
        let cfg = self.offsets()?;
        let proc = GameProcess::attach(&cfg.process.process_name, &cfg.process.module_name)
            .map_err(|e| e.to_string())?;

        // Keyed by build fingerprint, not just "is it cached": a game update can change the table
        // and would otherwise be served stale values for the lifetime of the app.
        let fingerprint = proc.pe_fingerprint("*").unwrap_or_default();
        if let Some((fp, t)) = self.gear_table.lock().unwrap().as_ref() {
            if *fp == fingerprint {
                return Ok(t.clone());
            }
        }

        // Only accept keys the item table knows as gear.
        let table = crate::save::item_table_snapshot();
        let valid: std::collections::HashSet<i64> = table
            .iter()
            .filter(|(_, row)| {
                row.get("GEARTYPE").and_then(|v| v.as_str()).map(|s| !s.is_empty()).unwrap_or(false)
            })
            .filter_map(|(k, _)| k.parse::<i64>().ok())
            .collect();
        if valid.is_empty() {
            return Err("item table empty — cannot validate gear keys".into());
        }

        let plausible = |stat: i32, mode: i32, val: i32| -> bool {
            (0..=63).contains(&stat) && (0..=2).contains(&mode) && (0..=1_000_000).contains(&val)
        };

        let mut out: HashMap<i64, Vec<i32>> = HashMap::new();
        for (base, size) in proc.readable_regions(4096) {
            let mut off = 0usize;
            const CHUNK: usize = 1 << 20;
            while off < size {
                let len = CHUNK.min(size - off);
                let buf = match proc.read_bytes(base + off, len) { Ok(b) => b, Err(_) => break };
                let words: Vec<i32> = buf
                    .chunks_exact(4)
                    .map(|c| i32::from_le_bytes([c[0], c[1], c[2], c[3]]))
                    .collect();
                for i in 0..words.len().saturating_sub(9) {
                    let key = words[i] as i64;
                    if key < 100_000 || !valid.contains(&key) { continue; }
                    let r = &words[i..i + 9];
                    // base values non-negative, and both inherent triples well-formed
                    if r[1] < 0 || r[2] < 0 { continue; }
                    if !plausible(r[3], r[4], r[5]) || !plausible(r[6], r[7], r[8]) { continue; }
                    out.entry(key).or_insert_with(|| r.to_vec());
                }
                if len < CHUNK { break; }
                off += len;
            }
        }
        if out.is_empty() {
            // Deliberately not cached: an empty result means the scan failed, not that the game
            // has no gear table, and caching it would make the failure permanent.
            return Err("no gear stat records found in memory".into());
        }
        *self.gear_table.lock().unwrap() = Some((fingerprint, out.clone()));
        Ok(out)
    }

    #[cfg(not(windows))]
    pub fn read_gear_stat_table(&self) -> Result<HashMap<i64, Vec<i32>>, String> {
        Err("memory reading is Windows-only".into())
    }


    /// Probe a fielded hero's StatsHolder -> MODIFIER_MGR, to locate the stat-modifier list.
    /// Needed for gear-swap simulation: FINAL_STATS gives only the aggregated product, so the
    /// per-bucket contributions (and their MOD_SOURCE) have to come from the modifier list.
    #[cfg(windows)]
    pub fn probe_modifier_mgr(&self, words: usize) -> Result<Value, String> {
        use crate::memory::GameProcess;
        let cfg = self.offsets()?;
        let proc = GameProcess::attach(&cfg.process.process_name, &cfg.process.module_name)
            .map_err(|e| e.to_string())?;
        let fingerprint = proc.pe_fingerprint("*").unwrap_or_default();
        let suffix = |s: &str| s.splitn(2, '-').nth(1).unwrap_or("").to_string();
        let fp = suffix(&fingerprint);
        let calib = cfg.calibrations().into_iter().find(|(k, _)| suffix(k) == fp).map(|(_, c)| c)
            .ok_or_else(|| format!("no calibration for build {fingerprint}"))?;
        let g = |c: &str, f: &str| cfg.game_off(c, f);

        let smi = calib.indices.get("StageManager").cloned().ok_or("no StageManager index")?;
        let k = proc.class_by_type_index(calib.anchor_rva, smi).map_err(|e| e.to_string())?;
        let sm = proc.singleton_instance(k).map_err(|e| e.to_string())?;
        let hl = proc.read_ptr(sm + g("StageManager", "HERO_LIST")).unwrap_or(0);
        if hl == 0 { return Err("no hero list".into()); }
        let h = proc.read_ptr(proc.il2cpp_array_data(hl)).unwrap_or(0);
        if h == 0 { return Err("no hero in slot 0".into()); }
        let uf = proc.read_ptr(h + g("Unit", "CACHE")).unwrap_or(0);
        let sh = proc.read_ptr(uf + g("StatsHolder", "MODIFIER_MGR").max(16) - 16 + 16).unwrap_or(0);
        let holder = proc.read_ptr(uf + g("HeroRuntime", "STATS_HOLDER")).unwrap_or(0);
        let mgr = proc.read_ptr(holder + g("StatsHolder", "MODIFIER_MGR")).unwrap_or(0);
        let _ = sh;
        if mgr == 0 { return Err(format!("MODIFIER_MGR null (holder=0x{holder:x})")); }

        let bytes = proc.read_bytes(mgr, words * 4).map_err(|e| e.to_string())?;
        let i32s: Vec<i32> = bytes.chunks_exact(4)
            .map(|c| i32::from_le_bytes([c[0], c[1], c[2], c[3]])).collect();
        // Try interpreting nearby pointers as List<T>
        let mut lists = Vec::new();
        for off in (0..words * 4).step_by(8) {
            if let Ok(p) = proc.read_ptr(mgr + off) {
                if p > 0x10000 {
                    if let Ok((items, size)) = proc.read_il2cpp_list(p) {
                        if size > 0 && size < 5000 && items > 0x10000 {
                            lists.push(json!({ "atOffset": format!("0x{off:x}"), "size": size }));
                        }
                    }
                }
            }
        }
        // Follow each pointer one level: report its class name and whether it looks like a
        // List<T> or a Dictionary, so the modifier storage shape can be identified.
        let mut children = Vec::new();
        for off in (0..words * 4).step_by(8) {
            let p = match proc.read_ptr(mgr + off) { Ok(p) if p > 0x10000 => p, _ => continue };
            let klass = proc.read_ptr(p).ok().filter(|k| *k > 0x10000);
            let name = klass.and_then(|k| proc.class_name(k));
            let as_list = proc.read_il2cpp_list(p).ok().filter(|(it, sz)| *it > 0x10000 && *sz > 0 && *sz < 5000);
            let dict_cnt = proc.read_i32(p + 0x20).ok().filter(|c| *c > 0 && *c < 5000);
            children.push(json!({
                "atOffset": format!("0x{off:x}"),
                "addr": format!("0x{p:x}"),
                "class": name,
                "listSize": as_list.map(|(_, s)| s),
                "dictCount": dict_cnt,
            }));
        }
        // Read each child dictionary with the 8-byte-value geometry and follow the values.
        // A Dictionary<StatType, List<StatModifier>> is the shape that would let us decompose a
        // hero's stats into FLAT/ADDITIVE/MULTIPLICATIVE buckets — which is what gear-swap
        // simulation needs, since FINAL_STATS only exposes the aggregated product.
        let mut buckets = serde_json::Map::new();
        for off in [0x10usize, 0x18, 0x20, 0x28, 0x30, 0x38, 0x40, 0x48] {
            let d = match proc.read_ptr(mgr + off) { Ok(d) if d > 0x10000 => d, _ => continue };
            let entries = proc.dict8b_items(d, 500).unwrap_or_default();
            if entries.is_empty() { continue; }
            let mut rows = Vec::new();
            for (stat_id, val) in entries.iter().take(80) {
                let ptr = *val as usize;
                let mut mods = Vec::new();
                if ptr > 0x10000 {
                    // try as List<StatModifier>
                    if let Ok(items) = proc.list_ptrs(ptr, 200) {
                        for m in items.iter().take(6) {
                            let st = proc.read_i32(m + g("StatModifier", "STAT_TYPE")).unwrap_or(-1);
                            let mt = proc.read_i32(m + g("StatModifier", "MOD_TYPE")).unwrap_or(-1);
                            let vf = proc.read_f32(m + g("StatModifier", "VALUE")).unwrap_or(0.0);
                            let vi = proc.read_i32(m + g("StatModifier", "VALUE")).unwrap_or(0);
                            let src = proc.read_i32(m + g("StatModifier", "MOD_SOURCE")).unwrap_or(-1);
                            mods.push(json!({
                                "stat": crate::engine::stat_name(st as i64),
                                "modType": mt, "valueF32": vf, "valueI32": vi, "source": src,
                            }));
                        }
                    }
                }
                if !mods.is_empty() || (*val != 0 && ptr < 0x10000) {
                    rows.push(json!({
                        "statId": stat_id,
                        "stat": crate::engine::stat_name(*stat_id as i64),
                        "rawValue": val,
                        "mods": mods,
                    }));
                }
            }
            buckets.insert(format!("0x{off:x}"), json!({
                "entryCount": entries.len(),
                "rows": rows.into_iter().take(8).collect::<Vec<_>>(),
            }));
        }

        Ok(json!({
            "ok": true,
            "statsHolder": format!("0x{holder:x}"),
            "modifierMgr": format!("0x{mgr:x}"),
            "mgrClass": proc.read_ptr(mgr).ok().and_then(|k| proc.class_name(k)),
            "candidateLists": lists,
            "children": children,
            "buckets": Value::Object(buckets),
        }))
    }

    #[cfg(not(windows))]
    pub fn probe_modifier_mgr(&self, _words: usize) -> Result<Value, String> {
        Err("memory reading is Windows-only".into())
    }

    /// Walk the live IL2CPP TypeInfoTable and return every class whose name contains one of
    /// `needles` (case-insensitive). Read-only: this only follows the type table the running
    /// process already has resident (`class_by_type_index` / `class_name`), the same primitive
    /// used everywhere else in this module — no metadata file parsing, no injection.
    ///
    /// Exists to go from "a string in global-metadata.dat looks relevant" (static analysis) to
    /// "here is the live class, ready to resolve fields against" (the one thing static analysis
    /// alone cannot give us, since global-metadata.dat's own field-layout parsing is a large,
    /// version-sensitive undertaking better avoided when this shortcut is available).
    #[cfg(windows)]
    pub fn scan_classes(&self, needles: &[String], max_index: usize) -> Result<Value, String> {
        use crate::memory::GameProcess;
        let cfg = self.offsets()?;
        let proc = GameProcess::attach(&cfg.process.process_name, &cfg.process.module_name)
            .map_err(|e| e.to_string())?;
        let fingerprint = proc.pe_fingerprint("*").unwrap_or_default();
        let suffix = |s: &str| s.splitn(2, '-').nth(1).unwrap_or("").to_string();
        let fp = suffix(&fingerprint);
        let calib = cfg.calibrations().into_iter().find(|(k, _)| suffix(k) == fp).map(|(_, c)| c)
            .ok_or_else(|| format!("no calibration for build {fingerprint}"))?;

        let needles_lower: Vec<String> = needles.iter().map(|n| n.to_lowercase()).collect();
        let mut hits = Vec::new();
        let mut scanned = 0usize;
        for idx in 0..max_index {
            let Ok(k) = proc.class_by_type_index(calib.anchor_rva, idx) else { continue };
            scanned += 1;
            let Some(name) = proc.class_name(k) else { continue };
            let nl = name.to_lowercase();
            if needles_lower.iter().any(|n| nl.contains(n.as_str())) {
                hits.push(json!({ "typeIndex": idx, "name": name }));
            }
        }
        Ok(json!({ "ok": true, "scanned": scanned, "maxIndex": max_index, "matches": hits }))
    }

    #[cfg(not(windows))]
    pub fn scan_classes(&self, _needles: &[String], _max_index: usize) -> Result<Value, String> {
        Err("memory reading is Windows-only".into())
    }

    /// Heap-scan for live instances of `type_index` and dump each one's first `window` bytes,
    /// interpreted as both f32 and i32 at every 4-byte offset. Read-only (find_instances +
    /// read_bytes only). Meant for eyeballing an unknown class's field layout by magnitude —
    /// e.g. a spawn-delay field should read as a small positive float, an offset-count field as
    /// a small positive int — not for anything that needs the field's *name* (that would require
    /// parsing global-metadata.dat's field table, which this sidesteps).
    #[cfg(windows)]
    pub fn dump_instances(&self, type_index: usize, limit: usize, window: usize) -> Result<Value, String> {
        use crate::memory::GameProcess;
        let cfg = self.offsets()?;
        let proc = GameProcess::attach(&cfg.process.process_name, &cfg.process.module_name)
            .map_err(|e| e.to_string())?;
        let fingerprint = proc.pe_fingerprint("*").unwrap_or_default();
        let suffix = |s: &str| s.splitn(2, '-').nth(1).unwrap_or("").to_string();
        let fp = suffix(&fingerprint);
        let calib = cfg.calibrations().into_iter().find(|(k, _)| suffix(k) == fp).map(|(_, c)| c)
            .ok_or_else(|| format!("no calibration for build {fingerprint}"))?;
        let klass = proc.class_by_type_index(calib.anchor_rva, type_index).map_err(|e| e.to_string())?;
        let name = proc.class_name(klass);

        let instances = proc.find_instances(klass, limit);
        let mut dumps = Vec::new();
        for addr in &instances {
            let buf = proc.read_bytes(*addr, window).unwrap_or_default();
            let mut fields = Vec::new();
            for off in (0..buf.len().saturating_sub(3)).step_by(4) {
                let b: [u8; 4] = buf[off..off + 4].try_into().unwrap();
                let f = f32::from_le_bytes(b);
                let i = i32::from_le_bytes(b);
                fields.push(json!({ "off": format!("0x{off:x}"), "f32": f, "i32": i }));
            }
            dumps.push(json!({ "addr": format!("0x{addr:x}"), "fields": fields }));
        }
        Ok(json!({ "ok": true, "class": name, "typeIndex": type_index, "instanceCount": instances.len(), "instances": dumps }))
    }

    #[cfg(not(windows))]
    pub fn dump_instances(&self, _type_index: usize, _limit: usize, _window: usize) -> Result<Value, String> {
        Err("memory reading is Windows-only".into())
    }

    /// Read `LogManager.LOG_LIST` and decode each entry's authoritative game-side record —
    /// `StageClearLog` (ACT/STAGE/CLEAR_TIME/IS_BOSS) or `StageFailedLog`
    /// (ACT/STAGE/NOW_WAVE/TOTAL_WAVE/IS_ACT_BOSS) — by reading the entry's own class name off
    /// its vtable, the same technique used everywhere else in this module.
    ///
    /// This is the fix for a real bug in the run tracker: `sample_once` inferred "stage cleared"
    /// from the monster list going empty, but a normal inter-wave gap (measured at ~0.9-1s, many
    /// times per stage) also empties the list, so a single clear was getting fragmented into many
    /// bogus few-second "success" records. `CLEAR_TIME` here is computed by the game itself —
    /// no inference needed, no gap to misread.
    #[cfg(windows)]
    pub fn read_stage_logs(&self, limit: usize) -> Result<Value, String> {
        use crate::memory::GameProcess;
        let cfg = self.offsets()?;
        let proc = GameProcess::attach(&cfg.process.process_name, &cfg.process.module_name)
            .map_err(|e| e.to_string())?;
        let fingerprint = proc.pe_fingerprint("*").unwrap_or_default();
        let suffix = |s: &str| s.splitn(2, '-').nth(1).unwrap_or("").to_string();
        let fp = suffix(&fingerprint);
        let calib = cfg.calibrations().into_iter().find(|(k, _)| suffix(k) == fp).map(|(_, c)| c)
            .ok_or_else(|| format!("no calibration for build {fingerprint}"))?;
        let g = |c: &str, f: &str| cfg.game_off(c, f);

        let i = calib.indices.get("LogManager").cloned().ok_or("no LogManager index")?;
        let k = proc.class_by_type_index(calib.anchor_rva, i).map_err(|e| e.to_string())?;
        let lm = proc.singleton_instance(k).map_err(|e| e.to_string())?;
        let list = proc.read_ptr(lm + g("LogManager", "LOG_LIST")).unwrap_or(0);
        let entries = proc.list_ptrs(list, limit as i32).unwrap_or_default();

        let mut clears = Vec::new();
        let mut fails = Vec::new();
        let mut other = HashMap::new();
        for e in &entries {
            let cls = proc.read_ptr(*e).unwrap_or(0);
            let name = proc.class_name(cls).unwrap_or_default();
            match name.as_str() {
                "StageClearLog" => {
                    let ct_off = e + g("StageClearLog", "CLEAR_TIME");
                    clears.push(json!({
                        "act": proc.read_i32(e + g("StageClearLog", "ACT")).ok(),
                        "stage": proc.read_i32(e + g("StageClearLog", "STAGE")).ok(),
                        "clearTimeF32": proc.read_f32(ct_off).ok(),
                        "clearTimeI32": proc.read_i32(ct_off).ok(),
                        "clearTimeI64": proc.read_i64(ct_off).ok(),
                        // ±4 bytes either side, in case CLEAR_TIME's real offset is off by a word
                        // (a wrong-but-close offset is a common failure mode elsewhere in this file).
                        "neighboursF32": (-8i64..=8).step_by(4).map(|d| proc.read_f32((ct_off as i64 + d) as usize).ok()).collect::<Vec<_>>(),
                        "isBoss": proc.read_i32(e + g("StageClearLog", "IS_BOSS")).ok(),
                    }));
                }
                "StageFailedLog" => fails.push(json!({
                    "act": proc.read_i32(e + g("StageFailedLog", "ACT")).ok(),
                    "stage": proc.read_i32(e + g("StageFailedLog", "STAGE")).ok(),
                    "nowWave": proc.read_i32(e + g("StageFailedLog", "NOW_WAVE")).ok(),
                    "totalWave": proc.read_i32(e + g("StageFailedLog", "TOTAL_WAVE")).ok(),
                })),
                _ => { *other.entry(name).or_insert(0) += 1; }
            }
        }
        Ok(json!({
            "ok": true, "totalEntries": entries.len(),
            "clears": clears, "fails": fails,
            "otherTypes": other,
        }))
    }

    #[cfg(not(windows))]
    pub fn read_stage_logs(&self, _limit: usize) -> Result<Value, String> {
        Err("memory reading is Windows-only".into())
    }

    /// Dump live monsters' current/max HP straight from the game.
    /// Settles whether a stage table's totalHP is on the same scale as what the game actually
    /// spawns — the game is the authority for numeric parameters.
    #[cfg(windows)]
    pub fn probe_monster_hp(&self) -> Result<Value, String> {
        use crate::memory::GameProcess;
        let cfg = self.offsets()?;
        let proc = GameProcess::attach(&cfg.process.process_name, &cfg.process.module_name)
            .map_err(|e| e.to_string())?;
        let fingerprint = proc.pe_fingerprint("*").unwrap_or_default();
        let suffix = |s: &str| s.splitn(2, '-').nth(1).unwrap_or("").to_string();
        let fp = suffix(&fingerprint);
        let calib = cfg.calibrations().into_iter().find(|(k, _)| suffix(k) == fp).map(|(_, c)| c)
            .ok_or_else(|| format!("no calibration for build {fingerprint}"))?;
        let g = |c: &str, f: &str| cfg.game_off(c, f);
        let i = calib.indices.get("MonsterSpawnManager").cloned().ok_or("no MonsterSpawnManager index")?;
        let k = proc.class_by_type_index(calib.anchor_rva, i).map_err(|e| e.to_string())?;
        let msm = proc.singleton_instance(k).map_err(|e| e.to_string())?;
        let list = proc.read_ptr(msm + g("MonsterSpawnManager", "MONSTER_LIST")).unwrap_or(0);
        let units = proc.list_ptrs(list, 600).unwrap_or_default();

        let mut mons = Vec::new();
        let mut stage_votes: HashMap<i64, i32> = HashMap::new();
        for u in &units {
            let hc = proc.read_ptr(u + g("Unit", "HEALTH_CONTROLLER")).unwrap_or(0);
            if hc == 0 { continue; }
            let cur = proc.read_f32(hc + g("UnitHealthController", "HP_CURRENT")).unwrap_or(0.0);
            let max = proc.read_f32(hc + g("UnitHealthController", "HP_MAX")).unwrap_or(0.0);
            if let Ok(sk) = proc.read_i32(u + g("Monster", "STAGE_KEY")) {
                let sk = sk as i64;
                if sk > 0 && sk < 10_000_000 { *stage_votes.entry(sk).or_insert(0) += 1; }
            }
            if max > 0.0 { mons.push(json!({ "hpCurrent": cur, "hpMax": max })); }
        }
        let stage = stage_votes.into_iter().max_by_key(|(_, n)| *n).map(|(k, _)| k);
        let maxes: Vec<f64> = mons.iter().filter_map(|m| m["hpMax"].as_f64()).collect();
        let avg = if maxes.is_empty() { 0.0 } else { maxes.iter().sum::<f64>() / maxes.len() as f64 };
        // Totals, not just the sampled 12: classifying a run's idle time needs the whole field.
        let sum_cur: f64 = mons.iter().filter_map(|m| m["hpCurrent"].as_f64()).sum();
        let sum_max: f64 = maxes.iter().sum();
        Ok(json!({
            "ok": true,
            "stageKey": stage,
            "aliveMonsters": mons.len(),
            "sumHpCurrent": sum_cur,
            "sumHpMax": sum_max,
            "avgHpMax": avg,
            "minHpMax": maxes.iter().cloned().fold(f64::INFINITY, f64::min),
            "maxHpMax": maxes.iter().cloned().fold(0.0, f64::max),
            "monsters": mons.into_iter().take(12).collect::<Vec<_>>(),
        }))
    }

    #[cfg(not(windows))]
    pub fn probe_monster_hp(&self) -> Result<Value, String> {
        Err("memory reading is Windows-only".into())
    }

    /// Full stat-modifier decomposition for each fielded hero, straight from the game.
    ///
    /// `StatsHolder.MODIFIER_MGR` holds a `Dictionary<StatType, List<StatModifier>>` (64 entries,
    /// one per StatType). Each modifier carries its MOD_SOURCE, so ITEM-sourced ones can be
    /// swapped out — which is what gear-swap simulation needs. FINAL_STATS only exposes the
    /// aggregated product, from which the buckets cannot be recovered (3 unknowns, 1 equation).
    ///
    /// Values are in the game's native units: ADDITIVE/MULTIPLICATIVE are fractions here, not the
    /// per-mille/percent integers the reference engine displays.
    #[cfg(windows)]
    pub fn read_party_modifiers(&self) -> Result<Vec<Value>, String> {
        use crate::memory::GameProcess;
        let cfg = self.offsets()?;
        let proc = GameProcess::attach(&cfg.process.process_name, &cfg.process.module_name)
            .map_err(|e| e.to_string())?;
        let fingerprint = proc.pe_fingerprint("*").unwrap_or_default();
        let suffix = |s: &str| s.splitn(2, '-').nth(1).unwrap_or("").to_string();
        let fp = suffix(&fingerprint);
        let calib = cfg
            .calibrations()
            .into_iter()
            .find(|(k, _)| suffix(k) == fp)
            .map(|(_, c)| c)
            .ok_or_else(|| format!("no calibration for build {fingerprint}"))?;
        let g = |c: &str, f: &str| cfg.game_off(c, f);

        let smi = calib.indices.get("StageManager").cloned().ok_or("no StageManager index")?;
        let k = proc.class_by_type_index(calib.anchor_rva, smi).map_err(|e| e.to_string())?;
        let sm = proc.singleton_instance(k).map_err(|e| e.to_string())?;
        let hl = proc.read_ptr(sm + g("StageManager", "HERO_LIST")).unwrap_or(0);
        if hl == 0 { return Ok(Vec::new()); }
        let n = proc.read_il2cpp_array_len(hl).unwrap_or(0).clamp(0, 12);

        let mut out = Vec::new();
        for slot in 0..n as usize {
            let h = proc.read_ptr(proc.il2cpp_array_data(hl) + slot * 8).unwrap_or(0);
            if h == 0 { continue; }
            let uf = proc.read_ptr(h + g("Unit", "CACHE")).unwrap_or(0);
            if uf == 0 { continue; }
            let hi = proc.read_ptr(uf + g("HeroRuntime", "INFO")).unwrap_or(0);
            let hk = if hi != 0 { proc.read_i32(hi + g("HeroInfoData", "HERO_KEY")).unwrap_or(0) as i64 } else { 0 };
            if hk <= 0 { continue; }
            let holder = proc.read_ptr(uf + g("HeroRuntime", "STATS_HOLDER")).unwrap_or(0);
            let mgr = if holder != 0 { proc.read_ptr(holder + g("StatsHolder", "MODIFIER_MGR")).unwrap_or(0) } else { 0 };
            if mgr == 0 { continue; }
            // Per-StatType dictionary at +0x10 (64 entries, matching the StatType enum).
            let dict = proc.read_ptr(mgr + 0x10).unwrap_or(0);
            if dict == 0 { continue; }

            let mut stats = serde_json::Map::new();
            for (stat_id, val) in proc.dict8b_items(dict, 500).unwrap_or_default() {
                let list = val as usize;
                if list <= 0x10000 { continue; }
                let items = match proc.list_ptrs(list, 400) { Ok(i) => i, Err(_) => continue };
                if items.is_empty() { continue; }
                let (mut flat, mut add, mut mul) = (Vec::new(), Vec::new(), Vec::new());
                // Keep (modType, source) PAIRED. Bucketing by source alone discards which bucket
                // each value lands in, and gear-swap reconciliation needs both together.
                let mut mods = Vec::new();
                for m in &items {
                    let mt = proc.read_i32(m + g("StatModifier", "MOD_TYPE")).unwrap_or(-1);
                    let v = proc.read_f32(m + g("StatModifier", "VALUE")).unwrap_or(0.0) as f64;
                    let src = proc.read_i32(m + g("StatModifier", "MOD_SOURCE")).unwrap_or(-1) as i64;
                    match mt {
                        0 => flat.push(v),
                        1 => add.push(v),
                        2 => mul.push(v),
                        _ => continue,
                    }
                    mods.push(json!({ "mode": mt, "value": v, "source": src }));
                }
                stats.insert(
                    crate::engine::stat_name(stat_id as i64).to_string(),
                    json!({
                        "flat": flat, "additive": add, "multiplicative": mul,
                        "count": items.len(), "mods": mods,
                        "statId": stat_id,
                    }),
                );
            }
            out.push(json!({ "heroKey": hk, "slot": slot, "stats": Value::Object(stats) }));
        }
        Ok(out)
    }

    #[cfg(not(windows))]
    pub fn read_party_modifiers(&self) -> Result<Vec<Value>, String> {
        Err("memory reading is Windows-only".into())
    }

    pub fn set_enabled(&self, on: bool) { self.inner.lock().unwrap().enabled = on; }

    pub fn offsets(&self) -> Result<Offsets, String> {
        let p = self.offsets_path();
        let txt = std::fs::read_to_string(&p)
            .map_err(|e| format!("cannot read {}: {e}", p.display()))?;
        serde_json::from_str(&txt).map_err(|e| format!("cannot parse {}: {e}", p.display()))
    }

    /// One-shot read of each fielded hero's FINAL_STATS (`Dict<StatType,float>`), the game's fully
    /// resolved stats after gear/attributes/runes/pets. Returns `[{heroKey, slot, stats:{id:val}}]`.
    /// Requires the game running with heroes on the field (the save has no final stats).
    #[cfg(windows)]
    pub fn read_party_stats(&self) -> Result<Vec<Value>, String> {
        use crate::memory::GameProcess;
        let cfg = self.offsets()?;
        let proc = GameProcess::attach(&cfg.process.process_name, &cfg.process.module_name)
            .map_err(|e| e.to_string())?;
        let fingerprint = proc.pe_fingerprint("*").unwrap_or_default();
        let suffix = |s: &str| s.splitn(2, '-').nth(1).unwrap_or("").to_string();
        let fp = suffix(&fingerprint);
        let calib = cfg
            .calibrations()
            .into_iter()
            .find(|(k, _)| suffix(k) == fp)
            .map(|(_, c)| c)
            .ok_or_else(|| format!("no calibration for build {fingerprint}"))?;
        let g = |cls: &str, f: &str| cfg.game_off(cls, f);

        let smi = calib.indices.get("StageManager").cloned().ok_or("no StageManager index")?;
        let k = proc.class_by_type_index(calib.anchor_rva, smi).map_err(|e| e.to_string())?;
        let sm = proc.singleton_instance(k).map_err(|e| e.to_string())?;
        let hl = proc.read_ptr(sm + g("StageManager", "HERO_LIST")).unwrap_or(0);
        if hl == 0 { return Ok(Vec::new()); }
        let n = proc.read_il2cpp_array_len(hl).unwrap_or(0).clamp(0, 12);

        let mut out = Vec::new();
        for s in 0..n as usize {
            let h = proc.read_ptr(proc.il2cpp_array_data(hl) + s * 8).unwrap_or(0);
            if h == 0 { continue; }
            let uf = proc.read_ptr(h + g("Unit", "CACHE")).unwrap_or(0);
            if uf == 0 { continue; }
            let hi = proc.read_ptr(uf + g("HeroRuntime", "INFO")).unwrap_or(0);
            let hk = if hi != 0 { proc.read_i32(hi + g("HeroInfoData", "HERO_KEY")).unwrap_or(0) as i64 } else { 0 };
            if hk <= 0 || hk >= 10_000_000 { continue; }
            let sh = proc.read_ptr(uf + g("HeroRuntime", "STATS_HOLDER")).unwrap_or(0);
            let fs = if sh != 0 { proc.read_ptr(sh + g("StatsHolder", "FINAL_STATS")).unwrap_or(0) } else { 0 };
            let mut stats = serde_json::Map::new();
            if fs != 0 {
                for (id, val) in proc.dictfloat_items(fs, 200).unwrap_or_default() {
                    stats.insert(id.to_string(), json!(val as f64));
                }
            }
            out.push(json!({ "heroKey": hk, "slot": s, "stats": stats }));
        }
        Ok(out)
    }

    #[cfg(not(windows))]
    pub fn read_party_stats(&self) -> Result<Vec<Value>, String> {
        Err("memory reading is Windows-only".into())
    }

    /// Locate the live `ItemInfoData` object for an ItemKey and dump its field window as i32s.
    ///
    /// Used to establish where the game keeps each item's stat lines. The game is the
    /// authoritative source for numeric parameters, but its asset files no longer ship a plain
    /// table (searched exhaustively), so the layout has to be recovered from a loaded object.
    #[cfg(windows)]
    pub fn probe_item_info(&self, want_key: i64, words: usize, deref: Option<usize>) -> Result<Value, String> {
        use crate::memory::GameProcess;
        let cfg = self.offsets()?;
        let proc = GameProcess::attach(&cfg.process.process_name, &cfg.process.module_name)
            .map_err(|e| e.to_string())?;
        let fingerprint = proc.pe_fingerprint("*").unwrap_or_default();
        let suffix = |s: &str| s.splitn(2, '-').nth(1).unwrap_or("").to_string();
        let fp = suffix(&fingerprint);
        let calib = cfg
            .calibrations()
            .into_iter()
            .find(|(k, _)| suffix(k) == fp)
            .map(|(_, c)| c)
            .ok_or_else(|| format!("no calibration for build {fingerprint}"))?;
        let idx = calib.indices.get("ItemInfoData").cloned().ok_or("no ItemInfoData index")?;
        let klass = proc.class_by_type_index(calib.anchor_rva, idx).map_err(|e| e.to_string())?;
        let name = proc.class_name(klass).unwrap_or_default();
        let key_off = match cfg.game_off("ItemInfoData", "ITEM_KEY") { 0 => 48, v => v };

        let instances = proc.find_instances(klass, 20000);
        let mut matched = None;
        for a in &instances {
            if proc.read_i32(a + key_off).ok().map(|v| v as i64) == Some(want_key) {
                matched = Some(*a);
                break;
            }
        }
        let addr = matched.ok_or_else(|| {
            format!("ItemKey {want_key} not found among {} ItemInfoData instances", instances.len())
        })?;

        // Optionally follow a pointer stored at `deref` within the record, and dump its target
        // instead — the stat lines live behind per-record pointers, not inline.
        let (addr, via) = match deref {
            Some(off) => {
                let p = proc.read_ptr(addr + off).map_err(|e| e.to_string())?;
                if p == 0 { return Err(format!("null pointer at +0x{off:x}")); }
                (p, Some(format!("+0x{off:x}")))
            }
            None => (addr, None),
        };

        let bytes = proc.read_bytes(addr, words * 4).map_err(|e| e.to_string())?;
        let i32s: Vec<i32> = bytes
            .chunks_exact(4)
            .map(|c| i32::from_le_bytes([c[0], c[1], c[2], c[3]]))
            .collect();
        let f32s: Vec<f32> = bytes
            .chunks_exact(4)
            .map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]]))
            .collect();
        Ok(json!({
            "ok": true,
            "className": name,
            "instanceCount": instances.len(),
            "address": format!("0x{addr:x}"),
            "via": via,
            "itemKeyOffset": key_off,
            "i32": i32s,
            "f32": f32s.iter().map(|v| if v.is_finite() && v.abs() < 1e9 { json!(v) } else { Value::Null }).collect::<Vec<_>>(),
        }))
    }

    #[cfg(not(windows))]
    pub fn probe_item_info(&self, _want_key: i64, _words: usize, _deref: Option<usize>) -> Result<Value, String> {
        Err("memory reading is Windows-only".into())
    }

    /// Background sampler (10 Hz while enabled).
    pub fn spawn_sampler(&self) {
        let me = self.clone();
        std::thread::spawn(move || loop {
            if !me.inner.lock().unwrap().enabled {
                std::thread::sleep(std::time::Duration::from_millis(600));
                continue;
            }
            me.sample_once();
            std::thread::sleep(std::time::Duration::from_millis(100));
        });
    }

    #[cfg(windows)]
    fn sample_once(&self) {
        use crate::memory::GameProcess;

        let cfg = match self.offsets() {
            Ok(o) => o,
            Err(e) => { self.fail(&e); return; }
        };

        let proc = match GameProcess::attach(&cfg.process.process_name, &cfg.process.module_name) {
            Ok(p) => p,
            Err(e) => { self.fail(&e.to_string()); return; }
        };

        // Identify the build and pick its pinned calibration.
        let fingerprint = proc.pe_fingerprint("*").unwrap_or_default();
        // Match on the TimeDateStamp+SizeOfImage suffix; the version prefix may differ.
        let suffix = |s: &str| s.splitn(2, '-').nth(1).unwrap_or("").to_string();
        let fp_suffix = suffix(&fingerprint);
        let calib = cfg
            .calibrations()
            .into_iter()
            .find(|(k, _)| suffix(k) == fp_suffix);

        let (build_id, calib) = match calib {
            Some((k, c)) => (k, c),
            None => {
                let mut g = self.inner.lock().unwrap();
                g.attached = true;
                g.error = Some(format!(
                    "attached, but no calibration for this game build ({fingerprint}) — add its anchor_rva + type indices to data/meter-offsets.json"
                ));
                g.live = Some(LiveSample { ts: now_ms(), attached: true, build: Some(fingerprint), ..Default::default() });
                return;
            }
        };

        let g_off = |cls: &str, field: &str| -> usize { cfg.game_off(cls, field) };
        let idx = |name: &str| calib.indices.get(name).cloned();

        // ── monsters → damage/DPS/kills ─────────────────────────────────────
        let mut alive = 0i64;
        let mut damage_this_tick = 0.0f64;
        let mut stage_key: Option<i64> = None;
        let mut current: HashMap<usize, f32> = HashMap::new();

        if let Some(i) = idx("MonsterSpawnManager") {
            if let Ok(k) = proc.class_by_type_index(calib.anchor_rva, i) {
                if let Ok(msm) = proc.singleton_instance(k) {
                    let list = proc.read_ptr(msm + g_off("MonsterSpawnManager", "MONSTER_LIST")).unwrap_or(0);
                    let units = proc.list_ptrs(list, cfg.tuning.monster_scan_cap).unwrap_or_default();
                    let mut stage_votes: HashMap<i64, i32> = HashMap::new();
                    for u in &units {
                        // one 16-byte read: hp_cur at +0x40, hp_max at +0x4C
                        if let Ok(hc) = proc.read_ptr(u + g_off("Unit", "HEALTH_CONTROLLER")) {
                            if hc != 0 {
                                if let Ok(hp) = proc.read_f32(hc + g_off("UnitHealthController", "HP_CURRENT")) {
                                    if hp > 0.0 { current.insert(*u, hp); alive += 1; }
                                }
                            }
                        }
                        if let Ok(sk) = proc.read_i32(u + g_off("Monster", "STAGE_KEY")) {
                            let sk = sk as i64;
                            if sk > 0 && sk < 10_000_000 { *stage_votes.entry(sk).or_insert(0) += 1; }
                        }
                    }
                    stage_key = stage_votes.into_iter().max_by_key(|(_, n)| *n).map(|(k, _)| k);
                }
            }
        }

        // ── combat gold: AggregateManager → GoldEarn(2) → SubKey 1 ──────────
        let gold_earn = cfg.gold_i64("COMBAT_SUBKEY", 1) as i32;
        let mut gold: Option<i64> = None;
        if let Some(i) = calib.idx_aggregate_manager {
            if let Ok(k) = proc.class_by_type_index(calib.anchor_rva, i) {
                if let Ok(inst) = proc.singleton_instance(k) {
                    let outer = proc.read_ptr(inst + g_off("AggregateManager", "AGGREGATES")).unwrap_or(0);
                    if outer != 0 {
                        for (key, val) in proc.dict8b_items(outer, 100_000).unwrap_or_default() {
                            if key == 2 {
                                // value is the inner Dict8B*
                                for (sub, v) in proc.dict8b_items(val as usize, 100_000).unwrap_or_default() {
                                    if sub == gold_earn && v > 0 && v < 1_000_000_000_000_000 {
                                        if gold.is_none() { gold = Some(v); }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }

        // ── live party (formation order) ────────────────────────────────────
        let mut party: Vec<i64> = Vec::new();
        if let Some(i) = idx("StageManager") {
            if let Ok(k) = proc.class_by_type_index(calib.anchor_rva, i) {
                if let Ok(sm) = proc.singleton_instance(k) {
                    let hl = proc.read_ptr(sm + g_off("StageManager", "HERO_LIST")).unwrap_or(0);
                    if hl != 0 {
                        let n = proc.read_il2cpp_array_len(hl).unwrap_or(0);
                        if n > 0 && n <= 12 {
                            for s in 0..n as usize {
                                let h = proc.read_ptr(proc.il2cpp_array_data(hl) + s * 8).unwrap_or(0);
                                if h == 0 { continue; } // empty formation slot
                                let uf = proc.read_ptr(h + g_off("Unit", "CACHE")).unwrap_or(0);
                                if uf == 0 { continue; }
                                let hi = proc.read_ptr(uf + g_off("HeroRuntime", "INFO")).unwrap_or(0);
                                if hi == 0 { continue; }
                                if let Ok(hk) = proc.read_i32(hi + g_off("HeroInfoData", "HERO_KEY")) {
                                    let hk = hk as i64;
                                    if hk > 0 && hk < 10_000_000 { party.push(hk); }
                                }
                            }
                        }
                    }
                }
            }
        }

        // ── authoritative run boundaries: LogManager.LOG_LIST ──────────────
        // Peek the previous tail snapshot (no lock held across the memory reads below — this
        // thread is the only writer, so a stale read here just means we redecode a couple of
        // already-seen entries next tick, never lose or duplicate a record).
        let (log_tail_before, log_seeded_before) = {
            let g = self.inner.lock().unwrap();
            (g.log_tail.clone(), g.log_seeded)
        };
        const LOG_TAIL_WINDOW: usize = 64;
        let mut new_clears: Vec<(i64, i64)> = Vec::new(); // (stageKey, clearTimeSec)
        let mut new_fails = 0usize;
        let mut log_tail_now: Vec<usize> = Vec::new();
        if let Some(i) = idx("LogManager") {
            if let Ok(k) = proc.class_by_type_index(calib.anchor_rva, i) {
                if let Ok(lm) = proc.singleton_instance(k) {
                    let list = proc.read_ptr(lm + g_off("LogManager", "LOG_LIST")).unwrap_or(0);
                    if let Ok((items, size)) = proc.read_il2cpp_list(list) {
                        if items != 0 && size > 0 {
                            let size = size as usize;
                            let start_i = size.saturating_sub(LOG_TAIL_WINDOW);
                            let seen_before: std::collections::HashSet<usize> =
                                log_tail_before.iter().copied().collect();
                            for slot in start_i..size {
                                let e = proc.read_ptr(proc.il2cpp_array_data(items) + slot * 8).unwrap_or(0);
                                if e == 0 { continue; }
                                log_tail_now.push(e);
                                // Only decode entries genuinely new to the tail, and only once we
                                // have a real baseline — decoding the whole backlog on first
                                // attach would replay the game's entire history as if it just
                                // happened. `seen_before` (not index/length) is what makes this
                                // correct across the game's own 2000-entry cap: once hit, the
                                // array shifts and length stops changing, so a length comparison
                                // would go silently blind forever, but each entry's own heap
                                // address stays stable regardless of its slot.
                                if !log_seeded_before || seen_before.contains(&e) { continue; }
                                let cls = proc.read_ptr(e).unwrap_or(0);
                                let name = proc.class_name(cls).unwrap_or_default();
                                match name.as_str() {
                                    "StageClearLog" => {
                                        let act = proc.read_i32(e + g_off("StageClearLog", "ACT")).unwrap_or(0) as i64;
                                        let stage = proc.read_i32(e + g_off("StageClearLog", "STAGE")).unwrap_or(0) as i64;
                                        // CLEAR_TIME is an i32 (whole seconds) — reading it as f32
                                        // decodes small integer bit patterns as denormalised
                                        // floats (~1e-43), which is how this was first misread.
                                        let ct = proc.read_i32(e + g_off("StageClearLog", "CLEAR_TIME")).unwrap_or(0) as i64;
                                        if act > 0 && stage > 0 && ct > 0 {
                                            // key = 1000 + act*100 + stageNo, matching farm_stages.json
                                            new_clears.push((1000 + act * 100 + stage, ct));
                                        }
                                    }
                                    "StageFailedLog" => new_fails += 1,
                                    _ => {}
                                }
                            }
                        }
                    }
                }
            }
        }

        // ── fold into state ─────────────────────────────────────────────────
        let ts = now_ms();
        let mut g = self.inner.lock().unwrap();
        g.attached = true;
        g.error = None;

        // Damage = sum of HP drops + full HP of units that vanished (killing blow).
        for (addr, hp) in &current {
            if let Some(prev) = g.last_hp.get(addr) {
                if hp < prev { damage_this_tick += (*prev - *hp) as f64; }
            }
        }
        let vanished: Vec<(usize, f32)> = g.last_hp.iter()
            .filter(|(a, _)| !current.contains_key(*a))
            .map(|(a, h)| (*a, *h)).collect();
        for (_, prev_hp) in vanished {
            if prev_hp > 0.0 { damage_this_tick += prev_hp as f64; }
        }
        g.last_hp = current;

        // `run_start_ts` is now ONLY a live-display "how long has combat been continuous"
        // ticker — it no longer decides when a RunRecord gets written. That decision comes
        // entirely from the log-tail decoded above, which is the game's own record of what
        // happened, not our inference from watching the monster list.
        let in_combat = alive > 0;
        let mut runs_changed = false;

        if !g.log_seeded {
            // First observation: adopt the current tail as the baseline without emitting
            // anything for it — those entries happened before this process started watching.
            g.log_tail = log_tail_now;
            g.log_seeded = true;
            g.run_start_gold = gold;
        } else {
            for (stage_key, clear_sec) in &new_clears {
                let rec = RunRecord {
                    ts,
                    outcome: "success".into(),
                    stage_key: Some(*stage_key),
                    difficulty: None,
                    clear_time: Some(*clear_sec as f64),
                    total_damage: Some(g.total_damage),
                    gold: match (gold, g.run_start_gold) {
                        (Some(a), Some(b)) if a >= b => Some((a - b) as f64),
                        _ => None,
                    },
                    xp: None,
                };
                g.runs.push(rec);
                runs_changed = true;
                g.total_damage = 0.0;
                g.kills = 0;
                g.window.clear();
                g.run_start_gold = gold;
            }
            // Failed attempts still end the accumulation window; not recorded as a RunRecord
            // yet (farm.rs's aggregator only wants "success", and a fail schema — which wave was
            // reached, etc. — is future scope, not needed to fix the fragmentation bug).
            if new_fails > 0 {
                g.total_damage = 0.0;
                g.kills = 0;
                g.window.clear();
                g.run_start_gold = gold;
            }
            if !log_tail_now.is_empty() { g.log_tail = log_tail_now; }
            if g.runs.len() > 2000 { let n = g.runs.len() - 2000; g.runs.drain(0..n); }
        }
        if in_combat && g.run_start_ts.is_none() {
            g.run_start_ts = Some(ts);
        }
        if !in_combat {
            g.run_start_ts = None;
        }

        if damage_this_tick > 0.0 {
            g.total_damage += damage_this_tick;
            g.window.push_back((ts, damage_this_tick));
        }
        let win_ms = cfg.tuning.dps_window_sec * 1000.0;
        while let Some(&(t0, _)) = g.window.front() {
            if ts - t0 > win_ms { g.window.pop_front(); } else { break; }
        }
        // Upstream semantics: fixed divisor (ramps up over the first window).
        // max(0.0) also normalises -0.0, which serializes as "-0.0" and reads like a bug.
        let dps = Some(
            (g.window.iter().map(|(_, d)| *d).sum::<f64>() / cfg.tuning.dps_window_sec).max(0.0),
        );

        // Kills from list shrinkage.
        if alive < g.last_alive { g.kills += g.last_alive - alive; }
        g.last_alive = alive;

        let elapsed = g.run_start_ts.map(|s| (ts - s) / 1000.0);
        let total_damage = g.total_damage;
        let kills = g.kills;
        g.live = Some(LiveSample {
            ts, attached: true, build: Some(build_id),
            stage_key, gold,
            total_damage: Some(total_damage),
            dps, kills, alive, party,
            elapsed_sec: elapsed,
        });
        drop(g);
        self.persist(runs_changed);
    }

    #[cfg(not(windows))]
    fn sample_once(&self) { self.fail("the live meter is Windows-only"); }

    fn fail(&self, msg: &str) {
        let mut g = self.inner.lock().unwrap();
        g.attached = false;
        g.error = Some(msg.to_string());
        g.live = None;
    }
}
