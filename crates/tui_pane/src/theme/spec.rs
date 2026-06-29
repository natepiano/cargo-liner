//! `StyleSpec` (color + modifier flags) and its TOML deserializer.
//!
//! Three accepted forms:
//!
//! 1. Bare color string: `"Yellow"` — color only.
//! 2. Bare color table: `{ r = 100, g = 200, b = 255 }` or `{ indexed = 208 }` — color only.
//! 3. Full spec table: `{ color = ..., bold = true, italic = false, ... }`.
//!
//! The custom `Deserialize` impl recognizes all three and emits errors
//! that name the offending value.

use core::fmt;
use core::fmt::Formatter;

use ratatui::style::Color;
use ratatui::style::Modifier;
use ratatui::style::Style;
use serde::Deserialize;
use serde::Deserializer;
use serde::de::Error as DeError;
use serde::de::MapAccess;
use serde::de::Visitor;
use toml::Table;
use toml::Value;
use toml::map::Map;

/// Style modifier flags carried by a [`StyleSpec`].
pub type Modifiers = Modifier;

/// Foreground color paired with style modifier flags.
///
/// Themes carry styles, not bare colors, so a variant can bundle
/// modifiers (Bold, Italic, etc.) with the color rather than requiring
/// every call site to add the modifier inline.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StyleSpec {
    /// The foreground color this spec applies.
    pub color:     Color,
    /// Style modifiers to combine with the color.
    pub modifiers: Modifiers,
}

impl StyleSpec {
    /// Build a [`StyleSpec`] from a bare color with no modifiers.
    #[must_use]
    pub const fn from_color(color: Color) -> Self {
        Self {
            color,
            modifiers: Modifiers::empty(),
        }
    }

    /// Build a [`StyleSpec`] from a color plus composed modifier flags.
    #[must_use]
    pub const fn with_modifiers(color: Color, modifiers: Modifiers) -> Self {
        Self { color, modifiers }
    }

    /// Build a [`StyleSpec`] from a color plus bold.
    #[must_use]
    pub const fn bold(color: Color) -> Self { Self::with_modifiers(color, Modifiers::BOLD) }

    /// Convert to a ratatui [`Style`] (foreground + modifier bits).
    #[must_use]
    pub fn style(self) -> Style { Style::default().fg(self.color).add_modifier(self.modifiers) }
}

impl<'de> Deserialize<'de> for StyleSpec {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        deserializer.deserialize_any(StyleSpecVisitor)
    }
}

struct StyleSpecVisitor;

impl<'de> Visitor<'de> for StyleSpecVisitor {
    type Value = StyleSpec;

    fn expecting(&self, f: &mut Formatter<'_>) -> fmt::Result {
        f.write_str(
            "a color name (e.g. \"Yellow\"), an { r, g, b } table, an \
             { indexed = N } table, or a full spec like \
             { color = ..., bold = true }",
        )
    }

    fn visit_str<E: DeError>(self, value: &str) -> Result<StyleSpec, E> {
        parse_named_color(value)
            .map(StyleSpec::from_color)
            .ok_or_else(|| {
                E::custom(format!(
                    "invalid color name {value:?} — expected one of: Black, \
                     Red, Green, Yellow, Blue, Magenta, Cyan, Gray, \
                     DarkGray, LightRed, LightGreen, LightYellow, \
                     LightBlue, LightMagenta, LightCyan, White, Reset"
                ))
            })
    }

    fn visit_map<A>(self, mut map: A) -> Result<StyleSpec, A::Error>
    where
        A: MapAccess<'de>,
    {
        // Buffer the table into a serde_json-style intermediate so we
        // can distinguish "bare color form" from "full spec form" after
        // seeing the keys.
        let mut keys: Vec<String> = Vec::new();
        let mut values: Vec<Value> = Vec::new();
        while let Some((k, v)) = map.next_entry::<String, Value>()? {
            keys.push(k);
            values.push(v);
        }

        if keys.contains(&"color".to_owned()) {
            // Full spec form.
            let mut color: Option<Color> = None;
            let mut modifiers = Modifiers::empty();
            for (k, v) in keys.into_iter().zip(values) {
                match k.as_str() {
                    "color" => color = Some(parse_color_value(&v).map_err(A::Error::custom)?),
                    "bold" => set_modifier(&mut modifiers, Modifiers::BOLD, &v)
                        .map_err(A::Error::custom)?,
                    "italic" => set_modifier(&mut modifiers, Modifiers::ITALIC, &v)
                        .map_err(A::Error::custom)?,
                    "dim" => set_modifier(&mut modifiers, Modifiers::DIM, &v)
                        .map_err(A::Error::custom)?,
                    "underline" => set_modifier(&mut modifiers, Modifiers::UNDERLINED, &v)
                        .map_err(A::Error::custom)?,
                    other => {
                        return Err(A::Error::custom(format!(
                            "unknown StyleSpec field {other:?} — expected one \
                             of: color, bold, italic, dim, underline"
                        )));
                    },
                }
            }
            let Some(color) = color else {
                return Err(A::Error::custom(
                    "StyleSpec table missing required \"color\" field",
                ));
            };
            Ok(StyleSpec { color, modifiers })
        } else {
            // Bare color form: { r, g, b } or { indexed = N }.
            let table = Value::Table(keys.into_iter().zip(values).collect::<Map<String, Value>>());
            let color = parse_color_value(&table).map_err(A::Error::custom)?;
            Ok(StyleSpec::from_color(color))
        }
    }
}

fn parse_bool(v: &Value) -> Result<bool, String> {
    v.as_bool()
        .ok_or_else(|| format!("expected boolean, got {v}"))
}

fn set_modifier(
    modifiers: &mut Modifiers,
    modifier: Modifiers,
    value: &Value,
) -> Result<(), String> {
    if parse_bool(value)? {
        modifiers.insert(modifier);
    } else {
        modifiers.remove(modifier);
    }
    Ok(())
}

fn parse_color_value(value: &Value) -> Result<Color, String> {
    match value {
        Value::String(name) => {
            parse_named_color(name).ok_or_else(|| format!("invalid color name {name:?}"))
        },
        Value::Table(table) => {
            if let Some(indexed) = table.get("indexed") {
                let raw = indexed
                    .as_integer()
                    .ok_or_else(|| format!("indexed must be an integer, got {indexed}"))?;
                let byte =
                    u8::try_from(raw).map_err(|_| format!("indexed {raw} out of range 0..=255"))?;
                if table.len() != 1 {
                    return Err(format!(
                        "indexed table must have only the \"indexed\" key; \
                         got {} keys",
                        table.len()
                    ));
                }
                Ok(Color::Indexed(byte))
            } else {
                let red = read_rgb_field(table, "r")?;
                let green = read_rgb_field(table, "g")?;
                let blue = read_rgb_field(table, "b")?;
                if table.len() != 3 {
                    return Err(format!(
                        "rgb table must have exactly r, g, b keys; got {} keys",
                        table.len()
                    ));
                }
                Ok(Color::Rgb(red, green, blue))
            }
        },
        other => Err(format!(
            "color must be a string or table, got {}",
            other.type_str()
        )),
    }
}

fn read_rgb_field(t: &Table, name: &str) -> Result<u8, String> {
    let v = t
        .get(name)
        .ok_or_else(|| format!("rgb table missing \"{name}\" field"))?;
    let n = v
        .as_integer()
        .ok_or_else(|| format!("rgb field \"{name}\" must be integer, got {v}"))?;
    u8::try_from(n).map_err(|_| format!("rgb field \"{name}\" = {n} out of range 0..=255"))
}

fn parse_named_color(s: &str) -> Option<Color> {
    Some(match s {
        "Reset" => Color::Reset,
        "Black" => Color::Black,
        "Red" => Color::Red,
        "Green" => Color::Green,
        "Yellow" => Color::Yellow,
        "Blue" => Color::Blue,
        "Magenta" => Color::Magenta,
        "Cyan" => Color::Cyan,
        "Gray" => Color::Gray,
        "DarkGray" => Color::DarkGray,
        "LightRed" => Color::LightRed,
        "LightGreen" => Color::LightGreen,
        "LightYellow" => Color::LightYellow,
        "LightBlue" => Color::LightBlue,
        "LightMagenta" => Color::LightMagenta,
        "LightCyan" => Color::LightCyan,
        "White" => Color::White,
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn style_spec_accepts_composed_modifier_flags() {
        let modifiers = Modifiers::BOLD | Modifiers::ITALIC;
        let spec = StyleSpec::with_modifiers(Color::Yellow, modifiers);

        assert_eq!(spec.modifiers, modifiers);
        assert_eq!(spec.style().add_modifier, modifiers);
    }

    #[test]
    fn style_spec_parses_legacy_modifier_fields_into_flags() {
        let parsed = toml::from_str::<StyleSpec>(
            r#"
            color = "Yellow"
            bold = true
            italic = true
            dim = false
            underline = true
            "#,
        )
        .ok()
        .map(|spec| (spec.color, spec.modifiers));

        assert_eq!(
            parsed,
            Some((
                Color::Yellow,
                Modifiers::BOLD | Modifiers::ITALIC | Modifiers::UNDERLINED,
            ))
        );
    }
}
