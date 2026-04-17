/// Map a tokei language name to a 2-char icon for the Lang column.
pub(crate) fn language_icon(language: &str) -> &'static str {
    match language {
        "Rust" => "\u{1f980}",                                  // 🦀
        "C" | "C++" | "C Header" | "C++ Header" => "\u{1f30a}", // 🌊
        "Go" => "Go",
        "Python" => "\u{1f40d}", // 🐍
        "JavaScript" | "JSX" => "JS",
        "TypeScript" | "TSX" => "TS",
        "Markdown" => "M\u{2193}", // M↓
        "Shell" | "Bash" | "Zsh" | "Fish" => "$_",
        "Liquid" => "\u{1f4a7}",      // 💧
        "TOML" => "\u{2699}\u{fe0f}", // ⚙️
        "JSON" => "{}",
        "HTML" => "\u{1f310}",       // 🌐
        "Plain Text" => "\u{1f4c4}", // 📄
        _ => "  ",
    }
}
