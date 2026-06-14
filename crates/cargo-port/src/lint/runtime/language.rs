#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProjectLanguage {
    Rust,
    NonRust,
}

impl From<bool> for ProjectLanguage {
    fn from(is_rust: bool) -> Self { if is_rust { Self::Rust } else { Self::NonRust } }
}

impl ProjectLanguage {
    pub const fn is_rust(self) -> bool { matches!(self, Self::Rust) }
}
