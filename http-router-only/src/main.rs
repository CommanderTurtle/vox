use std::{
    collections::HashMap,
    net::{IpAddr, Ipv6Addr, SocketAddr},
    path::{Path, PathBuf},
    sync::{Arc, RwLock},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use axum::{
    body::Body,
    extract::{Multipart, Path as AxumPath, State},
    http::{header, HeaderMap, HeaderValue, StatusCode},
    response::{IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
use base64::{engine::general_purpose::STANDARD as BASE64, Engine as _};
use reqwest::{Client, Url};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use thiserror::Error;
use tower_http::{
    cors::{Any, CorsLayer},
    trace::TraceLayer,
};
use vox_local_core::{
    crisper::{
        normalized_mode, transcribe_tagged as transcribe_crisper, CrisperOptions, CrisperTranscript,
    },
    longcat::{synthesize as synthesize_longcat, LongCatError, LongCatOptions, LongCatReference},
};

#[derive(Debug, Deserialize)]
struct Config {
    server: ServerConfig,
    backends: BackendConfig,
    routes: RouteConfig,
    #[serde(default)]
    crisper: CrisperConfig,
    #[serde(default)]
    longcat: LongCatConfig,
    #[serde(default)]
    voices: Vec<VoiceConfig>,
}

#[derive(Debug, Deserialize)]
struct ServerConfig {
    #[serde(default = "default_host")]
    host: String,
    #[serde(default = "default_port")]
    port: u16,
    #[serde(default)]
    api_key: String,
    #[serde(default = "default_cors")]
    cors_origin: String,
}

#[derive(Debug, Deserialize)]
struct BackendConfig {
    crisper_url: String,
    translator_url: String,
    #[serde(default)]
    translator_api_key: String,
    longcat_url: String,
    #[serde(default = "default_router_url")]
    router_url: String,
}

#[derive(Debug, Deserialize)]
struct RouteConfig {
    inbound: LanguageRoute,
    outbound: LanguageRoute,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
struct LanguageRoute {
    source_language: String,
    target_language: String,
}

#[derive(Debug, Deserialize)]
struct CrisperConfig {
    #[serde(default = "default_crisper_mode")]
    default_mode: String,
    #[serde(default = "default_language")]
    language: String,
    #[serde(default = "default_chunk_duration")]
    chunk_duration: f32,
    #[serde(default = "default_stride")]
    stride: f32,
    #[serde(default = "default_context_words")]
    context_words: u32,
    #[serde(default = "default_max_tokens")]
    max_new_tokens: u32,
    #[serde(default = "default_candidate_max_tokens")]
    candidate_max_new_tokens: u32,
    #[serde(default)]
    hotwords: String,
}

impl Default for CrisperConfig {
    fn default() -> Self {
        Self {
            default_mode: default_crisper_mode(),
            language: default_language(),
            chunk_duration: default_chunk_duration(),
            stride: default_stride(),
            context_words: default_context_words(),
            max_new_tokens: default_max_tokens(),
            candidate_max_new_tokens: default_candidate_max_tokens(),
            hotwords: String::new(),
        }
    }
}

#[derive(Debug, Deserialize)]
struct LongCatConfig {
    #[serde(default)]
    default_voice: String,
    #[serde(default = "default_steps")]
    steps: u32,
    #[serde(default = "default_guidance_strength")]
    guidance_strength: f32,
    #[serde(default = "default_guidance_method")]
    guidance_method: String,
    #[serde(default = "default_seed")]
    seed: u64,
    #[serde(default = "default_duration_scale")]
    duration_scale: f32,
    #[serde(default = "default_true")]
    concatenate_chunks: bool,
    #[serde(default = "default_characters_per_request")]
    characters_per_request: usize,
}

impl Default for LongCatConfig {
    fn default() -> Self {
        Self {
            default_voice: String::new(),
            steps: default_steps(),
            guidance_strength: default_guidance_strength(),
            guidance_method: default_guidance_method(),
            seed: default_seed(),
            duration_scale: default_duration_scale(),
            concatenate_chunks: true,
            characters_per_request: default_characters_per_request(),
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
struct VoiceConfig {
    name: String,
    audio_path: PathBuf,
    transcript_path: PathBuf,
}

#[derive(Clone)]
struct AppState {
    config: Arc<Config>,
    client: Client,
    subtitle_streams: Arc<RwLock<HashMap<String, SubtitleStreamControl>>>,
}

#[derive(Clone)]
struct SubtitleStreamControl {
    abort: tokio::task::AbortHandle,
    snapshot: Arc<RwLock<SubtitleStreamSnapshot>>,
}

#[derive(Debug, Error)]
enum ApiError {
    #[error("no audio is buffered yet")]
    NoContent,
    #[error("unauthorized")]
    Unauthorized,
    #[error("bad request: {0}")]
    BadRequest(String),
    #[error("backend unavailable: {0}")]
    Backend(String),
    #[error("internal error: {0}")]
    Internal(String),
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        if matches!(&self, Self::NoContent) {
            return StatusCode::NO_CONTENT.into_response();
        }
        let status = match self {
            Self::NoContent => StatusCode::NO_CONTENT,
            Self::Unauthorized => StatusCode::UNAUTHORIZED,
            Self::BadRequest(_) => StatusCode::BAD_REQUEST,
            Self::Backend(_) => StatusCode::BAD_GATEWAY,
            Self::Internal(_) => StatusCode::INTERNAL_SERVER_ERROR,
        };
        (status, Json(json!({"error": self.to_string()}))).into_response()
    }
}

#[derive(Debug, Deserialize)]
struct TranslateRequest {
    text: String,
    #[serde(default = "default_route")]
    route: String,
    source_language: Option<String>,
    target_language: Option<String>,
}

#[derive(Debug, Serialize)]
struct TranslateResponse {
    translation: String,
    source_language: String,
    target_language: String,
    route: String,
}

#[derive(Debug, Deserialize)]
struct SpeakRequest {
    text: String,
    #[serde(default)]
    translate: bool,
    #[serde(default = "default_route")]
    route: String,
    source_language: Option<String>,
    target_language: Option<String>,
    voice: Option<String>,
    seed: Option<u64>,
}

#[derive(Debug)]
struct AudioInput {
    bytes: Vec<u8>,
    filename: String,
    mode: String,
    language: Option<String>,
    translate: bool,
    route: String,
    source_language: Option<String>,
    target_language: Option<String>,
    voice: Option<String>,
    seed: Option<u64>,
}

#[derive(Debug, Serialize)]
struct TranscribeResponse {
    transcript: String,
    source_language: Option<String>,
    translated_text: Option<String>,
    output_text: String,
    mode: String,
}

#[derive(Debug, Clone, Deserialize)]
struct SystemAudioRequest {
    #[serde(default = "default_capture_source")]
    source: String,
    seconds: Option<f32>,
    mode: Option<String>,
    language: Option<String>,
    translate: Option<bool>,
    route: Option<String>,
    source_language: Option<String>,
    target_language: Option<String>,
    voice: Option<String>,
    seed: Option<u64>,
    output: Option<String>,
    /// Internal non-destructive router cursor. HTTP callers select the
    /// public stream id in the URL instead.
    #[serde(skip)]
    consumer: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
struct SubtitleStreamSnapshot {
    id: String,
    source: String,
    running: bool,
    processing: bool,
    revision: u64,
    transcript: String,
    translated_text: Option<String>,
    output_text: String,
    mode: String,
    last_error: Option<String>,
    updated_unix_ms: u128,
}

#[derive(Debug, Serialize)]
struct RoutedAudioResponse {
    transcript: String,
    translated_text: Option<String>,
    output_text: String,
    output: String,
    seed: Option<u64>,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "vox_http=info,tower_http=info".into()),
        )
        .init();

    let config_path = match std::env::var_os("VOX_HTTP_CONFIG").filter(|value| !value.is_empty()) {
        Some(path) => PathBuf::from(path),
        None => std::env::current_exe()?
            .parent()
            .ok_or("vox-http executable has no parent directory")?
            .join("config.toml"),
    };
    if !tokio::fs::try_exists(&config_path).await? {
        tokio::fs::write(&config_path, include_str!("../config.example.toml")).await?;
        tracing::info!(path = %config_path.display(), "created executable-local configuration");
    }
    let config: Config = toml::from_str(&tokio::fs::read_to_string(&config_path).await?)?;
    validate_config(&config)?;
    let address: SocketAddr = format!("{}:{}", config.server.host, config.server.port).parse()?;
    let cors = cors_layer(&config.server.cors_origin)?;
    let state = AppState {
        config: Arc::new(config),
        client: Client::builder()
            .timeout(Duration::from_secs(1800))
            .build()?,
        subtitle_streams: Arc::new(RwLock::new(HashMap::new())),
    };

    let app = Router::new()
        .route("/health", get(health))
        .route("/v1/routes", get(routes))
        .route("/v1/translate", post(translate))
        .route("/v1/transcribe", post(transcribe))
        .route("/v1/speak", post(speak))
        .route("/v1/transcribe-and-speak", post(transcribe_and_speak))
        .route("/v1/hoist", post(hoist))
        .route("/v1/playback", post(playback))
        .route("/v1/audio/transcribe", post(system_audio_transcribe))
        .route("/v1/audio/dub", post(system_audio_dub))
        .route("/v1/audio/clear", post(audio_clear))
        .route(
            "/v1/audio/stream",
            get(system_audio_stream_status)
                .post(system_audio_stream_start)
                .delete(system_audio_stream_stop),
        )
        .route("/v1/system-audio/transcribe", post(system_audio_transcribe))
        .route("/v1/system-audio/dub", post(system_audio_dub))
        .route("/v1/system-audio/clear", post(system_audio_clear))
        .route(
            "/v1/system-audio/stream",
            get(system_audio_stream_status)
                .post(system_audio_stream_start)
                .delete(system_audio_stream_stop),
        )
        .route("/v1/captions", get(caption_stream_list))
        .route(
            "/v1/captions/{id}",
            get(caption_stream_status)
                .post(caption_stream_start)
                .delete(caption_stream_stop),
        )
        .layer(cors)
        .layer(TraceLayer::new_for_http())
        .with_state(state);

    let listener = tokio::net::TcpListener::bind(address).await?;
    tracing::info!(%address, "vox-http listening; backend lifecycle remains operator-owned");
    axum::serve(listener, app).await?;
    Ok(())
}

async fn health(State(state): State<AppState>) -> Json<Value> {
    let crisper = get_backend_json(
        &state,
        &state.config.backends.crisper_url,
        "/api/health",
        "",
    );
    let translator = get_backend_json(
        &state,
        &state.config.backends.translator_url,
        "/health",
        &state.config.backends.translator_api_key,
    );
    let longcat = get_backend_json(
        &state,
        &state.config.backends.longcat_url,
        "/api/status",
        "",
    );
    let router = get_backend_json(&state, &state.config.backends.router_url, "/health", "");
    let (crisper, translator, longcat, router) = tokio::join!(crisper, translator, longcat, router);
    let ok = crisper.is_ok() && translator.is_ok() && longcat.is_ok() && router.is_ok();
    Json(json!({
        "ok": ok,
        "gateway": "vox-http",
        "lifecycle_control": false,
        "backends": {
            "crisper": health_value(crisper),
            "translator": health_value(translator),
            "longcat": health_value(longcat),
            "router": health_value(router),
        }
    }))
}

async fn routes(State(state): State<AppState>) -> Json<Value> {
    Json(json!({
        "inbound": state.config.routes.inbound,
        "outbound": state.config.routes.outbound,
        "crisper_modes": ["intended", "literal"],
        "voices": state.config.voices.iter().map(|voice| &voice.name).collect::<Vec<_>>(),
        "default_voice": state.config.longcat.default_voice,
        "longcat": {
            "default_seed": state.config.longcat.seed,
            "concatenate_chunks": state.config.longcat.concatenate_chunks,
            "characters_per_request": state.config.longcat.characters_per_request,
            "sentence_boundaries": true,
            "voice_audio_formats": ["wav", "mp3", "m4a"],
            "voice_transcript_format": "txt"
        },
        "audio_router": {
            "hoist_formats": ["wav", "mp3", "m4a"],
            "outputs": ["playback", "mic", "wav"],
            "live_audio_sources": ["microphone", "system"],
            "live_audio_actions": ["transcribe", "translate", "dub"],
            "streaming": "named concurrent caption streams; polls never repeat inference",
            "caption_endpoints": {
                "list": "GET /v1/captions",
                "start": "POST /v1/captions/{id}",
                "status": "GET /v1/captions/{id}",
                "stop": "DELETE /v1/captions/{id}"
            }
        },
        "common_targets": ["English", "Spanish", "French", "German", "Italian", "Portuguese", "Japanese", "Korean", "Mandarin Chinese", "Arabic", "Hindi"]
    }))
}

async fn translate(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(request): Json<TranslateRequest>,
) -> Result<Json<TranslateResponse>, ApiError> {
    authorize(&state, &headers)?;
    let (translation, source, target) = translate_text(
        &state,
        &request.text,
        &request.route,
        request.source_language.as_deref(),
        request.target_language.as_deref(),
    )
    .await?;
    Ok(Json(TranslateResponse {
        translation,
        source_language: source,
        target_language: target,
        route: request.route,
    }))
}

async fn transcribe(
    State(state): State<AppState>,
    headers: HeaderMap,
    multipart: Multipart,
) -> Result<Json<TranscribeResponse>, ApiError> {
    authorize(&state, &headers)?;
    let input = parse_audio_input(multipart, &state.config.crisper.default_mode).await?;
    let transcript = transcribe_audio(&state, &input).await?;
    let source_language = transcript.language.clone();
    let translated = if input.translate {
        Some(
            translate_text(
                &state,
                &transcript.text,
                &input.route,
                source_language
                    .as_deref()
                    .or(input.source_language.as_deref()),
                input.target_language.as_deref(),
            )
            .await?
            .0,
        )
    } else {
        None
    };
    let output = translated
        .clone()
        .unwrap_or_else(|| transcript.text.clone());
    Ok(Json(TranscribeResponse {
        transcript: transcript.text,
        source_language,
        translated_text: translated,
        output_text: output,
        mode: normalized_mode(&input.mode).to_string(),
    }))
}

async fn speak(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(request): Json<SpeakRequest>,
) -> Result<Response, ApiError> {
    authorize(&state, &headers)?;
    let text = if request.translate {
        translate_text(
            &state,
            &request.text,
            &request.route,
            request.source_language.as_deref(),
            request.target_language.as_deref(),
        )
        .await?
        .0
    } else {
        request.text
    };
    let seed = request.seed.unwrap_or(state.config.longcat.seed);
    wav_response(
        synthesize(&state, &text, request.voice.as_deref(), seed).await?,
        None,
        Some(&text),
        seed,
    )
}

async fn transcribe_and_speak(
    State(state): State<AppState>,
    headers: HeaderMap,
    multipart: Multipart,
) -> Result<Response, ApiError> {
    authorize(&state, &headers)?;
    let input = parse_audio_input(multipart, &state.config.crisper.default_mode).await?;
    let transcript = transcribe_audio(&state, &input).await?;
    let output = if input.translate {
        translate_text(
            &state,
            &transcript.text,
            &input.route,
            transcript
                .language
                .as_deref()
                .or(input.source_language.as_deref()),
            input.target_language.as_deref(),
        )
        .await?
        .0
    } else {
        transcript.text.clone()
    };
    let seed = input.seed.unwrap_or(state.config.longcat.seed);
    let wav = synthesize(&state, &output, input.voice.as_deref(), seed).await?;
    wav_response(wav, Some(&transcript.text), Some(&output), seed)
}

async fn hoist(
    State(state): State<AppState>,
    headers: HeaderMap,
    multipart: Multipart,
) -> Result<Json<Value>, ApiError> {
    authorize(&state, &headers)?;
    let input = parse_audio_input(multipart, &state.config.crisper.default_mode).await?;
    post_router_audio(&state, "/v1/forward", &input.bytes, &input.filename).await?;
    Ok(Json(json!({"status": "queued", "output": "microphone"})))
}

async fn playback(
    State(state): State<AppState>,
    headers: HeaderMap,
    multipart: Multipart,
) -> Result<Json<Value>, ApiError> {
    authorize(&state, &headers)?;
    let input = parse_audio_input(multipart, &state.config.crisper.default_mode).await?;
    post_router_audio(&state, "/v1/playback", &input.bytes, &input.filename).await?;
    Ok(Json(
        json!({"status": "playing", "output": "default_playback"}),
    ))
}

async fn system_audio_transcribe(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(request): Json<SystemAudioRequest>,
) -> Result<Json<TranscribeResponse>, ApiError> {
    authorize(&state, &headers)?;
    let input = take_router_audio(&state, &request).await?;
    let transcript = transcribe_audio(&state, &input).await?;
    let source_language = transcript.language.clone();
    let translated = if request.translate.unwrap_or(false) {
        Some(
            translate_text(
                &state,
                &transcript.text,
                request.route.as_deref().unwrap_or("inbound"),
                source_language
                    .as_deref()
                    .or(request.source_language.as_deref()),
                request.target_language.as_deref(),
            )
            .await?
            .0,
        )
    } else {
        None
    };
    let output = translated
        .clone()
        .unwrap_or_else(|| transcript.text.clone());
    Ok(Json(TranscribeResponse {
        transcript: transcript.text,
        source_language,
        translated_text: translated,
        output_text: output,
        mode: normalized_mode(
            request
                .mode
                .as_deref()
                .unwrap_or(&state.config.crisper.default_mode),
        )
        .into(),
    }))
}

async fn system_audio_dub(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(request): Json<SystemAudioRequest>,
) -> Result<Response, ApiError> {
    authorize(&state, &headers)?;
    let input = take_router_audio(&state, &request).await?;
    let transcript = transcribe_audio(&state, &input).await?;
    let translated = if request.translate.unwrap_or(true) {
        Some(
            translate_text(
                &state,
                &transcript.text,
                request.route.as_deref().unwrap_or("inbound"),
                transcript
                    .language
                    .as_deref()
                    .or(request.source_language.as_deref()),
                request.target_language.as_deref(),
            )
            .await?
            .0,
        )
    } else {
        None
    };
    let output_text = translated
        .clone()
        .unwrap_or_else(|| transcript.text.clone());
    let seed = request.seed.unwrap_or(state.config.longcat.seed);
    let wav = synthesize(&state, &output_text, request.voice.as_deref(), seed).await?;
    let destination = request.output.as_deref().unwrap_or("playback");
    match destination {
        "wav" => wav_response(wav, Some(&transcript.text), Some(&output_text), seed),
        "playback" | "mic" => {
            let path = if destination == "mic" {
                "/v1/forward"
            } else {
                "/v1/playback"
            };
            post_router_audio(&state, path, &wav, "vox-dub.wav").await?;
            Ok(Json(RoutedAudioResponse {
                transcript: transcript.text,
                translated_text: translated,
                output_text,
                output: destination.into(),
                seed: Some(seed),
            })
            .into_response())
        }
        other => Err(ApiError::BadRequest(format!(
            "unknown output '{other}'; use playback, mic, or wav"
        ))),
    }
}

async fn system_audio_clear(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<Value>, ApiError> {
    authorize(&state, &headers)?;
    clear_router_source(&state, "system").await
}

async fn audio_clear(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(request): Json<SystemAudioRequest>,
) -> Result<Json<Value>, ApiError> {
    authorize(&state, &headers)?;
    clear_router_source(&state, capture_source(&request)?).await
}

async fn clear_router_source(state: &AppState, source: &str) -> Result<Json<Value>, ApiError> {
    let response = state
        .client
        .post(format!(
            "{}/v1/audio/clear?source={source}",
            state.config.backends.router_url.trim_end_matches('/')
        ))
        .send()
        .await
        .map_err(backend_error)?;
    let status = response.status();
    let value = response.json::<Value>().await.map_err(backend_error)?;
    if !status.is_success() {
        return Err(ApiError::Backend(format!(
            "router returned {status}: {value}"
        )));
    }
    Ok(Json(value))
}

async fn system_audio_stream_start(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(request): Json<SystemAudioRequest>,
) -> Result<Json<SubtitleStreamSnapshot>, ApiError> {
    start_caption_stream(state, headers, "default".into(), request).await
}

async fn system_audio_stream_status(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<SubtitleStreamSnapshot>, ApiError> {
    caption_stream_status_inner(state, headers, "default".into()).await
}

async fn system_audio_stream_stop(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<SubtitleStreamSnapshot>, ApiError> {
    stop_caption_stream(state, headers, "default".into()).await
}

async fn caption_stream_start(
    State(state): State<AppState>,
    AxumPath(id): AxumPath<String>,
    headers: HeaderMap,
    Json(request): Json<SystemAudioRequest>,
) -> Result<Json<SubtitleStreamSnapshot>, ApiError> {
    start_caption_stream(state, headers, id, request).await
}

async fn caption_stream_status(
    State(state): State<AppState>,
    AxumPath(id): AxumPath<String>,
    headers: HeaderMap,
) -> Result<Json<SubtitleStreamSnapshot>, ApiError> {
    caption_stream_status_inner(state, headers, id).await
}

async fn caption_stream_stop(
    State(state): State<AppState>,
    AxumPath(id): AxumPath<String>,
    headers: HeaderMap,
) -> Result<Json<SubtitleStreamSnapshot>, ApiError> {
    stop_caption_stream(state, headers, id).await
}

async fn caption_stream_list(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<Value>, ApiError> {
    authorize(&state, &headers)?;
    let controls: Vec<_> = state
        .subtitle_streams
        .read()
        .map_err(|_| ApiError::Internal("subtitle stream map was poisoned".into()))?
        .values()
        .cloned()
        .collect();
    let mut streams = Vec::with_capacity(controls.len());
    for control in controls {
        streams.push(
            serde_json::to_value(
                control
                    .snapshot
                    .read()
                    .map_err(|_| ApiError::Internal("subtitle snapshot was poisoned".into()))?
                    .clone(),
            )
            .map_err(|error| ApiError::Internal(error.to_string()))?,
        );
    }
    Ok(Json(json!({"streams": streams})))
}

async fn start_caption_stream(
    state: AppState,
    headers: HeaderMap,
    id: String,
    mut request: SystemAudioRequest,
) -> Result<Json<SubtitleStreamSnapshot>, ApiError> {
    authorize(&state, &headers)?;
    validate_caption_id(&id)?;
    request.consumer = Some(format!("http-caption-{id}"));
    let mode = normalized_mode(
        request
            .mode
            .as_deref()
            .unwrap_or(&state.config.crisper.default_mode),
    )
    .to_string();
    let snapshot = Arc::new(RwLock::new(SubtitleStreamSnapshot {
        id: id.clone(),
        source: capture_source(&request)?.to_string(),
        running: true,
        processing: false,
        revision: 0,
        transcript: String::new(),
        translated_text: None,
        output_text: String::new(),
        mode,
        last_error: None,
        updated_unix_ms: unix_ms(),
    }));
    let task = tokio::spawn(run_system_audio_stream(
        state.clone(),
        request,
        snapshot.clone(),
    ));
    let control = SubtitleStreamControl {
        abort: task.abort_handle(),
        snapshot: snapshot.clone(),
    };
    let previous = state
        .subtitle_streams
        .write()
        .map_err(|_| ApiError::Internal("subtitle stream map was poisoned".into()))?
        .insert(id, control);
    if let Some(previous) = previous {
        previous.abort.abort();
    }
    let value = snapshot
        .read()
        .map_err(|_| ApiError::Internal("subtitle snapshot lock was poisoned".into()))?
        .clone();
    Ok(Json(value))
}

async fn caption_stream_status_inner(
    state: AppState,
    headers: HeaderMap,
    id: String,
) -> Result<Json<SubtitleStreamSnapshot>, ApiError> {
    authorize(&state, &headers)?;
    validate_caption_id(&id)?;
    let control = state
        .subtitle_streams
        .read()
        .map_err(|_| ApiError::Internal("subtitle stream map was poisoned".into()))?
        .get(&id)
        .cloned();
    let snapshot = match control {
        Some(control) => control
            .snapshot
            .read()
            .map_err(|_| ApiError::Internal("subtitle snapshot lock was poisoned".into()))?
            .clone(),
        None => SubtitleStreamSnapshot {
            id,
            source: "system".into(),
            running: false,
            processing: false,
            revision: 0,
            transcript: String::new(),
            translated_text: None,
            output_text: String::new(),
            mode: normalized_mode(&state.config.crisper.default_mode).into(),
            last_error: None,
            updated_unix_ms: unix_ms(),
        },
    };
    Ok(Json(snapshot))
}

async fn stop_caption_stream(
    state: AppState,
    headers: HeaderMap,
    id: String,
) -> Result<Json<SubtitleStreamSnapshot>, ApiError> {
    authorize(&state, &headers)?;
    validate_caption_id(&id)?;
    let control = state
        .subtitle_streams
        .read()
        .map_err(|_| ApiError::Internal("subtitle stream map was poisoned".into()))?
        .get(&id)
        .cloned();
    let Some(control) = control else {
        return caption_stream_status_inner(state, headers, id).await;
    };
    control.abort.abort();
    let value = {
        let mut snapshot = control
            .snapshot
            .write()
            .map_err(|_| ApiError::Internal("subtitle snapshot lock was poisoned".into()))?;
        snapshot.running = false;
        snapshot.processing = false;
        snapshot.updated_unix_ms = unix_ms();
        snapshot.clone()
    };
    Ok(Json(value))
}

fn validate_caption_id(id: &str) -> Result<(), ApiError> {
    if id.is_empty()
        || id.len() > 64
        || !id
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || matches!(character, '-' | '_'))
    {
        return Err(ApiError::BadRequest(
            "caption id must contain 1-64 ASCII letters, numbers, '-' or '_'".into(),
        ));
    }
    Ok(())
}

async fn run_system_audio_stream(
    state: AppState,
    request: SystemAudioRequest,
    snapshot: Arc<RwLock<SubtitleStreamSnapshot>>,
) {
    loop {
        let input = match take_router_audio(&state, &request).await {
            Ok(input) => input,
            Err(ApiError::NoContent) => {
                tokio::time::sleep(Duration::from_millis(10)).await;
                continue;
            }
            Err(error) => {
                update_stream_error(&snapshot, error.to_string());
                tokio::time::sleep(Duration::from_millis(250)).await;
                continue;
            }
        };
        if let Ok(mut value) = snapshot.write() {
            value.processing = true;
            value.last_error = None;
        }
        let transcript = match transcribe_audio(&state, &input).await {
            Ok(text) => text,
            Err(error) => {
                update_stream_error(&snapshot, error.to_string());
                continue;
            }
        };
        let translated = if request.translate.unwrap_or(false) {
            match translate_text(
                &state,
                &transcript.text,
                request.route.as_deref().unwrap_or("inbound"),
                transcript
                    .language
                    .as_deref()
                    .or(request.source_language.as_deref()),
                request.target_language.as_deref(),
            )
            .await
            {
                Ok((text, _, _)) => Some(text),
                Err(error) => {
                    update_stream_error(&snapshot, error.to_string());
                    continue;
                }
            }
        } else {
            None
        };
        let output = translated
            .clone()
            .unwrap_or_else(|| transcript.text.clone());
        if let Ok(mut value) = snapshot.write() {
            value.processing = false;
            value.revision = value.revision.saturating_add(1);
            value.transcript = transcript.text;
            value.translated_text = translated;
            value.output_text = output;
            value.last_error = None;
            value.updated_unix_ms = unix_ms();
        }
    }
}

fn update_stream_error(snapshot: &RwLock<SubtitleStreamSnapshot>, error: String) {
    if let Ok(mut value) = snapshot.write() {
        value.processing = false;
        value.last_error = Some(error);
        value.updated_unix_ms = unix_ms();
        // Deliberately preserve transcript/output_text: status polls during a
        // slow or failed next chunk continue rendering the previous caption.
    }
}

fn unix_ms() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
}

async fn take_router_audio(
    state: &AppState,
    request: &SystemAudioRequest,
) -> Result<AudioInput, ApiError> {
    let source = capture_source(request)?;
    let seconds = request.seconds.unwrap_or(1.5).clamp(0.5, 15.0);
    let default_consumer = format!("http-{source}");
    let consumer = request.consumer.as_deref().unwrap_or(&default_consumer);
    let response = state
        .client
        .get(format!(
            "{}/v1/audio/take?source={source}&min_seconds={seconds}&latest_seconds={seconds}&consumer={consumer}",
            state.config.backends.router_url.trim_end_matches('/')
        ))
        .send()
        .await
        .map_err(backend_error)?;
    if response.status() == StatusCode::NO_CONTENT {
        return Err(ApiError::NoContent);
    }
    let status = response.status();
    if !status.is_success() {
        return Err(ApiError::Backend(format!("router returned {status}")));
    }
    Ok(AudioInput {
        bytes: response.bytes().await.map_err(backend_error)?.to_vec(),
        filename: format!("{source}-audio.wav"),
        mode: request
            .mode
            .clone()
            .unwrap_or_else(|| state.config.crisper.default_mode.clone()),
        language: request.language.clone(),
        translate: request.translate.unwrap_or(false),
        route: request.route.clone().unwrap_or_else(default_route),
        source_language: request.source_language.clone(),
        target_language: request.target_language.clone(),
        voice: request.voice.clone(),
        seed: request.seed,
    })
}

fn default_capture_source() -> String {
    "system".into()
}

fn capture_source(request: &SystemAudioRequest) -> Result<&str, ApiError> {
    match request.source.trim().to_ascii_lowercase().as_str() {
        "microphone" | "mic" => Ok("microphone"),
        "system" | "system-audio" | "playback" => Ok("system"),
        other => Err(ApiError::BadRequest(format!(
            "unknown audio source '{other}'; use microphone or system"
        ))),
    }
}

async fn post_router_audio(
    state: &AppState,
    path: &str,
    bytes: &[u8],
    filename: &str,
) -> Result<(), ApiError> {
    let response = state
        .client
        .post(endpoint(&state.config.backends.router_url, path))
        .header(header::CONTENT_TYPE, audio_mime(filename))
        .body(bytes.to_vec())
        .send()
        .await
        .map_err(backend_error)?;
    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        return Err(ApiError::Backend(format!(
            "router returned {status}: {body}"
        )));
    }
    Ok(())
}

async fn parse_audio_input(
    mut multipart: Multipart,
    default_mode: &str,
) -> Result<AudioInput, ApiError> {
    let mut input = AudioInput {
        bytes: Vec::new(),
        filename: "recording.wav".into(),
        mode: default_mode.into(),
        language: None,
        translate: false,
        route: default_route(),
        source_language: None,
        target_language: None,
        voice: None,
        seed: None,
    };
    while let Some(field) = multipart
        .next_field()
        .await
        .map_err(|error| ApiError::BadRequest(error.to_string()))?
    {
        let name = field.name().unwrap_or_default().to_string();
        if name == "file" {
            input.filename = field.file_name().unwrap_or("recording.wav").to_string();
            input.bytes = field
                .bytes()
                .await
                .map_err(|error| ApiError::BadRequest(error.to_string()))?
                .to_vec();
            continue;
        }
        let value = field
            .text()
            .await
            .map_err(|error| ApiError::BadRequest(error.to_string()))?;
        match name.as_str() {
            "mode" => input.mode = value,
            "language" => input.language = Some(value),
            "translate" => {
                input.translate =
                    matches!(value.to_ascii_lowercase().as_str(), "1" | "true" | "yes")
            }
            "route" => input.route = value,
            "source_language" => input.source_language = nonempty(value),
            "target_language" => input.target_language = nonempty(value),
            "voice" => input.voice = nonempty(value),
            "seed" => {
                input.seed =
                    Some(value.parse().map_err(|_| {
                        ApiError::BadRequest("seed must be an unsigned integer".into())
                    })?)
            }
            _ => {}
        }
    }
    if input.bytes.is_empty() {
        return Err(ApiError::BadRequest(
            "multipart field 'file' is required".into(),
        ));
    }
    Ok(input)
}

async fn transcribe_audio(
    state: &AppState,
    input: &AudioInput,
) -> Result<CrisperTranscript, ApiError> {
    let cfg = &state.config.crisper;
    let options = CrisperOptions {
        base_url: state.config.backends.crisper_url.clone(),
        mitm_url: state.config.backends.translator_url.clone(),
        mitm_api_key: state.config.backends.translator_api_key.clone(),
        mode: input.mode.clone(),
        language: input
            .language
            .clone()
            .unwrap_or_else(|| cfg.language.clone()),
        candidate_max_new_tokens: cfg.candidate_max_new_tokens,
        chunk_duration: cfg.chunk_duration,
        stride: cfg.stride,
        context_words: cfg.context_words,
        max_new_tokens: cfg.max_new_tokens,
        hotwords: cfg.hotwords.clone(),
    };
    transcribe_crisper(
        &state.client,
        &input.bytes,
        &input.filename,
        audio_mime(&input.filename),
        &options,
    )
    .await
    .map_err(|error| ApiError::Backend(error.to_string()))
}

async fn translate_text(
    state: &AppState,
    text: &str,
    route_name: &str,
    source_override: Option<&str>,
    target_override: Option<&str>,
) -> Result<(String, String, String), ApiError> {
    if text.trim().is_empty() {
        return Err(ApiError::BadRequest("text is empty".into()));
    }
    let route = match route_name.to_ascii_lowercase().as_str() {
        "inbound" => &state.config.routes.inbound,
        "outbound" => &state.config.routes.outbound,
        other => return Err(ApiError::BadRequest(format!("unknown route '{other}'"))),
    };
    let source = source_override
        .unwrap_or(&route.source_language)
        .trim()
        .to_string();
    let target = target_override
        .unwrap_or(&route.target_language)
        .trim()
        .to_string();
    if source.is_empty() {
        return Err(ApiError::BadRequest("source_language is empty".into()));
    }
    if target.is_empty() {
        return Err(ApiError::BadRequest("target_language is empty".into()));
    }
    let mut request = state
        .client
        .post(endpoint(
            &state.config.backends.translator_url,
            "/translate",
        ))
        .json(&json!({
            "text": text,
            "source_language": source,
            "target_language": target,
        }));
    if !state.config.backends.translator_api_key.is_empty() {
        request = request.bearer_auth(&state.config.backends.translator_api_key);
    }
    let response = request.send().await.map_err(backend_error)?;
    let status = response.status();
    let value: Value = response.json().await.map_err(backend_error)?;
    if !status.is_success() {
        return Err(ApiError::Backend(format!(
            "translator returned {status}: {value}"
        )));
    }
    let translation = value
        .get("translation")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|text| !text.is_empty())
        .ok_or_else(|| ApiError::Backend("translator returned empty output".into()))?
        .to_string();
    Ok((translation, source, target))
}

async fn synthesize(
    state: &AppState,
    text: &str,
    voice_name: Option<&str>,
    seed: u64,
) -> Result<Vec<u8>, ApiError> {
    let cfg = &state.config.longcat;
    let voice = select_voice(state, voice_name)?;
    let reference = if let Some(voice) = voice {
        let audio = tokio::fs::read(expand_home(&voice.audio_path))
            .await
            .map_err(|error| ApiError::Internal(format!("voice '{}': {error}", voice.name)))?;
        let transcript = tokio::fs::read_to_string(expand_home(&voice.transcript_path))
            .await
            .map_err(|error| {
                ApiError::Internal(format!("voice transcript '{}': {error}", voice.name))
            })?;
        let filename = voice
            .audio_path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("reference.wav")
            .to_string();
        Some(LongCatReference {
            audio,
            filename,
            transcript,
        })
    } else {
        None
    };
    let options = LongCatOptions {
        base_url: state.config.backends.longcat_url.clone(),
        steps: cfg.steps,
        guidance_strength: cfg.guidance_strength,
        guidance_method: cfg.guidance_method.clone(),
        seed,
        duration_scale: cfg.duration_scale,
        concatenate: cfg.concatenate_chunks,
        characters_per_request: cfg.characters_per_request,
    };
    synthesize_longcat(&state.client, text, &options, reference.as_ref())
        .await
        .map_err(|error| {
            let message = error.to_string();
            if matches!(&error, LongCatError::EmptyText) {
                ApiError::BadRequest(message)
            } else {
                ApiError::Backend(message)
            }
        })
}

fn wav_response(
    wav: Vec<u8>,
    transcript: Option<&str>,
    output: Option<&str>,
    seed: u64,
) -> Result<Response, ApiError> {
    let mut response = Response::new(Body::from(wav));
    response
        .headers_mut()
        .insert(header::CONTENT_TYPE, HeaderValue::from_static("audio/wav"));
    response
        .headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    response.headers_mut().insert(
        header::CONTENT_DISPOSITION,
        HeaderValue::from_static("attachment; filename=vox-output.wav"),
    );
    response.headers_mut().insert(
        "x-vox-seed",
        HeaderValue::from_str(&seed.to_string())
            .map_err(|error| ApiError::Internal(error.to_string()))?,
    );
    if let Some(value) = transcript {
        response.headers_mut().insert(
            "x-vox-transcript-b64",
            HeaderValue::from_str(&BASE64.encode(value.as_bytes()))
                .map_err(|error| ApiError::Internal(error.to_string()))?,
        );
    }
    if let Some(value) = output {
        response.headers_mut().insert(
            "x-vox-output-text-b64",
            HeaderValue::from_str(&BASE64.encode(value.as_bytes()))
                .map_err(|error| ApiError::Internal(error.to_string()))?,
        );
    }
    Ok(response)
}

fn select_voice<'a>(
    state: &'a AppState,
    requested: Option<&str>,
) -> Result<Option<&'a VoiceConfig>, ApiError> {
    let name = requested
        .unwrap_or(&state.config.longcat.default_voice)
        .trim();
    if name.is_empty() {
        return Ok(None);
    }
    state
        .config
        .voices
        .iter()
        .find(|voice| voice.name.eq_ignore_ascii_case(name))
        .map(Some)
        .ok_or_else(|| ApiError::BadRequest(format!("unknown voice '{name}'")))
}

async fn get_backend_json(
    state: &AppState,
    base: &str,
    path: &str,
    key: &str,
) -> Result<Value, ApiError> {
    let mut request = state.client.get(endpoint(base, path));
    if !key.is_empty() {
        request = request.bearer_auth(key);
    }
    let response = request.send().await.map_err(backend_error)?;
    let status = response.status();
    let value: Value = response.json().await.map_err(backend_error)?;
    if !status.is_success() {
        return Err(ApiError::Backend(format!("{base}{path} returned {status}")));
    }
    Ok(value)
}

fn health_value(result: Result<Value, ApiError>) -> Value {
    result.unwrap_or_else(|error| json!({"ok": false, "error": error.to_string()}))
}

fn authorize(state: &AppState, headers: &HeaderMap) -> Result<(), ApiError> {
    let expected = state.config.server.api_key.trim();
    if expected.is_empty() {
        return Ok(());
    }
    let supplied = headers
        .get(header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("Bearer "))
        .unwrap_or_default();
    if supplied == expected {
        Ok(())
    } else {
        Err(ApiError::Unauthorized)
    }
}

fn validate_config(config: &Config) -> Result<(), String> {
    for (name, value) in [
        ("crisper_url", &config.backends.crisper_url),
        ("translator_url", &config.backends.translator_url),
        ("longcat_url", &config.backends.longcat_url),
        ("router_url", &config.backends.router_url),
    ] {
        let url = Url::parse(value).map_err(|error| format!("invalid {name}: {error}"))?;
        if !matches!(url.scheme(), "http" | "https") {
            return Err(format!("{name} must use http or https"));
        }
        let host = url
            .host_str()
            .ok_or_else(|| format!("{name} has no host"))?;
        if !private_host(host) {
            return Err(format!(
                "{name} must use a loopback, private, ULA/link-local, or .local host; got {host}"
            ));
        }
    }
    let mut voice_names = std::collections::HashSet::new();
    for voice in &config.voices {
        let name = voice.name.trim();
        if name.is_empty() {
            return Err("voice names must not be empty".into());
        }
        if !voice_names.insert(name.to_ascii_lowercase()) {
            return Err(format!("duplicate voice name '{name}'"));
        }
        let audio_extension = file_extension(&voice.audio_path);
        if !matches!(audio_extension.as_str(), "wav" | "mp3" | "m4a") {
            return Err(format!(
                "voice '{name}' audio_path must be a .wav, .mp3, or .m4a file"
            ));
        }
        if file_extension(&voice.transcript_path) != "txt" {
            return Err(format!(
                "voice '{name}' transcript_path must be a verbatim UTF-8 .txt file"
            ));
        }
    }
    let default_voice = config.longcat.default_voice.trim();
    if !default_voice.is_empty() && !voice_names.contains(&default_voice.to_ascii_lowercase()) {
        return Err(format!(
            "longcat.default_voice '{default_voice}' does not match a configured voice"
        ));
    }
    Ok(())
}

fn file_extension(path: &Path) -> String {
    path.extension()
        .and_then(|extension| extension.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase()
}

fn private_host(host: &str) -> bool {
    if host.eq_ignore_ascii_case("localhost") || host.to_ascii_lowercase().ends_with(".local") {
        return true;
    }
    match host.parse::<IpAddr>() {
        Ok(IpAddr::V4(ip)) => ip.is_loopback() || ip.is_private() || ip.is_link_local(),
        Ok(IpAddr::V6(ip)) => ip.is_loopback() || ipv6_ula(ip) || ipv6_link_local(ip),
        Err(_) => false,
    }
}

fn ipv6_ula(ip: Ipv6Addr) -> bool {
    ip.segments()[0] & 0xfe00 == 0xfc00
}

fn ipv6_link_local(ip: Ipv6Addr) -> bool {
    ip.segments()[0] & 0xffc0 == 0xfe80
}

fn cors_layer(origin: &str) -> Result<CorsLayer, Box<dyn std::error::Error>> {
    let layer = if origin.trim() == "*" {
        CorsLayer::new().allow_origin(Any)
    } else {
        CorsLayer::new().allow_origin(origin.parse::<HeaderValue>()?)
    };
    Ok(layer
        .allow_methods(Any)
        .allow_headers(Any)
        .expose_headers(Any))
}

fn endpoint(base: &str, path: &str) -> String {
    format!("{}{}", base.trim_end_matches('/'), path)
}

fn default_router_url() -> String {
    "http://127.0.0.1:8182".into()
}

fn backend_error(error: reqwest::Error) -> ApiError {
    ApiError::Backend(error.to_string())
}

fn audio_mime(filename: &str) -> &'static str {
    match Path::new(filename)
        .extension()
        .and_then(|extension| extension.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase()
        .as_str()
    {
        "mp3" => "audio/mpeg",
        "m4a" => "audio/mp4",
        "flac" => "audio/flac",
        "ogg" => "audio/ogg",
        "webm" => "audio/webm",
        _ => "audio/wav",
    }
}

fn expand_home(path: &Path) -> PathBuf {
    let value = path.to_string_lossy();
    if let Some(rest) = value.strip_prefix("~/") {
        if let Ok(home) = std::env::var("HOME") {
            return PathBuf::from(home).join(rest);
        }
    }
    path.to_path_buf()
}

fn nonempty(value: String) -> Option<String> {
    (!value.trim().is_empty()).then_some(value)
}

fn default_host() -> String {
    "0.0.0.0".into()
}
fn default_port() -> u16 {
    8180
}
fn default_cors() -> String {
    "*".into()
}
fn default_route() -> String {
    "inbound".into()
}
fn default_crisper_mode() -> String {
    "intended".into()
}
fn default_language() -> String {
    "auto".into()
}
fn default_chunk_duration() -> f32 {
    30.0
}
fn default_stride() -> f32 {
    26.0
}
fn default_context_words() -> u32 {
    12
}
fn default_max_tokens() -> u32 {
    256
}

fn default_candidate_max_tokens() -> u32 {
    24
}
fn default_steps() -> u32 {
    16
}
fn default_guidance_strength() -> f32 {
    4.0
}
fn default_guidance_method() -> String {
    "apg".into()
}
fn default_seed() -> u64 {
    1024
}
fn default_duration_scale() -> f32 {
    1.0
}
fn default_true() -> bool {
    true
}
fn default_characters_per_request() -> usize {
    240
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_only_private_backend_hosts() {
        assert!(private_host("127.0.0.1"));
        assert!(private_host("192.168.10.203"));
        assert!(private_host("alien.local"));
        assert!(private_host("fd00::1"));
        assert!(!private_host("example.com"));
        assert!(!private_host("8.8.8.8"));
    }

    #[test]
    fn longcat_defaults_match_desktop_sentence_flow() {
        let config = LongCatConfig::default();
        assert!(config.concatenate_chunks);
        assert_eq!(config.characters_per_request, 240);
    }

    #[test]
    fn caption_ids_are_safe_router_consumers() {
        assert!(validate_caption_id("german-to-english_1").is_ok());
        assert!(validate_caption_id("").is_err());
        assert!(validate_caption_id("has spaces").is_err());
        assert!(validate_caption_id("../shared").is_err());
    }
}
