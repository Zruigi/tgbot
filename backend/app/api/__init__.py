# -*- coding: utf-8 -*-
"""
API 模块导出 - 统一管理所有 API 路由
"""
from . import auth
from . import chat
from . import config
from . import group
from . import broadcast
from . import ai
from . import monitor
from . import potential
from . import saas

__all__ = [
    'auth',
    'chat',
    'config',
    'group',
    'broadcast',
    'ai',
    'monitor',
    'potential',
    'saas',
]

# 路由注册表（供 main.py 使用）
ROUTES = []

# 从各模块收集路由
for module in __all__:
    mod_obj = globals()[module]
    for name in dir(mod_obj):
        obj = getattr(mod_obj, name)
        if hasattr(obj, '__route__'):
            ROUTES.append(obj)
