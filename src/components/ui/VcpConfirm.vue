<script setup lang="ts">
defineOptions({
  inheritAttrs: false
});
defineProps<{
  title: string;
  message: string;
  isOpen: boolean;
  isDanger?: boolean;
  onlyConfirm?: boolean;
}>();

const emit = defineEmits<{
  (e: 'update:isOpen', value: boolean): void;
  (e: 'confirm'): void;
  (e: 'cancel'): void;
}>();

const handleConfirm = () => {
  emit('confirm');
  emit('update:isOpen', false);
};

const handleCancel = () => {
  emit('cancel');
  emit('update:isOpen', false);
};
</script>

<template>
  <Teleport to="body">
    <Transition name="fade">
      <div v-if="isOpen" v-bind="$attrs"
        class="fixed inset-0 z-dialog flex items-start justify-center pt-[15vh] bg-black/50"
        @click.self="onlyConfirm ? handleConfirm() : handleCancel()">
        <div
          class="vcp-confirm-modal bg-white dark:bg-zinc-900 w-11/12 max-w-sm rounded-xl shadow-xl border border-black/10 dark:border-white/10 p-5 transform transition-all relative overflow-hidden">
          <h3 class="text-base font-bold text-gray-900 dark:text-zinc-100 mb-2">{{ title }}</h3>
          <p class="text-xs text-gray-600 dark:text-gray-400 mb-5 leading-relaxed whitespace-pre-wrap text-left">{{ message }}</p>

          <div class="flex justify-end gap-2.5">
            <button v-if="!onlyConfirm" @click="handleCancel"
              class="px-4 py-2 rounded-lg text-xs font-semibold text-gray-600 dark:text-gray-400 hover:bg-black/5 dark:hover:bg-white/5 transition-colors active:opacity-80 active:scale-[0.98]">
              取消
            </button>
            <button @click="handleConfirm"
              class="px-4 py-2 rounded-lg text-xs font-semibold text-white shadow-md transition-all active:opacity-85 active:scale-[0.98]"
              :class="isDanger ? 'bg-danger hover:opacity-90' : 'bg-blue-600 hover:bg-blue-500'">
              确认
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

.fade-enter-active .vcp-confirm-modal {
  transition: all 0.25s cubic-bezier(0.16, 1, 0.3, 1);
}

.fade-leave-active .vcp-confirm-modal {
  transition: all 0.2s ease-out;
}

.fade-enter-from .vcp-confirm-modal,
.fade-leave-to .vcp-confirm-modal {
  transform: scale(0.98) translateY(6px);
  opacity: 0;
}
</style>
