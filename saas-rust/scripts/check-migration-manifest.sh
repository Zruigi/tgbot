#!/usr/bin/env bash
set -euo pipefail

script_directory=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
rust_root=$(cd "$script_directory/.." && pwd)
manifest_path="$script_directory/beta-migrations.list"

if [[ ! -r "$manifest_path" ]]; then
  echo "Migration gate failed: missing migration manifest $manifest_path." >&2
  exit 1
fi

mapfile -t listed_migrations < <(awk 'NF && $1 !~ /^#/ {print $1}' "$manifest_path")
if (( ${#listed_migrations[@]} == 0 )); then
  echo "Migration gate failed: migration manifest is empty." >&2
  exit 1
fi

declare -A seen_migrations=()
for migration in "${listed_migrations[@]}"; do
  if [[ -n "${seen_migrations[$migration]:-}" ]]; then
    echo "Migration gate failed: duplicate manifest entry $migration." >&2
    exit 1
  fi
  seen_migrations["$migration"]=1
  if [[ "$migration" != *.sql || ! -f "$rust_root/migrations/$migration" ]]; then
    echo "Migration gate failed: manifest entry is not a migration file: $migration." >&2
    exit 1
  fi
done

mapfile -t actual_migrations < <(
  find "$rust_root/migrations" -maxdepth 1 -type f -name '*.sql' -printf '%f\n' | LC_ALL=C sort
)
mapfile -t sorted_listed_migrations < <(
  printf '%s\n' "${listed_migrations[@]}" | LC_ALL=C sort
)

if ! diff -u \
  <(printf '%s\n' "${actual_migrations[@]}") \
  <(printf '%s\n' "${sorted_listed_migrations[@]}"); then
  echo "Migration gate failed: manifest and migrations directory differ." >&2
  exit 1
fi

printf 'Migration manifest passed: %d files, ordered execution list and directory are aligned.\n' "${#listed_migrations[@]}"
