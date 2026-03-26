# -*- coding: utf-8 -*-
import ast
from pathlib import Path

root = Path(r"d:\开发\tgbot")
files = list(root.glob("*.py")) + list((root / "handlers").glob("*.py")) + list((root / "services").glob("*.py"))

for p in sorted(files):
    try:
        tree = ast.parse(p.read_text(encoding='utf-8'))
    except Exception as e:
        print(f"## {p.relative_to(root)}\n  parse_error: {e}\n")
        continue
    imports = []
    for n in ast.walk(tree):
        if isinstance(n, ast.Import):
            for a in n.names:
                imports.append(a.name)
        elif isinstance(n, ast.ImportFrom):
            mod = n.module or ''
            imports.append(mod)
    print(f"## {p.relative_to(root)}")
    for i in sorted(set(i for i in imports if i)):
        print(" ", i)
    print()
