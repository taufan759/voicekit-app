//! A small client for the official ElevenLabs API, using the account's own API key.

use std::collections::HashMap;

use reqwest::multipart::{Form, Part};
use reqwest::{RequestBuilder, Response, StatusCode};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::Value;

pub const DEFAULT_BASE: &str = "https://api.elevenlabs.io";

#[derive(Debug, thiserror::Error)]
pub enum ApiError {
    #[error("{0}")]
    Message(String),
    #[error("Tidak bisa terhubung ke ElevenLabs. Cek koneksi internet ({0}).")]
    Network(#[from] reqwest::Error),
}

pub type ApiResult<T> = Result<T, ApiError>;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Subscription {
    pub tier: Option<String>,
    pub character_count: Option<u64>,
    pub character_limit: Option<u64>,
    pub can_use_instant_voice_cloning: Option<bool>,
    pub next_character_count_reset_unix: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Model {
    pub model_id: String,
    pub name: Option<String>,
    pub description: Option<String>,
    pub can_do_text_to_speech: Option<bool>,
    pub maximum_text_length_per_request: Option<u32>,
    #[serde(default)]
    pub languages: Vec<Language>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Language {
    pub language_id: String,
    pub name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Voice {
    pub voice_id: String,
    pub name: Option<String>,
    pub category: Option<String>,
    pub description: Option<String>,
    #[serde(default)]
    pub labels: HashMap<String, String>,
    pub preview_url: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct VoiceSettings {
    pub stability: f64,
    pub similarity_boost: f64,
    pub style: f64,
    pub speed: f64,
    pub use_speaker_boost: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct SpeechRequest<'a> {
    pub text: &'a str,
    pub model_id: &'a str,
    pub voice_settings: &'a VoiceSettings,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub language_code: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub seed: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub previous_text: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_text: Option<&'a str>,
}

#[derive(Deserialize)]
struct VoicePage {
    voices: Vec<Voice>,
    #[serde(default)]
    has_more: bool,
    next_page_token: Option<String>,
}

#[derive(Deserialize)]
struct CreatedVoice {
    voice_id: String,
}

pub struct Client {
    http: reqwest::Client,
    base: String,
    key: String,
}

impl Client {
    pub fn new(key: &str, base: &str) -> Client {
        let http = reqwest::Client::builder()
            .user_agent("VoiceKit/0.1")
            .build()
            .expect("an HTTP client");
        Client { http, base: base.trim_end_matches('/').to_string(), key: key.trim().to_string() }
    }

    fn get(&self, path: &str) -> RequestBuilder {
        self.http.get(format!("{}{path}", self.base)).header("xi-api-key", &self.key)
    }

    async fn json<T: DeserializeOwned>(&self, request: RequestBuilder) -> ApiResult<T> {
        Ok(check(request.send().await?).await?.json().await?)
    }

    pub async fn subscription(&self) -> ApiResult<Subscription> {
        self.json(self.get("/v1/user/subscription")).await
    }

    /// Models that can do text to speech.
    pub async fn models(&self) -> ApiResult<Vec<Model>> {
        let models: Vec<Model> = self.json(self.get("/v1/models")).await?;
        Ok(models.into_iter().filter(|m| m.can_do_text_to_speech.unwrap_or(true)).collect())
    }

    /// Voices in the account, optionally filtered by category ("premade", "cloned", ...).
    pub async fn voices(&self, category: Option<&str>) -> ApiResult<Vec<Voice>> {
        let mut voices = Vec::new();
        let mut token: Option<String> = None;
        for _ in 0..10 {
            let mut request = self.get("/v2/voices").query(&[("page_size", "100"), ("include_total_count", "false")]);
            if let Some(category) = category {
                request = request.query(&[("category", category)]);
            }
            if let Some(token) = &token {
                request = request.query(&[("next_page_token", token.as_str())]);
            }
            let page: VoicePage = self.json(request).await?;
            voices.extend(page.voices);
            match (page.has_more, page.next_page_token) {
                (true, Some(next)) => token = Some(next),
                _ => break,
            }
        }
        Ok(voices)
    }

    /// Generates speech and returns the audio bytes in `output_format`.
    pub async fn speech(&self, voice_id: &str, output_format: &str, request: &SpeechRequest<'_>) -> ApiResult<Vec<u8>> {
        let response = self
            .http
            .post(format!("{}/v1/text-to-speech/{voice_id}", self.base))
            .query(&[("output_format", output_format)])
            .header("xi-api-key", &self.key)
            .json(request)
            .send()
            .await?;
        Ok(check(response).await?.bytes().await?.to_vec())
    }

    /// Creates an instant voice clone from recordings and returns the new voice id.
    pub async fn clone_voice(
        &self,
        name: &str,
        description: &str,
        files: Vec<(String, Vec<u8>)>,
        remove_background_noise: bool,
    ) -> ApiResult<String> {
        let mut form = Form::new()
            .text("name", name.to_string())
            .text("remove_background_noise", remove_background_noise.to_string());
        if !description.trim().is_empty() {
            form = form.text("description", description.to_string());
        }
        for (file_name, data) in files {
            let mime = mime_for(&file_name);
            let part = Part::bytes(data).file_name(file_name).mime_str(mime)?;
            form = form.part("files", part);
        }
        let request = self
            .http
            .post(format!("{}/v1/voices/add", self.base))
            .header("xi-api-key", &self.key)
            .multipart(form);
        let created: CreatedVoice = self.json(request).await?;
        Ok(created.voice_id)
    }

    /// Downloads a public file such as a voice preview.
    pub async fn download(&self, url: &str) -> ApiResult<Vec<u8>> {
        Ok(check(self.http.get(url).send().await?).await?.bytes().await?.to_vec())
    }
}

fn mime_for(file_name: &str) -> &'static str {
    match file_name.rsplit('.').next().map(str::to_ascii_lowercase).as_deref() {
        Some("wav") => "audio/wav",
        Some("m4a") => "audio/mp4",
        Some("ogg") | Some("opus") => "audio/ogg",
        Some("flac") => "audio/flac",
        Some("webm") => "audio/webm",
        _ => "audio/mpeg",
    }
}

/// Turns error responses into readable messages.
async fn check(response: Response) -> ApiResult<Response> {
    let status = response.status();
    if status.is_success() {
        return Ok(response);
    }
    let body = response.text().await.unwrap_or_default();
    let detail = error_detail(&body);
    let message = match status {
        StatusCode::UNAUTHORIZED => format!("API key ditolak ElevenLabs. Cek lagi key-nya di Pengaturan.{}", suffix(&detail)),
        StatusCode::TOO_MANY_REQUESTS => format!("Terlalu banyak permintaan sekaligus, coba lagi sebentar lagi.{}", suffix(&detail)),
        _ if detail.is_empty() => format!("ElevenLabs menolak permintaan (HTTP {}).", status.as_u16()),
        _ => format!("ElevenLabs: {detail}"),
    };
    Err(ApiError::Message(message))
}

fn suffix(detail: &str) -> String {
    if detail.is_empty() { String::new() } else { format!(" ({detail})") }
}

/// Extracts `detail.message`, a validation list or plain text from an error body.
fn error_detail(body: &str) -> String {
    let Ok(value) = serde_json::from_str::<Value>(body) else {
        return body.chars().take(300).collect();
    };
    match value.get("detail") {
        Some(Value::String(s)) => s.clone(),
        Some(Value::Object(o)) => o
            .get("message")
            .and_then(Value::as_str)
            .or_else(|| o.get("status").and_then(Value::as_str))
            .unwrap_or_default()
            .to_string(),
        Some(Value::Array(items)) => items
            .iter()
            .filter_map(|i| i.get("msg").and_then(Value::as_str))
            .collect::<Vec<_>>()
            .join("; "),
        _ => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mockito::{Matcher, Server};

    fn settings() -> VoiceSettings {
        VoiceSettings { stability: 0.5, similarity_boost: 0.75, style: 0.0, speed: 1.0, use_speaker_boost: true }
    }

    #[tokio::test]
    async fn sends_speech_requests_with_the_key() {
        let mut server = Server::new_async().await;
        let mock = server
            .mock("POST", "/v1/text-to-speech/voice123")
            .match_header("xi-api-key", "secret")
            .match_query(Matcher::UrlEncoded("output_format".into(), "mp3_44100_128".into()))
            .match_body(Matcher::PartialJson(serde_json::json!({
                "text": "[excited] Halo!", "model_id": "eleven_v4", "language_code": "id",
                "voice_settings": { "stability": 0.5, "speed": 1.0 }
            })))
            .with_body(b"MP3DATA")
            .create_async()
            .await;
        let client = Client::new(" secret ", &server.url());
        let request = SpeechRequest {
            text: "[excited] Halo!", model_id: "eleven_v4", voice_settings: &settings(),
            language_code: Some("id"), seed: None, previous_text: None, next_text: None,
        };
        let audio = client.speech("voice123", "mp3_44100_128", &request).await.unwrap();
        assert_eq!(audio, b"MP3DATA");
        mock.assert_async().await;
    }

    #[tokio::test]
    async fn pages_through_voices() {
        let mut server = Server::new_async().await;
        server
            .mock("GET", "/v2/voices")
            // The first page has no token, so its query ends with the category.
            .match_query(Matcher::Regex("category=premade$".into()))
            .with_body(r#"{"voices":[{"voice_id":"a","name":"Ani","labels":{"accent":"indonesian"}}],"has_more":true,"next_page_token":"p2"}"#)
            .create_async()
            .await;
        server
            .mock("GET", "/v2/voices")
            .match_query(Matcher::UrlEncoded("next_page_token".into(), "p2".into()))
            .with_body(r#"{"voices":[{"voice_id":"b","name":"Budi","preview_url":"https://x/b.mp3"}],"has_more":false}"#)
            .create_async()
            .await;
        let voices = Client::new("k", &server.url()).voices(Some("premade")).await.unwrap();
        let ids: Vec<_> = voices.iter().map(|v| v.voice_id.as_str()).collect();
        assert_eq!(ids, vec!["a", "b"]);
        assert_eq!(voices[0].labels["accent"], "indonesian");
    }

    #[tokio::test]
    async fn keeps_only_text_to_speech_models() {
        let mut server = Server::new_async().await;
        server
            .mock("GET", "/v1/models")
            .with_body(r#"[{"model_id":"eleven_v4","name":"Eleven v4","can_do_text_to_speech":true,"maximum_text_length_per_request":10000,"languages":[{"language_id":"id","name":"Indonesian"}]},{"model_id":"sts","can_do_text_to_speech":false}]"#)
            .create_async()
            .await;
        let models = Client::new("k", &server.url()).models().await.unwrap();
        assert_eq!(models.len(), 1);
        assert_eq!(models[0].maximum_text_length_per_request, Some(10000));
    }

    #[tokio::test]
    async fn explains_errors() {
        let mut server = Server::new_async().await;
        server.mock("GET", "/v1/user/subscription").with_status(401)
            .with_body(r#"{"detail":{"status":"invalid_api_key","message":"Invalid API key"}}"#)
            .create_async().await;
        server.mock("POST", "/v1/voices/add").with_status(403)
            .with_body(r#"{"detail":{"message":"Your plan does not include instant voice cloning."}}"#)
            .create_async().await;
        let client = Client::new("bad", &server.url());
        let err = client.subscription().await.unwrap_err().to_string();
        assert!(err.contains("API key ditolak") && err.contains("Invalid API key"), "{err}");
        let err = client.clone_voice("Saya", "", vec![("a.mp3".into(), vec![1, 2])], false).await.unwrap_err().to_string();
        assert_eq!(err, "ElevenLabs: Your plan does not include instant voice cloning.");
    }

    #[tokio::test]
    async fn uploads_clone_samples() {
        let mut server = Server::new_async().await;
        let mock = server
            .mock("POST", "/v1/voices/add")
            .match_header("content-type", Matcher::Regex("multipart/form-data".into()))
            .match_body(Matcher::AllOf(vec![
                Matcher::Regex(r#"name="name"\r\n\r\nSuara Taufan"#.into()),
                Matcher::Regex(r#"filename="rekaman.wav""#.into()),
            ]))
            .with_body(r#"{"voice_id":"new123","requires_verification":false}"#)
            .create_async()
            .await;
        let id = Client::new("k", &server.url())
            .clone_voice("Suara Taufan", "narasi", vec![("rekaman.wav".into(), vec![0; 16])], true)
            .await
            .unwrap();
        assert_eq!(id, "new123");
        mock.assert_async().await;
    }
}
