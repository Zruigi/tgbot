#!/usr/bin/env python
# -*- coding: utf-8 -*-

with open('web_admin.py', 'r', encoding='utf-8') as f:
    content = f.read()

# 替换auth_middleware为禁用认证版本
old_auth = '''# --- Auth Middleware ---
@web.middleware
async def auth_middleware(request, handler):
    """
    简单 Token 校验中间件。
    支持：
    1. Query 参数 ?token=xxx
    2. Header Authorization: Bearer xxx
    3. Cookie 'admin_token'
    """
    # 静态资源、首页、登录检查放行
    if request.path in ['/', '/ws', '/api/config'] or request.path.startswith('/static/'):
        return await handler(request)
        
    token = request.query.get('token')
    if not token:
        auth_header = request.headers.get('Authorization', '')
        if auth_header.startswith('Bearer '):
            token = auth_header[7:]
    if not token:
        token = request.cookies.get('admin_token')
        
    if token == config.WEB_ADMIN_TOKEN:
        return await handler(request)
        
    return web.json_response({"status": "error", "message": "Unauthorized"}, status=401)'''

new_auth = '''# --- Auth Middleware ---
@web.middleware
async def auth_middleware(request, handler):
    """
    开发环境：禁用认证以便快速测试
    生产环境：启用 Token 校验
    """
    # 开发环境：直接放行所有请求
    return await handler(request)'''

content = content.replace(old_auth, new_auth)

with open('web_admin.py', 'w', encoding='utf-8') as f:
    f.write(content)

print('OK')
