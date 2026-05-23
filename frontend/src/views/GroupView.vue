<template>
  <div class="page-body group-layout">
    <!-- Left: group list -->
    <aside class="group-sidebar">
      <div class="sidebar-head">
        <span class="sidebar-title">群组列表</span>
        <span class="group-count badge badge-gray">{{ groups.length }}</span>
      </div>
      <div class="group-search">
        <input v-model="searchQ" class="input" placeholder="搜索群组…" />
      </div>
      <div class="group-list">
        <div
          v-for="g in filteredGroups" :key="g.group_id"
          class="group-item" :class="{ active: selectedGroupId === g.group_id }"
          @click="selectGroup(g)"
        >
          <div class="g-avatar">{{ groupInitial(g.title) }}</div>
          <div class="g-info">
            <div class="g-title truncate">{{ g.title || g.group_id }}</div>
            <div class="g-meta">
              <span class="badge" :class="g.monitor_enabled ? 'badge-green' : 'badge-gray'" style="font-size:10px">
                {{ g.monitor_enabled ? '监听中' : '未监听' }}
              </span>
              <span class="g-id mono">{{ g.group_id }}</span>
            </div>
          </div>
        </div>
        <div v-if="filteredGroups.length === 0" class="empty-hint">暂无群组</div>
      </div>
    </aside>

    <!-- Right: messages panel -->
    <main class="group-main">
      <div v-if="!selectedGroupId" class="empty-panel">
        <div class="empty-icon">👥</div>
        <div class="empty-text">选择左侧群组查看消息</div>
      </div>

      <template v-else>
        <!-- Panel header -->
        <div class="panel-header">
          <div class="panel-title">{{ selectedGroup?.title || selectedGroupId }}</div>
          <div class="panel-actions">
            <button class="btn btn-ghost btn-sm" @click="refreshMsgs" :disabled="msgsLoading">刷新</button>
            <button class="btn btn-ghost btn-sm" @click="openKeywords">关键词</button>
            <button class="btn btn-ghost btn-sm" @click="openUnverifiedUsers" title="未关注频道的用户">👤 未关注</button>
          </div>
        </div>

        <!-- Messages -->
        <div class="msgs-wrap" ref="msgsWrap">
          <div v-if="msgsLoading" class="empty-state">加载中…</div>
          <div v-else-if="groupMsgs.length === 0" class="empty-state">暂无消息记录</div>
          <div v-else class="msgs-list">
            <div
              v-for="msg in groupMsgs" :key="msg.id"
              class="msg-row" :class="{ 'msg-admin': msg.is_from_admin }"
            >
              <div class="msg-meta">
                <span class="msg-sender">{{ msg.sender_name || msg.sender_id }}</span>
                <span class="msg-time mono">{{ fmtTime(msg.message_ts ? msg.message_ts * 1000 : new Date(msg.timestamp).getTime()) }}</span>
                <button class="reply-btn" @click="setReply(msg)">↩ 回复</button>
              </div>
              <div v-if="msg.quoted_text" class="msg-quote">{{ msg.quoted_text }}</div>
              <div class="msg-body">{{ msg.content }}</div>
            </div>
          </div>
        </div>

        <!-- Reply bar -->
        <div v-if="replyTarget" class="reply-bar">
          <span class="reply-label">回复 {{ replyTarget.sender_name || replyTarget.sender_id }}：</span>
          <span class="reply-preview truncate">{{ replyTarget.content }}</span>
          <button class="btn btn-ghost btn-sm" @click="replyTarget = null">×</button>
        </div>

        <!-- Input -->
        <div class="input-row">
          <textarea
            v-model="inputText"
            class="input msg-input"
            placeholder="输入消息，Ctrl+Enter 发送…"
            rows="2"
            @keydown.ctrl.enter.prevent="sendMsg"
          />
          <button class="btn btn-primary send-btn" :disabled="sending || !inputText.trim()" @click="sendMsg">
            {{ sending ? '…' : '发送' }}
          </button>
        </div>
      </template>
    </main>

    <!-- Keywords modal -->
    <div v-if="showKw" class="modal-overlay" @click.self="showKw = false">
      <div class="modal-box">
        <h3>关键词配置</h3>
        <p style="font-size:12px;color:var(--text-secondary);margin-bottom:12px">群组 {{ selectedGroup?.title }}</p>
        <div class="field">
          <label>关键词（逗号分隔）</label>
          <textarea v-model="kwText" class="input" rows="4" placeholder="关键词1, 关键词2, …" />
        </div>
        <div class="modal-actions">
          <button class="btn btn-ghost" @click="showKw = false">取消</button>
          <button class="btn btn-primary" @click="saveKw">保存</button>
        </div>
      </div>
    </div>

    <!-- Unverified users modal -->
    <div v-if="showUnverified" class="modal-overlay" @click.self="showUnverified = false">
      <div class="modal-box" style="width: 500px; max-height: 80vh; display: flex; flex-direction: column;">
        <div style="display: flex; align-items: center; justify-content: space-between; margin-bottom: 16px;">
          <h3>未关注的人 ({{ unverifiedUsers.length }})</h3>
          <button class="btn btn-ghost btn-sm" @click="showUnverified = false">×</button>
        </div>

        <!-- Loading state -->
        <div v-if="unverifiedLoading" style="text-align: center; padding: 40px; color: var(--text-muted);">
          加载中…
        </div>

        <!-- Empty state -->
        <div v-else-if="unverifiedUsers.length === 0" style="text-align: center; padding: 40px; color: var(--text-muted);">
          所有用户都已关注频道！
        </div>

        <!-- User list -->
        <div v-else style="flex: 1; overflow-y: auto; margin-bottom: 16px;">
          <div style="display: flex; align-items: center; gap: 8px; padding: 8px 0; margin-bottom: 8px;">
            <input type="checkbox" :checked="selectedUsers.size === unverifiedUsers.length && unverifiedUsers.length > 0"
                   @change="selectAllUnverified" />
            <span style="font-size: 12px; font-weight: 600;">全选 ({{ selectedUsers.size }}/{{ unverifiedUsers.length }})</span>
          </div>

          <div style="display: flex; flex-direction: column; gap: 6px;">
            <div v-for="user in unverifiedUsers" :key="user.user_id"
                 style="display: flex; align-items: center; gap: 8px; padding: 8px; border-radius: 6px; border: 1px solid var(--border-muted);"
                 :style="selectedUsers.has(user.user_id) ? { background: 'rgba(88,166,255,.1)', borderColor: 'rgba(88,166,255,.3)' } : {}">
              <input type="checkbox" :checked="selectedUsers.has(user.user_id)"
                     @change="toggleUserSelection(user.user_id)" />
              <div style="flex: 1; min-width: 0;">
                <div style="font-size: 12px; font-weight: 600;">{{ user.first_name || user.username || 'Unknown' }}</div>
                <a :href="tgUserProfileHref(user.user_id, user.username)"
                   style="font-size: 11px; color: var(--text-muted);"
                   class="hover:underline"
                   @click.stop.prevent="openTgUserProfile(user.user_id, user.username)">{{ tgUserLinkLabel(user.user_id, user.username, user.first_name) }}</a>
              </div>
            </div>
          </div>
        </div>

        <!-- Actions -->
        <div style="display: flex; gap: 8px; justify-content: flex-end;">
          <button class="btn btn-ghost" @click="showUnverified = false">取消</button>
          <button class="btn btn-primary" :disabled="selectedUsers.size === 0 || isSendingReminders"
                  @click="sendReminders">
            {{ isSendingReminders ? '发送中…' : `@提醒 (${selectedUsers.size})` }}
          </button>
        </div>
      </div>
    </div>
  </div>
</template>

<script setup lang="ts">
import { ref, computed, nextTick, onMounted } from 'vue'
import { chatApi } from '@/api'
import { openTgUserProfile, tgUserProfileHref, tgUserLinkLabel } from '@/utils/tgUserLink'

interface Group {
  group_id: number
  title: string
  monitor_enabled: boolean
  keywords: string
}

interface GroupMsg {
  id: number
  group_id: number
  sender_id: number
  sender_name: string | null
  content: string | null
  quoted_text: string | null
  is_from_admin: boolean
  message_ts: number | null
  timestamp: string
}

const groups          = ref<Group[]>([])
const selectedGroupId = ref<number | null>(null)
const selectedGroup   = computed(() => groups.value.find(g => g.group_id === selectedGroupId.value) ?? null)
const groupMsgs       = ref<GroupMsg[]>([])
const msgsLoading     = ref(false)
const searchQ         = ref('')
const inputText       = ref('')
const sending         = ref(false)
const replyTarget     = ref<GroupMsg | null>(null)
const msgsWrap        = ref<HTMLElement | null>(null)
const showKw          = ref(false)
const kwText          = ref('')
const showUnverified  = ref(false)
const unverifiedUsers = ref<any[]>([])
const unverifiedLoading = ref(false)
const selectedUsers   = ref<Set<number>>(new Set())
const isSendingReminders = ref(false)

const filteredGroups = computed(() => {
  const q = searchQ.value.trim().toLowerCase()
  if (!q) return groups.value
  return groups.value.filter(g =>
    String(g.group_id).includes(q) ||
    (g.title || '').toLowerCase().includes(q)
  )
})

onMounted(() => loadGroups())

async function loadGroups() {
  // Bootstrap empty; populate by entering a group_id manually or via a future /groups endpoint.
  groups.value = []
}

async function selectGroup(g: Group) {
  selectedGroupId.value = g.group_id
  replyTarget.value = null
  inputText.value = ''
  await loadMsgs()
}

async function loadMsgs() {
  if (!selectedGroupId.value) return
  msgsLoading.value = true
  try {
    const res = await chatApi.getGroupMessages(selectedGroupId.value, 80, 0)
    groupMsgs.value = res.data
    await nextTick()
    scrollBottom()
  } finally {
    msgsLoading.value = false
  }
}

async function refreshMsgs() { await loadMsgs() }

async function sendMsg() {
  const text = inputText.value.trim()
  if (!text || !selectedGroupId.value || sending.value) return
  sending.value = true
  try {
    await chatApi.sendGroupMessage({
      group_id: selectedGroupId.value,
      content: text,
      reply_to_tg_msg_id: replyTarget.value?.message_ts ?? null,
      quoted_text: replyTarget.value?.content ?? null,
    })
    inputText.value = ''
    replyTarget.value = null
    await loadMsgs()
  } finally {
    sending.value = false
  }
}

function setReply(msg: GroupMsg) { replyTarget.value = msg }

function openKeywords() {
  kwText.value = selectedGroup.value?.keywords || ''
  showKw.value = true
}
function saveKw() {
  if (selectedGroup.value) selectedGroup.value.keywords = kwText.value
  showKw.value = false
}

async function openUnverifiedUsers() {
  if (!selectedGroupId.value) return
  showUnverified.value = true
  unverifiedLoading.value = true
  selectedUsers.value.clear()
  try {
    const res = await fetch(`/api/group_listen/unverified_users?group_id=${selectedGroupId.value}`, {
      headers: { 'Authorization': `Bearer ${localStorage.getItem('admin_token') || ''}` }
    })
    const data = await res.json()
    if (data.status === 'ok') {
      unverifiedUsers.value = data.data || []
    }
  } catch (e) {
    console.error('加载未关注用户失败:', e)
  } finally {
    unverifiedLoading.value = false
  }
}

function toggleUserSelection(userId: number) {
  if (selectedUsers.value.has(userId)) {
    selectedUsers.value.delete(userId)
  } else {
    selectedUsers.value.add(userId)
  }
}

function selectAllUnverified() {
  if (selectedUsers.value.size === unverifiedUsers.value.length) {
    selectedUsers.value.clear()
  } else {
    unverifiedUsers.value.forEach(u => selectedUsers.value.add(u.user_id))
  }
}

async function sendReminders() {
  if (selectedUsers.value.size === 0 || !selectedGroupId.value) return
  
  isSendingReminders.value = true
  try {
    const channelLink = prompt('请输入频道链接 (例如: https://t.me/mychannel):')
    if (!channelLink) return
    
    const res = await fetch('/api/group_listen/send_channel_reminder', {
      method: 'POST',
      headers: {
        'Authorization': `Bearer ${localStorage.getItem('admin_token') || ''}`,
        'Content-Type': 'application/json'
      },
      body: JSON.stringify({
        group_id: selectedGroupId.value,
        user_ids: Array.from(selectedUsers.value),
        channel_link: channelLink
      })
    })
    
    const data = await res.json()
    if (data.status === 'ok') {
      alert(`已发送 ${data.sent_count} 条提醒消息`)
      selectedUsers.value.clear()
      await openUnverifiedUsers()
    } else {
      alert(`发送失败: ${data.message}`)
    }
  } catch (e) {
    console.error('发送提醒失败:', e)
    alert('发送失败，请查看控制台')
  } finally {
    isSendingReminders.value = false
  }
}

function scrollBottom() {
  if (msgsWrap.value) msgsWrap.value.scrollTop = msgsWrap.value.scrollHeight
}
function groupInitial(title: string) { return (title || '?').charAt(0).toUpperCase() }
function fmtTime(ms: number) { return new Date(ms).toLocaleString('zh-CN', { hour12: false }) }
</script>

<style scoped>
.group-layout {
  display: flex; gap: 0;
  height: calc(100vh - var(--topbar-h));
  padding: 0; overflow: hidden;
}
.group-sidebar {
  width: 260px; flex-shrink: 0;
  display: flex; flex-direction: column;
  border-right: 1px solid var(--border);
  background: var(--bg-surface); overflow: hidden;
}
.sidebar-head {
  display: flex; align-items: center; justify-content: space-between;
  padding: 14px 16px 10px; border-bottom: 1px solid var(--border-muted);
}
.sidebar-title { font-size: 13px; font-weight: 700; }
.group-search { padding: 8px 12px; border-bottom: 1px solid var(--border-muted); }
.group-list { flex: 1; overflow-y: auto; }
.group-item {
  display: flex; align-items: center; gap: 10px;
  padding: 10px 14px; cursor: pointer;
  border-bottom: 1px solid var(--border-muted); transition: background .12s;
}
.group-item:hover { background: var(--bg-elevated); }
.group-item.active { background: var(--bg-hover); }
.g-avatar {
  width: 36px; height: 36px; border-radius: 50%;
  background: var(--bg-hover);
  display: flex; align-items: center; justify-content: center;
  font-size: 15px; font-weight: 700; color: var(--accent); flex-shrink: 0;
}
.g-info { flex: 1; min-width: 0; }
.g-title { font-size: 13px; font-weight: 500; }
.g-meta { display: flex; align-items: center; gap: 6px; margin-top: 3px; }
.g-id { font-size: 10px; color: var(--text-muted); }
.empty-hint { padding: 20px; text-align: center; font-size: 12px; color: var(--text-muted); }

.group-main {
  flex: 1; display: flex; flex-direction: column;
  overflow: hidden; background: var(--bg-base);
}
.empty-panel {
  flex: 1; display: flex; flex-direction: column;
  align-items: center; justify-content: center; gap: 12px;
}
.empty-icon { font-size: 48px; opacity: .3; }
.empty-text { font-size: 14px; color: var(--text-muted); }

.panel-header {
  display: flex; align-items: center; justify-content: space-between;
  padding: 12px 16px; border-bottom: 1px solid var(--border);
  background: var(--bg-surface);
}
.panel-title { font-size: 15px; font-weight: 600; }
.panel-actions { display: flex; gap: 6px; }

.msgs-wrap { flex: 1; overflow-y: auto; padding: 12px 16px; }
.msgs-list { display: flex; flex-direction: column; gap: 10px; }
.msg-row {
  background: var(--bg-surface); border: 1px solid var(--border-muted);
  border-radius: var(--radius-md); padding: 10px 12px;
}
.msg-admin {
  background: rgba(88,166,255,.06); border-color: rgba(88,166,255,.2);
}
.msg-meta { display: flex; align-items: center; gap: 8px; margin-bottom: 5px; }
.msg-sender { font-size: 12px; font-weight: 600; color: var(--accent); }
.msg-time { font-size: 10px; color: var(--text-muted); }
.reply-btn {
  margin-left: auto; background: none; border: none; cursor: pointer;
  font-size: 11px; color: var(--text-muted); padding: 0;
}
.reply-btn:hover { color: var(--accent); }
.msg-quote {
  font-size: 11px; color: var(--text-secondary);
  border-left: 2px solid var(--accent); padding-left: 7px; margin-bottom: 5px;
  white-space: pre-wrap; word-break: break-all;
}
.msg-body { font-size: 13px; white-space: pre-wrap; word-break: break-all; }

.reply-bar {
  display: flex; align-items: center; gap: 8px;
  padding: 6px 14px; background: var(--bg-elevated);
  border-top: 1px solid var(--border-muted); font-size: 12px;
}
.reply-label { color: var(--accent); font-weight: 600; white-space: nowrap; }
.reply-preview { flex: 1; color: var(--text-secondary); }

.input-row {
  display: flex; gap: 8px; padding: 10px 14px;
  border-top: 1px solid var(--border); background: var(--bg-surface);
}
.msg-input { flex: 1; resize: none; }
.send-btn { align-self: flex-end; }

.empty-state { padding: 40px; text-align: center; color: var(--text-muted); font-size: 13px; }

.field { margin-bottom: 12px; }
.field label { display: block; font-size: 11px; font-weight: 600;
  color: var(--text-secondary); margin-bottom: 5px; text-transform: uppercase; }
.modal-overlay {
  position: fixed; inset: 0; background: rgba(0,0,0,.6);
  display: flex; align-items: center; justify-content: center; z-index: 100;
}
.modal-box {
  background: var(--bg-surface); border: 1px solid var(--border);
  border-radius: var(--radius-lg); padding: 28px; width: 420px; max-width: 95vw;
}
.modal-box h3 { font-size: 16px; font-weight: 600; margin-bottom: 8px; }
.modal-actions { display: flex; justify-content: flex-end; gap: 8px; margin-top: 20px; }
</style>
