<template>
  <article v-if="currentState === 'ACTIVE_FREE'" class="uc-card access-panel">
    <div class="access-hero free">
      <div>
        <span class="access-kicker">ACTIVE_FREE</span>
        <h3>免费套餐已启用</h3>
        <p>{{ merchantName }} 已完成接入，基础能力可用，Pro 能力按需升级。</p>
      </div>
      <button class="uc-btn uc-btn-primary" type="button" @click="$emit('upgrade', 'pro')">
        升级 Pro
      </button>
    </div>

    <div class="access-section">
      <div class="access-section-head">
        <h4>当前可用功能</h4>
        <span>免费版</span>
      </div>
      <div class="feature-grid">
        <div v-for="item in freeFeatures" :key="item.key" class="feature-card enabled">
          <span class="material-symbols-outlined">{{ item.icon }}</span>
          <strong>{{ item.label }}</strong>
          <small>可用</small>
        </div>
      </div>
    </div>

    <div class="access-section">
      <div class="access-section-head">
        <h4>Pro 锁定功能</h4>
        <span>升级后启用</span>
      </div>
      <div class="locked-list">
        <button
          v-for="item in lockedFeatures"
          :key="item.key"
          class="locked-item"
          type="button"
          @click="$emit('upgrade', item.key)"
        >
          <span class="material-symbols-outlined">{{ item.icon }}</span>
          <div>
            <strong>{{ item.label }}</strong>
            <small>{{ item.desc }}</small>
          </div>
          <em>升级</em>
        </button>
      </div>
    </div>
  </article>

  <article v-else-if="isRestrictionState" class="uc-card access-panel">
    <div class="state-tabs" role="tablist" aria-label="账号限制状态">
      <button
        type="button"
        :class="{ active: displayMode === 'expired' }"
        @click="displayMode = 'expired'"
      >
        过期降级
      </button>
      <button
        type="button"
        :class="{ active: displayMode === 'suspended' }"
        @click="displayMode = 'suspended'"
      >
        账户暂停
      </button>
    </div>

    <template v-if="displayMode === 'expired'">
      <div class="access-hero expired">
        <div>
          <span class="access-kicker">EXPIRED_DOWNGRADED</span>
          <h3>套餐已过期，已降级为免费版</h3>
          <p>基础会话、潜客和商户设置仍可使用；Pro 能力暂停。</p>
          <small v-if="expiredAt">到期时间：{{ expiredAt }}</small>
        </div>
        <button class="uc-btn uc-btn-primary" type="button" @click="$emit('upgrade', 'renew')">
          续费恢复
        </button>
      </div>

      <div class="access-section">
        <div class="access-section-head">
          <h4>当前可用功能</h4>
          <span>免费版</span>
        </div>
        <div class="feature-grid compact">
          <div v-for="item in expiredAvailableFeatures" :key="item.key" class="feature-card enabled">
            <span class="material-symbols-outlined">{{ item.icon }}</span>
            <strong>{{ item.label }}</strong>
            <small>可用</small>
          </div>
        </div>
      </div>

      <div class="access-section">
        <div class="access-section-head">
          <h4>已暂停功能</h4>
          <span>续费后恢复</span>
        </div>
        <div class="paused-list">
          <div v-for="item in pausedFeatures" :key="item.key" class="paused-item">
            <span class="material-symbols-outlined">{{ item.icon }}</span>
            <strong>{{ item.label }}</strong>
            <small>续费后恢复</small>
          </div>
        </div>
      </div>
    </template>

    <template v-else>
      <div class="access-hero suspended">
        <div>
          <span class="access-kicker">SUSPENDED</span>
          <h3>账户已暂停</h3>
          <p>平台已限制发送、群聊监听与财务操作。请勿重复提交 Bot Token。</p>
        </div>
        <button class="uc-btn uc-btn-secondary" type="button" @click="$emit('contact')">
          联系平台
        </button>
      </div>

      <div class="readonly-list">
        <button type="button" @click="$emit('navigate', 'chat')">
          <span class="material-symbols-outlined">history</span>
          <strong>查看历史会话</strong>
          <i class="material-symbols-outlined">chevron_right</i>
        </button>
        <button type="button" @click="$emit('navigate', 'saas')">
          <span class="material-symbols-outlined">receipt_long</span>
          <strong>查看账单</strong>
          <i class="material-symbols-outlined">chevron_right</i>
        </button>
        <button type="button" @click="$emit('navigate', 'more')">
          <span class="material-symbols-outlined">security</span>
          <strong>账号安全</strong>
          <i class="material-symbols-outlined">chevron_right</i>
        </button>
      </div>
    </template>
  </article>
</template>

<script setup lang="ts">
import { computed, ref, watch } from 'vue'
import type { MerchantState } from '@/lib/merchantState'

const props = defineProps<{
  profile: Record<string, any> | null
  subscription: Record<string, any> | null
  currentState: MerchantState
}>()

defineEmits<{
  upgrade: [feature: string]
  contact: []
  navigate: [tab: string]
}>()

const displayMode = ref<'expired' | 'suspended'>(props.currentState === 'SUSPENDED' ? 'suspended' : 'expired')

watch(
  () => props.currentState,
  state => {
    if (state === 'SUSPENDED') displayMode.value = 'suspended'
    if (state === 'EXPIRED_DOWNGRADED') displayMode.value = 'expired'
  }
)

const isRestrictionState = computed(() => props.currentState === 'EXPIRED_DOWNGRADED' || props.currentState === 'SUSPENDED')
const merchantName = computed(() => props.profile?.merchant_name || '当前商户')
const expiredAt = computed(() => formatDate(props.subscription?.end_at || props.subscription?.expired_at || props.subscription?.expires_at))

const freeFeatures = [
  { key: 'potential', icon: 'person_add', label: '潜客管理' },
  { key: 'base_chat', icon: 'chat', label: '基础会话' },
  { key: 'merchant_settings', icon: 'tune', label: '商户设置' },
  { key: 'ai_settings', icon: 'smart_toy', label: 'AI 设置' },
]

const lockedFeatures = [
  { key: 'dual_bot', icon: 'forum', label: '双向机器人', desc: '自动回复、人工接管与完整会话链路' },
  { key: 'broadcast', icon: 'campaign', label: '群发 broadcast', desc: '向用户、群组、频道批量推送' },
  { key: 'group_chat', icon: 'groups', label: '群聊 group_chat', desc: '群消息监听、成员与撤回同步' },
  { key: 'shared_ai_translate', icon: 'account_balance_wallet', label: '财务 AI 辅助', desc: '财务识别、翻译与核对辅助' },
  { key: 'advanced_analytics', icon: 'analytics', label: '高级分析', desc: '转化、监控与运营报表' },
]

const expiredAvailableFeatures = [
  { key: 'base_chat', icon: 'chat', label: '基础会话' },
  { key: 'potential', icon: 'person_add', label: '潜客' },
  { key: 'merchant_settings', icon: 'tune', label: '商户设置' },
  { key: 'ai_templates', icon: 'smart_toy', label: 'AI 基础模板' },
]

const pausedFeatures = [
  { key: 'broadcast', icon: 'campaign', label: '群发' },
  { key: 'group_chat', icon: 'groups', label: '群聊' },
  { key: 'finance', icon: 'account_balance_wallet', label: '财务' },
  { key: 'monitor', icon: 'analytics', label: '监控' },
]

function formatDate(value?: string | null) {
  if (!value) return ''
  const date = new Date(value)
  if (Number.isNaN(date.getTime())) return String(value)
  return date.toLocaleString('zh-CN', { year: 'numeric', month: '2-digit', day: '2-digit', hour: '2-digit', minute: '2-digit' })
}
</script>

<style scoped>
.access-panel {
  display: flex;
  flex-direction: column;
  gap: 14px;
}

.access-hero {
  display: flex;
  align-items: flex-start;
  justify-content: space-between;
  gap: 12px;
  padding: 15px;
  border-radius: 14px;
  border: 1px solid transparent;
}

.access-hero.free {
  background: linear-gradient(135deg, rgba(36, 161, 222, 0.10), rgba(16, 185, 129, 0.08));
  border-color: rgba(36, 161, 222, 0.20);
}

.access-hero.expired {
  background: linear-gradient(135deg, rgba(245, 158, 11, 0.12), rgba(36, 161, 222, 0.06));
  border-color: rgba(245, 158, 11, 0.24);
}

.access-hero.suspended {
  background: linear-gradient(135deg, rgba(239, 68, 68, 0.12), rgba(148, 163, 184, 0.08));
  border-color: rgba(239, 68, 68, 0.24);
}

.dark .access-hero.free {
  background: linear-gradient(135deg, rgba(36, 161, 222, 0.15), rgba(16, 185, 129, 0.10));
}

.dark .access-hero.expired {
  background: linear-gradient(135deg, rgba(245, 158, 11, 0.16), rgba(36, 161, 222, 0.08));
}

.dark .access-hero.suspended {
  background: linear-gradient(135deg, rgba(239, 68, 68, 0.16), rgba(71, 85, 105, 0.12));
}

.access-kicker {
  display: inline-flex;
  margin-bottom: 6px;
  font-size: 10px;
  font-weight: 900;
  letter-spacing: 0.06em;
  color: #0f6fb2;
}

.access-hero.expired .access-kicker {
  color: #b45309;
}

.access-hero.suspended .access-kicker {
  color: #b91c1c;
}

.dark .access-kicker {
  color: #8fd8ff;
}

.dark .access-hero.expired .access-kicker {
  color: #fbbf24;
}

.dark .access-hero.suspended .access-kicker {
  color: #fca5a5;
}

.access-hero h3 {
  margin: 0;
  font-size: 17px;
  line-height: 1.25;
  font-weight: 900;
  color: #0f172a;
}

.dark .access-hero h3 {
  color: #edf4ff;
}

.access-hero p,
.access-hero small {
  display: block;
  margin: 6px 0 0;
  font-size: 12px;
  color: #64748b;
}

.dark .access-hero p,
.dark .access-hero small {
  color: #adc3e0;
}

.access-section {
  display: flex;
  flex-direction: column;
  gap: 9px;
}

.access-section-head {
  display: flex;
  justify-content: space-between;
  align-items: center;
  gap: 8px;
}

.access-section-head h4 {
  margin: 0;
  font-size: 14px;
  font-weight: 900;
  color: #0f172a;
}

.dark .access-section-head h4 {
  color: #edf4ff;
}

.access-section-head span {
  font-size: 11px;
  color: #64748b;
  font-weight: 800;
}

.dark .access-section-head span {
  color: #adc3e0;
}

.feature-grid {
  display: grid;
  grid-template-columns: repeat(4, minmax(0, 1fr));
  gap: 8px;
}

.feature-grid.compact {
  grid-template-columns: repeat(2, minmax(0, 1fr));
}

.feature-card {
  min-height: 82px;
  border-radius: 13px;
  padding: 10px;
  border: 1px solid rgb(148 163 184 / 0.18);
  background: rgb(248 250 252 / 0.82);
  display: flex;
  flex-direction: column;
  gap: 5px;
}

.dark .feature-card {
  border-color: rgb(73 106 148 / 0.26);
  background: rgb(9 20 35 / 0.76);
}

.feature-card.enabled {
  border-color: rgba(16, 185, 129, 0.22);
  background: rgba(236, 253, 245, 0.74);
}

.dark .feature-card.enabled {
  background: rgba(6, 78, 59, 0.18);
}

.feature-card .material-symbols-outlined {
  font-size: 22px;
  color: #0f6fb2;
}

.feature-card strong {
  font-size: 12px;
  line-height: 1.25;
  color: #0f172a;
}

.dark .feature-card strong {
  color: #edf4ff;
}

.feature-card small {
  margin-top: auto;
  width: fit-content;
  padding: 2px 6px;
  border-radius: 999px;
  background: rgba(16, 185, 129, 0.12);
  color: #047857;
  font-size: 10px;
  font-weight: 900;
}

.dark .feature-card small {
  color: #6ee7b7;
}

.locked-list,
.paused-list,
.readonly-list {
  display: flex;
  flex-direction: column;
  gap: 8px;
}

.locked-item,
.paused-item,
.readonly-list button {
  width: 100%;
  border: 1px solid rgb(148 163 184 / 0.18);
  border-radius: 13px;
  background: rgb(248 250 252 / 0.82);
  display: grid;
  align-items: center;
  gap: 10px;
  padding: 11px 12px;
  text-align: left;
}

.locked-item {
  grid-template-columns: 28px 1fr auto;
  cursor: pointer;
}

.paused-item {
  grid-template-columns: 28px 1fr auto;
}

.readonly-list button {
  grid-template-columns: 28px 1fr 20px;
  cursor: pointer;
}

.dark .locked-item,
.dark .paused-item,
.dark .readonly-list button {
  border-color: rgb(73 106 148 / 0.26);
  background: rgb(9 20 35 / 0.74);
}

.locked-item > .material-symbols-outlined,
.paused-item > .material-symbols-outlined,
.readonly-list button > .material-symbols-outlined:first-child {
  color: #64748b;
}

.locked-item strong,
.paused-item strong,
.readonly-list strong {
  display: block;
  font-size: 13px;
  color: #0f172a;
}

.dark .locked-item strong,
.dark .paused-item strong,
.dark .readonly-list strong {
  color: #edf4ff;
}

.locked-item small,
.paused-item small {
  display: block;
  margin-top: 2px;
  color: #64748b;
  font-size: 11px;
}

.dark .locked-item small,
.dark .paused-item small {
  color: #adc3e0;
}

.locked-item em,
.paused-item small {
  justify-self: end;
  border-radius: 999px;
  padding: 3px 7px;
  background: rgba(36, 161, 222, 0.10);
  color: #0f6fb2;
  font-size: 10px;
  font-style: normal;
  font-weight: 900;
  white-space: nowrap;
}

.paused-item small {
  background: rgba(148, 163, 184, 0.14);
  color: #475569;
}

.state-tabs {
  display: grid;
  grid-template-columns: repeat(2, minmax(0, 1fr));
  gap: 4px;
  padding: 4px;
  border-radius: 13px;
  background: rgb(241 245 249 / 0.8);
}

.dark .state-tabs {
  background: rgb(9 20 35 / 0.8);
}

.state-tabs button {
  border: 0;
  border-radius: 10px;
  min-height: 34px;
  color: #64748b;
  background: transparent;
  font-size: 12px;
  font-weight: 900;
}

.dark .state-tabs button {
  color: #adc3e0;
}

.state-tabs button.active {
  color: #0f172a;
  background: #fff;
  box-shadow: 0 4px 12px rgba(15, 23, 42, 0.08);
}

.dark .state-tabs button.active {
  color: #edf4ff;
  background: rgb(20 38 63 / 0.92);
}

.readonly-list i {
  justify-self: end;
  font-style: normal;
  color: #94a3b8;
}

@media (max-width: 720px) {
  .access-hero {
    flex-direction: column;
  }

  .access-hero .uc-btn {
    width: 100%;
  }

  .feature-grid {
    grid-template-columns: repeat(2, minmax(0, 1fr));
  }
}
</style>
