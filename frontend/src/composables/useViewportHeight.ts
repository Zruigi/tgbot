import { onBeforeUnmount, onMounted } from 'vue'

export function useViewportHeight() {
  let viewportBaseHeight = 0

  function syncViewportHeight() {
    const vv = window.visualViewport
    const height = vv ? vv.height : window.innerHeight
    if (!viewportBaseHeight || height > viewportBaseHeight) viewportBaseHeight = height
    const keyboardOffset = Math.max(0, viewportBaseHeight - height)
    document.documentElement.style.setProperty('--app-vh', `${height * 0.01}px`)
    document.documentElement.style.setProperty('--kb-offset', `${keyboardOffset}px`)
  }

  onMounted(() => {
    syncViewportHeight()
    window.addEventListener('resize', syncViewportHeight)
    window.visualViewport?.addEventListener('resize', syncViewportHeight)
  })

  onBeforeUnmount(() => {
    window.removeEventListener('resize', syncViewportHeight)
    window.visualViewport?.removeEventListener('resize', syncViewportHeight)
  })

  return {
    syncViewportHeight,
  }
}
