<template>
  <div class="uc-page bg-surface settings-page">
    <header class="uc-header">
      <div class="uc-header-wrap">
        <div>
          <h2 class="uc-title">
            <span class="material-symbols-outlined text-primary">{{ isAiFocusMode ? 'smart_toy' : 'settings' }}</span>
            {{ isAiFocusMode ? 'AI 设置' : '系统设置' }}
          </h2>
          <p class="uc-subtitle">{{ headerSubtitle }}</p>
        </div>
        <div class="uc-actions">
          <button class="uc-btn uc-btn-primary" @click="saveSettings" :disabled="saving">
            {{ saving ? '保存中...' : '保存配置' }}
          </button>
        </div>
      </div>
    </header>

    <main ref="canvasRef" class="uc-main">
      <div class="uc-main-wrap settings-wrap">
        <section ref="aiEngineSectionRef" class="uc-card" :class="{ 'ai-focus-card': focusedSection === 'ai' }">
          <h3 class="uc-card-title">AI 翻译引擎配置</h3>
          <p class="uc-card-subtitle">配置自动翻译所使用的大模型 API</p>

          <div class="settings-stack">
            <div class="uc-field">
              <span class="uc-label">API 模式</span>
              <div class="mode-grid">
                <label class="mode-item" :class="{ active: config.llm_api_mode === 'rented' }">
                  <input type="radio" v-model="config.llm_api_mode" value="rented" />
                  <div>
                    <div class="mode-title">租用 API</div>
                    <div class="mode-desc">系统预置节点，开箱可用</div>
                  </div>
                </label>
                <label class="mode-item" :class="{ active: config.llm_api_mode === 'own' }">
                  <input type="radio" v-model="config.llm_api_mode" value="own" />
                  <div>
                    <div class="mode-title">自有 API</div>
                    <div class="mode-desc">使用私有密钥与专属模型线路</div>
                  </div>
                </label>
              </div>
            </div>

            <div v-if="config.llm_api_mode === 'own'" class="sub-panel">
              <div class="uc-grid uc-grid-cols-2">
                <label class="uc-field">
                  <span class="uc-label">API 厂商 / 模型系列</span>
                  <select v-model="config.llm_model_type" class="uc-input">
                    <option value="openai">OpenAI / 兼容接口</option>
                    <option value="deepseek">DeepSeek</option>
                    <option value="gemini">Google Gemini</option>
                  </select>
                </label>

                <label class="uc-field">
                  <span class="uc-label">模型名称</span>
                  <input v-model="config.llm_model" class="uc-input" placeholder="gpt-4.1-mini / gemini-2.5-flash" />
                </label>
              </div>

              <label class="uc-field">
                <span class="uc-label">API Base URL</span>
                <input v-model="config.llm_base_url" class="uc-input" placeholder="https://api.openai.com/v1" />
              </label>

              <label class="uc-field">
                <span class="uc-label">API Key</span>
                <input v-model="config.llm_api_key" type="password" class="uc-input" placeholder="sk-xxxxxxxxxxxxxxxx" />
                <p class="helper-text">留空表示不修改当前密钥。</p>
              </label>
            </div>

            <div class="sub-panel accent-panel">
              <h4 class="sub-title">Vertex 一键自动配置（推荐）</h4>
              <p class="helper-text">输入 Vertex Key 与 OneAPI 地址后，自动生成可直接使用的中转配置。</p>

              <div class="uc-grid uc-grid-cols-3">
                <label class="uc-field">
                  <span class="uc-label">Vertex API Key</span>
                  <input v-model="vertexQuickForm.vertex_api_key" type="password" class="uc-input" placeholder="AIza..." />
                </label>

                <label class="uc-field">
                  <span class="uc-label">OneAPI Base URL</span>
                  <input v-model="vertexQuickForm.oneapi_base_url" class="uc-input" placeholder="https://oneapi.example.com" />
                </label>

                <label class="uc-field">
                  <span class="uc-label">Gemini 模型</span>
                  <select v-model="vertexQuickForm.model" class="uc-input">
                    <option v-for="m in quickGeminiModels" :key="m" :value="m">{{ m }}</option>
                  </select>
                </label>
              </div>

              <label class="uc-field">
                <span class="uc-label">中转 SK（可选）</span>
                <input v-model="vertexQuickForm.relay_api_key" class="uc-input" placeholder="sk-your-own-relay-key" />
              </label>

              <div class="uc-actions">
                <button class="uc-btn uc-btn-primary" @click="oneClickVertexSetup" :disabled="quickSetting">
                  {{ quickSetting ? '配置中...' : '一键自动配置' }}
                </button>
              </div>

              <div v-if="quickResult" class="quick-result-grid">
                <label class="uc-field">
                  <span class="uc-label">中转 URL</span>
                  <pre class="debug-block">{{ quickResult.oneapi_base_url }}</pre>
                </label>
                <label class="uc-field">
                  <span class="uc-label">模型 ID</span>
                  <pre class="debug-block">{{ quickResult.model }}</pre>
                </label>
                <label class="uc-field">
                  <span class="uc-label">调用地址</span>
                  <pre class="debug-block">{{ quickResult.model_infer_url }}</pre>
                </label>
                <label class="uc-field">
                  <span class="uc-label">中转 SK</span>
                  <pre class="debug-block">{{ quickResult.relay_api_key }}</pre>
                </label>
              </div>
            </div>

            <div class="sub-panel lock-panel">
              <h4 class="sub-title">模型锁定</h4>
              <p class="helper-text">开启后，后端将强制使用 `gemini-3.1-pro-preview`，忽略请求里传入的其它模型。</p>
              <div class="model-lock-row">
                <div>
                  <div class="toggle-title">当前状态：{{ config.llm_model_lock_enabled ? '已锁定' : '未锁定' }}</div>
                  <div class="helper-text">锁定模型：{{ config.llm_model_lock_target || MODEL_LOCK_TARGET }}</div>
                </div>
                <button class="uc-btn" :class="config.llm_model_lock_enabled ? 'uc-btn-danger' : 'uc-btn-primary'" @click="toggleModelLock" :disabled="modelLockSaving">
                  {{ modelLockSaving ? '处理中...' : (config.llm_model_lock_enabled ? '解除锁定' : '锁定为 gemini-3.1-pro-preview') }}
                </button>
              </div>
            </div>

            <div v-if="config.llm_api_mode === 'rented'" class="sub-panel info-panel">
              当前为租用模式，系统自动选择稳定节点并托管更新，无需手动维护 API 参数。
            </div>
          </div>
        </section>

        <section class="uc-card">
          <h3 class="uc-card-title">翻译策略</h3>
          <p class="uc-card-subtitle">全局开关控制客服私聊消息自动翻译</p>
          <div class="toggle-row">
            <div>
              <div class="toggle-title">全局自动翻译</div>
              <div class="helper-text">开启后，客服私聊中的外语消息将自动翻译为中文。</div>
            </div>
            <div class="toggle-switch">
              <input type="checkbox" v-model="config.global_auto_translate" id="auto-trans" class="switch" />
              <label for="auto-trans"></label>
            </div>
          </div>
        </section>

        <section ref="aiSkillsSectionRef" class="uc-card" :class="{ 'ai-focus-card': focusedSection === 'skills' }">
          <h3 class="uc-card-title">AI Skills 管理</h3>
          <p class="uc-card-subtitle">仅管理项目隔离后的 Skills，避免读取服务器全局目录</p>

          <div class="settings-stack">
            <div class="uc-actions">
              <button class="uc-btn uc-btn-ghost" @click="loadAiSkills" :disabled="skillsLoading || skillsSaving || skillsImporting">
                {{ skillsLoading ? '刷新中...' : '刷新列表' }}
              </button>
              <button class="uc-btn uc-btn-primary" @click="importBuiltinSkills" :disabled="skillsImporting || skillsLoading">
                {{ skillsImporting ? '导入中...' : '导入本地 Skills' }}
              </button>
              <button class="uc-btn uc-btn-ghost" @click="openSkillDrawer" :disabled="skillsLoading || skillsImporting">
                查看 Skills
              </button>
            </div>
            <p class="helper-text">默认仅扫描项目内 `skills/`。可通过 `AI_SKILL_LOCAL_DIRS` 指定项目内子目录。</p>

            <div class="sub-panel">
              <div class="uc-grid uc-grid-cols-2">
                <label class="uc-field">
                  <span class="uc-label">Skill 名称</span>
                  <input v-model="skillForm.name" class="uc-input" placeholder="例如: group_ad_review" />
                </label>
                <label class="uc-field">
                  <span class="uc-label">描述</span>
                  <input v-model="skillForm.description" class="uc-input" placeholder="一句话描述这个技能用途" />
                </label>
              </div>
              <label class="uc-field">
                <span class="uc-label">Prompt 模板</span>
                <textarea v-model="skillForm.prompt_template" class="uc-input debug-textarea" placeholder="该技能的提示词模板"></textarea>
              </label>
              <div class="uc-actions">
                <button class="uc-btn uc-btn-primary" @click="saveAiSkill" :disabled="skillsSaving">
                  {{ skillsSaving ? '保存中...' : '保存 Skill' }}
                </button>
              </div>
            </div>

            <div class="helper-text">当前自定义 Skill：{{ aiSkills.length }} 个（列表在抽屉中查看）</div>
          </div>
        </section>

        <section class="uc-card">
          <h3 class="uc-card-title">外观与主题</h3>
          <p class="uc-card-subtitle">自定义背景图、暗黑模式与定时切换</p>

          <div class="settings-stack">
            <div class="uc-field">
              <span class="uc-label">全局背景图</span>
              <p class="helper-text">应用到整个后台页面。建议选择浅色或深色背景，以匹配当前主题模式。</p>
              <div class="upload-row">
                <input ref="bgInputRef" type="file" accept="image/*" class="hidden-input" @change="handleBgUpload" />
                <button type="button" class="uc-btn uc-btn-ghost" @click="bgInputRef?.click()">选择图片</button>
                <button
                  v-if="themeStore.settings.customBackground"
                  type="button"
                  class="uc-btn uc-btn-danger"
                  @click="resetBackground"
                >
                  重置背景
                </button>
                <span v-if="themeStore.settings.customBackground" class="bg-ready">已设置</span>
              </div>
            </div>

            <div class="uc-field">
              <span class="uc-label">聊天背景图</span>
              <p class="helper-text">仅应用在会话列表里的消息区域，不影响其它模块页面。</p>
              <div class="upload-row">
                <input ref="chatBgInputRef" type="file" accept="image/*" class="hidden-input" @change="handleChatBgUpload" />
                <button type="button" class="uc-btn uc-btn-ghost" @click="chatBgInputRef?.click()">选择图片</button>
                <button
                  v-if="themeStore.settings.chatBackground"
                  type="button"
                  class="uc-btn uc-btn-danger"
                  @click="resetChatBackground"
                >
                  重置背景
                </button>
                <span v-if="themeStore.settings.chatBackground" class="bg-ready">已设置</span>
              </div>
            </div>

            <label class="uc-field">
              <span class="uc-label">暗黑模式</span>
              <select v-model="themeForm.darkMode" class="uc-input" @change="onDarkModeChange">
                <option value="auto">跟随系统</option>
                <option value="light">始终浅色</option>
                <option value="dark">始终深色</option>
                <option value="schedule">定时切换</option>
              </select>
              <p class="helper-text">
                {{ themeForm.darkMode === 'auto' ? '自动跟随设备系统主题。' : '' }}
                {{ themeForm.darkMode === 'light' ? '保持浅色主题。' : '' }}
                {{ themeForm.darkMode === 'dark' ? '保持深色主题。' : '' }}
                {{ themeForm.darkMode === 'schedule' ? '按设定时段自动切换。' : '' }}
              </p>
            </label>

            <div v-if="themeForm.darkMode === 'schedule'" class="sub-panel">
              <div class="uc-grid uc-grid-cols-2">
                <label class="uc-field">
                  <span class="uc-label">开启时间</span>
                  <input v-model="themeForm.scheduleStart" type="time" class="uc-input" @change="onScheduleTimeChange" />
                </label>
                <label class="uc-field">
                  <span class="uc-label">关闭时间</span>
                  <input v-model="themeForm.scheduleEnd" type="time" class="uc-input" @change="onScheduleTimeChange" />
                </label>
              </div>
              <p class="helper-text">支持跨夜设定，例如 20:00 ~ 07:00。</p>
            </div>
          </div>
        </section>

        <section ref="aiDebugSectionRef" class="uc-card" :class="{ 'ai-focus-card': focusedSection === 'debug' }">
          <h3 class="uc-card-title">AI 调试面板</h3>
          <p class="uc-card-subtitle">快速验证 `/api/openai/models` 与 `/api/openai/exec` 调用链路</p>

          <div class="settings-stack">
            <div class="uc-grid uc-grid-cols-3">
              <label class="uc-field">
                <span class="uc-label">可用模型</span>
                <select v-model="debugForm.model" class="uc-input" :disabled="debugLoadingModels || debugExecuting">
                  <option value="">自动选择（当前配置）</option>
                  <option v-for="m in debugModels" :key="m" :value="m">{{ m }}</option>
                </select>
              </label>

              <label class="uc-field">
                <span class="uc-label">Sandbox ID</span>
                <input v-model="debugForm.sandbox_id" class="uc-input" placeholder="sbx_settings_debug" />
              </label>

              <label class="uc-field">
                <span class="uc-label">User ID（可选）</span>
                <input v-model="debugForm.user_id" class="uc-input" placeholder="900000001" />
              </label>
            </div>

            <label class="uc-field">
              <span class="uc-label">测试输入</span>
              <textarea
                v-model="debugForm.input"
                class="uc-input debug-textarea"
                placeholder="输入你要测试的 prompt，例如：请返回一行 JSON，包含 ok=true"
              ></textarea>
            </label>

            <div class="uc-actions">
              <button class="uc-btn uc-btn-ghost" @click="loadDebugModels" :disabled="debugLoadingModels || debugExecuting">
                {{ debugLoadingModels ? '刷新中...' : '刷新模型列表' }}
              </button>
              <button class="uc-btn uc-btn-primary" @click="runDebugExec" :disabled="debugExecuting">
                {{ debugExecuting ? '执行中...' : '执行 Exec 测试' }}
              </button>
            </div>

            <div v-if="debugError" class="debug-error">{{ debugError }}</div>

            <div class="debug-result-grid">
              <label class="uc-field">
                <span class="uc-label">返回文本</span>
                <pre class="debug-block">{{ debugResult || '（尚无结果）' }}</pre>
              </label>
              <label class="uc-field">
                <span class="uc-label">响应元数据</span>
                <pre class="debug-block">{{ debugMetaText }}</pre>
              </label>
            </div>
          </div>
        </section>

        <section class="actions-foot">
          <button class="uc-btn uc-btn-primary" @click="saveSettings" :disabled="saving">
            {{ saving ? '保存中...' : '保存配置' }}
          </button>
        </section>
      </div>
    </main>

    <transition name="fade">
      <div v-if="showSkillsDrawer" class="drawer-overlay" @click.self="closeSkillDrawer">
        <aside class="skills-drawer">
          <div class="drawer-head">
            <div>
              <h3 class="uc-card-title">Skills 列表</h3>
              <p class="helper-text">抽屉展示，避免挤占设置页空间</p>
            </div>
            <button class="uc-btn uc-btn-ghost" @click="closeSkillDrawer">关闭</button>
          </div>

          <div class="drawer-tabs">
            <button class="drawer-tab" :class="{ active: skillDrawerTab === 'custom' }" @click="skillDrawerTab='custom'">
              自定义 ({{ aiSkills.length }})
            </button>
            <button class="drawer-tab" :class="{ active: skillDrawerTab === 'builtin' }" @click="skillDrawerTab='builtin'">
              本地扫描 ({{ builtinSkills.length }})
            </button>
          </div>

          <div class="skills-list">
            <div v-if="skillsLoading" class="helper-text">加载中...</div>
            <div v-else-if="activeSkillRows.length === 0" class="helper-text">
              {{ skillDrawerTab === 'custom' ? '暂无自定义 Skill' : '暂无本地扫描 Skill' }}
            </div>
            <div v-else class="skills-table">
              <div class="skills-row skills-head">
                <span>名称</span>
                <span>来源</span>
                <span>状态</span>
                <span>操作</span>
              </div>
              <div v-for="s in activeSkillRows" :key="s.id || s.name" class="skills-row">
                <div class="skills-name">
                  <div class="font-bold text-sm">{{ s.name }}</div>
                  <div class="text-[11px] text-slate-400 truncate">{{ s.description || '无描述' }}</div>
                </div>
                <span class="text-xs">{{ s.source || 'custom' }}</span>
                <span class="text-xs" :class="skillEnabled(s) ? 'text-emerald-600' : 'text-slate-400'">
                  {{ skillEnabled(s) ? '启用中' : '已停用' }}
                </span>
                <div class="skills-actions">
                  <button class="uc-btn uc-btn-ghost" @click="fillSkillForm(s); closeSkillDrawer()">编辑</button>
                  <button v-if="skillDrawerTab === 'custom'" class="uc-btn" :class="skillEnabled(s) ? 'uc-btn-danger' : 'uc-btn-primary'" @click="toggleSkill(s)">
                    {{ skillEnabled(s) ? '停用' : '启用' }}
                  </button>
                  <button v-else class="uc-btn uc-btn-primary" @click="fillSkillForm(s); closeSkillDrawer()">用作模板</button>
                </div>
              </div>
            </div>
          </div>
        </aside>
      </div>
    </transition>
  </div>
</template>

<script setup lang="ts">
import { ref, onMounted, onBeforeUnmount, reactive, computed } from 'vue'
import { configApi, sharedApi } from '@/api'
import { useThemeStore } from '@/stores/theme'
import { useAuthStore } from '@/stores/auth'

const auth = useAuthStore()
const isMerchant = computed(() => auth.sessionContext.role === 'merchant_owner')
const isAiFocusMode = ref(true)
const focusedSection = ref<string | null>(null)
const headerSubtitle = computed(() => isAiFocusMode.value ? '配置和调试 AI 模型' : '系统全局配置')

const saving = ref(false)
const canvasRef = ref<HTMLElement | null>(null)

const themeStore = useThemeStore()
const themeForm = reactive({
  darkMode: themeStore.settings.darkMode,
  scheduleStart: themeStore.settings.scheduleStart,
  scheduleEnd: themeStore.settings.scheduleEnd,
})
const bgInputRef = ref<HTMLInputElement | null>(null)
const chatBgInputRef = ref<HTMLInputElement | null>(null)

function onDarkModeChange() {
  themeStore.setDarkMode(themeForm.darkMode)
}

function onScheduleTimeChange() {
  themeStore.setSchedule(themeForm.scheduleStart, themeForm.scheduleEnd)
}

function readImageAsDataUrl(e: Event, onDone: (base64: string) => void) {
  const input = e.target as HTMLInputElement
  const file = input.files?.[0]
  if (!file) return

  if (file.size > 5 * 1024 * 1024) {
    alert('图片大小不能超过 5MB')
    input.value = ''
    return
  }

  const reader = new FileReader()
  reader.onload = () => {
    const base64 = reader.result as string
    onDone(base64)
    input.value = ''
  }
  reader.readAsDataURL(file)
}

function handleBgUpload(e: Event) {
  readImageAsDataUrl(e, (base64) => {
    themeStore.setCustomBackground(base64)
  })
}

function resetBackground() {
  themeStore.setCustomBackground(null)
  if (bgInputRef.value) bgInputRef.value.value = ''
}

function handleChatBgUpload(e: Event) {
  readImageAsDataUrl(e, (base64) => {
    themeStore.setChatBackground(base64)
  })
}

function resetChatBackground() {
  themeStore.setChatBackground(null)
  if (chatBgInputRef.value) chatBgInputRef.value.value = ''
}

const config = reactive({
  llm_api_mode: 'rented',
  llm_model_type: 'openai',
  llm_base_url: '',
  llm_api_key: '',
  llm_model: '',
  llm_model_lock_enabled: false,
  llm_model_lock_target: 'gemini-3.1-pro-preview',
  global_auto_translate: false,
})

type AiSkill = {
  id?: number
  name: string
  description?: string
  prompt_template?: string
  source?: string
  is_enabled?: boolean | number
  updated_at?: string
}

const aiSkills = ref<AiSkill[]>([])
const builtinSkills = ref<AiSkill[]>([])
const skillsLoading = ref(false)
const skillsSaving = ref(false)
const skillsImporting = ref(false)
const showSkillsDrawer = ref(false)
const skillDrawerTab = ref<'custom' | 'builtin'>('custom')
const skillForm = reactive<AiSkill>({
  name: '',
  description: '',
  prompt_template: '',
  source: 'custom',
  is_enabled: true,
})

const activeSkillRows = computed(() => (skillDrawerTab.value === 'custom' ? aiSkills.value : builtinSkills.value))

const MODEL_LOCK_TARGET = 'gemini-3.1-pro-preview'
const vertexQuickForm = reactive({
  vertex_api_key: '',
  oneapi_base_url: '',
  relay_api_key: '',
  model: 'gemini-2.5-flash-lite',
})

const quickGeminiModels = ref<string[]>([
  'gemini-3.1-pro-preview',
  'gemini-2.5-pro',
  'gemini-2.5-flash',
  'gemini-2.5-flash-lite',
  'gemini-2.0-flash',
  'gemini-2.0-flash-lite',
  'gemini-1.5-pro',
  'gemini-1.5-flash',
])
const quickSetting = ref(false)
const quickResult = ref<null | {
  oneapi_base_url: string
  model_infer_url: string
  model_list_url: string
  relay_api_key: string
  relay_api_key_masked: string
  model: string
}>(null)
const modelLockSaving = ref(false)

const debugLoadingModels = ref(false)
const debugExecuting = ref(false)
const debugError = ref('')
const debugModels = ref<string[]>([])
const debugResult = ref('')
const debugMeta = ref<Record<string, any> | null>(null)

const debugForm = reactive({
  model: '',
  input: '请返回一句简短的中文，说明当前模型调用链路正常。',
  sandbox_id: 'sbx_settings_debug',
  user_id: '',
})

const debugMetaText = computed(() => {
  if (!debugMeta.value) return '（尚无数据）'
  return JSON.stringify(debugMeta.value, null, 2)
})

onMounted(async () => {
  const res = await configApi.getConfig()
  if (res.status === 'ok') {
    Object.assign(config, res.data)
    config.global_auto_translate = res.data.global_auto_translate === 'true' || res.data.global_auto_translate === true
    config.llm_model_lock_enabled = res.data.llm_model_lock_enabled === 'true' || res.data.llm_model_lock_enabled === true
    config.llm_model_lock_target = String(res.data.llm_model_lock_target || MODEL_LOCK_TARGET).trim() || MODEL_LOCK_TARGET
    if (typeof res.data.llm_model === 'string' && res.data.llm_model.trim()) {
      debugForm.model = res.data.llm_model.trim()
      vertexQuickForm.model = res.data.llm_model.trim()
    }
    if (Array.isArray(res.data.gemini_model_options) && res.data.gemini_model_options.length) {
      quickGeminiModels.value = res.data.gemini_model_options.map((x: any) => String(x)).filter(Boolean)
    }
    if (config.llm_model_lock_enabled) {
      applyModelLockToForms()
    }
  }

  await loadDebugModels()
  await loadAiSkills()
  document.addEventListener('focusin', handleFocusIn)
})

onBeforeUnmount(() => {
  document.removeEventListener('focusin', handleFocusIn)
})

function handleFocusIn(ev: Event) {
  const target = ev.target as HTMLElement | null
  const canvas = canvasRef.value
  if (!target || !canvas) return
  if (target instanceof HTMLInputElement || target instanceof HTMLTextAreaElement || target instanceof HTMLSelectElement) {
    setTimeout(() => {
      const canvasRect = canvas.getBoundingClientRect()
      const targetRect = target.getBoundingClientRect()
      const delta = targetRect.top - canvasRect.top - canvasRect.height * 0.35
      canvas.scrollBy({ top: delta, behavior: 'smooth' })
    }, 220)
  }
}

async function loadDebugModels() {
  debugLoadingModels.value = true
  debugError.value = ''
  try {
    const res = await sharedApi.getOpenaiModels()
    const items = Array.isArray(res?.data) ? res.data : []
    debugModels.value = items.map((m: any) => String(m?.id || '')).filter(Boolean)
    if (!debugForm.model && debugModels.value.length) {
      debugForm.model = debugModels.value[0]
    }
  } catch (e) {
    debugError.value = '模型列表拉取失败，请检查后端 /api/openai/models'
  } finally {
    debugLoadingModels.value = false
  }
}

async function runDebugExec() {
  debugExecuting.value = true
  debugError.value = ''
  debugResult.value = ''
  debugMeta.value = null

  try {
    const model = (debugForm.model || config.llm_model || 'gemini-2.5-flash-lite').trim()
    const input = debugForm.input.trim()
    if (!input) {
      debugError.value = '请输入测试输入'
      return
    }

    const payload: { model: string; input: string; sandbox_id?: string; user_id?: number } = {
      model,
      input,
    }

    const sandboxId = debugForm.sandbox_id.trim()
    if (sandboxId) payload.sandbox_id = sandboxId

    const parsedUserId = Number(debugForm.user_id)
    if (debugForm.user_id.trim() && Number.isFinite(parsedUserId) && parsedUserId > 0) {
      payload.user_id = parsedUserId
    }

    const res = await sharedApi.execOpenai(payload)
    if (res?.error?.message) {
      debugError.value = String(res.error.message)
      debugMeta.value = res
      return
    }

    debugResult.value = String(res?.output_text || '')
    debugMeta.value = {
      model: res?.model,
      sandbox_id: res?.sandbox_id,
      usage: res?.usage || null,
      status: res?.status || 'ok',
    }
  } catch (e) {
    debugError.value = 'Exec 调用失败，请检查网络或后端日志'
  } finally {
    debugExecuting.value = false
  }
}

function skillEnabled(s: AiSkill): boolean {
  return s.is_enabled === true || Number(s.is_enabled) === 1
}

function fillSkillForm(s: AiSkill) {
  skillForm.name = String(s.name || '')
  skillForm.description = String(s.description || '')
  skillForm.prompt_template = String(s.prompt_template || '')
  skillForm.source = String(s.source || 'custom')
  skillForm.is_enabled = skillEnabled(s)
}

async function loadAiSkills() {
  skillsLoading.value = true
  try {
    const res = await sharedApi.getAiSkills(false)
    if (res?.status === 'ok') {
      const rows = Array.isArray(res.data) ? res.data : []
      aiSkills.value = rows.filter((x: AiSkill) => String(x.source || '').toLowerCase() !== 'builtin_fs')
    } else {
      alert('加载 Skill 列表失败: ' + String(res?.message || '未知错误'))
    }
  } catch (e) {
    alert('加载 Skill 列表失败，请检查网络或后端日志')
  } finally {
    skillsLoading.value = false
  }
}

async function loadDiscoveredSkills() {
  try {
    const res = await sharedApi.getDiscoveredAiSkills()
    if (res?.status === 'ok') {
      const rows = Array.isArray(res.data) ? res.data : []
      builtinSkills.value = rows.map((x: any) => ({
        name: String(x.name || ''),
        description: String(x.description || ''),
        source: String(x.source || 'builtin_scan'),
        is_enabled: false,
      }))
    } else {
      builtinSkills.value = []
    }
  } catch (e) {
    builtinSkills.value = []
  }
}

async function saveAiSkill() {
  const name = String(skillForm.name || '').trim()
  if (!name) {
    alert('请先输入 Skill 名称')
    return
  }
  skillsSaving.value = true
  try {
    const res = await sharedApi.upsertAiSkill({
      name,
      description: String(skillForm.description || '').trim(),
      prompt_template: String(skillForm.prompt_template || ''),
      source: String(skillForm.source || 'custom'),
      is_enabled: skillForm.is_enabled !== false,
    })
    if (res?.status === 'ok') {
      await loadAiSkills()
      alert('Skill 已保存')
    } else {
      alert('保存 Skill 失败: ' + String(res?.message || '未知错误'))
    }
  } catch (e) {
    alert('保存 Skill 失败，请检查网络或后端日志')
  } finally {
    skillsSaving.value = false
  }
}

async function toggleSkill(s: AiSkill) {
  const enabled = !skillEnabled(s)
  try {
    const res = await sharedApi.toggleAiSkill(String(s.name || ''), enabled)
    if (res?.status === 'ok') {
      await loadAiSkills()
    } else {
      alert('更新 Skill 状态失败: ' + String(res?.message || '未知错误'))
    }
  } catch (e) {
    alert('更新 Skill 状态失败，请检查网络或后端日志')
  }
}

async function importBuiltinSkills() {
  skillsImporting.value = true
  try {
    const res = await sharedApi.importBuiltinAiSkills()
    if (res?.status === 'ok') {
      await Promise.all([loadAiSkills(), loadDiscoveredSkills()])
      alert(res?.message || '内置 Skills 导入完成')
    } else {
      alert('导入失败: ' + String(res?.message || '未知错误'))
    }
  } catch (e) {
    alert('导入失败，请检查网络或后端日志')
  } finally {
    skillsImporting.value = false
  }
}

async function openSkillDrawer() {
  showSkillsDrawer.value = true
  skillDrawerTab.value = 'custom'
  await Promise.all([loadAiSkills(), loadDiscoveredSkills()])
}

function closeSkillDrawer() {
  showSkillsDrawer.value = false
}

async function saveSettings() {
  saving.value = true
  try {
    const lockTarget = String(config.llm_model_lock_target || MODEL_LOCK_TARGET).trim() || MODEL_LOCK_TARGET
    const payload: Record<string, any> = {
      ...config,
      llm_model_lock_enabled: config.llm_model_lock_enabled ? 'true' : 'false',
      llm_model_lock_target: lockTarget,
      global_auto_translate: config.global_auto_translate ? 'true' : 'false',
    }
    if (config.llm_model_lock_enabled) {
      payload.llm_model = lockTarget
    }
    const res = await configApi.saveConfig({
      ...payload,
    })
    if (res.status === 'ok') {
      alert('配置已成功保存')
    } else {
      alert('保存失败: ' + res.message)
    }
  } catch (e) {
    alert('请求失败，请检查网络')
  } finally {
    saving.value = false
  }
}

async function oneClickVertexSetup() {
  quickSetting.value = true
  quickResult.value = null

  try {
    const payload = {
      vertex_api_key: vertexQuickForm.vertex_api_key.trim(),
      oneapi_base_url: vertexQuickForm.oneapi_base_url.trim(),
      relay_api_key: vertexQuickForm.relay_api_key.trim(),
      model: vertexQuickForm.model.trim(),
    }

    const res = await configApi.setupVertexOneClick(payload)
    if (res?.status !== 'ok') {
      alert('一键配置失败: ' + String(res?.message || '未知错误'))
      return
    }

    const result = res.data || {}
    quickResult.value = {
      oneapi_base_url: String(result.oneapi_base_url || ''),
      model_infer_url: String(result.model_infer_url || ''),
      model_list_url: String(result.model_list_url || ''),
      relay_api_key: String(result.relay_api_key || ''),
      relay_api_key_masked: String(result.relay_api_key_masked || ''),
      model: String(result.model || ''),
    }

    config.llm_api_mode = 'own'
    config.llm_model_type = 'openai'
    config.llm_base_url = quickResult.value.oneapi_base_url
    config.llm_model = quickResult.value.model
    debugForm.model = quickResult.value.model
    if (config.llm_model_lock_enabled) {
      applyModelLockToForms()
    }

    const successText = [
      '✅ Vertex 一键配置完成',
      `中转 URL: ${quickResult.value.oneapi_base_url}`,
      `模型: ${quickResult.value.model}`,
      `中转 SK: ${quickResult.value.relay_api_key}`,
      `调用地址: ${quickResult.value.model_infer_url}`,
    ].join('\n')
    alert(successText)
  } catch (e) {
    alert('一键配置请求失败，请检查后端日志')
  } finally {
    quickSetting.value = false
  }
}

function applyModelLockToForms() {
  const lockTarget = String(config.llm_model_lock_target || MODEL_LOCK_TARGET).trim() || MODEL_LOCK_TARGET
  config.llm_model_lock_target = lockTarget
  config.llm_model = lockTarget
  debugForm.model = lockTarget
  vertexQuickForm.model = lockTarget
}

async function toggleModelLock() {
  modelLockSaving.value = true
  try {
    const nextEnabled = !config.llm_model_lock_enabled
    const lockTarget = String(config.llm_model_lock_target || MODEL_LOCK_TARGET).trim() || MODEL_LOCK_TARGET
    const payload: Record<string, any> = {
      llm_model_lock_enabled: nextEnabled ? 'true' : 'false',
      llm_model_lock_target: lockTarget,
    }
    if (nextEnabled) {
      payload.llm_model = lockTarget
    }

    const res = await configApi.saveConfig(payload)
    if (res?.status !== 'ok') {
      alert('模型锁定更新失败: ' + String(res?.message || '未知错误'))
      return
    }

    config.llm_model_lock_enabled = nextEnabled
    if (nextEnabled) {
      applyModelLockToForms()
      alert(`已锁定模型为 ${lockTarget}`)
    } else {
      alert('已解除模型锁定')
    }
  } catch (e) {
    alert('模型锁定请求失败，请检查后端日志')
  } finally {
    modelLockSaving.value = false
  }
}
</script>

<style scoped>
.settings-wrap {
  max-width: 1120px;
}

.settings-stack {
  margin-top: 12px;
  display: flex;
  flex-direction: column;
  gap: 12px;
}

.mode-grid {
  display: grid;
  grid-template-columns: repeat(2, minmax(0, 1fr));
  gap: 10px;
}

.mode-item {
  border: 1px solid rgb(148 163 184 / 0.24);
  border-radius: 13px;
  background: rgb(248 250 252 / 0.7);
  padding: 12px;
  display: flex;
  gap: 8px;
  align-items: flex-start;
  cursor: pointer;
  transition: all 0.18s ease;
}

.mode-item:hover {
  border-color: rgb(36 161 222 / 0.45);
}

.mode-item.active {
  border-color: rgb(36 161 222 / 0.62);
  box-shadow: 0 0 0 3px rgb(36 161 222 / 0.12);
}

.dark .mode-item {
  border-color: rgb(73 106 148 / 0.28);
  background: rgb(8 17 31 / 0.68);
}

.mode-title {
  font-size: 13px;
  font-weight: 800;
  color: #0f172a;
}

.mode-desc {
  margin-top: 2px;
  font-size: 12px;
  color: #64748b;
}

.dark .mode-title {
  color: #dbeafe;
}

.dark .mode-desc {
  color: #adc3e0;
}

.sub-panel {
  border: 1px solid rgb(148 163 184 / 0.22);
  border-radius: 14px;
  background: rgb(248 250 252 / 0.72);
  padding: 12px;
}

.dark .sub-panel {
  border-color: rgb(73 106 148 / 0.28);
  background: rgb(8 17 31 / 0.7);
}

.accent-panel {
  border-color: rgb(36 161 222 / 0.35);
}

.info-panel {
  font-size: 13px;
  color: #334155;
}

.dark .info-panel {
  color: #dbeafe;
}

.sub-title {
  font-size: 15px;
  font-weight: 800;
  color: #0f172a;
}

.dark .sub-title {
  color: #e2e8f0;
}

.lock-panel {
  border-color: rgb(14 165 233 / 0.3);
}

.model-lock-row {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 12px;
  margin-top: 10px;
}

.helper-text {
  font-size: 12px;
  color: #64748b;
}

.dark .helper-text {
  color: #adc3e0;
}

.toggle-row {
  margin-top: 10px;
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 12px;
}

.toggle-title {
  font-size: 14px;
  font-weight: 800;
  color: #0f172a;
}

.dark .toggle-title {
  color: #e2e8f0;
}

.switch {
  display: none;
}

.switch + label {
  display: block;
  width: 44px;
  height: 24px;
  border-radius: 99px;
  background: rgb(148 163 184 / 0.5);
  position: relative;
  cursor: pointer;
  transition: all 0.2s ease;
}

.switch + label::after {
  content: '';
  position: absolute;
  width: 18px;
  height: 18px;
  top: 3px;
  left: 3px;
  border-radius: 50%;
  background: #fff;
  transition: all 0.2s ease;
}

.switch:checked + label {
  background: #24a1de;
}

.switch:checked + label::after {
  left: 23px;
}

.upload-row {
  display: flex;
  align-items: center;
  flex-wrap: wrap;
  gap: 8px;
}

.bg-ready {
  font-size: 12px;
  font-weight: 700;
  color: rgb(6 95 70);
}

.dark .bg-ready {
  color: rgb(110 231 183);
}

.hidden-input {
  display: none;
}

.debug-textarea {
  min-height: 102px;
  resize: vertical;
}

.debug-error {
  border-radius: 12px;
  border: 1px solid rgb(239 68 68 / 0.35);
  background: rgb(254 226 226 / 0.86);
  color: #b91c1c;
  padding: 10px 12px;
  font-size: 12px;
}

.dark .debug-error {
  border-color: rgb(248 113 113 / 0.32);
  background: rgb(127 29 29 / 0.4);
  color: #fecaca;
}

.debug-result-grid,
.quick-result-grid {
  display: grid;
  grid-template-columns: repeat(2, minmax(0, 1fr));
  gap: 10px;
}

.skills-list {
  display: flex;
  flex-direction: column;
  gap: 8px;
}

.skills-table {
  border: 1px solid rgb(148 163 184 / 0.24);
  border-radius: 12px;
  overflow: hidden;
  background: rgb(248 250 252 / 0.72);
}

.dark .skills-table {
  border-color: rgb(73 106 148 / 0.32);
  background: rgb(8 17 31 / 0.66);
}

.skills-row {
  display: grid;
  grid-template-columns: minmax(0, 2fr) minmax(0, 1fr) minmax(0, 1fr) auto;
  gap: 10px;
  align-items: center;
  padding: 10px 12px;
  border-bottom: 1px solid rgb(148 163 184 / 0.14);
}

.skills-row:last-child {
  border-bottom: none;
}

.skills-head {
  background: rgb(241 245 249 / 0.9);
  font-size: 12px;
  font-weight: 800;
  color: #475569;
}

.dark .skills-head {
  background: rgb(15 23 42 / 0.8);
  color: #cbd5e1;
}

.skills-name {
  min-width: 0;
}

.skills-actions {
  display: flex;
  align-items: center;
  gap: 6px;
}

.drawer-overlay {
  position: fixed;
  inset: 0;
  z-index: 70;
  background: rgb(2 6 23 / 0.48);
  display: flex;
  justify-content: flex-end;
}

.skills-drawer {
  width: min(760px, 94vw);
  height: 100%;
  background: rgb(248 250 252);
  border-left: 1px solid rgb(148 163 184 / 0.24);
  box-shadow: -12px 0 30px rgb(2 6 23 / 0.18);
  padding: 16px;
  display: flex;
  flex-direction: column;
  gap: 12px;
}

.dark .skills-drawer {
  background: rgb(2 12 24);
  border-left-color: rgb(73 106 148 / 0.35);
}

.drawer-head {
  display: flex;
  align-items: flex-start;
  justify-content: space-between;
  gap: 8px;
}

.drawer-tabs {
  display: flex;
  align-items: center;
  gap: 8px;
}

.drawer-tab {
  border: 1px solid rgb(148 163 184 / 0.26);
  background: rgb(241 245 249 / 0.75);
  color: #475569;
  font-size: 12px;
  font-weight: 700;
  padding: 6px 10px;
  border-radius: 10px;
  transition: all 0.16s ease;
}

.drawer-tab.active {
  border-color: rgb(36 161 222 / 0.55);
  background: rgb(36 161 222 / 0.13);
  color: #0369a1;
}

.dark .drawer-tab {
  border-color: rgb(73 106 148 / 0.32);
  background: rgb(15 23 42 / 0.65);
  color: #cbd5e1;
}

.dark .drawer-tab.active {
  border-color: rgb(56 189 248 / 0.55);
  background: rgb(14 116 144 / 0.22);
  color: #7dd3fc;
}

.debug-block {
  margin: 0;
  border: 1px solid rgb(148 163 184 / 0.24);
  border-radius: 11px;
  background: rgb(248 250 252 / 0.8);
  color: #0f172a;
  padding: 10px;
  font-size: 12px;
  line-height: 1.45;
  white-space: pre-wrap;
  word-break: break-word;
  max-height: 260px;
  overflow: auto;
}

.dark .debug-block {
  border-color: rgb(73 106 148 / 0.3);
  background: rgb(6 14 26 / 0.8);
  color: #dbeafe;
}

.actions-foot {
  display: flex;
  justify-content: flex-end;
  padding-top: 2px;
}

@media (max-width: 768px) {
  .mode-grid,
  .debug-result-grid,
  .quick-result-grid {
    grid-template-columns: 1fr;
  }

  .skills-row {
    grid-template-columns: 1fr;
    gap: 6px;
  }

  .skills-drawer {
    width: 100vw;
    padding: 12px;
  }

  .toggle-row {
    flex-direction: column;
    align-items: flex-start;
  }

  .model-lock-row {
    flex-direction: column;
    align-items: flex-start;
  }

  .actions-foot .uc-btn {
    width: 100%;
  }
}
</style>
