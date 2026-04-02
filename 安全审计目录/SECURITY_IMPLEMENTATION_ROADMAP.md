# 🗺️ 安全加固实施路线图

**文档版本**: v1.0  
**生成日期**: 2026-03-30  
**总耗时**: 6周  
**团队规模**: 3-5人

---

## 📅 总体时间表

```
Week 1-2: Phase 1 - 基础安全加固
├── 认证系统加固
├── 权限检查框架
├── 数据库约束
└── CSRF保护

Week 3-4: Phase 2 - 数据安全加固
├── 财务并发控制
├── 输入验证系统
├── 前端XSS防护
└── CSP头部署

Week 5-6: Phase 3 - 监控与审计
├── 安全日志系统
├── 异常检测
├── 渗透测试
└── 文档与培训
```

---

## 🎯 Week 1: 认证系统加固

### Day 1-2: 凭证管理现代化

**任务**:
```python
# 1. 创建 SecurityConfig 类
# 位置: config.py

class SecurityConfig:
    ADMIN_TOKEN_HASH = os.getenv('ADMIN_TOKEN_HASH')
    TOKEN_EXPIRY_DAYS = int(os.getenv('TOKEN_EXPIRY_DAYS', 90))
    OTP_EXPIRY_MINUTES = int(os.getenv('OTP_EXPIRY_MINUTES', 5))
    MAX_LOGIN_ATTEMPTS = 5
    LOGIN_ATTEMPT_WINDOW = 300

# 2. 实现 verify_token() 函数
def verify_token(token: str) -> bool:
    return hashlib.sha256(token.encode()).hexdigest() == SecurityConfig.ADMIN_TOKEN_HASH

# 3. 更新 .env 文件
ADMIN_TOKEN_HASH=<sha256(your_token)>
TOKEN_EXPIRY_DAYS=90
```

**验证**:
- [ ] 配置文件正确加载
- [ ] Token验证函数工作正常
- [ ] 环境变量已设置

---

### Day 3-4: OTP系统加固

**任务**:
```python
# 1. 创建 OTPManager 类
# 位置: web_admin.py

class OTPManager:
    def __init__(self):
        self._otp_store: Dict[str, Dict] = {}
        self._failed_attempts: Dict[str, Dict] = {}
    
    def generate_otp(self, client_ip: str) -> Tuple[str, str]:
        # 实现OTP生成逻辑
        pass
    
    def verify_otp(self, otp: str, client_ip: str) -> bool:
        # 实现OTP验证逻辑
        pass

# 2. 创建全局实例
otp_manager = OTPManager()
```

**验证**:
- [ ] OTP生成正常
- [ ] OTP一次性使用有效
- [ ] 速率限制工作正常
- [ ] 登录端点集成成功

---

### Day 5: 速率限制中间件

**任务**:
```python
# 1. 创建 RateLimiter 类
class RateLimiter:
    def __init__(self):
        self._attempts: Dict[str, list] = defaultdict(list)
    
    def is_allowed(self, key: str, max_attempts: int, window_seconds: int) -> bool:
        # 实现速率限制逻辑
        pass

# 2. 创建中间件
@web.middleware
async def rate_limit_middleware(request, handler):
    # 实现中间件逻辑
    pass
```

**验证**:
- [ ] 登录端点速率限制工作
- [ ] 广播端点速率限制工作
- [ ] 超限返回429状态码

---

## 🎯 Week 2: 权限与CSRF

### Day 1-2: 权限检查框架

**任务**:
```python
# 1. 添加权限检查函数到 database.py
async def admin_can_view_user_finance(admin_id: int, user_id: int) -> bool:
    pass

async def admin_can_manage_group(admin_id: int, group_id: int) -> bool:
    pass

# 2. 创建权限装饰器
def require_permission(permission_check_func):
    def decorator(handler):
        @wraps(handler)
        async def wrapper(request):
            # 实现装饰器逻辑
            pass
        return wrapper
    return decorator
```

**验证**:
- [ ] 权限检查函数工作正常
- [ ] 装饰器正确应用
- [ ] 所有API端点都有权限检查

---

### Day 3-4: 数据库约束

**任务**:
```sql
-- 1. 添加唯一约束
ALTER TABLE finance_ledgers 
ADD CONSTRAINT unique_tx_hash UNIQUE(tx_hash) WHERE tx_hash IS NOT NULL;

-- 2. 添加检查约束
ALTER TABLE finance_ledgers 
ADD CONSTRAINT check_amount_positive CHECK(amount > 0);

-- 3. 创建索引
CREATE INDEX idx_finance_ledgers_user_id ON finance_ledgers(user_id);
```

**验证**:
- [ ] 约束已添加
- [ ] 索引已创建
- [ ] 迁移脚本成功运行

---

### Day 5: CSRF保护

**任务**:
```typescript
// 1. 创建 csrf.ts 模块
// 位置: frontend/src/api/csrf.ts

const csrfToken = ref<string>('')

export async function initCSRFToken() {
    const response = await fetch('/api/csrf-token', {
        method: 'GET',
        credentials: 'include'
    })
    const data = await response.json()
    csrfToken.value = data.token
}

export async function apiPost<T>(url: string, body: any): Promise<T> {
    return fetch(url, {
        method: 'POST',
        headers: {
            'X-CSRF-Token': getCSRFToken()
        },
        body: JSON.stringify(body)
    })
}
```

**验证**:
- [ ] CSRF Token生成正常
- [ ] Token验证工作正常
- [ ] 所有POST请求都包含Token

---

## 📊 里程碑检查点

### Checkpoint 1: Week 2结束
```
✅ 认证系统加固完成
✅ 权限检查框架完成
✅ CSRF保护完成
✅ Phase 1单元测试通过
✅ 消除所有Critical漏洞
```

### Checkpoint 2: Week 4结束
```
✅ 财务并发控制完成
✅ 输入验证系统完成
✅ 前端XSS防护完成
✅ Phase 2集成测试通过
✅ 消除所有High漏洞
```

### Checkpoint 3: Week 6结束
```
✅ 安全日志系统完成
✅ 异常检测完成
✅ 渗透测试通过
✅ 最终安全审计通过
✅ 消除所有漏洞
```

---

## 👥 团队分工

### 后端开发 (2人)
- Week 1-2: 认证、权限、数据库
- Week 3-4: 财务并发、输入验证
- Week 5-6: 日志、异常检测

### 前端开发 (1人)
- Week 1-2: CSRF保护
- Week 3-4: XSS防护、CSP
- Week 5-6: 文档、培训

### QA/测试 (1人)
- Week 1-6: 持续测试
- Week 5-6: 渗透测试

### 技术主管 (1人)
- Week 1-6: 架构审查、进度跟踪

---

## 📈 成功指标

| 指标 | 目标 | Week 2 | Week 4 | Week 6 |
|------|------|--------|--------|--------|
| Critical漏洞 | 0 | 0 | 0 | 0 |
| High漏洞 | 0 | 0 | 0 | 0 |
| Medium漏洞 | 0 | 5 | 0 | 0 |
| 代码覆盖率 | >80% | 60% | 75% | 85% |
| 测试通过率 | 100% | 95% | 98% | 100% |

---

**开始日期**: 2026-03-31  
**预计完成**: 2026-05-12  
**总耗时**: 6周
