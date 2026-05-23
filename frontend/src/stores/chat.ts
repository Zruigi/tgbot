import { defineStore } from 'pinia'
import { ref, reactive } from 'vue'

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

export const useChatStore = defineStore('chat', () => {
  const users = ref<any[]>([])
  const messages = ref<any[]>([])
  const selectedUserId = ref<number | null>(null)
  const selectedUserName = ref('')
  const isLoading = ref(false)
  const fileUrls = reactive<Record<string, string>>({})
  const fileErrors = reactive<Record<string, boolean>>({})

  async function fetchUsers() {
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
    const res = await fetch(`/api/messages/${userId}?page=${page}`, { headers: authH(), keepalive: true })
    const d = await res.json()
    console.log('Chat Data Received:', d)
    if (d.status === 'ok') {
      messages.value = d.data || []
      autoLoadMedia()
    }
  }

  async function fetchMoreMessages(userId: number, page: number): Promise<any[]> {
    const res = await fetch(`/api/messages/${userId}?page=${page}`, { headers: authH(), keepalive: true })
    const d = await res.json()
    if (d.status === 'ok') return d.data || []
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
    loadFile,
    autoLoadFile,
    autoLoadMedia,
    getSelectedUserAvatar,
    forceReloadFile
  }
})
