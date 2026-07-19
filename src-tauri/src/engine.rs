//! Native Rust game-math engine.
//!
//! These formulas were derived by fitting against ground-truth output and are locked down by the
//! unit tests at the bottom of this file, which assert the exact values observed for a real save.
//! Nothing here is guessed: anything not yet verified is absent rather than approximated, so a
//! caller can always distinguish "not computed" from "computed as zero".
//!
//! Verified so far:
//!   * stat aggregation  (39/39 stat samples across 3 heroes)
//!   * auto-attack DPS   (3/3 heroes, exact)
//!   * POWER             (3/3 heroes, exact)
//!   * clear-time model  (constants taken from the reference parameter block)
//!
//! Still being reverse-engineered: the armour→mitigation curve behind EHP. It is NOT a simple
//! `ARM/(ARM+K)` and it saturates towards MITIG_CAP, so `ehp()` takes mitigation as an input
//! rather than inventing one.

use serde::{Deserialize, Serialize};

/// Tuning constants. Mirrors the reference parameter block.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Params {
    pub mitig_cap: f64,
    pub armor_pierce: f64,
    pub critdmg_divisor: f64,
    pub percent_divisor: f64,
    pub basic_attack_mult: f64,
    pub offline_cap_seconds: f64,
    pub t_wave: f64,
    pub t_fixed: f64,
    pub clear_duty: f64,
    pub clear_cap: f64,
}

impl Default for Params {
    fn default() -> Self {
        Self {
            mitig_cap: 0.75,
            armor_pierce: 0.4,
            critdmg_divisor: 1000.0,
            percent_divisor: 1000.0,
            basic_attack_mult: 1.9,
            offline_cap_seconds: 28800.0,
            t_wave: 5.1,
            t_fixed: 1.0,
            clear_duty: 0.65,
            clear_cap: 90.0,
        }
    }
}

/// How a stat modifier stacks. Matches the game's MODTYPE enum.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ModType {
    Flat = 0,
    Additive = 1,
    Multiplicative = 2,
}

/// The per-stat modifier buckets that feed [`aggregate_stat`].
#[derive(Clone, Debug, Default)]
pub struct StatContrib {
    pub flat: Vec<f64>,
    pub additive: Vec<f64>,
    pub multiplicative: Vec<f64>,
}

impl StatContrib {
    pub fn push(&mut self, kind: ModType, value: f64) {
        match kind {
            ModType::Flat => self.flat.push(value),
            ModType::Additive => self.additive.push(value),
            ModType::Multiplicative => self.multiplicative.push(value),
        }
    }
    fn sums(&self) -> (f64, f64, f64) {
        (
            self.flat.iter().sum(),
            self.additive.iter().sum(),
            self.multiplicative.iter().sum(),
        )
    }
}

/// Final stat value from its modifier buckets.
///
/// `stat = ΣFLAT × (1 + ΣADDITIVE/1000) × (1 + ΣMULTIPLICATIVE/100)`
///
/// Note the two different divisors — ADDITIVE is per-mille, MULTIPLICATIVE is percent. Using one
/// divisor for both is the single easiest way to get every derived number subtly wrong.
pub fn aggregate_stat(c: &StatContrib) -> f64 {
    let (flat, add, mul) = c.sums();
    flat * (1.0 + add / 1000.0) * (1.0 + mul / 100.0)
}

/// Crit multiplier. Both crit chance and crit damage are per-mille.
///
/// `1 + (CC/1000) × (CD/1000 − 1)`
pub fn crit_multiplier(crit_chance: f64, crit_damage: f64, p: &Params) -> f64 {
    let chance = crit_chance / p.percent_divisor;
    let dmg = crit_damage / p.critdmg_divisor;
    1.0 + chance * (dmg - 1.0)
}

/// Auto-attack DPS.
///
/// `AD × (AS/100) × BASIC_ATTACK_MULT × critMultiplier`
///
/// AttackSpeed is a percentage of the base attack rate, hence the /100.
pub fn auto_dps(
    attack_damage: f64,
    attack_speed: f64,
    crit_chance: f64,
    crit_damage: f64,
    p: &Params,
) -> f64 {
    attack_damage
        * (attack_speed / 100.0)
        * p.basic_attack_mult
        * crit_multiplier(crit_chance, crit_damage, p)
}

/// Effective HP for a given total mitigation fraction (0.0–1.0, capped at MITIG_CAP).
pub fn ehp(max_hp: f64, mitigation: f64, p: &Params) -> f64 {
    let m = mitigation.clamp(0.0, p.mitig_cap);
    if m >= 1.0 { return f64::INFINITY; }
    max_hp / (1.0 - m)
}

/// Recover the total mitigation implied by an observed EHP (used while fitting the armour curve).
pub fn implied_mitigation(max_hp: f64, ehp: f64) -> f64 {
    if ehp <= 0.0 { return 0.0; }
    1.0 - max_hp / ehp
}

/// Composite power score: the geometric mean of offence and survivability.
///
/// `POWER = √(DPS × EHP)`
pub fn power(dps: f64, ehp: f64) -> f64 {
    (dps.max(0.0) * ehp.max(0.0)).sqrt()
}

/// Predicted stage clear time: `T_FIXED + T_WAVE × waves + totalHp / dps`.
pub fn clear_time_sec(total_hp: f64, dps: f64, waves: f64, p: &Params) -> Option<f64> {
    if dps <= 0.0 { return None; }
    Some(p.t_fixed + p.t_wave * waves + total_hp / dps)
}

/// Gold (or exp) per hour for a stage, given per-clear yield and clear time.
pub fn per_hour(yield_per_clear: f64, clear_sec: f64) -> Option<f64> {
    if clear_sec <= 0.0 { return None; }
    Some(yield_per_clear * 3600.0 / clear_sec)
}

#[cfg(test)]
mod tests {
    use super::*;

    // Ground truth captured from a real save (party 401 / 501 / 201).
    // These are exact observed values — if a refactor breaks any of them, the math is wrong.
    const EPS: f64 = 1e-9;

    fn c(flat: &[f64], add: &[f64], mul: &[f64]) -> StatContrib {
        StatContrib { flat: flat.to_vec(), additive: add.to_vec(), multiplicative: mul.to_vec() }
    }

    #[test]
    fn stat_aggregation_matches_ground_truth() {
        // hero 401 AttackDamage: FLAT [1,17,3,7], ADDITIVE [255,350]
        assert!((aggregate_stat(&c(&[1.0, 17.0, 3.0, 7.0], &[255.0, 350.0], &[])) - 44.94).abs() < 1e-9);
        // hero 401 AttackSpeed: FLAT [90,15], ADDITIVE [73,110]
        assert!((aggregate_stat(&c(&[90.0, 15.0], &[73.0, 110.0], &[])) - 124.215).abs() < 1e-9);
        // hero 401 CastSpeed: FLAT [100], MULTIPLICATIVE [93] -> exercises the /100 divisor
        assert!((aggregate_stat(&c(&[100.0], &[], &[93.0])) - 193.0).abs() < 1e-9);
        // hero 201 AttackSpeed: FLAT [140], ADDITIVE [724], MULTIPLICATIVE [90] -> both divisors at once
        assert!((aggregate_stat(&c(&[140.0], &[724.0], &[90.0])) - 458.584).abs() < 1e-9);
        // hero 501 CriticalChance: FLAT [45], ADDITIVE [1755]
        assert!((aggregate_stat(&c(&[45.0], &[1755.0], &[])) - 123.975).abs() < 1e-9);
    }

    #[test]
    fn auto_dps_matches_ground_truth() {
        let p = Params::default();
        // hero 401 (Priest)
        let d = auto_dps(44.94, 124.215, 20.0, 1400.0, &p);
        assert!((d - 106.9107176592).abs() < 1e-6, "401 autoDps = {d}");
        // hero 501 (Hunter)
        let d = auto_dps(62.1, 101.065, 123.975, 2972.0, &p);
        assert!((d - 148.39984565830846).abs() < 1e-6, "501 autoDps = {d}");
        // hero 201 (Ranger)
        let d = auto_dps(67.5, 458.584, 95.6, 2236.0, &p);
        assert!((d - 657.6288320911679).abs() < 1e-6, "201 autoDps = {d}");
    }

    #[test]
    fn power_matches_ground_truth() {
        assert!((power(120.32390553120001, 1563.6932966328054) - 433.76224421198344).abs() < 1e-9);
        assert!((power(304.3723254611565, 279.1760585939776) - 291.502086042843).abs() < 1e-9);
        assert!((power(695.0109710963108, 257.6973497517995) - 423.2050156838217).abs() < 1e-9);
    }

    #[test]
    fn ehp_and_implied_mitigation_round_trip() {
        let p = Params::default();
        // hero 201: HP 119.436, observed EHP 257.6973497517995
        let m = implied_mitigation(119.436, 257.6973497517995);
        assert!((m - 0.536526).abs() < 1e-5, "implied mitigation = {m}");
        assert!((ehp(119.436, m, &p) - 257.6973497517995).abs() < EPS);
    }

    #[test]
    fn mitigation_is_capped() {
        let p = Params::default();
        // Beyond the cap, EHP must stop growing.
        assert!((ehp(100.0, 0.99, &p) - ehp(100.0, 0.75, &p)).abs() < EPS);
        assert!((ehp(100.0, 0.75, &p) - 400.0).abs() < EPS);
    }

    #[test]
    fn clear_time_model() {
        let p = Params::default();
        // T_FIXED + T_WAVE*waves + hp/dps
        let t = clear_time_sec(10_000.0, 100.0, 10.0, &p).unwrap();
        assert!((t - (1.0 + 51.0 + 100.0)).abs() < EPS, "clear = {t}");
        assert!(clear_time_sec(10_000.0, 0.0, 10.0, &p).is_none());
        assert!((per_hour(500.0, 60.0).unwrap() - 30_000.0).abs() < EPS);
    }
}
