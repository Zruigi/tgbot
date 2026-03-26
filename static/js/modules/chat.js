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
