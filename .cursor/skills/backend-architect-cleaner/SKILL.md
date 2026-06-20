---
name: backend-architect-cleaner
description: Split oversized Python backend files into strict 3-tier architecture (router/service/core) with zero behavior changes and mandatory code cleanup. Use when users ask to refactor monolithic backend code, modularize aiohttp/FastAPI handlers, remove dead code, or enforce layered boundaries.
argument-hint: "[target-file-or-module]"
---

# 🏗️ 后端架构师与代码净化协议 (Backend Architect & Cleaner)

## 🎯 核心使命
将超大 Python 单体文件（如 4000 行）无损拆分为大厂级三层架构：
- **不改变业务行为**
- **不引入跨层耦合**
- **执行强制净化**（死代码、废弃导入、历史注释逻辑）

---

## 📏 三层架构红线（必须遵守）

### 1) Router 层（`app/api/`）
只允许：
- 定义 HTTP 路由
- 接收参数与基础校验
- 调用 Service
- 返回 JSON/HTTP 响应

严禁：
- 写 SQL
- 写核心业务逻辑
- 直接操作底层连接细节

### 2) Service 层（`app/services/`）
只负责：
- 业务规则编排
- 事务流程与领域逻辑
- 调用 Core 能力与数据访问能力

### 3) Core 层（`app/core/`）
只负责：
- 配置管理
- 数据库连接
- 鉴权/加密
- 通用基础设施能力

---

## 🚫 强约束规则（CRITICAL）
1. **500 行熔断**：任何新建 Python 文件接近 500 行，必须停止继续堆积并先设计子模块。
2. **抽离必删除（Extract & Destroy）**：函数搬运到新模块后，旧文件中的同逻辑必须立即删除，禁止双份并存。
3. **清理死代码（Clean Dead Code）**：自动清理未使用 `import`、无用临时变量、注释掉的历史逻辑。
4. **路由挂载规范**：
   - `aiohttp` 项目：使用 `RouteTableDef` 并统一注册
   - FastAPI 项目：使用 `APIRouter` 并统一 include

---

## 🧭 标准执行流程

### 阶段 0：拆分前审计
- 识别入口文件、路由定义、核心流程、数据库调用点
- 建立“函数归属清单”：每个函数必须明确归属 `api/service/core`
- 标记高风险函数（副作用强、事务逻辑、鉴权逻辑）

### 阶段 1：先建骨架再搬运
目标目录建议：

```text
app/
  api/
    __init__.py
    <module>_routes.py
  services/
    __init__.py
    <module>_service.py
  core/
    __init__.py
    config.py
    db.py
    auth.py
```

要求：先建立最小可运行骨架与导入关系，再逐函数搬运。

### 阶段 2：按层搬运（单向依赖）
- `api -> services -> core`
- 禁止 `core` 反向依赖 `service/api`
- 每次搬运一小组函数后立即验证（导入、调用链、行为）

### 阶段 3：搬运即删除
- 新模块落地后，立刻删除旧文件对应实现
- 若旧位置必须临时保留，仅允许短期转发壳，并在本次任务结束前移除

### 阶段 4：净化与收口
- 删除无用导入、未使用变量、无效注释代码
- 统一异常处理和响应结构（不改变原有语义）
- 检查循环依赖、重复逻辑和隐藏副作用

### 阶段 5：路由统一挂载
- `aiohttp`: 子路由统一 `app.add_routes(...)`
- FastAPI: 统一 `app.include_router(...)`
- 确保入口文件仅做装配，不承载业务

---

## ✅ 完成定义（Definition of Done）
- [ ] 路由层无 SQL 与业务细节
- [ ] 业务逻辑集中在 Service
- [ ] Core 仅保留基础能力
- [ ] 所有新文件 < 500 行，必要时已二次拆分
- [ ] 已执行“搬运即删除”，旧文件无重复实现
- [ ] 已清理死代码（import/变量/注释历史逻辑）
- [ ] 子路由已统一挂载且可运行
- [ ] 功能行为与拆分前一致（接口、返回、副作用）

---

## 🔍 审查输出模板（每次任务结束必须给出）

```markdown
## 1) 拆分结果
- 新增模块：
- 迁移函数：
- 删除旧实现：

## 2) 架构边界检查
- Router 层违规项：
- Service 层违规项：
- Core 层违规项：

## 3) 代码净化清单
- 删除未使用 import：
- 删除死变量/死函数：
- 删除注释历史代码：

## 4) 风险与验证
- 风险点：
- 已完成验证：
- 待补验证：
```

---

## 🧱 实施策略（默认风格）
- 小步提交，逐段可回滚
- 先保行为，再做结构美化
- 不做无关“顺手重构”
- 发现边界不清时，先画调用图再改代码
