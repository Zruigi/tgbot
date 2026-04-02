<template>
  <div class="login-page">
    <div class="login-card">
      <div class="logo">
        <span class="logo-icon">⚡</span>
        <h1>TGBot Enterprise</h1>
        <p>管理控制台</p>
      </div>
      <form @submit.prevent="handleLogin">
        <div class="field">
          <label>访问令牌</label>
          <input
            v-model="tokenInput"
            type="password"
            class="input"
            placeholder="输入 WEB_ADMIN_TOKEN"
            autocomplete="current-password"
          />
        </div>
        <button type="submit" class="btn btn-primary w-full" :disabled="loading">
          {{ loading ? '验证中...' : '登录' }}
        </button>
        <p v-if="error" class="error-msg">{{ error }}</p>
      </form>
    </div>
  </div>
</template>

<script setup lang="ts">
import { ref, onMounted } from 'vue'
import { useRouter } from 'vue-router'
import { useAuthStore } from '@/stores/auth'

const router = useRouter()
const auth = useAuthStore()
const tokenInput = ref('')
const loading = ref(false)
const error = ref('')

onMounted(async () => {
  // OTP 自动登录：从 URL 参数中读取 otp
  const params = new URLSearchParams(window.location.search)
  const otp = params.get('otp')
  if (otp) {
    loading.value = true
    try {
      await auth.loginWithOtp(otp)
      if (auth.isAuthenticated) {
        // 清除 URL 中的 otp 参数，防止刷新重用
        window.history.replaceState({}, '', window.location.pathname)
        router.push('/chat')
        return
      } else {
        error.value = auth.error || 'OTP 已失效，请重新从 Telegram 获取登录链接'
      }
    } finally {
      loading.value = false
    }
  }
  // 已有 token 则直接跳转
  auth.checkToken()
  if (auth.isAuthenticated) router.push('/chat')
})

async function handleLogin() {
  error.value = ''
  loading.value = true
  try {
    await auth.login(tokenInput.value)
    if (auth.isAuthenticated) router.push('/chat')
    else error.value = auth.error || 'Token 错误，请重试'
  } catch {
    error.value = 'Token 错误，请重试'
  } finally {
    loading.value = false
  }
}
</script>

<style scoped>
.login-page {
  min-height: 100vh;
  display: flex;
  align-items: center;
  justify-content: center;
  background:
    radial-gradient(ellipse 80% 60% at 50% -10%, rgba(88,166,255,.12) 0%, transparent 70%),
    var(--bg-base);
}
.login-card {
  width: 380px;
  background: var(--bg-surface);
  border: 1px solid var(--border);
  border-radius: var(--radius-lg);
  padding: 40px;
  box-shadow: var(--shadow-lg);
}
.logo { text-align: center; margin-bottom: 32px; }
.logo-icon { font-size: 40px; display: block; margin-bottom: 8px; }
.logo h1 { font-size: 22px; font-weight: 700; color: var(--text-primary); }
.logo p { font-size: 13px; color: var(--text-secondary); margin-top: 4px; }
.field { margin-bottom: 16px; }
.field label { display: block; font-size: 12px; font-weight: 600;
  color: var(--text-secondary); margin-bottom: 6px; text-transform: uppercase;
  letter-spacing: .05em; }
.w-full { width: 100%; justify-content: center; padding: 10px; font-size: 14px; }
.error-msg { margin-top: 12px; text-align: center; color: var(--danger); font-size: 13px; }
</style>
