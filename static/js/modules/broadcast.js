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
                
