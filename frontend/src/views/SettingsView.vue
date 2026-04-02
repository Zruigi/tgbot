<template>
  <div class="settings-root">
    <div class="topbar-row">
      <h2>系统设置</h2>
    </div>

    <div ref="canvasRef" class="settings-canvas">
      <div class="settings-container">
        <!-- LLM API Configuration -->
        <div class="card settings-card">
          <div class="card-header">
            <h3>AI 翻译引擎配置</h3>
            <p class="subtitle">配置自动翻译所使用的大模型 API</p>
          </div>

          <div class="settings-group">
            <div class="field">
              <label>API 模式</label>
              <div class="radio-group">
                <label class="radio-item" :class="{ active: config.llm_api_mode === 'rented' }">
                  <input type="radio" v-model="config.llm_api_mode" value="rented" />
                  <div class="radio-content">
                    <span class="title">🚀 租用 API (一键适配)</span>
                    <span class="desc">使用系统预置的优化线路，无需自行配置密钥</span>
                  </div>
                </label>
                <label class="radio-item" :class="{ active: config.llm_api_mode === 'own' }">
                  <input type="radio" v-model="config.llm_api_mode" value="own" />
                  <div class="radio-content">
                    <span class="title">🔑 自有 API</span>
                    <span class="desc">填入您私人的 API Key，确保数据完全私有</span>
                  </div>
                </label>
              </div>
            </div>

            <div v-if="config.llm_api_mode === 'own'" class="own-api-fields">
              <div class="field">
                <label>API 厂商 / 模型系列</label>
                <select v-model="config.llm_model_type" class="input">
                  <option value="openai">OpenAI / 兼容接口</option>
                  <option value="deepseek">DeepSeek</option>
                  <option value="gemini">Google Gemini</option>
                </select>
              </div>

              <div class="field">
                <label>API Base URL</label>
                <input v-model="config.llm_base_url" class="input" placeholder="https://api.openai.com/v1" />
              </div>

              <div class="field">
                <label>API Key</label>
                <input v-model="config.llm_api_key" type="password" class="input" placeholder="sk-xxxxxxxxxxxxxxxx" />
                <p class="hint">填入后将加密存储。留空表示不修改现有密钥。</p>
              </div>

              <div class="field">
                <label>模型名称 (Model Name)</label>
                <input v-model="config.llm_model" class="input" placeholder="gpt-3.5-turbo" />
              </div>
            </div>

            <div class="vertex-oneclick-box">
              <h4>Vertex 一键自动配置（推荐）</h4>
              <p class="desc">输入 Vertex Key 与 OneAPI 地址，系统自动生成可直接使用的中转 URL + SK + 模型配置。</p>

              <div class="debug-grid">
                <div class="field">
                  <label>Vertex API Key</label>
                  <input v-model="vertexQuickForm.vertex_api_key" type="password" class="input" placeholder="AIza..." />
                </div>

                <div class="field">
                  <label>OneAPI Base URL</label>
                  <input v-model="vertexQuickForm.oneapi_base_url" class="input" placeholder="https://oneapi.example.com" />
                </div>

                <div class="field">
                  <label>Gemini 模型</label>
                  <select v-model="vertexQuickForm.model" class="input">
                    <option v-for="m in quickGeminiModels" :key="m" :value="m">{{ m }}</option>
                  </select>
                </div>
              </div>

              <div class="field">
                <label>中转 SK（可选，不填则自动生成）</label>
                <input v-model="vertexQuickForm.relay_api_key" class="input" placeholder="sk-your-own-relay-key" />
              </div>

              <div class="debug-actions">
                <button class="btn btn-primary" @click="oneClickVertexSetup" :disabled="quickSetting">
                  {{ quickSetting ? '配置中...' : '一键自动配置' }}
                </button>
              </div>

              <div v-if="quickResult" class="quick-result-box">
                <div class="field">
                  <label>中转 URL</label>
                  <pre class="debug-meta">{{ quickResult.oneapi_base_url }}</pre>
                </div>
                <div class="field">
                  <label>模型 ID</label>
                  <pre class="debug-meta">{{ quickResult.model }}</pre>
                </div>
                <div class="field">
                  <label>中转 API (chat/completions)</label>
                  <pre class="debug-meta">{{ quickResult.model_infer_url }}</pre>
                </div>
                <div class="field">
                  <label>中转 SK</label>
                  <pre class="debug-meta">{{ quickResult.relay_api_key }}</pre>
                </div>
              </div>
            </div>

            <div v-if="config.llm_api_mode === 'rented'" class="rented-api-info">
              <div class="info-box">
                <span class="info-icon">✨</span>
                <p>当前处于租用模式。系统将自动适配最佳节点，为您提供稳定的翻译服务。您无需承担配置和维护成本。</p>
              </div>
            </div>
          </div>
        </div>

        <!-- Global Translation Toggle -->
        <div class="card settings-card">
          <div class="settings-group">
            <div class="field flex-row">
              <div class="field-info">
                <label>全局自动翻译</label>
                <p class="desc">开启后，客服私聊中的外语消息将自动翻译为中文</p>
              </div>
              <div class="switch-container">
                <input type="checkbox" v-model="config.global_auto_translate" id="auto-trans" class="switch" />
                <label for="auto-trans"></label>
              </div>
            </div>
          </div>
        </div>

        <!-- AI Debug Panel -->
        <div class="card settings-card debug-card">
          <div class="card-header">
            <h3>AI 调试面板</h3>
            <p class="subtitle">用于快速验证 `/api/openai/models` 与 `/api/openai/exec` 接口</p>
          </div>

          <div class="settings-group">
            <div class="debug-grid">
              <div class="field">
                <label>可用模型</label>
                <select v-model="debugForm.model" class="input" :disabled="debugLoadingModels || debugExecuting">
                  <option value="">自动选择（使用当前配置模型）</option>
                  <option v-for="m in debugModels" :key="m" :value="m">{{ m }}</option>
                </select>
              </div>

              <div class="field">
                <label>Sandbox ID</label>
                <input v-model="debugForm.sandbox_id" class="input" placeholder="sbx_manual_debug" />
              </div>

              <div class="field">
                <label>User ID（可选）</label>
                <input v-model="debugForm.user_id" class="input" placeholder="900000001" />
              </div>
            </div>

            <div class="field">
              <label>测试输入</label>
              <textarea
                v-model="debugForm.input"
                class="input debug-textarea"
                placeholder="输入你要测试的 prompt，例如：请返回一行 JSON，包含 ok=true"
              ></textarea>
            </div>

            <div class="debug-actions">
              <button class="btn btn-ghost" @click="loadDebugModels" :disabled="debugLoadingModels || debugExecuting">
                {{ debugLoadingModels ? '刷新中...' : '刷新模型列表' }}
              </button>
              <button class="btn btn-primary" @click="runDebugExec" :disabled="debugExecuting">
                {{ debugExecuting ? '执行中...' : '执行 Exec 测试' }}
              </button>
            </div>

            <div v-if="debugError" class="debug-error">{{ debugError }}</div>

            <div class="debug-result-wrap">
              <div class="field">
                <label>返回文本</label>
                <pre class="debug-result">{{ debugResult || '（尚无结果）' }}</pre>
              </div>
              <div class="field">
                <label>响应元数据</label>
                <pre class="debug-meta">{{ debugMetaText }}</pre>
              </div>
            </div>
          </div>
        </div>

        <div class="actions-row">
          <button class="btn btn-primary" @click="saveSettings" :disabled="saving">
            {{ saving ? '保存中...' : '保存配置' }}
          </button>
        </div>

        <div class="canvas-bottom-limit" aria-hidden="true"></div>
      </div>
    </div>
  </div>
</template>

<script setup lang="ts">
import { ref, onMounted, onBeforeUnmount, reactive, computed } from 'vue'
import { api } from '@/api'

const saving = ref(false)
const canvasRef = ref<HTMLElement | null>(null)

const config = reactive({
  llm_api_mode: 'rented',
  llm_model_type: 'openai',
  llm_base_url: '',
  llm_api_key: '',
  llm_model: '',
  global_auto_translate: false
})

const vertexQuickForm = reactive({
  vertex_api_key: '',
  oneapi_base_url: '',
  relay_api_key: '',
  model: 'gemini-2.5-flash-lite'
})

const quickGeminiModels = ref<string[]>([
  'gemini-2.5-pro',
  'gemini-2.5-flash',
  'gemini-2.5-flash-lite',
  'gemini-2.0-flash',
  'gemini-2.0-flash-lite',
  'gemini-1.5-pro',
  'gemini-1.5-flash'
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
  user_id: ''
})

const debugMetaText = computed(() => {
  if (!debugMeta.value) return '（尚无数据）'
  return JSON.stringify(debugMeta.value, null, 2)
})

onMounted(async () => {
  const res = await api.getConfig()
  if (res.status === 'ok') {
    Object.assign(config, res.data)
    config.global_auto_translate = res.data.global_auto_translate === 'true'
    if (typeof res.data.llm_model === 'string' && res.data.llm_model.trim()) {
      debugForm.model = res.data.llm_model.trim()
      vertexQuickForm.model = res.data.llm_model.trim()
    }
    if (Array.isArray(res.data.gemini_model_options) && res.data.gemini_model_options.length) {
      quickGeminiModels.value = res.data.gemini_model_options.map((x: any) => String(x)).filter(Boolean)
    }
  }

  await loadDebugModels()
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
    const res = await api.getOpenaiModels()
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

    const res = await api.execOpenai(payload)
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
      status: res?.status || 'ok'
    }
  } catch (e) {
    debugError.value = 'Exec 调用失败，请检查网络或后端日志'
  } finally {
    debugExecuting.value = false
  }
}

async function saveSettings() {
  saving.value = true
  try {
    const res = await api.saveConfig({
      ...config,
      global_auto_translate: config.global_auto_translate ? 'true' : 'false'
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

    const res = await api.setupVertexOneClick(payload)
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
</script>

<style scoped>
.settings-root {
  height: 100%;
  min-height: 0;
  display: flex;
  flex-direction: column;
  overflow: hidden;
}

.settings-canvas {
  flex: 1;
  min-height: 0;
  overflow-y: auto;
  overflow-x: hidden;
  -webkit-overflow-scrolling: touch;
  touch-action: pan-y;
  overscroll-behavior-y: contain;
  padding: 20px;
  padding-bottom: calc(88px + env(safe-area-inset-bottom) + var(--kb-offset, 0px));
}

.settings-container {
  max-width: 800px;
  margin: 0 auto;
  display: flex;
  flex-direction: column;
  gap: 20px;
}

.canvas-bottom-limit {
  height: 8px;
}

.settings-card {
  padding: 24px;
}

.card-header {
  margin-bottom: 24px;
  border-bottom: 1px solid var(--border-muted);
  padding-bottom: 16px;
}

.card-header h3 {
  font-size: 18px;
  font-weight: 600;
  margin: 0 0 4px 0;
}

.subtitle {
  font-size: 13px;
  color: var(--text-secondary);
}

.settings-group {
  display: flex;
  flex-direction: column;
  gap: 20px;
}

.field {
  display: flex;
  flex-direction: column;
  gap: 8px;
}

.flex-row {
  flex-direction: row;
  align-items: center;
  justify-content: space-between;
}

.field label {
  font-size: 13px;
  font-weight: 600;
  color: var(--text-primary);
}

.field .desc {
  font-size: 12px;
  color: var(--text-secondary);
  margin: 2px 0 0 0;
}

.radio-group {
  display: grid;
  grid-template-columns: 1fr 1fr;
  gap: 12px;
  margin-top: 4px;
}

.radio-item {
  position: relative;
  padding: 16px;
  border: 1px solid var(--border);
  border-radius: var(--radius-md);
  cursor: pointer;
  transition: all 0.2s;
  display: flex;
  gap: 12px;
  align-items: flex-start;
}

.radio-item:hover {
  border-color: var(--accent);
  background: var(--bg-hover);
}

.radio-item.active {
  border-color: var(--accent);
  background: rgba(88, 166, 255, 0.05);
  box-shadow: 0 0 0 1px var(--accent);
}

.radio-item input {
  margin-top: 3px;
}

.radio-content {
  display: flex;
  flex-direction: column;
  gap: 4px;
}

.radio-content .title {
  font-size: 14px;
  font-weight: 600;
}

.radio-content .desc {
  font-size: 12px;
  color: var(--text-secondary);
  line-height: 1.4;
}

.own-api-fields {
  display: flex;
  flex-direction: column;
  gap: 16px;
  padding: 20px;
  background: var(--bg-elevated);
  border-radius: var(--radius-md);
  margin-top: 8px;
  border: 1px solid var(--border-muted);
}

.rented-api-info {
  padding: 20px;
  background: var(--bg-elevated);
  border-radius: var(--radius-md);
  margin-top: 8px;
  border: 1px dashed var(--accent);
}

.info-box {
  display: flex;
  gap: 12px;
  align-items: center;
}

.info-icon {
  font-size: 24px;
}

.info-box p {
  font-size: 13px;
  line-height: 1.5;
  color: var(--text-secondary);
  margin: 0;
}

.hint {
  font-size: 11px;
  color: var(--text-muted);
  margin: 4px 0 0 0;
}

.actions-row {
  display: flex;
  justify-content: flex-end;
  padding: 20px 0;
}

.debug-card {
  border: 1px solid color-mix(in srgb, var(--accent) 36%, var(--border));
  background: linear-gradient(180deg, color-mix(in srgb, var(--bg-elevated) 84%, var(--accent) 16%), var(--bg-card));
}

.debug-grid {
  display: grid;
  grid-template-columns: repeat(3, minmax(0, 1fr));
  gap: 12px;
}

.debug-textarea {
  min-height: 96px;
  resize: vertical;
}

.debug-actions {
  display: flex;
  gap: 10px;
  flex-wrap: wrap;
}

.debug-error {
  border: 1px solid color-mix(in srgb, #ff5f56 45%, var(--border));
  background: color-mix(in srgb, #ff5f56 12%, var(--bg-elevated));
  color: #ffb4af;
  padding: 10px 12px;
  border-radius: 10px;
  font-size: 12px;
  line-height: 1.4;
}

.debug-result-wrap {
  display: grid;
  grid-template-columns: 1.2fr 1fr;
  gap: 12px;
}

.debug-result,
.debug-meta {
  margin: 0;
  padding: 14px;
  border-radius: 10px;
  border: 1px solid var(--border-muted);
  background: color-mix(in srgb, var(--bg-elevated) 90%, #000 10%);
  color: var(--text-primary);
  max-height: 280px;
  overflow: auto;
  white-space: pre-wrap;
  word-break: break-word;
  font-size: 12px;
  line-height: 1.45;
}

.debug-meta {
  white-space: pre;
}

.quick-result-box {
  display: grid;
  grid-template-columns: 1fr 1fr;
  gap: 10px;
}

.vertex-oneclick-box {
  margin-top: 14px;
  border: 1px solid color-mix(in srgb, var(--accent) 35%, var(--border));
  border-radius: var(--radius-md);
  padding: 16px;
  background: color-mix(in srgb, var(--bg-elevated) 86%, var(--accent) 14%);
  display: flex;
  flex-direction: column;
  gap: 12px;
}

.vertex-oneclick-box h4 {
  margin: 0;
  font-size: 15px;
}

@media (max-width: 768px) {
  .settings-canvas {
    padding: 12px;
    padding-bottom: calc(124px + env(safe-area-inset-bottom) + var(--kb-offset, 0px));
  }

  .canvas-bottom-limit {
    height: calc(40px + env(safe-area-inset-bottom));
  }

  .settings-card {
    padding: 16px;
  }

  .radio-group {
    grid-template-columns: 1fr;
  }

  .flex-row {
    flex-direction: column;
    align-items: flex-start;
    gap: 12px;
  }

  .actions-row {
    justify-content: stretch;
  }

  .actions-row .btn {
    width: 100%;
    justify-content: center;
  }

  .debug-grid,
  .debug-result-wrap {
    grid-template-columns: 1fr;
  }
}

.switch {
  display: none;
}

.switch + label {
  display: block;
  width: 44px;
  height: 22px;
  background: var(--border);
  border-radius: 22px;
  position: relative;
  cursor: pointer;
  transition: all 0.2s;
}

.switch + label::after {
  content: '';
  position: absolute;
  left: 2px;
  top: 2px;
  width: 18px;
  height: 18px;
  background: white;
  border-radius: 50%;
  transition: all 0.2s;
}

.switch:checked + label {
  background: var(--success);
}

.switch:checked + label::after {
  left: 24px;
}
</style>
