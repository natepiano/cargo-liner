use unicode_width::UnicodeWidthChar;

pub(super) fn word_wrap(text: &str, max_width: usize) -> Vec<String> {
    if max_width == 0 {
        return vec![String::new()];
    }
    let mut lines = Vec::new();
    let mut current = String::new();
    let mut current_width = 0;
    for word in text.split_whitespace() {
        let word_width = word.chars().map(|c| c.width().unwrap_or(0)).sum::<usize>();
        if current_width > 0 && current_width + 1 + word_width > max_width {
            lines.push(current);
            current = String::new();
            current_width = 0;
        }
        if current_width > 0 {
            current.push(' ');
            current_width += 1;
        }
        if word_width > max_width {
            let wrapped = hard_wrap(word, max_width);
            for (i, part) in wrapped.into_iter().enumerate() {
                if i == 0 {
                    current.push_str(&part);
                    current_width += part.chars().map(|c| c.width().unwrap_or(0)).sum::<usize>();
                } else {
                    lines.push(current);
                    current = part;
                    current_width = current.chars().map(|c| c.width().unwrap_or(0)).sum();
                }
            }
        } else {
            current.push_str(word);
            current_width += word_width;
        }
    }
    if !current.is_empty() {
        lines.push(current);
    }
    if lines.is_empty() {
        lines.push(String::new());
    }
    lines
}

pub(super) fn hard_wrap(text: &str, max_width: usize) -> Vec<String> {
    if max_width == 0 {
        return vec![String::new()];
    }
    let mut lines = Vec::new();
    let mut current = String::new();
    let mut current_width = 0;
    for ch in text.chars() {
        let width = ch.width().unwrap_or(0);
        if current_width > 0 && current_width + width > max_width {
            lines.push(current);
            current = String::new();
            current_width = 0;
        }
        current.push(ch);
        current_width += width;
    }
    if !current.is_empty() {
        lines.push(current);
    }
    if lines.is_empty() {
        lines.push(String::new());
    }
    lines
}
