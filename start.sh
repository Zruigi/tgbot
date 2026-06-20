#!/bin/bash

# ==========================================
# 配置区域
# ==========================================
PROJECT_DIR="/home/ubuntu/tgbot"
LOG_ROOT="/home/ubuntu/tgbot/logs"
LOG_APP_DIR="${LOG_ROOT}/app"
LOG_BACKEND_DIR="${LOG_APP_DIR}/backend"
LOG_TUNNEL_DIR="${LOG_ROOT}/tunnel"
LOG_CLOUDFLARED_DIR="${LOG_TUNNEL_DIR}/cloudflared"
LOG_RUNTIME_DIR="${LOG_ROOT}/runtime"
LOG_AI_DIR="${LOG_ROOT}/ai"
LOG_FILE="${LOG_CLOUDFLARED_DIR}/cloudflared.log"
BACKEND_LOG="${LOG_BACKEND_DIR}/backend.log"
ENV_FILE=".env"
ENV_BACKUP_KEEP=5
PYTHON_BIN="${PROJECT_DIR}/venv/bin/python3"
if [ ! -x "$PYTHON_BIN" ]; then
    PYTHON_BIN="$(command -v python3)"
fi
PYTHON_CMD="${PYTHON_BIN} main.py"
TUNNEL_PORT=8080
LITELLM_PORT=4000
LITELLM_CONTAINER="litellm"
TUNNEL_MAX_RETRIES=3
TUNNEL_WAIT_BASE=10
TUNNEL_WAIT_MAX=45
BACKEND_ENV_RETRY_MAX=2
# 兼容模式：默认不因系统级 80 端口隧道而硬失败；设为 1 可恢复严格失败
STRICT_TUNNEL_SANITY="${STRICT_TUNNEL_SANITY:-0}"

# ==========================================
# 颜色定义
# ==========================================
GREEN='\033[0;32m'
RED='\033[0;31m'
YELLOW='\033[1;33m'
NC='\033[0m'

backup_env_file() {
    local env_file="$1"
    local keep_count="${2:-$ENV_BACKUP_KEEP}"

    [ -f "$env_file" ] || return 0

    cp "$env_file" "${env_file}.bak.$(date +%s)" 2>/dev/null || true

    [[ "$keep_count" =~ ^[0-9]+$ ]] || keep_count=$ENV_BACKUP_KEEP

    mapfile -t backups < <(ls -1t "${env_file}".bak.* 2>/dev/null || true)
    local total=${#backups[@]}
    if [ "$total" -le "$keep_count" ]; then
        return 0
    fi

    for ((i=keep_count; i<total; i++)); do
        rm -f "${backups[i]}" 2>/dev/null || true
    done
}

load_env_to_process() {
    local env_file="${1:-$ENV_FILE}"

    if [ ! -f "$env_file" ]; then
        echo -e "${YELLOW}⚠️ 未找到 .env 文件: ${env_file}${NC}"
        return 1
    fi

    while IFS='=' read -r key value; do
        [[ -z "$key" || "$key" =~ ^[[:space:]]*# ]] && continue
        key=$(echo "$key" | xargs)
        value=$(echo "$value" | xargs | sed "s/^['\"]//;s/['\"]$//")
        export "${key}=${value}"
    done < "$env_file"
}

write_env_url() {
    local url="$1"

    backup_env_file "$ENV_FILE"

    if grep -q "^WEB_BASE_URL=" "$ENV_FILE"; then
        sed -i "s|^WEB_BASE_URL=.*|WEB_BASE_URL=${url}|" "$ENV_FILE"
    else
        echo "WEB_BASE_URL=${url}" >> "$ENV_FILE"
    fi

    if grep -q "^FRONTEND_URL=" "$ENV_FILE"; then
        sed -i "s|^FRONTEND_URL=.*|FRONTEND_URL=${url}|" "$ENV_FILE"
    else
        echo "FRONTEND_URL=${url}" >> "$ENV_FILE"
    fi

    export WEB_BASE_URL="$url"
    export FRONTEND_URL="$url"
}

extract_tunnel_url() {
    local log_file="$1"
    grep -oP "https://[a-zA-Z0-9\-\.]+\.trycloudflare\.com" "$log_file" 2>/dev/null | tail -n 1
}

wait_backend_ready() {
    local timeout="${1:-20}"
    local elapsed=0

    while [ "$elapsed" -lt "$timeout" ]; do
        if curl -s --max-time 2 "http://localhost:${TUNNEL_PORT}" > /dev/null 2>&1; then
            return 0
        fi
        sleep 1
        elapsed=$((elapsed + 1))
    done

    return 1
}

get_backend_env_var() {
    local pid="$1"
    local key="$2"

    [ -r "/proc/${pid}/environ" ] || return 1
    tr '\0' '\n' < "/proc/${pid}/environ" | sed -n "s/^${key}=//p" | tail -n 1
}

validate_backend_env_url() {
    local pid="$1"
    local expected_url="$2"

    local pid_web_base=""
    local pid_frontend=""

    pid_web_base="$(get_backend_env_var "$pid" "WEB_BASE_URL" 2>/dev/null || true)"
    pid_frontend="$(get_backend_env_var "$pid" "FRONTEND_URL" 2>/dev/null || true)"

    # systemd 服务通过 EnvironmentFile 加载 .env，/proc/PID/environ 可能不含变量
    # 此时回退到直接读取 .env 文件验证
    if [ -z "$pid_web_base" ] && [ -z "$pid_frontend" ]; then
        pid_web_base="$(grep "^WEB_BASE_URL=" "$ENV_FILE" 2>/dev/null | cut -d= -f2- | sed "s/^['\"]//;s/['\"]$//")"
        pid_frontend="$(grep "^FRONTEND_URL=" "$ENV_FILE" 2>/dev/null | cut -d= -f2- | sed "s/^['\"]//;s/['\"]$//")"
    fi

    # 兼容判定：
    # 1) FRONTEND_URL 命中即可视为生效（按钮直接使用）
    # 2) FRONTEND_URL 为空时，允许 WEB_BASE_URL 命中
    if [ "$pid_frontend" = "$expected_url" ] || { [ -z "$pid_frontend" ] && [ "$pid_web_base" = "$expected_url" ]; }; then
        return 0
    fi

    echo -e "${YELLOW}⚠️ 后端进程环境变量未匹配目标域名${NC}"
    echo -e "${YELLOW}   期望 WEB_BASE_URL/FRONTEND_URL = ${expected_url}${NC}"
    echo -e "${YELLOW}   实际 WEB_BASE_URL=${pid_web_base:-<empty>} FRONTEND_URL=${pid_frontend:-<empty>}${NC}"
    return 1
}

sync_merchants_to_supabase_if_enabled() {
    load_env_to_process "$ENV_FILE" || true
    if [ "${SYNC_MERCHANTS_TO_SUPABASE_ON_START:-0}" != "1" ]; then
        return 0
    fi
    if [ -z "${SUPABASE_URL:-}" ] || [ -z "${SUPABASE_SERVICE_ROLE_KEY:-}" ]; then
        echo -e "${YELLOW}⚠️ 已开启商户同步，但缺少 SUPABASE_URL 或 SUPABASE_SERVICE_ROLE_KEY，跳过${NC}"
        return 0
    fi
    echo -e "${YELLOW}🔄 同步当前商户数据到 Supabase...${NC}"
    if "$PYTHON_BIN" "${PROJECT_DIR}/scripts/sync_merchants_to_supabase.py"; then
        echo -e "${GREEN}✅ 商户数据已同步到 Supabase${NC}"
    else
        echo -e "${RED}❌ 商户数据同步 Supabase 失败${NC}"
        return 1
    fi
}

is_pid_alive() {
    local pid="$1"
    [ -n "$pid" ] && kill -0 "$pid" 2>/dev/null
}

is_backend_port_in_use() {
    ss -ltn "sport = :${TUNNEL_PORT}" 2>/dev/null | awk 'NR>1 {print}' | grep -q .
}

get_backend_port_pids() {
    ss -ltnp "sport = :${TUNNEL_PORT}" 2>/dev/null | sed -n 's/.*pid=\([0-9]\+\).*/\1/p' | sort -u
}

wait_backend_shutdown() {
    local timeout="${1:-20}"
    local elapsed=0

    while [ "$elapsed" -lt "$timeout" ]; do
        local backend_pids=""
        backend_pids="$(pgrep -f "$PYTHON_CMD" 2>/dev/null || true)"
        if [ -z "$backend_pids" ] && ! is_backend_port_in_use; then
            return 0
        fi

        sleep 1
        elapsed=$((elapsed + 1))
    done

    return 1
}

kill_backend_processes() {
    # 优先停止 systemd 服务，防止 Restart=always 自动拉起
    if command -v systemctl >/dev/null 2>&1 && systemctl is-active --quiet tgbot.service 2>/dev/null; then
        echo -e "${YELLOW}  停止 systemd 服务 tgbot.service...${NC}"
        sudo systemctl stop tgbot.service 2>/dev/null || systemctl stop tgbot.service 2>/dev/null || true
        sleep 2
    fi

    pkill -f "$PYTHON_CMD" 2>/dev/null || true
    pkill -f "python3 main.py" 2>/dev/null || true
    pkill -f "python main.py" 2>/dev/null || true

    local port_pids=""
    port_pids="$(get_backend_port_pids 2>/dev/null || true)"
    if [ -n "$port_pids" ]; then
        echo -e "${YELLOW}⚠️ 检测到 ${TUNNEL_PORT} 端口占用进程，尝试清理: ${port_pids}${NC}"
        echo "$port_pids" | xargs -r kill 2>/dev/null || true
    fi

    if wait_backend_shutdown 20; then
        return 0
    fi

    local backend_pids=""
    backend_pids="$(pgrep -f "$PYTHON_CMD" 2>/dev/null || true)"
    if [ -z "$backend_pids" ]; then
        backend_pids="$(pgrep -f "python3 main.py|python main.py" 2>/dev/null || true)"
    fi

    port_pids="$(get_backend_port_pids 2>/dev/null || true)"
    if [ -n "$port_pids" ]; then
        backend_pids="${backend_pids}
${port_pids}"
    fi
    backend_pids="$(echo "$backend_pids" | awk 'NF' | sort -u)"

    if [ -n "$backend_pids" ]; then
        echo -e "${YELLOW}⚠️ 检测到后端进程未正常退出，尝试强制终止: ${backend_pids}${NC}"
        echo "$backend_pids" | xargs -r kill -9 2>/dev/null || true
    fi

    sleep 1
    wait_backend_shutdown 10
}

start_python_backend_instance() {
    load_env_to_process "$ENV_FILE"

    if is_backend_port_in_use; then
        echo -e "${RED}❌ 启动前检测到 ${TUNNEL_PORT} 端口仍被占用，旧后端可能未退出${NC}"
        return 1
    fi

    # 优先通过 systemd 服务启动（确保与 tgbot.service 协调）
    if command -v systemctl >/dev/null 2>&1 && [ -f /etc/systemd/system/tgbot.service ]; then
        # 将当前 shell 导出的环境变量同步到 .env，systemd 服务通过 EnvironmentFile 加载
        sudo systemctl start tgbot.service 2>/dev/null || systemctl start tgbot.service 2>/dev/null || true
        sleep 3
        if systemctl is-active --quiet tgbot.service 2>/dev/null; then
            PYTHON_PID=$(systemctl show --property MainPID --value tgbot.service 2>/dev/null)
            return 0
        fi
        echo -e "${YELLOW}⚠️ systemd 启动失败，回退到 nohup 方式${NC}"
    fi

    # 回退：使用当前 shell 已导出的环境变量启动
    nohup "$PYTHON_BIN" main.py > "$BACKEND_LOG" 2>&1 &

    PYTHON_PID=$!
}

ensure_no_stale_cloudflared_80() {
    local stale
    stale=$(ps -eo pid=,comm=,args= | awk '$2 == "cloudflared" && /tunnel --url/ && ($0 ~ /127\\.0\\.0\\.1:80([^0-9]|$)/ || $0 ~ /localhost:80([^0-9]|$)/) {print $1}')

    if [ -z "$stale" ]; then
        return 0
    fi

    echo -e "${YELLOW}⚠️ 检测到残留 80 端口 Cloudflare 隧道进程，尝试清理...${NC}"
    echo "$stale" | xargs -r kill -9 2>/dev/null || true
    sleep 1

    local stale_after
    stale_after=$(ps -eo pid=,comm=,args= | awk '$2 == "cloudflared" && /tunnel --url/ && ($0 ~ /127\\.0\\.0\\.1:80([^0-9]|$)/ || $0 ~ /localhost:80([^0-9]|$)/) {print $1}')
    if [ -n "$stale_after" ]; then
        echo -e "${YELLOW}⚠️ 仍存在 80 端口 Cloudflare 隧道进程: ${stale_after}${NC}"
        if [ "$STRICT_TUNNEL_SANITY" = "1" ]; then
            echo -e "${RED}❌ STRICT_TUNNEL_SANITY=1，按严格模式中止${NC}"
            return 1
        fi
        echo -e "${YELLOW}⚠️ 兼容模式继续执行（可导出 STRICT_TUNNEL_SANITY=1 开启严格模式）${NC}"
        return 0
    fi

    echo -e "${GREEN}✅ 已清理残留 80 端口 Cloudflare 隧道进程${NC}"
    return 0
}

ensure_only_expected_cloudflared_target() {
    local bad
    bad=$(ps -eo pid=,comm=,args= | awk -v target="http://localhost:${TUNNEL_PORT}" '$2 == "cloudflared" && /tunnel --url/ && index($0, target)==0 {print}')

    if [ -z "$bad" ]; then
        return 0
    fi

    echo -e "${YELLOW}⚠️ 发现非目标地址 Cloudflare 隧道进程，尝试清理...${NC}"
    echo "$bad"
    echo "$bad" | awk '{print $1}' | xargs -r kill -9 2>/dev/null || true
    sleep 1

    local bad_after
    bad_after=$(ps -eo pid=,comm=,args= | awk -v target="http://localhost:${TUNNEL_PORT}" '$2 == "cloudflared" && /tunnel --url/ && index($0, target)==0 {print}')
    if [ -n "$bad_after" ]; then
        echo -e "${YELLOW}⚠️ 存在非目标 Cloudflare 隧道进程:${NC}"
        echo "$bad_after"
        if [ "$STRICT_TUNNEL_SANITY" = "1" ]; then
            echo -e "${RED}❌ STRICT_TUNNEL_SANITY=1，按严格模式中止${NC}"
            return 1
        fi
        echo -e "${YELLOW}⚠️ 兼容模式继续执行（可导出 STRICT_TUNNEL_SANITY=1 开启严格模式）${NC}"
        return 0
    fi

    return 0
}

warn_if_system_cf_tunnel_running() {
    if command -v systemctl >/dev/null 2>&1; then
        if systemctl is-active --quiet cf-tunnel.service 2>/dev/null; then
            echo -e "${YELLOW}⚠️ 检测到系统服务 cf-tunnel.service 正在运行（通常绑定 80 端口隧道）${NC}"
            echo -e "${YELLOW}   当前脚本会启用独立 8080 隧道并优先使用其域名。${NC}"
        fi
    fi
}

start_tunnel_and_get_url() {
    local attempt=0
    NEW_URL=""
    TUNNEL_PID=""

    while [ "$attempt" -lt "$TUNNEL_MAX_RETRIES" ]; do
        attempt=$((attempt + 1))

        if [ "$attempt" -eq 1 ]; then
            echo -e "${GREEN}🌐 正在启动 Cloudflare Quick Tunnel...${NC}"
        else
            local backoff=$((attempt * 5))
            echo -e "${YELLOW}⚠️ 第 ${attempt}/${TUNNEL_MAX_RETRIES} 次尝试获取隧道域名（等待 ${backoff} 秒）...${NC}"
            sleep "$backoff"
        fi

        # 每次重试前先彻底清理旧 tunnel 进程
        pkill -9 cloudflared 2>/dev/null || true
        sleep 1

        ensure_no_stale_cloudflared_80 || return 1

        if [ -f "$LOG_FILE" ]; then
            cp "$LOG_FILE" "${LOG_FILE}.bak" 2>/dev/null || true
        fi
        : > "$LOG_FILE"

        nohup setsid cloudflared tunnel --no-autoupdate --url "http://localhost:${TUNNEL_PORT}" > "$LOG_FILE" 2>&1 &
        TUNNEL_PID=$!

        if [ -z "$TUNNEL_PID" ]; then
            echo -e "${YELLOW}⚠️ Cloudflare 进程启动失败，继续重试...${NC}"
            continue
        fi

        local wait_secs=$((TUNNEL_WAIT_BASE + attempt * 4))
        if [ "$wait_secs" -gt "$TUNNEL_WAIT_MAX" ]; then
            wait_secs=$TUNNEL_WAIT_MAX
        fi

        local waited=0
        while [ "$waited" -lt "$wait_secs" ]; do
            sleep 2
            waited=$((waited + 2))

            NEW_URL="$(extract_tunnel_url "$LOG_FILE")"
            if [ -n "$NEW_URL" ]; then
                break
            fi

            if grep -qi "429\|Too Many Requests\|failed to unmarshal\|error code: 11\|connection refused\|quota exceeded" "$LOG_FILE" 2>/dev/null; then
                echo -e "${YELLOW}⚠️ 隧道日志出现异常，准备重试...${NC}"
                break
            fi

            if [ $((waited % 10)) -eq 0 ]; then
                echo -e "${YELLOW}   等待中... (${waited}/${wait_secs}秒)${NC}"
            fi
        done

        # 最后再抓一次，满足“有可用域名就抓取使用”
        if [ -z "$NEW_URL" ]; then
            NEW_URL="$(extract_tunnel_url "$LOG_FILE")"
        fi

        if [ -n "$NEW_URL" ]; then
            ensure_only_expected_cloudflared_target || return 1
            echo -e "${GREEN}✅ 隧道已就绪 (第${attempt}次尝试): ${NEW_URL}${NC}"
            return 0
        fi

        kill "$TUNNEL_PID" 2>/dev/null || true
        TUNNEL_PID=""
    done

    return 1
}

show_backend_log_tail() {
    local lines="${1:-80}"

    if [ -f "$BACKEND_LOG" ] && [ -s "$BACKEND_LOG" ]; then
        echo -e "${YELLOW}📄 后端日志最后 ${lines} 行:${NC}"
        tail -n "$lines" "$BACKEND_LOG" 2>/dev/null || true
    elif command -v journalctl >/dev/null 2>&1; then
        echo -e "${YELLOW}📄 systemd 服务日志最后 ${lines} 行:${NC}"
        journalctl -u tgbot.service --no-pager -n "$lines" 2>/dev/null || true
    else
        echo -e "${YELLOW}⚠️ 后端日志文件不存在: ${BACKEND_LOG}${NC}"
    fi
}

is_pid_alive() {
    local pid="$1"
    [ -n "$pid" ] && kill -0 "$pid" 2>/dev/null
}

extract_backend_start_error() {
    local error_lines=""

    # 检查 nohup 后端日志
    if [ -f "$BACKEND_LOG" ]; then
        error_lines="$(grep -E "Traceback|ImportError|ModuleNotFoundError|SyntaxError|NameError|AttributeError|RuntimeError|ValueError" "$BACKEND_LOG" 2>/dev/null | tail -n 5)"
    fi

    # 检查 systemd 服务日志（journalctl）
    if [ -z "$error_lines" ] && command -v journalctl >/dev/null 2>&1; then
        error_lines="$(journalctl -u tgbot.service --since "1 min ago" --no-pager 2>/dev/null | grep -E "Traceback|ImportError|ModuleNotFoundError|SyntaxError|NameError|AttributeError|RuntimeError|ValueError" | tail -n 5)"
    fi

    echo "$error_lines"
}

start_backend_with_env_validation() {
    local expected_url="$1"
    local attempt=0
    local saw_env_mismatch=0

    while [ "$attempt" -le "$BACKEND_ENV_RETRY_MAX" ]; do
        attempt=$((attempt + 1))

        start_python_backend_instance
        if [ -z "$PYTHON_PID" ]; then
            echo -e "${RED}❌ 启动 Python 服务失败！${NC}"
            return 1
        fi

        echo -e "${GREEN}✅ Python 后端已启动 (PID: ${PYTHON_PID})${NC}"

        if ! wait_backend_ready 25; then
            if ! is_pid_alive "$PYTHON_PID"; then
                echo -e "${RED}❌ 后端进程在端口就绪前已退出，启动失败${NC}"
                local startup_error
                startup_error="$(extract_backend_start_error)"
                if [ -n "$startup_error" ]; then
                    echo -e "${RED}❌ 检测到后端启动异常:${NC}"
                    echo "$startup_error"
                fi
                show_backend_log_tail 80
                return 1
            fi

            echo -e "${YELLOW}⚠️ 后端端口未在预期时间内就绪，准备重试...${NC}"
            if command -v systemctl >/dev/null 2>&1 && systemctl is-active --quiet tgbot.service 2>/dev/null; then
                sudo systemctl restart tgbot.service 2>/dev/null || systemctl restart tgbot.service 2>/dev/null || true
            else
                kill "$PYTHON_PID" 2>/dev/null || true
            fi
            sleep 2
            continue
        fi

        if validate_backend_env_url "$PYTHON_PID" "$expected_url"; then
            local running_pids
            running_pids="$(pgrep -f "$PYTHON_CMD" 2>/dev/null || true)"
            local running_count=0
            if [ -n "$running_pids" ]; then
                running_count=$(echo "$running_pids" | awk 'NF' | wc -l)
            fi
            if [ "$running_count" -gt 1 ]; then
                echo -e "${RED}❌ 检测到多个后端实例同时运行: ${running_pids}${NC}"
                return 1
            fi
            echo -e "${GREEN}✅ 后端进程已加载最新域名环境变量${NC}"
            return 0
        fi

        saw_env_mismatch=1
        if [ "$attempt" -le "$BACKEND_ENV_RETRY_MAX" ]; then
            echo -e "${YELLOW}🔄 尝试重启后端以同步域名（第 ${attempt}/${BACKEND_ENV_RETRY_MAX} 次）...${NC}"
            if command -v systemctl >/dev/null 2>&1 && systemctl is-active --quiet tgbot.service 2>/dev/null; then
                sudo systemctl restart tgbot.service 2>/dev/null || systemctl restart tgbot.service 2>/dev/null || true
            else
                kill "$PYTHON_PID" 2>/dev/null || true
            fi
            sleep 2
        fi
    done

    if [ "$saw_env_mismatch" -eq 1 ]; then
        echo -e "${RED}❌ 后端进程端口已就绪，但环境变量始终未同步到最新域名，启动中止${NC}"
    else
        echo -e "${RED}❌ 后端未能在预期时间内启动，请检查后端日志${NC}"
        show_backend_log_tail 80
    fi
    return 1
}

# ==========================================
# 主流程
# ==========================================

echo -e "${YELLOW}开始执行自动化隧道启动流程...${NC}"

cd "$PROJECT_DIR" || exit 1
mkdir -p "$LOG_APP_DIR" "$LOG_BACKEND_DIR" "$LOG_TUNNEL_DIR" "$LOG_CLOUDFLARED_DIR" "$LOG_RUNTIME_DIR" "$LOG_AI_DIR"
warn_if_system_cf_tunnel_running

if [ ! -f "$ENV_FILE" ]; then
    echo -e "${RED}❌ 未找到 .env 文件: ${PROJECT_DIR}/${ENV_FILE}${NC}"
    exit 1
fi

# 1. 清理旧进程
echo -e "${YELLOW}🧹 清理旧的 Cloudflare、Python 和 LiteLLM 进程...${NC}"
pkill -9 cloudflared 2>/dev/null || true
if ! kill_backend_processes; then
    echo -e "${RED}❌ 无法确认旧后端已停止，重启中止${NC}"
    exit 1
fi
docker rm -f "${LITELLM_CONTAINER}" > /dev/null 2>&1 || true
sleep 2

if pgrep cloudflared > /dev/null 2>&1; then
    echo -e "${YELLOW}  发现残留 cloudflared 进程，继续强制清理...${NC}"
    pkill -9 -f cloudflared || true
    sleep 2
fi

ensure_no_stale_cloudflared_80 || exit 1

# 2. 构建前端
echo -e "${YELLOW}🔨 构建前端...${NC}"
cd "${PROJECT_DIR}/frontend" || exit 1

# 检查node_modules是否存在
if [ ! -d "node_modules" ]; then
    echo -e "${YELLOW}📦 安装前端依赖...${NC}"
    npm install || {
        echo -e "${RED}❌ 前端依赖安装失败${NC}"
        exit 1
    }
fi

# 构建前端
echo -e "${YELLOW}🏗️ 执行前端构建...${NC}"
npm run build || {
    echo -e "${RED}❌ 前端构建失败${NC}"
    exit 1
}

# 检查构建产物（Vite配置输出到../static/dist/）
if [ ! -f "${PROJECT_DIR}/static/dist/index.html" ] || [ ! -d "${PROJECT_DIR}/static/dist/assets" ]; then
    echo -e "${RED}❌ 前端构建产物不完整${NC}"
    echo -e "${YELLOW}   期望路径: ${PROJECT_DIR}/static/dist/${NC}"
    exit 1
fi

echo -e "${GREEN}✅ 前端构建完成${NC}"
cd "$PROJECT_DIR" || exit 1

sync_merchants_to_supabase_if_enabled || exit 1

# 2. 启动 LiteLLM 服务
echo -e "${GREEN}🤖 正在启动 LiteLLM AI 服务...${NC}"
if ! command -v docker &>/dev/null; then
    echo -e "${RED}❌ Docker 未安装或不在 PATH 中！${NC}"
    exit 1
fi

if [ ! -f "${PROJECT_DIR}/litellm_config.yaml" ]; then
    echo -e "${RED}❌ litellm_config.yaml 配置文件不存在！${NC}"
    exit 1
fi

GEMINI_API_KEY=$(grep GEMINI_API_KEY .env 2>/dev/null | cut -d= -f2)

docker run -d \
    --name "${LITELLM_CONTAINER}" \
    --restart unless-stopped \
    -p "${LITELLM_PORT}:4000" \
    -v "${PROJECT_DIR}/litellm_config.yaml:/app/config.yaml:ro" \
    -v "${PROJECT_DIR}/Google api/tgbot-490920-05d1ad1b3310.json:/app/Google_api/tgbot-490920-05d1ad1b3310.json:ro" \
    -e "GEMINI_API_KEY=${GEMINI_API_KEY}" \
    -e "GOOGLE_APPLICATION_CREDENTIALS=/app/Google_api/tgbot-490920-05d1ad1b3310.json" \
    ghcr.io/berriai/litellm:main-latest \
    --config /app/config.yaml \
    --host 0.0.0.0 > /dev/null 2>&1

if [ $? -ne 0 ]; then
    echo -e "${RED}❌ LiteLLM Docker 容器创建失败！${NC}"
    exit 1
fi

sleep 3
if docker ps -q --filter "name=${LITELLM_CONTAINER}" | grep -q . 2>/dev/null; then
    echo -e "${GREEN}✅ LiteLLM 容器已启动 (端口: ${LITELLM_PORT})${NC}"
else
    echo -e "${RED}❌ LiteLLM 容器启动失败！${NC}"
    echo -e "${YELLOW}查看日志: docker logs ${LITELLM_CONTAINER}${NC}"
    exit 1
fi

# 3. 启动隧道并获取域名（可用即用，不可用重试）
if ! start_tunnel_and_get_url; then
    echo -e "${RED}❌ 未能获取 Cloudflare 可用域名（重试 ${TUNNEL_MAX_RETRIES} 次后仍失败）${NC}"
    echo -e "${YELLOW}查看日志: tail -80 ${LOG_FILE}${NC}"
    tail -80 "$LOG_FILE" 2>/dev/null || true
    exit 1
fi

echo -e "${GREEN}✨ 获取到新域名: ${NEW_URL}${NC}"

# 4. 写入 .env + 导出当前环境
echo -e "${YELLOW}📝 正在写入 .env 中的 WEB_BASE_URL / FRONTEND_URL...${NC}"
write_env_url "$NEW_URL"
echo -e "${GREEN}✅ .env 与当前 shell 环境已同步新域名${NC}"

# 5. 启动后端并校验生效
kill_backend_processes
if ! start_backend_with_env_validation "$NEW_URL"; then
    exit 1
fi

# 6. 再次确认隧道目标进程没有偏移到 80 端口
ensure_no_stale_cloudflared_80 || exit 1
ensure_only_expected_cloudflared_target || exit 1

echo -e "${GREEN}==========================================${NC}"
echo -e "${GREEN}🎉 全部启动成功！${NC}"
echo -e "${GREEN}🌐 访问地址: ${NEW_URL}${NC}"
echo -e "${GREEN}📱 Mini App: Telegram 聊天底部菜单栏 → 🏪 控制台${NC}"
echo -e "${GREEN}🤖 LiteLLM: http://localhost:${LITELLM_PORT}${NC}"
echo -e "${GREEN}📄 后端日志: ${BACKEND_LOG}${NC}"
echo -e "${GREEN}📄 隧道日志: ${LOG_FILE}${NC}"
echo -e "${GREEN}📄 AI日志: docker logs ${LITELLM_CONTAINER}${NC}"
echo -e "${GREEN}==========================================${NC}"
