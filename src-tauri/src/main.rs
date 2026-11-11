#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod alerts;
mod commands;
mod favicon;
mod health;
mod native;
mod hub;
mod stats;
mod store;
mod vault;
mod views;
mod watchdog;

use alerts::Alerts;
use health::Monitor;
use hub::{Hub, MAIN_WINDOW, UI_WEBVIEW};
use tauri::{
    menu::{Menu, MenuItem},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
    window::WindowBuilder,
    AppHandle, Emitter, LogicalPosition, Manager, WebviewBuilder, WebviewUrl, Window, WindowEvent,
};
use std::path::Path;
use vault::Vault;

const LEGACY_IDENTIFIER: &str = "com.idlehub.app";

fn show_main(app: &AppHandle) {
    if let Some(window) = app.get_window(MAIN_WINDOW) {
        let _ = window.show();
        let _ = window.unminimize();
        let _ = window.set_focus();
    }
}

fn migrate_legacy_data(data_dir: &Path) {
    let Some(legacy) = data_dir.parent().map(|parent| parent.join(LEGACY_IDENTIFIER)) else {
        return;
    };
    if legacy.exists() && !data_dir.exists() {
        let _ = std::fs::rename(&legacy, data_dir);
    }
}

fn push_state(app: &AppHandle) {
    let _ = app.emit("state", commands::app_state(app));
}

fn build_tray(app: &AppHandle) -> tauri::Result<()> {
    let show = MenuItem::with_id(app, "show", "Mostrar IdlePort", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", "Sair", true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&show, &quit])?;
    let mut tray = TrayIconBuilder::with_id("main")
        .tooltip("IdlePort")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| match event.id().as_ref() {
            "show" => show_main(app),
            "quit" => {
                app.state::<Hub>().save_now();
                app.exit(0)
            }
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                show_main(tray.app_handle());
            }
        });
    if let Some(icon) = app.default_window_icon() {
        tray = tray.icon(icon.clone());
    }
    tray.build(app)?;
    Ok(())
}

fn on_window_event(window: &Window, event: &WindowEvent) {
    let app = window.app_handle();
    let hub = app.state::<Hub>();
    let label = window.label();

    if hub::is_popout(label) {
        match event {
            WindowEvent::Resized(_) | WindowEvent::ScaleFactorChanged { .. } => hub.fit_popout(app, window),
            WindowEvent::CloseRequested { api, .. } => {
                api.prevent_close();
                hub.dock(app, window);
                push_state(app);
            }
            _ => {}
        }
        return;
    }

    if label != MAIN_WINDOW {
        return;
    }
    match event {
        WindowEvent::Resized(_) | WindowEvent::ScaleFactorChanged { .. } => {
            let _ = hub.apply_layout(app);
            push_state(app);
        }
        WindowEvent::Focused(true) => alerts::clear_badge(app),
        WindowEvent::CloseRequested { api, .. } if hub.settings().close_to_tray => {
            api.prevent_close();
            let _ = window.hide();
        }
        WindowEvent::CloseRequested { .. } => {
            hub.save_now();
            app.exit(0)
        }
        _ => {}
    }
}

fn main() {
    tauri::Builder::default()
        .setup(|app| {
            views::mark_main_thread();
            let data_dir = app.path().app_local_data_dir()?;
            migrate_legacy_data(&data_dir);
            let hub = Hub::new(data_dir.join("hub.json"), data_dir.join("profiles"));
            hub.prune_orphan_profiles();
            app.manage(hub);
            app.manage(Vault::open(data_dir.join("vault.json")));
            app.manage(Monitor::default());
            app.manage(Alerts::default());
            app.manage(favicon::Favicons::default());

            let window = WindowBuilder::new(app, MAIN_WINDOW)
                .title("IdlePort")
                .inner_size(1280.0, 800.0)
                .min_inner_size(800.0, 500.0)
                .build()?;
            let size = window.inner_size()?.to_logical::<f64>(window.scale_factor()?);
            let ui = WebviewBuilder::new(UI_WEBVIEW, WebviewUrl::App("index.html".into()));
            #[cfg(windows)]
            let ui = ui.additional_browser_args(
                "--disable-features=msWebOOUI,msPdfOOUI,msSmartScreenProtection --autoplay-policy=no-user-gesture-required",
            );
            window.add_child(ui, LogicalPosition::new(0.0, 0.0), size)?;

            build_tray(app.handle())?;
            watchdog::spawn(app.handle().clone());
            Ok(())
        })
        .on_window_event(on_window_event)
        .invoke_handler(tauri::generate_handler![
            commands::get_state,
            commands::add_game,
            commands::remove_game,
            commands::add_session,
            commands::remove_session,
            commands::update_session,
            commands::select_session,
            commands::set_view,
            commands::set_covered,
            commands::set_running,
            commands::switch_group,
            commands::reload_session,
            commands::home_session,
            commands::pop_out,
            commands::update_settings,
            commands::test_alert,
            commands::get_overview,
            commands::vault_unlock,
            commands::vault_lock,
            commands::vault_set_master,
            commands::vault_save_login,
            commands::vault_forget,
            commands::vault_username,
            commands::vault_fill,
            commands::game_icon,
        ])
        .build(tauri::generate_context!())
        .expect("falha ao iniciar o IdlePort")
        .run(|_, _| {});
}
