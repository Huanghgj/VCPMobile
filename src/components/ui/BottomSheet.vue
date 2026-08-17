<script setup lang="ts">
import { watch } from 'vue';
import { useModalHistory } from '../../core/composables/useModalHistory';

export interface ActionItem {
  label: string;
  icon?: any; // lucide-vue-next component
  danger?: boolean;
  disabled?: boolean;
  handler: () => void;
}

const props = defineProps<{
  modelValue: boolean;
  title?: string;
  actions: ActionItem[];
}>();

const emit = defineEmits(['update:modelValue']);

const { registerModal, unregisterModal } = useModalHistory();
const modalId = 'BottomSheet';

watch(() => props.modelValue, (newVal) => {
  if (newVal) {
    registerModal(modalId, () => {
      emit('update:modelValue', false);
    });
  } else {
    unregisterModal(modalId);
  }
});

const close = () => {
  emit('update:modelValue', false);
};

const handleAction = (action: ActionItem) => {
  if (action.disabled) return;
  action.handler();
  close();
};
</script>

<template>
  <Teleport to="body">
    <!-- 遮罩层 -->
    <Transition name="fade">
      <div v-if="modelValue" class="fixed inset-0 bg-black/50 z-sheet" @click="close"
        @touchmove.prevent>
      </div>
    </Transition>

    <!-- 抽屉内容 -->
    <Transition name="slide-up">
      <div v-if="modelValue"
        class="fixed bottom-0 left-0 right-0 z-sheet bg-white dark:bg-zinc-900 rounded-t-2xl shadow-xl p-4 flex flex-col border-t border-black/10 dark:border-white/10"
        style="padding-bottom: calc(var(--vcp-safe-bottom, 20px) + 16px);">

        <!-- 顶部拉手条 -->
        <div class="w-10 h-1 bg-black/10 dark:bg-white/20 rounded-full mx-auto mb-3"></div>

        <!-- 标题 -->
        <div v-if="title" class="text-[10px] font-bold text-center text-gray-400 uppercase tracking-[0.15em] mb-3">
          {{ title }}
        </div>

        <!-- 操作项列表 -->
        <div class="flex flex-col gap-1.5 px-1">
          <button v-for="(action, index) in actions" :key="index" @click="handleAction(action)"
            :disabled="action.disabled"
            class="flex items-center justify-start px-4 py-3 rounded-xl active:opacity-80 active:scale-[0.99] transition-all text-[15px] font-bold border"
            :class="[
              action.danger
                ? 'bg-red-50 dark:bg-red-500/10 text-red-600 dark:text-red-400 border-red-100 dark:border-red-500/20 shadow-sm shadow-red-500/10'
                : 'bg-black/5 dark:bg-white/5 text-gray-800 dark:text-gray-200 border-transparent hover:bg-black/10 dark:hover:bg-white/10 shadow-sm',
              action.disabled ? 'opacity-40 cursor-not-allowed' : ''
            ]">
            <component v-if="action.icon" :is="action.icon" :size="18" class="mr-3.5 opacity-90"
              :class="action.danger ? 'text-red-500' : 'text-blue-500/80 dark:text-blue-400/80'" />
            <span class="tracking-tight">{{ action.label }}</span>
          </button>

          <!-- 取消按钮 -->
          <button @click="close"
            class="mt-2 py-3 rounded-xl text-[15px] font-medium bg-black/5 dark:bg-white/5 text-gray-500 dark:text-gray-400 active:opacity-80 active:scale-[0.99] transition-all border border-transparent flex items-center justify-center">
            取消
          </button>
        </div>
      </div>
    </Transition>
  </Teleport>
</template>

<style scoped>
.fade-enter-active,
.fade-leave-active {
  transition: opacity 0.3s cubic-bezier(0.16, 1, 0.3, 1);
}

.fade-enter-from,
.fade-leave-to {
  opacity: 0;
}

.slide-up-enter-active,
.slide-up-leave-active {
  transition: transform 0.4s cubic-bezier(0.16, 1, 0.3, 1);
}

.slide-up-enter-from,
.slide-up-leave-to {
  transform: translateY(100%);
}
</style>
