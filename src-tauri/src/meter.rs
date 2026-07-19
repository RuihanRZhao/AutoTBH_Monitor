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
    #[serde(default)]
    pub game: HashMap<String, HashMap<String, i64>>,
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
    run_stage: Option<i64>,
}

#[derive(Clone)]
pub struct Meter {
    pub inner: Arc<Mutex<MeterInner>>,
    pub data_dir: PathBuf,
}

impl Meter {
    pub fn new(data_dir: PathBuf) -> Self {
        let m = Self { inner: Arc::new(Mutex::new(MeterInner::default())), data_dir };
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

    fn persist(&self) {
        let g = self.inner.lock().unwrap();
        if let Ok(s) = serde_json::to_string(&g.runs) { Self::write_atomic(&self.runs_path(), &s); }
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
        self.persist();
        json!({ "ok": true, "archived": total, "total": total, "archive": ts.to_string() })
    }

    pub fn set_enabled(&self, on: bool) { self.inner.lock().unwrap().enabled = on; }

    pub fn offsets(&self) -> Result<Offsets, String> {
        let p = self.offsets_path();
        let txt = std::fs::read_to_string(&p)
            .map_err(|e| format!("cannot read {}: {e}", p.display()))?;
        serde_json::from_str(&txt).map_err(|e| format!("cannot parse {}: {e}", p.display()))
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

        let g_off = |cls: &str, field: &str| -> usize {
            cfg.game.get(cls).and_then(|m| m.get(field)).cloned().unwrap_or(0) as usize
        };
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
                                        gold = Some(v);
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

        if damage_this_tick > 0.0 {
            g.total_damage += damage_this_tick;
            g.window.push_back((ts, damage_this_tick));
        }
        let win_ms = cfg.tuning.dps_window_sec * 1000.0;
        while let Some(&(t0, _)) = g.window.front() {
            if ts - t0 > win_ms { g.window.pop_front(); } else { break; }
        }
        // Upstream semantics: fixed divisor (ramps up over the first window).
        let dps = Some(g.window.iter().map(|(_, d)| *d).sum::<f64>() / cfg.tuning.dps_window_sec);

        // Kills from list shrinkage.
        if alive < g.last_alive { g.kills += g.last_alive - alive; }
        g.last_alive = alive;

        // Run lifecycle: open on first monsters, close when the stage clears out or changes.
        let in_combat = alive > 0;
        let stage_changed = stage_key.is_some() && g.run_stage.is_some() && stage_key != g.run_stage;
        if in_combat && g.run_start_ts.is_none() {
            g.run_start_ts = Some(ts);
            g.run_start_gold = gold;
            g.run_stage = stage_key;
            g.total_damage = 0.0;
            g.kills = 0;
        } else if g.run_start_ts.is_some() && (!in_combat || stage_changed) {
            let start = g.run_start_ts.take().unwrap();
            let g0 = g.run_start_gold.take();
            let stage = g.run_stage.take();
            let clear = (ts - start) / 1000.0;
            let total_damage = g.total_damage;
            // Only record a meaningful attempt.
            if clear >= 3.0 && total_damage > 0.0 {
                let rec = RunRecord {
                    ts,
                    outcome: "success".into(),
                    stage_key: stage,
                    difficulty: None,
                    clear_time: Some(clear),
                    total_damage: Some(total_damage),
                    // never emit 0 for an unread value
                    gold: match (gold, g0) { (Some(a), Some(b)) if a >= b => Some((a - b) as f64), _ => None },
                    xp: None,
                };
                g.runs.push(rec);
                if g.runs.len() > 2000 { let n = g.runs.len() - 2000; g.runs.drain(0..n); }
            }
            g.total_damage = 0.0;
            g.kills = 0;
            g.window.clear();
        }
        if stage_key.is_some() && g.run_start_ts.is_some() && g.run_stage.is_none() {
            g.run_stage = stage_key;
        }

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
        self.persist();
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
