<template>
  <div class="flex h-full overflow-hidden bg-surface">
    <!-- 群组列表侧栏 -->
    <div class="border-r border-outline-variant/20 flex flex-col bg-surface-container-lowest/80" :class="mobilePanelView==='list'?'w-full md:w-80':'hidden md:flex md:w-80'">
      <div class="p-5 border-b border-outline-variant/10 space-y-3">
        <div class="flex items-center justify-between gap-2">
          <h2 class="text-lg font-black flex items-center"><span class="material-symbols-outlined mr-2 text-primary">groups</span>群组管理</h2>
          <div class="flex items-center gap-2 flex-shrink-0">
            <button
              @click="openRevokeSyncBindingModal"
              class="revoke-sync-entry-btn w-8 h-8 rounded-lg text-[10px] font-bold transition-all flex items-center justify-center relative"
              :class="revokeSyncChatInfo?.chat_id
                ? 'bg-emerald-50 text-emerald-700 border border-emerald-200/70 dark:bg-emerald-500/12 dark:text-emerald-300 dark:border-emerald-400/20'
                : 'bg-amber-50 text-amber-700 border border-amber-200/70 dark:bg-amber-500/12 dark:text-amber-300 dark:border-amber-400/20'"
              :title="revokeSyncChatInfo?.chat_id ? `当前绑定：${revokeSyncChatInfo.title || revokeSyncChatInfo.chat_id}` : '绑定校对信息群'"
            >
              <span class="material-symbols-outlined text-[14px]">{{ revokeSyncChatInfo?.chat_id ? 'hub' : 'link' }}</span>
              <span v-if="revokeSyncChatInfo?.chat_id" class="absolute top-1 right-1 w-1.5 h-1.5 rounded-full bg-emerald-500"></span>
            </button>
            <button @click="loadGroups" class="w-9 h-9 rounded-xl bg-surface-container text-primary flex items-center justify-center"><span class="material-symbols-outlined text-[18px]">refresh</span></button>
          </div>
        </div>
        <div v-if="verifyGrowthSummary" class="grid grid-cols-2 gap-2">
          <button @click="openVerifiedUsersModal('today')" class="flex items-center gap-2 px-3 py-2 bg-primary/10 rounded-xl border border-primary/15 cursor-pointer hover:bg-primary/10 transition-colors active:scale-95">
            <span class="material-symbols-outlined text-primary text-base">today</span>
            <div>
              <div class="text-base font-black text-primary leading-none">{{ verifyGrowthSummary.today }}</div>
              <div class="text-[9px] text-slate-400 mt-0.5">今日引导关注</div>
            </div>
          </button>
          <button @click="openVerifiedUsersModal('all')" class="flex items-center gap-2 px-3 py-2 bg-emerald-50 rounded-xl border border-emerald-200/60 cursor-pointer hover:bg-emerald-100/50 transition-colors active:scale-95">
            <span class="material-symbols-outlined text-emerald-600 text-base">verified</span>
            <div>
              <div class="text-base font-black text-emerald-600 leading-none">{{ verifyGrowthSummary.total }}</div>
              <div class="text-[9px] text-slate-400 mt-0.5">总引导数</div>
            </div>
          </button>
        </div>
        <!-- 全局消息搜索框 -->
        <div class="relative">
          <div class="flex items-center bg-surface-container rounded-xl px-3 py-2 gap-2">
            <span class="material-symbols-outlined text-slate-400 text-[18px]">search</span>
            <input v-model="globalSearchQuery" type="text" placeholder="搜索所有消息..."
              class="flex-1 bg-transparent border-none text-sm focus:ring-0 min-w-0"
              @focus="showSearchResults = globalSearchResults.length > 0"
              @blur="hideSearchResultsDelayed()"
            />
            <div v-if="isGlobalSearching" class="w-4 h-4 border-2 border-primary/30 border-t-primary rounded-full animate-spin flex-shrink-0"></div>
            <button v-else-if="globalSearchQuery" @click="globalSearchQuery='';globalSearchResults=[];showSearchResults=false" class="text-slate-300 hover:text-slate-500">
              <span class="material-symbols-outlined text-[16px]">close</span>
            </button>
          </div>
          <!-- 搜索结果下拉 -->
          <div v-if="showSearchResults && globalSearchResults.length > 0"
            class="absolute top-full left-0 right-0 mt-1 bg-white rounded-2xl shadow-xl border border-slate-100 z-50 max-h-72 overflow-y-auto no-scrollbar">
            <div v-for="item in globalSearchResults" :key="item.msg_id"
              @mousedown.prevent="jumpToSearchResult(item)"
              class="px-4 py-3 hover:bg-slate-50 cursor-pointer border-b border-slate-50 last:border-0 transition-colors">
              <div class="flex items-center gap-2 mb-1">
                <span class="text-[10px] font-black px-1.5 py-0.5 rounded-md"
                  :class="item.source_type==='group'?'bg-primary/10 text-primary':'bg-emerald-50 text-emerald-600'">
                  {{ item.source_type === 'group' ? '群聊' : '私聊' }}
                </span>
                <span v-if="item.username" class="text-[10px] text-slate-400">@{{ item.username }}</span>
                <span class="text-[10px] text-slate-300 ml-auto">{{ formatTime(item.timestamp) }}</span>
              </div>
              <p class="text-xs text-slate-700 line-clamp-2 leading-relaxed">{{ item.content }}</p>
            </div>
          </div>
          <div v-else-if="showSearchResults && !isGlobalSearching && globalSearchQuery"
            class="absolute top-full left-0 right-0 mt-1 bg-white rounded-2xl shadow-xl border border-slate-100 z-50 px-4 py-6 text-center text-sm text-slate-400">
            未找到相关消息
          </div>
        </div>
        <input v-model="groupSearchQuery" type="text" placeholder="搜索群组" class="w-full px-3 py-2.5 bg-surface-container rounded-xl border-none text-sm"/>
      </div>
      <div class="flex-1 overflow-y-auto no-scrollbar p-2 space-y-1">
        <div v-if="isGroupLoading" class="py-16 flex justify-center"><div class="w-7 h-7 border-4 border-primary/20 border-t-primary rounded-full animate-spin"></div></div>
        <div v-for="g in filteredGroups" :key="g.group_id" @click="selectGroupRoom(g);mobilePanelView='detail'"
             class="flex items-center p-3 rounded-2xl cursor-pointer border transition-all"
             :class="selectedGroupRoomId===g.group_id?'bg-primary/10 border-primary/25':'border-transparent bg-white/70 hover:bg-white'">
          <div class="w-10 h-10 rounded-xl flex-shrink-0 overflow-hidden bg-primary/10 flex items-center justify-center">
            <img v-if="groupAvatarMap[g.group_id] && fileUrls[groupAvatarMap[g.group_id]]" :src="fileUrls[groupAvatarMap[g.group_id]]" class="w-10 h-10 object-cover block"/>
            <span v-else class="text-primary font-black">{{ (g.group_title||'G').charAt(0).toUpperCase() }}</span>
          </div>
          <div class="flex-1 min-w-0 ml-3">
            <div class="text-sm font-bold truncate">{{ g.group_title }}</div>
            <div class="text-[10px] text-slate-400 truncate">{{ groupLastMessageMap[g.group_id] || String(g.group_id) }}</div>
          </div>
          <span v-if="g.is_monitored" class="w-2 h-2 rounded-full bg-emerald-500 flex-shrink-0"></span>
        </div>
      </div>
    </div>

    <!-- 群聊主区域 -->
    <div class="flex-1 flex flex-col overflow-hidden" :class="mobilePanelView==='detail'?'flex':'hidden md:flex'">
      <div v-if="!selectedGroupRoomId" class="flex-1 flex flex-col items-center justify-center cartesian-grid">
        <h2 class="text-xl font-bold text-slate-400">选择一个群组</h2>
      </div>
      <template v-else>
        <div class="px-6 py-4 bg-surface-container-lowest/80 border-b border-outline-variant/10 flex items-center justify-between">
          <div class="flex items-center gap-3">
            <button class="md:hidden" @click="mobilePanelView='list'"><span class="material-symbols-outlined text-slate-500">arrow_back</span></button>
            <h3 class="font-bold">{{ selectedGroupRoomTitle }}</h3>
          </div>
          <div class="flex items-center gap-2">
            <!-- 消息校对（Revoke Sync）开关 -->
            <button @click="toggleRevokeSync" :disabled="revokeSyncLoading"
              class="px-3 py-2 text-xs font-bold rounded-xl transition-all"
              :class="selectedGroupRevokeSync ? 'bg-amber-100 text-amber-700' : 'bg-slate-100 text-slate-500'"
              :title="selectedGroupRevokeSync ? '消息校对已开启（撤回同步删除）' : '开启消息校对'">
              <span class="material-symbols-outlined text-sm align-middle mr-1">{{ selectedGroupRevokeSync ? 'sync' : 'sync_disabled' }}</span>校对
            </button>
            <button @click="showGroupMembersModal=true;loadGroupMembers()" class="px-3 py-2 text-xs font-bold bg-surface-container rounded-xl"><span class="material-symbols-outlined text-sm align-middle mr-1">group</span>成员</button>
            <button @click="viewGroupHistory" class="px-3 py-2 text-xs font-bold bg-surface-container rounded-xl"><span class="material-symbols-outlined text-sm align-middle mr-1">history</span>历史</button>
            <button @click="openVerifyConfig" class="px-3 py-2 text-xs font-bold rounded-xl" :class="groupVerifyConfig[selectedGroupRoomId]?.verify_enabled?'bg-green-100 text-green-700':'bg-slate-100 text-slate-500'"><span class="material-symbols-outlined text-sm align-middle mr-1">verified_user</span>验证</button>
          </div>
        </div>
        <!-- 群聊消息区 -->
        <div class="flex-1 overflow-y-auto p-4 space-y-0.5 no-scrollbar relative" ref="msgListRef" @scroll="onMsgScroll">
          <!-- 触顶加载更多指示器 -->
          <div class="flex justify-center pb-2">
            <div v-if="isLoadingMoreGroupMsgs" class="flex items-center gap-2 text-xs text-slate-400">
              <div class="w-4 h-4 border-2 border-primary/30 border-t-primary rounded-full animate-spin"></div>
              加载更多...
            </div>
            <div v-else-if="!hasMoreGroupMsgs && groupRoomMessages.length > 0" class="text-[10px] text-slate-300 py-1">已加载全部消息</div>
          </div>
          <div v-if="isGroupRoomLoading" class="flex justify-center py-20"><div class="w-8 h-8 border-4 border-primary/20 border-t-primary rounded-full animate-spin"></div></div>
          <!-- 消息行：hover 显示内联工具栏 -->
          <div v-for="msg in groupRoomMessages" :key="msg.msg_id"
               :id="'groom-'+msg.tg_msg_id"
               class="group/msg relative flex items-end gap-2 py-1 px-2 rounded-2xl transition-colors duration-150 hover:bg-black/[0.025] md:pr-16"
               :class="msg.is_outgoing ? 'flex-row-reverse' : 'flex-row'">

            <!-- 头像 -->
            <div class="w-7 h-7 rounded-lg flex-shrink-0 overflow-hidden bg-primary/10 flex items-center justify-center self-end mb-1">
              <img v-if="groupRoomAvatarMap[msg.user_id] && fileUrls[groupRoomAvatarMap[msg.user_id]]"
                   :src="fileUrls[groupRoomAvatarMap[msg.user_id]]" class="w-7 h-7 object-cover"/>
              <span v-else class="text-[10px] font-black text-primary">{{ (msg.username||msg.first_name||'U').charAt(0).toUpperCase() }}</span>
            </div>

            <!-- 气泡 -->
            <div class="max-w-[68%] rounded-2xl px-4 py-2.5 shadow-sm select-none md:select-text touch-manipulation mobile-longpress-target"
                 @contextmenu.prevent
                 @touchstart.passive="onGroupMsgTouchStart(msg)"
                 @touchmove.passive="onGroupMsgTouchMove"
                 @touchend="onGroupMsgTouchEnd"
                 @touchcancel="clearGroupLongPressTimer"
                 :class="msg.is_outgoing ? 'bg-primary text-white rounded-tr-sm' : 'bg-white text-slate-800 rounded-tl-sm'">
              <!-- 发送者名字 -->
              <div v-if="!msg.is_outgoing" class="text-[10px] font-black text-primary mb-1">
                <a :href="tgUserProfileHref(msg.user_id, msg.username)"
                   class="hover:underline"
                   @click.stop.prevent="openTgUserProfile(msg.user_id, msg.username)">{{ tgUserLinkLabel(msg.user_id, msg.username, msg.first_name) }}</a>
                <span
                  v-if="getAdBadgeText(msg)"
                  class="ml-1.5 px-1.5 py-0.5 rounded-md text-[9px] font-black align-middle"
                  :class="getAdBadgeClass(msg)"
                  :title="getAdBadgeTitle(msg)"
                >
                  {{ getAdBadgeText(msg) }}
                </span>
              </div>
              <!-- 引用块：蓝色渐变 + 点击跳转 -->
              <div v-if="msg.quoted_tg_msg_id"
                   @click="scrollToMessage(msg.quoted_tg_msg_id)"
                   class="mb-2 rounded-xl border border-primary/25 bg-gradient-to-r from-primary/10 to-primary/5 px-3 py-2 cursor-pointer hover:from-primary/20 hover:to-primary/10 transition-all select-none">
                <div class="flex items-start gap-2">
                  <div class="w-0.5 self-stretch rounded-full bg-primary/60 flex-shrink-0"></div>
                  <div class="min-w-0 flex-1">
                    <div class="text-[10px] font-black text-primary">引用 #{{ msg.quoted_tg_msg_id }}</div>
                    <div class="text-xs truncate mt-0.5" :class="msg.is_outgoing ? 'text-white/70' : 'text-slate-500'">{{ msg.quoted_text || '点击跳转到原消息' }}</div>
                  </div>
                  <span class="material-symbols-outlined text-primary/60 text-sm flex-shrink-0">north</span>
                </div>
              </div>
              <!-- 多媒体 -->
              <div v-if="msg.msg_type && msg.msg_type !== 'text' && msg.file_id" class="message-media-shell mb-1.5 rounded-xl overflow-hidden">
                <img v-if="msg.msg_type==='photo'" :src="fileUrls[msg.file_id]"
                     class="max-w-full max-h-[280px] w-auto h-auto object-contain rounded-xl cursor-zoom-in"
                     @click="fileUrls[msg.file_id] && openUrl(fileUrls[msg.file_id])"/>
                <template v-else-if="msg.msg_type==='animation'||msg.msg_type==='video'">
                  <video v-if="fileUrls[msg.file_id]" :src="fileUrls[msg.file_id]" controls autoplay loop muted playsinline class="max-w-full max-h-[280px] w-auto h-auto rounded-xl"></video>
                </template>
                <template v-else-if="msg.msg_type==='voice'||msg.msg_type==='audio'">
                  <audio v-if="fileUrls[msg.file_id]" :src="fileUrls[msg.file_id]" controls class="h-8 max-w-[220px]"></audio>
                </template>
                <template v-else-if="msg.msg_type==='sticker'">
                  <video v-if="fileUrls[msg.file_id] && isVideoSticker(fileUrls[msg.file_id])" :src="fileUrls[msg.file_id]" autoplay loop muted playsinline class="w-28 h-28 object-contain"></video>
                  <img v-else-if="fileUrls[msg.file_id]" :src="fileUrls[msg.file_id]" class="w-28 h-28 object-contain"/>
                  <button v-else @click="fileStore.autoLoadFile(msg.file_id)" class="w-28 h-28 flex items-center justify-center rounded-xl bg-slate-100 text-slate-400">
                    <span class="material-symbols-outlined text-2xl">sentiment_satisfied</span>
                  </button>
                </template>
                <a v-else-if="fileUrls[msg.file_id]" :href="fileUrls[msg.file_id]" class="text-xs font-bold text-primary underline" @click.prevent="openUrl(fileUrls[msg.file_id])">下载文件</a>
                <button v-if="!['photo','animation','video','voice','audio','sticker'].includes(msg.msg_type||'') && !fileUrls[msg.file_id]" @click="fileStore.autoLoadFile(msg.file_id)" class="text-xs font-bold underline text-primary">加载媒体</button>
              </div>
              <!-- 正文 -->
              <p v-if="msg.content||msg.text" class="text-sm whitespace-pre-wrap break-words leading-relaxed">{{ msg.content || msg.text }}</p>
              <!-- 时间 -->
              <div class="text-[10px] mt-1 opacity-50 text-right">{{ formatTime(msg.message_ts||msg.timestamp) }}</div>
            </div>

            <!-- 内联操作工具栏：hover 渐现 -->
            <div class="hidden md:flex absolute right-2 top-1/2 -translate-y-1/2 items-center gap-0.5 opacity-0 group-hover/msg:opacity-100 transition-opacity duration-150 bg-white/90 backdrop-blur-sm rounded-xl px-1 py-1 border border-slate-100 shadow-sm">
              <button @click="quoteMessage(msg)"
                      class="w-7 h-7 flex items-center justify-center rounded-lg bg-white shadow-sm border border-slate-100 text-slate-400 hover:text-primary hover:border-primary/30 transition-all active:scale-90"
                      title="引用"><span class="material-symbols-outlined text-[14px]">reply</span></button>
              <button v-if="canDeleteGroupMessage(msg)" @click="deleteGroupMessageInline(msg)"
                      class="w-7 h-7 flex items-center justify-center rounded-lg bg-white shadow-sm border border-slate-100 text-slate-400 hover:text-rose-500 hover:border-rose-200 transition-all active:scale-90"
                      title="删除"><span class="material-symbols-outlined text-[14px]">delete</span></button>
              <button @click="$emit('chat', msg.user_id, msg.username||msg.first_name||String(msg.user_id))"
                      class="w-7 h-7 flex items-center justify-center rounded-lg bg-white shadow-sm border border-slate-100 text-slate-400 hover:text-emerald-500 hover:border-emerald-200 transition-all active:scale-90"
                      title="私聊"><span class="material-symbols-outlined text-[14px]">chat</span></button>
            </div>
            <div class="md:hidden ml-1">
              <button @click="openGroupMobileActionMenu(msg)" class="w-7 h-7 flex items-center justify-center rounded-lg bg-white shadow-sm border border-slate-100 text-slate-400 active:scale-90">
                <span class="material-symbols-outlined text-[14px]">more_horiz</span>
              </button>
            </div>
          </div>
        </div>

        <div v-if="groupMobileActionMsg" class="md:hidden absolute inset-0 z-40 bg-black/35" @click="closeGroupMobileActionMenu"></div>
        <div v-if="groupMobileActionMsg" class="md:hidden absolute left-4 right-4 bottom-6 z-50 rounded-2xl border border-outline-variant/30 bg-surface-container-highest shadow-2xl overflow-hidden">
          <button @click="handleGroupMobileQuote" class="w-full flex items-center gap-3 px-4 py-4 text-sm font-bold text-left hover:bg-primary/10">
            <span class="material-symbols-outlined text-primary">reply</span>
            引用回复
          </button>
          <button v-if="canDeleteGroupMessage(groupMobileActionMsg)" @click="handleGroupMobileDelete" class="w-full flex items-center gap-3 px-4 py-4 text-sm font-bold text-left text-rose-500 hover:bg-rose-50/70 border-t border-outline-variant/20">
            <span class="material-symbols-outlined">delete</span>
            删除消息
          </button>
          <button @click="handleGroupMobileChat" class="w-full flex items-center gap-3 px-4 py-4 text-sm font-bold text-left text-emerald-600 hover:bg-emerald-50/70 border-t border-outline-variant/20">
            <span class="material-symbols-outlined">chat</span>
            私聊该用户
          </button>
          <button @click="closeGroupMobileActionMenu" class="w-full px-4 py-3 text-xs font-bold text-slate-500 border-t border-outline-variant/20 hover:bg-surface-container">
            取消
          </button>
        </div>

        <!-- 磨砂玻璃圆球：动态透明度，随滚动距离线性显隐 -->
        <div
          ref="ballBtnRef"
          class="scroll-ball"
          :style="{
            opacity: ballOpacity,
            pointerEvents: ballOpacity < 0.05 ? 'none' : 'auto',
          }"
          @click="handleBallClick"
        >
          <span class="material-symbols-outlined text-slate-600 dark:text-slate-200 text-[22px]">expand_circle_down</span>
        </div>
        <div v-if="groupRoomReplyingTo" class="mx-4 mb-0 px-4 py-2.5 bg-primary/10 border border-primary/20 border-b-0 rounded-t-2xl flex items-center gap-3">
          <span class="material-symbols-outlined text-primary text-sm">reply</span>
          <div class="flex-1 min-w-0">
            <a :href="tgUserProfileHref(groupRoomReplyingTo.user_id, groupRoomReplyingTo.username)"
               class="text-[10px] font-black text-primary hover:underline"
               @click.stop.prevent="openTgUserProfile(groupRoomReplyingTo.user_id, groupRoomReplyingTo.username)">{{ tgUserLinkLabel(groupRoomReplyingTo.user_id, groupRoomReplyingTo.username, groupRoomReplyingTo.first_name) }}</a>
            <span class="text-xs text-slate-500 ml-2 truncate">{{ groupRoomReplyingTo.content }}</span>
          </div>
          <button @click="groupRoomReplyingTo=null" class="w-6 h-6 flex items-center justify-center rounded-lg hover:bg-primary/20 transition-all"><span class="material-symbols-outlined text-slate-400 text-sm">close</span></button>
        </div>
        <!-- 输入框 -->
        <div ref="inputBarRef" class="px-4 py-3 bg-surface-container-lowest border-t">
          <div class="flex items-end gap-2 bg-surface-container rounded-2xl px-4 py-2">
            <textarea v-model="groupRoomInput" @keydown.enter.exact.prevent="sendGroupRoomMessage" placeholder="发送消息..." rows="1" class="flex-1 bg-transparent border-none focus:ring-0 text-sm resize-none no-scrollbar py-1.5 max-h-32"></textarea>
            <button @click="sendGroupRoomMessage" :disabled="!groupRoomInput.trim()" class="w-9 h-9 bg-primary text-white rounded-xl flex items-center justify-center disabled:opacity-40 flex-shrink-0"><span class="material-symbols-outlined text-[18px]">send</span></button>
          </div>
        </div>
      </template>
    </div>
  </div>

  <!-- 群成员弹窗 -->
  <transition name="fade">
    <div v-if="showGroupMembersModal" class="fixed inset-0 z-[100] flex items-center justify-center p-6 bg-black/50 backdrop-blur-sm">
      <div class="bg-white w-full max-w-4xl rounded-3xl shadow-2xl overflow-clip flex flex-col max-h-[90vh] animate-in zoom-in-95 duration-200">
        <div class="px-6 py-5 border-b border-slate-100 flex items-center justify-between">
          <div class="flex items-center gap-3">
            <div class="w-9 h-9 rounded-xl bg-primary/10 flex items-center justify-center"><span class="material-symbols-outlined text-primary text-[18px]">groups</span></div>
            <div><h3 class="text-base font-black text-slate-800">群组成员</h3>
              <p class="text-[11px] text-slate-400">已记录 {{ groupMembers.length }} 人<span v-if="groupMemberSearchQuery" class="ml-1">· 筛选 {{ filteredGroupMembers.length }}</span></p>
            </div>
          </div>
          <button @click="showGroupMembersModal=false" class="w-8 h-8 flex items-center justify-center hover:bg-slate-100 rounded-xl"><span class="material-symbols-outlined text-slate-400 text-[18px]">close</span></button>
        </div>
        <div class="px-5 py-3 border-b border-slate-100 space-y-2 flex-shrink-0">
          <div class="relative">
            <span class="material-symbols-outlined absolute left-3 top-1/2 -translate-y-1/2 text-slate-300 text-[16px]">search</span>
            <input v-model="groupMemberSearchQuery" type="text" placeholder="搜索用户名 / 昵称 / ID" class="w-full pl-9 pr-3 py-2 bg-slate-50 rounded-xl border border-slate-200/80 text-sm focus:outline-none focus:ring-2 focus:ring-primary/20"/>
          </div>
          <div class="flex items-center justify-between gap-2 flex-wrap">
            <div class="text-[11px] text-slate-500">
              <span v-if="selectedMemberIds.length===0" class="text-slate-400">未选择</span>
              <span v-else class="px-2 py-0.5 bg-primary/10 text-primary font-black rounded-lg">已选 {{ selectedMemberIds.length }} 人</span>
            </div>
            <div class="flex items-center gap-1.5 flex-wrap">
              <button @click="toggleSelectAllFilteredMembers" class="px-2.5 py-1.5 text-[10px] font-bold rounded-lg bg-slate-100 text-slate-600 hover:bg-slate-200 active:scale-95 transition-all">全选</button>
              <button @click="clearSelectedMembers" class="px-2.5 py-1.5 text-[10px] font-bold rounded-lg bg-slate-100 text-slate-500 hover:bg-slate-200 active:scale-95">清空</button>
              <div class="w-px h-4 bg-slate-200"></div>
              <button @click="batchAction('mute')" :disabled="selectedMemberIds.length===0" class="px-2.5 py-1.5 text-[10px] font-bold rounded-lg bg-slate-100 text-slate-600 hover:bg-slate-200 disabled:opacity-40 active:scale-95 transition-all"><span class="material-symbols-outlined text-[12px] align-middle">volume_off</span> 禁言</button>
              <button @click="batchAction('unmute')" :disabled="selectedMemberIds.length===0" class="px-2.5 py-1.5 text-[10px] font-bold rounded-lg bg-slate-100 text-slate-600 hover:bg-slate-200 disabled:opacity-40 active:scale-95 transition-all"><span class="material-symbols-outlined text-[12px] align-middle">volume_up</span> 解禁</button>
              <button @click="batchAction('promote')" :disabled="selectedMemberIds.length===0" class="px-2.5 py-1.5 text-[10px] font-bold rounded-lg bg-slate-100 text-slate-600 hover:bg-slate-200 disabled:opacity-40 active:scale-95 transition-all"><span class="material-symbols-outlined text-[12px] align-middle">shield_person</span> 设管</button>
              <button @click="batchAction('demote')" :disabled="selectedMemberIds.length===0" class="px-2.5 py-1.5 text-[10px] font-bold rounded-lg bg-slate-100 text-slate-600 hover:bg-slate-200 disabled:opacity-40 active:scale-95 transition-all"><span class="material-symbols-outlined text-[12px] align-middle">remove_moderator</span> 撤管</button>
              <button @click="batchAction('kick')" :disabled="selectedMemberIds.length===0" class="px-2.5 py-1.5 text-[10px] font-bold rounded-lg bg-rose-50 text-rose-500 hover:bg-rose-100 disabled:opacity-40 active:scale-95 transition-all"><span class="material-symbols-outlined text-[12px] align-middle">person_remove</span> 移除</button>
            </div>
          </div>
          <div class="flex items-center gap-2">
            <input v-model="directUidInput" type="text" placeholder="输入 UID 直接操作..." class="flex-1 px-3 py-1.5 bg-slate-50 rounded-lg border border-slate-200/80 text-xs focus:outline-none focus:ring-2 focus:ring-primary/20" @keyup.enter="directUidAction('mute')"/>
            <button @click="directUidAction('mute')" class="px-2.5 py-1.5 text-[10px] font-bold rounded-lg bg-slate-100 text-slate-600 hover:bg-amber-50 hover:text-amber-600 transition-all active:scale-95"><span class="material-symbols-outlined text-[12px] align-middle">volume_off</span> 禁言</button>
            <button @click="directUidAction('unmute')" class="px-2.5 py-1.5 text-[10px] font-bold rounded-lg bg-slate-100 text-slate-600 hover:bg-emerald-50 hover:text-emerald-600 transition-all active:scale-95"><span class="material-symbols-outlined text-[12px] align-middle">volume_up</span> 解禁</button>
            <button @click="directUidAction('kick')" class="px-2.5 py-1.5 text-[10px] font-bold rounded-lg bg-rose-50 text-rose-500 hover:bg-rose-100 transition-all active:scale-95"><span class="material-symbols-outlined text-[12px] align-middle">person_remove</span> 移除</button>
            <span v-if="directUidResult" class="text-[10px]" :class="directUidResult.ok?'text-emerald-600':'text-rose-500'">{{ directUidResult.msg }}</span>
          </div>
        </div>
        <div class="flex-1 overflow-y-auto p-4 no-scrollbar" ref="memberListRef">
          <div v-if="isMembersLoading" class="flex flex-col items-center justify-center py-20 space-y-4">
            <div class="w-10 h-10 border-4 border-primary/20 border-t-primary rounded-full animate-spin"></div>
            <span class="text-sm text-slate-400">正在获取成员列表...</span>
          </div>
          <div v-else-if="groupMembers.length===0" class="flex flex-col items-center justify-center py-20 text-slate-400 italic">
            <span class="material-symbols-outlined text-5xl mb-4 opacity-20">person_off</span>暂无成员记录
          </div>
          <div v-else class="grid grid-cols-1 md:grid-cols-2 xl:grid-cols-3 gap-1.5">
            <div v-for="m in filteredGroupMembers" :key="m.user_id"
                 class="relative flex items-center gap-3 px-3 py-2.5 rounded-2xl border border-transparent transition-all group cursor-pointer"
                 :class="selectedMemberIds.includes(m.user_id)?'bg-primary/5 border-primary/15':'hover:bg-slate-50 hover:border-slate-200/60'"
                 @click="toggleMemberSelect(m.user_id)">
              <div class="w-4 h-4 rounded-full border-2 flex-shrink-0 flex items-center justify-center transition-all" :class="selectedMemberIds.includes(m.user_id)?'border-primary bg-primary':'border-slate-300'">
                <span v-if="selectedMemberIds.includes(m.user_id)" class="material-symbols-outlined text-white text-[10px]">check</span>
              </div>
              <div class="w-9 h-9 rounded-xl flex-shrink-0 overflow-hidden bg-primary/10 flex items-center justify-center text-primary font-black text-sm">
                <img v-if="m.avatar_file_id && fileUrls[m.avatar_file_id]" :src="fileUrls[m.avatar_file_id]" class="w-full h-full object-cover"/>
                <span v-else>{{ (m.first_name||m.username||'U').charAt(0).toUpperCase() }}</span>
              </div>
              <div class="flex-1 min-w-0">
                <div class="flex items-center gap-1.5">
                  <span class="text-sm font-bold text-slate-800 truncate">{{ m.first_name||m.username||'Unknown' }}</span>
                  <span v-if="['admin','administrator','creator'].includes((m.status||'').toLowerCase())" class="px-1.5 py-0.5 bg-slate-100 text-slate-500 text-[9px] font-black rounded-md flex-shrink-0">管理</span>
                </div>
                <div class="text-[10px] text-slate-400 truncate">
                  <a :href="tgUserProfileHref(m.user_id, m.username)" class="hover:underline text-primary" @click.stop.prevent="openTgUserProfile(m.user_id, m.username)">{{ tgUserLinkLabel(m.user_id, m.username, m.first_name) }}</a>
                </div>
              </div>
              <div class="absolute right-2 top-1/2 -translate-y-1/2 flex items-center gap-1 opacity-0 group-hover:opacity-100 transition-all bg-white/90 rounded-xl px-1 py-0.5 shadow-sm" @click.stop>
                <button @click="memberAction(m,'mute')" class="w-7 h-7 flex items-center justify-center rounded-lg hover:bg-amber-50 hover:text-amber-500 text-slate-400 transition-all" title="禁言"><span class="material-symbols-outlined text-[15px]">volume_off</span></button>
                <button @click="memberAction(m,'unmute')" class="w-7 h-7 flex items-center justify-center rounded-lg hover:bg-emerald-50 hover:text-emerald-500 text-slate-400 transition-all" title="解禁"><span class="material-symbols-outlined text-[15px]">volume_up</span></button>
                <button @click="memberAction(m,'kick')" class="w-7 h-7 flex items-center justify-center rounded-lg hover:bg-rose-50 hover:text-rose-500 text-slate-400 transition-all" title="移除"><span class="material-symbols-outlined text-[15px]">person_remove</span></button>
              </div>
            </div>
            <div ref="memberScrollSentinel" class="col-span-full h-4"></div>
          </div>
        </div>
        <div class="px-6 py-4 border-t border-slate-100 flex justify-end">
          <button @click="showGroupMembersModal=false" class="px-6 py-2.5 bg-primary text-white text-sm font-black rounded-xl">关闭</button>
        </div>
      </div>
    </div>
  </transition>

  <!-- 历史消息弹窗 -->
  <transition name="fade">
    <div v-if="showGroupMessagesModal" class="fixed inset-0 z-[100] flex items-center justify-center p-6 bg-black/60 backdrop-blur-sm">
      <div class="bg-surface-container-lowest w-full max-w-3xl rounded-3xl shadow-2xl overflow-hidden animate-in zoom-in-95 flex flex-col max-h-[85vh]">
        <div class="px-8 py-6 border-b border-outline-variant/10 flex items-center justify-between bg-surface-container-low/50">
          <h3 class="text-xl font-black flex items-center"><span class="material-symbols-outlined mr-3 text-amber-500">history</span>群组实时记录 (最近 50 条)</h3>
          <button @click="showGroupMessagesModal=false" class="w-10 h-10 flex items-center justify-center hover:bg-surface-container-high rounded-full transition-colors"><span class="material-symbols-outlined">close</span></button>
        </div>
        <div class="flex-1 overflow-y-auto p-6 space-y-4 no-scrollbar bg-surface-container-low/20">
          <div v-if="isMessagesLoading" class="flex flex-col items-center justify-center py-20 space-y-4">
            <div class="w-10 h-10 border-4 border-amber-500/20 border-t-amber-500 rounded-full animate-spin"></div>
            <span class="text-sm text-slate-400">正在抓取缓存消息...</span>
          </div>
          <div v-else-if="groupMessages.length===0" class="flex flex-col items-center justify-center py-20 text-slate-400 italic">
            <span class="material-symbols-outlined text-5xl mb-4 opacity-20">cloud_off</span>
            <p class="font-bold">暂无历史记录</p>
          </div>
          <div v-else class="space-y-3">
            <div v-for="m in groupMessages" :key="m.msg_id" :id="m.tg_msg_id?('group-msg-'+m.tg_msg_id):undefined"
                 class="p-4 bg-surface-container-lowest rounded-2xl border border-outline-variant/5 shadow-sm hover:shadow-md transition-all">
              <div class="flex items-center justify-between mb-2">
                <div class="flex items-center space-x-2 min-w-0">
                  <a :href="tgUserProfileHref(m.user_id, m.username)"
                     class="text-xs font-black text-primary hover:underline truncate"
                     @click.stop.prevent="openTgUserProfile(m.user_id, m.username)">{{ tgUserLinkLabel(m.user_id, m.username, m.first_name) }}</a>
                  <span v-if="m.sender_type" class="text-[9px] px-2 py-0.5 rounded-full bg-surface-container-high text-slate-500">{{ m.sender_type }}</span>
                  <span
                    v-if="getAdBadgeText(m)"
                    class="text-[9px] px-2 py-0.5 rounded-full font-black"
                    :class="getAdBadgeClass(m)"
                    :title="getAdBadgeTitle(m)"
                  >
                    {{ getAdBadgeText(m) }}
                  </span>
                </div>
                <span class="text-[9px] text-slate-400">{{ formatTime(m.message_ts||m.timestamp) }}</span>
              </div>
              <!-- 引用回复 -->
              <div v-if="m.quoted_tg_msg_id" @click="scrollToGroupMessage(m.quoted_tg_msg_id)" class="mb-2 rounded-xl border border-primary/20 bg-gradient-to-r from-primary/10 to-primary/5 px-3 py-2.5 cursor-pointer hover:from-primary/15 hover:to-primary/10 transition-all">
                <div class="flex items-start space-x-2">
                  <div class="w-1 self-stretch rounded-full bg-primary/70"></div>
                  <div class="min-w-0 flex-1">
                    <div class="text-[10px] font-black text-primary uppercase tracking-wider">引用消息 #{{ m.quoted_tg_msg_id }}</div>
                    <div class="text-xs text-slate-600 truncate mt-0.5">{{ m.quoted_text || '点击跳转到原消息' }}</div>
                  </div>
                  <span class="material-symbols-outlined text-primary/70 text-sm">north</span>
                </div>
              </div>
              <!-- 多媒体 -->
              <div v-if="m.msg_type && m.msg_type !== 'text' && m.file_id" class="message-media-shell mb-2 rounded-lg overflow-hidden flex justify-start">
                <img v-if="m.msg_type==='photo'" :src="fileUrls[m.file_id]" class="max-w-full max-h-[320px] w-auto h-auto object-contain rounded-lg cursor-zoom-in" @click="fileUrls[m.file_id] && openUrl(fileUrls[m.file_id])"/>
                <template v-else-if="m.msg_type==='animation'||m.msg_type==='video'">
                  <video v-if="fileUrls[m.file_id]" :src="fileUrls[m.file_id]" controls autoplay loop muted playsinline class="max-w-full max-h-[320px] w-auto h-auto rounded-lg"></video>
                </template>
                <template v-else-if="m.msg_type==='voice'||m.msg_type==='audio'">
                  <audio v-if="fileUrls[m.file_id]" :src="fileUrls[m.file_id]" controls class="h-8 max-w-[240px]"></audio>
                </template>
                <template v-else-if="m.msg_type==='sticker'">
                  <video v-if="fileUrls[m.file_id] && isVideoSticker(fileUrls[m.file_id])" :src="fileUrls[m.file_id]" autoplay loop muted playsinline controls class="w-24 h-24 object-contain rounded-lg"></video>
                  <img v-else-if="fileUrls[m.file_id]" :src="fileUrls[m.file_id]" class="w-24 h-24 object-contain rounded-lg"/>
                </template>
                <a v-else-if="fileUrls[m.file_id]" :href="fileUrls[m.file_id]" class="text-xs font-bold text-primary underline" @click.prevent="openUrl(fileUrls[m.file_id])">下载文件</a>
                <button v-if="!fileUrls[m.file_id]" @click="fileStore.autoLoadFile(m.file_id)" class="text-xs font-bold underline text-primary">加载媒体</button>
              </div>
              <div class="text-sm text-on-surface leading-relaxed whitespace-pre-wrap">{{ m.content }}</div>
              <div class="mt-3 flex items-center justify-end space-x-2">
                <button v-if="m.tg_msg_id && m.sender_type==='user'" @click="openGroupReplyModal(m)" class="px-3 py-1.5 text-[10px] font-black rounded-lg bg-primary/10 text-primary hover:bg-primary/20 transition-all">群内回复</button>
                <button v-if="canDeleteGroupMessage(m)" @click="deleteGroupMessage(m)" class="px-3 py-1.5 text-[10px] font-black rounded-lg bg-red-500/10 text-red-500 hover:bg-red-500/20 transition-all">删除</button>
              </div>
            </div>
          </div>
        </div>
        <div class="px-8 py-6 bg-surface-container-low flex justify-between items-center border-t border-outline-variant/10">
          <p class="text-[10px] text-slate-400 italic">※ 数据仅包含 Bot 在场且开启监听期间的消息</p>
          <button @click="showGroupMessagesModal=false" class="px-8 py-2.5 bg-primary text-on-primary text-sm font-black rounded-xl shadow-lg shadow-primary/20 hover:scale-[1.02] active:scale-95 transition-all">关闭</button>
        </div>
      </div>
    </div>
  </transition>

  <!-- 群内回复弹窗 -->
  <transition name="fade">
    <div v-if="showGroupReplyModal" class="fixed inset-0 z-[110] flex items-center justify-center p-6 bg-black/60 backdrop-blur-sm">
      <div class="bg-surface-container-lowest w-full max-w-lg rounded-3xl shadow-2xl overflow-hidden animate-in zoom-in-95">
        <div class="px-6 py-5 border-b border-outline-variant/10 flex items-center justify-between">
          <div class="flex items-center gap-3"><span class="material-symbols-outlined text-primary">reply</span><h3 class="text-base font-black">群内回复</h3></div>
          <button @click="closeGroupReplyModal" class="w-8 h-8 flex items-center justify-center hover:bg-slate-100 rounded-xl"><span class="material-symbols-outlined text-slate-400">close</span></button>
        </div>
        <div class="p-5 space-y-4">
          <div v-if="groupReplyTarget" class="p-3 bg-primary/5 rounded-xl border border-primary/15">
            <div class="text-[10px] font-black text-primary mb-1">@{{ groupReplyTarget.username }} · #{{ groupReplyTarget.tg_msg_id }}</div>
            <div class="text-xs text-slate-600 line-clamp-3">{{ groupReplyTarget.content }}</div>
          </div>
          <textarea v-model="groupReplyText" placeholder="输入回复内容..." rows="4" class="w-full px-4 py-3 bg-slate-50 rounded-xl border border-slate-200/80 text-sm focus:outline-none focus:ring-2 focus:ring-primary/20 resize-none"></textarea>
        </div>
        <div class="px-5 py-4 border-t flex gap-2">
          <button @click="closeGroupReplyModal" class="flex-1 py-2 bg-slate-100 text-slate-600 text-sm font-black rounded-xl">取消</button>
          <button @click="submitGroupReply" :disabled="!groupReplyText.trim()||isGroupReplySending" class="flex-1 py-2 bg-primary text-white text-sm font-black rounded-xl disabled:opacity-50">{{ isGroupReplySending?'发送中...':'发送回复' }}</button>
        </div>
      </div>
    </div>
  </transition>

  <!-- 入群验证配置弹窗 -->
  <transition name="fade">
    <div v-if="showVerifyConfigModal" class="fixed inset-0 z-[110] flex items-center justify-center p-4 bg-black/50 backdrop-blur-sm">
      <div class="bg-white w-full max-w-lg rounded-3xl shadow-2xl flex flex-col max-h-[90vh] animate-in zoom-in-95">
        <div class="px-6 py-5 border-b flex items-center justify-between flex-shrink-0">
          <div class="flex items-center gap-3"><span class="material-symbols-outlined text-primary">verified_user</span><h3 class="text-base font-black">频道关注验证配置</h3></div>
          <button @click="showVerifyConfigModal=false" class="w-8 h-8 flex items-center justify-center hover:bg-slate-100 rounded-xl"><span class="material-symbols-outlined text-slate-400">close</span></button>
        </div>
        <div class="flex-1 overflow-y-auto p-5 space-y-4">
          <!-- 开关 -->
          <div class="flex items-center justify-between p-3 bg-slate-50 rounded-2xl">
            <div>
              <div class="text-sm font-black">启用强制关注验证</div>
              <div class="text-[10px] text-slate-400 mt-0.5">开启后，未关注频道的用户发言将被拦截</div>
            </div>
            <label class="relative inline-flex items-center cursor-pointer"><input type="checkbox" class="sr-only peer" v-model="verifyConfigForm.verify_enabled"/><div class="w-11 h-6 bg-slate-200 rounded-full peer peer-checked:bg-primary peer-checked:after:translate-x-full after:content-none after:absolute after:top-[2px] after:left-[2px] after:bg-white after:border after:rounded-full after:h-5 after:w-5 after:transition-all"></div></label>
          </div>
          <!-- 验证类型 -->
          <div class="grid grid-cols-3 gap-2">
            <button v-for="vt in verifyTypes" :key="vt.id" @click="verifyConfigForm.verify_type=vt.id"
                    class="flex flex-col items-center gap-1 p-3 rounded-2xl border-2 text-xs font-black transition-all"
                    :class="verifyConfigForm.verify_type===vt.id?'border-primary bg-primary/10 text-primary':'border-slate-200 text-slate-500 hover:border-slate-300'">
              <span class="material-symbols-outlined text-[20px]">{{ vt.icon }}</span>{{ vt.label }}
            </button>
          </div>
          <!-- 超时设置 -->
          <div>
            <label class="text-[10px] font-black text-slate-400 uppercase tracking-wider">验证超时（秒）</label>
            <input type="number" v-model.number="verifyConfigForm.verify_timeout" min="30" max="600" placeholder="120" class="mt-1 w-full px-3 py-2 bg-slate-50 rounded-xl border text-sm focus:ring-2 focus:ring-primary/20 focus:border-primary/30"/>
          </div>
          <!-- 频道设置（仅 channel 类型） -->
          <transition name="slide-up">
            <div v-if="verifyConfigForm.verify_type==='channel'" class="space-y-3">
              <div>
                <label class="text-[10px] font-black text-slate-400 uppercase tracking-wider">强制关注的频道</label>
                <input type="text" v-model="verifyConfigForm.verify_channel_id" placeholder="@your_channel 或 频道数字ID" class="mt-1 w-full px-3 py-2 bg-slate-50 rounded-xl border text-sm focus:ring-2 focus:ring-primary/20 focus:border-primary/30"/>
              </div>
              <div>
                <label class="text-[10px] font-black text-slate-400 uppercase tracking-wider">拦截提示文案</label>
                <div class="text-[9px] text-slate-400 mb-1">支持变量：{user} = 用户名，{channel} = 频道</div>
                <textarea v-model="verifyConfigForm.verify_msg_template" rows="3"
                          placeholder="🚫 {user}，请先关注频道 {channel} 后再发言！"
                          class="w-full px-3 py-2 bg-slate-50 rounded-xl border text-sm resize-none focus:ring-2 focus:ring-primary/20 focus:border-primary/30"></textarea>
              </div>
              <!-- 统计展示 -->
              <div v-if="verifyStats" class="p-3 bg-gradient-to-r from-primary/10 to-primary/5 rounded-2xl border border-primary/15">
                <div class="text-[10px] font-black text-primary uppercase tracking-wider mb-2">当前群验证统计</div>
                <div class="flex items-center gap-4">
                  <div class="flex-1">
                    <div class="text-xl font-black text-primary">{{ verifyStats.verified }}</div>
                    <div class="text-[10px] text-slate-500">已验证</div>
                  </div>
                  <div class="flex-1">
                    <div class="text-xl font-black text-slate-400">{{ verifyStats.unverified }}</div>
                    <div class="text-[10px] text-slate-500">待验证</div>
                  </div>
                  <div class="flex-1">
                    <div class="text-xl font-black text-slate-600">{{ verifyStats.total ? Math.round(verifyStats.verified/verifyStats.total*100) : 0 }}%</div>
                    <div class="text-[10px] text-slate-500">通过率</div>
                  </div>
                </div>
                <!-- 进度条 -->
                <div class="mt-2 h-1.5 bg-slate-200 rounded-full overflow-hidden">
                  <div class="h-full bg-primary rounded-full transition-all duration-500"
                       :style="{width: verifyStats.total ? (verifyStats.verified/verifyStats.total*100)+'%' : '0%'}"></div>
                </div>
              </div>
            </div>
          </transition>
        </div>
        <div class="px-5 py-4 border-t flex gap-2 flex-shrink-0">
          <button @click="showVerifyConfigModal=false" class="flex-1 py-2 bg-slate-100 text-slate-600 text-sm font-black rounded-xl">取消</button>
          <button @click="saveVerifyConfig" class="flex-1 py-2 bg-primary text-white text-sm font-black rounded-xl hover:bg-primary/90 active:scale-95 transition-all">保存</button>
        </div>
      </div>
    </div>
  </transition>

  <!-- 已验证/未关注用户列表弹窗 -->
  <transition name="fade">
    <div v-if="isVerifiedUsersModalOpen" class="fixed inset-0 z-[100] flex items-center justify-center p-6 bg-black/50 backdrop-blur-sm">
      <div class="bg-surface-container-low w-full max-w-2xl rounded-3xl shadow-2xl overflow-hidden animate-in zoom-in-95 flex flex-col max-h-[85vh]">
        <div class="px-8 py-6 border-b border-outline-variant/10 flex items-center justify-between bg-surface-container-lowest/50">
          <div class="flex items-center gap-3">
            <span class="material-symbols-outlined text-primary text-[24px]">verified</span>
            <div>
              <h3 class="text-lg font-black">{{ verifiedUsersTab === 'verified' ? '已验证用户' : '未关注的人' }}</h3>
              <p class="text-[11px] text-slate-400 mt-0.5">
                <template v-if="verifiedUsersTab === 'verified'">共 {{ verifiedUsersList.length }} 人</template>
                <template v-else>共 {{ unverifiedUsers.length }} 人</template>
              </p>
            </div>
          </div>
          <button @click="closeVerifiedUsersModal" class="w-10 h-10 flex items-center justify-center hover:bg-surface-container-high rounded-full transition-colors"><span class="material-symbols-outlined">close</span></button>
        </div>
        <div class="px-8 py-4 border-b border-outline-variant/10 flex gap-4 bg-surface-container-lowest">
          <button @click="verifiedUsersTab='verified';loadVerifiedUsers()"
                  class="text-sm font-bold transition-colors"
                  :class="verifiedUsersTab==='verified'?'text-primary border-b-2 border-primary pb-1':'text-slate-400 hover:text-slate-600'">
            已验证用户
          </button>
          <button @click="verifiedUsersTab='unverified';loadUnverifiedUsers()"
                  class="text-sm font-bold transition-colors"
                  :class="verifiedUsersTab==='unverified'?'text-primary border-b-2 border-primary pb-1':'text-slate-400 hover:text-slate-600'">
            未关注的人
          </button>
        </div>
        <div class="flex-1 overflow-y-auto p-6 space-y-2 no-scrollbar bg-surface-container-low/20">
          <!-- 已验证用户列表 -->
          <div v-if="verifiedUsersTab === 'verified'">
            <div v-if="isLoadingVerified" class="flex flex-col items-center justify-center py-20 space-y-4">
              <div class="w-10 h-10 border-4 border-primary/20 border-t-primary rounded-full animate-spin"></div>
              <span class="text-sm text-slate-400">正在加载已验证用户...</span>
            </div>
            <div v-else-if="verifiedUsersList.length === 0" class="flex flex-col items-center justify-center py-20 text-slate-400 italic">
              <span class="material-symbols-outlined text-5xl mb-4 opacity-20">person_off</span>
              <p class="font-bold">暂无已验证用户</p>
            </div>
            <div v-else class="space-y-1.5">
              <div v-for="user in verifiedUsersList" :key="`${user.group_id}-${user.user_id}`"
                   class="w-full flex items-center gap-3 px-4 py-3 rounded-2xl border border-outline-variant/5 bg-surface-container-lowest hover:bg-surface-container-high transition-all">
                <!-- 头像 -->
                <div class="w-10 h-10 rounded-xl flex-shrink-0 overflow-hidden bg-primary/10 flex items-center justify-center">
                  <img v-if="user.avatar_file_id && fileUrls[user.avatar_file_id]" :src="fileUrls[user.avatar_file_id]" class="w-10 h-10 object-cover"/>
                  <span v-else class="text-primary font-black text-sm">{{ (user.first_name || user.username || 'U').charAt(0).toUpperCase() }}</span>
                </div>
                <!-- 用户信息 -->
                <div class="flex-1 min-w-0">
                  <div class="flex items-center gap-2">
                    <span class="text-sm font-bold text-slate-800 truncate">{{ user.first_name || user.username || 'Unknown' }}</span>
                  </div>
                  <div class="text-[10px] text-slate-400 truncate">
                    <a :href="tgUserProfileHref(user.user_id, user.username)" class="hover:underline text-primary" @click.stop.prevent="openTgUserProfile(user.user_id, user.username)">{{ tgUserLinkLabel(user.user_id, user.username, user.first_name) }}</a>
                  </div>
                  <div v-if="user.group_title" class="text-[10px] text-primary mt-0.5">📍 {{ user.group_title }}</div>
                </div>
                <!-- 验证时间 -->
                <div class="text-[10px] text-slate-400 flex-shrink-0">{{ formatDateTime(user.verified_at) }}</div>
                <!-- 禁言按钮 -->
                <button @click="openMuteModal(user)" class="w-8 h-8 flex items-center justify-center rounded-lg bg-amber-50 text-amber-600 hover:bg-amber-100 transition-colors flex-shrink-0" title="禁言">
                  <span class="material-symbols-outlined text-[16px]">volume_off</span>
                </button>
              </div>
            </div>
          </div>

          <!-- 未关注用户列表 -->
          <div v-if="verifiedUsersTab === 'unverified'">
            <div v-if="isLoadingUnverified" class="flex flex-col items-center justify-center py-20 space-y-4">
              <div class="w-10 h-10 border-4 border-amber-500/20 border-t-amber-500 rounded-full animate-spin"></div>
              <span class="text-sm text-slate-400">正在加载未关注用户...</span>
            </div>
            <div v-else-if="unverifiedUsers.length === 0" class="flex flex-col items-center justify-center py-20 text-slate-400 italic">
              <span class="material-symbols-outlined text-5xl mb-4 opacity-20">check_circle</span>
              <p class="font-bold">所有用户都已关注频道！</p>
            </div>
            <div v-else class="space-y-3">
              <!-- 全选按钮 -->
              <div class="flex items-center gap-2 px-4 py-2 bg-slate-50 rounded-xl">
                <input type="checkbox" :checked="selectedUsers.size === unverifiedUsers.length && unverifiedUsers.length > 0"
                       @change="selectAllUnverifiedUsers" class="w-4 h-4 cursor-pointer"/>
                <span class="text-sm font-bold text-slate-600">全选 ({{ selectedUsers.size }}/{{ unverifiedUsers.length }})</span>
              </div>
              
              <!-- 按群组分类显示 -->
              <div v-for="(groupUsers, groupId) in groupedUnverifiedUsers" :key="groupId" class="space-y-2">
                <!-- 群组头部 -->
                <div class="px-4 py-2 bg-primary/5 rounded-lg border border-primary/10 sticky top-0 z-10">
                  <div class="flex items-center justify-between">
                    <div class="flex items-center gap-2">
                      <span class="material-symbols-outlined text-primary text-[18px]">groups</span>
                      <div>
                        <p class="text-sm font-bold text-slate-800">{{ groupUsers[0].group_title || ('群组 ' + groupId) }}</p>
                        <p class="text-[10px] text-slate-400">{{ groupUsers.length }} 人未关注</p>
                      </div>
                    </div>
                    <button @click="toggleGroupSelection(groupId)" class="px-3 py-1 text-[11px] font-bold rounded-lg"
                            :class="isGroupFullySelected(groupId) ? 'bg-primary text-white' : 'bg-slate-100 text-slate-600 hover:bg-slate-200'">
                      {{ isGroupFullySelected(groupId) ? '已全选' : '全选' }}
                    </button>
                  </div>
                </div>
                
                <!-- 群组内用户列表 -->
                <div class="space-y-1.5 pl-2">
                  <button v-for="user in groupUsers" :key="`${user.group_id}-${user.user_id}`"
                          class="w-full flex items-center gap-3 px-4 py-3 rounded-2xl border transition-all active:scale-95"
                          :class="selectedUsers.has(user.user_id) ? 'border-amber-500 bg-amber-50' : 'border-outline-variant/5 bg-surface-container-lowest hover:bg-surface-container-high'">
                    <!-- 勾选框 -->
                    <input type="checkbox" :checked="selectedUsers.has(user.user_id)"
                           @change="toggleUserSelection(user.user_id)" class="w-4 h-4 cursor-pointer flex-shrink-0"/>
                    
                    <!-- 头像 -->
                    <div class="w-10 h-10 rounded-xl flex-shrink-0 overflow-hidden bg-primary/10 flex items-center justify-center">
                      <img v-if="user.avatar_file_id && fileUrls[user.avatar_file_id]" :src="fileUrls[user.avatar_file_id]" class="w-10 h-10 object-cover"/>
                      <span v-else class="text-primary font-black text-sm">{{ (user.first_name || user.username || 'U').charAt(0).toUpperCase() }}</span>
                    </div>
                    
                    <!-- 用户信息 -->
                    <div class="flex-1 min-w-0">
                      <div class="flex items-center gap-2">
                        <span class="text-sm font-bold text-slate-800 truncate">{{ user.first_name || user.username || 'Unknown' }}</span>
                      </div>
                      <div class="text-[10px] text-slate-400 truncate">
                      <a :href="tgUserProfileHref(user.user_id, user.username)" class="hover:underline text-primary" @click.stop.prevent="openTgUserProfile(user.user_id, user.username)">{{ tgUserLinkLabel(user.user_id, user.username, user.first_name) }}</a>
                    </div>
                    </div>
                  </button>
                </div>
              </div>
            </div>
          </div>
        </div>
        <div class="px-8 py-6 border-t border-outline-variant/10 flex justify-between bg-surface-container-lowest">
          <div v-if="verifiedUsersTab === 'unverified' && selectedUsers.size > 0" class="text-sm text-slate-500">
            已选择 {{ selectedUsers.size }} 人
          </div>
          <div v-else></div>
          
          <div class="flex gap-2">
            <button @click="sendChannelReminders" 
                    :disabled="selectedUsers.size === 0 || isSendingReminders"
                    class="px-6 py-2.5 bg-amber-500 text-white text-sm font-black rounded-xl shadow-lg shadow-amber-500/20 hover:scale-[1.02] active:scale-95 transition-all disabled:opacity-50 disabled:cursor-not-allowed">
              {{ isSendingReminders ? '@提醒中...' : `@提醒关注 (${selectedUsers.size})` }}
            </button>
            <button @click="closeVerifiedUsersModal" class="px-8 py-2.5 bg-primary text-on-primary text-sm font-black rounded-xl shadow-lg shadow-primary/20 hover:scale-[1.02] active:scale-95 transition-all">
              关闭
            </button>
          </div>
        </div>
      </div>
    </div>
  </transition>

  <!-- 校对信息群绑定弹窗 -->
  <transition name="fade">
    <div v-if="showRevokeSyncBindingModal" class="fixed inset-0 z-[110] flex items-center justify-center p-4 bg-black/50 backdrop-blur-sm">
      <div
        class="revoke-sync-modal w-full max-w-3xl rounded-3xl shadow-2xl flex flex-col max-h-[92vh] animate-in zoom-in-95 overflow-hidden"
        :class="themeStore.isDark
          ? 'bg-[#09111d] border border-[#2d4668]'
          : 'bg-surface-container-lowest border border-outline-variant/10'"
      >
        <div
          class="revoke-sync-modal-head px-6 py-5 border-b flex items-center justify-between"
          :class="themeStore.isDark
            ? 'border-[#2d4668] bg-[linear-gradient(135deg,rgba(36,161,222,0.18)_0%,rgba(36,161,222,0.06)_38%,rgba(9,17,29,0.92)_100%)]'
            : 'border-outline-variant/10 bg-gradient-to-r from-primary/10 via-primary/5 to-transparent'"
        >
          <div class="flex items-center gap-3">
            <span class="material-symbols-outlined" :class="themeStore.isDark ? 'text-amber-300' : 'text-amber-600'">hub</span>
            <div>
              <h3 class="text-base font-black" :class="themeStore.isDark ? 'text-slate-100' : 'text-slate-900'">校对群绑定与授权</h3>
              <p class="text-[11px] mt-0.5" :class="themeStore.isDark ? 'text-slate-400' : 'text-slate-500'">绑定校对信息群后，可直接选择哪些群可以使用它，并为群聊分配校对群。</p>
            </div>
          </div>
          <button
            @click="showRevokeSyncBindingModal=false"
            class="w-8 h-8 flex items-center justify-center rounded-xl transition-colors"
            :class="themeStore.isDark ? 'hover:bg-[#13253d]' : 'hover:bg-surface-container'"
          >
            <span class="material-symbols-outlined" :class="themeStore.isDark ? 'text-slate-300' : 'text-slate-400'">close</span>
          </button>
        </div>
        <div class="flex-1 overflow-y-auto p-5 grid grid-cols-1 xl:grid-cols-[320px_minmax(0,1fr)] gap-5">
          <div class="space-y-4">
            <div class="revoke-sync-panel rounded-2xl border p-4"
                 :class="revokeSyncChatInfo?.chat_id
                   ? (themeStore.isDark ? 'bg-emerald-500/10 border-emerald-400/20' : 'bg-emerald-50 border-emerald-200/70')
                   : (themeStore.isDark ? 'bg-[#102034] border-[#2d4668]' : 'bg-surface-container border-outline-variant/10')">
              <div class="text-xs font-black uppercase tracking-wider"
                   :class="revokeSyncChatInfo?.chat_id ? (themeStore.isDark ? 'text-emerald-300' : 'text-emerald-700') : (themeStore.isDark ? 'text-slate-400' : 'text-slate-500')">
                {{ revokeSyncChatInfo?.chat_id ? '当前已绑定' : '当前未绑定' }}
              </div>
              <div class="mt-2 text-sm font-bold break-all" :class="themeStore.isDark ? 'text-slate-100' : 'text-slate-800'">
                {{ revokeSyncChatInfo?.chat_id ? (revokeSyncChatInfo.title || revokeSyncChatInfo.chat_id) : '暂无校对信息群' }}
              </div>
              <div class="mt-1 text-[11px] break-all" :class="themeStore.isDark ? 'text-slate-400' : 'text-slate-500'" v-if="revokeSyncChatInfo?.chat_id">
                ID: {{ revokeSyncChatInfo.chat_id }}<span v-if="revokeSyncChatInfo.username" class="ml-1">· @{{ revokeSyncChatInfo.username }}</span><span v-if="revokeSyncChatInfo.type" class="ml-1">· {{ revokeSyncChatInfo.type }}</span>
              </div>
              <div v-if="revokeSyncChatInfo?.chat_id" class="mt-3 flex flex-wrap gap-2">
                <span class="px-2 py-1 rounded-lg text-[10px] font-bold"
                      :class="themeStore.isDark ? 'bg-emerald-500/15 text-emerald-300' : 'bg-emerald-100 text-emerald-700'">已授权 {{ revokeSyncAssignedGroupIds.length }} 个群</span>
                <span v-if="selectedGroupRoomId && revokeSyncAssignedGroupIds.includes(selectedGroupRoomId)" class="px-2 py-1 rounded-lg text-[10px] font-bold"
                      :class="themeStore.isDark ? 'bg-primary/20 text-sky-300' : 'bg-primary/10 text-primary'">当前群已分配</span>
              </div>
            </div>
            <div class="space-y-2">
              <label class="text-[10px] font-black uppercase tracking-wider" :class="themeStore.isDark ? 'text-slate-400' : 'text-slate-400'">群组 / 频道标识</label>
              <input
                v-model="revokeSyncBindingInput"
                type="text"
                placeholder="-1001234567890 或 @channel_username"
                class="revoke-sync-input w-full px-3 py-2 rounded-xl border text-sm focus:ring-2 focus:ring-amber-500/20 focus:border-amber-500/30"
                :class="themeStore.isDark ? 'bg-[#0b1525] border-[#2d4668] text-slate-100 placeholder:text-slate-500' : 'bg-surface-container border-outline-variant/10 text-slate-800'"
                @keyup.enter="saveRevokeSyncChatBinding"
              />
              <p class="text-[11px] leading-relaxed" :class="themeStore.isDark ? 'text-slate-400' : 'text-slate-500'">
                绑定前请先把机器人加入该群/频道，并确保机器人能发送消息和删除自己转发的暂存消息。
              </p>
            </div>
            <div v-if="selectedGroupRoomId && revokeSyncChatInfo?.chat_id"
                 class="revoke-sync-panel rounded-2xl border p-4"
                 :class="themeStore.isDark ? 'border-primary/20 bg-primary/10' : 'border-primary/15 bg-primary/5'">
              <div class="text-[10px] font-black text-primary uppercase tracking-wider">当前选中群快捷操作</div>
              <div class="mt-1 text-sm font-bold" :class="themeStore.isDark ? 'text-slate-100' : 'text-slate-800'">{{ selectedGroupRoomTitle || selectedGroupRoomId }}</div>
              <div class="mt-3 flex gap-2">
                <button
                  @click="assignCurrentGroupToRevokeSync"
                  class="flex-1 py-2 rounded-xl text-sm font-black transition-all"
                  :class="revokeSyncAssignedGroupIds.includes(selectedGroupRoomId)
                    ? (themeStore.isDark ? 'bg-[#17304a] text-slate-500 cursor-not-allowed' : 'bg-surface-container text-slate-400 cursor-not-allowed')
                    : 'bg-primary text-white hover:brightness-105'"
                  :disabled="revokeSyncSavingAssignments || revokeSyncAssignedGroupIds.includes(selectedGroupRoomId)"
                >
                  分配给当前群
                </button>
                <button
                  @click="removeCurrentGroupFromRevokeSync"
                  class="flex-1 py-2 rounded-xl text-sm font-black transition-all"
                  :class="revokeSyncAssignedGroupIds.includes(selectedGroupRoomId)
                    ? (themeStore.isDark ? 'bg-rose-500/14 text-rose-300 hover:bg-rose-500/18' : 'bg-rose-50 text-rose-600 hover:bg-rose-100')
                    : (themeStore.isDark ? 'bg-[#17304a] text-slate-500 cursor-not-allowed' : 'bg-surface-container text-slate-400 cursor-not-allowed')"
                  :disabled="revokeSyncSavingAssignments || !revokeSyncAssignedGroupIds.includes(selectedGroupRoomId)"
                >
                  移出当前群
                </button>
              </div>
            </div>
          </div>

          <div
            class="revoke-sync-assign-panel min-w-0 rounded-2xl border p-4 flex flex-col"
            :class="themeStore.isDark ? 'border-[#2d4668] bg-[#122034]' : 'border-outline-variant/10 bg-surface-container/80'"
          >
            <div class="flex items-center justify-between gap-3 flex-wrap">
              <div>
                <div class="text-sm font-black" :class="themeStore.isDark ? 'text-slate-100' : 'text-slate-800'">授权哪些群使用当前校对群</div>
                <div class="text-[11px] mt-0.5" :class="themeStore.isDark ? 'text-slate-400' : 'text-slate-500'">勾选后会把当前绑定的校对群分配给这些群。只对已授权群生效。</div>
              </div>
              <span
                class="text-[11px] font-bold px-2.5 py-1 rounded-full border"
                :class="themeStore.isDark ? 'bg-[#0b1525] border-[#2d4668] text-slate-300' : 'bg-surface-container-lowest border-outline-variant/10 text-slate-500'"
              >{{ revokeSyncAssignedGroupIds.length }} / {{ groupStore.groups.length }}</span>
            </div>
            <div class="mt-4 flex items-center gap-2">
              <input
                v-model="revokeSyncAssignmentSearchQuery"
                type="text"
                placeholder="搜索群组名称或 ID"
                class="revoke-sync-input flex-1 px-3 py-2 rounded-xl border text-sm focus:ring-2 focus:ring-primary/20"
                :class="themeStore.isDark ? 'bg-[#0b1525] border-[#2d4668] text-slate-100 placeholder:text-slate-500' : 'bg-surface-container-lowest border-outline-variant/10 text-slate-800'"
              />
              <button
                @click="toggleAssignAllFiltered"
                class="px-3 py-2 rounded-xl text-xs font-black"
                :class="themeStore.isDark ? 'bg-[#17304a] text-slate-300 hover:bg-[#214161]' : 'bg-surface-container text-slate-600 hover:bg-surface-container-high'"
              >切换当前筛选</button>
            </div>
            <div class="mt-4 flex-1 overflow-y-auto no-scrollbar space-y-2 pr-1 min-h-[260px] revoke-sync-scroll">
              <div v-if="filteredRevokeSyncAssignableGroups.length === 0" class="py-12 text-center text-sm" :class="themeStore.isDark ? 'text-slate-400' : 'text-slate-400'">
                没有匹配的群组
              </div>
              <button
                v-for="g in filteredRevokeSyncAssignableGroups"
                :key="'sync-bind-' + g.group_id"
                @click="toggleRevokeSyncGroupAssignment(g.group_id)"
                class="revoke-sync-group-item w-full text-left px-4 py-3 rounded-2xl border transition-all flex items-center gap-3"
                :class="revokeSyncAssignedGroupIds.includes(g.group_id)
                  ? (themeStore.isDark ? 'bg-emerald-500/10 border-emerald-400/20' : 'bg-emerald-50 border-emerald-200/70')
                  : (themeStore.isDark ? 'bg-[#0b1525] border-[#2d4668] hover:border-primary/30 hover:bg-[#11263f]' : 'bg-surface-container-lowest border-outline-variant/10 hover:border-primary/25 hover:bg-primary/5')"
              >
                <div class="w-5 h-5 rounded-md border flex items-center justify-center flex-shrink-0"
                     :class="revokeSyncAssignedGroupIds.includes(g.group_id)
                       ? 'bg-emerald-500 border-emerald-500 text-white'
                       : (themeStore.isDark ? 'border-[#4f6b92] text-transparent' : 'border-slate-300 text-transparent')">
                  <span class="material-symbols-outlined text-[12px]">check</span>
                </div>
                <div class="min-w-0 flex-1">
                  <div class="text-sm font-bold truncate" :class="themeStore.isDark ? 'text-slate-100' : 'text-slate-800'">{{ g.group_title || g.group_id }}</div>
                  <div class="text-[10px] mt-0.5" :class="themeStore.isDark ? 'text-slate-400' : 'text-slate-500'">
                    ID: {{ g.group_id }}
                    <span class="ml-1" :class="g.is_monitored ? 'text-emerald-600' : 'text-slate-400'">· {{ g.is_monitored ? '监听中' : '未监听' }}</span>
                    <span v-if="g.delete_on_revoke" class="ml-1 text-amber-600">· 已开启校对</span>
                  </div>
                </div>
                <span class="text-[10px] font-black px-2 py-1 rounded-lg flex-shrink-0"
                      :class="revokeSyncAssignedGroupIds.includes(g.group_id)
                        ? (themeStore.isDark ? 'bg-emerald-500/15 text-emerald-300' : 'bg-emerald-100 text-emerald-700')
                        : (themeStore.isDark ? 'bg-[#17304a] text-slate-400' : 'bg-surface-container text-slate-500')">
                  {{ revokeSyncAssignedGroupIds.includes(g.group_id) ? '已授权' : '未授权' }}
                </span>
              </button>
            </div>
          </div>
        </div>
        <div
          class="revoke-sync-footer px-5 py-4 border-t flex gap-2"
          :class="themeStore.isDark ? 'border-[#2d4668] bg-[#08111d]' : 'border-outline-variant/10 bg-surface-container-lowest/90'"
        >
          <button
            v-if="revokeSyncChatInfo?.chat_id"
            @click="unbindRevokeSyncChat"
            :disabled="revokeSyncChatLoading"
            class="px-4 py-2 bg-rose-50 text-rose-600 text-sm font-black rounded-xl dark:bg-rose-500/12 dark:text-rose-300 disabled:opacity-50"
          >
            解绑
          </button>
          <button
            v-if="revokeSyncChatInfo?.chat_id"
            @click="saveRevokeSyncAssignments"
            :disabled="revokeSyncSavingAssignments"
            class="px-4 py-2 bg-primary text-white text-sm font-black rounded-xl shadow-lg shadow-primary/20 disabled:opacity-50"
          >
            {{ revokeSyncSavingAssignments ? '保存中...' : '保存授权' }}
          </button>
          <button
            @click="showRevokeSyncBindingModal=false"
            class="flex-1 py-2 text-sm font-black rounded-xl"
            :class="themeStore.isDark ? 'bg-[#17304a] text-slate-300' : 'bg-surface-container text-slate-600'"
          >关闭</button>
          <button
            @click="saveRevokeSyncChatBinding"
            :disabled="revokeSyncChatLoading || !revokeSyncBindingInput.trim()"
            class="flex-1 py-2 bg-amber-500 text-white text-sm font-black rounded-xl shadow-lg shadow-amber-500/20 disabled:opacity-50"
          >
            {{ revokeSyncChatLoading ? '绑定中...' : (revokeSyncChatInfo?.chat_id ? '重新绑定' : '立即绑定') }}
          </button>
        </div>
      </div>
    </div>
  </transition>

  <transition name="fade">
    <div v-if="revokeSyncToast" class="revoke-sync-toast fixed right-5 bottom-5 z-[140] max-w-sm rounded-2xl border px-4 py-3 shadow-2xl backdrop-blur-sm"
         :class="revokeSyncToast.type === 'success'
           ? (themeStore.isDark ? 'bg-emerald-500/14 border-emerald-400/20 text-emerald-200' : 'bg-emerald-50/95 border-emerald-200 text-emerald-700')
           : (themeStore.isDark ? 'bg-rose-500/14 border-rose-400/20 text-rose-200' : 'bg-rose-50/95 border-rose-200 text-rose-600')">
      <div class="flex items-start gap-2">
        <span class="material-symbols-outlined text-[18px] mt-0.5">{{ revokeSyncToast.type === 'success' ? 'check_circle' : 'error' }}</span>
        <div class="text-sm font-bold leading-relaxed">{{ revokeSyncToast.message }}</div>
      </div>
    </div>
  </transition>

  <!-- 禁言时长选择弹窗 -->
  <transition name="fade">
    <div v-if="showMuteModal" class="fixed inset-0 z-[110] flex items-center justify-center p-4 bg-black/50 backdrop-blur-sm">
      <div class="bg-white w-full max-w-sm rounded-3xl shadow-2xl flex flex-col animate-in zoom-in-95">
        <div class="px-6 py-5 border-b flex items-center justify-between">
          <div class="flex items-center gap-3"><span class="material-symbols-outlined text-amber-500">volume_off</span><h3 class="text-base font-black">禁言用户</h3></div>
          <button @click="closeMuteModal" class="w-8 h-8 flex items-center justify-center hover:bg-slate-100 rounded-xl"><span class="material-symbols-outlined text-slate-400">close</span></button>
        </div>
        <div class="p-5 space-y-4">
          <div v-if="muteTargetUser" class="p-3 bg-amber-50 rounded-xl border border-amber-200/60">
            <div class="text-sm font-black text-amber-900">{{ muteTargetUser.first_name || muteTargetUser.username || 'Unknown' }}</div>
            <a v-if="muteTargetUser.user_id"
               :href="tgUserProfileHref(muteTargetUser.user_id, muteTargetUser.username)"
               class="text-[10px] text-amber-700 mt-0.5 hover:underline block"
               @click.stop.prevent="openTgUserProfile(muteTargetUser.user_id, muteTargetUser.username)">{{ tgUserLinkLabel(muteTargetUser.user_id, muteTargetUser.username, muteTargetUser.first_name) }}</a>
            <div class="text-[10px] text-amber-600 mt-1">📍 {{ muteTargetUser.group_title || ('群组 ' + muteTargetUser.group_id) }}</div>
          </div>
          <div>
            <label class="text-[10px] font-black text-slate-400 uppercase tracking-wider">禁言时长</label>
            <div class="mt-2 grid grid-cols-2 gap-2">
              <button @click="muteDuration = 300" :class="muteDuration === 300 ? 'bg-amber-500 text-white' : 'bg-slate-100 text-slate-600'" class="py-2 rounded-lg text-sm font-bold transition-all">5 分钟</button>
              <button @click="muteDuration = 3600" :class="muteDuration === 3600 ? 'bg-amber-500 text-white' : 'bg-slate-100 text-slate-600'" class="py-2 rounded-lg text-sm font-bold transition-all">1 小时</button>
              <button @click="muteDuration = 86400" :class="muteDuration === 86400 ? 'bg-amber-500 text-white' : 'bg-slate-100 text-slate-600'" class="py-2 rounded-lg text-sm font-bold transition-all">1 天</button>
              <button @click="muteDuration = 604800" :class="muteDuration === 604800 ? 'bg-amber-500 text-white' : 'bg-slate-100 text-slate-600'" class="py-2 rounded-lg text-sm font-bold transition-all">7 天</button>
            </div>
            <div class="mt-3">
              <label class="text-[10px] font-black text-slate-400 uppercase tracking-wider">自定义（秒）</label>
              <input v-model.number="muteDuration" type="number" min="0" placeholder="输入秒数" class="mt-1 w-full px-3 py-2 bg-slate-50 rounded-xl border text-sm focus:ring-2 focus:ring-amber-500/20 focus:border-amber-500/30"/>
            </div>
          </div>
        </div>
        <div class="px-5 py-4 border-t flex gap-2">
          <button @click="closeMuteModal" class="flex-1 py-2 bg-slate-100 text-slate-600 text-sm font-black rounded-xl">取消</button>
          <button @click="confirmMute" :disabled="isMuting" class="flex-1 py-2 bg-amber-500 text-white text-sm font-black rounded-xl disabled:opacity-50">{{ isMuting ? '禁言中...' : '确认禁言' }}</button>
        </div>
      </div>
    </div>
  </transition>
</template>
<script setup lang="ts">
import { ref, computed, onMounted, onUnmounted, watch, nextTick } from 'vue'
import { useGroupStore } from '@/stores/group'
import { useWsStore } from '@/stores/ws'
import { useFileStore } from '@/stores/fileStore'
import { useThemeStore } from '@/stores/theme'
import { openExternalLink } from '@/lib/telegram'
import { openTgUserProfile, tgUserProfileHref, tgUserLinkLabel } from '@/utils/tgUserLink'

const emit = defineEmits<{ chat: [userId: number, name: string] }>()
const msgListRef = ref<HTMLElement|null>(null)
const inputBarRef = ref<HTMLElement|null>(null)
const ballBtnRef  = ref<HTMLElement|null>(null)

// ---- 下翻球：动态透明度 ----
const ballOpacity = ref(0)

function onMsgScroll() {
  const el = msgListRef.value
  if (!el) return
  const dist = el.scrollHeight - el.scrollTop - el.clientHeight
  // dist < 100 → 0，100~500 → 线性，> 500 → 1
  if (dist < 100) {
    ballOpacity.value = 0
  } else if (dist > 500) {
    ballOpacity.value = 1
  } else {
    ballOpacity.value = (dist - 100) / 400
  }
  // 触顶加载更多
  if (el.scrollTop < 60) loadMoreGroupMessages()
}

function scrollToBottom(behavior: ScrollBehavior = 'smooth') {
  const el = msgListRef.value
  if (!el) return
  el.scrollTo({ top: el.scrollHeight, behavior })
}

// ---- 圆球点击：收缩弹跳 → 触发滚动 → 透明度随滚动自然消退 ----
let ballAnimating = false
async function handleBallClick() {
  if (ballAnimating || ballOpacity.value < 0.05) return
  ballAnimating = true
  const ball = ballBtnRef.value
  if (ball) {
    // 微收缩弹跳反馈
    const gsap = (await import('gsap')).default
    await gsap.to(ball, { scale: 0.82, duration: 0.10, ease: 'power2.in' })
    gsap.to(ball, { scale: 1, duration: 0.35, ease: 'elastic.out(1, 0.4)' })
  }
  // 触发滚动，透明度随 onMsgScroll 自动消退
  scrollToBottom('smooth')
  setTimeout(() => { ballAnimating = false }, 400)
}

const groupStore = useGroupStore()
const wsStore = useWsStore()
const fileStore = useFileStore()
const themeStore = useThemeStore()
const fileUrls = computed(() => fileStore.fileUrls)

const token = () => { const raw = sessionStorage.getItem('adm_s'); if (raw) { try { return decodeURIComponent(atob(raw)) } catch { return '' } } return localStorage.getItem('admin_token') || '' }
const authH = () => ({ 'Authorization': `Bearer ${token()}`, 'Content-Type': 'application/json' })

const mobilePanelView = ref<'list'|'detail'>('list')
const groupSearchQuery = ref('')
const selectedGroupRoomId = ref<number|null>(null)
const selectedGroupRoomTitle = ref('')
const groupRoomMessages = ref<any[]>([])
const groupRoomInput = ref('')
const groupRoomReplyingTo = ref<any|null>(null)
const isGroupRoomLoading = ref(false)
const isGroupLoading = computed(() => groupStore.isLoading)
const showGroupMembersModal = ref(false)
const showVerifyConfigModal = ref(false)
const showGroupMessagesModal = ref(false)
const showGroupReplyModal = ref(false)
const showRevokeSyncBindingModal = ref(false)
const isVerifiedUsersModalOpen = ref(false)
const verifiedUsersPeriod = ref<'today'|'all'>('all')
const verifiedUsersTab = ref<'verified'|'unverified'>('verified')
const verifiedUsersList = ref<any[]>([])
const isLoadingVerified = ref(false)
const unverifiedUsers = ref<any[]>([])
const isLoadingUnverified = ref(false)
const selectedUsers = ref<Set<number>>(new Set())
const isSendingReminders = ref(false)
const groupMessages = ref<any[]>([])
const isMessagesLoading = ref(false)
const groupReplyTarget = ref<any|null>(null)
const groupReplyText = ref('')
const isGroupReplySending = ref(false)
const groupMembers = ref<any[]>([])
const groupMemberSearchQuery = ref('')
const selectedMemberIds = ref<number[]>([])
const isMembersLoading = ref(false)
const groupVerifyConfig = ref<Record<number,any>>({})
const verifyConfigForm = ref({ verify_enabled: false, verify_type: 'captcha', verify_timeout: 120, verify_channel_id: '', verify_msg_template: '', verify_questions: [] as any[] })
const verifyStats = ref<any|null>(null)
const verifyGrowthSummary = ref<{today:number,total:number}|null>(null)
const groupAvatarMap = ref<Record<number,string>>({})
const groupRoomAvatarMap = ref<Record<number,string>>({})
const groupLastMessageMap = ref<Record<number,string>>({})
const memberListRef = ref<HTMLElement|null>(null)
const memberScrollSentinel = ref<HTMLElement|null>(null)
const directUidInput = ref('')
const directUidResult = ref<{ok:boolean,msg:string}|null>(null)
const verifyTypes = [{ id:'captcha',label:'数字验证码',icon:'pin' },{ id:'quiz',label:'答题验证',icon:'quiz' },{ id:'channel',label:'关注频道',icon:'campaign' }]
const showMuteModal = ref(false)
const muteTargetUser = ref<any|null>(null)
const muteDuration = ref(3600) // 默认禁言1小时
const isMuting = ref(false)
let memberObserver: IntersectionObserver|null = null
const MEMBER_PAGE_SIZE = 60
const memberVisibleCount = ref(MEMBER_PAGE_SIZE)

// ---- Revoke Sync 状态 ----
const selectedGroupRevokeSync = ref(false)
const revokeSyncLoading = ref(false)
const revokeSyncChatLoading = ref(false)
const revokeSyncSavingAssignments = ref(false)
const revokeSyncChatInfo = ref<any | null>(null)
const revokeSyncBindingInput = ref('')
const revokeSyncAssignedGroupIds = ref<number[]>([])
const revokeSyncAssignmentSearchQuery = ref('')
const revokeSyncToast = ref<{ type: 'success' | 'error', message: string } | null>(null)
let revokeSyncToastTimer: ReturnType<typeof setTimeout> | null = null
const groupLongPressTimer = ref<ReturnType<typeof setTimeout> | null>(null)
const groupMobileActionMsg = ref<any | null>(null)

function showRevokeSyncToast(message: string, type: 'success' | 'error' = 'success') {
  revokeSyncToast.value = { message, type }
  if (revokeSyncToastTimer) clearTimeout(revokeSyncToastTimer)
  revokeSyncToastTimer = setTimeout(() => {
    revokeSyncToast.value = null
    revokeSyncToastTimer = null
  }, 2600)
}

async function toggleRevokeSync() {
  if (!selectedGroupRoomId.value || revokeSyncLoading.value) return
  const newVal = !selectedGroupRevokeSync.value
  if (newVal && !revokeSyncChatInfo.value?.chat_id) {
    showRevokeSyncBindingModal.value = true
    showRevokeSyncToast('请先绑定校对信息群，再开启校对功能。', 'error')
    return
  }
  revokeSyncLoading.value = true
  try {
    const res = await fetch('/api/group_listen/revoke_sync', {
      method: 'POST', headers: authH(),
      body: JSON.stringify({ group_id: selectedGroupRoomId.value, enabled: newVal })
    })
    const d = await res.json()
    if (d.status === 'ok') {
      selectedGroupRevokeSync.value = newVal
      if (newVal && d.revoke_sync_chat_id && !revokeSyncAssignedGroupIds.value.includes(Number(selectedGroupRoomId.value))) {
        revokeSyncAssignedGroupIds.value = [...revokeSyncAssignedGroupIds.value, Number(selectedGroupRoomId.value)]
      }
      showRevokeSyncToast(newVal ? '已开启该群的校对功能' : '已关闭该群的校对功能', 'success')
    } else {
      if (d.message) showRevokeSyncToast(d.message, 'error')
      if (newVal && /绑定/.test(String(d.message || ''))) showRevokeSyncBindingModal.value = true
    }
  } finally { revokeSyncLoading.value = false }
}

async function loadRevokeSync() {
  if (!selectedGroupRoomId.value) return
  try {
    const res = await fetch(`/api/group_listen/list`, { headers: authH() })
    const d = await res.json()
    const g = (d.groups || d.data || []).find((x: any) => x.group_id === selectedGroupRoomId.value)
    selectedGroupRevokeSync.value = !!g?.delete_on_revoke
  } catch {}
}

function openRevokeSyncBindingModal() {
  revokeSyncBindingInput.value = revokeSyncChatInfo.value?.username
    ? `@${revokeSyncChatInfo.value.username}`
    : String(revokeSyncChatInfo.value?.chat_id || '')
  showRevokeSyncBindingModal.value = true
  loadRevokeSyncChatInfo()
}

async function loadRevokeSyncChatInfo() {
  try {
    const res = await fetch('/api/group_listen/sync_chat_info', { headers: authH() })
    const d = await res.json()
    revokeSyncChatInfo.value = d.status === 'ok' ? (d.data || null) : null
    revokeSyncAssignedGroupIds.value = Array.isArray(revokeSyncChatInfo.value?.assigned_groups)
      ? revokeSyncChatInfo.value.assigned_groups.map((g: any) => Number(g.group_id)).filter((id: number) => Number.isFinite(id))
      : []
  } catch {
    revokeSyncChatInfo.value = null
    revokeSyncAssignedGroupIds.value = []
  }
}

async function saveRevokeSyncChatBinding() {
  const chatId = revokeSyncBindingInput.value.trim()
  if (!chatId || revokeSyncChatLoading.value) return
  revokeSyncChatLoading.value = true
  try {
    const res = await fetch('/api/group_listen/create_sync_chat', {
      method: 'POST',
      headers: authH(),
      body: JSON.stringify({ chat_id: chatId }),
    })
    const d = await res.json()
    if (d.status === 'ok') {
      await loadRevokeSyncChatInfo()
      revokeSyncBindingInput.value = revokeSyncChatInfo.value?.username
        ? `@${revokeSyncChatInfo.value.username}`
        : String(revokeSyncChatInfo.value?.chat_id || '')
      showRevokeSyncToast(d.message || '校对信息群绑定成功', 'success')
    } else if (d.message) {
      showRevokeSyncToast(d.message, 'error')
    }
  } catch {
    showRevokeSyncToast('绑定校对信息群失败，请稍后重试。', 'error')
  } finally {
    revokeSyncChatLoading.value = false
  }
}

async function unbindRevokeSyncChat() {
  if (revokeSyncChatLoading.value) return
  if (!window.confirm('解绑后，已开启的校对任务将无法继续执行。确认解绑？')) return
  revokeSyncChatLoading.value = true
  try {
    const res = await fetch('/api/group_listen/create_sync_chat', {
      method: 'POST',
      headers: authH(),
      body: JSON.stringify({ unbind: true }),
    })
    const d = await res.json()
    if (d.status === 'ok') {
      revokeSyncChatInfo.value = null
      revokeSyncAssignedGroupIds.value = []
      revokeSyncBindingInput.value = ''
      selectedGroupRevokeSync.value = false
      showRevokeSyncToast(d.message || '已解绑校对信息群', 'success')
    } else if (d.message) {
      showRevokeSyncToast(d.message, 'error')
    }
  } catch {
    showRevokeSyncToast('解绑校对信息群失败，请稍后重试。', 'error')
  } finally {
    revokeSyncChatLoading.value = false
  }
}

const filteredRevokeSyncAssignableGroups = computed(() => {
  const q = revokeSyncAssignmentSearchQuery.value.trim().toLowerCase()
  if (!q) return groupStore.groups
  return groupStore.groups.filter((g: any) =>
    String(g.group_id).includes(q) || String(g.group_title || '').toLowerCase().includes(q)
  )
})

function toggleRevokeSyncGroupAssignment(groupId: number) {
  if (!revokeSyncChatInfo.value?.chat_id) {
    showRevokeSyncToast('请先绑定校对信息群。', 'error')
    return
  }
  const current = new Set(revokeSyncAssignedGroupIds.value)
  if (current.has(groupId)) current.delete(groupId)
  else current.add(groupId)
  revokeSyncAssignedGroupIds.value = [...current]
}

function toggleAssignAllFiltered() {
  if (!revokeSyncChatInfo.value?.chat_id) {
    showRevokeSyncToast('请先绑定校对信息群。', 'error')
    return
  }
  const visibleIds = filteredRevokeSyncAssignableGroups.value.map((g: any) => Number(g.group_id))
  const selected = new Set(revokeSyncAssignedGroupIds.value)
  const allSelected = visibleIds.length > 0 && visibleIds.every((id: number) => selected.has(id))
  for (const id of visibleIds) {
    if (allSelected) selected.delete(id)
    else selected.add(id)
  }
  revokeSyncAssignedGroupIds.value = [...selected]
}

async function saveRevokeSyncAssignments() {
  if (!revokeSyncChatInfo.value?.chat_id || revokeSyncSavingAssignments.value) return
  revokeSyncSavingAssignments.value = true
  try {
    const res = await fetch('/api/group_listen/sync_chat_assignments', {
      method: 'POST',
      headers: authH(),
      body: JSON.stringify({
        chat_id: revokeSyncChatInfo.value.chat_id,
        group_ids: revokeSyncAssignedGroupIds.value,
      }),
    })
    const d = await res.json()
    if (d.status === 'ok') {
      revokeSyncAssignedGroupIds.value = Array.isArray(d.data?.assigned_group_ids) ? d.data.assigned_group_ids.map((id: any) => Number(id)) : revokeSyncAssignedGroupIds.value
      await loadRevokeSyncChatInfo()
      await loadGroups()
      if (selectedGroupRoomId.value) await loadRevokeSync()
      showRevokeSyncToast(d.message || '校对群授权已保存', 'success')
    } else {
      showRevokeSyncToast(d.message || '保存授权失败', 'error')
    }
  } catch {
    showRevokeSyncToast('保存校对群授权失败，请稍后重试。', 'error')
  } finally {
    revokeSyncSavingAssignments.value = false
  }
}

function assignCurrentGroupToRevokeSync() {
  if (!selectedGroupRoomId.value) return
  if (!revokeSyncAssignedGroupIds.value.includes(Number(selectedGroupRoomId.value))) {
    revokeSyncAssignedGroupIds.value = [...revokeSyncAssignedGroupIds.value, Number(selectedGroupRoomId.value)]
  }
}

function removeCurrentGroupFromRevokeSync() {
  if (!selectedGroupRoomId.value) return
  revokeSyncAssignedGroupIds.value = revokeSyncAssignedGroupIds.value.filter((id) => id !== Number(selectedGroupRoomId.value))
}

// ---- 全局搜索状态 ----
const globalSearchQuery = ref('')
const globalSearchResults = ref<any[]>([])
const isGlobalSearching = ref(false)
const showSearchResults = ref(false)
let searchTimer: ReturnType<typeof setTimeout> | null = null

watch(globalSearchQuery, (q) => {
  if (searchTimer) clearTimeout(searchTimer)
  if (!q.trim() || q.length < 1) { globalSearchResults.value = []; showSearchResults.value = false; return }
  searchTimer = setTimeout(() => doGlobalSearch(q.trim()), 400)
})

async function doGlobalSearch(q: string) {
  isGlobalSearching.value = true
  showSearchResults.value = true
  try {
    const res = await fetch(`/api/search?q=${encodeURIComponent(q)}&limit=20`, { headers: authH() })
    const d = await res.json()
    globalSearchResults.value = d.status === 'ok' ? d.data : []
  } catch { globalSearchResults.value = [] }
  finally { isGlobalSearching.value = false }
}

async function jumpToSearchResult(item: any) {
  showSearchResults.value = false
  globalSearchQuery.value = ''
  if (item.source_type === 'group' && item.group_id) {
    // 跳转到群聊
    const g = groupStore.groups.find((x: any) => x.group_id === item.group_id)
    if (g) {
      mobilePanelView.value = 'detail'
      await selectGroupRoom(g)
      await nextTick()
      const el = document.getElementById('groom-' + item.tg_msg_id)
      if (el) { el.scrollIntoView({ behavior: 'smooth', block: 'center' }); el.classList.add('highlight-anim'); setTimeout(() => el.classList.remove('highlight-anim'), 1800) }
    }
  }
}

// ---- computed ----
const filteredGroups = computed(() => {
  const q = groupSearchQuery.value.toLowerCase()
  if (!q) return groupStore.groups
  return groupStore.groups.filter((g: any) =>
    (g.group_title||'').toLowerCase().includes(q) || String(g.group_id).includes(q)
  )
})

const filteredGroupMembers = computed(() => {
  const q = groupMemberSearchQuery.value.toLowerCase()
  const all = q ? groupMembers.value.filter((m: any) =>
    (m.first_name||'').toLowerCase().includes(q) || (m.username||'').toLowerCase().includes(q) || String(m.user_id).includes(q)
  ) : groupMembers.value
  return all.slice(0, memberVisibleCount.value)
})

// ---- lifecycle ----
onMounted(() => {
  loadGroups()
  loadVerifyGrowthSummary()
  loadRevokeSyncChatInfo()
})
onUnmounted(() => {
  memberObserver?.disconnect()
  clearGroupLongPressTimer()
  if (revokeSyncToastTimer) {
    clearTimeout(revokeSyncToastTimer)
    revokeSyncToastTimer = null
  }
})

watch(showGroupMembersModal, (v) => {
  if (v) { memberVisibleCount.value = MEMBER_PAGE_SIZE; setTimeout(setupScrollObserver, 400) }
  else { memberObserver?.disconnect(); memberObserver = null }
})

watch(() => wsStore.lastEvent, (evt) => {
  if (!evt) return
  if (evt.type === 'group_activity') {
    if (selectedGroupRoomId.value && evt.data.group_id === selectedGroupRoomId.value)
      selectGroupRoom({ group_id: selectedGroupRoomId.value, group_title: selectedGroupRoomTitle.value })
    loadGroups()
  } else if (evt.type === 'group_listen_update') {
    loadGroups()
  } else if (evt.type === 'group_message_deleted') {
    // 实时删除被撤回的群消息（revoke sync）
    const { group_id, tg_msg_id } = evt.data
    if (group_id && tg_msg_id) {
      if (selectedGroupRoomId.value === group_id) {
        groupRoomMessages.value = groupRoomMessages.value.filter(
          (m: any) => m.tg_msg_id !== tg_msg_id
        )
      }
      // 同时从历史弹窗列表移除
      groupMessages.value = groupMessages.value.filter(
        (m: any) => !(m.group_id === group_id && m.tg_msg_id === tg_msg_id)
      )
    }
  } else if (evt.type === 'group_revoke_sync_update') {
    // 同步 revoke 开关状态
    const { group_id, enabled } = evt.data
    if (group_id && selectedGroupRoomId.value === group_id) {
      selectedGroupRevokeSync.value = enabled
    }
  }
})

function setupScrollObserver() {
  if (!memberScrollSentinel.value) return
  memberObserver?.disconnect()
  memberObserver = new IntersectionObserver((entries) => {
    if (entries[0].isIntersecting) memberVisibleCount.value += MEMBER_PAGE_SIZE
  }, { threshold: 0.1 })
  memberObserver.observe(memberScrollSentinel.value)
}

// ---- group list ----
async function loadGroups() {
  await groupStore.fetchGroupList()
  // preload group avatars
  for (const g of groupStore.groups as any[]) {
    if (!groupAvatarMap.value[g.group_id]) {
      try {
        const res = await fetch(`/api/group_photo?group_id=${g.group_id}`, { headers: authH() })
        const d = await res.json()
        if (d.file_id) {
          groupAvatarMap.value = { ...groupAvatarMap.value, [g.group_id]: d.file_id }
          await fileStore.autoLoadFile(d.file_id)
        }
      } catch {}
    }
  }
}

// ---- group room ----
// ---- 消息分页 ----
const groupMsgPage = ref(1)
const hasMoreGroupMsgs = ref(true)
const isLoadingMoreGroupMsgs = ref(false)
const GROUP_MSG_PAGE_SIZE = 20

async function selectGroupRoom(g: any) {
  selectedGroupRoomId.value = g.group_id
  selectedGroupRoomTitle.value = g.group_title
  isGroupRoomLoading.value = true
  groupRoomMessages.value = []
  groupMsgPage.value = 1
  hasMoreGroupMsgs.value = true
  ballOpacity.value = 0
  try {
    const res = await fetch(`/api/group_messages?group_id=${g.group_id}&limit=${GROUP_MSG_PAGE_SIZE}&offset=0`, { headers: authH() })
    const d = await res.json()
    groupRoomMessages.value = d.status === 'ok' ? (d.messages || d.data || []) : []
    hasMoreGroupMsgs.value = groupRoomMessages.value.length >= GROUP_MSG_PAGE_SIZE
    // preload avatars for room messages
    const fileIds = groupRoomMessages.value.map((m: any) => m.file_id).filter(Boolean)
    await fileStore.preloadFiles(fileIds)
    for (const m of groupRoomMessages.value) {
      if (m.user_id && m.avatar_file_id) {
        groupRoomAvatarMap.value = { ...groupRoomAvatarMap.value, [m.user_id]: m.avatar_file_id }
        await fileStore.autoLoadFile(m.avatar_file_id)
      }
    }
    // update last message map
    if (groupRoomMessages.value.length > 0) {
      const last = groupRoomMessages.value[groupRoomMessages.value.length - 1]
      groupLastMessageMap.value = { ...groupLastMessageMap.value, [g.group_id]: last.content || '' }
    }
    // 滚动到最新消息（底部）
    await nextTick()
    scrollToBottom('instant')
    // 加载 revoke sync 状态
    await loadRevokeSync()
  } finally { isGroupRoomLoading.value = false }
}

async function loadMoreGroupMessages() {
  if (!selectedGroupRoomId.value || isLoadingMoreGroupMsgs.value || !hasMoreGroupMsgs.value) return
  isLoadingMoreGroupMsgs.value = true
  const el = msgListRef.value
  const prevScrollHeight = el?.scrollHeight || 0
  try {
    const offset = groupMsgPage.value * GROUP_MSG_PAGE_SIZE
    const res = await fetch(`/api/group_messages?group_id=${selectedGroupRoomId.value}&limit=${GROUP_MSG_PAGE_SIZE}&offset=${offset}`, { headers: authH() })
    const d = await res.json()
    const older = d.status === 'ok' ? (d.messages || d.data || []) : []
    if (older.length < GROUP_MSG_PAGE_SIZE) hasMoreGroupMsgs.value = false
    if (older.length > 0) {
      groupRoomMessages.value = [...older, ...groupRoomMessages.value]
      groupMsgPage.value++
      await nextTick()
      if (el) el.scrollTop = el.scrollHeight - prevScrollHeight
    } else {
      hasMoreGroupMsgs.value = false
    }
  } finally { isLoadingMoreGroupMsgs.value = false }
}

async function sendGroupRoomMessage() {
  if (!selectedGroupRoomId.value || !groupRoomInput.value.trim()) return
  const body: any = { group_id: selectedGroupRoomId.value, content: groupRoomInput.value.trim() }
  if (groupRoomReplyingTo.value) {
    body.reply_to_tg_msg_id = groupRoomReplyingTo.value.tg_msg_id
    body.quoted_text = (groupRoomReplyingTo.value.content || '').slice(0, 100)
  }
  await fetch('/api/group_messages/send', { method: 'POST', headers: authH(), body: JSON.stringify(body) })
  groupRoomInput.value = ''
  groupRoomReplyingTo.value = null
  await selectGroupRoom({ group_id: selectedGroupRoomId.value, group_title: selectedGroupRoomTitle.value })
  await nextTick()
  scrollToBottom('smooth')
}

function scrollToGroupRoomMessage(tgMsgId: number) {
  const el = document.getElementById('groom-' + tgMsgId)
  if (el) el.scrollIntoView({ behavior: 'smooth', block: 'center' })
}

/**
 * scrollToMessage — 带高亮闪烁动画的跳转
 * 为目标消息根节点增加短暂黄色背景，2秒后自动消除
 */
function scrollToMessage(tgMsgId: number) {
  const el = document.getElementById('groom-' + tgMsgId)
  if (!el) return
  el.scrollIntoView({ behavior: 'smooth', block: 'center' })
  // 闪烁动画
  el.classList.add('bg-yellow-100', '!bg-yellow-100')
  setTimeout(() => {
    el.classList.remove('bg-yellow-100', '!bg-yellow-100')
  }, 1800)
}

/** 点击内联引用按钮：将消息设为 replyingTo，显示预览条 */
function quoteMessage(msg: any) {
  groupRoomReplyingTo.value = msg
}

function openGroupMobileActionMenu(msg: any) {
  groupMobileActionMsg.value = msg
}

function closeGroupMobileActionMenu() {
  groupMobileActionMsg.value = null
}

function handleGroupMobileQuote() {
  if (!groupMobileActionMsg.value) return
  quoteMessage(groupMobileActionMsg.value)
  closeGroupMobileActionMenu()
}

async function handleGroupMobileDelete() {
  if (!groupMobileActionMsg.value) return
  await deleteGroupMessageInline(groupMobileActionMsg.value)
  closeGroupMobileActionMenu()
}

function handleGroupMobileChat() {
  if (!groupMobileActionMsg.value) return
  emit('chat', groupMobileActionMsg.value.user_id, groupMobileActionMsg.value.username || groupMobileActionMsg.value.first_name || String(groupMobileActionMsg.value.user_id))
  closeGroupMobileActionMenu()
}

function clearGroupLongPressTimer() {
  if (groupLongPressTimer.value) {
    clearTimeout(groupLongPressTimer.value)
    groupLongPressTimer.value = null
  }
}

function onGroupMsgTouchStart(msg: any) {
  clearGroupLongPressTimer()
  groupLongPressTimer.value = setTimeout(() => {
    if (typeof navigator !== 'undefined' && 'vibrate' in navigator) {
      navigator.vibrate(10)
    }
    openGroupMobileActionMenu(msg)
    clearGroupLongPressTimer()
  }, 380)
}

function onGroupMsgTouchMove() {
  clearGroupLongPressTimer()
}

function onGroupMsgTouchEnd() {
  clearGroupLongPressTimer()
}

/** 内联删除（直接在主消息列表操作） */
async function deleteGroupMessageInline(msg: any) {
  if (!selectedGroupRoomId.value || !msg.tg_msg_id) return
  await fetch('/api/group_messages/delete', {
    method: 'POST', headers: authH(),
    body: JSON.stringify({ group_id: selectedGroupRoomId.value, tg_msg_id: msg.tg_msg_id, user_id: msg.user_id })
  })
  groupRoomMessages.value = groupRoomMessages.value.filter((x: any) => x.tg_msg_id !== msg.tg_msg_id)
}

// ---- history modal ----
async function viewGroupHistory() {
  if (!selectedGroupRoomId.value) return
  showGroupMessagesModal.value = true
  isMessagesLoading.value = true
  try {
    const res = await fetch(`/api/group_messages?group_id=${selectedGroupRoomId.value}&limit=50&offset=0`, { headers: authH() })
    const d = await res.json()
    groupMessages.value = d.status === 'ok' ? (d.messages || d.data || []) : []
    const fileIds = groupMessages.value.map((m: any) => m.file_id).filter(Boolean)
    await fileStore.preloadFiles(fileIds)
  } finally { isMessagesLoading.value = false }
}

function scrollToGroupMessage(tgMsgId: number) {
  const el = document.getElementById('group-msg-' + tgMsgId)
  if (el) el.scrollIntoView({ behavior: 'smooth', block: 'center' })
}

function canDeleteGroupMessage(m: any): boolean {
  // show delete if sender is a regular user (not admin/bot sender)
  return m.sender_type === 'user' && !!m.tg_msg_id
}

async function deleteGroupMessage(m: any) {
  if (!selectedGroupRoomId.value) return
  await fetch('/api/group_messages/delete', {
    method: 'POST', headers: authH(),
    body: JSON.stringify({ group_id: selectedGroupRoomId.value, tg_msg_id: m.tg_msg_id, user_id: m.user_id })
  })
  groupMessages.value = groupMessages.value.filter((x: any) => x.tg_msg_id !== m.tg_msg_id)
}

// ---- reply modal ----
function openGroupReplyModal(m: any) {
  groupReplyTarget.value = m
  groupReplyText.value = ''
  showGroupReplyModal.value = true
}
function closeGroupReplyModal() {
  showGroupReplyModal.value = false
  groupReplyTarget.value = null
  groupReplyText.value = ''
}
async function submitGroupReply() {
  if (!groupReplyTarget.value || !groupReplyText.value.trim() || !selectedGroupRoomId.value) return
  isGroupReplySending.value = true
  try {
    await fetch('/api/group_messages/reply', {
      method: 'POST', headers: authH(),
      body: JSON.stringify({
        group_id: selectedGroupRoomId.value,
        tg_msg_id: groupReplyTarget.value.tg_msg_id,
        content: groupReplyText.value.trim(),
        quoted_text: groupReplyTarget.value.content
      })
    })
    closeGroupReplyModal()
  } finally { isGroupReplySending.value = false }
}

// ---- members ----
async function loadGroupMembers() {
  if (!selectedGroupRoomId.value) return
  isMembersLoading.value = true
  memberVisibleCount.value = MEMBER_PAGE_SIZE
  try {
    const res = await fetch(`/api/group_members?group_id=${selectedGroupRoomId.value}`, { headers: authH() })
    const d = await res.json()
    groupMembers.value = d.status === 'ok' ? (d.members || d.data || []) : []
    const fids = groupMembers.value.map((m: any) => m.avatar_file_id).filter(Boolean)
    await fileStore.preloadFiles(fids)
  } finally { isMembersLoading.value = false }
}

function toggleMemberSelect(id: number) {
  const i = selectedMemberIds.value.indexOf(id)
  if (i >= 0) selectedMemberIds.value.splice(i, 1)
  else selectedMemberIds.value.push(id)
}
function clearSelectedMembers() { selectedMemberIds.value = [] }
function toggleSelectAllFilteredMembers() {
  const ids = filteredGroupMembers.value.map((m: any) => m.user_id)
  const allSel = ids.every((id: number) => selectedMemberIds.value.includes(id))
  if (allSel) selectedMemberIds.value = selectedMemberIds.value.filter(id => !ids.includes(id))
  else ids.forEach((id: number) => { if (!selectedMemberIds.value.includes(id)) selectedMemberIds.value.push(id) })
}

async function batchAction(action: string) {
  if (!selectedGroupRoomId.value || selectedMemberIds.value.length === 0) return
  const ep = (action === 'promote' || action === 'demote')
    ? '/api/group_members/admin_batch'
    : '/api/group_members/action_batch'
  await fetch(ep, {
    method: 'POST', headers: authH(),
    body: JSON.stringify({ group_id: selectedGroupRoomId.value, user_ids: selectedMemberIds.value, action })
  })
  selectedMemberIds.value = []
  await loadGroupMembers()
}

async function memberAction(m: any, action: string) {
  await fetch('/api/group_members/action', {
    method: 'POST', headers: authH(),
    body: JSON.stringify({ group_id: selectedGroupRoomId.value, user_id: m.user_id, action })
  })
}

async function directUidAction(action: string) {
  const uid = parseInt(directUidInput.value.trim())
  if (!uid || !selectedGroupRoomId.value) return
  directUidResult.value = null
  try {
    const res = await fetch('/api/group_members/action', {
      method: 'POST', headers: authH(),
      body: JSON.stringify({ group_id: selectedGroupRoomId.value, user_id: uid, action })
    })
    const d = await res.json()
    directUidResult.value = { ok: d.status === 'ok', msg: d.status === 'ok' ? '操作成功' : (d.message || '操作失败') }
  } catch { directUidResult.value = { ok: false, msg: '网络错误' } }
  setTimeout(() => { directUidResult.value = null }, 3000)
}

// ---- verify config ----
async function openVerifyConfig() {
  if (!selectedGroupRoomId.value) return
  const res = await fetch(`/api/group_verify_config?group_id=${selectedGroupRoomId.value}`, { headers: authH() })
  const d = await res.json()
  const cfg = d.config || d.data || {}
  verifyConfigForm.value = {
    verify_enabled: !!cfg.verify_enabled,
    verify_type: cfg.verify_type || 'captcha',
    verify_timeout: cfg.verify_timeout || 120,
    verify_channel_id: cfg.verify_channel_id || '',
    verify_msg_template: cfg.verify_msg_template || '',
    verify_questions: cfg.verify_questions || []
  }
  // 同时加载统计数据
  verifyStats.value = null
  try {
    const sr = await fetch(`/api/group_stats/verify_stats?group_id=${selectedGroupRoomId.value}`, { headers: authH() })
    const sd = await sr.json()
    if (sd.status === 'ok') verifyStats.value = sd.data
  } catch {}
  showVerifyConfigModal.value = true
}

async function saveVerifyConfig() {
  if (!selectedGroupRoomId.value) return
  const payload = { group_id: selectedGroupRoomId.value, ...verifyConfigForm.value }
  const res = await fetch('/api/group_verify_config', { method: 'POST', headers: authH(), body: JSON.stringify(payload) })
  const d = await res.json()
  if (d.status === 'ok') {
    groupVerifyConfig.value = { ...groupVerifyConfig.value, [selectedGroupRoomId.value!]: { ...verifyConfigForm.value } }
    showVerifyConfigModal.value = false
  }
}

async function loadVerifyGrowthSummary() {
  try {
    const res = await fetch('/api/group_stats/verify_growth', { headers: authH() })
    const d = await res.json()
    if (d.status === 'ok') verifyGrowthSummary.value = d.summary
  } catch {}
}

async function openVerifiedUsersModal(period: 'today'|'all') {
  verifiedUsersPeriod.value = period
  isVerifiedUsersModalOpen.value = true
  console.log(`打开验证用户弹窗: ${period}`)
  try {
    // 默认显示已验证用户标签
    verifiedUsersTab.value = 'verified'
    await loadVerifiedUsers(period)
    console.log(`已加载 ${verifiedUsersList.value.length} 个已验证用户`)
  } catch (e) {
    console.error('加载验证用户失败:', e)
  }
}

async function loadVerifiedUsers(period: 'today'|'all' = 'all') {
  isLoadingVerified.value = true
  try {
    const res = await fetch(`/api/group_listen/verified_users?period=${period}`, {
      headers: { 'Authorization': `Bearer ${localStorage.getItem('admin_token') || ''}` }
    })
    const data = await res.json()
    if (data.status === 'ok') {
      verifiedUsersList.value = data.data || []
      console.log(`已加载 ${verifiedUsersList.value.length} 个已验证用户`)
    } else {
      console.error('API 返回错误:', data.message)
    }
  } catch (e) {
    console.error('加载已验证用户失败:', e)
  } finally {
    isLoadingVerified.value = false
  }
}

async function loadUnverifiedUsers(period: 'today'|'all' = 'all') {
  isLoadingUnverified.value = true
  try {
    const res = await fetch(`/api/group_listen/unverified_users?period=${period}`, {
      headers: { 'Authorization': `Bearer ${localStorage.getItem('admin_token') || ''}` }
    })
    const data = await res.json()
    if (data.status === 'ok') {
      unverifiedUsers.value = data.data || []
      console.log(`已加载 ${unverifiedUsers.value.length} 个未关注用户`)
    } else {
      console.error('API 返回错误:', data.message)
    }
  } catch (e) {
    console.error('加载未关注用户失败:', e)
  } finally {
    isLoadingUnverified.value = false
  }
}

function toggleUserSelection(userId: number) {
  if (selectedUsers.value.has(userId)) {
    selectedUsers.value.delete(userId)
  } else {
    selectedUsers.value.add(userId)
  }
}

function selectAllUnverifiedUsers() {
  if (selectedUsers.value.size === unverifiedUsers.value.length) {
    selectedUsers.value.clear()
  } else {
    unverifiedUsers.value.forEach(u => selectedUsers.value.add(u.user_id))
  }
}

// 计算属性：按群组分类的未关注用户
const groupedUnverifiedUsers = computed(() => {
  const grouped: Record<number, any[]> = {}
  unverifiedUsers.value.forEach(user => {
    if (!grouped[user.group_id]) {
      grouped[user.group_id] = []
    }
    grouped[user.group_id].push(user)
  })
  return grouped
})

// 检查群组是否全选
function isGroupFullySelected(groupId: number): boolean {
  const groupUsers = groupedUnverifiedUsers.value[groupId] || []
  return groupUsers.length > 0 && groupUsers.every(u => selectedUsers.value.has(u.user_id))
}

// 切换群组全选
function toggleGroupSelection(groupId: number) {
  const groupUsers = groupedUnverifiedUsers.value[groupId] || []
  const isFullySelected = isGroupFullySelected(groupId)
  
  groupUsers.forEach(user => {
    if (isFullySelected) {
      selectedUsers.value.delete(user.user_id)
    } else {
      selectedUsers.value.add(user.user_id)
    }
  })
}

async function sendChannelReminders() {
  if (selectedUsers.value.size === 0) {
    console.warn('未选择任何用户')
    return
  }
  
  isSendingReminders.value = true
  try {
    // 获取频道链接（从配置或用户输入）
    const channelLink = prompt('请输入频道链接 (例如: https://t.me/mychannel):')
    if (!channelLink) {
      isSendingReminders.value = false
      return
    }
    
    console.log(`📤 开始发送提醒...`)
    console.log(`频道链接: ${channelLink}`)
    console.log(`已选择用户数: ${selectedUsers.value.size}`)
    
    // 按群组分组发送
    const grouped: Record<number, number[]> = {}
    selectedUsers.value.forEach(userId => {
      const user = unverifiedUsers.value.find(u => u.user_id === userId)
      if (user) {
        if (!grouped[user.group_id]) {
          grouped[user.group_id] = []
        }
        grouped[user.group_id].push(userId)
        console.log(`用户 ${userId} 属于群组 ${user.group_id}`)
      }
    })
    
    console.log(`📊 群组分组结果:`, grouped)
    
    let totalSent = 0
    for (const [groupId, userIds] of Object.entries(grouped)) {
      try {
        console.log(`\n📤 发送到群组 ${groupId}，用户数: ${userIds.length}`)
        
        const payload = {
          group_id: parseInt(groupId),
          user_ids: userIds,
          channel_link: channelLink
        }
        
        console.log(`请求数据:`, JSON.stringify(payload, null, 2))
        
        const res = await fetch('/api/group_listen/send_channel_reminder', {
          method: 'POST',
          headers: {
            'Authorization': `Bearer ${localStorage.getItem('admin_token') || ''}`,
            'Content-Type': 'application/json'
          },
          body: JSON.stringify(payload)
        })
        
        console.log(`HTTP 状态码: ${res.status}`)
        
        const data = await res.json()
        console.log(`响应数据:`, data)
        
        if (data.status === 'ok') {
          const sentCount = data.sent_count || 0
          totalSent += sentCount
          console.log(`✅ 群组 ${groupId}: 已发送 ${sentCount} 条提醒消息`)
        } else {
          console.error(`❌ 群组 ${groupId} 发送失败:`, data.message)
        }
      } catch (e) {
        console.error(`❌ 群组 ${groupId} 请求失败:`, e)
      }
    }
    
    console.log(`\n✅ 总共已发送 ${totalSent} 条提醒消息`)
    selectedUsers.value.clear()
    // 重新加载未关注用户列表
    await loadUnverifiedUsers(verifiedUsersPeriod.value)
  } catch (e) {
    console.error('❌ 发送提醒请求失败:', e)
  } finally {
    isSendingReminders.value = false
  }
}

function closeVerifiedUsersModal() {
  isVerifiedUsersModalOpen.value = false
  verifiedUsersList.value = []
  unverifiedUsers.value = []
  selectedUsers.value.clear()
  verifiedUsersTab.value = 'verified'
}

function jumpToUserChat(user: any) {
  closeVerifiedUsersModal()
  emit('chat', user.user_id, user.username || user.first_name || String(user.user_id))
}

function openMuteModal(user: any) {
  muteTargetUser.value = user
  muteDuration.value = 3600 // 重置为默认1小时
  showMuteModal.value = true
}

function closeMuteModal() {
  showMuteModal.value = false
  muteTargetUser.value = null
}

async function confirmMute() {
  if (!muteTargetUser.value) return
  isMuting.value = true
  try {
    const res = await fetch('/api/group_members/action', {
      method: 'POST',
      headers: authH(),
      body: JSON.stringify({
        group_id: muteTargetUser.value.group_id,
        user_id: muteTargetUser.value.user_id,
        action: 'mute',
        duration: muteDuration.value
      })
    })
    const d = await res.json()
    if (d.status === 'ok') {
      console.log(`已禁言用户 ${muteTargetUser.value.user_id}，群组 ${muteTargetUser.value.group_id}，时长 ${muteDuration.value} 秒`)
      closeMuteModal()
    } else {
      console.error('禁言失败:', d.message)
    }
  } catch (e) {
    console.error('禁言请求失败:', e)
  } finally {
    isMuting.value = false
  }
}

// ---- utils ----
function hideSearchResultsDelayed() { setTimeout(() => { showSearchResults.value = false }, 200) }
function openUrl(url: string) { openExternalLink(url) }

function isVideoSticker(url: string): boolean {
  return url.endsWith('.webm') || url.includes('.webm?')
}

function getAdScore(msg: any): number {
  const v = Number(msg?.ad_score ?? 0)
  return Number.isFinite(v) ? v : 0
}

function getAdBadgeText(msg: any): string {
  const score = getAdScore(msg)
  const flagged = !!msg?.is_advertising
  if (score >= 60) return `广告 ${score}`
  if (flagged || score >= 40) return `疑似广告 ${score}`
  if (score >= 25) return `推广倾向 ${score}`
  return ''
}

function getAdBadgeClass(msg: any): string {
  const score = getAdScore(msg)
  if (score >= 60) return 'bg-rose-100 text-rose-600'
  if (score >= 40 || !!msg?.is_advertising) return 'bg-amber-100 text-amber-700'
  return 'bg-slate-100 text-slate-600'
}

function getAdBadgeTitle(msg: any): string {
  const findings = Array.isArray(msg?.ad_findings)
    ? msg.ad_findings.filter((x: any) => typeof x === 'string')
    : []
  const score = getAdScore(msg)
  if (!findings.length) return `广告评分: ${score}`
  return `广告评分: ${score}\n` + findings.join('\n')
}

function formatTime(ts: any): string {
  if (!ts) return ''
  const d = new Date(typeof ts === 'number' ? ts * 1000 : String(ts).replace(' ', 'T'))
  if (isNaN(d.getTime())) return ''
  return d.toLocaleString('zh-CN', { month: '2-digit', day: '2-digit', hour: '2-digit', minute: '2-digit' })
}

function formatDateTime(ts: any): string {
  if (!ts) return ''
  const d = new Date(typeof ts === 'number' ? ts * 1000 : String(ts).replace(' ', 'T'))
  if (isNaN(d.getTime())) return ''
  return d.toLocaleString('zh-CN', { month: '2-digit', day: '2-digit', hour: '2-digit', minute: '2-digit' })
}
</script>

<style scoped>
.revoke-sync-entry-btn {
  box-shadow: 0 8px 18px rgba(15, 23, 42, 0.08);
}

:global(html.dark) .revoke-sync-entry-btn {
  background: rgba(15, 30, 52, 0.94) !important;
  color: #8fdcff !important;
  border-color: rgba(73, 106, 148, 0.32) !important;
  box-shadow: 0 12px 26px rgba(2, 6, 23, 0.34);
}

:global(html.dark) .revoke-sync-modal {
  background:
    linear-gradient(180deg, rgba(9, 18, 31, 0.98) 0%, rgba(7, 13, 24, 0.98) 100%) !important;
  border-color: rgba(73, 106, 148, 0.24) !important;
  box-shadow: 0 30px 60px rgba(2, 6, 23, 0.62);
}

:global(html.dark) .revoke-sync-modal-head {
  background:
    linear-gradient(135deg, rgba(36, 161, 222, 0.16) 0%, rgba(36, 161, 222, 0.06) 38%, rgba(6, 13, 24, 0.08) 100%) !important;
}

:global(html.dark) .revoke-sync-panel {
  background: rgba(11, 22, 38, 0.88) !important;
  border-color: rgba(73, 106, 148, 0.22) !important;
}

:global(html.dark) .revoke-sync-assign-panel {
  background: rgba(10, 18, 32, 0.88) !important;
  border-color: rgba(73, 106, 148, 0.22) !important;
}

:global(html.dark) .revoke-sync-input {
  background: rgba(7, 13, 24, 0.92) !important;
  border-color: rgba(73, 106, 148, 0.24) !important;
  color: #edf4ff !important;
}

:global(html.dark) .revoke-sync-input::placeholder {
  color: rgba(161, 183, 213, 0.72) !important;
}

:global(html.dark) .revoke-sync-group-item {
  background: rgba(10, 18, 32, 0.9) !important;
  border-color: rgba(73, 106, 148, 0.18) !important;
}

:global(html.dark) .revoke-sync-group-item:hover {
  background: rgba(17, 39, 63, 0.92) !important;
  border-color: rgba(36, 161, 222, 0.26) !important;
}

:global(html.dark) .revoke-sync-footer {
  background: rgba(7, 13, 24, 0.94) !important;
}

:global(html.dark) .revoke-sync-toast {
  box-shadow: 0 24px 48px rgba(2, 6, 23, 0.52);
}

:global(html.dark) .revoke-sync-scroll::-webkit-scrollbar-thumb {
  background: rgba(79, 107, 146, 0.68);
}

:global(html.dark) .revoke-sync-scroll::-webkit-scrollbar-track {
  background: rgba(11, 21, 37, 0.42);
}

.mobile-longpress-target {
  -webkit-touch-callout: none;
  -webkit-user-select: none;
  user-select: none;
}

@media (min-width: 768px) {
  .mobile-longpress-target {
    -webkit-touch-callout: default;
    -webkit-user-select: text;
    user-select: text;
  }
}

/* ── 磨砂玻璃圆球 ── */
.scroll-ball {
  position: absolute;
  right: 16px;
  /* 垂直居中于消息容器，避开滚动内容遮挡 */
  bottom: 16px;
  width: 44px;
  height: 44px;
  border-radius: 50%;
  /* 磨砂玻璃：白昼模式 */
  background: rgba(255, 255, 255, 0.80);
  backdrop-filter: blur(20px);
  -webkit-backdrop-filter: blur(20px);
  /* 极柔和扩散阴影，无生硬边框 */
  box-shadow:
    0 0 0 1px rgba(0, 0, 0, 0.04),
    0 4px 24px rgba(0, 0, 0, 0.08),
    0 1px 4px rgba(0, 0, 0, 0.04);
  display: flex;
  align-items: center;
  justify-content: center;
  cursor: pointer;
  /* 透明度过渡：144Hz 流畅，无阶梯感 */
  transition: opacity 80ms linear;
  will-change: opacity, transform;
  transform: translateZ(0);
  z-index: 30;
}

:global(html.dark) .scroll-ball,
:global(body.dark) .scroll-ball {
  background: rgba(6, 13, 24, 0.84);
  box-shadow:
    0 0 0 1px rgba(73, 106, 148, 0.24),
    0 8px 30px rgba(2, 6, 23, 0.34),
    0 1px 6px rgba(4, 14, 29, 0.22);
}

.scroll-ball:active {
  /* 触摸反馈，配合 GSAP scale 使用 */
  transform: translateZ(0) scale(0.92);
}
</style>
