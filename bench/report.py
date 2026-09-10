#!/usr/bin/env python3
"""Validate frozen benchmark records and print comparable distributions."""
import json
import sys
from collections import defaultdict
from pathlib import Path

root = Path(__file__).parent
spec = json.loads((root / "frozen.json").read_text())
if spec["model"] == "EXTERNAL_MODEL_SELECTION_REQUIRED":
    raise SystemExit("gate: set a frozen model identity in bench/frozen.json before benchmark runs")
records = [json.loads(line) for line in sys.stdin if line.strip()]
required = {"arm", "task_id", "turns", "failed_invocations", "correct", "outcome", "elapsed_seconds"}
for record in records:
    missing = required - record.keys()
    if missing or record["arm"] not in spec["arms"] or record["task_id"] not in {t["id"] for t in spec["tasks"]}:
        raise SystemExit(f"invalid record: {record}")
    if isinstance(record["failed_invocations"], list):
        record["failed_invocations"] = len(record["failed_invocations"])
    if not isinstance(record["failed_invocations"], int):
        raise SystemExit(f"invalid failed_invocations: {record}")
groups = defaultdict(list)
for record in records:
    groups[record["arm"]].append(record)
report = {"source_revision": spec["source_revision"], "records": len(records), "arms": {}}
for arm, rows in groups.items():
    report["arms"][arm] = {
        "turns": sorted(r["turns"] for r in rows),
        "failed_invocations": sorted(r["failed_invocations"] for r in rows),
        "correctness": sum(bool(r["correct"]) for r in rows) / len(rows),
        "failures": [r["outcome"] for r in rows if r["outcome"] != "success"],
    }
if {"feature_on", "feature_off"} <= groups.keys():
    on, off = report["arms"]["feature_on"], report["arms"]["feature_off"]
    correctness_delta = on["correctness"] - off["correctness"]
    failed_delta = sum(on["failed_invocations"]) / len(on["failed_invocations"]) - sum(off["failed_invocations"]) / len(off["failed_invocations"])
    guards = spec["adoption_guards"]
    report["guards"] = {
        "correctness_delta": correctness_delta,
        "failed_call_delta": failed_delta,
        "pass": correctness_delta >= guards["minimum_correctness_delta"] and failed_delta <= guards["maximum_failed_call_delta"],
    }
print(json.dumps(report, sort_keys=True, indent=2))
