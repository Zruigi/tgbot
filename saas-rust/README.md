# TGBot SaaS Rust Backend

独立 SaaS 后端，面向 Supabase/Postgres。Python `main.py -> web_admin.py` 继续承载 Telegram Bot、旧 Web Admin 和非 SaaS 业务；SaaS 账户、租户、商户、订阅、支付订单等数据迁移到本服务。

## 运行前置

- Rust toolchain: `cargo`、`rustc`
- Supabase 项目或兼容 PostgreSQL
- 已按 `scripts/beta-migrations.list` 顺序执行全部迁移

## 环境变量

```bash
SAAS_BIND_ADDR=127.0.0.1:18090
SUPABASE_DATABASE_URL=postgresql://postgres:password@db.xxx.supabase.co:5432/postgres
SUPABASE_JWT_SECRET=...
INTERNAL_SAAS_SECRET=change-me
SAAS_SESSION_SECRET=<at-least-32-byte-random-value>
TGBOT_SECRET_ENCRYPTION_KEY=<32-byte-hex-or-base64-key>
# Optional local diagnostic mode only; default is false.
SAAS_ALLOW_REST_ONLY=false
```

## 隔离原则

- 商户请求的 `tenant_id` 不从 query/body 读取，只从 Supabase JWT 对应的 `tenant_memberships` 推导。
- 平台管理员由 `saas_user_roles` 表授权。
- 所有商户 API 必须先解析出 `tenant_id + merchant_id`，repository 查询必须带租户条件。
- 平台 API 可以跨租户查询，但只允许 `platform_admin`。
- 过渡期可以通过 `INTERNAL_SAAS_SECRET` 接收 Python 兼容代理转发的可信 session headers。
- `SAAS_SESSION_SECRET` 必须是至少 32 字节的部署专用随机值；不能使用代码中的固定默认值。
- `TGBOT_SECRET_ENCRYPTION_KEY` 必须提供，用于解密租户 Telegram Bot Token；密钥只注入受控 secret store，不写入仓库或命令行。

Beta 隔离工作区默认使用 `127.0.0.1:18090`；生产监听地址必须在获准的部署配置中显式设置，不能复用 Beta 默认值。

## 启动

```bash
cd saas-rust
cargo run -p tgbot-app
```

健康检查：

```bash
curl http://127.0.0.1:18090/api/health
# Readiness requires the PostgreSQL pool used by the MVP runtime.
curl http://127.0.0.1:18090/api/ready
```

`/api/health` is a liveness check. `/api/ready` returns success only after
PostgreSQL is connected. The service refuses to start when PostgreSQL is
unavailable unless `SAAS_ALLOW_REST_ONLY=true` is explicitly set for local
diagnostics; REST-only mode is not an MVP deployment mode.
