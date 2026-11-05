//! The dedicated web-search rows.

use crate::*;

/// The complete row list for the dedicated web-search mode.
///
/// Kept outside `Ui` so the empty, one-character, alias and address cases can
/// be tested without opening an X connection.
pub(crate) fn web_rows(query: &str, catalog: &websearch::Catalog) -> Vec<Item> {
    let trimmed = query.trim();
    if catalog.sources.is_empty() {
        return vec![Item {
            key: "search:none".into(),
            group: "Search".into(),
            name: "No search engines are configured".into(),
            detail: "Add [[search.sources]] to ~/.config/nirulauncher/config.toml".into(),
            keywords: String::new(),
            favorite: false,
            target: Target::Info,
        }];
    }
    if trimmed.is_empty() {
        return catalog
            .sources
            .iter()
            .map(websearch::provider_item)
            .collect();
    }
    // Keep a trailing space so `gh ` is an empty prefixed query and does not
    // fall back to searching for the word "gh".
    if let Some((source, rest)) = catalog.prefixed(query.trim_start()) {
        if rest.is_empty() {
            return vec![websearch::provider_item(source)];
        }
        return websearch::item(source, &rest).into_iter().collect();
    }
    let mut rows = Vec::new();
    if let Some(url) = websearch::web_address(trimmed) {
        rows.push(websearch::address_item(&url));
    }
    for source in &catalog.sources {
        if let Some(item) = websearch::item(source, trimmed) {
            rows.push(item);
        }
    }
    rows
}

#[cfg(test)]
mod web_rows_tests {
    #[test]
    fn an_empty_search_lists_only_providers() {
        let catalog = crate::websearch::catalog(&[], &[], None, &[]);
        let rows = crate::web_rows("", &catalog);
        assert_eq!(rows.len(), catalog.sources.len());
        assert!(rows
            .iter()
            .all(|item| item.group == "Search" && item.target_kind() == "info"));
        assert!(!rows.iter().any(|item| item.group == "Applications"));
    }

    /// One character is a search; Swedish letters need no extra prefix.
    #[test]
    fn one_character_and_swedish_letters_are_web_searches() {
        let catalog = crate::websearch::catalog(&[], &[], None, &[]);
        for text in ["C", "R", "X", "å", "ö"] {
            let rows = crate::web_rows(text, &catalog);
            assert_eq!(rows.len(), catalog.sources.len(), "{text}");
            assert!(
                rows.iter().all(|item| item.target_kind() == "spawn"),
                "{text} must open an engine"
            );
            assert!(
                rows.iter().all(|item| item.name.ends_with(text)),
                "{text} must be in every row"
            );
        }
    }

    #[test]
    fn an_address_is_the_first_row() {
        let catalog = crate::websearch::catalog(&[], &[], None, &[]);
        let rows = crate::web_rows("https://example.com/a?q=1", &catalog);
        assert_eq!(rows[0].key, "search:address");
        assert_eq!(rows[0].name, "Open address");
        assert!(rows[1..].iter().all(|item| item.group == "Search"));
    }

    #[test]
    fn an_alias_selects_one_provider_without_the_prefix() {
        let catalog = crate::websearch::catalog(&[], &[], None, &[]);
        let rows = crate::web_rows("gh niruwm", &catalog);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].name, "GitHub: niruwm");
        assert!(!rows[0].name.contains("gh "));
    }

    #[test]
    fn an_empty_prefixed_query_does_not_execute() {
        let catalog = crate::websearch::catalog(&[], &[], None, &[]);
        let rows = crate::web_rows("gh ", &catalog);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].target_kind(), "info");
        assert_eq!(rows[0].name, "GitHub");
    }

    #[test]
    fn a_provider_name_is_not_stolen_as_an_alias() {
        let catalog = crate::websearch::catalog(&[], &[], None, &[]);
        let rows = crate::web_rows("github niruwm", &catalog);
        assert_eq!(
            rows.len(),
            catalog.sources.len(),
            "a full name is an ordinary word"
        );
        assert!(rows.iter().all(|item| item.name.ends_with("github niruwm")));
    }
}
