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
