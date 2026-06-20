/** 导航项定义 */
export interface NavItemDef {
  /** 唯一标识，对应 activeTab 值 */
  id: string
  /** Material Symbols 图标名 */
  icon: string
  /** 显示文字 */
  label: string
  /** 分组：core=核心项(默认可见), secondary=次要项(滑动展开) */
  group: 'core' | 'secondary'
  /** 是否受角色权限控制（如 AI设置仅非商户可见） */
  roleRestricted?: boolean
}
