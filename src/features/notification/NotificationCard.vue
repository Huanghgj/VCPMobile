<script setup lang="ts">
import { computed, ref } from 'vue';
import type { VcpNotification } from '../../core/stores/notification';
import { format } from 'date-fns';
import { useNotificationStore } from '../../core/stores/notification';
import { useNotificationPresentation } from './composables/useNotificationPresentation';
import RagInsightDetails from './RagInsightDetails.vue';

const props = defineProps<{
  item: VcpNotification;
  copyIcon: any;
}>();

defineEmits<{
  copy: [];
}>();

const store = useNotificationStore();
const expanded = ref(false);
const { getIcon, getTypeColor, getActionButtonClass } = useNotificationPresentation();

const hasStructuredDetails = computed(() => !!props.item.observerKind && !!props.item.rawPayload);
const hasLongContent = computed(() => hasStructuredDetails.value || props.item.message.length > 220 || !!props.item.rawPayload?.notes?.length);
const visibleMessage = computed(() => {
  if (expanded.value || !hasLongContent.value) return props.item.message;
  return `${props.item.message.slice(0, 220)}...`;
});
const rawNotes = computed(() => Array.isArray(props.item.rawPayload?.notes) ? props.item.rawPayload.notes : []);

const handleAction = (action: { label: string; value: boolean; color: string }) => {
  store.executeAction(props.item.id, action);
};
</script>

<template>
  <div
    class="group relative p-4 rounded-2xl border border-white/5 hover:bg-white/10 transition-all bg-white/5">
    <div class="flex gap-3">
      <component :is="getIcon(props.item.type)" :size="16" :class="getTypeColor(props.item.type)"
        class="mt-0.5 shrink-0" />
      <div class="flex-1 min-w-0">
        <div class="flex justify-between items-start mb-1">
          <span class="text-[11px] font-bold opacity-90 truncate pr-2 text-primary-text">{{ props.item.title }}</span>
          <span class="text-[9px] font-mono opacity-30 whitespace-nowrap text-primary-text">{{
            format(props.item.timestamp, 'HH:mm:ss') }}</span>
        </div>

        <div v-if="props.item.isPreformatted"
          class="bg-black/20 p-1.5 rounded text-[0.85em] mt-1.5 whitespace-pre-wrap break-all font-mono opacity-90 text-primary-text"
          :class="expanded ? 'max-h-[70vh] overflow-y-auto' : 'max-h-[180px] overflow-y-auto'">
          {{ visibleMessage }}
        </div>
        <div v-else class="text-[12px] leading-relaxed break-words text-primary-text opacity-60">
          {{ visibleMessage }}
        </div>

        <RagInsightDetails v-if="expanded && hasStructuredDetails" :payload="props.item.rawPayload"
          :kind="props.item.observerKind" />



        <div v-if="rawNotes.length > 0 && expanded && !hasStructuredDetails" class="mt-3 space-y-2">
          <div v-for="(note, index) in rawNotes" :key="note.path || note.fileName || note.filename || index"
            class="rounded-xl bg-black/10 dark:bg-white/5 p-2 text-[11px] text-primary-text">
            <div class="font-bold opacity-80 break-words">{{ note.title || note.fileName || note.filename || note.path || `日记 ${index + 1}` }}</div>
            <div class="mt-1 opacity-65 whitespace-pre-wrap break-words">{{ note.content || note.snippet || note.text || note.preview || JSON.stringify(note, null, 2) }}</div>
          </div>
        </div>

        <button v-if="hasLongContent" @click="expanded = !expanded"
          class="mt-2 text-[11px] font-bold text-blue-400 active:scale-95 transition-transform">
          {{ expanded ? '收起内容' : '展开完整内容' }}
        </button>

        <div v-if="props.item.actions && props.item.actions.length > 0" class="mt-4 flex gap-2">
          <button v-for="action in props.item.actions" :key="action.label" @click="handleAction(action)"
            :class="getActionButtonClass(action)">
            {{ action.label }}
          </button>
        </div>
      </div>

      <button @click="$emit('copy')"
        class="opacity-0 group-hover:opacity-40 hover:!opacity-100 transition-opacity p-1 text-primary-text">
        <component :is="props.copyIcon" :size="14" />
      </button>
    </div>
  </div>
</template>
