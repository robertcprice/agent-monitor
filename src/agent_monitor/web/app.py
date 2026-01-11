"""FastAPI web dashboard for agent monitoring."""

import asyncio
import json
from datetime import datetime
from pathlib import Path
from typing import Optional

from fastapi import FastAPI, WebSocket, WebSocketDisconnect, HTTPException
from fastapi.responses import HTMLResponse
from fastapi.staticfiles import StaticFiles

from agent_monitor.config import DaemonConfig
from agent_monitor.storage import StorageManager

app = FastAPI(
    title="Agent Monitor",
    description="Web dashboard for monitoring AI agent sessions",
    version="0.1.0",
)

# Global storage reference
_storage: Optional[StorageManager] = None


async def get_storage() -> StorageManager:
    """Get or create storage manager."""
    global _storage
    if _storage is None:
        config = DaemonConfig()
        _storage = StorageManager(config.db_path)
        await _storage.initialize()
    return _storage


# WebSocket connections
class ConnectionManager:
    def __init__(self):
        self.active_connections: list[WebSocket] = []

    async def connect(self, websocket: WebSocket):
        await websocket.accept()
        self.active_connections.append(websocket)

    def disconnect(self, websocket: WebSocket):
        if websocket in self.active_connections:
            self.active_connections.remove(websocket)

    async def broadcast(self, message: dict):
        for connection in self.active_connections:
            try:
                await connection.send_json(message)
            except Exception:
                pass


manager = ConnectionManager()


@app.get("/", response_class=HTMLResponse)
async def index():
    """Serve the main dashboard page."""
    return DASHBOARD_HTML


@app.get("/api/sessions")
async def get_sessions(
    limit: int = 50,
    active_only: bool = False,
):
    """Get list of sessions."""
    storage = await get_storage()

    if active_only:
        sessions = await storage.get_active_sessions(limit=limit)
    else:
        sessions = await storage.get_recent_sessions(hours=168, limit=limit)

    return {
        "sessions": [s.to_dict() for s in sessions],
        "total": len(sessions),
    }


@app.get("/api/sessions/{session_id}")
async def get_session(session_id: str):
    """Get details for a specific session."""
    storage = await get_storage()
    session = await storage.get_session(session_id)

    if not session:
        raise HTTPException(status_code=404, detail="Session not found")

    return {"session": session.to_dict()}


@app.get("/api/sessions/{session_id}/events")
async def get_session_events(session_id: str, limit: int = 100):
    """Get events for a session."""
    storage = await get_storage()
    events = await storage.get_session_events(session_id, limit=limit)

    return {
        "events": [e.to_dict() for e in events],
        "total": len(events),
    }


@app.get("/api/metrics/summary")
async def get_metrics_summary(hours: int = 24):
    """Get summary metrics."""
    storage = await get_storage()
    metrics = await storage.get_summary_metrics(hours=hours)

    return {"metrics": metrics}


@app.get("/api/metrics/daily")
async def get_daily_metrics(days: int = 7):
    """Get daily metrics for charting."""
    config = DaemonConfig()
    stats_file = config.claude_home / "stats-cache.json"

    if not stats_file.exists():
        return {"daily": []}

    try:
        with open(stats_file) as f:
            stats = json.load(f)

        daily = stats.get("dailyActivity", [])
        return {"daily": daily[-days:]}

    except Exception as e:
        return {"daily": [], "error": str(e)}


@app.get("/api/events")
async def get_recent_events(limit: int = 50):
    """Get recent events across all sessions."""
    storage = await get_storage()
    events = await storage.get_recent_events(limit=limit)

    return {
        "events": [e.to_dict() for e in events],
        "total": len(events),
    }


@app.websocket("/ws/events")
async def websocket_events(websocket: WebSocket):
    """WebSocket for real-time event streaming."""
    await manager.connect(websocket)

    try:
        while True:
            # Keep connection alive
            data = await websocket.receive_text()

            # Handle client messages if needed
            try:
                message = json.loads(data)
                if message.get("type") == "ping":
                    await websocket.send_json({"type": "pong"})
            except json.JSONDecodeError:
                pass

    except WebSocketDisconnect:
        manager.disconnect(websocket)


# HTML Dashboard (embedded for simplicity)
DASHBOARD_HTML = """
<!DOCTYPE html>
<html lang="en">
<head>
    <meta charset="UTF-8">
    <meta name="viewport" content="width=device-width, initial-scale=1.0">
    <title>Agent Monitor</title>
    <script src="https://cdn.jsdelivr.net/npm/chart.js"></script>
    <style>
        * {
            margin: 0;
            padding: 0;
            box-sizing: border-box;
        }

        body {
            font-family: -apple-system, BlinkMacSystemFont, 'Segoe UI', Roboto, sans-serif;
            background: #0d1117;
            color: #c9d1d9;
            min-height: 100vh;
        }

        .container {
            max-width: 1400px;
            margin: 0 auto;
            padding: 20px;
        }

        header {
            display: flex;
            justify-content: space-between;
            align-items: center;
            margin-bottom: 24px;
            padding-bottom: 16px;
            border-bottom: 1px solid #30363d;
        }

        header h1 {
            font-size: 24px;
            font-weight: 600;
            display: flex;
            align-items: center;
            gap: 12px;
        }

        .status-dot {
            width: 10px;
            height: 10px;
            border-radius: 50%;
            background: #238636;
            animation: pulse 2s infinite;
        }

        @keyframes pulse {
            0%, 100% { opacity: 1; }
            50% { opacity: 0.5; }
        }

        .metrics-grid {
            display: grid;
            grid-template-columns: repeat(auto-fit, minmax(200px, 1fr));
            gap: 16px;
            margin-bottom: 24px;
        }

        .metric-card {
            background: #161b22;
            border: 1px solid #30363d;
            border-radius: 8px;
            padding: 20px;
        }

        .metric-card .label {
            font-size: 12px;
            color: #8b949e;
            text-transform: uppercase;
            margin-bottom: 8px;
        }

        .metric-card .value {
            font-size: 32px;
            font-weight: 600;
            color: #58a6ff;
        }

        .main-content {
            display: grid;
            grid-template-columns: 2fr 1fr;
            gap: 24px;
        }

        .card {
            background: #161b22;
            border: 1px solid #30363d;
            border-radius: 8px;
            padding: 20px;
        }

        .card-header {
            display: flex;
            justify-content: space-between;
            align-items: center;
            margin-bottom: 16px;
            padding-bottom: 12px;
            border-bottom: 1px solid #30363d;
        }

        .card-header h2 {
            font-size: 16px;
            font-weight: 600;
        }

        .session-list {
            list-style: none;
        }

        .session-item {
            display: flex;
            justify-content: space-between;
            align-items: center;
            padding: 12px;
            margin-bottom: 8px;
            background: #0d1117;
            border-radius: 6px;
            cursor: pointer;
            transition: background 0.2s;
        }

        .session-item:hover {
            background: #1f2937;
        }

        .session-info {
            display: flex;
            align-items: center;
            gap: 12px;
        }

        .session-status {
            width: 8px;
            height: 8px;
            border-radius: 50%;
        }

        .session-status.active {
            background: #238636;
        }

        .session-status.completed {
            background: #8b949e;
        }

        .session-name {
            font-weight: 500;
        }

        .session-meta {
            font-size: 12px;
            color: #8b949e;
        }

        .session-stats {
            text-align: right;
            font-size: 14px;
        }

        .event-log {
            max-height: 400px;
            overflow-y: auto;
        }

        .event-item {
            display: flex;
            gap: 12px;
            padding: 8px 0;
            border-bottom: 1px solid #21262d;
            font-size: 13px;
        }

        .event-time {
            color: #8b949e;
            font-family: monospace;
            min-width: 60px;
        }

        .event-type {
            min-width: 120px;
            font-weight: 500;
        }

        .event-type.tool { color: #f0883e; }
        .event-type.response { color: #58a6ff; }
        .event-type.thinking { color: #a371f7; }
        .event-type.file { color: #3fb950; }

        .event-content {
            color: #c9d1d9;
            flex: 1;
            overflow: hidden;
            text-overflow: ellipsis;
            white-space: nowrap;
        }

        .chart-container {
            height: 200px;
            margin-top: 16px;
        }

        .refresh-btn {
            background: #238636;
            color: white;
            border: none;
            padding: 8px 16px;
            border-radius: 6px;
            cursor: pointer;
            font-size: 14px;
        }

        .refresh-btn:hover {
            background: #2ea043;
        }

        @media (max-width: 900px) {
            .main-content {
                grid-template-columns: 1fr;
            }
        }
    </style>
</head>
<body>
    <div class="container">
        <header>
            <h1>
                <span class="status-dot"></span>
                Agent Monitor
            </h1>
            <button class="refresh-btn" onclick="refreshData()">Refresh</button>
        </header>

        <div class="metrics-grid">
            <div class="metric-card">
                <div class="label">Active Sessions</div>
                <div class="value" id="active-sessions">-</div>
            </div>
            <div class="metric-card">
                <div class="label">Total Messages</div>
                <div class="value" id="total-messages">-</div>
            </div>
            <div class="metric-card">
                <div class="label">Tool Calls</div>
                <div class="value" id="tool-calls">-</div>
            </div>
            <div class="metric-card">
                <div class="label">Today's Activity</div>
                <div class="value" id="today-activity">-</div>
            </div>
        </div>

        <div class="main-content">
            <div class="card">
                <div class="card-header">
                    <h2>Sessions</h2>
                    <span id="session-count">0 sessions</span>
                </div>
                <ul class="session-list" id="session-list">
                    <li class="session-item">Loading...</li>
                </ul>
            </div>

            <div class="card">
                <div class="card-header">
                    <h2>Live Events</h2>
                </div>
                <div class="event-log" id="event-log">
                    <div class="event-item">Connecting...</div>
                </div>
            </div>
        </div>

        <div class="card" style="margin-top: 24px;">
            <div class="card-header">
                <h2>Activity (Last 7 Days)</h2>
            </div>
            <div class="chart-container">
                <canvas id="activity-chart"></canvas>
            </div>
        </div>
    </div>

    <script>
        let chart = null;
        let ws = null;

        async function fetchData(url) {
            const response = await fetch(url);
            return response.json();
        }

        async function loadSessions() {
            const data = await fetchData('/api/sessions?limit=20');
            const list = document.getElementById('session-list');
            const countEl = document.getElementById('session-count');

            countEl.textContent = `${data.total} sessions`;

            if (data.sessions.length === 0) {
                list.innerHTML = '<li class="session-item">No sessions found</li>';
                return;
            }

            list.innerHTML = data.sessions.map(session => {
                const project = session.project_path.split('/').pop() || 'Unknown';
                const status = session.status || 'unknown';
                const statusClass = status === 'active' ? 'active' : 'completed';

                return `
                    <li class="session-item" onclick="selectSession('${session.id}')">
                        <div class="session-info">
                            <div class="session-status ${statusClass}"></div>
                            <div>
                                <div class="session-name">${project}</div>
                                <div class="session-meta">${session.agent_type} · ${session.message_count} messages</div>
                            </div>
                        </div>
                        <div class="session-stats">
                            ${status}
                        </div>
                    </li>
                `;
            }).join('');
        }

        async function loadMetrics() {
            const [summary, daily] = await Promise.all([
                fetchData('/api/metrics/summary?hours=24'),
                fetchData('/api/metrics/daily?days=7')
            ]);

            document.getElementById('active-sessions').textContent =
                summary.metrics?.active_sessions || 0;
            document.getElementById('total-messages').textContent =
                (summary.metrics?.total_messages || 0).toLocaleString();
            document.getElementById('tool-calls').textContent =
                (summary.metrics?.total_tools || 0).toLocaleString();

            // Today's activity from daily
            const today = daily.daily?.[daily.daily.length - 1] || {};
            document.getElementById('today-activity').textContent =
                (today.messageCount || 0).toLocaleString();

            // Update chart
            if (daily.daily) {
                updateChart(daily.daily);
            }
        }

        async function loadEvents() {
            const data = await fetchData('/api/events?limit=20');
            const log = document.getElementById('event-log');

            if (data.events.length === 0) {
                log.innerHTML = '<div class="event-item">No events yet</div>';
                return;
            }

            log.innerHTML = data.events.map(event => {
                const time = new Date(event.timestamp).toLocaleTimeString([], {
                    hour: '2-digit',
                    minute: '2-digit'
                });

                let typeClass = '';
                if (event.event_type.includes('tool') || event.event_type.includes('file')) {
                    typeClass = 'tool';
                } else if (event.event_type.includes('response')) {
                    typeClass = 'response';
                } else if (event.event_type.includes('thinking')) {
                    typeClass = 'thinking';
                }

                return `
                    <div class="event-item">
                        <span class="event-time">${time}</span>
                        <span class="event-type ${typeClass}">${event.event_type}</span>
                        <span class="event-content">${event.content || ''}</span>
                    </div>
                `;
            }).join('');
        }

        function updateChart(daily) {
            const ctx = document.getElementById('activity-chart').getContext('2d');

            const labels = daily.map(d => {
                const date = new Date(d.date);
                return date.toLocaleDateString([], { weekday: 'short' });
            });

            const messages = daily.map(d => d.messageCount || 0);
            const tools = daily.map(d => d.toolCallCount || 0);

            if (chart) {
                chart.destroy();
            }

            chart = new Chart(ctx, {
                type: 'bar',
                data: {
                    labels: labels,
                    datasets: [
                        {
                            label: 'Messages',
                            data: messages,
                            backgroundColor: 'rgba(88, 166, 255, 0.8)',
                        },
                        {
                            label: 'Tool Calls',
                            data: tools,
                            backgroundColor: 'rgba(240, 136, 62, 0.8)',
                        }
                    ]
                },
                options: {
                    responsive: true,
                    maintainAspectRatio: false,
                    plugins: {
                        legend: {
                            labels: { color: '#c9d1d9' }
                        }
                    },
                    scales: {
                        x: {
                            ticks: { color: '#8b949e' },
                            grid: { color: '#21262d' }
                        },
                        y: {
                            ticks: { color: '#8b949e' },
                            grid: { color: '#21262d' }
                        }
                    }
                }
            });
        }

        function selectSession(sessionId) {
            console.log('Selected session:', sessionId);
            // Could expand to show session details
        }

        function connectWebSocket() {
            const protocol = window.location.protocol === 'https:' ? 'wss:' : 'ws:';
            ws = new WebSocket(`${protocol}//${window.location.host}/ws/events`);

            ws.onopen = () => {
                console.log('WebSocket connected');
            };

            ws.onmessage = (event) => {
                const data = JSON.parse(event.data);
                if (data.type === 'event') {
                    // Add new event to log
                    loadEvents();
                }
            };

            ws.onclose = () => {
                console.log('WebSocket disconnected, reconnecting...');
                setTimeout(connectWebSocket, 5000);
            };
        }

        async function refreshData() {
            await Promise.all([
                loadSessions(),
                loadMetrics(),
                loadEvents()
            ]);
        }

        // Initial load
        refreshData();
        connectWebSocket();

        // Auto refresh every 30 seconds
        setInterval(refreshData, 30000);
    </script>
</body>
</html>
"""


def run_web(host: str = "127.0.0.1", port: int = 8765):
    """Run the web dashboard."""
    import uvicorn
    uvicorn.run(app, host=host, port=port)


if __name__ == "__main__":
    run_web()
