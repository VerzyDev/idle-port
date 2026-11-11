use crate::native;
use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use std::{
    cell::RefCell,
    collections::HashMap,
    path::PathBuf,
    sync::{mpsc, OnceLock},
    thread::{self, ThreadId},
    time::Duration,
};
use tauri::{AppHandle, Window};
use wry::{
    dpi::{LogicalPosition, LogicalSize},
    Rect, WebContext, WebView, WebViewBuilder, WebViewBuilderExtWindows, WebViewExtWindows,
};

const DISPATCH_TIMEOUT: Duration = Duration::from_secs(30);
const BACKGROUND_FRIENDLY_ARGS: &str = "--disable-features=msWebOOUI,msPdfOOUI,msSmartScreenProtection,CalculateNativeWinOcclusion,IntensiveWakeUpThrottling --disable-background-timer-throttling --disable-renderer-backgrounding --disable-backgrounding-occluded-windows";

static MAIN_THREAD: OnceLock<ThreadId> = OnceLock::new();

struct GameView {
    webview: WebView,
    _context: WebContext,
}

thread_local! {
    static VIEWS: RefCell<HashMap<String, GameView>> = RefCell::new(HashMap::new());
}

#[derive(Clone, Copy, PartialEq)]
pub struct Bounds {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
}

impl Bounds {
    fn rect(self) -> Rect {
        Rect {
            position: LogicalPosition::new(self.x, self.y).into(),
            size: LogicalSize::new(self.w, self.h).into(),
        }
    }
}

pub struct Spawn {
    pub id: String,
    pub url: String,
    pub data_dir: PathBuf,
    pub bounds: Bounds,
}

pub fn mark_main_thread() {
    let _ = MAIN_THREAD.set(thread::current().id());
}

pub fn on_main<T: Send + 'static>(app: &AppHandle, task: impl FnOnce() -> T + Send + 'static) -> Result<T, String> {
    if MAIN_THREAD.get() == Some(&thread::current().id()) {
        return Ok(task());
    }
    let (tx, rx) = mpsc::channel();
    app.run_on_main_thread(move || {
        let _ = tx.send(task());
    })
    .map_err(|e| e.to_string())?;
    rx.recv_timeout(DISPATCH_TIMEOUT)
        .map_err(|_| "a interface não respondeu".to_string())
}

pub fn spawn(app: &AppHandle, parent: &Window, spec: Spawn) -> Result<(), String> {
    let mut context = WebContext::new(Some(spec.data_dir));
    let webview = WebViewBuilder::new_with_web_context(&mut context)
        .with_url(spec.url)
        .with_bounds(spec.bounds.rect())
        .with_visible(true)
        .with_additional_browser_args(BACKGROUND_FRIENDLY_ARGS)
        .build_as_child(parent)
        .map_err(|e| e.to_string())?;
    native::attach(&webview, app.clone(), spec.id.clone());
    VIEWS.with(|views| {
        views.borrow_mut().insert(
            spec.id,
            GameView {
                webview,
                _context: context,
            },
        )
    });
    Ok(())
}

pub fn remove(id: &str) {
    let view = VIEWS.with(|views| views.borrow_mut().remove(id));
    drop(view);
}

pub fn ids() -> Vec<String> {
    VIEWS.with(|views| views.borrow().keys().cloned().collect())
}

pub fn with_view<T>(id: &str, task: impl FnOnce(&WebView) -> T) -> Option<T> {
    VIEWS.with(|views| views.borrow().get(id).map(|view| task(&view.webview)))
}

pub fn place(id: &str, bounds: Bounds, zoom: f64) -> bool {
    with_view(id, |webview| {
        webview.set_bounds(bounds.rect()).is_ok() && webview.zoom(zoom).is_ok()
    })
    .unwrap_or(false)
}

pub fn reparent(id: &str, hwnd: isize) -> Result<(), String> {
    with_view(id, |webview| webview.reparent(hwnd).map_err(|e| e.to_string()))
        .unwrap_or_else(|| Err("sessão não iniciada".into()))
}

pub fn hwnd_of(window: &Window) -> Option<isize> {
    match window.window_handle().ok()?.as_raw() {
        RawWindowHandle::Win32(handle) => Some(handle.hwnd.get()),
        _ => None,
    }
}
