import { defineStore } from 'pinia'
import { ref } from 'vue'
import { api } from '@/api'

const TOKEN_KEY = 'adm_s'
const SESSION_CTX_KEY = 'adm_ctx'

function encodeToken(t: string): string {
  return btoa(encodeURIComponent(t))
}

function decodeToken(s: string): string {
  try {
    return decodeURIComponent(atob(s))
  } catch {
    return ''
  }
}

function saveToken(t: string) {
  sessionStorage.setItem(TOKEN_KEY, encodeToken(t))
  const secure = location.protocol === 'https:' ? ';Secure' : ''
  document.cookie = `admin_token=${encodeURIComponent(t)};max-age=1800;path=/;SameSite=Strict${secure}`
}

function readCookieToken(): string {
  const cookie = document.cookie
    .split('; ')
    .find((item) => item.startsWith('admin_token='))
  if (!cookie) return ''
  try {
    return decodeURIComponent(cookie.split('=').slice(1).join('='))
  } catch {
    return ''
  }
}

function loadToken(): string {
  const raw = sessionStorage.getItem(TOKEN_KEY)
  if (raw) return decodeToken(raw)

  const legacy = localStorage.getItem('admin_token')
  if (legacy) {
    localStorage.removeItem('admin_token')
    saveToken(legacy)
    return legacy
  }

  const cookieToken = readCookieToken()
  if (cookieToken) {
    saveToken(cookieToken)
    return cookieToken
  }

  return ''
}

function clearToken() {
  sessionStorage.removeItem(TOKEN_KEY)
  localStorage.removeItem('admin_token')
  document.cookie = 'admin_token=;max-age=0;path=/'
}

function loadSessionContext() {
  try {
    const raw = sessionStorage.getItem(SESSION_CTX_KEY)
    if (!raw) return { role: '', actorRole: '', tenantId: '', merchantId: null as number | null }
    const parsed = JSON.parse(raw)
    return {
      role: String(parsed.role || ''),
      actorRole: String(parsed.actorRole || parsed.role || ''),
      tenantId: String(parsed.tenantId || ''),
      merchantId: typeof parsed.merchantId === 'number' ? parsed.merchantId : null,
    }
  } catch {
    return { role: '', actorRole: '', tenantId: '', merchantId: null as number | null }
  }
}

function normalizeSessionContext(raw?: any) {
  return {
    role: String(raw?.role || ''),
    actorRole: String((raw?.actor_role ?? raw?.actorRole ?? raw?.role) || ''),
    tenantId: String(raw?.tenant_id ?? raw?.tenantId ?? ''),
    merchantId: typeof raw?.merchant_id === 'number'
      ? raw.merchant_id
      : (typeof raw?.merchantId === 'number' ? raw.merchantId : null),
  }
}

function saveSessionContext(ctx: { role: string; actorRole?: string; tenantId: string; merchantId: number | null }) {
  sessionStorage.setItem(SESSION_CTX_KEY, JSON.stringify({
    role: ctx.role,
    actorRole: ctx.actorRole || ctx.role || '',
    tenantId: ctx.tenantId,
    merchantId: ctx.merchantId,
  }))
}

function clearSessionContext() {
  sessionStorage.removeItem(SESSION_CTX_KEY)
}

export const useAuthStore = defineStore('auth', () => {
  const token = ref(loadToken())
  const isAuthenticated = ref(!!token.value)
  const isLoading = ref(false)
  const error = ref('')
  const sessionContext = ref(loadSessionContext())
  const landingRoute = ref('/saas')
  const landingMode = ref('normal')
  const uiContext = ref<Record<string, any>>({})
  const miniappStatus = ref('')
  const miniappMessage = ref('')

  async function loginWithOtp(otp: string) {
    isLoading.value = true
    error.value = ''
    miniappStatus.value = ''
    miniappMessage.value = ''
    try {
      const res = await fetch('/api/login/otp', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ otp }),
      })
      const data = await res.json()
      if (data.status === 'ok' && data.token) {
        token.value = data.token
        saveToken(data.token)
        isAuthenticated.value = true

        const otpSession = normalizeSessionContext(data.session)
        sessionContext.value = otpSession
        saveSessionContext(otpSession)

        try {
          const contextRes = await api.getSessionContext()
          if (contextRes?.status === 'ok' && contextRes.data) {
            const canonicalSession = normalizeSessionContext(contextRes.data)
            sessionContext.value = canonicalSession
            saveSessionContext(canonicalSession)
          }
        } catch {}
      } else {
        error.value = data.message || 'OTP 登录失败'
      }
    } catch {
      error.value = '登录请求失败，请重试'
    } finally {
      isLoading.value = false
    }
  }

  async function loginWithTma(initData: string) {
    /** Telegram Mini App initData 认证 */
    isLoading.value = true
    error.value = ''
    miniappStatus.value = ''
    miniappMessage.value = ''
    try {
      const res = await fetch('/api/login/tma', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ init_data: initData }),
      })
      const data = await res.json()
      if (data.status === 'ok' && data.token) {
        token.value = data.token
        saveToken(data.token)
        isAuthenticated.value = true

        const tmaSession = normalizeSessionContext(data.session)
        sessionContext.value = tmaSession
        saveSessionContext(tmaSession)

        try {
          const contextRes = await api.getSessionContext()
          if (contextRes?.status === 'ok' && contextRes.data) {
            const canonicalSession = normalizeSessionContext(contextRes.data)
            sessionContext.value = canonicalSession
            saveSessionContext(canonicalSession)
          }
        } catch {}
      } else if (data.status === 'ok' && data.session?.role === 'unregistered') {
        // 未注册用户，引导注册
        error.value = '您尚未注册，请先在 Bot 中发送 /merchant 注册'
      } else {
        error.value = data.message || 'TMA 认证失败'
      }
    } catch {
      error.value = 'TMA 认证请求失败，请重试'
    } finally {
      isLoading.value = false
    }
  }

  async function bootstrapMiniApp(payload: {
    init_data?: string | null
    otp?: string | null
    grant?: string | null
    entry_intent?: string
    client_context?: Record<string, any>
  }) {
    isLoading.value = true
    error.value = ''
    miniappStatus.value = ''
    miniappMessage.value = ''
    try {
      const res = await fetch('/api/miniapp/bootstrap', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify(payload),
      })
      const data = await res.json()

      landingRoute.value = String(data?.landing?.route || '/saas')
      landingMode.value = String(data?.landing?.mode || 'normal')
      uiContext.value = data?.ui_context && typeof data.ui_context === 'object' ? data.ui_context : {}
      miniappStatus.value = String(data?.status || '')
      miniappMessage.value = String(data?.message || '')

      if (data?.status === 'ok' && data?.token) {
        token.value = data.token
        saveToken(data.token)
        isAuthenticated.value = true
        const ctx = normalizeSessionContext(data.session)
        sessionContext.value = ctx
        saveSessionContext(ctx)

        try {
          const contextRes = await api.getSessionContext()
          if (contextRes?.status === 'ok' && contextRes.data) {
            const canonicalSession = normalizeSessionContext(contextRes.data)
            sessionContext.value = canonicalSession
            saveSessionContext(canonicalSession)
          }
        } catch {}
        return data
      }

      if (data?.status === 'ok' && data?.session?.role === 'unregistered') {
        sessionContext.value = normalizeSessionContext(data.session)
        isAuthenticated.value = false
        return data
      }

      isAuthenticated.value = false
      return data
    } catch (e: any) {
      error.value = e?.message || 'Mini App 启动失败'
      miniappStatus.value = 'error'
      miniappMessage.value = error.value
      isAuthenticated.value = false
      return null
    } finally {
      isLoading.value = false
    }
  }


  async function login(inputToken: string) {
    isLoading.value = true
    error.value = ''
    miniappStatus.value = ''
    miniappMessage.value = ''
    try {
      const res = await api.login(inputToken)
      if (res.status === 'ok' && res.token) {
        token.value = res.token
        saveToken(res.token)
        isAuthenticated.value = true
        sessionContext.value = normalizeSessionContext(res.session)
        saveSessionContext(sessionContext.value)
      } else {
        error.value = res.message || 'Token 错误'
      }
    } catch {
      error.value = '登录请求失败，请重试'
    } finally {
      isLoading.value = false
    }
  }

  async function checkToken() {
    miniappStatus.value = ''
    miniappMessage.value = ''
    const t = loadToken()
    if (t) {
      token.value = t
      isAuthenticated.value = true
      try {
        const res = await api.getSessionContext()
        if (res?.status === 'ok' && res.data) {
          sessionContext.value = normalizeSessionContext(res.data)
          saveSessionContext(sessionContext.value)
        }
      } catch {}
    }
    sessionContext.value = loadSessionContext()
  }

  async function setSessionContext(ctx: Partial<{ role: string; tenantId: string; merchantId: number | null }>) {
    const nextCtx = {
      role: ctx.role || sessionContext.value.role || '',
      tenantId: ctx.tenantId ?? sessionContext.value.tenantId,
      merchantId: ctx.merchantId ?? sessionContext.value.merchantId,
    }
    const res = await api.updateSessionContext({
      role: nextCtx.role,
      tenant_id: nextCtx.tenantId,
      merchant_id: nextCtx.merchantId,
    })
    if (res?.status === 'ok') {
      if (res.token) {
        token.value = res.token
        saveToken(res.token)
      }
      sessionContext.value = normalizeSessionContext(res.data || nextCtx)
      saveSessionContext(sessionContext.value)
    }
  }

  function logout() {
    token.value = ''
    clearToken()
    clearSessionContext()
    sessionContext.value = { role: '', actorRole: '', tenantId: '', merchantId: null }
    landingRoute.value = '/saas'
    landingMode.value = 'normal'
    uiContext.value = {}
    miniappStatus.value = ''
    miniappMessage.value = ''
    isAuthenticated.value = false
  }

  return {
    token,
    isAuthenticated,
    isLoading,
    error,
    sessionContext,
    landingRoute,
    landingMode,
    uiContext,
    miniappStatus,
    miniappMessage,
    login,
    loginWithOtp,
    loginWithTma,
    bootstrapMiniApp,
    checkToken,
    setSessionContext,
    logout,
  }
})
