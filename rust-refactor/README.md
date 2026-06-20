# TGBot Pro - Rust 重构版本

## 架构概述

```
┌─────────────────────────────────────────────────────────────┐
│                    TGBot Pro 2.0 架构                        │
├─────────────────────────────────────────────────────────────┤
│  前端 (Vue3)  │  Telegram Bot  │  Admin API (FastAPI)       │
├─────────────────────────────────────────────────────────────┤
│                    AI Agent Orchestrator                    │
│         (工具调用: 广告检测/删除/美化/推送)                  │
├─────────────────────────────────────────────────────────────┤
│  PostgreSQL (行级安全)  │  Redis (缓存/队列)  │  LiteLLM   │
└─────────────────────────────────────────────────────────────┘
```

## 技术栈

| 层级 | 技术 |
|------|------|
| API框架 | FastAPI (Python 3.12 async) |
| 数据库 | PostgreSQL + Redis |
| AI网关 | LiteLLM (DeepSeek/Claude/Gemini) |
| OCR | PaddleOCR |
| Bot | python-telegram-bot 20+ |

## 核心特性

1. **多商户隔离** - 行级安全(RLS) + 商户ID上下文
2. **AI原生** - Agent作为核心调度，支持工具调用
3. **广告治理** - AI检测→美化→推送自动化
4. **高并发** - 异步IO + 连接池 + Redis缓存

## 快速开始

```bash
# 1. 启动基础设施
cd docker
docker-compose up -d

# 2. 安装依赖
cd backend
pip install -r requirements.txt

# 3. 运行API服务
uvicorn app.main:app --reload --port 8080
```

## 目录结构

```
rust-refactor/
├── backend/
│   ├── app/
│   │   ├── api/         # API路由
│   │   ├── core/        # 配置
│   │   ├── models/      # 数据模型
│   │   ├── agent/       # AI Agent
│   │   └── security/    # 安全
│   └── requirements.txt
├── bot/                  # Telegram Bot
├── frontend/             # Vue3 前端
├── docker/               # Docker配置
└── docs/                 # 文档
```

## API端点

- `POST /api/auth/login` - 商户登录
- `POST /api/ai/chat` - AI对话
- `POST /api/ai/detect-ad` - 广告检测
- `GET /api/users` - 用户列表
- `GET /api/groups` - 群组列表
- `GET /health` - 健康检查