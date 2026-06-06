<script setup lang="ts">
import { computed } from 'vue';
import { Activity, Cpu } from 'lucide-vue-next';
import { useNotificationStore } from '../../core/stores/notification';

const store = useNotificationStore();

const getStatusColor = (status: string) => {
  switch (status) {
    case 'connected':
    case 'ready':
    case 'open':
      return 'bg-emerald-500';
    case 'connecting':
    case 'initializing':
      return 'bg-amber-500';
    case 'error':
    case 'disconnected':
      return 'bg-rose-500';
    case 'closed':
    default:
      return 'bg-slate-400';
  }
};

const vcpStatusText = computed(() => store.vcpStatus.message || store.vcpStatus.status || 'Offline');
const coreStatusText = computed(() => store.vcpCoreStatus.message || store.vcpCoreStatus.status || 'Offline');
</script>

<template>
  <div class="bg-white/80 border-b border-pink-100/60 px-4 py-2.5 flex flex-col gap-2 text-[11px] select-none shadow-sm">
    <div class="flex items-center justify-between text-slate-500">
      <div class="flex items-center gap-3 min-w-0">
        <div class="flex items-center gap-1.5 min-w-0">
          <Activity class="w-3.5 h-3.5 text-pink-400 shrink-0" />
          <span class="text-slate-400 font-medium shrink-0">VCP:</span>
          <span :class="['w-1.5 h-1.5 rounded-full shrink-0', getStatusColor(store.vcpStatus.status)]"></span>
          <span class="text-slate-700 font-mono font-semibold truncate max-w-[88px]">{{ vcpStatusText }}</span>
        </div>
        <div class="flex items-center gap-1.5 min-w-0">
          <Cpu class="w-3.5 h-3.5 text-pink-400 shrink-0" />
          <span class="text-slate-400 font-medium shrink-0">Core:</span>
          <span :class="['w-1.5 h-1.5 rounded-full shrink-0', getStatusColor(store.vcpCoreStatus.status)]"></span>
          <span class="text-slate-700 font-mono font-semibold truncate max-w-[88px]">{{ coreStatusText }}</span>
        </div>
      </div>
      <div class="text-slate-400 font-mono text-[10px] shrink-0">
        未读:
        <span class="text-pink-500 font-bold">{{ store.unreadCount }}</span>
        / {{ store.historyList.length }}
      </div>
    </div>
  </div>
</template>
