<script setup lang="ts">
import { watch, computed } from 'vue';
import { useModalHistory } from '../../core/composables/useModalHistory';
import { useUpdateStore } from '../../core/stores/update';
import { marked } from 'marked';
import DOMPurify from 'dompurify';

const props = defineProps<{
  isOpen: boolean;
  version: string;
  releaseNotes?: string | null;
  apkSize?: number | null;
}>();

const emit = defineEmits<{
  (e: 'confirm'): void;
  (e: 'dismiss'): void;
  (e: 'update:isOpen', value: boolean): void;
}>();

const { registerModal, unregisterModal } = useModalHistory();
const modalId = 'UpdatePrompt';
const updateStore = useUpdateStore();

watch(
  () => props.isOpen,
  (newVal) => {
    if (newVal) {
      registerModal(modalId, () => {
        // 如果在下载中，物理返回键不应该直接关闭弹窗导致阻断（或者允许它退到后台）
        // 这里我们按标准处理
        emit('dismiss');
        emit('update:isOpen', false);
      });
    } else {
      unregisterModal(modalId);
    }
  },
);

const handleConfirm = () => {
  emit('confirm');
};

const handleDismiss = () => {
  emit('dismiss');
  emit('update:isOpen', false);
  if (updateStore.status === 'error') {
    updateStore.reset();
  }
};

const formatBytes = (bytes: number) => {
  if (bytes === 0) return '0 B';
  const k = 1024;
  const sizes = ['B', 'KB', 'MB', 'GB'];
  const i = Math.floor(Math.log(bytes) / Math.log(k));
  return parseFloat((bytes / Math.pow(k, i)).toFixed(2)) + ' ' + sizes[i];
};

// 配置 marked：支持 GFM 和换行
marked.setOptions({
  breaks: true,
  gfm: true,
});

const releaseNotesHtml = computed(() => {
  if (!props.releaseNotes) return '';
  const parsed = marked.parse(props.releaseNotes);
  return DOMPurify.sanitize(parsed as string);
});

const progressPercent = computed(() => {
  if (updateStore.status !== 'downloading') return 0;
  const progress = updateStore.downloadProgress;
  const total = updateStore.downloadTotal;
  if (!total || total === 0) return 0;
  return Math.min(100, Math.round((progress / total) * 100));
});
</script>

<template>
  <Teleport to="body">
    <Transition name="prompt-fade">
      <div
        v-if="isOpen"
        class="fixed inset-0 z-dialog flex items-start justify-center pt-[12vh] bg-black/60"
        @click.self="handleDismiss"
      >
        <div
          class="bg-white dark:bg-zinc-900 w-11/12 max-w-sm rounded-xl shadow-xl border border-black/10 dark:border-white/10 p-5 transform transition-all relative overflow-hidden"
        >
          <!-- 标题区 -->
          <div class="flex items-baseline justify-between mb-3">
            <h3 class="text-base font-bold text-gray-900 dark:text-zinc-100 tracking-tight">
              发现新版本
            </h3>
            <span class="inline-flex items-center px-2 py-0.5 rounded-full text-xs font-mono font-bold bg-blue-500/10 text-blue-600 dark:text-blue-400 border border-blue-500/15">
              {{ version }}
            </span>
          </div>

          <!-- 更新细则 -->
          <div
            v-if="releaseNotesHtml"
            class="vcp-markdown-block text-xs text-gray-700 dark:text-gray-300 opacity-90 max-h-[32vh] overflow-y-auto leading-relaxed mb-3 bg-black/5 dark:bg-white/5 rounded-lg p-3.5 border border-black/5 dark:border-white/5 custom-scrollbar"
            v-html="releaseNotesHtml"
          ></div>

          <!-- 文件大小信息 -->
          <div
            v-if="apkSize"
            class="flex items-center gap-1.5 text-[10px] text-gray-400 dark:text-gray-500 font-mono mb-3"
          >
            <svg xmlns="http://www.w3.org/2000/svg" class="h-3 w-3 opacity-60" fill="none" viewBox="0 0 24 24" stroke="currentColor">
              <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M4 16v1a3 3 0 003 3h10a3 3 0 003-3v-1m-4-4l-4 4m0 0l-4-4m4 4V4" />
            </svg>
            安装包大小: {{ formatBytes(apkSize) }}
          </div>

          <!-- 错误信息反馈 -->
          <div
            v-if="updateStore.status === 'error'"
            class="text-xs text-red-500 bg-red-500/10 dark:bg-red-400/10 border border-red-500/20 dark:border-red-400/20 rounded-lg p-3 mb-3"
          >
            <div class="font-bold mb-1 flex items-center gap-1">
              <svg xmlns="http://www.w3.org/2000/svg" class="h-3.5 w-3.5" viewBox="0 0 20 20" fill="currentColor">
                <path fill-rule="evenodd" d="M18 10a8 8 0 11-16 0 8 8 0 0116 0zm-7 4a1 1 0 11-2 0 1 1 0 012 0zm-1-9a1 1 0 00-1 1v4a1 1 0 102 0V6a1 1 0 00-1-1z" clip-rule="evenodd" />
              </svg>
              更新失败
            </div>
            <div class="font-mono break-all opacity-90 leading-tight">{{ updateStore.errorMsg }}</div>
          </div>

          <!-- 进度条容器 (仅在下载中展示) -->
          <div v-if="updateStore.status === 'downloading'" class="mt-2 space-y-1.5 mb-3">
            <div class="h-1.5 bg-black/10 dark:bg-white/10 rounded-full overflow-hidden">
              <div
                class="h-full bg-blue-500 transition-all duration-200"
                :style="{ width: progressPercent + '%' }"
              />
            </div>
            <div class="flex justify-between text-[10px] font-mono opacity-60">
              <span>已下载 {{ progressPercent }}%</span>
              <span v-if="updateStore.downloadTotal">
                {{ formatBytes(updateStore.downloadProgress) }} / {{ formatBytes(updateStore.downloadTotal) }}
              </span>
              <span v-else>{{ formatBytes(updateStore.downloadProgress) }}</span>
            </div>
          </div>

          <!-- 操作按钮区 -->
          <div class="flex justify-end gap-2.5 pt-1">
            <button
              :disabled="updateStore.status === 'downloading' || updateStore.status === 'installing'"
              @click="handleDismiss"
              class="px-4 py-2 rounded-lg text-xs font-semibold text-gray-500 dark:text-gray-400 hover:bg-black/5 dark:hover:bg-white/5 hover:text-gray-800 dark:hover:text-gray-200 transition-all active:opacity-80 active:scale-[0.98] duration-150 disabled:opacity-30 disabled:pointer-events-none"
            >
              取消
            </button>
            <button
              :disabled="updateStore.status === 'downloading' || updateStore.status === 'installing'"
              @click="handleConfirm"
              class="px-4 py-2 rounded-lg text-xs font-semibold bg-blue-600 hover:bg-blue-500 text-white shadow-md active:opacity-85 active:scale-[0.98] transition-all duration-150 disabled:opacity-60 disabled:pointer-events-none"
            >
              <span v-if="updateStore.status === 'downloading'">正在下载 {{ progressPercent }}%...</span>
              <span v-else-if="updateStore.status === 'installing'">正在安装...</span>
              <span v-else>立即更新</span>
            </button>
          </div>
        </div>
      </div>
    </Transition>
  </Teleport>
</template>

<style scoped>
/* 深度美化滚动条 */
.custom-scrollbar::-webkit-scrollbar {
  width: 4px;
}
.custom-scrollbar::-webkit-scrollbar-track {
  background: transparent;
}
.custom-scrollbar::-webkit-scrollbar-thumb {
  background: rgba(156, 163, 175, 0.25);
  border-radius: 99px;
}
.custom-scrollbar::-webkit-scrollbar-thumb:hover {
  background: rgba(156, 163, 175, 0.45);
}

/* 渐变平滑动画 */
.prompt-fade-enter-active {
  transition: opacity 0.25s ease-out;
}
.prompt-fade-leave-active {
  transition: opacity 0.2s ease-in;
}
.prompt-fade-enter-from,
.prompt-fade-leave-to {
  opacity: 0;
}
.prompt-fade-enter-from > div {
  transform: scale(0.98) translateY(8px);
  opacity: 0;
}
.prompt-fade-enter-to > div {
  transform: scale(1) translateY(0px);
  opacity: 1;
}
.prompt-fade-leave-from > div {
  transform: scale(1) translateY(0px);
  opacity: 1;
}
.prompt-fade-leave-to > div {
  transform: scale(0.98) translateY(4px);
  opacity: 0;
}
</style>
