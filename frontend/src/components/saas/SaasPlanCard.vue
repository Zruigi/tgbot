<template>
  <article class="uc-card plans">
    <h3 class="uc-card-title">套餐选择</h3>
    <p class="uc-card-subtitle">选择套餐以升级您的服务</p>
    
    <div v-if="loading" class="uc-empty">加载中...</div>
    
    <div v-else class="plans-grid">
      <div
        v-for="plan in plans"
        :key="plan.plan_code"
        class="plan-item"
        :class="{ active: selectedPlan === plan.plan_code, current: isCurrentPlan(plan.plan_code) }"
        @click="selectPlan(plan.plan_code)"
      >
        <div class="plan-header">
          <span class="plan-name">{{ plan.plan_name }}</span>
          <span v-if="isCurrentPlan(plan.plan_code)" class="current-badge">当前</span>
        </div>
        <div class="plan-price">
          <span class="price-value">{{ formatPrice(plan.price_usdt) }} USDT</span>
          <span class="price-duration">/{{ plan.duration_days }}天</span>
        </div>
        <div class="plan-features">
          <span v-for="(feat, idx) in parseFeatures(plan.features_json)" :key="idx" class="feat-tag">{{ feat }}</span>
        </div>
        <button
          v-if="!isCurrentPlan(plan.plan_code)"
          class="uc-btn uc-btn-primary"
          :disabled="creating || selectedPlan === plan.plan_code"
          @click.stop="createOrder(plan.plan_code)"
        >
          {{ creating && selectedPlan === plan.plan_code ? '创建中...' : '选择套餐' }}
        </button>
      </div>
    </div>
    
    <p v-if="message" class="message" :class="messageType">{{ message }}</p>
  </article>
</template>

<script setup lang="ts">
import { ref, onMounted } from 'vue'
import { api } from '@/api'

const emit = defineEmits<{ orderCreated: [orderId: number]; planActivated: [] }>()

const plans = ref<Record<string, any>[]>([])
const selectedPlan = ref<string>('')
const currentPlanCode = ref<string>('')
const loading = ref(false)
const creating = ref(false)
const message = ref('')
const messageType = ref<'success' | 'error'>('success')

function formatPrice(price: number | string): string {
  const num = typeof price === 'string' ? parseFloat(price) : price
  return isNaN(num) ? '0' : num.toFixed(2)
}

function parseFeatures(featuresJson: string): string[] {
  try {
    const obj = typeof featuresJson === 'string' ? JSON.parse(featuresJson) : featuresJson
    return Object.keys(obj || {}).filter(k => obj[k])
  } catch {
    return []
  }
}

function isCurrentPlan(planCode: string): boolean {
  return currentPlanCode.value === planCode
}

function selectPlan(code: string) {
  if (isCurrentPlan(code)) return
  selectedPlan.value = code
}

async function loadPlans() {
  loading.value = true
  try {
    const [plansRes, subRes, profileRes] = await Promise.all([
      api.getMerchantPlans(),
      api.getMerchantSubscription(),
      api.getMerchantProfile()
    ])
    plans.value = plansRes?.data ?? []
    currentPlanCode.value = subRes?.data?.plan_code ?? profileRes?.data?.plan_type ?? ''
  } catch (e) {
    console.error('加载套餐失败', e)
  } finally {
    loading.value = false
  }
}

async function createOrder(planCode: string) {
  creating.value = true
  selectedPlan.value = planCode
  message.value = ''

  try {
    if (planCode === 'free') {
      const res = await api.updateMerchantStatus({ status: 'active_free', onboarding_completed: true })
      if (res?.status === 'ok') {
        message.value = '免费套餐已启用'
        messageType.value = 'success'
        currentPlanCode.value = 'free'
        emit('planActivated')
        await loadPlans()
      } else {
        message.value = res?.message || '启用免费套餐失败'
        messageType.value = 'error'
      }
      return
    }

    const res = await api.createMerchantPaymentOrder({ plan_code: planCode })
    if (res?.status === 'ok' && res?.data?.id) {
      message.value = '订单创建成功！'
      messageType.value = 'success'
      emit('orderCreated', res.data.id)
    } else {
      message.value = res?.message || '创建订单失败'
      messageType.value = 'error'
    }
  } catch (e: any) {
    message.value = e?.message || '创建订单失败，请重试'
    messageType.value = 'error'
  } finally {
    creating.value = false
  }
}

onMounted(() => { void loadPlans() })

defineExpose({ loadPlans })
</script>

<style scoped>
.plans {
  display: flex;
  flex-direction: column;
  gap: 12px;
}

.plans-grid {
  display: grid;
  grid-template-columns: repeat(auto-fill, minmax(200px, 1fr));
  gap: 16px;
}

.plan-item {
  padding: 16px;
  border: 1px solid rgb(36 161 222 / 0.16);
  border-radius: 16px;
  background: linear-gradient(180deg, rgb(248 252 255 / 0.96), rgb(240 248 255 / 0.92));
  cursor: pointer;
  transition: all 0.2s;
  display: flex;
  flex-direction: column;
  gap: 8px;
}

.dark .plan-item {
  border-color: rgb(36 161 222 / 0.18);
  background: linear-gradient(180deg, rgb(9 19 34 / 0.92), rgb(12 28 49 / 0.88));
}

.plan-item:hover {
  border-color: rgb(36 161 222 / 0.35);
  transform: translateY(-2px);
  box-shadow: 0 18px 34px rgba(36, 161, 222, 0.10);
}

.dark .plan-item:hover {
  border-color: rgb(82 190 255 / 0.34);
  box-shadow: 0 18px 34px rgba(15, 71, 125, 0.24);
}

.plan-item.active {
  border-color: rgb(36 161 222 / 0.55);
  background: linear-gradient(180deg, rgba(222, 243, 255, 0.96), rgba(206, 236, 255, 0.9));
}

.dark .plan-item.active {
  background: linear-gradient(180deg, rgba(16, 52, 82, 0.92), rgba(13, 65, 104, 0.88));
}

.plan-item.current {
  border-color: rgb(36 161 222 / 0.45);
  background: linear-gradient(180deg, rgba(227, 246, 255, 0.96), rgba(216, 241, 255, 0.92));
}

.dark .plan-item.current {
  border-color: rgb(82 190 255 / 0.36);
  background: linear-gradient(180deg, rgba(12, 41, 66, 0.92), rgba(10, 52, 84, 0.88));
}

.plan-header {
  display: flex;
  justify-content: space-between;
  align-items: center;
}

.plan-name {
  font-size: 16px;
  font-weight: 700;
  color: #111827;
}

.dark .plan-name {
  color: #f1f5f9;
}

.current-badge {
  padding: 2px 8px;
  border-radius: 999px;
  background: linear-gradient(135deg, #249fe0, #1b78d2);
  color: #fff;
  font-size: 11px;
  font-weight: 700;
}

.dark .current-badge {
  background: linear-gradient(135deg, #39b8ff, #249fe0);
}

.plan-price {
  display: flex;
  align-items: baseline;
  gap: 4px;
}

.price-value {
  font-size: 24px;
  font-weight: 700;
  color: #111827;
}

.dark .price-value {
  color: #f1f5f9;
}

.price-duration {
  font-size: 14px;
  font-weight: 400;
  color: #6b7280;
}

.dark .price-duration {
  color: #9ca3af;
}

.plan-features {
  display: flex;
  flex-wrap: wrap;
  gap: 6px;
  margin: 4px 0;
}

.feat-tag {
  padding: 4px 8px;
  border-radius: 6px;
  background: rgba(36, 161, 222, 0.10);
  font-size: 12px;
  color: #155e8d;
}

.dark .feat-tag {
  background: rgba(36, 161, 222, 0.16);
  color: #a6dcff;
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

@media (max-width: 640px) {
  .plans-grid {
    grid-template-columns: 1fr;
  }
}
</style>
