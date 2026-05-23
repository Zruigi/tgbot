import { onMounted, onBeforeUnmount, watch } from 'vue'
import { useThemeStore } from '@/stores/theme'

/**
 * 检测当前时间是否在定时范围内
 * 支持跨夜场景（如 22:00 - 07:00）
 */
function isTimeInSchedule(start: string, end: string): boolean {
  const now = new Date()
  const currentMinutes = now.getHours() * 60 + now.getMinutes()

  const [startH, startM] = start.split(':').map(Number)
  const [endH, endM] = end.split(':').map(Number)

  const startMinutes = startH * 60 + startM
  let endMinutes = endH * 60 + endM

  // 跨夜处理
  if (endMinutes < startMinutes) {
    return currentMinutes >= startMinutes || currentMinutes < endMinutes
  }
  return currentMinutes >= startMinutes && currentMinutes < endMinutes
}

/**
 * 检查系统是否处于暗黑模式
 */
function systemPrefersDark(): boolean {
  return window.matchMedia('(prefers-color-scheme: dark)').matches
}

export function useTheme() {
  const themeStore = useThemeStore()
  let isThemeAnimating = false
  let animationEndTimer: ReturnType<typeof setTimeout> | null = null
  let animationFailSafeTimer: ReturnType<typeof setTimeout> | null = null
  let activeOverlay: HTMLDivElement | null = null
  let systemMediaQuery: MediaQueryList | null = null

  function clearThemeAnimation() {
    if (animationEndTimer) {
      clearTimeout(animationEndTimer)
      animationEndTimer = null
    }
    if (animationFailSafeTimer) {
      clearTimeout(animationFailSafeTimer)
      animationFailSafeTimer = null
    }
    document.documentElement.classList.remove('theme-transitioning')
    if (activeOverlay) {
      activeOverlay.remove()
      activeOverlay = null
    }
    isThemeAnimating = false
  }

  /** 根据当前设置计算是否应该启用暗黑模式 */
  function evaluateDarkMode(): boolean {
    const { darkMode, scheduleStart, scheduleEnd } = themeStore.settings

    switch (darkMode) {
      case 'light':
        return false
      case 'dark':
        return true
      case 'auto':
        return systemPrefersDark()
      case 'schedule':
        return isTimeInSchedule(scheduleStart, scheduleEnd)
      default:
        return false
    }
  }

  function animateThemeSwitch(nextDark: boolean) {
    if (window.matchMedia('(prefers-reduced-motion: reduce)').matches) {
      themeStore.setDarkState(nextDark)
      return
    }

    if (isThemeAnimating) {
      clearThemeAnimation()
    }

    document.querySelectorAll('.theme-refresh-overlay').forEach((el) => el.remove())

    const overlay = document.createElement('div')
    overlay.className = `theme-refresh-overlay ${nextDark ? 'to-dark' : 'to-light'}`
    overlay.innerHTML = '<div class="theme-refresh-overlay__veil"></div><div class="theme-refresh-overlay__progress"></div>'
    document.body.appendChild(overlay)
    activeOverlay = overlay

    document.documentElement.classList.add('theme-transitioning')
    isThemeAnimating = true

    requestAnimationFrame(() => requestAnimationFrame(() => {
      overlay.classList.add('is-running')
    }))

    // 在动画中段切换主题，形成“缓慢推进切换”的视觉效果
    window.setTimeout(() => {
      themeStore.setDarkState(nextDark)
    }, 560)

    animationEndTimer = window.setTimeout(() => {
      overlay.classList.add('is-ending')
      window.setTimeout(() => {
        clearThemeAnimation()
      }, 320)
    }, 1380)

    // 兜底，防止移动端 WebView 定时器异常导致蒙层残留
    animationFailSafeTimer = window.setTimeout(() => {
      clearThemeAnimation()
    }, 2600)
  }

  /** 立即评估并应用主题 */
  function applyTheme(animated = false) {
    const nextDark = evaluateDarkMode()
    if (nextDark === themeStore.isDark) return

    if (animated) {
      animateThemeSwitch(nextDark)
    } else {
      themeStore.setDarkState(nextDark)
    }
  }

  let scheduleTimer: ReturnType<typeof setInterval> | null = null

  /** 监听系统暗黑模式变化的 handler */
  function handleSystemChange() {
    if (themeStore.settings.darkMode === 'auto') {
      applyTheme(true)
    }
  }

  onMounted(() => {
    // 初始化：立即应用一次
    applyTheme(false)

    // 监听系统暗黑模式变化
    systemMediaQuery = window.matchMedia('(prefers-color-scheme: dark)')
    systemMediaQuery.addEventListener('change', handleSystemChange)

    // 定时模式：每分钟检查一次
    scheduleTimer = setInterval(() => {
      if (themeStore.settings.darkMode === 'schedule') {
        applyTheme(false)
      }
    }, 60000)
  })

  // 关键修复：主题设置变化时立即生效（不再需要刷新页面）
  watch(
    () => [
      themeStore.settings.darkMode,
      themeStore.settings.scheduleStart,
      themeStore.settings.scheduleEnd,
    ],
    () => {
      applyTheme(true)
    },
  )

  onBeforeUnmount(() => {
    if (scheduleTimer) {
      clearInterval(scheduleTimer)
      scheduleTimer = null
    }
    clearThemeAnimation()
    systemMediaQuery?.removeEventListener('change', handleSystemChange)
  })

  return {
    store: themeStore,
    applyTheme,
  }
}
