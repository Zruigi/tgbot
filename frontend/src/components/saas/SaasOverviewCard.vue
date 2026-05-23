<template>
  <article class="uc-card overview">
    <div class="overview-header">
      <h3 class="uc-card-title">接入状态</h3>
      <span class="status-badge" :class="statusClass">{{ statusText }}</span>
    </div>
    
    <!-- 状态进度指示器 -->
    <div class="progress-bar">
      <div
        v-for="(step, index) in progressSteps"
        :key="step.key"
        class="progress-step"
        :class="{ completed: step.completed, current: step.current }"
      >
        <div class="step-dot">
          <span v-if="step.completed" class="material-symbols-outlined" style="font-size:14px">check</span>
          <span v-else>{{ index + 1 }}</span>
        </div>
        <span class="step-label">{{ step.label }}</span>
      </div>
    </div>
    
    <div class="uc-grid uc-grid-cols-3 overview-grid">
      <div class="uc-stat">
        <span class="uc-stat-label">商户名称</span>
        <div class="uc-stat-value">{{ merchantName }}</div>
      </div>
      <div class="uc-stat">
        <span class="uc-stat-label">商户 ID</span>
        <div class="uc-stat-value">{{ profile?.id ?? '-' }}</div>
      </div>
      <div class="uc-stat">
        <span class="uc-stat-label">租户 ID</span>
        <div class="uc-stat-value">{{ tenantIdText }}</div>
      </div>
      <div class="uc-stat">
        <span class="uc-stat-label">负责人 TG</span>
        <div class="uc-stat-value">{{ profile?.owner_tg_user_id ?? '-' }}</div>
      </div>
      <div class="uc-stat">
        <span class="uc-stat-label">Bot Token</span>
        <div class="uc-stat-value" :class="hasBotToken ? 'text-emerald-600' : 'text-amber-600'">
          {{ hasBotToken ? '已接入' : '未接入' }}
        </div>
      </div>
      <div class="uc-stat">
        <span class="uc-stat-label">当前套餐</span>
        <div class="uc-stat-value">{{ planName }}</div>
      </div>
    </div>
  </article>
</template>

<script setup lang="ts">
import { computed } from 'vue'
import { resolveMerchantState, type MerchantState } from '@/lib/merchantState'

const props = defineProps<{
  profile: Record<string, any> | null
  subscription: Record<string, any> | null
  currentState?: MerchantState
}>()

const merchantName = computed(() => props.profile?.merchant_name || '未命名商户')
const tenantIdText = computed(() => props.profile?.tenant_id || '-')
const planName = computed(() => {
  // active_free 状态本身就是已选择免费套餐
  if (effectiveState.value === 'ACTIVE_FREE') return '免费套餐'
  if (effectiveState.value === 'ACTIVE_PRO') return props.subscription?.plan_name || '付费套餐'
  return props.subscription?.plan_name || props.subscription?.plan_code || '未开通套餐'
})
const hasBotToken = computed(() => !!props.profile?.bot_token)

const statusMap: Record<string, { text: string; class: string }> = {
  INIT: { text: '初始化', class: 'status-init' },
  WAIT_BOT_TOKEN: { text: '待接入 Bot', class: 'status-wait' },
  TOKEN_VALIDATING: { text: '校验中', class: 'status-validating' },
  ACTIVE_FREE: { text: '免费套餐', class: 'status-free' },
  ACTIVE_PRO: { text: '付费套餐', class: 'status-pro' },
  EXPIRED_DOWNGRADED: { text: '已降级', class: 'status-expired' },
  SUSPENDED: { text: '已暂停', class: 'status-suspended' },
}

const effectiveState = computed<MerchantState>(() => {
  return props.currentState || resolveMerchantState(props.profile, props.subscription)
})

const statusText = computed(() => {
  return statusMap[effectiveState.value]?.text || effectiveState.value
})

const statusClass = computed(() => {
  return statusMap[effectiveState.value]?.class || 'status-init'
})

// 状态进度步骤配置
const progressSteps = computed(() => {
  const state = effectiveState.value
  const hasBot = hasBotToken.value
  // active_free / active_pro 状态本身就意味着已选择套餐
  const isActiveState = state === 'ACTIVE_FREE' || state === 'ACTIVE_PRO' || state === 'EXPIRED_DOWNGRADED'
  const hasPlan = isActiveState || !!props.subscription?.plan_code
  const hasProPlan = state === 'ACTIVE_PRO'
  
  const steps = [
    { key: 'init', label: '初始化', completed: true },
    { key: 'bot', label: 'Bot 接入', completed: hasBot },
    { key: 'plan', label: '选择套餐', completed: hasPlan },
    { key: 'active', label: '功能开通', completed: hasPlan }  // 套餐选完即功能开通
  ]
  
  return steps.map((step, index) => ({
    ...step,
    current: !step.completed && (
      (index === 0 && state === 'INIT') ||
      (index === 1 && (state === 'WAIT_BOT_TOKEN' || !hasBot)) ||
      (index === 2 && hasBot && !hasPlan) ||
      (index === 3 && hasPlan && !hasProPlan)
    )
  }))
})
</script>

<style scoped>
.overview-header {
  display: flex;
  justify-content: space-between;
  align-items: center;
  margin-bottom: 16px;
}

.status-badge {
  display: inline-flex;
  padding: 4px 12px;
  border-radius: 999px;
  font-size: 12px;
  font-weight: 700;
}
.status-init { background: rgba(148,163,184,.14); color: #475569; }
.status-wait { background: rgba(245,158,11,.12); color: #b45309; }
.status-validating { background: rgba(59,130,246,.12); color: #1d4ed8; }
.status-free { background: rgba(36,161,222,.12); color: #0f6fb2; }
.status-pro { background: rgba(29,120,213,.14); color: #155e8d; }
.status-expired { background: rgba(239,68,68,.12); color: #b91c1c; }
.status-suspended { background: rgba(107,114,128,.12); color: #4b5563; }

.dark .status-init { background: rgba(148,163,184,.18); color: #94a3b8; }
.dark .status-wait { background: rgba(245,158,11,.18); color: #fbbf24; }
.dark .status-validating { background: rgba(59,130,246,.18); color: #60a5fa; }
.dark .status-free { background: rgba(36,161,222,.18); color: #8fd8ff; }
.dark .status-pro { background: rgba(29,120,213,.2); color: #9edcff; }
.dark .status-expired { background: rgba(239,68,68,.18); color: #fca5a5; }
.dark .status-suspended { background: rgba(107,114,128,.18); color: #9ca3af; }

/* 进度条样式 */
.progress-bar {
  display: flex;
  justify-content: space-between;
  align-items: center;
  margin-bottom: 20px;
  padding: 16px 8px;
  background: rgb(248 250 252 / 0.8);
  border-radius: 12px;
  position: relative;
}

.dark .progress-bar {
  background: rgb(30 41 59 / 0.5);
}

.progress-bar::before {
  content: '';
  position: absolute;
  top: 50%;
  left: 40px;
  right: 40px;
  height: 2px;
  background: #e5e7eb;
  transform: translateY(-50%);
  z-index: 0;
}

.dark .progress-bar::before {
  background: rgb(71 85 105 / 0.4);
}

.progress-step {
  display: flex;
  flex-direction: column;
  align-items: center;
  gap: 8px;
  z-index: 1;
  position: relative;
}

.step-dot {
  width: 28px;
  height: 28px;
  border-radius: 50%;
  display: flex;
  align-items: center;
  justify-content: center;
  font-size: 12px;
  font-weight: 700;
  background: #e5e7eb;
  color: #9ca3af;
  transition: all 0.3s;
}

.dark .step-dot {
  background: rgb(71 85 105 / 0.4);
  color: #94a3b8;
}

.progress-step.completed .step-dot {
  background: #24a1de;
  color: #fff;
}

.dark .progress-step.completed .step-dot {
  background: #059669;
}

.progress-step.current .step-dot {
  background: #4f46e5;
  color: #fff;
  box-shadow: 0 0 0 4px rgba(79,70,229,.2);
}

.dark .progress-step.current .step-dot {
  background: #6366f1;
  box-shadow: 0 0 0 4px rgba(99,102,241,.3);
}

.step-label {
  font-size: 11px;
  color: #6b7280;
  text-align: center;
}

.dark .step-label {
  color: #9ca3af;
}

.progress-step.completed .step-label {
  color: #10b981;
  font-weight: 600;
}

.progress-step.current .step-label {
  color: #4f46e5;
  font-weight: 600;
}

.dark .progress-step.current .step-label {
  color: #818cf8;
}

.overview-grid {
  gap: 8px;
}

@media (max-width: 640px) {
  .overview-grid {
    grid-template-columns: repeat(2, 1fr);
  }
  .progress-bar {
    flex-wrap: wrap;
    gap: 12px;
    justify-content: center;
  }
  .progress-bar::before {
    display: none;
  }
}
</style>
