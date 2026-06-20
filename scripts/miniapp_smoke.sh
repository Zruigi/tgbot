#!/usr/bin/env bash
set -euo pipefail

BASE_URL="${1:-${BASE_URL:-http://127.0.0.1:8080}}"
BASE_URL="${BASE_URL%/}"

TMP_DIR="$(mktemp -d)"
cleanup() {
    rm -rf "$TMP_DIR"
}
trap cleanup EXIT

fail() {
    echo "FAIL: $*" >&2
    exit 1
}

request() {
    local method="$1"
    local path="$2"
    local expected="$3"
    local body="${4:-}"
    local out="$TMP_DIR/$(echo "${method}_${path}" | tr '/?' '__').body"
    local code

    if [ -n "$body" ]; then
        code="$(curl -sS -o "$out" -w "%{http_code}" \
            -X "$method" \
            -H "Content-Type: application/json" \
            --data "$body" \
            "${BASE_URL}${path}")"
    else
        code="$(curl -sS -o "$out" -w "%{http_code}" \
            -X "$method" \
            "${BASE_URL}${path}")"
    fi

    if [ "$code" != "$expected" ]; then
        echo "Response body for ${method} ${path}:"
        sed -n '1,12p' "$out" || true
        fail "${method} ${path} expected HTTP ${expected}, got ${code}"
    fi

    echo "$out"
}

echo "Mini App smoke check: ${BASE_URL}"

health_body="$(request GET /healthz 200)"
grep -q '"status": "ok"' "$health_body" || fail "/healthz did not report ok"
grep -q '"index": true' "$health_body" || fail "/healthz did not find frontend index"
grep -q '"assets": true' "$health_body" || fail "/healthz did not find frontend assets"
echo "OK: healthz"

admin_body="$(request GET /admin 200)"
grep -q '/assets/' "$admin_body" || fail "/admin HTML did not reference built assets"
request GET /admin/saas 200 >/dev/null
echo "OK: admin SPA entry"

miniapp_body="$(request GET /miniapp 200)"
grep -q '/assets/' "$miniapp_body" || fail "/miniapp HTML did not reference built assets"
asset_path="$(grep -o '/assets/[^"]*\.js' "$miniapp_body" | head -n 1 || true)"
if [ -z "$asset_path" ]; then
    fail "could not find JS asset in /miniapp HTML"
fi
request GET "$asset_path" 200 >/dev/null
echo "OK: miniapp HTML and assets"

context_body="$(request GET /api/session/context 200)"
grep -q '"authenticated": false' "$context_body" || fail "unauthenticated context did not remain unauthenticated"
echo "OK: unauthenticated session context"

bootstrap_body="$(request POST /api/miniapp/bootstrap 401 '{}')"
grep -q 'reopen_in_telegram_required' "$bootstrap_body" || fail "empty Mini App bootstrap did not require Telegram reopen"
echo "OK: Mini App bootstrap rejects non-Telegram entry"

users_body="$TMP_DIR/protected_users.body"
protected_code="$(curl -sS -o "$users_body" -w "%{http_code}" "${BASE_URL}/api/users")"
if [ "$protected_code" != "401" ]; then
    echo "Response body for GET /api/users:"
    sed -n '1,12p' "$users_body" || true
    fail "protected API should return HTTP 401 without token, got ${protected_code}"
fi
echo "OK: protected API rejects anonymous access"

echo "PASS: Mini App is reachable and protected-api gate is active"
