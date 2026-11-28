//! Match manifest generator paths and generator exemptions.

use regex::Error;
use regex::Regex;

use crate::tip_view;
use crate::tip_view::GlobWildcards;

pub(super) struct WorkspaceGlob(Regex);

impl WorkspaceGlob {
    pub(super) fn new(pattern: &str) -> Result<Self, Error> {
        let mut expression = String::from("(?s)\\A");
        expression.push_str(posix_anchor(pattern));
        let mut parts = tip_view::path_parts(pattern).peekable();
        while let Some(part) = parts.next() {
            match (part, parts.peek()) {
                ("*", None) => expression.push_str("[^/]+"),
                ("*", Some(_)) => expression.push_str("[^/]+/"),
                ("**", None) => expression.push_str(".*"),
                ("**", Some(&"**")) => {},
                ("**", Some(_)) => expression.push_str("(?:.+/)?"),
                (part, next) => {
                    tip_view::push_fnmatch(&mut expression, part, GlobWildcards::PathComponent);
                    if next.is_some() {
                        expression.push('/');
                    }
                },
            }
        }
        expression.push_str("\\z");
        Regex::new(&expression).map(Self)
    }

    pub(super) fn matches(&self, path: &str) -> bool {
        self.0.is_match(if path == "." { "" } else { path })
    }
}

pub(super) fn posix_anchor(path: &str) -> &'static str {
    if path.starts_with("//") && !path.starts_with("///") {
        "//"
    } else if path.starts_with('/') {
        "/"
    } else {
        ""
    }
}

pub(super) fn is_exempt(path: &str, exemptions: &[String]) -> bool {
    let rooted = format!("/{path}");
    exemptions
        .iter()
        .map(|entry| entry.trim_matches('/'))
        .any(|entry| {
            !entry.is_empty()
                && (path == entry
                    || rooted.ends_with(&format!("/{entry}"))
                    || rooted.contains(&format!("/{entry}/")))
        })
}
