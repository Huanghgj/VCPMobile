<template>
  <AttachmentPreviewBase
    :file="file"
    :index="index"
    :show-remove="showRemove"
    size="auto"
    @remove="emit('remove', index)"
  >
    <div class="flex items-center gap-2.5 px-2.5 py-2 min-w-[120px] max-w-[180px]">
      <div class="relative w-7 h-7 shrink-0 rounded flex items-center justify-center bg-blue-500/10 dark:bg-blue-400/10 border border-blue-500/20 dark:border-blue-400/20">
        <img
          v-if="posterSrc"
          :src="posterSrc"
          :alt="displayName"
          class="absolute inset-0 w-full h-full object-cover rounded"
          loading="lazy"
          decoding="async"
        />
        <div
          v-if="posterSrc"
          class="absolute inset-0 rounded bg-black/25"
        ></div>
        <svg
          width="14"
          height="14"
          viewBox="0 0 24 24"
          fill="none"
          stroke="currentColor"
          stroke-width="2"
          stroke-linecap="round"
          stroke-linejoin="round"
          class="text-blue-500 dark:text-blue-400 relative z-[1]"
        >
          <polygon points="23 7 16 12 23 17 23 7"></polygon>
          <rect x="1" y="5" width="15" height="14" rx="2" ry="2"></rect>
        </svg>
      </div>
      <div class="flex flex-col min-w-0">
        <div class="text-[12px] font-medium truncate text-[var(--primary-text)] leading-tight mb-0.5">
          {{ displayName }}
        </div>
        <div class="text-[10px] opacity-50 truncate leading-none">
          VIDEO • {{ formatSize(file.size) }}
        </div>
      </div>
    </div>
  </AttachmentPreviewBase>
</template>

<script setup lang="ts">
import { computed } from "vue";
import { convertFileSrc } from "@tauri-apps/api/core";
import AttachmentPreviewBase from "../AttachmentPreviewBase.vue";
import { truncateFileName } from "../utils/truncateFileName";
import type { Attachment } from "../../../../core/types/chat";

const props = withDefaults(defineProps<{
  file: Attachment;
  index: number;
  showRemove?: boolean;
}>(), {
  showRemove: false
});

const emit = defineEmits<{ (e: "remove", index: number): void }>();

const displayName = computed(() => truncateFileName(props.file.name || 'Video'));

// 将本地物理路径转换为 WebView 可加载的 asset URL；网络/data/blob 地址原样透传。
const resolveLocalSrc = (raw?: string): string => {
  if (!raw) return "";
  if (raw.startsWith("http") || raw.startsWith("data:") || raw.startsWith("blob:")) {
    return raw;
  }
  try {
    return convertFileSrc(raw.replace("file://", ""));
  } catch {
    return "";
  }
};

// 缩略图作为首帧封面（若有），减少未播放时的黑屏。
const posterSrc = computed(() => resolveLocalSrc(props.file.thumbnailPath));

const formatSize = (bytes: number) => {
  if (!bytes) return '0 B';
  const k = 1024;
  const sizes = ['B', 'KB', 'MB', 'GB'];
  const i = Math.floor(Math.log(bytes) / Math.log(k));
  return parseFloat((bytes / Math.pow(k, i)).toFixed(1)) + ' ' + sizes[i];
};
</script>
