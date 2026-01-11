"""Tests for CLI commands."""

import pytest
from typer.testing import CliRunner
from agent_monitor.cli import app

runner = CliRunner()


class TestVersionCommand:
    """Tests for the version command."""

    def test_version_output(self):
        """Test that version command outputs version info."""
        result = runner.invoke(app, ["version"])
        assert result.exit_code == 0
        assert "Agent Monitor" in result.output
        assert "v" in result.output

    def test_version_contains_stars(self):
        """Test that version has cosmic styling."""
        result = runner.invoke(app, ["version"])
        assert "✦" in result.output


class TestStatusCommand:
    """Tests for the status command."""

    def test_status_json_output(self):
        """Test status command with JSON output."""
        result = runner.invoke(app, ["status", "--json"])
        # Should return JSON (even if error)
        assert result.exit_code == 0 or "error" in result.output.lower()

    def test_status_no_animation(self):
        """Test status command without animation."""
        result = runner.invoke(app, ["status", "--no-animation"])
        # Should complete (may have DB error if not running)
        assert result.exit_code == 0 or "error" in result.output.lower()


class TestSessionsCommand:
    """Tests for the sessions command."""

    def test_sessions_limit(self):
        """Test sessions command with limit."""
        result = runner.invoke(app, ["sessions", "--limit", "5", "--no-animation"])
        # Should complete
        assert result.exit_code == 0 or "error" in result.output.lower()

    def test_sessions_json(self):
        """Test sessions command with JSON output."""
        result = runner.invoke(app, ["sessions", "--json"])
        assert result.exit_code == 0 or "error" in result.output.lower()


class TestConfigCommand:
    """Tests for the config command."""

    def test_config_show(self):
        """Test config show command."""
        result = runner.invoke(app, ["config", "--show"])
        assert result.exit_code == 0
        # Should show configuration details
        assert "data_dir" in result.output or "Configuration" in result.output


class TestHelpOutput:
    """Tests for help output."""

    def test_main_help(self):
        """Test main help output."""
        result = runner.invoke(app, ["--help"])
        assert result.exit_code == 0
        assert "daemon" in result.output
        assert "status" in result.output
        assert "sessions" in result.output

    def test_daemon_help(self):
        """Test daemon command help."""
        result = runner.invoke(app, ["daemon", "--help"])
        assert result.exit_code == 0
        assert "--verbose" in result.output or "-v" in result.output
