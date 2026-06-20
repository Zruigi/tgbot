<template>
  <div class="overflow-x-auto">
    <table class="w-full text-left border-collapse text-sm">
      <thead>
        <tr class="bg-surface-container-low border-b border-outline-variant/10">
          <th class="px-6 py-4 text-[10px] font-black uppercase text-slate-400">时间</th>
          <th class="px-6 py-4 text-[10px] font-black uppercase text-slate-400">操作人</th>
          <th class="px-6 py-4 text-[10px] font-black uppercase text-slate-400">操作类型</th>
          <th class="px-6 py-4 text-[10px] font-black uppercase text-slate-400">目标表</th>
        </tr>
      </thead>
      <tbody class="divide-y divide-outline-variant/5">
        <tr v-for="log in logs" :key="log.audit_id" class="hover:bg-primary/5 transition-colors">
          <td class="px-6 py-4 text-xs text-on-surface">{{ formatTime(log.timestamp) }}</td>
          <td class="px-6 py-4 text-xs font-mono text-on-surface-variant">#{{ log.admin_userid }}</td>
          <td class="px-6 py-4">
            <span class="inline-block px-3 py-1 bg-primary/10 text-primary rounded-lg text-[10px] font-bold">
              {{ log.action }}
            </span>
          </td>
          <td class="px-6 py-4 text-xs text-on-surface-variant">{{ log.target_table }}</td>
        </tr>
      </tbody>
    </table>
  </div>
</template>

<script setup lang="ts">
import type { AuditLog } from '@/stores/finance'

defineProps<{
  logs: AuditLog[]
}>()

const formatTime = (timestamp: string) => {
  const date = new Date(timestamp)
  return date.toLocaleString('zh-CN', {
    year: 'numeric',
    month: '2-digit',
    day: '2-digit',
    hour: '2-digit',
    minute: '2-digit',
    second: '2-digit'
  })
}
</script>
