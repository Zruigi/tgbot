# TGBot Pro - Rust 重构版本

## 状态: ✅ Phase 1-4 完成

```
架构: FastAPI + PostgreSQL + Redis + LiteLLM + PaddleOCR
分支: rust-refactor
```

## 技术栈

| 层级 | 技术 |
|------|------|
| API框架 | FastAPI (Python 3.12 async) |
| 数据库 | PostgreSQL 16 + pgvector |
| 缓存/队列 | Redis 7 |
| AI网关 | LiteLLM (DeepSeek/Claude/Gemini) |
| OCR | PaddleOCR |
| Bot | python-telegram-bot 20+ |

## 核心功能

- ✅ **多商户隔离**: 行级安全(RLS) + 商户ID上下文
- ✅ **AI Agent**: 原生工具调用支持
- ✅ **广告治理**: 检测→美化→推送自动化
- ✅ **PaddleOCR**: 图片文字识别
- ✅ **高并发**: 异步IO + 连接池 + Redis缓存

## 快速部署

```bash
# 1. 克隆项目
git clone https://github.com/Zruigi/tgbot.git
cd tgbot

# 2. 切换到重构分支
git checkout rust-refactor

# 3. 启动基础设施
cd docker
docker-compose up -d

# 4. 配置环境变量
cp .env.example .env
# 编辑 .env 填入配置

# 5. 启动服务
# 后端API
cd ../backend
pip install -r requirements.txt
uvicorn app.main:app --host 0.0.0.0 --port 8080

# Bot (新开终端)
cd ../bot
python main.py
```

## 环境变量

```env
# 数据库
DATABASE_URL=postgresql+asyncpg://user:pass@localhost:5432/tgbot

# Redis
REDIS_URL=redis://localhost:6379

# LiteLLM
LITELLM_BASE_URL=http://localhost:4000/v1
LITELLM_API_KEY=sk-your-key

# PaddleOCR
PADDLEOCR_URL=http://localhost:8000

# Telegram Bot (多商户JSON)
BOT_TOKENS='{"merchant1": "token1"}'
```

## API端点

| 方法 | 路径 | 说明 |
|------|------|------|
| POST | /api/auth/login | 商户登录 |
| POST | /api/ai/chat | AI对话 |
| POST | /api/ai/detect-ad | 广告检测 |
| GET | /api/users | 用户列表 |
| GET | /api/groups | 群组列表 |
| GET | /health | 健康检查 |

## 目录结构

```
rust-refactor/
├── backend/
│   ├── app/
│   │   ├── api/         # 6个API路由
│   │   ├── agent/       # AI Agent引擎
│   │   ├── core/        # 配置
│   │   ├── models/      # 多商户模型
│   │   └── services/    # 业务服务
│   └── requirements.txt
├── bot/
│   ├── main.py          # Bot入口
│   └── handler.py       # 处理器
├── docker/
│   ├── docker-compose.yml
│   └── paddleocr/       # OCR服务
└── README.md
```

## 广告检测Workflow

```
群组消息 → 广告检测(AI评分≥40)
    ↓
[通过] → 正常处理
[广告] → 记录广告 → AI美化 → 推送到频道 → (可选)删除/禁言
```

## 开发说明

- 不修改原前端 (Vue2 + 原生JS)
- 新后端API与旧系统并存
- 通过Telegram Bot对接两个系统

## 许可证

MIT