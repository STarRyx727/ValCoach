#!/usr/bin/env python3
"""Fetch VALORANT map metadata from valorant-api.com and save as JSON."""

import json
import os
import sys
import urllib.request

API_URL = "https://valorant-api.com/v1/maps"
OUTPUT_DIR = os.path.join(os.path.dirname(os.path.dirname(os.path.abspath(__file__))), "data", "maps")

def main():
    print(f"Fetching maps from {API_URL}...")
    with urllib.request.urlopen(API_URL, timeout=30) as resp:
        data = json.loads(resp.read())

    maps = data.get("data", [])
    if not maps:
        print("No maps returned!")
        sys.exit(1)

    os.makedirs(OUTPUT_DIR, exist_ok=True)

    for m in maps:
        map_url = m.get("mapUrl", "")
        display_name = m.get("displayName", "")
        if not map_url or not display_name:
            continue

        callouts = []
        for c in m.get("callouts", []) or []:
            loc = c.get("location", {})
            callouts.append({
                "region_name": c.get("regionName", ""),
                "super_region_name": c.get("superRegionName", ""),
                "location": {"x": loc.get("x", 0), "y": loc.get("y", 0)},
            })

        meta = {
            "display_name": display_name,
            "map_url": map_url,
            "x_multiplier": m.get("xMultiplier", 1.0),
            "y_multiplier": m.get("yMultiplier", 1.0),
            "x_scalar_to_add": m.get("xScalarToAdd", 0.0),
            "y_scalar_to_add": m.get("yScalarToAdd", 0.0),
            "callouts": callouts,
        }

        out_path = os.path.join(OUTPUT_DIR, f"{map_url.replace('/', '_')}.json")
        with open(out_path, "w", encoding="utf-8") as f:
            json.dump(meta, f, indent=2, ensure_ascii=False)
        print(f"  {display_name} ({map_url}): {len(callouts)} callouts -> {out_path}")

    print(f"\nDone! {len(maps)} maps saved to {OUTPUT_DIR}")

if __name__ == "__main__":
    main()
