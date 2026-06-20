<template>
  <aside class="hidden md:flex flex-col h-full bg-surface-container w-20 lg:w-72 z-20 transition-all duration-100 ease-in-out border-r border-outline-variant/20">
    <div class="flex items-center px-6 py-6 border-b border-outline-variant/10">
      <span class="material-symbols-outlined text-primary text-3xl mr-3">smart_toy</span>
      <div class="hidden lg:block">
        <div class="text-primary font-black uppercase tracking-widest text-xs">Bot Admin</div>
        <div class="text-[10px] text-slate-400 mt-0.5">来访用户管理系统</div>
      </div>
    </div>

    <nav class="flex-1 flex flex-col px-2 py-4 space-y-1 overflow-y-auto no-scrollbar">
      <div class="px-4 mb-2 hidden lg:block text-[10px] font-bold text-slate-400 uppercase tracking-widest">消息中心</div>

      <a
        v-for="item in navItems"
        :key="item.id"
        href="#"
        class="flex items-center px-4 py-3 rounded-xl group transition-all duration-200 active:scale-95"
        :class="activeTab === item.id ? 'bg-primary/10 text-primary shadow-sm' : 'text-on-surface-variant hover:bg-surface-container-high'"
        @click.prevent="$emit('navigate', item.id)"
      >
        <span class="material-symbols-outlined mr-3">{{ item.icon }}</span>
        <span class="hidden lg:block font-bold text-sm">{{ item.desktopLabel }}</span>
      </a>

      <div class="flex-1"></div>

      <a
        href="#"
        class="flex items-center px-4 py-3 rounded-xl text-on-surface-variant hover:bg-error/10 hover:text-error transition-all duration-200"
        @click.prevent="$emit('logout')"
      >
        <span class="material-symbols-outlined mr-3">logout</span>
        <span class="hidden lg:block font-bold text-sm">退出登录</span>
      </a>
    </nav>
  </aside>
</template>

<script setup lang="ts">
import { computed } from 'vue'
import { getDesktopNavItems, type AppTabId } from '@/app/navigation/app-tabs'

const props = defineProps<{
  activeTab: string
  role?: string
}>()

defineEmits<{
  navigate: [tab: AppTabId]
  logout: []
}>()

const navItems = computed(() => getDesktopNavItems(props.role))
</script>
