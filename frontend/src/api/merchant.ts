/**
 * 商户专用 API
 * 仅商户可调用的接口
 */
import { request } from './core'

export const merchantApi = {
  // 商户资料
  getProfile: () => request('GET', '/api/merchant/profile'),

  // Bot Token 管理
  saveBotToken: (data: { bot_token: string }) => request('POST', '/api/merchant/bot_token', data),

  // 状态管理
  updateStatus: (data: { status: string; onboarding_completed?: boolean }) => request('POST', '/api/merchant/status', data),

  // 订阅与套餐
  getSubscription: () => request('GET', '/api/merchant/subscription'),
  getPlans: () => request('GET', '/api/merchant/plans'),

  // 支付订单
  createPaymentOrder: (data: { plan_code: string }) => request('POST', '/api/merchant/payment_orders', data),
  getLatestPaymentOrder: () => request('GET', '/api/merchant/payment_orders/latest'),
  markPaymentOrderPaid: (order_id: number) => request('POST', '/api/merchant/payment_orders/mark_paid', { order_id }),
  submitPaymentTx: (data: { order_id: number; tx_hash: string }) => request('POST', '/api/merchant/payment_orders/submit_tx', data),

  // 功能开关
  getFeatures: () => request('GET', '/api/merchant/features'),

  // AI 配额与用量
  getAiQuota: () => request('GET', '/api/merchant/ai/quota'),
  getAiUsage: (days = 7) => request('GET', `/api/merchant/ai/usage?days=${days}`),

  // 审计日志
  getAuditLogs: (limit = 50) => request('GET', `/api/merchant/audit_logs?limit=${limit}`),

  // 商户配置
  getPaymentConfig: () => request('GET', '/api/merchant/payment_config'),
}