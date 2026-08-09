/**
 * API 请求核心函数
 * 所有 API client 使用此函数发送请求
 */
import { useAuthStore } from '@/stores/auth'

const getToken = () => {
  const raw = sessionStorage.getItem('adm_s')
  if (raw) {
    try {
      return decodeURIComponent(atob(raw))
    } catch {
      return ''
    }
  }
  return localStorage.getItem('admin_token') || ''
}

const getSessionContextHeaders = () => {
  return {}
}

let csrfToken = ''
const refreshCsrf = async () => {
  try {
    const res = await fetch('/api/csrf-token', {
      headers: { 'Authorization': `Bearer ${getToken()}`, ...getSessionContextHeaders() }
    })
    const data = await res.json()
    if (data.status === 'ok') csrfToken = data.token
  } catch (e) {
    console.error('Failed to refresh CSRF token', e)
  }
}

export const request = async (method: string, url: string, body?: any) => {
  const headers: Record<string, string> = { 'Content-Type': 'application/json', ...getSessionContextHeaders() }
  const token = getToken()
  if (token) headers['Authorization'] = `Bearer ${token}`

  if (['POST', 'DELETE', 'PATCH', 'PUT'].includes(method.toUpperCase()) && !csrfToken) {
    await refreshCsrf()
  }
  if (csrfToken) headers['X-CSRF-Token'] = csrfToken

  const isExecCall = url.includes('/exec') || url.includes('/chat/completions')
  const controller = new AbortController()
  // LLM 后端超时 120s（litellm.rs），前端 exec 给 90s 缓冲，避免 35s 错配导致正常慢响应被误杀
  const timer = setTimeout(() => controller.abort(), isExecCall ? 90000 : 15000)

  try {
    const res = await fetch(url, {
      method,
      headers,
      body: body ? JSON.stringify(body) : undefined,
      keepalive: true,
      signal: controller.signal,
    })
    clearTimeout(timer)

    if (res.status === 403) {
      await refreshCsrf()
      headers['X-CSRF-Token'] = csrfToken
      const retryRes = await fetch(url, {
        method,
        headers,
        body: body ? JSON.stringify(body) : undefined,
        keepalive: true,
        signal: AbortSignal.timeout(isExecCall ? 90000 : 15000),
      })
      return retryRes.json()
    }

    return res.json()
  } catch (e: any) {
    if (e.name === 'AbortError') {
      throw new Error(isExecCall ? 'LLM 调用超时 (>90s)，可能是 API Key 无效或模型不可达' : '请求超时请检查网络连接')
    }
    throw e
  }
}