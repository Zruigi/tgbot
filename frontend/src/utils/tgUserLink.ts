import { openExternalLink } from '@/lib/telegram'

/**
 * Telegram 资料链接：有合法 username 时用 https://t.me/handle，否则 tg://user?id=
 * 仅当字符串符合 Telegram 用户名规则时才视为 handle，避免把昵称误当成用户名。
 */
function stripAt(s: string) {
  return s.trim().replace(/^@+/, '')
}

export function isLikelyTgUsername(raw?: string | null): boolean {
  if (!raw) return false
  const t = stripAt(String(raw))
  // 5–32，拉丁字母、数字、下划线，且至少含一个字母（排除纯数字误匹配）
  return /^[a-zA-Z0-9_]{5,32}$/.test(t) && /[a-zA-Z]/.test(t)
}

export function tgUserProfileHref(userId: number, usernameHint?: string | null): string {
  const t = usernameHint != null ? stripAt(String(usernameHint)) : ''
  if (t && isLikelyTgUsername(t)) {
    return `https://t.me/${encodeURIComponent(t)}?profile`
  }
  return `tg://user?id=${userId}`
}

export function openTgUserProfile(userId: number, usernameHint?: string | null): void {
  openExternalLink(tgUserProfileHref(userId, usernameHint))
}

/**
 * 超链展示文案：优先 @username，其次昵称/显示名，否则「用户 {id}」
 */
export function tgUserLinkLabel(
  userId: number,
  usernameHint?: string | null,
  firstNameHint?: string | null
): string {
  const u = usernameHint != null ? stripAt(String(usernameHint)) : ''
  if (u && isLikelyTgUsername(u)) return `@${u}`
  const display = (firstNameHint ?? '').trim() || (u || '').trim()
  if (display) return display
  return `用户 ${userId}`
}
