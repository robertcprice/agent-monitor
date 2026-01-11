"""Configuration management for agent-monitor."""

import json
import logging
from dataclasses import dataclass, field, asdict
from pathlib import Path
from typing import Any, Optional

logger = logging.getLogger(__name__)


@dataclass
class DaemonConfig:
    """Configuration for the daemon process."""

    # Paths
    data_dir: Path = field(default_factory=lambda: Path.home() / ".local/share/agent-monitor")
    log_dir: Path = field(default_factory=lambda: Path.home() / "Library/Logs")
    socket_path: Path = field(default_factory=lambda: Path("/tmp/agent-monitor.sock"))

    # Database
    db_name: str = "sessions.db"

    # Daemon settings
    poll_interval: float = 5.0  # Seconds between discovery polls
    event_queue_size: int = 10000
    max_log_age_days: int = 30

    # IPC settings
    ipc_enabled: bool = True
    http_enabled: bool = True
    http_port: int = 8420
    http_host: str = "127.0.0.1"

    # Claude Code settings
    claude_home: Path = field(default_factory=lambda: Path.home() / ".claude")
    auto_install_hooks: bool = True

    # Adapter settings
    enabled_adapters: list[str] = field(
        default_factory=lambda: ["claude_code", "cursor", "aider"]
    )
    adapter_plugin_dir: Optional[Path] = None

    @property
    def db_path(self) -> Path:
        """Full path to the database file."""
        return self.data_dir / self.db_name

    @property
    def log_path(self) -> Path:
        """Full path to the log file."""
        return self.log_dir / "agent-monitor.log"

    def ensure_directories(self) -> None:
        """Create required directories if they don't exist."""
        self.data_dir.mkdir(parents=True, exist_ok=True)
        self.log_dir.mkdir(parents=True, exist_ok=True)

    def to_dict(self) -> dict[str, Any]:
        """Convert to dictionary for serialization."""
        data = asdict(self)
        # Convert Path objects to strings
        for key, value in data.items():
            if isinstance(value, Path):
                data[key] = str(value)
        return data

    @classmethod
    def from_dict(cls, data: dict[str, Any]) -> "DaemonConfig":
        """Create from dictionary."""
        # Convert string paths back to Path objects
        path_fields = [
            "data_dir", "log_dir", "socket_path",
            "claude_home", "adapter_plugin_dir"
        ]
        for key in path_fields:
            if key in data and data[key] is not None:
                data[key] = Path(data[key])
        return cls(**data)

    @classmethod
    def load(cls, config_path: Optional[Path] = None) -> "DaemonConfig":
        """Load configuration from file, with defaults for missing values."""
        if config_path is None:
            config_path = Path.home() / ".config/agent-monitor/config.json"

        if not config_path.exists():
            logger.debug(f"No config file at {config_path}, using defaults")
            return cls()

        try:
            with open(config_path) as f:
                data = json.load(f)
            return cls.from_dict(data)
        except Exception as e:
            logger.warning(f"Error loading config from {config_path}: {e}")
            return cls()

    def save(self, config_path: Optional[Path] = None) -> None:
        """Save configuration to file."""
        if config_path is None:
            config_path = Path.home() / ".config/agent-monitor/config.json"

        config_path.parent.mkdir(parents=True, exist_ok=True)

        with open(config_path, "w") as f:
            json.dump(self.to_dict(), f, indent=2)


# Model cost rates (per 1M tokens)
MODEL_COSTS = {
    # Claude models
    "claude-opus-4-5-20251101": {"input": 15.0, "output": 75.0},
    "claude-sonnet-4-20250514": {"input": 3.0, "output": 15.0},
    "claude-haiku-3-5-20241022": {"input": 0.80, "output": 4.0},
    # Fallback for unknown models
    "default": {"input": 3.0, "output": 15.0},
}


def calculate_cost(
    tokens_input: int,
    tokens_output: int,
    model_id: Optional[str] = None,
) -> float:
    """Calculate estimated cost for token usage."""
    costs = MODEL_COSTS.get(model_id or "", MODEL_COSTS["default"])
    input_cost = (tokens_input / 1_000_000) * costs["input"]
    output_cost = (tokens_output / 1_000_000) * costs["output"]
    return input_cost + output_cost
