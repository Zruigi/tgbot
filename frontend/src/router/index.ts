import { createRouter, createWebHistory } from 'vue-router'
import { useAuthStore } from '@/stores/auth'
import { merchantApi } from '@/api/merchant'

const routes = [
  {
    path: '/',
    component: () => import('@/views/LoginView.vue'),
    meta: { public: true },
  },
  {
    path: '/onboarding',
    component: () => import('@/views/MerchantOnboardingView.vue'),
    meta: { merchantOnly: true },
  },
  {
    path: '/chat',
    component: () => import('@/views/ChatView.vue'),
  },
  {
    path: '/saas',
    component: () => import('@/views/SaasConsoleView.vue'),
  },
  {
    path: '/monitor',
    component: () => import('@/views/MonitorView.vue'),
  },
  {
    path: '/groups',
    component: () => import('@/views/GroupView.vue'),
  },
  {
    path: '/broadcast',
    component: () => import('@/views/BroadcastView.vue'),
  },
  {
    path: '/settings',
    component: () => import('@/views/SettingsView.vue'),
    meta: { title: '系统设置' },
  },
  {
    path: '/finance',
    component: () => import('@/views/OnChainFinancePanel.vue'),
    meta: { title: '链上财务' },
  },
  { path: '/:pathMatch(.*)*', redirect: '/saas' },
]

const router = createRouter({
  history: createWebHistory(),
  routes,
})

router.beforeEach(async (to: any) => {
  const auth = useAuthStore()
  
  // 公开页面不需要认证
  if (to.meta.public) {
    return
  }
  
  // 未认证用户重定向到登录页
  if (!auth.isAuthenticated) {
    return '/'
  }
  
  // 商户角色特殊处理
  if (auth.sessionContext?.role === 'merchant_owner') {
    // 如果访问onboarding页面，允许
    if (to.path === '/onboarding') {
      return
    }
    
    // 如果访问saas页面，允许
    if (to.path === '/saas') {
      return
    }
    
    // 检查商户是否完成注册
    try {
      const merchant = auth.sessionContext

      // 如果商户未完成注册（没有merchantId），跳转到onboarding
      if (!merchant?.merchantId || merchant?.merchantId === 0) {
        return '/onboarding'
      }

      // 检查商户是否完成 onboarding（选择套餐并确认）
      const profile = await merchantApi.getProfile()
      if (profile?.data && !profile.data.onboarding_completed) {
        return '/onboarding'
      }

      // 其他页面都重定向到saas
      return '/saas'
    } catch {
      return '/onboarding'
    }
  }
  
  // 管理员可以访问所有页面
})

export default router
