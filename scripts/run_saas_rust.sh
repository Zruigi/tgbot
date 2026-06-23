#!/bin/bash
set -euo pipefail

PROJECT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
SAAS_DIR="${PROJECT_DIR}/saas-rust"

if ! command -v cargo >/dev/null 2>&1; then
    echo "cargo 未安装，无法启动 Rust SaaS 后端" >&2
    exit 127
fi

cd "$SAAS_DIR"
exec cargo run
