/// Format an integer with comma-separated thousands (e.g. `1,234,567`).
#[must_use]
pub fn format_number(count: u64) -> String {
    let digits = count.to_string();
    let mut result = String::with_capacity(digits.len() + digits.len() / 3);
    for (index, ch) in digits.chars().enumerate() {
        if index > 0 && (digits.len() - index).is_multiple_of(3) {
            result.push(',');
        }
        result.push(ch);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::format_number;

    #[test]
    fn commas_separate_thousands_for_the_entire_unsigned_range() {
        for (number, expected) in [
            (0, "0"),
            (999, "999"),
            (1_000, "1,000"),
            (1_234_567, "1,234,567"),
            (u64::MAX, "18,446,744,073,709,551,615"),
        ] {
            assert_eq!(format_number(number), expected);
        }
    }
}
