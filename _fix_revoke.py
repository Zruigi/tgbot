import asyncio, aiosqlite, os, sys
sys.stdout.reconfigure(encoding='utf-8', errors='replace')
DB = os.path.join(os.getcwd(), 'crypto_bot.sqlite3')
async def fix():
    async with aiosqlite.connect(DB) as db:
        # 关闭所有群的 delete_on_revoke，彻底停止信息校对循环
        cur = await db.execute('UPDATE tracked_groups SET delete_on_revoke = 0')
        print(f'disabled delete_on_revoke for {cur.rowcount} groups')
        # 标记所有未校对消息
        cur2 = await db.execute('UPDATE group_messages SET revoke_checked = 1 WHERE COALESCE(revoke_checked, 0) = 0')
        print(f'marked {cur2.rowcount} messages as checked')
        await db.commit()
        print('done')
asyncio.run(fix())
