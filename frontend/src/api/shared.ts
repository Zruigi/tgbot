/**
 * 共享 API（无作用域歧义）
 * 商户和平台管理员都可以调用，但各自看到自己的数据
 */
import { request } from './core'

export const sharedApi = {
  // 用户聊天
  getUsers: () => request('GET', '/api/users'),
  getMessages: (userId: number, page = 1) => request('GET', `/api/messages/${userId}?page=${page}`),
  sendMessage: (userId: number, data: any) => request('POST', `/api/messages/${userId}`, data),

  // 群组
  getGroups: (page = 1, size = 20) => request('GET', `/api/group_listen/list?page=${page}&size=${size}`),
  toggleGroupMonitor: (groupId: number, enabled: boolean) => request('POST', '/api/group_listen/toggle', { group_id: groupId, enabled }),
  getGroupMembers: (groupId: number) => request('GET', `/api/group_members?group_id=${groupId}`),
  getGroupMessages: (groupId: number, page = 1) => request('GET', `/api/group_messages?group_id=${groupId}&page=${page}`),

  // 监控
  getMonitors: () => request('GET', '/api/monitors'),

  // 潜在用户
  getPotentialUsers: (offset = 0, limit = 20) => request('GET', `/api/potential_users?offset=${offset}&limit=${limit}`),
  getAvailableTags: () => request('GET', '/api/potential_users/tags'),

  // AI 相关
  getOpenaiModels: async () => {
    try {
      return await request('GET', '/api/openai/models')
    } catch {
      return await request('GET', '/v1/models')
    }
  },
  execOpenai: async (data: { model: string; input: string; sandbox_id?: string; user_id?: number; feature_key?: string }) => {
    try {
      return await request('POST', '/api/openai/exec', data)
    } catch {
      return await request('POST', '/v1/exec', data)
    }
  },
  getAiSkills: (includeBuiltin = false) => request('GET', `/api/ai/skills${includeBuiltin ? '?include_builtin=1' : ''}`),
  upsertAiSkill: (data: { name: string; description?: string; prompt_template?: string; source?: string; is_enabled?: boolean }) =>
    request('POST', '/api/ai/skills', data),
  toggleAiSkill: (name: string, isEnabled: boolean) => request('POST', '/api/ai/skills/toggle', { name, is_enabled: isEnabled }),
  importBuiltinAiSkills: () => request('POST', '/api/ai/skills/import_builtin'),
  getDiscoveredAiSkills: () => request('GET', '/api/ai/skills/discovered'),
}