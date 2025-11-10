#!/usr/bin/env python3
import csv
import sys
from pathlib import Path

FIELDNAMES = [
    "source",
    "kind",
    "commit_type",
    "count",
    "avg_ops",
    "max_ops",
    "avg_bytes",
    "max_bytes",
    "avg_commit_ms",
    "max_commit_ms",
    "duration_ms",
    "traversed",
    "pruned",
    "lock_hold_ms",
    "lock_yields",
    "lock_reacquires",
]


def parse_line(raw: str) -> dict:
    try:
        payload = raw.split("] ", 1)[1]
    except IndexError:
        return {}
    record = {}
    for token in payload.strip().split():
        if "=" not in token:
            continue
        key, value = token.split("=", 1)
        record[key] = value
    if not record:
        return {}
    record["kind"] = "commit" if "commit_type" in record else "summary"
    return record


def records_from_file(path: Path):
    with path.open("r", encoding="utf-8") as handle:
        for line in handle:
            if "[PRUNING METRICS]" not in line:
                continue
            record = parse_line(line)
            if record:
                record["source"] = str(path)
                yield record


def main(args: list[str]) -> int:
    if not args:
        print("Usage: collect_pruning_metrics.py <logfile> [<logfile>...]", file=sys.stderr)
        return 1

    writer = csv.DictWriter(sys.stdout, FIELDNAMES)
    writer.writeheader()
    for arg in args:
        path = Path(arg).expanduser()
        for record in records_from_file(path):
            writer.writerow({field: record.get(field, "") for field in FIELDNAMES})
    return 0


if __name__ == "__main__":
    raise SystemExit(main(sys.argv[1:]))
