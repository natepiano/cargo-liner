/// One facade path callers must stop writing, and what they write instead.
pub(in crate::fixes) struct FacadeRedirect {
    /// The crate-relative path callers reach the item through today, e.g.
    /// `parent::Widget` for a `pub use child::Widget` in `parent`.
    pub(in crate::fixes) facade_path:      Vec<String>,
    pub(in crate::fixes) target:           RedirectTarget,
    /// Callers whose module is exactly this one keep the facade path.
    pub(in crate::fixes) unchanged_module: Option<Vec<String>>,
}

/// The crate-relative path a redirected caller writes.
pub(in crate::fixes) enum RedirectTarget {
    /// Every caller writes this path.
    Everywhere(Vec<String>),
    /// Callers in `scope` or below it write `inside`; every other caller
    /// writes `outside`.
    Scoped {
        scope:   Vec<String>,
        inside:  Vec<String>,
        outside: Vec<String>,
    },
}

/// Which of a redirect's paths a caller received.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum TargetSide {
    /// The `Everywhere` path, or a `Scoped` redirect's `inside` path.
    Inside,
    /// A `Scoped` redirect's `outside` path.
    Outside,
}

impl FacadeRedirect {
    /// The path a caller in `caller_module` writes instead of the facade path,
    /// and which side of the redirect it came from, or `None` when that caller
    /// keeps the facade path.
    pub(super) fn target_for(&self, caller_module: &[String]) -> Option<(&[String], TargetSide)> {
        if self.unchanged_module.as_deref() == Some(caller_module) {
            return None;
        }
        match &self.target {
            RedirectTarget::Everywhere(path) => Some((path, TargetSide::Inside)),
            RedirectTarget::Scoped {
                scope,
                inside,
                outside,
            } => {
                if caller_module.starts_with(scope) {
                    Some((inside, TargetSide::Inside))
                } else {
                    Some((outside, TargetSide::Outside))
                }
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::FacadeRedirect;
    use super::RedirectTarget;
    use super::TargetSide;

    fn path(text: &str) -> Vec<String> { text.split("::").map(str::to_string).collect() }

    fn scoped() -> FacadeRedirect {
        FacadeRedirect {
            facade_path:      path("tool::panel::Widget"),
            target:           RedirectTarget::Scoped {
                scope:   path("tool"),
                inside:  path("tool::staging::Widget"),
                outside: path("tool::Widget"),
            },
            unchanged_module: Some(path("tool::panel")),
        }
    }

    #[test]
    fn a_scoped_redirect_picks_its_side_by_the_caller_module() {
        let redirect = scoped();
        let inside = path("tool::staging::Widget");
        let outside = path("tool::Widget");
        assert_eq!(
            redirect.target_for(&path("tool::render")),
            Some((inside.as_slice(), TargetSide::Inside))
        );
        assert_eq!(
            redirect.target_for(&path("app")),
            Some((outside.as_slice(), TargetSide::Outside))
        );
        assert_eq!(redirect.target_for(&path("tool::panel")), None);
    }
}
