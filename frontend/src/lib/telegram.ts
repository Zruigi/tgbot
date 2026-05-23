/**
 * Telegram Mini App 集成模块
 * 
 * 使用原生 window.Telegram.WebApp API
 * 
 * 关键：telegram-web-app.js 在非TMA环境也会创建 mock 对象，
 * 需要通过 initData 是否存在来判断是否真正在 TMA 环境中。
 */

interface TelegramWebApp {
  initData: string
  initDataUnsafe: {
    user?: {
      id: number
      first_name: string
      last_name?: string
      username?: string
      language_code?: string
    }
    start_param?: string
    auth_date?: number
  }
  colorScheme: 'light' | 'dark'
  themeParams: Record<string, string>
  platform: string
  version: string
  viewportStableHeight: number
  isExpanded: boolean
  expand(): void
  close(): void
  MainButton: {
    text: string
    isVisible: boolean
    isActive: boolean
    isProgressVisible: boolean
    setText(text: string): any
    show(): any
    hide(): any
    enable(): any
    disable(): any
    showProgress(leaveActive?: boolean): any
    hideProgress(): any
    onClick(fn: () => void): any
    offClick(fn: () => void): any
  }
  BackButton: {
    isVisible: boolean
    show(): any
    hide(): any
    onClick(fn: () => void): any
    offClick(fn: () => void): any
  }
  HapticFeedback: {
    impactOccurred(style: 'light' | 'medium' | 'heavy'): void
    notificationOccurred(type: 'error' | 'success' | 'warning'): void
    selectionChanged(): void
  }
  ClosingBehavior: {
    enableConfirmation(): void
    disableConfirmation(): void
  }
  openLink?(url: string, options?: {
    try_instant_view?: boolean
    try_browser?: string
  }): void
  openTelegramLink?(url: string): void
  onEvent(eventType: string, fn: () => void): void
  offEvent(eventType: string, fn: () => void): void
  ready(): void
}

declare global {
  interface Window {
    Telegram?: {
      WebApp: TelegramWebApp
    }
  }
}

let _webApp: TelegramWebApp | null = null
let _isRealTma = false

/**
 * 检测是否在真实的 Telegram Mini App 环境中
 * 
 * 关键：telegram-web-app.js 在非TMA环境也会创建 mock 对象，
 * 所以必须通过 initData 是否存在来判断。
 * 有 initData = 真正的 TMA 环境（Telegram 签名传递了用户数据）
 * 无 initData = 普通浏览器或 mock 环境
 */
export function isTelegramMiniApp(): boolean {
  return _isRealTma
}

/**
 * 获取 WebApp 实例
 */
function getWebApp(): TelegramWebApp | null {
  if (_webApp) return _webApp
  if (typeof window === 'undefined' || !window.Telegram?.WebApp) return null
  _webApp = window.Telegram.WebApp
  return _webApp
}

/**
 * 初始化 Telegram Mini App
 * 必须在 App 挂载前调用
 */
export function initTelegramMiniApp() {
  const webApp = getWebApp()
  if (!webApp) {
    console.log('[TMA] window.Telegram.WebApp not found')
    return null
  }

  // 关键判断：只有 initData 存在才是真正的 TMA 环境
  if (!webApp.initData) {
    console.log('[TMA] WebApp found but initData is empty — not in real TMA environment')
    _isRealTma = false
    return null
  }

  _isRealTma = true

  try {
    // 1. 先启用全屏模式（必须在 ready() 之前调用）
    //    某些平台需要延迟调用才能生效，所以立即调用一次 + 延迟再调用一次
    webApp.expand()
    setTimeout(() => {
      try { webApp.expand() } catch {}
    }, 100)
    setTimeout(() => {
      try { webApp.expand() } catch {}
    }, 500)

    // 2. 启用关闭确认（防止误关闭）
    webApp.ClosingBehavior.enableConfirmation()

    // 3. 应用 Telegram 主题
    applyTelegramTheme(webApp)

    // 4. 监听主题变化
    webApp.onEvent('themeChanged', () => {
      applyTelegramTheme(webApp)
    })

    // 5. 监听 viewport 变化
    webApp.onEvent('viewportChanged', () => {
      document.documentElement.style.setProperty(
        '--tg-viewport-height',
        `${webApp.viewportStableHeight}px`
      )
      // viewport 变化时再次尝试 expand
      if (!webApp.isExpanded) {
        try { webApp.expand() } catch {}
      }
    })

    // 6. 设置 viewport CSS 变量
    document.documentElement.style.setProperty(
      '--tg-viewport-height',
      `${webApp.viewportStableHeight}px`
    )

    // 7. 添加 Mini App 标记 class
    document.documentElement.classList.add('tg-mini-app')
    document.body.classList.add('tg-mini-app')

    // 8. 设置 body 样式确保全屏
    document.body.style.margin = '0'
    document.body.style.padding = '0'
    document.body.style.minHeight = '100dvh'
    document.body.style.overflow = 'hidden'

    // 9. 最后通知 Telegram 前端已就绪（必须在 expand 之后）
    webApp.ready()

    console.log('[TMA] Initialized successfully', {
      colorScheme: webApp.colorScheme,
      platform: webApp.platform,
      version: webApp.version,
      hasInitData: !!webApp.initData,
      isExpanded: webApp.isExpanded,
    })
  } catch (e) {
    console.error('[TMA] Initialization error:', e)
  }

  return webApp
}

/**
 * 应用 Telegram 主题到 CSS 变量
 */
function applyTelegramTheme(webApp: TelegramWebApp) {
  const root = document.documentElement
  const params = webApp.themeParams

  // 映射 Telegram 主题参数到 CSS 变量
  const themeMap: Record<string, string | undefined> = {
    '--tg-bg-color': params.bg_color,
    '--tg-text-color': params.text_color,
    '--tg-hint-color': params.hint_color,
    '--tg-link-color': params.link_color,
    '--tg-button-color': params.button_color,
    '--tg-button-text-color': params.button_text_color,
    '--tg-secondary-bg-color': params.secondary_bg_color,
    '--tg-header-bg-color': params.header_bg_color,
    '--tg-accent-text-color': params.accent_text_color,
    '--tg-section-bg-color': params.section_bg_color,
    '--tg-section-header-text-color': params.section_header_text_color,
    '--tg-subtitle-text-color': params.subtitle_text_color,
    '--tg-destructive-text-color': params.destructive_text_color,
  }

  for (const [cssVar, value] of Object.entries(themeMap)) {
    if (value) {
      root.style.setProperty(cssVar, value)
    }
  }
}

function isHttpUrl(url: string): boolean {
  return /^https?:\/\//i.test(url)
}

function isTelegramHttpUrl(url: string): boolean {
  return /^https?:\/\/t\.me\/.+/i.test(url)
}

function isTelegramDeepLink(url: string): boolean {
  return /^tg:\/\//i.test(url)
}

/**
 * 获取 Telegram initData 用于后端认证
 */
export function getTelegramInitData(): string | null {
  const webApp = getWebApp()
  if (!webApp || !webApp.initData) return null
  return webApp.initData
}

/**
 * 获取 Telegram 用户信息
 */
export function getTelegramUser() {
  const webApp = getWebApp()
  if (!webApp) return null
  return webApp.initDataUnsafe?.user || null
}

/**
 * 在 Telegram Mini App 中优先走原生能力打开外链，避免 WebView 吞掉点击。
 */
export function openExternalLink(url: string, options?: {
  tryInstantView?: boolean
}): void {
  const nextUrl = String(url || '').trim()
  if (!nextUrl || typeof window === 'undefined') return

  const webApp = getWebApp()

  if (_isRealTma && webApp) {
    if (isTelegramHttpUrl(nextUrl) && typeof webApp.openTelegramLink === 'function') {
      webApp.openTelegramLink(nextUrl)
      return
    }

    if (isHttpUrl(nextUrl) && typeof webApp.openLink === 'function') {
      webApp.openLink(
        nextUrl,
        options?.tryInstantView ? { try_instant_view: true } : undefined
      )
      return
    }
  }

  if (isTelegramDeepLink(nextUrl)) {
    window.location.href = nextUrl
    return
  }

  if (isHttpUrl(nextUrl)) {
    window.open(nextUrl, '_blank', 'noopener,noreferrer')
    return
  }

  window.location.href = nextUrl
}

/**
 * 控制 BackButton
 */
export const tgBackButton = {
  show() {
    if (_isRealTma) getWebApp()?.BackButton.show()
  },
  hide() {
    if (_isRealTma) getWebApp()?.BackButton.hide()
  },
  onClick(handler: () => void) {
    if (_isRealTma) getWebApp()?.BackButton.onClick(handler)
  },
  offClick(handler: () => void) {
    if (_isRealTma) getWebApp()?.BackButton.offClick(handler)
  },
}

/**
 * 控制 MainButton
 */
export const tgMainButton = {
  setText(text: string) {
    if (_isRealTma) getWebApp()?.MainButton.setText(text)
  },
  show() {
    if (_isRealTma) getWebApp()?.MainButton.show()
  },
  hide() {
    if (_isRealTma) getWebApp()?.MainButton.hide()
  },
  enable() {
    if (_isRealTma) getWebApp()?.MainButton.enable()
  },
  disable() {
    if (_isRealTma) getWebApp()?.MainButton.disable()
  },
  showProgress() {
    if (_isRealTma) getWebApp()?.MainButton.showProgress(true)
  },
  hideProgress() {
    if (_isRealTma) getWebApp()?.MainButton.hideProgress()
  },
  onClick(handler: () => void) {
    if (_isRealTma) getWebApp()?.MainButton.onClick(handler)
  },
  offClick(handler: () => void) {
    if (_isRealTma) getWebApp()?.MainButton.offClick(handler)
  },
}

/**
 * Haptic 反馈
 */
export const tgHaptic = {
  impact(style: 'light' | 'medium' | 'heavy' = 'medium') {
    if (_isRealTma) getWebApp()?.HapticFeedback.impactOccurred(style)
  },
  notification(type: 'error' | 'success' | 'warning') {
    if (_isRealTma) getWebApp()?.HapticFeedback.notificationOccurred(type)
  },
  selection() {
    if (_isRealTma) getWebApp()?.HapticFeedback.selectionChanged()
  },
}

/**
 * 关闭 Mini App
 */
export function closeMiniApp() {
  const webApp = getWebApp()
  if (webApp && _isRealTma) {
    webApp.ClosingBehavior.disableConfirmation()
    webApp.close()
  }
}

/**
 * 获取 Mini App 运行状态
 */
export function getMiniAppState() {
  const webApp = getWebApp()
  return {
    isMiniApp: _isRealTma,
    platform: webApp?.platform || 'unknown',
    colorScheme: webApp?.colorScheme || 'light',
    version: webApp?.version || 'unknown',
  }
}
