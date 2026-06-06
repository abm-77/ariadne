"""Execution resource requirements. Pass to `@action(resources=resources(...))`;
actors advertise resources via `Inventory.actor(..., resources=resources(...))`.
Participates in actor selection."""

from __future__ import annotations

from dataclasses import dataclass


@dataclass
class Resources:
    cpu: int | None = None
    memory: str | None = None
    disk: str | None = None
    gpu: int | None = None

    def to_tir(self) -> dict:
        d: dict = {}
        if self.cpu is not None:
            d["cpu"] = self.cpu
        if self.memory is not None:
            d["memory"] = self.memory
        if self.disk is not None:
            d["disk"] = self.disk
        if self.gpu is not None:
            d["gpu"] = self.gpu
        return d


def resources(
    cpu: int | None = None,
    memory: str | None = None,
    disk: str | None = None,
    gpu: int | None = None,
) -> Resources:
    """Declare execution resources (cpu cores, memory/disk sizes, gpu count)."""
    return Resources(cpu=cpu, memory=memory, disk=disk, gpu=gpu)
