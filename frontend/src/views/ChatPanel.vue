<template>
  <div class="flex h-full min-h-0 w-full animate-in fade-in duration-300">
    <div class="border-r border-outline-variant/20 flex flex-col bg-surface-container-lowest" :class="mobilePanelView==='list'?'w-full md:w-80':'hidden md:flex md:w-80'">
      <div class="p-6 border-b border-outline-variant/10">
        <div class="flex items-center justify-between mb-4">
          <h2 class="text-xl font-black">用户列表</h2>
          <span class="bg-primary/10 text-primary text-[10px] font-bold px-2 py-1 rounded-full">{{ chatStore.users.length }} 用户</span>
        </div>
        <div class="relative">
          <span class="material-symbols-outlined absolute left-3 top-1/2 -translate-y-1/2 text-slate-400 text-sm">search</span>
          <input type="text" v-model="searchQuery" placeholder="搜索 ID 或用户名..." class="w-full pl-10 pr-4 py-2.5 bg-surface-container rounded-xl border-none text-sm focus:ring-2 focus:ring-primary/20"/>
        </div>
      </div>
      <div class="flex-1 overflow-y-auto no-scrollbar px-2">
        <div v-if="chatStore.isLoading" class="flex flex-col items-center justify-center py-20">
          <div class="w-8 h-8 border-4 border-primary/20 border-t-primary rounded-full animate-spin"></div>
        </div>
        <div v-for="user in filteredUsers" :key="user.from_userid"
             @click="selectUser(user);mobilePanelView='detail'"
             class="chat-contact-item px-3 py-3 rounded-xl cursor-pointer transition-all duration-200 flex items-center space-x-4 border-b border-outline-variant/20 last:border-b-0"
             :class="chatStore.selectedUserId===user.from_userid?'bg-primary/12 border-primary/30 shadow-sm':'hover:bg-surface-container/80 border-outline-variant/20'">
          <div class="relative flex-shrink-0">
            <div v-if="user.avatar_file_id && chatStore.fileUrls[user.avatar_file_id]" class="w-12 h-12 rounded-2xl overflow-hidden shadow-sm">
              <img :src="userAvatarSrc(user)" class="w-12 h-12 object-cover block" @error="handleAvatarLoadError(user.avatar_file_id)"/>
            </div>
            <div v-else class="w-12 h-12 rounded-2xl bg-gradient-to-br from-primary to-primary-container flex items-center justify-center text-on-primary font-bold">
              {{ (user.from_username||user.username||'U').charAt(0).toUpperCase() }}
            </div>
            <div v-if="user.unread_count>0" class="absolute -top-1 -right-1 bg-error text-white text-[9px] font-bold w-5 h-5 rounded-full flex items-center justify-center border-2 border-surface-container-lowest">{{ user.unread_count }}</div>
          </div>
          <div class="flex-1 min-w-0">
            <div class="flex items-center justify-between mb-0.5">
              <span class="font-bold text-sm truncate">{{ getUserDisplayName(user) }}</span>
              <span class="text-[9px] text-slate-400">{{ formatShortTime(user.last_msg_time) }}</span>
            </div>
            <a v-if="!user.is_ai_assistant && getUserProfileUsernameHint(user)"
               :href="tgUserProfileHref(user.from_userid, getUserProfileUsernameHint(user))"
               class="text-[10px] text-slate-400 truncate hover:underline hover:text-primary block"
               @click.stop.prevent="openTgUserProfile(user.from_userid, getUserProfileUsernameHint(user))">{{ getUserIdentityLabel(user) }}</a>
            <span v-else class="text-[10px] text-slate-400 truncate block">{{ getUserIdentityLabel(user) }}</span>
          </div>
        </div>
      </div>
    </div>
    <div class="flex-1 min-w-0 flex flex-col bg-surface relative" :class="mobilePanelView==='detail'?'flex':'hidden md:flex'">
      <div v-if="!chatStore.selectedUserId" class="flex-1 flex flex-col items-center justify-center cartesian-grid">
        <div class="w-24 h-24 bg-primary/5 rounded-full flex items-center justify-center mb-6"><span class="material-symbols-outlined text-primary text-5xl opacity-40">forum</span></div>
        <h2 class="text-xl font-bold text-slate-400">选择一个用户开始对话</h2>
      </div>
      <div v-else class="flex-1 flex flex-col overflow-hidden">
        <div class="px-3 sm:px-4 md:px-6 py-3 md:py-4 bg-surface-container-lowest/80 backdrop-blur-md border-b border-outline-variant/10 flex items-center justify-between gap-2 md:gap-4 z-10 min-w-0" style="will-change: transform;">
          <div class="flex items-center gap-2 sm:gap-3 md:gap-4 min-w-0 flex-1">
            <button class="md:hidden w-8 h-8 flex items-center justify-center rounded-xl hover:bg-surface-container mr-1" @click="mobilePanelView='list'"><span class="material-symbols-outlined text-slate-500">arrow_back</span></button>
            <div v-if="avatarUrl" class="w-10 h-10 rounded-xl overflow-hidden shadow-sm border border-outline-variant/10 flex-shrink-0 relative group cursor-pointer" @click="handleSelectedAvatarClick">
              <img :src="avatarUrl" class="w-10 h-10 object-cover block" @error="handleSelectedAvatarError"/>
              <!-- ★ 头像刷新按钮（hover时显示） -->
              <div class="absolute inset-0 bg-black/30 opacity-0 group-hover:opacity-100 transition-opacity flex items-center justify-center rounded-xl pointer-events-none">
                <span class="material-symbols-outlined text-white text-sm">{{ isSelectedAiAssistant ? 'settings' : 'refresh' }}</span>
              </div>
            </div>
            <div v-else class="w-10 h-10 rounded-xl bg-primary/10 flex items-center justify-center text-primary font-bold">{{ (chatStore.selectedUserName||'U').charAt(0).toUpperCase() }}</div>
            <div class="min-w-0"><h3 class="font-bold text-sm md:text-base truncate">{{ getUserDisplayName(selectedUserRecord, chatStore.selectedUserId) }}</h3>
              <a v-if="chatStore.selectedUserId && !isSelectedAiAssistant && getUserProfileUsernameHint(selectedUserRecord)"
                 :href="tgUserProfileHref(chatStore.selectedUserId, getUserProfileUsernameHint(selectedUserRecord))"
                 class="text-[10px] text-slate-400 hover:underline hover:text-primary truncate block"
                 @click.stop.prevent="openTgUserProfile(chatStore.selectedUserId, getUserProfileUsernameHint(selectedUserRecord))">{{ getUserIdentityLabel(selectedUserRecord, chatStore.selectedUserId) }}</a>
              <span v-else-if="chatStore.selectedUserId" class="text-[10px] text-slate-400 truncate block">{{ getUserIdentityLabel(selectedUserRecord, chatStore.selectedUserId) }}</span>
            </div>
          </div>
          <div class="flex items-center gap-1.5 md:gap-2 shrink-0">
            <button @click="markAsRead" class="inline-flex items-center justify-center md:justify-start px-2.5 md:px-4 py-2 text-[11px] md:text-xs font-bold text-primary bg-primary/5 hover:bg-primary/10 rounded-lg md:rounded-xl"><span class="material-symbols-outlined text-sm md:mr-1.5">done_all</span><span class="hidden sm:inline">标记已读</span></button>
            <button @click="deleteChat" class="inline-flex items-center justify-center md:justify-start px-2.5 md:px-4 py-2 text-[11px] md:text-xs font-bold text-error bg-error/5 hover:bg-error/10 rounded-lg md:rounded-xl"><span class="material-symbols-outlined text-sm md:mr-1.5">delete</span><span class="hidden sm:inline">清空记录</span></button>
          </div>
        </div>
        <div class="flex-1 min-h-0 relative overflow-hidden flex flex-col">
          <div
            v-if="chatBackgroundLayerStyle"
            class="chat-history-background absolute inset-0 pointer-events-none"
            :style="chatBackgroundLayerStyle"
          ></div>
          <div
            ref="chatHistoryRef"
            @scroll="handleScroll"
            class="flex-1 overflow-y-auto py-6 space-y-4 no-scrollbar relative select-none"
            :class="themeStore.settings.chatBackground ? 'bg-transparent z-[1]' : 'bg-surface'"
            :style="{ willChange: 'scroll-position' }"
          >
          <!-- 触顶加载更多指示器 -->
          <div class="flex justify-center pb-2">
            <div v-if="isLoadingMore" class="flex items-center gap-2 text-xs text-slate-400">
              <div class="w-4 h-4 border-2 border-primary/30 border-t-primary rounded-full animate-spin"></div>
              加载更多...
            </div>
            <div v-else-if="!hasMoreMessages && chatStore.messages.length > 0" class="text-[10px] text-slate-300 py-1">已加载全部消息</div>
          </div>
          <div v-for="msg in chatStore.messages" :key="msg.msg_id" :id="'msg-'+msg.tg_msg_id"
               class="relative flex flex-col w-full px-4 md:pr-12 lg:pr-8 gap-2" :class="msg.from_userid==chatStore.selectedUserId?'items-start':'items-end'">
            <div v-if="msg.quoted_text" @click="scrollToMessage(msg.quoted_msg_id)"
                 class="mb-[-8px] mx-2 px-3 py-2 rounded-t-[12px] border cursor-pointer transition-colors flex items-center space-x-3 opacity-80 max-w-[85%] sm:max-w-[75%] md:max-w-[65%]"
                 :class="themeStore.isDark ? 'bg-[#17304a]/70 border-[#2d4668] hover:bg-[#214161]' : 'bg-surface-container-high/60 border-outline-variant/10 hover:bg-surface-container-highest'">
              <img v-if="msg.quoted_type==='photo' && getQuotedMediaUrl(msg)" :src="getQuotedMediaUrl(msg)" class="w-10 h-10 object-cover rounded-lg flex-shrink-0"/>
              <div class="flex-1 min-w-0">
                <div class="text-[9px] font-bold text-primary uppercase tracking-tighter mb-0.5">引用回复</div>
                <div class="chat-copy-text text-[11px] truncate italic">{{ msg.quoted_text }}</div>
              </div>
            </div>
            <div class="chat-bubble rounded-[16px] px-4 py-3 relative overflow-hidden group break-words transition-all duration-200 hover:shadow-md select-none touch-manipulation mobile-longpress-target"
                 @contextmenu.prevent="onMsgContextMenu($event, msg)"
                 @touchstart.passive="onMsgTouchStart(msg)"
                 @touchmove.passive="onMsgTouchMove"
                 @touchend="onMsgTouchEnd"
                 @touchcancel="clearLongPressTimer"
                 :class="[
                   msg.from_userid==chatStore.selectedUserId
                     ? 'bg-gradient-to-br from-primary/10 to-primary/5 border border-primary/15 shadow-sm'
                     : (themeStore.isDark
                        ? 'bg-[#122034] border border-[#2d4668] shadow-none text-slate-100'
                        : 'bg-surface-container-lowest border border-outline-variant/20 shadow-md shadow-slate-200/30'),
                   'max-w-[94%] sm:max-w-[86%] md:max-w-[74%] w-auto'
                 ]"
                 style="will-change: transform;">
              <div v-if="msg.from_userid!=chatStore.selectedUserId && msg.admin_userid"
                   class="mb-2 flex items-center pb-1"
                   :class="themeStore.isDark ? 'border-b border-[#2d4668]' : 'border-b border-slate-100'">
                <span class="text-[9px] px-1.5 py-0.5 rounded mr-2 font-black"
                      :class="themeStore.isDark ? 'bg-[#17304a] text-slate-300' : 'bg-slate-100 text-slate-500'">客服</span>
                <a :href="tgUserProfileHref(Number(msg.admin_userid), msg.admin_username)"
                   class="text-[10px] font-black text-primary hover:underline"
                   @click.stop.prevent="openTgUserProfile(Number(msg.admin_userid), msg.admin_username)">{{ tgUserLinkLabel(Number(msg.admin_userid), msg.admin_username) }}</a>
              </div>
              <div v-if="getMsgType(msg) !== 'text' && hasMediaSource(msg)"
                   class="message-media-shell mb-2 rounded-[16px] overflow-hidden flex justify-center border shadow-sm"
                   :class="themeStore.isDark ? 'border-[#2d4668] bg-[#0b1525]' : 'border-outline-variant/30 bg-surface-container-lowest'">
                <img v-if="getMsgType(msg) === 'photo' && getMediaUrl(msg)"
                     :src="getMediaUrl(msg)"
                     class="max-w-full max-h-[400px] w-auto h-auto object-contain cursor-zoom-in"
                     @error="chatStore.fileErrors[msg.file_id]=true"/>
                <button v-else-if="getMsgType(msg) === 'photo'" @click="chatStore.autoLoadFile(msg.file_id)" class="px-3 py-2 text-xs font-bold underline text-primary">正在加载媒体...</button>

                <div v-else-if="getMsgType(msg) === 'sticker'" class="relative p-1">
                  <template v-if="getMediaUrl(msg)">
                    <video v-if="isVideoSticker(getMediaUrl(msg))" :src="getMediaUrl(msg)" autoplay loop muted playsinline controls class="w-32 h-32 object-contain rounded-lg"></video>
                    <img v-else :src="getMediaUrl(msg)" class="w-32 h-32 object-contain"/>
                  </template>
                  <button v-else @click="chatStore.autoLoadFile(msg.file_id)" class="px-3 py-2 text-xs font-bold underline text-primary">正在加载媒体...</button>
                </div>

                <video v-else-if="(getMsgType(msg) === 'video' || getMsgType(msg) === 'animation') && getMediaUrl(msg)"
                       :src="getMediaUrl(msg)" controls loop autoplay muted playsinline class="max-w-full max-h-[400px] w-auto h-auto rounded-lg"></video>
                <button v-else-if="getMsgType(msg) === 'video' || getMsgType(msg) === 'animation'" @click="chatStore.autoLoadFile(msg.file_id)" class="px-3 py-2 text-xs font-bold underline text-primary">正在加载媒体...</button>

                <div v-else-if="getMsgType(msg) === 'voice' || getMsgType(msg) === 'audio'"
                     class="p-2 rounded-lg flex items-center space-x-3"
                     :class="themeStore.isDark ? 'bg-[#17304a]' : 'bg-black/5'">
                  <span class="material-symbols-outlined text-slate-400">{{ getMsgType(msg)==='voice'?'mic':'music_note' }}</span>
                  <audio v-if="getMediaUrl(msg)" :src="getMediaUrl(msg)" controls class="h-8 max-w-[200px]"></audio>
                  <button v-else @click="chatStore.autoLoadFile(msg.file_id)" class="text-xs font-bold underline">正在加载媒体...</button>
                </div>

                <div v-else class="p-3 rounded-lg flex items-center space-x-3" :class="themeStore.isDark ? 'bg-[#17304a]' : 'bg-black/5'">
                  <span class="material-symbols-outlined text-slate-400">description</span>
                  <a v-if="getMediaUrl(msg)" :href="getMediaUrl(msg)" class="text-xs font-bold text-primary underline" @click.prevent="openExternalLink(getMediaUrl(msg))">下载</a>
                  <button v-else @click="chatStore.autoLoadFile(msg.file_id)" class="text-xs font-bold underline">正在加载媒体...</button>
                </div>
              </div>
              <div v-else-if="getMsgType(msg) !== 'text' && getMsgStatus(msg) !== 'sending'"
                   class="mb-2 px-3 py-2 rounded-xl text-xs font-semibold border"
                   :class="themeStore.isDark ? 'bg-amber-500/12 text-amber-300 border-amber-400/20' : 'bg-amber-50 text-amber-600 border-amber-200/60'">
                媒体文件不可用（历史消息可能缺少 file_id）
              </div>
              <div v-if="msg.translated_text && msg.original_text" class="mb-2 p-3 rounded-lg border-l-4"
                   :class="msg.from_userid==chatStore.selectedUserId
                     ? 'bg-primary/10 border-primary/30'
                     : (themeStore.isDark ? 'bg-[#17304a] border-[#3b587f]' : 'bg-slate-50 border-slate-200')">
                <div class="text-[9px] font-black uppercase tracking-widest mb-1 flex items-center text-slate-400"><span class="material-symbols-outlined text-xs mr-1">translate</span>自动翻译</div>
                <div class="chat-copy-text text-[15px]">{{ msg.translated_text }}</div>
              </div>
              <div v-if="msg.original_text" class="chat-copy-text whitespace-pre-wrap break-words [overflow-wrap:anywhere] text-[15px] leading-relaxed font-medium">
                <span class="text-[12px] font-bold block pt-2 mt-2"
                      :class="themeStore.isDark ? 'text-slate-500 border-t border-[#2d4668]' : 'text-black/30 border-t border-black/5'">原文:</span>{{ msg.original_text }}
              </div>
              <div v-if="shouldShowPlainContent(msg)"
                   class="chat-copy-text whitespace-pre-wrap break-words [overflow-wrap:anywhere] text-[15px] leading-relaxed font-medium"
                   :class="msg.from_userid==chatStore.selectedUserId
                     ? (themeStore.isDark ? 'text-slate-200' : 'text-slate-700')
                     : (themeStore.isDark ? 'text-slate-100' : 'text-slate-900')">{{ msg.content }}</div>
              <div class="chat-copy-ignore flex items-center gap-2 mt-2 pt-1.5"
                   :class="msg.from_userid==chatStore.selectedUserId
                     ? 'border-t border-primary/10'
                     : (themeStore.isDark ? 'border-t border-[#2d4668]' : 'border-t border-slate-100')">
                <span class="text-[9px] text-slate-400 mr-auto">{{ formatTime(msg.timestamp) }}</span>
                <span v-if="msg.from_userid!=chatStore.selectedUserId" class="text-[9px] flex items-center space-x-1">
                  <span v-if="getMsgStatus(msg)==='thinking'" class="material-symbols-outlined text-[10px] text-primary animate-pulse">psychology</span>
                  <span v-else class="material-symbols-outlined text-[10px]" :class="getMsgStatus(msg)==='sent'?'text-primary':getMsgStatus(msg)==='failed'?'text-error':'text-slate-300'">{{ getMsgStatus(msg)==='sent'?'done_all':getMsgStatus(msg)==='failed'?'error':'schedule' }}</span>
                  <span class="text-[9px]" :class="getMsgStatus(msg)==='failed'?'text-error':'text-slate-400'">{{ getMsgStatus(msg)==='sent'?'已送达':getMsgStatus(msg)==='failed'?'发送失败':getMsgStatus(msg)==='thinking'?'正在思考...':'发送中' }}</span>
                </span>
              </div>
            </div>
            <div v-if="isAiAssistantUserId(msg.from_userid) && getAgentUiActions(msg).length"
                 class="flex flex-wrap gap-2 max-w-[94%] sm:max-w-[86%] md:max-w-[74%]">
              <button v-for="action in getAgentUiActions(msg)" :key="action.id + JSON.stringify(action.payload || {})"
                      @click="handleAgentUiAction(action)"
                      :disabled="agentActionBusy[actionBusyKey(action)]"
                      class="inline-flex items-center gap-1.5 px-3 py-2 text-xs font-bold border rounded-lg disabled:opacity-50"
                      :class="action.style === 'danger'
                        ? 'border-rose-300 text-rose-600 bg-rose-50 hover:bg-rose-100'
                        : action.style === 'primary'
                          ? 'border-primary/30 text-primary bg-primary/5 hover:bg-primary/10'
                          : 'border-outline-variant/30 text-slate-600 bg-surface-container hover:bg-surface-container-high'">
                <span class="material-symbols-outlined text-sm">{{ action.icon || 'touch_app' }}</span>
                <span>{{ action.label }}</span>
                <span v-if="action.description" class="font-normal opacity-70">{{ action.description }}</span>
              </button>
            </div>
            <div v-if="isAiAssistantUserId(msg.from_userid) && isAgentJobRunning(getAgentJob(msg))"
                 class="w-[min(32rem,94%)] px-3 py-2 border border-primary/15 bg-primary/5 rounded-lg">
              <div class="flex items-center justify-between text-xs font-bold text-primary mb-1.5">
                <span>{{ agentJobStatusText(getAgentJob(msg)) }}</span>
                <span>{{ Number(getAgentJob(msg)?.progress || 0) }}%</span>
              </div>
              <div class="h-1.5 bg-primary/10 rounded-full overflow-hidden">
                <div class="h-full bg-primary transition-all duration-300" :style="{ width: `${Number(getAgentJob(msg)?.progress || 0)}%` }"></div>
              </div>
            </div>
            <div class="hidden md:flex absolute top-1/2 -translate-y-1/2 right-0 items-center gap-1">
              <button @click="prepareReply(msg)"
                      class="p-1.5 rounded-lg border"
                      :class="themeStore.isDark ? 'bg-[#122034]/90 border-[#2d4668] hover:bg-[#17304a]' : 'bg-surface-container/80 border-outline-variant/20 hover:bg-primary/10'">
                <span class="material-symbols-outlined text-sm text-slate-500">reply</span>
              </button>
              <button v-if="msg.from_userid!=chatStore.selectedUserId" @click="deleteMessage(msg)"
                      class="p-1.5 rounded-lg border"
                      :class="themeStore.isDark ? 'bg-[#122034]/90 border-[#2d4668] hover:bg-rose-500/12' : 'bg-surface-container/80 border-outline-variant/20 hover:bg-rose-50'">
                <span class="material-symbols-outlined text-sm"
                      :class="themeStore.isDark ? 'text-slate-400 hover:text-rose-300' : 'text-slate-500 hover:text-rose-500'">delete</span>
              </button>
            </div>
            <div class="md:hidden mt-2 flex justify-end">
              <button @click="openMobileActionMenu(msg)" class="inline-flex items-center gap-1.5 px-2.5 py-1.5 rounded-lg bg-surface-container text-slate-500 text-[11px] font-bold border border-outline-variant/20">
                <span class="material-symbols-outlined text-sm">more_horiz</span>
                操作
              </button>
            </div>
          </div>
          </div>
        </div>

        <div v-if="mobileActionMsg" class="md:hidden absolute inset-0 z-40 bg-black/35" @click="closeMobileActionMenu"></div>
        <div v-if="mobileActionMsg" class="md:hidden absolute left-4 right-4 bottom-6 z-50 rounded-2xl border border-outline-variant/30 bg-surface-container-highest shadow-2xl overflow-hidden">
          <button @click="handleMobileReply" class="w-full flex items-center gap-3 px-4 py-4 text-sm font-bold text-left hover:bg-primary/10">
            <span class="material-symbols-outlined text-primary">reply</span>
            引用回复
          </button>
          <button v-if="mobileActionMsg.from_userid!=chatStore.selectedUserId" @click="handleMobileDelete" class="w-full flex items-center gap-3 px-4 py-4 text-sm font-bold text-left text-rose-500 hover:bg-rose-50/70 border-t border-outline-variant/20">
            <span class="material-symbols-outlined">delete</span>
            删除消息
          </button>
          <button @click="closeMobileActionMenu" class="w-full px-4 py-3 text-xs font-bold text-slate-500 border-t border-outline-variant/20 hover:bg-surface-container">
            取消
          </button>
        </div>
        <button v-show="!isNearBottom" @click="scrollToBottom" class="absolute bottom-28 md:bottom-32 right-3 md:right-8 w-12 h-12 md:w-14 md:h-14 bg-primary text-on-primary rounded-full shadow-2xl flex items-center justify-center hover:scale-110 active:scale-95 transition-all z-20 group">
          <span class="material-symbols-outlined text-3xl">keyboard_double_arrow_down</span>
          <span v-if="newMsgCount>0" class="absolute -top-1 -right-1 min-w-[24px] h-6 px-1.5 bg-error text-on-error text-[10px] font-black rounded-full flex items-center justify-center border-2 border-white">{{ newMsgCount }}</span>
        </button>
        <div class="px-3 sm:px-4 py-3 sm:py-4 md:py-6 bg-surface-container-lowest border-t border-outline-variant/10 z-10">
          <!-- ★ AI 正在输入/输出 状态指示器 -->
          <div v-if="isSelectedAiAssistant && aiTyping" class="mb-3 sm:mb-4 flex flex-wrap items-center gap-2 px-3 sm:px-4 py-2.5 sm:py-3 rounded-2xl bg-primary/5 border border-primary/20 overflow-hidden transition-all duration-300">
            <div class="flex items-center gap-1">
              <span class="w-2 h-2 rounded-full bg-primary animate-bounce [animation-delay:0ms]"></span>
              <span class="w-2 h-2 rounded-full bg-primary animate-bounce [animation-delay:150ms]"></span>
              <span class="w-2 h-2 rounded-full bg-primary animate-bounce [animation-delay:300ms]"></span>
            </div>
            <span class="text-sm font-medium text-primary">{{ aiTypingText ? '正在输出...' : 'AI 正在思考中...' }}</span>
            <!-- 预览文本（截断显示）-->
            <span v-if="aiTypingText" class="text-xs text-slate-400 truncate max-w-[42vw] sm:max-w-[200px] ml-auto">{{ aiTypingText.slice(0, 60) }}{{ aiTypingText.length>60?'...':'' }}</span>
          </div>

          <div v-if="replyingTo" class="mb-4 p-4 bg-primary/5 border-l-4 border-primary rounded-2xl flex items-center justify-between">
            <div class="flex-1 min-w-0"><div class="text-[10px] font-black text-primary uppercase tracking-widest mb-1">正在回复</div><div class="text-xs text-on-surface-variant truncate font-medium">{{ replyingTo.content||'...' }}</div></div>
            <button @click="replyingTo=null" class="w-8 h-8 flex items-center justify-center hover:text-error rounded-full"><span class="material-symbols-outlined text-lg">close</span></button>
          </div>
          <div v-if="selectedImage" class="mb-4 inline-block group">
            <div class="relative inline-block rounded-2xl overflow-hidden shadow-xl border-4 border-primary/20">
              <img :src="selectedImagePreview" class="h-32 object-cover"/>
              <div class="absolute inset-0 bg-black/40 opacity-0 group-hover:opacity-100 transition-opacity flex items-center justify-center">
                <button @click="clearImage" class="w-10 h-10 bg-error text-on-error rounded-full flex items-center justify-center"><span class="material-symbols-outlined">delete</span></button>
              </div>
            </div>
          </div>
          <div class="flex items-end gap-2 sm:gap-3 md:gap-4">
            <div class="flex-1 relative bg-surface-container rounded-3xl p-1 transition-all focus-within:ring-4 focus-within:ring-primary/10">
              <textarea v-model="inputText" @paste="handleInputPaste" @keydown.enter.exact.prevent="sendMessage" placeholder="输入回复内容..." rows="1" class="w-full bg-transparent border-none focus:ring-0 text-sm py-3 px-4 min-h-[50px] max-h-[150px] no-scrollbar resize-none font-medium"></textarea>
              <div class="absolute right-3 bottom-3">
                <button @click="imageInputRef?.click()" class="w-10 h-10 flex items-center justify-center text-primary hover:bg-primary/10 rounded-full"><span class="material-symbols-outlined">image</span></button>
                <input type="file" ref="imageInputRef" @change="handleImageSelect" accept="image/*" class="hidden"/>
              </div>
            </div>
            <button @click="sendMessage" :disabled="!inputText.trim()&&!selectedImage" class="w-12 h-12 sm:w-14 sm:h-14 bg-primary text-on-primary rounded-xl sm:rounded-2xl flex items-center justify-center shadow-lg shadow-primary/30 hover:scale-105 active:scale-95 transition-all disabled:opacity-50">
              <span class="material-symbols-outlined">send</span>
            </button>
          </div>
          <div class="mt-2 px-1 text-[10px] text-slate-400">Enter 发送，Shift+Enter 换行</div>
        </div>
      </div>
    </div>
  </div>
</template>
<script setup lang="ts">
import { ref, reactive, computed, onMounted, onUnmounted, watch, nextTick } from 'vue'
import { useRouter } from 'vue-router'
import { useChatStore } from '@/stores/chat'
import { useThemeStore } from '@/stores/theme'
import { openExternalLink } from '@/lib/telegram'
import { apiJson, apiFormData } from '@/utils/api'
import { isLikelyTgUsername, openTgUserProfile, tgUserProfileHref, tgUserLinkLabel } from '@/utils/tgUserLink'

const chatStore = useChatStore()
const themeStore = useThemeStore()
const router = useRouter()
const AI_ASSISTANT_USER_ID = -99999
const LEGACY_AI_ASSISTANT_USER_IDS = new Set([900000001])

function isAiAssistantUserId(userId: unknown): boolean {
  return Number(userId) === AI_ASSISTANT_USER_ID
}

function isVirtualAssistantUserId(userId: unknown): boolean {
  const normalized = Number(userId)
  return normalized === AI_ASSISTANT_USER_ID || LEGACY_AI_ASSISTANT_USER_IDS.has(normalized)
}

// ★ AI 正在输入/输出状态（独立于消息状态，用于全局UI提示）
const aiTyping = ref(false)
const aiTypingText = ref('') // 缓存AI回复文本，用于"正在输出中"的实时预览

const mobilePanelView = ref<'list'|'detail'>('list')
const searchQuery = ref('')
const inputText = ref('')
const replyingTo = ref<any|null>(null)
const selectedImage = ref<File|null>(null)
const selectedImagePreview = ref('')
const pendingMediaPreviewByMsgId = reactive<Record<number, string>>({})
const chatHistoryRef = ref<HTMLElement|null>(null)
const imageInputRef = ref<HTMLInputElement|null>(null)
const isNearBottom = ref(true)
const newMsgCount = ref(0)
const enableTranslation = ref(true)
const longPressTimer = ref<ReturnType<typeof setTimeout> | null>(null)
const mobileActionMsg = ref<any | null>(null)
let pollTimer: ReturnType<typeof setInterval>|null = null
const agentActionBusy = reactive<Record<string, boolean>>({})
const agentJobPollers = new Map<string, ReturnType<typeof setInterval>>()

const avatarUrl = computed(() => {
  const aid = chatStore.getSelectedUserAvatar()
  if (!aid) return ''
  if (!chatStore.fileUrls[aid]) chatStore.autoLoadFile(aid)
  return resolveMediaUrl(chatStore.fileUrls[aid] || '')
})

const normalizedUsers = computed(() => {
  const usersById = new Map<number, any>()
  for (const rawUser of chatStore.users || []) {
    const userId = Number(rawUser.from_userid ?? rawUser.user_id ?? 0)
    if (!userId || isVirtualAssistantUserId(userId) || usersById.has(userId)) continue
    usersById.set(userId, {
      ...rawUser,
      from_userid: userId,
      from_username: rawUser.from_username ?? rawUser.username ?? ''
    })
  }
  return [...usersById.values()]
})

// Agent Assistant contact - fixed entry
const agentAssistantContact = {
  from_userid: AI_ASSISTANT_USER_ID,
  from_username: 'Agent助手',
  username: 'agent_assistant',
  is_ai_assistant: true,
  avatar_file_id: null,
  unread_count: 0,
  last_msg_time: Date.now(),
  is_system_contact: true
};

const filteredUsers = computed(() => {
  const q = searchQuery.value.trim().toLowerCase()
  if (!q) return [agentAssistantContact, ...normalizedUsers.value]
  return normalizedUsers.value.filter((u: any) => {
    const uid = String(u.from_userid || '')
    const uname = String(u.from_username || '').toLowerCase()
    return uid.includes(q) || uname.includes(q)
  })
})

const selectedUserRecord = computed(() =>
  isAiAssistantUserId(chatStore.selectedUserId)
    ? agentAssistantContact
    : normalizedUsers.value.find((u: any) =>
      Number(u.from_userid || u.user_id) === Number(chatStore.selectedUserId)
    ) || null
)

const isSelectedAiAssistant = computed(() =>
  isAiAssistantUserId(chatStore.selectedUserId) || Boolean(selectedUserRecord.value?.is_ai_assistant)
)
const chatBackgroundLayerStyle = computed(() => {
  const bg = themeStore.settings.chatBackground
  if (!bg) return null

  const overlay = themeStore.isDark ? 'rgba(4, 14, 29, 0.52)' : 'rgba(245, 248, 252, 0.56)'
  return {
    backgroundImage: `linear-gradient(${overlay}, ${overlay}), url(${bg})`,
    backgroundSize: 'cover',
    backgroundPosition: 'center',
    backgroundRepeat: 'no-repeat',
  }
})

function getUserProfileUsernameHint(user: {
  username?: string | null
  from_username?: string | null
  first_name?: string | null
  is_ai_assistant?: boolean
} | null | undefined) {
  if (!user || user.is_ai_assistant) return ''
  const username = (user.username ?? user.from_username ?? '').trim()
  if (isLikelyTgUsername(username)) return username
  return ''
}

function getUserDisplayName(user: {
  first_name?: string | null
  username?: string | null
  from_username?: string | null
  from_userid?: number | null
  user_id?: number | null
} | null | undefined, fallbackUserId?: number | null): string {
  const firstName = String(user?.first_name ?? '').trim()
  if (firstName) return firstName

  const username = String(user?.from_username ?? user?.username ?? '').trim()
  if (username) return username

  const userId = Number(user?.from_userid ?? user?.user_id ?? fallbackUserId ?? 0)
  return userId ? `用户 ${userId}` : '未知用户'
}

function getUserIdentityLabel(user: {
  username?: string | null
  from_username?: string | null
  from_userid?: number | null
  user_id?: number | null
} | null | undefined, fallbackUserId?: number | null): string {
  const username = getUserProfileUsernameHint(user)
  if (username) return `@${username.replace(/^@+/, '')}`

  const userId = Number(user?.from_userid ?? user?.user_id ?? fallbackUserId ?? 0)
  return userId ? `UID: ${userId}` : 'UID: 未知'
}

function resolveMediaUrl(url: string | null | undefined) {
  const raw = (url || '').trim()
  if (!raw) return ''
  if (raw.startsWith('/api/file/') && !raw.includes('/content')) {
    return `${raw}/content`
  }
  if (raw.startsWith(window.location.origin)) return raw
  if (raw.startsWith('https://api.telegram.org/file/')) return raw
  return raw
}

function handleAvatarLoadError(fileId?: string | null) {
  if (!fileId) return
  chatStore.fileErrors[fileId] = true
  delete chatStore.fileUrls[fileId]
}

function handleSelectedAvatarError() {
  const avatarFileId = chatStore.getSelectedUserAvatar()
  if (avatarFileId) {
    handleAvatarLoadError(avatarFileId)
    // ★ 3秒后自动尝试重新加载
    setTimeout(() => { if (chatStore.fileErrors[avatarFileId]) { delete chatStore.fileErrors[avatarFileId]; chatStore.autoLoadFile(avatarFileId) } }, 3000)
  }
}

function handleSelectedAvatarClick() {
  if (isSelectedAiAssistant.value) {
    router.push({ path: '/settings', query: { tab: 'ai', mode: 'ai' } })
    return
  }
  refreshSelectedAvatar()
}

/** ★ 手动强制刷新头像 */
function refreshSelectedAvatar() {
  const avatarFileId = chatStore.getSelectedUserAvatar()
  if (avatarFileId) {
    chatStore.forceReloadFile(avatarFileId)
  }
}

function userAvatarSrc(user: { avatar_file_id?: string | null }) {
  return resolveMediaUrl(user.avatar_file_id ? chatStore.fileUrls[user.avatar_file_id] : '')
}

// 认证头辅助函数（统一支持 sessionStorage adm_s / localStorage / Cookie）
function getAdminToken() {
  const sessionRaw = sessionStorage.getItem('adm_s')
  if (sessionRaw) {
    try {
      return decodeURIComponent(atob(sessionRaw))
    } catch {
      return ''
    }
  }

  const localToken = localStorage.getItem('admin_token')
  if (localToken) return localToken

  const cookie = document.cookie
    .split('; ')
    .find((item) => item.startsWith('admin_token='))
  if (!cookie) return ''

  try {
    return decodeURIComponent(cookie.split('=').slice(1).join('='))
  } catch {
    return ''
  }
}

function authH() {
  const token = getAdminToken()
  const headers: Record<string, string> = {
    'Content-Type': 'application/json'
  }
  if (token) {
    headers['Authorization'] = `Bearer ${token}`
  }
  return headers
}

watch(() => chatStore.users, (list) => {
  for (const u of list || []) {
    if (u.avatar_file_id) chatStore.autoLoadFile(u.avatar_file_id)
  }
}, { immediate: true, deep: true })

onMounted(() => { fetchUsers(); loadConfig(); pollTimer = setInterval(pollLatestMessages, 15000) })
onUnmounted(() => {
  if (pollTimer) clearInterval(pollTimer)
  for (const timer of agentJobPollers.values()) clearInterval(timer)
  agentJobPollers.clear()
  clearLongPressTimer()
})

async function fetchUsers() { await chatStore.fetchUsers() }

// Polling: fetch page 1 and merge any new messages at the end
async function pollLatestMessages() {
  if (!chatStore.selectedUserId) return
  if (isSelectedAiAssistant.value) return
  try {
    const res = await fetch(`/api/messages/${chatStore.selectedUserId}?page=1`, {
      headers: authH(),
      keepalive: true
    })
    const d = await res.json()
    if (d.status !== 'ok') return
    const fresh: any[] = d.data || []
    if (fresh.length === 0) return

    const { appendedCount, updatedRows } = upsertMessagesFromLatest(fresh)
    if (appendedCount === 0 && updatedRows.length === 0) return

    triggerAutoLoadForMessages([...updatedRows, ...fresh])

    if (!isNearBottom.value && appendedCount > 0) {
      newMsgCount.value += appendedCount
    } else {
      await nextTick()
      scrollToBottom()
    }
  } catch {}
}

function upsertMessagesFromLatest(fresh: any[]) {
  const existingById = new Map(chatStore.messages.map((m: any) => [m.msg_id, m]))
  const updatedRows: any[] = []

  for (const incoming of fresh) {
    const existing = existingById.get(incoming.msg_id)
    if (!existing) continue

    const shouldPatch = (
      (!existing.file_id && incoming.file_id) ||
      (!existing.media_url && incoming.media_url) ||
      (existing.status !== incoming.status) ||
      (!existing.tg_msg_id && incoming.tg_msg_id)
    )

    if (shouldPatch) {
      const merged = { ...existing, ...incoming }
      existingById.set(incoming.msg_id, merged)
      updatedRows.push(merged)
    }
  }

  const newMsgs = fresh.filter((m: any) => !existingById.has(m.msg_id))

  if (updatedRows.length > 0 || newMsgs.length > 0) {
    const patchedCurrent = chatStore.messages.map((m: any) => existingById.get(m.msg_id) || m)
    chatStore.messages = [...patchedCurrent, ...newMsgs]
    cleanupPendingMediaPreviews(chatStore.messages)
  }

  return { appendedCount: newMsgs.length, updatedRows }
}


async function selectUser(u: any) {
  // Handle Agent Assistant
  if (u.is_ai_assistant || isAiAssistantUserId(u.from_userid)) {
    chatStore.selectedUserId = AI_ASSISTANT_USER_ID
    chatStore.selectedUserName = 'Agent 助手'
    chatStore.messages = []
    msgPage.value = 1
    hasMoreMessages.value = true
    newMsgCount.value = 0
    await fetchMessages()
    return
  }
  chatStore.selectedUserId = u.from_userid
  chatStore.selectedUserName = u.from_username || u.username || 'ID:' + u.from_userid
  chatStore.messages = []
  msgPage.value = 1
  hasMoreMessages.value = true
  await fetchMessages()
}

// Backend uses page param, limit is fixed at 50 server-side.
// We treat each page response < 50 items as "no more".
const BACKEND_PAGE_SIZE = 50
const msgPage = ref(1)
const hasMoreMessages = ref(true)
const isLoadingMore = ref(false)

async function fetchMessages() {
  if (!chatStore.selectedUserId) return
  // Always fetch page 1 (latest messages), reset state
  await chatStore.fetchMessages(chatStore.selectedUserId, 1)
  cleanupPendingMediaPreviews(chatStore.messages)
  triggerAutoLoadForMessages(chatStore.messages)
  for (const msg of chatStore.messages) {
    const job = getAgentJob(msg)
    if (isAgentJobRunning(job)) startAgentJobPolling(String(job.job_id), msg)
  }
  msgPage.value = 1
  hasMoreMessages.value = (chatStore.messages.length >= BACKEND_PAGE_SIZE)
  await nextTick()
  if (isNearBottom.value) scrollToBottom()
}

async function loadMoreMessages() {
  if (!chatStore.selectedUserId || isLoadingMore.value || !hasMoreMessages.value) return
  isLoadingMore.value = true
  const el = chatHistoryRef.value
  const prevScrollHeight = el?.scrollHeight || 0
  try {
    const nextPage = msgPage.value + 1
    const older = await chatStore.fetchMoreMessages(chatStore.selectedUserId, nextPage)
    if (older.length < BACKEND_PAGE_SIZE) hasMoreMessages.value = false
    if (older.length > 0) {
      chatStore.messages = [...older, ...chatStore.messages]
      triggerAutoLoadForMessages(older)
      msgPage.value = nextPage
      await nextTick()
      // 保持滚动位置不跳动
      if (el) el.scrollTop = el.scrollHeight - prevScrollHeight
    } else {
      hasMoreMessages.value = false
    }
  } finally {
    isLoadingMore.value = false
  }
}

function triggerAutoLoadForMessages(messageList: any[]) {
  for (const m of messageList || []) {
    if (m.file_id) chatStore.autoLoadFile(m.file_id)
    if (m.quoted_file_id) chatStore.autoLoadFile(m.quoted_file_id)
  }
}

function cleanupPendingMediaPreviews(messageList: any[]) {
  for (const m of messageList || []) {
    const mid = Number(m?.msg_id || 0)
    if (!mid) continue
    if (m?.file_id || m?.media_url || m?.status === 'failed') {
      if (pendingMediaPreviewByMsgId[mid]) delete pendingMediaPreviewByMsgId[mid]
    }
  }
}

function scrollToBottom() {
  if (chatHistoryRef.value) chatHistoryRef.value.scrollTop = chatHistoryRef.value.scrollHeight
}

function handleScroll() {
  if (!chatHistoryRef.value) return
  const el = chatHistoryRef.value
  isNearBottom.value = el.scrollHeight - el.scrollTop - el.clientHeight < 80
  if (isNearBottom.value) newMsgCount.value = 0
  // 触顶加载更多
  if (el.scrollTop < 60) loadMoreMessages()
}

function scrollToMessage(id: number) {
  const el = document.getElementById('msg-' + id)
  if (el) { el.scrollIntoView({ behavior: 'smooth', block: 'center' }); el.classList.add('highlight-anim'); setTimeout(() => el.classList.remove('highlight-anim'), 1500) }
}

function prepareReply(msg: any) { replyingTo.value = msg }

function openMobileActionMenu(msg: any) {
  mobileActionMsg.value = msg
}

function closeMobileActionMenu() {
  mobileActionMsg.value = null
}

function handleMobileReply() {
  if (!mobileActionMsg.value) return
  prepareReply(mobileActionMsg.value)
  closeMobileActionMenu()
}

async function handleMobileDelete() {
  if (!mobileActionMsg.value) return
  await deleteMessage(mobileActionMsg.value)
  closeMobileActionMenu()
}

function onMsgContextMenu(event: MouseEvent, msg: any) {
  if (window.matchMedia('(max-width: 767px)').matches) {
    event.preventDefault()
    openMobileActionMenu(msg)
  }
}

function clearLongPressTimer() {
  if (longPressTimer.value) {
    clearTimeout(longPressTimer.value)
    longPressTimer.value = null
  }
}

function onMsgTouchStart(msg: any) {
  clearLongPressTimer()
  longPressTimer.value = setTimeout(() => {
    if (typeof navigator !== 'undefined' && 'vibrate' in navigator) {
      navigator.vibrate(10)
    }
    openMobileActionMenu(msg)
    clearLongPressTimer()
  }, 380)
}

function onMsgTouchMove() {
  clearLongPressTimer()
}

function onMsgTouchEnd() {
  clearLongPressTimer()
}

async function deleteMessage(msg: any) {
  if (!chatStore.selectedUserId || !msg.tg_msg_id) return
  if (isSelectedAiAssistant.value) return
  if (!confirm('确定删除这条消息？')) return
  await fetch(`/api/messages/${chatStore.selectedUserId}/delete`, {
    method: 'POST', headers: authH(), keepalive: true,
    body: JSON.stringify({ tg_msg_id: msg.tg_msg_id })
  })
  chatStore.messages = chatStore.messages.filter((m: any) => m.tg_msg_id !== msg.tg_msg_id)
}
function clearImage() { selectedImage.value = null; selectedImagePreview.value = '' }

function handleImageSelect(e: Event) {
  const f = (e.target as HTMLInputElement).files?.[0]
  if (!f) return
  selectedImage.value = f
  selectedImagePreview.value = URL.createObjectURL(f)
}

function handleInputPaste(e: ClipboardEvent) {
  const items = e.clipboardData?.items
  if (!items) return
  for (const item of items) {
    if (item.type.startsWith('image/')) {
      const file = item.getAsFile()
      if (file) {
        e.preventDefault()
        selectedImage.value = file
        selectedImagePreview.value = URL.createObjectURL(file)
        return
      }
    }
  }
}

function getMsgType(msg: any) {
  return msg.msg_type || msg.type || 'text'
}

function hasMediaSource(msg: any) {
  if (msg?.file_id || msg?.media_url) return true
  const mid = Number(msg?.msg_id || 0)
  return Boolean(mid && pendingMediaPreviewByMsgId[mid])
}

function getMediaUrl(msg: any) {
  if (msg?.file_id && chatStore.fileUrls[msg.file_id]) return resolveMediaUrl(chatStore.fileUrls[msg.file_id])
  if (msg?.media_url) return resolveMediaUrl(msg.media_url)
  const mid = Number(msg?.msg_id || 0)
  if (mid && pendingMediaPreviewByMsgId[mid]) return pendingMediaPreviewByMsgId[mid]
  return ''
}

function getQuotedMediaUrl(msg: any) {
  if (msg?.quoted_file_id && chatStore.fileUrls[msg.quoted_file_id]) return resolveMediaUrl(chatStore.fileUrls[msg.quoted_file_id])
  if (msg?.quoted_media_url) return resolveMediaUrl(msg.quoted_media_url)
  return ''
}

function getMsgTypeLabel(msg: any) {
  const t = getMsgType(msg)
  if (t === 'photo') return '图片'
  if (t === 'animation') return '动图'
  if (t === 'video') return '视频'
  if (t === 'sticker') return '贴纸'
  if (t === 'voice') return '语音'
  if (t === 'audio') return '音频'
  return '媒体'
}

const MEDIA_PLACEHOLDER_SET = new Set([
  '[图片]',
  '[photo]',
  '[媒体]',
  '[文件]',
])

function normalizeImageUrlCandidate(raw: string): string {
  let url = String(raw || '').trim().replace(/^['"]|['"]$/g, '')
  url = url.replace(/[.,;!?]+$/g, '')
  const leftParenCount = (url.match(/\(/g)?.length || 0)
  const rightParenCount = (url.match(/\)/g)?.length || 0)
  if (url.endsWith(')') && leftParenCount < rightParenCount) {
    url = url.slice(0, -1)
  }
  return url
}

function looksLikeImageUrl(url: string): boolean {
  const u = String(url || '').trim()
  if (!/^https?:\/\//i.test(u)) return false
  const lower = u.toLowerCase()
  if (/\.(png|jpe?g|gif|webp|bmp|svg|avif|heic|heif)(?:$|[?#])/.test(lower)) return true
  if (
    lower.includes('image.pollinations.ai') ||
    lower.includes('oaidalleapiprod') ||
    lower.includes('images.openai.com') ||
    lower.includes('replicate.delivery') ||
    lower.includes('/images/') ||
    lower.includes('/image/')
  ) {
    return true
  }
  return false
}

function extractImageUrlsFromText(text: string): string[] {
  const raw = String(text || '')
  if (!raw.trim()) return []
  const out: string[] = []
  const seen = new Set<string>()

  const mdMatches = raw.matchAll(/!\[[^\]]*]\((https?:\/\/[^)\s]+)\)/gi)
  for (const m of mdMatches) {
    const url = normalizeImageUrlCandidate(m[1] || '')
    if (url && !seen.has(url) && looksLikeImageUrl(url)) {
      seen.add(url)
      out.push(url)
    }
  }

  const rawMatches = raw.matchAll(/https?:\/\/[^\s<>"\]\)]+/gi)
  for (const m of rawMatches) {
    const url = normalizeImageUrlCandidate(m[0] || '')
    if (url && !seen.has(url) && looksLikeImageUrl(url)) {
      seen.add(url)
      out.push(url)
    }
  }
  return out
}

function removeImageUrlsFromText(text: string, imageUrls: string[]): string {
  let out = String(text || '')
  out = out.replace(/!\[[^\]]*]\((https?:\/\/[^)\s]+)\)/gi, ' ')
  for (const url of imageUrls || []) {
    if (url) out = out.split(url).join(' ')
  }
  return out.replace(/[ \t]{2,}/g, ' ').replace(/\n{3,}/g, '\n\n').trim()
}

function shouldShowPlainContent(msg: any) {
  if (msg?.original_text) return false
  if (isStickerPlaceholder(msg)) return false

  const raw = (msg?.content ?? '').toString().trim()
  if (!raw) return false

  const t = getMsgType(msg)
  if (t !== 'text' && MEDIA_PLACEHOLDER_SET.has(raw.toLowerCase())) {
    return false
  }
  return true
}

function getMsgStatus(msg: any) {
  if (msg.status) return msg.status
  // 旧数据兼容：管理员发出的消息没有状态字段时默认按已送达展示
  return msg.from_userid !== chatStore.selectedUserId ? 'sent' : 'received'
}

function getAgentUiPayload(msg: any): any {
  if (msg?.ui_actions || msg?.job) return { ui_actions: msg.ui_actions || [], job: msg.job || null }
  const raw = msg?.agent_ui_json
  if (!raw) return { ui_actions: [], job: null }
  if (typeof raw === 'object') return raw
  try { return JSON.parse(String(raw)) } catch { return { ui_actions: [], job: null } }
}

function getAgentUiActions(msg: any): any[] {
  const actions = getAgentUiPayload(msg)?.ui_actions
  return Array.isArray(actions) ? actions : []
}

function getAgentJob(msg: any): any {
  return getAgentUiPayload(msg)?.job || null
}

function isAgentJobRunning(job: any): boolean {
  return Boolean(job && ['prepared', 'queued', 'scanning', 'classifying'].includes(String(job.status || '')))
}

function agentJobStatusText(job: any): string {
  const labels: Record<string, string> = {
    prepared: '等待确认', queued: '等待扫描', scanning: '正在聚类历史消息', classifying: '正在判断未知文本簇'
  }
  return labels[String(job?.status || '')] || '处理中'
}

function actionBusyKey(action: any): string {
  return `${String(action?.id || '')}:${JSON.stringify(action?.payload || {})}`
}

function appendAgentApiResponse(res: any): any {
  const content = String(res?.reply ?? res?.data?.reply ?? '')
  const msg: any = {
    msg_id: Date.now() + Math.floor(Math.random() * 1000),
    from_userid: AI_ASSISTANT_USER_ID,
    to_userid: 0,
    content,
    timestamp: new Date().toISOString(),
    status: 'received',
    type: 'text',
    msg_type: 'text',
    ui_actions: Array.isArray(res?.ui_actions) ? res.ui_actions : [],
    job: res?.job || null,
  }
  chatStore.messages = [...chatStore.messages, msg]
  if (isAgentJobRunning(msg.job)) startAgentJobPolling(String(msg.job.job_id), msg)
  nextTick(() => scrollToBottom())
  return msg
}

async function handleAgentUiAction(action: any, silent = false) {
  const key = actionBusyKey(action)
  if (agentActionBusy[key]) return
  agentActionBusy[key] = true
  try {
    const res = await apiJson('/api/ai/chat', 'POST', {
      userId: AI_ASSISTANT_USER_ID,
      content: silent ? '查看扫描结果' : String(action?.label || '执行操作'),
      agent_action: { id: action?.id, payload: action?.payload || {} },
    }, { isAiCall: true })
    if (res?.status !== 'ok') throw new Error(res?.message || 'Agent 操作失败')
    appendAgentApiResponse(res)
    setTimeout(() => fetchMessages(), 800)
  } catch (error: any) {
    chatStore.messages = [...chatStore.messages, {
      msg_id: Date.now(), from_userid: AI_ASSISTANT_USER_ID, to_userid: 0,
      content: `[错误] ${error?.message || 'Agent 操作失败'}`,
      timestamp: new Date().toISOString(), status: 'error', type: 'text', msg_type: 'text',
    }]
  } finally {
    agentActionBusy[key] = false
  }
}

function startAgentJobPolling(jobId: string, targetMsg: any) {
  if (!jobId || agentJobPollers.has(jobId)) return
  const timer = setInterval(async () => {
    try {
      const response = await fetch(`/api/agent/ad-scans/${encodeURIComponent(jobId)}`, { headers: authH() })
      const data = await response.json()
      if (!response.ok || data?.status !== 'ok') return
      targetMsg.job = data.job
      chatStore.messages = chatStore.messages.map((msg: any) => msg.msg_id === targetMsg.msg_id ? { ...msg, job: data.job } : msg)
      if (['completed', 'partial', 'failed', 'cancelled'].includes(String(data.job?.status || ''))) {
        clearInterval(timer)
        agentJobPollers.delete(jobId)
        if (['completed', 'partial'].includes(String(data.job?.status || ''))) {
          await handleAgentUiAction({ id: 'get_ad_scan_result', payload: { job_id: jobId }, label: '查看扫描结果' }, true)
        }
      }
    } catch {}
  }, 2000)
  agentJobPollers.set(jobId, timer)
}

async function sendMessage() {
  if (!chatStore.selectedUserId || (!inputText.value.trim() && !selectedImage.value)) return
  const sentContent = inputText.value.trim()
  const imageToSend = selectedImage.value // 保存引用，防止后续清空导致丢失
  const replyTarget = replyingTo.value // 保存引用
  const optimisticPreviewUrl = imageToSend ? URL.createObjectURL(imageToSend) : ''

  // Optimistic: immediately show outgoing message
  const optimisticMsg: any = {
    msg_id: Date.now(), // temp id
    from_userid: 0, // 0 = admin/outgoing
    content: sentContent || '',
    timestamp: new Date().toISOString(),
    status: 'sending',
    type: imageToSend ? 'photo' : 'text',
    msg_type: imageToSend ? 'photo' : 'text',
    media_url: optimisticPreviewUrl || null,
    quoted_text: replyTarget?.content || null,
  }
  chatStore.messages = [...chatStore.messages, optimisticMsg]
  inputText.value = ''
  selectedImage.value = null
  selectedImagePreview.value = ''
  replyingTo.value = null
  await nextTick()
  scrollToBottom()

  // 立即发送请求，不阻塞 UI
  sendMessageAsync(
    chatStore.selectedUserId,
    sentContent,
    imageToSend,
    replyTarget,
    optimisticMsg,
    isSelectedAiAssistant.value,
    optimisticPreviewUrl
  )
}

async function sendMessageAsync(
  userId: number,
  content: string,
  image: File | null,
  replyTarget: any,
  optimisticMsg: any,
  isAiAssistant: boolean,
  optimisticPreviewUrl: string
) {
  try {
    let ok = false
    console.log('[发送消息] 开始发送:', { userId, hasImage: !!image, content, isAiAssistant })

    if (isAiAssistant) {
      if (image) {
        chatStore.messages = chatStore.messages.map((m: any) =>
          m.msg_id === optimisticMsg.msg_id ? { ...m, status: 'failed' } : m
        )
        alert('AI 助手会话暂不支持发送图片，请切换到真实用户会话发送')
        return
      }

      // 显示"正在思考..."状态
      chatStore.messages = chatStore.messages.map((m: any) =>
        m.msg_id === optimisticMsg.msg_id ? { ...m, status: 'thinking' } : m
      )

      // ★ 显示全局"AI 正在输入..."状态
      aiTyping.value = true
      aiTypingText.value = ''
      const aiHeartbeatMsgId = Date.now() + 3
      const heartbeatTexts = [
        '正在分析指令...',
        '正在调用工具读取系统数据...',
        '正在检查群/频道权限与消息证据...',
        '正在等待模型整理结果...',
      ]
      let heartbeatIndex = 0
      chatStore.messages = [...chatStore.messages, {
        msg_id: aiHeartbeatMsgId,
        from_userid: AI_ASSISTANT_USER_ID,
        to_userid: 0,
        content: heartbeatTexts[0],
        timestamp: new Date().toISOString(),
        status: 'thinking',
        type: 'text',
        msg_type: 'text',
      }]
      const heartbeatTimer = setInterval(() => {
        heartbeatIndex = Math.min(heartbeatIndex + 1, heartbeatTexts.length - 1)
        const nextText = heartbeatTexts[heartbeatIndex]
        aiTypingText.value = nextText
        chatStore.messages = chatStore.messages.map((m: any) =>
          m.msg_id === aiHeartbeatMsgId ? { ...m, content: nextText, timestamp: new Date().toISOString() } : m
        )
      }, 1800)

      try {
        // AI 请求统一走独立路由，避免与 /api/messages/{user_id} 发生匹配歧义
        const res = await apiJson('/api/ai/chat', 'POST', {
          userId,
          content,
          // model: 由后端根据配置自动选择，不传则用默认值
        }, { isAiCall: true })
        console.log('[发送消息] AI 响应:', res)
        ok = res.status === 'ok'
        const aiReplyRaw = String((res as any).reply ?? (res as any).data?.reply ?? '')
        const aiReplyTextFromServer = String((res as any).reply_text ?? '')
        const aiMsgTypeFromServer = String((res as any).reply_msg_type ?? '').toLowerCase()
        const aiMediaUrlFromServer = String((res as any).media_url ?? '')

        if (!ok) {
          // ★ 区分后端返回的错误类型，显示准确提示
          const errMsg = (res as any).message || (res as any)?.error?.message || 'AI 助手回复失败'
          console.error('[发送消息] AI 返回错误:', errMsg)
          // ★ LLM 未配置 / 配置缺失时，弹窗引导去设置页
          const rawReply = String((res as any).reply ?? '')
          const llmUnconfigured =
            errMsg.includes('LLM') || errMsg.includes('llm') || rawReply.includes('[LLM 错误]')
          if (llmUnconfigured) {
            const goSettings = window.confirm('AI 助手需要使用大模型 API。当前 LLM 配置不可用，是否前往「设置 - AI 设置」完成配置？')
            if (goSettings && (window as any).router) {
              ;(window as any).router.push('/settings?focus=ai')
            } else if (goSettings) {
              window.location.hash = '#/settings?focus=ai'
            }
          }
          // 不再用alert()，改用状态提示
          chatStore.messages = [...chatStore.messages, {
            msg_id: Date.now() + 2,
            from_userid: userId, to_userid: 0,
            content: `[错误] ${errMsg}`,
            timestamp: new Date().toISOString(), status: 'error',
            type: 'text', msg_type: 'text'
          }]
        }

        if (ok && (aiReplyRaw || aiMediaUrlFromServer)) {
          let aiMsgType: 'text' | 'photo' = aiMsgTypeFromServer === 'photo' ? 'photo' : 'text'
          let aiMediaUrl = aiMediaUrlFromServer
          let aiReplyText = aiReplyTextFromServer || aiReplyRaw

          // 兼容旧后端：如果没返回结构化字段，则从文本里提取图片链接
          if (!aiMediaUrl && aiReplyRaw) {
            const imageUrls = extractImageUrlsFromText(aiReplyRaw)
            if (imageUrls.length > 0) {
              aiMediaUrl = imageUrls[0]
              aiMsgType = 'photo'
              aiReplyText = removeImageUrlsFromText(aiReplyRaw, imageUrls) || '[图片]'
            }
          }
          if (aiMsgType === 'photo' && !aiReplyText) {
            aiReplyText = '[图片]'
          }

          // ★ AI 回复文本缓存（用于"正在输出中"预览）
          aiTypingText.value = aiReplyText
          chatStore.messages = chatStore.messages.filter((m: any) => m.msg_id !== aiHeartbeatMsgId)
          
          const aiMsg = {
            msg_id: Date.now() + 1,
            from_userid: userId,
            to_userid: 0,
            content: aiReplyText,
            media_url: aiMediaUrl || null,
            timestamp: new Date().toISOString(),
            status: 'received',
            type: aiMsgType,
            msg_type: aiMsgType,
            ui_actions: Array.isArray((res as any).ui_actions) ? (res as any).ui_actions : [],
            job: (res as any).job || null,
          }
          chatStore.messages = [...chatStore.messages, aiMsg]
          if (isAgentJobRunning((aiMsg as any).job)) {
            startAgentJobPolling(String((aiMsg as any).job.job_id), aiMsg)
          }
          await nextTick()
          scrollToBottom()

          // 更新用户消息状态为已发送
          chatStore.messages = chatStore.messages.map((m: any) =>
            m.msg_id === optimisticMsg.msg_id ? { ...m, status: 'sent' } : m
          )
        } else if (!aiReplyRaw && !aiMediaUrlFromServer && ok) {
          // 成功但无回复内容
          console.warn('[发送消息] AI 返回成功但无回复内容')
          chatStore.messages = chatStore.messages.filter((m: any) => m.msg_id !== aiHeartbeatMsgId)
          chatStore.messages = [...chatStore.messages, {
            msg_id: Date.now() + 2,
            from_userid: userId, to_userid: 0,
            content: '[AI 返回了空响应]',
            timestamp: new Date().toISOString(), status: 'received',
            type: 'text', msg_type: 'text'
          }]
          ok = false
        }
      } catch (err: any) {
        console.error('[发送消息] AI 请求异常:', err)
        chatStore.messages = chatStore.messages.filter((m: any) => m.msg_id !== aiHeartbeatMsgId)
        // 标记为失败并显示具体错误
        chatStore.messages = chatStore.messages.map((m: any) =>
          m.msg_id === optimisticMsg.msg_id ? { ...m, status: 'failed' } : m
        )
        // 显示用户友好的错误消息而非alert
        const errMsg = err.message || String(err)
        chatStore.messages = [...chatStore.messages, {
          msg_id: Date.now() + 2,
          from_userid: userId, to_userid: 0,
          content: errMsg.includes('超时') ? '⏰ AI 思考时间过长，已超时' :
                   errMsg.includes('401') || errMsg.includes('403') ? '🔐 认证失败，请重新登录' :
                   `❌ ${errMsg.slice(0, 150)}`,
          timestamp: new Date().toISOString(), status: 'error',
          type: 'text', msg_type: 'text'
        }]
      } finally {
        clearInterval(heartbeatTimer)
        // ★ 无论成败，都关闭"正在输入中"状态
        aiTyping.value = false
      }
    } else if (image) {
      const fd = new FormData()
      fd.append('image', image)
      if (content) fd.append('content', content)
      if (replyTarget?.tg_msg_id) fd.append('reply_to_msg_id', String(replyTarget.tg_msg_id))

      console.log('[发送消息] 发送图片 FormData...')
      console.log('[FormData] 内容:', {
        image: `${image.name} (${(image.size / 1024).toFixed(2)} KB, ${image.type})`,
        content: content,
        reply_to_msg_id: replyTarget?.tg_msg_id
      })
      
      try {
        const res = await apiFormData(`/api/messages/${userId}`, fd)
        console.log('[发送消息] 图片响应:', res)
        ok = res.status === 'ok'
        const returnedMsgId = Number((res as any)?.msg_id || (res as any)?.data?.msg_id || 0)
        if (ok && returnedMsgId > 0) {
          if (optimisticPreviewUrl) {
            pendingMediaPreviewByMsgId[returnedMsgId] = optimisticPreviewUrl
          }
          const oldTempId = Number(optimisticMsg.msg_id || 0)
          chatStore.messages = chatStore.messages.map((m: any) =>
            Number(m.msg_id) === oldTempId ? { ...m, msg_id: returnedMsgId, status: 'sending' } : m
          )
          if (oldTempId && oldTempId !== returnedMsgId && pendingMediaPreviewByMsgId[oldTempId]) {
            delete pendingMediaPreviewByMsgId[oldTempId]
          }
        }
      } catch (err: any) {
        console.error('[发送消息] 图片发送失败:', err)
        console.error('[错误详情]:', err.message)
        console.error('[错误堆栈]:', err.stack)
        throw err // 重新抛出以便外层捕获
      }
    } else {
      const body: any = { content, do_translate: enableTranslation.value }
      if (replyTarget?.tg_msg_id) body.reply_to_msg_id = replyTarget.tg_msg_id

      console.log('[发送消息] 发送文本 JSON:', body)
      const res = await apiJson(`/api/messages/${userId}`, 'POST', body)
      console.log('[发送消息] 文本响应:', res)
      ok = res.status === 'ok'
    }

    if (!ok) {
      console.error('[发送消息] 标记为失败')
      const failedMid = Number(optimisticMsg.msg_id || 0)
      if (failedMid && pendingMediaPreviewByMsgId[failedMid]) {
        delete pendingMediaPreviewByMsgId[failedMid]
      }
      chatStore.messages = chatStore.messages.map((m: any) =>
        m.msg_id === optimisticMsg.msg_id ? { ...m, status: 'failed' } : m
      )
      return
    }

    // 发送成功后的状态处理：
    // - AI 会话：后端已同步返回 reply，可直接标记 sent
    // - 普通会话：后端先落库(status=sending)再异步投递 Telegram，
    //            不能在这里提前标记 sent，改为等待后端状态回写并刷新
    if (isAiAssistant) {
      chatStore.messages = chatStore.messages.map((m: any) =>
        m.msg_id === optimisticMsg.msg_id ? { ...m, status: 'sent' } : m
      )
      return
    }

    // 普通会话：保持 optimistic 消息为 sending，拉取后端真实状态
    setTimeout(() => fetchMessages(), 300)
    setTimeout(() => fetchMessages(), 1800)
  } catch (err) {
    console.error('[发送消息] 异常错误:', err)
    // 标记为失败
    chatStore.messages = chatStore.messages.map((m: any) =>
      m.msg_id === optimisticMsg.msg_id ? { ...m, status: 'failed' } : m
    )
    const failedMid = Number(optimisticMsg.msg_id || 0)
    if (failedMid && pendingMediaPreviewByMsgId[failedMid]) {
      delete pendingMediaPreviewByMsgId[failedMid]
    }
    // 显示具体错误信息给用户（仅非 AI 请求，AI 请求已在内部处理错误）
    if (!isAiAssistant) {
      const errMsg = err instanceof Error ? err.message : String(err)
      alert(`发送失败: ${errMsg.slice(0, 150)}`)
    }
  }
}

async function markAsRead() {
  if (!chatStore.selectedUserId) return
  if (isSelectedAiAssistant.value) {
    newMsgCount.value = 0
    return
  }
  const uid = Number(chatStore.selectedUserId)
  const res = await fetch(`/api/mark_read/${uid}`, { method: 'POST', headers: authH(), keepalive: true })
  const data = await res.json().catch(() => ({ status: 'error' }))
  if (!res.ok || data.status !== 'ok') {
    console.error('标记已读失败:', data)
    return
  }

  // 立即更新本地 UI，避免红点残留造成“点击无效”感
  chatStore.users = chatStore.users.map((u: any) =>
    Number(u.from_userid || u.user_id) === uid ? { ...u, unread_count: 0 } : u
  )
  newMsgCount.value = 0

  // 再向后端同步拉取一次，保持最终一致性
  await fetchUsers()
}

async function deleteChat() {
  if (!chatStore.selectedUserId || !confirm('确定清空记录？')) return
  if (isSelectedAiAssistant.value) {
    chatStore.messages = []
    return
  }
  await fetch(`/api/delete_chat/${chatStore.selectedUserId}`, { method: 'POST', headers: authH(), keepalive: true })
  chatStore.messages = []
}

async function loadConfig() {
  try { const r = await fetch('/api/config', { headers: authH() }); const d = await r.json(); if (d.status==='ok') enableTranslation.value = d.data?.global_auto_translate==='true' } catch {}
}

function isVideoSticker(url: string) { return url.includes('.webm') }
function isStickerPlaceholder(msg: any) { return (msg.type==='sticker'||msg.msg_type==='sticker') && !msg.content }

function formatTime(ts: any) {
  if (!ts) return ''
  const d = new Date(typeof ts==='number' ? ts*1000 : String(ts).replace(' ','T'))
  if (isNaN(d.getTime())) return ''
  return d.toLocaleString('zh-CN',{month:'2-digit',day:'2-digit',hour:'2-digit',minute:'2-digit'})
}

function formatShortTime(ts: any) {
  if (!ts) return ''
  const d = new Date(String(ts).replace(' ','T'))
  if (isNaN(d.getTime())) return ''
  const now = new Date()
  if (d.toDateString()===now.toDateString()) return d.toLocaleTimeString('zh-CN',{hour:'2-digit',minute:'2-digit'})
  return d.toLocaleDateString('zh-CN',{month:'2-digit',day:'2-digit'})
}
</script>

<style scoped>
.chat-contact-item {
  border-bottom-color: rgba(148, 163, 184, 0.24);
}

.chat-history-background {
  background-attachment: scroll;
  background-origin: border-box;
  transform: translateZ(0);
  will-change: transform;
}

:global(html.dark) .chat-contact-item {
  border-bottom-color: rgba(93, 128, 173, 0.42);
}

.mobile-longpress-target {
  -webkit-touch-callout: none;
  -webkit-user-select: none;
  user-select: none;
}

.chat-bubble {
  max-width: 100%;
  overflow-wrap: anywhere;
}

@media (max-width: 767px) {
  .chat-contact-item {
    gap: 0.625rem;
  }

  .mobile-longpress-target,
  .mobile-longpress-target * {
    -webkit-touch-callout: none !important;
    -webkit-user-select: none !important;
    user-select: none !important;
  }

  .mobile-longpress-target {
    -webkit-tap-highlight-color: transparent;
    touch-action: pan-y;
  }
}

@media (min-width: 768px) {
  .mobile-longpress-target {
    -webkit-touch-callout: default;
    -webkit-user-select: none;
    user-select: none;
  }

  .chat-copy-text,
  .chat-copy-text * {
    -webkit-touch-callout: default;
    -webkit-user-select: text;
    user-select: text;
  }

  .chat-copy-ignore,
  .chat-copy-ignore * {
    -webkit-touch-callout: none;
    -webkit-user-select: none;
    user-select: none;
  }
}
</style>
