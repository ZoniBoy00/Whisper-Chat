//! System tray icon ("minimize to tray") support.
//!
//! A persistent tray icon with a "Show/Hide Whisper" + "Quit" menu (like
//! Signal/WhatsApp Desktop). Left-click is disabled so a double-click shows the
//! main window, and the menu stays on right-click. The tray is created once in
//! `setup` and kept alive by Tauri's internal tray registry, so the window can
//! be hidden to the tray (see the `minimize_to_tray` setting handled in the
//! `CloseRequested` window-event handler in `lib.rs`) without quitting.

use tauri::menu::{Menu, MenuItem};
use tauri::tray::{TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Manager};

/// Show the main window and bring it to the foreground.
fn show_main_window(app: &AppHandle) {
    if let Some(main) = app.get_webview_window("main") {
        let _ = main.show();
        let _ = main.set_focus();
    }
}

/// Toggle the main window: hide it when visible, show it when hidden.
fn toggle_main_window(app: &AppHandle) {
    if let Some(main) = app.get_webview_window("main") {
        if main.is_visible().unwrap_or(false) {
            let _ = main.hide();
        } else {
            let _ = main.show();
            let _ = main.set_focus();
        }
    }
}

/// Create the tray icon, its menu and the click handlers. The returned
/// [`TrayIcon`] is owned by Tauri's tray registry, so the icon stays alive for
/// the whole process even though we drop the handle here.
pub fn setup_tray(app: &AppHandle) -> tauri::Result<()> {
    let show_hide = MenuItem::with_id(app, "toggle", "Show/Hide Whisper", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", "Quit", true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&show_hide, &quit])?;

    let icon = app.default_window_icon().cloned().ok_or_else(|| {
        tauri::Error::InvalidIcon(std::io::Error::new(
            std::io::ErrorKind::NotFound,
            "no default window icon configured",
        ))
    })?;

    TrayIconBuilder::with_id("whisper-tray")
        .icon(icon)
        .tooltip("Whisper")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| match event.id().as_ref() {
            "toggle" => toggle_main_window(app),
            "quit" => app.exit(0),
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::DoubleClick { .. } = event {
                show_main_window(tray.app_handle());
            }
        })
        .build(app)?;

    Ok(())
}
