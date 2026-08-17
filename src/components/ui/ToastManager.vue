<script setup lang="ts">
import { useNotificationStore } from '../../core/stores/notification';
import ToastItem from './ToastItem.vue';

const store = useNotificationStore();
</script>

<template>
  <div class="vcp-toast-stack fixed left-0 right-0 z-toast pointer-events-none px-4 flex flex-col items-center gap-2.5">
    <TransitionGroup name="toast">
      <ToastItem v-for="toast in store.activeToasts" :key="toast.id" :toast="toast" />
    </TransitionGroup>
  </div>
</template>

<style scoped>
.vcp-toast-stack {
  top: calc(var(--vcp-safe-top, env(safe-area-inset-top, 0px)) + 16px);
}

:global(html.vcp-android-runtime) .vcp-toast-stack {
  top: calc(var(--vcp-safe-top, env(safe-area-inset-top, 0px)) + 12px);
}

.toast-enter-active {
  transition: all 0.3s cubic-bezier(0.16, 1, 0.3, 1);
}

.toast-leave-active {
  transition: all 0.25s ease-out;
}

.toast-enter-from {
  opacity: 0;
  transform: translateY(-16px);
}

.toast-leave-to {
  opacity: 0;
  transform: translateY(-12px);
}

.toast-move {
  transition: transform 0.3s ease;
}
</style>
