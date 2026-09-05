#!/usr/bin/env python3
"""Stream a ValCoach/ValorantReplayParser export and print a compact JSON summary."""

from __future__ import annotations

import argparse
import json
import math
from collections import Counter
from pathlib import Path
from typing import Any, Iterable


def iter_ndjson(path: Path) -> Iterable[dict[str, Any]]:
    with path.open("r", encoding="utf-8") as source:
        for line_number, line in enumerate(source, 1):
            if not line.strip():
                continue
            try:
                value = json.loads(line)
            except json.JSONDecodeError as error:
                raise ValueError(f"{path}:{line_number}: invalid JSON: {error}") from error
            if not isinstance(value, dict):
                raise ValueError(f"{path}:{line_number}: record must be a JSON object")
            yield value


def numeric_values(value: Any) -> Iterable[float]:
    if isinstance(value, bool):
        return
    if isinstance(value, (int, float)):
        yield float(value)
    elif isinstance(value, dict):
        for child in value.values():
            yield from numeric_values(child)
    elif isinstance(value, list):
        for child in value:
            yield from numeric_values(child)


def find_file(directory: Path, candidates: tuple[str, ...]) -> Path:
    for candidate in candidates:
        path = directory / candidate
        if path.is_file():
            return path
    raise FileNotFoundError(f"none of {', '.join(candidates)} exists in {directory}")


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("directory", type=Path)
    parser.add_argument("--output", type=Path)
    args = parser.parse_args()

    directory = args.directory.resolve()
    events_path = find_file(directory, ("parser_events.ndjson", "events.ndjson"))
    movement_path = find_file(directory, ("movement.ndjson",))
    event_types: Counter[str] = Counter()
    actor_guids: set[int] = set()
    character_guids: set[int] = set()
    event_lines = movement_lines = nan_count = inf_count = 0
    time_min: int | None = None
    time_max: int | None = None
    bounds: dict[str, list[float | None]] = {
        "x": [None, None],
        "y": [None, None],
        "z": [None, None],
    }

    for event in iter_ndjson(events_path):
        event_lines += 1
        event_types[str(event.get("type", "<missing>"))] += 1
        actor = event.get("actor_net_guid")
        if isinstance(actor, int):
            actor_guids.add(actor)
        for number in numeric_values(event):
            nan_count += int(math.isnan(number))
            inf_count += int(math.isinf(number))

    for movement in iter_ndjson(movement_path):
        movement_lines += 1
        timestamp = movement.get("time_ms")
        if isinstance(timestamp, int):
            time_min = timestamp if time_min is None else min(time_min, timestamp)
            time_max = timestamp if time_max is None else max(time_max, timestamp)
        character = movement.get("shooter_character_net_guid")
        if isinstance(character, int):
            character_guids.add(character)
        position = movement.get("position")
        if isinstance(position, dict):
            for axis in bounds:
                value = position.get(axis)
                if isinstance(value, (int, float)) and math.isfinite(value):
                    current = bounds[axis]
                    current[0] = float(value) if current[0] is None else min(current[0], float(value))
                    current[1] = float(value) if current[1] is None else max(current[1], float(value))
        for number in numeric_values(movement):
            nan_count += int(math.isnan(number))
            inf_count += int(math.isinf(number))

    summary = {
        "events_lines": event_lines,
        "movement_lines": movement_lines,
        "event_types": dict(sorted(event_types.items())),
        "distinct_actor_guids": len(actor_guids),
        "distinct_character_guids": len(character_guids),
        "movement_time_min_ms": time_min,
        "movement_time_max_ms": time_max,
        "position_bounds": bounds,
        "nan_count": nan_count,
        "inf_count": inf_count,
    }
    text = json.dumps(summary, indent=2, ensure_ascii=False)
    print(text)
    if args.output:
        args.output.parent.mkdir(parents=True, exist_ok=True)
        args.output.write_text(text + "\n", encoding="utf-8")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
