<script setup lang="ts">
import { computed } from 'vue';
import { Inbox } from 'lucide-vue-next';
import { format, isToday, isYesterday } from 'date-fns';
import type { VcpNotification } from '../../core/stores/notification';
import { useNotificationStore } from '../../core/stores/notification';
import NotificationCard from './NotificationCard.vue';

const props = defineProps<{
  items: VcpNotification[];
}>();

interface GroupedNotifications {
  title: string;
  items: VcpNotification[];
}

const store = useNotificationStore();

const groupedNotifications = computed<GroupedNotifications[]>(() => {
  const groups: Record<string, VcpNotification[]> = {};

  props.items.forEach((item) => {
    const date = new Date(item.timestamp);
    let groupKey = '';

    if (isToday(date)) {
      groupKey = '今天';
    } else if (isYesterday(date)) {
      groupKey = '昨天';
    } else {
      groupKey = format(date, 'yyyy-MM-dd');
    }

    if (!groups[groupKey]) {
      groups[groupKey] = [];
    }
    groups[groupKey].push(item);
  });

  return Object.keys(groups)
    .map((key) => ({
      title: key,
      items: groups[key].sort((a, b) => b.timestamp - a.timestamp),
    }))
    .sort((a, b) => b.items[0].timestamp - a.items[0].timestamp);
});
</script>

<template>
  <div class="flex-1 overflow-y-auto overflow-x-hidden vcp-scrollable no-rubber-band bg-transparent pb-6">
    <div
      v-if="props.items.length === 0"
      class="h-full flex flex-col items-center justify-center p-8 text-center select-none"
    >
      <div class="w-12 h-12 rounded-full bg-pink-50 flex items-center justify-center border border-pink-100/60 mb-3">
        <Inbox class="w-5 h-5 text-pink-400" />
      </div>
      <p class="text-xs text-slate-500 font-mono font-bold tracking-wider">NO NOTIFICATIONS</p>
      <p class="text-[10px] text-slate-400 mt-1">当前分类下无系统通知</p>
    </div>

    <TransitionGroup v-else name="list" tag="div" class="space-y-3">
      <div v-for="group in groupedNotifications" :key="group.title" class="flex flex-col">
        <div class="px-4 py-1.5 text-[9px] font-mono font-bold text-pink-500/80 tracking-wider sticky top-0 z-20 bg-[#fff5f7] flex items-center gap-1.5">
          <span class="w-1 h-2.5 rounded-full bg-pink-400"></span>
          <span>{{ group.title }}</span>
        </div>

        <div class="mt-1">
          <NotificationCard
            v-for="item in group.items"
            :key="item.id"
            :item="item"
            @delete="store.removeHistoryItem($event)"
            @action="store.executeAction($event.id, $event.action)"
          />
        </div>
      </div>
    </TransitionGroup>
  </div>
</template>

<style scoped>
.list-enter-active,
.list-leave-active {
  transition: all 0.25s cubic-bezier(0.3, 0, 0.2, 1);
}

.list-enter-from {
  opacity: 0;
  transform: translateX(24px);
}

.list-leave-to {
  opacity: 0;
  transform: translateX(80px) scale(0.96);
}
</style>
