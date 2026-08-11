import { defineStore } from 'pinia'
import { ref, reactive } from 'vue'

// ★ 按租户隔离的 localStorage 冷加载缓存（A 商户进来看 A 的缓存，B 商户看 B 的）
import {
  cacheGet,
  cacheSet,
  getCurrentTenantId,
  tenantCacheKey,
  readMessagesCache,
  writeMessagesCache,
  readFileCache,
  writeFileCache,
} from '@/utils/cache'

const readCookieToken = () => {
  if (typeof document === 'undefined') return ''
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

const token = () => {
  const sessionRaw = sessionStorage.getItem('adm_s')
  if (sessionRaw) {
    try {
      return decodeURIComponent(atob(sessionRaw))
    } catch {
      return ''
    }
  }
  return localStorage.getItem('admin_token') || readCookieToken()
}
const authH = () => ({ 'Authorization': `Bearer ${token()}`, 'Content-Type': 'application/json' })

// Agent 助手虚拟发送者 ID（与 ChatPanel.vue 的 AI_ASSISTANT_USER_ID 一致）
const AI_ASSISTANT_SENDER_ID = -99999

/** 把后端 agent.conversations 行映射为聊天渲染字段 */
function mapAgentConversations(list: any[], fallbackUserId: number): any[] {
  return (list || []).map((m: any) => ({
    msg_id: m.id,
    from_userid: m.role === 'user' ? (m.user_id || fallbackUserId) : AI_ASSISTANT_SENDER_ID,
    to_userid: 0,
    content: m.content || '',
    media_url: m.media_url || null,
    timestamp: m.created_at,
    status: m.status === 'error' ? 'error' : 'received',
    type: m.msg_type || 'text',
    msg_type: m.msg_type || 'text',
    requires_confirmation: !!m.requires_confirmation,
    ui_actions: Array.isArray(m.ui_actions) ? m.ui_actions : [],
    job: m.job || null,
  }))
}

export const useChatStore = defineStore('chat', () => {
  const users = ref<any[]>([])
  const messages = ref<any[]>([])
  const selectedUserId = ref<number | null>(null)
  const selectedUserName = ref('')
  const isLoading = ref(false)
  const fileUrls = reactive<Record<string, string>>({})
  const fileErrors = reactive<Record<string, boolean>>({})

  async function fetchUsers() {
    // ★ 冷启动：先出租户隔离的用户列表缓存（秒出），再后台刷新
    const tenantId = getCurrentTenantId()
    if (users.value.length === 0) {
      const cachedUsers = cacheGet<any[]>(tenantCacheKey(tenantId, 'users'))
      if (Array.isArray(cachedUsers) && cachedUsers.length > 0) {
        users.value = [...cachedUsers]
        // 预加载缓存头像；未缓存的走后台加载
        const cachedFiles = readFileCache(tenantId)
        for (const u of users.value) {
          if (!u.avatar_file_id) continue
          if (cachedFiles[u.avatar_file_id]) {
            fileUrls[u.avatar_file_id] = cachedFiles[u.avatar_file_id]
          } else {
            autoLoadFile(u.avatar_file_id)
          }
        }
      } else {
        // 无用户缓存也先把头像缓存捞出来（聊天记录里的图片/语音秒出）
        const cachedFiles = readFileCache(tenantId)
        for (const [fileId, url] of Object.entries(cachedFiles)) {
          if (!fileUrls[fileId]) fileUrls[fileId] = url
        }
      }
    }
    isLoading.value = true
    try {
      const res = await fetch('/api/users', { headers: authH(), keepalive: true })
      const d = await res.json()
      console.log('Users API Response:', d)
      if (d.status === 'ok') {
        // 强制触发 Vue 响应式更新 - 使用扩展运算符创建新数组
        const newData = d.data || []
        console.log(`Setting users: ${newData.length} items`, newData)
        users.value = [...newData]
        console.log(`Users updated: ${users.value.length}`)
        // 刷新成功 → 写回租户缓存，下次冷启动直接命中
        cacheSet(tenantCacheKey(tenantId, 'users'), users.value)
        for (const u of users.value) {
          if (u.avatar_file_id && !fileUrls[u.avatar_file_id]) {
            autoLoadFile(u.avatar_file_id)
          }
        }
      } else {
        console.error('Failed to fetch users:', d)
        users.value = []
      }
    } catch (err) {
      console.error('Error fetching users:', err)
      users.value = []
    } finally {
      isLoading.value = false
    }
  }

  async function fetchMessages(userId: number, page = 1) {
    // ★ 冷启动：进入会话先出该联系人的最近消息缓存（秒出），再后台刷新
    const tenantId = getCurrentTenantId()
    if (page === 1 && messages.value.length === 0) {
      const cached = readMessagesCache(tenantId, userId)
      if (cached && cached.length > 0) {
        messages.value = cached
        autoLoadMedia()
      }
    }
    const res = await fetch(`/api/messages/${userId}?page=${page}`, { headers: authH(), keepalive: true })
    const d = await res.json()
    console.log('Chat Data Received:', d)
    if (d.status === 'ok') {
      messages.value = d.data || []
      if (page === 1 && messages.value.length > 0) {
        writeMessagesCache(tenantId, userId, messages.value)
      }
      autoLoadMedia()
    }
  }

  async function fetchMoreMessages(userId: number, page: number): Promise<any[]> {
    const res = await fetch(`/api/messages/${userId}?page=${page}`, { headers: authH(), keepalive: true })
    const d = await res.json()
    if (d.status === 'ok') return d.data || []
    return []
  }

  /** ★ Agent 会话历史：走 /api/ai/conversations（替代 /api/messages/-99999，后者会 400/403） */
  async function fetchAgentMessages(userId: number, page = 1) {
    const res = await fetch(`/api/ai/conversations?page=${page}`, { headers: authH(), keepalive: true })
    const d = await res.json()
    console.log('Agent Conversations Received:', d)
    if (d.status === 'ok') {
      messages.value = mapAgentConversations(d.data || [], userId)
      autoLoadMedia()
    }
  }

  async function fetchMoreAgentMessages(userId: number, page: number): Promise<any[]> {
    const res = await fetch(`/api/ai/conversations?page=${page}`, { headers: authH(), keepalive: true })
    const d = await res.json()
    if (d.status === 'ok') return mapAgentConversations(d.data || [], userId)
    return []
  }

  function autoLoadMedia(targetMessages: any[] = messages.value) {
    for (const msg of targetMessages) {
      const t = msg.msg_type || msg.type || ''
      if (msg.file_id && !fileUrls[msg.file_id] && !fileErrors[msg.file_id]) {
        if (['photo', 'sticker', 'voice', 'audio', 'animation', 'video'].includes(t)) {
          autoLoadFile(msg.file_id)
        }
      }
      if (msg.quoted_file_id && !fileUrls[msg.quoted_file_id] && !fileErrors[msg.quoted_file_id]) {
        autoLoadFile(msg.quoted_file_id)
      }
    }
  }

  async function autoLoadFile(fileId: string) {
    if (!fileId || fileUrls[fileId] || fileErrors[fileId]) return
    try {
      const res = await fetch(`/api/file/${fileId}`, { headers: authH(), keepalive: true })
      const d = await res.json()
      if (d.url) {
        fileUrls[fileId] = d.url
        // ★ 头像/文件 URL（base64 data URL，自包含不过期）写回租户缓存
        writeFileCache(getCurrentTenantId(), fileId, d.url)
      } else {
        // ★ 不立即标记为永久失败 —— 允许后续重试（5秒自动重试一次）
        setTimeout(() => { delete fileErrors[fileId]; }, 5000)
        fileErrors[fileId] = true
      }
    } catch {
      // ★ 同上：允许重试而非永久标记
      setTimeout(() => { delete fileErrors[fileId]; }, 5000)
      fileErrors[fileId] = true
    }
  }

  /** ★ 手动强制重新加载文件（清除缓存后重新请求） */
  async function forceReloadFile(fileId: string) {
    if (!fileId) return
    delete fileUrls[fileId]
    delete fileErrors[fileId]
    await autoLoadFile(fileId)
  }

  async function loadFile(fileId: string) {
    await autoLoadFile(fileId)
  }

  function getSelectedUserAvatar(): string {
    const u = users.value.find((u: any) => u.from_userid === selectedUserId.value)
    return u?.avatar_file_id || ''
  }

  return {
    users,
    messages,
    selectedUserId,
    selectedUserName,
    isLoading,
    fileUrls,
    fileErrors,
    fetchUsers,
    fetchMessages,
    fetchMoreMessages,
    fetchAgentMessages,
    fetchMoreAgentMessages,
    loadFile,
    autoLoadFile,
    autoLoadMedia,
    getSelectedUserAvatar,
    forceReloadFile
  }
})
