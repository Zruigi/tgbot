# 链上财务管理系统 - 本地部署指南 (Step 9)

## 🚀 本地开发环境部署

### 环境要求

- Python 3.9+
- Node.js 18+
- SQLite 3
- 网络连接（用于 Telegram API）

---

## 📋 部署步骤

### 1️⃣ 后端部署

#### 1.1 环境配置

```bash
cd /home/ubuntu/tgbot

# 激活虚拟环境
source venv/bin/activate

# 验证依赖
pip list | grep -E "aiohttp|python-telegram-bot|aiosqlite"
```

#### 1.2 数据库初始化

```bash
# 运行初始化脚本
python3 << 'PYEOF'
import asyncio
from finance_core import init_finance_db, init_default_chart_of_accounts

async def init():
    await init_finance_db()
    await init_default_chart_of_accounts()
    print("✅ 财务数据库初始化完成")

asyncio.run(init())
PYEOF
```

#### 1.3 启动后端服务

```bash
# 方式 1：直接运行
python main.py

# 方式 2：后台运行（使用 nohup）
nohup python main.py > backend.log 2>&1 &

# 方式 3：使用 screen（推荐）
screen -S tgbot-backend
python main.py
# 按 Ctrl+A 然后 D 分离 screen
```

**预期输出**：
```
✅ 财务模块数据库初始化完成
✅ 财务 API 路由已注册（v1 + 兼容旧路由）
Bot 正在启动...
```

**验证后端**：
```bash
# 检查 8080 端口
lsof -i :8080

# 测试 API
curl -H "Authorization: Bearer YOUR_TOKEN" \
  http://localhost:8080/api/v1/monitors/finance/monitors
```

---

### 2️⃣ 前端部署

#### 2.1 环境配置

```bash
cd /home/ubuntu/tgbot/frontend

# 验证 Node.js
node --version  # v18+
npm --version   # 9+

# 安装依赖
npm install

# 验证依赖
npm list vue vite typescript
```

#### 2.2 开发服务器启动

```bash
# 方式 1：直接运行
npm run dev

# 方式 2：后台运行
nohup npm run dev > frontend.log 2>&1 &

# 方式 3：使用 screen（推荐）
screen -S tgbot-frontend
npm run dev
# 按 Ctrl+A 然后 D 分离 screen
```

**预期输出**：
```
  VITE v5.4.21  ready in 123 ms

  ➜  Local:   http://localhost:5173/
  ➜  press h to show help
```

**验证前端**：
```bash
# 检查 5173 端口
lsof -i :5173

# 访问前端
curl http://localhost:5173
```

---

### 3️⃣ 系统验证

#### 3.1 后端 API 验证

```bash
# 获取监听列表
curl -H "Authorization: Bearer YOUR_TOKEN" \
  http://localhost:8080/api/v1/monitors/finance/monitors

# 获取统计数据
curl -H "Authorization: Bearer YOUR_TOKEN" \
  http://localhost:8080/api/v1/monitors/finance/summary

# 获取会计科目
curl -H "Authorization: Bearer YOUR_TOKEN" \
  http://localhost:8080/api/v1/monitors/finance/account_categories
```

#### 3.2 前端访问验证

```bash
# 打开浏览器访问
http://localhost:5173

# 预期：
# 1. 显示登录页面
# 2. 输入管理员 Token 登录
# 3. 点击侧边栏"链上财务"进入系统
# 4. 显示 4 个 Tab（链上监听、对账工作台、财务报表、审计日志）
```

#### 3.3 集成测试

```bash
cd /home/ubuntu/tgbot
source venv/bin/activate

# 运行完整功能测试
python test_onchain_finance_complete.py

# 预期输出：所有测试通过
```

---

## 🔧 常见问题排查

### 问题 1：后端启动失败

```
ModuleNotFoundError: No module named 'finance_core'
```

**解决**：
```bash
# 确保在正确目录
cd /home/ubuntu/tgbot

# 检查文件是否存在
ls -la finance_core.py finance_api.py

# 重新激活虚拟环境
source venv/bin/activate
```

### 问题 2：前端构建失败

```
error during build
```

**解决**：
```bash
cd /home/ubuntu/tgbot/frontend

# 清理缓存
rm -rf node_modules/.vite
rm -rf dist

# 重新构建
npm run build
```

### 问题 3：无法连接到后端

```
Failed to fetch /api/v1/monitors/finance/monitors
```

**解决**：
```bash
# 检查后端是否运行
lsof -i :8080

# 检查防火墙
sudo ufw status

# 测试连接
curl http://localhost:8080/api/v1/monitors/finance/monitors
```

### 问题 4：数据库锁定

```
database is locked
```

**解决**：
```bash
# 关闭所有 Python 进程
pkill -f "python main.py"

# 等待 2 秒
sleep 2

# 重启后端
python main.py
```

### 问题 5：Token 认证失败

```
401 Unauthorized
```

**解决**：
```bash
# 检查 Token 是否正确
echo $WEB_ADMIN_TOKEN

# 检查 .env 文件
cat .env | grep WEB_ADMIN_TOKEN

# 重新设置 Token
export WEB_ADMIN_TOKEN="your_token_here"
```

---

## 📊 系统监控

### 后端日志

```bash
# 实时查看后端日志
tail -f backend.log

# 查看最后 100 行
tail -100 backend.log

# 搜索错误
grep "ERROR" backend.log
```

### 前端日志

```bash
# 实时查看前端日志
tail -f frontend.log

# 查看最后 100 行
tail -100 frontend.log

# 搜索错误
grep "error" frontend.log
```

### 数据库检查

```bash
# 进入 SQLite
sqlite3 /home/ubuntu/tgbot/tgbot.db

# 查看表
.tables

# 查看表结构
.schema address_monitors

# 查询数据
SELECT COUNT(*) FROM address_monitors;

# 退出
.quit
```

---

## 🔐 本地开发安全建议

1. **Token 管理**
   - 不要在代码中硬编码 Token
   - 使用 `.env` 文件管理敏感信息
   - 定期更换 Token

2. **数据库备份**
   - 定期备份 `tgbot.db`
   - 保存备份到安全位置

3. **日志管理**
   - 定期清理日志文件
   - 监控异常日志

4. **网络安全**
   - 仅在本地网络使用
   - 不要暴露到公网

---

## 📈 性能监控

### CPU 使用率

```bash
# 监控 Python 进程
top -p $(pgrep -f "python main.py")

# 监控 Node.js 进程
top -p $(pgrep -f "npm run dev")
```

### 内存使用率

```bash
# 查看内存使用
ps aux | grep -E "python|node"

# 详细内存信息
free -h
```

### 网络连接

```bash
# 查看网络连接
netstat -tuln | grep -E "5173|8080"

# 实时网络监控
iftop
```

---

## ✅ 部署检查清单

- [ ] Python 虚拟环境激活
- [ ] 数据库初始化完成
- [ ] 后端服务启动成功
- [ ] 前端服务启动成功
- [ ] 后端 API 可访问
- [ ] 前端页面可访问
- [ ] 登录功能正常
- [ ] 链上财务模块可进入
- [ ] 所有 4 个 Tab 可切换
- [ ] 集成测试全部通过

---

## 🎉 部署完成

系统已就绪！

**访问地址**：`http://localhost:5173`
**后端 API**：`http://localhost:8080`
**数据库**：`/home/ubuntu/tgbot/tgbot.db`

**下一步**：
1. 登录系统
2. 进入"链上财务"模块
3. 开始使用各项功能

