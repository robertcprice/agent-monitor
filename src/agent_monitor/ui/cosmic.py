"""Cosmic-themed animated UI components inspired by sms-hub design.

Design Philosophy:
- Black void background with high contrast elements
- Aurora blue (#7AC9FF) as primary accent color
- Cosmic violet (#BFA6FF) as secondary accent
- Glow effects and subtle animations
- Retro-futuristic sci-fi aesthetic
"""

import asyncio
import random
import sys
import time
from contextlib import contextmanager
from dataclasses import dataclass
from typing import Generator, Iterator, Optional

from rich.console import Console, RenderableType
from rich.panel import Panel
from rich.style import Style
from rich.table import Table
from rich.text import Text
from rich.progress import Progress, SpinnerColumn, TextColumn, BarColumn, TaskID
from rich.live import Live
from rich.align import Align
from rich.box import ROUNDED, DOUBLE, HEAVY


# Cosmic color palette
VOID_BLACK = "#000000"
NEBULA_GREY = "#0a0a0a"
AURORA_BLUE = "#7AC9FF"
COSMIC_VIOLET = "#BFA6FF"
STELLAR_WHITE = "#FFFFFF"
PULSE_CYAN = "#00D4FF"
DEEP_SPACE = "#0f0f09"

# Gradient stops for animations
GRADIENT_BLUE = ["#1a3a5c", "#2d5a8e", "#4a90c9", "#7AC9FF", "#a8dcff"]
GRADIENT_VIOLET = ["#3d2a5c", "#5c3d8e", "#8e5cc9", "#BFA6FF", "#d4c4ff"]

# Unicode symbols for cosmic effects
STARS = ["✦", "✧", "★", "☆", "⋆", "✶", "✴", "✵", "✷", "✸", "·", "•"]
COSMIC_CHARS = ["░", "▒", "▓", "█", "▄", "▀", "◆", "◇", "○", "●"]
SPINNERS = ["◐", "◓", "◑", "◒"]
ORBITAL_SPINNER = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"]
COSMIC_SPINNER = ["✶", "✷", "✵", "✴", "✶", "✷", "✵", "✴"]
WAVE_CHARS = ["▁", "▂", "▃", "▄", "▅", "▆", "▇", "█", "▇", "▆", "▅", "▄", "▃", "▂"]


# ASCII Art Banner
BANNER_ART = r"""
    ╭──────────────────────────────────────────────────────────────────╮
    │                                                                  │
    │   ░█▀█░█▀▀░█▀▀░█▀█░▀█▀░░░█▄█░█▀█░█▀█░▀█▀░▀█▀░█▀█░█▀▄           │
    │   ░█▀█░█░█░█▀▀░█░█░░█░░░░█░█░█░█░█░█░░█░░░█░░█░█░█▀▄           │
    │   ░▀░▀░▀▀▀░▀▀▀░▀░▀░░▀░░░░▀░▀░▀▀▀░▀░▀░▀▀▀░░▀░░▀▀▀░▀░▀           │
    │                                                                  │
    │         ✦ AI Session Monitoring • Real-time Insights ✦          │
    │                                                                  │
    ╰──────────────────────────────────────────────────────────────────╯
"""

MINI_BANNER = r"""
  ╭─────────────────────────────────────────────╮
  │  ✦ AGENT MONITOR ✦  v{version:<12}        │
  │     AI Session Monitoring • Real-time      │
  ╰─────────────────────────────────────────────╯
"""

COMPACT_LOGO = "✦ AGENT MONITOR ✦"


@dataclass
class CosmicTheme:
    """Theme configuration for cosmic UI."""

    primary: str = AURORA_BLUE
    secondary: str = COSMIC_VIOLET
    background: str = VOID_BLACK
    surface: str = NEBULA_GREY
    text: str = STELLAR_WHITE
    accent: str = PULSE_CYAN
    dim: str = "#666666"
    success: str = "#00ff88"
    warning: str = "#ffaa00"
    error: str = "#ff4466"


class CosmicConsole(Console):
    """Extended Rich Console with cosmic theme support."""

    def __init__(self, theme: Optional[CosmicTheme] = None, **kwargs):
        super().__init__(**kwargs)
        self.theme = theme or CosmicTheme()
        self._animation_frame = 0

    def cosmic_print(self, text: str, style: Optional[str] = None, **kwargs):
        """Print with cosmic styling."""
        if style is None:
            style = f"bold {self.theme.primary}"
        self.print(text, style=style, **kwargs)

    def glow(self, text: str, color: str = AURORA_BLUE, intensity: int = 2) -> Text:
        """Create glowing text effect."""
        return glow_text(text, color, intensity)

    def starfield(self, width: int = 60) -> str:
        """Generate a decorative starfield line."""
        return starfield_line(width)

    def pulse_print(self, text: str, frames: int = 10, delay: float = 0.05):
        """Print text with a pulse animation."""
        for i in range(frames):
            brightness = abs((i % 10) - 5) / 5
            color_val = int(127 + 128 * brightness)
            self.print(f"\r{text}", end="", style=f"rgb({color_val},{color_val},255)")
            time.sleep(delay)
        self.print()


def glow_text(text: str, color: str = AURORA_BLUE, intensity: int = 2) -> Text:
    """Create a glowing text effect using Rich Text styling.

    Args:
        text: The text to style
        color: Base color for the glow
        intensity: Glow intensity (1-3)

    Returns:
        Rich Text object with glow styling
    """
    styled = Text()

    if intensity >= 3:
        styled.append(text, style=f"bold {color} on {NEBULA_GREY}")
    elif intensity >= 2:
        styled.append(text, style=f"bold {color}")
    else:
        styled.append(text, style=color)

    return styled


def starfield_line(width: int = 60, density: float = 0.15) -> str:
    """Generate a decorative starfield line.

    Args:
        width: Width of the line in characters
        density: Star density (0.0 - 1.0)

    Returns:
        String with randomly placed star characters
    """
    line = []
    for _ in range(width):
        if random.random() < density:
            star = random.choice(STARS[:6])  # Use brighter stars
            line.append(star)
        else:
            line.append(" ")
    return "".join(line)


def cosmic_banner(version: str = "0.1.0", animate: bool = True) -> Text:
    """Generate the cosmic-themed banner.

    Args:
        version: Version string to display
        animate: Whether to return animated version

    Returns:
        Rich Text with styled banner
    """
    banner_text = Text()

    # Add top starfield
    banner_text.append(starfield_line(70) + "\n", style=f"dim {AURORA_BLUE}")

    # Banner with gradient effect
    lines = BANNER_ART.strip().split("\n")
    for i, line in enumerate(lines):
        # Create gradient effect from blue to violet
        if "░█" in line or "█▀" in line or "█░" in line:
            banner_text.append(line + "\n", style=f"bold {AURORA_BLUE}")
        elif "╭" in line or "╰" in line:
            banner_text.append(line + "\n", style=f"{COSMIC_VIOLET}")
        elif "✦" in line:
            banner_text.append(line + "\n", style=f"bold {PULSE_CYAN}")
        else:
            banner_text.append(line + "\n", style=f"dim {STELLAR_WHITE}")

    # Add bottom starfield
    banner_text.append(starfield_line(70) + "\n", style=f"dim {COSMIC_VIOLET}")

    return banner_text


def mini_banner(version: str = "0.1.0") -> Text:
    """Generate a compact banner for status displays."""
    banner_text = Text()
    formatted = MINI_BANNER.format(version=version)

    for line in formatted.strip().split("\n"):
        if "✦" in line:
            banner_text.append(line + "\n", style=f"bold {AURORA_BLUE}")
        elif "╭" in line or "╰" in line:
            banner_text.append(line + "\n", style=f"{COSMIC_VIOLET}")
        else:
            banner_text.append(line + "\n", style=f"dim {STELLAR_WHITE}")

    return banner_text


def cosmic_panel(
    content: RenderableType,
    title: str = "",
    subtitle: str = "",
    border_style: str = AURORA_BLUE,
    glow: bool = True,
) -> Panel:
    """Create a cosmic-styled panel.

    Args:
        content: Content to display in the panel
        title: Panel title
        subtitle: Panel subtitle
        border_style: Color for the border
        glow: Whether to add glow effect

    Returns:
        Rich Panel with cosmic styling
    """
    title_styled = Text(title, style=f"bold {AURORA_BLUE}") if title else None
    subtitle_styled = Text(subtitle, style=f"dim {COSMIC_VIOLET}") if subtitle else None

    return Panel(
        content,
        title=title_styled,
        subtitle=subtitle_styled,
        border_style=Style(color=border_style, bold=glow),
        box=ROUNDED,
        padding=(1, 2),
    )


def cosmic_table(
    title: str = "",
    columns: Optional[list[tuple[str, str, str]]] = None,
    border_style: str = AURORA_BLUE,
    row_styles: Optional[list[str]] = None,
) -> Table:
    """Create a cosmic-styled table.

    Args:
        title: Table title
        columns: List of (name, style, justify) tuples for columns
                 justify can be: "left", "center", "right"
        border_style: Color for table borders
        row_styles: Alternating row styles

    Returns:
        Rich Table with cosmic styling
    """
    table = Table(
        title=Text(title, style=f"bold {AURORA_BLUE}") if title else None,
        title_style=f"bold {AURORA_BLUE}",
        border_style=border_style,
        header_style=f"bold {STELLAR_WHITE}",
        row_styles=row_styles or [f"dim {STELLAR_WHITE}", STELLAR_WHITE],
        box=ROUNDED,
        show_edge=True,
        pad_edge=True,
    )

    if columns:
        for col in columns:
            if len(col) == 3:
                name, style, justify = col
            elif len(col) == 2:
                name, style = col
                justify = "left"
            else:
                name = col[0]
                style = STELLAR_WHITE
                justify = "left"

            # Handle special style values that are actually justifications
            if style in ("left", "center", "right"):
                justify = style
                style = STELLAR_WHITE
            elif style.startswith("dim ") and style.endswith((" left", " center", " right")):
                parts = style.rsplit(" ", 1)
                style = parts[0]
                justify = parts[1]
            elif style.endswith((" left", " center", " right")):
                parts = style.rsplit(" ", 1)
                style = parts[0] if parts[0] else STELLAR_WHITE
                justify = parts[1]

            table.add_column(name, style=style or STELLAR_WHITE, justify=justify)

    return table


class TypewriterText:
    """Typewriter animation effect for text."""

    def __init__(self, text: str, delay: float = 0.03, style: str = AURORA_BLUE):
        self.text = text
        self.delay = delay
        self.style = style

    def animate(self, console: Console):
        """Animate the text with typewriter effect."""
        for i, char in enumerate(self.text):
            console.print(char, end="", style=self.style)
            if char not in " \n":
                time.sleep(self.delay)
        console.print()  # Newline at end


class CosmicSpinner:
    """Animated spinner with cosmic theme."""

    def __init__(
        self,
        text: str = "Processing...",
        spinner_type: str = "orbital",
        color: str = AURORA_BLUE,
    ):
        self.text = text
        self.color = color
        self.frame = 0

        if spinner_type == "orbital":
            self.chars = ORBITAL_SPINNER
        elif spinner_type == "cosmic":
            self.chars = COSMIC_SPINNER
        elif spinner_type == "wave":
            self.chars = WAVE_CHARS
        else:
            self.chars = SPINNERS

    def get_frame(self) -> str:
        """Get current spinner frame."""
        char = self.chars[self.frame % len(self.chars)]
        self.frame += 1
        return f"{char} {self.text}"

    @contextmanager
    def spin(self, console: Console) -> Generator[None, None, None]:
        """Context manager for spinner animation."""
        stop_event = False

        async def animate():
            while not stop_event:
                frame = self.get_frame()
                console.print(f"\r{frame}", end="", style=self.color)
                await asyncio.sleep(0.1)

        try:
            yield
        finally:
            stop_event = True
            console.print()  # Clear the line


def animated_spinner(text: str = "Loading...", spinner_type: str = "orbital") -> Progress:
    """Create an animated spinner with cosmic styling.

    Args:
        text: Text to display next to spinner
        spinner_type: Type of spinner animation

    Returns:
        Rich Progress object configured as spinner
    """
    return Progress(
        SpinnerColumn(spinner_name="dots", style=AURORA_BLUE),
        TextColumn(f"[{AURORA_BLUE}]{text}"),
        transient=True,
    )


def pulse_animation(
    console: Console,
    text: str,
    duration: float = 1.0,
    color_start: str = AURORA_BLUE,
    color_end: str = COSMIC_VIOLET,
):
    """Display text with a pulsing color animation.

    Args:
        console: Rich Console to print to
        text: Text to animate
        duration: Animation duration in seconds
        color_start: Starting color
        color_end: Ending color
    """
    frames = int(duration * 20)
    for i in range(frames):
        # Oscillate between colors
        t = (i / frames) * 2
        if t > 1:
            t = 2 - t

        # Interpolate between colors (simplified)
        color = color_start if t < 0.5 else color_end
        console.print(f"\r{text}", end="", style=f"bold {color}")
        time.sleep(duration / frames)
    console.print()


def status_indicator(status: str) -> Text:
    """Generate a cosmic-styled status indicator.

    Args:
        status: Status string (active, idle, completed, error, etc.)

    Returns:
        Rich Text with styled status indicator
    """
    indicators = {
        "active": ("●", "#00ff88", "✦ Active"),
        "idle": ("◐", AURORA_BLUE, "◐ Idle"),
        "completed": ("✓", COSMIC_VIOLET, "✓ Done"),
        "error": ("✗", "#ff4466", "✗ Error"),
        "crashed": ("⚠", "#ffaa00", "⚠ Crashed"),
        "pending": ("○", "#666666", "○ Pending"),
    }

    symbol, color, label = indicators.get(status.lower(), ("?", "#666666", f"? {status}"))

    text = Text()
    text.append(f"{symbol} ", style=f"bold {color}")
    text.append(status.capitalize(), style=color)

    return text


def cosmic_divider(width: int = 60, style: str = "single") -> Text:
    """Generate a cosmic-styled divider line.

    Args:
        width: Width of divider
        style: Divider style (single, double, stars, gradient)

    Returns:
        Rich Text with styled divider
    """
    text = Text()

    if style == "double":
        text.append("═" * width, style=AURORA_BLUE)
    elif style == "stars":
        text.append(starfield_line(width), style=f"dim {COSMIC_VIOLET}")
    elif style == "gradient":
        # Create a gradient divider
        third = width // 3
        text.append("─" * third, style=f"dim {COSMIC_VIOLET}")
        text.append("═" * third, style=AURORA_BLUE)
        text.append("─" * third, style=f"dim {COSMIC_VIOLET}")
    else:
        text.append("─" * width, style=f"dim {AURORA_BLUE}")

    return text


def session_card(
    project: str,
    agent_type: str,
    status: str,
    messages: int,
    duration: str,
    tokens: str = "0K",
    cost: str = "$0.00",
) -> Panel:
    """Generate a cosmic-styled session card.

    Args:
        project: Project name
        agent_type: Type of agent (claude_code, cursor, etc.)
        status: Session status
        messages: Message count
        duration: Duration string
        tokens: Token usage string
        cost: Cost string

    Returns:
        Rich Panel with session info
    """
    # Build card content
    content = Text()

    # Status indicator
    status_text = status_indicator(status)
    content.append_text(status_text)
    content.append("\n\n")

    # Project name
    content.append(f"📁 ", style="dim")
    content.append(f"{project[:25]}\n", style=f"bold {STELLAR_WHITE}")

    # Agent type
    content.append(f"🤖 ", style="dim")
    content.append(f"{agent_type}\n", style=AURORA_BLUE)

    # Metrics
    content.append("\n")
    content.append(f"💬 {messages} msgs  ", style=f"dim {STELLAR_WHITE}")
    content.append(f"⏱ {duration}  ", style=f"dim {STELLAR_WHITE}")
    content.append(f"📊 {tokens}\n", style=f"dim {STELLAR_WHITE}")
    content.append(f"💰 {cost}", style=COSMIC_VIOLET)

    return Panel(
        content,
        border_style=AURORA_BLUE if status == "active" else "dim",
        box=ROUNDED,
        padding=(0, 1),
    )


def animate_startup(console: Console, version: str = "0.1.0"):
    """Play the cosmic startup animation.

    Args:
        console: Rich Console to render to
        version: Version string to display
    """
    # Clear screen
    console.clear()

    # Phase 1: Starfield builds up
    for _ in range(3):
        console.print(starfield_line(70), style=f"dim {AURORA_BLUE}")
        time.sleep(0.05)

    # Phase 2: Banner fade in
    banner = cosmic_banner(version)
    console.print(banner)

    # Phase 3: Status line
    console.print()
    console.print(
        f"  ✦ v{version} • Ready",
        style=f"bold {AURORA_BLUE}"
    )
    console.print(cosmic_divider(60, "gradient"))
    console.print()


def format_tokens(count: int) -> str:
    """Format token count with cosmic styling."""
    if count >= 1_000_000:
        return f"{count / 1_000_000:.1f}M"
    elif count >= 1_000:
        return f"{count / 1_000:.1f}K"
    else:
        return str(count)


def format_cost(amount: float) -> str:
    """Format cost with cosmic styling."""
    if amount >= 1.0:
        return f"${amount:.2f}"
    elif amount >= 0.01:
        return f"${amount:.2f}"
    else:
        return f"${amount:.3f}"


def format_duration(seconds: float) -> str:
    """Format duration with cosmic styling."""
    if seconds >= 3600:
        return f"{seconds / 3600:.1f}h"
    elif seconds >= 60:
        return f"{seconds / 60:.1f}m"
    else:
        return f"{seconds:.0f}s"
