<template>
  <AttachmentPreviewBase
    :file="file"
    :index="index"
    :size="size"
    :show-remove="showRemove"
    @remove="emit('remove', index)"
  >
    <!-- Image Card -->
    <div class="w-full h-full rounded-xl overflow-hidden bg-black/5 dark:bg-white/5">
      <img
        :src="safeSrc"
        :alt="file.name"
        class="w-full h-full object-cover"
        loading="lazy"
        decoding="async"
      />
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

interface Props {
  file: Attachment;
  index: number;
  size?: 'small' | 'medium' | 'large';
  showRemove?: boolean;
}

const props = withDefaults(defineProps<Props>(), {
  size: 'medium',
  showRemove: false
});

const emit = defineEmits<{ (e: "remove", index: number): void }>();
const cachedSrc = ref("");
let warmPreviewRunId = 0;

const safeSrc = computed(() => {
  return cachedSrc.value || getAttachmentPreviewSrc(props.file, { allowOriginal: true });
});

const warmPreview = async () => {
  const runId = ++warmPreviewRunId;
  cachedSrc.value = getAttachmentPreviewSrc(props.file, { allowOriginal: true });
  const src = await ensureAttachmentThumbnailCached(props.file, { allowOriginal: true });
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
