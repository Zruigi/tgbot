
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
                // ======= MODULE: MONITOR =======
                async loadMonitors() {
                    this.isMonitorLoading = true;
                    try {
                        const res = await fetch('/api/monitors');
                        const json = await res.json();
                        if (json.status === 'ok') {
                            this.monitors = json.data;
                            // 默认选中第一个地址
                            if (json.data.length > 0 && !this.activeMonitorId) {
                                this.activeMonitorId = json.data[0].monitor_id;
                            }
                        }
                    } catch (e) { console.error(e) }
                    finally { this.isMonitorLoading = false; }
                    this.loadUserQueryMonitors();
                    this.loadCounterpartyAliases();
                    this.loadLedger();
                },
                selectMonitor(m) {
                    this.activeMonitorId = m.monitor_id;
                    this.activeMonitor = m;
                    this.ledgerPage = 1;
                    this.loadLedger();
                },
                selectAllMonitors() {
                    this.activeMonitorId = null;
                    this.activeMonitor = null;
                    this.ledgerPage = 1;
                    this.loadLedger();
                },
                async loadLedger() {
                    this.ledgerLoading = true;
                    try {
                        const params = new URLSearchParams({
                            period: this.ledgerPeriod,
                            page: this.ledgerPage,
                            page_size: 50,
                        });
                        if (this.activeMonitorId !== null) params.set('monitor_id', this.activeMonitorId);
                        if (this.ledgerDirection) params.set('direction', this.ledgerDirection);
                        const res = await fetch(`/api/monitors/transactions?${params}`);
                        const json = await res.json();
                        if (json.status === 'ok') {
                            this.ledgerTxs = (json.txs || []).map(tx => ({...tx, _open: false}));
                            this.ledgerSummary = json.summary || { inflow:0, outflow:0, profit:0, total_count:0 };
                            this.ledgerTotal = json.total || 0;
                            this.ledgerPage = json.page || 1;
                            this.ledgerPages = json.pages || 1;
                            const aliasMap = {};
                            this.counterpartyAliases.forEach(a => { aliasMap[a.address] = a.name; });
                            this.ledgerTxs.forEach(tx => {
                                if (!tx.payer && tx.direction==='in' && aliasMap[tx.counterparty]) tx.payer = aliasMap[tx.counterparty];
                                if (!tx.payee && tx.direction==='out' && aliasMap[tx.counterparty]) tx.payee = aliasMap[tx.counterparty];
                            });
                        }
                    } catch(e): console.error(e);
                    finally: this.ledgerLoading = false;
                },
                async loadUserQueryMonitors() {
                    try {
                        const res = await fetch('/api/monitors/user_queries');
                        const json = await res.json();
                        if (json.status === 'ok') this.userQueryMonitors = json.data;
                    } catch (e) { console.error(e); }
                },
                async claimMonitor(m) {
                    if (!confirm(`将地址 ${m.address} 转为您的监听地址？`)) return;
                    try {
                        const res = await fetch(`/api/monitors/${m.monitor_id}/claim`, { method: 'POST' });
                        const json = await res.json();
                        if (json.status === 'ok') {
                            this.userQueryMonitors = this.userQueryMonitors.filter(x => x.monitor_id !== m.monitor_id);
                            this.loadMonitors();
                        } else { alert('操作失败: ' + json.message); }
                    } catch (e) { console.error(e); }
                },
                async loadMonitorTxs(m) {
                    try {
                        const res = await fetch(`/api/monitors/pnl_summary?monitor_ids=${m.monitor_id}&addresses=${encodeURIComponent(m.address)}`);
                        const json = await res.json();
                        m._txs = (json.data && json.data.transactions) ? json.data.transactions : [];
                        // 自动用别名填充付款方/收款方
                        const aliasMap = {};
                        this.counterpartyAliases.forEach(a => { aliasMap[a.address] = a.name; });
                        m._txs.forEach(tx => {
                            if (!tx.payer && tx.direction === 'in' && aliasMap[tx.counterparty]) tx.payer = aliasMap[tx.counterparty];
                            if (!tx.payee && tx.direction === 'out' && aliasMap[tx.counterparty]) tx.payee = aliasMap[tx.counterparty];
                        });
                    } catch(e) { m._txs = []; console.error(e); }
                },
                // 快速从交易记录里保存对方地址别名
                async saveCounterpartyAlias(address) {
                    if (!address) return;
                    const name = prompt(`为地址 ${address.slice(0,12)}... 设置备注名：`);
                    if (!name) return;
                    try {
                        const res = await fetch('/api/monitors/counterparty_aliases', {
                            method: 'POST',
                            headers: {'Content-Type': 'application/json'},
                            body: JSON.stringify({ address, name })
                        });
                        const json = await res.json();
                        if (json.status === 'ok') {
                            await this.loadCounterpartyAliases();
                            // 刷新所有已展开的交易记录
                            this.monitors.forEach(m => { if (m._txs) this.loadMonitorTxs(m); });
                        }
                    } catch(e) { console.error(e); }
                },
                async loadCounterpartyAliases() {
                    try {
                        const res = await fetch('/api/monitors/counterparty_aliases');
                        const json = await res.json();
                        if (json.status === 'ok') this.counterpartyAliases = json.data;
                    } catch(e) { console.error(e); }
                },
                async deleteCounterpartyAlias(aliasId) {
                    if (!confirm('确认删除此地址备注？')) return;
                    try {
                        const res = await fetch(`/api/monitors/counterparty_aliases/${aliasId}`, { method: 'DELETE' });
                        if ((await res.json()).status === 'ok') this.loadCounterpartyAliases();
                    } catch(e) { console.error(e); }
                },
                async saveCounterpartyAliasDirect() {
                    const address = (this.aliasForm.address || '').trim();
                    const name = (this.aliasForm.name || '').trim();
                    if (!address || !name) { alert('地址和备注名不能为空'); return; }
                    this.aliasLoading = true;
                    try {
                        const res = await fetch('/api/monitors/counterparty_aliases', {
                            method: 'POST',
                            headers: {'Content-Type': 'application/json'},
                            body: JSON.stringify({ address, name, note: this.aliasForm.note || '' })
                        });
                        const json = await res.json();
                        if (json.status === 'ok') {
                            this.aliasForm = { address: '', name: '', note: '' };
                            await this.loadCounterpartyAliases();
                            this.monitors.forEach(m => { if (m._txs) this.loadMonitorTxs(m); });
                        } else { alert('保存失败: ' + json.message); }
                    } catch(e) { console.error(e); }
                    finally { this.aliasLoading = false; }
                },
                async loadYearSummary() {
                    this.isYearSummaryLoading = true;
                    try {
                        const params = new URLSearchParams({ year: this.yearSummaryYear });
                        if (this.monitorScopeMode !== 'all' && this.selectedMonitorIds.length > 0) {
                            params.set('monitor_ids', this.selectedMonitorIds.join(','));
                        }
                        const res = await fetch(`/api/monitors/year_summary?${params}`);
                        const json = await res.json();
                        if (json.status === 'ok') this.yearSummary = json;
                    } catch(e) { console.error(e); }
                    finally { this.isYearSummaryLoading = false; }
                },
                async loadMonitorPnlSummary() {
                    try {
                        const params = new URLSearchParams();
                        if (this.monitorScopeMode !== 'all' && this.selectedMonitorIds.length > 0) {
                            params.set('monitor_ids', this.selectedMonitorIds.join(','));
                            params.set('addresses', this.selectedMonitorAddresses.join(','));
                        }
                        const q = params.toString();
                        const res = await fetch(`/api/monitors/pnl_summary${q ? ('?' + q) : ''}`);
                        const json = await res.json();
                        if (json.status === 'ok') this.monitorPnl = json.data;
                    } catch (e) { console.error(e) }
                },
                setMonitorScopeAll() {
                    this.monitorScopeMode = 'all';
                    this.selectedMonitorIds = [];
                    this.loadMonitorPnlSummary();
                },
                toggleMonitorSelection(monitorId) {
                    const idx = this.selectedMonitorIds.indexOf(monitorId);
                    if (idx >= 0) this.selectedMonitorIds.splice(idx, 1);
                    else this.selectedMonitorIds.push(monitorId);
                    this.monitorScopeMode = this.selectedMonitorIds.length > 0 ? 'selected' : 'all';
                    this.loadMonitorPnlSummary();
                },
                async exportMonitorExcel(period = 'month') {
                    try {
                        const params = new URLSearchParams({ period, annotated_only: String(!!this.exportAnnotatedOnly) });
                        if (period === 'year') {
                            params.set('year', this.yearSummaryYear);
                        }
                        if (this.monitorScopeMode !== 'all' && this.selectedMonitorIds.length > 0) {
                            params.set('monitor_ids', this.selectedMonitorIds.join(','));
                            params.set('addresses', this.selectedMonitorAddresses.join(','));
                        }
                        const url = `/api/monitors/export?${params.toString()}`;
                        const res = await fetch(url);
                        if (!res.ok) {
                            const err = await res.json().catch(() => ({}));
                            alert('导出失败: ' + (err.message || res.status));
                            return;
                        }
                        const blob = await res.blob();
                        const a = document.createElement('a');
                        a.href = URL.createObjectURL(blob);
                        const cd = res.headers.get('Content-Disposition') || '';
                        const match = cd.match(/filename="([^"]+)"/);
                        a.download = match ? match[1] : `monitor_export_${period}.xlsx`;
                        document.body.appendChild(a);
                        a.click();
                        document.body.removeChild(a);
                        URL.revokeObjectURL(a.href);
                    } catch (e) {
                        console.error(e);
                        alert('导出失败，请稍后重试');
                    }
                },
                async saveAllMonitorTxMeta() {
                    const items = (this.monitorPnl.transactions || []).map(tx => ({
                        tx_id: tx.tx_id,
                        income_reason: tx.income_reason || '',
                        expense_reason: tx.expense_reason || '',
                        payer: tx.payer || '',
                        payee: tx.payee || ''
                    }));
                    if (!items.length) return;
                    try {
                        const res = await fetch('/api/monitors/tx_meta/batch', {
                            method: 'POST',
                            headers: { 'Content-Type': 'application/json' },
                            body: JSON.stringify({ items })
                        });
                        const json = await res.json();
                        if (json.status === 'ok') {
                            alert(`批量保存成功，已更新 ${json.updated} 条`);
                            this.loadMonitorPnlSummary();
                        } else {
                            alert('批量保存失败: ' + (json.message || '未知错误'));
                        }
                    } catch (e) {
                        console.error(e);
                        alert('批量保存失败，请稍后重试');
                    }
                },
                async updateMonitorTxMeta(tx) {
                    try {
                        const res = await fetch('/api/monitors/tx_meta', {
                            method: 'POST',
                            headers: { 'Content-Type': 'application/json' },
                            body: JSON.stringify({
                                tx_id: tx.tx_id,
                                income_reason: tx.income_reason || '',
                                expense_reason: tx.expense_reason || '',
                                payer: tx.payer || '',
                                payee: tx.payee || ''
                            })
                        });
                        const json = await res.json();
                        if (json.status !== 'ok') {
                            alert('保存流水备注失败: ' + (json.message || '未知错误'));
                        }
                    } catch (e) {
                        console.error(e);
                        alert('保存流水备注失败，请稍后重试');
                    }
                },
                async updateMonitorRemark(monitor, remark) {
                    try {
                        const res = await fetch(`/api/monitors/${monitor.monitor_id}/remark`, {
                            method: 'POST',
                            headers: { 'Content-Type': 'application/json' },
                            body: JSON.stringify({ remark })
                        });
                        const json = await res.json();
                        if (json.status === 'ok') monitor.remark = remark;
                    } catch (e) { console.error(e) }
                },
                async addMonitor() {
                    if (!this.newMonitor.address) return;
                    try {
                        const res = await fetch('/api/monitors', {
                            method: 'POST',
                            headers: { 'Content-Type': 'application/json' },
                            body: JSON.stringify(this.newMonitor)
                        });
                        const json = await res.json();
                        if (json.status === 'ok') {
                            this.showAddMonitorModal = false;
                            this.newMonitor.address = '';
                            this.newMonitor.remark = '';
                            this.loadMonitors();
                            this.loadMonitorPnlSummary();
                        } else {
                            alert('新增失败: ' + json.message);
                        }
                    } catch (e) { console.error(e) }
                },
                async deleteMonitor(id) {
                    if (!confirm('确定要删除该监听地址吗？')) return;
                    try {
                        const res = await fetch(`/api/monitors/${id}`, { method: 'DELETE' });
                        const json = await res.json();
                        if (json.status === 'ok') {
                            this.selectedMonitorIds = this.selectedMonitorIds.filter(x => x !== id);
                            if (this.selectedMonitorIds.length === 0) this.monitorScopeMode = 'all';
                            this.loadMonitors();
                            this.loadMonitorPnlSummary();
                        } else {
                            alert('删除失败: ' + json.message);
                        }
                    } catch (e) { console.error(e) }
                },

                // --- 潜在用户逻辑 ---

                // ======= MODULE: POTENTIAL =======
                async loadPotentialUsers() {
                    this.isPotentialLoading = true;
                    try {
                        const params = new URLSearchParams({
                            limit: this.potentialLimit,
                            offset: this.potentialOffset,
                            keyword: this.potentialFilters.keyword,
                            group_id: this.potentialFilters.group_id,
                            tag: this.potentialFilters.tag
                        });
                        const res = await fetch(`/api/potential_users?${params}`);
                        const json = await res.json();
                        if (json.status === 'ok') {
                            this.potentialUsers = json.data;
                            this.potentialTotal = json.total;
                        }
                    } catch (e) { console.error(e) }
                    finally { this.isPotentialLoading = false; }
                },
                async loadAvailableTags() {
                    try {
                        const res = await fetch('/api/potential_users/tags');
                        const json = await res.json();
                        if (json.status === 'ok') {
                            this.availableTags = json.data;
                        }
                    } catch (e) { console.error(e) }
                },
                async syncPotentialUsers() {
                    this.isSyncing = true;
                    try {
                        const res = await fetch('/api/potential_users/sync', { method: 'POST' });
                        const json = await res.json();
                        if (json.status !== 'ok') {
                            alert('同步失败: ' + (json.message || '未知错误'));
                            return;
                        }
                        alert(json.summary || '同步成功');
                        await this.loadPotentialUsers();
                        await this.loadAvailableTags();
                    } catch (e) {
                        console.error(e);
                        alert('同步失败，请稍后重试');
                    } finally { this.isSyncing = false; }
                },
                async promoteUser(userId) {
                    try {
                        const res = await fetch(`/api/potential_users/${userId}/promote`, { method: 'POST' });
                        const json = await res.json();
                        if (json.status === 'ok') {
                            alert(json.message || '转正成功');
                            this.loadPotentialUsers();
                            this.fetchUsers(); // 刷新会话列表
                        } else {
                            alert('操作失败: ' + json.message);
                        }
                    } catch (e) { console.error(e) }
                },
                handlePotentialPage(direction) {
                    if (direction === 'next' && this.potentialOffset + this.potentialLimit < this.potentialTotal) {
                        this.potentialOffset += this.potentialLimit;
                        this.loadPotentialUsers();
                    } else if (direction === 'prev' && this.potentialOffset > 0) {
                        this.potentialOffset -= this.potentialLimit;
                        this.loadPotentialUsers();
                    }
                },
                async sendInvite(userId) {
                    try {
                        const res = await fetch(`/api/messages/${userId}`, {
                            method: 'POST',
                            headers: { 'Content-Type': 'application/json' },
                            body: JSON.stringify({
                                content: '👋 你好！我们注意到你对我们的项目感兴趣。欢迎加入我们的社区！',
                                do_translate: false
                            })
                        })
                        const json = await res.json()
                        if (json.status === 'ok') alert('邀请已发送')
                        else alert('发送失败: ' + json.message)
                    } catch (e) {
                        alert('发送失败: ' + e.message)
                    }
                },

                // ======= MODULE: GROUP =======
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

                // ======= MODULE: CHAT =======
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

                // ======= MODULE: BROADCAST =======
                async loadBroadcastTargets() {
                    this.isBroadcastTargetsLoading = true
                    try {
                        const res = await fetch('/api/broadcast/targets')
                        const json = await res.json()
                        if (json.status === 'ok') this.broadcastTargets = json.data || []
                    } catch (e) { console.error(e) }
                    finally { this.isBroadcastTargetsLoading = false }
                },
                async syncBroadcastTargets() {
                    try {
                        const res = await fetch('/api/broadcast/targets/sync', { method: 'POST' })
                        const json = await res.json()
                        if (json.status === 'ok') {
                            this.broadcastTargets = json.data || []
                            alert(`同步完成：扫描 ${json.summary?.scanned || 0}，更新 ${json.summary?.updated || 0}`)
                        } else {
                            alert('同步失败: ' + (json.message || '未知错误'))
                        }
                    } catch (e) {
                        console.error(e)
                        alert('同步失败，请稍后重试')
                    }
                },
                async loadPushTasks() {
                    try {
                        const res = await fetch('/api/push_tasks')
                        const json = await res.json()
                        if (json.status === 'ok') this.pushTasks = json.data || []
                    } catch (e) { console.error(e) }
                },
                async createPushTask() {
                    if (!this.pushTaskForm.start_at) {
                        alert('请先选择开始时间')
                        return
                    }
                    if (this.broadcastForm.selectedChatIds.length === 0) {
                        alert('请先选择群/频道目标')
                        return
                    }
                    const draft = this.broadcastDrafts.find(d => d.id === this.activeBroadcastDraftId)
                    const content = draft ? (draft.content || '') : (this.broadcastComposer.content || '').trim()
                    if (!content) {
                        alert('定时任务暂仅支持文本内容，请先输入文本')
                        return
                    }
                    try {
                        const payload = {
                            task_name: this.pushTaskForm.task_name || `周期推送-${Date.now()}`,
                            content,
                            do_translate: this.broadcastForm.doTranslate,
                            target_chat_ids: this.broadcastForm.selectedChatIds,
                            start_at: new Date(this.pushTaskForm.start_at).toISOString(),
                            interval_hours: Number(this.pushTaskForm.interval_hours || 6)
                        }
                        const res = await fetch('/api/push_tasks', {
                            method: 'POST',
                            headers: { 'Content-Type': 'application/json' },
                            body: JSON.stringify(payload)
                        })
                        const json = await res.json()
                        if (json.status === 'ok') {
                            alert('周期任务已创建')
                            this.loadPushTasks()
                        } else {
                            alert('创建任务失败: ' + (json.message || '未知错误'))
                        }
                    } catch (e) {
                        console.error(e)
                        alert('创建任务失败，请稍后重试')
                    }
                },
                async updatePushTaskStatus(taskId, status) {
                    try {
                        const res = await fetch('/api/push_tasks/status', {
                            method: 'POST',
                            headers: { 'Content-Type': 'application/json' },
                            body: JSON.stringify({ task_id: taskId, status })
                        })
                        const json = await res.json()
                        if (json.status === 'ok') this.loadPushTasks()
                    } catch (e) { console.error(e) }
                },
                async runPushTaskNow(taskId) {
                    try {
                        const res = await fetch('/api/push_tasks/run_now', {
                            method: 'POST',
                            headers: { 'Content-Type': 'application/json' },
                            body: JSON.stringify({ task_id: taskId })
                        })
                        const json = await res.json()
                        if (json.status === 'ok') {
                            alert(`执行完成：成功 ${json.summary?.success || 0}，失败 ${json.summary?.failed || 0}`)
                            this.loadPushTasks()
                        } else {
                            alert('立即执行失败: ' + (json.message || '未知错误'))
                        }
                    } catch (e) { console.error(e) }
                },
                triggerBroadcastUpload() { this.$refs.broadcastFileInput.click() },
                handleBroadcastImageSelect(e) {
                    const file = e.target.files[0]
                    if (!file) return
                    this.broadcastComposer.image = file
                    const reader = new FileReader()
                    reader.onload = (e) => this.broadcastComposer.imagePreview = e.target.result
                    reader.readAsDataURL(file)
                },
                removeBroadcastImage() {
                    this.broadcastComposer.image = null
                    this.broadcastComposer.imagePreview = null
                    this.$refs.broadcastFileInput.value = ''
                },
                saveBroadcastDraft() {
                    const content = (this.broadcastComposer.content || '').trim()
                    if (!content && !this.broadcastComposer.imagePreview) {
                        alert('请先输入内容或上传图片后再保存草稿')
                        return
                    }

                    const draft = {
                        id: Date.now(),
                        content,
                        image: this.broadcastComposer.image,
                        imagePreview: this.broadcastComposer.imagePreview,
                        createdAt: new Date().toISOString()
                    }
                    this.broadcastDrafts.unshift(draft)
                    this.activeBroadcastDraftId = draft.id

                    this.broadcastComposer.content = ''
                    this.removeBroadcastImage()
                    alert('草稿已保存（未发送）')
                },
                activateBroadcastDraft(draftId) {
                    this.activeBroadcastDraftId = draftId
                },
                async submitBroadcast() {
                    if (this.broadcastForm.isSending) return
                    const draft = this.broadcastDrafts.find(d => d.id === this.activeBroadcastDraftId)

                    let sendContent = ''
                    let sendImage = null

                    if (draft) {
                        sendContent = draft.content || ''
                        sendImage = draft.image || null
                    } else {
                        sendContent = (this.broadcastComposer.content || '').trim()
                        sendImage = this.broadcastComposer.image || null
                    }

                    if (!sendContent && !sendImage) {
                        alert('请先输入内容或上传图片，或选择一条草稿后再发送')
                        return
                    }

                    if (this.broadcastForm.mode === 'chat') {
                        const dedupedChatIds = [...new Set(this.broadcastForm.selectedChatIds.map(id => Number(id)).filter(Boolean))]
                        if (dedupedChatIds.length === 0) {
                            alert('请选择至少一个群/频道目标')
                            return
                        }
                        this.broadcastForm.selectedChatIds = dedupedChatIds
                        this.broadcastForm.isSending = true
                        this.broadcastForm.progress = 0
                        this.broadcastForm.total = dedupedChatIds.length

                        const formData = new FormData()
                        formData.append('content', sendContent)
                        formData.append('do_translate', this.broadcastForm.doTranslate)
                        formData.append('chat_ids', JSON.stringify(dedupedChatIds))
                        formData.append('retry_count', String(this.broadcastForm.retryCount || 1))
                        if (sendImage) formData.append('image', sendImage)

                        try {
                            const res = await fetch('/api/broadcast/chats', { method: 'POST', body: formData })
                            const json = await res.json()
                            if (json.status !== 'ok') {
                                alert('群/频道群发失败: ' + (json.message || '未知错误'))
                            } else {
                                const s = json.summary || {}
                                alert(`发送完成：成功 ${s.success || 0}，失败 ${s.failed || 0}`)
                                this.loadPushTasks()
                            }
                        } catch (e) {
                            alert('网络错误，任务创建失败')
                        } finally {
                            this.broadcastForm.isSending = false
                            this.broadcastForm.progress = 0
                            this.broadcastForm.total = 0
                        }
                        return
                    }

                    if (this.broadcastForm.selectedUserIds.length === 0) return
                    const dedupedUserIds = [...new Set(this.broadcastForm.selectedUserIds.map(id => Number(id)).filter(Boolean))]
                    if (dedupedUserIds.length === 0) {
                        alert('请选择至少一个有效接收对象')
                        return
                    }

                    if (dedupedUserIds.length !== this.broadcastForm.selectedUserIds.length) {
                        const ok = confirm(`你当前选择了 ${this.broadcastForm.selectedUserIds.length} 项，其中存在重复用户。去重后将发送给 ${dedupedUserIds.length} 位用户，是否继续？`)
                        if (!ok) return
                    }

                    this.broadcastForm.selectedUserIds = dedupedUserIds
                    this.broadcastForm.isSending = true
                    this.broadcastForm.progress = 0
                    this.broadcastForm.total = dedupedUserIds.length
                    
                    const formData = new FormData()
                    formData.append('content', sendContent)
                    formData.append('do_translate', this.broadcastForm.doTranslate)
                    formData.append('user_ids', JSON.stringify(dedupedUserIds))
                    if (sendImage) {
                        formData.append('image', sendImage)
                    }

                    try {
                        const res = await fetch('/api/broadcast', {
                            method: 'POST',
                            body: formData
                        })
                        const json = await res.json()
                        if (json.status !== 'ok') {
                            alert('群发任务创建失败: ' + json.message)
                            this.broadcastForm.isSending = false
                            this.broadcastForm.total = 0
                            return
                        }

                        if (typeof json.count === 'number') {
                            this.broadcastForm.total = json.count
                        }

                        if (!this.broadcastForm.total || this.broadcastForm.total <= 0) {
                            this.broadcastForm.isSending = false
                            this.broadcastForm.progress = 0
                            alert('没有可发送的有效对象，请检查选择列表')
                            return
                        }
                    } catch (e) {
                        alert('网络错误，任务创建失败')
                        this.broadcastForm.isSending = false
                        this.broadcastForm.total = 0
                    }
                },
                

                // --- Core Methods ---
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
    