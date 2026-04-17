/// Map a tokei language name to a 2-char icon for the Lang column.
pub(crate) fn language_icon(language: &str) -> &'static str {
    match language.to_ascii_lowercase().as_str() {
        "rust" => "\u{1f980}",                                                 // 🦀
        "c" | "c++" | "c header" | "c++ header" | "c++ module" => "\u{1f30a}", // 🌊
        "go" => "Go",
        "python" => "\u{1f40d}", // 🐍
        "javascript" | "jsx" => "JS",
        "typescript" | "tsx" => "TS",
        "markdown" => "M\u{2193}", // M↓
        "shell" | "bash" | "zsh" | "fish" => "$_",
        "liquid" => "\u{1f4a7}",      // 💧
        "toml" => "\u{2699}\u{fe0f}", // ⚙️
        "json" => "{}",
        "html" => "\u{1f310}",       // 🌐
        "plain text" => "\u{1f4c4}", // 📄
        "xml" => "<>",
        "glsl" => "\u{1f53a}", // 🔺
        "yaml" => "Y:",
        "bitbake" => "\u{1f35e}",          // 🍞
        "cmake" => "\u{1f528}",            // 🔨
        "makefile" => "\u{1f6e0}\u{fe0f}", // 🛠️
        "autoconf" => "\u{1f527}",         // 🔧
        "asciidoc" => "A\u{2193}",         // A↓
        "batch" => "C:",
        _ => "  ",
    }
}
