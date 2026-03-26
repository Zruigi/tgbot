                async toggleGroupRevokeSync() {
                    if (!this.selectedGroupRoomId) return;
                    const newVal = !this.selectedGroupRevokeSync;
                    try {
                        const res = await fetch('/api/group_listen/revoke_sync', {
                            method: 'POST',
                            headers: { 'Content-Type': 'application/json' },
                            body: JSON.stringify({ group_id: this.selectedGroupRoomId, enabled: newVal })
                        });
                        const json = await res.json();
                        if (json.status === 'ok') {
                            this.selectedGroupRevokeSync = newVal;
                            // 同步到群列表数据
                            const g = this.groups.find(x => x.group_id === this.selectedGroupRoomId);
                            if (g) g.delete_on_revoke = newVal ? 1 : 0;
                        } else {
                            alert('切换失败: ' + (json.message || '未知错误'));
                        }
                    } catch (e) {
                        console.error('toggleGroupRevokeSync failed:', e);
                        alert('切换失败，请稍后重试');
                    }
                },
                async loadRevokeSyncChatInfo() {
                    try {
                        const res = await fetch('/api/group_listen/sync_chat_info');
                        const json = await res.json();
                        if (json.status === 'ok' && json.chat_id) {
                            this.revokeSyncChatId = json.chat_id;
                            this.revokeSyncChatTitle = json.title || '';
                        }
                    } catch (e) { console.error(e); }
                },
                async createRevokeSyncChat() {
                    if (this.revokeSyncChatLoading) return;
                    if (this.revokeSyncChatId) {
                        const reset = confirm(`校对暂存群已设置\n群 ID: ${this.revokeSyncChatId}\n名称: ${this.revokeSyncChatTitle || '(未知)'}\n\n点确定可重新设置一个新的群 ID`);
                        if (!reset) return;
                    }
                    const input = prompt(
                        '请输入校对暂存群的群 ID：\n\n步骤：\n1. 手动创建一个 Telegram 群\n2. 把 Bot 拉进群并设为管理员\n3. 发送任意消息，转发给 @userinfobot 获取群 ID（负数）\n4. 把群 ID 粘贴到下方',
                        this.revokeSyncChatId || ''
                    );
                    if (!input || !input.trim()) return;
                    this.revokeSyncChatLoading = true;
                    try {
                        const res = await fetch('/api/group_listen/create_sync_chat', {
                            method: 'POST',
                            headers: { 'Content-Type': 'application/json' },
                            body: JSON.stringify({ chat_id: input.trim() })
                        });
                        const json = await res.json();
                        if (json.status === 'ok') {
                            this.revokeSyncChatId = json.chat_id;
                            this.revokeSyncChatTitle = json.title || '';
                            alert(`校对暂存群设置成功！\n群名: ${json.title}\n群 ID: ${json.chat_id}\n\nBot 将在此群内验证消息是否被撤回，不会在原群刷屏。`);
                        } else {
                            alert('设置失败: ' + (json.message || '未知错误'));
                        }
                    } catch (e) {
                        console.error('createRevokeSyncChat failed:', e);
                        alert('设置失败，请稍后重试');
                    } finally {
                        this.revokeSyncChatLoading = false;
                    }
                },
                async selectGroupRoom(g) {
                    if (!g || !g.group_id) return;
                    // 切换群时显示遮罩，不清空旧消息（避免空白闪烁）
                    if (this.selectedGroupRoomId !== g.group_id) {
                        this.groupRoomSwitching = true;
                        this.groupRoomInput = '';
                        this.groupRoomReplyingTo = null;
                    }
                    this.selectedGroupRoomId = g.group_id;
                    this.selectedGroupRoomTitle = g.group_title || String(g.group_id);
                    this.selectedGroupRevokeSync = !!(g.delete_on_revoke);
                    // 触发入场动画
                    this.groupRoomEntering = false;
                    this.$nextTick(() => { this.groupRoomEntering = true; });
                    await Promise.all([
                        // 1. 获取真实群头像（优先，若已缓存则跳过）
                        (async () => {
                            if (this.groupAvatarMap[g.group_id]) return;
                            try {
                                const res = await fetch(`/api/group_photo?group_id=${g.group_id}`);
                                const json = await res.json();
                                if (json.status === 'ok' && json.file_id) {
                                    this.groupAvatarMap = { ...this.groupAvatarMap, [g.group_id]: json.file_id };
                                    if (!this.fileUrls[json.file_id] && !this.fileErrors[json.file_id]) {
                                        this.autoLoadFile(json.file_id);
                                    }
                                }
                            } catch (e) { console.warn('获取群头像失败:', e); }
                        })(),
                        // 2. 静默预加载成员头像
                        (async () => {
                            try {
                                const res = await fetch(`/api/group_members?group_id=${g.group_id}`);
                                const json = await res.json();
                                if (json.status === 'ok' && json.members) {
                                    this.groupMembers = json.members;
                                    const avatarMap = { ...this.groupRoomAvatarMap };
                                    json.members.forEach(member => {
                                        if (member.user_id && member.avatar_file_id) {
                                            avatarMap[member.user_id] = member.avatar_file_id;
                                        }
                                    });
                                    this.groupRoomAvatarMap = avatarMap;
                                    // 批量预加载成员头像 URL
                                    Object.values(avatarMap).forEach(fid => {
                                        if (fid && !this.fileUrls[fid] && !this.fileErrors[fid]) this.autoLoadFile(fid);
                                    });
                                }
                            } catch (e) { console.warn('预加载成员头像失败:', e); }
                        })()
                    ]);
                    await this.refreshGroupRoomMessages();
                },
                async refreshGroupRoomMessages() {
                    if (!this.selectedGroupRoomId) return;
                    // 首次加载才显示 loading，静默刷新不闪跳
                    const isFirstLoad = this.groupRoomMessages.length === 0;
                    if (isFirstLoad) this.isGroupRoomLoading = true;
                    try {
                        const res = await fetch(`/api/group_messages?group_id=${this.selectedGroupRoomId}&limit=100`);
                        const json = await res.json();
                        if (json.status === 'ok') {
                            const incoming = json.messages || [];
                            if (isFirstLoad || this.groupRoomSwitching) {
                                // 首次加载或切换群：直接替换（遮罩覆盖下用户看不到空白）
                                this.groupRoomMessages = incoming;
                            } else {
                                // 增量追加：只加入新 tg_msg_id
                                const existingIds = new Set(this.groupRoomMessages.map(m => m.tg_msg_id || m.msg_id));
                                const newMsgs = incoming.filter(m => !existingIds.has(m.tg_msg_id || m.msg_id));
                                if (newMsgs.length > 0) {
                                    this.groupRoomMessages.push(...newMsgs);
                                    // 只在有新消息时滚动到底
                                    this.$nextTick(() => {
                                        const el = document.getElementById('groupRoomHistory');
                                        if (el) el.scrollTop = el.scrollHeight;
                                    });
                                }
                            }
                            // 记录最新一条消息供群列表预览
                            if (this.groupRoomMessages.length > 0) {
                                const last = this.groupRoomMessages[this.groupRoomMessages.length - 1];
                                this.groupLastMessageMap[this.selectedGroupRoomId] = {
                                    content: last.content || '',
                                    msg_type: last.msg_type || 'text',
                                    time: last.timestamp || last.message_ts,
                                    username: last.username || ('U' + last.user_id),
                                };
                            }
                            // 从 groupMembers 缓存填充头像映射，同时直接用消息里的 avatar_file_id
                            const avatarMap = {...this.groupRoomAvatarMap};
                            // 优先从消息自带的 avatar_file_id 填充（不依赖成员列表）
                            (incoming).forEach(m => {
                                if (m.user_id && m.avatar_file_id && !avatarMap[m.user_id]) {
                                    avatarMap[m.user_id] = m.avatar_file_id;
                                }
                            });
                            // 再从 groupMembers 缓存补充
                            (this.groupMembers || []).forEach(member => {
                                if (member.user_id && member.avatar_file_id) {
                                    avatarMap[member.user_id] = member.avatar_file_id;
                                }
                            });
                            this.groupRoomAvatarMap = avatarMap;
                            // 自动加载头像 URL（数据已从 API 带入，无需手动拉取成员列表）
                            Object.values(avatarMap).forEach(fid => {
                                if (fid && !this.fileUrls[fid] && !this.fileErrors[fid]) this.autoLoadFile(fid);
                            });
                            (this.groupRoomMessages).forEach(m => {
                                if (m.file_id && !this.fileUrls[m.file_id] && !this.fileErrors[m.file_id]) {
                                    this.autoLoadFile(m.file_id);
                                }
                            });
                            if (isFirstLoad) {
                                this.$nextTick(() => {
                                    const el = document.getElementById('groupRoomHistory');
                                    if (el) el.scrollTop = el.scrollHeight;
                                });
                            }
                        }
                    } catch (e) {
                        console.error('加载群聊工作台消息失败:', e);
                    } finally {
                        this.isGroupRoomLoading = false;
                        // 遮罩淡出
                        if (this.groupRoomSwitching) {
                            setTimeout(() => { this.groupRoomSwitching = false; }, 280);
                        }
                        // 首次加载完成后，延时重置入场动画标志（动画播完再关闭，避免刷新重触）
                        if (this.groupRoomEntering) {
                            setTimeout(() => { this.groupRoomEntering = false; }, 800);
                        }
                    }
                },
                prepareGroupRoomReply(msg) {
                    this.groupRoomReplyingTo = msg;
                },
                scrollToGroupRoomMessage(tgMsgId) {
                    if (!tgMsgId) return;
                    const el = document.getElementById('group-room-msg-' + tgMsgId);
                    if (!el) return;
                    el.scrollIntoView({ behavior: 'smooth', block: 'center' });
                    el.classList.add('highlight-anim');
                    setTimeout(() => el.classList.remove('highlight-anim'), 2500);
                },
                async sendGroupRoomMessage() {
                    if (this.isGroupRoomSending || !this.selectedGroupRoomId) return;
                    const content = (this.groupRoomInput || '').trim();
                    if (!content) return;

                    this.isGroupRoomSending = true;
                    try {
                        if (this.groupRoomReplyingTo && this.groupRoomReplyingTo.tg_msg_id) {
                            const res = await fetch('/api/group_messages/reply', {
                                method: 'POST',
                                headers: { 'Content-Type': 'application/json' },
                                body: JSON.stringify({
                                    group_id: this.selectedGroupRoomId,
                                    tg_msg_id: this.groupRoomReplyingTo.tg_msg_id,
                                    content,
                                    quoted_text: this.groupRoomReplyingTo.content || ''
                                })
                            });
                            const json = await res.json();
                            if (json.status !== 'ok') {
                                alert('发送失败: ' + (json.message || '未知错误'));
                                return;
                            }
                        } else {
                            const res = await fetch('/api/group_messages/send', {
                                method: 'POST',
                                headers: { 'Content-Type': 'application/json' },
                                body: JSON.stringify({
                                    group_id: this.selectedGroupRoomId,
                                    content
                                })
                            });
                            const json = await res.json();
                            if (json.status !== 'ok') {
                                alert('发送失败: ' + (json.message || '未知错误'));
                                return;
                            }
                        }

                        this.groupRoomInput = '';
                        this.groupRoomReplyingTo = null;
                        await this.refreshGroupRoomMessages();
                    } catch (e) {
                        console.error('群聊消息发送失败:', e);
                        alert('发送失败，请稍后重试');
                    } finally {
                        this.isGroupRoomSending = false;
                    }
                },
                async loadGroups() {
                    // 首次加载才显示 loading，静默刷新不闪跳
                    if (this.groups.length === 0) this.isGroupLoading = true;
                    try {
                        const res = await fetch(`/api/group_listen/list?page=${this.groupPage}&size=${this.groupPageSize}`);
                        const json = await res.json();
                        if (json.status === 'ok') {
                            const newData = json.data;
                            if (this.groups.length === 0) {
                                // 首次直接赋值
                                this.groups = newData.map(g => ({
                                    ...g,
                                    isEditingKws: false,
                                    tempKeywords: g.custom_keywords || ''
                                }));
                            } else {
                                // 增量更新：只改变字段，不重建数组，避免闪跳
                                const existingMap = {};
                                this.groups.forEach(g => { existingMap[g.group_id] = g; });
                                newData.forEach(ng => {
                                    if (existingMap[ng.group_id]) {
                                        // 原地更新，保留 isEditingKws/tempKeywords
                                        Object.assign(existingMap[ng.group_id], ng);
                                    } else {
                                        this.groups.push({ ...ng, isEditingKws: false, tempKeywords: ng.custom_keywords || '' });
                                    }
                                });
                                // 删除已不存在的群
                                const newIds = new Set(newData.map(g => g.group_id));
                                this.groups = this.groups.filter(g => newIds.has(g.group_id));
                            }
                            this.groupTotal = json.total;
                            this.$nextTick(() => this.preloadGroupLastMessages());
                        }
                    } catch (e) { console.error(e) }
                    finally { this.isGroupLoading = false; }
                },
                async preloadGroupLastMessages() {
                    // 并发拉取每个群最新 1 条消息，填充 groupLastMessageMap
                    const groups = this.groups || [];
                    await Promise.all(groups.map(async g => {
                        if (this.groupLastMessageMap[g.group_id] && this.groupAvatarMap[g.group_id]) return; // 已有缓存跳过
                        try {
                            const res = await fetch(`/api/group_messages?group_id=${g.group_id}&limit=1`);
                            const json = await res.json();
                            if (json.status === 'ok' && json.messages && json.messages.length > 0) {
                                const last = json.messages[json.messages.length - 1];
                                this.groupLastMessageMap[g.group_id] = {
                                    content: last.content || '',
                                    msg_type: last.msg_type || 'text',
                                    time: last.timestamp || last.message_ts,
                                    username: last.username || ('U' + last.user_id),
                                };
                            }
                            // 获取真实群头像（仅未缓存时）
                            if (!this.groupAvatarMap[g.group_id]) {
                                const pres = await fetch(`/api/group_photo?group_id=${g.group_id}`);
                                const pjson = await pres.json();
                                if (pjson.status === 'ok' && pjson.file_id) {
                                    this.groupAvatarMap = { ...this.groupAvatarMap, [g.group_id]: pjson.file_id };
                                    if (!this.fileUrls[pjson.file_id] && !this.fileErrors[pjson.file_id]) {
                                        this.autoLoadFile(pjson.file_id);
                                    }
                                }
                            }
                        } catch (e) { /* 忽略单个群拉取失败 */ }
                    }));
                },
                async toggleGroupMonitor(group) {
                    const newStatus = !group.is_monitored;
                    try {
                        const res = await fetch('/api/group_listen/toggle', {
                            method: 'POST',
                            headers: { 'Content-Type': 'application/json' },
                            body: JSON.stringify({
                                group_id: group.group_id,
                                listen_status: newStatus
                            })
                        });
                        const json = await res.json();
                        if (json.code === 200) {
                            group.is_monitored = newStatus;
                            alert(`群组 ${group.group_title} 监控已${newStatus ? '开启' : '关闭'}`);
                        }
                    } catch (e) { console.error(e) }
                },
                async saveGroupKws(group) {
                    try {
                        const res = await fetch('/api/groups/config', {
                            method: 'POST',
                            headers: { 'Content-Type': 'application/json' },
                            body: JSON.stringify({
                                group_id: group.group_id,
                                is_monitored: group.is_monitored,
                                custom_keywords: group.tempKeywords
                            })
                        });
                        const json = await res.json();
                        if (json.status === 'ok') {
                            group.custom_keywords = group.tempKeywords;
                            group.isEditingKws = false;
                        }
                    } catch (e) { console.error(e) }
                },
                async viewGroupMembers(groupId) {
                    this.selectedGroupId = groupId;
                    this.showGroupMembersModal = true;
                    this.groupMemberSearchQuery = '';
                    this.selectedMemberIds = [];
                    this.batchActionResult = null;
                    this.isMembersLoading = true;
                    this.groupMembers = [];
                    this.groupMemberCount = null;
                    fetch(`/api/group_member_count?group_id=${groupId}`)
                        .then(r => r.json())
                        .then(j => { if (j.status === 'ok') this.groupMemberCount = j.count; })
                        .catch(() => {});
                    try {
                        const res = await fetch(`/api/group_members?group_id=${groupId}`);
                        const json = await res.json();
                        if (json.status === 'ok') {
                            this.groupMembers = json.members;
                            const avatarMap = {...this.groupRoomAvatarMap};
                            (this.groupMembers || []).forEach(member => {
                                if (member.user_id && member.avatar_file_id) {
                                    avatarMap[member.user_id] = member.avatar_file_id;
                                }
                            });
                            this.groupRoomAvatarMap = avatarMap;
                            (this.groupMembers || []).forEach(m => {
                                if (m.avatar_file_id && !this.fileUrls[m.avatar_file_id] && !this.fileErrors[m.avatar_file_id]) {
                                    this.autoLoadFile(m.avatar_file_id)
                                }
                            })
                        }
                    } catch (e) {
                        console.error('加载群成员失败:', e);
                    } finally {
                        this.isMembersLoading = false;
                    }
                },
                async openVerifyConfig(groupId) {
                    const res = await fetch(`/api/group_verify_config?group_id=${groupId}`);
                    const data = await res.json();
                    const cfg = data.config || {};
                    this.verifyConfigForm = {
                        verify_enabled: !!cfg.verify_enabled,
                        verify_type: cfg.verify_type || 'captcha',
                        verify_timeout: cfg.verify_timeout || 120,
                        verify_channel_id: cfg.verify_channel_id || '',
                        verify_questions: cfg.verify_questions || [],
                    };
                    this.showVerifyConfigModal = true;
                },
                async saveVerifyConfig() {
                    try {
                        const payload = { group_id: this.selectedGroupRoomId, ...this.verifyConfigForm };
                        const res = await fetch('/api/group_verify_config', {
                            method: 'POST',
                            headers: { 'Content-Type': 'application/json' },
                            body: JSON.stringify(payload),
                        });
                        const data = await res.json();
                        if (data.status === 'ok') {
                            this.groupVerifyConfig[this.selectedGroupRoomId] = { ...this.verifyConfigForm };
                            this.showVerifyConfigModal = false;
                            this.showToast('验证配置已保存', 'success');
                        } else {
                            this.showToast('保存失败: ' + data.message, 'error');
                        }
                    } catch(e) {
                        this.showToast('保存失败: ' + e.message, 'error');
                    }
                },

                toggleSelectAllFilteredMembers() {
                    const rows = this.filteredGroupMembers || [];
                    if (!rows.length) return;
                    if (this.isAllFilteredMembersSelected) {
                        const rowIds = new Set(rows.map(m => m.user_id));
                        this.selectedMemberIds = (this.selectedMemberIds || []).filter(id => !rowIds.has(id));
                        return;
                    }
                    const merged = new Set(this.selectedMemberIds || []);
                    rows.forEach(m => merged.add(m.user_id));
                    this.selectedMemberIds = Array.from(merged);
                },
                selectNormalMembersFromFiltered() {
                    const rows = this.filteredGroupMembers || [];
                    const merged = new Set(this.selectedMemberIds || []);
                    rows.forEach(m => {
                        const status = String(m.status || '').toLowerCase();
                        if (status !== 'admin' && status !== 'creator' && status !== 'administrator') {
                            merged.add(m.user_id);
                        }
                    });
                    this.selectedMemberIds = Array.from(merged);
                },
                selectAdminMembersFromFiltered() {
                    const rows = this.filteredGroupMembers || [];
                    const merged = new Set(this.selectedMemberIds || []);
                    rows.forEach(m => {
                        const status = String(m.status || '').toLowerCase();
                        if (status === 'admin' || status === 'creator' || status === 'administrator') {
                            merged.add(m.user_id);
                        }
                    });
                    this.selectedMemberIds = Array.from(merged);
                },
                clearSelectedMembers() {
                    this.selectedMemberIds = [];
                },
                appendBatchOperationLog(actionLabel, success, failed, failedList) {
                    const log = {
                        actionLabel,
                        success,
                        failed,
                        failedList: failedList || [],
                        time: new Date().toLocaleTimeString(),
                    };
                    this.batchOperationLogs = [log, ...(this.batchOperationLogs || [])].slice(0, 10);
                },
                copyBatchResultAsText() {
                    if (!this.batchActionResult) return;
                    const r = this.batchActionResult;
                    const lines = [
                        `操作: ${r.actionLabel}`,
                        `成功: ${r.success}`,
                        `失败: ${r.failed}`,
                    ];
                    (r.failedList || []).forEach(f => lines.push(`UID ${f.user_id}: ${f.reason || '未知原因'}`));
                    const text = lines.join('\n');
                    if (navigator.clipboard && navigator.clipboard.writeText) {
                        navigator.clipboard.writeText(text).then(() => alert('批量结果已复制')).catch(() => alert('复制失败，请手动复制'));
                        return;
                    }
                    alert(text);
                },
                downloadBatchResultJson() {
                    if (!this.batchActionResult) return;
                    const payload = {
                        ...this.batchActionResult,
                        exported_at: new Date().toISOString(),
                        group_id: this.selectedGroupId,
                    };
                    const blob = new Blob([JSON.stringify(payload, null, 2)], { type: 'application/json;charset=utf-8' });
                    const url = URL.createObjectURL(blob);
                    const a = document.createElement('a');
                    a.href = url;
                    a.download = `group_batch_result_${this.selectedGroupId || 'unknown'}_${Date.now()}.json`;
                    document.body.appendChild(a);
                    a.click();
                    document.body.removeChild(a);
                    URL.revokeObjectURL(url);
                },
                async batchGroupMemberAction(action) {
                    if (!this.selectedGroupId) return;
                    const ids = (this.selectedMemberIds || []).slice();
                    if (!ids.length) return;

                    let confirmText = '';
                    if (action === 'mute') confirmText = `确认批量禁言 ${ids.length} 人（30分钟）？`;
                    else if (action === 'unmute') confirmText = `确认批量解除禁言 ${ids.length} 人？`;
                    else confirmText = `确认批量移除 ${ids.length} 人？`;
                    if (!confirm(confirmText)) return;

                    this.isBatchMemberActionLoading = true;
                    try {
                        const res = await fetch('/api/group_members/action_batch', {
                            method: 'POST',
                            headers: { 'Content-Type': 'application/json' },
                            body: JSON.stringify({
                                group_id: this.selectedGroupId,
                                user_ids: ids,
                                action,
                                duration_minutes: 30,
                            })
                        });
                        const json = await res.json();
                        if (json.status !== 'ok') {
                            alert('批量操作失败: ' + (json.message || '未知错误'));
                            return;
                        }

                        if (action === 'kick') {
                            const removed = new Set(json.success_ids || []);
                            this.groupMembers = this.groupMembers.filter(m => !removed.has(m.user_id));
                        }

                        const failed = (json.summary && json.summary.failed) || 0;
                        const success = (json.summary && json.summary.success) || 0;
                        this.batchActionResult = {
                            actionLabel: action === 'mute' ? '批量禁言' : (action === 'unmute' ? '批量解禁' : '批量移除'),
                            success,
                            failed,
                            failedList: json.failed || [],
                        };
                        this.appendBatchOperationLog(this.batchActionResult.actionLabel, success, failed, json.failed || []);
                        this.selectedMemberIds = [];
                        alert(`批量操作完成：成功 ${success} 人，失败 ${failed} 人`);
                    } catch (e) {
                        console.error('批量群成员操作失败:', e);
                        alert('批量操作失败，请稍后重试');
                    } finally {
                        this.isBatchMemberActionLoading = false;
                    }
                },
                async batchGroupMemberAdminAction(action) {
                    if (!this.selectedGroupId) return;
                    const ids = (this.selectedMemberIds || []).slice();
                    if (!ids.length) return;

                    const confirmText = action === 'promote'
                        ? `确认批量设为管理员 ${ids.length} 人？（使用默认权限模板）`
                        : `确认批量撤销管理员 ${ids.length} 人？`;
                    if (!confirm(confirmText)) return;

                    this.isBatchMemberActionLoading = true;
                    try {
                        const res = await fetch('/api/group_members/admin_batch', {
                            method: 'POST',
                            headers: { 'Content-Type': 'application/json' },
                            body: JSON.stringify({
                                group_id: this.selectedGroupId,
                                user_ids: ids,
                                action,
                            })
                        });
                        const json = await res.json();
                        if (json.status !== 'ok') {
                            alert('批量管理员操作失败: ' + (json.message || '未知错误'));
                            return;
                        }

                        const successSet = new Set(json.success_ids || []);
                        this.groupMembers = (this.groupMembers || []).map(m => {
                            if (!successSet.has(m.user_id)) return m;
                            return { ...m, status: action === 'promote' ? 'admin' : 'member' };
                        });

                        const failed = (json.summary && json.summary.failed) || 0;
                        const success = (json.summary && json.summary.success) || 0;
                        this.batchActionResult = {
                            actionLabel: action === 'promote' ? '批量设管理员' : '批量撤销管理员',
                            success,
                            failed,
                            failedList: json.failed || [],
                        };
                        this.appendBatchOperationLog(this.batchActionResult.actionLabel, success, failed, json.failed || []);
                        this.selectedMemberIds = [];
                        alert(`批量管理员操作完成：成功 ${success} 人，失败 ${failed} 人`);
                    } catch (e) {
                        console.error('批量管理员操作失败:', e);
                        alert('批量管理员操作失败，请稍后重试');
                    } finally {
                        this.isBatchMemberActionLoading = false;
                    }
                },
                _setupMemberScrollObserver() {
                    if (this._memberObserver) this._memberObserver.disconnect();
                    const sentinel = this.$refs.memberScrollSentinel;
                    if (!sentinel) return;
                    this._memberObserver = new IntersectionObserver((entries) => {
                        if (entries[0].isIntersecting) {
                            this.memberVisibleCount += 20;
                            this.$nextTick(() => this._setupMemberScrollObserver());
                        }
                    }, { threshold: 0.1 });
                    this._memberObserver.observe(sentinel);
                },
                async directUidAction(action) {
                    const uid = parseInt((this.directUidInput || '').trim());
                    if (!uid || isNaN(uid)) {
                        this.directUidResult = { ok: false, msg: 'UID 格式不正确' };
                        return;
                    }
                    if (!this.selectedGroupId) {
                        this.directUidResult = { ok: false, msg: '请先选择群组' };
                        return;
                    }
                    this.directUidResult = { ok: true, msg: '执行中...' };
                    try {
                        const res = await fetch('/api/group_members/action', {
                            method: 'POST',
                            headers: { 'Content-Type': 'application/json' },
                            body: JSON.stringify({ group_id: this.selectedGroupId, user_id: uid, action, duration_minutes: 30 })
                        });
                        const json = await res.json();
                        if (json.status === 'ok') {
                            this.directUidResult = { ok: true, msg: `✓ UID ${uid} ${action === 'mute' ? '已禁言' : action === 'unmute' ? '已解禁' : '已移除'}` };
                            this.directUidInput = '';
                            setTimeout(() => { this.directUidResult = null; }, 3000);
                        } else {
                            this.directUidResult = { ok: false, msg: json.message || '操作失败' };
                        }
                    } catch(e) {
                        this.directUidResult = { ok: false, msg: '网络错误' };
                    }
                },
                async groupMemberAction(member, action) {
                    if (!this.selectedGroupId || !member || !member.user_id) return;
                    const key = `${action}-${member.user_id}`;
                    if (this.memberActionBusy[key]) return;

                    let confirmText = '';
                    if (action === 'mute') confirmText = `确认禁言 @${member.username || member.user_id} 30 分钟？`;
                    else if (action === 'unmute') confirmText = `确认解除禁言 @${member.username || member.user_id}？`;
                    else confirmText = `确认移除 @${member.username || member.user_id}？`;
                    if (!confirm(confirmText)) return;

                    this.memberActionBusy[key] = true;
                    try {
                        const res = await fetch('/api/group_members/action', {
                            method: 'POST',
                            headers: { 'Content-Type': 'application/json' },
                            body: JSON.stringify({
                                group_id: this.selectedGroupId,
                                user_id: member.user_id,
                                action,
                                duration_minutes: 30
                            })
                        });
                        const json = await res.json();
                        if (json.status !== 'ok') {
                            alert('操作失败: ' + (json.message || '未知错误'));
                            return;
                        }

                        if (action === 'kick') {
                            this.groupMembers = this.groupMembers.filter(m => m.user_id !== member.user_id);
                        }
                        alert('操作成功');
                    } catch (e) {
                        console.error('群成员操作失败:', e);
                        alert('操作失败，请稍后重试');
                    } finally {
                        this.memberActionBusy[key] = false;
                    }
                },
                async promoteGroupAdmin(member) {
                    if (!this.selectedGroupId || !member || !member.user_id) return;
                    this.adminPermissionTarget = member;
                    this.adminPermissionLoading = true;
                    this.adminPermissionMeta = {
                        member_status: '',
                        is_admin: false,
                        is_owner: false,
                    };
                    this.adminPermissionForm = {
                        can_manage_chat: true,
                        can_delete_messages: true,
                        can_manage_video_chats: false,
                        can_restrict_members: true,
                        can_promote_members: false,
                        can_change_info: false,
                        can_invite_users: true,
                        can_pin_messages: true,
                        can_post_messages: false,
                        can_edit_messages: false,
                        can_post_stories: false,
                        can_edit_stories: false,
                        can_delete_stories: false,
                        is_anonymous: false,
                    };
                    this.showAdminPermissionModal = true;

                    try {
                        const res = await fetch(`/api/group_members/permissions?group_id=${this.selectedGroupId}&user_id=${member.user_id}`);
                        const json = await res.json();
                        if (json.status !== 'ok') {
                            this.closeAdminPermissionModal();
                            alert('读取管理员权限失败: ' + (json.message || '未知错误'));
                            return;
                        }
                        this.adminPermissionMeta = {
                            member_status: json.member_status || '',
                            is_admin: !!json.is_admin,
                            is_owner: !!json.is_owner,
                        };
                        if (json.permissions && typeof json.permissions === 'object') {
                            this.adminPermissionForm = {
                                ...this.adminPermissionForm,
                                ...json.permissions,
                            };
                        }
                    } catch (e) {
                        console.error('加载管理员权限失败:', e);
                        this.closeAdminPermissionModal();
                        alert('读取管理员权限失败，请稍后重试');
                    } finally {
                        this.adminPermissionLoading = false;
                    }
                },
                closeAdminPermissionModal() {
                    this.showAdminPermissionModal = false;
                    this.adminPermissionTarget = null;
                    this.adminPermissionLoading = false;
                    this.adminPermissionMeta = {
                        member_status: '',
                        is_admin: false,
                        is_owner: false,
                    };
                },
                async confirmDemoteGroupAdmin() {
                    const member = this.adminPermissionTarget;
                    if (!this.selectedGroupId || !member || !member.user_id) return;
                    if (this.adminPermissionMeta.is_owner) { alert('群主权限不可撤销'); return; }
                    if (!confirm(`确认撤销 @${member.username || member.user_id} 的管理员权限？`)) return;
                    const key = `admin-${member.user_id}`;
                    if (this.memberActionBusy[key]) return;
                    this.memberActionBusy[key] = true;
                    this.adminPermissionLoading = true;
                    try {
                        const res = await fetch('/api/group_members/demote_admin', {
                            method: 'POST',
                            headers: { 'Content-Type': 'application/json' },
                            body: JSON.stringify({ group_id: this.selectedGroupId, user_id: member.user_id })
                        });
                        const json = await res.json();
                        if (json.status !== 'ok') { alert('撤销失败: ' + (json.message || '未知错误')); return; }
                        const idx = this.groupMembers.findIndex(x => x.user_id === member.user_id);
                        if (idx >= 0) this.groupMembers[idx].status = 'member';
                        this.closeAdminPermissionModal();
                        alert('已撤销管理员权限');
                    } catch (e) {
                        console.error('撤销管理员失败:', e);
                        alert('撤销失败，请稍后重试');
                    } finally {
                        this.memberActionBusy[key] = false;
                        this.adminPermissionLoading = false;
                    }
                },
                async confirmPromoteGroupAdmin() {
                    const member = this.adminPermissionTarget;
                    if (!this.selectedGroupId || !member || !member.user_id) return;
                    if (this.adminPermissionMeta.is_owner) {
                        alert('群主权限不可修改');
                        return;
                    }
                    const key = `admin-${member.user_id}`;
                    if (this.memberActionBusy[key]) return;

                    this.memberActionBusy[key] = true;
                    try {
                        const res = await fetch('/api/group_members/promote_admin', {
                            method: 'POST',
                            headers: { 'Content-Type': 'application/json' },
                            body: JSON.stringify({
                                group_id: this.selectedGroupId,
                                user_id: member.user_id,
                                permissions: this.adminPermissionForm
                            })
                        });
                        const json = await res.json();
                        if (json.status !== 'ok') {
                            alert('设为管理员失败: ' + (json.message || '未知错误'));
                            return;
                        }
                        const idx = this.groupMembers.findIndex(x => x.user_id === member.user_id);
                        if (idx >= 0) this.groupMembers[idx].status = 'admin';
                        const wasAdmin = this.adminPermissionMeta.is_admin;
                        this.closeAdminPermissionModal();
                        alert(wasAdmin ? '管理员权限已更新' : '已设置为管理员（权限已应用）');
                    } catch (e) {
                        console.error('设管理员失败:', e);
                        alert('设为管理员失败，请稍后重试');
                    } finally {
                        this.memberActionBusy[key] = false;
                    }
                },
                async demoteGroupAdmin(member) {
                    if (!this.selectedGroupId || !member || !member.user_id) return;
                    const key = `demote-${member.user_id}`;
                    if (this.memberActionBusy[key]) return;

                    if (!confirm(`确认撤销 @${member.username || member.user_id} 的管理员权限？`)) return;

                    this.memberActionBusy[key] = true;
                    try {
                        const res = await fetch('/api/group_members/demote_admin', {
                            method: 'POST',
                            headers: { 'Content-Type': 'application/json' },
                            body: JSON.stringify({
                                group_id: this.selectedGroupId,
                                user_id: member.user_id
                            })
                        });
                        const json = await res.json();
                        if (json.status !== 'ok') {
                            alert('撤销管理员失败: ' + (json.message || '未知错误'));
                            return;
                        }
                        const idx = this.groupMembers.findIndex(x => x.user_id === member.user_id);
                        if (idx >= 0) this.groupMembers[idx].status = 'member';
                        alert('已撤销管理员权限');
                    } catch (e) {
                        console.error('撤销管理员失败:', e);
                        alert('撤销管理员失败，请稍后重试');
                    } finally {
                        this.memberActionBusy[key] = false;
                    }
                },
                async viewGroupHistory(groupId) {
                    this.selectedGroupId = groupId;
                    this.showGroupMessagesModal = true;
                    this.isMessagesLoading = true;
                    this.groupMessages = [];
                    try {
                        const res = await fetch(`/api/group_messages?group_id=${groupId}&limit=50`);
                        const json = await res.json();
                        if (json.status === 'ok') {
                            this.groupMessages = json.messages;
                            (this.groupMessages || []).forEach(m => {
                                if (m.file_id && !this.fileUrls[m.file_id] && !this.fileErrors[m.file_id]) {
                                    this.autoLoadFile(m.file_id)
                                }
                            })
                        }
                    } catch (e) {
                        console.error("加载群消息失败:", e);
                    } finally {
                        this.isMessagesLoading = false;
                    }
                },
                canDeleteGroupMessage(m) {
                    return !!(m && m.tg_msg_id && m.sender_type !== 'bot');
                },
                scrollToGroupMessage(tgMsgId) {
                    if (!tgMsgId) return;
                    const el = document.getElementById('group-msg-' + tgMsgId);
                    if (!el) {
                        alert('引用的原消息不在当前 50 条记录内');
                        return;
                    }
                    el.scrollIntoView({ behavior: 'smooth', block: 'center' });
                    el.classList.add('highlight-anim');
                    setTimeout(() => el.classList.remove('highlight-anim'), 2500);
                },
                chatFromGroupMessage(m) {
                    const name = m.username || m.first_name || `User${m.user_id}`;
                    this.selectUser(m.user_id, name);
                    this.activeTab = 'chat';
                    this.showGroupMessagesModal = false;
                },
                openGroupReplyModal(m) {
                    if (!this.selectedGroupId || !m || !m.tg_msg_id) return;
                    this.groupReplyTarget = m;
                    this.groupReplyText = '';
                    this.showGroupReplyModal = true;
                },
                closeGroupReplyModal() {
                    this.showGroupReplyModal = false;
                    this.groupReplyTarget = null;
                    this.groupReplyText = '';
                    this.isGroupReplySending = false;
                },
                async submitGroupReply() {
                    if (this.isGroupReplySending) return;
                    if (!this.selectedGroupId || !this.groupReplyTarget || !this.groupReplyTarget.tg_msg_id) return;
                    const text = (this.groupReplyText || '').trim();
                    if (!text) return;

                    this.isGroupReplySending = true;
                    try {
                        const res = await fetch('/api/group_messages/reply', {
                            method: 'POST',
                            headers: { 'Content-Type': 'application/json' },
                            body: JSON.stringify({
                                group_id: this.selectedGroupId,
                                tg_msg_id: this.groupReplyTarget.tg_msg_id,
                                content: text,
                                quoted_text: this.groupReplyTarget.content || ''
                            })
                        });

                        let json = null;
                        try {
                            json = await res.json();
                        } catch (_) {
                            const rawText = await res.text();
                            throw new Error(`HTTP ${res.status} ${res.statusText}${rawText ? `\n${rawText.slice(0, 200)}` : ''}`);
                        }

                        if (!res.ok || json.status !== 'ok') {
                            const msg = (json && json.message) ? json.message : `HTTP ${res.status} ${res.statusText}`;
                            if (res.status === 404) {
                                alert('群内回复失败: 接口不存在（通常是服务未重启）。请重启 main.py 后重试。');
                                return;
                            }
                            alert('群内回复失败: ' + msg);
                            return;
                        }

                        this.closeGroupReplyModal();
                        await this.viewGroupHistory(this.selectedGroupId);
                    } catch (e) {
                        console.error('群内回复失败:', e);
                        alert('群内回复失败: ' + (e?.message || '网络错误，请稍后重试'));
                    } finally {
                        this.isGroupReplySending = false;
                    }
                },
                async deleteGroupMessage(m) {
                    const groupId = this.selectedGroupRoomId || this.selectedGroupId;
                    if (!groupId || !m.tg_msg_id) return;
                    if (!confirm('确认删除这条群消息？')) return;
                    try {
                        const res = await fetch('/api/group_messages/delete', {
                            method: 'POST',
                            headers: { 'Content-Type': 'application/json' },
                            body: JSON.stringify({
                                group_id: groupId,
                                tg_msg_id: m.tg_msg_id,
                                user_id: m.user_id
                            })
                        });
                        const json = await res.json();
                        if (json.status === 'ok') {
                            this.groupMessages = this.groupMessages.filter(x => x.msg_id !== m.msg_id);
                            this.groupRoomMessages = (this.groupRoomMessages || []).filter(x => x.msg_id !== m.msg_id);
                        } else {
                            alert('删除失败: ' + (json.message || '未知错误'));
                        }
                    } catch (e) {
                        console.error('删除群消息失败:', e);
                    }
                },
                initWebSocket() {
                    const protocol = window.location.protocol === 'https:' ? 'wss:' : 'ws:';
                    const wsUrl = protocol + '//' + window.location.host + '/ws';
                    this.ws = new WebSocket(wsUrl);
                    
                    this.ws.onmessage = (event) => {
                        try {
                            const msg = JSON.parse(event.data);
                            if (msg.type === 'monitor_update' || msg.type === 'monitor_tx') {
                                this.loadMonitors();
                                this.loadMonitorPnlSummary();
                                if (msg.type === 'monitor_tx') {
                                    const tx = msg.data;
                                    alert(`💰 发现新交易！\n网络: ${tx.network}\n方向: ${tx.direction}\n金额: ${tx.value} USDT\n地址: ${tx.address}`);
                                }
                            } else if (msg.type === 'potential_promoted' || msg.type === 'potential_sync') {
                                this.loadPotentialUsers();
                                this.fetchUsers();
                            } else if (msg.type === 'group_message_deleted') {
                                // 信息校对：实时移除已撤回消息
                                if (msg.data && msg.data.group_id === this.selectedGroupRoomId) {
                                    this.groupRoomMessages = this.groupRoomMessages.filter(m => m.tg_msg_id !== msg.data.tg_msg_id);
                                    this.groupMessages = this.groupMessages.filter(m => m.tg_msg_id !== msg.data.tg_msg_id);
                                }
                            } else if (msg.type === 'group_listen_update' || msg.type === 'group_activity') {
                                this.loadGroups();
                                if (this.selectedGroupRoomId) {
                                    this.refreshGroupRoomMessages();
                                }
                                if (msg.type === 'group_activity' && this.activeTab === 'group_management') {
                                    console.log("群组活动更新:", msg.data);
                                }
                            } else if (msg.type === 'status_update') {
                                // 处理群发进度
                                if (this.broadcastForm.isSending) {
                                    this.broadcastForm.progress++;
                                    if (this.broadcastForm.progress >= this.broadcastForm.total) {
                                        setTimeout(() => {
                                            this.broadcastForm.isSending = false;
                                            alert(`群发任务已完成！成功向 ${this.broadcastForm.total} 位用户投递消息。`);
                                            this.broadcastForm.progress = 0;
                                            this.broadcastForm.total = 0;
                                            this.broadcastForm.selectedUserIds = [];
                                            this.activeTab = 'chat'; // 返回聊天列表
                                            this.fetchUsers(); // 刷新用户列表显示最后一条消息
                                        }, 1000);
                                    }
                                }
                                
                                // 处理单聊实时推送
                                if (this.selectedUserId && msg.data.user_id === this.selectedUserId) {
                                    this.pollMessages(this.selectedUserId);
                                }
                            } else if (msg.type === 'new_message') {
                                // 有新消息进入
                                this.fetchUsers();
                                if (this.selectedUserId && msg.data.from_userid === this.selectedUserId) {
                                    this.pollMessages(this.selectedUserId);
                                }
                            } else if (msg.type === 'push_task_update') {
                                this.loadPushTasks();
                            } else if (msg.type === 'broadcast_targets_sync') {
                                this.loadBroadcastTargets();
                            }
                        } catch (e) { console.error("WS Message error:", e) }
                    };
                    
                    this.ws.onclose = () => {
                        console.log("WebSocket closed, retrying in 3s...");
                        setTimeout(() => this.initWebSocket(), 3000);
                    };
                    
                    this.ws.onerror = (e) => {
                        console.error("WebSocket error:", e);
                    };
                },
                fetchUsers() {
                    this.fetchUsersAsync()
                },
