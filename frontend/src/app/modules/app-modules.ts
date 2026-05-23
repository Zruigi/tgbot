import { defineAsyncComponent, type Component } from 'vue'
import type { AppTabId } from '@/app/navigation/app-tabs'

export interface AppModuleDef {
  id: AppTabId
  component: Component
  legacy: boolean
}

export const APP_MODULES: Record<AppTabId, AppModuleDef> = {
  chat: {
    id: 'chat',
    legacy: true,
    component: defineAsyncComponent(() => import('@/views/ChatPanel.vue')),
  },
  group_management: {
    id: 'group_management',
    legacy: true,
    component: defineAsyncComponent(() => import('@/views/GroupPanel.vue')),
  },
  broadcast: {
    id: 'broadcast',
    legacy: true,
    component: defineAsyncComponent(() => import('@/views/BroadcastPanel.vue')),
  },
  onchain_finance: {
    id: 'onchain_finance',
    legacy: true,
    component: defineAsyncComponent(() => import('@/views/OnChainFinancePanel.vue')),
  },
  more: {
    id: 'more',
    legacy: false,
    component: defineAsyncComponent(() => import('@/views/MorePanel.vue')),
  },
  potential_users: {
    id: 'potential_users',
    legacy: true,
    component: defineAsyncComponent(() => import('@/views/PotentialPanel.vue')),
  },
  saas: {
    id: 'saas',
    legacy: false,
    component: defineAsyncComponent(() => import('@/views/SaasConsoleView.vue')),
  },
  settings: {
    id: 'settings',
    legacy: true,
    component: defineAsyncComponent(() => import('@/views/SettingsView.vue')),
  },
  monitor: {
    id: 'monitor',
    legacy: true,
    component: defineAsyncComponent(() => import('@/views/MonitorPanel.vue')),
  },
}

export function resolveAppModule(tab: string): AppModuleDef {
  return APP_MODULES[tab as AppTabId] || APP_MODULES.saas
}
