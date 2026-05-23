# TGBot SaaS Rust Backend

独立 SaaS 后端，面向 Supabase/Postgres。Python `main.py -> web_admin.py` 继续承载 Telegram Bot、旧 Web Admin 和非 SaaS 业务；SaaS 账户、租户、商户、订阅、支付订单等数据迁移到本服务。

## 运行前置

- Rust toolchain: `cargo`、`rustc`
- Supabase 项目或兼容 PostgreSQL
- 已执行 `migrations/0001_saas_core.sql`

## 环境变量

```bash
SAAS_BIND_ADDR=127.0.0.1:8090
SUPABASE_DATABASE_URL=postgresql://postgres:password@db.xxx.supabase.co:5432/postgres
SUPABASE_JWT_SECRET=...
INTERNAL_SAAS_SECRET=change-me
```

## 隔离原则

- 商户请求的 `tenant_id` 不从 query/body 读取，只从 Supabase JWT 对应的 `tenant_memberships` 推导。
- 平台管理员由 `saas_user_roles` 表授权。
- 所有商户 API 必须先解析出 `tenant_id + merchant_id`，repository 查询必须带租户条件。
- 平台 API 可以跨租户查询，但只允许 `platform_admin`。
- 过渡期可以通过 `INTERNAL_SAAS_SECRET` 接收 Python 兼容代理转发的可信 session headers。

## 启动

```bash
cd saas-rust
cargo run
```

健康检查：

```bash
curl http://127.0.0.1:8090/api/health
```
