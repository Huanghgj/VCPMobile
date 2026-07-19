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
        v-if="safeSrc"
        :src="safeSrc" 
        :alt="file.name"
        class="w-full h-full object-cover"
        loading="lazy"
        decoding="async"
        @error="tryNextSource"
      />
      <div
        v-else
        class="w-full h-full px-2 flex items-center justify-center text-center text-[10px] text-black/45 dark:text-white/45 break-all"
      >
        {{ file.name }}
      </div>
    </div>
  </AttachmentPreviewBase>
</template>

<script setup lang="ts">
import { computed, ref, watch } from "vue";
import { convertFileSrc } from "@tauri-apps/api/core";
import AttachmentPreviewBase from "../AttachmentPreviewBase.vue";
import type { Attachment } from "../../../../core/types/chat";

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

const normalizeSource = (src?: string) => {
  if (!src) return "";
  if (
    src.startsWith("http:") ||
    src.startsWith("https:") ||
    src.startsWith("data:") ||
    src.startsWith("blob:") ||
    src.startsWith("asset:") ||
    src.startsWith("tauri:")
  ) {
    return src;
  }
  try {
    return convertFileSrc(src.replace("file://", ""));
  } catch {
    return "";
  }
};

const sourceCandidates = computed(() => {
  const candidates = [
    props.file.thumbnailPath,
    props.file.resolvedSrc,
    props.file.internalPath,
    props.file.src,
  ]
    .map(normalizeSource)
    .filter(Boolean);

  return [...new Set(candidates)];
});

const candidateIndex = ref(0);
const candidateSignature = computed(() => sourceCandidates.value.join("\n"));

watch(candidateSignature, () => {
  candidateIndex.value = 0;
});

const safeSrc = computed(() => {
  return sourceCandidates.value[candidateIndex.value] || "";
});

const tryNextSource = () => {
  candidateIndex.value += 1;
};
</script>
