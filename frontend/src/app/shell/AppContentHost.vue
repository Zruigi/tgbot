<template>
  <main class="flex-1 flex flex-col relative h-full bg-surface overflow-hidden pb-16 md:pb-0">
    <TopScopeBar />

    <div v-if="showAfterHoursBanner" class="mx-4 mt-3 rounded-2xl border border-primary/15 bg-primary/10 px-4 py-3 text-sm text-on-surface-variant">
      当前为非营业时段，已切换到值班模式。你仍可查看数据，敏感操作建议在营业时段处理。
    </div>

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
import { computed } from 'vue'
import { useChatStore } from '@/stores/chat'
import type { AppTabId } from '@/app/navigation/app-tabs'
import { resolveAppModule } from '@/app/modules/app-modules'
import TopScopeBar from '@/app/shell/TopScopeBar.vue'

const props = defineProps<{
  activeTab: string
  showAfterHoursBanner: boolean
  navGateNotice: string
}>()

const emit = defineEmits<{
  navigate: [tab: AppTabId | string]
  reopenTelegram: []
}>()

const chatStore = useChatStore()
const activeModule = computed(() => resolveAppModule(props.activeTab))

function onOpenChat(userId: number, name: string) {
  chatStore.selectedUserId = userId
  chatStore.selectedUserName = name
  emit('navigate', 'chat')
}
</script>
