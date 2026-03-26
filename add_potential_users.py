#!/usr/bin/env python
# -*- coding: utf-8 -*-

with open('admin_new.html', 'r', encoding='utf-8') as f:
    content = f.read()

# 1. 在HTML中添加潜在用户标签页按钮（在聊天界面之前）
tab_button = '''
                    <!-- Potential Users Tab -->
                    <button @click="currentView = 'potential_users'" 
                            :class="['px-4 py-2 rounded-lg font-bold transition-all', currentView === 'potential_users' ? 'bg-primary text-white' : 'bg-surface-container text-on-surface hover:bg-surface-container-high']">
                        <span class="material-symbols-outlined mr-2">person_add</span>
                        潜在用户
                    </button>
'''

# 找到聊天界面的开始位置，在其前面添加标签页按钮
chat_start = '<div class="flex-1 flex flex-col bg-surface relative">'
if chat_start in content and tab_button not in content:
    # 找到标签页按钮区域
    tab_area = content.find('<!-- Chat Header -->')
    if tab_area > 0:
        # 向前找到按钮区域
        button_area = content.rfind('</button>', 0, tab_area)
        if button_area > 0:
            # 在最后一个按钮后添加潜在用户按钮
            insert_pos = content.find('\n', button_area) + 1
            content = content[:insert_pos] + '                    ' + tab_button.strip() + '\n' + content[insert_pos:]

# 2. 在聊天界面外层添加潜在用户视图
potential_users_view = '''
                    <!-- Potential Users View -->
                    <div v-if="currentView === 'potential_users'" class="flex-1 flex flex-col bg-surface">
                        <div class="px-6 py-4 bg-surface-container-lowest/80 backdrop-blur-md border-b border-outline-variant/10">
                            <h2 class="text-lg font-bold text-on-surface">潜在用户 ({{ potentialUsers.length }})</h2>
                            <p class="text-xs text-slate-400 mt-1">从群组中自动捕捉的用户</p>
                        </div>
                        <div class="flex-1 overflow-y-auto p-6 space-y-3">
                            <div v-if="potentialUsers.length === 0" class="flex items-center justify-center h-full text-slate-400">
                                <div class="text-center">
                                    <span class="material-symbols-outlined text-5xl opacity-20 block mb-2">person_off</span>
                                    <p>暂无潜在用户</p>
                                </div>
                            </div>
                            <div v-for="user in potentialUsers" :key="user.user_id" 
                                 class="p-4 bg-surface-container-lowest rounded-xl border border-outline-variant/10 hover:border-primary/30 transition-all cursor-pointer hover:shadow-md"
                                 @click="selectedUserId = user.user_id; currentView = 'chat'">
                                <div class="flex items-start justify-between">
                                    <div class="flex-1">
                                        <div class="font-bold text-on-surface">{{ user.first_name }} {{ user.last_name }}</div>
                                        <div class="text-xs text-slate-400 mt-1">@{{ user.username }}</div>
                                        <div class="text-xs text-slate-500 mt-2">
                                            <span class="inline-block bg-primary/10 text-primary px-2 py-1 rounded mr-2">{{ user.keywords }}</span>
                                        </div>
                                        <div class="text-xs text-slate-400 mt-2">
                                            群组: {{ user.group_title }}
                                        </div>
                                        <div class="text-xs text-slate-400 mt-1">
                                            捕捉时间: {{ new Date(user.captured_at).toLocaleString() }}
                                        </div>
                                    </div>
                                    <button @click.stop="sendInvite(user.user_id)" 
                                            class="px-3 py-1 bg-primary text-white text-xs font-bold rounded-lg hover:bg-primary/80 transition-colors">
                                        邀请
                                    </button>
                                </div>
                            </div>
                        </div>
                    </div>
'''

# 在聊天界面前添加潜在用户视图
chat_view_start = '<div v-if="!selectedUserId" class="flex-1 flex flex-col items-center justify-center'
if chat_view_start in content and potential_users_view not in content:
    insert_pos = content.find(chat_view_start)
    content = content[:insert_pos] + potential_users_view + '\n\n                    ' + content[insert_pos:]

# 3. 在data中添加新的数据字段
data_section = 'data() { return {'
if data_section in content:
    insert_pos = content.find(data_section) + len(data_section)
    new_data = '''
            currentView: 'chat',
            potentialUsers: [],'''
    if 'currentView' not in content:
        content = content[:insert_pos] + new_data + content[insert_pos:]

# 4. 在methods中添加新的方法
methods_section = 'methods: {'
if methods_section in content:
    insert_pos = content.find(methods_section) + len(methods_section)
    new_methods = '''
                async loadPotentialUsers() {
                    try {
                        const res = await fetch('/api/potential_users')
                        const data = await res.json()
                        this.potentialUsers = data.users || []
                    } catch (e) {
                        console.error('Failed to load potential users:', e)
                    }
                },
                async sendInvite(userId) {
                    try {
                        await fetch(`/api/messages/${userId}`, {
                            method: 'POST',
                            headers: { 'Content-Type': 'application/json' },
                            body: JSON.stringify({
                                content: '👋 你好！我们注意到你对我们的项目感兴趣。欢迎加入我们的社区！',
                                do_translate: false
                            })
                        })
                        alert('邀请已发送')
                    } catch (e) {
                        alert('发送失败: ' + e.message)
                    }
                },'''
    if 'loadPotentialUsers' not in content:
        content = content[:insert_pos] + new_methods + content[insert_pos:]

# 5. 在mounted中添加加载潜在用户
mounted_section = 'mounted() {'
if mounted_section in content:
    insert_pos = content.find(mounted_section) + len(mounted_section)
    new_mounted = '''
            this.loadPotentialUsers()
            setInterval(() => this.loadPotentialUsers(), 5000)'''
    if 'loadPotentialUsers' not in content[insert_pos:insert_pos+200]:
        content = content[:insert_pos] + new_mounted + content[insert_pos:]

with open('admin_new.html', 'w', encoding='utf-8') as f:
    f.write(content)

print('OK')
