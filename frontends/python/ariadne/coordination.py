"""Concurrency control. Pass to `@workflow(coordination=...)` or
`@action(coordination=...)`. Members of a group are coordinated; cancel-previous
cancels an in-progress run, exclusive queues them."""

from __future__ import annotations

from dataclasses import dataclass


@dataclass
class Coordination:
    group: str
    cancel_in_progress: bool = False

    def to_tir(self) -> dict:
        d: dict = {"group": self.group}
        if self.cancel_in_progress:
            d["cancel_in_progress"] = True
        return d


def group(name: str, cancel_in_progress: bool = False) -> Coordination:
    """Coordinate runs sharing a group name."""
    return Coordination(group=name, cancel_in_progress=cancel_in_progress)


def exclusive(name: str) -> Coordination:
    """Only one run in the group at a time; others queue."""
    return Coordination(group=name, cancel_in_progress=False)


def cancel_previous(name: str) -> Coordination:
    """A new run cancels any in-progress run in the group."""
    return Coordination(group=name, cancel_in_progress=True)
