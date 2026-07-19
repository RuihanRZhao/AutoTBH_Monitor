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

/// StatType id → name (game's EStatType enum). Used to label FINAL_STATS and diff vs the original.
pub fn stat_name(id: i64) -> &'static str {
    match id {
        1 => "AttackDamage", 2 => "AttackSpeed", 3 => "CriticalChance", 4 => "CriticalDamage",
        5 => "MaxHp", 6 => "Armor", 7 => "MovementSpeed", 8 => "AreaOfEffect",
        9 => "BaseAttackCountReduction", 10 => "CooldownReduction", 11 => "SkillRangeExpansion",
        12 => "FireResistance", 13 => "ColdResistance", 14 => "LightningResistance",
        15 => "ChaosResistance", 16 => "DodgeChance", 17 => "BlockChance", 18 => "MaxDodgeChance",
        19 => "MaxBlockChance", 20 => "Multistrike", 21 => "HpLeech", 22 => "ProjectileCount",
        23 => "HpRegenPerSec", 24 => "PhysicalDamagePercent", 25 => "FireDamagePercent",
        26 => "ColdDamagePercent", 27 => "LightningDamagePercent", 28 => "ChaosDamagePercent",
        29 => "MaxFireResistance", 30 => "MaxColdResistance", 31 => "MaxLightningResistance",
        32 => "MaxChaosResistance", 33 => "AddHpPerHit", 34 => "DamageReduction",
        35 => "PhysicalDamageReduction", 36 => "FireDamageReduction", 37 => "ColdDamageReduction",
        38 => "LightningDamageReduction", 39 => "ChaosDamageReduction", 40 => "DamageAbsorption",
        41 => "DamageAddition", 42 => "PhysicalDamageAddition", 43 => "FireDamageAddition",
        44 => "ColdDamageAddition", 45 => "LightningDamageAddition", 46 => "ChaosDamageAddition",
        47 => "IncreaseExpAmount", 48 => "AdditionalExp", 49 => "CastSpeed", 50 => "SkillHealIncrease",
        51 => "SkillDurationIncrease", 52 => "AllElementalResistance", 53 => "IncreaseProjectileDamage",
        54 => "IncreaseMeleeDamage", 55 => "IncreaseAreaOfEffectDamage", 56 => "IncreaseSummonDamage",
        57 => "IncreaseProjectileSpeed", 58 => "AddHpPerKill", 59 => "AddAllSkillLevel",
        60 => "ElementalBlockChance", 61 => "ElementalDodgeChance", 62 => "MaxElementalBlockChance",
        63 => "MaxElementalDodgeChance", _ => "Unknown",
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

/// Auto-attack DPS, in the REFERENCE engine's stat units.
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

/// Auto-attack DPS from the GAME's own `FINAL_STATS`, which are already normalised
/// (AS is a multiplier, crit chance/damage are fractions) — so no divisors are applied.
///
/// `AD × AS × BASIC_ATTACK_MULT × (1 + CC × (CD − 1))`
///
/// This agrees exactly with [`auto_dps`] whenever the hero has no MULTIPLICATIVE modifiers.
/// See [`MULTIPLICATIVE_DIVISOR_NOTE`] for the case where they diverge.
pub fn auto_dps_game(
    attack_damage: f64,
    attack_speed: f64,
    crit_chance: f64,
    crit_damage: f64,
    p: &Params,
) -> f64 {
    attack_damage * attack_speed * p.basic_attack_mult * (1.0 + crit_chance * (crit_damage - 1.0))
}

/// The game and the reference engine disagree on how MULTIPLICATIVE modifiers scale.
///
/// Measured against a live process, with `FINAL_STATS` as the authority:
///   game:      `stat = (ΣFLAT/100) × (1 + ΣADDITIVE/1000) × (1 + ΣMULTIPLICATIVE/1000)`
///   reference: `stat =  ΣFLAT      × (1 + ΣADDITIVE/1000) × (1 + ΣMULTIPLICATIVE/100)`
///
/// With no MULTIPLICATIVE term the two agree exactly (game × 100 == reference), confirmed on
/// two heroes across every stat. With one they diverge by ~10× on that term — e.g. a hero with
/// AttackSpeed FLAT 140 / ADDITIVE 724 / MULTIPLICATIVE 90 reads 2.630824 in game
/// (= 1.4 × 1.724 × 1.09) but 458.584 from the reference (= 140 × 1.724 × 1.9).
///
/// We treat the game as authoritative and compute from `FINAL_STATS`; matching the reference
/// bit-for-bit here would mean reproducing an inflated number.
pub const MULTIPLICATIVE_DIVISOR_NOTE: &str =
    "game divides MULTIPLICATIVE by 1000; the reference engine divides by 100";

/// Scale factor from a game `FINAL_STATS` value to the reference engine's display units.
/// Only factors confirmed against live data are listed; anything else returns `None` rather
/// than inventing a conversion.
pub fn game_to_display_scale(stat_id: i64) -> Option<f64> {
    Some(match stat_id {
        1 | 5 | 6 => 1.0,                       // AttackDamage, MaxHp, Armor
        2 | 7 | 23 => 100.0,                    // AttackSpeed, MovementSpeed, HpRegenPerSec
        3 | 4 | 10 | 16 | 25 | 26 => 1000.0,    // CriticalChance/Damage, CDR, Dodge, Fire/ColdDamage%
        40 => 10.0,                             // DamageAbsorption
        _ => return None,
    })
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
    fn auto_dps_from_game_units_matches_reference_without_multiplicative() {
        let p = Params::default();
        // Live FINAL_STATS read from the running game (f32, hence the 1e-4 tolerance).
        // hero 401 — no MULTIPLICATIVE on any DPS stat, so it must equal the reference exactly.
        let d = auto_dps_game(44.94, 1.2421499490737915, 0.019999999552965164, 1.399999976158142, &p);
        assert!((d - 106.9107176592).abs() < 1e-4, "401 autoDps(game) = {d}");
        // hero 501 — likewise.
        let d = auto_dps_game(62.1, 1.0106500387191772, 0.12397500872612, 2.971999406814575, &p);
        assert!((d - 148.39984565830846).abs() < 1e-4, "501 autoDps(game) = {d}");
    }

    #[test]
    fn multiplicative_divisor_diverges_from_reference() {
        // hero 201 AttackSpeed: FLAT 140, ADDITIVE 724, MULTIPLICATIVE 90.
        // Game stores 2.630824 = 1.4 × 1.724 × 1.09  (MULTIPLICATIVE / 1000)
        let game_as: f64 = 1.4 * 1.724 * 1.09;
        assert!((game_as - 2.630824).abs() < 1e-5, "game AS = {game_as}");
        // Reference reports 458.584 = 140 × 1.724 × 1.9  (MULTIPLICATIVE / 100)
        let reference_as = aggregate_stat(&c(&[140.0], &[724.0], &[90.0]));
        assert!((reference_as - 458.584).abs() < 1e-9, "reference AS = {reference_as}");
        // They differ by the divisor ratio on that term — this is expected, not a regression.
        assert!(reference_as > game_as * 100.0);
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
