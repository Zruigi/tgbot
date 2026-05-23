<template>
  <article class="uc-card features">
    <h3 class="uc-card-title">功能开通</h3>
    <p class="uc-card-subtitle">查看已开通功能和 AI 配额</p>

    <div v-if="loading" class="uc-empty">加载中...</div>

    <div v-else class="feature-list">
      <div
        v-for="feat in features"
        :key="feat.key"
        class="feature-item"
        :class="{ enabled: feat.enabled }"
      >
        <span class="feature-name">{{ formatFeatureName(feat.key) }}</span>
        <span class="feature-status" :class="feat.enabled ? 'text-emerald-600' : ''">
          {{ feat.enabled ? '已开通' : '未开通' }}
        </span>
      </div>
      <div v-if="features.length === 0" class="uc-empty">暂无功能数据</div>
    </div>

    <div v-if="aiQuota" class="quota-section">
      <h4 class="sub-title">AI 配额</h4>
      <div class="quota-bar">
        <div class="quota-fill" :style="{ width: quotaPercent + '%' }"></div>
      </div>
      <div class="quota-info">
        <span>剩余: {{ quotaText }}</span>
        <span>总额度: {{ quotaTotalText }}</span>
      </div>
    </div>

    <div v-if="aiUsage" class="usage-section">
      <h4 class="sub-title">近 {{ aiUsage.days || 7 }} 天用量</h4>
      <div class="uc-grid uc-grid-cols-3" style="gap: 12px">
        <div class="uc-stat">
          <span class="uc-stat-label">调用次数</span>
          <div class="uc-stat-value">{{ aiUsage.total_calls ?? 0 }}</div>
        </div>
        <div class="uc-stat">
          <span class="uc-stat-label">消耗 Token</span>
          <div class="uc-stat-value">{{ formatToken(aiUsage.total_tokens) }}</div>
        </div>
        <div class="uc-stat">
          <span class="uc-stat-label">估算费用</span>
          <div class="uc-stat-value">${{ (aiUsage.estimated_cost || 0).toFixed(4) }}</div>
        </div>
      </div>
    </div>
  </article>
</template>

<script setup lang="ts">
import { ref, computed, onMounted } from 'vue'
import { api } from '@/api'

const features = ref<{ key: string; enabled: boolean }[]>([])
const aiQuota = ref<Record<string, any> | null>(null)
const aiUsage = ref<Record<string, any> | null>(null)
const loading = ref(false)

const featureNameMap: Record<string, string> = {
  dual_bot: '双向机器人',
  broadcast: '群发功能',
  group_chat: '群聊功能',
  subscription_check: '订阅查验',
  shared_ai_translate: 'AI 翻译',
  advanced_analytics: '高级分析',
  custom_bot: '自定义机器人',
  api_access: 'API 访问',
}

function formatFeatureName(key: string): string {
  return featureNameMap[key] || key
}

const quotaLeft = computed(() => {
  const v = aiQuota.value
  if (!v) return 0
  return v.remaining ?? v.remaining_quota ?? v.quota_remaining ?? v.left ?? 0
})

const quotaTotal = computed(() => {
  const v = aiQuota.value
  if (!v) return 0
  return v.limit ?? v.quota_limit ?? v.total ?? v.monthly_limit ?? 0
})

const quotaPercent = computed(() => {
  if (quotaTotal.value <= 0) return 0
  return Math.min(100, (quotaLeft.value / quotaTotal.value) * 100)
})

const quotaText = computed(() => {
  const v = quotaLeft.value
  return typeof v === 'number' ? v.toLocaleString() : String(v)
})

const quotaTotalText = computed(() => {
  const v = quotaTotal.value
  return typeof v === 'number' ? v.toLocaleString() : String(v)
})

function formatToken(tokens?: number): string {
  if (!tokens) return '0'
  if (tokens >= 1000000) return (tokens / 1000000).toFixed(1) + 'M'
  if (tokens >= 1000) return (tokens / 1000).toFixed(1) + 'K'
  return String(tokens)
}

async function loadData() {
  loading.value = true
  try {
    const [featRes, quotaRes, usageRes] = await Promise.all([
      api.getMerchantFeatures(),
      api.getMerchantAiQuota(),
      api.getMerchantAiUsage(7)
    ])

    const featData = featRes?.data
    if (featData && typeof featData === 'object' && !Array.isArray(featData)) {
      features.value = Object.entries(featData).map(([key, value]) => ({
        key,
        enabled: typeof value === 'boolean' ? value : Boolean((value as any)?.enabled ?? (value as any)?.is_enabled)
      }))
    } else {
      features.value = []
    }

    aiQuota.value = quotaRes?.data ?? null
    aiUsage.value = usageRes?.data ?? null
  } catch (e) {
    console.error('加载功能数据失败', e)
  } finally {
    loading.value = false
  }
}

onMounted(() => { void loadData() })

defineExpose({ loadData })
</script>

<style scoped>
.features {
  display: flex;
  flex-direction: column;
  gap: 12px;
}

.feature-list {
  display: flex;
  flex-direction: column;
  gap: 8px;
}

.feature-item {
  display: flex;
  justify-content: space-between;
  align-items: center;
  padding: 12px 16px;
  border-radius: 12px;
  background: rgb(248 250 252 / 0.8);
}

.dark .feature-item {
  background: rgb(30 41 59 / 0.4);
}

.feature-item.enabled {
  background: rgba(36,161,222,.08);
}

.dark .feature-item.enabled {
  background: rgba(36,161,222,.12);
}

.feature-name {
  font-size: 14px;
  color: #111827;
}

.dark .feature-name {
  color: #f1f5f9;
}

.feature-status {
  font-size: 12px;
  font-weight: 700;
  color: #9ca3af;
}

.dark .feature-status {
  color: #94a3b8;
}

.feature-item.enabled .feature-status {
  color: #0f6fb2;
}

.dark .feature-item.enabled .feature-status {
  color: #8fd8ff;
}

.quota-section,
.usage-section {
  margin-top: 8px;
  padding-top: 16px;
  border-top: 1px solid rgb(148 163 184 / 0.18);
}

.dark .quota-section,
.dark .usage-section {
  border-top-color: rgb(71 85 105 / 0.22);
}

.sub-title {
  margin: 0 0 12px;
  font-size: 14px;
  font-weight: 600;
  color: #374151;
}

.dark .sub-title {
  color: #d1d5db;
}

.quota-bar {
  height: 8px;
  background: rgb(226 232 240 / 0.8);
  border-radius: 4px;
  overflow: hidden;
  margin-bottom: 8px;
}

.dark .quota-bar {
  background: rgb(51 65 85 / 0.5);
}

.quota-fill {
  height: 100%;
  background: linear-gradient(90deg, #4f46e5, #7c3aed);
  border-radius: 4px;
  transition: width 0.3s;
}

.quota-info {
  display: flex;
  justify-content: space-between;
  font-size: 13px;
  color: #6b7280;
}

.dark .quota-info {
  color: #9ca3af;
}

@media (max-width: 640px) {
  .uc-grid-cols-3 {
    grid-template-columns: 1fr;
  }
}
</style>
