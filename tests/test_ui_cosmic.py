"""Tests for cosmic UI components."""

import pytest
from rich.text import Text
from rich.panel import Panel
from rich.table import Table

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
    format_tokens,
    format_cost,
    format_duration,
    AURORA_BLUE,
    COSMIC_VIOLET,
    STELLAR_WHITE,
)


class TestCosmicTheme:
    """Tests for CosmicTheme."""

    def test_default_colors(self):
        """Test default theme colors."""
        theme = CosmicTheme()

        assert theme.primary == AURORA_BLUE
        assert theme.secondary == COSMIC_VIOLET
        assert theme.text == STELLAR_WHITE

    def test_custom_colors(self):
        """Test custom theme colors."""
        theme = CosmicTheme(primary="#FF0000")
        assert theme.primary == "#FF0000"


class TestCosmicConsole:
    """Tests for CosmicConsole."""

    def test_creation(self):
        """Test console creation."""
        console = CosmicConsole()
        assert console.theme is not None

    def test_with_custom_theme(self):
        """Test console with custom theme."""
        theme = CosmicTheme(primary="#00FF00")
        console = CosmicConsole(theme=theme)
        assert console.theme.primary == "#00FF00"


class TestBanners:
    """Tests for banner generation."""

    def test_cosmic_banner(self):
        """Test cosmic banner generation."""
        banner = cosmic_banner("1.0.0")

        assert isinstance(banner, Text)
        # Should contain decorative elements
        assert len(str(banner)) > 0

    def test_mini_banner(self):
        """Test mini banner generation."""
        banner = mini_banner("1.0.0")

        assert isinstance(banner, Text)
        assert "1.0.0" in str(banner) or len(str(banner)) > 0


class TestStarfield:
    """Tests for starfield generation."""

    def test_starfield_width(self):
        """Test starfield has correct width."""
        line = starfield_line(50)
        assert len(line) == 50

    def test_starfield_density(self):
        """Test starfield with different densities."""
        sparse = starfield_line(100, density=0.05)
        dense = starfield_line(100, density=0.50)

        # Dense should have more non-space characters
        sparse_stars = sum(1 for c in sparse if c != " ")
        dense_stars = sum(1 for c in dense if c != " ")

        assert dense_stars > sparse_stars


class TestGlowText:
    """Tests for glow text effect."""

    def test_glow_text(self):
        """Test glow text creation."""
        text = glow_text("Hello", AURORA_BLUE)

        assert isinstance(text, Text)
        assert "Hello" in str(text)

    def test_glow_intensity(self):
        """Test different glow intensities."""
        low = glow_text("Test", intensity=1)
        high = glow_text("Test", intensity=3)

        # Both should be Text objects
        assert isinstance(low, Text)
        assert isinstance(high, Text)


class TestCosmicPanel:
    """Tests for cosmic panel."""

    def test_panel_creation(self):
        """Test panel creation."""
        panel = cosmic_panel("Test content", title="Test")

        assert isinstance(panel, Panel)

    def test_panel_with_subtitle(self):
        """Test panel with subtitle."""
        panel = cosmic_panel("Content", title="Title", subtitle="Sub")

        assert isinstance(panel, Panel)


class TestCosmicTable:
    """Tests for cosmic table."""

    def test_table_creation(self):
        """Test table creation."""
        table = cosmic_table(
            title="Test Table",
            columns=[
                ("Col1", AURORA_BLUE, "left"),
                ("Col2", COSMIC_VIOLET, "right"),
            ],
        )

        assert isinstance(table, Table)

    def test_table_two_tuple_columns(self):
        """Test table with 2-tuple columns."""
        table = cosmic_table(
            title="Test",
            columns=[
                ("Name", AURORA_BLUE),
                ("Value", COSMIC_VIOLET),
            ],
        )

        assert isinstance(table, Table)


class TestStatusIndicator:
    """Tests for status indicator."""

    def test_active_status(self):
        """Test active status indicator."""
        indicator = status_indicator("active")

        assert isinstance(indicator, Text)
        assert "●" in str(indicator) or "active" in str(indicator).lower()

    def test_completed_status(self):
        """Test completed status indicator."""
        indicator = status_indicator("completed")

        assert isinstance(indicator, Text)

    def test_error_status(self):
        """Test error status indicator."""
        indicator = status_indicator("error")

        assert isinstance(indicator, Text)


class TestCosmicDivider:
    """Tests for cosmic divider."""

    def test_single_divider(self):
        """Test single-style divider."""
        divider = cosmic_divider(40, "single")

        assert isinstance(divider, Text)
        assert len(str(divider)) >= 40

    def test_gradient_divider(self):
        """Test gradient-style divider."""
        divider = cosmic_divider(60, "gradient")

        assert isinstance(divider, Text)


class TestFormatters:
    """Tests for formatting functions."""

    def test_format_tokens_thousands(self):
        """Test token formatting for thousands."""
        assert format_tokens(1500) == "1.5K"
        assert format_tokens(999) == "999"

    def test_format_tokens_millions(self):
        """Test token formatting for millions."""
        assert format_tokens(1_500_000) == "1.5M"

    def test_format_cost(self):
        """Test cost formatting."""
        assert format_cost(1.50) == "$1.50"
        assert format_cost(0.05) == "$0.05"

    def test_format_duration_seconds(self):
        """Test duration formatting for seconds."""
        assert format_duration(30) == "30s"
        assert format_duration(45.5) == "46s"

    def test_format_duration_minutes(self):
        """Test duration formatting for minutes."""
        assert format_duration(120) == "2.0m"
        assert format_duration(90) == "1.5m"

    def test_format_duration_hours(self):
        """Test duration formatting for hours."""
        assert format_duration(7200) == "2.0h"
        assert format_duration(5400) == "1.5h"
