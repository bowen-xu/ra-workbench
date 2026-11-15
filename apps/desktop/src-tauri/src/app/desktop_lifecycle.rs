use tauri::{AppHandle, Emitter, Manager, RunEvent};

pub const APP_QUIT_REQUEST_EVENT: &str = "app-quit-request";

#[cfg(target_os = "macos")]
#[path = "desktop_lifecycle/macos_termination.rs"]
mod macos_termination;
#[cfg(target_os = "macos")]
pub use macos_termination::install_termination_guard;

/// The macOS predefined Quit item calls NSApplication.terminate directly.
/// A custom item keeps menu/accelerator exit behind the existing async save gate.
#[cfg(target_os = "macos")]
pub fn install_quit_menu(app: &AppHandle) -> Result<(), Box<dyn std::error::Error>> {
    use tauri::menu::{Menu, MenuItem};
    const QUIT_ID: &str = "research-assistant-request-quit";
    let menu = Menu::default(app)?;
    let items = menu.items()?;
    let application = items
        .first()
        .and_then(|item| item.as_submenu())
        .ok_or_else(|| std::io::Error::other("Missing native application menu"))?;
    let application_items = application.items()?;
    let (position, native_quit) = application_items
        .iter()
        .enumerate()
        .next_back()
        .and_then(|(position, item)| item.as_predefined_menuitem().map(|item| (position, item)))
        .ok_or_else(|| std::io::Error::other("Missing native Quit item"))?;
    let text = native_quit.text()?;
    // Verified against the pinned Tauri 2.11.5 default menu. Fail explicitly
    // if that contract changes; never leave two Cmd+Q handlers installed.
    if !text.starts_with("Quit ") {
        return Err(std::io::Error::other("Unexpected native Quit item").into());
    }
    let quit = MenuItem::with_id(app, QUIT_ID, text, true, Some("CmdOrCtrl+Q"))?;
    application.remove_at(position)?;
    application.insert(&quit, position)?;
    app.set_menu(menu)?;
    app.on_menu_event(|app, event| {
        if event.id().0 == QUIT_ID
            && let Some(window) = app.get_webview_window("main")
        {
            // A hidden window can still own unflushed writing sessions.
            if let Err(error) = window.emit(APP_QUIT_REQUEST_EVENT, ()) {
                eprintln!("Could not request safe application quit: {error}");
            }
        }
    });
    Ok(())
}

pub fn should_defer_exit(code: Option<i32>) -> bool {
    code.is_none()
}

#[cfg(target_os = "macos")]
pub fn should_restore_main_window(has_visible_windows: bool) -> bool {
    !has_visible_windows
}

#[tauri::command]
pub async fn confirm_app_quit(app: AppHandle) -> Result<(), String> {
    let service = app
        .state::<std::sync::Arc<crate::agent::service::AgentSessionService>>()
        .inner()
        .clone();
    service
        .shutdown()
        .await
        .map_err(|_| "Agent 会话尚未完成保存，请稍后重试退出。".to_owned())?;
    app.state::<std::sync::Arc<crate::runtime_plugins::native::NativePreviewService>>()
        .shutdown()
        .await
        .map_err(|error| error.to_string())?;
    app.state::<std::sync::Arc<crate::runtime_plugins::literature_trace::service::LiteratureTraceService>>()
        .shutdown()
        .await
        .map_err(|error| error.to_string())?;
    app.state::<crate::browser_connector::BrowserConnector>()
        .shutdown()
        .await?;
    let database = app
        .state::<crate::storage::AppStore>()
        .shared_agent_connection();
    tauri::async_runtime::spawn_blocking(move || database.shutdown())
        .await
        .map_err(|error| format!("DATA_SHUTDOWN_WORKER: {error}"))?
        .map_err(|error| error.to_string())?;
    app.exit(0);
    Ok(())
}

pub fn handle_run_event(app: &AppHandle, event: RunEvent) {
    match event {
        RunEvent::ExitRequested { code, api, .. } if should_defer_exit(code) => {
            api.prevent_exit();
            let _ = app.emit(APP_QUIT_REQUEST_EVENT, ());
        }
        #[cfg(target_os = "macos")]
        RunEvent::Reopen {
            has_visible_windows,
            ..
        } if should_restore_main_window(has_visible_windows) => {
            if let Some(window) = app.get_webview_window("main") {
                let _ = window.show();
                let _ = window.set_focus();
            }
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::should_defer_exit;

    #[cfg(target_os = "macos")]
    use super::should_restore_main_window;

    #[test]
    fn defers_native_quit_but_allows_confirmed_programmatic_exit() {
        assert!(should_defer_exit(None));
        assert!(!should_defer_exit(Some(0)));
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn restores_the_main_window_only_when_reopen_has_no_visible_window() {
        assert!(should_restore_main_window(false));
        assert!(!should_restore_main_window(true));
    }
}
