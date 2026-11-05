// Hides the console window in release builds on Windows.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod eleven;
mod store;
mod text;

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

use base64::Engine;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tauri::{Manager, State};
use tauri_plugin_opener::OpenerExt;

use eleven::{Client, Model, SpeechRequest, Subscription, Voice, VoiceSettings};
use store::{Data, HistoryItem, Settings};

/// Requests are capped here when the model doesn't say its own limit.
const DEFAULT_CHUNK: usize = 5_000;
/// Upload limit for one clone sample; ElevenLabs rejects bigger files anyway.
const MAX_SAMPLE_BYTES: u64 = 20 * 1024 * 1024;

struct AppState {
    data: Mutex<Data>,
    data_path: PathBuf,
    cache_dir: PathBuf,
    api_base: String,
}

impl AppState {
    fn client(&self) -> Result<Client, String> {
        let key = store::load_key().ok_or("Masukkan API key ElevenLabs dulu di menu Pengaturan.")?;
        Ok(Client::new(&key, &self.api_base))
    }

    fn update<T>(&self, change: impl FnOnce(&mut Data) -> T) -> Result<T, String> {
        let mut data = self.data.lock().map_err(|e| e.to_string())?;
        let result = change(&mut data);
        data.save(&self.data_path)?;
        Ok(result)
    }
}

#[derive(Serialize)]
struct Snapshot {
    has_key: bool,
    settings: Settings,
    history: Vec<HistoryItem>,
}

#[tauri::command]
fn get_state(state: State<AppState>) -> Result<Snapshot, String> {
    let data = state.data.lock().map_err(|e| e.to_string())?;
    Ok(Snapshot { has_key: store::load_key().is_some(), settings: data.settings.clone(), history: data.history.clone() })
}

/// Checks the key against the account before storing it.
#[tauri::command]
async fn set_api_key(key: String, state: State<'_, AppState>) -> Result<Subscription, String> {
    if key.trim().is_empty() {
        return Err("API key masih kosong.".into());
    }
    let subscription = Client::new(&key, &state.api_base).subscription().await.map_err(|e| e.to_string())?;
    store::save_key(&key)?;
    Ok(subscription)
}

#[tauri::command]
fn clear_api_key() -> Result<(), String> {
    store::delete_key()
}

#[tauri::command]
async fn account(state: State<'_, AppState>) -> Result<Subscription, String> {
    state.client()?.subscription().await.map_err(|e| e.to_string())
}

#[tauri::command]
async fn list_models(state: State<'_, AppState>) -> Result<Vec<Model>, String> {
    state.client()?.models().await.map_err(|e| e.to_string())
}

#[tauri::command]
async fn list_voices(category: Option<String>, state: State<'_, AppState>) -> Result<Vec<Voice>, String> {
    let category = category.filter(|c| !c.is_empty());
    state.client()?.voices(category.as_deref()).await.map_err(|e| e.to_string())
}

/// Voice previews are cached on disk, so browsing the list again is instant and free.
#[tauri::command]
async fn voice_preview(url: String, state: State<'_, AppState>) -> Result<String, String> {
    if !url.starts_with("https://") {
        return Err("Alamat preview tidak valid.".into());
    }
    let hash: String = Sha256::digest(url.as_bytes()).iter().map(|b| format!("{b:02x}")).collect();
    let name = format!("{hash}.mp3");
    let path = state.cache_dir.join("previews").join(name);
    let bytes = match fs::read(&path) {
        Ok(bytes) => bytes,
        Err(_) => {
            // Previews are public files, so no key is sent.
            let bytes = Client::new("", &state.api_base).download(&url).await.map_err(|e| e.to_string())?;
            if let Some(dir) = path.parent() {
                fs::create_dir_all(dir).map_err(|e| e.to_string())?;
            }
            fs::write(&path, &bytes).map_err(|e| e.to_string())?;
            bytes
        }
    };
    Ok(data_url(&bytes))
}

#[derive(Deserialize)]
struct GenerateRequest {
    text: String,
    voice_id: String,
    voice_name: String,
    model_id: String,
    voice_settings: VoiceSettings,
    language_code: String,
    seed: Option<u32>,
    output_format: String,
    max_chars: Option<usize>,
}

/// Generates the script, splitting it into several requests when it is longer than the model
/// allows, and saves one MP3. Each piece gets its neighbours as context for a smooth read.
#[tauri::command]
async fn generate(request: GenerateRequest, state: State<'_, AppState>) -> Result<HistoryItem, String> {
    let text = request.text.trim();
    if text.is_empty() {
        return Err("Tulis teksnya dulu.".into());
    }
    if request.voice_id.is_empty() {
        return Err("Pilih suaranya dulu.".into());
    }
    let client = state.client()?;
    let chunks = text::chunk_text(text, request.max_chars.unwrap_or(DEFAULT_CHUNK));
    let language = Some(request.language_code.as_str()).filter(|l| !l.is_empty());

    let mut parts = Vec::with_capacity(chunks.len());
    for (i, chunk) in chunks.iter().enumerate() {
        let speech = SpeechRequest {
            text: chunk,
            model_id: &request.model_id,
            voice_settings: &request.voice_settings,
            language_code: language,
            seed: request.seed,
            previous_text: i.checked_sub(1).map(|p| chunks[p].as_str()),
            next_text: chunks.get(i + 1).map(String::as_str),
        };
        let audio = client
            .speech(&request.voice_id, &request.output_format, &speech)
            .await
            .map_err(|e| if chunks.len() > 1 { format!("Bagian {} dari {}: {e}", i + 1, chunks.len()) } else { e.to_string() })?;
        parts.push(audio);
    }
    let audio = text::concat_mp3(parts);

    let output_dir = output_dir(&state)?;
    fs::create_dir_all(&output_dir).map_err(|e| format!("Folder hasil tidak bisa dibuat: {e}"))?;
    let created_ms = now_ms();
    let base = format!("{}_{}_{}", text::slug(text, 40), text::slug(&request.voice_name, 20), created_ms / 1000);
    let path = unique_path(&output_dir, &base, extension(&request.output_format));
    fs::write(&path, &audio).map_err(|e| format!("Gagal menyimpan audio: {e}"))?;

    let item = HistoryItem {
        id: created_ms.to_string(),
        created_ms,
        text: text.to_string(),
        voice_id: request.voice_id.clone(),
        voice_name: request.voice_name.clone(),
        model_id: request.model_id.clone(),
        voice_settings: request.voice_settings.clone(),
        language_code: request.language_code.clone(),
        path: path.to_string_lossy().into_owned(),
        characters: text.chars().count(),
        parts: chunks.len(),
    };
    state.update(|data| {
        data.settings.model_id = request.model_id;
        data.settings.voice_id = request.voice_id;
        data.settings.voice_name = request.voice_name;
        data.settings.voice_settings = request.voice_settings;
        data.settings.language_code = request.language_code;
        data.settings.output_format = request.output_format;
        data.add_history(item.clone());
    })?;
    Ok(item)
}

/// Clones the user's own voice. The person must confirm the recordings are theirs or that
/// they have the speaker's permission.
#[tauri::command]
async fn clone_voice(
    name: String,
    description: String,
    files: Vec<String>,
    remove_background_noise: bool,
    consent: bool,
    state: State<'_, AppState>,
) -> Result<String, String> {
    if !consent {
        return Err("Centang dulu pernyataan bahwa ini suaramu sendiri atau kamu punya izin pemiliknya.".into());
    }
    if name.trim().is_empty() {
        return Err("Beri nama suaranya dulu.".into());
    }
    if files.is_empty() {
        return Err("Pilih minimal satu rekaman suara.".into());
    }
    let mut samples = Vec::with_capacity(files.len());
    for file in &files {
        let path = Path::new(file);
        let size = fs::metadata(path).map_err(|e| format!("\"{file}\" tidak bisa dibaca: {e}"))?.len();
        if size > MAX_SAMPLE_BYTES {
            return Err(format!("\"{}\" lebih dari 20 MB. Potong rekamannya dulu.", file_name(path)));
        }
        samples.push((file_name(path), fs::read(path).map_err(|e| e.to_string())?));
    }
    state
        .client()?
        .clone_voice(name.trim(), &description, samples, remove_background_noise)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
fn read_audio(path: String) -> Result<String, String> {
    let bytes = fs::read(&path).map_err(|_| "File audio sudah tidak ada (mungkin dipindah atau dihapus).".to_string())?;
    Ok(data_url(&bytes))
}

#[tauri::command]
fn set_output_dir(dir: String, state: State<AppState>) -> Result<Settings, String> {
    state.update(|data| {
        data.settings.output_dir = dir;
        data.settings.clone()
    })
}

/// Removes the entry from the history only; the audio file stays where it is.
#[tauri::command]
fn forget_history(id: String, state: State<AppState>) -> Result<Vec<HistoryItem>, String> {
    state.update(|data| {
        data.history.retain(|h| h.id != id);
        data.history.clone()
    })
}

#[tauri::command]
fn output_folder(state: State<AppState>) -> Result<String, String> {
    Ok(output_dir(&state)?.to_string_lossy().into_owned())
}

#[tauri::command]
fn open_path(app: tauri::AppHandle, path: String) -> Result<(), String> {
    app.opener().open_path(path, None::<&str>).map_err(|e| e.to_string())
}

#[tauri::command]
fn reveal_path(app: tauri::AppHandle, path: String) -> Result<(), String> {
    app.opener().reveal_item_in_dir(path).map_err(|e| e.to_string())
}

/// Opens a folder in Explorer, creating it first so a fresh output folder can be opened too.
#[tauri::command]
fn open_folder(app: tauri::AppHandle, path: String) -> Result<(), String> {
    fs::create_dir_all(&path).map_err(|e| e.to_string())?;
    app.opener().open_path(path, None::<&str>).map_err(|e| e.to_string())
}

/// Opens ElevenLabs pages (such as the API key page) in the browser; nothing else.
#[tauri::command]
fn open_elevenlabs(app: tauri::AppHandle, path: String) -> Result<(), String> {
    let path = path.trim_start_matches('/');
    app.opener().open_url(format!("https://elevenlabs.io/{path}"), None::<&str>).map_err(|e| e.to_string())
}

fn output_dir(state: &AppState) -> Result<PathBuf, String> {
    let data = state.data.lock().map_err(|e| e.to_string())?;
    if !data.settings.output_dir.is_empty() {
        return Ok(PathBuf::from(&data.settings.output_dir));
    }
    Ok(store::default_output_dir(None))
}

fn extension(output_format: &str) -> &'static str {
    match output_format.split('_').next() {
        Some("wav") => "wav",
        Some("opus") => "opus",
        Some("pcm") => "pcm",
        _ => "mp3",
    }
}

fn data_url(bytes: &[u8]) -> String {
    format!("data:audio/mpeg;base64,{}", base64::engine::general_purpose::STANDARD.encode(bytes))
}

fn file_name(path: &Path) -> String {
    path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| "rekaman".into())
}

fn now_ms() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0)
}

fn unique_path(dir: &Path, base: &str, ext: &str) -> PathBuf {
    (1..)
        .map(|n| if n == 1 { dir.join(format!("{base}.{ext}")) } else { dir.join(format!("{base} ({n}).{ext}")) })
        .find(|p| !p.exists())
        .expect("an unused file name")
}

fn main() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .setup(|app| {
            let config_dir = app.path().app_config_dir()?;
            let cache_dir = app.path().app_cache_dir()?;
            let data_path = config_dir.join("voicekit.json");
            let mut data = Data::load(&data_path);
            if data.settings.output_dir.is_empty() {
                let documents = app.path().document_dir().ok();
                data.settings.output_dir = store::default_output_dir(documents).to_string_lossy().into_owned();
            }
            let api_base = std::env::var("VOICEKIT_API_BASE").unwrap_or_else(|_| eleven::DEFAULT_BASE.into());
            app.manage(AppState { data: Mutex::new(data), data_path, cache_dir, api_base });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            get_state,
            set_api_key,
            clear_api_key,
            account,
            list_models,
            list_voices,
            voice_preview,
            generate,
            clone_voice,
            read_audio,
            set_output_dir,
            forget_history,
            output_folder,
            open_path,
            reveal_path,
            open_folder,
            open_elevenlabs,
        ])
        .run(tauri::generate_context!())
        .expect("VoiceKit gagal dijalankan");
}
