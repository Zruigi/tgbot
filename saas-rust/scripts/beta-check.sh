#!/usr/bin/env bash
set -euo pipefail

script_directory=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
rust_root=$(cd "$script_directory/.." && pwd)
repository_root=$(cd "$rust_root/.." && pwd)

if ! git -C "$repository_root" diff --quiet origin/master -- frontend static; then
  echo "Beta gate failed: frontend or static UI files differ from origin/master." >&2
  exit 1
fi

node "$script_directory/check-api-parity.mjs" \
  --report "$rust_root/target/beta-reports/api-static-registration.json"
cargo test --manifest-path "$rust_root/Cargo.toml" \
  -p tgbot-app --features dev-polling --bin bot-polling
cargo test --workspace --manifest-path "$rust_root/Cargo.toml"
cargo check --workspace --all-targets --manifest-path "$rust_root/Cargo.toml"

echo "Beta static gates passed. Runtime integration and acceptance gates are still required."
