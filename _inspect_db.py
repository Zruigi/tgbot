# -*- coding: utf-8 -*-
import sqlite3
from pathlib import Path

db = Path(r"d:\开发\tgbot\crypto_bot.sqlite3")
con = sqlite3.connect(db)

all_tables = [r[0] for r in con.execute("SELECT name FROM sqlite_master WHERE type='table' ORDER BY name").fetchall() if not r[0].startswith('sqlite_')]
print("DB:", db)
print("\n-- tables --")
for name in all_tables:
    cnt = con.execute(f"SELECT COUNT(*) FROM {name}").fetchone()[0]
    print(f"{name}\trows={cnt}")

print("\n-- indexes --")
for row in con.execute("SELECT name, tbl_name, sql FROM sqlite_master WHERE type='index' AND sql IS NOT NULL ORDER BY tbl_name, name").fetchall():
    print(row[0], "|", row[1], "|", row[2])

print("\n-- columns --")
for name in all_tables:
    cols = con.execute(f"PRAGMA table_info({name})").fetchall()
    print(f"[{name}]")
    for c in cols:
        cid, col, ctype, notnull, dflt, pk = c
        print(f"  {col}\t{ctype}\tnotnull={notnull}\tdefault={dflt}\tpk={pk}")
