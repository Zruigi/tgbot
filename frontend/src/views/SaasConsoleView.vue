<template>
  <div class="uc-page bg-surface">
    <header class="uc-header">
      <div class="uc-header-wrap">
        <div>
          <h2 class="uc-title">
            <span class="material-symbols-outlined text-primary">{{ isPlatformAdmin ? 'storefront' : 'point_of_sale' }}</span>
            {{ isPlatformAdmin ? '商户中心' : '商户工作台' }}
          </h2>
          <p class="uc-subtitle">{{ isPlatformAdmin ? '不再输入商户 ID，直接查看商户详细信息。' : '完善接入信息、选择套餐、提交支付、开通功能、进入业务。' }}</p>
        </div>
        <div class="uc-actions">
          <button class="uc-btn uc-btn-primary" @click="loadDashboard" :disabled="loading">
            <span class="material-symbols-outlined text-base">refresh</span>
            {{ loading ? '刷新中...' : '刷新数据' }}
          </button>
        </div>
      </div>
    </header>

    <main class="uc-main">
      <div class="uc-main-wrap">

    <div v-if="error" class="uc-toast">{{ error }}</div>

    <!-- 商户工作台视图 -->
    <template v-if="!isPlatformAdmin">
      <section class="merchant-workspace">
        <!-- 1. 接入状态卡（始终显示） -->
        <SaasOverviewCard
          :profile="profile"
          :subscription="subscription"
          :current-state="currentState"
        />

        <!-- 下一步引导卡（根据状态动态显示） -->
        <SaasNextStepCard
          v-if="nextStepInfo"
          :step="nextStepInfo"
          @action="handleNextStepAction"
        />

        <SaasAccessStatePanel
          v-if="shouldShowAccessStatePanel"
          :profile="profile"
          :subscription="subscription"
          :current-state="currentState"
          @upgrade="onAccessUpgrade"
          @contact="onAccessContact"
          @navigate="onNavigate"
        />

        <!-- 2. Bot 接入卡：WAIT_BOT_TOKEN / INIT 状态显示 -->
        <SaasBotTokenCard
          ref="botTokenCardRef"
          v-if="shouldShowBotToken"
          :current-token="profile?.bot_token"
          :current-state="currentState"
          @saved="onBotTokenSaved"
        />

        <!-- 3. 套餐购买卡：非待接入 Bot 状态显示 -->
        <SaasPlanCard
          v-if="shouldShowPlan"
          ref="planCardRef"
          @order-created="onOrderCreated"
          @plan-activated="onPlanActivated"
        />

        <!-- 4. 支付凭证卡：有待支付订单时显示 -->
        <SaasPaymentCard
          v-if="shouldShowPayment"
          ref="paymentCardRef"
          :trigger-refresh="paymentRefreshTrigger"
          @paid="onPaymentPaid"
        />

        <!-- 5. 功能开通卡：ACTIVE 状态显示 -->
        <SaasFeatureCard
          v-if="shouldShowFeature"
          ref="featureCardRef"
        />

        <!-- 6. 快捷业务入口卡（始终显示但引导未解锁功能） -->
        <SaasQuickActionsCard
          ref="quickActionsRef"
          @upgrade-guide="onUpgradeGuide"
          @navigate="onNavigate"
        />
      </section>
    </template>

    <!-- 平台管理员视图 -->
    <template v-else>
      <section class="uc-grid uc-grid-cols-3" style="gap:12px">
        <div class="uc-stat"><span class="uc-stat-label">商户总数</span><div class="uc-stat-value">{{ merchantRows.length }}</div></div>
        <div class="uc-stat"><span class="uc-stat-label">付费商户</span><div class="uc-stat-value">{{ paidMerchantCount }}</div></div>
        <div class="uc-stat"><span class="uc-stat-label">即将到期</span><div class="uc-stat-value">{{ expiringMerchantCount }}</div></div>
      </section>

      <PlatformPlanManager />

      <section class="uc-grid" style="gap:12px">
        <div class="uc-card">
          <div style="display:flex;justify-content:space-between;align-items:center;margin-bottom:12px">
            <h3 class="uc-card-title">商户数据</h3>
            <div class="uc-tabs">
              <button class="uc-tab" :class="{ active: merchantFilter==='paid' }" @click="merchantFilter='paid'">付费商户</button>
              <button class="uc-tab" :class="{ active: merchantFilter==='all' }" @click="merchantFilter='all'">全部商户</button>
            </div>
          </div>
          <div class="merchant-table-wrap">
            <table class="merchant-table">
              <thead>
                <tr>
                  <th>UserID</th>
                  <th>用户名</th>
                  <th>注册时间</th>
                  <th>付费选项</th>
                  <th>到期时间</th>
                </tr>
              </thead>
              <tbody>
                <tr v-for="m in filteredMerchantRows" :key="m.id">
                  <td>{{ m.user_id ?? '-' }}</td>
                  <td>{{ m.username || '-' }}</td>
                  <td>{{ fmt(m.registered_at) }}</td>
                  <td><span class="merchant-badge" :class="m.isPaid ? 'paid' : 'free'">{{ m.paid_option || 'free' }}</span></td>
                  <td>{{ fmt(m.expires_at) }}</td>
                </tr>
              </tbody>
            </table>
            <div v-if="filteredMerchantRows.length===0" class="uc-empty">暂无商户</div>
          </div>
        </div>
      </section>
    </template>

      </div>
    </main>
  </div>
</template>

<script setup lang="ts">
import { computed, onMounted, onUnmounted, ref } from 'vue'
import { api } from '@/api'
import { useAuthStore } from '@/stores/auth'
import { resolveMerchantState, type MerchantState } from '@/lib/merchantState'

const emit = defineEmits<{
  navigate: [tab: string]
}>()

import SaasOverviewCard from '@/components/saas/SaasOverviewCard.vue'
import SaasBotTokenCard from '@/components/saas/SaasBotTokenCard.vue'
import SaasPlanCard from '@/components/saas/SaasPlanCard.vue'
import SaasPaymentCard from '@/components/saas/SaasPaymentCard.vue'
import SaasFeatureCard from '@/components/saas/SaasFeatureCard.vue'
import SaasQuickActionsCard from '@/components/saas/SaasQuickActionsCard.vue'
import SaasNextStepCard from '@/components/saas/SaasNextStepCard.vue'
import type { NextStep } from '@/components/saas/SaasNextStepCard.vue'
import SaasAccessStatePanel from '@/components/saas/SaasAccessStatePanel.vue'
import PlatformPlanManager from '@/components/saas/PlatformPlanManager.vue'

const auth = useAuthStore()
const loading = ref(false)
const error = ref('')
const merchantFilter = ref<'paid' | 'all'>('paid')
const merchantRows = ref<Record<string, any>[]>([])
let ws: WebSocket | null = null

// 商户端数据
const profile = ref<Record<string, any> | null>(null)
const subscription = ref<Record<string, any> | null>(null)
const paymentRefreshTrigger = ref(0)

// 子组件引用
const botTokenCardRef = ref<InstanceType<typeof SaasBotTokenCard> | null>(null)
const planCardRef = ref<InstanceType<typeof SaasPlanCard> | null>(null)
const paymentCardRef = ref<InstanceType<typeof SaasPaymentCard> | null>(null)
const featureCardRef = ref<InstanceType<typeof SaasFeatureCard> | null>(null)
const quickActionsRef = ref<InstanceType<typeof SaasQuickActionsCard> | null>(null)

const isPlatformAdmin = computed(() => auth.sessionContext?.role === 'platform_admin')
const filteredMerchantRows = computed(() => merchantFilter.value === 'paid' ? merchantRows.value.filter((x) => x.isPaid) : merchantRows.value)
const paidMerchantCount = computed(() => merchantRows.value.filter((x) => x.isPaid).length)
const expiringMerchantCount = computed(() => {
  const now = Date.now()
  const in30Days = now + 30 * 86400 * 1000
  return merchantRows.value.filter((x) => {
    if (!x.expires_at) return false
    const t = new Date(x.expires_at).getTime()
    return !Number.isNaN(t) && t >= now && t <= in30Days
  }).length
})

// ============================================================
// 状态机配置 - Phase C 核心
// ============================================================

/** 根据 profile 和 subscription 计算当前商户状态 */
const currentState = computed<MerchantState>(() => {
  return resolveMerchantState(profile.value, subscription.value)
})

/** 状态机配置：每个状态应该显示哪些卡片 */
interface StateConfig {
  showBotToken: boolean
  showPlan: boolean
  showPayment: boolean
  showFeature: boolean
  nextStep: NextStep | null
}

const stateMachine: Record<MerchantState, StateConfig> = {
  INIT: {
    showBotToken: true,
    showPlan: false,
    showPayment: false,
    showFeature: false,
    nextStep: {
      title: '开始接入',
      description: '提交 Bot Token 完成商户接入',
      buttonText: '去接入',
      action: 'scrollToBotToken',
      urgency: 'high'
    }
  },
  WAIT_BOT_TOKEN: {
    showBotToken: true,
    showPlan: false,
    showPayment: false,
    showFeature: false,
    nextStep: {
      title: '提交 Bot Token',
      description: '完成 Telegram Bot 接入以继续配置',
      buttonText: '去提交',
      action: 'scrollToBotToken',
      urgency: 'high'
    }
  },
  TOKEN_VALIDATING: {
    showBotToken: true,
    showPlan: false,
    showPayment: false,
    showFeature: false,
    nextStep: {
      title: '等待 Bot Token 校验',
      description: '系统正在验证 Token，请稍候...',
      buttonText: '刷新状态',
      action: 'refresh',
      urgency: 'normal'
    }
  },
  ACTIVE_FREE: {
    showBotToken: false,
    showPlan: true,
    showPayment: false,
    showFeature: true,
    nextStep: {
      title: '升级套餐解锁更多功能',
      description: '付费套餐可解锁群发、AI 翻译、高级分析等功能',
      buttonText: '查看套餐',
      action: 'scrollToPlan',
      urgency: 'normal'
    }
  },
  ACTIVE_PRO: {
    showBotToken: false,
    showPlan: true,  // 允许续费
    showPayment: false,
    showFeature: true,
    nextStep: null
  },
  EXPIRED_DOWNGRADED: {
    showBotToken: false,
    showPlan: true,
    showPayment: true,  // 显示待支付订单
    showFeature: true,
    nextStep: {
      title: '套餐已过期，功能受限',
      description: '续费以恢复全部功能',
      buttonText: '立即续费',
      action: 'scrollToPlan',
      urgency: 'critical'
    }
  },
  SUSPENDED: {
    showBotToken: false,
    showPlan: true,
    showPayment: true,
    showFeature: false,
    nextStep: {
      title: '账户已暂停',
      description: '请联系平台管理员恢复服务',
      buttonText: '联系客服',
      action: 'contactSupport',
      urgency: 'critical'
    }
  }
}

/** 当前状态配置 */
const stateConfig = computed(() => stateMachine[currentState.value] || stateMachine.INIT)

/** 卡片显示控制 */
const shouldShowBotToken = computed(() => stateConfig.value.showBotToken)
const shouldShowPlan = computed(() => stateConfig.value.showPlan)
const shouldShowPayment = computed(() => stateConfig.value.showPayment)
const shouldShowFeature = computed(() => stateConfig.value.showFeature)
const shouldShowAccessStatePanel = computed(() =>
  currentState.value === 'ACTIVE_FREE'
  || currentState.value === 'EXPIRED_DOWNGRADED'
  || currentState.value === 'SUSPENDED'
)

/** 下一步引导信息 */
const nextStepInfo = computed(() => stateConfig.value.nextStep)

/** 处理下一步引导按钮点击 */
function handleNextStepAction(action: string) {
  switch (action) {
    case 'scrollToBotToken':
      scrollToCard('botTokenCard')
      break
    case 'scrollToPlan':
      scrollToCard('planCard')
      break
    case 'scrollToPayment':
      scrollToCard('paymentCard')
      break
    case 'refresh':
      void loadDashboard()
      break
    case 'contactSupport':
      // 打开客服联系方式或弹窗
      alert('请联系平台管理员或发送邮件至 support@example.com')
      break
    default:
      console.warn('Unknown next step action:', action)
  }
}

/** 滚动到指定卡片 */
function scrollToCard(cardType: 'botTokenCard' | 'planCard' | 'paymentCard' | 'featureCard' | 'quickActionsCard') {
  let el: HTMLElement | null = null
  
  switch (cardType) {
    case 'botTokenCard':
      el = botTokenCardRef.value?.$el ?? null
      break
    case 'planCard':
      el = planCardRef.value?.$el ?? null
      break
    case 'paymentCard':
      el = paymentCardRef.value?.$el ?? null
      break
    case 'featureCard':
      el = featureCardRef.value?.$el ?? null
      break
    case 'quickActionsCard':
      el = quickActionsRef.value?.$el ?? null
      break
  }
  
  if (el) {
    el.scrollIntoView({ behavior: 'smooth', block: 'center' })
    // 高亮效果
    el.classList.add('highlight')
    setTimeout(() => el?.classList.remove('highlight'), 2000)
  }
}

/** 快捷入口升级引导处理 */
function onUpgradeGuide(feature: string) {
  // 滚动到套餐卡片
  scrollToCard('planCard')
}

function onAccessUpgrade(feature: string) {
  void feature
  scrollToCard('planCard')
}

function onAccessContact() {
  alert('请联系平台管理员或发送邮件至 support@example.com')
}

/** 快捷入口导航处理 - 切换到对应的 tab */
function onNavigate(tab: string) {
  // 发出事件让 App.vue 处理
  emit('navigate', tab)
}

function fmt(value?: string | null) {
  if (!value) return '-'
  const date = new Date(value)
  return Number.isNaN(date.getTime()) ? String(value) : date.toLocaleString('zh-CN', { year: 'numeric', month: '2-digit', day: '2-digit', hour: '2-digit', minute: '2-digit' })
}

function mergeRows(merchants: Record<string, any>[]) {
  return merchants.map((m) => {
    return {
      id: m.id,
      user_id: m.user_id ?? m.owner_tg_user_id ?? '-',
      username: m.username || m.owner_username || m.merchant_name || '-',
      registered_at: m.registered_at || m.created_at || null,
      paid_option: m.paid_option || m.plan_type || 'free',
      expires_at: m.expires_at || m.subscription_expires_at || m.subscription_end_at || null,
      isPaid: Boolean(m.isPaid || !['', 'free'].includes(String(m.paid_option || m.plan_type || '').toLowerCase())),
    }
  }).sort((a, b) => Number(b.isPaid) - Number(a.isPaid))
}

async function loadPlatform() {
  const m = await api.getPlatformMerchants({ limit: 200 })
  merchantRows.value = mergeRows(m?.data ?? [])
}

async function loadMerchant() {
  const [p, s] = await Promise.all([api.getMerchantProfile(), api.getMerchantSubscription()])
  profile.value = p?.data ?? null
  subscription.value = s?.data ?? null
}

async function loadDashboard() {
  loading.value = true
  error.value = ''
  try {
    if (isPlatformAdmin.value) {
      await loadPlatform()
    } else {
      await loadMerchant()
    }
  } catch (e: any) {
    error.value = e?.message || '加载商户模块失败'
  } finally {
    loading.value = false
  }
}

function onBotTokenSaved() {
  // Bot Token 保存成功后刷新 profile
  loadMerchant()
}

function onOrderCreated(orderId: number) {
  // 订单创建后刷新支付卡片
  paymentRefreshTrigger.value++
}

function onPlanActivated() {
  void loadMerchant()
  featureCardRef.value?.loadData()
  quickActionsRef.value?.loadFeatures()
}

function onPaymentPaid() {
  // 支付标记后刷新相关数据
  loadMerchant()
  planCardRef.value?.loadPlans()
  featureCardRef.value?.loadData()
  quickActionsRef.value?.loadFeatures()
}

function connectWebSocket() {
  if (ws || !isPlatformAdmin.value) return
  
  const wsProtocol = location.protocol === 'https:' ? 'wss:' : 'ws:'
  ws = new WebSocket(`${wsProtocol}//${location.host}/ws`)
  
  ws.onmessage = (event) => {
    try {
      const data = JSON.parse(event.data)
      if (data.type === 'merchant_bot_token_updated') {
        // 商户提交了新的Bot Token，刷新商户列表
        console.log('收到商户Bot Token更新通知:', data.data)
        void loadPlatform()
      }
    } catch (e) {
      // 忽略解析错误
    }
  }
  
  ws.onerror = () => {
    console.error('WebSocket连接错误')
  }
  
  ws.onclose = () => {
    ws = null
    // 5秒后重连
    setTimeout(connectWebSocket, 5000)
  }
}

onMounted(() => {
  void loadDashboard()
  connectWebSocket()
})

onUnmounted(() => {
  if (ws) {
    ws.close()
    ws = null
  }
})
</script>

<style scoped>
/* 商户工作台卡片间距 */
.merchant-workspace {
  display: flex;
  flex-direction: column;
  gap: 12px;
  min-height: 0;
  flex: 1;
}

.merchant-table-wrap {
  overflow:auto;
  border:1px solid rgb(148 163 184 / 0.16);
  border-radius:8px;
}
.merchant-table {
  width:100%;
  min-width:760px;
  border-collapse:collapse;
  font-size:13px;
}
.merchant-table th,
.merchant-table td {
  padding:10px 12px;
  text-align:left;
  border-bottom:1px solid rgb(148 163 184 / 0.14);
  white-space:nowrap;
}
.merchant-table th {
  color:#475569;
  font-size:12px;
  font-weight:700;
  background:rgb(248 250 252 / 0.88);
}
.merchant-table tbody tr:last-child td { border-bottom:0; }
.dark .merchant-table-wrap { border-color:rgb(73 106 148 / 0.22); }
.dark .merchant-table th {
  color:#adc3e0;
  background:rgb(8 17 31 / 0.82);
}
.dark .merchant-table td { border-bottom-color:rgb(73 106 148 / 0.18); }

/* 付费/未付费徽章 */
.merchant-badge {
  display:inline-flex; padding:4px 8px; border-radius:999px;
  font-size:11px; font-weight:700; flex-shrink:0;
}
.merchant-badge.paid { background:rgba(16,185,129,.12); color:#047857; }
.merchant-badge.free { background:rgba(148,163,184,.14); color:#475569; }

.dark .merchant-badge.paid { background:rgba(16,185,129,.18); color:#6ee7b7; }
.dark .merchant-badge.free { background:rgba(148,163,184,.18); color:#94a3b8; }

/* 卡片滚动高亮动画 */
::global(.highlight) {
  animation: card-highlight 2s ease-out;
}
@keyframes card-highlight {
  0% {
    box-shadow: 0 0 0 4px rgba(36,161,222,.4);
    transform: scale(1.01);
  }
  70% {
    box-shadow: 0 0 0 8px rgba(36,161,222,.2);
  }
  100% {
    box-shadow: 0 8px 20px rgba(15,23,42,.06);
    transform: scale(1);
  }
}

/* 响应式 */
@media (max-width:1080px) {
  .uc-grid-cols-3,
  .uc-grid-cols-4 { grid-template-columns:repeat(2,minmax(0,1fr)); }
}
@media (max-width:768px) {
  .uc-grid-cols-3,
  .uc-grid-cols-4,
  .uc-grid-cols-2 { grid-template-columns:1fr; }
}
</style>
