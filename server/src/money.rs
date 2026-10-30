//! Electricity cost: tariff periods in config.json (`money`). Each period
//! starts on a date and has a price per kWh, optionally a cheaper night zone.
//! Cost per day = Σ over hours of grid kWh × the price for that hour; «saved»
//! is what the house used *not* from the grid, priced the same way.

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Tariff {
    /// First day this price applies, "YYYY-MM-DD".
    pub from: String,
    /// Price per kWh (day zone, or the only price).
    pub price: f64,
    /// Night-zone price; none = single-zone tariff.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub night_price: Option<f64>,
    /// Night zone, local hours: [night_from, 24) ∪ [0, night_to).
    #[serde(default = "d_nf")]
    pub night_from: u8,
    #[serde(default = "d_nt")]
    pub night_to: u8,
}
fn d_nf() -> u8 { 23 }
fn d_nt() -> u8 { 7 }

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Money {
    #[serde(default = "d_cur")]
    pub currency: String,
    #[serde(default)]
    pub tariffs: Vec<Tariff>,
}
fn d_cur() -> String { "грн".into() }

impl Default for Money {
    fn default() -> Self { Money { currency: d_cur(), tariffs: vec![] } }
}

impl Money {
    /// Price for a local day and hour, None before the first tariff.
    pub fn price(&self, day: &str, hour: u8) -> Option<f64> {
        self.zone(day, hour).map(|z| z.0)
    }

    /// (price, is the night zone) — night only for two-zone tariffs.
    pub fn zone(&self, day: &str, hour: u8) -> Option<(f64, bool)> {
        let t = self.tariffs.iter().filter(|t| t.from.as_str() <= day).max_by(|a, b| a.from.cmp(&b.from))?;
        let night = if t.night_from > t.night_to {
            hour >= t.night_from || hour < t.night_to
        } else {
            hour >= t.night_from && hour < t.night_to
        };
        Some(match (t.night_price, night) {
            (Some(n), true) => (n, true),
            _ => (t.price, false),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn periods_and_zones() {
        let m = Money {
            currency: "грн".into(),
            tariffs: vec![
                Tariff { from: "2024-06-01".into(), price: 4.32, night_price: None, night_from: 23, night_to: 7 },
                Tariff { from: "2026-11-01".into(), price: 5.0, night_price: Some(2.5), night_from: 23, night_to: 7 },
            ],
        };
        assert_eq!(m.price("2024-01-01", 12), None, "before any tariff");
        assert_eq!(m.price("2026-10-31", 2), Some(4.32), "single zone: night = day");
        assert_eq!(m.price("2026-11-01", 12), Some(5.0));
        assert_eq!(m.price("2026-11-01", 23), Some(2.5));
        assert_eq!(m.price("2026-11-01", 6), Some(2.5));
        assert_eq!(m.price("2026-11-01", 7), Some(5.0));
        assert_eq!(m.zone("2026-11-01", 1), Some((2.5, true)));
        assert_eq!(m.zone("2026-10-31", 1), Some((4.32, false)), "single-zone tariff has no night");
    }
}
