<template>
  <div
    class="app-shell bg-surface text-on-surface antialiased overflow-hidden"
    :style="backgroundStyle"
  >
    <LoginGate
      v-if="!auth.isAuthenticated"
      :is-loading="auth.isLoading"
      :error="auth.error"
      :miniapp-status="auth.miniappStatus"
      :miniapp-message="auth.miniappMessage"
      :session-role="auth.sessionContext?.role"
      @login="handleLogin"
      @reopen-telegram="handleReopenTelegram"
    />

    <div v-else class="flex h-full w-full relative flex-col md:flex-row">
      <DesktopSidebar
        :active-tab="activeTab"
        :role="auth.sessionContext?.role"
        @navigate="navigateTo"
        @logout="auth.logout"
      />

      <MobileBottomNav v-model="navTab" class="md:hidden" />

      <AppContentHost
        :active-tab="activeTab"
        :nav-gate-notice="navGateNotice"
        @navigate="navigateTo"
        @reopen-telegram="handleReopenTelegram"
      />
    </div>
  </div>
</template>

<script setup lang="ts">
import { computed, onMounted, watch } from 'vue'
import { useAuthStore } from '@/stores/auth'
import { useThemeStore } from '@/stores/theme'
import { useTheme } from '@/composables/useTheme'
import { useAppNavigation } from '@/composables/useAppNavigation'
import { useMiniAppEntry } from '@/composables/useMiniAppEntry'
import { useRealtimeConnection } from '@/composables/useRealtimeConnection'
import { useViewportHeight } from '@/composables/useViewportHeight'
import { TAB_BY_ID, routeToTab } from '@/app/navigation/app-tabs'
import { closeMiniApp, isTelegramMiniApp } from '@/lib/telegram'
import LoginGate from '@/app/shell/LoginGate.vue'
import DesktopSidebar from '@/app/shell/DesktopSidebar.vue'
import MobileBottomNav from '@/app/shell/MobileBottomNav.vue'
import AppContentHost from '@/app/shell/AppContentHost.vue'

const auth = useAuthStore()
const themeStore = useThemeStore()
const { activeTab, navTab, navGateNotice, navigateTo } = useAppNavigation()
const { bootstrapMiniAppEntry } = useMiniAppEntry(navigateTo)

useTheme()
useViewportHeight()
useRealtimeConnection()

const isMiniAppEntry = computed(() => location.pathname === '/miniapp')

const backgroundStyle = computed(() => {
  const bg = themeStore.settings.customBackground
  if (!bg) return {}
  return {
    backgroundImage: `url(${bg})`,
    backgroundSize: 'cover',
    backgroundPosition: 'center',
    backgroundRepeat: 'no-repeat',
    backgroundAttachment: 'fixed',
  }
})

onMounted(async () => {
  let bootstrapAttempted = false
  try {
    const bootstrapResult = await bootstrapMiniAppEntry()
    bootstrapAttempted = !!bootstrapResult?.attempted
  } catch (e) {
    console.error('[MiniApp] bootstrap failed:', e)
  }

  if (!bootstrapAttempted) {
    await auth.checkToken()
    if (auth.isAuthenticated) {
      await navigateTo(routeToTab(location.pathname))
      syncAdminUrl(activeTab.value)
    }
  }
})

async function handleLogin(token: string) {
  await auth.login(token)
  if (auth.isAuthenticated && !isMiniAppEntry.value) {
    await navigateTo('saas')
    syncAdminUrl('saas')
  }
}

function syncAdminUrl(tab: string) {
  if (isMiniAppEntry.value || typeof window === 'undefined') return
  const route = TAB_BY_ID[tab as keyof typeof TAB_BY_ID]?.route || '/saas'
  const nextPath = `/admin${route}`
  if (location.pathname !== nextPath) {
    window.history.replaceState({}, '', nextPath)
  }
}

watch(activeTab, (tab) => {
  if (auth.isAuthenticated) {
    syncAdminUrl(tab)
  }
})

function handleReopenTelegram() {
  if (isTelegramMiniApp()) {
    closeMiniApp()
    return
  }
  window.location.reload()
}
</script>

<style scoped>
.app-shell {
  height: calc(var(--app-vh, 1vh) * 100);
  min-height: calc(var(--app-vh, 1vh) * 100);
}
</style>
