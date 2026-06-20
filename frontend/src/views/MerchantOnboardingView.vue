<template>
  <div class="onboarding-page">
    <!-- 加载动画 -->
    <div v-if="loading" class="loading-overlay">
      <div class="loading-spinner"></div>
      <p class="loading-text">{{ loadingText }}</p>
    </div>

    <div class="onboarding-card">
      <!-- 步骤指示器 -->
      <div class="steps">
        <div class="step" :class="{ active: currentStep >= 1, completed: currentStep > 1 }">
          <div class="step-number">1</div>
          <div class="step-label">注册Bot</div>
        </div>
        <div class="step-line" :class="{ active: currentStep > 1 }"></div>
        <div class="step" :class="{ active: currentStep >= 2, completed: currentStep > 2 }">
          <div class="step-number">2</div>
          <div class="step-label">选择套餐</div>
        </div>
        <div class="step-line" :class="{ active: currentStep > 2 }"></div>
        <div class="step" :class="{ active: currentStep >= 3, completed: currentStep > 3 }">
          <div class="step-number">3</div>
          <div class="step-label">完成接入</div>
        </div>
      </div>

      <!-- 步骤1: 注册Bot -->
      <div v-if="currentStep === 1" class="step-content">
        <h2>注册 Telegram Bot</h2>
        <p class="description">请按照以下步骤创建并注册您的Telegram Bot：</p>
        
        <div class="instructions">
          <div class="instruction-item">
            <span class="number">1</span>
            <p>在Telegram中搜索 <strong>@BotFather</strong></p>
          </div>
          <div class="instruction-item">
            <span class="number">2</span>
            <p>发送 <code>/newbot</code> 创建新Bot</p>
          </div>
          <div class="instruction-item">
            <span class="number">3</span>
            <p>按提示设置Bot名称和用户名</p>
          </div>
          <div class="instruction-item">
            <span class="number">4</span>
            <p>复制Bot Token并粘贴到下方</p>
          </div>
        </div>

        <div class="form-group">
          <label>Bot Token</label>
          <input
            v-model="botToken"
            type="text"
            placeholder="例如：123456789:ABCdefGHIjklMNOpqrsTUVwxyz"
            class="input-field"
            :disabled="submitting"
          />
          <p v-if="error" class="error-text">{{ error }}</p>
        </div>

        <button class="btn btn-primary w-full" :disabled="submitting || !botToken" @click="submitBotToken">
          {{ submitting ? '验证中...' : '提交 Bot Token' }}
        </button>
      </div>

      <!-- 步骤2: 选择套餐 -->
      <div v-if="currentStep === 2" class="step-content">
        <h2>选择套餐</h2>
        <p class="description">请选择适合您的套餐方案：</p>

        <div class="plan-cards">
          <div
            v-for="plan in availablePlans"
            :key="plan.plan_code"
            class="plan-card"
            :class="{ selected: selectedPlan === plan.plan_code }"
            @click="selectedPlan = plan.plan_code"
          >
            <div class="plan-header">
              <h3>{{ plan.plan_name }}</h3>
              <span class="plan-price">{{ formatPrice(plan.price_usdt) }} USDT</span>
            </div>
            <ul class="plan-features">
              <li v-for="feature in parseFeatures(plan.features_json)" :key="feature">✓ {{ feature }}</li>
              <li>{{ plan.duration_days }} 天服务周期</li>
            </ul>
          </div>
        </div>

        <p v-if="error" class="error-text">{{ error }}</p>

        <button class="btn btn-primary w-full" :disabled="submitting || !selectedPlan" @click="selectPlan">
          {{ submitting ? '处理中...' : (selectedPlan === 'free' ? '立即开始' : '创建订单') }}
        </button>
      </div>

      <!-- 步骤3: 完成接入 -->
      <div v-if="currentStep === 3" class="step-content">
        <h2>🎉 接入完成！</h2>
        <div class="status-icon success">
          <span class="material-symbols-outlined">check_circle</span>
        </div>
        <p class="description">您的Bot已成功接入，正在加载工作台...</p>

        <div class="progress-bar">
          <div class="progress-fill" :style="{ width: progressWidth + '%' }"></div>
        </div>
        <p class="progress-text">{{ progressText }}</p>
      </div>
    </div>
  </div>
</template>

<script setup lang="ts">
import { ref, onMounted } from 'vue'
import { useRouter } from 'vue-router'
import { useAuthStore } from '@/stores/auth'
import * as api from '@/api'
import { resolveMerchantState } from '@/lib/merchantState'

const router = useRouter()
const auth = useAuthStore()

const currentStep = ref(1)
const botToken = ref('')
const selectedPlan = ref('free')
const availablePlans = ref<Record<string, any>[]>([])
const submitting = ref(false)
const error = ref('')
const loading = ref(false)
const loadingText = ref('加载中...')
const progressWidth = ref(0)
const progressText = ref('正在初始化...')

onMounted(async () => {
  await Promise.all([checkMerchantStatus(), loadPlans()])
})

function formatPrice(value: unknown): string {
  const num = Number(value ?? 0)
  return Number.isFinite(num) ? num.toFixed(2) : '0.00'
}

function parseFeatures(featuresJson: unknown): string[] {
  try {
    const parsed = typeof featuresJson === 'string' ? JSON.parse(featuresJson || '{}') : (featuresJson || {})
    const enabled = Object.keys(parsed).filter((key) => Boolean(parsed[key]))
    return enabled.length ? enabled : ['基础功能']
  } catch {
    return ['基础功能']
  }
}

async function loadPlans() {
  try {
    const res = await api.getMerchantPlans()
    availablePlans.value = res?.data ?? []
    if (!availablePlans.value.length) {
      availablePlans.value = [{
        plan_code: 'free',
        plan_name: '免费套餐',
        price_usdt: 0,
        duration_days: 30,
        features_json: JSON.stringify({ base_bot: true, dual_bot: true, group_chat: true }),
      }]
    }
    if (!availablePlans.value.some((item) => item.plan_code === selectedPlan.value)) {
      selectedPlan.value = String(availablePlans.value[0]?.plan_code || 'free')
    }
  } catch (e) {
    console.error('Load merchant plans failed:', e)
  }
}

async function checkMerchantStatus() {
  loading.value = true
  loadingText.value = '检查商户状态...'
  
  try {
    const profile = await api.getMerchantProfile()
    if (profile?.status === 'ok' && profile?.data) {
      const merchant = profile.data
      const hasBot = !!(merchant.bot_id || merchant.bot_token)
      const onboardingCompleted = !!merchant.onboarding_completed
      const merchantState = resolveMerchantState(merchant, null)

      console.log('[Onboarding] Merchant state:', merchantState, 'hasBot:', hasBot, 'onboardingCompleted:', onboardingCompleted, merchant)

      // 如果已完成 onboarding，直接跳转工作台
      if (onboardingCompleted && (merchantState === 'ACTIVE_FREE' || merchantState === 'ACTIVE_PRO')) {
        currentStep.value = 3
        router.push('/saas')
        return
      }

      // 如果已有bot但未完成onboarding，跳到步骤2（选择套餐）
      if (hasBot && !onboardingCompleted) {
        currentStep.value = 2
      }
    }
  } catch (e: any) {
    console.error('Check merchant status failed:', e)
  } finally {
    loading.value = false
  }
}

async function submitBotToken() {
  if (!botToken.value || submitting.value) return
  
  submitting.value = true
  error.value = ''
  
  try {
    const res = await api.saveMerchantBotToken({ bot_token: botToken.value })
    if (res?.status === 'ok') {
      currentStep.value = 2 // 进入选择套餐步骤
    } else {
      error.value = res?.message || '提交失败'
    }
  } catch (e: any) {
    error.value = e?.message || '提交失败，请重试'
  } finally {
    submitting.value = false
  }
}

async function selectPlan() {
  if (submitting.value) return
  
  submitting.value = true
  error.value = ''
  
  try {
    if (selectedPlan.value === 'free') {
      // 免费套餐，直接完成接入
      currentStep.value = 3
      await completeOnboarding()
    } else {
      // 付费套餐，创建支付订单
      const order = await api.createMerchantPaymentOrder({ plan_code: selectedPlan.value })
      if (order?.status === 'ok') {
        // 跳转到支付页面
        router.push('/saas')
      } else {
        error.value = order?.message || '创建订单失败'
      }
    }
  } catch (e: any) {
    error.value = e?.message || '操作失败，请重试'
  } finally {
    submitting.value = false
  }
}

async function completeOnboarding() {
  // 显示加载动画
  loading.value = true
  loadingText.value = '正在完成接入...'
  
  // 模拟加载进度
  const steps = [
    { width: 20, text: '正在初始化功能...' },
    { width: 40, text: '正在配置配额...' },
    { width: 60, text: '正在生成快照...' },
    { width: 80, text: '正在更新状态...' },
    { width: 100, text: '接入完成！' },
  ]
  
  for (const step of steps) {
    progressWidth.value = step.width
    progressText.value = step.text
    await new Promise(resolve => setTimeout(resolve, 500))
  }
  
  // 标注商户为已完成注册
  try {
    await api.updateMerchantStatus({ 
      status: 'active_free',
      onboarding_completed: true 
    })
  } catch (e: any) {
    console.error('Update merchant status failed:', e)
  }
  
  // 等待1秒后跳转到工作台
  await new Promise(resolve => setTimeout(resolve, 1000))
  
  loading.value = false
  router.push('/saas')
}
</script>

<style scoped>
.onboarding-page {
  min-height: 100vh;
  display: flex;
  align-items: center;
  justify-content: center;
  background:
    radial-gradient(circle at top, rgba(36, 161, 222, 0.20), transparent 45%),
    linear-gradient(135deg, #edf8ff 0%, #d9f0ff 48%, #eff7ff 100%);
  padding: 20px;
}

.loading-overlay {
  position: fixed;
  top: 0;
  left: 0;
  right: 0;
  bottom: 0;
  background: rgba(0, 0, 0, 0.7);
  display: flex;
  flex-direction: column;
  align-items: center;
  justify-content: center;
  z-index: 1000;
}

.loading-spinner {
  width: 50px;
  height: 50px;
  border: 4px solid #f3f3f3;
  border-top: 4px solid #24a1de;
  border-radius: 50%;
  animation: spin 1s linear infinite;
}

@keyframes spin {
  0% { transform: rotate(0deg); }
  100% { transform: rotate(360deg); }
}

.loading-text {
  color: white;
  margin-top: 20px;
  font-size: 18px;
}

.onboarding-card {
  background: white;
  border-radius: 16px;
  padding: 40px;
  max-width: 600px;
  width: 100%;
  box-shadow: 0 10px 40px rgba(0, 0, 0, 0.2);
}

.steps {
  display: flex;
  align-items: center;
  justify-content: space-between;
  margin-bottom: 40px;
}

.step {
  display: flex;
  flex-direction: column;
  align-items: center;
  flex: 1;
}

.step-number {
  width: 40px;
  height: 40px;
  border-radius: 50%;
  background: #e0e0e0;
  color: #999;
  display: flex;
  align-items: center;
  justify-content: center;
  font-weight: bold;
  font-size: 18px;
  transition: all 0.3s;
}

.step.active .step-number {
  background: #24a1de;
  color: white;
}

.step.completed .step-number {
  background: #4caf50;
  color: white;
}

.step-label {
  margin-top: 8px;
  font-size: 14px;
  color: #666;
}

.step.active .step-label {
  color: #24a1de;
  font-weight: bold;
}

.step-line {
  flex: 1;
  height: 2px;
  background: #e0e0e0;
  margin: 0 10px;
  margin-bottom: 24px;
  transition: all 0.3s;
}

.step-line.active {
  background: #24a1de;
}

.step-content {
  animation: fadeIn 0.3s;
}

@keyframes fadeIn {
  from { opacity: 0; transform: translateY(10px); }
  to { opacity: 1; transform: translateY(0); }
}

h2 {
  font-size: 24px;
  margin-bottom: 16px;
  color: #333;
}

.description {
  color: #666;
  margin-bottom: 24px;
}

.instructions {
  margin-bottom: 24px;
}

.instruction-item {
  display: flex;
  align-items: center;
  margin-bottom: 12px;
}

.instruction-item .number {
  width: 28px;
  height: 28px;
  border-radius: 50%;
  background: #24a1de;
  color: white;
  display: flex;
  align-items: center;
  justify-content: center;
  font-weight: bold;
  margin-right: 12px;
  flex-shrink: 0;
}

.form-group {
  margin-bottom: 24px;
}

label {
  display: block;
  margin-bottom: 8px;
  font-weight: bold;
  color: #333;
}

.input-field {
  width: 100%;
  padding: 12px;
  border: 2px solid #e0e0e0;
  border-radius: 8px;
  font-size: 16px;
  transition: border-color 0.3s;
}

.input-field:focus {
  outline: none;
  border-color: #24a1de;
}

.error-text {
  color: #f44336;
  margin-top: 8px;
}

.btn {
  padding: 12px 24px;
  border-radius: 8px;
  font-size: 16px;
  font-weight: bold;
  cursor: pointer;
  transition: all 0.3s;
  border: none;
}

.btn-primary {
  background: linear-gradient(135deg, #24a1de, #1d78d5);
  color: white;
}

.btn-primary:hover:not(:disabled) {
  filter: brightness(1.05);
}

.btn-primary:disabled {
  background: #ccc;
  cursor: not-allowed;
}

.w-full {
  width: 100%;
}

.plan-cards {
  display: grid;
  grid-template-columns: 1fr 1fr;
  gap: 16px;
  margin-bottom: 24px;
}

.plan-card {
  border: 1px solid rgba(36, 161, 222, 0.18);
  border-radius: 12px;
  padding: 20px;
  cursor: pointer;
  transition: all 0.3s;
  background: rgba(255, 255, 255, 0.72);
}

.plan-card:hover {
  border-color: rgba(36, 161, 222, 0.4);
  box-shadow: 0 14px 28px rgba(36, 161, 222, 0.12);
  transform: translateY(-2px);
}

.plan-card.selected {
  border-color: #24a1de;
  background: linear-gradient(180deg, rgba(228, 246, 255, 0.96), rgba(216, 240, 255, 0.92));
}

.plan-header {
  display: flex;
  justify-content: space-between;
  align-items: center;
  margin-bottom: 16px;
}

.plan-header h3 {
  font-size: 18px;
  color: #333;
}

.plan-price {
  font-size: 20px;
  font-weight: bold;
  color: #0f6fb2;
}

.plan-features {
  list-style: none;
  padding: 0;
  margin: 0;
}

.plan-features li {
  padding: 4px 0;
  color: #666;
}

.status-icon {
  text-align: center;
  margin: 24px 0;
}

.status-icon.success .material-symbols-outlined {
  font-size: 64px;
  color: #4caf50;
}

.progress-bar {
  width: 100%;
  height: 8px;
  background: #e0e0e0;
  border-radius: 4px;
  overflow: hidden;
  margin: 24px 0;
}

.progress-fill {
  height: 100%;
  background: linear-gradient(90deg, #24a1de, #1d78d5);
  transition: width 0.5s;
}

.progress-text {
  text-align: center;
  color: #666;
}
</style>
