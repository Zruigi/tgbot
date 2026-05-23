import { defineStore } from 'pinia'
import { ref, computed } from 'vue'

export type DarkModeType = 'auto' | 'light' | 'dark' | 'schedule'

export interface ThemeSettings {
  /** 暗黑模式类型 */
  darkMode: DarkModeType
  /** 定时开启时间 (HH:mm) */
  scheduleStart: string
  /** 定时关闭时间 (HH:mm) */
  scheduleEnd: string
  /** 自定义背景图 (Base64) */
  customBackground: string | null
  /** 聊天区背景图 (Base64) */
  chatBackground: string | null
}

const STORAGE_KEY = 'tgbot-theme-settings'
const DEFAULT_SETTINGS: ThemeSettings = {
  darkMode: 'auto',
  scheduleStart: '20:00',
  scheduleEnd: '07:00',
  customBackground: null,
  chatBackground: null,
}

function loadSettings(): ThemeSettings {
  try {
    const raw = localStorage.getItem(STORAGE_KEY)
    if (raw) {
      const parsed = JSON.parse(raw) as Partial<ThemeSettings>
      return { ...DEFAULT_SETTINGS, ...parsed }
    }
  } catch { /* ignore */ }
  return { ...DEFAULT_SETTINGS }
}

function saveSettings(settings: ThemeSettings): void {
  try {
    localStorage.setItem(STORAGE_KEY, JSON.stringify({
      darkMode: settings.darkMode,
      scheduleStart: settings.scheduleStart,
      scheduleEnd: settings.scheduleEnd,
      customBackground: settings.customBackground,
      chatBackground: settings.chatBackground,
    }))
  } catch { /* ignore quota exceeded */ }
}

export const useThemeStore = defineStore('theme', () => {
  const settings = ref<ThemeSettings>(loadSettings())

  // 当前是否处于暗黑模式（由 composable 计算）
  const isDark = ref(false)

  const computedIsDark = computed(() => isDark.value)

  function setDarkMode(type: DarkModeType) {
    settings.value.darkMode = type
    saveSettings(settings.value)
  }

  function setSchedule(start: string, end: string) {
    settings.value.scheduleStart = start
    settings.value.scheduleEnd = end
    saveSettings(settings.value)
  }

  function setCustomBackground(base64: string | null) {
    settings.value.customBackground = base64
    saveSettings(settings.value)
  }

  function setChatBackground(base64: string | null) {
    settings.value.chatBackground = base64
    saveSettings(settings.value)
  }

  function setDarkState(dark: boolean) {
    isDark.value = dark
    // 同步到 html 元素 class，Tailwind darkMode: 'class' 需要
    document.documentElement.setAttribute('data-theme', dark ? 'dark' : 'light')
    if (dark) {
      document.documentElement.classList.add('dark')
    } else {
      document.documentElement.classList.remove('dark')
    }
  }

  return {
    settings,
    isDark,
    computedIsDark,
    setDarkMode,
    setSchedule,
    setCustomBackground,
    setChatBackground,
    setDarkState,
  }
})
