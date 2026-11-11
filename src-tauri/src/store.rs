use serde::{Deserialize, Serialize};
use std::{fs, path::Path};

fn yes() -> bool {
    true
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Session {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub group: String,
    #[serde(default = "yes")]
    pub running: bool,
    #[serde(default = "yes")]
    pub auto_recover: bool,
    #[serde(default)]
    pub playtime_secs: u64,
}

impl Session {
    pub fn new(id: String, name: String) -> Self {
        Self {
            id,
            name,
            group: String::new(),
            running: true,
            auto_recover: true,
            playtime_secs: 0,
        }
    }
}

#[derive(Clone, Serialize, Deserialize)]
pub struct Game {
    pub id: String,
    pub name: String,
    pub url: String,
    pub sessions: Vec<Session>,
}

#[derive(Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum View {
    #[default]
    Focus,
    Grid,
    Main,
    Panel,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct WatchWord {
    pub text: String,
    pub reload: bool,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Settings {
    pub view: View,
    pub close_to_tray: bool,
    pub notify: bool,
    pub sound: bool,
    pub discord_webhook: String,
    pub telegram_token: String,
    pub telegram_chat_id: String,
    pub watch_words: Vec<WatchWord>,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            view: View::Focus,
            close_to_tray: true,
            notify: true,
            sound: true,
            discord_webhook: String::new(),
            telegram_token: String::new(),
            telegram_chat_id: String::new(),
            watch_words: Vec::new(),
        }
    }
}

#[derive(Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Store {
    pub games: Vec<Game>,
    pub settings: Settings,
}

impl Store {
    pub fn load(path: &Path) -> Self {
        fs::read_to_string(path)
            .ok()
            .and_then(|raw| serde_json::from_str(&raw).ok())
            .unwrap_or_default()
    }

    pub fn save(&self, path: &Path) -> Result<(), String> {
        write_json(path, self)
    }

    pub fn game_mut(&mut self, game_id: &str) -> Result<&mut Game, String> {
        self.games
            .iter_mut()
            .find(|g| g.id == game_id)
            .ok_or_else(|| "jogo não encontrado".to_string())
    }

    pub fn session(&self, session_id: &str) -> Option<(&Game, &Session)> {
        self.games.iter().find_map(|g| {
            g.sessions
                .iter()
                .find(|s| s.id == session_id)
                .map(|s| (g, s))
        })
    }

    pub fn session_mut(&mut self, session_id: &str) -> Result<&mut Session, String> {
        self.sessions_mut()
            .find(|s| s.id == session_id)
            .ok_or_else(|| "sessão não encontrada".to_string())
    }

    pub fn sessions(&self) -> impl Iterator<Item = (&Game, &Session)> {
        self.games
            .iter()
            .flat_map(|g| g.sessions.iter().map(move |s| (g, s)))
    }

    pub fn sessions_mut(&mut self) -> impl Iterator<Item = &mut Session> {
        self.games.iter_mut().flat_map(|g| g.sessions.iter_mut())
    }

    pub fn session_ids(&self) -> impl Iterator<Item = &str> {
        self.sessions().map(|(_, s)| s.id.as_str())
    }
}

pub fn write_json<T: Serialize>(path: &Path, value: &T) -> Result<(), String> {
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    let json = serde_json::to_string_pretty(value).map_err(|e| e.to_string())?;
    let tmp = path.with_extension("json.tmp");
    fs::write(&tmp, json).map_err(|e| e.to_string())?;
    fs::rename(&tmp, path).map_err(|e| e.to_string())
}
