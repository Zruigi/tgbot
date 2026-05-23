export type MerchantState =
  | 'INIT'
  | 'WAIT_BOT_TOKEN'
  | 'TOKEN_VALIDATING'
  | 'ACTIVE_FREE'
  | 'ACTIVE_PRO'
  | 'EXPIRED_DOWNGRADED'
  | 'SUSPENDED'

function normalizeText(value: unknown): string {
  return String(value ?? '').trim().toLowerCase()
}

export function resolveMerchantState(profile?: Record<string, any> | null, subscription?: Record<string, any> | null): MerchantState {
  const profileStatus = normalizeText(profile?.status)
  const subscriptionStatus = normalizeText(subscription?.status)
  const planType = normalizeText(profile?.plan_type || subscription?.plan_code)
  const hasBotToken = Boolean(profile?.bot_token)

  if (profileStatus === 'suspended') return 'SUSPENDED'
  if (subscriptionStatus === 'expired') return 'EXPIRED_DOWNGRADED'
  if (profileStatus === 'disabled') return 'SUSPENDED'

  if (!hasBotToken) return 'WAIT_BOT_TOKEN'

  if (
    profileStatus === 'active_paid'
    || profileStatus === 'active_pro'
    || planType === 'pro'
    || planType === 'paid'
    || subscriptionStatus === 'active_subscription'
  ) {
    return 'ACTIVE_PRO'
  }

  if (
    profileStatus === 'active_free'
    || profileStatus === 'active'
    || planType === 'free'
    || subscriptionStatus === 'active'
    || subscriptionStatus === 'free'
  ) {
    return 'ACTIVE_FREE'
  }

  return 'INIT'
}
