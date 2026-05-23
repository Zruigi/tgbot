import { computed, onBeforeUnmount, ref, watch } from 'vue'
import { useAuthStore } from '@/stores/auth'
import { FEATURE_LABELS, TAB_BY_ID, type AppTabId } from '@/app/navigation/app-tabs'
import { useMerchantFeatureGate } from '@/composables/useMerchantFeatureGate'
import { tgHaptic } from '@/lib/telegram'

function normalizeTab(tab: string): AppTabId {
  return TAB_BY_ID[tab as AppTabId] ? tab as AppTabId : 'saas'
}

export function useAppNavigation() {
  const auth = useAuthStore()
  const featureGate = useMerchantFeatureGate()
  const activeTab = ref<AppTabId>('saas')
  const navGateNotice = ref('')
  let navNoticeTimer: number | null = null

  const navTab = computed({
    get: () => activeTab.value,
    set: (tab: string) => {
      void navigateTo(tab)
    },
  })

  const hasConfirmedMerchantSession = computed(() => {
    return auth.sessionContext.role === 'merchant_owner'
      && !!auth.sessionContext.tenantId
      && auth.sessionContext.merchantId != null
  })

  watch(hasConfirmedMerchantSession, (isMerchantSession) => {
    if (isMerchantSession) activeTab.value = 'saas'
  }, { immediate: true })

  watch(
    () => `${auth.isAuthenticated}:${auth.sessionContext?.role}:${auth.sessionContext?.tenantId}:${auth.sessionContext?.merchantId}`,
    () => {
      navGateNotice.value = ''
    }
  )

  function showGateNotice(featureKey: string) {
    const label = FEATURE_LABELS[featureKey] || '该功能'
    navGateNotice.value = `${label}需要升级套餐或恢复服务，已为你打开商户控制台。`
    tgHaptic.notification('warning')
    if (navNoticeTimer) window.clearTimeout(navNoticeTimer)
    navNoticeTimer = window.setTimeout(() => {
      navGateNotice.value = ''
      navNoticeTimer = null
    }, 4200)
  }

  async function navigateTo(tab: string) {
    const target = normalizeTab(tab || 'chat')
    const gate = await featureGate.canAccessTab(target)

    if (!gate.allowed && gate.featureKey) {
      activeTab.value = 'saas'
      showGateNotice(gate.featureKey)
      return
    }

    activeTab.value = target
  }

  onBeforeUnmount(() => {
    if (navNoticeTimer) window.clearTimeout(navNoticeTimer)
  })

  return {
    activeTab,
    navTab,
    navGateNotice,
    navigateTo,
  }
}
