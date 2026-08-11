/**
 * 按租户隔离的 localStorage 冷加载缓存工具
 *
 * 目标：冷启动（进入 Mini App / 刷新页面）时先用上一次缓存的数据快速渲染，
 * 再在后台调 API 刷新 —— 不同商户（tenant）用不同 key 天然隔离：
 *   A 商户进入 → 读 A 的缓存；B 商户进入 → 读 B 的缓存。
 *
 * 存储策略：
 * - users / messages / files 全部落 localStorage（Telegram Mini App WebView
 *   与浏览器均持久；sessionStorage 在部分 WebView 会话中不保留）
 * - 所有 key 前缀 `tgbot_cache_<tenantId>_`
 * - 头像 URL 是 base64 data URL（后端 /api/file 返回，自包含不过期），可安全持久化
 */

const CACHE_PREFIX = 'tgbot_cache_'
const SESSION_CTX_KEY = 'adm_ctx' // 与 stores/auth.ts 保持一致

/** 当前会话的租户隔离维度：有 tenantId 用 tenantId，平台管理员空租户用 platform */
export function getCurrentTenantId(): string {
  try {
    const raw = sessionStorage.getItem(SESSION_CTX_KEY)
    if (raw) {
      const parsed = JSON.parse(raw)
      const tenantId = String(parsed?.tenantId || parsed?.tenant_id || '')
      if (tenantId) return tenantId
    }
  } catch {
    /* ignore */
  }
  return 'platform'
}

export function tenantCacheKey(tenantId: string, name: string): string {
  return `${CACHE_PREFIX}${tenantId || 'default'}_${name}`
}

export function cacheGet<T>(key: string): T | null {
  try {
    const raw = localStorage.getItem(key)
    if (!raw) return null
    return JSON.parse(raw) as T
  } catch {
    return null
  }
}

export function cacheSet(key: string, value: unknown): void {
  try {
    localStorage.setItem(key, JSON.stringify(value))
  } catch {
    /* 配额满 / 隐私模式：静默忽略 */
  }
}

/** 清除所有 tgbot_cache_* 前缀的缓存（logout 时调用，避免跨账号数据残留） */
export function clearAllTenantCaches(): void {
  try {
    const keys: string[] = []
    for (let i = 0; i < localStorage.length; i++) {
      const key = localStorage.key(i)
      if (key && key.startsWith(CACHE_PREFIX)) keys.push(key)
    }
    keys.forEach((key) => localStorage.removeItem(key))
  } catch {
    /* ignore */
  }
}

// ---------------------------------------------------------------------------
// 消息缓存（按 联系人 userId 分组，LRU 限制联系人数量）
// ---------------------------------------------------------------------------

const MESSAGES_CACHE_MAX_USERS = 10

interface MessagesCacheEntry {
  msgs: any[]
  t: number // 最近写入时间戳
}

export function readMessagesCache(tenantId: string, userId: number): any[] | null {
  const map = cacheGet<Record<string, MessagesCacheEntry>>(
    tenantCacheKey(tenantId, 'messages'),
  )
  const entry = map?.[String(userId)]
  if (!entry || !Array.isArray(entry.msgs) || entry.msgs.length === 0) return null
  return entry.msgs
}

export function writeMessagesCache(tenantId: string, userId: number, msgs: any[]): void {
  if (!Array.isArray(msgs) || msgs.length === 0) return
  const key = tenantCacheKey(tenantId, 'messages')
  const map = cacheGet<Record<string, MessagesCacheEntry>>(key) || {}
  map[String(userId)] = { msgs, t: Date.now() }
  // LRU 裁剪：只保留最近写入的 N 个联系人
  const entries = Object.entries(map).sort((a, b) => (b[1]?.t || 0) - (a[1]?.t || 0))
  const trimmed: Record<string, MessagesCacheEntry> = {}
  for (const [uid, entry] of entries.slice(0, MESSAGES_CACHE_MAX_USERS)) {
    trimmed[uid] = entry
  }
  cacheSet(key, trimmed)
}

// ---------------------------------------------------------------------------
// 文件/头像缓存（base64 data URL，LRU 限制条目数与估算体积）
// ---------------------------------------------------------------------------

const FILE_CACHE_MAX_ITEMS = 300
const FILE_CACHE_MAX_BYTES = 4 * 1024 * 1024 // 4MB，留出 localStorage 余量

interface FileCacheEntry {
  url: string
  t: number
}

export function readFileCache(tenantId: string): Record<string, string> {
  const map = cacheGet<Record<string, FileCacheEntry>>(
    tenantCacheKey(tenantId, 'files'),
  )
  if (!map) return {}
  const now = Date.now()
  const out: Record<string, string> = {}
  for (const [fileId, entry] of Object.entries(map)) {
    if (entry && typeof entry.url === 'string' && now - (entry.t || 0) < 24 * 3600 * 1000) {
      out[fileId] = entry.url
    }
  }
  return out
}

export function writeFileCache(tenantId: string, fileId: string, url: string): void {
  if (!fileId || !url) return
  const key = tenantCacheKey(tenantId, 'files')
  const map = cacheGet<Record<string, FileCacheEntry>>(key) || {}
  map[fileId] = { url, t: Date.now() }
  // LRU 裁剪：按时间戳排序，保留最近 N 项且总估算体积 ≤ 上限
  const entries = Object.entries(map).sort((a, b) => (b[1]?.t || 0) - (a[1]?.t || 0))
  const trimmed: Record<string, FileCacheEntry> = {}
  let totalBytes = 0
  for (const [fid, entry] of entries) {
    const size = (entry?.url?.length || 0) * 2 // base64 ≈ 字节数
    if (trimmedCount(trimmed) >= FILE_CACHE_MAX_ITEMS || totalBytes + size > FILE_CACHE_MAX_BYTES) {
      break
    }
    trimmed[fid] = entry
    totalBytes += size
  }
  cacheSet(key, trimmed)
}

function trimmedCount(map: Record<string, unknown>): number {
  return Object.keys(map).length
}
