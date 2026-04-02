import { defineStore } from 'pinia'
import { ref } from 'vue'

export const useAuthStore = defineStore('auth', () => {
  const token = ref(localStorage.getItem('admin_token') || '')
  const isAuthenticated = ref(!!token.value)
  const isLoading = ref(false)
  const error = ref('')

  async function login(inputToken: string) {
    isLoading.value = true
    error.value = ''
    try {
      const res = await fetch('/api/login', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ token: inputToken })
      })
      const data = await res.json()
      if (data.status === 'ok') {
        token.value = inputToken
        localStorage.setItem('admin_token', inputToken)
        document.cookie = `admin_token=${inputToken};max-age=1800;path=/;SameSite=Strict`
        isAuthenticated.value = true
      } else {
        error.value = data.message || 'Token 错误'
      }
    } catch (e: any) {
      error.value = e.message || '登录失败'
    } finally {
      isLoading.value = false
    }
  }

  async function loginWithOtp(otp: string) {
    isLoading.value = true
    error.value = ''
    try {
      const res = await fetch('/api/login/otp', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ otp })
      })
      const data = await res.json()
      if (data.status === 'ok' && data.token) {
        token.value = data.token
        localStorage.setItem('admin_token', data.token)
        document.cookie = `admin_token=${data.token};max-age=1800;path=/;SameSite=Strict`
        isAuthenticated.value = true
      } else {
        error.value = data.message || 'OTP 登录失败'
      }
    } catch (e: any) {
      error.value = e.message || 'OTP 登录失败'
    } finally {
      isLoading.value = false
    }
  }

  function checkToken() {
    const t = localStorage.getItem('admin_token')
    if (t) {
      token.value = t
      isAuthenticated.value = true
    }
  }

  function logout() {
    token.value = ''
    isAuthenticated.value = false
    localStorage.removeItem('admin_token')
    document.cookie = 'admin_token=;max-age=0;path=/'
  }

  return { token, isAuthenticated, isLoading, error, login, loginWithOtp, checkToken, logout }
})
