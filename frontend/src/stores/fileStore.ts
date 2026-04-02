import { defineStore } from 'pinia'
import { ref } from 'vue'

/**
 * 全局文件/头像 URL 缓存 Store
 * 无论群聊、私聊、潜在用户还是群发，只要 file_id 相同，直接复用缓存。
 * 严禁使用 $set！使用展开运算符触发响应式更新。
 */
export const useFileStore = defineStore('file', () => {
  // 全局响应式缓存 file_id -> url
  const fileUrls = ref<Record<string, string>>({})
  // 正在加载中的 file_id，防止重复请求
  const loading = ref<Set<string>>(new Set())

const getToken = () => {
  const sessionRaw = sessionStorage.getItem('adm_s')
  if (sessionRaw) {
    try {
      return decodeURIComponent(atob(sessionRaw))
    } catch {
      return ''
    }
  }

  const localToken = localStorage.getItem('admin_token')
  if (localToken) return localToken

  const cookie = document.cookie
    .split('; ')
    .find((item) => item.startsWith('admin_token='))
  if (!cookie) return ''
  try {
    return decodeURIComponent(cookie.split('=').slice(1).join('='))
  } catch {
    return ''
  }
}

  /**
   * 智能加载：若缓存中已有则直接返回，否则请求 /api/file/{fileId}
   */
  async function autoLoadFile(fileId: string | null | undefined): Promise<string | null> {
    if (!fileId) return null
    if (fileUrls.value[fileId]) return fileUrls.value[fileId]
    if (loading.value.has(fileId)) return null

    loading.value.add(fileId)
    try {
      const res = await fetch(`/api/file/${fileId}`, {
        headers: { 'Authorization': `Bearer ${getToken()}` }
      })
      const data = await res.json()
      if (data.url) {
        // 使用展开运算符触发 Vue 3 响应式更新
        fileUrls.value = { ...fileUrls.value, [fileId]: data.url }
        return data.url
      }
    } catch {
      // 静默失败，不中断其他操作
    } finally {
      loading.value.delete(fileId)
    }
    return null
  }

  /**
   * 批量预加载一组 file_id（并发加载，跳过已有缓存）
   */
  async function preloadFiles(fileIds: (string | null | undefined)[]): Promise<void> {
    const unique = [...new Set(fileIds.filter((id): id is string => !!id && !fileUrls.value[id]))]
    if (unique.length === 0) return
    await Promise.allSettled(unique.map(id => autoLoadFile(id)))
  }

  return { fileUrls, autoLoadFile, preloadFiles }
})
