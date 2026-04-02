<template>
  <div class="app-shell bg-surface text-on-surface antialiased overflow-hidden">
    <!-- Login Screen -->
    <div v-if="!auth.isAuthenticated" class="flex items-center justify-center min-h-screen cartesian-grid">
      <div class="bg-surface-container-lowest border border-outline-variant/20 rounded-3xl p-10 w-full max-w-sm mx-4 shadow-2xl shadow-primary/10">
        <div class="flex items-center justify-center mb-6">
          <span class="material-symbols-outlined text-primary text-5xl mr-3">smart_toy</span>
          <div>
            <div class="text-primary font-black uppercase tracking-widest text-xs">Bot Admin</div>
            <div class="text-2xl font-black">管理后台</div>
          </div>
        </div>
        <div v-if="auth.isLoading" class="flex flex-col items-center py-6">
          <div class="w-10 h-10 border-4 border-primary/20 border-t-primary rounded-full animate-spin mb-3"></div>
          <p class="text-sm text-slate-400">正在登录...</p>
        </div>
        <template v-else>
          <input v-model="tokenInput" type="password" placeholder="输入管理员 Token"
            class="w-full bg-surface-container border-none rounded-2xl px-4 py-3 mb-3 text-sm focus:ring-2 focus:ring-primary/20 transition-all"
            @keyup.enter="doLogin"/>
          <div v-if="auth.error" class="text-error text-sm mb-3 text-center">{{ auth.error }}</div>
          <button @click="doLogin"
            class="w-full bg-primary text-on-primary rounded-2xl py-3 font-black shadow-xl shadow-primary/30 hover:brightness-105 hover:scale-[1.01] active:scale-[0.98] transition-all">登录</button>
        </template>
      </div>
    </div>

    <!-- Main App -->
    <div v-else class="flex h-full w-full relative flex-col md:flex-row">
      <!-- Sidebar -->
      <aside class="hidden md:flex flex-col h-full bg-surface-container w-20 lg:w-72 z-20 transition-all duration-100 ease-in-out border-r border-outline-variant/20">
        <div class="flex items-center px-6 py-6 border-b border-outline-variant/10">
          <span class="material-symbols-outlined text-primary text-3xl mr-3">smart_toy</span>
          <div class="hidden lg:block">
            <div class="text-primary font-black uppercase tracking-widest text-xs">Bot Admin</div>
            <div class="text-[10px] text-slate-400 mt-0.5">来访用户管理系统</div>
          </div>
        </div>
        <nav class="flex-1 flex flex-col px-2 py-4 space-y-1 overflow-y-auto no-scrollbar">
          <div class="px-4 mb-2 hidden lg:block text-[10px] font-bold text-slate-400 uppercase tracking-widest">消息中心</div>
          <a href="#" @click.prevent="activeTab='chat'" class="flex items-center px-4 py-3 rounded-xl group transition-all duration-200 active:scale-95"
             :class="activeTab==='chat'?'bg-primary/10 text-primary shadow-sm':'text-on-surface-variant hover:bg-surface-container-high'">
            <span class="material-symbols-outlined mr-3">chat</span><span class="hidden lg:block font-bold text-sm">会话列表</span>
          </a>
          <a href="#" @click.prevent="activeTab='broadcast'" class="flex items-center px-4 py-3 rounded-xl group transition-all duration-200 active:scale-95"
             :class="activeTab==='broadcast'?'bg-primary/10 text-primary shadow-sm':'text-on-surface-variant hover:bg-surface-container-high'">
            <span class="material-symbols-outlined mr-3">campaign</span><span class="hidden lg:block font-bold text-sm">群发功能</span>
          </a>
          <a href="#" @click.prevent="activeTab='onchain_finance'" class="flex items-center px-4 py-3 rounded-xl group transition-all duration-200 active:scale-95"
             :class="activeTab==='onchain_finance'?'bg-primary/10 text-primary shadow-sm':'text-on-surface-variant hover:bg-surface-container-high'">
            <span class="material-symbols-outlined mr-3">account_balance_wallet</span><span class="hidden lg:block font-bold text-sm">链上财务</span>
          </a>
          <a href="#" @click.prevent="activeTab='settings'" class="flex items-center px-4 py-3 rounded-xl group transition-all duration-200 active:scale-95"
             :class="activeTab==='settings'?'bg-primary/10 text-primary shadow-sm':'text-on-surface-variant hover:bg-surface-container-high'">
            <span class="material-symbols-outlined mr-3">settings</span><span class="hidden lg:block font-bold text-sm">系统设置</span>
          </a>
          <a href="#" @click.prevent="activeTab='potential_users'" class="flex items-center px-4 py-3 rounded-xl group transition-all duration-200 active:scale-95"
             :class="activeTab==='potential_users'?'bg-primary/10 text-primary shadow-sm':'text-on-surface-variant hover:bg-surface-container-high'">
            <span class="material-symbols-outlined mr-3">person_add</span><span class="hidden lg:block font-bold text-sm">潜在用户</span>
          </a>
          <a href="#" @click.prevent="activeTab='group_management'" class="flex items-center px-4 py-3 rounded-xl group transition-all duration-200 active:scale-95"
             :class="activeTab==='group_management'?'bg-primary/10 text-primary shadow-sm':'text-on-surface-variant hover:bg-surface-container-high'">
            <span class="material-symbols-outlined mr-3">groups</span><span class="hidden lg:block font-bold text-sm">群组管理</span>
          </a>
          <div class="flex-1"></div>
          <a href="#" @click.prevent="auth.logout" class="flex items-center px-4 py-3 rounded-xl text-on-surface-variant hover:bg-error/10 hover:text-error transition-all duration-200">
            <span class="material-symbols-outlined mr-3">logout</span><span class="hidden lg:block font-bold text-sm">退出登录</span>
          </a>
        </nav>
        <div class="p-4 border-t border-outline-variant/10">
          <div class="bg-surface-container-high rounded-2xl p-4 hidden lg:block">
            <div class="text-[10px] font-bold text-primary uppercase tracking-tighter mb-1">当前环境</div>
            <div class="text-xs font-bold truncate">Production 2026</div>
            <div class="flex items-center gap-1.5 mt-2">
              <span class="w-2 h-2 rounded-full" :class="wsStore.connected?'bg-emerald-500 animate-pulse':'bg-slate-300'"></span>
              <span class="text-[9px] font-bold" :class="wsStore.connected?'text-emerald-600':'text-slate-400'">{{ wsStore.connected?'实时推送已连接':'推送未连接' }}</span>
            </div>
          </div>
        </div>
      </aside>

      <!-- Mobile Bottom Tab Bar -->
      <nav class="md:hidden fixed bottom-0 left-0 right-0 z-50 bg-white border-t border-slate-200 flex items-stretch">
        <a href="#" @click.prevent="activeTab='chat'" class="flex-1 flex flex-col items-center justify-center py-2 gap-0.5 transition-all" :class="activeTab==='chat'?'text-primary':'text-slate-400'">
          <span class="material-symbols-outlined text-[22px]">chat</span><span class="text-[10px] font-bold">会话</span>
        </a>
        <a href="#" @click.prevent="activeTab='group_management'" class="flex-1 flex flex-col items-center justify-center py-2 gap-0.5 transition-all" :class="activeTab==='group_management'?'text-primary':'text-slate-400'">
          <span class="material-symbols-outlined text-[22px]">groups</span><span class="text-[10px] font-bold">群组</span>
        </a>
        <a href="#" @click.prevent="activeTab='broadcast'" class="flex-1 flex flex-col items-center justify-center py-2 gap-0.5 transition-all" :class="activeTab==='broadcast'?'text-primary':'text-slate-400'">
          <span class="material-symbols-outlined text-[22px]">campaign</span><span class="text-[10px] font-bold">群发</span>
        </a>
        <a href="#" @click.prevent="activeTab='potential_users'" class="flex-1 flex flex-col items-center justify-center py-2 gap-0.5 transition-all" :class="activeTab==='potential_users'?'text-primary':'text-slate-400'">
          <span class="material-symbols-outlined text-[22px]">person_add</span><span class="text-[10px] font-bold">潜在</span>
        </a>
        <a href="#" @click.prevent="activeTab='onchain_finance'" class="flex-1 flex flex-col items-center justify-center py-2 gap-0.5 transition-all" :class="activeTab==='onchain_finance'?'text-primary':'text-slate-400'">
          <span class="material-symbols-outlined text-[22px]">account_balance_wallet</span><span class="text-[10px] font-bold">链上财务</span>
        </a>
        <a href="#" @click.prevent="activeTab='settings'" class="flex-1 flex flex-col items-center justify-center py-2 gap-0.5 transition-all" :class="activeTab==='settings'?'text-primary':'text-slate-400'">
          <span class="material-symbols-outlined text-[22px]">settings</span><span class="text-[10px] font-bold">设置</span>
        </a>
      </nav>

      <!-- Main Content -->
      <main class="flex-1 flex flex-col relative h-full bg-surface overflow-hidden pb-16 md:pb-0">
        <ChatPanel v-if="activeTab==='chat'" />
        <GroupPanel v-else-if="activeTab==='group_management'" />
        <BroadcastPanel v-else-if="activeTab==='broadcast'" />
        <PotentialPanel v-else-if="activeTab==='potential_users'" @chat="onChatFromPotential" />
        <OnChainFinancePanel v-else-if="activeTab==='onchain_finance'" />
        <SettingsView v-else-if="activeTab==='settings'" />
      </main>
    </div>
  </div>
</template>

<script setup lang="ts">
import { ref, onMounted, onBeforeUnmount, watch } from 'vue'
import { useAuthStore } from '@/stores/auth'
import { useChatStore } from '@/stores/chat'
import { useWsStore } from '@/stores/ws'
import ChatPanel from '@/views/ChatPanel.vue'
import GroupPanel from '@/views/GroupPanel.vue'
import BroadcastPanel from '@/views/BroadcastPanel.vue'
import PotentialPanel from '@/views/PotentialPanel.vue'
import OnChainFinancePanel from '@/views/OnChainFinancePanel.vue'
import SettingsView from '@/views/SettingsView.vue'

const auth = useAuthStore()
const chatStore = useChatStore()
const wsStore = useWsStore()
const tokenInput = ref('')
const activeTab = ref('chat')
let viewportBaseHeight = 0

function syncViewportHeight() {
  const vv = window.visualViewport
  const height = vv ? vv.height : window.innerHeight
  if (!viewportBaseHeight || height > viewportBaseHeight) viewportBaseHeight = height
  const keyboardOffset = Math.max(0, viewportBaseHeight - height)
  document.documentElement.style.setProperty('--app-vh', `${height * 0.01}px`)
  document.documentElement.style.setProperty('--kb-offset', `${keyboardOffset}px`)
}

// Connect WS when authenticated, disconnect on logout
watch(() => auth.isAuthenticated, (authed) => {
  if (authed) wsStore.connect()
  else wsStore.disconnect()
}, { immediate: true })

onMounted(() => {
  const params = new URLSearchParams(location.search)
  const otp = params.get('otp')
  if (otp) {
    history.replaceState({}, '', location.pathname)
    auth.loginWithOtp(otp)
    return
  }
  auth.checkToken()

  syncViewportHeight()
  window.addEventListener('resize', syncViewportHeight)
  window.visualViewport?.addEventListener('resize', syncViewportHeight)

  // 网络状态监听：手机端网络恢复时自动重连 WebSocket
  window.addEventListener('online', () => {
    console.log('[Network] 网络已恢复，重新连接 WebSocket')
    if (auth.isAuthenticated) {
      wsStore.disconnect()
      setTimeout(() => wsStore.connect(), 500)
    }
  })

  window.addEventListener('offline', () => {
    console.log('[Network] 网络已断开')
    wsStore.disconnect()
  })
})

onBeforeUnmount(() => {
  window.removeEventListener('resize', syncViewportHeight)
  window.visualViewport?.removeEventListener('resize', syncViewportHeight)
})

async function doLogin() {
  if (!tokenInput.value.trim()) return
  await auth.login(tokenInput.value.trim())
}

function onChatFromPotential(userId: number, name: string) {
  chatStore.selectedUserId = userId
  chatStore.selectedUserName = name
  activeTab.value = 'chat'
}
</script>

<style scoped>
.app-shell {
  height: calc(var(--app-vh, 1vh) * 100);
  min-height: calc(var(--app-vh, 1vh) * 100);
}
</style>
