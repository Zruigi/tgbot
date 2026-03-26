import asyncio, aiosqlite, os, sys
sys.stdout.reconfigure(encoding='utf-8', errors='replace')
DB = os.path.join(os.getcwd(), 'crypto_bot.sqlite3')
async def q():
    async with aiosqlite.connect(DB) as db:
        async with db.execute('SELECT revoke_checked, COUNT(*) FROM group_messages WHERE group_id=-1001991086404 GROUP BY revoke_checked') as c:
            rows = await c.fetchall()
            for r in rows: print(f'revoke_checked={r[0]}: {r[1]} rows')
asyncio.run(q())
