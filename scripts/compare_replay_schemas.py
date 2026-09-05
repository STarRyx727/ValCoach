#!/usr/bin/env python3
"""Compare stable event/property fingerprints from two replay export directories."""

from __future__ import annotations

import argparse
import json
from collections import Counter, defaultdict
from pathlib import Path


def fingerprint(directory: Path) -> dict[str, object]:
    events = directory / "parser_events.ndjson"
    if not events.exists():
        events = directory / "events.ndjson"
    types: Counter[str] = Counter()
    keys: dict[str, set[str]] = defaultdict(set)
    with events.open("r", encoding="utf-8") as source:
        for line in source:
            if not line.strip():
                continue
            event = json.loads(line)
            event_type = str(event.get("type", "<missing>"))
            types[event_type] += 1
            keys[event_type].update(str(key) for key in event)
    return {
        "event_types": dict(sorted(types.items())),
        "keys_by_type": {key: sorted(value) for key, value in sorted(keys.items())},
    }


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("left", type=Path)
    parser.add_argument("right", type=Path)
    args = parser.parse_args()
    left = fingerprint(args.left.resolve())
    right = fingerprint(args.right.resolve())
    print(json.dumps({"left": left, "right": right, "equal": left == right}, indent=2))
    return 0 if left == right else 2


if __name__ == "__main__":
    raise SystemExit(main())
