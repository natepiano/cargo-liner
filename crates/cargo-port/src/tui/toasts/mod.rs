mod format;
mod manager;
mod render;

pub(super) use format::format_toast_items;
pub(super) use manager::ToastManager;
pub(super) use manager::ToastStyle;
pub(super) use manager::ToastTaskId;
pub(super) use manager::ToastView;
pub(super) use manager::TrackedItem;
pub(super) use manager::TrackedItemKey;
pub(super) use render::render_toasts;

use super::constants::TOAST_WIDTH;

pub(super) const fn toast_body_width() -> usize { TOAST_WIDTH.saturating_sub(2) as usize }
