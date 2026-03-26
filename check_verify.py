# -*- coding: utf-8 -*-
import sqlite3

conn = sqlite3.connect('crypto_bot.sqlite3')
conn.row_factory = sqlite3.Row
c = conn.cursor()

print('=== 所有表 ===')
c.execute("SELECT name FROM sqlite_master WHERE type='table' ORDER BY name")
for r in c.fetchall():
    print(' ', r[0])

print('\n=== join_verify_sessions 表结构 ===')
try:
    c.execute('PRAGMA table_info(join_verify_sessions)')
    rows = c.fetchall()
    if rows:
        for r in rows:
            print(' ', dict(r))
    else:
        print('  表存在但无列（异常）')
except Exception as e:
    print('  查询失败:', e)

print('\n=== join_verify_sessions 最近5条记录 ===')
try:
    c.execute('SELECT * FROM join_verify_sessions ORDER BY session_id DESC LIMIT 5')
    rows = c.fetchall()
    if rows:
        for r in rows:
            print(' ', dict(r))
    else:
        print('  无记录')
except Exception as e:
    print('  查询失败:', e)

conn.close()
