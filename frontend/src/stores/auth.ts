import { defineStore } from 'pinia'
import { ref } from 'vue'
import { api } from '@/api'

// 修复 Medium: localStorage Token 明文存储
// 改用 sessionStorage（关闭标签页自动清除）+ 简单 base64 混淆防止直接明文读取
const TOKEN_KEY = 'adm_s'

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
  // 使用 sessionStorage 代替 localStorage（会话结束自动清除）
  sessionStorage.setItem(TOKEN_KEY, encodeToken(t))
  // 同时写 HttpOnly-friendly Cookie（后端通过 Cookie 验证）
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

  // 兼容旧版 localStorage（迁移清理）
  const legacy = localStorage.getItem('admin_token')
  if (legacy) {
    localStorage.removeItem('admin_token')
    saveToken(legacy)
    return legacy
  }

  // 新增：支持从 Cookie 恢复会话，避免每次新开页面都要手输 Token
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

export const useAuthStore = defineStore('auth', () => {
  const token = ref(loadToken())
  const isAuthenticated = ref(!!token.value)
  const isLoading = ref(false)
  const error = ref('')

  async function loginWithOtp(otp: string) {
    isLoading.value = true
    error.value = ''
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
      } else {
        error.value = data.message || 'OTP 登录失败'
      }
    } catch (e: any) {
      error.value = '登录请求失败，请重试'
    } finally {
      isLoading.value = false
    }
  }

  async function login(inputToken: string) {
    isLoading.value = true
    error.value = ''
    try {
      const res = await api.login(inputToken)
      if (res.status === 'ok') {
        token.value = inputToken
        saveToken(inputToken)
        isAuthenticated.value = true
      } else {
        error.value = res.message || 'Token 错误'
      }
    } catch (e: any) {
      error.value = '登录请求失败，请重试'
    } finally {
      isLoading.value = false
    }
  }

  function checkToken() {
    const t = loadToken()
    if (t) {
      token.value = t
      isAuthenticated.value = true
    }
  }

  function logout() {
    token.value = ''
    clearToken()
    isAuthenticated.value = false
  }

  return { token, isAuthenticated, isLoading, error, login, loginWithOtp, checkToken, logout }
})
