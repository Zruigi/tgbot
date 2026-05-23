<template>
  <div class="flex h-full overflow-hidden bg-surface">
    <!-- 左侧：目标选择摘要 -->
    <div class="border-r border-outline-variant/20 flex flex-col bg-surface-container-lowest/80"
         :class="mobilePanelView==='list'?'w-full md:w-[360px]':'hidden md:flex md:w-[360px]'">
      <div class="p-4 md:p-5 border-b border-outline-variant/10 space-y-4">
        <div class="flex items-center justify-between gap-3">
          <h2 class="text-base md:text-lg font-black flex items-center min-w-0">
            <span class="material-symbols-outlined mr-2 text-primary">campaign</span>群发目标
          </h2>
          <span class="bg-primary/10 text-primary text-[10px] font-bold px-2 py-1 rounded-full shrink-0">
            {{ totalSelected }} 已选
          </span>
        </div>

        <div class="grid grid-cols-3 gap-2">
          <button @click="openTargetDrawer('users')"
                  class="rounded-2xl px-3 py-3 text-left transition-all border"
                  :class="selectedUserIds.length ? 'bg-primary/10 border-primary/25 text-primary' : 'bg-surface-container border-transparent text-slate-600'">
            <span class="material-symbols-outlined text-lg">person</span>
            <div class="mt-2 text-[10px] font-black uppercase tracking-widest">联系人</div>
            <div class="mt-1 text-xs font-bold">{{ selectedUserIds.length }}/{{ users.length }}</div>
          </button>
          <button @click="openTargetDrawer('groups')"
                  class="rounded-2xl px-3 py-3 text-left transition-all border"
                  :class="selectedGroupCount ? 'bg-cyan-500/10 border-cyan-500/25 text-cyan-700' : 'bg-surface-container border-transparent text-slate-600'">
            <span class="material-symbols-outlined text-lg">groups</span>
            <div class="mt-2 text-[10px] font-black uppercase tracking-widest">群组</div>
            <div class="mt-1 text-xs font-bold">{{ selectedGroupCount }}/{{ groupTargets.length }}</div>
          </button>
          <button @click="openTargetDrawer('channels')"
                  class="rounded-2xl px-3 py-3 text-left transition-all border"
                  :class="selectedChannelCount ? 'bg-sky-500/10 border-sky-500/25 text-sky-700' : 'bg-surface-container border-transparent text-slate-600'">
            <span class="material-symbols-outlined text-lg">campaign</span>
            <div class="mt-2 text-[10px] font-black uppercase tracking-widest">频道</div>
            <div class="mt-1 text-xs font-bold">{{ selectedChannelCount }}/{{ channelTargets.length }}</div>
          </button>
        </div>

        <div class="flex items-center gap-2">
          <button @click="syncTargets"
                  class="flex-1 py-2 rounded-xl bg-emerald-500/10 text-emerald-700 text-[11px] font-bold flex items-center justify-center gap-1.5">
            <span class="material-symbols-outlined text-sm">sync</span>
            同步群组/频道
          </button>
          <button @click="clearAll"
                  :disabled="totalSelected === 0"
                  class="px-4 py-2 rounded-xl bg-slate-100 text-slate-500 text-[11px] font-bold disabled:opacity-40">
            清空
          </button>
        </div>
      </div>

      <div class="flex-1 min-h-0 overflow-y-auto no-scrollbar p-4 space-y-3">
        <div class="rounded-2xl border border-outline-variant/10 bg-surface-container-low/70 p-4">
          <div class="flex items-center justify-between gap-3">
            <div>
              <div class="text-[10px] font-black uppercase tracking-widest text-primary">添选记忆</div>
              <p class="mt-1 text-xs text-slate-500 leading-relaxed">{{ selectionMemoryHint }}</p>
            </div>
            <span class="material-symbols-outlined text-primary">checklist</span>
          </div>
        </div>

        <div v-if="selectedPreviewItems.length > 0" class="space-y-2">
          <div class="text-[10px] font-black uppercase tracking-widest text-slate-400 px-1">已选对象</div>
          <div class="flex flex-wrap gap-2">
            <span v-for="item in selectedPreviewItems" :key="item.key"
                  class="inline-flex max-w-full items-center gap-1.5 rounded-full bg-primary/10 text-primary px-3 py-1.5 text-[11px] font-bold">
              <span class="material-symbols-outlined text-sm">{{ item.icon }}</span>
              <span class="truncate max-w-[180px]">{{ item.label }}</span>
            </span>
            <span v-if="hiddenSelectedCount > 0"
                  class="inline-flex items-center rounded-full bg-surface-container text-slate-500 px-3 py-1.5 text-[11px] font-bold">
              +{{ hiddenSelectedCount }}
            </span>
          </div>
        </div>

        <div v-else class="rounded-2xl border border-dashed border-outline-variant/30 p-5 text-center text-xs text-slate-400">
          暂未选择目标，点击上方联系人、群组或频道开始添加。
        </div>
      </div>

      <!-- 手机端：继续编辑消息按钮 -->
      <div class="md:hidden p-4 border-t border-outline-variant/10">
        <button @click="mobilePanelView='detail'"
                class="w-full py-3.5 bg-primary text-white rounded-2xl font-black shadow-lg shadow-primary/20 flex items-center justify-center gap-2">
          <span class="material-symbols-outlined">edit_note</span>
          <span>继续编辑消息</span>
        </button>
      </div>
    </div>

    <!-- 右侧：消息编辑面板 -->
    <div class="flex-1 flex flex-col bg-surface overflow-hidden"
         :class="mobilePanelView==='detail'?'flex':'hidden md:flex'">

      <!-- 顶部工具栏 -->
      <div class="px-6 py-4 bg-surface-container-lowest/80 border-b border-outline-variant/10 flex items-center justify-between">
        <div class="flex items-center gap-3">
          <button class="md:hidden" @click="mobilePanelView='list'">
            <span class="material-symbols-outlined text-slate-500">arrow_back</span>
          </button>
          <h2 class="text-lg font-black flex items-center">
            <span class="material-symbols-outlined mr-2 text-primary">edit_note</span>编辑消息
          </h2>
        </div>
        <div class="text-[11px] font-bold text-primary bg-primary/10 px-3 py-1.5 rounded-lg flex items-center gap-1">
          <span class="material-symbols-outlined text-sm">visibility</span>
          实时预览
        </div>
      </div>

      <!-- 编辑 + 实时预览区域 -->
      <div class="flex-1 overflow-hidden grid grid-cols-1 xl:grid-cols-2">
        <!-- 编辑区域 -->
        <div class="overflow-y-auto no-scrollbar p-4 md:p-6 border-r border-outline-variant/10">
          <div class="max-w-2xl mx-auto space-y-5">
            <!-- 消息内容 -->
            <div class="space-y-2">
              <label class="text-[10px] font-black text-primary uppercase tracking-widest">消息内容</label>

              <div class="flex flex-wrap gap-2">
                <button @click="applyHtmlTag('b')" type="button" class="px-2.5 py-1 text-[11px] font-bold rounded-lg bg-surface-container text-slate-700 hover:bg-primary/10 hover:text-primary transition-all">粗体</button>
                <button @click="applyHtmlTag('i')" type="button" class="px-2.5 py-1 text-[11px] font-bold rounded-lg bg-surface-container text-slate-700 hover:bg-primary/10 hover:text-primary transition-all">斜体</button>
                <button @click="applyHtmlTag('u')" type="button" class="px-2.5 py-1 text-[11px] font-bold rounded-lg bg-surface-container text-slate-700 hover:bg-primary/10 hover:text-primary transition-all">下划线</button>
                <button @click="applyHtmlTag('code')" type="button" class="px-2.5 py-1 text-[11px] font-bold rounded-lg bg-surface-container text-slate-700 hover:bg-primary/10 hover:text-primary transition-all">代码</button>
                <button @click="insertHtmlLink" type="button" class="px-2.5 py-1 text-[11px] font-bold rounded-lg bg-surface-container text-slate-700 hover:bg-primary/10 hover:text-primary transition-all">链接</button>
              </div>

              <div class="border rounded-2xl p-4 focus-within:border-primary/40 transition-colors"
                   :class="themeStore.isDark ? 'bg-[#0d1828] border-[#2d4668]' : 'bg-white border-outline-variant/20'">
                <textarea ref="contentTextarea" v-model="composer.content"
                          placeholder="输入广播消息正文...&#10;&#10;支持 HTML 格式：&lt;b&gt;粗体&lt;/b&gt; &lt;i&gt;斜体&lt;/i&gt; &lt;a href='url'&gt;链接&lt;/a&gt;"
                          class="w-full bg-transparent border-none focus:ring-0 text-sm min-h-[160px] no-scrollbar resize-none leading-relaxed"
                          :class="themeStore.isDark ? 'text-slate-100 placeholder:text-slate-500' : 'text-slate-800 placeholder:text-slate-400'"></textarea>
              </div>
              <p class="text-[10px] text-slate-400">{{ composer.content.length }} 字符</p>
            </div>

            <!-- 图片上传 -->
            <div class="space-y-2">
              <label class="text-[10px] font-black text-primary uppercase tracking-widest">附加图片（可选）</label>
              <div class="flex items-center gap-3">
                <div v-if="selectedImage" class="relative w-20 h-20 rounded-xl overflow-hidden border border-outline-variant/20">
                  <img :src="selectedImagePreview" class="w-full h-full object-cover"/>
                  <button @click="clearImage" class="absolute top-0.5 right-0.5 w-5 h-5 bg-black/60 text-white rounded-full flex items-center justify-center">
                    <span class="material-symbols-outlined text-xs">close</span>
                  </button>
                </div>
                <button @click="imageInput?.click()"
                        class="flex-1 py-3 border-2 border-dashed border-outline-variant/30 rounded-xl text-slate-400 hover:border-primary/40 hover:text-primary transition-all flex items-center justify-center gap-2">
                  <span class="material-symbols-outlined text-lg">add_photo_alternate</span>
                  <span class="text-xs font-bold">选择图片</span>
                </button>
                <input ref="imageInput" type="file" accept="image/*" @change="handleImageSelect" class="hidden"/>
              </div>
            </div>

            <!-- 自定义按钮 -->
            <div class="space-y-2">
              <label class="text-[10px] font-black text-primary uppercase tracking-widest">自定义跳转按钮（可选）</label>
              <div class="border rounded-2xl p-4 space-y-3"
                   :class="themeStore.isDark ? 'bg-[#0d1828] border-[#2d4668]' : 'bg-white border-outline-variant/20'">
                <div v-for="(btn, idx) in composer.buttons" :key="idx"
                     class="flex items-center gap-2 p-2 bg-surface-container rounded-xl">
                  <span class="material-symbols-outlined text-slate-400 text-sm">link</span>
                  <input v-model="btn.text" placeholder="按钮文字" class="flex-1 bg-transparent text-xs border-none focus:ring-0 min-w-0"/>
                  <input v-model="btn.url" placeholder="https://... 或 @用户名" class="flex-1 bg-transparent text-xs border-none focus:ring-0 min-w-0"/>
                  <button @click="removeButton(idx)" class="w-6 h-6 flex items-center justify-center text-rose-400 hover:text-rose-600">
                    <span class="material-symbols-outlined text-sm">close</span>
                  </button>
                </div>
                <button @click="addButton"
                        class="w-full py-2 border border-dashed border-outline-variant/30 rounded-xl text-xs font-bold text-slate-400 hover:border-primary/40 hover:text-primary transition-all flex items-center justify-center gap-1">
                  <span class="material-symbols-outlined text-sm">add</span> 添加按钮
                </button>
              </div>
              <p class="text-[10px] text-slate-400">最多 6 个按钮，每行最多 2 个</p>
            </div>

            <!-- 选项开关 -->
            <div class="flex items-center gap-6 border p-4 rounded-2xl"
                 :class="themeStore.isDark ? 'bg-[#0d1828]/90 border-[#2d4668]' : 'bg-white/90 border-outline-variant/15'">
              <div class="flex items-center gap-3 cursor-pointer" @click="composer.doTranslate=!composer.doTranslate">
                <div class="w-10 h-6 rounded-full relative transition-all" :class="composer.doTranslate?'bg-primary':'bg-slate-300'">
                  <div class="absolute top-1 w-4 h-4 bg-white rounded-full transition-all" :class="composer.doTranslate?'left-5':'left-1'"></div>
                </div>
                <span class="text-xs font-semibold" :class="themeStore.isDark ? 'text-slate-200' : 'text-slate-700'">自动翻译</span>
              </div>
            </div>
          </div>
        </div>

        <!-- 实时预览区域 -->
        <div class="overflow-y-auto no-scrollbar p-4 md:p-6 bg-surface-container-lowest/50">
          <div class="max-w-md mx-auto">
            <div class="rounded-2xl shadow-lg border overflow-hidden"
                 :class="themeStore.isDark ? 'bg-[#0d1828] border-[#2d4668]' : 'bg-white border-outline-variant/10'">
              <!-- 消息头部 -->
              <div class="px-4 py-3 border-b flex items-center gap-2"
                   :class="themeStore.isDark ? 'bg-[#09111d] border-[#2d4668]' : 'bg-surface-container-lowest border-outline-variant/10'">
                <span class="material-symbols-outlined text-primary text-sm">campaign</span>
                <span class="text-xs font-bold" :class="themeStore.isDark ? 'text-slate-300' : 'text-slate-600'">消息预览</span>
              </div>
              <!-- 消息内容 -->
              <div class="p-4 space-y-3">
                <div v-if="selectedImagePreview" class="rounded-xl overflow-hidden">
                  <img :src="selectedImagePreview" class="w-full object-contain max-h-64"/>
                </div>
                <div v-if="composer.content" class="text-sm whitespace-pre-wrap leading-relaxed" :class="themeStore.isDark ? 'text-slate-100' : 'text-slate-800'" v-html="formatPreview(composer.content)"></div>
                <div v-else class="text-sm text-slate-300 italic">暂无内容</div>
                <div v-if="composer.buttons.length > 0" class="pt-2 border-t border-outline-variant/10 space-y-1.5">
                  <div v-for="(row, rIdx) in buttonRows" :key="rIdx" class="flex gap-1.5">
                    <button v-for="(btn, bIdx) in row" :key="bIdx"
                            class="flex-1 py-2 px-3 bg-primary/10 text-primary text-xs font-bold rounded-lg text-center truncate">
                      {{ btn.text }}
                    </button>
                  </div>
                </div>
              </div>
              <div class="px-4 py-2 border-t flex items-center justify-between"
                   :class="themeStore.isDark ? 'bg-[#09111d] border-[#2d4668]' : 'bg-surface-container-lowest border-outline-variant/10'">
                <span class="text-[10px] text-slate-400">目标: 用户 {{ selectedUserIds.length }} / 群频道 {{ selectedChatIds.length }}</span>
                <span class="text-[10px] text-slate-400">{{ new Date().toLocaleString('zh-CN') }}</span>
              </div>
            </div>
          </div>
        </div>
      </div>

      <!-- 底部发送栏 -->
      <div class="px-4 md:px-6 py-4 bg-surface-container-lowest/90 border-t border-outline-variant/10">
        <button @click="submitBroadcast"
                :disabled="!canSend || isSending"
                class="w-full py-3.5 bg-primary text-white rounded-2xl font-black shadow-lg shadow-primary/20 hover:brightness-105 disabled:opacity-50 disabled:cursor-not-allowed transition-all flex items-center justify-center gap-2">
          <span v-if="!isSending" class="material-symbols-outlined">send</span>
          <span v-else class="material-symbols-outlined animate-spin">refresh</span>
          <span>{{ isSending ? '发送中...' : sendButtonText }}</span>
        </button>
      </div>
    </div>

    <!-- 群发结果弹窗 -->
    <transition name="fade">
      <div v-if="showResultModal" class="fixed inset-0 z-[100] flex items-center justify-center p-4 bg-black/50 backdrop-blur-sm">
        <div class="bg-surface-container-low w-full max-w-lg rounded-3xl shadow-2xl overflow-hidden animate-in zoom-in-95 flex flex-col max-h-[85vh]">
          <!-- 弹窗头部 -->
          <div class="px-6 py-5 border-b border-outline-variant/10 flex items-center justify-between bg-surface-container-lowest/50">
            <div class="flex items-center gap-3">
              <span class="material-symbols-outlined text-2xl" :class="broadcastResult.success > 0 ? 'text-emerald-500' : 'text-amber-500'">
                {{ broadcastResult.success > 0 ? 'check_circle' : 'warning' }}
              </span>
              <div>
                <h3 class="text-lg font-black">群发完成</h3>
                <p class="text-[11px] text-slate-400 mt-0.5">{{ new Date().toLocaleString('zh-CN') }}</p>
              </div>
            </div>
            <button @click="showResultModal = false" class="w-10 h-10 flex items-center justify-center hover:bg-surface-container-high rounded-full transition-colors">
              <span class="material-symbols-outlined">close</span>
            </button>
          </div>

          <!-- 统计卡片 -->
          <div class="px-6 py-4 grid grid-cols-3 gap-3 bg-surface-container-lowest">
            <div class="bg-primary/10 rounded-2xl p-3 text-center">
              <div class="text-2xl font-black text-primary">{{ broadcastResult.total }}</div>
              <div class="text-[10px] font-bold text-primary/70 mt-1">总计</div>
            </div>
            <div class="bg-emerald-500/10 rounded-2xl p-3 text-center">
              <div class="text-2xl font-black text-emerald-600">{{ broadcastResult.success }}</div>
              <div class="text-[10px] font-bold text-emerald-600/70 mt-1">成功</div>
            </div>
            <div class="bg-rose-500/10 rounded-2xl p-3 text-center">
              <div class="text-2xl font-black text-rose-600">{{ broadcastResult.failed }}</div>
              <div class="text-[10px] font-bold text-rose-600/70 mt-1">失败</div>
            </div>
          </div>

          <!-- 失败明细 -->
          <div v-if="broadcastResult.failedItems && broadcastResult.failedItems.length > 0" class="px-6 py-3 border-t border-outline-variant/10 bg-surface-container-lowest">
            <h4 class="text-xs font-bold text-rose-600 mb-2 flex items-center gap-1">
              <span class="material-symbols-outlined text-sm">error</span> 失败明细
            </h4>
            <div class="max-h-32 overflow-y-auto no-scrollbar space-y-1">
              <div v-for="(item, idx) in broadcastResult.failedItems" :key="idx"
                   class="text-[11px] text-slate-500 bg-rose-500/5 rounded-lg px-3 py-2 flex justify-between">
                <span class="font-mono">{{ item.chat_id }}</span>
                <span class="truncate ml-2 text-rose-500/70">{{ item.reason }}</span>
              </div>
            </div>
          </div>

          <!-- 可复制的报告文本 -->
          <div class="px-6 py-3 border-t border-outline-variant/10 bg-surface-container-lowest">
            <div class="flex items-center justify-between mb-2">
              <h4 class="text-xs font-bold text-slate-600 flex items-center gap-1">
                <span class="material-symbols-outlined text-sm">description</span> 报告文本
              </h4>
              <button @click="copyReport"
                      class="text-[10px] font-bold text-primary hover:text-primary/80 flex items-center gap-1 px-2 py-1 rounded-lg bg-primary/10">
                <span class="material-symbols-outlined text-sm">{{ copied ? 'check' : 'content_copy' }}</span>
                {{ copied ? '已复制' : '复制' }}
              </button>
            </div>
            <pre class="text-[10px] text-slate-500 bg-surface-container rounded-xl p-3 whitespace-pre-wrap break-words max-h-32 overflow-y-auto no-scrollbar font-mono leading-relaxed">{{ reportText }}</pre>
          </div>

          <!-- 底部操作 -->
          <div class="px-6 py-4 border-t border-outline-variant/10 flex gap-3 bg-surface-container-lowest">
            <button @click="showResultModal = false"
                    class="flex-1 py-3 bg-surface-container text-slate-600 rounded-2xl font-bold hover:bg-surface-container-high transition-all">
              关闭
            </button>
            <button @click="copyReport"
                    class="flex-1 py-3 bg-primary text-white rounded-2xl font-bold shadow-lg shadow-primary/20 hover:brightness-105 transition-all flex items-center justify-center gap-1">
              <span class="material-symbols-outlined text-sm">content_copy</span>
              复制报告
            </button>
          </div>
        </div>
      </div>
    </transition>

    <transition name="fade">
      <div v-if="mobileTargetDrawerOpen" class="fixed inset-0 z-[90] bg-black/50 backdrop-blur-sm" @click="closeTargetDrawer">
        <div class="absolute inset-x-0 bottom-0 md:inset-x-auto md:right-6 md:top-6 md:bottom-6 md:w-[420px] rounded-t-[28px] md:rounded-[28px] bg-surface-container-lowest shadow-2xl border border-outline-variant/10 overflow-hidden max-h-[82vh] md:max-h-none"
             @click.stop>
          <div class="px-5 pt-3 pb-4 border-b border-outline-variant/10 bg-surface-container-lowest/95">
            <div class="w-12 h-1.5 rounded-full bg-slate-300 mx-auto mb-4"></div>
            <div class="flex items-center justify-between">
              <div>
                <div class="text-lg font-black text-slate-800">选择群发对象</div>
                <p class="text-[11px] text-slate-400 mt-1">联系人、群组、频道可跨选项添选，已勾选对象会持续保留。</p>
              </div>
              <button @click="closeTargetDrawer" class="w-10 h-10 rounded-full flex items-center justify-center bg-surface-container text-slate-500">
                <span class="material-symbols-outlined">close</span>
              </button>
            </div>

            <div class="mt-4 grid grid-cols-3 gap-2">
              <button @click="mobileTargetDrawerTab='users'"
                      :class="mobileTargetDrawerTab==='users' ? 'bg-primary text-white shadow-lg shadow-primary/20' : 'bg-surface-container text-slate-600'"
                      class="rounded-2xl px-3 py-3 text-left transition-all">
                <div class="text-[10px] font-black uppercase tracking-widest">联系人</div>
                <div class="mt-1 text-xs font-bold">{{ selectedUserIds.length }}/{{ users.length }}</div>
              </button>
              <button @click="mobileTargetDrawerTab='groups'"
                      :class="mobileTargetDrawerTab==='groups' ? 'bg-cyan-600 text-white shadow-lg shadow-cyan-600/20' : 'bg-surface-container text-slate-600'"
                      class="rounded-2xl px-4 py-3 text-left transition-all">
                <div class="text-[10px] font-black uppercase tracking-widest">群组</div>
                <div class="mt-1 text-xs font-bold">{{ selectedGroupCount }}/{{ groupTargets.length }} 已选</div>
              </button>
              <button @click="mobileTargetDrawerTab='channels'"
                      :class="mobileTargetDrawerTab==='channels' ? 'bg-sky-600 text-white shadow-lg shadow-sky-600/20' : 'bg-surface-container text-slate-600'"
                      class="rounded-2xl px-4 py-3 text-left transition-all">
                <div class="text-[10px] font-black uppercase tracking-widest">频道</div>
                <div class="mt-1 text-xs font-bold">{{ selectedChannelCount }}/{{ channelTargets.length }} 已选</div>
              </button>
            </div>

            <div class="mt-3 grid grid-cols-3 gap-2">
              <button @click="selectAllCurrentDrawerTab"
                      class="py-2 rounded-xl bg-primary/10 text-primary text-[10px] font-bold uppercase">
                全选
              </button>
              <button @click="clearCurrentDrawerTab"
                      class="py-2 rounded-xl bg-slate-100 text-slate-500 text-[10px] font-bold uppercase">
                清空
              </button>
              <button @click="syncTargets"
                      class="py-2 rounded-xl bg-emerald-500/10 text-emerald-700 text-[10px] font-bold uppercase">
                同步
              </button>
            </div>
          </div>

          <div class="overflow-y-auto no-scrollbar px-3 py-3 space-y-2 max-h-[calc(82vh-230px)] md:max-h-[calc(100vh-270px)]">
            <div v-if="isCurrentDrawerLoading" class="py-10 text-center text-xs text-slate-400">加载中...</div>
            <div v-else-if="currentDrawerItems.length === 0" class="py-10 text-center text-xs text-slate-400">
              当前暂无{{ currentDrawerLabel }}目标
            </div>
            <button v-for="item in currentDrawerItems" :key="item.key"
                    @click="toggleDrawerItem(item)"
                    class="w-full flex items-center p-3 rounded-2xl border transition-all text-left"
                    :class="item.selected ? 'bg-primary/10 border-primary/25' : 'border-transparent bg-white/70'">
              <div class="w-11 h-11 rounded-2xl mr-3 flex-shrink-0 overflow-hidden flex items-center justify-center bg-surface-container-high text-slate-500">
                <img v-if="item.avatarFileId && fileUrls[item.avatarFileId]" :src="fileUrls[item.avatarFileId]" class="w-11 h-11 object-cover block"/>
                <span v-else class="material-symbols-outlined text-[18px]">{{ item.icon }}</span>
              </div>
              <div class="flex-1 min-w-0">
                <div class="text-sm font-bold truncate text-slate-800">{{ item.title }}</div>
                <div class="mt-1 text-[11px] text-slate-400">
                  {{ item.subtitle }}
                </div>
              </div>
              <span class="material-symbols-outlined text-primary text-lg">
                {{ item.selected ? 'check_circle' : 'radio_button_unchecked' }}
              </span>
            </button>
          </div>

          <div class="px-4 py-4 border-t border-outline-variant/10 bg-surface-container-lowest">
            <button @click="closeTargetDrawer"
                    class="w-full py-3.5 rounded-2xl bg-primary text-white font-black shadow-lg shadow-primary/20">
              完成选择（{{ totalSelected }}）
            </button>
          </div>
        </div>
      </div>
    </transition>
  </div>
</template>

<script setup lang="ts">
import { ref, reactive, computed, onMounted, onBeforeUnmount } from 'vue'
import { useFileStore } from '@/stores/fileStore'
import { useThemeStore } from '@/stores/theme'

const fileStore = useFileStore()
const themeStore = useThemeStore()
const fileUrls = computed(() => fileStore.fileUrls)

const getToken = () => {
  const raw = sessionStorage.getItem('adm_s')
  if (raw) {
    try {
      return decodeURIComponent(atob(raw))
    } catch {
      return ''
    }
  }
  return localStorage.getItem('admin_token') || ''
}

let csrfToken = ''
const ensureCsrfToken = async () => {
  if (csrfToken) return csrfToken
  const t = getToken()
  if (!t) return ''
  try {
    const res = await fetch('/api/csrf-token', {
      headers: { 'Authorization': `Bearer ${t}` }
    })
    const data = await res.json()
    if (data.status === 'ok') csrfToken = data.token || ''
  } catch {
    csrfToken = ''
  }
  return csrfToken
}

const authH = () => {
  const t = getToken()
  const headers: Record<string, string> = { 'Content-Type': 'application/json' }
  if (t) headers['Authorization'] = `Bearer ${t}`
  if (csrfToken) headers['X-CSRF-Token'] = csrfToken
  return headers
}

function escapeHtmlText(value: string): string {
  return String(value || '')
    .replace(/&/g, '&amp;')
    .replace(/</g, '&lt;')
    .replace(/>/g, '&gt;')
}

function escapeHtmlAttr(value: string): string {
  return escapeHtmlText(value)
    .replace(/"/g, '&quot;')
    .replace(/'/g, '&#39;')
}

function normalizeTelegramUrl(value: string): string {
  const raw = String(value || '').trim()
  if (/^https?:\/\//i.test(raw)) return raw
  if (/^@[A-Za-z0-9_]{5,32}$/.test(raw)) return `https://t.me/${raw.slice(1)}`
  if (/^[A-Za-z0-9_]{5,32}$/.test(raw)) return `https://t.me/${raw}`
  return ''
}

function prepareButtonsForSubmit(): { text: string, url: string }[] {
  const result: { text: string, url: string }[] = []
  for (const button of composer.buttons) {
    const text = String(button.text || '').trim()
    const rawUrl = String(button.url || '').trim()
    if (!text && !rawUrl) continue
    if (!text || !rawUrl) throw new Error('请完整填写按钮文字和链接')
    const normalizedUrl = normalizeTelegramUrl(rawUrl)
    if (!normalizedUrl) throw new Error('按钮链接必须是 http(s) 地址或 Telegram 用户名')
    result.push({ text, url: normalizedUrl })
  }
  return result.slice(0, 6)
}

async function parseJsonResponse(res: Response): Promise<any> {
  const text = await res.text()
  if (!text) return {}
  try {
    return JSON.parse(text)
  } catch {
    throw new Error(`服务器返回了非 JSON 响应（HTTP ${res.status}），请检查接口路由`)
  }
}

// 移动端面板状态
const mobilePanelView = ref<'list'|'detail'>('list')
const mobileTargetDrawerOpen = ref(false)
const mobileTargetDrawerTab = ref<'users'|'groups'|'channels'>('users')

// 数据
const users = ref<any[]>([])
const targets = ref<any[]>([])
const isTargetsLoading = ref(false)

// 选择状态
const selectedUserIds = ref<number[]>([])
const selectedChatIds = ref<number[]>([])

// 编辑器状态
const composer = reactive({
  content: '',
  doTranslate: false,
  buttons: [] as {text: string, url: string}[]
})
const selectedImage = ref<File | null>(null)
const selectedImagePreview = ref<string>('')
const imageInput = ref<HTMLInputElement | null>(null)
const contentTextarea = ref<HTMLTextAreaElement | null>(null)
const isSending = ref(false)

// 群发结果弹窗
const showResultModal = ref(false)
const copied = ref(false)
const broadcastResult = reactive({
  total: 0,
  success: 0,
  failed: 0,
  failedItems: [] as {chat_id: string | number, reason: string}[],
  content: '',
  buttons: [] as {text: string, url: string}[],
  selectedUsers: [] as number[],
  selectedChats: [] as number[],
})

// 生成报告文本
const reportText = computed(() => {
  const lines = [
    `📢 群发报告`,
    `━━━━━━━━━━━━━━━━`,
    `📊 总计: ${broadcastResult.total} 个目标`,
    `👤 用户: ${broadcastResult.selectedUsers.length} 个`,
    `👥 群/频道: ${broadcastResult.selectedChats.length} 个`,
    `✅ 成功: ${broadcastResult.success} 个`,
    `❌ 失败: ${broadcastResult.failed} 个`,
    `━━━━━━━━━━━━━━━━`,
    `📝 内容: ${broadcastResult.content.length > 50 ? broadcastResult.content.substring(0, 50) + '...' : broadcastResult.content}`,
  ]
  if (broadcastResult.buttons.length > 0) {
    lines.push(`🔘 按钮 (${broadcastResult.buttons.length}个):`)
    for (const btn of broadcastResult.buttons) {
      lines.push(`  🔗 ${btn.text}: ${btn.url}`)
    }
  }
  if (broadcastResult.failedItems.length > 0) {
    lines.push(`━━━━━━━━━━━━━━━━`)
    lines.push(`⚠️ 失败明细:`)
    for (const item of broadcastResult.failedItems.slice(0, 10)) {
      lines.push(`  • ${item.chat_id}: ${item.reason.substring(0, 40)}`)
    }
    if (broadcastResult.failedItems.length > 10) {
      lines.push(`  ... 还有 ${broadcastResult.failedItems.length - 10} 条`)
    }
  }
  return lines.join('\n')
})

// 计算属性
const totalSelected = computed(() => selectedUserIds.value.length + selectedChatIds.value.length)
const groupTargets = computed(() => targets.value.filter((t: any) => t.chat_type !== 'channel'))
const channelTargets = computed(() => targets.value.filter((t: any) => t.chat_type === 'channel'))
const selectedGroupCount = computed(() => groupTargets.value.filter((t: any) => selectedChatIds.value.includes(Number(t.chat_id))).length)
const selectedChannelCount = computed(() => channelTargets.value.filter((t: any) => selectedChatIds.value.includes(Number(t.chat_id))).length)
const currentDrawerLabel = computed(() => {
  if (mobileTargetDrawerTab.value === 'users') return '联系人'
  if (mobileTargetDrawerTab.value === 'groups') return '群组'
  return '频道'
})
const currentDrawerItems = computed(() => {
  if (mobileTargetDrawerTab.value === 'users') {
    return users.value.map((u: any) => ({
      kind: 'user' as const,
      key: `user-${u.from_userid}`,
      id: Number(u.from_userid),
      title: u.from_username || u.username || `用户 ${u.from_userid}`,
      subtitle: `联系人 · ID ${u.from_userid}`,
      icon: 'person',
      avatarFileId: u.avatar_file_id,
      selected: selectedUserIds.value.includes(Number(u.from_userid)),
    }))
  }

  const source = mobileTargetDrawerTab.value === 'groups' ? groupTargets.value : channelTargets.value
  return source.map((t: any) => ({
    kind: 'chat' as const,
    key: `chat-${t.chat_id}`,
    id: Number(t.chat_id),
    title: t.chat_title || String(t.chat_id),
    subtitle: `${t.chat_type === 'channel' ? '频道' : '群组'} · ID ${t.chat_id}`,
    icon: t.chat_type === 'channel' ? 'campaign' : 'groups',
    selected: selectedChatIds.value.includes(Number(t.chat_id)),
  }))
})
const isCurrentDrawerLoading = computed(() => mobileTargetDrawerTab.value !== 'users' && isTargetsLoading.value)
const selectionMemoryHint = computed(() => {
  if (totalSelected.value === 0) return '从联系人、群组、频道里分别添选目标，切换选项不会清空之前的勾选。'
  return `已暂存联系人 ${selectedUserIds.value.length}、群组 ${selectedGroupCount.value}、频道 ${selectedChannelCount.value}。`
})
const selectedPreviewItems = computed(() => {
  const userItems = users.value
    .filter((u: any) => selectedUserIds.value.includes(Number(u.from_userid)))
    .map((u: any) => ({
      key: `user-${u.from_userid}`,
      icon: 'person',
      label: u.from_username || u.username || `用户 ${u.from_userid}`,
    }))
  const chatItems = targets.value
    .filter((t: any) => selectedChatIds.value.includes(Number(t.chat_id)))
    .map((t: any) => ({
      key: `chat-${t.chat_id}`,
      icon: t.chat_type === 'channel' ? 'campaign' : 'groups',
      label: t.chat_title || String(t.chat_id),
    }))
  return [...userItems, ...chatItems].slice(0, 8)
})
const hiddenSelectedCount = computed(() => Math.max(totalSelected.value - selectedPreviewItems.value.length, 0))

const canSend = computed(() => {
  return totalSelected.value > 0 && (composer.content.trim() || selectedImage.value)
})

const sendButtonText = computed(() => {
  if (totalSelected.value === 0) return '请选择发送目标'
  return `发送给 ${selectedUserIds.value.length} 个用户 + ${selectedChatIds.value.length} 个群/频道`
})

// 按钮按行分组（每行最多 2 个）
const buttonRows = computed(() => {
  const rows: {text: string, url: string}[][] = []
  for (let i = 0; i < composer.buttons.length; i += 2) {
    rows.push(composer.buttons.slice(i, i + 2))
  }
  return rows
})

// 生命周期
onMounted(async () => {
  await ensureCsrfToken()
  await Promise.all([loadUsers(), loadTargets()])
})

onBeforeUnmount(() => {
  resetBroadcastSession()
})

// 加载用户列表
async function loadUsers() {
  const res = await fetch('/api/users', { headers: authH() })
  const d = await res.json()
  if (d.status === 'ok') {
    users.value = d.data || []
    const fids = users.value.map((u: any) => u.avatar_file_id).filter(Boolean)
    await fileStore.preloadFiles(fids)
  }
}

// 加载群/频道目标
async function loadTargets() {
  isTargetsLoading.value = true
  try {
    const res = await fetch('/api/broadcast/targets', { headers: authH() })
    const d = await res.json()
    if (d.status === 'ok') targets.value = d.data || []
  } finally {
    isTargetsLoading.value = false
  }
}

// 同步目标
async function syncTargets() {
  await fetch('/api/broadcast/targets/sync', { method: 'POST', headers: authH() })
  await loadTargets()
}

// 选择操作
function selectAllUsers() {
  selectedUserIds.value = users.value.map((u: any) => Number(u.from_userid))
}

function selectAllTargets() {
  selectedChatIds.value = targets.value.map((t: any) => Number(t.chat_id))
}

function clearSelectedUsers() {
  selectedUserIds.value = []
}

function clearSelectedTargets() {
  selectedChatIds.value = []
}

function clearAll() {
  clearSelectedUsers()
  clearSelectedTargets()
}

function openTargetDrawer(tab: 'users' | 'groups' | 'channels') {
  mobileTargetDrawerTab.value = tab
  mobileTargetDrawerOpen.value = true
}

function closeTargetDrawer() {
  mobileTargetDrawerOpen.value = false
}

function selectAllCurrentDrawerTab() {
  if (mobileTargetDrawerTab.value === 'users') {
    const nextUserIds = users.value.map((u: any) => Number(u.from_userid))
    selectedUserIds.value = Array.from(new Set([...selectedUserIds.value, ...nextUserIds]))
    return
  }

  const source = mobileTargetDrawerTab.value === 'groups' ? groupTargets.value : channelTargets.value
  const nextIds = source.map((t: any) => Number(t.chat_id))
  selectedChatIds.value = Array.from(new Set([...selectedChatIds.value, ...nextIds]))
}

function clearCurrentDrawerTab() {
  if (mobileTargetDrawerTab.value === 'users') {
    selectedUserIds.value = []
    return
  }

  const source = mobileTargetDrawerTab.value === 'groups' ? groupTargets.value : channelTargets.value
  const currentIds = new Set(source.map((t: any) => Number(t.chat_id)))
  selectedChatIds.value = selectedChatIds.value.filter((id) => !currentIds.has(id))
}

function toggleUser(id: number) {
  const i = selectedUserIds.value.indexOf(id)
  if (i >= 0) selectedUserIds.value.splice(i, 1)
  else selectedUserIds.value.push(id)
}

function toggleTarget(id: number) {
  const i = selectedChatIds.value.indexOf(id)
  if (i >= 0) selectedChatIds.value.splice(i, 1)
  else selectedChatIds.value.push(id)
}

function toggleDrawerItem(item: { kind: 'user' | 'chat', id: number }) {
  if (item.kind === 'user') toggleUser(item.id)
  else toggleTarget(item.id)
}

// HTML 快捷格式工具
function wrapSelection(openTag: string, closeTag: string) {
  const el = contentTextarea.value
  if (!el) return

  const start = el.selectionStart ?? composer.content.length
  const end = el.selectionEnd ?? composer.content.length
  const selected = composer.content.slice(start, end)
  const insertText = `${openTag}${selected || '内容'}${closeTag}`

  composer.content = `${composer.content.slice(0, start)}${insertText}${composer.content.slice(end)}`

  requestAnimationFrame(() => {
    el.focus()
    const cursor = start + insertText.length
    el.setSelectionRange(cursor, cursor)
  })
}

function applyHtmlTag(tag: 'b' | 'i' | 'u' | 'code') {
  wrapSelection(`<${tag}>`, `</${tag}>`)
}

function insertHtmlLink() {
  const el = contentTextarea.value
  if (!el) return

  const start = el.selectionStart ?? composer.content.length
  const end = el.selectionEnd ?? composer.content.length
  const selected = composer.content.slice(start, end)
  const text = selected || '点击查看'
  const url = window.prompt('请输入链接 URL（例如 https://example.com）', 'https://')
  if (!url) return
  const normalizedUrl = normalizeTelegramUrl(url)
  if (!normalizedUrl) {
    alert('链接必须是 http(s) 地址或 Telegram 用户名')
    return
  }

  const insertText = `<a href="${escapeHtmlAttr(normalizedUrl)}">${escapeHtmlText(text)}</a>`
  composer.content = `${composer.content.slice(0, start)}${insertText}${composer.content.slice(end)}`

  requestAnimationFrame(() => {
    el.focus()
    const cursor = start + insertText.length
    el.setSelectionRange(cursor, cursor)
  })
}

// 图片处理
function handleImageSelect(e: Event) {
  const input = e.target as HTMLInputElement
  if (input.files && input.files[0]) {
    selectedImage.value = input.files[0]
    const reader = new FileReader()
    reader.onload = (ev) => {
      selectedImagePreview.value = ev.target?.result as string
    }
    reader.readAsDataURL(input.files[0])
  }
}

function clearImage() {
  selectedImage.value = null
  selectedImagePreview.value = ''
  if (imageInput.value) imageInput.value.value = ''
}

function resetBroadcastSession() {
  selectedUserIds.value = []
  selectedChatIds.value = []
  mobilePanelView.value = 'list'
  mobileTargetDrawerOpen.value = false
  mobileTargetDrawerTab.value = 'users'
  composer.content = ''
  composer.doTranslate = false
  composer.buttons = []
  showResultModal.value = false
  copied.value = false
  broadcastResult.total = 0
  broadcastResult.success = 0
  broadcastResult.failed = 0
  broadcastResult.failedItems = []
  broadcastResult.content = ''
  broadcastResult.buttons = []
  broadcastResult.selectedUsers = []
  broadcastResult.selectedChats = []
  clearImage()
}

// 按钮管理
function addButton() {
  if (composer.buttons.length >= 6) return
  composer.buttons.push({ text: '', url: '' })
}

function removeButton(idx: number) {
  composer.buttons.splice(idx, 1)
}

// 预览格式化
function formatPreview(text: string): string {
  return text
    .replace(/\n/g, '<br>')
}

// 提交发送
async function submitBroadcast() {
  if (!canSend.value || isSending.value) return

  isSending.value = true
  try {
    await ensureCsrfToken()
    const formData = new FormData()
    formData.append('content', composer.content)
    formData.append('do_translate', String(composer.doTranslate))
    formData.append('user_ids', JSON.stringify(selectedUserIds.value))
    formData.append('chat_ids', JSON.stringify(selectedChatIds.value))
    const submitButtons = prepareButtonsForSubmit()
    if (submitButtons.length > 0) {
      formData.append('buttons', JSON.stringify(submitButtons))
    }
    if (selectedImage.value) {
      formData.append('image', selectedImage.value)
    }

    const res = await fetch('/api/broadcast/unified', {
      method: 'POST',
      headers: {
        'Authorization': `Bearer ${getToken()}`,
        ...(csrfToken ? { 'X-CSRF-Token': csrfToken } : {})
      },
      body: formData
    })

    const data = await parseJsonResponse(res)
    if (res.status === 403) {
      csrfToken = ''
      await ensureCsrfToken()
      throw new Error(data.message || '认证已刷新，请重试发送')
    }
    if (!res.ok) {
      throw new Error(data.message || `发送接口返回 HTTP ${res.status}`)
    }

    if (data.status === 'ok') {
      // 填充结果数据
      broadcastResult.total = data.summary.total
      broadcastResult.success = data.summary.success
      broadcastResult.failed = data.summary.failed
      broadcastResult.failedItems = data.summary.failed_items || data.failed_items || []
      broadcastResult.content = composer.content
      broadcastResult.buttons = [...submitButtons]
      broadcastResult.selectedUsers = [...selectedUserIds.value]
      broadcastResult.selectedChats = [...selectedChatIds.value]
      showResultModal.value = true

      // 清空表单
      composer.content = ''
      composer.buttons = []
      selectedUserIds.value = []
      selectedChatIds.value = []
      mobileTargetDrawerOpen.value = false
      mobileTargetDrawerTab.value = 'users'
      clearImage()
    } else {
      alert('发送失败: ' + (data.message || '未知错误'))
    }
  } catch (e: any) {
    alert('发送失败: ' + (e?.message || '网络错误'))
  } finally {
    isSending.value = false
  }
}

// 复制报告
async function copyReport() {
  try {
    await navigator.clipboard.writeText(reportText.value)
    copied.value = true
    setTimeout(() => { copied.value = false }, 2000)
  } catch {
    // Fallback for older browsers
    const textarea = document.createElement('textarea')
    textarea.value = reportText.value
    textarea.style.position = 'fixed'
    textarea.style.opacity = '0'
    document.body.appendChild(textarea)
    textarea.select()
    document.execCommand('copy')
    document.body.removeChild(textarea)
    copied.value = true
    setTimeout(() => { copied.value = false }, 2000)
  }
}
</script>

<style scoped>
.broadcast-list-items {
  position: relative;
  z-index: 1;
}

.broadcast-list-lock {
  box-shadow: 0 1px 0 rgba(57, 87, 126, 0.28);
}

.line-clamp-2 {
  display: -webkit-box;
  -webkit-line-clamp: 2;
  -webkit-box-orient: vertical;
  overflow: hidden;
}
</style>
