use crate::{
    alerts,
    health::{Monitor, Overview},
    hub::{Hub, HubSnapshot},
    native,
    store::{Settings, View},
    vault::{Credential, Vault, VaultStatus},
};
use serde::Serialize;
use tauri::{AppHandle, Manager, State};

#[derive(Clone, Serialize)]
pub struct AppState {
    #[serde(flatten)]
    hub: HubSnapshot,
    vault: VaultStatus,
}

type Reply = Result<AppState, String>;

pub fn app_state(app: &AppHandle) -> AppState {
    AppState {
        hub: app.state::<Hub>().snapshot(),
        vault: app.state::<Vault>().status(),
    }
}

fn refreshed(app: &AppHandle) -> Reply {
    app.state::<Hub>().sync(app)?;
    Ok(app_state(app))
}

#[tauri::command]
pub async fn get_state(app: AppHandle) -> Reply {
    refreshed(&app)
}

#[tauri::command]
pub async fn add_game(app: AppHandle, hub: State<'_, Hub>, name: String, url: String) -> Reply {
    hub.add_game(&name, &url)?;
    refreshed(&app)
}

#[tauri::command]
pub async fn remove_game(app: AppHandle, hub: State<'_, Hub>, vault: State<'_, Vault>, game_id: String) -> Reply {
    for id in hub.remove_game(&app, &game_id)? {
        vault.forget(&id)?;
    }
    refreshed(&app)
}

#[tauri::command]
pub async fn add_session(app: AppHandle, hub: State<'_, Hub>, game_id: String) -> Reply {
    hub.add_session(&game_id)?;
    refreshed(&app)
}

#[tauri::command]
pub async fn remove_session(app: AppHandle, hub: State<'_, Hub>, vault: State<'_, Vault>, session_id: String) -> Reply {
    hub.remove_session(&app, &session_id)?;
    vault.forget(&session_id)?;
    refreshed(&app)
}

#[tauri::command]
pub async fn update_session(
    app: AppHandle,
    hub: State<'_, Hub>,
    session_id: String,
    name: String,
    group: String,
    auto_recover: bool,
) -> Reply {
    hub.update_session(&session_id, &name, &group, auto_recover)?;
    refreshed(&app)
}

#[tauri::command]
pub async fn select_session(app: AppHandle, hub: State<'_, Hub>, session_id: Option<String>) -> Reply {
    hub.select(session_id)?;
    refreshed(&app)
}

#[tauri::command]
pub async fn set_view(app: AppHandle, hub: State<'_, Hub>, view: View) -> Reply {
    hub.set_view(view)?;
    refreshed(&app)
}

#[tauri::command]
pub async fn set_covered(app: AppHandle, hub: State<'_, Hub>, covered: bool) -> Reply {
    hub.set_covered(covered);
    refreshed(&app)
}

#[tauri::command]
pub async fn set_running(app: AppHandle, hub: State<'_, Hub>, session_id: String, running: bool) -> Reply {
    hub.set_running(&session_id, running)?;
    refreshed(&app)
}

#[tauri::command]
pub async fn switch_group(app: AppHandle, hub: State<'_, Hub>, group: String) -> Reply {
    hub.switch_group(&group)?;
    refreshed(&app)
}

#[tauri::command]
pub async fn reload_session(app: AppHandle, hub: State<'_, Hub>, monitor: State<'_, Monitor>, session_id: String) -> Reply {
    hub.reload(&app, &session_id)?;
    monitor.reloaded_manually(&session_id);
    Ok(app_state(&app))
}

#[tauri::command]
pub async fn home_session(app: AppHandle, hub: State<'_, Hub>, monitor: State<'_, Monitor>, session_id: String) -> Reply {
    hub.go_home(&app, &session_id)?;
    monitor.reloaded_manually(&session_id);
    Ok(app_state(&app))
}

#[tauri::command]
pub async fn pop_out(app: AppHandle, hub: State<'_, Hub>, session_id: String) -> Reply {
    hub.pop_out(&app, &session_id)?;
    Ok(app_state(&app))
}

#[tauri::command]
pub async fn update_settings(app: AppHandle, hub: State<'_, Hub>, settings: Settings) -> Reply {
    hub.update_settings(settings)?;
    Ok(app_state(&app))
}

#[tauri::command]
pub async fn test_alert(app: AppHandle) -> Reply {
    alerts::send(&app, "test", "IdlePort", "Teste de alerta: está tudo funcionando.");
    Ok(app_state(&app))
}

#[tauri::command]
pub async fn get_overview(hub: State<'_, Hub>, monitor: State<'_, Monitor>) -> Result<Overview, String> {
    let ids: Vec<String> = hub.running_sessions().into_iter().map(|s| s.id).collect();
    let mut overview = monitor.overview(&ids);
    overview.playtime = hub.playtimes();
    Ok(overview)
}

#[tauri::command]
pub async fn vault_unlock(app: AppHandle, vault: State<'_, Vault>, password: String) -> Reply {
    vault.unlock(&password)?;
    Ok(app_state(&app))
}

#[tauri::command]
pub async fn vault_lock(app: AppHandle, vault: State<'_, Vault>) -> Reply {
    vault.lock_vault();
    Ok(app_state(&app))
}

#[tauri::command]
pub async fn vault_set_master(app: AppHandle, vault: State<'_, Vault>, password: Option<String>) -> Reply {
    vault.set_master(password.as_deref())?;
    Ok(app_state(&app))
}

#[tauri::command]
pub async fn vault_save_login(
    app: AppHandle,
    vault: State<'_, Vault>,
    session_id: String,
    username: String,
    password: String,
) -> Reply {
    vault.save_login(&session_id, &Credential { username, password })?;
    Ok(app_state(&app))
}

#[tauri::command]
pub async fn vault_forget(app: AppHandle, vault: State<'_, Vault>, session_id: String) -> Reply {
    vault.forget(&session_id)?;
    Ok(app_state(&app))
}

#[tauri::command]
pub async fn vault_username(vault: State<'_, Vault>, session_id: String) -> Result<Option<String>, String> {
    Ok(vault.credential(&session_id)?.map(|c| c.username))
}


#[tauri::command]
pub async fn vault_fill(app: AppHandle, vault: State<'_, Vault>, session_id: String) -> Reply {
    let credential = vault
        .credential(&session_id)?
        .ok_or("nenhum login salvo para esta sessão")?;
    native::fill_login(&app, &session_id, &credential.username, &credential.password)?;
    Ok(app_state(&app))
}

#[tauri::command]
pub async fn game_icon(app: AppHandle, url: String) -> Result<Option<String>, String> {
    tauri::async_runtime::spawn_blocking(move || app.state::<crate::favicon::Favicons>().get(&url))
        .await
        .map_err(|e| e.to_string())
}
