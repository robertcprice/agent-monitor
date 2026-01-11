//! Integration layer for external applications.
//! Provides REST API, WebSocket, SSE, webhooks, and file-based integrations.

use anyhow::Result;
use axum::{
    body::Body,
    extract::{Path, Query, State},
    http::{header, StatusCode},
    response::{sse::Event, IntoResponse, Response, Sse},
    routing::{delete, get, post},
    Json, Router,
};
use chrono::{DateTime, Utc};
use futures_util::stream::Stream;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::convert::Infallible;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{broadcast, RwLock};
use tokio_stream::wrappers::BroadcastStream;
use tokio_stream::StreamExt as _;
use tracing::{error, warn};

use crate::models::{Session, SessionEvent};
use crate::storage::Storage;
use crate::analytics::RateLimiterState;

// =============================================================================
// API Types and Responses
// =============================================================================

/// Standard API response wrapper
#[derive(Debug, Serialize)]
pub struct ApiResponse<T: Serialize> {
    pub success: bool,
    pub data: Option<T>,
    pub error: Option<String>,
    pub meta: ResponseMeta,
}

#[derive(Debug, Serialize)]
pub struct ResponseMeta {
    pub timestamp: DateTime<Utc>,
    pub request_id: String,
    pub version: &'static str,
}

impl<T: Serialize> ApiResponse<T> {
    pub fn success(data: T) -> Self {
        Self {
            success: true,
            data: Some(data),
            error: None,
            meta: ResponseMeta {
                timestamp: Utc::now(),
                request_id: uuid::Uuid::new_v4().to_string(),
                version: env!("CARGO_PKG_VERSION"),
            },
        }
    }

    pub fn error(msg: &str) -> ApiResponse<()> {
        ApiResponse {
            success: false,
            data: None,
            error: Some(msg.to_string()),
            meta: ResponseMeta {
                timestamp: Utc::now(),
                request_id: uuid::Uuid::new_v4().to_string(),
                version: env!("CARGO_PKG_VERSION"),
            },
        }
    }
}

/// Paginated response
#[derive(Debug, Serialize)]
pub struct PaginatedResponse<T: Serialize> {
    pub items: Vec<T>,
    pub total: usize,
    pub page: usize,
    pub per_page: usize,
    pub total_pages: usize,
}

/// Session summary for list views
#[derive(Debug, Serialize)]
pub struct SessionSummary {
    pub id: String,
    pub agent_type: String,
    pub project_name: String,
    pub project_path: String,
    pub status: String,
    pub message_count: i64,
    pub estimated_cost: f64,
    pub started_at: DateTime<Utc>,
    pub last_activity_at: DateTime<Utc>,
    pub duration_seconds: f64,
}

impl From<&Session> for SessionSummary {
    fn from(s: &Session) -> Self {
        Self {
            id: s.id.clone(),
            agent_type: s.agent_type.to_string(),
            project_name: s.project_path.split('/').last().unwrap_or("unknown").to_string(),
            project_path: s.project_path.clone(),
            status: s.status.to_string(),
            message_count: s.message_count,
            estimated_cost: s.estimated_cost,
            started_at: s.started_at,
            last_activity_at: s.last_activity_at,
            duration_seconds: s.duration_seconds,
        }
    }
}

/// Event summary for list views
#[derive(Debug, Serialize)]
pub struct EventSummary {
    pub id: String,
    pub session_id: String,
    pub event_type: String,
    pub timestamp: DateTime<Utc>,
    pub preview: String,
    pub has_content: bool,
    pub tool_name: Option<String>,
}

impl From<&SessionEvent> for EventSummary {
    fn from(e: &SessionEvent) -> Self {
        let preview = e.content.as_ref()
            .map(|c| {
                let first_line = c.lines().next().unwrap_or("");
                if first_line.len() > 100 {
                    format!("{}...", &first_line[..100])
                } else {
                    first_line.to_string()
                }
            })
            .unwrap_or_default();

        Self {
            id: e.id.clone(),
            session_id: e.session_id.clone(),
            event_type: format!("{:?}", e.event_type),
            timestamp: e.timestamp,
            preview,
            has_content: e.content.is_some(),
            tool_name: e.tool_name.clone(),
        }
    }
}

/// Health check response
#[derive(Debug, Serialize)]
pub struct HealthResponse {
    pub status: String,
    pub version: String,
    pub uptime_seconds: u64,
    pub database_ok: bool,
    pub active_sessions: i64,
    pub total_events_24h: i64,
}

/// System info response
#[derive(Debug, Serialize)]
pub struct SystemInfo {
    pub version: String,
    pub rust_version: String,
    pub os: String,
    pub arch: String,
    pub pid: u32,
    pub started_at: DateTime<Utc>,
    pub config: ConfigInfo,
}

#[derive(Debug, Serialize)]
pub struct ConfigInfo {
    pub data_dir: String,
    pub socket_path: String,
    pub http_port: u16,
    pub rate_limit_enabled: bool,
}

/// Status file format (Ralph-compatible)
#[derive(Debug, Serialize)]
pub struct StatusFile {
    pub daemon_status: String,
    pub version: String,
    pub timestamp: DateTime<Utc>,
    pub uptime_seconds: u64,
    pub sessions: SessionsStatus,
    pub analytics: AnalyticsStatus,
}

#[derive(Debug, Serialize)]
pub struct SessionsStatus {
    pub active_count: i64,
    pub total_24h: i64,
    pub by_agent_type: HashMap<String, i64>,
}

#[derive(Debug, Serialize)]
pub struct AnalyticsStatus {
    pub total_messages: i64,
    pub total_cost: f64,
    pub rate_limit: Option<RateLimiterState>,
}

// =============================================================================
// Query Parameters
// =============================================================================

#[derive(Debug, Deserialize)]
pub struct SessionsQueryParams {
    #[serde(default = "default_page")]
    pub page: usize,
    #[serde(default = "default_per_page")]
    pub per_page: usize,
    pub agent_type: Option<String>,
    pub status: Option<String>,
    pub project: Option<String>,
    #[serde(default)]
    pub active_only: bool,
}

fn default_page() -> usize { 1 }
fn default_per_page() -> usize { 50 }

#[derive(Debug, Deserialize)]
pub struct EventsQueryParams {
    #[serde(default = "default_page")]
    pub page: usize,
    #[serde(default = "default_per_page")]
    pub per_page: usize,
    pub session_id: Option<String>,
    pub event_type: Option<String>,
    pub since: Option<DateTime<Utc>>,
    pub until: Option<DateTime<Utc>>,
}

#[derive(Debug, Deserialize)]
pub struct ExportQueryParams {
    pub format: Option<String>,  // json, csv, jsonl
    pub since: Option<DateTime<Utc>>,
    pub until: Option<DateTime<Utc>>,
    pub session_id: Option<String>,
}

// =============================================================================
// Webhook System
// =============================================================================

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WebhookConfig {
    pub id: String,
    pub url: String,
    pub events: Vec<String>,  // session_start, session_end, event, error, etc.
    pub secret: Option<String>,
    pub enabled: bool,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Serialize)]
pub struct WebhookPayload {
    pub event_type: String,
    pub timestamp: DateTime<Utc>,
    pub data: serde_json::Value,
    pub signature: Option<String>,
}

pub struct WebhookManager {
    webhooks: Arc<RwLock<Vec<WebhookConfig>>>,
    client: reqwest::Client,
}

impl WebhookManager {
    pub fn new() -> Self {
        Self {
            webhooks: Arc::new(RwLock::new(Vec::new())),
            client: reqwest::Client::builder()
                .timeout(Duration::from_secs(10))
                .build()
                .unwrap_or_default(),
        }
    }

    pub async fn register(&self, config: WebhookConfig) {
        let mut webhooks = self.webhooks.write().await;
        webhooks.push(config);
    }

    pub async fn unregister(&self, id: &str) -> bool {
        let mut webhooks = self.webhooks.write().await;
        let len_before = webhooks.len();
        webhooks.retain(|w| w.id != id);
        webhooks.len() < len_before
    }

    pub async fn list(&self) -> Vec<WebhookConfig> {
        self.webhooks.read().await.clone()
    }

    pub async fn trigger(&self, event_type: &str, data: serde_json::Value) {
        let webhooks = self.webhooks.read().await;

        for webhook in webhooks.iter() {
            if !webhook.enabled {
                continue;
            }

            if !webhook.events.contains(&event_type.to_string())
                && !webhook.events.contains(&"*".to_string()) {
                continue;
            }

            let payload = WebhookPayload {
                event_type: event_type.to_string(),
                timestamp: Utc::now(),
                data: data.clone(),
                signature: webhook.secret.as_ref().map(|s| {
                    // HMAC-SHA256 signature
                    use std::collections::hash_map::DefaultHasher;
                    use std::hash::{Hash, Hasher};
                    let mut hasher = DefaultHasher::new();
                    s.hash(&mut hasher);
                    data.to_string().hash(&mut hasher);
                    format!("sha256={:016x}", hasher.finish())
                }),
            };

            let url = webhook.url.clone();
            let client = self.client.clone();
            let event_type_owned = event_type.to_string();

            tokio::spawn(async move {
                match client
                    .post(&url)
                    .json(&payload)
                    .header("Content-Type", "application/json")
                    .header("X-Webhook-Event", event_type_owned)
                    .send()
                    .await
                {
                    Ok(resp) => {
                        if !resp.status().is_success() {
                            warn!("Webhook {} returned status {}", url, resp.status());
                        }
                    }
                    Err(e) => {
                        error!("Webhook {} failed: {}", url, e);
                    }
                }
            });
        }
    }
}

impl Default for WebhookManager {
    fn default() -> Self {
        Self::new()
    }
}

// =============================================================================
// Status File Writer
// =============================================================================

pub struct StatusFileWriter {
    path: PathBuf,
    storage: Storage,
    started_at: DateTime<Utc>,
}

impl StatusFileWriter {
    pub fn new(path: PathBuf, storage: Storage) -> Self {
        Self {
            path,
            storage,
            started_at: Utc::now(),
        }
    }

    pub async fn write_status(&self) -> Result<()> {
        let metrics = self.storage.get_summary_metrics(24).await?;
        let sessions = self.storage.get_active_sessions(100).await?;

        // Count by agent type
        let mut by_agent_type: HashMap<String, i64> = HashMap::new();
        for session in &sessions {
            *by_agent_type.entry(session.agent_type.to_string()).or_insert(0) += 1;
        }

        let status = StatusFile {
            daemon_status: "running".to_string(),
            version: env!("CARGO_PKG_VERSION").to_string(),
            timestamp: Utc::now(),
            uptime_seconds: (Utc::now() - self.started_at).num_seconds() as u64,
            sessions: SessionsStatus {
                active_count: metrics.active_sessions,
                total_24h: metrics.total_sessions,
                by_agent_type,
            },
            analytics: AnalyticsStatus {
                total_messages: metrics.total_messages,
                total_cost: metrics.total_cost,
                rate_limit: None,
            },
        };

        let json = serde_json::to_string_pretty(&status)?;
        tokio::fs::write(&self.path, json).await?;

        Ok(())
    }

    pub async fn start_periodic_updates(self: Arc<Self>, interval_secs: u64) {
        let mut interval = tokio::time::interval(Duration::from_secs(interval_secs));

        loop {
            interval.tick().await;
            if let Err(e) = self.write_status().await {
                error!("Failed to write status file: {}", e);
            }
        }
    }
}

// =============================================================================
// Integration App State
// =============================================================================

#[derive(Clone)]
pub struct IntegrationState {
    pub storage: Storage,
    pub event_tx: broadcast::Sender<SessionEvent>,
    pub webhook_manager: Arc<WebhookManager>,
    pub started_at: DateTime<Utc>,
    pub api_keys: Arc<RwLock<HashMap<String, ApiKeyInfo>>>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ApiKeyInfo {
    pub name: String,
    pub created_at: DateTime<Utc>,
    pub last_used: Option<DateTime<Utc>>,
    pub permissions: Vec<String>,
}

impl IntegrationState {
    pub fn new(storage: Storage) -> Self {
        let (event_tx, _) = broadcast::channel(1000);

        Self {
            storage,
            event_tx,
            webhook_manager: Arc::new(WebhookManager::new()),
            started_at: Utc::now(),
            api_keys: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    pub async fn add_api_key(&self, key: String, info: ApiKeyInfo) {
        self.api_keys.write().await.insert(key, info);
    }

    pub async fn validate_api_key(&self, key: &str) -> bool {
        self.api_keys.read().await.contains_key(key)
    }

    pub async fn uptime_seconds(&self) -> u64 {
        (Utc::now() - self.started_at).num_seconds() as u64
    }
}

// =============================================================================
// API Handlers
// =============================================================================

/// Health check endpoint
pub async fn health_handler(State(state): State<IntegrationState>) -> Json<ApiResponse<HealthResponse>> {
    let metrics = state.storage.get_summary_metrics(24).await.ok();
    let events_24h = state.storage.get_recent_events(1).await.map(|e| e.len() as i64).unwrap_or(0);

    Json(ApiResponse::success(HealthResponse {
        status: "healthy".to_string(),
        version: env!("CARGO_PKG_VERSION").to_string(),
        uptime_seconds: state.uptime_seconds().await,
        database_ok: metrics.is_some(),
        active_sessions: metrics.as_ref().map(|m| m.active_sessions).unwrap_or(0),
        total_events_24h: events_24h,
    }))
}

/// System info endpoint
pub async fn info_handler(State(state): State<IntegrationState>) -> Json<ApiResponse<SystemInfo>> {
    Json(ApiResponse::success(SystemInfo {
        version: env!("CARGO_PKG_VERSION").to_string(),
        rust_version: env!("CARGO_PKG_RUST_VERSION").to_string(),
        os: std::env::consts::OS.to_string(),
        arch: std::env::consts::ARCH.to_string(),
        pid: std::process::id(),
        started_at: state.started_at,
        config: ConfigInfo {
            data_dir: dirs::data_dir()
                .map(|p| p.join("agent-monitor").to_string_lossy().to_string())
                .unwrap_or_default(),
            socket_path: "/tmp/agent-monitor.sock".to_string(),
            http_port: 8765,
            rate_limit_enabled: false,
        },
    }))
}

/// List sessions with pagination and filtering
pub async fn list_sessions_handler(
    State(state): State<IntegrationState>,
    Query(params): Query<SessionsQueryParams>,
) -> Json<ApiResponse<PaginatedResponse<SessionSummary>>> {
    let all_sessions = if params.active_only {
        state.storage.get_active_sessions(1000).await
    } else {
        state.storage.get_recent_sessions(168, 1000).await
    };

    match all_sessions {
        Ok(sessions) => {
            // Apply filters
            let filtered: Vec<_> = sessions.iter()
                .filter(|s| {
                    params.agent_type.as_ref().map(|t| s.agent_type.to_string() == *t).unwrap_or(true)
                })
                .filter(|s| {
                    params.status.as_ref().map(|st| s.status.to_string() == *st).unwrap_or(true)
                })
                .filter(|s| {
                    params.project.as_ref().map(|p| s.project_path.contains(p)).unwrap_or(true)
                })
                .collect();

            let total = filtered.len();
            let total_pages = (total + params.per_page - 1) / params.per_page;
            let start = (params.page - 1) * params.per_page;
            let items: Vec<SessionSummary> = filtered
                .into_iter()
                .skip(start)
                .take(params.per_page)
                .map(|s| s.into())
                .collect();

            Json(ApiResponse::success(PaginatedResponse {
                items,
                total,
                page: params.page,
                per_page: params.per_page,
                total_pages,
            }))
        }
        Err(e) => Json(ApiResponse::success(PaginatedResponse {
            items: vec![],
            total: 0,
            page: 1,
            per_page: params.per_page,
            total_pages: 0,
        })),
    }
}

/// Get single session with full details
pub async fn get_session_handler(
    State(state): State<IntegrationState>,
    Path(session_id): Path<String>,
) -> impl IntoResponse {
    match state.storage.get_session(&session_id).await {
        Ok(Some(session)) => Json(ApiResponse::success(session)).into_response(),
        Ok(None) => (
            StatusCode::NOT_FOUND,
            Json(ApiResponse::<()>::error("Session not found")),
        ).into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ApiResponse::<()>::error(&e.to_string())),
        ).into_response(),
    }
}

/// Get events for a session
pub async fn get_session_events_handler(
    State(state): State<IntegrationState>,
    Path(session_id): Path<String>,
    Query(params): Query<EventsQueryParams>,
) -> Json<ApiResponse<PaginatedResponse<EventSummary>>> {
    match state.storage.get_session_events(&session_id, 1000).await {
        Ok(events) => {
            let total = events.len();
            let total_pages = (total + params.per_page - 1) / params.per_page;
            let start = (params.page - 1) * params.per_page;
            let items: Vec<EventSummary> = events
                .iter()
                .skip(start)
                .take(params.per_page)
                .map(|e| e.into())
                .collect();

            Json(ApiResponse::success(PaginatedResponse {
                items,
                total,
                page: params.page,
                per_page: params.per_page,
                total_pages,
            }))
        }
        Err(_) => Json(ApiResponse::success(PaginatedResponse {
            items: vec![],
            total: 0,
            page: 1,
            per_page: params.per_page,
            total_pages: 0,
        })),
    }
}

/// Get single event with full content
pub async fn get_event_handler(
    State(state): State<IntegrationState>,
    Path(event_id): Path<String>,
) -> impl IntoResponse {
    // Get all recent events and find by ID
    match state.storage.get_recent_events(10000).await {
        Ok(events) => {
            if let Some(event) = events.into_iter().find(|e| e.id == event_id) {
                Json(ApiResponse::success(event)).into_response()
            } else {
                (
                    StatusCode::NOT_FOUND,
                    Json(ApiResponse::<()>::error("Event not found")),
                ).into_response()
            }
        }
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ApiResponse::<()>::error(&e.to_string())),
        ).into_response(),
    }
}

/// List all events with pagination
pub async fn list_events_handler(
    State(state): State<IntegrationState>,
    Query(params): Query<EventsQueryParams>,
) -> Json<ApiResponse<PaginatedResponse<EventSummary>>> {
    let limit = params.per_page * 10; // Get more for filtering

    match state.storage.get_recent_events(limit).await {
        Ok(events) => {
            // Apply filters
            let filtered: Vec<_> = events.iter()
                .filter(|e| {
                    params.session_id.as_ref().map(|id| &e.session_id == id).unwrap_or(true)
                })
                .filter(|e| {
                    params.event_type.as_ref()
                        .map(|t| format!("{:?}", e.event_type).to_lowercase() == t.to_lowercase())
                        .unwrap_or(true)
                })
                .filter(|e| {
                    params.since.map(|s| e.timestamp >= s).unwrap_or(true)
                })
                .filter(|e| {
                    params.until.map(|u| e.timestamp <= u).unwrap_or(true)
                })
                .collect();

            let total = filtered.len();
            let total_pages = (total + params.per_page - 1) / params.per_page;
            let start = (params.page - 1) * params.per_page;
            let items: Vec<EventSummary> = filtered
                .into_iter()
                .skip(start)
                .take(params.per_page)
                .map(|e| e.into())
                .collect();

            Json(ApiResponse::success(PaginatedResponse {
                items,
                total,
                page: params.page,
                per_page: params.per_page,
                total_pages,
            }))
        }
        Err(_) => Json(ApiResponse::success(PaginatedResponse {
            items: vec![],
            total: 0,
            page: 1,
            per_page: params.per_page,
            total_pages: 0,
        })),
    }
}

/// Export data in various formats
pub async fn export_handler(
    State(state): State<IntegrationState>,
    Query(params): Query<ExportQueryParams>,
) -> impl IntoResponse {
    let format = params.format.as_deref().unwrap_or("json");

    let sessions = state.storage.get_recent_sessions(168, 1000).await.unwrap_or_default();
    let events = if let Some(ref sid) = params.session_id {
        state.storage.get_session_events(sid, 10000).await.unwrap_or_default()
    } else {
        state.storage.get_recent_events(10000).await.unwrap_or_default()
    };

    match format {
        "csv" => {
            let mut csv = String::from("timestamp,session_id,event_type,content_preview\n");
            for event in &events {
                let preview = event.content.as_ref()
                    .map(|c| c.lines().next().unwrap_or("").replace(",", ";").replace("\n", " "))
                    .unwrap_or_default();
                csv.push_str(&format!(
                    "{},{},{:?},{}\n",
                    event.timestamp.to_rfc3339(),
                    event.session_id,
                    event.event_type,
                    preview.chars().take(100).collect::<String>()
                ));
            }

            Response::builder()
                .header(header::CONTENT_TYPE, "text/csv")
                .header(header::CONTENT_DISPOSITION, "attachment; filename=\"events.csv\"")
                .body(Body::from(csv))
                .unwrap()
                .into_response()
        }
        "jsonl" => {
            let lines: Vec<String> = events.iter()
                .map(|e| serde_json::to_string(e).unwrap_or_default())
                .collect();

            Response::builder()
                .header(header::CONTENT_TYPE, "application/jsonl")
                .header(header::CONTENT_DISPOSITION, "attachment; filename=\"events.jsonl\"")
                .body(Body::from(lines.join("\n")))
                .unwrap()
                .into_response()
        }
        _ => {
            // JSON (default)
            let export = serde_json::json!({
                "exported_at": Utc::now().to_rfc3339(),
                "sessions": sessions,
                "events": events,
            });

            Response::builder()
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(serde_json::to_string_pretty(&export).unwrap_or_default()))
                .unwrap()
                .into_response()
        }
    }
}

/// Server-Sent Events stream for real-time updates
pub async fn sse_handler(
    State(state): State<IntegrationState>,
) -> Sse<impl Stream<Item = Result<Event, Infallible>>> {
    let rx = state.event_tx.subscribe();

    let stream = BroadcastStream::new(rx)
        .filter_map(|result| {
            match result {
                Ok(event) => {
                    let data = serde_json::to_string(&EventSummary::from(&event)).ok()?;
                    Some(Ok(Event::default()
                        .event("event")
                        .data(data)))
                }
                Err(_) => None,
            }
        });

    Sse::new(stream).keep_alive(
        axum::response::sse::KeepAlive::new()
            .interval(Duration::from_secs(30))
            .text("keep-alive"),
    )
}

/// Register a webhook
pub async fn register_webhook_handler(
    State(state): State<IntegrationState>,
    Json(config): Json<WebhookConfig>,
) -> impl IntoResponse {
    state.webhook_manager.register(config.clone()).await;
    Json(ApiResponse::success(config))
}

/// List webhooks
pub async fn list_webhooks_handler(
    State(state): State<IntegrationState>,
) -> Json<ApiResponse<Vec<WebhookConfig>>> {
    let webhooks = state.webhook_manager.list().await;
    Json(ApiResponse::success(webhooks))
}

/// Delete a webhook
pub async fn delete_webhook_handler(
    State(state): State<IntegrationState>,
    Path(webhook_id): Path<String>,
) -> impl IntoResponse {
    if state.webhook_manager.unregister(&webhook_id).await {
        (StatusCode::OK, Json(ApiResponse::success(serde_json::json!({"deleted": true}))))
    } else {
        (StatusCode::NOT_FOUND, Json(ApiResponse::success(serde_json::json!({"error": "Webhook not found", "deleted": false}))))
    }
}

/// Get current status (Ralph-compatible)
pub async fn status_handler(
    State(state): State<IntegrationState>,
) -> Json<StatusFile> {
    let metrics = state.storage.get_summary_metrics(24).await.ok();
    let sessions = state.storage.get_active_sessions(100).await.unwrap_or_default();

    let mut by_agent_type: HashMap<String, i64> = HashMap::new();
    for session in &sessions {
        *by_agent_type.entry(session.agent_type.to_string()).or_insert(0) += 1;
    }

    Json(StatusFile {
        daemon_status: "running".to_string(),
        version: env!("CARGO_PKG_VERSION").to_string(),
        timestamp: Utc::now(),
        uptime_seconds: state.uptime_seconds().await,
        sessions: SessionsStatus {
            active_count: metrics.as_ref().map(|m| m.active_sessions).unwrap_or(0),
            total_24h: metrics.as_ref().map(|m| m.total_sessions).unwrap_or(0),
            by_agent_type,
        },
        analytics: AnalyticsStatus {
            total_messages: metrics.as_ref().map(|m| m.total_messages).unwrap_or(0),
            total_cost: metrics.as_ref().map(|m| m.total_cost).unwrap_or(0.0),
            rate_limit: None,
        },
    })
}

// =============================================================================
// Router Builder
// =============================================================================

/// Create the full integration API router
pub fn create_integration_router(state: IntegrationState) -> Router {
    Router::new()
        // Health and info
        .route("/health", get(health_handler))
        .route("/info", get(info_handler))
        .route("/status", get(status_handler))

        // Sessions
        .route("/api/v1/sessions", get(list_sessions_handler))
        .route("/api/v1/sessions/:id", get(get_session_handler))
        .route("/api/v1/sessions/:id/events", get(get_session_events_handler))

        // Events
        .route("/api/v1/events", get(list_events_handler))
        .route("/api/v1/events/:id", get(get_event_handler))

        // Export
        .route("/api/v1/export", get(export_handler))

        // Real-time
        .route("/api/v1/stream", get(sse_handler))

        // Webhooks
        .route("/api/v1/webhooks", get(list_webhooks_handler).post(register_webhook_handler))
        .route("/api/v1/webhooks/:id", delete(delete_webhook_handler))

        .with_state(state)
}

// =============================================================================
// OpenAPI Documentation
// =============================================================================

pub const OPENAPI_SPEC: &str = r#"
openapi: 3.0.3
info:
  title: Agent Monitor API
  description: |
    REST API for monitoring AI agent sessions (Claude Code, Cursor, Aider, etc.)

    ## Authentication
    Use API key in the `X-API-Key` header for authenticated endpoints.

    ## Real-time Updates
    - WebSocket: Connect to `/api/ws` for bidirectional communication
    - SSE: Connect to `/api/v1/stream` for server-sent events

    ## Webhooks
    Register webhooks to receive push notifications for events.
  version: 0.1.0
  contact:
    name: Agent Monitor
servers:
  - url: http://localhost:8765
    description: Local daemon

paths:
  /health:
    get:
      summary: Health check
      tags: [System]
      responses:
        '200':
          description: Service health status

  /info:
    get:
      summary: System information
      tags: [System]
      responses:
        '200':
          description: System info including version and config

  /status:
    get:
      summary: Status file (Ralph-compatible)
      tags: [System]
      responses:
        '200':
          description: Current daemon status

  /api/v1/sessions:
    get:
      summary: List sessions
      tags: [Sessions]
      parameters:
        - name: page
          in: query
          schema:
            type: integer
            default: 1
        - name: per_page
          in: query
          schema:
            type: integer
            default: 50
        - name: agent_type
          in: query
          schema:
            type: string
        - name: status
          in: query
          schema:
            type: string
        - name: active_only
          in: query
          schema:
            type: boolean
      responses:
        '200':
          description: Paginated list of sessions

  /api/v1/sessions/{id}:
    get:
      summary: Get session details
      tags: [Sessions]
      parameters:
        - name: id
          in: path
          required: true
          schema:
            type: string
      responses:
        '200':
          description: Session details
        '404':
          description: Session not found

  /api/v1/sessions/{id}/events:
    get:
      summary: Get session events
      tags: [Sessions]
      parameters:
        - name: id
          in: path
          required: true
          schema:
            type: string
      responses:
        '200':
          description: Paginated list of events

  /api/v1/events:
    get:
      summary: List all events
      tags: [Events]
      parameters:
        - name: session_id
          in: query
          schema:
            type: string
        - name: event_type
          in: query
          schema:
            type: string
        - name: since
          in: query
          schema:
            type: string
            format: date-time
      responses:
        '200':
          description: Paginated list of events

  /api/v1/events/{id}:
    get:
      summary: Get event details
      tags: [Events]
      responses:
        '200':
          description: Event with full content

  /api/v1/export:
    get:
      summary: Export data
      tags: [Export]
      parameters:
        - name: format
          in: query
          schema:
            type: string
            enum: [json, csv, jsonl]
            default: json
      responses:
        '200':
          description: Exported data

  /api/v1/stream:
    get:
      summary: Server-Sent Events stream
      tags: [Real-time]
      responses:
        '200':
          description: SSE stream of events

  /api/v1/webhooks:
    get:
      summary: List webhooks
      tags: [Webhooks]
      responses:
        '200':
          description: List of registered webhooks
    post:
      summary: Register webhook
      tags: [Webhooks]
      requestBody:
        required: true
        content:
          application/json:
            schema:
              type: object
              properties:
                url:
                  type: string
                events:
                  type: array
                  items:
                    type: string
      responses:
        '200':
          description: Webhook registered

  /api/v1/webhooks/{id}:
    delete:
      summary: Delete webhook
      tags: [Webhooks]
      responses:
        '200':
          description: Webhook deleted
"#;

/// Serve OpenAPI spec
pub async fn openapi_handler() -> impl IntoResponse {
    Response::builder()
        .header(header::CONTENT_TYPE, "application/yaml")
        .body(Body::from(OPENAPI_SPEC))
        .unwrap()
}
