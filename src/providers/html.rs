//! Shared constant-selector cache and HTML text helpers.
use scraper::{ElementRef, Selector};
use std::collections::HashMap;
pub(crate) fn selector(value: &'static str) -> Selector {
    // Only internal constant selectors enter this cache. Each parsing thread reuses
    // compiled selectors without contending with other provider workers.
    thread_local! {
        static SELECTORS: std::cell::RefCell<HashMap<&'static str, Selector>> =
            std::cell::RefCell::new(HashMap::new());
    }
    SELECTORS.with(|selectors| {
        selectors
            .borrow_mut()
            .entry(value)
            .or_insert_with(|| Selector::parse(value).expect("static selector is valid"))
            .clone()
    })
}

pub(crate) fn element_text(element: ElementRef<'_>, separator: &str) -> String {
    element
        .text()
        .map(str::trim)
        .filter(|text| !text.is_empty())
        .collect::<Vec<_>>()
        .join(separator)
}
