<script setup lang="ts">
import { computed, ref } from 'vue';
import { ChevronDown, ChevronUp, Image, Loader2, Music, Settings, Video } from 'lucide-vue-next';
import MarkdownBlock from './MarkdownBlock.vue';
import type { ContentBlock } from '../../../core/composables/useContentProcessor';
import { useOverlayStore } from '../../../core/stores/overlay';
import {
  isLikelyMarkdownToolField,
  splitToolResultSegments,
  type ToolMediaInfo,
} from '../../../core/utils/toolPreview';

const props = defineProps<{
  type: 'tool-use' | 'tool-result';
  content?: string;
  block: ContentBlock;
}>();

const isExpanded = ref(props.type === 'tool-result' ? false : true);
const overlayStore = useOverlayStore();

const isPendingResult = computed(() => props.type === 'tool-result' && props.block.is_complete === false);
const isRunning = computed(() => (props.type === 'tool-use' && !props.block.is_complete) || isPendingResult.value);
const toolStatus = computed(() => props.block.status || (isPendingResult.value ? '接收中' : ''));
const details = computed(() =>
  (props.block.details || []).map((item) => ({
    ...item,
    segments: item.value ? splitToolResultSegments(item.key, item.value) : [],
    useMarkdown: isLikelyMarkdownToolField(item.key),
  })),
);

const toggleExpand = () => {
  isExpanded.value = !isExpanded.value;
};

const mediaIcon = (media: ToolMediaInfo) => {
  if (media.type === 'video') return Video;
  if (media.type === 'audio') return Music;
  return Image;
};

const previewLabel = (media: ToolMediaInfo) => {
  if (media.type === 'video') return '预览视频';
  if (media.type === 'audio') return '预览音频';
  return '预览图片';
};

const openMedia = (media: ToolMediaInfo, title?: string) => {
  overlayStore.openMediaViewer({
    src: media.src,
    originalSrc: media.originalSrc,
    mediaType: media.type,
    title: title || media.title,
    mimeType: media.mimeType,
  });
};
</script>

<template>
  <div class="vcp-tool-block my-2 rounded-xl transition-all duration-200 overflow-hidden" :class="[
    type === 'tool-use' ? 'is-tool-use' : 'is-tool-result',
    isPendingResult ? 'is-pending' : '',
    isRunning ? 'is-running' : '',
    isExpanded ? 'shadow-md' : 'shadow-sm'
  ]">
    <!-- Header -->
    <div class="tool-header-content flex items-center justify-between p-3 cursor-pointer select-none"
      @click="toggleExpand">
      <div class="flex items-center gap-2">
        <div class="tool-icon-container p-1.5 rounded-lg">
          <Settings v-if="type === 'tool-use'" :size="14" />
          <span v-else class="text-lg leading-none">📊</span>
        </div>
        <div>
          <span class="tool-label text-[10px] font-bold leading-none mb-1 flex items-center gap-1">
            {{ type === 'tool-use' ? 'VCP-ToolUse' : 'VCP-ToolResult' }}
            <Loader2 v-if="isRunning" :size="10" class="animate-spin" />
          </span>
          <span class="tool-name text-xs font-bold font-mono">
            {{ block.tool_name || 'Unknown Tool' }}
          </span>
        </div>
      </div>

      <div class="flex items-center gap-2">
        <span v-if="toolStatus" class="tool-status text-[10px] px-1.5 py-0.5 rounded font-bold">
          {{ toolStatus }}
        </span>
        <component :is="isExpanded ? ChevronUp : ChevronDown" :size="16" class="opacity-50" />
      </div>
    </div>

    <!-- Content -->
    <div v-if="isExpanded"
      class="tool-header-content border-t border-black/10 dark:border-white/10 p-3 animate-slide-down tool-content-scrollable vcp-scrollable">
      <template v-if="type === 'tool-use'">
        <pre class="text-[11px] font-mono whitespace-pre-wrap break-words">{{ content }}</pre>
      </template>
      <template v-else>
        <div v-if="isPendingResult" class="text-xs opacity-70">
          工具结果接收中，完整结果到达后会自动替换为可展开预览。
        </div>
        <div v-else class="space-y-2">
          <div v-for="(item, index) in details" :key="`${item.key}-${index}`"
            class="text-xs flex flex-col sm:flex-row sm:items-start">
            <span class="detail-key font-bold mr-2 whitespace-nowrap mt-0.5">{{ item.key }}:</span>
            <div class="mt-1 sm:mt-0 flex-1 min-w-0">
              <div v-if="item.segments.some(segment => segment.type === 'media')" class="space-y-2">
                <template v-for="(segment, segmentIndex) in item.segments" :key="`${item.key}-${segmentIndex}`">
                  <MarkdownBlock v-if="segment.type === 'text' && segment.content.trim() && item.useMarkdown"
                    :content="segment.content" class="compact-markdown" />
                  <pre v-else-if="segment.type === 'text' && segment.content.trim()"
                    class="tool-plain-value text-[11px] font-mono whitespace-pre-wrap break-words">{{ segment.content }}</pre>

                  <button v-else-if="segment.type === 'media' && segment.media.type === 'image'"
                    class="tool-media-preview group block w-full overflow-hidden rounded-lg border border-black/10 dark:border-white/10 bg-black/5 dark:bg-white/5 text-left active:scale-[0.99]"
                    @click="openMedia(segment.media, item.key)">
                    <img :src="segment.media.src" class="tool-media-image" loading="lazy" decoding="async"
                      :alt="segment.media.title || item.key" />
                    <span class="tool-media-caption">
                      <Image :size="14" />
                      <span>{{ previewLabel(segment.media) }}</span>
                    </span>
                  </button>

                  <button v-else-if="segment.type === 'media' && segment.media.type === 'video'"
                    class="tool-media-preview group block w-full overflow-hidden rounded-lg border border-black/10 dark:border-white/10 bg-black/5 dark:bg-white/5 text-left active:scale-[0.99]"
                    @click="openMedia(segment.media, item.key)">
                    <span class="tool-video-frame">
                      <video :src="segment.media.src" class="tool-media-video" preload="metadata" playsinline muted />
                      <span class="tool-video-overlay">
                        <Video :size="18" />
                        <span>{{ previewLabel(segment.media) }}</span>
                      </span>
                    </span>
                  </button>

                  <button v-else-if="segment.type === 'media'"
                    class="tool-media-audio flex w-full items-center gap-2 rounded-lg border border-black/10 dark:border-white/10 bg-black/5 dark:bg-white/5 px-3 py-2 text-left active:scale-[0.99]"
                    @click="openMedia(segment.media, item.key)">
                    <component :is="mediaIcon(segment.media)" :size="15" class="shrink-0 opacity-70" />
                    <span class="min-w-0 flex-1 truncate font-bold">{{ previewLabel(segment.media) }}</span>
                    <span class="shrink-0 text-[10px] opacity-45">点击打开</span>
                  </button>
                </template>
              </div>
              <MarkdownBlock v-else-if="item.useMarkdown" :content="item.value || ''" class="compact-markdown" />
              <pre v-else class="tool-plain-value text-[11px] font-mono whitespace-pre-wrap break-words">{{ item.value }}</pre>
            </div>
          </div>
          <div v-if="block.footer" class="mt-2 pt-2 border-t border-black/10 dark:border-white/10 text-xs opacity-70">
            <MarkdownBlock :content="block.footer" class="compact-markdown" />
          </div>
        </div>
      </template>
    </div>
  </div>
</template>

<style scoped>
/* --- Animations --- */
@keyframes vcp-icon-rotate {
  0% {
    transform: rotate(0deg);
  }

  100% {
    transform: rotate(360deg);
  }
}

.animate-slide-down {
  animation: slideDown 0.18s cubic-bezier(0.4, 0, 0.2, 1);
}

@keyframes slideDown {
  from {
    opacity: 0;
    transform: translateY(-10px);
  }

  to {
    opacity: 1;
    transform: translateY(0);
  }
}

/* --- Tool Use Bubble --- */
.vcp-tool-block.is-tool-use {
  background: linear-gradient(145deg, #3a7bd5 0%, #00d2ff 100%) !important;
  color: #ffffff !important;
  border: none !important;
  position: relative;
}

.vcp-tool-block.is-tool-use .tool-header-content {
  position: relative;
  z-index: 1;
}

.vcp-tool-block.is-tool-use .tool-icon-container {
  background: transparent !important;
  color: rgba(255, 255, 255, 0.9) !important;
}

.vcp-tool-block.is-tool-use.is-running .tool-icon-container {
  animation: vcp-icon-rotate 2.5s linear infinite;
}

.vcp-tool-block.is-tool-use .tool-label {
  color: #f1c40f !important;
}

.vcp-tool-block.is-tool-use .tool-name {
  color: #ffffff !important;
}

.vcp-tool-block.is-tool-use pre {
  background-color: rgba(0, 0, 0, 0.2);
  color: #f0f0f0;
  border-radius: 6px;
  padding: 10px;
}

/* --- Tool Result Bubble (Light/Dark Theme Fixed) --- */

/* 修复：将亮色模式设为默认基础样式 */
.vcp-tool-block.is-tool-result {
  background: linear-gradient(145deg, #f4f6f8, #e8eaf0);
  border: 1px solid rgba(0, 0, 0, 0.1);
  color: #333;
}

.vcp-tool-block.is-tool-result .tool-label {
  color: #2e7d32 !important;
  /* 深一点的绿色适应亮色背景 */
}

.vcp-tool-block.is-tool-result .tool-name {
  color: #0277bd;
  background-color: rgba(2, 119, 189, 0.1);
  padding: 2px 6px;
  border-radius: 4px;
}

.vcp-tool-block.is-tool-result .tool-status {
  color: #1b5e20;
  background-color: rgba(76, 175, 80, 0.15);
}

.vcp-tool-block.is-tool-result .detail-key {
  color: #546e7a;
}

.tool-plain-value {
  margin: 0;
  padding: 8px;
  border-radius: 6px;
  background: rgba(0, 0, 0, 0.04);
  color: inherit;
}

.tool-media-preview {
  color: inherit;
  position: relative;
}

.tool-media-image {
  display: block;
  width: 100%;
  max-height: min(48vh, 420px);
  object-fit: contain;
}

.tool-media-caption {
  display: flex;
  align-items: center;
  gap: 6px;
  padding: 8px 10px;
  font-size: 11px;
  font-weight: 700;
  opacity: 0.72;
}

.tool-video-frame {
  display: block;
  position: relative;
  width: 100%;
  aspect-ratio: 16 / 9;
  background: rgba(0, 0, 0, 0.45);
}

.tool-media-video {
  width: 100%;
  height: 100%;
  object-fit: cover;
}

.tool-video-overlay {
  position: absolute;
  inset: 0;
  display: flex;
  align-items: center;
  justify-content: center;
  gap: 8px;
  background: rgba(0, 0, 0, 0.25);
  color: white;
  font-size: 12px;
  font-weight: 800;
}

.tool-media-audio {
  color: inherit;
}

/* 修复：适配 Vue/Tailwind 标准的暗黑模式选择器 */
html.dark .vcp-tool-block.is-tool-result {
  background: linear-gradient(145deg, #1c1c1e, #2c2c2e);
  border: 1px solid rgba(255, 255, 255, 0.08);
  color: #f2f2f7;
}

html.dark .vcp-tool-block.is-tool-result .tool-label {
  color: #4caf50 !important;
}

html.dark .vcp-tool-block.is-tool-result .tool-name {
  color: #64d2ff;
  background-color: rgba(100, 210, 255, 0.15);
}

html.dark .vcp-tool-block.is-tool-result .tool-status {
  color: #c8e6c9;
  background-color: rgba(76, 175, 80, 0.2);
}

html.dark .vcp-tool-block.is-tool-result .detail-key {
  color: #8e8e93;
}

/* 修复：工具内子级 Markdown 的压缩排版，去除无意义的段落边距，恢复正常换行 */
:deep(.compact-markdown p) {
  margin-top: 0 !important;
  margin-bottom: 4px !important;
}

.tool-content-scrollable {
  max-height: 400px;
  overflow-y: auto;
}
</style>
