<template>
  <div class="uc-page more-page">
    <header class="more-topbar">
      <button class="more-icon-btn" type="button" @click="emit('navigate', 'saas')" aria-label="商户控制台">
        <span class="material-symbols-outlined">storefront</span>
      </button>
      <div>
        <h2>更多</h2>
        <p>商户状态、套餐与次级功能入口</p>
      </div>
      <button class="more-icon-btn" type="button" :disabled="loading" @click="loadData" aria-label="刷新">
        <span class="material-symbols-outlined" :class="{ spinning: loading }">refresh</span>
      </button>
    </header>

    <main class="more-content no-scrollbar">
      <section class="more-card merchant-card">
        <div class="merchant-avatar">{{ merchantInitial }}</div>
        <div class="merchant-main">
          <div class="merchant-title-row">
            <h3>{{ merchantName }}</h3>
            <span class="more-badge" :class="stateClass">{{ stateLabel }}</span>
          </div>
          <p>{{ botHandleText }}</p>
          <div class="merchant-meta">
            <span>{{ roleText }}</span>
            <span>{{ tenantText }}</span>
          </div>
        </div>
        <button class="more-link-btn" type="button" @click="emit('navigate', 'saas')">
          控制台
        </button>
      </section>

      <section class="more-card plan-card">
        <div class="section-head">
          <div>
            <h3>{{ planTitle }}</h3>
            <p>{{ planSubtitle }}</p>
          </div>
          <button class="more-outline-btn" type="button" @click="emit('navigate', 'saas')">管理套餐</button>
        </div>

        <div class="usage-list">
          <div v-for="row in usageRows" :key="row.label" class="usage-row">
            <div class="usage-row__head">
              <span>{{ row.label }}</span>
              <strong>{{ row.value }}</strong>
            </div>
            <div class="usage-track">
              <span :style="{ width: row.percent + '%' }"></span>
            </div>
          </div>
        </div>
      </section>

      <section class="shortcut-section">
        <div class="section-head">
          <div>
            <h3>功能入口</h3>
            <p>底部主入口之外的管理能力统一放在这里</p>
          </div>
        </div>

        <div class="shortcut-grid">
          <button
            v-for="item in shortcutItems"
            :key="item.key"
            class="shortcut-tile"
            type="button"
            :class="{ muted: item.muted, locked: item.locked }"
            @click="handleShortcut(item)"
          >
            <span class="material-symbols-outlined">{{ item.icon }}</span>
            <strong>{{ item.label }}</strong>
            <small>
              <span v-if="item.locked" class="material-symbols-outlined">lock</span>
              {{ item.caption }}
            </small>
          </button>
        </div>
      </section>

      <section class="more-card system-card">
        <div class="system-row">
          <span class="material-symbols-outlined">verified_user</span>
          <div>
            <strong>系统运行稳定</strong>
            <p>免费功能与付费能力按套餐状态开放</p>
          </div>
        </div>
        <button class="more-danger-btn" type="button" @click="logout">
          <span class="material-symbols-outlined">logout</span>
          退出登录
        </button>
      </section>

      <p v-if="error" class="more-error">{{ error }}</p>
    </main>
  </div>
</template>

<script setup lang="ts">
import { computed, onMounted, ref } from 'vue'
import { api } from '@/api'
import { useAuthStore } from '@/stores/auth'
import { MORE_SHORTCUTS, type AppTabId, type MoreShortcutDef } from '@/app/navigation/app-tabs'
import { resolveMerchantState, type MerchantState } from '@/lib/merchantState'
import { normalizeFeatureMap } from '@/composables/useMerchantFeatureGate'

interface ShortcutItem extends MoreShortcutDef {
  label: string
  caption: string
  tab?: AppTabId
  locked?: boolean
}

const emit = defineEmits<{
  navigate: [tab: string]
  reopenTelegram: []
}>()

const auth = useAuthStore()
const profile = ref<Record<string, any> | null>(null)
const subscription = ref<Record<string, any> | null>(null)
const aiUsage = ref<Record<string, any> | null>(null)
const features = ref<Record<string, boolean> | null>(null)
const loading = ref(false)
const error = ref('')

const currentState = computed<MerchantState>(() => resolveMerchantState(profile.value, subscription.value))
const isMerchantOwner = computed(() => auth.sessionContext?.role === 'merchant_owner')

const merchantName = computed(() => {
  return profile.value?.merchant_name
    || (isMerchantOwner.value ? `商户 #${auth.sessionContext?.merchantId || '-'}` : '平台管理')
})

const merchantInitial = computed(() => {
  const source = merchantName.value.replace(/^商户\s*/, '').trim()
  return source.slice(0, 2) || '商户'
})

const botHandleText = computed(() => {
  const username = profile.value?.bot_username || profile.value?.bot_handle
  if (username) return username.startsWith('@') ? username : `@${username}`
  return profile.value?.bot_token ? 'Bot 已连接' : 'Bot 尚未接入'
})

const roleText = computed(() => {
  if (auth.sessionContext?.role === 'platform_admin') return 'Platform Admin'
  if (auth.sessionContext?.role === 'merchant_owner') return 'Owner'
  return auth.sessionContext?.role || 'Session'
})

const tenantText = computed(() => auth.sessionContext?.tenantId || profile.value?.tenant_id || '未绑定租户')

const stateMeta: Record<MerchantState, { label: string; className: string }> = {
  INIT: { label: '初始化', className: 'neutral' },
  WAIT_BOT_TOKEN: { label: '待接入', className: 'warning' },
  TOKEN_VALIDATING: { label: '校验中', className: 'info' },
  ACTIVE_FREE: { label: 'Active Free', className: 'info' },
  ACTIVE_PRO: { label: 'Active Pro', className: 'success' },
  EXPIRED_DOWNGRADED: { label: '已降级', className: 'warning' },
  SUSPENDED: { label: '已暂停', className: 'danger' },
}

const stateLabel = computed(() => stateMeta[currentState.value]?.label || currentState.value)
const stateClass = computed(() => stateMeta[currentState.value]?.className || 'neutral')

const planTitle = computed(() => {
  if (currentState.value === 'ACTIVE_PRO') return subscription.value?.plan_name || 'Pro 套餐'
  if (currentState.value === 'ACTIVE_FREE') return '免费套餐'
  if (currentState.value === 'EXPIRED_DOWNGRADED') return '套餐已过期'
  if (currentState.value === 'SUSPENDED') return '账户已暂停'
  return '待完成接入'
})

const planSubtitle = computed(() => {
  const endAt = subscription.value?.end_at || subscription.value?.expired_at
  if (endAt) return `到期时间：${formatDate(endAt)}`
  if (currentState.value === 'ACTIVE_FREE') return '基础功能可用，Pro 能力需要升级'
  if (currentState.value === 'WAIT_BOT_TOKEN') return '提交 Bot Token 后开启业务功能'
  return '进入商户控制台查看完整状态'
})

const usageRows = computed(() => {
  const calls = Number(aiUsage.value?.total_calls || 0)
  const tokens = Number(aiUsage.value?.total_tokens || 0)
  return [
    { label: '会话处理', value: calls ? `${calls.toLocaleString()} 次` : '待统计', percent: Math.min(100, Math.max(18, calls / 10)) },
    { label: 'AI 用量', value: tokens ? formatCompact(tokens) : '待统计', percent: Math.min(100, Math.max(12, tokens / 10000)) },
  ]
})

const shortcutItems = computed<ShortcutItem[]>(() => {
  return MORE_SHORTCUTS.map(item => {
    const locked = isKnownLocked(item.featureKey)
    const label = isMerchantOwner.value && item.merchantLabel ? item.merchantLabel : item.label
    const baseCaption = isMerchantOwner.value && item.merchantCaption ? item.merchantCaption : item.caption
    return {
      ...item,
      label,
      caption: locked ? 'Pro 解锁' : baseCaption,
      tab: isMerchantOwner.value && item.merchantTab ? item.merchantTab : item.tab,
      locked,
    }
  })
})

function isKnownLocked(featureKey: string) {
  return features.value ? features.value[featureKey] === false : false
}

function formatDate(value: string) {
  const date = new Date(value)
  if (Number.isNaN(date.getTime())) return value
  return date.toLocaleString('zh-CN', { month: '2-digit', day: '2-digit', hour: '2-digit', minute: '2-digit' })
}

function formatCompact(value: number) {
  if (value >= 1000000) return `${(value / 1000000).toFixed(1)}M`
  if (value >= 1000) return `${(value / 1000).toFixed(1)}K`
  return value.toLocaleString()
}

async function loadData() {
  loading.value = true
  error.value = ''
  try {
    const [profileRes, subscriptionRes, usageRes, featureRes] = await Promise.allSettled([
      api.getMerchantProfile(),
      api.getMerchantSubscription(),
      api.getMerchantAiUsage(7),
      api.getMerchantFeatures(),
    ])

    if (profileRes.status === 'fulfilled') profile.value = profileRes.value?.data ?? null
    if (subscriptionRes.status === 'fulfilled') subscription.value = subscriptionRes.value?.data ?? null
    if (usageRes.status === 'fulfilled') aiUsage.value = usageRes.value?.data ?? null
    if (featureRes.status === 'fulfilled') {
      features.value = normalizeFeatureMap(featureRes.value?.data)
    }
  } catch (e: any) {
    error.value = e?.message || '加载更多页失败'
  } finally {
    loading.value = false
  }
}

function handleShortcut(item: ShortcutItem) {
  if (item.action === 'telegram') {
    emit('reopenTelegram')
    return
  }
  if (item.action === 'logout') {
    logout()
    return
  }
  if (item.tab) emit('navigate', item.tab)
}

function logout() {
  auth.logout()
}

onMounted(() => {
  void loadData()
})
</script>

<style scoped>
.more-page {
  background:
    radial-gradient(circle at 12% 0%, rgba(36, 161, 222, 0.08), transparent 38%),
    linear-gradient(180deg, #f7fbff 0%, #f5f7fb 100%);
}

.dark .more-page {
  background:
    radial-gradient(circle at 12% 0%, rgba(36, 161, 222, 0.12), transparent 38%),
    linear-gradient(180deg, #08111f 0%, #0b1424 100%);
}

.more-topbar {
  min-height: 58px;
  display: grid;
  grid-template-columns: 44px 1fr 44px;
  align-items: center;
  gap: 10px;
  padding: 8px 16px 6px;
  border-bottom: 1px solid rgb(148 163 184 / 0.16);
  background: rgb(255 255 255 / 0.84);
  backdrop-filter: blur(12px);
}

.dark .more-topbar {
  background: rgb(8 17 31 / 0.86);
  border-bottom-color: rgb(73 106 148 / 0.24);
}

.more-topbar h2 {
  margin: 0;
  font-size: 20px;
  line-height: 1.2;
  font-weight: 900;
  color: #0f172a;
}

.dark .more-topbar h2 {
  color: #edf4ff;
}

.more-topbar p {
  margin: 2px 0 0;
  font-size: 11px;
  color: #64748b;
}

.dark .more-topbar p {
  color: #adc3e0;
}

.more-icon-btn {
  width: 40px;
  height: 40px;
  border: 0;
  border-radius: 12px;
  display: inline-flex;
  align-items: center;
  justify-content: center;
  color: #0f6fb2;
  background: rgb(231 246 255 / 0.78);
}

.dark .more-icon-btn {
  color: #8fd8ff;
  background: rgb(10 43 73 / 0.72);
}

.more-icon-btn:disabled {
  opacity: 0.55;
}

.spinning {
  animation: more-spin 0.8s linear infinite;
}

@keyframes more-spin {
  to { transform: rotate(360deg); }
}

.more-content {
  flex: 1;
  min-height: 0;
  overflow-y: auto;
  padding: 14px 14px 96px;
  display: flex;
  flex-direction: column;
  gap: 12px;
}

.more-card {
  border: 1px solid rgb(148 163 184 / 0.18);
  border-radius: 16px;
  background: rgb(255 255 255 / 0.92);
  box-shadow: 0 10px 26px rgba(15, 23, 42, 0.06);
  padding: 14px;
}

.dark .more-card {
  border-color: rgb(73 106 148 / 0.26);
  background: rgb(13 26 45 / 0.88);
  box-shadow: 0 14px 30px rgba(2, 6, 23, 0.34);
}

.merchant-card {
  display: grid;
  grid-template-columns: 48px 1fr auto;
  align-items: center;
  gap: 12px;
}

.merchant-avatar {
  width: 48px;
  height: 48px;
  border-radius: 15px;
  display: flex;
  align-items: center;
  justify-content: center;
  background: linear-gradient(135deg, #24a1de, #14b8a6);
  color: #fff;
  font-size: 14px;
  font-weight: 900;
}

.merchant-main {
  min-width: 0;
}

.merchant-title-row {
  display: flex;
  align-items: center;
  gap: 8px;
  min-width: 0;
}

.merchant-title-row h3 {
  margin: 0;
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  font-size: 16px;
  font-weight: 900;
  color: #0f172a;
}

.dark .merchant-title-row h3 {
  color: #edf4ff;
}

.merchant-main p {
  margin: 4px 0 0;
  font-size: 12px;
  color: #64748b;
}

.dark .merchant-main p {
  color: #adc3e0;
}

.merchant-meta {
  display: flex;
  flex-wrap: wrap;
  gap: 6px;
  margin-top: 7px;
}

.merchant-meta span {
  display: inline-flex;
  max-width: 100%;
  padding: 3px 7px;
  border-radius: 999px;
  background: rgb(241 245 249 / 0.9);
  color: #475569;
  font-size: 10px;
  font-weight: 800;
}

.dark .merchant-meta span {
  background: rgb(30 48 73 / 0.72);
  color: #c8d8ef;
}

.more-badge {
  flex: 0 0 auto;
  display: inline-flex;
  padding: 3px 7px;
  border-radius: 999px;
  font-size: 10px;
  font-weight: 900;
}

.more-badge.success { background: rgba(16, 185, 129, 0.12); color: #047857; }
.more-badge.info { background: rgba(36, 161, 222, 0.12); color: #0f6fb2; }
.more-badge.warning { background: rgba(245, 158, 11, 0.14); color: #b45309; }
.more-badge.danger { background: rgba(239, 68, 68, 0.12); color: #b91c1c; }
.more-badge.neutral { background: rgba(148, 163, 184, 0.14); color: #475569; }

.dark .more-badge.success { color: #6ee7b7; }
.dark .more-badge.info { color: #8fd8ff; }
.dark .more-badge.warning { color: #fbbf24; }
.dark .more-badge.danger { color: #fca5a5; }
.dark .more-badge.neutral { color: #cbd5e1; }

.more-link-btn,
.more-outline-btn,
.more-danger-btn {
  border: 0;
  border-radius: 12px;
  min-height: 36px;
  padding: 0 12px;
  font-size: 12px;
  font-weight: 900;
  white-space: nowrap;
}

.more-link-btn {
  background: #24a1de;
  color: #fff;
}

.more-outline-btn {
  border: 1px solid rgb(36 161 222 / 0.24);
  background: rgb(231 246 255 / 0.78);
  color: #0f6fb2;
}

.dark .more-outline-btn {
  background: rgb(10 43 73 / 0.72);
  color: #8fd8ff;
}

.section-head {
  display: flex;
  justify-content: space-between;
  align-items: flex-start;
  gap: 12px;
}

.section-head h3 {
  margin: 0;
  color: #0f172a;
  font-size: 15px;
  font-weight: 900;
}

.dark .section-head h3 {
  color: #edf4ff;
}

.section-head p {
  margin: 3px 0 0;
  color: #64748b;
  font-size: 12px;
}

.dark .section-head p {
  color: #adc3e0;
}

.usage-list {
  display: flex;
  flex-direction: column;
  gap: 10px;
  margin-top: 14px;
}

.usage-row__head {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 10px;
  margin-bottom: 6px;
  font-size: 12px;
  color: #64748b;
}

.dark .usage-row__head {
  color: #adc3e0;
}

.usage-row__head strong {
  color: #0f172a;
  font-size: 12px;
}

.dark .usage-row__head strong {
  color: #edf4ff;
}

.usage-track {
  height: 7px;
  border-radius: 999px;
  overflow: hidden;
  background: rgb(226 232 240 / 0.86);
}

.dark .usage-track {
  background: rgb(51 65 85 / 0.56);
}

.usage-track span {
  display: block;
  height: 100%;
  border-radius: inherit;
  background: linear-gradient(90deg, #24a1de, #10b981);
}

.shortcut-section {
  display: flex;
  flex-direction: column;
  gap: 10px;
}

.shortcut-grid {
  display: grid;
  grid-template-columns: repeat(3, minmax(0, 1fr));
  gap: 10px;
}

.shortcut-tile {
  min-height: 94px;
  border: 1px solid rgb(148 163 184 / 0.18);
  border-radius: 16px;
  background: rgb(255 255 255 / 0.9);
  color: #0f172a;
  display: flex;
  flex-direction: column;
  align-items: flex-start;
  justify-content: space-between;
  gap: 6px;
  padding: 12px;
  text-align: left;
  box-shadow: 0 8px 20px rgba(15, 23, 42, 0.05);
}

.dark .shortcut-tile {
  border-color: rgb(73 106 148 / 0.26);
  background: rgb(13 26 45 / 0.88);
  color: #edf4ff;
}

.shortcut-tile:active {
  transform: scale(0.98);
}

.shortcut-tile .material-symbols-outlined {
  color: #0f6fb2;
  font-size: 23px;
}

.dark .shortcut-tile .material-symbols-outlined {
  color: #8fd8ff;
}

.shortcut-tile strong {
  font-size: 13px;
  line-height: 1.15;
  font-weight: 900;
}

.shortcut-tile small {
  color: #64748b;
  font-size: 10px;
  line-height: 1.25;
  display: inline-flex;
  align-items: center;
  gap: 3px;
}

.dark .shortcut-tile small {
  color: #adc3e0;
}

.shortcut-tile small .material-symbols-outlined {
  min-width: 12px;
  font-size: 12px;
  color: currentColor;
}

.shortcut-tile.muted {
  background: rgb(248 250 252 / 0.82);
}

.dark .shortcut-tile.muted {
  background: rgb(9 20 35 / 0.82);
}

.shortcut-tile.locked {
  border-color: rgb(245 158 11 / 0.24);
  background: rgb(255 251 235 / 0.82);
}

.dark .shortcut-tile.locked {
  border-color: rgb(245 158 11 / 0.24);
  background: rgb(120 53 15 / 0.18);
}

.system-card {
  display: flex;
  flex-direction: column;
  gap: 12px;
}

.system-row {
  display: flex;
  gap: 10px;
  align-items: flex-start;
}

.system-row > .material-symbols-outlined {
  color: #10b981;
}

.system-row strong {
  display: block;
  font-size: 13px;
  color: #0f172a;
}

.dark .system-row strong {
  color: #edf4ff;
}

.system-row p {
  margin: 2px 0 0;
  font-size: 12px;
  color: #64748b;
}

.dark .system-row p {
  color: #adc3e0;
}

.more-danger-btn {
  width: 100%;
  min-height: 42px;
  display: inline-flex;
  align-items: center;
  justify-content: center;
  gap: 8px;
  border: 1px solid rgb(239 68 68 / 0.24);
  background: rgb(254 226 226 / 0.78);
  color: #b91c1c;
}

.dark .more-danger-btn {
  background: rgb(127 29 29 / 0.26);
  color: #fecaca;
}

.more-error {
  margin: 0;
  padding: 10px 12px;
  border-radius: 12px;
  color: #b91c1c;
  background: rgb(254 226 226 / 0.76);
  font-size: 12px;
}

@media (max-width: 360px) {
  .shortcut-grid {
    grid-template-columns: repeat(2, minmax(0, 1fr));
  }

  .merchant-card {
    grid-template-columns: 44px 1fr;
  }

  .more-link-btn {
    grid-column: 1 / -1;
  }
}
</style>
