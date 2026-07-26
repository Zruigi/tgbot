-- 0005_indexes.sql - 热点路径补充索引
-- 用于优化查询性能，应在所有迁移完成后执行

BEGIN;

-- =====================================================
-- 支付订单索引
-- =====================================================
CREATE INDEX IF NOT EXISTS idx_payment_orders_tenant ON payment_orders(tenant_id);
CREATE INDEX IF NOT EXISTS idx_payment_orders_status ON payment_orders(tenant_id, status);
CREATE INDEX IF NOT EXISTS idx_payment_orders_date ON payment_orders(tenant_id, created_at DESC);
CREATE INDEX IF NOT EXISTS idx_payment_orders_merchant ON payment_orders(merchant_id);

-- =====================================================
-- AI 使用量索引
-- =====================================================
CREATE INDEX IF NOT EXISTS idx_ai_usage_tenant ON ai_usage_logs(tenant_id);
CREATE INDEX IF NOT EXISTS idx_ai_usage_date ON ai_usage_logs(tenant_id, created_at DESC);

-- =====================================================
-- 广播任务结果索引
-- =====================================================
CREATE INDEX IF NOT EXISTS idx_broadcast_results_tenant ON broadcast.broadcast_results(tenant_id);
CREATE INDEX IF NOT EXISTS idx_broadcast_results_task ON broadcast.broadcast_results(push_task_id);
CREATE INDEX IF NOT EXISTS idx_broadcast_results_status ON broadcast.broadcast_results(status);

-- =====================================================
-- job_queue 索引（主查询路径）
-- =====================================================
-- idx_job_queue_poll 已在 0001_core 中通过 job_queue 表创建
-- 补充复合索引
CREATE INDEX IF NOT EXISTS idx_job_queue_tenant_type ON job_queue(tenant_id, job_type);
CREATE INDEX IF NOT EXISTS idx_job_queue_created ON job_queue(created_at DESC);

COMMIT;
