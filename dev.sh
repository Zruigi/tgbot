#!/bin/bash
# ============================================================
# TGBot 一键启动/停止/状态 脚本 2.1
# 功能: 前端 + 后端 + LiteLLM容器 + Cloudflare隧道 一键管理
# 用法: ./dev.sh [start|stop|restart|status|tunnel|logs]
# ============================================================

set -e

# ==========================================
# 配置区域（按需修改）
# ==========================================
PROJECT_DIR="/home/ubuntu/tgbot"
LOG_DIR="/home/ubuntu/tgbot/logs"
APP_LOG_DIR="${LOG_DIR}/app"
BACKEND_LOG_DIR="${APP_LOG_DIR}/backend"
TUNNEL_LOG_DIR="${LOG_DIR}/tunnel"
CLOUDFLARED_LOG_DIR="${TUNNEL_LOG_DIR}/cloudflared"
FRONTEND_LOG_DIR="${LOG_DIR}/frontend"
VITE_LOG_DIR="${FRONTEND_LOG_DIR}/vite"
AI_LOG_DIR="${LOG_DIR}/ai"
LITELLM_LOG_DIR="${AI_LOG_DIR}/litellm"
RUNTIME_LOG_DIR="${LOG_DIR}/runtime"
PROCESS_LOG_DIR="${RUNTIME_LOG_DIR}/process"
TUNNEL_LOG="${CLOUDFLARED_LOG_DIR}/cloudflared.log"
BACKEND_LOG="${BACKEND_LOG_DIR}/backend.log"
FRONTEND_LOG="${VITE_LOG_DIR}/frontend.stdout"
LITELLM_LOG="${LITELLM_LOG_DIR}/litellm.log"
PID_FILE="${PROCESS_LOG_DIR}/dev.pids"
ENV_FILE="${PROJECT_DIR}/.env"
ENV_BACKUP_KEEP=5            # 限制 .env 备份数量，默认保留最近 N 份

# 端口配置
TUNNEL_PORT=8080              # Web Admin 后端端口
VITE_PORT=5173               # Vite 前端开发服务器端口
LITELLM_PORT=4000             # LiteLLM 代理端口 (默认)
BACKEND_ENV_RETRY_MAX=2       # 后端环境变量生效校验重试次数
# 兼容模式：默认不因系统级 80 端口隧道而硬失败；设为 1 可恢复严格失败
STRICT_TUNNEL_SANITY="${STRICT_TUNNEL_SANITY:-0}"

PYTHON_BIN="${PROJECT_DIR}/venv/bin/python3"
if [ ! -x "$PYTHON_BIN" ]; then
    PYTHON_BIN="$(command -v python3)"
fi
PYTHON_CMD="$PYTHON_BIN main.py"

RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
CYAN='\033[0;36m'
BOLD='\033[1m'
NC='\033[0m'

VITE_CMD="vite"
CLOUDFLARED_CMD="cloudflared tunnel"
LITELLM_CONTAINER="litellm"   # Docker 容器名
# ==========================================
# 工具函数
# ==========================================

banner() {
    echo ""
    echo -e "${CYAN}${BOLD}╔══════════════════════════════════════════╗${NC}"
    echo -e "${CYAN}${BOLD}║       🤖  TGBot DevOps 控制台 2.1       ║${NC}"
    echo -e "${CYAN}${BOLD}╚══════════════════════════════════════════╝${NC}"
    echo -e " ${BOLD}项目目录:${NC} ${PROJECT_DIR}"
    echo -e " ${BOLD}时间:${NC}      $(date '+%Y-%m-%d %H:%M:%S')"
    echo ""
}

log() {
    local level=$1; shift
    case $level in
        INFO)  echo -e "${GREEN}[INFO]${NC}  $*" ;;
        WARN)  echo -e "${YELLOW}[WARN]${NC}  $*" ;;
        ERROR) echo -e "${RED}[ERROR]${NC} $*" ;;
        STEP)  echo -e "\n${BLUE}[STEP]${NC}  ${BOLD}$*${NC}" ;;
        OK)    echo -e "${GREEN}✅ $*${NC}" ;;
        FAIL)  echo -e "${RED}❌ $*${NC}" ;;
        *)     echo "$*" ;;
    esac
}

ensure_dirs() {
    mkdir -p "$APP_LOG_DIR" "$BACKEND_LOG_DIR" "$TUNNEL_LOG_DIR" "$CLOUDFLARED_LOG_DIR" \
        "$FRONTEND_LOG_DIR" "$VITE_LOG_DIR" \
        "$AI_LOG_DIR" "$LITELLM_LOG_DIR" \
        "$RUNTIME_LOG_DIR" "$PROCESS_LOG_DIR" 2>/dev/null || true
    touch "$PID_FILE" 2>/dev/null || true
    # 清空 PID 文件
    > "$PID_FILE"
}

save_pid() {
    local name=$1 pid=$2
    echo "${name}:${pid}" >> "$PID_FILE"
}

get_pid() {
    local name=$1
    grep "^${name}:" "$PID_FILE" 2>/dev/null | cut -d: -f2 | tail -1
}

wait_for_port() {
    local port=$1 name=$2 timeout=${3:-15}
    local elapsed=0
    while [ $elapsed -lt $timeout ]; do
        if ss -tlnp 2>/dev/null | grep -q ":${port} " || nc -z localhost "$port" 2>/dev/null; then
            return 0
        fi
        sleep 1
        elapsed=$((elapsed + 1))
    done
    log FAIL "${name} 端口 ${port} 在 ${timeout}s 内未响应"
    return 1
}

get_env_var_from_pid() {
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
    pid_web_base="$(get_env_var_from_pid "$pid" "WEB_BASE_URL" 2>/dev/null || true)"
    pid_frontend="$(get_env_var_from_pid "$pid" "FRONTEND_URL" 2>/dev/null || true)"

    # 兼容判定：
    # 1) FRONTEND_URL 命中即可视为生效（按钮直接使用）
    # 2) FRONTEND_URL 为空时，允许 WEB_BASE_URL 命中
    if [ "$pid_frontend" = "$expected_url" ] || { [ -z "$pid_frontend" ] && [ "$pid_web_base" = "$expected_url" ]; }; then
        return 0
    fi

    log WARN "后端进程环境未匹配新域名: expected=${expected_url}"
    log WARN "WEB_BASE_URL=${pid_web_base:-<empty>} FRONTEND_URL=${pid_frontend:-<empty>}"
    return 1
}

kill_stale_cloudflared_80() {
    local stale
    stale=$(ps -eo pid=,comm=,args= | awk '$2 == "cloudflared" && /tunnel --url/ && ($0 ~ /127\\.0\\.0\\.1:80([^0-9]|$)/ || $0 ~ /localhost:80([^0-9]|$)/) {print $1}')
    if [ -z "$stale" ]; then
        return 0
    fi

    log WARN "检测到残留 80 端口 Cloudflare 隧道进程，开始清理..."
    echo "$stale" | xargs -r kill -9 2>/dev/null || true
    sleep 1

    local stale_after
    stale_after=$(ps -eo pid=,comm=,args= | awk '$2 == "cloudflared" && /tunnel --url/ && ($0 ~ /127\\.0\\.0\\.1:80([^0-9]|$)/ || $0 ~ /localhost:80([^0-9]|$)/) {print $1}')
    if [ -n "$stale_after" ]; then
        log WARN "仍存在 80 端口 Cloudflare 隧道进程: ${stale_after}"
        if [ "$STRICT_TUNNEL_SANITY" = "1" ]; then
            log FAIL "STRICT_TUNNEL_SANITY=1，按严格模式中止"
            return 1
        fi
        log WARN "兼容模式继续执行（可导出 STRICT_TUNNEL_SANITY=1 开启严格模式）"
        return 0
    fi

    log OK "残留 80 端口 Cloudflare 隧道进程已清理"
    return 0
}

ensure_only_expected_cloudflared_target() {
    local bad
    bad=$(ps -eo pid=,comm=,args= | awk -v target="http://localhost:${TUNNEL_PORT}" '$2 == "cloudflared" && /tunnel --url/ && index($0, target)==0 {print}')
    if [ -z "$bad" ]; then
        return 0
    fi

    log WARN "发现非目标隧道进程，尝试清理..."
    echo "$bad"
    echo "$bad" | awk '{print $1}' | xargs -r kill -9 2>/dev/null || true
    sleep 1

    local bad_after
    bad_after=$(ps -eo pid=,comm=,args= | awk -v target="http://localhost:${TUNNEL_PORT}" '$2 == "cloudflared" && /tunnel --url/ && index($0, target)==0 {print}')
    if [ -n "$bad_after" ]; then
        log WARN "存在非目标 Cloudflare 隧道进程"
        echo "$bad_after"
        if [ "$STRICT_TUNNEL_SANITY" = "1" ]; then
            log FAIL "STRICT_TUNNEL_SANITY=1，按严格模式中止"
            return 1
        fi
        log WARN "兼容模式继续执行（可导出 STRICT_TUNNEL_SANITY=1 开启严格模式）"
        return 0
    fi

    return 0
}

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

warn_if_system_cf_tunnel_running() {
    if command -v systemctl >/dev/null 2>&1; then
        if systemctl is-active --quiet cf-tunnel.service 2>/dev/null; then
            log WARN "检测到系统服务 cf-tunnel.service 正在运行（通常绑定 80 端口隧道）"
            log WARN "当前脚本会使用独立 8080 隧道并优先写入新域名"
        fi
    fi
}

# ==========================================
# 停止所有服务
# ==========================================

stop_all() {
    banner
    log STEP "正在停止所有服务..."

    local stopped=0

    # 1. 停止 Python 后端
    if pkill -f "$PYTHON_CMD" 2>/dev/null; then
        log OK "Python 后端已停止"
        stopped=$((stopped + 1))
    else
        log INFO "Python 后端未运行"
    fi

    # 2. 停止 Vite 前端
    if pkill -f "$VITE_CMD" 2>/dev/null || pkill -f "node.*vite" 2>/dev/null; then
        log OK "Vite 前端已停止"
        stopped=$((stopped + 1))
    else
        log INFO "Vite 前端未运行"
    fi

    # 3. 停止 Cloudflare Tunnel
    if pkill -f "$CLOUDFLARED_CMD" 2>/dev/null; then
        log OK "Cloudflare Tunnel 已停止"
        stopped=$((stopped + 1))
    else
        log INFO "Cloudflare Tunnel 未运行"
    fi

    # 4. 停止 LiteLLM 容器
    if docker ps -q --filter "name=${LITELLM_CONTAINER}" | grep -q . 2>/dev/null; then
        docker stop "${LITELLM_CONTAINER}" >/dev/null 2>&1 && \
            docker rm "${LITELLM_CONTAINER}" >/dev/null 2>&1
        log OK "LiteLLM 容器已停止并移除"
        stopped=$((stopped + 1))
    else
        log INFO "LiteLLM 容器未运行"
    fi

    sleep 1
    ensure_dirs
    
    echo ""
    if [ $stopped -gt 0 ]; then
        log OK "已停止 ${stopped} 个服务"
    else
        log INFO "没有运行中的服务"
    fi
}

# ==========================================
# 启动 LiteLLM 容器
# ==========================================

start_litellm() {
    log STEP "启动 LiteLLM 容器..."
    
    # 检查是否已有运行中
    if docker ps -q --filter "name=${LITELLM_CONTAINER}" | grep -q . 2>/dev/null; then
        log WARN "LiteLLM 已在运行中，跳过启动"
        local port=$(docker port ${LITELLM_CONTAINER} 2>/dev/null | head -1 | grep -oP '\d+$' || echo ${LITELLM_PORT})
        save_pid "litellm" "$(docker inspect -f '{{.State.Pid}}' ${LITELLM_CONTAINER})"
        log OK "LiteLLM 运行中 → http://localhost:${port}"
        return 0
    fi

    # 检查 Docker 是否可用
    if ! command -v docker &>/dev/null; then
        log FAIL "Docker 未安装或不在 PATH 中"
        return 1
    fi

    # 检查 litellm_config.yaml 是否存在
    if [ ! -f "${PROJECT_DIR}/litellm_config.yaml" ]; then
        log FAIL "litellm_config.yaml 不存在"
        return 1
    fi

    # 启动容器
    docker run -d \
        --name "${LITELLM_CONTAINER}" \
        --restart unless-stopped \
        -p "${LITELLM_PORT}:4000" \
        -v "${PROJECT_DIR}/litellm_config.yaml:/app/config.yaml:ro" \
        -e "GEMINI_API_KEY=${GEMINI_API_KEY:-$(grep GEMINI_API_KEY .env 2>/dev/null | cut -d= -f2)}" \
        ghcr.io/berriai/litellm:main-latest \
        --config /app/config.yaml \
        --host 0.0.0.0 \
        > /dev/null 2>&1

    if [ $? -eq 0 ]; then
        # 等待容器就绪
        sleep 2
        
        if wait_for_port $LITELLM_PORT "LiteLLM" 20; then
            local llm_pid=$(docker inspect -f '{{.State.Pid}}' ${LITELLM_CONTAINER})
            save_pid "litellm" "$llm_pid"
            log OK "LiteLLM 容器已启动 → http://localhost:${LITELLM_PORT}"
            
            # 收集日志
            docker logs ${LITELLM_CONTAINER} > "$LITELLM_LOG" 2>&1 &
            return 0
        else
            log FAIL "LiteLLM 端口 ${LITELLM_PORT} 未就绪"
            docker logs ${LITELLM_CONTAINER} 2>&1 | tail -10
            return 1
        fi
    else
        log FAIL "Docker 启动失败"
        return 1
    fi
}

# ==========================================
# 启动前端开发服务器
# ==========================================

start_frontend() {
    log STEP "启动 Vite 前端开发服务器..."

    cd "${PROJECT_DIR}/frontend"

    # 检查 node_modules
    if [ ! -d "node_modules" ]; then
        log INFO "检测到缺少依赖，正在执行 npm install..."
        npm install --prefer-offline 2>/dev/null
    fi

    # 后台启动 Vite，使用独立会话避免脚本退出时带走前端开发进程。
    setsid sh -c "cd '${PROJECT_DIR}/frontend' && exec npm run dev > '${FRONTEND_LOG}' 2>&1 < /dev/null" &
    local vite_pid=$!
    
    if [ -n "$vite_pid" ]; then
        if wait_for_port $VITE_PORT "Vite 前端" 15; then
            save_pid "frontend" "$vite_pid"
            log OK "Vite 前端已启动 → http://localhost:${VITE_PORT}"
            cd "$PROJECT_DIR"
            return 0
        else
            log FAIL "Vite 前端启动失败，查看日志: $FRONTEND_LOG"
            tail -15 "$FRONTEND_LOG" 2>/dev/null
            kill "$vite_pid" 2>/dev/null || true
            cd "$PROJECT_DIR"
            return 1
        fi
    else
        log FAIL "无法获取 Vite 进程 ID"
        cd "$PROJECT_DIR"
        return 1
    fi
}

# ==========================================
# 加载 .env 到环境变量（供子进程继承）
# ==========================================

load_env_to_process() {
    # 将 .env 文件中的 KEY=VALUE 导出到当前 shell 环境变量
    local env_file="${1:-$ENV_FILE}"
    
    if [ ! -f "$env_file" ]; then
        log WARN ".env 文件不存在: ${env_file}"
        return 1
    fi
    
    # 逐行读取 .env，跳过注释和空行，export 每个变量
    while IFS='=' read -r key value; do
        # 跳过空行和注释（支持 # 开头和行尾 # 注释）
        [[ -z "$key" || "$key" =~ ^[[:space:]]*# ]] && continue
        
        # 去掉 key 首尾空白
        key=$(echo "$key" | xargs)
        
        # 去掉 value 首尾空白和可能的引号/换行
        value=$(echo "$value" | xargs | sed "s/^['\"]//;s/['\"]$//")
        
        # 导出到当前环境（子进程会自动继承）
        export "${key}=${value}"
    done < "$env_file"
    
    # 关键验证：确认 WEB_BASE_URL 和 FRONTEND_URL 已导出
    if [ -n "$WEB_BASE_URL" ]; then
        log INFO "已加载域名: ${WEB_BASE_URL}"
    else
        log WARN ".env 中未找到 WEB_BASE_URL (文件: ${env_file})"
    fi
}

# ==========================================
# 启动 Python 后端（带 .env 环境变量注入）
# ==========================================

start_backend() {
    local expected_url="${1:-}"
    local attempt=0

    while [ "$attempt" -le "$BACKEND_ENV_RETRY_MAX" ]; do
        attempt=$((attempt + 1))
        log STEP "启动 Python 后端服务... (第 ${attempt}/$((BACKEND_ENV_RETRY_MAX + 1)) 次)"
        cd "$PROJECT_DIR"

        # ★ 关键：加载 .env 到环境变量，确保 config.py 的 os.getenv() 能读到最新值
        load_env_to_process

        # 使用当前 shell 已导出的环境变量启动，避免 env -i + xargs 丢变量
        nohup $PYTHON_CMD > "$BACKEND_LOG" 2>&1 &
        local py_pid=$!

        if [ -z "$py_pid" ]; then
            log FAIL "Python 后端启动失败（无法获取 PID）"
            return 1
        fi

        if ! wait_for_port $TUNNEL_PORT "Python 后端" 20; then
            log FAIL "Python 后端端口未就绪，查看日志: $BACKEND_LOG"
            tail -20 "$BACKEND_LOG" 2>/dev/null
            kill "$py_pid" 2>/dev/null || true
            sleep 2
            continue
        fi

        # 触发一次本地请求，帮助尽快暴露启动异常（不关心响应内容）
        if command -v curl &>/dev/null; then
            curl -s "http://localhost:${TUNNEL_PORT}/api/search?q=test" >/dev/null 2>&1 || true
        fi

        if [ -n "$expected_url" ] && ! validate_backend_env_url "$py_pid" "$expected_url"; then
            if [ "$attempt" -le "$BACKEND_ENV_RETRY_MAX" ]; then
                log WARN "后端环境变量未加载到新域名，准备重启重试..."
                kill "$py_pid" 2>/dev/null || true
                sleep 2
                continue
            fi
            log FAIL "后端环境变量最终仍未生效，启动失败"
            kill "$py_pid" 2>/dev/null || true
            return 1
        fi

        save_pid "backend" "$py_pid"
        log OK "Python 后端已启动 (PID: ${py_pid}) → http://localhost:${TUNNEL_PORT}"
        log INFO "WEB_BASE_URL=${WEB_BASE_URL:-未设置}"
        log INFO "FRONTEND_URL=${FRONTEND_URL:-未设置}"
        return 0
    done

    log FAIL "Python 后端启动失败（超过重试上限）"
    return 1
}

# ==========================================
# 启动 Cloudflare 隧道 + 提取域名（含重试机制）
# ==========================================

# 隧道重试配置
TUNNEL_MAX_RETRIES=3          # 最大重试次数
TUNNEL_WAIT_BASE=8            # 基础等待秒数（每次递增）
TUNNEL_WAIT_MAX=20            # 单次最大等待秒数

start_tunnel() {
    local attempt=0
    
    while [ $attempt -lt $TUNNEL_MAX_RETRIES ]; do
        attempt=$((attempt + 1))
        
        if [ $attempt -eq 1 ]; then
            log STEP "启动 Cloudflare Quick Tunnel..."
        else
            local wait_time=$(( TUNNEL_WAIT_BASE * attempt ))
            [ $wait_time -gt $TUNNEL_WAIT_MAX ] && wait_time=$TUNNEL_WAIT_MAX
            log WARN "第 ${attempt}/${TUNNEL_MAX_RETRIES} 次重试 (等待 ${wait_time}s)..."
            sleep $wait_time
        fi

        # 清理旧隧道进程（仅首次或上次失败后）
        pkill -f "$CLOUDFLARED_CMD" 2>/dev/null || true
        # 确保端口释放
        sleep 2
        kill_stale_cloudflared_80 || return 1

        # 清空日志，开始新记录
        > "$TUNNEL_LOG"

        # 启动新隧道（使用 --no-autoupdate 避免 update 检查干扰）
        nohup setsid cloudflared tunnel --no-autoupdate --url "http://localhost:${TUNNEL_PORT}" > "$TUNNEL_LOG" 2>&1 &
        local tunnel_pid=$!
        
        if [ -z "$tunnel_pid" ]; then
            log FAIL "Cloudflare 进程启动失败 (尝试 ${attempt})"
            continue
        fi

        # 等待域名分配（渐进式等待）
        local wait_secs=$(( TUNNEL_WAIT_BASE + (attempt * 2) ))
        [ $wait_secs -gt $TUNNEL_WAIT_MAX ] && wait_secs=$TUNNEL_WAIT_MAX
        log INFO "等待隧道分配域名 (约 ${wait_secs} 秒, 第${attempt}次)..."
        
        # 分段等待，每 2 秒检查一次是否已拿到域名
        local waited=0
        local got_url=false
        while [ $waited -lt $wait_secs ]; do
            sleep 2
            waited=$((waited + 2))
            
            # 尝试提前提取
            local early_url=$(grep -oP "https://[a-zA-Z0-9\-\.]+\.trycloudflare\.com" "$TUNNEL_LOG" 2>/dev/null | tail -n 1)
            if [ -n "$early_url" ]; then
                new_url="$early_url"
                got_url=true
                break
            fi
            
            # 检查是否有致命错误
            if grep -qi "error code: 1101\|error code: 1000\|failed to unmarshal\|connection refused\|quota exceeded\|too many requests" "$TUNNEL_LOG" 2>/dev/null; then
                log WARN "检测到隧道错误 (尝试 ${attempt}), 将重试..."
                kill "$tunnel_pid" 2>/dev/null || true
                break
            fi
            
            echo -ne "\r       已等待 ${waited}/${wait_secs} 秒... "
        done

        if $got_url; then
            # 成功获取到 URL
            ensure_only_expected_cloudflared_target || return 1
            save_pid "cloudflared" "$tunnel_pid"
            _update_env_url "$new_url"
            log OK "Cloudflare Tunnel 已启动 (第${attempt}次尝试成功)"
            echo -e "       ${BOLD}🌐 公网地址: ${CYAN}${new_url}${NC}"
            return 0
        fi
        
        # 未在等待时间内拿到域名，再最后检查一次
        new_url=$(grep -oP "https://[a-zA-Z0-9\-\.]+\.trycloudflare\.com" "$TUNNEL_LOG" 2>/dev/null | tail -n 1)
        if [ -n "$new_url" ]; then
            ensure_only_expected_cloudflared_target || return 1
            save_pid "cloudflared" "$tunnel_pid"
            _update_env_url "$new_url"
            log OK "Cloudflare Tunnel 已启动 (延迟获取)"
            echo -e "       ${BOLD}🌐 公网地址: ${CYAN}${new_url}${NC}"
            return 0
        fi

        # 本次失败，清理并重试
        kill "$tunnel_pid" 2>/dev/null || true
    done

    # 所有重试均失败 — 进入降级模式
    _handle_tunnel_failure
    return 1
}

# 更新 .env 中的 URL
_update_env_url() {
    local url="$1"
    
    if [ -f "$ENV_FILE" ]; then
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

        # ★ 关键：同步刷新当前 shell 环境变量（供后续子进程使用）
        export WEB_BASE_URL="${url}"
        export FRONTEND_URL="${url}"
    fi

    # ★ 隧道域名变化后，提示需要重启后端以生效
    log INFO "域名已写入 .env 和当前环境变量，后端新 OTP 链接将使用新域名"
}

# ==========================================
# 仅重新抓取隧道域名（不重启服务）— 增强版：自动重启后端刷新域名
# ==========================================

refresh_tunnel_only() {
    banner
    log STEP "仅刷新 Cloudflare Tunnel 域名（含后端域名热刷新）..."
    ensure_dirs
    warn_if_system_cf_tunnel_running
    
    local old_url=$(grep WEB_BASE_URL= "$ENV_FILE" 2>/dev/null | cut -d= -f2)
    
    # 只重启隧道
    if start_tunnel; then
        local new_url=$(grep WEB_BASE_URL= "$ENV_FILE" 2>/dev/null | cut -d= -f2)
        
        # 检测域名是否发生变化
        if [ "$old_url" != "$new_url" ] && [ -n "$old_url" ] && [ -n "$new_url" ]; then
            log WARN "域名已变更！旧: ${old_url} → 新: ${new_url}"
            log STEP "正在重启 Python 后端以刷新域名..."
            
            pkill -f "$PYTHON_CMD" 2>/dev/null || true
            sleep 2
            
            # 用新环境变量重启后端
            if ! start_backend "$new_url"; then
                log FAIL "后端重启失败，域名可能尚未生效"
                return 1
            fi
            
            echo ""
            log OK "✨ 域名已全面刷新！"
            echo -e "       ${BOLD}🌐 新公网地址: ${CYAN}${new_url}${NC}"
            echo ""
            echo -e "  ${YELLOW}💡 提示:${NC}"
            echo -e "     Telegram 中点击「Web 管理后台」获取的新链接将使用新域名"
            echo -e "     如需立即测试登录，请在 TG Bot 中重新点击进入后台按钮"
        else
            log OK "隧道域名未变或首次启动，无需重启后端"
        fi
    else
        return 1
    fi
}
_handle_tunnel_failure() {
    log FAIL "Cloudflare Tunnel 在 ${TUNNEL_MAX_RETRIES} 次尝试后仍未成功"
    echo ""
    echo -e "  ${RED}═══ 隧道启动失败诊断 ═══${NC}"
    echo ""
    echo -e "  ${YELLOW}可能的原因:${NC}"
    echo "    1. Cloudflare Quick Tunnel 服务临时故障 (500/503)"
    echo "    2. 网络不稳定或 DNS 解析问题"
    echo "    3. IP 被 Cloudflare 限流 (短时间内请求过多)"
    echo "    4. cloudflared 版本过旧"
    echo ""
    echo -e "  ${YELLOW}隧道日志关键行:${NC}"
    tail -15 "$TUNNEL_LOG" 2>/dev/null | sed 's/^/    /'
    echo ""
    echo -e "  ${YELLOW}建议操作:${NC}"
    echo "    1. 稍后执行: ./dev.sh tunnel          # 仅重试隧道"
    echo "    2. 或执行:     ./dev.sh restart         # 全量重启"
    echo "    3. 手动检查:   cloudflared --version     # 确认版本 >= 2024.x"
    echo ""
    echo -e "  ${CYAN}⚠️  后端服务仍在本地正常运行:${NC}"
    echo -e "     本地访问: http://localhost:${TUNNEL_PORT}"
    echo -e "     本地前端: http://localhost:${VITE_PORT}"
    echo ""

    # 尝试用 .env 中已有的 URL（如果有备份）
    if [ -f "${ENV_FILE}.bak."* ]; then
        local latest_bak=$(ls -t ${ENV_FILE}.bak.* 2>/dev/null | head -1)
        if [ -n "$latest_bak" ]; then
            local saved_url=$(grep WEB_BASE_URL= "$latest_bak" 2>/dev/null | cut -d= -f2)
            if [ -n "$saved_url" ]; then
                echo -e "  ${YELLOW}💡 注意: 上次的公网地址可能仍有效: ${saved_url}${NC}"
                echo -e "     可以直接在浏览器中尝试打开此地址。"
            fi
        fi
    fi
}

# ==========================================
# 全量启动（有序编排）
# ==========================================

start_all() {
    banner
    ensure_dirs
    warn_if_system_cf_tunnel_running
    load_env_to_process || true
    
    local failures=0
    local total_services=4

    # Step 1: LiteLLM（AI 模型代理，最先启动）
    start_litellm || failures=$((failures + 1))

    # Step 2: Cloudflare 隧道（先拿域名，再启动后端）
    local expected_url=""
    if start_tunnel; then
        expected_url=$(grep '^WEB_BASE_URL=' "$ENV_FILE" 2>/dev/null | cut -d= -f2)
    else
        failures=$((failures + 1))
    fi

    # Step 3: Python 后端（校验域名环境变量已生效）
    if [ -n "$expected_url" ]; then
        start_backend "$expected_url" || failures=$((failures + 1))
    else
        start_backend || failures=$((failures + 1))
    fi

    # Step 4: Vite 前端
    start_frontend || failures=$((failures + 1))

    # 汇总报告
    echo ""
    echo -e "${BOLD}═══════════════════════════════════════════${NC}"
    if [ $failures -eq 0 ]; then
        echo -e "${GREEN}${BOLD}🎉 全部服务启动成功！${NC}"
    else
        echo -e "${YELLOW}⚠️  部分服务启动异常 (${failures}/${total_services})${NC}"
    fi
    echo -e "${BOLD}═══════════════════════════════════════════${NC}"
    echo ""
    echo -e "  服务地址:"
    echo -e "    🔗 公网访问: $(grep WEB_BASE_URL= "$ENV_FILE" 2>/dev/null | cut -d= -f2)"
    echo -e "    🖥  本地后端: http://localhost:${TUNNEL_PORT}"
    echo -e "    💻  本地前端: http://localhost:${VITE_PORT}"
    echo -e "    🤖  AI代理:   http://localhost:${LITELLM_PORT}"
    echo ""
    echo -e "  日志文件:"
    echo -e "    📄 后端:     ${BACKEND_LOG}"
    echo -e "    📄 前端:     ${FRONTEND_LOG}"
    echo -e "    📄 隧道:     ${TUNNEL_LOG}"
    echo -e "    📄 LiteLLM:  ${LITELLM_LOG}"
    echo ""
    echo -e "  常用命令:"
    echo -e "    ./dev.sh status    查看服务状态"
    echo -e "    ./dev.sh logs      查看实时日志"
    echo -e "    ./dev.sh stop      停止所有服务"
    echo -e "    ./dev.sh restart   重启所有服务"
    echo -e "    ./dev.sh tunnel    刷新隧道域名（变更时重启后端）"
    echo ""

    return $failures
}

# ==========================================
# 查看状态
# ==========================================

show_status() {
    banner
    echo -e "${BOLD}  服务状态一览${NC}"
    echo -e "  ──────────────────────────────────────"
    
    local total=0 running=0

    # Python 后端
    total=$((total + 1))
    if pgrep -f "$PYTHON_CMD" > /dev/null 2>&1; then
        local bp=$(pgrep -f "$PYTHON_CMD" | head -1)
        echo -e "  🐍 Python 后端 : ${GREEN}● 运行中${NC}  (PID: ${bp}, 端口: ${TUNNEL_PORT})"
        running=$((running + 1))
    else
        echo -e "  🐍 Python 后端 : ${RED}○ 未运行${NC}"
    fi

    # Vite 前端
    total=$((total + 1))
    if ss -tlnp 2>/dev/null | grep -q ":${VITE_PORT} " || pgrep -f "vite" > /dev/null 2>&1; then
        echo -e "  💻 Vite 前端  : ${GREEN}● 运行中${NC}  (端口: ${VITE_PORT})"
        running=$((running + 1))
    else
        echo -e "  💻 Vite 前端  : ${RED}○ 未运行${NC}"
    fi

    # Cloudflare Tunnel
    total=$((total + 1))
    if pgrep -f "$CLOUDFLARED_CMD" > /dev/null 2>&1; then
        local url=$(grep -oP "https://\S+\.trycloudflare\.com" "$TUNNEL_LOG" 2>/dev/null | tail -1)
        echo -e "  🌐 CF 隧道   : ${GREEN}● 运行中${NC}  (${url:-域名未知})"
        running=$((running + 1))
    else
        echo -e "  🌐 CF 隧道   : ${RED}○ 未运行${NC}"
    fi

    # LiteLLM
    total=$((total + 1))
    if docker ps -q --filter "name=${LITELLM_CONTAINER}" | grep -q . 2>/dev/null; then
        echo -e "  🤖 LiteLLM  : ${GREEN}● 运行中${NC}  (端口: ${LITELLM_PORT})"
        running=$((running + 1))
    elif command -v docker &>/dev/null; then
        echo -e "  🤖 LiteLLM  : ${RED}○ 未运行${NC}"
    else
        echo -e "  🤖 LiteLLM  : ${YELLOW}? Docker不可用${NC}"
    fi

    echo -e "  ──────────────────────────────────────"
    echo -e "  总计: ${running}/${total} 个服务在线"
    echo ""
    
    # 显示公网 URL
    if [ -f "$ENV_FILE" ] && grep -q "WEB_BASE_URL=" "$ENV_FILE"; then
        echo -e "  当前公网地址: $(grep WEB_BASE_URL= "$ENV_FILE" | cut -d= -f2)"
    fi
}

# ==========================================
# 实时日志
# ==========================================

show_logs() {
    local target=${1:-all}
    case $target in
        backend|be|b)
            tail -f "$BACKEND_LOG" 2>/dev/null || echo "日志文件不存在: $BACKEND_LOG"
            ;;
        frontend|fe|f)
            tail -f "$FRONTEND_LOG" 2>/dev/null || echo "日志文件不存在: $FRONTEND_LOG"
            ;;
        tunnel|cf|t)
            tail -f "$TUNNEL_LOG" 2>/dev/null || echo "日志文件不存在: $TUNNEL_LOG"
            ;;
        litellm|llm|l)
            docker logs -f ${LITELLM_CONTAINER} 2>/dev/null || echo "LiteLLM 容器未运行"
            ;;
        all|a)
            echo -e "${YELLOW}显示所有日志（Ctrl+C 退出）...${NC}\n"
            tail -f "$BACKEND_LOG" "$FRONTEND_LOG" "$TUNNEL_LOG" "$LITELLM_LOG" 2>/dev/null
            ;;
        *)
            echo "用法: ./dev.sh logs [backend|frontend|tunnel|litellm|all]"
            ;;
    esac
}

# ==========================================
# 主入口
# ==========================================

case "${1:-help}" in
    start|s)
        start_all
        ;;
    stop)
        stop_all
        ;;
    restart|r)
        stop_all
        sleep 2
        start_all
        ;;
    status|st)
        show_status
        ;;
    tunnel|tf)
        refresh_tunnel_only
        ;;
    logs|log)
        show_logs "${2:-all}"
        ;;
    help|--help|-h|"")
        banner
        echo -e " ${BOLD}用法:${NC} ./dev.sh <命令> [参数]"
        echo ""
        echo -e " ${BOLD}命令:${NC}"
        echo -e "  start, s          一键启动全部服务（LiteLLM + 后端 + 前端 + 隧道）"
        echo -e "  stop              停止全部服务并清理进程"
        echo -e "  restart, r        重启全部服务"
        echo -e "  status, st        查看各服务运行状态和端口"
        echo -e "  tunnel, tf        刷新 Cloudflare 隧道域名（变更时自动重启后端）"
        echo -e "  logs, log [类型]   实时跟踪日志"
        echo -e "                    类型: backend(b)/frontend(f)/tunnel(t)/litellm(l)/all(a)"
        echo -e "  help, -h, h       显示此帮助信息"
        echo ""
        echo -e " ${BOLD}示例:${NC}"
        echo "  ./dev.sh                    # 显示帮助"
        echo "  ./dev.sh start              # 一键全量启动"
        echo "  ./dev.sh logs frontend      # 只看前端日志"
        echo "  ./dev.sh tunnel             # 刷新隧道域名"
        echo "  ./dev.sh restart            # 重启全部"
        echo ""
        echo -e " ${BOLD}服务架构:${NC}"
        echo "  用户 → CF隧道(8080) → aiohttp后端(main.py) ← LiteLLM(4000)"
        echo "  用户 → Vite(5173)         → API Proxy → 后端"
        ;;
    *)
        log ERROR "未知命令: ${1}"
        echo "使用 './dev.sh help' 查看可用命令"
        exit 1
        ;;
esac
