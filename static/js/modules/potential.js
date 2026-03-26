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
