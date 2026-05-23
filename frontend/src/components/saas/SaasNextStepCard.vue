<template>
  <article class="next-step" :class="stepClass">
    <span class="step-icon material-symbols-outlined">{{ stepIcon }}</span>
    <div class="step-content">
      <h3 class="step-title">{{ step.title }}</h3>
      <p v-if="step.description" class="step-desc">{{ step.description }}</p>
    </div>
    <button class="uc-btn step-btn" @click="$emit('action', step.action)">
      {{ step.buttonText || '去完成' }}
      <span class="material-symbols-outlined" style="font-size: 16px">arrow_forward</span>
    </button>
  </article>
</template>

<script setup lang="ts">
import { computed } from 'vue'

export interface NextStep {
  title: string
  description?: string
  buttonText?: string
  action: string
  urgency?: 'normal' | 'high' | 'critical'
}

const props = defineProps<{
  step: NextStep
}>()

defineEmits<{
  action: [action: string]
}>()

const stepClass = computed(() => {
  switch (props.step.urgency) {
    case 'critical': return 'critical'
    case 'high': return 'high'
    default: return 'normal'
  }
})

const stepIcon = computed(() => {
  switch (props.step.urgency) {
    case 'critical': return 'warning'
    case 'high': return 'schedule'
    default: return 'task_alt'
  }
})
</script>

<style scoped>
.next-step {
  display: flex;
  align-items: center;
  gap: 16px;
  padding: 16px 20px;
  border-radius: 16px;
  background: linear-gradient(135deg, rgba(36,161,222,.10), rgba(29,120,213,.08));
  border: 1px solid rgba(36,161,222,.18);
  transition: all 0.2s;
}

.dark .next-step {
  background: linear-gradient(135deg, rgba(36,161,222,.14), rgba(29,120,213,.1));
  border-color: rgba(36,161,222,.22);
}

.next-step:hover {
  border-color: rgba(36,161,222,.3);
  box-shadow: 0 4px 20px rgba(36,161,222,.12);
}

.dark .next-step:hover {
  border-color: rgba(82,190,255,.35);
  box-shadow: 0 4px 20px rgba(36,161,222,.18);
}

.next-step.critical {
  background: linear-gradient(135deg, rgba(239,68,68,.1), rgba(220,38,38,.06));
  border-color: rgba(239,68,68,.2);
}

.dark .next-step.critical {
  background: linear-gradient(135deg, rgba(239,68,68,.15), rgba(220,38,38,.1));
}

.next-step.critical:hover {
  border-color: rgba(239,68,68,.35);
  box-shadow: 0 4px 20px rgba(239,68,68,.15);
}

.next-step.high {
  background: linear-gradient(135deg, rgba(245,158,11,.1), rgba(217,119,6,.06));
  border-color: rgba(245,158,11,.2);
}

.dark .next-step.high {
  background: linear-gradient(135deg, rgba(245,158,11,.15), rgba(217,119,6,.1));
}

.next-step.high:hover {
  border-color: rgba(245,158,11,.35);
  box-shadow: 0 4px 20px rgba(245,158,11,.15);
}

.step-icon {
  font-size: 32px;
  flex-shrink: 0;
  color: #24a1de;
}

.dark .step-icon {
  color: #8fd8ff;
}

.next-step.critical .step-icon {
  color: #dc2626;
}

.dark .next-step.critical .step-icon {
  color: #f87171;
}

.next-step.high .step-icon {
  color: #d97706;
}

.dark .next-step.high .step-icon {
  color: #fbbf24;
}

.step-content {
  flex: 1;
}

.step-title {
  margin: 0 0 4px;
  font-size: 16px;
  font-weight: 700;
  color: #111827;
}

.dark .step-title {
  color: #f1f5f9;
}

.step-desc {
  margin: 0;
  font-size: 14px;
  color: #6b7280;
}

.dark .step-desc {
  color: #9ca3af;
}

.step-btn {
  display: flex;
  align-items: center;
  gap: 6px;
  white-space: nowrap;
  background: #24a1de;
}

.dark .step-btn {
  background: #24a1de;
}

.step-btn:hover {
  background: #1d78d5;
}

.dark .step-btn:hover {
  background: #1d78d5;
}

.next-step.critical .step-btn {
  background: #dc2626;
}

.dark .next-step.critical .step-btn {
  background: #ef4444;
}

.next-step.critical .step-btn:hover {
  background: #b91c1c;
}

.dark .next-step.critical .step-btn:hover {
  background: #dc2626;
}

.next-step.high .step-btn {
  background: #d97706;
}

.dark .next-step.high .step-btn {
  background: #f59e0b;
}

.next-step.high .step-btn:hover {
  background: #b45309;
}

.dark .next-step.high .step-btn:hover {
  background: #d97706;
}

@media (max-width: 640px) {
  .next-step {
    flex-direction: column;
    text-align: center;
  }
  
  .step-content {
    order: 2;
  }
  
  .step-icon {
    order: 1;
  }
  
  .step-btn {
    order: 3;
    width: 100%;
    justify-content: center;
  }
}
</style>
