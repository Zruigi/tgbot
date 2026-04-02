import { defineStore } from 'pinia'
import { ref } from 'vue'
import { chatApi } from '@/api'

export interface InboxUser {
  from_userid: number
  from_username: string
  summary: string
  unread_count: number
  last_active: string
  avatar_file_id: string | null
}

export interface Message {
  msg_id: number
  from_userid: number
  to_userid: number
  content: string | null
  msg_type: string
  status: string
  timestamp: string
  tg_msg_id: number | null
  quoted_text: string | null
  translated_text: string | null
}

export const useChatStore = defineStore('chat', () => {
  const users = ref<InboxUser[]>([])
  const messages = ref<Message[]>([])
  const selectedUserId = ref<number | null>(null)
  const loading = ref(false)

  async function fetchUsers() {
    const res = await chatApi.getUsers()
    users.value = res.data
  }

  async function selectUser(userId: number) {
    selectedUserId.value = userId
    loading.value = true
    try {
      const res = await chatApi.getMessages(userId)
      messages.value = res.data
    } finally {
      loading.value = false
    }
  }

  async function sendMessage(userId: number, content: string, replyToMsgId?: number) {
    await chatApi.sendMessage(userId, { content, reply_to_msg_id: replyToMsgId })
    await selectUser(userId)
  }

  async function markRead(userId: number) {
    await chatApi.markRead(userId)
    const u = users.value.find(u => u.from_userid === userId)
    if (u) u.unread_count = 0
  }

  return { users, messages, selectedUserId, loading, fetchUsers, selectUser, sendMessage, markRead }
})
