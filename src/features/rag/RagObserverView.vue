<script setup lang="ts">
import { computed, ref, watch } from 'vue';
import { format } from 'date-fns';
import { ChevronDown, ChevronUp, RefreshCcw, Trash2, X } from 'lucide-vue-next';
import { useRagObserverStore, type RagObserverEvent, type RagObserverFilter } from '../../core/stores/ragObserver';
import RagInsightDetails from '../notification/RagInsightDetails.vue';

const props = defineProps<{
  isOpen: boolean;
  zIndex: number;
}>();

const emit = defineEmits<{
  close: [];
}>();

const store = useRagObserverStore();
const expandedIds = ref(new Set<string>());

const statusClass = computed(() => {
  if (store.status === 'connected') return 'bg-green-500/12 text-green-500 border-green-500/20';
  if (store.status === 'connecting') return 'bg-yellow-500/12 text-yellow-600 dark:text-yellow-400 border-yellow-500/20';
  if (store.status === 'error') return 'bg-red-500/12 text-red-500 border-red-500/20';
  return 'bg-black/5 dark:bg-white/5 text-primary-text border-black/10 dark:border-white/10';
});

const kindLabel = (kind: RagObserverEvent['kind']) => {
  const labels: Record<RagObserverEvent['kind'], string> = {
    rag: 'RAG',
    chain: 'CHAIN',
    memo: 'MEMO',
    'daily-note': 'DIARY',
    'agent-chat': 'CHAT',
    'agent-notice': 'AGENT',
    dream: 'DREAM',
    'tool-approval': 'APPROVAL',
    'tool-result': 'RESULT',
    'tool-log': 'LOG',
    'video-status': 'VIDEO',
    system: 'SYSTEM',
    notification: 'NOTICE',
    unknown: 'RAW',
  };
  return labels[kind];
};

const kindClass = (kind: RagObserverEvent['kind']) => {
  if (kind === 'rag') return 'bg-blue-500/12 text-blue-500';
  if (kind === 'chain') return 'bg-violet-500/12 text-violet-500';
  if (kind === 'memo') return 'bg-emerald-500/12 text-emerald-500';
  if (kind === 'daily-note') return 'bg-amber-500/12 text-amber-600 dark:text-amber-400';
  if (kind === 'dream') return 'bg-pink-500/12 text-pink-500';
  if (kind === 'tool-approval') return 'bg-orange-500/12 text-orange-500';
  if (kind === 'tool-result') return 'bg-green-500/12 text-green-500';
  if (kind === 'tool-log') return 'bg-cyan-500/12 text-cyan-500';
  if (kind === 'video-status') return 'bg-rose-500/12 text-rose-500';
  if (kind === 'system') return 'bg-slate-500/12 text-slate-500 dark:text-slate-300';
  if (kind === 'notification') return 'bg-sky-500/12 text-sky-500';
  return 'bg-black/5 dark:bg-white/8 text-primary-text';
};

const filterOptions: Array<{ key: RagObserverFilter; label: string; count: () => number }> = [
  { key: 'all', label: '全部', count: () => store.events.length },
  { key: 'rag', label: 'RAG', count: () => store.events.filter((event) => event.kind === 'rag').length },
  { key: 'chain', label: '链路', count: () => store.events.filter((event) => event.kind === 'chain').length },
  { key: 'chat', label: '会话', count: () => store.events.filter((event) => event.kind === 'agent-chat' || event.kind === 'agent-notice').length },
  { key: 'memo', label: '记忆', count: () => store.events.filter((event) => event.kind === 'memo' || event.kind === 'daily-note').length },
  { key: 'dream', label: '梦境', count: () => store.events.filter((event) => event.kind === 'dream').length },
  { key: 'tool', label: '工具', count: () => store.events.filter((event) => event.kind === 'tool-approval' || event.kind === 'tool-result' || event.kind === 'tool-log' || event.kind === 'video-status').length },
  { key: 'notification', label: '通知', count: () => store.events.filter((event) => event.kind === 'notification' || event.kind === 'system').length },
  { key: 'unknown', label: 'RAW', count: () => store.events.filter((event) => event.kind === 'unknown').length },
];

const isExpanded = (id: string) => expandedIds.value.has(id);

const toggleExpand = (id: string) => {
  const next = new Set(expandedIds.value);
  if (next.has(id)) {
    next.delete(id);
  } else {
    next.add(id);
  }
  expandedIds.value = next;
};

const reconnect = () => {
  store.disconnect();
  void store.connect();
};

watch(
  () => props.isOpen,
  (isOpen) => {
    if (isOpen) {
      store.open();
    } else {
      store.close();
    }
  },
  { immediate: true },
);

const closeView = () => {
  store.close();
  emit('close');
};
</script>

<template>
  <Teleport to="#vcp-feature-overlays">
    <Transition name="rag-observer">
      <div v-if="props.isOpen"
        class="rag-observer fixed inset-0 flex flex-col text-primary-text pointer-events-auto"
        :style="{ zIndex: props.zIndex }">
        <header class="rag-observer__header pt-safe shrink-0 border-b border-black/5 dark:border-white/10">
          <div class="flex items-center justify-between gap-3 px-4 py-3.5">
            <div class="min-w-0">
              <div class="text-sm font-black tracking-tight">VCPInfo Observer</div>
              <div class="mt-1 flex items-center gap-2 text-[10px] opacity-70">
                <span class="rounded-full border px-2 py-0.5 font-bold" :class="statusClass">
                  {{ store.statusMessage }}
                </span>
                <span class="font-mono">{{ store.events.length }}/200</span>
              </div>
            </div>

            <div class="flex items-center gap-1 shrink-0">
              <button
                class="rag-observer__icon-button h-10 w-10 rounded-xl flex items-center justify-center active:scale-95"
                title="重新连接"
                @click="reconnect">
                <RefreshCcw :size="17" />
              </button>
              <button
                class="rag-observer__icon-button h-10 w-10 rounded-xl flex items-center justify-center active:scale-95"
                title="清空"
                @click="store.clear()">
                <Trash2 :size="17" />
              </button>
              <button
                class="rag-observer__icon-button h-10 w-10 rounded-xl flex items-center justify-center active:scale-95"
                title="关闭"
                @click="closeView">
                <X :size="20" />
              </button>
            </div>
          </div>

          <div class="rag-observer__filters vcp-scrollable flex gap-2 overflow-x-auto px-4 pb-3">
            <button
              v-for="option in filterOptions"
              :key="option.key"
              class="rag-observer__filter shrink-0 rounded-full px-3 py-1.5 text-[11px] font-black"
              :class="{ 'is-active': store.activeFilter === option.key }"
              @click="store.setFilter(option.key)"
            >
              <span>{{ option.label }}</span>
              <span class="font-mono opacity-55">{{ option.count() }}</span>
            </button>
          </div>
        </header>

        <main class="rag-observer__body flex-1 min-h-0 overflow-y-auto vcp-scrollable px-3 py-3">
          <div v-if="store.events.length === 0" class="h-full flex flex-col items-center justify-center text-center px-8">
            <div class="rag-observer__empty">
              <div class="text-[11px] font-black uppercase tracking-[0.2em]">Waiting For VCPInfo</div>
              <div class="mt-2 max-w-[260px] text-xs leading-relaxed opacity-65">
                发起对话后，这里会显示 RAG 召回、元思考链、记忆回溯和 Agent 观察事件。
              </div>
            </div>
          </div>
          <div v-else-if="store.filteredEvents.length === 0" class="h-full flex flex-col items-center justify-center text-center px-8">
            <div class="rag-observer__empty">
              <div class="text-[11px] font-black uppercase tracking-[0.2em]">No Events</div>
              <div class="mt-2 max-w-[260px] text-xs leading-relaxed opacity-65">
                当前分类还没有事件。
              </div>
            </div>
          </div>

          <div v-else class="space-y-2">
            <article v-for="event in store.filteredEvents" :key="event.id"
              class="rag-observer__event rounded-lg overflow-hidden">
              <button class="w-full px-3 py-3 text-left active:bg-black/5 dark:active:bg-white/5"
                @click="toggleExpand(event.id)">
                <div class="flex items-start justify-between gap-2">
                  <div class="min-w-0">
                    <div class="mb-1 flex items-center gap-2">
                      <span class="rounded px-1.5 py-0.5 text-[9px] font-black tracking-wider"
                        :class="kindClass(event.kind)">
                        {{ kindLabel(event.kind) }}
                      </span>
                      <span class="font-mono text-[9px] opacity-45">{{ format(event.timestamp, 'HH:mm:ss') }}</span>
                    </div>
                    <div class="truncate text-xs font-bold opacity-90">{{ event.title }}</div>
                  </div>
                  <component :is="isExpanded(event.id) ? ChevronUp : ChevronDown" :size="16" class="shrink-0 opacity-45" />
                </div>
                <pre class="rag-observer__summary mt-2 whitespace-pre-wrap break-words text-[11px] leading-relaxed">{{ event.summary }}</pre>
              </button>

              <RagInsightDetails v-if="isExpanded(event.id)" :payload="event.payload" :kind="event.kind as any" />
            </article>
          </div>
        </main>
      </div>
    </Transition>
  </Teleport>
</template>

<style scoped>
.rag-observer {
  background-color: var(--primary-bg);
  background-image:
    linear-gradient(180deg, color-mix(in srgb, var(--secondary-bg) 86%, transparent), var(--primary-bg) 52%),
    radial-gradient(circle at 18% 0%, color-mix(in srgb, #3b82f6 10%, transparent), transparent 34%);
}

.rag-observer__header {
  background-color: var(--secondary-bg);
  background-color: color-mix(in srgb, var(--secondary-bg) 96%, var(--primary-bg));
  box-shadow: 0 10px 30px rgba(0, 0, 0, 0.12);
}

.rag-observer__body {
  background-color: var(--primary-bg);
  background-color: color-mix(in srgb, var(--primary-bg) 98%, var(--secondary-bg));
  padding-bottom: calc(var(--vcp-safe-bottom, 20px) + 12px);
}

.rag-observer__filters {
  scrollbar-width: none;
}

.rag-observer__filters::-webkit-scrollbar {
  display: none;
}

.rag-observer__filter {
  border: 1px solid color-mix(in srgb, var(--primary-text) 9%, transparent);
  background-color: color-mix(in srgb, var(--primary-text) 5%, transparent);
  color: color-mix(in srgb, var(--primary-text) 62%, transparent);
  display: inline-flex;
  align-items: center;
  gap: 6px;
  transition: background-color 0.18s ease, color 0.18s ease, border-color 0.18s ease;
}

.rag-observer__filter.is-active {
  border-color: color-mix(in srgb, #3b82f6 48%, transparent);
  background-color: color-mix(in srgb, #3b82f6 16%, var(--secondary-bg));
  color: var(--primary-text);
}

.rag-observer__icon-button {
  border: 1px solid color-mix(in srgb, var(--primary-text) 10%, transparent);
  background-color: rgba(128, 128, 128, 0.12);
  background-color: color-mix(in srgb, var(--primary-text) 7%, transparent);
  transition: background-color 0.18s ease, border-color 0.18s ease, transform 0.18s ease;
}

.rag-observer__icon-button:active {
  background-color: rgba(128, 128, 128, 0.18);
  background-color: color-mix(in srgb, var(--primary-text) 12%, transparent);
}

.rag-observer__empty {
  border: 1px solid color-mix(in srgb, var(--primary-text) 8%, transparent);
  border-radius: 18px;
  background-color: var(--secondary-bg);
  background-color: color-mix(in srgb, var(--secondary-bg) 72%, var(--primary-bg));
  padding: 28px 22px;
  box-shadow: 0 18px 50px rgba(0, 0, 0, 0.12);
}

.rag-observer__event {
  border: 1px solid color-mix(in srgb, var(--primary-text) 9%, transparent);
  background-color: var(--secondary-bg);
  background-color: color-mix(in srgb, var(--secondary-bg) 82%, var(--primary-bg));
  box-shadow: 0 10px 26px rgba(0, 0, 0, 0.08);
}

.rag-observer__summary {
  opacity: 0.68;
}

.rag-observer-enter-active,
.rag-observer-leave-active {
  transition: transform 0.22s ease, opacity 0.22s ease;
}

.rag-observer-enter-from,
.rag-observer-leave-to {
  opacity: 0;
  transform: translateY(10px);
}
</style>
