// Copyright (C) 2026 Marcos Gabriel Miller
//! es-AR presentation helpers. Every user-provided string goes through `escape`.
use chrono::{DateTime, Datelike, Utc};
use chrono_tz::Tz;
use rust_decimal::{Decimal, RoundingStrategy};

pub use teloxide::utils::html::escape;

/// `1500.5` → `1.500,50`
pub fn number(value: Decimal) -> String {
    let value = value.round_dp_with_strategy(2, RoundingStrategy::MidpointAwayFromZero);
    let negative = value.is_sign_negative() && !value.is_zero();
    let text = format!("{:.2}", value.abs());
    let (int_part, frac_part) = text.split_once('.').unwrap_or((&text, "00"));

    let mut grouped = String::new();
    for (index, digit) in int_part.chars().enumerate() {
        if index > 0 && (int_part.len() - index) % 3 == 0 {
            grouped.push('.');
        }
        grouped.push(digit);
    }
    format!("{}{grouped},{frac_part}", if negative { "−" } else { "" })
}

/// `$ 1.500,50`
pub fn money(value: Decimal, symbol: &str) -> String {
    if symbol.is_empty() {
        number(value)
    } else {
        format!("{} {}", escape(symbol), number(value))
    }
}

/// Signed amount for a transaction row: `− $ 1.500,50` for expenses, `+ …` for income.
pub fn signed_money(value: Decimal, symbol: &str, sign: Option<char>) -> String {
    match sign {
        Some(sign) => format!("{sign} {}", money(value, symbol)),
        None => money(value, symbol),
    }
}

/// `08/10 14:32` in the current year, `08/10/2025` otherwise.
pub fn short_date(value: DateTime<Utc>, tz: Tz, now: DateTime<Utc>) -> String {
    let local = value.with_timezone(&tz);
    if local.year() == now.with_timezone(&tz).year() {
        local.format("%d/%m %H:%M").to_string()
    } else {
        local.format("%d/%m/%Y").to_string()
    }
}

pub fn full_date(value: DateTime<Utc>, tz: Tz) -> String {
    value
        .with_timezone(&tz)
        .format("%d/%m/%Y %H:%M")
        .to_string()
}

pub fn truncate(text: &str, max_chars: usize) -> String {
    if text.chars().count() <= max_chars {
        text.to_string()
    } else {
        format!(
            "{}…",
            text.chars()
                .take(max_chars.saturating_sub(1))
                .collect::<String>()
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;
    use chrono_tz::America::Argentina::Buenos_Aires as BA;
    use std::str::FromStr;

    fn dec(text: &str) -> Decimal {
        Decimal::from_str(text).unwrap()
    }

    #[test]
    fn numbers_es_ar() {
        assert_eq!(number(dec("1500.5")), "1.500,50");
        assert_eq!(number(dec("0")), "0,00");
        assert_eq!(number(dec("999")), "999,00");
        assert_eq!(number(dec("1234567.891")), "1.234.567,89");
        assert_eq!(number(dec("-45")), "−45,00");
        assert_eq!(money(dec("10"), "US$"), "US$ 10,00");
        assert_eq!(money(dec("10"), "<b>"), "&lt;b&gt; 10,00");
        assert_eq!(signed_money(dec("10"), "$", Some('−')), "− $ 10,00");
    }

    #[test]
    fn dates() {
        let now = Utc.with_ymd_and_hms(2026, 10, 8, 12, 0, 0).unwrap();
        let this_year = Utc.with_ymd_and_hms(2026, 10, 8, 17, 32, 0).unwrap();
        let last_year = Utc.with_ymd_and_hms(2025, 10, 8, 17, 32, 0).unwrap();
        assert_eq!(short_date(this_year, BA, now), "08/10 14:32");
        assert_eq!(short_date(last_year, BA, now), "08/10/2025");
        assert_eq!(full_date(this_year, BA), "08/10/2026 14:32");
    }

    #[test]
    fn truncation() {
        assert_eq!(truncate("cortado con medialunas", 10), "cortado c…");
        assert_eq!(truncate("corto", 10), "corto");
    }
}
