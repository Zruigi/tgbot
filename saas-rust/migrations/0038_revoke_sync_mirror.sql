-- 0038_revoke_sync_mirror.sql
-- (2026-08-14): 校对群双模式 —— 镜像保留开关。
-- revoke_sync_mirror=true 时 revoke_sync_worker 转发到校对群后【保留副本】（供人工核验）；
-- false（默认）保持 Python 探测式行为：转发成功即删副本、标记 revoke_checked。
-- 撤回同步删除（转发失败 → 删 DB 记录）在两种模式下行为一致。

BEGIN;

ALTER TABLE groups.tracked_groups
  ADD COLUMN IF NOT EXISTS revoke_sync_mirror boolean NOT NULL DEFAULT false;

COMMIT;
