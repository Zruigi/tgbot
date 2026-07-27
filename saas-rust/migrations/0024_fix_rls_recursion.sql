BEGIN;

-- Supabase provisions authenticated/anon outside application migrations. A vanilla
-- PostgreSQL test environment must do the same before running this file. Keeping
-- role lifecycle out of the migration lets it run under a NOCREATEROLE deploy role.

-- Create SECURITY DEFINER helper function to break the RLS recursion on saas_user_roles
-- The deploy role must own auth.saas_user_roles (or have BYPASSRLS); the helper
-- then reads it with row_security OFF without granting callers direct table access.
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

REVOKE ALL ON FUNCTION auth.is_platform_admin(uuid) FROM PUBLIC;

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

REVOKE ALL ON FUNCTION auth.current_user_is_platform_admin() FROM PUBLIC;
GRANT EXECUTE ON FUNCTION auth.current_user_is_platform_admin() TO authenticated;

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
