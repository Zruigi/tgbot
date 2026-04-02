// 这个文件包含 Vue 应用的主要逻辑
// 由于原始代码在 admin_new.html 中，这里只添加新增的未关注用户功能

// 在现有的 data() 中添加：
// showUnverifiedUsersModal: false,
// unverifiedUsersForModal: [],
// unverifiedUsersLoading: false,
// selectedUnverifiedUsers: new Set(),
// isSendingUnverifiedReminders: false,
// currentUnverifiedGroupId: null,

// 在现有的 methods 中添加以下方法：

window._unverifiedUsersMethods = {
  async openUnverifiedUsersModal(groupId) {
    this.currentUnverifiedGroupId = groupId
    this.showUnverifiedUsersModal = true
    this.unverifiedUsersLoading = true
    this.selectedUnverifiedUsers.clear()
    try {
      const res = await fetch(`/api/group_listen/unverified_users?group_id=${groupId}`, {
        headers: { 'Authorization': `Bearer ${localStorage.getItem('admin_token') || ''}` }
      })
      const data = await res.json()
      if (data.status === 'ok') {
        this.unverifiedUsersForModal = data.data || []
        console.log(`已加载 ${this.unverifiedUsersForModal.length} 个未关注用户`)
      }
    } catch (e) {
      console.error('加载未关注用户失败:', e)
    } finally {
      this.unverifiedUsersLoading = false
    }
  },

  toggleUnverifiedUserSelection(userId) {
    if (this.selectedUnverifiedUsers.has(userId)) {
      this.selectedUnverifiedUsers.delete(userId)
    } else {
      this.selectedUnverifiedUsers.add(userId)
    }
  },

  selectAllUnverifiedUsers() {
    if (this.selectedUnverifiedUsers.size === this.unverifiedUsersForModal.length) {
      this.selectedUnverifiedUsers.clear()
    } else {
      this.unverifiedUsersForModal.forEach(u => this.selectedUnverifiedUsers.add(u.user_id))
    }
  },

  async sendUnverifiedReminders() {
    if (this.selectedUnverifiedUsers.size === 0 || !this.currentUnverifiedGroupId) return
    
    this.isSendingUnverifiedReminders = true
    try {
      const channelLink = prompt('请输入频道链接 (例如: @mychannel 或 https://t.me/mychannel):')
      if (!channelLink) return
      
      const res = await fetch('/api/group_listen/send_channel_reminder', {
        method: 'POST',
        headers: {
          'Authorization': `Bearer ${localStorage.getItem('admin_token') || ''}`,
          'Content-Type': 'application/json'
        },
        body: JSON.stringify({
          group_id: this.currentUnverifiedGroupId,
          user_ids: Array.from(this.selectedUnverifiedUsers),
          channel_link: channelLink
        })
      })
      
      const data = await res.json()
      if (data.status === 'ok') {
        alert(`已发送 ${data.sent_count} 条提醒消息`)
        this.selectedUnverifiedUsers.clear()
        await this.openUnverifiedUsersModal(this.currentUnverifiedGroupId)
      } else {
        alert(`发送失败: ${data.message}`)
      }
    } catch (e) {
      console.error('发送提醒失败:', e)
      alert('发送失败，请查看控制台')
    } finally {
      this.isSendingUnverifiedReminders = false
    }
  }
}
