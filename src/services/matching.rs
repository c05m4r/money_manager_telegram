// Copyright (C) 2026 Marcos Gabriel Miller
use unicode_normalization::{UnicodeNormalization, char::is_combining_mark};

/// Lowercase, without accents and surrounding spaces: "Café " → "cafe".
pub fn normalize(text: &str) -> String {
    text.trim()
        .nfd()
        .filter(|c| !is_combining_mark(*c))
        .collect::<String>()
        .to_lowercase()
}

#[derive(Debug, PartialEq)]
pub enum Match<'a, T> {
    One(&'a T),
    /// Several prefix matches; the caller should ask which one.
    Ambiguous(Vec<&'a T>),
    None,
}

/// Exact normalized match first, then a unique prefix match.
pub fn find_by_name<'a, T>(items: &'a [T], query: &str, name: impl Fn(&T) -> &str) -> Match<'a, T> {
    let query = normalize(query);
    if query.is_empty() {
        return Match::None;
    }
    if let Some(item) = items.iter().find(|item| normalize(name(item)) == query) {
        return Match::One(item);
    }
    let prefixed: Vec<&T> = items
        .iter()
        .filter(|item| normalize(name(item)).starts_with(&query))
        .collect();
    match prefixed.len() {
        0 => Match::None,
        1 => Match::One(prefixed[0]),
        _ => Match::Ambiguous(prefixed),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes_accents_and_case() {
        assert_eq!(normalize("  Café Ñandú "), "cafe nandu");
    }

    #[test]
    fn exact_then_prefix() {
        #[derive(Debug, PartialEq)]
        struct Item(&'static str);
        fn by(item: &Item) -> &str {
            item.0
        }
        let items = ["Coffee", "Cashback", "Clothes", "Gift", "Gifts"].map(Item);
        let found = |query| match find_by_name(&items, query, by) {
            Match::One(item) => vec![item.0],
            Match::Ambiguous(items) => items.iter().map(|item| item.0).collect(),
            Match::None => vec![],
        };
        assert_eq!(found("coffee"), ["Coffee"]);
        assert_eq!(found("cof"), ["Coffee"]);
        assert_eq!(found("gift"), ["Gift"]);
        assert!(matches!(find_by_name(&items, "c", by), Match::Ambiguous(_)));
        assert_eq!(found("c"), ["Coffee", "Cashback", "Clothes"]);
        assert!(found("zzz").is_empty());
        assert!(found(" ").is_empty());
    }
}
