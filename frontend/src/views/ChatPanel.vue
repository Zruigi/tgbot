<template>
  <div class="flex h-full animate-in fade-in duration-300">
    <div class="border-r border-outline-variant/20 flex flex-col bg-surface-container-lowest/50" :class="mobilePanelView==='list'?'w-full md:w-80':'hidden md:flex md:w-80'">
      <div class="p-6 border-b border-outline-variant/10">
        <div class="flex items-center justify-between mb-4">
          <h2 class="text-xl font-black">用户列表</h2>
          <span class="bg-primary/10 text-primary text-[10px] font-bold px-2 py-1 rounded-full">{{ chatStore.users.length }} 用户</span>
        </div>
        <div class="relative">
          <span class="material-symbols-outlined absolute left-3 top-1/2 -translate-y-1/2 text-slate-400 text-sm">search</span>
          <input type="text" v-model="searchQuery" placeholder="搜索 ID 或用户名..." class="w-full pl-10 pr-4 py-2.5 bg-surface-container rounded-xl border-none text-sm focus:ring-2 focus:ring-primary/20"/>
        </div>
      </div>
      <div class="flex-1 overflow-y-auto no-scrollbar">
        <div v-if="chatStore.isLoading" class="flex flex-col items-center justify-center py-20">
          <div class="w-8 h-8 border-4 border-primary/20 border-t-primary rounded-full animate-spin"></div>
        </div>
        <div v-for="user in filteredUsers" :key="user.from_userid"
             @click="selectUser(user);mobilePanelView='detail'"
             class="px-4 py-3 mx-2 my-1 rounded-2xl cursor-pointer transition-all duration-200 flex items-center space-x-4"
             :class="chatStore.selectedUserId===user.from_userid?'bg-primary/10 border-primary/20 shadow-sm':'hover:bg-surface-container border-transparent hover:shadow-sm'">
          <div class="relative flex-shrink-0">
            <div v-if="user.avatar_file_id && chatStore.fileUrls[user.avatar_file_id]" class="w-12 h-12 rounded-2xl overflow-hidden shadow-sm">
              <img :src="chatStore.fileUrls[user.avatar_file_id]" class="w-12 h-12 object-cover block"/>
            </div>
            <div v-else class="w-12 h-12 rounded-2xl bg-gradient-to-br from-primary to-primary-container flex items-center justify-center text-on-primary font-bold">
              {{ (user.from_username||user.username||'U').charAt(0).toUpperCase() }}
            </div>
            <div v-if="user.unread_count>0" class="absolute -top-1 -right-1 bg-error text-white text-[9px] font-bold w-5 h-5 rounded-full flex items-center justify-center border-2 border-white">{{ user.unread_count }}</div>
          </div>
          <div class="flex-1 min-w-0">
            <div class="flex items-center justify-between mb-0.5">
              <span class="font-bold text-sm truncate">{{ user.from_username||user.username||'用户 '+user.from_userid }}</span>
              <span class="text-[9px] text-slate-400">{{ formatShortTime(user.last_msg_time) }}</span>
            </div>
            <div class="text-[10px] text-slate-400 truncate">ID: {{ user.from_userid }}</div>
          </div>
        </div>
      </div>
    </div>
    <div class="flex-1 flex flex-col bg-surface relative" :class="mobilePanelView==='detail'?'flex':'hidden md:flex'">
      <div v-if="!chatStore.selectedUserId" class="flex-1 flex flex-col items-center justify-center cartesian-grid">
        <div class="w-24 h-24 bg-primary/5 rounded-full flex items-center justify-center mb-6"><span class="material-symbols-outlined text-primary text-5xl opacity-40">forum</span></div>
        <h2 class="text-xl font-bold text-slate-400">选择一个用户开始对话</h2>
      </div>
      <div v-else class="flex-1 flex flex-col overflow-hidden">
        <div class="px-6 py-4 bg-surface-container-lowest/80 backdrop-blur-md border-b border-outline-variant/10 flex items-center justify-between z-10">
          <div class="flex items-center space-x-4">
            <button class="md:hidden w-8 h-8 flex items-center justify-center rounded-xl hover:bg-surface-container mr-1" @click="mobilePanelView='list'"><span class="material-symbols-outlined text-slate-500">arrow_back</span></button>
            <div v-if="avatarUrl" class="w-10 h-10 rounded-xl overflow-hidden shadow-sm border border-outline-variant/10 flex-shrink-0"><img :src="avatarUrl" class="w-10 h-10 object-cover block"/></div>
            <div v-else class="w-10 h-10 rounded-xl bg-primary/10 flex items-center justify-center text-primary font-bold">{{ (chatStore.selectedUserName||'U').charAt(0).toUpperCase() }}</div>
            <div><h3 class="font-bold">{{ chatStore.selectedUserName||'用户 '+chatStore.selectedUserId }}</h3><div class="text-[10px] text-slate-400">UID: {{ chatStore.selectedUserId }}</div></div>
          </div>
          <div class="flex items-center space-x-2">
            <button @click="markAsRead" class="flex items-center px-4 py-2 text-xs font-bold text-primary bg-primary/5 hover:bg-primary/10 rounded-xl"><span class="material-symbols-outlined text-sm mr-1.5">done_all</span>标记已读</button>
            <button @click="deleteChat" class="flex items-center px-4 py-2 text-xs font-bold text-error bg-error/5 hover:bg-error/10 rounded-xl"><span class="material-symbols-outlined text-sm mr-1.5">delete</span>清空记录</button>
          </div>
        </div>
        <div ref="chatHistoryRef" @scroll="handleScroll" class="flex-1 overflow-y-auto p-6 space-y-6 bg-surface-container-low/30 no-scrollbar relative">
          <!-- 触顶加载更多指示器 -->
          <div class="flex justify-center pb-2">
            <div v-if="isLoadingMore" class="flex items-center gap-2 text-xs text-slate-400">
              <div class="w-4 h-4 border-2 border-primary/30 border-t-primary rounded-full animate-spin"></div>
              加载更多...
            </div>
            <div v-else-if="!hasMoreMessages && chatStore.messages.length > 0" class="text-[10px] text-slate-300 py-1">已加载全部消息</div>
          </div>
          <div v-for="msg in chatStore.messages" :key="msg.msg_id" :id="'msg-'+msg.tg_msg_id"
               class="relative flex flex-col w-full md:pr-16" :class="msg.from_userid==chatStore.selectedUserId?'items-start':'items-end'">
            <div v-if="msg.quoted_text" @click="scrollToMessage(msg.quoted_msg_id)"
                 class="mb-[-8px] mx-2 px-3 py-2 bg-surface-container-high/60 rounded-t-[12px] border border-outline-variant/10 cursor-pointer hover:bg-surface-container-highest transition-colors flex items-center space-x-3 opacity-80 max-w-[70%]">
              <img v-if="msg.quoted_type==='photo' && getQuotedMediaUrl(msg)" :src="getQuotedMediaUrl(msg)" class="w-10 h-10 object-cover rounded-lg flex-shrink-0"/>
              <div class="flex-1 min-w-0">
                <div class="text-[9px] font-bold text-primary uppercase tracking-tighter mb-0.5">引用回复</div>
                <div class="text-[11px] truncate italic">{{ msg.quoted_text }}</div>
              </div>
            </div>
            <div class="chat-bubble rounded-[16px] px-4 py-3 relative overflow-hidden group break-words transition-all duration-200 hover:shadow-md select-none md:select-text touch-manipulation mobile-longpress-target"
                 @contextmenu.prevent="onMsgContextMenu($event, msg)"
                 @touchstart.passive="onMsgTouchStart(msg)"
                 @touchmove.passive="onMsgTouchMove"
                 @touchend="onMsgTouchEnd"
                 @touchcancel="clearLongPressTimer"
                 :class="[
                   msg.from_userid==chatStore.selectedUserId
                     ? 'bg-gradient-to-br from-primary/8 to-primary/5 border border-primary/15 shadow-sm'
                     : 'bg-white border border-slate-100 shadow-md shadow-slate-200/40',
                   'w-full max-w-[92%] md:max-w-[78%]'
                 ]">
              <div v-if="msg.from_userid!=chatStore.selectedUserId && msg.admin_userid" class="mb-2 flex items-center border-b border-slate-100 pb-1">
                <span class="text-[9px] bg-slate-100 text-slate-500 px-1.5 py-0.5 rounded mr-2 font-black">客服</span>
                <a :href="'tg://user?id='+msg.admin_userid" class="text-[10px] font-black text-primary hover:underline">@{{ msg.admin_userid }}</a>
              </div>
              <div v-if="getMsgType(msg) !== 'text' && hasMediaSource(msg)" class="message-media-shell mb-2 rounded-[16px] overflow-hidden flex justify-center border border-slate-200/60 shadow-sm bg-surface-container-lowest">
                <img v-if="getMsgType(msg) === 'photo' && getMediaUrl(msg)"
                     :src="getMediaUrl(msg)"
                     class="max-w-full max-h-[400px] w-auto h-auto object-contain cursor-zoom-in"
                     @error="chatStore.fileErrors[msg.file_id]=true"/>
                <button v-else-if="getMsgType(msg) === 'photo'" @click="chatStore.autoLoadFile(msg.file_id)" class="px-3 py-2 text-xs font-bold underline text-primary">正在加载媒体...</button>

                <div v-else-if="getMsgType(msg) === 'sticker'" class="relative p-1">
                  <template v-if="getMediaUrl(msg)">
                    <video v-if="isVideoSticker(getMediaUrl(msg))" :src="getMediaUrl(msg)" autoplay loop muted playsinline controls class="w-32 h-32 object-contain rounded-lg"></video>
                    <img v-else :src="getMediaUrl(msg)" class="w-32 h-32 object-contain"/>
                  </template>
                  <button v-else @click="chatStore.autoLoadFile(msg.file_id)" class="px-3 py-2 text-xs font-bold underline text-primary">正在加载媒体...</button>
                </div>

                <video v-else-if="(getMsgType(msg) === 'video' || getMsgType(msg) === 'animation') && getMediaUrl(msg)"
                       :src="getMediaUrl(msg)" controls loop autoplay muted playsinline class="max-w-full max-h-[400px] w-auto h-auto rounded-lg"></video>
                <button v-else-if="getMsgType(msg) === 'video' || getMsgType(msg) === 'animation'" @click="chatStore.autoLoadFile(msg.file_id)" class="px-3 py-2 text-xs font-bold underline text-primary">正在加载媒体...</button>

                <div v-else-if="getMsgType(msg) === 'voice' || getMsgType(msg) === 'audio'" class="p-2 bg-black/5 rounded-lg flex items-center space-x-3">
                  <span class="material-symbols-outlined text-slate-400">{{ getMsgType(msg)==='voice'?'mic':'music_note' }}</span>
                  <audio v-if="getMediaUrl(msg)" :src="getMediaUrl(msg)" controls class="h-8 max-w-[200px]"></audio>
                  <button v-else @click="chatStore.autoLoadFile(msg.file_id)" class="text-xs font-bold underline">正在加载媒体...</button>
                </div>

                <div v-else class="p-3 bg-black/5 rounded-lg flex items-center space-x-3">
                  <span class="material-symbols-outlined text-slate-400">description</span>
                  <a v-if="getMediaUrl(msg)" :href="getMediaUrl(msg)" target="_blank" class="text-xs font-bold text-primary underline">下载</a>
                  <button v-else @click="chatStore.autoLoadFile(msg.file_id)" class="text-xs font-bold underline">正在加载媒体...</button>
                </div>
              </div>
              <div v-else-if="getMsgType(msg) !== 'text'" class="mb-2 px-3 py-2 rounded-xl bg-amber-50 text-amber-600 text-xs font-semibold border border-amber-200/60">
                媒体文件不可用（历史消息可能缺少 file_id）
              </div>
              <div v-if="msg.translated_text && msg.original_text" class="mb-2 p-3 rounded-lg border-l-4"
                   :class="msg.from_userid==chatStore.selectedUserId?'bg-primary/8 border-primary/30':'bg-slate-50 border-slate-200'">
                <div class="text-[9px] font-black uppercase tracking-widest mb-1 flex items-center text-slate-400"><span class="material-symbols-outlined text-xs mr-1">translate</span>自动翻译</div>
                <div class="text-[15px]">{{ msg.translated_text }}</div>
              </div>
              <div v-if="msg.original_text" class="whitespace-pre-wrap text-[15px] leading-relaxed font-medium">
                <span class="text-[12px] text-black/30 font-bold block border-t border-black/5 pt-2 mt-2">原文:</span>{{ msg.original_text }}
              </div>
              <div v-if="!msg.original_text && msg.content && !isStickerPlaceholder(msg)" class="whitespace-pre-wrap text-[15px] leading-relaxed font-medium" :class="msg.from_userid==chatStore.selectedUserId?'text-slate-700':'text-slate-900'">{{ msg.content }}</div>
              <div v-else-if="!msg.original_text && getMsgType(msg)!=='text'" class="text-[13px] font-semibold" :class="msg.from_userid==chatStore.selectedUserId?'text-slate-500':'text-slate-600'">[{{ getMsgTypeLabel(msg) }}]</div>
              <div class="flex items-center gap-2 mt-2 pt-1.5" :class="msg.from_userid==chatStore.selectedUserId?'border-t border-primary/10':'border-t border-slate-100'">
                <span class="text-[9px] text-slate-400 mr-auto">{{ formatTime(msg.timestamp) }}</span>
                <span v-if="msg.from_userid!=chatStore.selectedUserId" class="text-[9px] flex items-center space-x-1">
                  <span class="material-symbols-outlined text-[10px]" :class="getMsgStatus(msg)==='sent'?'text-primary':getMsgStatus(msg)==='failed'?'text-error':'text-slate-300'">{{ getMsgStatus(msg)==='sent'?'done_all':getMsgStatus(msg)==='failed'?'error':'schedule' }}</span>
                  <span class="text-[9px]" :class="getMsgStatus(msg)==='failed'?'text-error':'text-slate-400'">{{ getMsgStatus(msg)==='sent'?'已送达':getMsgStatus(msg)==='failed'?'发送失败':'发送中' }}</span>
                </span>
              </div>
            </div>
            <div class="hidden md:flex absolute top-1/2 -translate-y-1/2 right-0 items-center gap-1">
              <button @click="prepareReply(msg)" class="p-1.5 rounded-lg bg-surface-container/80 border border-outline-variant/20 hover:bg-primary/10">
                <span class="material-symbols-outlined text-sm text-slate-500">reply</span>
              </button>
              <button v-if="msg.from_userid!=chatStore.selectedUserId" @click="deleteMessage(msg)" class="p-1.5 rounded-lg bg-surface-container/80 border border-outline-variant/20 hover:bg-rose-50">
                <span class="material-symbols-outlined text-sm text-slate-500 hover:text-rose-500">delete</span>
              </button>
            </div>
            <div class="md:hidden mt-2 flex justify-end">
              <button @click="openMobileActionMenu(msg)" class="inline-flex items-center gap-1.5 px-2.5 py-1.5 rounded-lg bg-surface-container text-slate-500 text-[11px] font-bold border border-outline-variant/20">
                <span class="material-symbols-outlined text-sm">more_horiz</span>
                操作
              </button>
            </div>
          </div>
        </div>

        <div v-if="mobileActionMsg" class="md:hidden absolute inset-0 z-40 bg-black/35" @click="closeMobileActionMenu"></div>
        <div v-if="mobileActionMsg" class="md:hidden absolute left-4 right-4 bottom-6 z-50 rounded-2xl border border-outline-variant/30 bg-surface-container-highest shadow-2xl overflow-hidden">
          <button @click="handleMobileReply" class="w-full flex items-center gap-3 px-4 py-4 text-sm font-bold text-left hover:bg-primary/8">
            <span class="material-symbols-outlined text-primary">reply</span>
            引用回复
          </button>
          <button v-if="mobileActionMsg.from_userid!=chatStore.selectedUserId" @click="handleMobileDelete" class="w-full flex items-center gap-3 px-4 py-4 text-sm font-bold text-left text-rose-500 hover:bg-rose-50/70 border-t border-outline-variant/20">
            <span class="material-symbols-outlined">delete</span>
            删除消息
          </button>
          <button @click="closeMobileActionMenu" class="w-full px-4 py-3 text-xs font-bold text-slate-500 border-t border-outline-variant/20 hover:bg-surface-container">
            取消
          </button>
        </div>
        <button v-show="!isNearBottom" @click="scrollToBottom" class="absolute bottom-32 right-8 w-14 h-14 bg-primary text-on-primary rounded-full shadow-2xl flex items-center justify-center hover:scale-110 active:scale-95 transition-all z-20 group">
          <span class="material-symbols-outlined text-3xl">keyboard_double_arrow_down</span>
          <span v-if="newMsgCount>0" class="absolute -top-1 -right-1 min-w-[24px] h-6 px-1.5 bg-error text-on-error text-[10px] font-black rounded-full flex items-center justify-center border-2 border-white">{{ newMsgCount }}</span>
        </button>
        <div class="p-6 bg-surface-container-lowest border-t border-outline-variant/10 z-10">
          <div v-if="replyingTo" class="mb-4 p-4 bg-primary/5 border-l-4 border-primary rounded-2xl flex items-center justify-between">
            <div class="flex-1 min-w-0"><div class="text-[10px] font-black text-primary uppercase tracking-widest mb-1">正在回复</div><div class="text-xs text-on-surface-variant truncate font-medium">{{ replyingTo.content||'...' }}</div></div>
            <button @click="replyingTo=null" class="w-8 h-8 flex items-center justify-center hover:text-error rounded-full"><span class="material-symbols-outlined text-lg">close</span></button>
          </div>
          <div v-if="selectedImage" class="mb-4 inline-block group">
            <div class="relative inline-block rounded-2xl overflow-hidden shadow-xl border-4 border-primary/20">
              <img :src="selectedImagePreview" class="h-32 object-cover"/>
              <div class="absolute inset-0 bg-black/40 opacity-0 group-hover:opacity-100 transition-opacity flex items-center justify-center">
                <button @click="clearImage" class="w-10 h-10 bg-error text-on-error rounded-full flex items-center justify-center"><span class="material-symbols-outlined">delete</span></button>
              </div>
            </div>
          </div>
          <div class="flex items-end space-x-4">
            <div class="flex-1 relative bg-surface-container rounded-3xl p-1 transition-all focus-within:ring-4 focus-within:ring-primary/10">
              <textarea v-model="inputText" @paste="handleInputPaste" @keydown.enter.exact.prevent="sendMessage" placeholder="输入回复内容..." rows="1" class="w-full bg-transparent border-none focus:ring-0 text-sm py-3 px-4 min-h-[50px] max-h-[150px] no-scrollbar resize-none font-medium"></textarea>
              <div class="absolute right-3 bottom-3">
                <button @click="imageInputRef?.click()" class="w-10 h-10 flex items-center justify-center text-primary hover:bg-primary/10 rounded-full"><span class="material-symbols-outlined">image</span></button>
                <input type="file" ref="imageInputRef" @change="handleImageSelect" accept="image/*" class="hidden"/>
              </div>
            </div>
            <button @click="sendMessage" :disabled="!inputText.trim()&&!selectedImage" class="w-14 h-14 bg-primary text-on-primary rounded-2xl flex items-center justify-center shadow-lg shadow-primary/30 hover:scale-105 active:scale-95 transition-all disabled:opacity-50">
              <span class="material-symbols-outlined">send</span>
            </button>
          </div>
          <div class="mt-2 px-1 text-[10px] text-slate-400">Enter 发送，Shift+Enter 换行</div>
        </div>
      </div>
    </div>
  </div>
</template>
<script setup lang="ts">
import { ref, computed, onMounted, onUnmounted, watch, nextTick } from 'vue'
import { useChatStore } from '@/stores/chat'

const chatStore = useChatStore()
const readCookieToken = () => {
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

const mobilePanelView = ref<'list'|'detail'>('list')
const searchQuery = ref('')
const inputText = ref('')
const replyingTo = ref<any|null>(null)
const selectedImage = ref<File|null>(null)
const selectedImagePreview = ref('')
const chatHistoryRef = ref<HTMLElement|null>(null)
const imageInputRef = ref<HTMLInputElement|null>(null)
const isNearBottom = ref(true)
const newMsgCount = ref(0)
const enableTranslation = ref(true)
const longPressTimer = ref<ReturnType<typeof setTimeout> | null>(null)
const mobileActionMsg = ref<any | null>(null)
let pollTimer: ReturnType<typeof setInterval>|null = null

const avatarUrl = computed(() => {
  const aid = chatStore.getSelectedUserAvatar()
  if (!aid) return ''
  if (!chatStore.fileUrls[aid]) chatStore.autoLoadFile(aid)
  return chatStore.fileUrls[aid] || ''
})

const normalizedUsers = computed(() =>
  (chatStore.users || []).map((u: any) => ({
    ...u,
    from_userid: Number(u.from_userid ?? u.user_id ?? 0),
    from_username: u.from_username ?? u.username ?? ''
  }))
)

const filteredUsers = computed(() => {
  const q = searchQuery.value.trim().toLowerCase()
  if (!q) return normalizedUsers.value
  return normalizedUsers.value.filter((u: any) => {
    const uid = String(u.from_userid || '')
    const uname = String(u.from_username || '').toLowerCase()
    return uid.includes(q) || uname.includes(q)
  })
})

const selectedUserRecord = computed(() =>
  normalizedUsers.value.find((u: any) =>
    Number(u.from_userid || u.user_id) === Number(chatStore.selectedUserId)
  ) || null
)

const isSelectedAiAssistant = computed(() => Boolean(selectedUserRecord.value?.is_ai_assistant))

watch(() => chatStore.users, (list) => {
  for (const u of list || []) {
    if (u.avatar_file_id) chatStore.autoLoadFile(u.avatar_file_id)
  }
}, { immediate: true, deep: true })

onMounted(() => { fetchUsers(); loadConfig(); pollTimer = setInterval(pollLatestMessages, 15000) })
onUnmounted(() => {
  if (pollTimer) clearInterval(pollTimer)
  clearLongPressTimer()
})

async function fetchUsers() { await chatStore.fetchUsers() }

// Polling: fetch page 1 and merge any new messages at the end
async function pollLatestMessages() {
  if (!chatStore.selectedUserId) return
  try {
    const res = await fetch(`/api/messages/${chatStore.selectedUserId}?page=1`, {
      headers: authH(),
      keepalive: true
    })
    const d = await res.json()
    if (d.status !== 'ok') return
    const fresh: any[] = d.data || []
    if (fresh.length === 0) return

    const { appendedCount, updatedRows } = upsertMessagesFromLatest(fresh)
    if (appendedCount === 0 && updatedRows.length === 0) return

    triggerAutoLoadForMessages([...updatedRows, ...fresh])

    if (!isNearBottom.value && appendedCount > 0) {
      newMsgCount.value += appendedCount
    } else {
      await nextTick()
      scrollToBottom()
    }
  } catch {}
}

function upsertMessagesFromLatest(fresh: any[]) {
  const existingById = new Map(chatStore.messages.map((m: any) => [m.msg_id, m]))
  const updatedRows: any[] = []

  for (const incoming of fresh) {
    const existing = existingById.get(incoming.msg_id)
    if (!existing) continue

    const shouldPatch = (
      (!existing.file_id && incoming.file_id) ||
      (!existing.media_url && incoming.media_url) ||
      (existing.status !== incoming.status) ||
      (!existing.tg_msg_id && incoming.tg_msg_id)
    )

    if (shouldPatch) {
      const merged = { ...existing, ...incoming }
      existingById.set(incoming.msg_id, merged)
      updatedRows.push(merged)
    }
  }

  const newMsgs = fresh.filter((m: any) => !existingById.has(m.msg_id))

  if (updatedRows.length > 0 || newMsgs.length > 0) {
    const patchedCurrent = chatStore.messages.map((m: any) => existingById.get(m.msg_id) || m)
    chatStore.messages = [...patchedCurrent, ...newMsgs]
  }

  return { appendedCount: newMsgs.length, updatedRows }
}


async function selectUser(u: any) {
  chatStore.selectedUserId = u.from_userid
  chatStore.selectedUserName = u.from_username || u.username || 'ID:' + u.from_userid
  chatStore.messages = []
  msgPage.value = 1
  hasMoreMessages.value = true
  await fetchMessages()
}

// Backend uses page param, limit is fixed at 50 server-side.
// We treat each page response < 50 items as "no more".
const BACKEND_PAGE_SIZE = 50
const msgPage = ref(1)
const hasMoreMessages = ref(true)
const isLoadingMore = ref(false)

async function fetchMessages() {
  if (!chatStore.selectedUserId) return
  // Always fetch page 1 (latest messages), reset state
  await chatStore.fetchMessages(chatStore.selectedUserId, 1)
  triggerAutoLoadForMessages(chatStore.messages)
  msgPage.value = 1
  hasMoreMessages.value = (chatStore.messages.length >= BACKEND_PAGE_SIZE)
  await nextTick()
  if (isNearBottom.value) scrollToBottom()
}

async function loadMoreMessages() {
  if (!chatStore.selectedUserId || isLoadingMore.value || !hasMoreMessages.value) return
  isLoadingMore.value = true
  const el = chatHistoryRef.value
  const prevScrollHeight = el?.scrollHeight || 0
  try {
    const nextPage = msgPage.value + 1
    const older = await chatStore.fetchMoreMessages(chatStore.selectedUserId, nextPage)
    if (older.length < BACKEND_PAGE_SIZE) hasMoreMessages.value = false
    if (older.length > 0) {
      chatStore.messages = [...older, ...chatStore.messages]
      triggerAutoLoadForMessages(older)
      msgPage.value = nextPage
      await nextTick()
      // 保持滚动位置不跳动
      if (el) el.scrollTop = el.scrollHeight - prevScrollHeight
    } else {
      hasMoreMessages.value = false
    }
  } finally {
    isLoadingMore.value = false
  }
}

function triggerAutoLoadForMessages(messageList: any[]) {
  for (const m of messageList || []) {
    if (m.file_id) chatStore.autoLoadFile(m.file_id)
    if (m.quoted_file_id) chatStore.autoLoadFile(m.quoted_file_id)
  }
}

function scrollToBottom() {
  if (chatHistoryRef.value) chatHistoryRef.value.scrollTop = chatHistoryRef.value.scrollHeight
}

function handleScroll() {
  if (!chatHistoryRef.value) return
  const el = chatHistoryRef.value
  isNearBottom.value = el.scrollHeight - el.scrollTop - el.clientHeight < 80
  if (isNearBottom.value) newMsgCount.value = 0
  // 触顶加载更多
  if (el.scrollTop < 60) loadMoreMessages()
}

function scrollToMessage(id: number) {
  const el = document.getElementById('msg-' + id)
  if (el) { el.scrollIntoView({ behavior: 'smooth', block: 'center' }); el.classList.add('highlight-anim'); setTimeout(() => el.classList.remove('highlight-anim'), 1500) }
}

function prepareReply(msg: any) { replyingTo.value = msg }

function openMobileActionMenu(msg: any) {
  mobileActionMsg.value = msg
}

function closeMobileActionMenu() {
  mobileActionMsg.value = null
}

function handleMobileReply() {
  if (!mobileActionMsg.value) return
  prepareReply(mobileActionMsg.value)
  closeMobileActionMenu()
}

async function handleMobileDelete() {
  if (!mobileActionMsg.value) return
  await deleteMessage(mobileActionMsg.value)
  closeMobileActionMenu()
}

function onMsgContextMenu(event: MouseEvent, msg: any) {
  if (window.matchMedia('(max-width: 767px)').matches) {
    event.preventDefault()
    openMobileActionMenu(msg)
  }
}

function clearLongPressTimer() {
  if (longPressTimer.value) {
    clearTimeout(longPressTimer.value)
    longPressTimer.value = null
  }
}

function onMsgTouchStart(msg: any) {
  clearLongPressTimer()
  longPressTimer.value = setTimeout(() => {
    if (typeof navigator !== 'undefined' && 'vibrate' in navigator) {
      navigator.vibrate(10)
    }
    openMobileActionMenu(msg)
    clearLongPressTimer()
  }, 380)
}

function onMsgTouchMove() {
  clearLongPressTimer()
}

function onMsgTouchEnd() {
  clearLongPressTimer()
}

async function deleteMessage(msg: any) {
  if (!chatStore.selectedUserId || !msg.tg_msg_id) return
  if (!confirm('确定删除这条消息？')) return
  await fetch(`/api/messages/${chatStore.selectedUserId}/delete`, {
    method: 'POST', headers: authH(), keepalive: true,
    body: JSON.stringify({ tg_msg_id: msg.tg_msg_id })
  })
  chatStore.messages = chatStore.messages.filter((m: any) => m.tg_msg_id !== msg.tg_msg_id)
}
function clearImage() { selectedImage.value = null; selectedImagePreview.value = '' }

function handleImageSelect(e: Event) {
  const f = (e.target as HTMLInputElement).files?.[0]
  if (!f) return
  selectedImage.value = f
  selectedImagePreview.value = URL.createObjectURL(f)
}

function handleInputPaste(e: ClipboardEvent) {
  const items = e.clipboardData?.items
  if (!items) return
  for (const item of items) {
    if (item.type.startsWith('image/')) {
      const file = item.getAsFile()
      if (file) {
        e.preventDefault()
        selectedImage.value = file
        selectedImagePreview.value = URL.createObjectURL(file)
        return
      }
    }
  }
}

function getMsgType(msg: any) {
  return msg.msg_type || msg.type || 'text'
}

function hasMediaSource(msg: any) {
  return Boolean(msg?.file_id || msg?.media_url)
}

function getMediaUrl(msg: any) {
  if (msg?.file_id && chatStore.fileUrls[msg.file_id]) return chatStore.fileUrls[msg.file_id]
  if (msg?.media_url) return msg.media_url
  return ''
}

function getQuotedMediaUrl(msg: any) {
  if (msg?.quoted_file_id && chatStore.fileUrls[msg.quoted_file_id]) return chatStore.fileUrls[msg.quoted_file_id]
  if (msg?.quoted_media_url) return msg.quoted_media_url
  return ''
}

function getMsgTypeLabel(msg: any) {
  const t = getMsgType(msg)
  if (t === 'photo') return '图片'
  if (t === 'animation') return '动图'
  if (t === 'video') return '视频'
  if (t === 'sticker') return '贴纸'
  if (t === 'voice') return '语音'
  if (t === 'audio') return '音频'
  return '媒体'
}

function getMsgStatus(msg: any) {
  if (msg.status) return msg.status
  // 旧数据兼容：管理员发出的消息没有状态字段时默认按已送达展示
  return msg.from_userid !== chatStore.selectedUserId ? 'sent' : 'received'
}

async function sendMessage() {
  if (!chatStore.selectedUserId || (!inputText.value.trim() && !selectedImage.value)) return
  const t = token()
  const sentContent = inputText.value.trim()
  const imageToSend = selectedImage.value // 保存引用，防止后续清空导致丢失
  const replyTarget = replyingTo.value // 保存引用
  const optimisticPreviewUrl = imageToSend ? URL.createObjectURL(imageToSend) : ''

  // Optimistic: immediately show outgoing message
  const optimisticMsg: any = {
    msg_id: Date.now(), // temp id
    from_userid: 0, // 0 = admin/outgoing
    content: sentContent || '[图片]',
    timestamp: new Date().toISOString(),
    status: 'sending',
    type: imageToSend ? 'photo' : 'text',
    msg_type: imageToSend ? 'photo' : 'text',
    media_url: optimisticPreviewUrl || null,
    quoted_text: replyTarget?.content || null,
  }
  chatStore.messages = [...chatStore.messages, optimisticMsg]
  inputText.value = ''
  selectedImage.value = null
  selectedImagePreview.value = ''
  replyingTo.value = null
  await nextTick()
  scrollToBottom()

  // 立即发送请求，不阻塞 UI
  sendMessageAsync(
    chatStore.selectedUserId,
    sentContent,
    imageToSend,
    replyTarget,
    optimisticMsg,
    t,
    isSelectedAiAssistant.value
  )
}

async function sendMessageAsync(
  userId: number,
  content: string,
  image: File | null,
  replyTarget: any,
  optimisticMsg: any,
  token: string,
  isAiAssistant: boolean
) {
  try {
    let ok = false

    if (isAiAssistant) {
      if (image) {
        chatStore.messages = chatStore.messages.map((m: any) =>
          m.msg_id === optimisticMsg.msg_id ? { ...m, status: 'failed' } : m
        )
        alert('AI 助手会话暂不支持发送图片，请切换到真实用户会话发送')
        return
      }

      const res = await fetch(`/api/messages/${userId}/ai`, {
        method: 'POST',
        headers: authH(),
        body: JSON.stringify({ content }),
        keepalive: true
      })
      const d = await res.json().catch(() => ({}))
      ok = res.ok && d.status === 'ok'

      if (ok && d.reply) {
        const aiMsg = {
          msg_id: Date.now() + 1,
          from_userid: userId,
          to_userid: 0,
          content: d.reply,
          timestamp: new Date().toISOString(),
          status: 'received',
          type: 'text',
          msg_type: 'text'
        }
        chatStore.messages = [...chatStore.messages, aiMsg]
        await nextTick()
        scrollToBottom()
      }
    } else if (image) {
      const fd = new FormData()
      fd.append('image', image)
      if (content) fd.append('content', content)
      if (replyTarget?.tg_msg_id) fd.append('reply_to_msg_id', String(replyTarget.tg_msg_id))

      const res = await fetch(`/api/messages/${userId}`, {
        method: 'POST',
        headers: { 'Authorization': 'Bearer ' + token },
        body: fd,
        keepalive: true
      })
      const d = await res.json().catch(() => ({}))
      ok = res.ok && d.status === 'ok'
    } else {
      const body: any = { content, do_translate: enableTranslation.value }
      if (replyTarget?.tg_msg_id) body.reply_to_msg_id = replyTarget.tg_msg_id

      const res = await fetch(`/api/messages/${userId}`, {
        method: 'POST',
        headers: authH(),
        body: JSON.stringify(body),
        keepalive: true
      })
      const d = await res.json().catch(() => ({}))
      ok = res.ok && d.status === 'ok'
    }

    if (!ok) {
      chatStore.messages = chatStore.messages.map((m: any) =>
        m.msg_id === optimisticMsg.msg_id ? { ...m, status: 'failed' } : m
      )
      return
    }

    // 发送成功后，更新消息状态为已发送
    chatStore.messages = chatStore.messages.map((m: any) =>
      m.msg_id === optimisticMsg.msg_id ? { ...m, status: 'sent' } : m
    )

    // 异步刷新消息列表（不阻塞 UI）
    if (!isAiAssistant) {
      setTimeout(() => fetchMessages(), 500)
    }
  } catch (err) {
    console.error('发送消息失败:', err)
    // 标记为失败
    chatStore.messages = chatStore.messages.map((m: any) =>
      m.msg_id === optimisticMsg.msg_id ? { ...m, status: 'failed' } : m
    )
  }
}

async function markAsRead() {
  if (!chatStore.selectedUserId) return
  await fetch(`/api/mark_read/${chatStore.selectedUserId}`, { method: 'POST', headers: authH(), keepalive: true })
  await fetchUsers()
}

async function deleteChat() {
  if (!chatStore.selectedUserId || !confirm('确定清空记录？')) return
  await fetch(`/api/delete_chat/${chatStore.selectedUserId}`, { method: 'POST', headers: authH(), keepalive: true })
  chatStore.messages = []
}

async function loadConfig() {
  try { const r = await fetch('/api/config', { headers: authH() }); const d = await r.json(); if (d.status==='ok') enableTranslation.value = d.data?.global_auto_translate==='true' } catch {}
}

function isVideoSticker(url: string) { return url.includes('.webm') }
function isStickerPlaceholder(msg: any) { return (msg.type==='sticker'||msg.msg_type==='sticker') && !msg.content }

function formatTime(ts: any) {
  if (!ts) return ''
  const d = new Date(typeof ts==='number' ? ts*1000 : String(ts).replace(' ','T'))
  if (isNaN(d.getTime())) return ''
  return d.toLocaleString('zh-CN',{month:'2-digit',day:'2-digit',hour:'2-digit',minute:'2-digit'})
}

function formatShortTime(ts: any) {
  if (!ts) return ''
  const d = new Date(String(ts).replace(' ','T'))
  if (isNaN(d.getTime())) return ''
  const now = new Date()
  if (d.toDateString()===now.toDateString()) return d.toLocaleTimeString('zh-CN',{hour:'2-digit',minute:'2-digit'})
  return d.toLocaleDateString('zh-CN',{month:'2-digit',day:'2-digit'})
}
</script>

<style scoped>
.mobile-longpress-target {
  -webkit-touch-callout: none;
  -webkit-user-select: none;
  user-select: none;
}

@media (min-width: 768px) {
  .mobile-longpress-target {
    -webkit-touch-callout: default;
    -webkit-user-select: text;
    user-select: text;
  }
}
</style>

