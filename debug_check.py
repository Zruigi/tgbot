# -*- coding: utf-8 -*-
import asyncio
import aiosqlite
import sys
sys.path.insert(0, '.')
import config

async def check():
    db_path = config.DB_NAME
    async with aiosqlite.connect(db_path) as db:
        db.row_factory = aiosqlite.Row
        
        # 查看 messages 表结构
        async with db.execute('PRAGMA table_info(messages)') as cursor:
            cols = await cursor.fetchall()
            print('=== messages 表字段 ===')
            for c in cols:
                print(f'  {c[1]} ({c[2]})')
        
        # 查看最新的几条消息
        async with db.execute('SELECT msg_id, from_userid, to_userid, content, type, status, is_deleted, timestamp FROM messages ORDER BY msg_id DESC LIMIT 10') as cursor:
            rows = await cursor.fetchall()
            print(f'\n=== 最新10条消息 ===')
            for r in rows:
                print(f'  msg_id={r["msg_id"]}, from={r["from_userid"]}, to={r["to_userid"]}, type={r["type"]}, status={r["status"]}, is_deleted={r["is_deleted"]}, content={str(r["content"] or "")[:40]}')
        
        # 查看 admin_inbox 表
        async with db.execute('SELECT * FROM admin_inbox') as cursor:
            inbox_rows = await cursor.fetchall()
            print(f'\n=== admin_inbox 内容 ===')
            for r in inbox_rows:
                print(f'  {dict(r)}')
        
        print(f'\n=== config.ADMIN_ID = {config.ADMIN_ID} (type={type(config.ADMIN_ID).__name__}) ===')
        
        # 模拟 get_chat_history 查询
        if inbox_rows:
            user_id = inbox_rows[0]['from_userid']
            admin_id = config.ADMIN_ID
            print(f'\n=== 模拟查询: user_id={user_id}, admin_id={admin_id} ===')
            async with db.execute('''
                SELECT m.msg_id, m.from_userid, m.to_userid, m.content, m.type, m.status, m.is_deleted
                FROM messages m
                WHERE ((m.from_userid = ? AND m.to_userid = ?) 
                   OR (m.from_userid = ? AND m.to_userid = ?))
                   AND m.is_deleted = 0
                ORDER BY m.timestamp ASC
                LIMIT 50
            ''', (user_id, admin_id, admin_id, user_id)) as cursor2:
                msgs = await cursor2.fetchall()
                print(f'  查询结果: {len(msgs)} 条消息')
                for m in msgs:
                    print(f'    msg_id={m["msg_id"]}, from={m["from_userid"]}, to={m["to_userid"]}, type={m["type"]}, status={m["status"]}, content={str(m["content"] or "")[:40]}')
        
        # 检查数据库文件名
        print(f'\n=== config.DB_NAME = {config.DB_NAME} ===')

asyncio.run(check())
