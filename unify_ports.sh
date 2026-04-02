#!/usr/bin/env bash
set -euo pipefail

ROOT="/home/ubuntu/tgbot"
LOG_DIR="$ROOT/logs"
BACKEND_PORT=8080
GATEWAY_PORT=80
DEV_PORT=5173
CLOUDFLARED_BIN="/usr/bin/cloudflared"
CLOUDFLARED_LOG="$LOG_DIR/cloudflared.log"
CLOUDFLARED_STDOUT="$LOG_DIR/cloudflared.stdout"
LEGACY_CLOUDFLARED_LOG="$ROOT/cloudflared.log"
BACKEND_LOG="$LOG_DIR/backend.log"

mkdir -p "$LOG_DIR"

printf "\n[1/6] 关闭旧开发端口与 Vite 进程（%s）...\n" "$DEV_PORT"
fuser -k "${DEV_PORT}/tcp" >/dev/null 2>&1 || true
pkill -f "npm run dev" >/dev/null 2>&1 || true
pkill -f "vite --host" >/dev/null 2>&1 || true

printf "[2/6] 清理历史 cloudflared 隧道（避免多端口并发暴露）...\n"
pkill -f "cloudflared tunnel --url" >/dev/null 2>&1 || true
sleep 1
# 清理遗留日志，避免读取到旧 tunnel URL
: > "$CLOUDFLARED_LOG"
: > "$CLOUDFLARED_STDOUT"
rm -f "$LEGACY_CLOUDFLARED_LOG" >/dev/null 2>&1 || true

printf "[3/6] 校验网关端口 %s（nginx）...\n" "$GATEWAY_PORT"
if ! ss -ltn "( sport = :$GATEWAY_PORT )" | grep -q ":${GATEWAY_PORT}"; then
  echo "❌ 未检测到 ${GATEWAY_PORT} 端口监听，请先启动 nginx（sudo systemctl start nginx）"
  exit 1
fi

printf "[4/6] 确认后端 %s 在线（必要时自动拉起）...\n" "$BACKEND_PORT"
if ! ss -ltn "( sport = :$BACKEND_PORT )" | grep -q ":${BACKEND_PORT}"; then
  nohup "$ROOT/venv/bin/python3" "$ROOT/main.py" >"$BACKEND_LOG" 2>&1 &
  sleep 2
fi

if ! ss -ltn "( sport = :$BACKEND_PORT )" | grep -q ":${BACKEND_PORT}"; then
  echo "❌ 后端 ${BACKEND_PORT} 未启动成功，请查看 $BACKEND_LOG"
  exit 1
fi

printf "[5/6] 启动单一隧道：cloudflared -> 127.0.0.1:%s ...\n" "$GATEWAY_PORT"
nohup "$CLOUDFLARED_BIN" tunnel --url "http://127.0.0.1:${GATEWAY_PORT}" --logfile "$CLOUDFLARED_LOG" >"$CLOUDFLARED_STDOUT" 2>&1 &
sleep 2

printf "[6/6] 输出端口收敛状态与外网入口...\n"
python3 - <<'PY'
import os
import re
import subprocess

root = "/home/ubuntu/tgbot"
log_path = f"{root}/logs/cloudflared.log"

def sh(cmd: str) -> str:
    r = subprocess.run(["bash", "-lc", cmd], capture_output=True, text=True)
    return r.stdout.strip()

print("\n核心端口状态（生产应只关注 80 / 8080）：")
for p in (80, 8080, 5173):
    out = sh(f"ss -ltnp '( sport = :{p} )'")
    ok = bool(out and f":{p}" in out)
    print(f"- {p}: {'✅' if ok else '❌'}")

print("\n当前 cloudflared 进程（应仅 1 条 tunnel --url）：")
cf = sh("pgrep -af 'cloudflared tunnel --url'")
print(cf or "(none)")

url = ""
if os.path.exists(log_path):
    with open(log_path, "r", encoding="utf-8", errors="ignore") as f:
        text = f.read()
    m = re.findall(r"https://[\\w\-]+\\.trycloudflare\\.com", text)
    if m:
        url = m[-1]

if url:
    print("\n✅ 外网入口（统一网关）:")
    print(url)
else:
    print("\n⚠️ 尚未解析到 trycloudflare 地址，请稍后查看 logs/cloudflared.log")
PY
