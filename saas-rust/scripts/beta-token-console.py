#!/usr/bin/env python3

import json
import os
import re
import ssl
import subprocess
import threading
from http import HTTPStatus
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path


rust_root = Path(__file__).resolve().parent.parent
stack_script = rust_root / "scripts" / "beta-real-bot-stack.sh"
bridge_log_path = Path("/tmp/tgbot-beta-real-bot/bridge.log")
token_pattern = re.compile(r"\A[0-9]{6,}:[A-Za-z0-9_-]{20,}\Z")
operation_lock = threading.Lock()
last_operation = {"name": "idle", "ok": True, "message": "等待操作"}


def clean_environment(token=None):
    environment = {
        "PATH": os.environ.get("PATH", "/usr/local/bin:/usr/bin:/bin"),
        "HOME": os.environ.get("HOME", "/home/ubuntu"),
        "BETA_TEST_BOT_ENABLED": "true",
    }
    if token is not None:
        environment["BOT_TOKEN"] = token
    return environment


def run_stack(command, token=None, capture_output=False):
    output_stream = subprocess.PIPE if capture_output else subprocess.DEVNULL
    return subprocess.run(
        ["bash", str(stack_script), command],
        cwd=rust_root,
        env=clean_environment(token),
        stdin=subprocess.DEVNULL,
        stdout=output_stream,
        stderr=output_stream,
        text=True,
        timeout=120,
        check=False,
    )


def set_operation(name, ok, message):
    global last_operation
    last_operation = {"name": name, "ok": ok, "message": message}


def start_failure_message(completed):
    output = "\n".join(
        value for value in (completed.stdout, completed.stderr) if value
    )
    if "401 Unauthorized" in output or '"error_code":401' in output:
        return "切换失败：Telegram 返回 401，当前 token 无效、已失效或不是该 Bot 的 token"
    return "切换失败；请确认 token 有效"


def stack_status():
    try:
        result = run_stack("status", capture_output=True)
    except (OSError, subprocess.SubprocessError):
        return {"ok": False, "app": "unknown", "bridge": "unknown", "health": "unknown"}
    status_lines = {}
    for line in result.stdout.splitlines():
        for field in line.strip().split():
            key, separator, value = field.partition("=")
            if separator and key in {"app", "bridge", "app_health"}:
                status_lines[key] = value
    bot_identity = "unknown"
    try:
        bridge_log = bridge_log_path.read_text(encoding="utf-8", errors="replace")
        identity_match = re.search(
            r"bot_id=(\d+).*?bot_username=\"([^\"]+)\"",
            bridge_log,
        )
        if identity_match:
            bot_identity = f"@{identity_match.group(2)} (#{identity_match.group(1)})"
    except OSError:
        pass
    return {
        "ok": result.returncode == 0,
        "app": status_lines.get("app", "unknown"),
        "bridge": status_lines.get("bridge", "unknown"),
        "health": status_lines.get("app_health", "unknown"),
        "bot": bot_identity,
    }


def json_response(handler, payload, status=HTTPStatus.OK):
    body = json.dumps(payload, ensure_ascii=False).encode("utf-8")
    handler.send_response(status)
    handler.send_header("Content-Type", "application/json; charset=utf-8")
    handler.send_header("Content-Length", str(len(body)))
    handler.send_header("Cache-Control", "no-store")
    handler.send_header("X-Content-Type-Options", "nosniff")
    handler.end_headers()
    handler.wfile.write(body)


page = """<!doctype html>
<html lang="zh-CN">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width,initial-scale=1">
<title>Rust Beta 控制台</title>
<style>
:root{color-scheme:dark;font-family:Inter,system-ui,-apple-system,BlinkMacSystemFont,"Segoe UI",sans-serif;background:#0b1020;color:#e8ecf7}
body{margin:0;min-height:100vh;display:grid;place-items:center;background:radial-gradient(circle at 15% 15%,#1b3560 0,#0b1020 48%,#070a13 100%)}
main{width:min(680px,calc(100% - 32px));padding:32px;border:1px solid #2c3c60;border-radius:24px;background:rgba(13,20,38,.9);box-shadow:0 24px 80px #0008}
h1{margin:0 0 8px;font-size:28px}p{color:#aebbd4;line-height:1.6}.notice{padding:12px 14px;border-radius:12px;background:#152743;color:#bcd4ff;font-size:14px}
label{display:block;margin:24px 0 8px;color:#dbe5fb;font-weight:600}input{box-sizing:border-box;width:100%;padding:14px 16px;border:1px solid #40527a;border-radius:12px;background:#080d19;color:#fff;font-size:16px;outline:none}input:focus{border-color:#79a9ff;box-shadow:0 0 0 3px #477dff33}
.actions{display:flex;gap:10px;flex-wrap:wrap;margin-top:18px}button{border:0;border-radius:12px;padding:12px 18px;background:#4f82ff;color:white;font-weight:700;cursor:pointer}button.secondary{background:#263653}button.danger{background:#8d3f55}button:disabled{opacity:.55;cursor:wait}
.status{display:grid;grid-template-columns:repeat(4,1fr);gap:10px;margin-top:26px}.card{padding:14px;border:1px solid #2a3a5d;border-radius:14px;background:#101a30}.key{display:block;color:#91a4c8;font-size:12px;text-transform:uppercase}.value{display:block;margin-top:5px;font-size:18px;font-weight:700}.good{color:#70e1ad}.bad{color:#ff9ba8}
#message{min-height:24px;margin-top:18px;color:#bcd4ff}.foot{margin-top:22px;font-size:12px;color:#8292b1}
@media(max-width:720px){main{padding:22px}.status{grid-template-columns:repeat(2,1fr)}}
</style>
</head>
<body>
<main>
<h1>Rust Beta 人工验收控制台</h1>
<p>输入一次性测试 Bot token，启动隔离 Beta Webhook/Worker 栈。token 只传给当前启动进程，不写入网页、日志、仓库或数据库。</p>
<div class="notice">当前外部入口使用临时 HTTPS；请只输入一次性测试 token。点击“启动 Beta”会切换当前隔离栈使用的新 token。</div>
<label for="token">BOT_TOKEN</label>
<input id="token" type="password" autocomplete="off" spellcheck="false" placeholder="只在此处输入一次性测试 token">
<div class="actions"><button id="start">启动 Beta</button><button class="secondary" id="refresh">刷新状态</button><button class="danger" id="stop">停止 Beta</button></div>
<div class="status"><div class="card"><span class="key">App</span><span class="value" id="app">—</span></div><div class="card"><span class="key">Bridge</span><span class="value" id="bridge">—</span></div><div class="card"><span class="key">Health</span><span class="value" id="health">—</span></div><div class="card"><span class="key">Bot</span><span class="value" id="bot">—</span></div></div>
<div id="message"></div>
<div class="foot">此控制台只控制 /home/ubuntu/tgbot-rust-beta/saas-rust 的隔离 Beta，不连接生产数据库、不重启生产服务、不执行上线。</div>
</main>
<script>
const tokenInput=document.getElementById('token');
const message=document.getElementById('message');
const buttons=[document.getElementById('start'),document.getElementById('refresh'),document.getElementById('stop')];
function paint(id,value){const node=document.getElementById(id);node.textContent=value||'unknown';node.className='value '+(value==='running'||value==='ok'?'good':value==='stopped'||value==='unavailable'?'bad':'')}
async function refresh(){const response=await fetch('/api/status',{cache:'no-store'});const data=await response.json();paint('app',data.app);paint('bridge',data.bridge);paint('health',data.health);paint('bot',data.bot);}
async function action(path,body){buttons.forEach(button=>button.disabled=true);message.textContent='处理中…';try{const response=await fetch(path,{method:'POST',headers:{'Content-Type':'application/json'},body:body?JSON.stringify(body):'{}'});const data=await response.json();message.textContent=data.message||'操作完成';await refresh()}catch(error){message.textContent='控制台请求失败，请检查服务状态'}finally{buttons.forEach(button=>button.disabled=false)}}
document.getElementById('start').onclick=async()=>{const token=tokenInput.value.trim();if(!token){message.textContent='请输入一次性测试 token';return}await action('/api/start',{token});tokenInput.value=''};
document.getElementById('refresh').onclick=()=>action('/api/status');
document.getElementById('stop').onclick=()=>action('/api/stop');
refresh();setInterval(refresh,5000);
</script>
</body>
</html>"""


class ConsoleHandler(BaseHTTPRequestHandler):
    def log_message(self, format_string, *args):
        return

    def send_page(self):
        body = page.encode("utf-8")
        self.send_response(HTTPStatus.OK)
        self.send_header("Content-Type", "text/html; charset=utf-8")
        self.send_header("Content-Length", str(len(body)))
        self.send_header("Cache-Control", "no-store")
        self.send_header("Content-Security-Policy", "default-src 'self'; style-src 'self' 'unsafe-inline'; script-src 'self' 'unsafe-inline'")
        self.send_header("X-Content-Type-Options", "nosniff")
        self.end_headers()
        self.wfile.write(body)

    def do_GET(self):
        if self.path == "/":
            self.send_page()
            return
        if self.path == "/api/status":
            payload = stack_status()
            payload["operation"] = last_operation
            json_response(self, payload)
            return
        json_response(self, {"ok": False, "message": "not found"}, HTTPStatus.NOT_FOUND)

    def do_POST(self):
        if self.path == "/api/status":
            payload = stack_status()
            payload["operation"] = last_operation
            json_response(self, payload)
            return
        if self.path not in {"/api/start", "/api/stop"}:
            json_response(self, {"ok": False, "message": "not found"}, HTTPStatus.NOT_FOUND)
            return
        content_length = int(self.headers.get("Content-Length", "0"))
        if content_length > 4096:
            json_response(self, {"ok": False, "message": "请求过大"}, HTTPStatus.REQUEST_ENTITY_TOO_LARGE)
            return
        try:
            body = json.loads(self.rfile.read(content_length) or b"{}")
        except (ValueError, UnicodeDecodeError):
            json_response(self, {"ok": False, "message": "请求格式无效"}, HTTPStatus.BAD_REQUEST)
            return
        token = str(body.get("token", "")).strip() if isinstance(body, dict) else ""
        if self.path == "/api/start" and not token_pattern.fullmatch(token):
            json_response(self, {"ok": False, "message": "token 格式无效"}, HTTPStatus.BAD_REQUEST)
            return
        with operation_lock:
            try:
                if self.path == "/api/start":
                    stopped = run_stack("stop")
                    completed = (
                        run_stack("start", token, capture_output=True)
                        if stopped.returncode == 0
                        else stopped
                    )
                else:
                    completed = run_stack("stop")
                result = completed.returncode == 0
            except (OSError, subprocess.SubprocessError):
                result = False
            if self.path == "/api/start":
                message = (
                    "Beta 已切换，可在 Telegram 中开始人工验收"
                    if result
                    else start_failure_message(completed)
                )
                set_operation("switch", result, message)
            else:
                message = "Beta 已停止" if result else "停止失败，请检查隔离栈状态"
                set_operation("stop", result, message)
        json_response(self, {"ok": result, "message": message}, HTTPStatus.OK if result else HTTPStatus.CONFLICT)


def main():
    bind_address = os.environ.get("BETA_TOKEN_CONSOLE_BIND", "127.0.0.1")
    port = int(os.environ.get("BETA_TOKEN_CONSOLE_PORT", "18080"))
    loopback_addresses = {"127.0.0.1", "localhost", "::1"}
    public_binding = bind_address not in loopback_addresses
    if public_binding and os.environ.get("BETA_TOKEN_CONSOLE_ALLOW_PUBLIC") != "true":
        raise SystemExit("Refusing non-loopback Beta token console without explicit BETA_TOKEN_CONSOLE_ALLOW_PUBLIC=true")
    if not stack_script.is_file():
        raise SystemExit(f"Missing Beta stack script: {stack_script}")
    server = ThreadingHTTPServer((bind_address, port), ConsoleHandler)
    scheme = "http"
    if public_binding:
        certificate_path = os.environ.get("BETA_TOKEN_CONSOLE_TLS_CERT", "/tmp/tgbot-beta-token-console.crt")
        key_path = os.environ.get("BETA_TOKEN_CONSOLE_TLS_KEY", "/tmp/tgbot-beta-token-console.key")
        if not Path(certificate_path).is_file() or not Path(key_path).is_file():
            server.server_close()
            raise SystemExit("External Beta token console requires BETA_TOKEN_CONSOLE_TLS_CERT and BETA_TOKEN_CONSOLE_TLS_KEY")
        tls_context = ssl.SSLContext(ssl.PROTOCOL_TLS_SERVER)
        tls_context.load_cert_chain(certificate_path, key_path)
        server.socket = tls_context.wrap_socket(server.socket, server_side=True)
        scheme = "https"
    print(f"Beta token console listening at {scheme}://{bind_address}:{port}", flush=True)
    try:
        server.serve_forever()
    except KeyboardInterrupt:
        pass
    finally:
        server.server_close()


if __name__ == "__main__":
    main()
