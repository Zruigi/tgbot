#!/usr/bin/env bash
set -euo pipefail

script_directory=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
rust_root=$(cd "$script_directory/.." && pwd)
binary_path="$rust_root/target/release/bot-polling"

if [[ "${BETA_TEST_BOT_ENABLED:-}" != "true" ]]; then
  printf 'Refusing test Bot polling; set BETA_TEST_BOT_ENABLED=true explicitly\n' >&2
  exit 1
fi
if [[ -z "${BOT_TOKEN:-}" ]]; then
  printf 'Refusing test Bot polling; inject BOT_TOKEN into this process environment\n' >&2
  exit 1
fi

if [[ ! -x "$binary_path" ]]; then
  env -u BOT_TOKEN -u TELOXIDE_TOKEN cargo build --release --features dev-polling --bin bot-polling
fi

cd /tmp
exec env -i \
  PATH="$PATH" \
  HOME="${HOME:-/home/ubuntu}" \
  BOT_TOKEN="$BOT_TOKEN" \
  TELEGRAM_POLLING_ENABLED=true \
  RUST_LOG="${RUST_LOG:-info}" \
  "$binary_path"
