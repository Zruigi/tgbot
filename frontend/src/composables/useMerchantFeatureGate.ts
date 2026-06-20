import { computed, ref, watch } from 'vue'
import { api } from '@/api'
import { useAuthStore } from '@/stores/auth'
import { TAB_FEATURE_MAP, type AppTabId } from '@/app/navigation/app-tabs'

export function normalizeFeatureMap(data: unknown): Record<string, boolean> | null {
  if (!data || typeof data !== 'object' || Array.isArray(data)) return null
  return Object.entries(data as Record<string, unknown>).reduce((acc, [key, value]) => {
    acc[key] = typeof value === 'boolean'
      ? value
      : Boolean((value as any)?.enabled ?? (value as any)?.is_enabled)
    return acc
  }, {} as Record<string, boolean>)
}

export function useMerchantFeatureGate() {
  const auth = useAuthStore()
  const features = ref<Record<string, boolean> | null>(null)
  let featureLoadPromise: Promise<Record<string, boolean> | null> | null = null

  const isMerchantScopedSession = computed(() => {
    return auth.sessionContext?.role === 'merchant_owner'
      && !!auth.sessionContext?.tenantId
      && auth.sessionContext?.merchantId != null
  })

  function reset() {
    features.value = null
    featureLoadPromise = null
  }

  watch(
    () => `${auth.isAuthenticated}:${auth.sessionContext?.role}:${auth.sessionContext?.tenantId}:${auth.sessionContext?.merchantId}`,
    reset
  )

  async function loadFeatures() {
    if (!isMerchantScopedSession.value) return null
    if (features.value) return features.value
    if (featureLoadPromise) return featureLoadPromise

    featureLoadPromise = api.getMerchantFeatures()
      .then(res => {
        features.value = normalizeFeatureMap(res?.data)
        return features.value
      })
      .catch(error => {
        console.warn('[Navigation] feature gate lookup failed:', error)
        return null
      })
      .finally(() => {
        featureLoadPromise = null
      })

    return featureLoadPromise
  }

  async function canAccessTab(tab: string) {
    const featureKey = TAB_FEATURE_MAP[tab as AppTabId]
    if (!featureKey || !isMerchantScopedSession.value) {
      return { allowed: true, featureKey }
    }

    const currentFeatures = await loadFeatures()
    if (currentFeatures && currentFeatures[featureKey] === false) {
      return { allowed: false, featureKey }
    }

    return { allowed: true, featureKey }
  }

  function isKnownLocked(featureKey?: string) {
    if (!featureKey || !features.value) return false
    return features.value[featureKey] === false
  }

  return {
    features,
    isMerchantScopedSession,
    loadFeatures,
    canAccessTab,
    isKnownLocked,
    reset,
  }
}
