<script setup lang="ts">
import { computed, ref, watch } from 'vue';
import { convertFileSrc, invoke } from '@tauri-apps/api/core';
import { CheckCircle2, Download, Loader2, Music, Video, X } from 'lucide-vue-next';
import type { MediaViewerConfig } from '../../core/types/overlay';
import { useNotificationStore } from '../../core/stores/notification';

interface SavedMediaInfo {
  name: string;
  path: string;
  mimeType: string;
  size: number;
  hash: string;
}

const props = defineProps<{
  config: MediaViewerConfig;
}>();

const emit = defineEmits<{ (e: 'close'): void }>();

const notificationStore = useNotificationStore();
const saving = ref(false);
const savedInfo = ref<SavedMediaInfo | null>(null);
const currentSrc = ref('');

watch(
  () => props.config,
  (config) => {
    currentSrc.value = config.src;
    savedInfo.value = null;
    saving.value = false;
  },
  { immediate: true },
);

const sourceForSave = computed(() => props.config.originalSrc || props.config.src);
const canSave = computed(() => /^(https?:|data:)/i.test(sourceForSave.value));
const isImage = computed(() => props.config.mediaType === 'image');
const isVideo = computed(() => props.config.mediaType === 'video');
const isAudio = computed(() => props.config.mediaType === 'audio');

const renderSrc = computed(() => {
  if (!currentSrc.value) return '';
  if (/^(https?:|data:|blob:|asset:|http:\/\/asset\.localhost)/i.test(currentSrc.value)) {
    return currentSrc.value;
  }
  try {
    return convertFileSrc(currentSrc.value.replace('file://', ''));
  } catch {
    return currentSrc.value;
  }
});

const title = computed(() => props.config.title || (isImage.value ? '图片预览' : isVideo.value ? '视频预览' : '音频预览'));

const saveLabel = computed(() => {
  if (saving.value) return '保存中';
  if (savedInfo.value) return '已保存';
  if (!canSave.value) return '本地文件';
  return '保存到本地';
});

const formatSize = (size: number) => {
  if (!Number.isFinite(size) || size <= 0) return '';
  if (size > 1024 * 1024) return `${(size / 1024 / 1024).toFixed(1)} MB`;
  return `${Math.max(1, Math.round(size / 1024))} KB`;
};

const saveMedia = async () => {
  if (!canSave.value || saving.value || savedInfo.value) return;

  saving.value = true;
  try {
    const saved = await invoke<SavedMediaInfo>('save_media_from_url', {
      url: sourceForSave.value,
      suggestedName: props.config.title || undefined,
      mimeType: props.config.mimeType || undefined,
    });
    savedInfo.value = saved;
    currentSrc.value = saved.path;
    notificationStore.addNotification({
      type: 'success',
      title: '媒体已保存',
      message: saved.path,
      toastOnly: true,
      duration: 2400,
    });
  } catch (error) {
    notificationStore.addNotification({
      type: 'error',
      title: '媒体保存失败',
      message: String(error),
      toastOnly: true,
      duration: 3600,
    });
  } finally {
    saving.value = false;
  }
};
</script>

<template>
  <Transition name="media-viewer">
    <div class="fixed inset-0 z-[1100] flex flex-col bg-black/90 backdrop-blur-xl pointer-events-auto">
      <header class="flex items-center justify-between gap-3 px-4 py-3 border-b border-white/10 shrink-0">
        <div class="min-w-0">
          <div class="text-sm font-bold text-white truncate">{{ title }}</div>
          <div class="text-[10px] text-white/45 uppercase tracking-widest truncate">
            {{ savedInfo?.mimeType || config.mimeType || config.mediaType }}
            <span v-if="savedInfo?.size"> · {{ formatSize(savedInfo.size) }}</span>
          </div>
        </div>

        <div class="flex items-center gap-2 shrink-0">
          <button
            class="h-10 px-3 rounded-full bg-white/8 text-white/80 border border-white/10 flex items-center gap-2 text-xs font-bold disabled:opacity-45 active:scale-95 transition"
            :disabled="saving || !!savedInfo || !canSave"
            @click="saveMedia"
          >
            <Loader2 v-if="saving" :size="16" class="animate-spin" />
            <CheckCircle2 v-else-if="savedInfo" :size="16" />
            <Download v-else :size="16" />
            <span>{{ saveLabel }}</span>
          </button>

          <button class="w-10 h-10 rounded-full bg-white/8 text-white flex items-center justify-center active:scale-95 transition"
            @click="emit('close')">
            <X :size="22" />
          </button>
        </div>
      </header>

      <main class="flex-1 min-h-0 overflow-auto flex items-center justify-center p-3">
        <img
          v-if="isImage"
          :src="renderSrc"
          :alt="title"
          class="max-w-full max-h-full object-contain rounded-lg select-none"
        />

        <video
          v-else-if="isVideo"
          :src="renderSrc"
          class="max-w-full max-h-full rounded-lg bg-black"
          controls
          playsinline
          preload="metadata"
        />

        <div v-else-if="isAudio" class="w-full max-w-xl flex flex-col items-center gap-5 px-5">
          <div class="w-20 h-20 rounded-full bg-white/10 border border-white/10 flex items-center justify-center text-white/70">
            <Music :size="34" />
          </div>
          <audio :src="renderSrc" class="w-full" controls preload="metadata" />
        </div>

        <div v-else class="text-white/55 flex flex-col items-center gap-4">
          <Video :size="40" />
          <span class="text-sm">暂不支持预览该媒体类型</span>
        </div>
      </main>

      <footer v-if="savedInfo" class="px-4 py-3 border-t border-white/10 text-[10px] text-white/45 break-all shrink-0">
        {{ savedInfo.path }}
      </footer>
    </div>
  </Transition>
</template>

<style scoped>
.media-viewer-enter-active,
.media-viewer-leave-active {
  transition: opacity 0.18s ease;
}

.media-viewer-enter-from,
.media-viewer-leave-to {
  opacity: 0;
}
</style>
