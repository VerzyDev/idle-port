use crate::{
    health::Monitor,
    store::{Game, Session, Settings, Store, View, WatchWord},
    views::{self, Bounds, Spawn},
};
use serde::Serialize;
use std::{
    collections::{HashMap, HashSet},
    fs,
    path::PathBuf,
    sync::{
        atomic::{AtomicU64, Ordering},
        Mutex, MutexGuard,
    },
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
use tauri::{window::WindowBuilder, AppHandle, LogicalPosition, LogicalSize, Manager, Url, Window};

pub const MAIN_WINDOW: &str = "main";
pub const UI_WEBVIEW: &str = "ui";
const POPOUT_PREFIX: &str = "pop-";
const SIDEBAR_WIDTH: f64 = 260.0;
const TOOLBAR_HEIGHT: f64 = 44.0;
const LABEL_HEIGHT: f64 = 26.0;
const GAP: f64 = 8.0;
const THUMB_WIDTH: f64 = 300.0;
const PLAYTIME_SAVE_INTERVAL: Duration = Duration::from_secs(60);

#[derive(Clone, Copy, PartialEq)]
struct Placement {
    bounds: Bounds,
    zoom: f64,
}

struct Content {
    window: LogicalSize<f64>,
    area: Bounds,
}

impl Content {
    fn of(window: &Window) -> Option<Self> {
        let scale = window.scale_factor().ok()?;
        let size = window.inner_size().ok()?.to_logical::<f64>(scale);
        let area = Bounds {
            x: SIDEBAR_WIDTH,
            y: TOOLBAR_HEIGHT,
            w: size.width - SIDEBAR_WIDTH,
            h: size.height - TOOLBAR_HEIGHT,
        };
        (area.w >= 1.0 && area.h >= 1.0).then_some(Self { window: size, area })
    }

    fn hidden(&self) -> Placement {
        Placement {
            bounds: Bounds {
                x: -(self.area.w + 200.0),
                ..self.area
            },
            zoom: 1.0,
        }
    }
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Tile {
    session_id: String,
    x: f64,
    y: f64,
    width: f64,
    height: f64,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HubSnapshot {
    games: Vec<Game>,
    active: Option<String>,
    settings: Settings,
    covered: bool,
    popped: Vec<String>,
    tiles: Vec<Tile>,
}

pub struct RunningSession {
    pub id: String,
    pub title: String,
    pub auto_recover: bool,
}

struct Inner {
    store: Store,
    active: Option<String>,
    covered: bool,
    popped: HashSet<String>,
    tiles: Vec<Tile>,
    applied: HashMap<String, Placement>,
    playtime_saved: Instant,
}

pub struct Hub {
    store_path: PathBuf,
    profiles_dir: PathBuf,
    inner: Mutex<Inner>,
    spawn_lock: Mutex<()>,
}

impl Hub {
    pub fn new(store_path: PathBuf, profiles_dir: PathBuf) -> Self {
        let store = Store::load(&store_path);
        let active = store
            .sessions()
            .find(|(_, s)| s.running)
            .map(|(_, s)| s.id.clone());
        Self {
            store_path,
            profiles_dir,
            inner: Mutex::new(Inner {
                store,
                active,
                covered: false,
                popped: HashSet::new(),
                tiles: Vec::new(),
                applied: HashMap::new(),
                playtime_saved: Instant::now(),
            }),
            spawn_lock: Mutex::new(()),
        }
    }

    fn lock(&self) -> MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn mutate<T>(&self, change: impl FnOnce(&mut Inner) -> Result<T, String>) -> Result<T, String> {
        let mut inner = self.lock();
        let result = change(&mut inner)?;
        inner.store.save(&self.store_path)?;
        Ok(result)
    }

    pub fn snapshot(&self) -> HubSnapshot {
        let inner = self.lock();
        HubSnapshot {
            games: inner.store.games.clone(),
            active: inner.active.clone(),
            settings: inner.store.settings.clone(),
            covered: inner.covered,
            popped: inner.popped.iter().cloned().collect(),
            tiles: inner.tiles.clone(),
        }
    }

    pub fn settings(&self) -> Settings {
        self.lock().store.settings.clone()
    }

    pub fn watch_words(&self) -> Vec<WatchWord> {
        self.lock().store.settings.watch_words.clone()
    }

    pub fn session_title(&self, session_id: &str) -> Option<String> {
        self.lock()
            .store
            .session(session_id)
            .map(|(g, s)| format!("{} · {}", g.name, s.name))
    }

    pub fn running_sessions(&self) -> Vec<RunningSession> {
        self.lock()
            .store
            .sessions()
            .filter(|(_, s)| s.running)
            .map(|(g, s)| RunningSession {
                id: s.id.clone(),
                title: format!("{} · {}", g.name, s.name),
                auto_recover: s.auto_recover,
            })
            .collect()
    }

    pub fn add_playtime(&self, ids: &[String], secs: u64) {
        if ids.is_empty() || secs == 0 {
            return;
        }
        let mut inner = self.lock();
        for session in inner.store.sessions_mut() {
            if ids.contains(&session.id) {
                session.playtime_secs += secs;
            }
        }
        if inner.playtime_saved.elapsed() >= PLAYTIME_SAVE_INTERVAL {
            inner.playtime_saved = Instant::now();
            let _ = inner.store.save(&self.store_path);
        }
    }

    pub fn save_now(&self) {
        let _ = self.lock().store.save(&self.store_path);
    }

    pub fn playtimes(&self) -> HashMap<String, u64> {
        self.lock()
            .store
            .sessions()
            .map(|(_, s)| (s.id.clone(), s.playtime_secs))
            .collect()
    }

    pub fn add_game(&self, name: &str, url: &str) -> Result<(), String> {
        let url = normalize_url(url)?;
        let name = match name.trim() {
            "" => url.host_str().unwrap_or("Jogo").to_string(),
            trimmed => trimmed.to_string(),
        };
        let session = Session::new(new_id(), "Conta 1".into());
        self.mutate(|inner| {
            inner.active = Some(session.id.clone());
            inner.covered = false;
            inner.store.games.push(Game {
                id: new_id(),
                name,
                url: url.to_string(),
                sessions: vec![session],
            });
            Ok(())
        })
    }

    pub fn remove_game(&self, app: &AppHandle, game_id: &str) -> Result<Vec<String>, String> {
        let removed = self.mutate(|inner| {
            let index = inner
                .store
                .games
                .iter()
                .position(|g| g.id == game_id)
                .ok_or("jogo não encontrado")?;
            let game = inner.store.games.remove(index);
            let ids: Vec<String> = game.sessions.into_iter().map(|s| s.id).collect();
            if inner.active.as_ref().is_some_and(|a| ids.contains(a)) {
                inner.active = None;
            }
            Ok(ids)
        })?;
        removed.iter().for_each(|id| self.destroy_session(app, id, true));
        Ok(removed)
    }

    pub fn add_session(&self, game_id: &str) -> Result<(), String> {
        self.mutate(|inner| {
            let game = inner.store.game_mut(game_id)?;
            let session = Session::new(new_id(), format!("Conta {}", game.sessions.len() + 1));
            let id = session.id.clone();
            game.sessions.push(session);
            inner.active = Some(id);
            inner.covered = false;
            Ok(())
        })
    }

    pub fn remove_session(&self, app: &AppHandle, session_id: &str) -> Result<(), String> {
        self.mutate(|inner| {
            for game in &mut inner.store.games {
                game.sessions.retain(|s| s.id != session_id);
            }
            if inner.active.as_deref() == Some(session_id) {
                inner.active = None;
            }
            Ok(())
        })?;
        self.destroy_session(app, session_id, true);
        Ok(())
    }

    pub fn update_session(&self, session_id: &str, name: &str, group: &str, auto_recover: bool) -> Result<(), String> {
        let name = name.trim();
        if name.is_empty() {
            return Err("nome vazio".into());
        }
        self.mutate(|inner| {
            let session = inner.store.session_mut(session_id)?;
            session.name = name.to_string();
            session.group = group.trim().to_string();
            session.auto_recover = auto_recover;
            Ok(())
        })
    }

    pub fn select(&self, session_id: Option<String>) -> Result<(), String> {
        self.mutate(|inner| {
            if session_id.is_some() && inner.store.settings.view == View::Panel {
                inner.store.settings.view = View::Focus;
            }
            inner.active = session_id;
            inner.covered = false;
            Ok(())
        })
    }

    pub fn set_view(&self, view: View) -> Result<(), String> {
        self.mutate(|inner| {
            inner.store.settings.view = view;
            inner.covered = false;
            Ok(())
        })
    }

    pub fn set_covered(&self, covered: bool) {
        self.lock().covered = covered;
    }

    pub fn set_running(&self, session_id: &str, running: bool) -> Result<(), String> {
        self.mutate(|inner| {
            inner.store.session_mut(session_id)?.running = running;
            Ok(())
        })
    }

    pub fn switch_group(&self, group: &str) -> Result<(), String> {
        self.mutate(|inner| {
            for session in inner.store.sessions_mut() {
                session.running = session.group == group;
            }
            let still_running = inner
                .active
                .as_deref()
                .and_then(|a| inner.store.session(a))
                .is_some_and(|(_, s)| s.running);
            if !still_running {
                inner.active = inner
                    .store
                    .sessions()
                    .find(|(_, s)| s.running)
                    .map(|(_, s)| s.id.clone());
            }
            Ok(())
        })
    }

    pub fn update_settings(&self, mut settings: Settings) -> Result<(), String> {
        settings.watch_words.retain(|w| !w.text.trim().is_empty());
        self.mutate(|inner| {
            settings.view = inner.store.settings.view;
            inner.store.settings = settings;
            Ok(())
        })
    }

    pub fn reload(&self, app: &AppHandle, session_id: &str) -> Result<(), String> {
        let id = session_id.to_string();
        views::on_main(app, move || {
            views::with_view(&id, |webview| webview.reload().map_err(|e| e.to_string()))
        })?
        .unwrap_or_else(|| Err("sessão não iniciada".into()))
    }

    pub fn go_home(&self, app: &AppHandle, session_id: &str) -> Result<(), String> {
        let url = self
            .lock()
            .store
            .session(session_id)
            .map(|(g, _)| g.url.clone())
            .ok_or("sessão não encontrada")?;
        let id = session_id.to_string();
        views::on_main(app, move || {
            views::with_view(&id, |webview| webview.load_url(&url).map_err(|e| e.to_string()))
        })?
        .unwrap_or_else(|| Err("sessão não iniciada".into()))
    }

    pub fn respawn(&self, app: &AppHandle, session_id: &str) -> Result<(), String> {
        self.destroy_session(app, session_id, false);
        self.sync(app)
    }

    pub fn sync(&self, app: &AppHandle) -> Result<(), String> {
        let _guard = self.spawn_lock.lock().unwrap_or_else(|e| e.into_inner());
        let window = app
            .get_window(MAIN_WINDOW)
            .ok_or("janela principal não encontrada")?;
        let Some(content) = Content::of(&window) else {
            return Ok(());
        };

        let existing: HashSet<String> = views::on_main(app, views::ids)?.into_iter().collect();
        let (to_spawn, to_close) = {
            let inner = self.lock();
            let running: HashMap<String, String> = inner
                .store
                .sessions()
                .filter(|(_, s)| s.running)
                .map(|(g, s)| (s.id.clone(), g.url.clone()))
                .collect();
            let to_spawn: Vec<(String, String)> = running
                .iter()
                .filter(|(id, _)| !existing.contains(*id))
                .map(|(id, url)| (id.clone(), url.clone()))
                .collect();
            let to_close: Vec<String> = existing
                .iter()
                .filter(|id| !running.contains_key(*id))
                .cloned()
                .collect();
            (to_spawn, to_close)
        };

        for id in to_close {
            self.destroy_session(app, &id, false);
        }

        let hidden = content.hidden().bounds;
        let monitor = app.state::<Monitor>();
        for (id, url) in to_spawn {
            monitor.started(&id);
            let spec = Spawn {
                id: id.clone(),
                url,
                data_dir: self.profiles_dir.join(&id),
                bounds: hidden,
            };
            let handle = app.clone();
            views::on_main(app, move || {
                let parent = handle
                    .get_window(MAIN_WINDOW)
                    .ok_or("janela principal não encontrada")?;
                views::spawn(&handle, &parent, spec)
            })??;
            self.lock().applied.remove(&id);
        }
        self.apply_layout(app)
    }

    pub fn apply_layout(&self, app: &AppHandle) -> Result<(), String> {
        let window = app
            .get_window(MAIN_WINDOW)
            .ok_or("janela principal não encontrada")?;
        let Some(content) = Content::of(&window) else {
            return Ok(());
        };

        if let Some(ui) = app.get_webview(UI_WEBVIEW) {
            ui.set_position(LogicalPosition::new(0.0, 0.0))
                .map_err(|e| e.to_string())?;
            ui.set_size(content.window).map_err(|e| e.to_string())?;
        }

        let changes: Vec<(String, Placement)> = {
            let mut inner = self.lock();
            let (visible, tiles) = plan(&inner, content.area);
            inner.tiles = tiles;
            let hidden = content.hidden();
            inner
                .store
                .sessions()
                .filter(|(_, s)| s.running && !inner.popped.contains(&s.id))
                .map(|(_, s)| (s.id.clone(), visible.get(&s.id).copied().unwrap_or(hidden)))
                .filter(|(id, placement)| inner.applied.get(id) != Some(placement))
                .collect()
        };
        if changes.is_empty() {
            return Ok(());
        }

        let placed: Vec<(String, Placement)> = views::on_main(app, move || {
            changes
                .into_iter()
                .filter(|(id, placement)| views::place(id, placement.bounds, placement.zoom))
                .collect()
        })?;
        let mut inner = self.lock();
        for (id, placement) in placed {
            inner.applied.insert(id, placement);
        }
        Ok(())
    }

    pub fn pop_out(&self, app: &AppHandle, session_id: &str) -> Result<(), String> {
        let title = self.session_title(session_id).ok_or("sessão não encontrada")?;
        self.set_running(session_id, true)?;
        self.sync(app)?;

        let label = popout_label(session_id);
        if let Some(existing) = app.get_window(&label) {
            return existing.set_focus().map_err(|e| e.to_string());
        }
        let window = WindowBuilder::new(app, &label)
            .title(title)
            .inner_size(1100.0, 750.0)
            .min_inner_size(320.0, 240.0)
            .build()
            .map_err(|e| e.to_string())?;
        let hwnd = views::hwnd_of(&window).ok_or("janela sem identificador nativo")?;
        let id = session_id.to_string();
        if let Err(e) = views::on_main(app, move || views::reparent(&id, hwnd))? {
            let _ = window.destroy();
            return Err(e);
        }
        {
            let mut inner = self.lock();
            inner.popped.insert(session_id.to_string());
            inner.applied.remove(session_id);
        }
        self.fit_popout(app, &window);
        self.apply_layout(app)
    }

    pub fn fit_popout(&self, app: &AppHandle, window: &Window) {
        let Some(id) = window.label().strip_prefix(POPOUT_PREFIX).map(str::to_string) else {
            return;
        };
        let (Ok(scale), Ok(size)) = (window.scale_factor(), window.inner_size()) else {
            return;
        };
        let size = size.to_logical::<f64>(scale);
        if size.width < 1.0 || size.height < 1.0 {
            return;
        }
        let bounds = Bounds {
            x: 0.0,
            y: 0.0,
            w: size.width,
            h: size.height,
        };
        let _ = views::on_main(app, move || views::place(&id, bounds, 1.0));
    }

    pub fn dock(&self, app: &AppHandle, window: &Window) {
        if let Some(id) = window.label().strip_prefix(POPOUT_PREFIX).map(str::to_string) {
            if let Some(main_hwnd) = app.get_window(MAIN_WINDOW).as_ref().and_then(views::hwnd_of) {
                let target = id.clone();
                let _ = views::on_main(app, move || views::reparent(&target, main_hwnd));
            }
            let mut inner = self.lock();
            inner.popped.remove(&id);
            inner.applied.remove(&id);
        }
        let _ = window.destroy();
        let _ = self.apply_layout(app);
    }

    fn destroy_session(&self, app: &AppHandle, session_id: &str, wipe_profile: bool) {
        let id = session_id.to_string();
        let _ = views::on_main(app, move || views::remove(&id));
        if let Some(popout) = app.get_window(&popout_label(session_id)) {
            let _ = popout.destroy();
        }
        {
            let mut inner = self.lock();
            inner.popped.remove(session_id);
            inner.applied.remove(session_id);
        }
        app.state::<Monitor>().stopped(session_id);
        if wipe_profile {
            let _ = fs::remove_dir_all(self.profiles_dir.join(session_id));
        }
    }

    pub fn prune_orphan_profiles(&self) {
        let inner = self.lock();
        let Ok(entries) = fs::read_dir(&self.profiles_dir) else {
            return;
        };
        for entry in entries.flatten() {
            let name = entry.file_name();
            let name = name.to_string_lossy();
            if !inner.store.session_ids().any(|id| id == name) {
                let _ = fs::remove_dir_all(entry.path());
            }
        }
    }
}

fn plan(inner: &Inner, area: Bounds) -> (HashMap<String, Placement>, Vec<Tile>) {
    let mut placements = HashMap::new();
    let mut tiles = Vec::new();
    let view = inner.store.settings.view;
    if inner.covered || view == View::Panel {
        return (placements, tiles);
    }

    let visible: Vec<&str> = inner
        .store
        .sessions()
        .filter(|(_, s)| s.running && !inner.popped.contains(&s.id))
        .map(|(_, s)| s.id.as_str())
        .collect();
    let Some(&first) = visible.first() else {
        return (placements, tiles);
    };
    let active = inner.active.as_deref().filter(|a| visible.contains(a));
    let full = Placement { bounds: area, zoom: 1.0 };

    match view {
        View::Focus => {
            if let Some(active) = active {
                placements.insert(active.to_string(), full);
            }
        }
        View::Grid => {
            let count = visible.len();
            let cols = (count as f64).sqrt().ceil() as usize;
            let rows = count.div_ceil(cols);
            let cell_w = area.w / cols as f64;
            let cell_h = area.h / rows as f64;
            for (i, id) in visible.iter().enumerate() {
                let cell = Bounds {
                    x: area.x + (i % cols) as f64 * cell_w,
                    y: area.y + (i / cols) as f64 * cell_h,
                    w: cell_w,
                    h: cell_h,
                };
                place_tile(&mut placements, &mut tiles, id, cell, area);
            }
        }
        View::Main => {
            let main = active.unwrap_or(first);
            let others: Vec<&str> = visible.iter().copied().filter(|id| *id != main).collect();
            if others.is_empty() {
                placements.insert(main.to_string(), full);
            } else {
                let thumb_w = THUMB_WIDTH.min(area.w * 0.4);
                let main_area = Bounds {
                    w: area.w - thumb_w,
                    ..area
                };
                placements.insert(main.to_string(), Placement { bounds: main_area, zoom: 1.0 });
                let slot_h = (area.h / others.len() as f64).min(thumb_w * 0.75 + LABEL_HEIGHT);
                for (i, id) in others.iter().enumerate() {
                    let cell = Bounds {
                        x: area.x + main_area.w,
                        y: area.y + i as f64 * slot_h,
                        w: thumb_w,
                        h: slot_h,
                    };
                    place_tile(&mut placements, &mut tiles, id, cell, main_area);
                }
            }
        }
        View::Panel => {}
    }
    (placements, tiles)
}

fn place_tile(placements: &mut HashMap<String, Placement>, tiles: &mut Vec<Tile>, id: &str, cell: Bounds, reference: Bounds) {
    let inset = Bounds {
        x: cell.x + GAP / 2.0,
        y: cell.y + GAP / 2.0,
        w: (cell.w - GAP).max(1.0),
        h: (cell.h - GAP).max(1.0),
    };
    tiles.push(Tile {
        session_id: id.to_string(),
        x: inset.x,
        y: inset.y,
        width: inset.w,
        height: LABEL_HEIGHT,
    });
    let bounds = Bounds {
        y: inset.y + LABEL_HEIGHT,
        h: (inset.h - LABEL_HEIGHT).max(1.0),
        ..inset
    };
    let zoom = (bounds.w / reference.w).min(bounds.h / reference.h).clamp(0.25, 1.0);
    placements.insert(id.to_string(), Placement { bounds, zoom });
}

pub fn is_popout(label: &str) -> bool {
    label.starts_with(POPOUT_PREFIX)
}

fn popout_label(session_id: &str) -> String {
    format!("{POPOUT_PREFIX}{session_id}")
}

fn new_id() -> String {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or_default();
    format!("{nanos:x}{:x}", COUNTER.fetch_add(1, Ordering::Relaxed))
}

fn normalize_url(raw: &str) -> Result<Url, String> {
    let raw = raw.trim();
    let candidate = if raw.contains("://") {
        raw.to_string()
    } else {
        format!("https://{raw}")
    };
    let url: Url = candidate.parse().map_err(|_| "URL inválida".to_string())?;
    match url.scheme() {
        "http" | "https" => Ok(url),
        _ => Err("use uma URL http ou https".into()),
    }
}
