<template>
  <article class="uc-card payment">
    <h3 class="uc-card-title">支付凭证</h3>
    <p class="uc-card-subtitle">查看订单状态，提交支付凭证</p>

    <div v-if="!order" class="uc-empty">
      暂无待支付订单，请先在套餐选择中创建订单
    </div>

    <div v-else class="order-detail">
      <div class="order-header">
        <span class="order-id">订单 #{{ order.id }}</span>
        <span class="order-status" :class="orderStatusClass">{{ orderStatusText }}</span>
      </div>

      <div class="order-info">
        <div class="uc-grid uc-grid-cols-2" style="gap: 8px">
          <div class="uc-stat">
            <span class="uc-stat-label">套餐</span>
            <div class="uc-stat-value">{{ order.plan_code }}</div>
          </div>
          <div class="uc-stat">
            <span class="uc-stat-label">金额</span>
            <div class="uc-stat-value text-primary">{{ order.amount_usdt }} USDT</div>
          </div>
        </div>
        
        <div v-if="order.payment_address" class="uc-field" style="margin-top: 12px">
          <span class="uc-label">支付地址</span>
          <code class="code-block">{{ order.payment_address }}</code>
        </div>
        
        <div v-if="order.memo" class="uc-field">
          <span class="uc-label">备注/Memo</span>
          <code class="code-block">{{ order.memo }}</code>
        </div>
        
        <div v-if="order.tx_hash" class="uc-field">
          <span class="uc-label">交易哈希</span>
          <code class="code-block tx-hash">{{ order.tx_hash }}</code>
        </div>
        
        <div v-if="order.created_at" class="uc-stat" style="margin-top: 8px">
          <span class="uc-stat-label">创建时间</span>
          <div class="uc-stat-value">{{ formatTime(order.created_at) }}</div>
        </div>
      </div>

      <div v-if="canSubmitTx" class="action-section">
        <div class="input-row">
          <input
            v-model="txInput"
            type="text"
            class="uc-input"
            placeholder="请输入交易哈希 (tx_hash)"
            :disabled="submitting"
            @keyup.enter="submitTx"
          />
          <button class="uc-btn uc-btn-primary" :disabled="submitting || !txInput" @click="submitTx">
            {{ submitting ? '提交中...' : '提交 TX' }}
          </button>
        </div>

        <button class="uc-btn uc-btn-secondary" :disabled="marking" @click="markPaid">
          {{ marking ? '标记中...' : '我已付款' }}
        </button>
      </div>

      <p v-if="message" class="message" :class="messageType">{{ message }}</p>
    </div>
  </article>
</template>

<script setup lang="ts">
import { ref, computed, onMounted, watch } from 'vue'
import { api } from '@/api'

const props = defineProps<{
  triggerRefresh?: number
}>()

const emit = defineEmits<{ paid: [] }>()

const order = ref<Record<string, any> | null>(null)
const txInput = ref('')
const submitting = ref(false)
const marking = ref(false)
const message = ref('')
const messageType = ref<'success' | 'error'>('success')

const statusMap: Record<string, { text: string; class: string }> = {
  pending: { text: '待支付', class: 'status-pending' },
  user_submitted: { text: '已提交凭证', class: 'status-submitted' },
  paid_pending_review: { text: '待平台确认', class: 'status-review' },
  confirmed: { text: '已确认', class: 'status-confirmed' },
  rejected: { text: '已拒绝', class: 'status-rejected' },
  cancelled: { text: '已取消', class: 'status-cancelled' },
}

const orderStatusText = computed(() => {
  const status = order.value?.status || 'pending'
  return statusMap[status]?.text || status
})

const orderStatusClass = computed(() => {
  const status = order.value?.status || 'pending'
  return statusMap[status]?.class || 'status-pending'
})

const canSubmitTx = computed(() => {
  const status = order.value?.status
  return status === 'pending' || status === 'user_submitted'
})

function formatTime(time: string): string {
  if (!time) return '-'
  const date = new Date(time)
  return Number.isNaN(date.getTime()) ? time : date.toLocaleString('zh-CN')
}

async function loadOrder() {
  try {
    const res = await api.getMerchantLatestPaymentOrder()
    order.value = res?.data ?? null
  } catch (e) {
    console.error('加载订单失败', e)
  }
}

async function submitTx() {
  if (!txInput.value || !order.value?.id || submitting.value) return

  submitting.value = true
  message.value = ''

  try {
    const res = await api.submitMerchantPaymentTx({
      order_id: order.value.id,
      tx_hash: txInput.value
    })
    if (res?.status === 'ok') {
      message.value = '交易哈希提交成功！'
      messageType.value = 'success'
      txInput.value = ''
      await loadOrder()
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

async function markPaid() {
  if (!order.value?.id || marking.value) return

  marking.value = true
  message.value = ''

  try {
    const res = await api.markMerchantPaymentOrderPaid(order.value.id)
    if (res?.status === 'ok') {
      message.value = '已标记为待审核，等待平台确认'
      messageType.value = 'success'
      emit('paid')
      await loadOrder()
    } else {
      message.value = res?.message || '标记失败'
      messageType.value = 'error'
    }
  } catch (e: any) {
    message.value = e?.message || '标记失败，请重试'
    messageType.value = 'error'
  } finally {
    marking.value = false
  }
}

watch(() => props.triggerRefresh, () => { void loadOrder() })

onMounted(() => { void loadOrder() })

defineExpose({ loadOrder })
</script>

<style scoped>
.payment {
  display: flex;
  flex-direction: column;
  gap: 12px;
}

.order-header {
  display: flex;
  justify-content: space-between;
  align-items: center;
  padding-bottom: 12px;
  border-bottom: 1px solid rgb(148 163 184 / 0.18);
}

.dark .order-header {
  border-bottom-color: rgb(71 85 105 / 0.22);
}

.order-id {
  font-size: 16px;
  font-weight: 700;
  color: #111827;
}

.dark .order-id {
  color: #f1f5f9;
}

.order-status {
  padding: 4px 12px;
  border-radius: 999px;
  font-size: 12px;
  font-weight: 700;
}
.status-pending { background: rgba(245,158,11,.12); color: #b45309; }
.status-submitted { background: rgba(59,130,246,.12); color: #1d4ed8; }
.status-review { background: rgba(139,92,246,.12); color: #7c3aed; }
.status-confirmed { background: rgba(16,185,129,.12); color: #047857; }
.status-rejected { background: rgba(239,68,68,.12); color: #b91c1c; }
.status-cancelled { background: rgba(107,114,128,.12); color: #4b5563; }

.dark .status-pending { background: rgba(245,158,11,.18); color: #fbbf24; }
.dark .status-submitted { background: rgba(59,130,246,.18); color: #60a5fa; }
.dark .status-review { background: rgba(139,92,246,.18); color: #a78bfa; }
.dark .status-confirmed { background: rgba(16,185,129,.18); color: #6ee7b7; }
.dark .status-rejected { background: rgba(239,68,68,.18); color: #fca5a5; }
.dark .status-cancelled { background: rgba(107,114,128,.18); color: #9ca3af; }

.order-info {
  padding: 8px 0;
}

.code-block {
  font-size: 12px;
  color: #111827;
  background: rgb(248 250 252 / 0.8);
  padding: 8px 12px;
  border-radius: 8px;
  word-break: break-all;
  font-family: monospace;
  display: block;
  margin-top: 4px;
}

.dark .code-block {
  background: rgb(30 41 59 / 0.5);
  color: #e2e8f0;
}

.tx-hash {
  font-size: 11px;
}

.action-section {
  display: flex;
  flex-direction: column;
  gap: 12px;
  margin-top: 8px;
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
  background: rgba(16,185,129,.1);
  color: #047857;
}
.message.error {
  background: rgba(239,68,68,.1);
  color: #b91c1c;
}

.dark .message.success {
  background: rgba(16,185,129,.15);
  color: #6ee7b7;
}
.dark .message.error {
  background: rgba(239,68,68,.15);
  color: #fca5a5;
}
</style>
