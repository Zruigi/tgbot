import { onBeforeUnmount, onMounted, watch } from 'vue'
import { useAuthStore } from '@/stores/auth'
import { useWsStore } from '@/stores/ws'

export function useRealtimeConnection() {
  const auth = useAuthStore()
  const wsStore = useWsStore()

  watch(
    () => auth.isAuthenticated,
    authed => {
      if (authed) wsStore.connect()
      else wsStore.disconnect()
    },
    { immediate: true }
  )

  function handleOnline() {
    console.log('[Network] 网络已恢复，重新连接 WebSocket')
    if (auth.isAuthenticated) {
      wsStore.disconnect()
      setTimeout(() => wsStore.connect(), 500)
    }
  }

  function handleOffline() {
    console.log('[Network] 网络已断开')
    wsStore.disconnect()
  }

  onMounted(() => {
    window.addEventListener('online', handleOnline)
    window.addEventListener('offline', handleOffline)
  })

  onBeforeUnmount(() => {
    window.removeEventListener('online', handleOnline)
    window.removeEventListener('offline', handleOffline)
  })
}
