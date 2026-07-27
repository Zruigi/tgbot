BEGIN;

-- The application/database owner is intentionally not FORCEd through these
-- policies: background workers use that owner role. Authenticated SQL clients
-- are restricted to the tenant selected by the request transaction, while a
-- platform administrator may inspect all tenants.
ALTER TABLE groups.ad_text_fingerprints ENABLE ROW LEVEL SECURITY;
ALTER TABLE groups.ad_sender_profiles ENABLE ROW LEVEL SECURITY;
ALTER TABLE groups.ad_group_policies ENABLE ROW LEVEL SECURITY;
ALTER TABLE groups.ad_scan_jobs ENABLE ROW LEVEL SECURITY;
ALTER TABLE groups.ad_governance_clusters ENABLE ROW LEVEL SECURITY;
ALTER TABLE groups.ad_governance_cluster_messages ENABLE ROW LEVEL SECURITY;
ALTER TABLE groups.ad_governance_actions ENABLE ROW LEVEL SECURITY;
ALTER TABLE groups.ad_warning_dedup ENABLE ROW LEVEL SECURITY;
ALTER TABLE groups.ad_delete_snapshots ENABLE ROW LEVEL SECURITY;

DROP POLICY IF EXISTS ad_text_fingerprints_tenant ON groups.ad_text_fingerprints;
CREATE POLICY ad_text_fingerprints_tenant ON groups.ad_text_fingerprints
  FOR ALL
  USING (auth.current_user_is_platform_admin() OR tenant_id = current_setting('app.tenant_id', true))
  WITH CHECK (auth.current_user_is_platform_admin() OR tenant_id = current_setting('app.tenant_id', true));

DROP POLICY IF EXISTS ad_sender_profiles_tenant ON groups.ad_sender_profiles;
CREATE POLICY ad_sender_profiles_tenant ON groups.ad_sender_profiles
  FOR ALL
  USING (auth.current_user_is_platform_admin() OR tenant_id = current_setting('app.tenant_id', true))
  WITH CHECK (auth.current_user_is_platform_admin() OR tenant_id = current_setting('app.tenant_id', true));

DROP POLICY IF EXISTS ad_group_policies_tenant ON groups.ad_group_policies;
CREATE POLICY ad_group_policies_tenant ON groups.ad_group_policies
  FOR ALL
  USING (auth.current_user_is_platform_admin() OR tenant_id = current_setting('app.tenant_id', true))
  WITH CHECK (auth.current_user_is_platform_admin() OR tenant_id = current_setting('app.tenant_id', true));

DROP POLICY IF EXISTS ad_scan_jobs_tenant ON groups.ad_scan_jobs;
CREATE POLICY ad_scan_jobs_tenant ON groups.ad_scan_jobs
  FOR ALL
  USING (auth.current_user_is_platform_admin() OR tenant_id = current_setting('app.tenant_id', true))
  WITH CHECK (auth.current_user_is_platform_admin() OR tenant_id = current_setting('app.tenant_id', true));

DROP POLICY IF EXISTS ad_governance_clusters_tenant ON groups.ad_governance_clusters;
CREATE POLICY ad_governance_clusters_tenant ON groups.ad_governance_clusters
  FOR ALL
  USING (auth.current_user_is_platform_admin() OR tenant_id = current_setting('app.tenant_id', true))
  WITH CHECK (auth.current_user_is_platform_admin() OR tenant_id = current_setting('app.tenant_id', true));

DROP POLICY IF EXISTS ad_governance_cluster_messages_tenant ON groups.ad_governance_cluster_messages;
CREATE POLICY ad_governance_cluster_messages_tenant ON groups.ad_governance_cluster_messages
  FOR ALL
  USING (auth.current_user_is_platform_admin() OR tenant_id = current_setting('app.tenant_id', true))
  WITH CHECK (auth.current_user_is_platform_admin() OR tenant_id = current_setting('app.tenant_id', true));

DROP POLICY IF EXISTS ad_governance_actions_tenant ON groups.ad_governance_actions;
CREATE POLICY ad_governance_actions_tenant ON groups.ad_governance_actions
  FOR ALL
  USING (auth.current_user_is_platform_admin() OR tenant_id = current_setting('app.tenant_id', true))
  WITH CHECK (auth.current_user_is_platform_admin() OR tenant_id = current_setting('app.tenant_id', true));

DROP POLICY IF EXISTS ad_warning_dedup_tenant ON groups.ad_warning_dedup;
CREATE POLICY ad_warning_dedup_tenant ON groups.ad_warning_dedup
  FOR ALL
  USING (auth.current_user_is_platform_admin() OR tenant_id = current_setting('app.tenant_id', true))
  WITH CHECK (auth.current_user_is_platform_admin() OR tenant_id = current_setting('app.tenant_id', true));

DROP POLICY IF EXISTS ad_delete_snapshots_tenant ON groups.ad_delete_snapshots;
CREATE POLICY ad_delete_snapshots_tenant ON groups.ad_delete_snapshots
  FOR ALL
  USING (auth.current_user_is_platform_admin() OR tenant_id = current_setting('app.tenant_id', true))
  WITH CHECK (auth.current_user_is_platform_admin() OR tenant_id = current_setting('app.tenant_id', true));

COMMIT;
