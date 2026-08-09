/**
 * 统一 API 入口
 *
 * 推荐使用专用的 API client:
 * - platformApi: 平台管理员专用接口 (/api/platform/*)
 * - merchantApi: 商户专用接口 (/api/merchant/*)
 * - sharedApi: 共享接口（聊天、群组、AI 等，数据按 scope 隔离）
 * - authApi: 认证与会话管理
 */
import { request } from './core'
import { platformApi } from './platform'
import { merchantApi } from './merchant'
import { sharedApi } from './shared'

// 认证相关 API
export const authApi = {
  login: (token: string) => request('POST', '/api/login', { token }),
  loginOtp: (otp: string) => request('POST', '/api/login/otp', { otp }),
  getSessionContext: () => request('GET', '/api/session/context'),
  updateSessionContext: (data: { role?: string; tenant_id?: string; merchant_id?: number | null }) =>
    request('POST', '/api/session/context', data),
}

// 配置相关 API
export const configApi = {
  getConfig: () => request('GET', '/api/config'),
  saveConfig: (data: any) => request('POST', '/api/config', data),
}

// 导出 request 供其他模块使用
export { request }

// 兼容旧用法：导出合并的 api 对象
export const api = {
  ...authApi,
  ...configApi,
  ...sharedApi,
  // 平台 API（带命名空间）
  getPlatformMerchants: platformApi.getMerchants,
  createPlatformMerchant: platformApi.createMerchant,
  confirmPlatformMerchantOnboarding: platformApi.confirmMerchantOnboarding,
  getPlatformPlans: platformApi.getPlans,
  upsertPlatformPlan: platformApi.upsertPlan,
  getPlatformSubscriptions: platformApi.getSubscriptions,
  getPlatformPaymentOrders: platformApi.getPaymentOrders,
  confirmPlatformPaymentOrder: platformApi.confirmPaymentOrder,
  rejectPlatformPaymentOrder: platformApi.rejectPaymentOrder,
  // 商户 API（带命名空间）
  getMerchantProfile: merchantApi.getProfile,
  saveMerchantBotToken: merchantApi.saveBotToken,
  updateMerchantStatus: merchantApi.updateStatus,
  getMerchantSubscription: merchantApi.getSubscription,
  getMerchantPlans: merchantApi.getPlans,
  createMerchantPaymentOrder: merchantApi.createPaymentOrder,
  getMerchantLatestPaymentOrder: merchantApi.getLatestPaymentOrder,
  markMerchantPaymentOrderPaid: merchantApi.markPaymentOrderPaid,
  submitMerchantPaymentTx: merchantApi.submitPaymentTx,
  getMerchantFeatures: merchantApi.getFeatures,
  getMerchantAiQuota: merchantApi.getAiQuota,
  getMerchantAiUsage: merchantApi.getAiUsage,
  getMerchantAuditLogs: merchantApi.getAuditLogs,
}

// 导出专用 API 对象供新代码使用
export { platformApi, merchantApi, sharedApi }