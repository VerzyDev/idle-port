const COMMANDS: &[&str] = &[
    "get_state",
    "add_game",
    "remove_game",
    "add_session",
    "remove_session",
    "update_session",
    "select_session",
    "set_view",
    "set_covered",
    "set_running",
    "switch_group",
    "reload_session",
    "home_session",
    "pop_out",
    "update_settings",
    "test_alert",
    "get_overview",
    "vault_unlock",
    "vault_lock",
    "vault_set_master",
    "vault_save_login",
    "vault_forget",
    "vault_username",
    "vault_fill",
    "game_icon",
];

fn main() {
    println!("cargo:rerun-if-changed=windows/app.manifest");
    tauri_build::try_build(
        tauri_build::Attributes::new()
            .app_manifest(tauri_build::AppManifest::new().commands(COMMANDS))
            .windows_attributes(
                tauri_build::WindowsAttributes::new().app_manifest(include_str!("windows/app.manifest")),
            ),
    )
    .expect("failed to run tauri-build");
}
