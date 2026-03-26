#!/usr/bin/env python
# -*- coding: utf-8 -*-

with open('admin_new.html', 'r', encoding='utf-8') as f:
    content = f.read()

# 添加链上监听和潜在用户的内容区域
new_tabs = '''
            <!-- 2. Broadcast Tab Content -->
            <div v-show="activeTab === 'broadcast'" class="flex-1 flex flex-col p-6 overflow-y-auto">
                <h2 class="text-2xl font-bold mb-6">群发功能</h2>
                <div class="bg-surface-container-lowest rounded-2xl p-6 border border-outline-variant/10">
                    <div class="space-y-4">
                        <div>
                            <label class="block text-sm font-bold mb-2">消息内容</label>
                            <textarea v-model="broadcastForm.content" placeholder="输入要群发的消息..." 
                                      class="w-full p-4 bg-surface-container rounded-xl border-none text-sm focus:ring-2 focus:ring-primary/20 resize-none h-32"></textarea>
                        </div>
                        <button @click="sendBroadcast" :disabled="broadcastForm.isSending"
                                class="w-full px-6 py-3 bg-primary text-white font-bold rounded-xl hover:bg-primary/80 disabled:opacity-50 transition-all">
                            {{ broadcastForm.isSending ? '发送中...' : '发送群消息' }}
                        </button>
                        <div v-if="broadcastForm.isSending" class="w-full bg-surface-container rounded-xl p-4">
                            <div class="text-sm font-bold mb-2">进度: {{ broadcastForm.progress }}/{{ broadcastForm.total }}</div>
                            <div class="w-full bg-surface-container-high rounded-full h-2">
                                <div class="bg-primary h-2 rounded-full transition-all" :style="{width: (broadcastForm.progress/broadcastForm.total*100) + '%'}"></div>
                            </div>
                        </div>
                    </div>
                </div>
            </div>

            <!-- 3. On-Chain Monitors Tab -->
            <div v-show="activeTab === 'onchain_monitors'" class="flex-1 flex flex-col p-6 overflow-y-auto">
                <h2 class="text-2xl font-bold mb-6">链上监听</h2>
                <div class="grid grid-cols-1 md:grid-cols-2 gap-6">
                    <div v-for="monitor in monitors" :key="monitor.monitor_id" 
                         class="bg-surface-container-lowest rounded-2xl p-6 border border-outline-variant/10 hover:border-primary/30 transition-all">
                        <div class="flex items-start justify-between mb-4">
                            <div>
                                <h3 class="font-bold text-lg">{{ monitor.wallet_address }}</h3>
                                <p class="text-xs text-slate-400 mt-1">{{ monitor.network }}</p>
                            </div>
                            <button @click="deactivateMonitor(monitor.monitor_id)"
                                    class="px-3 py-1 bg-error/10 text-error text-xs font-bold rounded-lg hover:bg-error/20 transition-colors">
                                停用
                            </button>
                        </div>
                        <div class="space-y-2 text-sm">
                            <div class="flex justify-between">
                                <span class="text-slate-400">状态:</span>
                                <span class="font-bold" :class="monitor.status === 'active' ? 'text-green-500' : 'text-slate-400'">
                                    {{ monitor.status === 'active' ? '监听中' : '已停用' }}
                                </span>
                            </div>
                            <div class="flex justify-between">
                                <span class="text-slate-400">创建时间:</span>
                                <span class="font-bold">{{ new Date(monitor.created_at).toLocaleString() }}</span>
                            </div>
                        </div>
                    </div>
                </div>
                <div v-if="monitors.length === 0" class="flex items-center justify-center h-64 text-slate-400">
                    <div class="text-center">
                        <span class="material-symbols-outlined text-5xl opacity-20 block mb-2">account_balance_wallet</span>
                        <p>暂无监听任务</p>
                    </div>
                </div>
            </div>

            <!-- 4. Potential Users Tab -->
            <div v-show="activeTab === 'potential_users'" class="flex-1 flex flex-col p-6 overflow-y-auto">
                <h2 class="text-2xl font-bold mb-6">潜在用户 ({{ potentialUsers.length }})</h2>
                <div class="space-y-3">
                    <div v-if="potentialUsers.length === 0" class="flex items-center justify-center h-64 text-slate-400">
                        <div class="text-center">
                            <span class="material-symbols-outlined text-5xl opacity-20 block mb-2">person_off</span>
                            <p>暂无潜在用户</p>
                        </div>
                    </div>
                    <div v-for="user in potentialUsers" :key="user.user_id" 
                         class="p-4 bg-surface-container-lowest rounded-xl border border-outline-variant/10 hover:border-primary/30 transition-all">
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
                            <button @click="sendInvite(user.user_id)" 
                                    class="px-3 py-1 bg-primary text-white text-xs font-bold rounded-lg hover:bg-primary/80 transition-colors ml-4">
                                邀请
                            </button>
                        </div>
                    </div>
                </div>
            </div>
'''

# 找到第一个标签页内容的结束位置，在其后添加新标签页
# 查找 "<!-- 2. Broadcast Tab Content -->" 或类似的标记
broadcast_marker = '<!-- 2. Broadcast Tab Content -->'
if broadcast_marker not in content:
    # 如果没有找到，在主要内容区域的最后添加
    main_end = content.rfind('</main>')
    if main_end > 0:
        # 在</main>前插入新标签页
        insert_pos = content.rfind('</div>', 0, main_end)
        if insert_pos > 0:
            content = content[:insert_pos] + new_tabs + '\n            ' + content[insert_pos:]

# 确保data中有这些字段
if 'monitors: []' not in content:
    data_insert = content.find('potentialUsers: []')
    if data_insert > 0:
        data_insert = content.find(',', data_insert) + 1
        content = content[:data_insert] + '\n            monitors: [],\n            broadcastForm: { content: "", isSending: false, progress: 0, total: 0, selectedUserIds: [] },' + content[data_insert:]

with open('admin_new.html', 'w', encoding='utf-8') as f:
    f.write(content)

print('OK')
