// Copyright (C) 2026 Marcos Gabriel Miller
//! Pure parsers for user input. No I/O, fully unit-tested.
use chrono::{DateTime, Datelike, Duration, NaiveDate, NaiveTime, TimeZone, Utc};
use chrono_tz::Tz;
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use std::str::FromStr;

pub const INVALID_AMOUNT: &str = "Monto inválido. Ejemplos: 1500, 1500,50, 1.500,50";

/// Parses a positive amount with at most 2 decimals, accepting `.` or `,` as decimal or
/// thousands separator (see `design.md` §9.2).
pub fn parse_amount(input: &str) -> Result<Decimal, String> {
    let text = input.trim().trim_start_matches('$').trim();
    if text.is_empty()
        || !text
            .chars()
            .all(|c| c.is_ascii_digit() || c == '.' || c == ',')
    {
        return Err(INVALID_AMOUNT.into());
    }

    let last_dot = text.rfind('.');
    let last_comma = text.rfind(',');
    let normalized = match (last_dot, last_comma) {
        (Some(dot), Some(comma)) => {
            // Both present: the last one is the decimal separator.
            let (decimal, thousands) = if dot > comma { ('.', ',') } else { (',', '.') };
            let (int_part, frac_part) = text.rsplit_once(decimal).expect("separator present");
            if int_part.contains(decimal) || !valid_thousands(int_part, thousands) {
                return Err(INVALID_AMOUNT.into());
            }
            format!("{}.{}", int_part.replace(thousands, ""), frac_part)
        }
        (Some(_), None) | (None, Some(_)) => {
            let separator = if last_dot.is_some() { '.' } else { ',' };
            let parts: Vec<&str> = text.split(separator).collect();
            if parts.len() == 2 && (1..=2).contains(&parts[1].len()) {
                format!("{}.{}", parts[0], parts[1])
            } else if parts.iter().skip(1).all(|part| part.len() == 3)
                && valid_thousands(text, separator)
            {
                text.replace(separator, "")
            } else {
                return Err(INVALID_AMOUNT.into());
            }
        }
        (None, None) => text.to_string(),
    };

    let amount = Decimal::from_str(&normalized).map_err(|_| INVALID_AMOUNT.to_string())?;
    if amount <= Decimal::ZERO || amount.scale() > 2 {
        return Err(INVALID_AMOUNT.into());
    }
    Ok(amount.normalize())
}

/// `1.234.567` style grouping: first group 1–3 digits without a leading zero, the rest exactly 3.
fn valid_thousands(text: &str, separator: char) -> bool {
    let mut groups = text.split(separator);
    let first = groups.next().unwrap_or_default();
    (1..=3).contains(&first.len())
        && !first.starts_with('0')
        && groups.all(|group| group.len() == 3)
}

/// `today`/`hoy`, `yesterday`/`ayer`, `dd/mm` (current year) or `dd/mm/yyyy`.
pub fn parse_date(token: &str, today: NaiveDate) -> Option<NaiveDate> {
    match token.trim().to_lowercase().as_str() {
        "today" | "hoy" => return Some(today),
        "yesterday" | "ayer" => return Some(today - Duration::days(1)),
        _ => {}
    }
    let parts: Vec<&str> = token.trim().split('/').collect();
    let (day, month, year) = match parts.as_slice() {
        [day, month] => (day.parse().ok()?, month.parse().ok()?, today.year()),
        [day, month, year] if year.len() == 4 => {
            (day.parse().ok()?, month.parse().ok()?, year.parse().ok()?)
        }
        [day, month, year] if year.len() == 2 => (
            day.parse().ok()?,
            month.parse().ok()?,
            2000 + year.parse::<i32>().ok()?,
        ),
        _ => return None,
    };
    NaiveDate::from_ymd_opt(year, month, day)
}

/// `mm/yyyy` → (year, month).
pub fn parse_month(token: &str) -> Option<(i32, u32)> {
    let (month, year) = token.trim().split_once('/')?;
    let month: u32 = month.parse().ok()?;
    let year: i32 = year.parse().ok()?;
    ((1..=12).contains(&month) && (2000..=2100).contains(&year)).then_some((year, month))
}

/// A date the user typed becomes 12:00 local time, so converting to UTC never changes the day.
/// No date means "now".
pub fn to_utc(date: Option<NaiveDate>, tz: Tz, now: DateTime<Utc>) -> DateTime<Utc> {
    match date {
        Some(date) if date != now.with_timezone(&tz).date_naive() => tz
            .from_local_datetime(
                &date.and_time(NaiveTime::from_hms_opt(12, 0, 0).expect("valid time")),
            )
            .single()
            .map(|local| local.with_timezone(&Utc))
            .unwrap_or(now),
        _ => now,
    }
}

/// `[start of month, start of next month)` in `tz`, as UTC instants.
pub fn month_bounds(year: i32, month: u32, tz: Tz) -> Option<(DateTime<Utc>, DateTime<Utc>)> {
    let start = NaiveDate::from_ymd_opt(year, month, 1)?;
    let next = if month == 12 {
        NaiveDate::from_ymd_opt(year + 1, 1, 1)?
    } else {
        NaiveDate::from_ymd_opt(year, month + 1, 1)?
    };
    let local = |date: NaiveDate| {
        tz.from_local_datetime(&date.and_time(NaiveTime::MIN))
            .earliest()
    };
    Some((
        local(start)?.with_timezone(&Utc),
        local(next)?.with_timezone(&Utc),
    ))
}

/// `/expense 1.500,50 ayer #coffee @visa cortado con medialunas`
#[derive(Debug, Clone, PartialEq, Default)]
pub struct QuickEntry {
    pub amount: Option<Decimal>,
    pub date: Option<NaiveDate>,
    pub category: Option<String>,
    pub account: Option<String>,
    pub description: Option<String>,
}

pub fn parse_quick_entry(args: &str, today: NaiveDate) -> Result<QuickEntry, String> {
    let mut words = args.split_whitespace().peekable();
    let mut entry = QuickEntry::default();

    let Some(first) = words.next() else {
        return Ok(entry);
    };
    entry.amount = Some(parse_amount(first)?);

    if let Some(date) = words.peek().and_then(|word| parse_date(word, today)) {
        entry.date = Some(date);
        words.next();
    }

    let mut description = Vec::new();
    for word in words {
        if let Some(name) = word.strip_prefix('#').filter(|name| !name.is_empty()) {
            entry.category = Some(name.to_string());
        } else if let Some(name) = word.strip_prefix('@').filter(|name| !name.is_empty()) {
            entry.account = Some(name.to_string());
        } else {
            description.push(word);
        }
    }
    entry.description = parse_description(&description.join(" "))?;
    Ok(entry)
}

/// Optional free text, 1–400 characters like the backend.
pub fn parse_description(text: &str) -> Result<Option<String>, String> {
    let text = text.trim();
    if text.is_empty() {
        Ok(None)
    } else if text.chars().count() > 400 {
        Err("La descripción puede tener hasta 400 caracteres.".into())
    } else {
        Ok(Some(text.to_string()))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Flow {
    Expense,
    Income,
}

/// `/transactions [@account] [#category] [expenses|income] [month:mm/yyyy] [from:dd/mm] [to:dd/mm] [text]`
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct ListFilters {
    pub account: Option<String>,
    pub category: Option<String>,
    pub flow: Option<Flow>,
    pub from: Option<NaiveDate>,
    pub to: Option<NaiveDate>,
    pub search: Option<String>,
}

pub fn parse_list_filters(args: &str, today: NaiveDate) -> Result<ListFilters, String> {
    let mut filters = ListFilters::default();
    let mut search = Vec::new();
    for word in args.split_whitespace() {
        let lower = word.to_lowercase();
        if let Some(name) = word.strip_prefix('#').filter(|name| !name.is_empty()) {
            filters.category = Some(name.to_string());
        } else if let Some(name) = word.strip_prefix('@').filter(|name| !name.is_empty()) {
            filters.account = Some(name.to_string());
        } else if matches!(lower.as_str(), "expenses" | "expense" | "gastos") {
            filters.flow = Some(Flow::Expense);
        } else if matches!(lower.as_str(), "income" | "ingresos") {
            filters.flow = Some(Flow::Income);
        } else if let Some(value) = lower.strip_prefix("month:") {
            let (year, month) = parse_month(value).ok_or("Mes inválido, usá month:mm/yyyy")?;
            filters.from = NaiveDate::from_ymd_opt(year, month, 1);
            filters.to = month_last_day(year, month);
        } else if let Some(value) = lower.strip_prefix("from:") {
            filters.from = Some(
                parse_date(value, today)
                    .ok_or("Fecha inválida en from:, usá dd/mm o dd/mm/yyyy")?,
            );
        } else if let Some(value) = lower.strip_prefix("to:") {
            filters.to = Some(
                parse_date(value, today).ok_or("Fecha inválida en to:, usá dd/mm o dd/mm/yyyy")?,
            );
        } else {
            search.push(word);
        }
    }
    if !search.is_empty() {
        filters.search = Some(search.join(" "));
    }
    Ok(filters)
}

fn month_last_day(year: i32, month: u32) -> Option<NaiveDate> {
    let next = if month == 12 {
        NaiveDate::from_ymd_opt(year + 1, 1, 1)?
    } else {
        NaiveDate::from_ymd_opt(year, month + 1, 1)?
    };
    next.pred_opt()
}

/// A list reference: `3`, `#3` or a full UUID.
#[derive(Debug, PartialEq)]
pub enum Reference {
    Position(i64),
    Uuid(uuid::Uuid),
    Name(String),
}

pub fn parse_reference(token: &str) -> Option<Reference> {
    let token = token.trim();
    if token.is_empty() {
        return None;
    }
    if let Ok(position) = token.trim_start_matches('#').parse::<i64>() {
        return (position > 0).then_some(Reference::Position(position));
    }
    if let Ok(uuid) = uuid::Uuid::parse_str(token) {
        return Some(Reference::Uuid(uuid));
    }
    Some(Reference::Name(
        token.trim_start_matches(['@', '#']).to_string(),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono_tz::America::Argentina::Buenos_Aires as BA;

    fn d(year: i32, month: u32, day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(year, month, day).unwrap()
    }

    #[test]
    fn amounts_from_design_table() {
        let ok = |input: &str, expected: &str| {
            assert_eq!(
                parse_amount(input).unwrap(),
                Decimal::from_str(expected).unwrap(),
                "{input}"
            )
        };
        ok("1500", "1500");
        ok("1500,5", "1500.5");
        ok("1.500", "1500");
        ok("1.500,50", "1500.50");
        ok("1,500.50", "1500.50");
        ok("12.5", "12.5");
        ok("0,5", "0.5");
        ok("12,05", "12.05");
        ok("1.234.567,89", "1234567.89");
        ok("$1500", "1500");
        ok("12.345", "12345");
        for bad in [
            "0", "-3", "1,234,5", "abc", "", "1.5.0", "1234.567", "1.2345", "0,001", "1500.505",
            "12,3456",
        ] {
            assert!(parse_amount(bad).is_err(), "{bad} should fail");
        }
    }

    #[test]
    fn dates() {
        let today = d(2026, 10, 8);
        assert_eq!(parse_date("hoy", today), Some(today));
        assert_eq!(parse_date("Today", today), Some(today));
        assert_eq!(parse_date("ayer", today), Some(d(2026, 10, 7)));
        assert_eq!(parse_date("yesterday", today), Some(d(2026, 10, 7)));
        assert_eq!(parse_date("01/10", today), Some(d(2026, 10, 1)));
        assert_eq!(parse_date("31/12/2025", today), Some(d(2025, 12, 31)));
        assert_eq!(parse_date("31/12/25", today), Some(d(2025, 12, 31)));
        assert_eq!(parse_date("31/02", today), None);
        assert_eq!(parse_date("cafe", today), None);
        assert_eq!(parse_date("1500", today), None);
    }

    #[test]
    fn explicit_dates_become_local_noon() {
        let now = Utc.with_ymd_and_hms(2026, 10, 8, 2, 30, 0).unwrap(); // 23:30 on 07/10 in Buenos Aires
        assert_eq!(to_utc(None, BA, now), now);
        assert_eq!(
            to_utc(Some(d(2026, 10, 7)), BA, now),
            now,
            "local today keeps the current time"
        );
        assert_eq!(
            to_utc(Some(d(2026, 10, 1)), BA, now),
            Utc.with_ymd_and_hms(2026, 10, 1, 15, 0, 0).unwrap()
        );
    }

    #[test]
    fn months() {
        assert_eq!(parse_month("10/2026"), Some((2026, 10)));
        assert_eq!(parse_month("13/2026"), None);
        assert_eq!(parse_month("abc"), None);
        let (from, to) = month_bounds(2026, 10, BA).unwrap();
        assert_eq!(from, Utc.with_ymd_and_hms(2026, 10, 1, 3, 0, 0).unwrap());
        assert_eq!(to, Utc.with_ymd_and_hms(2026, 11, 1, 3, 0, 0).unwrap());
        let (_, to) = month_bounds(2026, 12, BA).unwrap();
        assert_eq!(to, Utc.with_ymd_and_hms(2027, 1, 1, 3, 0, 0).unwrap());
    }

    #[test]
    fn quick_entry_full() {
        let today = d(2026, 10, 8);
        let entry =
            parse_quick_entry("1.500,50 ayer #coffee cortado @visa con medialunas", today).unwrap();
        assert_eq!(
            entry,
            QuickEntry {
                amount: Some(Decimal::from_str("1500.50").unwrap()),
                date: Some(d(2026, 10, 7)),
                category: Some("coffee".into()),
                account: Some("visa".into()),
                description: Some("cortado con medialunas".into()),
            }
        );
    }

    #[test]
    fn quick_entry_minimal_and_errors() {
        let today = d(2026, 10, 8);
        assert_eq!(parse_quick_entry("", today).unwrap(), QuickEntry::default());
        let entry = parse_quick_entry("500", today).unwrap();
        assert_eq!(
            (entry.amount, entry.date, entry.category),
            (Some(Decimal::from(500)), None, None)
        );
        assert!(parse_quick_entry("abc #coffee", today).is_err());
        assert!(parse_quick_entry(&format!("10 {}", "x".repeat(401)), today).is_err());
    }

    #[test]
    fn list_filters() {
        let today = d(2026, 10, 8);
        let filters =
            parse_list_filters("@visa #coffee gastos month:09/2026 super", today).unwrap();
        assert_eq!(filters.account.as_deref(), Some("visa"));
        assert_eq!(filters.category.as_deref(), Some("coffee"));
        assert_eq!(filters.flow, Some(Flow::Expense));
        assert_eq!(
            (filters.from, filters.to),
            (Some(d(2026, 9, 1)), Some(d(2026, 9, 30)))
        );
        assert_eq!(filters.search.as_deref(), Some("super"));

        let filters = parse_list_filters("income from:01/10 to:05/10/2026", today).unwrap();
        assert_eq!(filters.flow, Some(Flow::Income));
        assert_eq!(
            (filters.from, filters.to),
            (Some(d(2026, 10, 1)), Some(d(2026, 10, 5)))
        );
        assert_eq!(
            parse_list_filters("", today).unwrap(),
            ListFilters::default()
        );
        assert!(parse_list_filters("month:13/2026", today).is_err());
        assert!(parse_list_filters("from:xx", today).is_err());
    }

    #[test]
    fn references() {
        assert_eq!(parse_reference("3"), Some(Reference::Position(3)));
        assert_eq!(parse_reference("#3"), Some(Reference::Position(3)));
        assert_eq!(parse_reference("0"), None);
        assert_eq!(parse_reference(""), None);
        let uuid = uuid::Uuid::new_v4();
        assert_eq!(
            parse_reference(&uuid.to_string()),
            Some(Reference::Uuid(uuid))
        );
        assert_eq!(
            parse_reference("@visa"),
            Some(Reference::Name("visa".into()))
        );
    }
}
