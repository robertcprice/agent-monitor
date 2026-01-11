"""Agent adapters for monitoring different AI tools."""

from agent_monitor.adapters.base import BaseAdapter
from agent_monitor.adapters.claude_code import ClaudeCodeAdapter
from agent_monitor.adapters.cursor import CursorAdapter
from agent_monitor.adapters.aider import AiderAdapter
from agent_monitor.adapters.plugin import PluginAdapter, PluginManifest, PluginDiscovery

__all__ = [
    "BaseAdapter",
    "ClaudeCodeAdapter",
    "CursorAdapter",
    "AiderAdapter",
    "PluginAdapter",
    "PluginManifest",
    "PluginDiscovery",
]
