# 🛡️ 安全加固快速参考指南

**用途**: 开发团队在实施安全加固时的快速查阅手册  
**更新**: 2026-03-30

---

## 🚨 Critical漏洞速查表

| # | 漏洞 | 位置 | CVSS | 修复方案 | 优先级 |
|---|------|------|------|---------|--------|
| 1 | 财务数据无租户隔离 | finance_api.py:150 | 9.3 | 添加权限检查 | P0 |
| 2 | 地址输入无验证 | group_monitor.py:100 | 9.1 | 正则验证 | P0 |
| 3 | 财务对账无原子性 | finance_core.py:120 | 8.9 | 悲观锁 | P0 |
| 4 | 凭证明文存储 | config.py:1-50 | 8.8 | 哈希验证 | P0 |
| 5 | 群组操作无权限检查 | web_admin.py:1650 | 8.7 | 权限装饰器 | P0 |
| 6 | 金额精度无验证 | finance_core.py:89 | 8.5 | Decimal类型 | P0 |
| 7 | 无CSRF保护 | web_admin.py:全部 | 8.3 | CSRF Token | P0 |
| 8 | OTP无一次性保证 | web_admin.py:1103 | 8.1 | OTP管理器 | P0 |

---

## 📋 Phase 1 快速检查清单（第1-2周）

### 认证加固
- [ ] 创建 `SecurityConfig` 类
- [ ] 实现 `verify_token()` 函数
- [ ] 创建 `OTPManager` 类
- [ ] 创建 `RateLimiter` 类
- [ ] 添加 `rate_limit_middleware`
- [ ] 更新 `.env` 配置

### 权限检查
- [ ] 添加 `admin_can_view_user_finance()` 函数
- [ ] 添加 `admin_can_manage_group()` 函数
- [ ] 添加 `admin_can_manage_monitor()` 函数
- [ ] 创建 `@require_permission` 装饰器
- [ ] 更新所有API端点

### 数据库约束
- [ ] 添加 `unique_tx_hash` 约束
- [ ] 添加 `check_amount_positive` 约束
- [ ] 创建索引
- [ ] 运行迁移脚本

### CSRF保护
- [ ] 创建 `csrf.ts` 模块
- [ ] 实现 `initCSRFToken()` 函数
- [ ] 实现 `apiPost()` 函数
- [ ] 添加 `csrf_middleware`
- [ ] 更新所有POST请求

---

## 🔧 常见问题解决

### Q1: 如何生成ADMIN_TOKEN_HASH?
```bash
python -c "import hashlib; print(hashlib.sha256(b'your_token').hexdigest())"
```

### Q2: 如何测试OTP系统?
```python
from web_admin import otp_manager

# 生成OTP
otp, otp_hash = otp_manager.generate_otp('127.0.0.1')
print(f"OTP: {otp}")

# 验证OTP
result = otp_manager.verify_otp(otp, '127.0.0.1')
print(f"Verified: {result}")
```

### Q3: 如何测试权限检查?
```python
import asyncio
from database import admin_can_view_user_finance

async def test():
    result = await admin_can_view_user_finance(admin_id=1, user_id=999)
    print(f"Can view: {result}")

asyncio.run(test())
```

### Q4: 如何测试CSRF保护?
```bash
# 获取CSRF Token
TOKEN=$(curl -s http://localhost:8080/api/csrf-token \
  -H "Authorization: Bearer <token>" | jq -r '.token')

# 使用Token发送请求
curl -X POST http://localhost:8080/api/broadcast/chats \
  -H "Authorization: Bearer <token>" \
  -H "X-CSRF-Token: $TOKEN" \
  -H "Content-Type: application/json" \
  -d '{"chat_ids": [123], "content": "test"}'
```

---

## 🧪 测试用例模板

### 权限测试
```python
import pytest
from database import admin_can_view_user_finance

@pytest.mark.asyncio
async def test_admin_can_view_own_user():
    result = await admin_can_view_user_finance(admin_id=1, user_id=1)
    assert result is True

@pytest.mark.asyncio
async def test_admin_cannot_view_other_user():
    result = await admin_can_view_user_finance(admin_id=1, user_id=999)
    assert result is False
```

---

## 📊 进度跟踪

### Phase 1进度
```
Week 1:
  [████████░░] 80% - 认证加固系统
  [██████░░░░] 60% - 权限检查框架
  [████░░░░░░] 40% - 数据库约束

Week 2:
  [██████████] 100% - CSRF保护
  [██████████] 100% - 单元测试
  [████████░░] 80% - 集成测试
```

---

**最后更新**: 2026-03-30  
**下一步**: 开始Phase 1，预计2周内完成
