<script setup lang="ts">
import type { OverlayActionItem } from '../../core/types/overlay';

defineProps<{
  isOpen: boolean;
  title?: string;
  actions: OverlayActionItem[];
}>();

const emit = defineEmits(['close', 'action-click']);

const handleBackdropClick = () => {
  emit('close');
};

const handleAction = (action: OverlayActionItem) => {
  if (action.disabled) return;
  action.handler();
  emit('action-click', action);
};
</script>

<template>
  <Teleport to="body">
    <Transition name="fade">
      <div v-if="isOpen" class="fixed inset-0 bg-black/40 pointer-events-auto z-dialog"
        @click="handleBackdropClick">
        <div
          class="absolute left-1/2 -translate-x-1/2 w-[calc(100%-24px)] max-w-sm rounded-2xl border border-black/10 dark:border-white/10 bg-white dark:bg-zinc-900 shadow-xl overflow-hidden"
          :style="{ bottom: 'calc(var(--vcp-safe-bottom, 48px) + 24px)' }"
          @click.stop>
          <div v-if="title" class="px-4 pt-4 pb-2.5 border-b border-black/5 dark:border-white/10">
            <h3 class="text-xs font-bold uppercase tracking-wider text-gray-500 dark:text-gray-400">{{ title }}</h3>
          </div>
          <div class="p-1.5 space-y-0.5">
            <button v-for="action in actions" :key="action.label" @click="handleAction(action)"
              :disabled="action.disabled"
              class="w-full flex items-center gap-3 px-3.5 py-2.5 rounded-xl text-left transition-all active:opacity-80 active:scale-[0.99]" :class="[
                action.danger ? 'text-red-500 hover:bg-red-500/10' : 'hover:bg-black/5 dark:hover:bg-white/5',
                action.disabled ? 'opacity-40 cursor-not-allowed' : ''
              ]">
              <component v-if="action.icon" :is="action.icon" class="w-4 h-4 shrink-0" />
              <span class="text-sm font-semibold">{{ action.label }}</span>
            </button>
          </div>
        </div>
      </div>
    </Transition>
  </Teleport>
</template>

<style scoped>
.fade-enter-active,
.fade-leave-active {
  transition: opacity 0.25s ease;
}

.fade-enter-from,
.fade-leave-to {
  opacity: 0;
}
</style>
