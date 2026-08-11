<template>
  <main class="flex-1 flex flex-col relative h-full bg-surface overflow-hidden pb-16 md:pb-0">
    <TopScopeBar />

    <div v-if="navGateNotice" class="mx-4 mt-3 rounded-2xl border border-amber-200/70 bg-amber-50/80 px-4 py-3 text-sm font-bold text-amber-800">
      {{ navGateNotice }}
    </div>

    <component
      :is="activeModule.component"
      :key="activeModule.id"
      @chat="onOpenChat"
      @navigate="$emit('navigate', $event)"
      @reopen-telegram="$emit('reopenTelegram')"
    />
  </main>
</template>

<script setup lang="ts">
import { computed, onMounted } from 'vue'
import { useChatStore } from '@/stores/chat'
import { useFileStore } from '@/stores/fileStore'
import type { AppTabId } from '@/app/navigation/app-tabs'
import { resolveAppModule } from '@/app/modules/app-modules'
import TopScopeBar from '@/app/shell/TopScopeBar.vue'

const props = defineProps<{
  activeTab: string
  navGateNotice: string
}>()

const emit = defineEmits<{
  navigate: [tab: AppTabId | string]
  reopenTelegram: []
}>()

const chatStore = useChatStore()
const fileStore = useFileStore()
const activeModule = computed(() => resolveAppModule(props.activeTab))

// ★ 进入主界面立即水合租户头像/文件缓存（冷启动秒出，不等接口返回）
onMounted(() => {
  try { fileStore.ensureHydrated() } catch { /* ignore */ }
})

function onOpenChat(userId: number, name: string) {
  chatStore.selectedUserId = userId
  chatStore.selectedUserName = name
  emit('navigate', 'chat')
}
</script>
