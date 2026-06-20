<template>
  <nav
    class="fixed bottom-0 left-0 right-0 z-50"
    role="tablist"
    aria-label="主导航"
  >
    <div class="nav-glass-bg">
      <div
        ref="scrollContainerRef"
        class="nav-scroll-container"
        @scroll="onScroll"
      >
        <div class="nav-track">
          <span
            class="nav-active-pill"
            :class="{ 'nav-active-pill--ready': activeIndicatorReady }"
            :style="activeIndicatorStyle"
            aria-hidden="true"
          />
          <NavItem
            v-for="item in visibleItems"
            :key="item.id"
            :item="item"
            :active="activeNavId === item.id"
            @select="onSelect"
          />
        </div>
      </div>

      <ScrollHint
        v-if="hasSecondaryItems"
        :visible="showScrollHint"
      />
    </div>

    <div class="safe-area-spacer" />
  </nav>
</template>

<script setup lang="ts">
import { computed, nextTick, onBeforeUnmount, onMounted, ref, watch } from 'vue'
import NavItem from './NavItem.vue'
import ScrollHint from './ScrollHint.vue'
import { isSecondaryTab, MOBILE_PRIMARY_TABS } from '@/app/navigation/app-tabs'
import type { NavItemDef } from './nav-types'

interface MobileBottomNavProps {
  modelValue: string
}

const props = defineProps<MobileBottomNavProps>()

const emit = defineEmits<{
  (e: 'update:modelValue', tab: string): void
}>()

const visibleItems = computed<NavItemDef[]>(() =>
  MOBILE_PRIMARY_TABS.map(item => ({
    id: item.id,
    icon: item.icon,
    label: item.label,
    group: 'core',
  }))
)

const activeNavId = computed(() => {
  return isSecondaryTab(props.modelValue) ? 'more' : props.modelValue
})

const hasSecondaryItems = computed(() =>
  visibleItems.value.some(item => item.group === 'secondary')
)

const scrollContainerRef = ref<HTMLElement | null>(null)
const showScrollHint = ref(true)
const activeIndicatorReady = ref(false)
const activeIndicatorStyle = ref<Record<string, string>>({
  opacity: '0',
  width: '0px',
  height: '0px',
  transform: 'translate3d(0, 0, 0)',
})

let hasMounted = false
let scrollFrame = 0
let resizeTimer: number | null = null
const ACTIVE_PILL_INSET_X = 5
const ACTIVE_PILL_INSET_TOP = 2
const ACTIVE_PILL_INSET_BOTTOM = 4

function getItemElement(id: string) {
  return scrollContainerRef.value?.querySelector<HTMLElement>(`.nav-item[data-nav-id="${id}"]`) ?? null
}

function onScroll() {
  scheduleScrollState()
}

function scheduleScrollState() {
  if (scrollFrame) return
  scrollFrame = window.requestAnimationFrame(() => {
    scrollFrame = 0
    refreshScrollState()
  })
}

function refreshScrollState() {
  const container = scrollContainerRef.value
  if (!container) return

  const threshold = 12
  const maxScrollLeft = Math.max(0, container.scrollWidth - container.clientWidth)
  showScrollHint.value = hasSecondaryItems.value && container.scrollLeft < maxScrollLeft - threshold
}

function updateActiveIndicator() {
  const target = getItemElement(activeNavId.value)
  if (!target) {
    activeIndicatorStyle.value = {
      ...activeIndicatorStyle.value,
      opacity: '0',
    }
    return
  }

  activeIndicatorStyle.value = {
    width: `${Math.max(target.offsetWidth - ACTIVE_PILL_INSET_X * 2, 0)}px`,
    height: `${Math.max(target.offsetHeight - ACTIVE_PILL_INSET_TOP - ACTIVE_PILL_INSET_BOTTOM, 0)}px`,
    transform: `translate3d(${target.offsetLeft + ACTIVE_PILL_INSET_X}px, ${ACTIVE_PILL_INSET_TOP}px, 0)`,
    opacity: '1',
  }

  if (!activeIndicatorReady.value) {
    activeIndicatorReady.value = true
  }
}

function scrollItemIntoFocus(tabId: string, behavior: ScrollBehavior = 'smooth') {
  const container = scrollContainerRef.value
  const target = getItemElement(tabId)
  if (!container || !target) return

  const rawLeft = target.offsetLeft - (container.clientWidth - target.clientWidth) / 2
  const maxScrollLeft = Math.max(0, container.scrollWidth - container.clientWidth)
  const nextLeft = Math.min(Math.max(rawLeft, 0), maxScrollLeft)

  container.scrollTo({
    left: nextLeft,
    behavior,
  })

  if (behavior === 'auto') {
    refreshScrollState()
  } else {
    scheduleScrollState()
  }
}

function onSelect(tabId: string) {
  emit('update:modelValue', tabId)
  scrollItemIntoFocus(tabId, 'smooth')
}

function handleResize() {
  if (resizeTimer) {
    window.clearTimeout(resizeTimer)
  }
  resizeTimer = window.setTimeout(() => {
    updateActiveIndicator()
    scrollItemIntoFocus(props.modelValue, 'auto')
  }, 80)
}

watch(
  () => props.modelValue,
  async tabId => {
    await nextTick()
    updateActiveIndicator()
    scrollItemIntoFocus(isSecondaryTab(tabId) ? 'more' : tabId, hasMounted ? 'smooth' : 'auto')
  },
  { immediate: true }
)

watch(
  () => visibleItems.value.map(item => item.id).join('|'),
  async () => {
    await nextTick()
    updateActiveIndicator()
    scrollItemIntoFocus(props.modelValue, hasMounted ? 'smooth' : 'auto')
  }
)

onMounted(async () => {
  await nextTick()
  updateActiveIndicator()
  scrollItemIntoFocus(props.modelValue, 'auto')
  refreshScrollState()
  hasMounted = true
  window.addEventListener('resize', handleResize)
  window.visualViewport?.addEventListener('resize', handleResize)
})

onBeforeUnmount(() => {
  if (scrollFrame) {
    window.cancelAnimationFrame(scrollFrame)
  }
  if (resizeTimer) {
    window.clearTimeout(resizeTimer)
  }
  window.removeEventListener('resize', handleResize)
  window.visualViewport?.removeEventListener('resize', handleResize)
})
</script>
