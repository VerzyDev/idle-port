use crate::store::write_json;
use aes_gcm::{
    aead::{rand_core::RngCore, Aead, AeadCore, KeyInit, OsRng},
    Aes256Gcm, Key, Nonce,
};
use argon2::Argon2;
use base64::{engine::general_purpose::STANDARD as B64, Engine};
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    fs,
    path::PathBuf,
    sync::{Mutex, MutexGuard},
};

const KEYRING_SERVICE: &str = "IdlePort";
const LEGACY_KEYRING_SERVICE: &str = "IdleHub";
const KEYRING_USER: &str = "vault-key";
// Must stay unchanged: existing master-password vaults are verified against it.
const CHECK_PLAINTEXT: &[u8] = b"idlehub-vault";

type VaultKey = [u8; 32];

#[derive(Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Mode {
    #[default]
    Device,
    Master,
}

#[derive(Clone, Serialize, Deserialize)]
struct Sealed {
    nonce: String,
    data: String,
}

#[derive(Default, Serialize, Deserialize)]
#[serde(default)]
struct VaultFile {
    mode: Mode,
    salt: String,
    check: Option<Sealed>,
    entries: HashMap<String, Sealed>,
}

#[derive(Serialize, Deserialize)]
pub struct Credential {
    pub username: String,
    pub password: String,
}

#[derive(Clone, Serialize)]
pub struct VaultStatus {
    mode: Mode,
    unlocked: bool,
    logins: Vec<String>,
}

struct Inner {
    file: VaultFile,
    key: Option<VaultKey>,
}

pub struct Vault {
    path: PathBuf,
    inner: Mutex<Inner>,
}

impl Vault {
    pub fn open(path: PathBuf) -> Self {
        let file: VaultFile = fs::read_to_string(&path)
            .ok()
            .and_then(|raw| serde_json::from_str(&raw).ok())
            .unwrap_or_default();
        let key = match file.mode {
            Mode::Device => device_key().ok(),
            Mode::Master => None,
        };
        Self {
            path,
            inner: Mutex::new(Inner { file, key }),
        }
    }

    fn lock(&self) -> MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(|e| e.into_inner())
    }

    pub fn status(&self) -> VaultStatus {
        let inner = self.lock();
        VaultStatus {
            mode: inner.file.mode,
            unlocked: inner.key.is_some(),
            logins: inner.file.entries.keys().cloned().collect(),
        }
    }

    pub fn unlock(&self, password: &str) -> Result<(), String> {
        let mut inner = self.lock();
        if inner.file.mode != Mode::Master {
            return Ok(());
        }
        let salt = B64.decode(&inner.file.salt).map_err(|e| e.to_string())?;
        let key = derive_key(password, &salt)?;
        let check = inner.file.check.as_ref().ok_or("cofre corrompido")?;
        match open(&key, check) {
            Ok(plain) if plain == CHECK_PLAINTEXT => {
                inner.key = Some(key);
                Ok(())
            }
            _ => Err("senha mestra incorreta".into()),
        }
    }

    pub fn lock_vault(&self) {
        let mut inner = self.lock();
        if inner.file.mode == Mode::Master {
            inner.key = None;
        }
    }

    pub fn set_master(&self, password: Option<&str>) -> Result<(), String> {
        let mut inner = self.lock();
        let current = inner.key.ok_or("desbloqueie o cofre primeiro")?;
        let plain: Vec<(String, Vec<u8>)> = inner
            .file
            .entries
            .iter()
            .map(|(id, sealed)| open(&current, sealed).map(|p| (id.clone(), p)))
            .collect::<Result<_, _>>()?;

        let (mode, key, salt, check) = match password.map(str::trim).filter(|p| !p.is_empty()) {
            Some(password) => {
                let mut salt = [0u8; 16];
                OsRng.fill_bytes(&mut salt);
                let key = derive_key(password, &salt)?;
                let check = seal(&key, CHECK_PLAINTEXT)?;
                (Mode::Master, key, B64.encode(salt), Some(check))
            }
            None => (Mode::Device, device_key()?, String::new(), None),
        };

        let mut entries = HashMap::new();
        for (id, data) in plain {
            entries.insert(id, seal(&key, &data)?);
        }
        inner.file = VaultFile {
            mode,
            salt,
            check,
            entries,
        };
        inner.key = Some(key);
        write_json(&self.path, &inner.file)
    }

    pub fn save_login(&self, session_id: &str, credential: &Credential) -> Result<(), String> {
        let mut inner = self.lock();
        let key = inner.key.ok_or("cofre bloqueado")?;
        let data = serde_json::to_vec(credential).map_err(|e| e.to_string())?;
        inner.file.entries.insert(session_id.to_string(), seal(&key, &data)?);
        write_json(&self.path, &inner.file)
    }

    pub fn forget(&self, session_id: &str) -> Result<(), String> {
        let mut inner = self.lock();
        if inner.file.entries.remove(session_id).is_some() {
            write_json(&self.path, &inner.file)?;
        }
        Ok(())
    }

    pub fn credential(&self, session_id: &str) -> Result<Option<Credential>, String> {
        let inner = self.lock();
        let Some(sealed) = inner.file.entries.get(session_id) else {
            return Ok(None);
        };
        let key = inner.key.ok_or("cofre bloqueado")?;
        let plain = open(&key, sealed)?;
        serde_json::from_slice(&plain)
            .map(Some)
            .map_err(|e| e.to_string())
    }
}

fn device_key() -> Result<VaultKey, String> {
    let entry = keyring::Entry::new(KEYRING_SERVICE, KEYRING_USER).map_err(|e| e.to_string())?;
    match entry.get_password() {
        Ok(encoded) => B64
            .decode(encoded)
            .map_err(|e| e.to_string())?
            .try_into()
            .map_err(|_| "chave do cofre inválida".to_string()),
        Err(keyring::Error::NoEntry) => {
            let encoded = legacy_device_key().unwrap_or_else(|| {
                let mut key = [0u8; 32];
                OsRng.fill_bytes(&mut key);
                B64.encode(key)
            });
            entry.set_password(&encoded).map_err(|e| e.to_string())?;
            B64.decode(encoded)
                .map_err(|e| e.to_string())?
                .try_into()
                .map_err(|_| "chave do cofre inválida".to_string())
        }
        Err(e) => Err(e.to_string()),
    }
}

fn legacy_device_key() -> Option<String> {
    let entry = keyring::Entry::new(LEGACY_KEYRING_SERVICE, KEYRING_USER).ok()?;
    let encoded = entry.get_password().ok()?;
    let _ = entry.delete_credential();
    Some(encoded)
}

fn derive_key(password: &str, salt: &[u8]) -> Result<VaultKey, String> {
    let mut key = [0u8; 32];
    Argon2::default()
        .hash_password_into(password.as_bytes(), salt, &mut key)
        .map_err(|e| e.to_string())?;
    Ok(key)
}

fn seal(key: &VaultKey, plaintext: &[u8]) -> Result<Sealed, String> {
    let cipher = Aes256Gcm::new(Key::<Aes256Gcm>::from_slice(key));
    let nonce = Aes256Gcm::generate_nonce(&mut OsRng);
    let data = cipher
        .encrypt(&nonce, plaintext)
        .map_err(|_| "falha ao criptografar".to_string())?;
    Ok(Sealed {
        nonce: B64.encode(nonce),
        data: B64.encode(data),
    })
}

fn open(key: &VaultKey, sealed: &Sealed) -> Result<Vec<u8>, String> {
    let cipher = Aes256Gcm::new(Key::<Aes256Gcm>::from_slice(key));
    let nonce = B64.decode(&sealed.nonce).map_err(|e| e.to_string())?;
    let data = B64.decode(&sealed.data).map_err(|e| e.to_string())?;
    if nonce.len() != 12 {
        return Err("cofre corrompido".into());
    }
    cipher
        .decrypt(Nonce::from_slice(&nonce), data.as_ref())
        .map_err(|_| "não foi possível abrir o cofre".to_string())
}
