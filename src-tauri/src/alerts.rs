use crate::{
    health::Monitor,
    hub::{Hub, MAIN_WINDOW},
};
use serde::Serialize;
use serde_json::json;
use std::{
    collections::HashMap,
    sync::Mutex,
    time::{Duration, Instant},
};
use tauri::{image::Image, AppHandle, Emitter, Manager, UserAttentionType};

const REPEAT_COOLDOWN: Duration = Duration::from_secs(60);
const BADGE_SIZE: u32 = 16;

#[derive(Default)]
pub struct Alerts {
    last_sent: Mutex<HashMap<String, Instant>>,
}

#[derive(Clone, Serialize)]
struct AlertPayload {
    title: String,
    body: String,
    sound: bool,
}

impl Alerts {
    fn allowed(&self, key: &str) -> bool {
        let now = Instant::now();
        let mut last_sent = self.last_sent.lock().unwrap_or_else(|e| e.into_inner());
        if last_sent.get(key).is_some_and(|t| now - *t < REPEAT_COOLDOWN) {
            return false;
        }
        last_sent.insert(key.to_string(), now);
        true
    }
}

pub fn send(app: &AppHandle, key: &str, title: &str, body: &str) {
    if !app.state::<Alerts>().allowed(key) {
        return;
    }
    let settings = app.state::<Hub>().settings();
    app.state::<Monitor>().log(title, format!("Alerta: {body}"));

    if settings.notify {
        show_toast(app, title, body);
    }
    let _ = app.emit(
        "alert",
        AlertPayload {
            title: title.into(),
            body: body.into(),
            sound: settings.sound,
        },
    );
    if let Some(window) = app.get_window(MAIN_WINDOW) {
        if !window.is_focused().unwrap_or(false) || !window.is_visible().unwrap_or(true) {
            let badge = badge_pixels();
            let _ = window.set_overlay_icon(Some(Image::new(&badge, BADGE_SIZE, BADGE_SIZE)));
            let _ = window.request_user_attention(Some(UserAttentionType::Informational));
        }
    }

    let text = format!("{title}\n{body}");
    let webhook = settings.discord_webhook.trim().to_string();
    let telegram = (
        settings.telegram_token.trim().to_string(),
        settings.telegram_chat_id.trim().to_string(),
    );
    let app = app.clone();
    std::thread::spawn(move || {
        if !webhook.is_empty() {
            if let Err(e) = ureq::post(&webhook).send_json(json!({ "content": text })) {
                app.state::<Monitor>().log("Discord", format!("Falha ao enviar: {e}"));
            }
        }
        if !telegram.0.is_empty() && !telegram.1.is_empty() {
            let url = format!("https://api.telegram.org/bot{}/sendMessage", telegram.0);
            if let Err(e) = ureq::post(&url).send_json(json!({ "chat_id": telegram.1, "text": text })) {
                app.state::<Monitor>().log("Telegram", format!("Falha ao enviar: {e}"));
            }
        }
    });
}

fn show_toast(app: &AppHandle, title: &str, body: &str) {
    let mut toast = notify_rust::Notification::new();
    toast.summary(title).body(body);
    if let Some(aumid) = package_aumid() {
        toast.app_id(&aumid);
    } else if is_installed() {
        toast.app_id(&app.config().identifier);
    }
    let _ = toast.show();
}

#[cfg(windows)]
fn package_aumid() -> Option<String> {
    #[link(name = "kernel32")]
    extern "system" {
        fn GetCurrentApplicationUserModelId(length: *mut u32, aumid: *mut u16) -> i32;
    }
    let mut buffer = [0u16; 256];
    let mut length = buffer.len() as u32;
    let status = unsafe { GetCurrentApplicationUserModelId(&mut length, buffer.as_mut_ptr()) };
    (status == 0).then(|| String::from_utf16_lossy(&buffer[..length.saturating_sub(1) as usize]))
}

#[cfg(not(windows))]
fn package_aumid() -> Option<String> {
    None
}

fn is_installed() -> bool {
    let Some(dir) = std::env::current_exe().ok().and_then(|exe| exe.parent().map(|p| p.to_path_buf())) else {
        return false;
    };
    !(dir.ends_with("target/debug") || dir.ends_with("target/release"))
}

pub fn clear_badge(app: &AppHandle) {
    if let Some(window) = app.get_window(MAIN_WINDOW) {
        let _ = window.set_overlay_icon(None);
    }
}

fn badge_pixels() -> Vec<u8> {
    let center = (BADGE_SIZE as f32 - 1.0) / 2.0;
    let radius = BADGE_SIZE as f32 / 2.0;
    (0..BADGE_SIZE * BADGE_SIZE)
        .flat_map(|i| {
            let dx = (i % BADGE_SIZE) as f32 - center;
            let dy = (i / BADGE_SIZE) as f32 - center;
            let alpha = (radius - (dx * dx + dy * dy).sqrt()).clamp(0.0, 1.0);
            [235, 64, 52, (alpha * 255.0) as u8]
        })
        .collect()
}
