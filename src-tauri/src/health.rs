use crate::{native::Failure, store::WatchWord};
use serde::Serialize;
use std::{
    collections::{HashMap, HashSet, VecDeque},
    sync::{Mutex, MutexGuard},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

const FROZEN_AFTER: Duration = Duration::from_secs(45);
const STARTUP_GRACE: Duration = Duration::from_secs(90);
const HEALTHY_RESET: Duration = Duration::from_secs(300);
const BASE_BACKOFF: Duration = Duration::from_secs(30);
const MAX_BACKOFF: Duration = Duration::from_secs(900);
const ATTENTION_AFTER_ATTEMPTS: u32 = 3;
const HISTORY_LIMIT: usize = 300;
const SAMPLE_LIMIT: usize = 120;

#[derive(Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Status {
    Starting,
    Ok,
    Offline,
    SiteDown,
    Frozen,
    Recovering,
}

pub enum Action {
    Reload { reason: String, needs_attention: bool },
    Respawn { reason: String },
    Attention(String),
}

struct Health {
    status: Status,
    started: Instant,
    alive_since: Instant,
    last_ok: Option<Instant>,
    online: bool,
    was_offline: bool,
    navigation_failed: bool,
    world: Option<i64>,
    found: HashSet<String>,
    reloads: u32,
    attempts: u32,
    next_allowed: Instant,
    healthy_since: Option<Instant>,
    attention_sent: bool,
    pending_reload: Option<String>,
    pending_respawn: Option<String>,
    memory: u64,
    cpu: f32,
    samples: VecDeque<u32>,
}

impl Health {
    fn new(now: Instant) -> Self {
        Self {
            status: Status::Starting,
            started: now,
            alive_since: now,
            last_ok: None,
            online: true,
            was_offline: false,
            navigation_failed: false,
            world: None,
            found: HashSet::new(),
            reloads: 0,
            attempts: 0,
            next_allowed: now,
            healthy_since: None,
            attention_sent: false,
            pending_reload: None,
            pending_respawn: None,
            memory: 0,
            cpu: 0.0,
            samples: VecDeque::new(),
        }
    }

    fn restart(&mut self, now: Instant) {
        self.status = Status::Starting;
        self.started = now;
        self.last_ok = None;
        self.was_offline = false;
        self.navigation_failed = false;
        self.world = None;
        self.found.clear();
        self.pending_reload = None;
        self.healthy_since = None;
    }

    fn problem(&self, now: Instant) -> Option<(Status, Option<String>)> {
        if let Some(reason) = &self.pending_reload {
            return Some((Status::Recovering, Some(reason.clone())));
        }
        if !self.online {
            return Some((Status::Offline, None));
        }
        if self.was_offline {
            return Some((Status::Recovering, Some("a internet voltou".into())));
        }
        if self.navigation_failed {
            return Some((Status::SiteDown, Some("o site não carregou (fora do ar ou sem conexão)".into())));
        }
        match self.last_ok {
            Some(ok) if now - ok > FROZEN_AFTER => Some((Status::Frozen, Some("a página parou de responder".into()))),
            None if now - self.started > STARTUP_GRACE => Some((Status::Frozen, Some("a página não terminou de carregar".into()))),
            _ => None,
        }
    }

    fn reload(&mut self, now: Instant, reason: String) -> Action {
        self.attempts += 1;
        self.reloads += 1;
        let factor = 1u32 << (self.attempts - 1).min(5);
        self.next_allowed = now + (BASE_BACKOFF * factor).min(MAX_BACKOFF);
        self.restart(now);
        self.status = Status::Recovering;
        let needs_attention = self.attempts >= ATTENTION_AFTER_ATTEMPTS && !self.attention_sent;
        if needs_attention {
            self.attention_sent = true;
        }
        Action::Reload { reason, needs_attention }
    }
}

#[derive(Clone, Serialize)]
pub struct Event {
    ts: u64,
    session: String,
    text: String,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HealthView {
    status: Status,
    reloads: u32,
    memory_mb: u32,
    cpu: f32,
    uptime_secs: u64,
    samples: Vec<u32>,
}

#[derive(Clone, Serialize)]
pub struct Overview {
    pub playtime: HashMap<String, u64>,
    health: HashMap<String, HealthView>,
    history: Vec<Event>,
}

#[derive(Default)]
struct Inner {
    health: HashMap<String, Health>,
    history: VecDeque<Event>,
}

#[derive(Default)]
pub struct Monitor {
    inner: Mutex<Inner>,
}

impl Monitor {
    fn lock(&self) -> MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn with<T>(&self, id: &str, f: impl FnOnce(&mut Health) -> T) -> T {
        let now = Instant::now();
        let mut inner = self.lock();
        let health = inner
            .health
            .entry(id.to_string())
            .or_insert_with(|| Health::new(now));
        f(health)
    }

    pub fn started(&self, id: &str) {
        let now = Instant::now();
        self.with(id, |h| {
            h.restart(now);
            h.alive_since = now;
            h.pending_respawn = None;
        });
    }

    pub fn reloaded_manually(&self, id: &str) {
        let now = Instant::now();
        self.with(id, |h| h.restart(now));
    }

    pub fn is_ok(&self, id: &str) -> bool {
        self.lock().health.get(id).is_some_and(|h| h.status == Status::Ok)
    }

    pub fn stopped(&self, id: &str) {
        self.lock().health.remove(id);
    }

    pub fn world(&self, id: &str) -> Option<i64> {
        self.lock().health.get(id).and_then(|h| h.world)
    }

    pub fn set_world(&self, id: &str, world: Option<i64>) {
        self.with(id, |h| h.world = world);
    }

    pub fn navigation_finished(&self, id: &str, success: bool) {
        self.with(id, |h| {
            h.world = None;
            h.navigation_failed = !success;
        });
    }

    pub fn process_failed(&self, id: &str, failure: Failure) {
        self.with(id, |h| match failure {
            Failure::Reload(reason) => h.pending_reload = Some(reason.into()),
            Failure::Respawn(reason) => h.pending_respawn = Some(reason.into()),
        });
    }

    pub fn log(&self, session: &str, text: impl Into<String>) {
        let ts = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or_default();
        let mut inner = self.lock();
        inner.history.push_front(Event {
            ts,
            session: session.to_string(),
            text: text.into(),
        });
        inner.history.truncate(HISTORY_LIMIT);
    }

    pub fn probed(&self, id: &str, online: bool, found: Vec<String>, words: &[WatchWord]) -> Vec<WatchWord> {
        let now = Instant::now();
        self.with(id, |h| {
            h.last_ok = Some(now);
            if !online {
                h.was_offline = true;
            }
            h.online = online;

            let found: HashSet<String> = found.into_iter().map(|w| w.to_lowercase()).collect();
            let new_words: Vec<WatchWord> = words
                .iter()
                .filter(|w| {
                    let key = w.text.to_lowercase();
                    found.contains(&key) && !h.found.contains(&key)
                })
                .cloned()
                .collect();
            h.found = found;
            if let Some(word) = new_words.iter().find(|w| w.reload) {
                h.pending_reload = Some(format!("o texto \"{}\" apareceu", word.text));
            }
            new_words
        })
    }

    pub fn set_usage(&self, id: &str, memory: u64, cpu: f32) {
        if let Some(health) = self.lock().health.get_mut(id) {
            health.memory = memory;
            health.cpu = cpu;
            health.samples.push_back((memory / 1_048_576) as u32);
            if health.samples.len() > SAMPLE_LIMIT {
                health.samples.pop_front();
            }
        }
    }

    pub fn evaluate(&self, id: &str, auto_recover: bool) -> Option<Action> {
        let now = Instant::now();
        self.with(id, |h| {
            if let Some(reason) = h.pending_respawn.take() {
                h.reloads += 1;
                return Some(Action::Respawn { reason });
            }

            let Some((status, reason)) = h.problem(now) else {
                if h.last_ok.is_some() {
                    h.status = Status::Ok;
                    let since = *h.healthy_since.get_or_insert(now);
                    if now - since > HEALTHY_RESET {
                        h.attempts = 0;
                        h.attention_sent = false;
                    }
                }
                return None;
            };

            h.status = status;
            h.healthy_since = None;
            let reason = reason?;

            if !auto_recover {
                h.pending_reload = None;
                if h.attention_sent {
                    return None;
                }
                h.attention_sent = true;
                return Some(Action::Attention(reason));
            }
            if now < h.next_allowed {
                return None;
            }
            Some(h.reload(now, reason))
        })
    }

    pub fn overview(&self, running: &[String]) -> Overview {
        let now = Instant::now();
        let inner = self.lock();
        let health = running
            .iter()
            .filter_map(|id| {
                inner.health.get(id).map(|h| {
                    (
                        id.clone(),
                        HealthView {
                            status: h.status,
                            reloads: h.reloads,
                            memory_mb: (h.memory / 1_048_576) as u32,
                            cpu: h.cpu,
                            uptime_secs: (now - h.alive_since).as_secs(),
                            samples: h.samples.iter().copied().collect(),
                        },
                    )
                })
            })
            .collect();
        Overview {
            playtime: HashMap::new(),
            health,
            history: inner.history.iter().take(100).cloned().collect(),
        }
    }
}
