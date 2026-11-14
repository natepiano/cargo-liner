// `THEMES_DIRNAME` names the themes directory.
pub(super) const THEMES_DIRNAME: &str = "themes";

#[cfg(test)]
pub(super) const DARK_TEMPLATE: &str = include_str!("../../themes/default_dark.toml");
#[cfg(test)]
pub(super) const HC_TEMPLATE: &str = include_str!("../../themes/high_contrast.toml");
#[cfg(test)]
pub(super) const LIGHT_TEMPLATE: &str = include_str!("../../themes/default_light.toml");
