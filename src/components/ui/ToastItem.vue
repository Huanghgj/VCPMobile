<script setup lang="ts">
import { computed, ref } from 'vue';
import { useSwipe } from '@vueuse/core';
import { Info, CheckCircle, AlertTriangle, X, Cpu, User } from 'lucide-vue-next';
import { useNotificationStore, type VcpNotification } from '../../core/stores/notification';

const props = defineProps<{
  toast: VcpNotification;
}>();

const store = useNotificationStore();
const isActionRunning = ref(false);

const getIcon = (type: string) => {
  switch (type) {
    case 'success': return CheckCircle;
    case 'warning': return AlertTriangle;
    case 'error': return X;
    case 'tool': return Cpu;
    case 'agent': return User;
    default: return Info;
  }
};

const getIconColor = (type: string) => {
  switch (type) {
    case 'success': return 'text-green-500';
    case 'warning': return 'text-amber-500';
    case 'error': return 'text-red-500';
    case 'tool': return 'text-purple-500';
    case 'agent': return 'text-blue-500';
    default: return 'text-blue-400';
  }
};

const dismissToast = (id: string) => {
  store.activeToasts = store.activeToasts.filter((t: VcpNotification) => t.id !== id);
};

const el = ref<HTMLElement | null>(null);
const { isSwiping, lengthX } = useSwipe(el, {
  onSwipeEnd(_, direction) {
    if ((direction === 'left' || direction === 'right') && Math.abs(lengthX.value) > 60) {
      dismissToast(props.toast.id);
    }
  },
});

const swipeStyle = computed(() => {
  if (isSwiping.value) {
    const opacity = Math.max(0, 1 - Math.abs(lengthX.value) / 200);
    return {
      transform: `translateX(${-lengthX.value}px)`,
      opacity,
      transition: 'none' // 正在滑动时禁用过渡，保证实时触控跟随
    };
  }
  // 松手/不在滑动时，平滑回弹归位，呈现 iOS 级别高档物理阻尼动效
  return {
    transform: 'translateX(0px)',
    opacity: 1,
    transition: 'transform 0.25s cubic-bezier(0.16, 1, 0.3, 1), opacity 0.25s ease-out'
  };
});

const handleClick = () => {
  if (props.toast.actions?.length) return;
  dismissToast(props.toast.id);
};

const handleAction = async (action: NonNullable<VcpNotification['actions']>[number]) => {
  if (isActionRunning.value) return;
  isActionRunning.value = true;
  try {
    await store.executeAction(props.toast.id, action);
  } finally {
    isActionRunning.value = false;
  }
};
</script>

<template>
  <div
    ref="el"
    class="pointer-events-auto flex items-center justify-between gap-3 px-3.5 py-2.5 rounded-xl bg-white/90 dark:bg-zinc-900/90 backdrop-blur-md border border-black/5 dark:border-white/10 shadow-[0_8px_30px_rgba(0,0,0,0.12)] w-full max-w-[calc(100vw-32px)] sm:w-[320px] overflow-hidden transition-all active:scale-[0.98] cursor-pointer touch-none select-none no-swipe"
    :style="swipeStyle"
    @click="handleClick"
  >
    <div class="flex items-start gap-3 min-w-0 flex-1">
      <component :is="getIcon(toast.type)" :size="14" :class="getIconColor(toast.type)" class="mt-0.5 shrink-0 opacity-80" />
      <div class="flex flex-col min-w-0 flex-1">
        <span class="text-[11px] font-bold text-primary-text leading-tight tracking-wide truncate">{{ toast.title }}</span>
        <span v-if="toast.subtitle" class="text-[9px] text-primary-text opacity-40 leading-snug truncate mt-0.5">
          {{ toast.subtitle }}
        </span>
        <div v-if="toast.tags && toast.tags.length > 0" class="flex flex-wrap gap-1 mt-1 max-h-[36px] overflow-y-auto pr-1">
          <span v-for="(tag, idx) in toast.tags" :key="`${tag}-${idx}`"
            class="text-[7.5px] font-bold px-1.5 py-0.5 rounded bg-blue-500/10 text-blue-500/90 shrink-0">
            {{ tag }}
          </span>
        </div>

        <div v-if="toast.isPreformatted"
          class="mt-1 p-1.5 bg-black/[0.04] dark:bg-black/25 rounded text-[8px] max-h-[60px] overflow-y-auto whitespace-pre-wrap break-all font-mono opacity-60 text-primary-text leading-normal select-text"
          @click.stop
          @mousedown.stop>
          {{ toast.message }}
        </div>
        <p v-else-if="toast.message" class="text-[9.5px] text-primary-text opacity-50 break-words mt-0.5 leading-snug select-text"
          @click.stop
          @mousedown.stop>
          {{ toast.message }}
        </p>

        <div v-if="toast.actions && toast.actions.length > 0" class="mt-2 grid grid-cols-2 gap-1.5">
          <button
            v-for="act in toast.actions"
            :key="act.label"
            class="rounded-md px-2 py-1.5 text-[10px] font-bold text-white shadow-sm active:scale-95 disabled:opacity-60"
            :class="act.color"
            :disabled="isActionRunning"
            @click.stop="handleAction(act)"
          >
            {{ act.label }}
          </button>
        </div>
      </div>
    </div>

    <button @click.stop="dismissToast(toast.id)"
      class="p-1 opacity-20 hover:opacity-100 text-primary-text transition-opacity shrink-0 ml-1 self-start">
      <X :size="12" />
    </button>
  </div>
</template>
