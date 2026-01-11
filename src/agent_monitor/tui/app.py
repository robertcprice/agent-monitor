"""Textual TUI application for agent monitoring with cosmic theme."""

import asyncio
import json
import socket
from datetime import datetime
from pathlib import Path
from typing import Optional

from textual.app import App, ComposeResult
from textual.binding import Binding
from textual.containers import Container, Horizontal, Vertical, ScrollableContainer
from textual.widgets import (
    Header,
    Footer,
    Static,
    DataTable,
    Label,
    ProgressBar,
    Placeholder,
    RichLog,
)
from textual.reactive import reactive
from textual.timer import Timer

from rich.text import Text
from rich.panel import Panel
from rich.table import Table

# Cosmic color constants
AURORA_BLUE = "#7AC9FF"
COSMIC_VIOLET = "#BFA6FF"
STELLAR_WHITE = "#FFFFFF"
PULSE_CYAN = "#00D4FF"
VOID_BLACK = "#000000"
NEBULA_GREY = "#0a0a0a"

# Unicode stars for decorative elements
STARS = ["✦", "✧", "★", "☆", "⋆", "✶"]


class SessionCard(Static):
    """Widget displaying a single session with cosmic styling."""

    def __init__(self, session: dict, index: int = 0, **kwargs):
        super().__init__(**kwargs)
        self.session_data = session
        self.index = index

    def compose(self) -> ComposeResult:
        yield Static(self._render_session())

    def _render_session(self) -> Text:
        """Render session as Rich Text with cosmic styling."""
        s = self.session_data
        project = Path(s.get("project_path", "")).name or "Unknown"
        status = s.get("status", "unknown")
        messages = s.get("message_count", 0)

        # Status emoji with cosmic colors
        status_styles = {
            "active": ("●", f"bold {PULSE_CYAN}"),
            "idle": ("◐", AURORA_BLUE),
            "completed": ("✓", COSMIC_VIOLET),
            "crashed": ("✗", "#ff4466"),
        }
        status_emoji, status_style = status_styles.get(status, ("○", "dim"))

        # Build text with cosmic styling
        text = Text()
        text.append(f"[{self.index + 1}] ", style=f"bold {AURORA_BLUE}")
        text.append(f"{status_emoji} ", style=status_style)
        text.append(f"{project[:20]:<20} ", style=f"bold {STELLAR_WHITE}")
        text.append(f"{messages:>5} msgs ", style=f"dim {AURORA_BLUE}")
        text.append(f"({status})", style=f"italic {COSMIC_VIOLET}")

        return text

    def update_session(self, session: dict) -> None:
        """Update session data."""
        self.session_data = session
        self.query_one(Static).update(self._render_session())


class SessionList(ScrollableContainer):
    """Scrollable list of session cards with cosmic styling."""

    sessions: reactive[list] = reactive([], recompose=True)

    def compose(self) -> ComposeResult:
        if not self.sessions:
            yield Static(f"[{COSMIC_VIOLET}]✦ No sessions found[/{COSMIC_VIOLET}]", classes="dim")
        else:
            for i, session in enumerate(self.sessions):
                yield SessionCard(session, index=i, id=f"session-{i}")

    def update_sessions(self, sessions: list) -> None:
        """Update sessions list."""
        self.sessions = sessions


class MetricsPanel(Static):
    """Panel showing aggregate metrics with cosmic styling."""

    metrics: reactive[dict] = reactive({})

    def render(self) -> Text:
        m = self.metrics
        text = Text()

        # Header with cosmic styling
        text.append("✦ ", style=f"bold {PULSE_CYAN}")
        text.append("METRICS\n", style=f"bold underline {AURORA_BLUE}")
        text.append("\n")

        # Metrics with cosmic colors
        text.append(f"Sessions:    ", style=f"dim {STELLAR_WHITE}")
        text.append(f"{m.get('total_sessions', 0):>8,}\n", style=f"bold {STELLAR_WHITE}")

        text.append(f"Messages:    ", style=f"dim {STELLAR_WHITE}")
        text.append(f"{m.get('total_messages', 0):>8,}\n", style=f"bold {STELLAR_WHITE}")

        text.append(f"Tool Calls:  ", style=f"dim {STELLAR_WHITE}")
        text.append(f"{m.get('total_tools', 0):>8,}\n", style=f"bold {STELLAR_WHITE}")

        text.append("\n")

        text.append(f"Today:       ", style=f"dim {STELLAR_WHITE}")
        text.append(f"{m.get('today_messages', 0):>8,}\n", style=f"bold {AURORA_BLUE}")

        text.append(f"Active:      ", style=f"dim {STELLAR_WHITE}")
        text.append(f"{m.get('active_sessions', 0):>8}\n", style=f"bold {PULSE_CYAN}")

        return text

    def update_metrics(self, metrics: dict) -> None:
        """Update metrics data."""
        self.metrics = metrics


class EventLog(RichLog):
    """Scrolling log of recent events with cosmic styling."""

    def __init__(self, **kwargs):
        super().__init__(highlight=True, markup=True, **kwargs)

    def add_event(self, event: dict) -> None:
        """Add an event to the log with cosmic styling."""
        event_type = event.get("event_type", "unknown")
        project = Path(event.get("working_directory", "")).name or "?"
        content = event.get("content", "")[:60]
        timestamp = datetime.now().strftime("%H:%M:%S")

        # Color by event type with cosmic palette
        type_colors = {
            "prompt_received": AURORA_BLUE,
            "tool_started": "#ffaa00",
            "tool_completed": PULSE_CYAN,
            "session_started": COSMIC_VIOLET,
            "session_ended": "dim",
        }
        color = type_colors.get(event_type, STELLAR_WHITE)

        self.write(
            f"[dim {AURORA_BLUE}]{timestamp}[/] "
            f"[{color}]{event_type:<16}[/] "
            f"[bold {STELLAR_WHITE}]{project}[/] "
            f"{content}"
        )


class StatusBar(Static):
    """Status bar showing connection state with cosmic styling."""

    connected: reactive[bool] = reactive(False)
    last_update: reactive[str] = reactive("")

    def render(self) -> Text:
        text = Text()

        if self.connected:
            text.append(" ● ", style=f"bold {PULSE_CYAN}")
            text.append("Connected", style=PULSE_CYAN)
        else:
            text.append(" ○ ", style=f"dim {COSMIC_VIOLET}")
            text.append("Disconnected", style=f"dim {COSMIC_VIOLET}")

        if self.last_update:
            text.append(f"  │  ", style=f"dim {AURORA_BLUE}")
            text.append(f"Last update: {self.last_update}", style=f"dim {STELLAR_WHITE}")

        return text


class AgentMonitorApp(App):
    """Main TUI application for agent monitoring with cosmic theme."""

    CSS = """
    Screen {
        background: #000000;
        layout: grid;
        grid-size: 2 3;
        grid-columns: 2fr 1fr;
        grid-rows: 1fr 1fr auto;
    }

    #sessions-container {
        row-span: 2;
        border: solid #7AC9FF;
        background: #0a0a0a;
        padding: 1;
    }

    #sessions-title {
        dock: top;
        height: 1;
        text-style: bold;
        color: #7AC9FF;
        background: #0a0a0a;
    }

    #sessions-list {
        height: 100%;
        background: #000000;
    }

    #metrics-panel {
        border: solid #BFA6FF;
        background: #0a0a0a;
        padding: 1;
    }

    #events-container {
        border: solid #00D4FF;
        background: #0a0a0a;
        padding: 1;
    }

    #events-title {
        dock: top;
        height: 1;
        text-style: bold;
        color: #00D4FF;
        background: #0a0a0a;
    }

    #events-log {
        height: 100%;
        background: #000000;
    }

    #status-bar {
        column-span: 2;
        height: 1;
        background: #0a0a0a;
        padding: 0 1;
        border-top: solid #7AC9FF;
    }

    SessionCard {
        height: 1;
        padding: 0 1;
        background: #000000;
    }

    SessionCard:hover {
        background: #0f0f09;
    }

    .dim {
        color: #666666;
    }

    Header {
        background: #0a0a0a;
        color: #7AC9FF;
    }

    Footer {
        background: #0a0a0a;
    }

    Footer > .footer--key {
        background: #7AC9FF;
        color: #000000;
    }

    Footer > .footer--description {
        color: #BFA6FF;
    }
    """

    BINDINGS = [
        Binding("q", "quit", "Quit"),
        Binding("r", "refresh", "Refresh"),
        Binding("d", "toggle_dark", "Theme"),
        Binding("1-9", "select_session", "Select", show=False),
    ]

    def __init__(self):
        super().__init__()
        self.socket_path = Path("/tmp/agent-monitor.sock")
        self._refresh_timer: Optional[Timer] = None
        self._sessions: list = []
        self._metrics: dict = {}

    def compose(self) -> ComposeResult:
        yield Header(show_clock=True)

        with Container(id="sessions-container"):
            yield Static("✦ ACTIVE SESSIONS ✦", id="sessions-title")
            yield SessionList(id="sessions-list")

        yield MetricsPanel(id="metrics-panel")

        with Container(id="events-container"):
            yield Static("✦ LIVE EVENTS ✦", id="events-title")
            yield EventLog(id="events-log")

        yield StatusBar(id="status-bar")
        yield Footer()

    async def on_mount(self) -> None:
        """Called when app is mounted."""
        self.title = "✦ Agent Monitor ✦"
        self.sub_title = "AI Session Monitoring"

        # Initial data load
        await self.refresh_data()

        # Set up periodic refresh
        self._refresh_timer = self.set_interval(2.0, self.refresh_data)

    async def refresh_data(self) -> None:
        """Refresh data from daemon or database."""
        status_bar = self.query_one("#status-bar", StatusBar)

        # Try IPC first
        if self.socket_path.exists():
            try:
                data = await self._query_daemon()
                if data:
                    status_bar.connected = True
                    status_bar.last_update = datetime.now().strftime("%H:%M:%S")
                    await self._update_from_ipc(data)
                    return
            except Exception:
                pass

        # Fallback to direct database
        status_bar.connected = False
        await self._update_from_database()
        status_bar.last_update = datetime.now().strftime("%H:%M:%S")

    async def _query_daemon(self) -> Optional[dict]:
        """Query the daemon via IPC."""
        try:
            reader, writer = await asyncio.open_unix_connection(str(self.socket_path))

            # Get sessions
            request = {"action": "get_sessions"}
            writer.write(json.dumps(request).encode() + b"\n")
            await writer.drain()

            response = await asyncio.wait_for(reader.readline(), timeout=2.0)
            sessions_data = json.loads(response.decode())

            # Get metrics
            request = {"action": "get_metrics"}
            writer.write(json.dumps(request).encode() + b"\n")
            await writer.drain()

            response = await asyncio.wait_for(reader.readline(), timeout=2.0)
            metrics_data = json.loads(response.decode())

            writer.close()
            await writer.wait_closed()

            return {
                "sessions": sessions_data.get("sessions", []),
                "metrics": metrics_data.get("metrics", {}),
            }

        except Exception:
            return None

    async def _update_from_ipc(self, data: dict) -> None:
        """Update UI from IPC data."""
        sessions = data.get("sessions", [])
        metrics = data.get("metrics", {})

        # Update sessions list
        session_list = self.query_one("#sessions-list", SessionList)
        session_list.update_sessions(sessions)

        # Update metrics
        metrics_panel = self.query_one("#metrics-panel", MetricsPanel)
        metrics_panel.update_metrics(metrics)

        self._sessions = sessions
        self._metrics = metrics

    async def _update_from_database(self) -> None:
        """Update UI directly from database."""
        import sqlite3
        from pathlib import Path

        db_path = Path.home() / ".local/share/agent-monitor/sessions.db"
        if not db_path.exists():
            return

        try:
            conn = sqlite3.connect(db_path)
            conn.row_factory = sqlite3.Row

            # Get sessions
            cursor = conn.execute("""
                SELECT * FROM sessions
                ORDER BY last_activity_at DESC
                LIMIT 20
            """)
            sessions = [dict(row) for row in cursor]

            # Get metrics
            cursor = conn.execute("SELECT COUNT(*) as count FROM sessions")
            total_sessions = cursor.fetchone()["count"]

            cursor = conn.execute("SELECT SUM(message_count) as total FROM sessions")
            total_messages = cursor.fetchone()["total"] or 0

            cursor = conn.execute(
                "SELECT COUNT(*) as count FROM sessions WHERE status = 'active'"
            )
            active_sessions = cursor.fetchone()["count"]

            conn.close()

            # Also get stats from stats-cache.json
            stats_file = Path.home() / ".claude/stats-cache.json"
            total_tools = 0
            today_messages = 0
            if stats_file.exists():
                with open(stats_file) as f:
                    stats = json.load(f)
                daily = stats.get("dailyActivity", [])
                total_tools = sum(d.get("toolCallCount", 0) for d in daily)
                if daily:
                    today = datetime.now().strftime("%Y-%m-%d")
                    today_data = next((d for d in daily if d.get("date") == today), {})
                    today_messages = today_data.get("messageCount", 0)

            metrics = {
                "total_sessions": total_sessions,
                "total_messages": total_messages,
                "active_sessions": active_sessions,
                "total_tools": total_tools,
                "today_messages": today_messages,
            }

            # Update sessions list
            session_list = self.query_one("#sessions-list", SessionList)
            session_list.update_sessions(sessions)

            # Update metrics
            metrics_panel = self.query_one("#metrics-panel", MetricsPanel)
            metrics_panel.update_metrics(metrics)

            self._sessions = sessions
            self._metrics = metrics

        except Exception as e:
            # Log error to events
            events_log = self.query_one("#events-log", EventLog)
            events_log.write(f"[#ff4466]✗ Error loading data: {e}[/]")

    def action_refresh(self) -> None:
        """Manual refresh action."""
        self.run_worker(self.refresh_data())

    def action_toggle_dark(self) -> None:
        """Toggle dark mode."""
        self.dark = not self.dark


def run_tui():
    """Run the TUI application."""
    app = AgentMonitorApp()
    app.run()


if __name__ == "__main__":
    run_tui()
