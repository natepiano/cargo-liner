// cargo mend user-facing invocations
pub(super) const CARGO_MEND_FIX: &str = "cargo mend --fix";
pub(super) const CARGO_MEND_FIX_ALL: &str = "cargo mend --fix-all";
pub(super) const CARGO_MEND_FIX_COMPILER: &str = "cargo mend --fix-compiler";
pub(super) const CARGO_MEND_FIX_PUB_USE: &str = "cargo mend --fix-pub-use";

// fix-availability hint strings (full sentence; `concat!` cannot interpolate
// `const` items so the literal is materialized once here per invocation)
pub(super) const HINT_FIXABLE_WITH_FIX: &str =
    "this warning is auto-fixable with `cargo mend --fix`";
pub(super) const HINT_FIXABLE_WITH_FIX_PUB_USE: &str =
    "this warning is auto-fixable with `cargo mend --fix-pub-use`";
