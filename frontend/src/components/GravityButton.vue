<template>
  <div
    v-show="visible"
    ref="wrapRef"
    class="gravity-btn-wrap"
    @click="handleClick"
  >
    <div ref="pillRef" class="gravity-pill">
      <div ref="dotsRef" class="gravity-dots">
        <span ref="dot0" class="gravity-dot"></span>
        <span ref="dot1" class="gravity-dot"></span>
        <span ref="dot2" class="gravity-dot"></span>
      </div>
      <span ref="labelRef" class="gravity-label">回到底部</span>
    </div>
  </div>
</template>

<script setup lang="ts">
import { ref, watch, onMounted, onUnmounted } from 'vue'
import gsap from 'gsap'

const props = defineProps<{
  show:      boolean
  container?: HTMLElement | null  // 消息滚动容器（relative 父级）
  inputBar?:  HTMLElement | null  // 输入框容器，用于计算碰撞落点
}>()

const emit = defineEmits<{ trigger: [] }>()

// ── DOM refs ──────────────────────────────────────────────
const wrapRef  = ref<HTMLElement | null>(null)
const pillRef  = ref<HTMLElement | null>(null)
const dotsRef  = ref<HTMLElement | null>(null)
const dot0     = ref<HTMLElement | null>(null)
const dot1     = ref<HTMLElement | null>(null)
const dot2     = ref<HTMLElement | null>(null)
const labelRef = ref<HTMLElement | null>(null)

const visible   = ref(false)
let dotsTimeline: gsap.core.Timeline | null = null
let isAnimating = false

watch(() => props.show, (val) => { val ? enterWaiting() : leaveHide() })
onMounted(() => { if (props.show) enterWaiting() })
onUnmounted(() => { dotsTimeline?.kill() })

// ══════════════════════════════════════════════════════════
// 计算停驻位和碰撞落点
// ══════════════════════════════════════════════════════════
function calcPositions() {
  const wrap = wrapRef.value
  const container = props.container
  const inputBar  = props.inputBar
  if (!wrap || !container) return { fallDist: 0 }

  const containerRect = container.getBoundingClientRect()

  // 停驻位：wrap 的 CSS top=50%，即容器高度一半
  // wrap 的自然 top in container = containerH/2 - pillH/2
  const pillH = wrap.offsetHeight   // 32px
  const anchorTopInContainer = container.clientHeight / 2 - pillH / 2

  // 碰撞落点：inputBar 顶部 in 容器坐标 = inputBar.getBoundingClientRect().top - containerRect.top
  // 落点的 top in container = ibTop - containerTop - pillH（按钮底部刚好碰到 ibTop）
  let collisionTopInContainer: number
  if (inputBar) {
    const ibTop = inputBar.getBoundingClientRect().top
    const contTop = containerRect.top
    // 落点使按钮底部对齐输入框顶部
    collisionTopInContainer = (ibTop - contTop) - pillH
  } else {
    collisionTopInContainer = container.clientHeight - pillH - 4
  }

  // GSAP translateY = 从停驻位到碰撞位需要移动的距离
  const fallDist = collisionTopInContainer - anchorTopInContainer
  return { fallDist: Math.max(fallDist, 0) }
}

// ══════════════════════════════════════════════════════════
// 状态1：等待态
// 停驻：右侧边框中央，从上方掉落入场
// ══════════════════════════════════════════════════════════
function enterWaiting() {
  const wrap = wrapRef.value
  const pill = pillRef.value
  if (!wrap || !pill) { visible.value = true; return }
  isAnimating = false
  visible.value = true

  gsap.set(pill, {
    scaleX: 1, scaleY: 1,
    borderRadius: '999px',
    backgroundColor: 'rgba(36,161,222,0.92)',
    boxShadow: '0 8px 24px -4px rgba(36,161,222,0.4)',
  })
  gsap.set(dotsRef.value,  { opacity: 1 })
  gsap.set(labelRef.value, { opacity: 0 })

  // 从上方 28px 掉落入场，elastic 弹跳
  gsap.fromTo(wrap,
    { y: -28, opacity: 0 },
    {
      y: 0, opacity: 1,
      duration: 0.52,
      ease: 'elastic.out(1, 0.55)',
      onComplete: startDots,
    }
  )
}

function startDots() {
  dotsTimeline?.kill()
  const dots = [dot0.value, dot1.value, dot2.value].filter(Boolean) as HTMLElement[]
  gsap.set(dots, { y: 0 })
  dotsTimeline = gsap.timeline({ repeat: -1, repeatDelay: 0.05 })
  dots.forEach((d, i) => {
    dotsTimeline!.to(d, {
      y: -6, duration: 0.30,
      ease: 'sine.inOut', yoyo: true, repeat: 1,
    }, i * 0.10)
  })
}

// ══════════════════════════════════════════════════════════
// 点击动画：聚合 → 史莱姆拉伸 → 内容态 → 下坠至输入框顶 → 碰撞
// ══════════════════════════════════════════════════════════
function handleClick() {
  if (isAnimating) return
  isAnimating = true
  dotsTimeline?.kill()

  const pill = pillRef.value
  const wrap = wrapRef.value
  if (!pill || !wrap) return

  // 在动画开始前计算落点（此时 wrap 还在停驻位）
  const { fallDist } = calcPositions()

  const tl = gsap.timeline({
    onComplete: () => {
      isAnimating = false
      emit('trigger')
      leaveHide()
    }
  })

  // ── 1. 三点淡出 ──────────────────────────────────────────
  tl.to(dotsRef.value, { opacity: 0, duration: 0.08, ease: 'power2.in' })

  // ── 2. 聚合：胶囊坍缩成圆球 ──────────────────────────────
  // pill width=88px, scaleX=0.364 ≈ 32px 正圆
  tl.to(pill, {
    scaleX: 0.364, scaleY: 1,
    borderRadius: '999px',
    duration: 0.20,
    ease: 'power3.in',
  })

  // ── 3. 史莱姆向右拉伸（水滴甩出感）─────────────────────────
  tl.to(pill, {
    scaleX: 1.45, scaleY: 0.68,
    duration: 0.13,
    ease: 'power2.out',
  })

  // ── 4. 内容态：圆角 → 4px 锐利矩形 + 文字淡入 ─────────────
  tl.to(pill, {
    scaleX: 1, scaleY: 1,
    borderRadius: '4px',
    duration: 0.24,
    ease: 'elastic.out(1.2, 0.45)',
  })
  tl.to(labelRef.value, {
    opacity: 1, duration: 0.14, ease: 'power1.out',
  }, '<0.06')

  // 停顿
  tl.to({}, { duration: 0.26 })

  // ── 5. 重力下坠：落到 inputBar 顶部 ──────────────────────
  tl.to(wrap, {
    y: `+=${fallDist}`,
    duration: 0.36,
    ease: 'power3.in',
  })

  // ── 6. 碰撞：Y 轴压扁（撞击输入框顶部边界）──────────────────
  tl.to(pill, {
    scaleX: 1.55, scaleY: 0.45,
    duration: 0.06,
    ease: 'power3.out',
  }, '<0.01')

  // elastic 弹起，肉感回弹
  tl.to(pill, {
    scaleX: 1, scaleY: 1,
    duration: 0.50,
    ease: 'elastic.out(1.1, 0.38)',
  })

  // 第1次竖向反弹 (-10px)
  tl.to(wrap, { y: `-=10`, duration: 0.12, ease: 'power2.out' }, '<0.01')
  tl.to(wrap, { y: `+=10`, duration: 0.10, ease: 'power2.in' })

  // 第2次微反弹 (-4px)
  tl.to(wrap, { y: `-=4`, duration: 0.08, ease: 'power2.out' })
  tl.to(wrap, { y: `+=4`, duration: 0.07, ease: 'power2.in' })

  // ── 7. 消失 ───────────────────────────────────────────────
  tl.to(wrap, {
    opacity: 0, scaleX: 0.5, scaleY: 0.5,
    duration: 0.15,
    ease: 'power2.in',
  })
}

function leaveHide() {
  dotsTimeline?.kill()
  if (wrapRef.value) gsap.set(wrapRef.value, { y: 0, opacity: 0, scale: 1 })
  visible.value = false
}
</script>

<style scoped>
.gravity-btn-wrap {
  /*
   * 停驻位：消息滚动容器右侧边框处，垂直居中
   * position: absolute 在 relative 的消息容器内
   * right: -16px 让按钮骑在右侧边框上（宽88px，一半在外）
   * top: 50% + translateY(-50%) 垂直居中
   * translateY 由 GSAP 控制，所以用 CSS var 处理基准位移
   */
  position: absolute;
  right: 8px;
  top: 50%;
  transform: translateY(-50%);
  z-index: 30;
  cursor: pointer;
  will-change: transform, opacity;
}

.gravity-pill {
  width: 88px;
  height: 32px;
  border-radius: 999px;
  background-color: rgba(36, 161, 222, 0.92);
  backdrop-filter: blur(8px);
  -webkit-backdrop-filter: blur(8px);
  display: flex;
  align-items: center;
  justify-content: center;
  gap: 5px;
  box-shadow:
    0 8px 24px -4px rgba(36, 161, 222, 0.40),
    0 2px 8px rgba(0, 0, 0, 0.10);
  will-change: transform, border-radius;
  transform-origin: center center;
  overflow: hidden;
  position: relative;
}

.gravity-dots {
  display: flex;
  align-items: center;
  gap: 5px;
  position: absolute;
  pointer-events: none;
}

.gravity-dot {
  display: block;
  width: 6px;
  height: 6px;
  border-radius: 50%;
  background: rgba(255, 255, 255, 0.9);
  will-change: transform;
}

.gravity-label {
  font-size: 11px;
  font-weight: 900;
  color: #fff;
  letter-spacing: 0.03em;
  white-space: nowrap;
  opacity: 0;
  position: absolute;
  pointer-events: none;
  will-change: opacity;
}
</style>
