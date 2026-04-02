const getToken = () => {
  const raw = sessionStorage.getItem('adm_s')
  if (raw) {
    try {
      return decodeURIComponent(atob(raw))
    } catch {
      return ''
    }
  }
  return ''
}

// CSRF Token 获取与存储
let csrfToken = ''
const refreshCsrf = async () => {
  try {
    const res = await fetch('/api/csrf-token', {
      headers: { 'Authorization': `Bearer ${getToken()}` }
    })
    const data = await res.json()
    if (data.status === 'ok') csrfToken = data.token
  } catch (e) {
    console.error('Failed to refresh CSRF token', e)
  }
}

const request = async (method: string, url: string, body?: any) => {
  const headers: Record<string, string> = { 'Content-Type': 'application/json' }
  const token = getToken()
  if (token) headers['Authorization'] = `Bearer ${token}`
  
  // 如果是状态变更请求且没有 CSRF token，尝试获取一次
  if (['POST', 'DELETE', 'PATCH', 'PUT'].includes(method.toUpperCase()) && !csrfToken) {
    await refreshCsrf()
  }
  if (csrfToken) headers['X-CSRF-Token'] = csrfToken

  const res = await fetch(url, {
    method,
    headers,
    body: body ? JSON.stringify(body) : undefined,
    keepalive: true
  })
  
  if (res.status === 403) {
    // CSRF 可能过期，尝试刷新一次并重试
    await refreshCsrf()
    headers['X-CSRF-Token'] = csrfToken
    const retryRes = await fetch(url, {
      method,
      headers,
      body: body ? JSON.stringify(body) : undefined,
      keepalive: true
    })
    return retryRes.json()
  }
  
  return res.json()
}

export const api = {
  login: (token: string) => request('POST', '/api/login', { token }),
  loginOtp: (otp: string) => request('POST', '/api/login/otp', { otp }),
  getUsers: () => request('GET', '/api/users'),
  getMessages: (userId: number, page = 1) => request('GET', `/api/messages/${userId}?page=${page}`),
  sendMessage: (userId: number, data: any) => request('POST', `/api/messages/${userId}`, data),
  getConfig: () => request('GET', '/api/config'),
  saveConfig: (data: any) => request('POST', '/api/config', data),
  setupVertexOneClick: (data: { vertex_api_key: string; oneapi_base_url?: string; relay_api_key?: string; model?: string }) => request('POST', '/api/config/vertex/oneclick', data),
  getGroups: (page = 1, size = 20) => request('GET', `/api/group_listen/list?page=${page}&size=${size}`),
  toggleGroupMonitor: (groupId: number, enabled: boolean) => request('POST', '/api/group_listen/toggle', { group_id: groupId, enabled }),
  getGroupMembers: (groupId: number) => request('GET', `/api/group_members?group_id=${groupId}`),
  getGroupMessages: (groupId: number, page = 1) => request('GET', `/api/group_messages?group_id=${groupId}&page=${page}`),
  getMonitors: () => request('GET', '/api/monitors'),
  getPotentialUsers: (offset = 0, limit = 20) => request('GET', `/api/potential_users?offset=${offset}&limit=${limit}`),
  getAvailableTags: () => request('GET', '/api/potential_users/tags'),
  getOpenaiModels: async () => {
    try {
      return await request('GET', '/api/openai/models')
    } catch {
      return await request('GET', '/v1/models')
    }
  },
  execOpenai: async (data: { model: string; input: string; sandbox_id?: string; user_id?: number }) => {
    try {
      return await request('POST', '/api/openai/exec', data)
    } catch {
      return await request('POST', '/v1/exec', data)
    }
  },
}
