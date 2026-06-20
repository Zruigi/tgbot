/**
 * 平台专用 API
 * 仅平台管理员可调用的接口
 */
import { request } from './core'

export const platformApi = {
  // 商户管理
  getMerchants: (params: { status?: string; limit?: number; offset?: number } = {}) => {
    const q = new URLSearchParams()
    if (params.status) q.set('status', params.status)
    if (typeof params.limit === 'number') q.set('limit', String(params.limit))
    if (typeof params.offset === 'number') q.set('offset', String(params.offset))
    return request('GET', `/api/platform/merchants${q.toString() ? `?${q.toString()}` : ''}`)
  },
  createMerchant: (data: { owner_tg_user_id: number; merchant_name: string; tenant_id?: string; bot_token?: string }) =>
    request('POST', '/api/platform/merchants', data),
  confirmMerchantOnboarding: (merchant_id: number, note?: string) =>
    request('POST', '/api/platform/merchants/confirm_onboarding', { merchant_id, note }),

  // 套餐管理
  getPlans: () => request('GET', '/api/platform/plans'),
  upsertPlan: (data: { plan_code: string; plan_name: string; price_usdt: number; duration_days: number; features_json: string; status?: string }) =>
    request('POST', '/api/platform/plans', data),

  // 订阅管理
  getSubscriptions: (params: { status?: string; merchant_id?: number; limit?: number } = {}) => {
    const q = new URLSearchParams()
    if (params.status) q.set('status', params.status)
    if (typeof params.merchant_id === 'number') q.set('merchant_id', String(params.merchant_id))
    if (typeof params.limit === 'number') q.set('limit', String(params.limit))
    return request('GET', `/api/platform/subscriptions${q.toString() ? `?${q.toString()}` : ''}`)
  },

  // 支付订单
  getPaymentOrders: (params: { status?: string; merchant_id?: number; limit?: number } = {}) => {
    const q = new URLSearchParams()
    if (params.status) q.set('status', params.status)
    if (typeof params.merchant_id === 'number') q.set('merchant_id', String(params.merchant_id))
    if (typeof params.limit === 'number') q.set('limit', String(params.limit))
    return request('GET', `/api/platform/payment_orders${q.toString() ? `?${q.toString()}` : ''}`)
  },
  confirmPaymentOrder: (order_id: number) => request('POST', '/api/platform/payment_orders/confirm', { order_id }),
  rejectPaymentOrder: (order_id: number) => request('POST', '/api/platform/payment_orders/reject', { order_id }),

  // 平台配置
  getPlatformPaymentConfig: () => request('GET', '/api/platform/payment_config'),
  savePlatformPaymentConfig: (data: any) => request('POST', '/api/platform/payment_config', data),
}