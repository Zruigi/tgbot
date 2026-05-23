<template>
  <div class="flex items-center justify-center min-h-screen cartesian-grid">
    <div class="bg-surface-container-lowest border border-outline-variant/20 rounded-3xl p-10 w-full max-w-sm mx-4 shadow-2xl shadow-primary/10">
      <div class="flex items-center justify-center mb-6">
        <span class="material-symbols-outlined text-primary text-5xl mr-3">smart_toy</span>
        <div>
          <div class="text-primary font-black uppercase tracking-widest text-xs">Bot Admin</div>
          <div class="text-2xl font-black">管理后台</div>
        </div>
      </div>

      <div v-if="sessionRole === 'unregistered'" class="text-center space-y-4">
        <p class="text-sm text-on-surface-variant">当前 Telegram 账号还没有商户档案。</p>
        <p class="text-xs text-slate-400">请先回到机器人里发送注册命令，完成商户接入后再进入 Mini App。</p>
        <button class="login-primary-btn" @click="$emit('reopenTelegram')">
          返回 Telegram
        </button>
      </div>

      <div v-if="miniappStatus === 'reopen_in_telegram_required'" class="text-center space-y-4">
        <p class="text-sm text-on-surface-variant">{{ miniappMessage || '当前浏览器入口已失效，请返回 Telegram 重新打开 Mini App。' }}</p>
        <button class="login-primary-btn" @click="$emit('reopenTelegram')">
          返回 Telegram
        </button>
      </div>

      <div v-if="isLoading" class="flex flex-col items-center py-6">
        <div class="w-10 h-10 border-4 border-primary/20 border-t-primary rounded-full animate-spin mb-3"></div>
        <p class="text-sm text-slate-400">正在登录...</p>
      </div>

      <template v-else-if="miniappStatus !== 'reopen_in_telegram_required' && sessionRole !== 'unregistered'">
        <input
          v-model="tokenInput"
          type="password"
          placeholder="输入管理员 Token"
          class="w-full bg-surface-container border-none rounded-2xl px-4 py-3 mb-3 text-sm focus:ring-2 focus:ring-primary/20 transition-all"
          @keyup.enter="submit"
        />
        <div v-if="error" class="text-error text-sm mb-3 text-center">{{ error }}</div>
        <button class="login-primary-btn" @click="submit">登录</button>
      </template>
    </div>
  </div>
</template>

<script setup lang="ts">
import { ref } from 'vue'

defineProps<{
  isLoading: boolean
  error: string
  miniappStatus: string
  miniappMessage: string
  sessionRole?: string
}>()

const emit = defineEmits<{
  login: [token: string]
  reopenTelegram: []
}>()

const tokenInput = ref('')

function submit() {
  const token = tokenInput.value.trim()
  if (!token) return
  emit('login', token)
}
</script>

<style scoped>
.login-primary-btn {
  width: 100%;
  border-radius: 16px;
  padding: 12px 16px;
  background: var(--color-primary, #24a1de);
  color: var(--color-on-primary, #fff);
  font-weight: 900;
  box-shadow: 0 18px 32px rgba(36, 161, 222, 0.24);
  transition: transform 0.18s ease, filter 0.18s ease;
}

.login-primary-btn:hover {
  filter: brightness(1.05);
  transform: scale(1.01);
}

.login-primary-btn:active {
  transform: scale(0.98);
}
</style>
