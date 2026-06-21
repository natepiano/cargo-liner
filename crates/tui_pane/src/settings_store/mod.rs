//! Framework settings store, registry, rows, and TOML table helpers.
//!
//! `SettingsStore` owns generic settings persistence for apps that
//! embed `tui_pane`: path resolution, TOML load/save, dirty state, and
//! registered settings metadata. Apps register their own settings through
//! `SettingsRegistry`; framework-owned setting types live with their
//! owning framework module.

mod errors;
mod registry;
mod row;
mod store;
mod table;

pub(super) use errors::invalid;
pub use registry::SettingCodecs;
pub use registry::SettingsRegistry;
pub use registry::SettingsSection;
pub use row::SettingsRow;
pub use row::SettingsRowKind;
pub use row::SettingsRowPayload;
pub use store::LoadedSettings;
pub use store::SettingsError;
pub use store::SettingsFileSpec;
pub use store::SettingsStore;
pub use table::read_array;
pub use table::read_bool;
pub use table::read_float;
pub use table::read_int;
pub use table::read_string;
pub use table::write_value;
