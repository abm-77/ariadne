"""Escape hatch for explicit, tool-specific implementations: run a raw command
on the actor or inside a container when a semantic action does not fit. The
action still declares its inputs, outputs, and consequences; only the command is
explicit."""

from ._impl import container, shell

__all__ = ["container", "shell"]
