//! Steam Community Market currency table + FX helpers.
//! Port of the original `currency.mjs`. Steam `currency=N` codes (ECurrencyCode).

use serde_json::{json, Value};
use std::collections::HashMap;

#[derive(Clone, Copy)]
pub struct Currency {
    pub iso: &'static str,
    pub symbol: &'static str,
    pub decimals: u32,
    pub name: &'static str,
    pub country: &'static str,
}

/// The 41 Steam market currencies, keyed by Steam currency code.
pub fn currencies() -> &'static [(u32, Currency)] {
    &[
        (1,  Currency { iso: "USD", symbol: "$",    decimals: 2, name: "US Dollar",          country: "US" }),
        (2,  Currency { iso: "GBP", symbol: "£",    decimals: 2, name: "British Pound",       country: "GB" }),
        (3,  Currency { iso: "EUR", symbol: "€",    decimals: 2, name: "Euro",                country: "DE" }),
        (4,  Currency { iso: "CHF", symbol: "CHF",  decimals: 2, name: "Swiss Franc",         country: "CH" }),
        (5,  Currency { iso: "RUB", symbol: "₽",    decimals: 2, name: "Russian Ruble",       country: "RU" }),
        (6,  Currency { iso: "PLN", symbol: "zł",   decimals: 2, name: "Polish Zloty",        country: "PL" }),
        (7,  Currency { iso: "BRL", symbol: "R$",   decimals: 2, name: "Brazilian Real",      country: "BR" }),
        (8,  Currency { iso: "JPY", symbol: "¥",    decimals: 0, name: "Japanese Yen",        country: "JP" }),
        (9,  Currency { iso: "NOK", symbol: "kr",   decimals: 2, name: "Norwegian Krone",     country: "NO" }),
        (10, Currency { iso: "IDR", symbol: "Rp",   decimals: 0, name: "Indonesian Rupiah",   country: "ID" }),
        (11, Currency { iso: "MYR", symbol: "RM",   decimals: 2, name: "Malaysian Ringgit",   country: "MY" }),
        (12, Currency { iso: "PHP", symbol: "₱",    decimals: 2, name: "Philippine Peso",     country: "PH" }),
        (13, Currency { iso: "SGD", symbol: "S$",   decimals: 2, name: "Singapore Dollar",    country: "SG" }),
        (14, Currency { iso: "THB", symbol: "฿",    decimals: 2, name: "Thai Baht",           country: "TH" }),
        (15, Currency { iso: "VND", symbol: "₫",    decimals: 0, name: "Vietnamese Dong",     country: "VN" }),
        (16, Currency { iso: "KRW", symbol: "₩",    decimals: 0, name: "South Korean Won",    country: "KR" }),
        (17, Currency { iso: "TRY", symbol: "₺",    decimals: 2, name: "Turkish Lira",        country: "TR" }),
        (18, Currency { iso: "UAH", symbol: "₴",    decimals: 2, name: "Ukrainian Hryvnia",   country: "UA" }),
        (19, Currency { iso: "MXN", symbol: "Mex$", decimals: 2, name: "Mexican Peso",        country: "MX" }),
        (20, Currency { iso: "CAD", symbol: "CDN$", decimals: 2, name: "Canadian Dollar",     country: "CA" }),
        (21, Currency { iso: "AUD", symbol: "A$",   decimals: 2, name: "Australian Dollar",   country: "AU" }),
        (22, Currency { iso: "NZD", symbol: "NZ$",  decimals: 2, name: "New Zealand Dollar",  country: "NZ" }),
        (23, Currency { iso: "CNY", symbol: "¥",    decimals: 2, name: "Chinese Yuan",        country: "CN" }),
        (24, Currency { iso: "INR", symbol: "₹",    decimals: 2, name: "Indian Rupee",        country: "IN" }),
        (25, Currency { iso: "CLP", symbol: "CLP$", decimals: 0, name: "Chilean Peso",        country: "CL" }),
        (26, Currency { iso: "PEN", symbol: "S/",   decimals: 2, name: "Peruvian Sol",        country: "PE" }),
        (27, Currency { iso: "COP", symbol: "COL$", decimals: 2, name: "Colombian Peso",      country: "CO" }),
        (28, Currency { iso: "ZAR", symbol: "R",    decimals: 2, name: "South African Rand",  country: "ZA" }),
        (29, Currency { iso: "HKD", symbol: "HK$",  decimals: 2, name: "Hong Kong Dollar",    country: "HK" }),
        (30, Currency { iso: "TWD", symbol: "NT$",  decimals: 2, name: "Taiwan Dollar",       country: "TW" }),
        (31, Currency { iso: "SAR", symbol: "SR",   decimals: 2, name: "Saudi Riyal",         country: "SA" }),
        (32, Currency { iso: "AED", symbol: "AED",  decimals: 2, name: "UAE Dirham",          country: "AE" }),
        (33, Currency { iso: "SEK", symbol: "kr",   decimals: 2, name: "Swedish Krona",       country: "SE" }),
        (34, Currency { iso: "ARS", symbol: "ARS$", decimals: 2, name: "Argentine Peso",      country: "AR" }),
        (35, Currency { iso: "ILS", symbol: "₪",    decimals: 2, name: "Israeli Shekel",      country: "IL" }),
        (36, Currency { iso: "BYN", symbol: "Br",   decimals: 2, name: "Belarusian Ruble",    country: "BY" }),
        (37, Currency { iso: "KZT", symbol: "₸",    decimals: 2, name: "Kazakhstani Tenge",   country: "KZ" }),
        (38, Currency { iso: "KWD", symbol: "KD",   decimals: 3, name: "Kuwaiti Dinar",       country: "KW" }),
        (39, Currency { iso: "QAR", symbol: "QR",   decimals: 2, name: "Qatari Riyal",        country: "QA" }),
        (40, Currency { iso: "CRC", symbol: "₡",    decimals: 2, name: "Costa Rican Colón",   country: "CR" }),
        (41, Currency { iso: "UYU", symbol: "$U",   decimals: 2, name: "Uruguayan Peso",      country: "UY" }),
    ]
}

pub fn get(code: u32) -> Option<Currency> {
    currencies().iter().find(|(c, _)| *c == code).map(|(_, v)| *v)
}

pub fn iso_for(code: u32) -> Option<&'static str> {
    get(code).map(|c| c.iso)
}

/// Full currency list as JSON (for `/api/currency`).
pub fn list_json() -> Value {
    Value::Array(
        currencies()
            .iter()
            .map(|(code, c)| json!({ "code": code, "iso": c.iso, "name": c.name, "symbol": c.symbol }))
            .collect(),
    )
}

pub fn info_json(code: u32) -> Value {
    match get(code) {
        Some(c) => json!({
            "code": code, "iso": c.iso, "symbol": c.symbol,
            "decimals": c.decimals, "name": c.name, "country": c.country
        }),
        None => Value::Null,
    }
}

/// Steam `sell_price` (minor unit) → internal main×100 convention. Port of `sellPriceToCents`.
pub fn sell_price_to_cents(sell_price: f64, decimals: i32) -> i64 {
    let d = if decimals.is_negative() { 2 } else { decimals };
    let v = (sell_price * 10f64.powi(2 - d)).round() as i64;
    v.max(1)
}

/// Offline USD→X fallback rates (local units per 1 USD). Port of `FX_FALLBACK`.
pub fn fx_fallback() -> &'static [(&'static str, f64)] {
    &[
        ("USD", 1.0), ("EUR", 0.92), ("GBP", 0.79), ("CHF", 0.88), ("RUB", 92.0), ("PLN", 4.0),
        ("BRL", 5.4), ("JPY", 156.0), ("NOK", 10.7), ("IDR", 16200.0), ("MYR", 4.7), ("PHP", 58.0),
        ("SGD", 1.35), ("THB", 36.0), ("VND", 25400.0), ("KRW", 1370.0), ("TRY", 32.0), ("UAH", 40.0),
        ("MXN", 18.0), ("CAD", 1.37), ("AUD", 1.52), ("NZD", 1.65), ("CNY", 7.2), ("INR", 83.0),
        ("CLP", 940.0), ("PEN", 3.8), ("COP", 4000.0), ("ZAR", 18.5), ("HKD", 7.8), ("TWD", 32.0),
        ("SAR", 3.75), ("AED", 3.67), ("SEK", 10.5), ("ARS", 900.0), ("ILS", 3.7), ("BYN", 3.3),
        ("KZT", 470.0), ("KWD", 0.31), ("QAR", 3.64), ("CRC", 510.0), ("UYU", 39.0),
    ]
}

/// Frankfurter `{ rates: {..} }` → ISO→rate map with USD:1 injected. Port of `parseFrankfurter`.
pub fn parse_frankfurter(json: &Value) -> Option<HashMap<String, f64>> {
    let rates = json.get("rates")?.as_object()?;
    let mut out = HashMap::new();
    out.insert("USD".to_string(), 1.0);
    for (k, v) in rates {
        if let Some(n) = v.as_f64() {
            if n.is_finite() && n > 0.0 {
                out.insert(k.clone(), n);
            }
        }
    }
    if out.len() > 1 {
        Some(out)
    } else {
        None
    }
}

/// Local units per 1 USD for an ISO code, from a live map with fallback. Port of `rateForIso`.
pub fn rate_for_iso(rates: Option<&HashMap<String, f64>>, iso: &str) -> Option<f64> {
    if let Some(r) = rates {
        if let Some(v) = r.get(iso) {
            if v.is_finite() && *v > 0.0 {
                return Some(*v);
            }
        }
    }
    fx_fallback()
        .iter()
        .find(|(k, _)| *k == iso)
        .map(|(_, v)| *v)
        .filter(|v| v.is_finite() && *v > 0.0)
}
