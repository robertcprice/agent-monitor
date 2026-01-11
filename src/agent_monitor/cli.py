"""CLI interface for agent-monitor with cosmic-themed animated UI."""

import asyncio
import json
import logging
import sys
import time
from datetime import datetime
from pathlib import Path
from typing import Optional

import typer
from rich.console import Console
from rich.table import Table
from rich.panel import Panel
from rich.text import Text
from rich.live import Live
from rich.progress import Progress, SpinnerColumn, TextColumn

from agent_monitor import __version__
from agent_monitor.config import DaemonConfig
from agent_monitor.daemon import run_daemon
from agent_monitor.storage import StorageManager
from agent_monitor.models import AgentType, SessionStatus
from agent_monitor.ui.cosmic import (
    CosmicConsole,
    CosmicTheme,
    cosmic_banner,
    mini_banner,
    cosmic_panel,
    cosmic_table,
    cosmic_divider,
    starfield_line,
    glow_text,
    status_indicator,
    animate_startup,
    format_tokens,
    format_cost,
    format_duration,
    AURORA_BLUE,
    COSMIC_VIOLET,
    STELLAR_WHITE,
    PULSE_CYAN,
    ORBITAL_SPINNER,
)

app = typer.Typer(
    name="agent-monitor",
    help="Monitor AI agent sessions across multiple tools.",
    no_args_is_help=True,
    rich_markup_mode="rich",
)

# Use cosmic console
console = CosmicConsole(force_terminal=True)
theme = CosmicTheme()


def setup_logging(verbose: bool = False, debug: bool = False) -> None:
    """Configure logging."""
    level = logging.DEBUG if debug else (logging.INFO if verbose else logging.WARNING)
    logging.basicConfig(
        level=level,
        format="%(asctime)s [%(levelname)s] %(name)s: %(message)s",
        datefmt="%Y-%m-%d %H:%M:%S",
    )


def cosmic_loading(text: str = "Loading...") -> Progress:
    """Create a cosmic-styled loading spinner."""
    return Progress(
        SpinnerColumn(spinner_name="dots", style=AURORA_BLUE),
        TextColumn(f"[{AURORA_BLUE}]{text}[/{AURORA_BLUE}]"),
        transient=True,
        console=console,
    )


@app.command()
def daemon(
    config_path: Optional[Path] = typer.Option(
        None,
        "--config", "-c",
        help="Path to configuration file",
    ),
    verbose: bool = typer.Option(False, "--verbose", "-v", help="Verbose output"),
    debug: bool = typer.Option(False, "--debug", "-d", help="Debug output"),
    no_animation: bool = typer.Option(False, "--no-animation", help="Skip startup animation"),
) -> None:
    """Run the agent monitor daemon."""
    setup_logging(verbose, debug)

    config = DaemonConfig.load(config_path) if config_path else DaemonConfig()

    if not no_animation:
        animate_startup(console, __version__)
    else:
        console.print(mini_banner(__version__))

    # Daemon status panel
    status_content = Text()
    status_content.append("✦ ", style=f"bold {PULSE_CYAN}")
    status_content.append("Daemon Starting\n\n", style=f"bold {STELLAR_WHITE}")
    status_content.append("📁 Data:   ", style=f"dim")
    status_content.append(f"{config.data_dir}\n", style=AURORA_BLUE)
    status_content.append("🔌 Socket: ", style=f"dim")
    status_content.append(f"{config.socket_path}\n", style=AURORA_BLUE)
    status_content.append("🌐 Port:   ", style=f"dim")
    status_content.append(f"{config.http_port}", style=AURORA_BLUE)

    console.print(cosmic_panel(
        status_content,
        title="✦ Agent Monitor Daemon ✦",
        subtitle=f"v{__version__}",
    ))
    console.print()

    try:
        asyncio.run(run_daemon(config))
    except KeyboardInterrupt:
        console.print()
        console.print(cosmic_divider(50, "gradient"))
        console.print(
            f"  ✦ Shutting down gracefully...",
            style=f"bold {COSMIC_VIOLET}"
        )
        console.print(cosmic_divider(50, "gradient"))


@app.command()
def hook(
    event_type: str = typer.Argument(..., help="Hook event type"),
) -> None:
    """
    Handle hook events from Claude Code.

    This command is called by Claude Code hooks to report events.
    Reads event data from stdin and forwards to the daemon.
    """
    import socket
    import select

    # Read stdin if available (non-blocking)
    stdin_data = ""
    if select.select([sys.stdin], [], [], 0.0)[0]:
        stdin_data = sys.stdin.read()

    # Build event message
    message = {
        "type": "hook_event",
        "event_type": event_type,
        "timestamp": datetime.now().isoformat(),
        "data": json.loads(stdin_data) if stdin_data else {},
    }

    # Try to send to daemon
    socket_path = Path("/tmp/agent-monitor.sock")
    if socket_path.exists():
        try:
            with socket.socket(socket.AF_UNIX, socket.SOCK_STREAM) as sock:
                sock.connect(str(socket_path))
                sock.sendall((json.dumps(message) + "\n").encode())
        except Exception:
            pass  # Silently fail - don't block Claude Code


@app.command()
def status(
    json_output: bool = typer.Option(False, "--json", "-j", help="JSON output"),
    no_animation: bool = typer.Option(False, "--no-animation", help="Skip animations"),
) -> None:
    """Show daemon and session status."""

    async def get_status() -> dict:
        config = DaemonConfig()
        if not config.db_path.exists():
            return {"error": "Database not found. Is the daemon running?"}

        storage = StorageManager(config.db_path)
        await storage.initialize()

        sessions = await storage.get_active_sessions()
        metrics = await storage.get_summary_metrics(hours=24)

        await storage.close()

        return {
            "active_sessions": len(sessions),
            "sessions": [s.to_dict() for s in sessions],
            "metrics": metrics,
        }

    # Show loading animation
    if not json_output and not no_animation:
        with cosmic_loading("Fetching status..."):
            time.sleep(0.3)  # Brief pause for effect

    try:
        result = asyncio.run(get_status())
    except Exception as e:
        if json_output:
            console.print(json.dumps({"error": str(e)}))
        else:
            console.print(f"[{theme.error}]✗ Error: {e}[/{theme.error}]")
        return

    if json_output:
        console.print(json.dumps(result, indent=2, default=str))
        return

    if "error" in result:
        console.print(f"[{theme.error}]{result['error']}[/{theme.error}]")
        return

    # Cosmic status header
    if not no_animation:
        console.print(starfield_line(60), style=f"dim {AURORA_BLUE}")

    # Status metrics panel
    metrics = result["metrics"]
    status_content = Text()

    # Active sessions with glow effect
    status_content.append("● ", style=f"bold {theme.success}")
    status_content.append("Active Sessions: ", style="bold")
    status_content.append(f"{result['active_sessions']}\n", style=f"bold {AURORA_BLUE}")

    # 24h metrics
    status_content.append("\n")
    status_content.append("📊 ", style="dim")
    status_content.append("24-Hour Summary\n", style=f"bold {STELLAR_WHITE}")
    status_content.append("─" * 30 + "\n", style=f"dim {AURORA_BLUE}")

    status_content.append("   Sessions:  ", style="dim")
    status_content.append(f"{metrics['total_sessions']:>8}\n", style=STELLAR_WHITE)

    status_content.append("   Messages:  ", style="dim")
    status_content.append(f"{metrics['total_messages']:>8}\n", style=STELLAR_WHITE)

    status_content.append("   Cost:      ", style="dim")
    status_content.append(f"${metrics['total_cost']:>7.2f}\n", style=COSMIC_VIOLET)

    console.print(cosmic_panel(
        status_content,
        title="✦ Agent Monitor Status ✦",
        subtitle=f"v{__version__}",
    ))

    if result["sessions"]:
        console.print()

        # Cosmic-styled sessions table
        table = cosmic_table(
            title="✦ Active Sessions ✦",
            columns=[
                ("Project", f"bold {AURORA_BLUE}", "left"),
                ("Type", COSMIC_VIOLET, "left"),
                ("Messages", STELLAR_WHITE, "right"),
                ("Duration", STELLAR_WHITE, "right"),
                ("Status", STELLAR_WHITE, "center"),
            ],
        )

        for session in result["sessions"]:
            project = Path(session["project_path"]).name or "—"
            agent_type = session["agent_type"]
            messages = str(session["message_count"])
            duration = format_duration(session['duration_seconds'])
            status_emoji = "🟢" if session["status"] == "active" else "⚪"

            table.add_row(project, agent_type, messages, duration, status_emoji)

        console.print(table)

    if not no_animation:
        console.print(starfield_line(60), style=f"dim {COSMIC_VIOLET}")


@app.command()
def sessions(
    limit: int = typer.Option(20, "--limit", "-n", help="Number of sessions to show"),
    all_sessions: bool = typer.Option(False, "--all", "-a", help="Show all sessions, not just active"),
    json_output: bool = typer.Option(False, "--json", "-j", help="JSON output"),
    no_animation: bool = typer.Option(False, "--no-animation", help="Skip animations"),
) -> None:
    """List recent sessions."""

    async def get_sessions() -> list:
        config = DaemonConfig()
        storage = StorageManager(config.db_path)
        await storage.initialize()

        if all_sessions:
            sessions = await storage.get_recent_sessions(hours=168, limit=limit)
        else:
            sessions = await storage.get_active_sessions(limit=limit)

        await storage.close()
        return [s.to_dict() for s in sessions]

    # Show loading animation
    if not json_output and not no_animation:
        with cosmic_loading("Loading sessions..."):
            time.sleep(0.3)

    try:
        result = asyncio.run(get_sessions())
    except Exception as e:
        if json_output:
            console.print(json.dumps({"error": str(e)}))
        else:
            console.print(f"[{theme.error}]✗ Error: {e}[/{theme.error}]")
        return

    if json_output:
        console.print(json.dumps(result, indent=2, default=str))
        return

    if not result:
        console.print(f"[{COSMIC_VIOLET}]✦ No sessions found[/{COSMIC_VIOLET}]")
        return

    # Cosmic header
    if not no_animation:
        console.print(starfield_line(80), style=f"dim {AURORA_BLUE}")

    # Cosmic sessions table
    table = cosmic_table(
        title="✦ Sessions ✦",
        columns=[
            ("ID", f"dim {STELLAR_WHITE}", "left"),
            ("Project", f"bold {AURORA_BLUE}", "left"),
            ("Type", COSMIC_VIOLET, "left"),
            ("Status", STELLAR_WHITE, "center"),
            ("Messages", STELLAR_WHITE, "right"),
            ("Tokens", STELLAR_WHITE, "right"),
            ("Cost", COSMIC_VIOLET, "right"),
            ("Started", f"dim {STELLAR_WHITE}", "right"),
        ],
    )

    for session in result:
        session_id = session["id"][:8]
        project = Path(session["project_path"]).name or "—"
        agent_type = session["agent_type"]
        status = session["status"]
        messages = str(session["message_count"])
        tokens = format_tokens(session['tokens_input'] + session['tokens_output'])
        cost = format_cost(session['estimated_cost'])

        started = datetime.fromisoformat(session["started_at"])
        started_str = started.strftime("%H:%M")

        # Status with cosmic styling
        status_display = Text()
        if status == "active":
            status_display.append("● ", style=f"bold {theme.success}")
            status_display.append("active", style=theme.success)
        elif status == "completed":
            status_display.append("✓ ", style=f"dim {COSMIC_VIOLET}")
            status_display.append("done", style=f"dim {COSMIC_VIOLET}")
        elif status == "crashed":
            status_display.append("✗ ", style=theme.error)
            status_display.append("crash", style=theme.error)
        else:
            status_display.append("○ ", style="dim")
            status_display.append(status, style="dim")

        table.add_row(
            session_id,
            project,
            agent_type,
            status_display,
            messages,
            tokens,
            cost,
            started_str,
        )

    console.print(table)

    if not no_animation:
        console.print(starfield_line(80), style=f"dim {COSMIC_VIOLET}")


@app.command()
def install_hooks() -> None:
    """Install Claude Code hooks for real-time monitoring."""
    from agent_monitor.adapters.claude_code import install_hooks as do_install

    config = DaemonConfig()

    console.print(starfield_line(50), style=f"dim {AURORA_BLUE}")
    console.print(f"  ✦ Installing Claude Code Hooks...", style=f"bold {AURORA_BLUE}")

    async def install():
        await do_install(config.claude_home)

    with cosmic_loading("Installing hooks..."):
        asyncio.run(install())
        time.sleep(0.5)

    console.print(f"  ✦ Hooks installed successfully!", style=f"bold {theme.success}")
    console.print(starfield_line(50), style=f"dim {COSMIC_VIOLET}")


@app.command()
def config(
    show: bool = typer.Option(False, "--show", "-s", help="Show current configuration"),
    init: bool = typer.Option(False, "--init", "-i", help="Create default configuration file"),
) -> None:
    """Manage configuration."""
    config_path = Path.home() / ".config/agent-monitor/config.json"

    if init:
        cfg = DaemonConfig()
        cfg.save(config_path)
        console.print(f"[{theme.success}]✦ Configuration created at {config_path}[/{theme.success}]")
        return

    if show or not init:
        if config_path.exists():
            cfg = DaemonConfig.load(config_path)
        else:
            cfg = DaemonConfig()
            console.print(f"[{COSMIC_VIOLET}]✦ No config file found, showing defaults[/{COSMIC_VIOLET}]")

        # Pretty print config with cosmic styling
        config_content = Text()
        config_dict = cfg.to_dict()

        for key, value in config_dict.items():
            config_content.append(f"  {key}: ", style=f"dim {STELLAR_WHITE}")
            config_content.append(f"{value}\n", style=AURORA_BLUE)

        console.print(cosmic_panel(
            config_content,
            title="✦ Configuration ✦",
        ))


@app.command()
def tui() -> None:
    """Launch the terminal UI dashboard."""
    from agent_monitor.tui import run_tui

    console.print(f"  ✦ Launching TUI Dashboard...", style=f"bold {AURORA_BLUE}")
    run_tui()


@app.command()
def web(
    host: str = typer.Option("127.0.0.1", "--host", "-h", help="Host to bind to"),
    port: int = typer.Option(8765, "--port", "-p", help="Port to bind to"),
) -> None:
    """Launch the web dashboard."""
    from agent_monitor.web import run_web

    console.print(starfield_line(50), style=f"dim {AURORA_BLUE}")
    console.print(f"  ✦ Starting Web Dashboard", style=f"bold {AURORA_BLUE}")
    console.print(f"  🌐 http://{host}:{port}", style=COSMIC_VIOLET)
    console.print(starfield_line(50), style=f"dim {COSMIC_VIOLET}")
    console.print()

    run_web(host, port)


@app.command()
def cleanup(
    deduplicate: bool = typer.Option(True, "--deduplicate/--no-deduplicate", help="Remove duplicate sessions"),
    stale_hours: int = typer.Option(24, "--stale-hours", help="Mark sessions as completed after this many inactive hours"),
    mark_stale: bool = typer.Option(True, "--mark-stale/--no-mark-stale", help="Mark stale sessions as completed"),
    dry_run: bool = typer.Option(False, "--dry-run", "-n", help="Show what would be done without making changes"),
    json_output: bool = typer.Option(False, "--json", "-j", help="JSON output"),
) -> None:
    """Clean up duplicate and stale sessions in the database."""

    async def do_cleanup() -> dict:
        config = DaemonConfig()
        if not config.db_path.exists():
            return {"error": "Database not found. Is the daemon running?"}

        storage = StorageManager(config.db_path)
        await storage.initialize()

        results = {
            "deduplicated": {"duplicates_found": 0, "duplicates_removed": 0, "events_migrated": 0},
            "stale_cleaned": 0,
        }

        if dry_run:
            # Just count what would be affected
            async with storage._db.execute(
                """
                SELECT agent_type, external_id, COUNT(*) as cnt
                FROM sessions
                GROUP BY agent_type, external_id
                HAVING cnt > 1
                """
            ) as cursor:
                duplicates = await cursor.fetchall()
                results["deduplicated"]["duplicates_found"] = sum(d["cnt"] - 1 for d in duplicates)

            async with storage._db.execute(
                """
                SELECT COUNT(*) FROM sessions
                WHERE status = 'active'
                    AND last_activity_at < datetime('now', ? || ' hours')
                """,
                (f"-{stale_hours}",),
            ) as cursor:
                row = await cursor.fetchone()
                results["stale_cleaned"] = row[0] if row else 0

            results["dry_run"] = True
        else:
            if deduplicate:
                results["deduplicated"] = await storage.deduplicate_sessions()

            if mark_stale:
                results["stale_cleaned"] = await storage.cleanup_stale_sessions(
                    inactive_hours=stale_hours,
                    mark_completed=True,
                )

        await storage.close()
        return results

    if not json_output:
        console.print(starfield_line(50), style=f"dim {AURORA_BLUE}")
        console.print(f"  ✦ {'Analyzing' if dry_run else 'Running'} Database Cleanup...", style=f"bold {AURORA_BLUE}")

    with cosmic_loading("Processing..." if not json_output else ""):
        try:
            result = asyncio.run(do_cleanup())
        except Exception as e:
            if json_output:
                console.print(json.dumps({"error": str(e)}))
            else:
                console.print(f"[{theme.error}]✗ Error: {e}[/{theme.error}]")
            return

    if json_output:
        console.print(json.dumps(result, indent=2, default=str))
        return

    if "error" in result:
        console.print(f"[{theme.error}]{result['error']}[/{theme.error}]")
        return

    # Display results
    cleanup_content = Text()

    if dry_run:
        cleanup_content.append("🔍 ", style="dim")
        cleanup_content.append("DRY RUN - No changes made\n\n", style=f"bold {COSMIC_VIOLET}")

    # Deduplication results
    dedup = result["deduplicated"]
    cleanup_content.append("📋 ", style="dim")
    cleanup_content.append("Deduplication:\n", style=f"bold {STELLAR_WHITE}")
    cleanup_content.append(f"   Duplicates found:    {dedup['duplicates_found']:>6}\n", style=AURORA_BLUE)
    if not dry_run:
        cleanup_content.append(f"   Duplicates removed:  {dedup['duplicates_removed']:>6}\n", style=theme.success)
        cleanup_content.append(f"   Events migrated:     {dedup['events_migrated']:>6}\n", style=STELLAR_WHITE)

    # Stale session results
    cleanup_content.append("\n")
    cleanup_content.append("⏰ ", style="dim")
    cleanup_content.append("Stale Sessions:\n", style=f"bold {STELLAR_WHITE}")
    cleanup_content.append(f"   Sessions affected:   {result['stale_cleaned']:>6}\n", style=AURORA_BLUE if dry_run else theme.success)

    console.print(cosmic_panel(
        cleanup_content,
        title="✦ Cleanup Results ✦",
    ))

    console.print(starfield_line(50), style=f"dim {COSMIC_VIOLET}")


@app.command()
def version() -> None:
    """Show version information."""
    console.print(starfield_line(40), style=f"dim {AURORA_BLUE}")

    version_text = Text()
    version_text.append("  ✦ ", style=f"bold {PULSE_CYAN}")
    version_text.append("Agent Monitor ", style=f"bold {STELLAR_WHITE}")
    version_text.append(f"v{__version__}", style=f"bold {AURORA_BLUE}")

    console.print(version_text)
    console.print(starfield_line(40), style=f"dim {COSMIC_VIOLET}")


@app.callback()
def main() -> None:
    """Agent Monitor - Monitor AI agent sessions across multiple tools."""
    pass


if __name__ == "__main__":
    app()
