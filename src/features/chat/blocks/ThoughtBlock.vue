<script setup lang="ts">
import { computed, ref } from 'vue';
import { ChevronDown, ChevronUp, Loader2 } from 'lucide-vue-next';
import MarkdownBlock from './MarkdownBlock.vue';
import type { ContentBlock } from '../../../core/composables/useContentProcessor';

const props = defineProps<{
  content: string;
  block: ContentBlock;
  isStreaming?: boolean;
}>();

const isExpanded = ref(false);

const toggleExpand = () => {
  isExpanded.value = !isExpanded.value;
};

const stateText = computed(() => {
  if (props.block.is_complete === false) return '生成中';
  return isExpanded.value ? '已展开' : '已折叠';
});
</script>

<template>
  <div class="vcp-thought-block">
    <div class="thought-header" role="button" :aria-expanded="isExpanded" tabindex="0" @click="toggleExpand"
      @keydown.enter.prevent="toggleExpand" @keydown.space.prevent="toggleExpand">
      <span class="thought-icon">🧠</span>
      <span class="thought-label flex items-center gap-1">
        {{ block.theme || '元思考链' }}
        <Loader2 v-if="block.is_complete === false" :size="10" class="animate-spin" />
      </span>
      <span class="thought-state">{{ stateText }}</span>
      <component :is="isExpanded ? ChevronUp : ChevronDown" :size="14" class="opacity-40 ml-auto" />
    </div>

    <div v-if="isExpanded" class="thought-content animate-slide-down">
      <div class="thought-body">
        <MarkdownBlock :content="content" :is-streaming="isStreaming" />
      </div>
    </div>
  </div>
</template>

<style scoped>
.vcp-thought-block {
  background: rgba(0, 0, 0, 0.03) !important;
  border-radius: 12px !important;
  border: 1px solid rgba(0, 0, 0, 0.1);
  margin: 10px 0 !important;
  position: relative;
  font-size: 0.92em !important;
  line-height: 1.6;
  width: fit-content;
  max-width: 98%;
  transition: all 0.3s ease;
}

html.dark .vcp-thought-block {
  background: rgba(120, 120, 128, 0.05) !important;
  border-color: rgba(120, 120, 128, 0.2);
}

.thought-header {
  display: flex;
  align-items: center;
  gap: 8px;
  cursor: pointer;
  user-select: none;
  opacity: 0.8;
  transition: opacity 0.2s;
  padding: 10px 15px !important;
}

.thought-header:hover {
  opacity: 1;
}

.thought-icon {
  font-size: 1.1em;
  filter: grayscale(0.5);
}

.thought-label {
  font-weight: 600;
  font-size: 0.95em;
}

.thought-state {
  border-radius: 999px;
  border: 1px solid rgba(120, 120, 128, 0.22);
  font-size: 0.72em;
  line-height: 1;
  opacity: 0.58;
  padding: 3px 7px;
}

.thought-content {
  padding: 0 15px 10px 15px;
  border-top: 1px dashed rgba(120, 120, 128, 0.2);
  margin-top: 5px;
  padding-top: 10px;
}

.thought-body {
  font-style: italic;
  opacity: 0.8;
}

.animate-slide-down {
  animation: slideDown 0.3s cubic-bezier(0.4, 0, 0.2, 1);
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
</style>
