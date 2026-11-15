use std::sync::OnceLock;

use percent_encoding::percent_decode_str;
use regex::Regex;

fn doi_pattern() -> &'static Regex {
    static DOI_PATTERN: OnceLock<Regex> = OnceLock::new();
    DOI_PATTERN.get_or_init(|| {
        Regex::new(r"10(?:\.[0-9]{4,})?/[^\s]*[^\s\.,]")
            .expect("the DOI extraction pattern must compile")
    })
}

fn is_url_like(value: &str) -> bool {
    value
        .get(..value.len().min(8))
        .is_some_and(|prefix| prefix.to_ascii_lowercase().starts_with("http:"))
        || value
            .get(..value.len().min(9))
            .is_some_and(|prefix| prefix.to_ascii_lowercase().starts_with("https:"))
}

fn decode_doi_input(value: &str) -> String {
    if is_url_like(value) {
        return percent_decode_str(value)
            .decode_utf8()
            .map(|decoded| decoded.into_owned())
            .unwrap_or_else(|_| value.to_owned());
    }

    let opening = ["%3C", "%3c"]
        .into_iter()
        .filter_map(|marker| value.find(marker))
        .min();
    let closing = ["%3E", "%3e"]
        .into_iter()
        .filter_map(|marker| value.find(marker))
        .min();
    if opening.zip(closing).is_some_and(|(start, end)| start < end) {
        return value
            .replace("%3C", "<")
            .replace("%3c", "<")
            .replace("%3E", ">")
            .replace("%3e", ">");
    }
    value.to_owned()
}

fn strip_prefix_ignore_ascii_case<'a>(value: &'a str, prefix: &str) -> Option<&'a str> {
    let candidate = value.get(..prefix.len())?;
    candidate
        .eq_ignore_ascii_case(prefix)
        .then(|| &value[prefix.len()..])
}

fn dedicated_doi_candidate(value: &str) -> Option<&str> {
    let mut candidate = value.trim();
    for prefix in [
        "https://dx.doi.org/",
        "http://dx.doi.org/",
        "https://doi.org/",
        "http://doi.org/",
        "dx.doi.org/",
        "doi.org/",
        "info:doi/",
        "info:doi:",
        "doi:",
    ] {
        if let Some(stripped) = strip_prefix_ignore_ascii_case(candidate, prefix) {
            candidate = stripped.trim();
            break;
        }
    }

    let without_opening_wrapper = candidate.trim_start_matches(['(', '[', '{', '<']);
    without_opening_wrapper
        .starts_with("10")
        .then_some(candidate)
}

fn remove_dedicated_field_whitespace(value: &str) -> String {
    dedicated_doi_candidate(value)
        .filter(|candidate| candidate.chars().any(char::is_whitespace))
        .map(|candidate| {
            candidate
                .chars()
                .filter(|character| !character.is_whitespace())
                .collect()
        })
        .unwrap_or_else(|| value.to_owned())
}

/// Extract a DOI using Zotero-compatible token and trailing-delimiter rules.
///
/// Research Assistant also removes Unicode whitespace from a dedicated DOI
/// field after a recognized DOI/resolver prefix. DOI names cannot contain
/// whitespace, and this repairs line wrapping introduced while copying a DOI
/// without joining arbitrary surrounding prose.
pub(crate) fn clean_doi(value: &str) -> Option<String> {
    let decoded = decode_doi_input(value.trim());
    let prepared = remove_dedicated_field_whitespace(&decoded);
    let matched = doi_pattern().find(&prepared)?;
    let mut result = matched.as_str();

    let trailing_bracket = result.chars().last();
    let opening_bracket = match trailing_bracket {
        Some(']') => Some('['),
        Some(')') => Some('('),
        Some('}') => Some('{'),
        _ => None,
    };
    if let Some(opening_bracket) = opening_bracket {
        let before_doi = &prepared[..matched.start()];
        let last_opening = before_doi.rfind(opening_bracket);
        let last_closing = trailing_bracket.and_then(|bracket| before_doi.rfind(bracket));
        if last_opening.is_some_and(|opening| last_closing.is_none_or(|closing| opening > closing))
        {
            result = &result[..result.len() - 1];
        }
    }

    Some(result.to_owned())
}

pub(crate) fn normalized_doi(value: &str) -> String {
    clean_doi(value)
        .unwrap_or_else(|| value.trim().to_owned())
        .to_lowercase()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_plain_short_and_surrounded_dois() {
        assert_eq!(
            clean_doi("10.1088/1748-9326/11/4/048002").as_deref(),
            Some("10.1088/1748-9326/11/4/048002")
        );
        assert_eq!(clean_doi("10/aabbe").as_deref(), Some("10/aabbe"));
        assert_eq!(
            clean_doi("Foo bar 10.1088/1748-9326/11/4/048002. Foo bar").as_deref(),
            Some("10.1088/1748-9326/11/4/048002")
        );
    }

    #[test]
    fn decodes_url_encoded_doi_characters() {
        let expected = "10.1002/1096-9128(200005)12:6<375::AID-CPE480>3.0.CO;2-M";
        assert_eq!(
            clean_doi(
                "HTTPS://DOI.ORG/10.1002/1096-9128(200005)12:6%3C375::AID-CPE480%3E3.0.CO;2-M"
            )
            .as_deref(),
            Some(expected)
        );
        assert_eq!(
            clean_doi("Foo 10.1002/1096-9128(200005)12:6%3C375::AID-CPE480%3E3.0.CO;2-M. Bar")
                .as_deref(),
            Some(expected)
        );
    }

    #[test]
    fn removes_only_an_unmatched_trailing_wrapper() {
        let doi = "10.1088/1748-9326/11/4/048002";
        assert_eq!(clean_doi(&format!("Foo ({doi}) bar")).as_deref(), Some(doi));
        assert_eq!(clean_doi(&format!("Foo [{doi}] bar")).as_deref(), Some(doi));
        assert_eq!(
            clean_doi("10.1000/example(with-parentheses)").as_deref(),
            Some("10.1000/example(with-parentheses)")
        );
    }

    #[test]
    fn repairs_whitespace_inside_a_dedicated_doi_field() {
        assert_eq!(
            clean_doi("10.1007/978-3-032-33195-3_ 30").as_deref(),
            Some("10.1007/978-3-032-33195-3_30")
        );
        assert_eq!(
            clean_doi("DOI: 10.1007/978-3-032-33195-3_\n30").as_deref(),
            Some("10.1007/978-3-032-33195-3_30")
        );
        assert_eq!(
            clean_doi("https://doi.org/10.1007/978-3-032-33195-3_\u{00a0}30").as_deref(),
            Some("10.1007/978-3-032-33195-3_30")
        );
    }

    #[test]
    fn does_not_join_whitespace_in_surrounding_prose() {
        assert_eq!(
            clean_doi("See 10.1007/978-3-032-33195-3_30 for details").as_deref(),
            Some("10.1007/978-3-032-33195-3_30")
        );
        assert_eq!(clean_doi("not a DOI"), None);
    }

    #[test]
    fn normalizes_case_after_cleaning() {
        assert_eq!(
            normalized_doi("HTTPS://DOI.ORG/10.1000/Example"),
            "10.1000/example"
        );
    }
}
