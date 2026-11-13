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
pub(in crate::fixes) enum TargetSide {
    /// The `Everywhere` path, or a `Scoped` redirect's `inside` path.
    Inside,
    /// A `Scoped` redirect's `outside` path.
    Outside,
}

/// Whether a caller keeps the facade path or receives a replacement path.
#[derive(Debug, PartialEq, Eq)]
pub(in crate::fixes) enum CallerRedirect<'a> {
    KeepFacade,
    Redirect {
        path: &'a [String],
        side: TargetSide,
    },
}

impl FacadeRedirect {
    /// Selects retention or redirection for the caller's module.
    pub(in crate::fixes) fn target_for(&self, caller_module: &[String]) -> CallerRedirect<'_> {
        if self.unchanged_module.as_deref() == Some(caller_module) {
            return CallerRedirect::KeepFacade;
        }
        let (path, side) = match &self.target {
            RedirectTarget::Everywhere(path) => (path.as_slice(), TargetSide::Inside),
            RedirectTarget::Scoped {
                scope,
                inside,
                outside,
            } => {
                if caller_module.starts_with(scope) {
                    (inside.as_slice(), TargetSide::Inside)
                } else {
                    (outside.as_slice(), TargetSide::Outside)
                }
            },
        };
        CallerRedirect::Redirect { path, side }
    }
}

#[cfg(test)]
mod tests {
    use super::CallerRedirect;
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
        let facade_redirect = scoped();
        let inside = path("tool::staging::Widget");
        let outside = path("tool::Widget");
        assert_eq!(
            facade_redirect.target_for(&path("tool::render")),
            CallerRedirect::Redirect {
                path: inside.as_slice(),
                side: TargetSide::Inside,
            }
        );
        assert_eq!(
            facade_redirect.target_for(&path("app")),
            CallerRedirect::Redirect {
                path: outside.as_slice(),
                side: TargetSide::Outside,
            }
        );
        assert_eq!(
            facade_redirect.target_for(&path("tool::panel")),
            CallerRedirect::KeepFacade
        );
    }
}
