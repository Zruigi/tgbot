<template>
  <div class="page-body">
    <h2 style="margin-bottom:20px">群发推送</h2>
    <div class="card" style="max-width:560px">
      <div class="field">
        <label>消息内容</label>
        <textarea v-model="content" class="input" rows="4" placeholder="输入群发内容..."/>
      </div>
      <div class="field">
        <label>目标群组 ID（逗号分隔）</label>
        <input v-model="chatIdsRaw" class="input" placeholder="-100123456, -100789012" />
      </div>
      <div class="field-row">
        <label>
          <input type="checkbox" v-model="doTranslate" /> 自动翻译
        </label>
      </div>
      <button class="btn btn-primary" :disabled="sending" @click="send">
        {{ sending ? '发送中...' : '一键群发' }}
      </button>
      <div v-if="result" class="result-box">
        <span class="badge badge-green">成功 {{ result.success }}</span>
        <span class="badge badge-red" style="margin-left:8px">失败 {{ result.failed }}</span>
      </div>
    </div>
  </div>
</template>

<script setup lang="ts">
import { ref } from 'vue'
import { chatApi } from '@/api'

const content = ref('')
const chatIdsRaw = ref('')
const doTranslate = ref(false)
const sending = ref(false)
const result = ref<{success:number, failed:number} | null>(null)

async function send() {
  const chatIds = chatIdsRaw.value
    .split(',')
    .map(s => parseInt(s.trim()))
    .filter(n => !isNaN(n))
  if (!content.value.trim() || chatIds.length === 0) return
  sending.value = true
  try {
    const res = await chatApi.broadcastChats({ chat_ids: chatIds, content: content.value, do_translate: doTranslate.value })
    result.value = res.data.summary
  } finally {
    sending.value = false
  }
}
</script>

<style scoped>
.field { margin-bottom: 14px; }
.field label { display: block; font-size: 11px; font-weight: 600;
  color: var(--text-secondary); margin-bottom: 5px; text-transform: uppercase; }
.field-row { display: flex; align-items: center; gap: 8px; margin-bottom: 14px;
  font-size: 13px; color: var(--text-secondary); }
.result-box { margin-top: 14px; }
</style>
