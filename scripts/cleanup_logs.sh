#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
LOG_DIR="${LOG_DIR:-$ROOT_DIR/logs}"
RETENTION_HOURS="${LOG_RETENTION_HOURS:-48}"

mkdir -p "$LOG_DIR"

if ! [[ "$RETENTION_HOURS" =~ ^[0-9]+$ ]] || [ "$RETENTION_HOURS" -lt 1 ]; then
  echo "Invalid LOG_RETENTION_HOURS: $RETENTION_HOURS"
  exit 1
fi

python3 - "$LOG_DIR" "$RETENTION_HOURS" <<'PY'
import sys
from pathlib import Path
from datetime import datetime, timedelta

log_dir = Path(sys.argv[1])
retention_hours = int(sys.argv[2])
cutoff = datetime.now() - timedelta(hours=retention_hours)

deleted = []
for path in log_dir.rglob("*.log"):
    try:
        mtime = datetime.fromtimestamp(path.stat().st_mtime)
        if mtime < cutoff:
            path.unlink(missing_ok=True)
            deleted.append(path.name)
    except Exception as exc:
        print(f"WARN: failed to process {path.name}: {exc}")

print(f"Log cleanup complete. retention_hours={retention_hours}, deleted={len(deleted)}")
if deleted:
    print("Deleted:")
    for name in deleted:
        print(f"- {name}")
PY
