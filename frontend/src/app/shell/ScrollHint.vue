<template>
  <!--
    右侧滑动提示
    - 需求点：当次要项被隐藏时，右侧显示渐变遮罩 + 箭头，提示用户可向左滑动
    - 需求点：当滚动到底部（次要项全部可见）时，提示消失
    - position: absolute 定位在 nav-glass-bg 右侧
  -->
  <div
    class="nav-scroll-hint"
    :class="{ 'nav-scroll-hint--hidden': !visible }"
  >
    <div class="nav-scroll-hint__gradient" />
    <span class="material-symbols-outlined nav-scroll-hint__arrow">chevron_right</span>
  </div>
</template>

<script setup lang="ts">
interface ScrollHintProps {
  /** 是否可见（次要项未全部滑入时为 true） */
  visible: boolean
}

defineProps<ScrollHintProps>()
</script>
