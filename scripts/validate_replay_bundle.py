#!/usr/bin/env python3
"""Strict structural validator for ValCoach Replay Bundle v1."""

from __future__ import annotations

import argparse
import json
import math
from pathlib import Path
from typing import Any, Iterable


def iter_ndjson(path: Path) -> Iterable[tuple[int, dict[str, Any]]]:
    with path.open("r", encoding="utf-8") as source:
        for line_number, line in enumerate(source, 1):
            if not line.strip():
                continue
            value = json.loads(line)
            if not isinstance(value, dict):
                raise ValueError(f"{path}:{line_number}: record must be a JSON object")
            yield line_number, value


def is_finite_json(value: Any) -> bool:
    if isinstance(value, bool) or value is None or isinstance(value, (str, int)):
        return True
    if isinstance(value, float):
        return math.isfinite(value)
    if isinstance(value, list):
        return all(is_finite_json(child) for child in value)
    if isinstance(value, dict):
        return all(is_finite_json(child) for child in value.values())
    return False


def check_records(path: Path, required: tuple[str, ...]) -> tuple[int, list[str]]:
    count = 0
    errors: list[str] = []
    previous_time: int | None = None
    for line_number, record in iter_ndjson(path):
        count += 1
        missing = [field for field in required if field not in record]
        if missing:
            errors.append(f"{path.name}:{line_number}: missing {', '.join(missing)}")
        timestamp = record.get("time_ms")
        if not isinstance(timestamp, int) or timestamp < 0:
            errors.append(f"{path.name}:{line_number}: invalid time_ms")
        if previous_time is not None and isinstance(timestamp, int) and timestamp < previous_time:
            errors.append(f"{path.name}:{line_number}: time_ms is not monotonic")
        if isinstance(timestamp, int):
            previous_time = timestamp
        if not is_finite_json(record):
            errors.append(f"{path.name}:{line_number}: non-finite or unsupported JSON value")
        if len(errors) >= 50:
            break
    return count, errors


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("bundle", type=Path)
    parser.add_argument("--output", type=Path)
    args = parser.parse_args()
    bundle = args.bundle.resolve()
    manifest = json.loads((bundle / "manifest.json").read_text(encoding="utf-8"))
    errors: list[str] = []
    checks: dict[str, Any] = {}

    if manifest.get("schema_version") != 1:
        errors.append("manifest schema_version must be 1")
    for artifact in manifest.get("artifacts", []):
        if not (bundle / artifact).is_file():
            errors.append(f"declared artifact is missing: {artifact}")

    server_count, server_errors = check_records(
        bundle / "server_events.ndjson", ("schema_version", "replay_id", "group", "time_ms")
    )
    errors.extend(server_errors)
    checks["server_events"] = server_count
    expected = manifest.get("records", {}).get("server_events")
    if server_count != expected:
        errors.append(f"server event count mismatch: manifest={expected}, actual={server_count}")

    backend_status = manifest.get("backend", {}).get("status")
    if backend_status == "complete":
        event_count, event_errors = check_records(
            bundle / "parser_events.ndjson", ("type", "time_ms")
        )
        movement_count, movement_errors = check_records(
            bundle / "movement.ndjson", ("type", "time_ms", "position")
        )
        errors.extend(event_errors)
        errors.extend(movement_errors)
        checks["parser_events"] = event_count
        checks["movement"] = movement_count
        records = manifest.get("records", {})
        if event_count != records.get("normalized_events"):
            errors.append("parser event count does not match manifest")
        if movement_count != records.get("movement_samples"):
            errors.append("movement count does not match manifest")
        if manifest.get("integrity", {}).get("malformed_packets") != 0:
            errors.append("malformed_packets must be zero for a production Global bundle")
    elif backend_status != "unsupported":
        errors.append(f"backend status is not terminal: {backend_status!r}")

    result = {
        "validation_result": "PASS" if not errors else "FAIL",
        "backend_status": backend_status,
        "checks": checks,
        "errors": errors,
    }
    text = json.dumps(result, indent=2, ensure_ascii=False)
    print(text)
    if args.output:
        args.output.parent.mkdir(parents=True, exist_ok=True)
        args.output.write_text(text + "\n", encoding="utf-8")
    return 0 if not errors else 1


if __name__ == "__main__":
    raise SystemExit(main())
