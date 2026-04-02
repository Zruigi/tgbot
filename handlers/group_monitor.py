# -*- coding: utf-8 -*-
"""
群组监听模块 - 自动捕捉潜在用户，并写入 [用户标签画像] 便于 Web 端筛选。
扩展：在 KEYWORD_PROFILE_TAGS 中增加「关键词 → 画像标签」映射即可扩展分类。
"""
import logging
import re
import httpx
from datetime import datetime, timedelta
from typing import Optional
from html import escape
from telegram import Update
from telegram.ext import ContextTypes, MessageHandler, filters
import config
import database

# 头像缓存：user_id -> avatar_file_id，避免重复调用 get_chat
_avatar_cache: dict = {}
from services.security_shield import ADDR_REGEX

logger = logging.getLogger(__name__)

# --- 2026 规格：群组监听配置缓存 ---
# 缓存格式：{group_id: {"is_monitored": bool, "custom_keywords": list}}
_group_config_cache = {}

# 默认画像标签引擎配置 (支持正则管道符 |)
KEYWORD_PROFILE_TAGS = {
    "加入|申请|想进": "拉新意向",
    "邀请|拉我": "拉新意向",
    "合作|合伙|商务": "合作意向",
    "推荐|分享": "推荐",
    "招聘|找人|兼职": "招聘",
    "项目|开发|进度": "项目相关",
    "融资|资金|天使轮": "投融资",
    "投资|理财|赚钱": "投融资",
}

# 预编译正则缓存
_compiled_profile_patterns = {
    re.compile(k, re.IGNORECASE): v for k, v in KEYWORD_PROFILE_TAGS.items()
}

DEFAULT_KEYWORDS = []
for k in KEYWORD_PROFILE_TAGS.keys():
    DEFAULT_KEYWORDS.extend(k.split('|'))


async def _get_group_config(group_id: int) -> dict:
    """获取群组配置（带缓存逻辑）"""
    if group_id in _group_config_cache:
        return _group_config_cache[group_id]
    
    try:
        # 从数据库加载配置
        async with database.aiosqlite.connect(database.DB_NAME) as db:
            db.row_factory = database.aiosqlite.Row
            async with db.execute("SELECT is_monitored, custom_keywords FROM tracked_groups WHERE group_id = ?", (group_id,)) as cursor:
                row = await cursor.fetchone()
                if row:
                    custom_kws = []
                    if row['custom_keywords']:
                        custom_kws = [k.strip() for k in row['custom_keywords'].split(',') if k.strip()]
                    
                    config_data = {
                        "is_monitored": bool(row['is_monitored']),
                        "custom_keywords": custom_kws
                    }
                    _group_config_cache[group_id] = config_data
                    return config_data
    except Exception as e:
        logger.error("加载群组配置失败: %s", e)
    
    return {"is_monitored": False, "custom_keywords": []}


def clear_group_cache(group_id: Optional[int] = None):
    """清除缓存 (Web 端更新配置后调用)"""
    if group_id:
        _group_config_cache.pop(group_id, None)
    else:
        _group_config_cache.clear()


def _guess_network_by_address(address: str) -> Optional[str]:
    if address.startswith('T') and len(address) == 34:
        return 'trc20'
    if address.startswith('0x') and len(address) == 42:
        return 'bsc'
    return None


async def _query_address_snapshot(address: str, network: str) -> dict:
    result = {
        "address": address,
        "network": network.upper(),
        "tx_count_24h": 0,
        "inflow_24h": 0.0,
        "outflow_24h": 0.0,
        "latest_tx": None,
        # enhanced fields
        "account_type": None,
        "created_at": None,
        "last_active": None,
        "trx_balance": None,
        "usdt_balance": None,
        "energy_used": None,
        "energy_limit": None,
        "bandwidth_used": None,
        "bandwidth_limit": None,
        "free_bandwidth_used": None,
        "free_bandwidth_limit": None,
        "total_tx_in": None,
        "total_tx_out": None,
        "recent_txs": [],
        "permissions": [],
    }
    now_ms = int(datetime.now().timestamp() * 1000)
    since_ms = now_ms - 24 * 60 * 60 * 1000

    async with httpx.AsyncClient(timeout=15.0) as client:
        if network == 'trc20':
            headers = {"TRON-PRO-API-KEY": config.TRONGRID_API_KEY}

            # 1. Account info
            try:
                acc_resp = await client.get(f"https://api.trongrid.io/v1/accounts/{address}", headers=headers)
                acc_data = acc_resp.json()
                acc = (acc_data.get('data') or [{}])[0]
                if acc:
                    result['account_type'] = '普通账户' if not acc.get('account_type') else str(acc['account_type'])
                    ct = acc.get('create_time')
                    if ct:
                        result['created_at'] = datetime.fromtimestamp(int(ct)/1000).strftime('%Y-%m-%d %H:%M:%S')
                    lt = acc.get('latest_opration_time') or acc.get('latest_operation_time')
                    if lt:
                        result['last_active'] = datetime.fromtimestamp(int(lt)/1000).strftime('%Y-%m-%d %H:%M:%S')
                    result['trx_balance'] = round(int(acc.get('balance') or 0) / 1e6, 2)
                    # USDT TRC20 balance
                    for tok in (acc.get('trc20') or []):
                        usdt = tok.get('TR7NHqjeKQxGTCi8q8ZY4pL8otSzgjLj6t')
                        if usdt is not None:
                            result['usdt_balance'] = round(int(usdt) / 1e6, 2)
                    # Energy
                    res_info = acc.get('account_resource') or {}
                    result['energy_used'] = res_info.get('energy_usage', 0)
                    result['energy_limit'] = res_info.get('energy_limit', 0)
                    result['bandwidth_used'] = acc.get('net_usage', 0)
                    result['bandwidth_limit'] = acc.get('net_limit', 0)
                    result['free_bandwidth_used'] = acc.get('free_net_usage', 0)
                    result['free_bandwidth_limit'] = acc.get('free_net_limit', 600)
                    # Permissions
                    for perm in (acc.get('owner_permission', {}).get('keys') or []):
                        result['permissions'].append({'type': 'owner', 'address': perm.get('address'), 'weight': perm.get('weight')})
                    for perm in (acc.get('active_permission') or []):
                        for key in (perm.get('keys') or []):
                            result['permissions'].append({'type': 'active', 'address': key.get('address'), 'weight': key.get('weight')})
            except Exception as e:
                logger.debug(f'TRON account info error: {e}')

            # 2. TRC20 transactions (24h stats + recent)
            try:
                url = f"https://api.trongrid.io/v1/accounts/{address}/transactions/trc20?limit=50&order_by=block_timestamp,desc"
                resp = await client.get(url, headers=headers)
                rows = resp.json().get('data') or []
                recent = []
                for tx in rows:
                    if tx.get('token_info', {}).get('address') != 'TR7NHqjeKQxGTCi8q8ZY4pL8otSzgjLj6t':
                        continue
                    ts = int(tx.get('block_timestamp') or 0)
                    val = round(float(tx.get('value') or 0) / 1e6, 2)
                    direction = '转入' if str(tx.get('to') or '') == address else '转出'
                    tx_time = datetime.fromtimestamp(ts/1000).strftime('%Y-%m-%d %H:%M:%S') if ts else ''
                    if len(recent) < 10:
                        recent.append({'time': tx_time, 'direction': direction, 'amount': val})
                    if ts >= since_ms:
                        if direction == '转入':
                            result['inflow_24h'] += val
                        else:
                            result['outflow_24h'] += val
                        result['tx_count_24h'] += 1
                    if not result['latest_tx']:
                        result['latest_tx'] = tx.get('transaction_id')
                result['recent_txs'] = recent
            except Exception as e:
                logger.debug(f'TRON tx error: {e}')

            # 3. Total tx counts
            try:
                in_resp = await client.get(f"https://api.trongrid.io/v1/accounts/{address}/transactions/trc20?limit=1&direction=to", headers=headers)
                result['total_tx_in'] = in_resp.json().get('meta', {}).get('page_size')
                out_resp = await client.get(f"https://api.trongrid.io/v1/accounts/{address}/transactions/trc20?limit=1&direction=from", headers=headers)
                result['total_tx_out'] = out_resp.json().get('meta', {}).get('page_size')
            except Exception:
                pass

        else:  # bsc
            try:
                url = (
                    f"https://api.bscscan.com/api?module=account&action=tokentx"
                    f"&contractaddress=0x55d398326f99059fF775485246999027B3197955"
                    f"&address={address}&sort=desc&apikey={config.BSCSCAN_API_KEY}"
                )
                resp = await client.get(url)
                rows = resp.json().get('result') or []
                recent = []
                for tx in rows:
                    ts = int(tx.get('timeStamp') or 0) * 1000
                    val = round(float(tx.get('value') or 0) / 1e18, 2)
                    direction = '转入' if str(tx.get('to') or '').lower() == address.lower() else '转出'
                    tx_time = datetime.fromtimestamp(ts/1000).strftime('%Y-%m-%d %H:%M:%S') if ts else ''
                    if len(recent) < 5:
                        recent.append({'time': tx_time, 'direction': direction, 'amount': val})
                    if ts >= since_ms:
                        if direction == '转入':
                            result['inflow_24h'] += val
                        else:
                            result['outflow_24h'] += val
                        result['tx_count_24h'] += 1
                    if not result['latest_tx']:
                        result['latest_tx'] = tx.get('hash')
                result['recent_txs'] = recent
                result['total_tx_out'] = sum(1 for r in rows if str(r.get('from') or '').lower() == address.lower())
                result['total_tx_in'] = sum(1 for r in rows if str(r.get('to') or '').lower() == address.lower())
            except Exception as e:
                logger.debug(f'BSC tx error: {e}')

    result['inflow_24h'] = round(result['inflow_24h'], 2)
    result['outflow_24h'] = round(result['outflow_24h'], 2)
    return result


async def _handle_group_address_auto_query(update: Update, context: ContextTypes.DEFAULT_TYPE) -> None:
    message = update.effective_message
    if not message:
        return
    text = message.text or message.caption or ''
    if not text:
        return

    raw_addresses = re.findall(ADDR_REGEX, text)
    if not raw_addresses:
        return

    group_id = message.chat.id
    user_id = message.from_user.id if message.from_user else 0

    for addr in raw_addresses[:3]:
        network = _guess_network_by_address(addr)
        if network not in ('trc20', 'bsc'):
            continue

        prev_hit = await database.get_group_address_hit(group_id, addr)
        if prev_hit and prev_hit.get('created_at'):
            try:
                prev_at = datetime.fromisoformat(str(prev_hit['created_at']).replace(' ', 'T'))
                if datetime.now() - prev_at < timedelta(minutes=30):
                    continue
            except Exception:
                pass

        try:
            snapshot = await _query_address_snapshot(addr, network)
            await database.upsert_group_address_hit(
                group_id=group_id,
                user_id=user_id,
                address=addr,
                network=network,
                source_tg_msg_id=message.message_id,
                query_result=snapshot,
            )

            tip_lines = [
                f"👤账户类型: {snapshot.get('account_type') or '未知'}",
                f"🔍查询地址: <code>{addr}</code>",
            ]
            if snapshot.get('created_at'):
                tip_lines.append(f"⏰创建时间: {snapshot['created_at']}")
            if snapshot.get('last_active'):
                tip_lines.append(f"🌟最后活跃: {snapshot['last_active']}")
            tip_lines.append("➖➖➖➖资源➖➖➖➖")
            if snapshot.get('trx_balance') is not None:
                tip_lines.append(f"💰 TRX  余额：  {snapshot['trx_balance']} TRX")
            tip_lines.append(f"💰USDT余额： {snapshot.get('usdt_balance') or '0.00'} USDT")
            eu = snapshot.get('energy_used', 0); el = snapshot.get('energy_limit', 0)
            tip_lines.append(f"🔋能量： {eu} / {el}")
            bu = snapshot.get('bandwidth_used', 0); bl = snapshot.get('bandwidth_limit', 0)
            tip_lines.append(f"📡质押带宽： {bu} / {bl}")
            fbu = snapshot.get('free_bandwidth_used', 0); fbl = snapshot.get('free_bandwidth_limit', 600)
            tip_lines.append(f"📡免费带宽： {fbu} / {fbl}")
            perms = snapshot.get('permissions') or []
            if perms:
                tip_lines.append("➖➖➖➖权限➖➖➖➖")
                owner_perms = [p for p in perms if p['type'] == 'owner']
                active_perms = [p for p in perms if p['type'] == 'active']
                if owner_perms:
                    tip_lines.append("拥有者 (Owner) 权限")
                    for p in owner_perms:
                        tip_lines.append(f"<code>{p['address']}</code>(权重: {p['weight']})")
                if active_perms:
                    tip_lines.append("活跃 (Active) 权限")
                    for p in active_perms:
                        tip_lines.append(f"<code>{p['address']}</code>(权重: {p['weight']})")
            tip_lines.append("➖➖➖➖最近交易➖➖➖➖")
            ti = snapshot.get('total_tx_in'); to_ = snapshot.get('total_tx_out')
            if ti is not None and to_ is not None:
                tip_lines.append(f"⤴️USDT支出笔数：{to_}    ⤵️USDT收入笔数：{ti}")
            else:
                tip_lines.append(f"24h 笔数: <b>{snapshot['tx_count_24h']}</b>")
                tip_lines.append(f"24h 转入: <b>{snapshot['inflow_24h']:.2f} USDT</b>")
                tip_lines.append(f"24h 转出: <b>{snapshot['outflow_24h']:.2f} USDT</b>")
            for tx in (snapshot.get('recent_txs') or []):
                tip_lines.append(f"{tx['time']} {tx['direction']} {tx['amount']} USDT")
            tip = '\n'.join(tip_lines)
            await message.reply_html(tip)

            try:
                from web_admin import notify_web_clients
                await notify_web_clients("group_address_hit", {
                    "group_id": group_id,
                    "address": addr,
                    "network": snapshot['network'],
                    "snapshot": snapshot,
                })
            except Exception:
                pass
        except Exception as ex:
            logger.warning("群内地址自动查询失败: %s", ex)



def _profile_tags_from_keywords(matched_keywords: list, text: str) -> str:
    """根据匹配到的关键词和全文内容生成标签 (去重合并)"""
    tags = set()
    # 1. 简单命中关键词映射
    for kw in matched_keywords:
        for pattern_str, tag in KEYWORD_PROFILE_TAGS.items():
            if kw in pattern_str.split('|'):
                tags.add(tag)
    
    # 2. 增强正则匹配
    for pattern, tag in _compiled_profile_patterns.items():
        if pattern.search(text):
            tags.add(tag)
            
    return ",".join(sorted(list(tags)))


async def track_group_seen(update: Update, context: ContextTypes.DEFAULT_TYPE) -> None:
    """所有群组消息的前置处理器：自动登记/更新群组信息"""
    message = update.effective_message
    if not message or not message.chat or message.chat.type not in ["group", "supergroup"]:
        return
        
    chat = message.chat
    # 幂等写入/更新
    try:
        await database.upsert_tracked_group(chat.id, chat.title or "")
        await database.upsert_broadcast_target(
            chat_id=chat.id,
            chat_title=chat.title or "未命名群组",
            chat_type=chat.type,
            chat_username=getattr(chat, "username", None),
            can_broadcast=True,
        )
        
        # 2026 规格：如果群组已开启监听，则记录消息历史（用于“爬取最近50条消息”类似需求）
        group_cfg = await _get_group_config(chat.id)
        if group_cfg["is_monitored"]:
            msg_type = 'text'
            media_file_id = None
            msg_content = message.text or message.caption or ''

            if message.photo:
                msg_type = 'photo'
                media_file_id = message.photo[-1].file_id
                if not msg_content:
                    msg_content = '[图片]'
            elif message.animation:
                msg_type = 'animation'
                media_file_id = message.animation.file_id
                if not msg_content:
                    msg_content = '[动图]'
            elif message.sticker:
                msg_type = 'sticker'
                media_file_id = message.sticker.file_id
                if not msg_content:
                    msg_content = f"[表情包] {message.sticker.emoji or ''}".strip()
            elif message.video:
                msg_type = 'video'
                media_file_id = message.video.file_id
                if not msg_content:
                    msg_content = '[视频]'
            elif message.voice:
                msg_type = 'voice'
                media_file_id = message.voice.file_id
                if not msg_content:
                    msg_content = '[语音]'
            elif message.audio:
                msg_type = 'audio'
                media_file_id = message.audio.file_id
                if not msg_content:
                    msg_content = '[音频]'
            elif message.document:
                msg_type = 'document'
                media_file_id = message.document.file_id
                if not msg_content:
                    msg_content = f"[文件] {message.document.file_name or ''}".strip()
            elif not msg_content:
                msg_type = 'media'
                msg_content = f"[{message.effective_attachment.__class__.__name__ if message.effective_attachment else '媒体'}]"

            sender_type = 'user'
            if message.sender_chat:
                sender_type = 'channel'
            elif message.from_user and str(message.from_user.id) == str(config.ADMIN_ID):
                sender_type = 'admin'
            elif message.from_user and message.from_user.is_bot:
                sender_type = 'bot'

            quoted_tg_msg_id = message.reply_to_message.message_id if message.reply_to_message else None
            quoted_text = None
            if message.reply_to_message:
                quoted_text = message.reply_to_message.text or message.reply_to_message.caption
                if not quoted_text:
                    quoted_text = '[媒体消息]'

            # 获取头像（带缓存，每用户只调一次 get_chat）
            _avatar = None
            if message.from_user:
                _uid = message.from_user.id
                if _uid not in _avatar_cache:
                    try:
                        _chat_info = await context.bot.get_chat(_uid)
                        _photo = getattr(_chat_info, 'photo', None)
                        _avatar_cache[_uid] = _photo.small_file_id if _photo else ''
                    except Exception:
                        _avatar_cache[_uid] = ''
                _avatar = _avatar_cache[_uid] or None

            await database.save_group_message(
                group_id=chat.id,
                user_id=message.from_user.id if message.from_user else 0,
                username=message.from_user.username if message.from_user else "System",
                content=msg_content,
                msg_type=msg_type,
                file_id=media_file_id,
                tg_msg_id=message.message_id,
                sender_type=sender_type,
                quoted_tg_msg_id=quoted_tg_msg_id,
                quoted_text=quoted_text,
                message_ts=int(message.date.timestamp()) if message.date else None,
                avatar_file_id=_avatar,
            )

        # 始终把发言人写入 group_members（即使未开启监听，也可用于“成员管理”弹窗展示）
        if message.from_user and not message.from_user.is_bot:
            await database.save_group_member(
                user_id=message.from_user.id,
                username=message.from_user.username or f"User{message.from_user.id}",
                first_name=message.from_user.first_name,
                last_name=message.from_user.last_name,
                group_id=chat.id,
                group_title=chat.title,
            )
            # 如果拿到了头像，同步更新 admin_inbox（用户私聊头像）
            if _avatar:
                await database.update_user_avatar(message.from_user.id, _avatar)

        # ===== 频道关注验证拦截 =====
        if message.from_user and not message.from_user.is_bot:
            await _check_channel_verify(update, context, chat.id, message)

        # P2-1：群内地址自动识别与一次性查询（含同地址冷却去重）
        await _handle_group_address_auto_query(update, context)
            
        logger.debug(f"📊 [活跃监测] 群组={chat.title}({chat.id}) 类型={'文本' if message.text else '媒体/表情'}")
    except Exception as e:
        logger.debug("track_group_seen: %s", e)

    # 2026 规格：通过 WebSocket 通知前端群组活跃状态
    try:
        from web_admin import notify_web_clients
        await notify_web_clients("group_activity", {
            "group_id": chat.id,
            "group_title": chat.title,
            "timestamp": message.date.isoformat()
        })
    except Exception as ws_ex:
        logger.debug("WS 活动通知失败: %s", ws_ex)


async def track_channel_seen(update: Update, context: ContextTypes.DEFAULT_TYPE) -> None:
    """记录机器人可见的频道，用于广播目标选择"""
    message = update.effective_message
    if not message or not message.chat or message.chat.type != "channel":
        return

    chat = message.chat
    try:
        await database.upsert_broadcast_target(
            chat_id=chat.id,
            chat_title=chat.title or "未命名频道",
            chat_type="channel",
            chat_username=getattr(chat, "username", None),
            can_broadcast=True,
        )
    except Exception as e:
        logger.debug("track_channel_seen: %s", e)


async def monitor_group_keywords(update: Update, context: ContextTypes.DEFAULT_TYPE) -> None:
    """
    2026 增强版：群组关键词监控
    支持：全局关键词 + 群组自定义关键词、正则匹配优化、作用域缓存、防洪策略。
    """
    message = update.message
    if not message or not message.text or not message.from_user:
        return

    user = message.from_user
    chat = message.chat
    
    # 忽略管理员本人的消息
    if str(user.id) == str(config.ADMIN_ID):
        return

    # 1. 自动登记群组 (确保库里有这个群)
    # 此处已由 track_group_seen 处理，但作为防抖逻辑保留
    # await database.upsert_tracked_group(chat.id, chat.title)

    # 2. 检查监听状态 (使用缓存优化性能)
    group_cfg = await _get_group_config(chat.id)
    if not group_cfg["is_monitored"]:
        return

    # 3. 构造匹配池：全局 + 自定义
    keywords_pool = set(DEFAULT_KEYWORDS + group_cfg["custom_keywords"])
    
    text_content = message.text
    matched_keywords = []
    
    # 2026 规格：增强匹配算法 (支持正则与边界检查)
    for kw in keywords_pool:
        # 简单匹配，对于自定义关键词很有用
        if kw.lower() in text_content.lower():
            matched_keywords.append(kw)

    # 4. 基于标签引擎的画像提取
    profile_tags = _profile_tags_from_keywords(matched_keywords, text_content)

    if not profile_tags and not matched_keywords:
        return

    try:
        # 5. 存入潜在用户库 (database.py 已处理累积逻辑)
        await database.upsert_potential_user(
            user_id=user.id,
            username=user.username or f"User{user.id}",
            first_name=user.first_name,
            last_name=user.last_name,
            message_text=message.text,
            keywords=",".join(matched_keywords),
            profile_tags=profile_tags,
            group_id=chat.id,
            group_title=chat.title,
        )

        logger.info(
            "🎯 [群组监控] 命中! 用户=%s(@%s) 关键词=%s 群组=%s",
            user.id, user.username, matched_keywords, chat.title
        )

        # 6. WebSocket 实时推送
        try:
            from web_admin import notify_web_clients
            await notify_web_clients("potential_sync", {
                "user_id": user.id,
                "username": user.username,
                "keywords": matched_keywords,
                "group_title": chat.title,
                "profile_tags": profile_tags
            })
        except Exception as ws_ex:
            logger.debug("WS 推送失败: %s", ws_ex)

        # 7. 自动私聊邀请 (带防洪逻辑：同一用户 24 小时内只发一次)
        # 逻辑预留：后续可添加 database.check_last_invite_time
        try:
            invite_text = (
                f"👋 <b>你好 {escape(user.first_name or '朋友')}！</b>\n\n"
                f"我们在群组 <b>{escape(chat.title)}</b> 注意到你提到了关于 <code>{matched_keywords[0]}</code> 的内容。\n\n"
                "欢迎直接咨询我们的官方客服，我们将为您提供 1对1 的专业解答和支持！"
            )
            await context.bot.send_message(
                chat_id=user.id,
                text=invite_text,
                parse_mode="HTML",
            )
        except Exception as invite_err:
            # 用户可能设置了禁止私聊或未开始机器人
            logger.warning("自动邀请发送失败 (用户未开启机器人或隐私限制): %s", invite_err)

    except Exception as e:
        logger.error("处理潜在用户流程异常: %s", e)


async def handle_new_group_member(update: Update, context: ContextTypes.DEFAULT_TYPE) -> None:
    """处理新成员加入群组"""
    message = update.message
    if not message or not message.new_chat_members:
        return

    for member in message.new_chat_members:
        try:
            await database.save_group_member(
                user_id=member.id,
                username=member.username or f"User{member.id}",
                first_name=member.first_name,
                last_name=member.last_name,
                group_id=message.chat.id,
                group_title=message.chat.title,
            )

            logger.info("✅ 新成员加入: %s", member.username or member.id)

        except Exception as e:
            logger.error("保存群组成员失败: %s", e)


async def handle_group_left_member(update: Update, context: ContextTypes.DEFAULT_TYPE) -> None:
    """处理成员离开群组"""
    message = update.message
    if not message or not message.left_chat_member:
        return

    member = message.left_chat_member
    try:
        await database.update_group_member_status(
            user_id=member.id,
            group_id=message.chat.id,
            status="left",
        )

        logger.info("✅ 成员离开: %s", member.username or member.id)

    except Exception as e:
        logger.error("更新成员状态失败: %s", e)


# 频道验证冷却缓存：避免对同一用户在短时间内重复调用 get_chat_member
# 格式：(group_id, user_id) -> timestamp
_verify_cooldown: dict = {}
VERIFY_COOLDOWN_SECS = 30  # 30秒内同一用户只检测一次


async def _check_channel_verify(
    update: Update,
    context: ContextTypes.DEFAULT_TYPE,
    group_id: int,
    message,
) -> None:
    """
    频道关注验证拦截器：
    - 仅当群组开启 verify_type='channel' 且 verify_channel_id 非空时生效
    - 管理员/群主免检
    - is_verified=True 的成员直接放行
    - 未验证成员：实时调用 get_chat_member；通过则更新 DB；未通过则删消息+发提示
    """
    import time
    try:
        # 频道身份/匿名身份发言（sender_chat 存在）不做“用户关注频道”校验
        if message.sender_chat:
            return

        # 非用户消息无需校验（避免对系统消息/匿名主体误判）
        if not message.from_user:
            return

        async with database.aiosqlite.connect(database.DB_NAME) as db:
            db.row_factory = database.aiosqlite.Row
            async with db.execute(
                "SELECT verify_enabled, verify_type, verify_channel_id, verify_msg_template FROM tracked_groups WHERE group_id=?",
                (group_id,)
            ) as cur:
                row = await cur.fetchone()

        if not row or not row['verify_enabled'] or row['verify_type'] != 'channel':
            return
        channel_id = row['verify_channel_id']
        if not channel_id:
            return

        user = message.from_user
        user_id = user.id
        user_name = user.first_name or user.username or f'UID{user_id}'

        # Telegram 系统账号/桥接协议账号放行（避免被误判为普通待验证用户）
        if user_id == 777000:
            return

        # 管理员/群主免检
        try:
            member_status = await context.bot.get_chat_member(chat_id=group_id, user_id=user_id)
            if member_status.status in ('administrator', 'creator'):
                return
        except Exception:
            pass

        # 冷却期内已知已验证则跳过
        cache_key = (group_id, user_id)
        now = time.time()
        if cache_key in _verify_cooldown and now - _verify_cooldown[cache_key] < VERIFY_COOLDOWN_SECS:
            return
        _verify_cooldown[cache_key] = now

        # 检查 DB 中 is_verified
        is_verified = await database.get_member_verified(group_id, user_id)
        if is_verified:
            return

        # 实时调用 Telegram 检测频道成员资格
        from handlers.join_verify import _check_channel_membership
        in_channel = await _check_channel_membership(context.bot, user_id, channel_id)

        if in_channel:
            # 通过：更新 DB，放行
            await database.set_member_verified(group_id, user_id)
            logger.info("[频道验证] 用户 %s 已关注频道 %s，放行", user_id, channel_id)
            return

        # 未通过：删除消息 + 发临时提示
        try:
            await context.bot.delete_message(chat_id=group_id, message_id=message.message_id)
        except Exception as del_err:
            logger.warning("[频道验证] 删除消息失败: %s", del_err)

        # 构建提示文案
        template = row['verify_msg_template'] or (
            f"🚫 {user_name}，请先关注频道 {channel_id} 后再发言！"
        )
        tip_text = template.replace('{user}', f'<a href="tg://user?id={user_id}">{user_name}</a>') \
                            .replace('{channel}', str(channel_id))

        try:
            tip_msg = await context.bot.send_message(
                chat_id=group_id,
                text=tip_text,
                parse_mode='HTML'
            )
            # 5秒后自动删除提示
            import asyncio
            asyncio.create_task(_auto_delete_msg(context.bot, group_id, tip_msg.message_id, 5))
        except Exception as tip_err:
            logger.warning("[频道验证] 发送提示失败: %s", tip_err)

    except Exception as e:
        logger.error("[频道验证] 拦截异常: %s", e)


async def _auto_delete_msg(bot, chat_id: int, message_id: int, delay: int) -> None:
    """延迟删除消息"""
    import asyncio
    await asyncio.sleep(delay)
    try:
        await bot.delete_message(chat_id=chat_id, message_id=message_id)
    except Exception:
        pass


# 2026 优化：使用显式媒体过滤器，确保表情包、图片等均能触发活跃状态更新
group_track_handler = MessageHandler(
    filters.ChatType.GROUPS & (filters.ALL) & ~filters.COMMAND, 
    track_group_seen
)

group_message_handler = MessageHandler(
    filters.TEXT & filters.ChatType.GROUPS & ~filters.COMMAND,
    monitor_group_keywords,
)

channel_track_handler = MessageHandler(
    filters.ChatType.CHANNEL & (filters.ALL) & ~filters.COMMAND,
    track_channel_seen,
)

new_member_handler = MessageHandler(
    filters.StatusUpdate.NEW_CHAT_MEMBERS,
    handle_new_group_member,
)

left_member_handler = MessageHandler(
    filters.StatusUpdate.LEFT_CHAT_MEMBER,
    handle_group_left_member,
)
