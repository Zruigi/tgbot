import { defineStore } from 'pinia'
import { ref, shallowRef } from 'vue'

export type WsEventType =
  | 'status_update'
  | 'queue_update'
  | 'push_task_update'
  | 'broadcast_targets_sync'
  | 'broadcast_chat_result'
  | 'monitor_update'
  | 'potential_sync'
  | 'potential_promoted'
  | 'group_listen_update'
  | 'group_revoke_sync_update'
  | 'group_activity'
  | 'group_message_deleted'

export interface WsEvent {
  type: WsEventType
  data: Record<string, any>
}

// 修复 Medium: WebSocket 无心跳超时清理
// pong 超时阈值：超过 60s 未收到 pong 则主动断线重连
const PING_INTERVAL_MS = 25000
const PONG_TIMEOUT_MS = 60000

export const useWsStore = defineStore('ws', () => {
  const connected = ref(false)
  const lastEvent = shallowRef<WsEvent | null>(null)
  const lastEventByType = ref<Partial<Record<WsEventType, WsEvent>>>({})

  let ws: WebSocket | null = null
  let reconnectTimer: ReturnType<typeof setTimeout> | null = null
  let pingTimer: ReturnType<typeof setInterval> | null = null
  let pongTimeoutTimer: ReturnType<typeof setTimeout> | null = null
  let shouldReconnect = false

  function connect() {
    if (ws && (ws.readyState === WebSocket.OPEN || ws.readyState === WebSocket.CONNECTING)) return
    shouldReconnect = true

    const proto = location.protocol === 'https:' ? 'wss' : 'ws'
    // 从 sessionStorage 读取 token（配合 auth.ts 修复）
    const raw = sessionStorage.getItem('adm_s') || ''
    let token = ''
    try { token = raw ? decodeURIComponent(atob(raw)) : '' } catch { token = '' }
    const url = `${proto}://${location.host}/ws?token=${encodeURIComponent(token)}`

    ws = new WebSocket(url)

    ws.onopen = () => {
      connected.value = true
      console.log('[WS] Connected')
      _startPing()
    }

    ws.onmessage = (e) => {
      if (e.data === 'pong') {
        // 收到 pong，清除超时计时器
        _clearPongTimeout()
        return
      }
      try {
        const event = JSON.parse(e.data) as WsEvent
        lastEvent.value = event
        lastEventByType.value = { ...lastEventByType.value, [event.type]: event }
      } catch {
        // ignore malformed
      }
    }

    ws.onclose = () => {
      connected.value = false
      _clearPing()
      _clearPongTimeout()
      console.log('[WS] Disconnected')
      if (shouldReconnect) {
        reconnectTimer = setTimeout(() => connect(), 3000)
      }
    }

    ws.onerror = () => {
      ws?.close()
    }
  }

  function disconnect() {
    shouldReconnect = false
    _clearPing()
    _clearPongTimeout()
    if (reconnectTimer) { clearTimeout(reconnectTimer); reconnectTimer = null }
    ws?.close()
    ws = null
    connected.value = false
  }

  function _startPing() {
    _clearPing()
    pingTimer = setInterval(() => {
      if (ws && ws.readyState === WebSocket.OPEN) {
        ws.send('ping')
        // 发出 ping 后启动 pong 超时计时
        _clearPongTimeout()
        pongTimeoutTimer = setTimeout(() => {
          console.warn('[WS] Pong timeout — forcing reconnect')
          ws?.close()  // 触发 onclose → 自动重连
        }, PONG_TIMEOUT_MS)
      }
    }, PING_INTERVAL_MS)
  }

  function _clearPing() {
    if (pingTimer) { clearInterval(pingTimer); pingTimer = null }
  }

  function _clearPongTimeout() {
    if (pongTimeoutTimer) { clearTimeout(pongTimeoutTimer); pongTimeoutTimer = null }
  }

  return { connected, lastEvent, lastEventByType, connect, disconnect }
})
