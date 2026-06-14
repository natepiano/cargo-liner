use tui_pane::ClipboardBackend;
use tui_pane::KeyBind;
use tui_pane::OverlayAction;
use tui_pane::SystemClipboard;

use crate::scan::BackgroundMsg;
use crate::sccache;
use crate::sccache::Config;
use crate::tui::app::App;
use crate::tui::integration::AppGlobalAction;

pub(super) fn open_sccache_stats_overlay(app: &mut App) {
    app.overlays.close_finder();
    app.overlays.open_sccache();
    match sccache::config_from_env() {
        Config::NotConfigured => app.overlays.sccache_pane.show_not_configured(),
        Config::Configured { source } => {
            let request_id = app.overlays.sccache_pane.start_loading(source);
            let sender = app.background.background_sender();
            std::thread::spawn(move || {
                let result = sccache::read_stats();
                let _ = sender.send(BackgroundMsg::SccacheStats { request_id, result });
            });
        },
    }
}

pub(super) fn dispatch_sccache_overlay(app: &mut App, bind: &KeyBind) -> bool {
    if !app.overlays.is_sccache_open() {
        return false;
    }
    if let Some(action) = app.framework_keymap.overlay().action_for(bind)
        && matches!(action, OverlayAction::Cancel)
    {
        app.overlays.close_sccache();
        return true;
    }
    if let Some(scope) = app.framework_keymap.globals::<AppGlobalAction>()
        && matches!(scope.action_for(bind), Some(AppGlobalAction::Copy))
    {
        copy_selected_value(app);
        return true;
    }
    app.overlays.sccache_pane.handle_navigation_key(bind.code);
    true
}

fn copy_selected_value(app: &mut App) {
    let Some(target) = app.overlays.sccache_pane.selected_copy_value().cloned() else {
        app.show_timed_toast("Copy", "Nothing to copy");
        return;
    };
    let mut clipboard = SystemClipboard::new();
    match clipboard.write_clipboard(&target.value) {
        Ok(()) => app.show_timed_toast("Copy", format!("Copied {}", target.label)),
        Err(reason) if reason.is_unavailable() => {
            app.show_timed_toast("Clipboard unavailable", reason.to_string());
        },
        Err(reason) => app.show_timed_toast("Copy failed", reason.to_string()),
    }
}
