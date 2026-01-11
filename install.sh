#!/bin/bash
# Agent Monitor Installation Script
# Installs the agent-monitor daemon and sets up LaunchAgent

set -e

# Colors
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m' # No Color

echo -e "${BLUE}╔════════════════════════════════════════════════════════════╗${NC}"
echo -e "${BLUE}║          Agent Monitor - Installation Script               ║${NC}"
echo -e "${BLUE}╚════════════════════════════════════════════════════════════╝${NC}"
echo

# Check for Python
if ! command -v python3 &> /dev/null; then
    echo -e "${RED}Error: Python 3 is required but not installed.${NC}"
    exit 1
fi

PYTHON_VERSION=$(python3 -c 'import sys; print(f"{sys.version_info.major}.{sys.version_info.minor}")')
echo -e "${GREEN}✓${NC} Python $PYTHON_VERSION found"

# Check for pip
if ! command -v pip3 &> /dev/null; then
    echo -e "${RED}Error: pip3 is required but not installed.${NC}"
    exit 1
fi
echo -e "${GREEN}✓${NC} pip3 found"

# Determine installation method
USE_UV=false
if command -v uv &> /dev/null; then
    echo -e "${GREEN}✓${NC} uv found (using uv for installation)"
    USE_UV=true
fi

# Get script directory
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

# Install the package
echo
echo -e "${YELLOW}Installing agent-monitor...${NC}"

if [ "$USE_UV" = true ]; then
    uv pip install -e "$SCRIPT_DIR"
else
    pip3 install -e "$SCRIPT_DIR"
fi

echo -e "${GREEN}✓${NC} agent-monitor installed"

# Create config directory
CONFIG_DIR="$HOME/.config/agent-monitor"
mkdir -p "$CONFIG_DIR/adapters"
echo -e "${GREEN}✓${NC} Config directory created: $CONFIG_DIR"

# Create data directory
DATA_DIR="$HOME/.local/share/agent-monitor"
mkdir -p "$DATA_DIR"
echo -e "${GREEN}✓${NC} Data directory created: $DATA_DIR"

# Install Claude Code hooks
echo
echo -e "${YELLOW}Installing Claude Code hooks...${NC}"
if [ -d "$HOME/.claude" ]; then
    if command -v agent-monitor &> /dev/null; then
        agent-monitor install-hooks 2>/dev/null || true
        echo -e "${GREEN}✓${NC} Claude Code hooks installed"
    else
        # Try with uv
        uv run python -m agent_monitor.cli install-hooks 2>/dev/null || true
        echo -e "${GREEN}✓${NC} Claude Code hooks installed"
    fi
else
    echo -e "${YELLOW}!${NC} Claude Code not found, skipping hook installation"
fi

# Setup LaunchAgent (macOS only)
if [[ "$OSTYPE" == "darwin"* ]]; then
    echo
    echo -e "${YELLOW}Setting up LaunchAgent...${NC}"

    LAUNCH_AGENTS_DIR="$HOME/Library/LaunchAgents"
    PLIST_FILE="$LAUNCH_AGENTS_DIR/com.user.agent-monitor.plist"

    mkdir -p "$LAUNCH_AGENTS_DIR"

    # Find the installed command path
    if command -v agent-monitor &> /dev/null; then
        AGENT_MONITOR_PATH=$(which agent-monitor)
    else
        AGENT_MONITOR_PATH="$HOME/.local/bin/agent-monitor"
    fi

    # Create plist file
    cat > "$PLIST_FILE" << EOF
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>Label</key>
    <string>com.user.agent-monitor</string>

    <key>ProgramArguments</key>
    <array>
        <string>$AGENT_MONITOR_PATH</string>
        <string>daemon</string>
        <string>--verbose</string>
    </array>

    <key>RunAtLoad</key>
    <true/>

    <key>KeepAlive</key>
    <dict>
        <key>SuccessfulExit</key>
        <false/>
        <key>Crashed</key>
        <true/>
    </dict>

    <key>ThrottleInterval</key>
    <integer>10</integer>

    <key>StandardOutPath</key>
    <string>$HOME/Library/Logs/agent-monitor.log</string>

    <key>StandardErrorPath</key>
    <string>$HOME/Library/Logs/agent-monitor-error.log</string>

    <key>WorkingDirectory</key>
    <string>$HOME</string>

    <key>EnvironmentVariables</key>
    <dict>
        <key>PATH</key>
        <string>/opt/homebrew/bin:/usr/local/bin:/usr/bin:/bin:$HOME/.local/bin</string>
        <key>PYTHONUNBUFFERED</key>
        <string>1</string>
    </dict>

    <key>ProcessType</key>
    <string>Background</string>

    <key>LowPriorityIO</key>
    <true/>

    <key>Nice</key>
    <integer>10</integer>
</dict>
</plist>
EOF

    echo -e "${GREEN}✓${NC} LaunchAgent plist created"

    # Load the LaunchAgent
    launchctl unload "$PLIST_FILE" 2>/dev/null || true
    launchctl load "$PLIST_FILE"
    echo -e "${GREEN}✓${NC} LaunchAgent loaded"

    # Start the service
    launchctl start com.user.agent-monitor 2>/dev/null || true
    echo -e "${GREEN}✓${NC} Service started"
fi

# Create example plugin manifest
EXAMPLE_PLUGIN="$CONFIG_DIR/adapters/example.yaml.disabled"
cat > "$EXAMPLE_PLUGIN" << 'EOF'
# Example custom agent plugin
# Rename to example.yaml to enable

name: my_custom_agent
display_name: My Custom Agent
description: A custom AI agent for specific tasks
version: 1.0.0

# Process detection
process_pattern: "python.*my_agent"
process_name: my_agent

# Data paths (supports ~ expansion)
log_path: ~/.my_agent/logs/
history_path: ~/.my_agent/history.jsonl

# Log parsing
log_format: plain
message_pattern: "\\[USER\\]|\\[AGENT\\]"

# Capabilities
capabilities:
  historical_data: true
  token_tracking: false

poll_interval: 30
EOF
echo -e "${GREEN}✓${NC} Example plugin manifest created"

# Print summary
echo
echo -e "${GREEN}╔════════════════════════════════════════════════════════════╗${NC}"
echo -e "${GREEN}║          Installation Complete!                            ║${NC}"
echo -e "${GREEN}╚════════════════════════════════════════════════════════════╝${NC}"
echo
echo -e "Usage:"
echo -e "  ${BLUE}agent-monitor status${NC}     - Check daemon status"
echo -e "  ${BLUE}agent-monitor sessions${NC}   - List sessions"
echo -e "  ${BLUE}agent-monitor tui${NC}        - Launch terminal dashboard"
echo -e "  ${BLUE}agent-monitor web${NC}        - Launch web dashboard"
echo
echo -e "Logs:"
echo -e "  ${BLUE}~/Library/Logs/agent-monitor.log${NC}"
echo
echo -e "Config:"
echo -e "  ${BLUE}~/.config/agent-monitor/${NC}"
echo
