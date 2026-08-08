#!/usr/bin/env bash
# ============================================================================
# tunnel-guard.sh — Cloudflare Quick Tunnel 守护脚本 (Rust tgbot 版)
#
# 作用: 定期检测当前 FRONTEND_URL 隧道域名是否存活; 失效时自动:
#   1. 杀掉旧的 cloudflared 进程
#   2. 重启临时隧道, 抓取新 trycloudflare 域名
#   3. 更新 .env.production 的 WEB_BASE_URL / FRONTEND_URL / TELEGRAM_WEBHOOK_BASE_URL
#   4. 用与 Rust 端一致的 HMAC 算法重设 Telegram webhook (secret_token)
#   5. 重设「控制台」菜单按钮 (全局 + 管理员)
#   6. 发送 Telegram 通知给管理员
#
# 由 systemd timer 每 2 分钟触发一次。幂等: 域名存活时直接退出。
# ============================================================================
set -uo pipefail

ENV_FILE="/home/ubuntu/tgbot-rust-beta/saas-rust/.env.production"
CF_LOG="/tmp/cloudflared-rust.log"
GUARD_LOG="/tmp/tunnel-guard.log"
ORIGIN_URL="http://localhost:18090"
ADMIN_ID="6889612019"
LOCK_FILE="/tmp/tunnel-guard.lock"
MAX_WAIT_SECS=40
TELEGRAM_API="https://api.telegram.org"

log() { echo "[$(date '+%Y-%m-%d %H:%M:%S')] $*" >> "$GUARD_LOG"; }

# ---- 读取 .env.production 变量 ----
env_get() { grep -oP "^$1=\K.*" "$ENV_FILE" 2>/dev/null | head -n1 | tr -d '"' | tr -d "'"; }

# ---- 域名存活检测 ----
url_alive() {
  local url="$1"
  [[ -z "$url" ]] && return 1
  local code
  code=$(curl -s -o /dev/null -w '%{http_code}' --max-time 10 "$url" 2>/dev/null)
  [[ "$code" == "200" ]]
}

# ---- 从 cloudflared 日志提取域名 ----
extract_cf_url() {
  grep -oP 'https://[a-zA-Z0-9\-\.]+\.trycloudflare\.com' "$CF_LOG" 2>/dev/null | tail -n1
}

# ---- 重启隧道并等待新域名 ----
restart_tunnel() {
  log "重启 cloudflared 隧道 (origin=$ORIGIN_URL)"
  local old_pids
  old_pids=$(ps -eo pid=,args= 2>/dev/null | awk '$2=="cloudflared" && /--url/ && /localhost:18090/ {print $1}')
  for p in $old_pids; do
    kill "$p" 2>/dev/null || true
  done
  sleep 2
  : > "$CF_LOG"
  nohup cloudflared tunnel --no-autoupdate --url "$ORIGIN_URL" >> "$CF_LOG" 2>&1 &
  local url="" waited=0
  while [ "$waited" -lt "$MAX_WAIT_SECS" ]; do
    url=$(extract_cf_url)
    if [ -n "$url" ] && url_alive "$url"; then
      echo "$url"
      return 0
    fi
    sleep 2
    waited=$((waited + 2))
  done
  echo ""
  return 1
}

# ---- 更新 .env.production ----
update_env() {
  local new_url="$1"
  for key in WEB_BASE_URL FRONTEND_URL TELEGRAM_WEBHOOK_BASE_URL; do
    if grep -q "^${key}=" "$ENV_FILE"; then
      sed -i "s|^${key}=.*|${key}=${new_url}|" "$ENV_FILE"
    else
      echo "${key}=${new_url}" >> "$ENV_FILE"
    fi
  done
  log "已更新 .env.production 三个 URL -> ${new_url}"
}

# ---- 用与 Rust 一致的算法派生 webhook secret ----
derive_webhook_secret() {
  local master_key="$1" bot_id="$2" bot_token="$3"
  python3 - "$master_key" "$bot_id" "$bot_token" <<'PYEOF'
import sys, hmac, hashlib
master_key, bot_id, bot_token = sys.argv[1], sys.argv[2], sys.argv[3]
mac = hmac.new(master_key.encode(), f"telegram-webhook\0{bot_id}\0{bot_token}".encode(), hashlib.sha256)
print("wh_" + mac.hexdigest())
PYEOF
}

# ---- 重设 webhook ----
update_webhook() {
  local new_url="$1"
  local bot_id="$2" bot_token="$3" master_key="$4"
  local secret webhook_url resp
  secret=$(derive_webhook_secret "$master_key" "$bot_id" "$bot_token")
  webhook_url="${new_url}/telegram/webhook/${bot_id}"
  resp=$(curl -s --max-time 15 "${TELEGRAM_API}/bot${bot_token}/setWebhook" \
    -F "url=${webhook_url}" \
    -F "secret_token=${secret}" \
    -F "max_connections=40")
  log "setWebhook -> ${webhook_url} | resp=${resp}"
}

# ---- 重设菜单按钮 ----
update_menu_button() {
  local new_url="$1" bot_token="$2"
  local btn_url="${new_url}/miniapp?entry=admin_console"
  local payload
  payload=$(printf '{"menu_button":{"type":"web_app","text":"控制台","web_app":{"url":"%s"}}}' "$btn_url")
  curl -s --max-time 15 "${TELEGRAM_API}/bot${bot_token}/setChatMenuButton" \
    -H "Content-Type: application/json" -d "$payload" > /dev/null 2>&1
  payload=$(printf '{"chat_id":%s,"menu_button":{"type":"web_app","text":"控制台","web_app":{"url":"%s"}}}' "$ADMIN_ID" "$btn_url")
  curl -s --max-time 15 "${TELEGRAM_API}/bot${bot_token}/setChatMenuButton" \
    -H "Content-Type: application/json" -d "$payload" > /dev/null 2>&1
  log "已重设菜单按钮 -> ${btn_url}"
}

# ---- 通知管理员 ----
notify_admin() {
  local msg="$1" bot_token="$2"
  curl -s --max-time 15 "${TELEGRAM_API}/bot${bot_token}/sendMessage" \
    -H "Content-Type: application/json" \
    -d "$(python3 -c "import json,sys; print(json.dumps({'chat_id':${ADMIN_ID},'text':sys.argv[1]}))" "$msg")" > /dev/null 2>&1
}

# ============================ main ============================
main() {
  # 简单互斥锁, 防止 timer 重叠执行
  if command -v flock >/dev/null 2>&1; then
    exec 9>"$LOCK_FILE"
    flock -n 9 || { log "另一个实例正在运行, 跳过"; exit 0; }
  fi

  local bot_token master_key config_url
  bot_token=$(env_get "BOT_TOKEN")
  master_key=$(env_get "TGBOT_SECRET_ENCRYPTION_KEY")
  config_url=$(env_get "FRONTEND_URL")

  if [[ -z "$bot_token" || -z "$master_key" ]]; then
    log "错误: .env.production 缺少 BOT_TOKEN 或 TGBOT_SECRET_ENCRYPTION_KEY, 退出"
    exit 1
  fi

  if url_alive "$config_url"; then
    exit 0  # 一切正常
  fi

  log "检测到 FRONTEND_URL 失效: ${config_url:-<empty>}"

  local new_url
  new_url=$(restart_tunnel)
  if [[ -z "$new_url" ]]; then
    log "恢复失败: 未能获取存活隧道域名"
    notify_admin "⚠️ tgbot 隧道自动恢复失败, 请检查服务器 cloudflared" "$bot_token"
    exit 1
  fi

  local bot_id
  bot_id=$(echo "$bot_token" | cut -d: -f1)

  update_env "$new_url"
  update_webhook "$new_url" "$bot_id" "$bot_token" "$master_key"
  update_menu_button "$new_url" "$bot_token"
  notify_admin "🔄 tgbot 隧道域名已自动更新: ${new_url}" "$bot_token"
  log "恢复完成, 新域名=${new_url}"
  exit 0
}

main "$@"
