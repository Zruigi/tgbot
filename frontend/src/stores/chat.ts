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
      if (d.status === 'ok') {
        users.value = d.data || []
        for (const u of users.value) {
          if (u.avatar_file_id && !fileUrls[u.avatar_file_id]) {
            autoLoadFile(u.avatar_file_id)
          }
        }
      }
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
        fileErrors[fileId] = true
      }
    } catch {
      fileErrors[fileId] = true
    }
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
    getSelectedUserAvatar
  }
})
