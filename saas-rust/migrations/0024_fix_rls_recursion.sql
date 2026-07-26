BEGIN;

-- 兼容 shim:原生 PG 无 Supabase 的 authenticated/anon 角色,补齐以保证本文件
-- 在 vanilla PG(beta)与真 Supabase 上均可幂等执行
DO $do$ BEGIN
  IF NOT EXISTS (SELECT FROM pg_roles WHERE rolname = 'authenticated') THEN
    CREATE ROLE authenticated NOLOGIN;
  END IF;
  IF NOT EXISTS (SELECT FROM pg_roles WHERE rolname = 'anon') THEN
    CREATE ROLE anon NOLOGIN;
  END IF;
END $do$;

-- Create SECURITY DEFINER helper function to break the RLS recursion on saas_user_roles
-- This function is owned by postgres and reads saas_user_roles with row_security OFF
CREATE OR REPLACE FUNCTION auth.is_platform_admin(user_id_param uuid)
RETURNS boolean
LANGUAGE sql
SECURITY DEFINER
SET row_security = OFF
SET search_path = auth, public, pg_temp
AS $$
  SELECT EXISTS (
    SELECT 1
    FROM auth.saas_user_roles r
    WHERE r.user_id = user_id_param
      AND r.role = 'platform_admin'::text
  );
$$;

GRANT EXECUTE ON FUNCTION auth.is_platform_admin(uuid) TO PUBLIC;

-- Create SECURITY DEFINER helper function to check if user is platform admin (for current user)
CREATE OR REPLACE FUNCTION auth.current_user_is_platform_admin()
RETURNS boolean
LANGUAGE sql
SECURITY DEFINER
SET row_security = OFF
SET search_path = auth, public, pg_temp
AS $$
  SELECT auth.is_platform_admin(auth.uid());
$$;

GRANT EXECUTE ON FUNCTION auth.current_user_is_platform_admin() TO PUBLIC;

-- Drop and recreate the saas_user_roles policy to use the helper function instead of direct subquery
-- This breaks the RLS recursion cycle
DROP POLICY IF EXISTS saas_user_roles_select ON auth.saas_user_roles;

CREATE POLICY saas_user_roles_select ON auth.saas_user_roles
  FOR SELECT
  USING (
    auth.current_user_is_platform_admin() OR user_id = auth.uid()
  );

-- Recreate merchants policy to use the helper function as well (for consistency)
DROP POLICY IF EXISTS merchants_platform_or_member ON public.merchants;

CREATE POLICY merchants_platform_or_member ON public.merchants
  FOR SELECT
  USING (
    auth.current_user_is_platform_admin() OR EXISTS (
      SELECT 1
      FROM public.tenant_memberships m
      WHERE m.user_id = auth.uid()
        AND m.tenant_id = merchants.tenant_id
    )
  );

COMMIT;
