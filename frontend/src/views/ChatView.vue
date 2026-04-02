<template>
  <div class="chat-view">
    <div class="user-panel">
      <div class="panel-header">
        <span>会话列表</span>
        <span class="badge badge-blue">{{ store.users.length }}</span>
      </div>
      <div v-if="store.users.length === 0" class="empty-hint">暂无会话</div>
      <div
        v-for="u in store.users" :key="u.from_userid"
        class="user-item" :class="{ active: store.selectedUserId === u.from_userid }"
        @click="selectUser(u)"
      >
        <div class="avatar">{{ initials(u.from_username) }}</div>
        <div class="user-info">
          <div class="user-name truncate">{{ u.from_username || `User ${u.from_userid}` }}</div>
          <div class="user-summary truncate">{{ u.summary || '...' }}</div>
        </div>
        <span v-if="u.unread_count > 0" class="unread-dot">{{ u.unread_count }}</span>
      </div>
    </div>

    <div class="msg-panel">
      <div v-if="!store.selectedUserId" class="no-selection">← 选择一个会话</div>
      <template v-else>
        <div class="msg-topbar">
          <strong>{{ currentUser?.from_username || `User ${store.selectedUserId}` }}</strong>
          <button class="btn btn-ghost btn-sm" @click="markRead">标记已读</button>
        </div>
        <div class="msg-list" ref="msgListEl">
          <div
            v-for="m in store.messages" :key="m.msg_id"
            class="msg-bubble" :class="isAdmin(m.from_userid) ? 'from-admin' : 'from-user'"
          >
            <div class="bubble-content">
              <span v-if="m.quoted_text" class="quoted">{{ m.quoted_text }}</span>
              {{ m.translated_text || m.content }}
            </div>
            <span class="msg-time">{{ fmtTime(m.timestamp) }}</span>
          </div>
        </div>
        <div class="composer">
          <textarea v-model="draft" class="input composer-input" rows="2"
            placeholder="输入消息 (Enter 发送)..."
            @keydown.enter.exact.prevent="send" />
          <button class="btn btn-primary" @click="send">发送</button>
        </div>
      </template>
    </div>
  </div>
</template>

<script setup lang="ts">
import { ref, computed, onMounted, nextTick, watch } from 'vue'
import { useChatStore } from '@/stores/chat'

const store = useChatStore()
const draft = ref('')
const msgListEl = ref<HTMLElement | null>(null)

onMounted(() => store.fetchUsers())

watch(() => store.messages, () => nextTick(scrollBottom))

const currentUser = computed(() =>
  store.users.find(u => u.from_userid === store.selectedUserId)
)

function isAdmin(uid: number) { return uid !== store.selectedUserId }
function initials(n?: string) { return n ? n.slice(0,2).toUpperCase() : '?' }
function fmtTime(ts?: string) {
  if (!ts) return ''
  return new Date(ts).toLocaleTimeString('zh-CN', { hour: '2-digit', minute: '2-digit' })
}
function scrollBottom() {
  if (msgListEl.value) msgListEl.value.scrollTop = msgListEl.value.scrollHeight
}
async function selectUser(u: any) {
  store.selectedUserId = u.from_userid
  store.selectedUserName = u.from_username || u.username || 'User ' + u.from_userid
  store.messages = []
  await store.fetchMessages(u.from_userid, 1)
  nextTick(scrollBottom)
}
async function send() {
  if (!draft.value.trim() || !store.selectedUserId) return
  const body = { content: draft.value.trim() }
  await fetch(`/api/messages/${store.selectedUserId}`, {
    method: 'POST',
    headers: { 'Authorization': `Bearer ${localStorage.getItem('admin_token')||''}`, 'Content-Type': 'application/json' },
    body: JSON.stringify(body)
  })
  draft.value = ''
  await store.fetchMessages(store.selectedUserId, 1)
}
async function markRead() {
  if (!store.selectedUserId) return
  await fetch(`/api/mark_read/${store.selectedUserId}`, {
    method: 'POST',
    headers: { 'Authorization': `Bearer ${localStorage.getItem('admin_token')||''}`, 'Content-Type': 'application/json' }
  })
  await store.fetchUsers()
}
</script>

<style scoped>
.chat-view { display: flex; height: 100vh; overflow: hidden; }
.user-panel {
  width: 280px; flex-shrink: 0; border-right: 1px solid var(--border);
  background: var(--bg-surface); display: flex; flex-direction: column; overflow-y: auto;
}
.panel-header {
  padding: 14px 16px; display: flex; align-items: center;
  justify-content: space-between; border-bottom: 1px solid var(--border);
  font-size: 13px; font-weight: 600;
}
.user-item {
  display: flex; align-items: center; gap: 10px;
  padding: 10px 14px; cursor: pointer; border-bottom: 1px solid var(--border-muted);
  transition: background .12s;
}
.user-item:hover, .user-item.active { background: var(--bg-hover); }
.avatar {
  width: 36px; height: 36px; border-radius: 50%;
  background: var(--bg-hover); display: flex; align-items: center;
  justify-content: center; font-size: 12px; font-weight: 700;
  color: var(--accent); flex-shrink: 0;
}
.user-info { flex: 1; min-width: 0; }
.user-name { font-size: 13px; font-weight: 500; }
.user-summary { font-size: 11px; color: var(--text-secondary); margin-top: 2px; }
.unread-dot {
  background: var(--accent); color: #fff;
  border-radius: 99px; padding: 1px 6px; font-size: 11px; font-weight: 700;
}
.empty-hint { padding: 24px; text-align: center; color: var(--text-muted); font-size: 13px; }
.msg-panel { flex: 1; display: flex; flex-direction: column; overflow: hidden; }
.no-selection {
  flex: 1; display: flex; align-items: center; justify-content: center;
  color: var(--text-muted); font-size: 14px;
}
.msg-topbar {
  padding: 12px 18px; border-bottom: 1px solid var(--border);
  display: flex; align-items: center; justify-content: space-between;
  background: var(--bg-surface);
}
.msg-list { flex: 1; overflow-y: auto; padding: 16px; display: flex; flex-direction: column; gap: 10px; }
.msg-bubble { display: flex; flex-direction: column; max-width: 68%; }
.from-admin { align-self: flex-end; align-items: flex-end; }
.from-user  { align-self: flex-start; align-items: flex-start; }
.bubble-content {
  padding: 8px 12px; border-radius: var(--radius-md);
  font-size: 13px; line-height: 1.55; white-space: pre-wrap; word-break: break-word;
}
.from-admin .bubble-content { background: var(--accent); color: #fff; }
.from-user  .bubble-content { background: var(--bg-elevated); color: var(--text-primary); }
.quoted {
  display: block; font-size: 11px; opacity: .7;
  border-left: 2px solid rgba(255,255,255,.4); padding-left: 6px; margin-bottom: 4px;
}
.msg-time { font-size: 10px; color: var(--text-muted); margin-top: 3px; }
.composer {
  padding: 12px 16px; border-top: 1px solid var(--border);
  display: flex; gap: 8px; align-items: flex-end;
  background: var(--bg-surface);
}
.composer-input { resize: none; flex: 1; }
</style>
