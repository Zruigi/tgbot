<template>
  <a
    href="#"
    role="tab"
    :aria-selected="active"
    :aria-label="item.label"
    :data-nav-id="item.id"
    class="nav-item relative"
    :class="{ 'nav-item--active': active }"
    @click.prevent="$emit('select', item.id)"
  >
    <span class="material-symbols-outlined nav-item__icon">{{ item.icon }}</span>
    <span class="nav-item__label">{{ item.label }}</span>
  </a>
</template>

<script setup lang="ts">
import type { NavItemDef } from './nav-types'

interface NavItemProps {
  item: NavItemDef
  active: boolean
}

defineProps<NavItemProps>()

defineEmits<{
  (e: 'select', id: string): void
}>()
</script>
