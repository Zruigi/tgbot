# -*- coding: utf-8 -*-
import sqlite3
from pathlib import Path
for name in [r"d:\开发\tgbot\crypto_bot.sqlite3", r"d:\开发\tgbot\bot.db"]:
    p = Path(name)
    if not p.exists():
        print(name, "NOT FOUND")
        continue
    con = sqlite3.connect(p)
    tables = [r[0] for r in con.execute("select name from sqlite_master where type='table' and name not like 'sqlite_%'").fetchall()]
    print("DB", name, "tables", len(tables))
    print(tables)
