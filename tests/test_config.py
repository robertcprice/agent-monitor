"""Tests for configuration module."""

import pytest
import tempfile
from pathlib import Path

from agent_monitor.config import DaemonConfig, calculate_cost


class TestDaemonConfig:
    """Tests for DaemonConfig."""

    def test_default_config(self):
        """Test default configuration values."""
        config = DaemonConfig()

        assert config.poll_interval == 5.0
        assert config.http_port == 8420
        assert "agent-monitor" in str(config.data_dir)

    def test_db_path(self):
        """Test database path property."""
        config = DaemonConfig()
        assert config.db_path.suffix == ".db"
        assert "sessions" in config.db_path.name

    def test_socket_path(self):
        """Test socket path."""
        config = DaemonConfig()
        assert config.socket_path == Path("/tmp/agent-monitor.sock")

    def test_config_to_dict(self):
        """Test config serialization."""
        config = DaemonConfig()
        data = config.to_dict()

        assert "data_dir" in data
        assert "poll_interval" in data
        assert "http_port" in data

    def test_config_save_load(self):
        """Test saving and loading config."""
        with tempfile.NamedTemporaryFile(suffix=".json", delete=False) as f:
            config_path = Path(f.name)

        try:
            # Save config
            config = DaemonConfig()
            config.save(config_path)

            # Load config
            loaded = DaemonConfig.load(config_path)

            assert loaded.poll_interval == config.poll_interval
            assert loaded.http_port == config.http_port
        finally:
            config_path.unlink(missing_ok=True)


class TestCostCalculation:
    """Tests for cost calculation."""

    def test_opus_cost(self):
        """Test Opus model cost calculation."""
        # 1M input tokens, 1M output tokens
        cost = calculate_cost(1_000_000, 1_000_000, model_id="claude-opus-4-5-20251101")
        # Opus 4.5: $15/1M input, $75/1M output = $90 total
        assert cost == pytest.approx(90.0, rel=0.01)

    def test_sonnet_cost(self):
        """Test Sonnet model cost calculation."""
        cost = calculate_cost(1_000_000, 1_000_000, model_id="claude-sonnet-4-20250514")
        # Sonnet 4: $3/1M input, $15/1M output = $18 total
        assert cost == pytest.approx(18.0, rel=0.01)

    def test_haiku_cost(self):
        """Test Haiku model cost calculation."""
        cost = calculate_cost(1_000_000, 1_000_000, model_id="claude-haiku-3-5-20241022")
        # Haiku 3.5: $0.80/1M input, $4/1M output = $4.80 total
        assert cost == pytest.approx(4.80, rel=0.01)

    def test_zero_tokens(self):
        """Test cost with zero tokens."""
        cost = calculate_cost(0, 0)
        assert cost == 0.0

    def test_small_token_count(self):
        """Test cost with small token count."""
        # 1000 tokens should result in tiny cost
        # Default: $3/1M input + $15/1M output = $0.000018 total
        cost = calculate_cost(1000, 1000)
        assert cost < 0.10  # Should be well under a dime

    def test_default_model(self):
        """Test cost with default model (unknown model_id)."""
        cost = calculate_cost(1_000_000, 1_000_000, model_id="unknown-model")
        # Default uses Sonnet pricing: $3/1M input, $15/1M output = $18 total
        assert cost == pytest.approx(18.0, rel=0.01)
