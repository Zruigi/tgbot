export type AppTabId =
  | 'chat'
  | 'group_management'
  | 'broadcast'
  | 'onchain_finance'
  | 'more'
  | 'potential_users'
  | 'saas'
  | 'settings'
  | 'monitor'

export interface AppTabDef {
  id: AppTabId
  icon: string
  label: string
  desktopLabel: string
  route: string
  featureKey?: string
  mobilePrimary?: boolean
  secondary?: boolean
  platformOnly?: boolean
}

export interface QuickActionDef {
  key: string
  name: string
  icon: string
  tab: AppTabId
  feature?: string
}

export interface MoreShortcutDef {
  key: string
  icon: string
  label: string
  caption: string
  tab?: AppTabId
  merchantTab?: AppTabId
  featureKey?: string
  platformOnly?: boolean
  merchantLabel?: string
  platformLabel?: string
  merchantCaption?: string
  platformCaption?: string
  action?: 'telegram'
  muted?: boolean
}

export const APP_TABS: AppTabDef[] = [
  { id: 'chat', icon: 'chat', label: '会话', desktopLabel: '会话列表', route: '/chat', mobilePrimary: true },
  { id: 'group_management', icon: 'groups', label: '群聊', desktopLabel: '群组管理', route: '/groups', featureKey: 'group_chat', mobilePrimary: true },
  { id: 'broadcast', icon: 'campaign', label: '群发', desktopLabel: '群发功能', route: '/broadcast', featureKey: 'broadcast', mobilePrimary: true },
  { id: 'onchain_finance', icon: 'account_balance_wallet', label: '财务', desktopLabel: '链上财务', route: '/finance', featureKey: 'shared_ai_translate', mobilePrimary: true },
  { id: 'more', icon: 'more_horiz', label: '更多', desktopLabel: '更多', route: '/more', mobilePrimary: true, secondary: true },
  { id: 'potential_users', icon: 'person_add', label: '潜客', desktopLabel: '潜在用户', route: '/potential', secondary: true },
  { id: 'saas', icon: 'storefront', label: '商户', desktopLabel: '商户管理', route: '/saas', secondary: true },
  { id: 'settings', icon: 'settings', label: 'AI 设置', desktopLabel: 'AI 设置', route: '/settings', platformOnly: true, secondary: true },
  { id: 'monitor', icon: 'analytics', label: '监控', desktopLabel: '链上监控', route: '/monitor', featureKey: 'advanced_analytics', secondary: true },
]

export const TAB_BY_ID = APP_TABS.reduce((acc, tab) => {
  acc[tab.id] = tab
  return acc
}, {} as Record<AppTabId, AppTabDef>)

export const MOBILE_PRIMARY_TABS = APP_TABS.filter(tab => tab.mobilePrimary)
export const SECONDARY_TAB_IDS = new Set(APP_TABS.filter(tab => tab.secondary).map(tab => tab.id))

export const TAB_FEATURE_MAP = APP_TABS.reduce((acc, tab) => {
  if (tab.featureKey) acc[tab.id] = tab.featureKey
  return acc
}, {} as Partial<Record<AppTabId, string>>)

export const FEATURE_LABELS: Record<string, string> = {
  broadcast: '群发功能',
  group_chat: '群聊管理',
  shared_ai_translate: '财务 AI 能力',
  advanced_analytics: '高级监控',
  dual_bot: '双向机器人',
}

export const SAAS_QUICK_ACTIONS: QuickActionDef[] = [
  { key: 'chat', name: '会话', icon: 'chat', tab: 'chat' },
  { key: 'broadcast', name: '群发', icon: 'campaign', tab: 'broadcast', feature: 'broadcast' },
  { key: 'potential', name: '潜在用户', icon: 'person_add', tab: 'potential_users' },
  { key: 'group', name: '群组', icon: 'group', tab: 'group_management', feature: 'group_chat' },
  { key: 'finance', name: '财务', icon: 'account_balance_wallet', tab: 'onchain_finance', feature: 'shared_ai_translate' },
  { key: 'monitor', name: '监控', icon: 'analytics', tab: 'monitor', feature: 'advanced_analytics' },
  { key: 'settings', name: '设置', icon: 'settings', tab: 'settings' },
]

export const MORE_SHORTCUTS: MoreShortcutDef[] = [
  { key: 'potential', icon: 'person_add', label: '潜客', caption: '线索与转化', tab: 'potential_users' },
  {
    key: 'console',
    icon: 'storefront',
    label: '商户管理',
    merchantLabel: '商户控制台',
    caption: '套餐与接入',
    tab: 'saas',
  },
  {
    key: 'settings',
    icon: 'settings',
    label: 'AI 设置',
    merchantLabel: '商户设置',
    caption: '模型与工具',
    merchantCaption: 'Bot 与套餐',
    tab: 'settings',
    merchantTab: 'saas',
  },
  { key: 'monitor', icon: 'analytics', label: '监控', caption: '链上与数据', tab: 'monitor', featureKey: 'advanced_analytics' },
  { key: 'security', icon: 'security', label: '账号安全', caption: '会话与权限', tab: 'saas' },
  { key: 'telegram', icon: 'open_in_new', label: '返回 Telegram', caption: '回到机器人', action: 'telegram', muted: true },
]

export function routeToTab(route: string): AppTabId {
  const normalized = String(route || '').trim().replace(/^\/admin(?=\/|$)/, '') || '/saas'
  return APP_TABS.find(tab => tab.route === normalized)?.id || 'saas'
}

export function isSecondaryTab(tab: string): boolean {
  return SECONDARY_TAB_IDS.has(tab as AppTabId)
}

export function getDesktopNavItems(role?: string) {
  const isMerchantOwner = role === 'merchant_owner'
  return APP_TABS
    .filter(tab => tab.id !== 'more')
    .filter(tab => !(tab.platformOnly && isMerchantOwner))
    .map(tab => ({
      ...tab,
      desktopLabel: tab.id === 'saas' && isMerchantOwner ? '商户控制台' : tab.desktopLabel,
    }))
}
