import { defineStore } from 'pinia'
import { ref } from 'vue'
import { api } from '@/api'

export interface Group {
  group_id: number
  group_title: string
  is_monitored: number
  updated_at: string
  created_at: string
  custom_keywords: string
  verify_enabled: number
  verify_type: string
  is_online: boolean
}

export interface VerifiedUser {
  user_id: number
  username: string
  first_name: string
  avatar_file_id: string | null
  verified_at: string
}

export const useGroupStore = defineStore('group', () => {
  const groups = ref<Group[]>([])
  const isLoading = ref(false)
  const error = ref('')
  const total = ref(0)
  const page = ref(1)
  const pageSize = 20
  const verifiedUsersList = ref<VerifiedUser[]>([])
  const isLoadingVerifiedUsers = ref(false)

  async function fetchGroupList(p = 1) {
    isLoading.value = true
    error.value = ''
    try {
      const res = await api.getGroups(p, pageSize)
      if (res.status === 'ok') {
        groups.value = res.data || []
        total.value = res.total || res.data?.length || 0
        page.value = p
      } else {
        error.value = res.message || '获取群组列表失败'
      }
    } catch (e: any) {
      error.value = e.message || '网络错误'
    } finally {
      isLoading.value = false
    }
  }

  async function fetchVerifiedUsers(groupId?: number, period: 'today' | 'all' = 'all') {
    isLoadingVerifiedUsers.value = true
    try {
      let url = `/api/group_listen/verified_users?period=${period}`
      if (groupId) {
        url += `&group_id=${groupId}`
      }
      const res = await fetch(url, {
        headers: { 'Authorization': `Bearer ${localStorage.getItem('admin_token') || ''}` }
      })
      const data = await res.json()
      if (data.status === 'ok') {
        verifiedUsersList.value = data.data || []
        console.log(`已加载 ${verifiedUsersList.value.length} 个已验证用户`)
      } else {
        console.error('API 返回错误:', data.message)
      }
    } catch (e: any) {
      console.error('获取已验证用户失败:', e)
    } finally {
      isLoadingVerifiedUsers.value = false
    }
  }

  return { 
    groups, 
    isLoading, 
    error, 
    total, 
    page, 
    pageSize, 
    fetchGroupList,
    verifiedUsersList,
    isLoadingVerifiedUsers,
    fetchVerifiedUsers
  }
})
