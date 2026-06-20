<template>
  <div class="scope-bar flex items-center gap-3 px-4 py-2 bg-surface-container border-b border-outline-variant/20 text-sm">
    <!-- 身份显示 -->
    <div class="flex items-center gap-1.5">
      <span class="material-symbols-outlined text-base" :class="identityColor">{{ identityIcon }}</span>
      <span class="font-bold" :class="identityColor">{{ identityLabel }}</span>
    </div>

    <!-- 分隔线 -->
    <div class="w-px h-4 bg-outline-variant"></div>

    <!-- 数据范围 -->
    <div class="flex items-center gap-1.5 text-on-surface-variant">
      <span class="material-symbols-outlined text-base">dns</span>
      <span>{{ scopeLabel }}</span>
    </div>

    <!-- tenant_id / merchant_id 显示 -->
    <template v-if="auth.sessionContext?.tenantId">
      <div class="w-px h-4 bg-outline-variant"></div>
      <div class="flex items-center gap-1 text-on-surface-variant">
        <span class="font-mono text-xs bg-surface-container px-1.5 py-0.5 rounded">
          {{ auth.sessionContext.tenantId }}
        </span>
        <span v-if="auth.sessionContext?.merchantId" class="text-xs">
          #{{ auth.sessionContext.merchantId }}
        </span>
      </div>
    </template>

    <!-- 平台管理员代入商户视角时显示返回按钮 -->
    <template v-if="showReturnButton">
      <div class="flex-1"></div>
      <button
        @click="switchToPlatform"
        class="flex items-center gap-1 px-3 py-1 rounded-lg bg-surface-container-high text-xs font-bold hover:bg-primary/10 hover:text-primary transition-colors"
      >
        <span class="material-symbols-outlined text-base">arrow_back</span>
        返回平台视角
      </button>
    </template>

    <!-- 商户视角切换按钮 -->
    <template v-else-if="showSwitchToMerchant">
      <div class="flex-1"></div>
      <button
        @click="showMerchantSelector = true"
        class="flex items-center gap-1 px-3 py-1 rounded-lg bg-surface-container-high text-xs font-bold hover:bg-primary/10 hover:text-primary transition-colors"
      >
        <span class="material-symbols-outlined text-base">swap_horiz</span>
        代入商户视角
      </button>
    </template>

    <!-- 商户selector弹窗 -->
    <div v-if="showMerchantSelector" class="fixed inset-0 z-50 flex items-center justify-center bg-black/50" @click.self="showMerchantSelector = false">
      <div class="bg-surface-container-lowest rounded-2xl p-4 w-80 shadow-2xl">
        <h3 class="font-bold mb-3">选择商户视角</h3>
        <p class="text-xs text-on-surface-variant mb-3">选择要代入的商户，进入后可查看该商户数据</p>
        <input
          v-model="inputMerchantId"
          type="number"
          placeholder="输入 merchant_id"
          class="w-full px-3 py-2 rounded-xl bg-surface border border-outline-variant/20 text-sm mb-3"
        />
        <div class="flex gap-2">
          <button @click="showMerchantSelector = false" class="flex-1 px-3 py-2 rounded-xl bg-surface-container text-sm font-bold">取消</button>
          <button @click="switchToMerchant" class="flex-1 px-3 py-2 rounded-xl bg-primary text-on-primary text-sm font-bold">确认</button>
        </div>
      </div>
    </div>
  </div>
</template>

<script setup lang="ts">
import { computed, ref } from 'vue'
import { useAuthStore } from '@/stores/auth'

const auth = useAuthStore()
const showMerchantSelector = ref(false)
const inputMerchantId = ref<number | null>(null)

// 当前角色���否是平台管理员
const isPlatformAdmin = computed(() => {
  return auth.sessionContext?.role === 'platform_admin'
})

// 当前是否处于商户视角（平台管理员代入或商户本人）
const isMerchantScope = computed(() => {
  return auth.sessionContext?.role === 'merchant_owner'
})

// 是否显示返回平台按钮（平台管理员代入商户视角时）
const showReturnButton = computed(() => {
  return isPlatformAdmin.value && isMerchantScope.value && !!auth.sessionContext?.tenantId
})

// 是否显示切换到商户按钮（平台管理员且当前在平台视角时）
const showSwitchToMerchant = computed(() => {
  return isPlatformAdmin.value && !isMerchantScope.value
})

// 身份图标
const identityIcon = computed(() => {
  if (isPlatformAdmin.value && isMerchantScope.value) return 'person'
  if (isPlatformAdmin.value) return 'admin_panel_settings'
  return 'store'
})

// 身份标签颜色
const identityColor = computed(() => {
  if (isPlatformAdmin.value && isMerchantScope.value) return 'text-amber-500'
  if (isPlatformAdmin.value) return 'text-primary'
  return 'text-emerald-500'
})

// 身份标签文字
const identityLabel = computed(() => {
  if (isPlatformAdmin.value && isMerchantScope.value) return '平台管理员 (代入��户视角)'
  if (isPlatformAdmin.value) return '平台管理员'
  return '商户管理员'
})

// 数据范围标签
const scopeLabel = computed(() => {
  if (isMerchantScope.value) {
    return '当前商户数据'
  }
  return '平台全局数据'
})

async function switchToPlatform() {
  await auth.setSessionContext({
    role: 'platform_admin',
    tenantId: '',
    merchantId: null
  })
}

async function switchToMerchant() {
  if (!inputMerchantId.value) return
  showMerchantSelector.value = false
  // 这里需要先获取商户的 tenant_id，可以通过 merchant_id 查询
  // 暂时简单处理，实际需要调用 API 获取
  await auth.setSessionContext({
    role: 'merchant_owner',
    tenantId: '', // TODO: 通过 API 获取
    merchantId: inputMerchantId.value
  })
}
</script>

<style scoped>
.scope-bar {
  min-height: 40px;
}
</style>