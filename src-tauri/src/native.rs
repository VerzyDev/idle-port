use crate::{alerts, health::Monitor, hub::Hub, views};
use serde::Deserialize;
use serde_json::{json, Value};
use std::{sync::mpsc, time::Duration};
use tauri::{AppHandle, Manager};
use webview2_com::{
    take_pwstr, CallDevToolsProtocolMethodCompletedHandler, Microsoft::Web::WebView2::Win32::*,
    NavigationCompletedEventHandler, NotificationReceivedEventHandler,
    PermissionRequestedEventHandler, ProcessFailedEventHandler,
};
use windows_core::{Interface, BOOL, PCWSTR, PWSTR};
use wry::{WebView, WebViewExtWindows};

const CDP_TIMEOUT: Duration = Duration::from_secs(4);
const NO_RESPONSE: &str = "a página não respondeu";
const WORLD_NAME: &str = "idleport";

pub enum Failure {
    Reload(&'static str),
    Respawn(&'static str),
}

#[derive(Deserialize)]
pub struct Probe {
    pub online: bool,
    pub found: Vec<String>,
}

fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(Some(0)).collect()
}

fn read_pwstr(read: impl FnOnce(*mut PWSTR) -> windows_core::Result<()>) -> String {
    let mut value = PWSTR::null();
    match read(&mut value) {
        Ok(()) => take_pwstr(value),
        Err(_) => String::new(),
    }
}

pub fn attach(webview: &WebView, app: AppHandle, session_id: String) {
    let core = webview.webview();
    let mut token = 0i64;
    unsafe {
        let (handle, id) = (app.clone(), session_id.clone());
        let _ = core.add_ProcessFailed(
            &ProcessFailedEventHandler::create(Box::new(move |_, args| {
                let Some(args) = args else { return Ok(()) };
                let mut kind = COREWEBVIEW2_PROCESS_FAILED_KIND::default();
                args.ProcessFailedKind(&mut kind)?;
                let failure = match kind {
                    COREWEBVIEW2_PROCESS_FAILED_KIND_BROWSER_PROCESS_EXITED => {
                        Some(Failure::Respawn("o navegador da conta fechou"))
                    }
                    COREWEBVIEW2_PROCESS_FAILED_KIND_RENDER_PROCESS_EXITED => {
                        Some(Failure::Reload("a página fechou inesperadamente"))
                    }
                    COREWEBVIEW2_PROCESS_FAILED_KIND_RENDER_PROCESS_UNRESPONSIVE => {
                        Some(Failure::Reload("a página travou"))
                    }
                    _ => None,
                };
                if let Some(failure) = failure {
                    handle.state::<Monitor>().process_failed(&id, failure);
                }
                Ok(())
            })),
            &mut token,
        );

        let (handle, id) = (app.clone(), session_id.clone());
        let _ = core.add_NavigationCompleted(
            &NavigationCompletedEventHandler::create(Box::new(move |_, args| {
                let Some(args) = args else { return Ok(()) };
                let mut success = BOOL::default();
                let mut status = COREWEBVIEW2_WEB_ERROR_STATUS::default();
                args.IsSuccess(&mut success)?;
                args.WebErrorStatus(&mut status)?;
                let canceled = status == COREWEBVIEW2_WEB_ERROR_STATUS_OPERATION_CANCELED;
                handle
                    .state::<Monitor>()
                    .navigation_finished(&id, success.as_bool() || canceled);
                Ok(())
            })),
            &mut token,
        );

        let _ = core.add_PermissionRequested(
            &PermissionRequestedEventHandler::create(Box::new(move |_, args| {
                let Some(args) = args else { return Ok(()) };
                let mut kind = COREWEBVIEW2_PERMISSION_KIND::default();
                args.PermissionKind(&mut kind)?;
                if kind == COREWEBVIEW2_PERMISSION_KIND_NOTIFICATIONS {
                    args.SetState(COREWEBVIEW2_PERMISSION_STATE_ALLOW)?;
                }
                Ok(())
            })),
            &mut token,
        );

        if let Ok(core24) = core.cast::<ICoreWebView2_24>() {
            let (handle, id) = (app, session_id);
            let _ = core24.add_NotificationReceived(
                &NotificationReceivedEventHandler::create(Box::new(move |_, args| {
                    let Some(args) = args else { return Ok(()) };
                    let notification = args.Notification()?;
                    let title = read_pwstr(|p| notification.Title(p));
                    let body = read_pwstr(|p| notification.Body(p));
                    args.SetHandled(true)?;
                    let text = match (title.is_empty(), body.is_empty()) {
                        (false, false) => format!("{title}: {body}"),
                        (false, true) => title,
                        _ => body,
                    };
                    let session = handle
                        .state::<Hub>()
                        .session_title(&id)
                        .unwrap_or_else(|| "IdlePort".into());
                    alerts::send(&handle, &format!("notification:{id}:{text}"), &session, &text);
                    Ok(())
                })),
                &mut token,
            );
        }
    }
}

fn cdp(app: &AppHandle, session_id: &str, method: &str, params: Value) -> Result<Value, String> {
    let (tx, rx) = mpsc::channel::<Result<String, String>>();
    let id = session_id.to_string();
    let method = wide(method);
    let params = wide(&params.to_string());
    views::on_main(app, move || {
        let reply = tx.clone();
        let issued = views::with_view(&id, move |webview| unsafe {
            webview
                .webview()
                .CallDevToolsProtocolMethod(
                    PCWSTR(method.as_ptr()),
                    PCWSTR(params.as_ptr()),
                    &CallDevToolsProtocolMethodCompletedHandler::create(Box::new(move |status, json| {
                        let _ = reply.send(status.map(|_| json).map_err(|e| e.to_string()));
                        Ok(())
                    })),
                )
                .map_err(|e| e.to_string())
        });
        match issued {
            Some(Ok(())) => {}
            Some(Err(e)) => {
                let _ = tx.send(Err(e));
            }
            None => {
                let _ = tx.send(Err("sessão não iniciada".into()));
            }
        }
    })?;
    let raw = rx
        .recv_timeout(CDP_TIMEOUT)
        .map_err(|_| NO_RESPONSE.to_string())??;
    serde_json::from_str(&raw).map_err(|e| e.to_string())
}

fn create_world(app: &AppHandle, session_id: &str) -> Result<i64, String> {
    let tree = cdp(app, session_id, "Page.getFrameTree", json!({}))?;
    let frame_id = tree["frameTree"]["frame"]["id"]
        .as_str()
        .ok_or("frame principal não encontrado")?
        .to_string();
    let world = cdp(
        app,
        session_id,
        "Page.createIsolatedWorld",
        json!({ "frameId": frame_id, "worldName": WORLD_NAME, "grantUniveralAccess": false }),
    )?;
    world["executionContextId"]
        .as_i64()
        .ok_or_else(|| "mundo isolado não criado".to_string())
}

fn evaluate_in(app: &AppHandle, session_id: &str, context: i64, expression: &str) -> Result<Value, String> {
    let reply = cdp(
        app,
        session_id,
        "Runtime.evaluate",
        json!({ "expression": expression, "contextId": context, "returnByValue": true, "silent": true }),
    )?;
    if let Some(details) = reply.get("exceptionDetails") {
        return Err(details["text"].as_str().unwrap_or("erro no script").to_string());
    }
    Ok(reply["result"]["value"].clone())
}

fn evaluate_isolated(app: &AppHandle, session_id: &str, expression: &str) -> Result<Value, String> {
    let monitor = app.state::<Monitor>();
    let context = match monitor.world(session_id) {
        Some(context) => context,
        None => {
            let context = create_world(app, session_id)?;
            monitor.set_world(session_id, Some(context));
            context
        }
    };
    match evaluate_in(app, session_id, context, expression) {
        Ok(value) => Ok(value),
        Err(error) if error == NO_RESPONSE => Err(error),
        Err(_) => {
            let context = create_world(app, session_id)?;
            monitor.set_world(session_id, Some(context));
            evaluate_in(app, session_id, context, expression)
        }
    }
}

pub fn probe(app: &AppHandle, session_id: &str, words: &[String]) -> Result<Probe, String> {
    let words: Vec<String> = words.iter().map(|w| w.to_lowercase()).collect();
    let expression = format!(
        r#"(() => {{
  const words = {words};
  const text = words.length && document.body ? document.body.innerText.toLowerCase() : "";
  return {{ online: navigator.onLine, found: words.filter((w) => text.includes(w)) }};
}})()"#,
        words = serde_json::to_string(&words).map_err(|e| e.to_string())?
    );
    let value = evaluate_isolated(app, session_id, &expression)?;
    serde_json::from_value(value).map_err(|e| e.to_string())
}

const FIND_FIELDS: &str = r#"
  const usable = (el) => el && el.getClientRects().length > 0 && !el.disabled && !el.readOnly;
  const inputs = [...document.querySelectorAll("input")].filter(usable);
  const password = inputs.find((el) => el.type === "password");
  const userTypes = ["email", "text", "tel", ""];
  const user = password ? inputs.slice(0, inputs.indexOf(password)).reverse().find((el) => userTypes.includes(el.type)) : null;
  const fields = { user, password };
"#;

fn focus_field(app: &AppHandle, session_id: &str, field: &str) -> Result<bool, String> {
    let expression = format!(
        "(() => {{ {FIND_FIELDS} const el = fields[{field}]; if (!el) return false; el.focus(); el.select(); return true; }})()",
        field = serde_json::to_string(field).map_err(|e| e.to_string())?
    );
    Ok(evaluate_isolated(app, session_id, &expression)?.as_bool().unwrap_or(false))
}

fn type_into(app: &AppHandle, session_id: &str, field: &str, text: &str) -> Result<bool, String> {
    if !focus_field(app, session_id, field)? {
        return Ok(false);
    }
    cdp(app, session_id, "Input.insertText", json!({ "text": text }))?;
    let expression = format!(
        r#"(() => {{ {FIND_FIELDS}
  const el = fields[{field}];
  const text = {text};
  if (!el) return false;
  if (el.value === text) return true;
  Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value").set.call(el, text);
  el.dispatchEvent(new Event("input", {{ bubbles: true }}));
  el.dispatchEvent(new Event("change", {{ bubbles: true }}));
  return true;
}})()"#,
        field = serde_json::to_string(field).map_err(|e| e.to_string())?,
        text = serde_json::to_string(text).map_err(|e| e.to_string())?
    );
    Ok(evaluate_isolated(app, session_id, &expression)?.as_bool().unwrap_or(false))
}

pub fn fill_login(app: &AppHandle, session_id: &str, username: &str, password: &str) -> Result<(), String> {
    if !username.is_empty() {
        type_into(app, session_id, "user", username)?;
    }
    if !type_into(app, session_id, "password", password)? {
        return Err("não encontrei o campo de senha nesta página".into());
    }
    Ok(())
}
