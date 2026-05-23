/**
 * API 工具函数
 */

// CSRF Token 获取与存储（与 api/index.ts 保持一致）
let csrfToken = ''
const refreshCsrf = async (): Promise<void> => {
  try {
    const headers = getAuthHeaders()
    const res = await fetch('/api/csrf-token', { headers })
    const data = await res.json()
    if (data.status === 'ok' && data.token) {
      csrfToken = data.token
    }
  } catch (e) {
    console.error('[API] Failed to refresh CSRF token:', e)
  }
}

// 获取认证头 - 优先从 sessionStorage 获取 (adm_s)，兼容 localStorage (admin_token)
function getAuthHeaders(includeCsrf = false): Record<string, string> {
  let token = ''

  // 优先从 sessionStorage 获取 (adm_s 是新格式)
  const sessionToken = sessionStorage.getItem('adm_s')
  if (sessionToken) {
    try {
      token = decodeURIComponent(atob(sessionToken))
    } catch {}
  }

  // 兼容旧的 localStorage (admin_token)
  if (!token) {
    token = localStorage.getItem('admin_token') || ''
  }

  const headers: Record<string, string> = {
    'Content-Type': 'application/json',
  }

  if (token) {
    headers['Authorization'] = `Bearer ${token}`
  }

  // CSRF Token（按需添加）
  if (includeCsrf && csrfToken) {
    headers['X-CSRF-Token'] = csrfToken
  }

  return headers
}

/**
 * 发送 JSON 请求（支持自定义超时）
 */
export async function apiJson(
  url: string,
  method: string = 'GET',
  body?: any,
  options?: { timeoutMs?: number; isAiCall?: boolean }
): Promise<any> {
  // AI 请求默认超时 120 秒；消息发送/安全检测链路给到 60 秒，其它请求 15 秒
  const isAiCall = options?.isAiCall ?? url.includes('/ai')
  const isMessageSendCall = method.toUpperCase() === 'POST' && /\/api\/messages\/\d+$/.test(url)
  const timeoutMs = options?.timeoutMs ?? (isAiCall ? 120000 : isMessageSendCall ? 60000 : 15000)

  // POST/PUT/PATCH/DELETE 请求需要 CSRF Token
  const needCsrf = ['POST', 'DELETE', 'PATCH', 'PUT'].includes(method.toUpperCase())

  if (needCsrf && !csrfToken) {
    await refreshCsrf()
  }

  const controller = new AbortController()
  const timer = setTimeout(() => controller.abort(), timeoutMs)

  try {
    const fetchOptions: RequestInit = {
      method,
      headers: getAuthHeaders(needCsrf),
      signal: controller.signal,
    }

    if (body && (method === 'POST' || method === 'PUT' || method === 'PATCH')) {
      fetchOptions.body = JSON.stringify(body)
    }

    console.log(`[API] ${method} ${url}`, body ? { body, timeoutMs } : { timeoutMs })

    const response = await fetch(url, fetchOptions)
    clearTimeout(timer)

    // 处理 403（CSRF 过期），刷新并重试一次
    if (response.status === 403 && needCsrf) {
      console.warn('[API] 403 received, refreshing CSRF and retrying...')
      await refreshCsrf()

      const retryController = new AbortController()
      const retryTimer = setTimeout(() => retryController.abort(), timeoutMs)

      const retryResponse = await fetch(url, {
        method,
        headers: getAuthHeaders(needCsrf),
        body: fetchOptions.body,
        signal: retryController.signal,
      })
      clearTimeout(retryTimer)

      if (!retryResponse.ok) {
        const errorText = await retryResponse.text()
        throw new Error(`HTTP ${retryResponse.status}: ${errorText}`)
      }

      return await retryResponse.json()
    }

    if (!response.ok) {
      const errorText = await response.text()
      let errorMessage = `HTTP ${response.status}`

      try {
        // 尝试解析错误详情
        const errorJson = JSON.parse(errorText)
        errorMessage = errorJson.message || errorJson.error?.message || errorMessage
      } catch {
        // 非 JSON 错误响应，保留原始文本
        if (errorText) errorMessage += `: ${errorText.slice(0, 200)}`
      }

      throw new Error(errorMessage)
    }

    // 安全解析 JSON 响应
    const contentType = response.headers.get('content-type') || ''
    if (!contentType.includes('application/json')) {
      const text = await response.text()
      console.warn('[API] Non-JSON response:', text.slice(0, 200))
      // 尝试解析（某些后端可能不设置正确的 Content-Type）
      try {
        return JSON.parse(text)
      } catch {
        throw new Error(`服务器返回非 JSON 格式数据: ${text.slice(0, 100)}`)
      }
    }

    return await response.json()
  } catch (e: any) {
    clearTimeout(timer)

    if (e.name === 'AbortError') {
      if (isAiCall) {
        throw new Error('AI 响应超时 (>120秒)，请稍后重试或简化问题')
      }
      if (isMessageSendCall) {
        throw new Error('发送超时 (>60秒)，后端可能仍在进行安全检测或 Telegram 投递，请稍后刷新会话查看结果')
      }
      throw new Error('请求超时，请检查网络连接')
    }

    throw e
  }
}

/**
 * 发送 FormData 请求（用于文件上传）
 */
export async function apiFormData(
  url: string,
  formData: FormData,
  method: string = 'POST'
): Promise<any> {
  // POST 请求需要 CSRF Token
  if (!csrfToken) {
    await refreshCsrf()
  }

  const controller = new AbortController()
  const timer = setTimeout(() => controller.abort(), 60000) // 文件上传超时 60 秒

  try {
    // 使用统一的认证头（兼容新旧 Token 格式）
    const headers: Record<string, string> = {}

    // 复用 getAuthHeaders 的 Token 逻辑，但不要 Content-Type
    let token = ''
    const sessionToken = sessionStorage.getItem('adm_s')
    if (sessionToken) {
      try { token = decodeURIComponent(atob(sessionToken)) } catch {}
    }
    if (!token) token = localStorage.getItem('admin_token') || ''

    if (token) headers['Authorization'] = `Bearer ${token}`
    if (csrfToken) headers['X-CSRF-Token'] = csrfToken

    // 注意：不要设置 Content-Type，让浏览器自动设置 boundary
    const options: RequestInit = {
      method,
      headers,
      body: formData,
      signal: controller.signal,
    }

    console.log(`[API] ${method} ${url} (FormData)`)

    const response = await fetch(url, options)
    clearTimeout(timer)

    if (!response.ok) {
      const errorText = await response.text()
      throw new Error(`HTTP ${response.status}: ${errorText}`)
    }

    return await response.json()
  } catch (e: any) {
    clearTimeout(timer)
    if (e.name === 'AbortError') {
      throw new Error('上传超时 (>60秒)，请检查网络或文件大小')
    }
    throw e
  }
}
// cache bust Sun Apr  5 17:43:42 CST 2026
