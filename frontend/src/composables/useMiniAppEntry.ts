import { useAuthStore } from '@/stores/auth'
import { routeToTab } from '@/app/navigation/app-tabs'
import { getTelegramInitData, tgHaptic } from '@/lib/telegram'

export function useMiniAppEntry(navigateTo: (tab: string) => Promise<void> | void) {
  const auth = useAuthStore()

  function applyLanding(route: string) {
    void navigateTo(routeToTab(route))
    if (typeof window !== 'undefined') {
      window.history.replaceState({}, '', route || '/saas')
    }
  }

  async function bootstrapMiniAppEntry() {
    const params = new URLSearchParams(location.search)
    const otp = params.get('otp')
    const grant = params.get('grant')
    const entry = params.get('entry') || 'menu'
    const isMiniAppLaunch = location.pathname === '/miniapp'
    const initData = getTelegramInitData()

    const shouldBootstrap = !!initData || !!otp || !!grant || isMiniAppLaunch
    if (!shouldBootstrap) return { attempted: false, success: false }

    const data = await auth.bootstrapMiniApp({
      init_data: initData,
      otp,
      grant,
      entry_intent: entry,
      client_context: {
        pathname: location.pathname,
        search: location.search,
        preview: 'normal',
        user_agent: navigator.userAgent,
        has_proof_cookie: document.cookie.includes('entry_proof='),
      },
    })

    if (data?.status === 'ok' && data?.landing?.route) {
      applyLanding(data.landing.route)
      tgHaptic.notification('success')
      return { attempted: true, success: true }
    }

    if (data?.status === 'ok' && data?.session?.role === 'unregistered') {
      applyLanding('/saas')
      return { attempted: true, success: true }
    }

    return { attempted: true, success: false }
  }

  return {
    applyLanding,
    bootstrapMiniAppEntry,
  }
}
