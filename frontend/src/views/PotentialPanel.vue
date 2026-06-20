<template>
  <div class="flex-1 flex flex-col p-10 animate-in slide-in-from-right duration-500 overflow-y-auto no-scrollbar">
    <div class="flex flex-col md:flex-row items-center justify-between mb-8 gap-4">
      <div><h2 class="text-2xl font-black flex items-center"><span class="material-symbols-outlined mr-3 text-primary text-3xl">person_add</span>潜在用户捕捉</h2><p class="text-sm text-slate-400 mt-1">从监听群组中自动发现的潜在商机用户。</p></div>
      <div class="flex items-center space-x-3">
        <button @click="load" class="p-3 bg-surface-container hover:bg-surface-container-high rounded-2xl transition-all"><span class="material-symbols-outlined text-primary">refresh</span></button>
        <button @click="sync" :disabled="isSyncing" class="px-6 py-3 bg-primary text-on-primary rounded-2xl font-black shadow-lg flex items-center space-x-2 hover:scale-105 active:scale-95 transition-all">
          <span class="material-symbols-outlined" :class="{'animate-spin':isSyncing}">sync</span><span>{{ isSyncing?'同步中...':'立即同步' }}</span>
        </button>
      </div>
    </div>
    <div class="bg-surface-container-lowest p-6 rounded-3xl border border-outline-variant/10 shadow-sm mb-8 flex flex-wrap items-center gap-4">
      <div class="flex flex-col space-y-1.5 min-w-[180px]">
        <label class="text-[10px] font-black text-slate-400 uppercase tracking-widest">画像标签</label>
        <select v-model="filters.tag" class="bg-surface-container border-none rounded-xl text-sm px-4 py-2.5"><option value="">全部标签</option><option v-for="tag in availableTags" :key="tag" :value="tag">{{ tag }}</option></select>
      </div>
      <div class="flex flex-col space-y-1.5 flex-1 min-w-[200px]">
        <label class="text-[10px] font-black text-slate-400 uppercase tracking-widest">关键词搜索</label>
        <input type="text" v-model="filters.keyword" placeholder="搜索关键词..." class="bg-surface-container border-none rounded-xl text-sm px-4 py-2.5"/>
      </div>
      <button @click="load" class="mt-5 px-8 py-2.5 bg-primary/10 text-primary font-black rounded-xl hover:bg-primary/20 transition-all">应用筛选</button>
    </div>
    <div v-if="isLoading" class="flex flex-col items-center justify-center py-40"><div class="w-12 h-12 border-4 border-primary/20 border-t-primary rounded-full animate-spin mb-4"></div></div>
    <div v-else-if="users.length===0" class="flex flex-col items-center justify-center py-40 text-slate-300 italic"><span class="material-symbols-outlined text-6xl mb-4 opacity-20">person_off</span><p>未发现符合条件的潜在用户</p></div>
    <div v-else class="grid grid-cols-1 md:grid-cols-2 xl:grid-cols-3 gap-6">
      <div v-for="user in users" :key="user.user_id" class="bg-surface-container-lowest rounded-3xl border border-outline-variant/10 p-6 hover:border-primary/30 transition-all hover:shadow-xl hover:scale-[1.01] cursor-pointer">
        <div class="flex items-start justify-between mb-4">
          <div class="flex items-center space-x-4">
            <div class="w-14 h-14 rounded-2xl overflow-hidden bg-gradient-to-br from-primary to-primary-container flex items-center justify-center text-on-primary text-xl font-black">
              <img v-if="user.avatar_file_id && fileUrls[user.avatar_file_id]" :src="fileUrls[user.avatar_file_id]" class="w-14 h-14 object-cover block"/>
              <span v-else>{{ (user.first_name||user.username||'U').charAt(0).toUpperCase() }}</span>
            </div>
            <div><h3 class="font-black text-on-surface">{{ user.first_name }} {{ user.last_name }}</h3><div class="text-xs text-slate-400">@{{ user.username }}</div></div>
          </div>
          <span v-if="user.profile_tags" class="px-2 py-1 bg-amber-50 text-amber-600 text-[9px] font-black rounded-lg border border-amber-100">{{ user.profile_tags }}</span>
        </div>
        <div class="bg-surface-container-low rounded-2xl p-4 mb-4"><p class="text-sm text-on-surface italic line-clamp-2">"{{ user.message_text }}"</p></div>
        <div class="flex items-center justify-between pt-4 border-t border-outline-variant/5 mb-4">
          <div class="text-xs font-bold text-slate-600 truncate max-w-[120px]">{{ user.group_title }}</div>
          <div class="text-[10px] text-slate-400">{{ formatTime(user.captured_at) }}</div>
        </div>
        <div class="flex space-x-3">
          <button @click="promote(user.user_id)" class="flex-1 py-3 bg-primary text-on-primary text-xs font-black rounded-xl shadow-lg hover:scale-105 active:scale-95 transition-all">一键转正</button>
          <button @click="emit('chat',user.user_id,user.first_name||user.username)" class="flex-1 py-3 bg-surface-container text-primary text-xs font-black rounded-xl hover:bg-primary/10 transition-all flex items-center justify-center space-x-2"><span class="material-symbols-outlined text-sm">chat</span><span>进入会话</span></button>
        </div>
      </div>
    </div>
    <div v-if="total>limit" class="mt-10 flex items-center justify-center space-x-6">
      <button @click="changePage('prev')" :disabled="offset===0" class="w-12 h-12 rounded-full bg-surface-container flex items-center justify-center disabled:opacity-30 hover:bg-primary/10"><span class="material-symbols-outlined">chevron_left</span></button>
      <span class="text-sm font-black text-slate-400">{{ Math.floor(offset/limit)+1 }} / {{ Math.ceil(total/limit) }}</span>
      <button @click="changePage('next')" :disabled="offset+limit>=total" class="w-12 h-12 rounded-full bg-surface-container flex items-center justify-center disabled:opacity-30 hover:bg-primary/10"><span class="material-symbols-outlined">chevron_right</span></button>
    </div>
  </div>
</template>

<script setup lang="ts">
import { ref, reactive, computed, onMounted, watch } from 'vue'
import { useWsStore } from '@/stores/ws'
import { useFileStore } from '@/stores/fileStore'
const wsStore = useWsStore()
const fileStore = useFileStore()
const fileUrls = computed(() => fileStore.fileUrls)
const emit = defineEmits<{ chat: [userId: number, name: string] }>()
const token = () => localStorage.getItem('admin_token') || ''
const authH = () => ({ 'Authorization': `Bearer ${token()}`, 'Content-Type': 'application/json' })
const users = ref<any[]>([])
const availableTags = ref<string[]>([])
const isLoading = ref(false)
const isSyncing = ref(false)
const total = ref(0)
const offset = ref(0)
const limit = 20
const filters = reactive({ tag: '', keyword: '' })
onMounted(() => { load(); loadTags() })
watch(() => wsStore.lastEvent, (evt) => {
  if (!evt) return
  if (evt.type === 'potential_sync' || evt.type === 'potential_promoted') load()
})
async function load() {
  isLoading.value = true
  try {
    const p = new URLSearchParams({ offset: String(offset.value), limit: String(limit) })
    if (filters.tag) p.set('tag', filters.tag)
    if (filters.keyword) p.set('keyword', filters.keyword)
    const res = await fetch('/api/potential_users?'+p, { headers: authH() })
    const d = await res.json()
    if (d.status==='ok') {
      users.value = d.data || []
      total.value = d.total || 0
      // 预加载头像
      const fids = users.value.map((u: any) => u.avatar_file_id).filter(Boolean)
      await fileStore.preloadFiles(fids)
    }
  } finally { isLoading.value=false }
}
async function loadTags() { const res=await fetch('/api/potential_users/tags',{headers:authH()}); const d=await res.json(); if(d.status==='ok') availableTags.value=d.data||[] }
async function sync() { isSyncing.value=true; try { await fetch('/api/potential_users/sync',{method:'POST',headers:authH()}); await load() } finally { isSyncing.value=false } }
async function promote(userId:number) { await fetch(`/api/potential_users/${userId}/promote`,{method:'POST',headers:authH()}); await load() }
function changePage(dir:'prev'|'next') { if(dir==='prev') offset.value=Math.max(0,offset.value-limit); else offset.value+=limit; load() }
function formatTime(ts:string|null|undefined):string { if(!ts) return '-'; const d=new Date(ts.replace(' ','T')); return isNaN(d.getTime())?'-':d.toLocaleString('zh-CN',{month:'2-digit',day:'2-digit',hour:'2-digit',minute:'2-digit'}) }
</script>
