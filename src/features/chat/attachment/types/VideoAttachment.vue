<template>
  <AttachmentPreviewBase
    :file="file"
    :index="index"
    :show-remove="showRemove"
    size="auto"
    @remove="emit('remove', index)"
  >
    <div class="relative w-full h-full">
      <!-- Video Thumbnail -->
      <div class="w-full h-full rounded-xl overflow-hidden bg-black/5 dark:bg-white/5">
        <img
          :src="thumbnailSrc"
          :alt="file.name"
          class="w-full h-full object-cover"
          loading="lazy"
          decoding="async"
        />
        <!-- Video Play Button -->
        <div class="absolute inset-0 flex items-center justify-center">
          <div class="w-8 h-8 rounded-full bg-blue-500/20 backdrop-blur-md flex items-center justify-center border border-blue-500/30">
            <svg
              width="14"
              height="14"
              viewBox="0 0 24 24"
              fill="none"
              stroke="currentColor"
              stroke-width="2.5"
              stroke-linecap="round"
              stroke-linejoin="round"
              class="text-blue-500"
            >
              <polygon points="23 7 16 12 23 17 23 7"></polygon>
              <rect x="1" y="5" width="15" height="14" rx="2" ry="2"></rect>
            </svg>
          </div>
        </div>
      </div>
    </div>
  </AttachmentPreviewBase>
</template>

<script setup lang="ts">
import { computed, onMounted, ref, watch } from "vue";
import AttachmentPreviewBase from "../AttachmentPreviewBase.vue";
import type { Attachment } from "../../../../core/stores/chatManager";
import {
  ensureAttachmentThumbnailCached,
  getAttachmentPreviewSrc,
} from "../../../../core/utils/mediaCache";

const props = withDefaults(defineProps<{
  file: Attachment;
  index: number;
  showRemove?: boolean;
}>(), {
  showRemove: false
});

const emit = defineEmits<{ (e: "remove", index: number): void }>();
const cachedSrc = ref("");
let warmPreviewRunId = 0;

const thumbnailSrc = computed(() => {
  return cachedSrc.value || getAttachmentPreviewSrc(props.file, { thumbnailOnly: true });
});

const warmPreview = async () => {
  const runId = ++warmPreviewRunId;
  cachedSrc.value = getAttachmentPreviewSrc(props.file, { thumbnailOnly: true });
  const src = await ensureAttachmentThumbnailCached(props.file, { thumbnailOnly: true });
  if (src && runId === warmPreviewRunId) cachedSrc.value = src;
};

onMounted(() => {
  void warmPreview();
});

watch(
  () => [props.file.thumbnailPath, props.file.src, props.file.internalPath, props.file.hash, props.file.size],
  () => {
    void warmPreview();
  },
);
</script>
