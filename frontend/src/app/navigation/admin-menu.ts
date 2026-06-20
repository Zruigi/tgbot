/**
 * 平台管理员菜单配置
 * 定义平台管理员可见的完整菜单树
 */

export interface MenuItem {
  id: string
  label: string
  icon: string
  tab?: string        // 对应 activeTab 值
  route?: string      // 路由路径
  children?: MenuItem[]
  requiredScope: 'platform' | 'merchant' | 'both'
}

export const adminMenu: MenuItem[] = [
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
    id: 'merchant_center',
    label: '商户中心',
    icon: 'storefront',
    tab: 'saas',
    requiredScope: 'platform'
  },
  {
    id: 'finance',
    label: '财务风控',
    icon: 'account_balance_wallet',
    tab: 'onchain_finance',
    requiredScope: 'both'
  },
  {
    id: 'settings',
    label: '系统设置',
    icon: 'settings',
    tab: 'settings',
    requiredScope: 'platform'
  }
]

export function getAdminMenuItems(): MenuItem[] {
  return adminMenu
}