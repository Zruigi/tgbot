#!/usr/bin/env python
# -*- coding: utf-8 -*-

# 添加潜在用户API到web_admin.py
with open('web_admin.py', 'r', encoding='utf-8') as f:
    content = f.read()

# 新的API函数
new_api = '''
async def api_get_potential_users(request):
    """获取潜在用户列表"""
    try:
        users = await database.get_potential_users(limit=100)
        return web.json_response({
            "status": "ok",
            "users": users,
            "total": len(users)
        })
    except Exception as e:
        logger.error(f"获取潜在用户失败: {e}")
        return web.json_response({"status": "error", "message": str(e)}, status=500)

async def api_get_group_members(request):
    """获取群组成员列表"""
    try:
        group_id = request.query.get('group_id')
        if not group_id:
            return web.json_response({"status": "error", "message": "group_id required"}, status=400)
        
        members = await database.get_group_members(int(group_id))
        return web.json_response({
            "status": "ok",
            "members": members,
            "total": len(members)
        })
    except Exception as e:
        logger.error(f"获取群组成员失败: {e}")
        return web.json_response({"status": "error", "message": str(e)}, status=500)
'''

# 找到路由定义的位置
insert_pos = content.find("async def start_web_server(bot):")
if insert_pos > 0:
    # 在start_web_server之前插入新的API函数
    content = content[:insert_pos] + new_api + "\n\n" + content[insert_pos:]

# 添加新的路由
old_route = "app.router.add_delete('/api/chat/{user_id}', api_delete_chat)"
new_route = old_route + "\n    app.router.add_get('/api/potential_users', api_get_potential_users)\n    app.router.add_get('/api/group_members', api_get_group_members)"

content = content.replace(old_route, new_route)

with open('web_admin.py', 'w', encoding='utf-8') as f:
    f.write(content)

print('OK')
