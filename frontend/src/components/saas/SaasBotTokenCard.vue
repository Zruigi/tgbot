<template>
  <article class="uc-card bot-token">
    <div class="bot-token-head">
      <div>
        <h3 class="uc-card-title">Bot 接入</h3>
        <p class="uc-card-subtitle">提交 Telegram Bot Token 以完成接入</p>
      </div>
      <span class="bot-state" :class="stateClass">{{ stateText }}</span>
    </div>

    <div class="bot-status-card">
      <span class="material-symbols-outlined">storefront</span>
      <div>
        <strong>{{ statusTitle }}</strong>
        <p>接入后才能开启会话、群聊、群发与财务</p>
      </div>
    </div>

    <div v-if="isValidating" class="validation-progress">
      <div class="validation-progress__head">
        <span class="material-symbols-outlined spinning">sync</span>
        <strong>TOKEN_VALIDATING · 预计 5-10 秒</strong>
      </div>
      <div class="validation-track">
        <span></span>
      </div>
      <p>正在检查 Bot 权限与 webhook，请稍候。</p>
    </div>
    
    <div class="input-row">
      <input
        v-model="tokenInput"
        type="password"
        class="uc-input"
        placeholder="请输入 Bot Token (如 123456:ABC-DEF...)"
        :disabled="submitting"
        @keyup.enter="handleSubmit"
      />
      <button class="uc-btn uc-btn-primary" :disabled="submitting || !tokenInput" @click="handleSubmit">
        {{ submitting ? '提交中...' : '提交' }}
      </button>
    </div>
    
    <p v-if="message" class="message" :class="messageType">{{ message }}</p>

    <div class="bot-timeline">
      <div
        v-for="(step, index) in timelineSteps"
        :key="step.key"
        class="timeline-step"
        :class="{ done: step.done, current: step.current }"
      >
        <span class="timeline-dot">
          <span v-if="step.done" class="material-symbols-outlined">check</span>
          <span v-else>{{ index + 1 }}</span>
        </span>
        <div>
          <strong>{{ step.title }}</strong>
          <p>{{ step.desc }}</p>
          <button
            v-if="step.key === 'botfather'"
            class="timeline-link"
            type="button"
            @click="openBotFather"
          >
            <span class="material-symbols-outlined">open_in_new</span>
            打开 BotFather
          </button>
        </div>
      </div>
    </div>
    
    <div v-if="currentToken" class="current-token">
      <div class="uc-grid uc-grid-cols-2" style="gap: 8px">
        <div class="uc-stat">
          <span class="uc-stat-label">当前 Token</span>
          <div class="uc-stat-value" style="font-family: monospace; font-size: 12px">{{ maskToken(currentToken) }}</div>
        </div>
      </div>
    </div>
  </article>
</template>

<script setup lang="ts">
import { computed, ref } from 'vue'
import { api } from '@/api'
import { openExternalLink } from '@/lib/telegram'
import type { MerchantState } from '@/lib/merchantState'

const props = defineProps<{
  currentToken?: string | null
  currentState?: MerchantState
}>()

const emit = defineEmits<{ saved: [] }>()

const tokenInput = ref('')
const submitting = ref(false)
const message = ref('')
const messageType = ref<'success' | 'error'>('success')

const isValidating = computed(() => props.currentState === 'TOKEN_VALIDATING' || submitting.value)
const stateText = computed(() => {
  if (isValidating.value) return '校验中'
  if (props.currentToken) return '已提交'
  return '等待 Token'
})
const stateClass = computed(() => {
  if (messageType.value === 'error' && message.value) return 'error'
  if (isValidating.value) return 'validating'
  if (props.currentToken) return 'ready'
  return 'waiting'
})
const statusTitle = computed(() => {
  if (isValidating.value) return '正在校验 Bot Token'
  if (props.currentToken) return 'Bot Token 已提交'
  return '等待 Bot Token'
})

const timelineSteps = computed(() => {
  const hasToken = Boolean(props.currentToken || tokenInput.value)
  const validating = isValidating.value
  const completed = Boolean(props.currentToken && !validating)
  return [
    { key: 'botfather', title: '创建或打开 Telegram BotFather', desc: '从官方 BotFather 获取 Bot Token', done: true, current: false },
    { key: 'copy', title: '复制 Bot Token', desc: '只从 BotFather 粘贴，避免复制错误', done: hasToken || completed, current: !hasToken },
    { key: 'paste', title: '粘贴 Bot Token', desc: '提交后系统会自动校验权限', done: hasToken || completed, current: hasToken && !validating && !completed },
    { key: 'validate', title: '系统校验', desc: '检查 Bot 权限与 webhook', done: completed, current: validating },
    { key: 'done', title: '完成接入并进入工作台', desc: '接入完成后开启业务功能', done: completed, current: false },
  ]
})

function maskToken(token: string): string {
  if (token.length <= 12) return '***'
  return token.slice(0, 6) + '...' + token.slice(-4)
}

async function handleSubmit() {
  if (!tokenInput.value || submitting.value) return

  submitting.value = true
  message.value = ''

  try {
    const res = await api.saveMerchantBotToken({ bot_token: tokenInput.value })
    if (res?.status === 'ok') {
      message.value = 'Bot Token 提交成功！'
      messageType.value = 'success'
      tokenInput.value = ''
      emit('saved')
    } else {
      message.value = res?.message || '提交失败'
      messageType.value = 'error'
    }
  } catch (e: any) {
    message.value = e?.message || '提交失败，请重试'
    messageType.value = 'error'
  } finally {
    submitting.value = false
  }
}

function openBotFather() {
  openExternalLink('https://t.me/BotFather')
}
</script>

<style scoped>
.bot-token {
  display: flex;
  flex-direction: column;
  gap: 12px;
}

.bot-token-head {
  display: flex;
  align-items: flex-start;
  justify-content: space-between;
  gap: 12px;
}

.bot-state {
  display: inline-flex;
  padding: 4px 9px;
  border-radius: 999px;
  font-size: 11px;
  font-weight: 900;
  white-space: nowrap;
}

.bot-state.waiting {
  background: rgba(245, 158, 11, 0.12);
  color: #b45309;
}

.bot-state.validating {
  background: rgba(36, 161, 222, 0.12);
  color: #0f6fb2;
}

.bot-state.ready {
  background: rgba(16, 185, 129, 0.12);
  color: #047857;
}

.bot-state.error {
  background: rgba(239, 68, 68, 0.12);
  color: #b91c1c;
}

.dark .bot-state.waiting { color: #fbbf24; }
.dark .bot-state.validating { color: #8fd8ff; }
.dark .bot-state.ready { color: #6ee7b7; }
.dark .bot-state.error { color: #fca5a5; }

.bot-status-card {
  display: flex;
  align-items: flex-start;
  gap: 11px;
  padding: 13px;
  border-radius: 13px;
  border: 1px solid rgba(36, 161, 222, 0.16);
  background: rgba(231, 246, 255, 0.72);
}

.dark .bot-status-card {
  border-color: rgba(36, 161, 222, 0.24);
  background: rgba(10, 43, 73, 0.42);
}

.bot-status-card > .material-symbols-outlined {
  color: #0f6fb2;
}

.bot-status-card strong {
  display: block;
  font-size: 14px;
  color: #0f172a;
}

.dark .bot-status-card strong {
  color: #edf4ff;
}

.bot-status-card p {
  margin: 3px 0 0;
  color: #64748b;
  font-size: 12px;
}

.dark .bot-status-card p {
  color: #adc3e0;
}

.validation-progress {
  border: 1px solid rgba(36, 161, 222, 0.18);
  border-radius: 13px;
  background: rgb(248 250 252 / 0.82);
  padding: 12px;
}

.dark .validation-progress {
  border-color: rgba(36, 161, 222, 0.24);
  background: rgb(9 20 35 / 0.74);
}

.validation-progress__head {
  display: flex;
  align-items: center;
  gap: 8px;
  color: #0f6fb2;
  font-size: 12px;
}

.validation-track {
  height: 7px;
  border-radius: 999px;
  overflow: hidden;
  margin-top: 10px;
  background: rgb(226 232 240 / 0.9);
}

.dark .validation-track {
  background: rgb(51 65 85 / 0.56);
}

.validation-track span {
  display: block;
  width: 68%;
  height: 100%;
  border-radius: inherit;
  background: linear-gradient(90deg, #24a1de, #10b981);
}

.validation-progress p {
  margin: 8px 0 0;
  font-size: 12px;
  color: #64748b;
}

.dark .validation-progress p {
  color: #adc3e0;
}

.spinning {
  animation: bot-spin 0.8s linear infinite;
}

@keyframes bot-spin {
  to { transform: rotate(360deg); }
}

.input-row {
  display: flex;
  gap: 12px;
  align-items: flex-start;
}

.input-row .uc-input {
  flex: 1;
}

.message {
  margin: 0;
  padding: 10px 14px;
  border-radius: 10px;
  font-size: 14px;
}
.message.success {
  background: rgba(36,161,222,.12);
  color: #0f6fb2;
}
.message.error {
  background: rgba(239,68,68,.1);
  color: #b91c1c;
}

.dark .message.success {
  background: rgba(36,161,222,.18);
  color: #8fd8ff;
}
.dark .message.error {
  background: rgba(239,68,68,.15);
  color: #fca5a5;
}

.current-token {
  margin-top: 8px;
  padding: 12px;
  background: rgb(248 250 252 / 0.8);
  border-radius: 12px;
}

.dark .current-token {
  background: rgb(30 41 59 / 0.5);
}

.bot-timeline {
  display: flex;
  flex-direction: column;
  gap: 0;
  border: 1px solid rgb(148 163 184 / 0.16);
  border-radius: 14px;
  padding: 12px;
  background: rgb(255 255 255 / 0.72);
}

.dark .bot-timeline {
  border-color: rgb(73 106 148 / 0.24);
  background: rgb(9 20 35 / 0.58);
}

.timeline-step {
  display: grid;
  grid-template-columns: 28px 1fr;
  gap: 10px;
  position: relative;
  padding-bottom: 16px;
}

.timeline-step:last-child {
  padding-bottom: 0;
}

.timeline-step:not(:last-child)::before {
  content: "";
  position: absolute;
  left: 13px;
  top: 27px;
  bottom: 2px;
  width: 2px;
  border-radius: 999px;
  background: rgb(226 232 240);
}

.dark .timeline-step:not(:last-child)::before {
  background: rgb(51 65 85);
}

.timeline-dot {
  position: relative;
  z-index: 1;
  width: 28px;
  height: 28px;
  border-radius: 999px;
  display: inline-flex;
  align-items: center;
  justify-content: center;
  background: #e2e8f0;
  color: #64748b;
  font-size: 11px;
  font-weight: 900;
}

.timeline-step.done .timeline-dot {
  background: #24a1de;
  color: #fff;
}

.timeline-step.current .timeline-dot {
  background: #fff;
  color: #0f6fb2;
  border: 2px solid #24a1de;
  box-shadow: 0 0 0 4px rgba(36, 161, 222, 0.12);
}

.dark .timeline-step.current .timeline-dot {
  background: #08111f;
  color: #8fd8ff;
}

.timeline-dot .material-symbols-outlined {
  font-size: 15px;
}

.timeline-step strong {
  display: block;
  font-size: 13px;
  color: #0f172a;
}

.dark .timeline-step strong {
  color: #edf4ff;
}

.timeline-step p {
  margin: 2px 0 0;
  font-size: 12px;
  color: #64748b;
}

.dark .timeline-step p {
  color: #adc3e0;
}

.timeline-link {
  margin-top: 8px;
  min-height: 30px;
  border: 1px solid rgb(148 163 184 / 0.24);
  border-radius: 10px;
  background: rgb(248 250 252 / 0.9);
  color: #0f6fb2;
  display: inline-flex;
  align-items: center;
  gap: 5px;
  padding: 0 10px;
  font-size: 12px;
  font-weight: 800;
}

.timeline-link .material-symbols-outlined {
  font-size: 16px;
}

@media (max-width: 640px) {
  .bot-token-head,
  .input-row {
    flex-direction: column;
  }

  .input-row .uc-btn {
    width: 100%;
  }
}
</style>
