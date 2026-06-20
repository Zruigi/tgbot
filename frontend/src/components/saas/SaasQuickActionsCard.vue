<template>
  <article class="uc-card quick-actions">
    <h3 class="uc-card-title">快捷入口</h3>
    <p class="uc-card-subtitle">快速访问核心业务功能</p>

    <div class="actions-grid">
      <div
        v-for="action in actions"
        :key="action.key"
        class="action-item"
        :class="{ disabled: !action.enabled }"
        @click="handleClick(action)"
      >
        <span class="action-icon material-symbols-outlined">{{ action.icon }}</span>
        <span class="action-name">{{ action.name }}</span>
        <span v-if="!action.enabled" class="action-tip">升级套餐解锁</span>
      </div>
    </div>
  </article>
</template>

<script setup lang="ts">
import { ref, computed, onMounted } from 'vue'
import { api } from '@/api'
import { SAAS_QUICK_ACTIONS, type AppTabId } from '@/app/navigation/app-tabs'

const emit = defineEmits<{
  upgradeGuide: [feature: string | undefined]
  navigate: [tab: string]
}>()

interface ActionItem {
  key: string
  name: string
  icon: string
  tab: AppTabId
  feature?: string
  enabled: boolean
  tip?: string
}

const features = ref<Record<string, boolean>>({})
const actions = ref<ActionItem[]>([])

const actionConfigs = SAAS_QUICK_ACTIONS

function checkFeature(featureKey?: string): boolean {
  if (!featureKey) return true
  return features.value[featureKey] ?? false
}

function getTip(featureKey?: string): string | undefined {
  if (!featureKey) return undefined
  if (!features.value[featureKey]) return '升级套餐解锁'
  return undefined
}

async function loadFeatures() {
  try {
    const res = await api.getMerchantFeatures()
    const data = res?.data
    if (data && typeof data === 'object' && !Array.isArray(data)) {
      features.value = Object.entries(data).reduce((acc, [key, value]) => {
        acc[key] = typeof value === 'boolean' ? value : Boolean((value as any)?.enabled ?? (value as any)?.is_enabled)
        return acc
      }, {} as Record<string, boolean>)
    }
  } catch (e) {
    console.error('加载功能开关失败', e)
  }

  actions.value = actionConfigs.map(cfg => ({
    ...cfg,
    enabled: checkFeature(cfg.feature),
    tip: getTip(cfg.feature)
  }))
}

function handleClick(action: ActionItem) {
  if (!action.enabled) {
    // 触发升级引导
    emit('upgradeGuide', action.feature)
    return
  }
  emit('navigate', action.tab)
}

onMounted(() => { void loadFeatures() })

defineExpose({ loadFeatures })
</script>

<style scoped>
.quick-actions {
  display: flex;
  flex-direction: column;
  gap: 12px;
}

.actions-grid {
  display: grid;
  grid-template-columns: repeat(3, 1fr);
  gap: 12px;
}

.action-item {
  display: flex;
  flex-direction: column;
  align-items: center;
  padding: 20px 16px;
  border-radius: 16px;
  background: rgb(248 250 252 / 0.8);
  cursor: pointer;
  transition: all 0.2s;
  position: relative;
  overflow: hidden;
  border: 1px solid transparent;
}

.dark .action-item {
  background: rgb(30 41 59 / 0.4);
}

.action-item:hover:not(.disabled) {
  background: #eef2ff;
  border-color: rgb(79 70 229 / 0.2);
  transform: translateY(-2px);
}

.dark .action-item:hover:not(.disabled) {
  background: rgba(99,102,241,.1);
  border-color: rgb(99 102 241 / 0.3);
}

.action-item.disabled {
  opacity: 0.6;
  cursor: not-allowed;
}

.action-icon {
  font-size: 32px;
  margin-bottom: 8px;
  color: #4f46e5;
}

.dark .action-icon {
  color: #818cf8;
}

.action-name {
  font-size: 14px;
  font-weight: 600;
  color: #111827;
}

.dark .action-name {
  color: #f1f5f9;
}

.action-tip {
  position: absolute;
  bottom: 4px;
  font-size: 10px;
  color: #9ca3af;
}

.dark .action-tip {
  color: #64748b;
}

@media (max-width: 640px) {
  .actions-grid {
    grid-template-columns: repeat(2, 1fr);
  }
}
</style>
