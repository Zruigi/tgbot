<template>
  <div class="layout">
    <!-- Sidebar -->
    <aside class="sidebar">
      <div class="sidebar-header">
        <span class="brand">⚡ TGBot</span>
      </div>
      <nav class="sidebar-nav">
        <RouterLink to="/chat" class="nav-item" active-class="active">💬 客服消息</RouterLink>
        <RouterLink to="/monitor" class="nav-item" active-class="active">📊 链上监控</RouterLink>
        <RouterLink to="/groups" class="nav-item" active-class="active">👥 群组管理</RouterLink>
        <RouterLink to="/broadcast" class="nav-item" active-class="active">📢 群发推送</RouterLink>
        <RouterLink to="/finance" class="nav-item" active-class="active">💰 链上财务</RouterLink>
        <RouterLink to="/settings" class="nav-item" active-class="active">⚙️ 系统设置</RouterLink>
      </nav>
      <div class="sidebar-footer">
        <button class="btn btn-ghost btn-sm" @click="logout">退出登录</button>
      </div>
    </aside>
    <!-- Content -->
    <div class="main-content">
      <RouterView />
    </div>
  </div>
</template>

<script setup lang="ts">
import { RouterView, RouterLink, useRouter } from 'vue-router'
import { useAuthStore } from '@/stores/auth'

const auth = useAuthStore()
const router = useRouter()
function logout() { auth.logout(); router.push('/') }
</script>

<style scoped>
.sidebar-header {
  padding: 18px 20px 12px;
  border-bottom: 1px solid var(--border-muted);
}
.brand { font-size: 16px; font-weight: 700; color: var(--text-primary); }
.sidebar-nav { flex: 1; padding: 8px 0; }
.sidebar-footer { padding: 12px 16px; border-top: 1px solid var(--border-muted); }
</style>
