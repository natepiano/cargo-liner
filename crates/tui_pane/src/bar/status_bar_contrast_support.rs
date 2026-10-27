//! Independent drawn-cell contrast oracle shared by the bar regression modules.
use ratatui::buffer::Buffer;
use ratatui::style::Color;

pub(super) fn rgb(color: Color) -> [u8; 3] {
    assert_ne!(color, Color::Reset, "drawn bar must have an explicit color");
    let index = match color {
        Color::Rgb(r, g, b) => return [r, g, b],
        Color::Indexed(index) => index,
        Color::Black | Color::Reset => 0,
        Color::Red => 1,
        Color::Green => 2,
        Color::Yellow => 3,
        Color::Blue => 4,
        Color::Magenta => 5,
        Color::Cyan => 6,
        Color::Gray => 7,
        Color::DarkGray => 8,
        Color::LightRed => 9,
        Color::LightGreen => 10,
        Color::LightYellow => 11,
        Color::LightBlue => 12,
        Color::LightMagenta => 13,
        Color::LightCyan => 14,
        Color::White => 15,
    };
    let ansi = [
        [0, 0, 0],
        [205, 0, 0],
        [0, 205, 0],
        [205, 205, 0],
        [0, 0, 238],
        [205, 0, 205],
        [0, 205, 205],
        [229, 229, 229],
        [127, 127, 127],
        [255, 0, 0],
        [0, 255, 0],
        [255, 255, 0],
        [92, 92, 255],
        [255, 0, 255],
        [0, 255, 255],
        [255, 255, 255],
    ];
    match index {
        0..=15 => ansi[usize::from(index)],
        16..=231 => {
            let levels = [0, 95, 135, 175, 215, 255];
            let value = usize::from(index - 16);
            [levels[value / 36], levels[value / 6 % 6], levels[value % 6]]
        },
        _ => [8 + (index - 232) * 10; 3],
    }
}
fn luminance(channels: [f64; 3]) -> f64 {
    let linear = channels.map(|channel| {
        let value = channel / 255.0;
        if value <= 0.04045 {
            value / 12.92
        } else {
            ((value + 0.055) / 1.055).powf(2.4)
        }
    });
    f64::mul_add(
        linear[2],
        0.0722,
        f64::mul_add(linear[1], 0.7152, linear[0] * 0.2126),
    )
}
fn covered_contrast(ink: Color, ground: Color) -> f64 {
    let ink = rgb(ink).map(f64::from);
    let ground = rgb(ground).map(f64::from);
    let covered =
        std::array::from_fn(|index| f64::mul_add(ground[index], 47.0, ink[index] * 208.0) / 255.0);
    let first = luminance(covered);
    let second = luminance(ground);
    (first.max(second) + 0.05) / (first.min(second) + 0.05)
}
pub fn failures(buffer: &Buffer, words: &[&str], case: &str) -> Vec<String> {
    let text: String = buffer
        .content
        .iter()
        .map(ratatui::buffer::Cell::symbol)
        .collect();
    let mut failures = Vec::new();
    for word in words {
        let Some(offset) = text.find(word) else {
            failures.push(format!("{case}: missing drawn span {word:?}: {text}"));
            continue;
        };
        let start = text[..offset].chars().count();
        for cell in buffer.content.iter().skip(start).take(word.chars().count()) {
            if cell.symbol().trim().is_empty() {
                continue;
            }
            let ratio = covered_contrast(cell.fg, cell.bg);
            if ratio < 4.5 {
                failures.push(format!(
                    "{case} {word:?} {:?} ink={:?} ground={:?}: {ratio:.6}:1",
                    cell.symbol(),
                    cell.fg,
                    cell.bg
                ));
            }
        }
    }
    failures
}
