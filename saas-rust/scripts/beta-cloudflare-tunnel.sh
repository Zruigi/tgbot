#!/usr/bin/env bash
set -euo pipefail

# Isolated Cloudflare Quick Tunnel wrapper for the local Rust Beta service.
#
# This script manages only the cloudflared process whose PID is recorded in
# its own state directory. It never calls pkill, starts Python, reads a
# credential file, or forwards an origin other than loopback.

script_directory=$(cd "$(dirname "$(readlink -f "${BASH_SOURCE[0]}")")" && pwd)
rust_root=$(cd "$script_directory/.." && pwd)
state_directory=${BETA_CF_STATE_DIR:-"$rust_root/target/beta-cloudflare"}
pid_file="$state_directory/cloudflared.pid"
log_file=${BETA_CF_LOG_FILE:-"$state_directory/cloudflared.log"}
home_directory="$state_directory/home"
origin_host=${BETA_CF_ORIGIN_HOST:-127.0.0.1}
origin_port=${BETA_CF_ORIGIN_PORT:-18090}
probe_path=${BETA_CF_PROBE_PATH:-/healthz}
wait_seconds=${BETA_CF_WAIT_SECONDS:-30}

dry_run=false
command_name=status

usage() {
  cat <<'USAGE'
Usage: beta-cloudflare-tunnel.sh [--dry-run] <start|status|url|probe|stop>

Environment overrides (safe local defaults):
  BETA_CF_BIN           cloudflared executable (default: PATH, then /usr/local/bin/cloudflared)
  BETA_CF_ORIGIN_HOST   must remain 127.0.0.1 (default: 127.0.0.1)
  BETA_CF_ORIGIN_PORT   local Rust service port (default: 18090)
  BETA_CF_PROBE_PATH    local/public probe path (default: /healthz)
  BETA_CF_WAIT_SECONDS  URL wait timeout for start (default: 30)
  BETA_CF_STATE_DIR     PID/log/home directory (default: target/beta-cloudflare)
  BETA_CF_LOG_FILE      log path (default: <state>/cloudflared.log)

The Quick Tunnel is anonymous and temporary. No Cloudflare token or config is
loaded by this wrapper. The origin is intentionally restricted to loopback.
USAGE
}

while (($# > 0)); do
  case "$1" in
    --dry-run)
      dry_run=true
      shift
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    start|status|url|probe|stop)
      if [[ "$command_name" != status ]]; then
        printf 'Only one command may be supplied\n' >&2
        usage >&2
        exit 2
      fi
      command_name=$1
      shift
      ;;
    *)
      printf 'Unknown argument: %s\n' "$1" >&2
      usage >&2
      exit 2
      ;;
  esac
done

if [[ "$origin_host" != 127.0.0.1 ]]; then
  printf 'Refusing non-loopback Cloudflare origin host: %s (must be 127.0.0.1)\n' "$origin_host" >&2
  exit 1
fi
if ! [[ "$origin_port" =~ ^[0-9]+$ ]] || ((origin_port < 1024 || origin_port > 65535)); then
  printf 'Invalid Cloudflare origin port: %s\n' "$origin_port" >&2
  exit 1
fi
if ! [[ "$wait_seconds" =~ ^[0-9]+$ ]] || ((wait_seconds < 1 || wait_seconds > 300)); then
  printf 'Invalid Cloudflare wait timeout: %s\n' "$wait_seconds" >&2
  exit 1
fi
if [[ "$probe_path" != /* ]] || [[ "$probe_path" == *$'\n'* ]]; then
  printf 'Invalid Cloudflare probe path: %s\n' "$probe_path" >&2
  exit 1
fi

origin_url="http://${origin_host}:${origin_port}"

find_cloudflared() {
  if [[ -n "${BETA_CF_BIN:-}" ]]; then
    printf '%s\n' "$BETA_CF_BIN"
  elif command -v cloudflared >/dev/null 2>&1; then
    command -v cloudflared
  elif [[ -x /usr/local/bin/cloudflared ]]; then
    printf '%s\n' /usr/local/bin/cloudflared
  else
    return 1
  fi
}

cloudflared_bin=''
if cloudflared_bin=$(find_cloudflared); then
  [[ -x "$cloudflared_bin" ]] || {
    printf 'Cloudflared executable is not executable: %s\n' "$cloudflared_bin" >&2
    exit 1
  }
elif [[ "$command_name" == start && "$dry_run" == false ]]; then
  printf 'cloudflared was not found; set BETA_CF_BIN or install cloudflared\n' >&2
  exit 1
fi

origin_fragment=" tunnel --no-autoupdate --url ${origin_url}"

read_pid() {
  [[ -r "$pid_file" ]] || return 1
  local pid
  pid=$(<"$pid_file")
  [[ "$pid" =~ ^[0-9]+$ ]] || return 1
  printf '%s\n' "$pid"
}

process_args() {
  local pid=$1
  ps -p "$pid" -o args= 2>/dev/null | sed 's/^[[:space:]]*//'
}

process_matches() {
  local pid=$1 args
  [[ "$pid" =~ ^[0-9]+$ ]] || return 1
  kill -0 "$pid" >/dev/null 2>&1 || return 1
  args=$(process_args "$pid")
  [[ "$args" == *"$origin_fragment"* ]]
}

external_origin_pids() {
  ps -eo pid=,comm=,args= 2>/dev/null \
    | awk -v fragment="$origin_fragment" '$2 == "cloudflared" && index($0, fragment) > 0 {print $1}'
}

extract_url() {
  [[ -r "$log_file" ]] || return 0
  grep -Eo 'https://[A-Za-z0-9.-]+\.trycloudflare\.com' "$log_file" 2>/dev/null \
    | tail -n 1 || true
}

cleanup_stale_pid() {
  local pid
  pid=$(read_pid 2>/dev/null || true)
  [[ -n "$pid" ]] || return 0
  if ! process_matches "$pid"; then
    rm -f "$pid_file"
  fi
}

show_status() {
  local pid url external
  cleanup_stale_pid
  pid=$(read_pid 2>/dev/null || true)
  if [[ -n "$pid" ]] && process_matches "$pid"; then
    printf 'state=running\n'
    printf 'pid=%s\n' "$pid"
  else
    printf 'state=stopped\n'
  fi
  external=$(external_origin_pids | tr '\n' ' ' | sed 's/[[:space:]]*$//')
  if [[ -n "$external" && "$external" != "$pid" ]]; then
    printf 'unmanaged_origin_pids=%s\n' "$external"
  fi
  printf 'origin=%s\n' "$origin_url"
  url=$(extract_url)
  [[ -n "$url" ]] && printf 'url=%s\n' "$url"
  printf 'log=%s\n' "$log_file"
}

check_origin() {
  command -v curl >/dev/null 2>&1 || {
    printf 'curl is required for origin/probe checks\n' >&2
    return 1
  }
  curl -fsS --max-time 3 "${origin_url}${probe_path}" >/dev/null
}

start_tunnel() {
  local pid url elapsed external
  if [[ "$dry_run" == true ]]; then
    printf 'cloudflared=%s\n' "${cloudflared_bin:-${BETA_CF_BIN:-cloudflared}}"
    printf 'origin=%s\n' "$origin_url"
    printf 'command=env -i PATH=<system> HOME=%s XDG_CONFIG_HOME=%s XDG_CACHE_HOME=%s %s tunnel --no-autoupdate --url %s\n' \
      "$home_directory" "$home_directory/config" "$home_directory/cache" \
      "${cloudflared_bin:-${BETA_CF_BIN:-cloudflared}}" "$origin_url"
    return 0
  fi

  mkdir -p "$state_directory" "$home_directory/config" "$home_directory/cache"
  chmod 700 "$state_directory" "$home_directory" "$home_directory/config" "$home_directory/cache"
  cleanup_stale_pid
  pid=$(read_pid 2>/dev/null || true)
  if [[ -n "$pid" ]] && process_matches "$pid"; then
    printf 'Cloudflare Quick Tunnel already running (pid=%s)\n' "$pid"
    url=$(extract_url)
    [[ -n "$url" ]] && printf '%s\n' "$url"
    return 0
  fi

  external=$(external_origin_pids | tr '\n' ' ' | sed 's/[[:space:]]*$//')
  if [[ -n "$external" ]]; then
    printf 'Refusing duplicate tunnel for %s; unmanaged cloudflared pid(s): %s\n' \
      "$origin_url" "$external" >&2
    printf 'No existing process was stopped. Inspect with: %s status\n' "$0" >&2
    return 1
  fi

  : >"$log_file"
  # HOME/XDG paths point at an empty private directory so user Cloudflare
  # credentials/configuration cannot be picked up accidentally.
  nohup env -i \
    PATH="${PATH:-/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin}" \
    HOME="$home_directory" \
    XDG_CONFIG_HOME="$home_directory/config" \
    XDG_CACHE_HOME="$home_directory/cache" \
    "$cloudflared_bin" tunnel --no-autoupdate --url "$origin_url" \
    >>"$log_file" 2>&1 &
  pid=$!
  printf '%s\n' "$pid" >"$pid_file"

  for ((elapsed=0; elapsed<wait_seconds; elapsed++)); do
    if ! process_matches "$pid"; then
      printf 'Cloudflare Quick Tunnel exited before publishing a URL; inspect %s\n' "$log_file" >&2
      rm -f "$pid_file"
      return 1
    fi
    url=$(extract_url)
    if [[ -n "$url" ]]; then
      printf 'Cloudflare Quick Tunnel running (pid=%s)\n' "$pid"
      printf '%s\n' "$url"
      return 0
    fi
    sleep 1
  done
  printf 'Timed out waiting for Cloudflare Quick Tunnel URL; inspect %s\n' "$log_file" >&2
  return 1
}

stop_tunnel() {
  local pid waited
  pid=$(read_pid 2>/dev/null || true)
  if [[ -z "$pid" ]]; then
    printf 'Cloudflare Quick Tunnel is not running\n'
    return 0
  fi
  if ! process_matches "$pid"; then
    printf 'Refusing to stop pid=%s because it is not this wrapper'\''s cloudflared process\n' "$pid" >&2
    return 1
  fi
  if [[ "$dry_run" == true ]]; then
    printf 'would-stop pid=%s\n' "$pid"
    return 0
  fi
  kill "$pid" >/dev/null 2>&1 || true
  for ((waited=0; waited<20; waited++)); do
    if ! kill -0 "$pid" >/dev/null 2>&1; then
      rm -f "$pid_file"
      printf 'Cloudflare Quick Tunnel stopped (pid=%s)\n' "$pid"
      return 0
    fi
    sleep 0.25
  done
  # This is still the exact process recorded by this wrapper; never use pkill.
  kill -KILL "$pid" >/dev/null 2>&1 || true
  rm -f "$pid_file"
  printf 'Cloudflare Quick Tunnel stopped (pid=%s)\n' "$pid"
}

probe_tunnel() {
  local pid url
  pid=$(read_pid 2>/dev/null || true)
  [[ -n "$pid" ]] && process_matches "$pid" || {
    printf 'This wrapper has no running tunnel process; run start first\n' >&2
    return 1
  }
  url=$(extract_url)
  [[ -n "$url" ]] || {
    printf 'No Quick Tunnel URL is available; run start first\n' >&2
    return 1
  }
  printf 'origin_probe='
  if check_origin; then printf 'ok\n'; else printf 'failed\n'; return 1; fi
  printf 'public_url=%s\n' "$url"
  curl -fsS --max-time 10 "${url}${probe_path}" >/dev/null
  printf 'public_probe=ok\n'
}

case "$command_name" in
  start) start_tunnel ;;
  status) show_status ;;
  url)
    url=$(extract_url)
    [[ -n "$url" ]] || {
      printf 'No Quick Tunnel URL is available\n' >&2
      exit 1
    }
    printf '%s\n' "$url"
    ;;
  probe) probe_tunnel ;;
  stop) stop_tunnel ;;
esac

