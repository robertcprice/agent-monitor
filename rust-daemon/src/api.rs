//! API endpoints for web dashboard and IPC server.

use anyhow::Result;
use axum::{
    extract::{
        ws::{Message, WebSocket, WebSocketUpgrade},
        Path, Query, State,
    },
    response::{Html, IntoResponse},
    routing::get,
    Json, Router,
};
use futures_util::{SinkExt, StreamExt};
use serde::Deserialize;
use std::net::SocketAddr;
use std::path::PathBuf;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::{UnixListener, UnixStream};
use tokio::sync::broadcast;
use tower_http::cors::CorsLayer;
use tracing::{error, info, debug};

use crate::storage::Storage;
use crate::integrations::{IntegrationState, create_integration_router, openapi_handler};

/// IPC Server using Unix sockets.
pub struct IpcServer {
    socket_path: PathBuf,
    storage: Storage,
}

impl IpcServer {
    /// Create a new IPC server.
    pub fn new(socket_path: &PathBuf, storage: Storage) -> Self {
        Self {
            socket_path: socket_path.clone(),
            storage,
        }
    }

    /// Run the IPC server.
    pub async fn run(&self) -> Result<()> {
        // Remove existing socket
        if self.socket_path.exists() {
            std::fs::remove_file(&self.socket_path)?;
        }

        let listener = UnixListener::bind(&self.socket_path)?;
        info!("IPC server listening at {:?}", self.socket_path);

        loop {
            match listener.accept().await {
                Ok((stream, _)) => {
                    let storage = self.storage.clone();
                    tokio::spawn(async move {
                        if let Err(e) = handle_client(stream, storage).await {
                            error!("Client error: {}", e);
                        }
                    });
                }
                Err(e) => {
                    error!("Accept error: {}", e);
                }
            }
        }
    }
}

async fn handle_client(stream: UnixStream, storage: Storage) -> Result<()> {
    let (reader, mut writer) = stream.into_split();
    let mut reader = BufReader::new(reader);
    let mut line = String::new();

    while reader.read_line(&mut line).await? > 0 {
        let request: serde_json::Value = serde_json::from_str(&line)?;
        let action = request.get("action").and_then(|v| v.as_str()).unwrap_or("");

        let response = match action {
            "get_sessions" => {
                let sessions = storage.get_active_sessions(100).await?;
                serde_json::json!({ "sessions": sessions })
            }
            "get_metrics" => {
                let metrics = storage.get_summary_metrics(24).await?;
                serde_json::json!({ "metrics": metrics })
            }
            "get_events" => {
                let events = storage.get_recent_events(50).await?;
                serde_json::json!({ "events": events })
            }
            _ => {
                serde_json::json!({ "error": format!("Unknown action: {}", action) })
            }
        };

        let response_str = serde_json::to_string(&response)? + "\n";
        writer.write_all(response_str.as_bytes()).await?;
        line.clear();
    }

    Ok(())
}

/// Application state for web server.
#[derive(Clone)]
pub struct AppState {
    pub storage: Storage,
    /// Broadcast channel for real-time updates
    pub update_tx: broadcast::Sender<String>,
}

/// Query parameters for sessions endpoint.
#[derive(Deserialize)]
pub struct SessionsQuery {
    #[serde(default = "default_limit")]
    limit: usize,
    #[serde(default)]
    active_only: bool,
}

fn default_limit() -> usize {
    50
}

/// Query parameters for metrics endpoint.
#[derive(Deserialize)]
pub struct MetricsQuery {
    #[serde(default = "default_hours")]
    hours: i64,
}

fn default_hours() -> i64 {
    24
}

/// Run the web server.
pub async fn run_web_server(host: &str, port: u16, storage: Storage) -> Result<()> {
    // Create broadcast channel for real-time updates
    let (update_tx, _) = broadcast::channel::<String>(100);

    let state = AppState {
        storage: storage.clone(),
        update_tx: update_tx.clone(),
    };

    // Create integration state for the new v1 API
    let integration_state = IntegrationState::new(storage.clone());
    let integration_router = create_integration_router(integration_state);

    // Build main app router with state
    let main_router = Router::new()
        .route("/", get(index_handler))
        .route("/api/sessions", get(sessions_handler))
        .route("/api/sessions/:id", get(session_handler))
        .route("/api/metrics/summary", get(metrics_handler))
        .route("/api/events", get(events_handler))
        .route("/api/ws", get(websocket_handler))
        .route("/openapi.yaml", get(openapi_handler))
        .with_state(state);

    // Merge integration router (has its own state already applied)
    let app = Router::new()
        .merge(main_router)
        .merge(integration_router)
        .layer(CorsLayer::permissive());

    // Start periodic broadcast of updates
    let broadcast_storage = storage.clone();
    let broadcast_tx = update_tx.clone();
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(tokio::time::Duration::from_secs(5));
        loop {
            interval.tick().await;
            if let Ok(sessions) = broadcast_storage.get_active_sessions(50).await {
                if let Ok(metrics) = broadcast_storage.get_summary_metrics(24).await {
                    let update = serde_json::json!({
                        "type": "update",
                        "sessions": sessions,
                        "metrics": metrics,
                        "timestamp": chrono::Utc::now().to_rfc3339(),
                    });
                    let _ = broadcast_tx.send(serde_json::to_string(&update).unwrap_or_default());
                }
            }
        }
    });

    let addr: SocketAddr = format!("{}:{}", host, port).parse()?;
    info!("Web server listening on http://{}", addr);

    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, app).await?;

    Ok(())
}

/// WebSocket upgrade handler.
async fn websocket_handler(
    ws: WebSocketUpgrade,
    State(state): State<AppState>,
) -> impl IntoResponse {
    ws.on_upgrade(|socket| handle_websocket(socket, state))
}

/// Handle WebSocket connection.
async fn handle_websocket(socket: WebSocket, state: AppState) {
    let (mut sender, mut receiver) = socket.split();

    // Subscribe to updates
    let mut rx = state.update_tx.subscribe();

    // Send initial data
    if let Ok(sessions) = state.storage.get_active_sessions(50).await {
        if let Ok(metrics) = state.storage.get_summary_metrics(24).await {
            let initial = serde_json::json!({
                "type": "initial",
                "sessions": sessions,
                "metrics": metrics,
                "timestamp": chrono::Utc::now().to_rfc3339(),
            });
            let _ = sender.send(Message::Text(serde_json::to_string(&initial).unwrap_or_default().into())).await;
        }
    }

    debug!("WebSocket client connected");

    // Handle bidirectional communication
    loop {
        tokio::select! {
            // Broadcast updates to client
            msg = rx.recv() => {
                match msg {
                    Ok(update) => {
                        if sender.send(Message::Text(update.into())).await.is_err() {
                            break;
                        }
                    }
                    Err(_) => break,
                }
            }
            // Handle incoming messages from client
            msg = receiver.next() => {
                match msg {
                    Some(Ok(Message::Text(text))) => {
                        // Handle client commands
                        if let Ok(cmd) = serde_json::from_str::<serde_json::Value>(&text) {
                            let action = cmd.get("action").and_then(|v| v.as_str()).unwrap_or("");
                            match action {
                                "refresh" => {
                                    if let Ok(sessions) = state.storage.get_active_sessions(50).await {
                                        if let Ok(metrics) = state.storage.get_summary_metrics(24).await {
                                            let update = serde_json::json!({
                                                "type": "refresh",
                                                "sessions": sessions,
                                                "metrics": metrics,
                                            });
                                            let _ = sender.send(Message::Text(
                                                serde_json::to_string(&update).unwrap_or_default().into()
                                            )).await;
                                        }
                                    }
                                }
                                "ping" => {
                                    let _ = sender.send(Message::Text(
                                        r#"{"type":"pong"}"#.into()
                                    )).await;
                                }
                                _ => {}
                            }
                        }
                    }
                    Some(Ok(Message::Ping(data))) => {
                        let _ = sender.send(Message::Pong(data)).await;
                    }
                    Some(Ok(Message::Close(_))) | None => break,
                    _ => {}
                }
            }
        }
    }

    debug!("WebSocket client disconnected");
}

/// Index handler - serve HTML dashboard.
async fn index_handler() -> Html<&'static str> {
    Html(DASHBOARD_HTML)
}

/// Sessions handler.
async fn sessions_handler(
    State(state): State<AppState>,
    Query(query): Query<SessionsQuery>,
) -> impl IntoResponse {
    let sessions = if query.active_only {
        state.storage.get_active_sessions(query.limit).await
    } else {
        state.storage.get_recent_sessions(168, query.limit).await
    };

    match sessions {
        Ok(sessions) => Json(serde_json::json!({
            "sessions": sessions,
            "total": sessions.len(),
        })),
        Err(e) => Json(serde_json::json!({ "error": e.to_string() })),
    }
}

/// Single session handler.
async fn session_handler(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    // TODO: Implement get_session by ID
    Json(serde_json::json!({ "error": "Not implemented" }))
}

/// Metrics handler.
async fn metrics_handler(
    State(state): State<AppState>,
    Query(query): Query<MetricsQuery>,
) -> impl IntoResponse {
    match state.storage.get_summary_metrics(query.hours).await {
        Ok(metrics) => Json(serde_json::json!({ "metrics": metrics })),
        Err(e) => Json(serde_json::json!({ "error": e.to_string() })),
    }
}

/// Events handler.
async fn events_handler(State(state): State<AppState>) -> impl IntoResponse {
    match state.storage.get_recent_events(50).await {
        Ok(events) => Json(serde_json::json!({
            "events": events,
            "total": events.len(),
        })),
        Err(e) => Json(serde_json::json!({ "error": e.to_string() })),
    }
}

/// HTML Dashboard with WebSocket real-time updates.
const DASHBOARD_HTML: &str = r#"
<!DOCTYPE html>
<html lang="en">
<head>
    <meta charset="UTF-8">
    <meta name="viewport" content="width=device-width, initial-scale=1.0">
    <title>Agent Monitor ✦ Cosmic Dashboard</title>
    <style>
        :root {
            --aurora-blue: #7AC9FF;
            --cosmic-violet: #BFA6FF;
            --stellar-white: #FFFFFF;
            --pulse-cyan: #00D4FF;
            --deep-space: #0a0e14;
            --nebula-dark: #121820;
            --galaxy-border: #1e2832;
        }
        * { margin: 0; padding: 0; box-sizing: border-box; }
        body {
            font-family: 'SF Mono', 'Menlo', 'Monaco', 'Courier New', monospace;
            background: var(--deep-space);
            color: var(--stellar-white);
            min-height: 100vh;
            padding: 20px;
        }
        .container { max-width: 1400px; margin: 0 auto; }
        .header {
            display: flex;
            align-items: center;
            justify-content: space-between;
            margin-bottom: 30px;
            padding-bottom: 20px;
            border-bottom: 1px solid var(--galaxy-border);
        }
        h1 {
            color: var(--aurora-blue);
            font-size: 24px;
            display: flex;
            align-items: center;
            gap: 10px;
        }
        .starfield { color: #444; font-size: 12px; }
        .connection-status {
            display: flex;
            align-items: center;
            gap: 8px;
            font-size: 12px;
        }
        .status-dot {
            width: 8px;
            height: 8px;
            border-radius: 50%;
            animation: pulse 2s infinite;
        }
        .status-dot.connected { background: var(--pulse-cyan); }
        .status-dot.disconnected { background: #f74c00; animation: none; }
        @keyframes pulse {
            0%, 100% { opacity: 1; }
            50% { opacity: 0.5; }
        }
        .card {
            background: var(--nebula-dark);
            border: 1px solid var(--galaxy-border);
            border-radius: 12px;
            padding: 20px;
            margin-bottom: 20px;
            position: relative;
            overflow: hidden;
        }
        .card::before {
            content: '';
            position: absolute;
            top: 0;
            left: 0;
            right: 0;
            height: 2px;
            background: linear-gradient(90deg, var(--aurora-blue), var(--cosmic-violet));
        }
        .card h2 {
            font-size: 12px;
            text-transform: uppercase;
            letter-spacing: 1px;
            margin-bottom: 15px;
            color: var(--cosmic-violet);
        }
        .metric {
            font-size: 36px;
            font-weight: 700;
            color: var(--aurora-blue);
        }
        .metric-label { font-size: 11px; color: #666; margin-top: 4px; }
        .grid { display: grid; grid-template-columns: repeat(4, 1fr); gap: 20px; }
        @media (max-width: 900px) { .grid { grid-template-columns: repeat(2, 1fr); } }
        .session-item {
            padding: 15px;
            background: rgba(122, 201, 255, 0.05);
            border: 1px solid var(--galaxy-border);
            border-radius: 8px;
            margin-bottom: 10px;
            transition: all 0.2s;
        }
        .session-item:hover {
            background: rgba(122, 201, 255, 0.1);
            border-color: var(--aurora-blue);
        }
        .session-header { display: flex; justify-content: space-between; align-items: center; }
        .session-name { font-weight: 600; color: var(--stellar-white); }
        .session-type {
            font-size: 11px;
            padding: 2px 8px;
            border-radius: 4px;
            background: rgba(191, 166, 255, 0.2);
            color: var(--cosmic-violet);
        }
        .session-meta {
            font-size: 12px;
            color: #666;
            margin-top: 8px;
            display: flex;
            gap: 15px;
        }
        .status-active { color: var(--pulse-cyan); }
        .status-completed { color: #666; }
        .status-crashed { color: #f74c00; }
        .rust-badge {
            background: linear-gradient(135deg, #f74c00, #ff6b35);
            color: white;
            padding: 4px 10px;
            border-radius: 6px;
            font-size: 11px;
            font-weight: 600;
        }
        .update-flash {
            animation: flash 0.5s ease-out;
        }
        @keyframes flash {
            0% { background: rgba(122, 201, 255, 0.3); }
            100% { background: transparent; }
        }
    </style>
</head>
<body>
    <div class="container">
        <div class="header">
            <h1>
                <span>✦</span>
                Agent Monitor
                <span class="rust-badge">🦀 Rust</span>
            </h1>
            <div class="connection-status">
                <div class="status-dot disconnected" id="ws-status"></div>
                <span id="ws-label">Connecting...</span>
            </div>
        </div>
        <div class="starfield">✦   ⋆  ★    ✧  ✶    ★   ⋆  ✦     ⋆   ★       ✧    ✶   ⋆  ★      ✦</div>

        <div class="grid">
            <div class="card">
                <h2>● Active Sessions</h2>
                <div class="metric" id="active-sessions">-</div>
                <div class="metric-label">Currently running</div>
            </div>
            <div class="card">
                <h2>◆ Total Messages</h2>
                <div class="metric" id="total-messages">-</div>
                <div class="metric-label">Last 24 hours</div>
            </div>
            <div class="card">
                <h2>⚡ Tool Calls</h2>
                <div class="metric" id="total-tools">-</div>
                <div class="metric-label">Last 24 hours</div>
            </div>
            <div class="card">
                <h2>◎ Estimated Cost</h2>
                <div class="metric" id="total-cost">-</div>
                <div class="metric-label">Last 24 hours</div>
            </div>
        </div>

        <div class="card">
            <h2>✧ Active Sessions</h2>
            <div id="sessions-list">Connecting to daemon...</div>
        </div>
    </div>

    <script>
        let ws;
        let reconnectAttempts = 0;

        function connectWebSocket() {
            const protocol = window.location.protocol === 'https:' ? 'wss:' : 'ws:';
            ws = new WebSocket(`${protocol}//${window.location.host}/api/ws`);

            ws.onopen = () => {
                reconnectAttempts = 0;
                document.getElementById('ws-status').className = 'status-dot connected';
                document.getElementById('ws-label').textContent = 'Live';
            };

            ws.onclose = () => {
                document.getElementById('ws-status').className = 'status-dot disconnected';
                document.getElementById('ws-label').textContent = 'Reconnecting...';
                setTimeout(() => {
                    reconnectAttempts++;
                    connectWebSocket();
                }, Math.min(1000 * Math.pow(2, reconnectAttempts), 30000));
            };

            ws.onmessage = (event) => {
                try {
                    const data = JSON.parse(event.data);
                    updateDashboard(data);
                } catch (e) {
                    console.error('Parse error:', e);
                }
            };
        }

        function updateDashboard(data) {
            if (data.metrics) {
                updateMetric('active-sessions', data.metrics.active_sessions || 0);
                updateMetric('total-messages', (data.metrics.total_messages || 0).toLocaleString());
                updateMetric('total-tools', (data.metrics.total_tools || 0).toLocaleString());
                updateMetric('total-cost', '$' + (data.metrics.total_cost || 0).toFixed(2));
            }

            if (data.sessions) {
                const list = document.getElementById('sessions-list');
                if (data.sessions.length > 0) {
                    list.innerHTML = data.sessions.map(s => {
                        const project = s.project_path.split('/').pop() || 'Unknown';
                        const statusClass = 'status-' + (s.status || 'completed');
                        const tokens = formatTokens(s.tokens_input + s.tokens_output);
                        return `
                            <div class="session-item">
                                <div class="session-header">
                                    <span class="session-name">${project}</span>
                                    <span class="session-type">${s.agent_type}</span>
                                </div>
                                <div class="session-meta">
                                    <span>${s.message_count} msgs</span>
                                    <span>${tokens} tokens</span>
                                    <span>$${(s.estimated_cost || 0).toFixed(2)}</span>
                                    <span class="${statusClass}">● ${s.status}</span>
                                </div>
                            </div>
                        `;
                    }).join('');
                } else {
                    list.innerHTML = '<div class="session-item">No active sessions</div>';
                }
            }
        }

        function updateMetric(id, value) {
            const el = document.getElementById(id);
            if (el.textContent !== String(value)) {
                el.textContent = value;
                el.classList.add('update-flash');
                setTimeout(() => el.classList.remove('update-flash'), 500);
            }
        }

        function formatTokens(count) {
            if (count >= 1000000) return (count / 1000000).toFixed(1) + 'M';
            if (count >= 1000) return (count / 1000).toFixed(1) + 'K';
            return count;
        }

        connectWebSocket();
    </script>
</body>
</html>
"#;
