# Agent Monitor

A background daemon for monitoring AI agent sessions across multiple tools.

## Features

- **Multi-agent monitoring**: Track Claude Code, Cursor, Aider, and custom agents
- **Real-time updates**: Claude Code hook integration for instant event streaming
- **Unified view**: See all sessions in one place with TUI or web dashboard
- **Token & cost tracking**: Monitor usage and costs across all sessions
- **Historical data**: SQLite storage for session history and analytics

## Installation

```bash
# Install with pip
pip install agent-monitor

# Or install from source
git clone https://github.com/robertcprice/agent-monitor.git
cd agent-monitor
pip install -e .
```

## Quick Start

```bash
# Start the daemon
agent-monitor daemon

# Check status
agent-monitor status

# List sessions
agent-monitor sessions

# Install Claude Code hooks
agent-monitor install-hooks
```

## Architecture

```
┌─────────────────────────────────────────────────────────────────┐
│                    LaunchAgent (plist)                          │
└────────────────────────────┬────────────────────────────────────┘
                             │
┌────────────────────────────▼────────────────────────────────────┐
│                 Session Monitor Daemon (Python)                  │
│  ┌──────────┐ ┌──────────┐ ┌──────────┐ ┌──────────┐           │
│  │ Adapters │ │ Storage  │ │Event Bus │ │ IPC/API  │           │
│  │ Registry │ │ (SQLite) │ │ (asyncio)│ │ (Socket) │           │
│  └──────────┘ └──────────┘ └──────────┘ └──────────┘           │
└─────────────────────────────────────────────────────────────────┘
```

## Configuration

Configuration file location: `~/.config/agent-monitor/config.json`

```bash
# Create default config
agent-monitor config --init

# Show current config
agent-monitor config --show
```

## LaunchAgent (macOS)

To run as a background service:

```bash
# Copy the plist file
cp launchd/com.user.agent-monitor.plist ~/Library/LaunchAgents/

# Load the service
launchctl load ~/Library/LaunchAgents/com.user.agent-monitor.plist

# Start the service
launchctl start com.user.agent-monitor
```

## Development

```bash
# Clone the repo
git clone https://github.com/robertcprice/agent-monitor.git
cd agent-monitor

# Create virtual environment
python -m venv .venv
source .venv/bin/activate

# Install with dev dependencies
pip install -e ".[dev]"

# Run tests
pytest
```

## License

MIT
