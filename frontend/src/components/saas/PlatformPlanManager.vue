<template>
  <section class="uc-card plan-admin">
    <div class="plan-admin__header">
      <div>
        <h3 class="uc-card-title">套餐管理</h3>
        <p class="uc-card-subtitle">可视化新增和修改套餐名称、金额、周期与状态</p>
      </div>
      <button class="uc-btn uc-btn-primary" @click="startCreate">新增套餐</button>
    </div>

    <div v-if="loading" class="uc-empty">加载中...</div>

    <div v-else class="plan-admin__grid">
      <button
        v-for="plan in plans"
        :key="plan.plan_code"
        class="plan-admin__item"
        :class="{ active: draft.plan_code === plan.plan_code }"
        @click="selectPlan(plan)"
      >
        <div class="plan-admin__item-top">
          <span class="plan-admin__name">{{ plan.plan_name }}</span>
          <span class="plan-admin__badge" :class="plan.status === 'active' ? 'is-active' : 'is-inactive'">
            {{ plan.status === 'active' ? '启用中' : '已停用' }}
          </span>
        </div>
        <div class="plan-admin__meta">{{ plan.plan_code }}</div>
        <div class="plan-admin__price">{{ formatPrice(plan.price_usdt) }} USDT / {{ plan.duration_days }} 天</div>
      </button>
    </div>

    <div class="plan-admin__editor">
      <div class="plan-admin__form-grid">
        <label class="uc-field">
          <span class="uc-label">套餐代号</span>
          <input v-model="draft.plan_code" class="uc-input" placeholder="如 free / pro / vip" :disabled="saving || !isCreating" />
        </label>
        <label class="uc-field">
          <span class="uc-label">套餐名称</span>
          <input v-model="draft.plan_name" class="uc-input" placeholder="如 免费套餐 / 专业版" :disabled="saving" />
        </label>
        <label class="uc-field">
          <span class="uc-label">金额 (USDT)</span>
          <input v-model.number="draft.price_usdt" class="uc-input" type="number" min="0" step="0.001" :disabled="saving" />
        </label>
        <label class="uc-field">
          <span class="uc-label">周期 (天)</span>
          <input v-model.number="draft.duration_days" class="uc-input" type="number" min="1" step="1" :disabled="saving" />
        </label>
        <label class="uc-field">
          <span class="uc-label">状态</span>
          <select v-model="draft.status" class="uc-input" :disabled="saving">
            <option value="active">启用</option>
            <option value="inactive">停用</option>
          </select>
        </label>
      </div>

      <label class="uc-field">
        <span class="uc-label">功能快照 JSON</span>
        <textarea
          v-model="draft.features_json"
          class="plan-admin__textarea"
          rows="6"
          :disabled="saving"
          placeholder='{"dual_bot": true, "broadcast": true}'
        ></textarea>
      </label>

      <div class="plan-admin__actions">
        <button class="uc-btn uc-btn-secondary" :disabled="saving" @click="resetDraft">重置</button>
        <button class="uc-btn uc-btn-primary" :disabled="saving" @click="savePlan">
          {{ saving ? '保存中...' : (isCreating ? '创建套餐' : '保存修改') }}
        </button>
      </div>

      <p v-if="message" class="plan-admin__message" :class="messageType">{{ message }}</p>
    </div>
  </section>
</template>

<script setup lang="ts">
import { onMounted, ref } from 'vue'
import { api } from '@/api'

type PlanDraft = {
  plan_code: string
  plan_name: string
  price_usdt: number
  duration_days: number
  features_json: string
  status: string
}

const plans = ref<Record<string, any>[]>([])
const loading = ref(false)
const saving = ref(false)
const isCreating = ref(false)
const message = ref('')
const messageType = ref<'success' | 'error'>('success')

const emptyDraft = (): PlanDraft => ({
  plan_code: '',
  plan_name: '',
  price_usdt: 0,
  duration_days: 30,
  features_json: JSON.stringify({ base_bot: true }, null, 2),
  status: 'active',
})

const draft = ref<PlanDraft>(emptyDraft())

function formatPrice(value: unknown): string {
  const num = Number(value ?? 0)
  return Number.isFinite(num) ? num.toFixed(2) : '0.00'
}

function normalizeFeaturesJson(input: string): string {
  try {
    return JSON.stringify(JSON.parse(input || '{}'))
  } catch {
    throw new Error('功能快照 JSON 格式不正确')
  }
}

async function loadPlans() {
  loading.value = true
  try {
    const res = await api.getPlatformPlans()
    plans.value = res?.data ?? []
    if (!isCreating.value && plans.value.length && !draft.value.plan_code) {
      selectPlan(plans.value[0])
    }
  } catch (e: any) {
    message.value = e?.message || '加载套餐失败'
    messageType.value = 'error'
  } finally {
    loading.value = false
  }
}

function selectPlan(plan: Record<string, any>) {
  isCreating.value = false
  draft.value = {
    plan_code: String(plan.plan_code || ''),
    plan_name: String(plan.plan_name || ''),
    price_usdt: Number(plan.price_usdt || 0),
    duration_days: Number(plan.duration_days || 30),
    features_json: JSON.stringify(
      typeof plan.features_json === 'string' ? JSON.parse(plan.features_json || '{}') : (plan.features_json || {}),
      null,
      2,
    ),
    status: String(plan.status || 'active'),
  }
  message.value = ''
}

function startCreate() {
  isCreating.value = true
  draft.value = emptyDraft()
  message.value = ''
}

function resetDraft() {
  if (isCreating.value) {
    draft.value = emptyDraft()
    return
  }
  const current = plans.value.find((item) => item.plan_code === draft.value.plan_code)
  if (current) selectPlan(current)
}

async function savePlan() {
  if (!draft.value.plan_code.trim()) {
    message.value = '套餐代号不能为空'
    messageType.value = 'error'
    return
  }
  if (!draft.value.plan_name.trim()) {
    message.value = '套餐名称不能为空'
    messageType.value = 'error'
    return
  }

  saving.value = true
  message.value = ''
  try {
    const payload = {
      plan_code: draft.value.plan_code.trim(),
      plan_name: draft.value.plan_name.trim(),
      price_usdt: Number(draft.value.price_usdt || 0),
      duration_days: Number(draft.value.duration_days || 30),
      status: draft.value.status || 'active',
      features_json: normalizeFeaturesJson(draft.value.features_json),
    }
    const res = await api.upsertPlatformPlan(payload)
    if (res?.status === 'ok') {
      message.value = isCreating.value ? '套餐创建成功' : '套餐更新成功'
      messageType.value = 'success'
      isCreating.value = false
      await loadPlans()
      const current = plans.value.find((item) => item.plan_code === payload.plan_code)
      if (current) selectPlan(current)
    } else {
      message.value = res?.message || '保存失败'
      messageType.value = 'error'
    }
  } catch (e: any) {
    message.value = e?.message || '保存失败'
    messageType.value = 'error'
  } finally {
    saving.value = false
  }
}

onMounted(() => {
  void loadPlans()
})
</script>

<style scoped>
.plan-admin {
  display: flex;
  flex-direction: column;
  gap: 16px;
}

.plan-admin__header {
  display: flex;
  justify-content: space-between;
  align-items: center;
  gap: 12px;
}

.plan-admin__grid {
  display: grid;
  grid-template-columns: repeat(auto-fit, minmax(220px, 1fr));
  gap: 12px;
}

.plan-admin__item {
  text-align: left;
  border: 1px solid rgb(36 161 222 / 0.16);
  border-radius: 18px;
  padding: 16px;
  background: linear-gradient(180deg, rgb(248 252 255 / 0.96), rgb(240 248 255 / 0.92));
  transition: all .18s ease;
}

.plan-admin__item:hover,
.plan-admin__item.active {
  border-color: rgb(36 161 222 / 0.4);
  box-shadow: 0 18px 34px rgba(36, 161, 222, 0.10);
  transform: translateY(-2px);
}

.dark .plan-admin__item {
  border-color: rgb(36 161 222 / 0.18);
  background: linear-gradient(180deg, rgb(9 19 34 / 0.92), rgb(12 28 49 / 0.88));
}

.dark .plan-admin__item:hover,
.dark .plan-admin__item.active {
  border-color: rgb(82 190 255 / 0.34);
  box-shadow: 0 18px 34px rgba(15, 71, 125, 0.24);
}

.plan-admin__item-top {
  display: flex;
  justify-content: space-between;
  gap: 8px;
  align-items: center;
}

.plan-admin__name {
  font-weight: 800;
  color: #0f172a;
}

.dark .plan-admin__name {
  color: #edf4ff;
}

.plan-admin__badge {
  border-radius: 999px;
  padding: 4px 10px;
  font-size: 11px;
  font-weight: 700;
}

.plan-admin__badge.is-active {
  background: rgba(36, 161, 222, 0.12);
  color: #0f6fb2;
}

.plan-admin__badge.is-inactive {
  background: rgba(148, 163, 184, 0.16);
  color: #475569;
}

.plan-admin__meta {
  margin-top: 10px;
  color: #5f7c98;
  font-size: 12px;
}

.plan-admin__price {
  margin-top: 6px;
  font-size: 18px;
  font-weight: 800;
  color: #0b6aa7;
}

.dark .plan-admin__price {
  color: #7cd2ff;
}

.plan-admin__editor {
  border: 1px solid rgb(36 161 222 / 0.14);
  border-radius: 20px;
  padding: 18px;
  background: linear-gradient(180deg, rgba(246, 251, 255, 0.95), rgba(237, 247, 255, 0.92));
}

.dark .plan-admin__editor {
  border-color: rgb(36 161 222 / 0.18);
  background: linear-gradient(180deg, rgba(8, 22, 39, 0.92), rgba(11, 30, 52, 0.88));
}

.plan-admin__form-grid {
  display: grid;
  grid-template-columns: repeat(auto-fit, minmax(180px, 1fr));
  gap: 12px;
}

.plan-admin__textarea {
  width: 100%;
  min-height: 128px;
  padding: 12px 14px;
  border-radius: 16px;
  border: 1px solid rgb(36 161 222 / 0.16);
  background: rgb(255 255 255 / 0.78);
  color: #0f172a;
  font: inherit;
  resize: vertical;
}

.plan-admin__textarea:focus {
  outline: none;
  border-color: rgb(36 161 222 / 0.45);
  box-shadow: 0 0 0 4px rgba(36, 161, 222, 0.10);
}

.dark .plan-admin__textarea {
  background: rgb(8 17 31 / 0.72);
  color: #edf4ff;
  border-color: rgb(36 161 222 / 0.18);
}

.plan-admin__actions {
  display: flex;
  justify-content: flex-end;
  gap: 10px;
}

.plan-admin__message {
  margin: 0;
  padding: 12px 14px;
  border-radius: 14px;
  font-size: 14px;
}

.plan-admin__message.success {
  background: rgba(36, 161, 222, 0.12);
  color: #0f6fb2;
}

.plan-admin__message.error {
  background: rgba(239, 68, 68, 0.12);
  color: #b91c1c;
}

@media (max-width: 720px) {
  .plan-admin__header {
    flex-direction: column;
    align-items: stretch;
  }
}
</style>
