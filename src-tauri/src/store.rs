//! Settings and generation history, kept as JSON in the app's config folder. The API key
//! lives in the Windows Credential Manager instead, never in this file.

use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::eleven::VoiceSettings;

const KEYRING_SERVICE: &str = "VoiceKit";
const KEYRING_USER: &str = "elevenlabs-api-key";
/// Older entries are dropped once the history grows past this.
const MAX_HISTORY: usize = 500;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub output_dir: String,
    pub model_id: String,
    pub voice_id: String,
    pub voice_name: String,
    pub voice_settings: VoiceSettings,
    pub language_code: String,
    pub output_format: String,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            output_dir: String::new(),
            model_id: "eleven_v4".into(),
            voice_id: String::new(),
            voice_name: String::new(),
            voice_settings: VoiceSettings {
                stability: 0.5,
                similarity_boost: 0.75,
                style: 0.0,
                speed: 1.0,
                use_speaker_boost: true,
            },
            language_code: String::new(),
            output_format: "mp3_44100_128".into(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HistoryItem {
    pub id: String,
    pub created_ms: u64,
    pub text: String,
    pub voice_id: String,
    pub voice_name: String,
    pub model_id: String,
    pub voice_settings: VoiceSettings,
    pub language_code: String,
    pub path: String,
    pub characters: usize,
    pub parts: usize,
}

#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Data {
    pub settings: Settings,
    pub history: Vec<HistoryItem>,
}

impl Data {
    /// Reads the saved data; a missing or damaged file starts fresh.
    pub fn load(path: &Path) -> Data {
        fs::read(path)
            .ok()
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
            .unwrap_or_default()
    }

    pub fn save(&self, path: &Path) -> Result<(), String> {
        if let Some(dir) = path.parent() {
            fs::create_dir_all(dir).map_err(|e| e.to_string())?;
        }
        let tmp = path.with_extension("json.tmp");
        let json = serde_json::to_vec_pretty(self).map_err(|e| e.to_string())?;
        fs::write(&tmp, json).map_err(|e| e.to_string())?;
        fs::rename(&tmp, path).map_err(|e| e.to_string())
    }

    pub fn add_history(&mut self, item: HistoryItem) {
        self.history.insert(0, item);
        self.history.truncate(MAX_HISTORY);
    }
}

/// Where audio goes when the user hasn't picked a folder: drive D when there is one,
/// since system drives fill up fast, otherwise the Documents folder.
pub fn default_output_dir(documents: Option<PathBuf>) -> PathBuf {
    let d = Path::new("D:\\");
    if cfg!(windows) && d.exists() {
        return d.join("VoiceKit");
    }
    documents.unwrap_or_else(|| PathBuf::from(".")).join("VoiceKit")
}

pub fn load_key() -> Option<String> {
    let entry = keyring::Entry::new(KEYRING_SERVICE, KEYRING_USER).ok()?;
    entry.inner.get_password().ok().filter(|k| !k.trim().is_empty())
}

pub fn save_key(key: &str) -> Result<(), String> {
    let entry = keyring::Entry::new(KEYRING_SERVICE, KEYRING_USER).map_err(|e| e.to_string())?;
    entry.inner.set_password(key.trim()).map_err(|e| format!("Gagal menyimpan API key: {e}"))
}

pub fn delete_key() -> Result<(), String> {
    let entry = keyring::Entry::new(KEYRING_SERVICE, KEYRING_USER).map_err(|e| e.to_string())?;
    match entry.inner.delete_credential() {
        Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
        Err(e) => Err(e.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_and_caps_history() {
        let dir = std::env::temp_dir().join(format!("voicekit-store-{}", std::process::id()));
        let path = dir.join("data.json");
        let mut data = Data::load(&path);
        assert_eq!(data.settings.model_id, "eleven_v4");
        for i in 0..(MAX_HISTORY + 3) {
            data.add_history(HistoryItem {
                id: i.to_string(),
                created_ms: i as u64,
                text: "Halo".into(),
                voice_id: "v".into(),
                voice_name: "V".into(),
                model_id: "eleven_v4".into(),
                voice_settings: data.settings.voice_settings.clone(),
                language_code: String::new(),
                path: "x.mp3".into(),
                characters: 4,
                parts: 1,
            });
        }
        data.settings.voice_name = "Ani".into();
        data.save(&path).unwrap();
        let loaded = Data::load(&path);
        assert_eq!(loaded.history.len(), MAX_HISTORY);
        assert_eq!(loaded.history[0].id, (MAX_HISTORY + 2).to_string());
        assert_eq!(loaded.settings.voice_name, "Ani");
        fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn tolerates_missing_fields() {
        let data: Data = serde_json::from_str(r#"{"settings":{"voice_id":"abc"}}"#).unwrap();
        assert_eq!(data.settings.voice_id, "abc");
        assert_eq!(data.settings.output_format, "mp3_44100_128");
        assert!(data.history.is_empty());
    }
}
