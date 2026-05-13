use toml::Table;
use toml::Value;

use super::SettingsError;
use super::invalid;

/// Read an integer value from `section.key`.
#[must_use]
pub fn read_int(table: &Table, section: &str, key: &str) -> Option<i64> {
    table
        .get(section)
        .and_then(Value::as_table)
        .and_then(|section| section.get(key))
        .and_then(Value::as_integer)
}

/// Read a floating-point value from `section.key`.
#[must_use]
pub fn read_float(table: &Table, section: &str, key: &str) -> Option<f64> {
    table
        .get(section)
        .and_then(Value::as_table)
        .and_then(|section| section.get(key))
        .and_then(|value| {
            value.as_float().or_else(|| {
                value
                    .as_integer()
                    .and_then(|integer| integer.to_string().parse().ok())
            })
        })
}

/// Read a boolean value from `section.key`.
#[must_use]
pub fn read_bool(table: &Table, section: &str, key: &str) -> Option<bool> {
    table
        .get(section)
        .and_then(Value::as_table)
        .and_then(|section| section.get(key))
        .and_then(Value::as_bool)
}

/// Read a string value from `section.key`.
#[must_use]
pub fn read_string<'a>(table: &'a Table, section: &str, key: &str) -> Option<&'a str> {
    table
        .get(section)
        .and_then(Value::as_table)
        .and_then(|section| section.get(key))
        .and_then(Value::as_str)
}

/// Read an array value from `section.key`.
#[must_use]
pub fn read_array<'a>(table: &'a Table, section: &str, key: &str) -> Option<&'a [Value]> {
    table
        .get(section)
        .and_then(Value::as_table)
        .and_then(|section| section.get(key))
        .and_then(Value::as_array)
        .map(Vec::as_slice)
}

/// Write a TOML value to `section.key`, creating `section` when needed.
///
/// # Errors
///
/// Returns [`SettingsError`] if `section` exists but is not a TOML table.
pub fn write_value(
    table: &mut Table,
    section: &str,
    key: &str,
    value: Value,
) -> Result<(), SettingsError> {
    let section_value = table
        .entry(section.to_string())
        .or_insert_with(|| Value::Table(Table::new()));
    let Value::Table(section_table) = section_value else {
        return Err(invalid(section, key, "section is not a table"));
    };
    section_table.insert(key.to_string(), value);
    Ok(())
}
