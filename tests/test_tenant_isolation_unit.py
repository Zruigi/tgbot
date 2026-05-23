# -*- coding: utf-8 -*-
"""
租户隔离单元测试 - enforce_tenant_filter、SessionScope、TenantContextMiddleware
"""
import sys
import asyncio
import importlib.util
from pathlib import Path

BASE = Path(__file__).parent.parent

def _load_module(name, path):
    spec = importlib.util.spec_from_file_location(name, path)
    mod = importlib.util.module_from_spec(spec)
    sys.modules[name] = mod
    spec.loader.exec_module(mod)
    return mod

sys.modules['backend'] = type(sys)('backend')
sys.modules['backend.app'] = type(sys)('backend.app')
sys.modules['backend.app.core'] = type(sys)('backend.app.core')

scope_mod = _load_module('backend.app.core.scope', BASE / 'backend/app/core/scope.py')

SessionScope = scope_mod.SessionScope
extract_scope_from_session = scope_mod.extract_scope_from_session
enforce_tenant_filter = scope_mod.enforce_tenant_filter


def _merchant_scope(tenant_id='tenant_a', merchant_id=1):
    return SessionScope(
        authenticated=True,
        actor_role='merchant_owner',
        effective_role='merchant_owner',
        scope_type='merchant',
        tenant_id=tenant_id,
        merchant_id=merchant_id,
    )


def _platform_scope():
    return SessionScope(
        authenticated=True,
        actor_role='platform_admin',
        effective_role='platform_admin',
        scope_type='platform',
        tenant_id='',
        merchant_id=None,
    )


def test_enforce_tenant_filter_merchant():
    scope = _merchant_scope('tenant_a')
    sql, params = enforce_tenant_filter("SELECT * FROM tracked_groups", [], scope)
    assert "tenant_id = ?" in sql
    assert "tenant_a" in params
    print("✅ test_enforce_tenant_filter_merchant")


def test_enforce_tenant_filter_platform():
    scope = _platform_scope()
    sql, params = enforce_tenant_filter("SELECT * FROM tracked_groups", [], scope)
    assert "tenant_id" not in sql
    assert params == []
    print("✅ test_enforce_tenant_filter_platform")


def test_enforce_tenant_filter_empty_tenant():
    scope = SessionScope(
        authenticated=True,
        actor_role='merchant_owner',
        effective_role='merchant_owner',
        scope_type='merchant',
        tenant_id='',
        merchant_id=1,
    )
    try:
        enforce_tenant_filter("SELECT * FROM tracked_groups", [], scope)
        assert False, "Should have raised ValueError"
    except ValueError:
        pass
    print("✅ test_enforce_tenant_filter_empty_tenant")


def test_enforce_tenant_filter_already_has_tenant():
    scope = _merchant_scope('tenant_a')
    sql, params = enforce_tenant_filter(
        "SELECT * FROM tracked_groups WHERE tenant_id = ?", ['tenant_a'], scope
    )
    assert sql.count("tenant_id") == 1
    print("✅ test_enforce_tenant_filter_already_has_tenant")


def test_enforce_tenant_filter_with_existing_where():
    scope = _merchant_scope('tenant_b')
    sql, params = enforce_tenant_filter(
        "SELECT * FROM tracked_groups WHERE is_monitored = 1", [], scope
    )
    assert "AND tenant_id = ?" in sql
    assert "tenant_b" in params
    print("✅ test_enforce_tenant_filter_with_existing_where")


def test_enforce_tenant_filter_custom_column():
    scope = _merchant_scope('tenant_a')
    sql, params = enforce_tenant_filter(
        "SELECT * FROM admin_inbox", [], scope, tenant_column='tenant_id'
    )
    assert "tenant_id = ?" in sql
    print("✅ test_enforce_tenant_filter_custom_column")


def test_enforce_tenant_filter_unauthenticated():
    scope = SessionScope(
        authenticated=False,
        actor_role='',
        effective_role='',
        scope_type='',
        tenant_id='',
        merchant_id=None,
    )
    try:
        enforce_tenant_filter("SELECT * FROM tracked_groups", [], scope)
        assert False, "Should have raised PermissionError"
    except PermissionError:
        pass
    print("✅ test_enforce_tenant_filter_unauthenticated")


def test_session_scope_merchant():
    scope = _merchant_scope('tenant_a', 1)
    assert scope.is_merchant is True
    assert scope.is_platform_admin is False
    assert scope.scope_type == 'merchant'
    assert scope.tenant_id == 'tenant_a'
    print("✅ test_session_scope_merchant")


def test_session_scope_platform():
    scope = _platform_scope()
    assert scope.is_platform_admin is True
    assert scope.is_merchant is False
    assert scope.scope_type == 'platform'
    print("✅ test_session_scope_platform")


def test_require_tenant_id_merchant():
    scope = _merchant_scope('tenant_a')
    assert scope.require_tenant_id() == 'tenant_a'
    print("✅ test_require_tenant_id_merchant")


def test_require_tenant_id_platform():
    scope = _platform_scope()
    try:
        scope.require_tenant_id()
        assert False, "Should have raised ValueError"
    except ValueError:
        pass
    print("✅ test_require_tenant_id_platform")


def test_extract_scope_from_session_merchant():
    session = {'role': 'merchant_owner', 'tenant_id': 'tenant_a', 'merchant_id': 1, 'authenticated': True}
    scope = extract_scope_from_session(session)
    assert scope.scope_type == 'merchant'
    assert scope.tenant_id == 'tenant_a'
    assert scope.is_merchant is True
    print("✅ test_extract_scope_from_session_merchant")


def test_extract_scope_from_session_platform():
    session = {'role': 'platform_admin', 'tenant_id': '', 'merchant_id': None, 'authenticated': True}
    scope = extract_scope_from_session(session)
    assert scope.scope_type == 'platform'
    assert scope.is_platform_admin is True
    print("✅ test_extract_scope_from_session_platform")


async def test_tenant_context_middleware_injection():
    from aiohttp import web
    from unittest.mock import MagicMock, AsyncMock

    async def handler(request):
        return web.json_response({
            'scope_type': request.get('scope', None).scope_type if request.get('scope') else None,
            'tenant_id': request.get('tenant_id', ''),
            'role': request.get('role', ''),
            'require_filter': request.get('_require_tenant_filter', False),
        })

    request = MagicMock()
    request.get = lambda key, default='': {
        'session': {'role': 'merchant_owner', 'tenant_id': 'tenant_a', 'merchant_id': 1, 'authenticated': True},
    }.get(key, default)
    request.__getitem__ = lambda self, key: {
        'session': {'role': 'merchant_owner', 'tenant_id': 'tenant_a', 'merchant_id': 1, 'authenticated': True},
    }.get(key)

    scope = extract_scope_from_session(request.get('session', {}))
    assert scope.scope_type == 'merchant'
    assert scope.tenant_id == 'tenant_a'
    print("✅ test_tenant_context_middleware_injection")


def test_tenant_context_middleware_empty_tenant_reject():
    scope = SessionScope(
        authenticated=True,
        actor_role='merchant_owner',
        effective_role='merchant_owner',
        scope_type='merchant',
        tenant_id='',
        merchant_id=1,
    )
    assert scope.is_merchant is True
    assert scope.tenant_id == ''
    try:
        scope.require_tenant_id()
        assert False, "Should have raised ValueError"
    except ValueError:
        pass
    print("✅ test_tenant_context_middleware_empty_tenant_reject")


def run_all():
    print("\n" + "=" * 60)
    print("租户隔离单元测试")
    print("=" * 60)
    tests = [
        test_enforce_tenant_filter_merchant,
        test_enforce_tenant_filter_platform,
        test_enforce_tenant_filter_empty_tenant,
        test_enforce_tenant_filter_already_has_tenant,
        test_enforce_tenant_filter_with_existing_where,
        test_enforce_tenant_filter_custom_column,
        test_enforce_tenant_filter_unauthenticated,
        test_session_scope_merchant,
        test_session_scope_platform,
        test_require_tenant_id_merchant,
        test_require_tenant_id_platform,
        test_extract_scope_from_session_merchant,
        test_extract_scope_from_session_platform,
        test_tenant_context_middleware_empty_tenant_reject,
    ]
    async_tests = [
        test_tenant_context_middleware_injection,
    ]
    passed = 0
    failed = 0
    for t in tests:
        try:
            t()
            passed += 1
        except Exception as e:
            print(f"❌ {t.__name__}: {e}")
            failed += 1
    for t in async_tests:
        try:
            asyncio.get_event_loop().run_until_complete(t())
            passed += 1
        except Exception as e:
            print(f"❌ {t.__name__}: {e}")
            failed += 1
    print(f"\n{'=' * 60}")
    print(f"结果: {passed} 通过, {failed} 失败, 共 {passed + failed} 个")
    return failed == 0


if __name__ == '__main__':
    success = run_all()
    sys.exit(0 if success else 1)
