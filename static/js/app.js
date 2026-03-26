
        const { createApp } = Vue

        const app = createApp({
            data() {
                return {
                    users: [],
                    searchQuery: '',
                    selectedUserId: null,
                    selectedUserName: null,
                    messages: [],
                    inputText: '',
                    page: 1,
                    pollTimer: null,
                    fileUrls: {},
                    fileErrors: {},
                    viewer: null,
                    replyingTo: null,
                    selectedImage: null,
                    selectedImagePreview: null,
                    showSettings: false,
                    enableTranslation: true,
                    configData: {
                        ocr_force_enabled: 'true'
                    },
                    
                    // 群发功能状态 (2026)
                    activeTab: 'chat',
                    broadcastForm: {
                        selectedUserIds: [],
                        selectedChatIds: [],
                        doTranslate: true,
                        isSending: false,
                        progress: 0,
                        total: 0,
                        mode: 'user',
                        retryCount: 1
                    },
                    broadcastTargets: [],
                    isBroadcastTargetsLoading: false,
                    pushTaskForm: {
                        task_name: '',
                        start_at: '',
                        interval_hours: 6,
                    },
                    pushTasks: [],
                    broadcastComposer: {
                        content: '',
                        image: null,
                        imagePreview: null
                    },
                    broadcastDrafts: [],
                    activeBroadcastDraftId: null,
                    
                    // 智能滚动逻辑 (2026)
                    isNearBottom: true,
                    scrollCache: {}, // userId -> scrollTop
                    newMsgCount: 0,
                    lastScrollHeight: 0,

                    // 2026 规格：链上监听
                    monitors: [],
                    showAddMonitorModal: false,
                    newMonitor: {
                        user_id: 0,
                        address: '',
                        network: 'TRC20',
                        remark: ''
                    },
                    isMonitorLoading: false,
                    showMonitorAddressesModal: false,
                    userQueryMonitors: [],
                    monitorPnl: {
                        day: { inflow: 0, outflow: 0, profit: 0 },
                        week: { inflow: 0, outflow: 0, profit: 0 },
                        month: { inflow: 0, outflow: 0, profit: 0 },
                        comparison: {
                            day_vs_prev_day: { current: 0, previous: 0, change: 0, change_percent: null },
                            week_vs_prev_week: { current: 0, previous: 0, change: 0, change_percent: null },
                            month_vs_prev_month: { current: 0, previous: 0, change: 0, change_percent: null }
                        },
                        series: { day: [], week: [], month: [] },
                        transactions: []
                    },
                    selectedMonitorIds: [],
                    monitorScopeMode: 'all',
                    chartPeriod: 'day',
                    exportAnnotatedOnly: false,
                    // 企业报表：对方地址别名
                    counterpartyAliases: [],
                    showAliasModal: false,
                    aliasForm: { address: '', name: '', note: '' },
                    aliasLoading: false,
                    // 年账单
                    yearSummary: null,
                    yearSummaryYear: new Date().getFullYear(),
                    isYearSummaryLoading: false,
                    showYearSummaryModal: false,
                    // 地址标签页切换
                    activeMonitorId: null,

                    // 2026 规格：潜在用户
                    potentialUsers: [],
                    potentialTotal: 0,
                    potentialOffset: 0,
                    potentialLimit: 12,
                    potentialFilters: {
                        keyword: '',
                        group_id: '',
                        tag: '',
                        start_time: '',
                        end_time: ''
                    },
                    availableTags: [], // 存储从 API 获取的所有可用标签
                    isSyncing: false,
                    isPotentialLoading: false,

                    // 2026 规格：群组管理
                    groups: [],
                    groupTotal: 0,
                    groupPage: 1,
                    groupPageSize: 10,
                    isGroupLoading: false,
                    selectedGroupId: null,
                    showGroupMembersModal: false,
                    showVerifyConfigModal: false,
                    verifyConfigForm: { verify_enabled: false, verify_type: 'captcha', verify_timeout: 120, verify_channel_id: '', verify_questions: [] },
                    groupVerifyConfig: {},
                    groupMembers: [],
                    groupMemberSearchQuery: '',
                    selectedMemberIds: [],
                    isBatchMemberActionLoading: false,
                    batchActionResult: null,
                    batchOperationLogs: [],
                    isMembersLoading: false,
                    showGroupMessagesModal: false,
                    groupMessages: [],
                    isMessagesLoading: false,
                    isUsersLoading: false,
                    showGroupReplyModal: false,
                    groupReplyTarget: null,
                    groupReplyText: '',
                    isGroupReplySending: false,
                    memberActionBusy: {},
                    showAdminPermissionModal: false,
                    adminPermissionTarget: null,
                    adminPermissionLoading: false,
                    adminPermissionMeta: {
                        member_status: '',
                        is_admin: false,
                        is_owner: false,
                    },
                    adminPermissionForm: {
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
                    },

                    // 群聊工作台（P0）
                    groupSearchQuery: '',
                    selectedGroupRoomId: null,
                    selectedGroupRoomTitle: '',
                    selectedGroupRevokeSync: false,
                    revokeSyncChatId: null,
                    revokeSyncChatTitle: '',
                    revokeSyncChatLoading: false,
                    groupRoomMessages: [],
                    groupRoomAvatarMap: {}, // user_id -> avatar_file_id
                    groupAvatarMap: {}, // group_id -> avatar_file_id (群头像)
                    groupLastMessageMap: {}, // group_id -> {content, time}
                    groupRoomEntering: false, // 控制入场动画
                    groupRoomSwitching: false, // 控制切换遮罩
                    groupMemberCount: null, // 群实际总人数（Telegram API）
                    directUidInput: '', // 直接输入 UID 操作
                    directUidResult: null, // 直接 UID 操作结果
                    memberVisibleCount: 30, // 无限滚动：当前可见条数
                    mobilePanelView: 'list', // 手机端视图：list | detail
                    isGroupRoomLoading: false,
                    groupRoomInput: '',
                    groupRoomReplyingTo: null,
                    isGroupRoomSending: false
                }
            },
            computed: {
                filteredUsers() {
                    if (!this.searchQuery) return this.users
                    const q = this.searchQuery.toLowerCase()
                    return this.users.filter(u => 
                        String(u.from_userid).includes(q) || 
                        (u.username && u.username.toLowerCase().includes(q))
                    )
                },
                broadcastUsers() {
                    const map = new Map()
                    this.users.forEach(u => {
                        if (!map.has(u.from_userid)) {
                            map.set(u.from_userid, u)
                        }
                    })
                    return Array.from(map.values())
                },
                filteredBroadcastTargets() {
                    return (this.broadcastTargets || []).filter(t => !!t.can_broadcast)
                },
                selectedMonitorAddresses() {
                    if (this.monitorScopeMode === 'all') return [];
                    return this.monitors
                        .filter(m => this.selectedMonitorIds.includes(m.monitor_id))
                        .map(m => m.address);
                },
                chartBars() {
                    const rows = (this.monitorPnl.series && this.monitorPnl.series[this.chartPeriod]) || [];
                    let maxVal = 1;
                    rows.forEach(r => {
                        maxVal = Math.max(maxVal, Number(r.inflow || 0), Number(r.outflow || 0));
                    });
                    return rows.map(r => ({
                        ...r,
                        inHeight: Math.max(4, Math.round((Number(r.inflow || 0) / maxVal) * 140)),
                        outHeight: Math.max(4, Math.round((Number(r.outflow || 0) / maxVal) * 140)),
                    }));
                },
                selectedMonitorDetails() {
                    const idSet = new Set(this.selectedMonitorIds || []);
                    const picked = this.monitors.filter(m => idSet.has(m.monitor_id));
                    return picked.map(m => {
                        const txs = (this.monitorPnl.transactions || []).filter(tx => tx.monitor_id === m.monitor_id);
                        const inflow = txs.filter(tx => tx.direction === 'in').reduce((s, tx) => s + Number(tx.amount || 0), 0);
                        const outflow = txs.filter(tx => tx.direction === 'out').reduce((s, tx) => s + Number(tx.amount || 0), 0);
                        return {
                            ...m,
                            period_inflow: inflow,
                            period_outflow: outflow,
                            period_profit: inflow - outflow,
                            tx_count: txs.length,
                        };
                    });
                },
                selectedExpenseTransactions() {
                    const idSet = new Set(this.selectedMonitorIds || []);
                    return (this.monitorPnl.transactions || [])
                        .filter(tx => idSet.has(tx.monitor_id) && tx.direction === 'out')
                        .sort((a, b) => Number(b.tx_timestamp || 0) - Number(a.tx_timestamp || 0));
                },
                filteredGroupRooms() {
                    if (!this.groupSearchQuery) return this.groups;
                    const q = String(this.groupSearchQuery).toLowerCase();
                    return this.groups.filter(g => {
                        const title = String(g.group_title || '').toLowerCase();
                        return title.includes(q) || String(g.group_id || '').includes(q);
                    });
                },
                filteredGroupMembers() {
                    const rows = this.groupMembers || [];
                    const q = String(this.groupMemberSearchQuery || '').trim().toLowerCase();
                    if (!q) return rows;
                    return rows.filter(m => {
                        const username = String(m.username || '').toLowerCase();
                        const first = String(m.first_name || '').toLowerCase();
                        const last = String(m.last_name || '').toLowerCase();
                        const uid = String(m.user_id || '');
                        const status = String(m.status || '').toLowerCase();
                        return username.includes(q) || first.includes(q) || last.includes(q) || uid.includes(q) || status.includes(q);
                    });
                },
                visibleGroupMembers() {
                    return this.filteredGroupMembers.slice(0, this.memberVisibleCount);
                },
                isAllFilteredMembersSelected() {
                    const rows = this.filteredGroupMembers || [];
                    if (!rows.length) return false;
                    const selected = new Set(this.selectedMemberIds || []);
                    return rows.every(m => selected.has(m.user_id));
                },
                selectedAdminCount() {
                    const selected = new Set(this.selectedMemberIds || []);
                    return (this.groupMembers || []).filter(m => {
                        if (!selected.has(m.user_id)) return false;
                        const status = String(m.status || '').toLowerCase();
                        return status === 'admin' || status === 'administrator' || status === 'creator';
                    }).length;
                },
                selectedNormalCount() {
                    const selected = new Set(this.selectedMemberIds || []);
                    return (this.groupMembers || []).filter(m => {
                        if (!selected.has(m.user_id)) return false;
                        const status = String(m.status || '').toLowerCase();
                        return status !== 'admin' && status !== 'administrator' && status !== 'creator';
                    }).length;
                }
            },
            mounted() {
                // 恢复持久化状态
                const savedTab = localStorage.getItem('activeTab');
                if (savedTab) this.activeTab = savedTab;
                
                // 确保所有弹窗初始关闭
                this.showAdminPermissionModal = false;
                this.showGroupMembersModal = false;
                this.showMonitorAddressesModal = false;
                this.showAddMonitorModal = false;
                this.adminPermissionLoading = false;
                
                const savedUserId = localStorage.getItem('selectedUserId');
                const savedUserName = localStorage.getItem('selectedUserName');
                if (savedUserId) {
                    this.selectedUserId = parseInt(savedUserId);
                    this.selectedUserName = savedUserName;
                    this.selectUser(this.selectedUserId, this.selectedUserName);
                }

                this.loadPotentialUsers()
                setInterval(() => this.loadPotentialUsers(), 10000)
                this.loadMonitors();
                this.loadMonitorPnlSummary()
                this.loadGroups()
                this.loadRevokeSyncChatInfo()
                this.fetchUsers()
                setInterval(this.fetchUsers, 5000)
                // 群组工作台自动轮询（5秒刷新一次，仅在已选中群时有效）
                setInterval(() => { if (this.selectedGroupRoomId) this.refreshGroupRoomMessages(); }, 5000)
                // 群组列表每 15 秒刷新一次
                setInterval(() => this.loadGroups(), 15000)
                this.fetchConfig()
                this.initWebSocket()
                this.loadAvailableTags()
                this.loadBroadcastTargets()
                this.loadPushTasks()
            },
            watch: {
                groupMemberSearchQuery() {
                    // 搜索变化时重置无限滚动
                    this.memberVisibleCount = 30;
                    this.$nextTick(() => this._setupMemberScrollObserver());
                },
                showGroupMembersModal(val) {
                    if (val) {
                        this.memberVisibleCount = 30;
                        this.$nextTick(() => this._setupMemberScrollObserver());
                    } else {
                        if (this._memberObserver) { this._memberObserver.disconnect(); this._memberObserver = null; }
                    }
                },
                activeTab(newTab) {
                    localStorage.setItem('activeTab', newTab);
                    this.mobilePanelView = 'list'; // 切换 tab 时重置手机端视图
                    // 切换到群聊工作台时，预加载各群最新一条消息
                    if (newTab === 'group_management') {
                        this.preloadGroupLastMessages();
                    }
                },
                selectedUserId(newId) {
                    if (newId) localStorage.setItem('selectedUserId', newId);
                    else localStorage.removeItem('selectedUserId');
                },
                selectedUserName(newName) {
                    if (newName) localStorage.setItem('selectedUserName', newName);
                    else localStorage.removeItem('selectedUserName');
                }
            },
            methods: {
                // --- Core Methods (monitor/group/chat/broadcast in modules/) ---
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
                async fetchUsersAsync() {
                    this.isUsersLoading = true
                    try {
                        const res = await fetch('/api/users')
                        const json = await res.json()
                        if (json.status === 'ok') {
                            this.users = json.data
                            // 自动加载列表中的用户头像
                            this.users.forEach(u => {
                                if (u.avatar_file_id && !this.fileUrls[u.avatar_file_id] && !this.fileErrors[u.avatar_file_id]) {
                                    this.autoLoadFile(u.avatar_file_id)
                                }
                            })
                        }
                    } catch (e) { console.error("Fetch users failed", e) }
                    finally { this.isUsersLoading = false }
                },
                getSelectedUserAvatar() {
                    const user = this.users.find(u => u.from_userid === this.selectedUserId)
                    return user ? user.avatar_file_id : null
                },
                async fetchConfig() {
                    try {
                        const res = await fetch('/api/config')
                        const json = await res.json()
                        if (json.status === 'ok') {
                            this.configData = json.data
                            this.enableTranslation = json.data.global_auto_translate === 'true'
                        }
                    } catch (e) { console.error("Fetch config failed", e) }
                },
                async selectUser(userId, userName) {
                    // 切换用户前保存当前滚动位置
                    if (this.selectedUserId) {
                        const el = document.getElementById('chatHistory')
                        if (el) this.scrollCache[this.selectedUserId] = el.scrollTop
                    }

                    this.selectedUserId = userId
                    this.selectedUserName = userName
                    this.page = 1
                    this.replyingTo = null 
                    this.newMsgCount = 0 // 切换用户重置未读数
                    
                    await this.fetchMessages(userId)
                    
                    // 恢复滚动位置或滚到底部
                    this.$nextTick(() => {
                        setTimeout(() => {
                            const el = document.getElementById('chatHistory')
                            if (el) {
                                if (this.scrollCache[userId] !== undefined) {
                                    el.scrollTop = this.scrollCache[userId]
                                } else {
                                    el.scrollTop = el.scrollHeight
                                }
                                // 更新智能滚动状态
                                const threshold = 100
                                this.isNearBottom = (el.scrollHeight - el.scrollTop - el.clientHeight) < threshold
                            }
                        }, 50)
                    })

                    if (this.pollTimer) clearInterval(this.pollTimer)
                    this.pollTimer = setInterval(() => this.pollMessages(userId), 2000)

                    // 自动标记已读
                    const user = this.users.find(u => u.from_userid === userId)
                    if(user && user.unread_count > 0) {
                        this.markAsRead()
                    }
                },
                handleScroll(e) {
                    const el = e.target
                    const threshold = 100
                    this.isNearBottom = (el.scrollHeight - el.scrollTop - el.clientHeight) < threshold
                    if (this.isNearBottom) this.newMsgCount = 0
                },
                async fetchMessages(userId) {
                    try {
                        const res = await fetch(`/api/messages/${userId}?page=${this.page}`)
                        const json = await res.json()
                        if (json.status === 'ok') {
                            this.messages = json.data
                            this.autoLoadMedia()
                        }
                    } catch (e) { console.error("Fetch messages failed", e) }
                },
                async pollMessages(userId) {
                    try {
                        const res = await fetch(`/api/messages/${userId}?page=${this.page}`)
                        const json = await res.json()
                        if (json.status === 'ok') {
                            const realMsgs = json.data
                            const oldRealCount = this.messages.filter(m => !String(m.msg_id).startsWith('tmp-')).length
                            this.messages = realMsgs
                            
                            if (realMsgs.length > oldRealCount) {
                                if (this.isNearBottom) {
                                    this.scrollToBottom(true)
                                    this.markAsRead()
                                } else {
                                    this.newMsgCount += (realMsgs.length - oldRealCount)
                                }
                            } else if (oldRealCount === 0) {
                                this.scrollToBottom(true)
                            }
                            this.autoLoadMedia()
                        }
                    } catch (e) { console.error("Poll messages failed", e) }
                },
                autoLoadMedia() {
                    this.messages.forEach(msg => {
                        if (msg.file_id && !this.fileUrls[msg.file_id] && !this.fileErrors[msg.file_id]) {
                            this.autoLoadFile(msg.file_id)
                        }
                        if (msg.quoted_file_id && !this.fileUrls[msg.quoted_file_id] && !this.fileErrors[msg.quoted_file_id]) {
                            this.autoLoadFile(msg.quoted_file_id)
                        }
                    })
                },
                async autoLoadFile(fileId) {
                    try {
                        const res = await fetch(`/api/file/${fileId}`)
                        const json = await res.json()
                        if (json.status === 'ok') {
                            this.fileUrls = { ...this.fileUrls, [fileId]: json.url }
                        } else {
                            this.fileErrors = { ...this.fileErrors, [fileId]: true }
                        }
                    } catch (e) { this.fileErrors = { ...this.fileErrors, [fileId]: true } }
                },
                async loadFile(fileId) {
                    try {
                        const res = await fetch(`/api/file/${fileId}`)
                        const json = await res.json()
                        if (json.status === 'ok') {
                            this.fileUrls = { ...this.fileUrls, [fileId]: json.url }
                            this.scrollToBottom()
                        } else alert("获取失败：" + json.message)
                    } catch (e) { alert("获取出错") }
                },
                scrollToBottom(force = false) {
                    if (force || this.isNearBottom) {
                        this.$nextTick(() => {
                            setTimeout(() => {
                                const el = document.getElementById('chatHistory')
                                if (el) {
                                    el.scrollTop = el.scrollHeight
                                    this.isNearBottom = true
                                    this.newMsgCount = 0
                                }
                            }, 50)
                        })
                    }
                },
                scrollToMessage(tgMsgId) {
                    if (!tgMsgId) return
                    const el = document.getElementById('msg-' + tgMsgId)
                    if (el) {
                        el.scrollIntoView({ behavior: 'smooth', block: 'center' })
                        this.isNearBottom = false
                        el.classList.add('highlight-anim')
                        setTimeout(() => el.classList.remove('highlight-anim'), 3000)
                    } else alert("原消息可能在更早的历史记录中")
                },
                isVideoSticker(url) {
                    if (!url) return false;
                    const cleanUrl = String(url).split('?')[0].toLowerCase();
                    return cleanUrl.endsWith('.webm') || cleanUrl.endsWith('.mp4');
                },
                isStickerPlaceholder(msg) {
                    if (!msg || msg.type !== 'sticker') return false;
                    const text = String(msg.content || '').trim().toLowerCase();
                    return text === '[sticker]' || text === '[表情包]' || text.startsWith('[表情包:');
                },
                initViewer() {
                    this.$nextTick(() => {
                        const container = document.getElementById('chatHistory')
                        if (!container) return
                        if (this.viewer) this.viewer.update()
                        else {
                            this.viewer = new Viewer(container, {
                                filter: (image) => image.tagName === 'IMG' && !image.classList.contains('flex-shrink-0'),
                                title: false, navbar: false, toolbar: { zoomIn: 1, zoomOut: 1, oneToOne: 1, reset: 1, prev: 1, next: 1, rotateLeft: 1, rotateRight: 1 }
                            })
                        }
                    })
                },
                getBubbleStyle(msg) {
                    const raw = String((msg?.original_text || msg?.translated_text || msg?.content || '')).trim()
                    const len = raw.length
                    const hasMedia = msg?.type && msg.type !== 'text'
                    const hasTranslation = !!(msg?.translated_text && msg?.original_text)

                    let minCh = 11
                    let maxCh = 32

                    if (len <= 8) maxCh = 18
                    else if (len <= 20) maxCh = 24
                    else if (len <= 48) maxCh = 30
                    else if (len <= 90) maxCh = 36
                    else if (len <= 180) maxCh = 42
                    else maxCh = 50

                    if (hasTranslation) {
                        minCh += 2
                        maxCh += 7
                    }
                    if (hasMedia) {
                        minCh = Math.max(minCh, 18)
                        maxCh = Math.max(maxCh, 34)
                    }

                    return {
                        '--bubble-min-ch': `${minCh}ch`,
                        '--bubble-max-ch': `${maxCh}ch`
                    }
                },
                formatTime(iso) {
                    if (!iso) return ''
                    let d
                    if (typeof iso === 'number' || (typeof iso === 'string' && /^\d{10,13}$/.test(iso.toString().trim()))) {
                        // Unix 时间戳（秒或毫秒）直接解析，天然 UTC 无歧义
                        const ts = Number(iso)
                        d = new Date(ts < 1e12 ? ts * 1000 : ts)
                    } else {
                        // 数据库字符串（如 "2026-03-25 02:09:00"）无时区标识
                        // 服务器在北京时间（UTC+8）运行，补上 +08:00 避免浏览器误解为 UTC
                        const s = String(iso).trim().replace(' ', 'T')
                        d = new Date(s.includes('+') || s.includes('Z') ? s : s + '+08:00')
                    }
                    if (isNaN(d.getTime())) return String(iso)
                    return d.toLocaleString('zh-CN', { month: '2-digit', day: '2-digit', hour: '2-digit', minute: '2-digit' })
                },
                formatShortTime(iso) {
                    if (!iso) return ''
                    const d = new Date(iso)
                    return d.getHours().toString().padStart(2, '0') + ':' + d.getMinutes().toString().padStart(2, '0')
                },
                prepareReply(msg) {
                    this.replyingTo = msg
                    this.$nextTick(() => document.querySelector('textarea')?.focus())
                },
                cancelReply() { this.replyingTo = null },
                triggerFileUpload() { this.$refs.fileInput.click() },
                handleImageSelect(e) {
                    const file = e.target.files[0]
                    if (!file) return
                    this.selectedImage = file
                    const reader = new FileReader()
                    reader.onload = (e) => this.selectedImagePreview = e.target.result
                    reader.readAsDataURL(file)
                },
                removeSelectedImage() {
                    this.selectedImage = null
                    this.selectedImagePreview = null
                    this.$refs.fileInput.value = ''
                },
                async sendMessage() {
                    if ((!this.inputText.trim() && !this.selectedImage) || !this.selectedUserId) return
                    
                    const userId = this.selectedUserId
                    const text = this.inputText.trim()
                    const image = this.selectedImage
                    const replyId = this.replyingTo ? this.replyingTo.tg_msg_id : null
                    
                    // 乐观更新
                    const tmpId = 'tmp-' + Date.now()
                    const optimisticMsg = {
                        msg_id: tmpId,
                        from_userid: 0,
                        content: text || (image ? '[图片]' : ''),
                        timestamp: new Date().toISOString(),
                        status: 'sending',
                        type: image ? 'photo' : 'text',
                        quoted_text: this.replyingTo ? (this.replyingTo.content || '[媒体]') : null,
                        quoted_msg_id: replyId
                    }
                    this.messages.push(optimisticMsg)
                    this.scrollToBottom(true)
                    
                    this.inputText = ''
                    this.replyingTo = null
                    const imageToUpload = this.selectedImage
                    const doTranslate = this.enableTranslation
                    this.removeSelectedImage()

                    try {
                        let res
                        if (imageToUpload) {
                            const formData = new FormData()
                            formData.append('image', imageToUpload)
                            formData.append('content', text)
                            if (replyId) formData.append('reply_to_msg_id', replyId)
                            formData.append('do_translate', doTranslate)
                            res = await fetch(`/api/messages/${userId}`, { method: 'POST', body: formData })
                        } else {
                            res = await fetch(`/api/messages/${userId}`, {
                                method: 'POST',
                                headers: { 'Content-Type': 'application/json' },
                                body: JSON.stringify({ 
                                    content: text, 
                                    reply_to_msg_id: replyId,
                                    do_translate: doTranslate
                                })
                            })
                        }
                        const json = await res.json()
                        if (json.status !== 'ok') alert("发送失败: " + json.message)
                        this.pollMessages(userId)
                    } catch (e) { alert("网络错误，发送失败") }
                },
                async markAsRead() {
                    if (!this.selectedUserId) return
                    try {
                        await fetch(`/api/mark_read/${this.selectedUserId}`, { method: 'POST' })
                        this.fetchUsers()
                    } catch (e) { console.error(e) }
                },
                async deleteChat() {
                    if (!this.selectedUserId || !confirm("确定要删除该用户的本地所有聊天记录吗？（不会影响 Telegram 上的消息）")) return
                    try {
                        const res = await fetch(`/api/delete_chat/${this.selectedUserId}`, { method: 'POST' })
                        if ((await res.json()).status === 'ok') {
                            this.messages = []
                            this.fetchUsers()
                        }
                    } catch (e) { console.error(e) }
                },
                // 群发功能方法 (2026)
                toggleUserSelection(userId) {
                    const idx = this.broadcastForm.selectedUserIds.indexOf(userId)
                    if (idx > -1) this.broadcastForm.selectedUserIds.splice(idx, 1)
                    else this.broadcastForm.selectedUserIds.push(userId)
                },
                toggleBroadcastTargetSelection(chatId) {
                    const idx = this.broadcastForm.selectedChatIds.indexOf(chatId)
                    if (idx > -1) this.broadcastForm.selectedChatIds.splice(idx, 1)
                    else this.broadcastForm.selectedChatIds.push(chatId)
                },
                selectAllUsers() {
                    this.broadcastForm.selectedUserIds = this.broadcastUsers.map(u => u.from_userid)
                },
                selectAllBroadcastTargets() {
                    this.broadcastForm.selectedChatIds = this.filteredBroadcastTargets.map(t => t.chat_id)
                },
                showToast(message, type) {
                    alert(message);
                },
                closeSettings() { this.showSettings = false },
                async saveSettings() {
                    try {
                        const res = await fetch('/api/config', { 
                            method: 'POST', 
                            headers: { 'Content-Type': 'application/json' }, 
                            body: JSON.stringify({ ...this.configData, global_auto_translate: this.enableTranslation.toString() }) 
                        })
                        if ((await res.json()).status === 'ok') { 
                            alert("配置已成功保存并实时生效！")
                            this.closeSettings() 
                        }
                    } catch (e) { console.error(e) }
                }
            }
        })

    // 延迟挂载，等登录验证完成后由 mountApp() 调用
    window._vueApp = app;
    window.mountApp = function() {
        if (!window._vueMounted) {
            window._vueMounted = true;
            app.mount('#app');
        }
    };
    