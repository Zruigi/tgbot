import { createRouter, createWebHistory } from 'vue-router'
import { useAuthStore } from '@/stores/auth'

const routes = [
  {
    path: '/',
    component: () => import('@/views/LoginView.vue'),
    meta: { public: true },
  },
  {
    path: '/chat',
    component: () => import('@/views/ChatView.vue'),
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
  { path: '/:pathMatch(.*)*', redirect: '/chat' },
]

const router = createRouter({
  history: createWebHistory(),
  routes,
})

router.beforeEach((to: any) => {
  const auth = useAuthStore()
  if (!to.meta.public && !auth.isAuthenticated) {
    return '/'
  }
  // 已登录时访问根路径，跳到主界面
  if (to.path === '/' && auth.isAuthenticated) {
    return '/chat'
  }
})

export default router
