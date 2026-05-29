<script setup lang="ts">
import { computed } from 'vue';
import {
  CheckCheck,
  Trash2,
  Cpu,
  Activity
} from 'lucide-vue-next';

interface StatusObj {
  status: 'open'|'closed'|'error'|'connecting'|'connected'|'disconnected'|'ready'|'initializing';
  message: string;
  source: string;
}

const props = defineProps<{
  vcpStatus: StatusObj;
  vcpCoreStatus: StatusObj;
  unreadCount: number;
  totalCount: number;
}>();

const emit = defineEmits<{
  (e: 'markAllRead'): void;
  (e: 'clearHistory'): void;
}>();

const getStatusColor = (status: string) => {
  switch (status) {
    case 'connected':
    case 'ready':
    case 'open':
      return 'bg-emerald-500 shadow-emerald-200';
    case 'connecting':
    case 'initializing':
      return 'bg-amber-500 shadow-amber-200';
    case 'error':
    case 'disconnected':
      return 'bg-rose-500 shadow-rose-200';
    case 'closed':
    default:
      return 'bg-slate-400';
  }
};

const vcpStatusText = computed(() => props.vcpStatus.message || props.vcpStatus.status || 'Offline');
const coreStatusText = computed(() => props.vcpCoreStatus.message || props.vcpCoreStatus.status || 'Offline');
</script>

<template>
  <div class="bg-white/80 border-b border-pink-100/60 px-4 py-2.5 flex flex-col gap-2 text-[11px] select-none shadow-sm">
    <!-- Status Indicators -->
    <div class="flex items-center justify-between text-slate-500">
      <div class="flex items-center gap-3">
        <div class="flex items-center gap-1.5">
          <Activity class="w-3.5 h-3.5 text-pink-400" />
          <span class="text-slate-400 font-medium">VCP:</span>
          <span :class="['w-1.5 h-1.5 rounded-full shadow-sm', getStatusColor(vcpStatus.status)]"></span>
          <span class="text-slate-700 font-mono font-semibold truncate max-w-[80px]">{{ vcpStatusText }}</span>
        </div>
        <div class="flex items-center gap-1.5">
          <Cpu class="w-3.5 h-3.5 text-pink-400" />
          <span class="text-slate-400 font-medium">Core:</span>
          <span :class="['w-1.5 h-1.5 rounded-full shadow-sm', getStatusColor(vcpCoreStatus.status)]"></span>
          <span class="text-slate-700 font-mono font-semibold truncate max-w-[80px]">{{ coreStatusText }}</span>
        </div>
      </div>
      <div class="text-slate-400 font-mono text-[10px]">
        未读: <span class="text-pink-500 font-bold">{{ unreadCount }}</span> / {{ totalCount }}
      </div>
    </div>

    <!-- Quick Actions -->
    <div class="flex items-center justify-between mt-1 pt-2 border-t border-pink-100/40">
      <div class="text-pink-400 uppercase tracking-wider font-bold text-[8px] font-mono">
        System Operations
      </div>
      <div class="flex items-center gap-2">
        <button
          @click="emit('markAllRead')"
          class="flex items-center gap-1 px-2.5 py-1 rounded-lg bg-pink-50 hover:bg-pink-100 text-pink-600 border border-pink-100/60 transition-all duration-150 active:scale-95 motion-reduce:transition-none font-medium"
        >
          <CheckCheck class="w-3 h-3 text-pink-500" />
          <span>已读全部</span>
        </button>
        <button
          @click="emit('clearHistory')"
          class="flex items-center gap-1 px-2.5 py-1 rounded-lg bg-slate-50 hover:bg-rose-50 text-slate-500 hover:text-rose-600 border border-slate-100 hover:border-rose-100 transition-all duration-150 active:scale-95 motion-reduce:transition-none font-medium"
        >
          <Trash2 class="w-3 h-3" />
          <span>清空</span>
        </button>
      </div>
    </div>
  </div>
</template>