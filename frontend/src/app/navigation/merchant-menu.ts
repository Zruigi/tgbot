/**
 * 商户管理员菜单配置
 * 定义商户可见的菜单树（与平台共享模板，数据层隔离）
 */

import type { MenuItem } from './admin-menu'

export const merchantMenu: MenuItem[] = [
  {
    id: 'merchant_console',
    label: '商户工作台',
    icon: 'storefront',
    tab: 'saas',
    requiredScope: 'merchant'
  },
  {
    id: 'customer_ops',
    label: '客户运营',
    icon: 'chat',
    children: [
      { id: 'chat', label: '会话列表', icon: 'chat', tab: 'chat', requiredScope: 'both' },
      { id: 'broadcast', label: '群发功能', icon: 'campaign', tab: 'broadcast', requiredScope: 'both' },
      { id: 'potential_users', label: '潜在用户', icon: 'person_add', tab: 'potential_users', requiredScope: 'both' },
    ]
  },
  {
    id: 'group_ops',
    label: '社群运营',
    icon: 'groups',
    children: [
      { id: 'group_management', label: '群组管理', icon: 'groups', tab: 'group_management', requiredScope: 'both' },
    ]
  },
  {
    id: 'finance',
    label: '财务中心',
    icon: 'account_balance_wallet',
    tab: 'onchain_finance',
    requiredScope: 'merchant'
  },
  {
    id: 'merchant_settings',
    label: '商户设置',
    icon: 'settings',
    tab: 'settings',
    requiredScope: 'merchant'
  }
]

export function getMerchantMenuItems(): MenuItem[] {
  return merchantMenu
}