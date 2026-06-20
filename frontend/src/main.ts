import { createApp } from 'vue'
import { createPinia } from 'pinia'
import App from './App.vue'
import { initTelegramMiniApp } from './lib/telegram'
import { useThemeStore } from './stores/theme'
import './style.css'

/**
 * 等待 Telegram WebApp SDK 加载完成
 * telegram-web-app.js 是异步加载的，需要等待它就绪
 */
function waitForTelegramSDK(maxWait = 3000): Promise<void> {
  return new Promise((resolve) => {
    if (typeof window !== 'undefined' && window.Telegram?.WebApp) {
      resolve()
      return
    }
    const start = Date.now()
    const check = () => {
      if (window.Telegram?.WebApp) {
        resolve()
      } else if (Date.now() - start < maxWait) {
        requestAnimationFrame(check)
      } else {
        // 超时，不在TMA环境中，继续正常渲染
        console.log('[TMA] SDK not loaded after timeout, proceeding without TMA')
        resolve()
      }
    }
    requestAnimationFrame(check)
  })
}

async function bootstrap() {
  // 等待 Telegram SDK 就绪（最多3秒）
  await waitForTelegramSDK()

  const app = createApp(App)
  app.use(createPinia())

  // 初始化 Telegram Mini App SDK（必须在挂载前完成）
  initTelegramMiniApp()

  // 初始化主题 store（必须在挂载前完成，避免白屏闪烁）
  const store = useThemeStore()

  const mode = store.settings.darkMode

  function isInSchedule(start: string, end: string): boolean {
    const [sh, sm] = start.split(':').map(Number)
    const [eh, em] = end.split(':').map(Number)
    if ([sh, sm, eh, em].some((n) => Number.isNaN(n))) return false

    const now = new Date()
    const currentMinutes = now.getHours() * 60 + now.getMinutes()
    const startMinutes = sh * 60 + sm
    const endMinutes = eh * 60 + em

    if (endMinutes < startMinutes) {
      return currentMinutes >= startMinutes || currentMinutes < endMinutes
    }
    return currentMinutes >= startMinutes && currentMinutes < endMinutes
  }

  let shouldDark = false
  if (mode === 'dark') {
    shouldDark = true
  } else if (mode === 'auto') {
    shouldDark = window.matchMedia('(prefers-color-scheme: dark)').matches
  } else if (mode === 'schedule') {
    shouldDark = isInSchedule(store.settings.scheduleStart, store.settings.scheduleEnd)
  }

  store.setDarkState(shouldDark)

  app.mount('#app')
}

bootstrap().catch((e) => {
  console.error('[Bootstrap] Failed:', e)
  // 即使失败也要挂载应用，不能白屏
  const app = createApp(App)
  app.use(createPinia())
  app.mount('#app')
})
