<script setup lang="ts">
import { computed } from "vue";
import { renderSafeMarkdown } from "../../core/utils/safeMarkdown";

interface Props {
  text: string;
  isQuery?: boolean;
}

const props = withDefaults(defineProps<Props>(), {
  isQuery: false,
});

const renderedHtml = computed(() => {
  let rawText = props.text || "";

  if (props.isQuery) {
    // 转义特殊 HTML 字符，防止在 v-html 渲染 query/response 文本时因浏览器误判 <Tauri> 等标签而吞字
    rawText = rawText.replace(/</g, "&lt;").replace(/>/g, "&gt;");
  }

  // 修复 Markdown 引擎将 "[AI]:" 或 "[USER]:" 识别为隐藏链接定义（Link Reference Definition）从而吞字的 Bug
  const safeText = rawText.replace(/^(\s*)\[([^\]]+)\]:/gm, "$1\\[$2\\]:");
  return renderSafeMarkdown(safeText);
});
</script>

<template>
  <div v-html="renderedHtml"></div>
</template>
