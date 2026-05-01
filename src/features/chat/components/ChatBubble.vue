<script setup lang="ts">
defineProps<{
  isUser: boolean;
  isStreaming: boolean;
  bubbleStyle?: Record<string, string>;
}>();
</script>

<template>
  <div class="w-full min-w-0 flex flex-col" :class="[
    isUser ? 'items-end' : 'items-start',
    isStreaming ? 'streaming' : '',
  ]">
    <div
      class="vcp-bubble-container rounded-2xl transition-colors duration-200 relative min-w-[60px] min-h-[36px]"
      :class="isUser
          ? 'p-3 w-fit max-w-[85%]'
          : 'p-1.5 w-fit max-w-[100%]'
        " :style="bubbleStyle">
      <slot />
    </div>

    <slot name="footer" />
  </div>
</template>

<style scoped>
.vcp-bubble-container {
  word-break: break-word;
}

.streaming .vcp-bubble-container::before {
  content: "";
  position: absolute;
  inset: -1px;
  border-radius: inherit;
  border: 1px solid var(--highlight-text, #3b82f6);
  pointer-events: none;
  z-index: 1;
  opacity: 0.45;
}
</style>
