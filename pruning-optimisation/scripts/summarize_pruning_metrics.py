#!/usr/bin/env python3
import csv
import glob
import os
import sys
from collections import defaultdict


def load_latest_metrics(base_dir: str):
    pattern = os.path.join(base_dir, "simpa-prune-*.csv")
    files = sorted(glob.glob(pattern))
    if not files:
        return []

    # Group by run label (from CSV), pick newest file per label
    latest_by_label = {}
    for path in files:
        with open(path, "r", encoding="utf-8") as f:
            reader = csv.DictReader(f)
            rows = list(reader)
            if not rows:
                continue
            label = rows[-1].get("simpa_run_label", "") or "(no-label)"
            latest_by_label[label] = path

    # Load all rows per label
    results = []
    for label, path in sorted(latest_by_label.items()):
        with open(path, "r", encoding="utf-8") as f:
            reader = csv.DictReader(f)
            rows = list(reader)
            if not rows:
                continue
            for row in rows:
                row["_label"] = label
                row["_file"] = os.path.basename(path)
                results.append(row)
    return results


def as_float(val: str):
    try:
        return float(val)
    except Exception:
        return None


def main():
    base_dir = sys.argv[1] if len(sys.argv) > 1 else os.path.join(
        os.getcwd(), "pruning-optimisation", "baseline", "experiments"
    )
    rows = load_latest_metrics(base_dir)
    if not rows:
        print("No metrics CSV files found.")
        return

    # Aggregate per label: mean of summary fields + mean of commit stats
    per_label = {}
    for r in rows:
        label = r.get("_label", "(no-label)")
        per_label.setdefault(label, {"summary": [], "commit": [], "_file": r.get("_file", "")})
        kind = r.get("kind", "")
        if kind == "summary":
            per_label[label]["summary"].append(r)
        elif kind == "commit":
            per_label[label]["commit"].append(r)

    def mean(vals):
        vals = [v for v in vals if v is not None]
        return sum(vals) / len(vals) if vals else None

    def max_val(vals):
        vals = [v for v in vals if v is not None]
        return max(vals) if vals else None

    def group_for_label(label: str) -> str:
        if label.startswith("reach-"):
            return "reach-batched (no bodies)"
        return "full-batch (bodies+headers)"

    fields = [
        "_label",
        "group",
        "summary_count",
        "summary_pruned_count",
        "duration_ms_avg",
        "duration_ms_max",
        "traversed_avg",
        "pruned_avg",
        "commit_count_avg",
        "avg_ops_avg",
        "avg_bytes_avg",
        "avg_commit_ms_avg",
        "max_ops_max",
        "max_bytes_max",
        "max_commit_ms_max",
        "config_lock_max_ms",
        "config_batch_max_ms",
        "config_batch_max_ops",
        "config_batch_max_bytes",
        "config_batch_max_blocks",
        "config_batch_bodies",
        "_file",
    ]

    rows_out = []
    for label, data in per_label.items():
        summary = data["summary"]
        commits = data["commit"]
        summary_count = len(summary)
        summary_pruned_count = sum(1 for r in summary if (as_float(r.get("pruned", "")) or 0) > 0)

        duration_vals = [as_float(r.get("duration_ms", "")) for r in summary]
        traversed_vals = [as_float(r.get("traversed", "")) for r in summary]
        pruned_vals = [as_float(r.get("pruned", "")) for r in summary]

        commit_count_vals = [as_float(r.get("count", "")) for r in commits]
        avg_ops_vals = [as_float(r.get("avg_ops", "")) for r in commits]
        avg_bytes_vals = [as_float(r.get("avg_bytes", "")) for r in commits]
        avg_commit_ms_vals = [as_float(r.get("avg_commit_ms", "")) for r in commits]
        max_ops_vals = [as_float(r.get("max_ops", "")) for r in commits]
        max_bytes_vals = [as_float(r.get("max_bytes", "")) for r in commits]
        max_commit_ms_vals = [as_float(r.get("max_commit_ms", "")) for r in commits]

        # Pull config fields from the last summary row that has them
        config_row = next((r for r in reversed(summary) if r.get("config_batch_max_blocks") is not None), summary[-1] if summary else {})

        rows_out.append({
            "_label": label,
            "group": group_for_label(label),
            "summary_count": summary_count,
            "summary_pruned_count": summary_pruned_count,
            "duration_ms_avg": mean(duration_vals),
            "duration_ms_max": max_val(duration_vals),
            "traversed_avg": mean(traversed_vals),
            "pruned_avg": mean(pruned_vals),
            "commit_count_avg": mean(commit_count_vals),
            "avg_ops_avg": mean(avg_ops_vals),
            "avg_bytes_avg": mean(avg_bytes_vals),
            "avg_commit_ms_avg": mean(avg_commit_ms_vals),
            "max_ops_max": max_val(max_ops_vals),
            "max_bytes_max": max_val(max_bytes_vals),
            "max_commit_ms_max": max_val(max_commit_ms_vals),
            "config_lock_max_ms": config_row.get("config_lock_max_ms", ""),
            "config_batch_max_ms": config_row.get("config_batch_max_ms", ""),
            "config_batch_max_ops": config_row.get("config_batch_max_ops", ""),
            "config_batch_max_bytes": config_row.get("config_batch_max_bytes", ""),
            "config_batch_max_blocks": config_row.get("config_batch_max_blocks", ""),
            "config_batch_bodies": config_row.get("config_batch_bodies", ""),
            "_file": data["_file"],
        })

    rows_out.sort(key=lambda r: r.get("duration_ms_avg") or float("inf"))

    writer = csv.writer(sys.stdout)
    writer.writerow(fields)
    for r in rows_out:
        writer.writerow([r.get(f, "") for f in fields])


if __name__ == "__main__":
    main()
