"""Workflow triggers: how a workflow begins execution. Pass to
`@workflow(triggers=[...])`. A trigger controls workflow entry, distinct from a
condition (which controls execution after entry)."""

from __future__ import annotations

from dataclasses import dataclass, field
from typing import Any


class _Trigger:
    def to_tir(self) -> dict[str, Any]:
        raise NotImplementedError


@dataclass
class _PullRequest(_Trigger):
    def to_tir(self) -> dict[str, Any]:
        return {"kind": "pull_request"}


@dataclass
class _Push(_Trigger):
    branches: list[str] = field(default_factory=list)

    def to_tir(self) -> dict[str, Any]:
        d: dict[str, Any] = {"kind": "push"}
        if self.branches:
            d["branches"] = list(self.branches)
        return d


@dataclass
class _Tag(_Trigger):
    pattern: str

    def to_tir(self) -> dict[str, Any]:
        return {"kind": "tag", "pattern": self.pattern}


@dataclass
class _Schedule(_Trigger):
    cron: str

    def to_tir(self) -> dict[str, Any]:
        return {"kind": "schedule", "cron": self.cron}


@dataclass
class _Manual(_Trigger):
    def to_tir(self) -> dict[str, Any]:
        return {"kind": "manual"}


def pull_request() -> _PullRequest:
    """Run on pull requests."""
    return _PullRequest()


def push(branches: list[str] | None = None) -> _Push:
    """Run on pushes, optionally restricted to branches."""
    return _Push(branches=branches or [])


def tag(pattern: str) -> _Tag:
    """Run on tag pushes matching a glob (e.g. "v*")."""
    return _Tag(pattern=pattern)


def schedule(cron: str) -> _Schedule:
    """Run on a cron schedule."""
    return _Schedule(cron=cron)


def manual() -> _Manual:
    """Allow manual dispatch."""
    return _Manual()
