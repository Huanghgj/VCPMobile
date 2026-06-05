<script setup lang="ts">
import { computed, ref } from "vue";
import { Brain, ChevronDown, ChevronUp, Loader2 } from "lucide-vue-next";
import { renderMarkdownNodes } from "../../../core/utils/astRenderer";
import type { ContentBlock } from "../../../core/types/chat";

const props = defineProps<{
  block: ContentBlock;
  messageId: string;
}>();

const isExpanded = ref(!props.block.is_complete);

const toggleExpand = () => {
  isExpanded.value = !isExpanded.value;
};

const title = computed(() => props.block.theme || "思考过程");

const summary = computed(() => {
  const raw = props.block.content || "";
  const compact = raw.replace(/\s+/g, " ").trim();
  if (!compact) return props.block.is_complete ? "已折叠" : "正在整理思路";
  return compact.length > 36 ? `${compact.slice(0, 36)}...` : compact;
});

function escapeHtml(text: string): string {
  return text
    .replace(/&/g, "&amp;")
    .replace(/</g, "&lt;")
    .replace(/>/g, "&gt;")
    .replace(/"/g, "&quot;")
    .replace(/'/g, "&#039;");
}
</script>

<template>
  <div class="vcp-thought-block">
    <button
      class="vcp-thought-header"
      type="button"
      data-vcp-ui-control="thought-toggle"
      @click.stop="toggleExpand"
    >
      <span class="vcp-thought-icon">
        <Brain :size="14" :stroke-width="2.2" />
      </span>
      <span class="vcp-thought-title">
        <span class="vcp-thought-label">
          {{ title }}
          <Loader2 v-if="!block.is_complete" :size="10" class="animate-spin" />
        </span>
        <span v-if="!isExpanded" class="vcp-thought-summary">{{ summary }}</span>
      </span>
      <component :is="isExpanded ? ChevronUp : ChevronDown" :size="14" class="opacity-40 ml-auto" />
    </button>

    <div v-show="isExpanded" class="vcp-thought-content animate-slide-down">
      <div
        class="thought-body"
        v-html="
          block.nodes && block.nodes.length > 0
            ? renderMarkdownNodes(block.nodes, messageId, block.hash)
            : escapeHtml(block.content || '')
        "
      />
    </div>
  </div>
</template>

<style scoped>
.vcp-thought-block {
  background: color-mix(in srgb, var(--secondary-bg) 80%, transparent) !important;
  border-radius: 10px !important;
  border: 1px solid color-mix(in srgb, var(--primary-text) 10%, transparent);
  margin: 8px 0 !important;
  position: relative;
  font-size: 0.9em !important;
  line-height: 1.5;
  width: 100%;
  max-width: 100%;
  overflow: hidden;
  transition: background-color 0.2s ease, border-color 0.2s ease;
}

html.dark .vcp-thought-block {
  background: rgba(120, 120, 128, 0.08) !important;
  border-color: rgba(120, 120, 128, 0.22);
}

.vcp-thought-header {
  appearance: none;
  border: 0;
  background: transparent;
  width: 100%;
  display: flex;
  align-items: center;
  gap: 8px;
  cursor: pointer;
  user-select: none;
  color: inherit;
  text-align: left;
  transition: opacity 0.2s;
  padding: 9px 11px !important;
}

.vcp-thought-header:hover {
  opacity: 1;
}

.vcp-thought-icon {
  width: 24px;
  height: 24px;
  border-radius: 7px;
  display: inline-flex;
  align-items: center;
  justify-content: center;
  color: #6366f1;
  background: rgba(99, 102, 241, 0.1);
  flex: 0 0 auto;
}

.vcp-thought-title {
  min-width: 0;
  display: flex;
  flex-direction: column;
  gap: 2px;
}

.vcp-thought-label {
  display: flex;
  align-items: center;
  gap: 5px;
  font-weight: 700;
  font-size: 0.86em;
}

.vcp-thought-summary {
  font-size: 0.76em;
  opacity: 0.55;
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}

.vcp-thought-content {
  padding: 0 11px 10px 43px;
  border-top: 1px dashed rgba(120, 120, 128, 0.2);
  padding-top: 9px;
}

.thought-body {
  opacity: 0.82;
  font-size: 0.9em;
  user-select: text;
  white-space: pre-wrap;
  word-break: break-word;
}

.thought-body :deep(p),
.thought-body :deep(ul),
.thought-body :deep(ol),
.thought-body :deep(blockquote) {
  white-space: normal;
}

.thought-body :deep(pre) {
  overflow-x: auto;
  white-space: pre;
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
