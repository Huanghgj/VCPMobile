<script setup lang="ts">
import { computed, ref } from 'vue';
import type { VcpObserverKind } from '../../core/stores/notification';

const props = defineProps<{
  payload: any;
  kind?: VcpObserverKind;
}>();

const showMoreResults = ref(false);
const showMoreStages = ref(false);
const showRaw = ref(false);

const compact = (value: any, limit = 420) => {
  const text = value === undefined || value === null
    ? ''
    : typeof value === 'string'
      ? value
      : safeJson(value);
  const normalized = text.replace(/\s+/g, ' ').trim();
  return normalized.length > limit ? `${normalized.slice(0, limit)}...` : normalized;
};

const safeJson = (value: any) => {
  try {
    return JSON.stringify(value, null, 2);
  } catch {
    return String(value);
  }
};

const asArray = (value: any): any[] => Array.isArray(value) ? value : [];

const payloadType = computed(() => String(props.payload?.type || props.kind || 'unknown'));
const vcpLogData = computed(() => props.payload?.data && typeof props.payload.data === 'object' ? props.payload.data : null);

const observerKind = computed<VcpObserverKind | undefined>(() => {
  if (props.kind) return props.kind;
  if (payloadType.value === 'RAG_RETRIEVAL_DETAILS') return 'rag';
  if (payloadType.value === 'META_THINKING_CHAIN') return 'chain';
  if (payloadType.value === 'AI_MEMO_RETRIEVAL') return 'memo';
  if (payloadType.value === 'DailyNote' || payloadType.value === 'daily_note_recall_result') return 'daily-note';
  if (payloadType.value.startsWith('AGENT_DREAM_')) return 'dream';
  if (payloadType.value === 'AGENT_PRIVATE_CHAT_PREVIEW') return 'agent-chat';
  if (payloadType.value === 'tool_approval_request') return 'tool-approval';
  if (payloadType.value === 'tool_result') return 'tool-result';
  if (payloadType.value === 'vcp_log' || payloadType.value === 'vcp-log-message') return 'tool-log';
  if (payloadType.value === 'video_generation_status') return 'video-status';
  if (payloadType.value === 'notification') return 'notification';
  if (payloadType.value === 'error' || payloadType.value === 'connection_ack' || payloadType.value === 'vcp_log_status' || payloadType.value === 'vcp-log-status' || payloadType.value === 'connection_status') return 'system';
  if (props.payload?.source === 'AgentAssistant') return 'agent-notice';
  return undefined;
});

const ragResults = computed(() => asArray(props.payload?.results));
const visibleRagResults = computed(() => showMoreResults.value ? ragResults.value : ragResults.value.slice(0, 4));
const hiddenRagResultCount = computed(() => Math.max(0, ragResults.value.length - visibleRagResults.value.length));

const chainStages = computed(() => asArray(props.payload?.stages));
const visibleChainStages = computed(() => showMoreStages.value ? chainStages.value : chainStages.value.slice(0, 3));
const hiddenStageCount = computed(() => Math.max(0, chainStages.value.length - visibleChainStages.value.length));

const notes = computed(() => {
  if (Array.isArray(props.payload?.notes)) return props.payload.notes;
  if (Array.isArray(props.payload?.data?.notes)) return props.payload.data.notes;
  return [];
});
const visibleNotes = computed(() => showMoreResults.value ? notes.value : notes.value.slice(0, 4));
const hiddenNoteCount = computed(() => Math.max(0, notes.value.length - visibleNotes.value.length));

const dreamSeeds = computed(() => asArray(props.payload?.seeds).slice(0, 3));
const dreamAssociations = computed(() => asArray(props.payload?.associations).slice(0, 4));
const dreamOperations = computed(() => asArray(props.payload?.operations).slice(0, 6));

const rawText = computed(() => safeJson(props.payload));

const scoreText = (value: any) => {
  if (typeof value === 'number') return value.toFixed(3);
  if (value === undefined || value === null || value === '') return 'N/A';
  return String(value);
};

const resultTitle = (item: any, index: number) =>
  item?.title || item?.fileName || item?.filename || item?.path || item?.file || item?.source || `结果 ${index + 1}`;

const resultBody = (item: any) =>
  item?.text || item?.content || item?.snippet || item?.preview || item?.message || item;

const toolCommand = computed(() => {
  const data = props.payload?.data || {};
  const args = data.args || {};
  return args.command || safeJson(args);
});

const toolLogContent = computed(() => {
  const content = vcpLogData.value?.content ?? props.payload?.content ?? props.payload?.message;
  if (typeof content !== 'string') return compact(content, 900);
  try {
    const parsed = JSON.parse(content);
    if (parsed?.original_plugin_output !== undefined) return compact(parsed.original_plugin_output, 900);
    if (parsed?.plugin_error || parsed?.error || parsed?.message) {
      return compact(parsed.plugin_error || parsed.error || parsed.message, 900);
    }
  } catch {
    // Plain text content is expected for many tool logs.
  }
  return compact(content, 900);
});

const toolResultData = computed(() => {
  if (observerKind.value === 'tool-result') {
    return props.payload?.data && typeof props.payload.data === 'object' ? props.payload.data : props.payload;
  }
  return vcpLogData.value || {};
});

const toolStatus = computed(() =>
  toolResultData.value?.status || (toolResultData.value?.error ? 'error' : 'success')
);

const toolResultContent = computed(() => {
  const data = toolResultData.value || {};
  const result = data.result ?? data.output ?? data.content ?? data.message ?? data.error ?? data.original_plugin_output ?? data;
  if (typeof result === 'string') {
    try {
      const parsed = JSON.parse(result);
      return compact(parsed?.original_plugin_output ?? parsed?.message ?? parsed, 1600);
    } catch {
      return compact(result, 1600);
    }
  }
  return compact(result, 1600);
});

const videoStatusData = computed(() =>
  props.payload?.data && typeof props.payload.data === 'object' ? props.payload.data : props.payload
);

const videoStatusContent = computed(() => {
  const data = videoStatusData.value || {};
  return compact(data?.original_plugin_output?.message || data?.message || data?.error || data, 1200);
});

const systemContent = computed(() =>
  compact(props.payload?.message || props.payload?.data?.message || props.payload?.data || props.payload, 1200)
);
</script>

<template>
  <div class="mt-3 border-t border-black/10 dark:border-white/10 pt-3 text-[11px] text-primary-text">
    <div class="mb-2 flex flex-wrap items-center gap-1.5 opacity-70">
      <span class="font-black uppercase tracking-[0.16em]">Observer</span>
      <span class="rounded-full border border-black/10 dark:border-white/10 px-2 py-0.5 font-mono">{{ payloadType }}</span>
    </div>

    <template v-if="observerKind === 'rag'">
      <div class="space-y-2">
        <div class="font-bold opacity-85">RAG 检索：{{ props.payload?.dbName || 'Unknown' }}</div>
        <div class="flex flex-wrap gap-1.5 opacity-70">
          <span>K={{ props.payload?.k ?? 'N/A' }}</span>
          <span>耗时={{ props.payload?.useTime ?? 'N/A' }}</span>
          <span>ReRank={{ props.payload?.useRerank ?? 'N/A' }}</span>
          <span>结果={{ ragResults.length }}</span>
        </div>
        <div v-if="props.payload?.query" class="whitespace-pre-wrap break-words opacity-80">{{ compact(props.payload.query, 260) }}</div>
        <div v-if="Array.isArray(props.payload?.coreTags) && props.payload.coreTags.length" class="flex flex-wrap gap-1">
          <span v-for="tag in props.payload.coreTags.slice(0, 10)" :key="tag"
            class="rounded-full bg-blue-500/10 px-2 py-0.5 text-blue-500">{{ tag }}</span>
        </div>
        <div class="divide-y divide-black/10 dark:divide-white/10">
          <div v-for="(item, index) in visibleRagResults" :key="index" class="py-2">
            <div class="mb-1 flex items-center justify-between gap-2">
              <span class="min-w-0 truncate font-bold opacity-80">{{ resultTitle(item, index) }}</span>
              <span class="shrink-0 font-mono opacity-55">{{ scoreText(item?.score) }}</span>
            </div>
            <div class="whitespace-pre-wrap break-words opacity-65">{{ compact(resultBody(item), 360) }}</div>
          </div>
        </div>
        <button v-if="hiddenRagResultCount > 0" class="text-[11px] font-bold text-blue-400" @click="showMoreResults = true">
          显示剩余 {{ hiddenRagResultCount }} 条
        </button>
      </div>
    </template>

    <template v-else-if="observerKind === 'chain'">
      <div class="space-y-2">
        <div class="font-bold opacity-85">元思考链：{{ props.payload?.chainName || 'Unknown' }}</div>
        <div v-if="props.payload?.query" class="whitespace-pre-wrap break-words opacity-75">{{ compact(props.payload.query, 260) }}</div>
        <div class="divide-y divide-black/10 dark:divide-white/10">
          <div v-for="(stage, index) in visibleChainStages" :key="index" class="py-2">
            <div class="mb-1 font-bold opacity-80">
              阶段 {{ stage?.stage ?? index + 1 }}：{{ stage?.clusterName || 'Unknown' }}
              <span class="font-mono opacity-50">({{ stage?.resultCount ?? asArray(stage?.results).length }})</span>
            </div>
            <div v-for="(item, resultIndex) in asArray(stage?.results).slice(0, 2)" :key="resultIndex"
              class="mt-1 whitespace-pre-wrap break-words opacity-65">
              {{ compact(resultBody(item), 260) }}
            </div>
          </div>
        </div>
        <button v-if="hiddenStageCount > 0" class="text-[11px] font-bold text-blue-400" @click="showMoreStages = true">
          显示剩余 {{ hiddenStageCount }} 阶段
        </button>
      </div>
    </template>

    <template v-else-if="observerKind === 'memo'">
      <div class="space-y-2">
        <div class="font-bold opacity-85">记忆回溯</div>
        <div class="opacity-70">模式={{ props.payload?.mode || 'N/A' }} · 日记数={{ props.payload?.diaryCount ?? 'N/A' }}</div>
        <div class="max-h-64 overflow-y-auto whitespace-pre-wrap break-words opacity-75 vcp-scrollable">
          {{ compact(props.payload?.extractedMemories || props.payload?.message || props.payload, 1600) }}
        </div>
      </div>
    </template>

    <template v-else-if="observerKind === 'daily-note'">
      <div class="space-y-2">
        <div class="font-bold opacity-85">日记召回：{{ props.payload?.dbName || props.payload?.query || 'Unknown' }}</div>
        <div class="opacity-70">模式={{ props.payload?.action === 'FullTextRecall' ? '全量召回' : props.payload?.action || '直接引入' }}</div>
        <div v-if="notes.length" class="divide-y divide-black/10 dark:divide-white/10">
          <div v-for="(note, index) in visibleNotes" :key="index" class="py-2">
            <div class="font-bold opacity-80">{{ resultTitle(note, index) }}</div>
            <div class="whitespace-pre-wrap break-words opacity-65">{{ compact(resultBody(note), 360) }}</div>
          </div>
        </div>
        <div v-else class="whitespace-pre-wrap break-words opacity-75">{{ compact(props.payload?.message || props.payload, 900) }}</div>
        <button v-if="hiddenNoteCount > 0" class="text-[11px] font-bold text-blue-400" @click="showMoreResults = true">
          显示剩余 {{ hiddenNoteCount }} 条
        </button>
      </div>
    </template>

    <template v-else-if="observerKind === 'dream'">
      <div class="space-y-2">
        <div class="font-bold opacity-85">Agent 梦境：{{ props.payload?.agentName || 'Unknown' }}</div>
        <div class="flex flex-wrap gap-1.5 opacity-70">
          <span v-if="props.payload?.dreamId">DreamID={{ props.payload.dreamId }}</span>
          <span v-if="props.payload?.seedCount !== undefined">种子={{ props.payload.seedCount }}</span>
          <span v-if="props.payload?.associationCount !== undefined">联想={{ props.payload.associationCount }}</span>
          <span v-if="props.payload?.operationCount !== undefined">操作={{ props.payload.operationCount }}</span>
        </div>
        <div v-if="props.payload?.message || props.payload?.error || props.payload?.narrative"
          class="max-h-64 overflow-y-auto whitespace-pre-wrap break-words opacity-75 vcp-scrollable">
          {{ compact(props.payload?.message || props.payload?.error || props.payload?.narrative, 1400) }}
        </div>
        <div v-if="dreamSeeds.length" class="divide-y divide-black/10 dark:divide-white/10">
          <div v-for="(seed, index) in dreamSeeds" :key="index" class="py-1.5 opacity-70">
            {{ resultTitle(seed, index) }}：{{ compact(seed?.snippet || seed, 220) }}
          </div>
        </div>
        <div v-if="dreamAssociations.length" class="divide-y divide-black/10 dark:divide-white/10">
          <div v-for="(item, index) in dreamAssociations" :key="index" class="py-1.5 opacity-70">
            {{ resultTitle(item, index) }} <span class="font-mono opacity-50">{{ scoreText(item?.score) }}</span>
          </div>
        </div>
        <div v-if="dreamOperations.length" class="divide-y divide-black/10 dark:divide-white/10">
          <div v-for="(op, index) in dreamOperations" :key="index" class="py-1.5 opacity-70">
            {{ op?.type || 'operation' }} · {{ op?.status || 'unknown' }} · {{ op?.operationId || index + 1 }}
          </div>
        </div>
      </div>
    </template>

    <template v-else-if="observerKind === 'agent-chat' || observerKind === 'agent-notice'">
      <div class="space-y-2">
        <div class="font-bold opacity-85">{{ props.payload?.agentName || props.payload?.source || 'Agent' }}</div>
        <div class="max-h-64 overflow-y-auto whitespace-pre-wrap break-words opacity-75 vcp-scrollable">
          {{ compact(props.payload?.response || props.payload?.query || props.payload?.message || props.payload, 1400) }}
        </div>
      </div>
    </template>

    <template v-else-if="observerKind === 'tool-approval'">
      <div class="space-y-2">
        <div class="font-bold opacity-85">{{ props.payload?.data?.maid || '未知助手' }} 请求调用 {{ props.payload?.data?.toolName || '未知工具' }}</div>
        <div class="font-mono whitespace-pre-wrap break-words opacity-75">{{ compact(toolCommand, 900) }}</div>
      </div>
    </template>

    <template v-else-if="observerKind === 'tool-result'">
      <div class="space-y-2">
        <div class="font-bold opacity-85">
          {{ toolResultData?.toolName || toolResultData?.tool_name || toolResultData?.name || 'VCP 工具' }}
        </div>
        <div class="flex flex-wrap gap-1.5 opacity-70">
          <span>状态={{ toolStatus }}</span>
          <span v-if="toolResultData?.source">来源={{ toolResultData.source }}</span>
        </div>
        <div class="max-h-64 overflow-y-auto whitespace-pre-wrap break-words opacity-75 vcp-scrollable">
          {{ toolResultContent }}
        </div>
      </div>
    </template>

    <template v-else-if="observerKind === 'tool-log'">
      <div class="space-y-2">
        <div class="font-bold opacity-85">{{ vcpLogData?.tool_name || props.payload?.data?.toolName || props.payload?.data?.name || 'VCP 工具' }}</div>
        <div class="flex flex-wrap gap-1.5 opacity-70">
          <span v-if="vcpLogData?.status">状态={{ vcpLogData.status }}</span>
          <span v-if="vcpLogData?.source">来源={{ vcpLogData.source }}</span>
        </div>
        <div class="max-h-64 overflow-y-auto whitespace-pre-wrap break-words font-mono opacity-75 vcp-scrollable">
          {{ toolLogContent }}
        </div>
      </div>
    </template>

    <template v-else-if="observerKind === 'video-status'">
      <div class="space-y-2">
        <div class="font-bold opacity-85">视频生成状态</div>
        <div class="flex flex-wrap gap-1.5 opacity-70">
          <span v-if="videoStatusData?.status">状态={{ videoStatusData.status }}</span>
          <span v-if="videoStatusData?.timestamp">时间={{ videoStatusData.timestamp }}</span>
        </div>
        <div class="max-h-64 overflow-y-auto whitespace-pre-wrap break-words opacity-75 vcp-scrollable">
          {{ videoStatusContent }}
        </div>
      </div>
    </template>

    <template v-else-if="observerKind === 'system' || observerKind === 'notification'">
      <div class="space-y-2">
        <div class="font-bold opacity-85">{{ props.payload?.title || props.payload?.type || '系统事件' }}</div>
        <div class="max-h-64 overflow-y-auto whitespace-pre-wrap break-words opacity-75 vcp-scrollable">
          {{ systemContent }}
        </div>
      </div>
    </template>

    <template v-else>
      <div class="max-h-64 overflow-y-auto whitespace-pre-wrap break-words opacity-75 vcp-scrollable">
        {{ compact(props.payload?.message || props.payload?.data || props.payload, 1400) }}
      </div>
    </template>

    <div class="mt-3 border-t border-black/10 dark:border-white/10 pt-2">
      <button class="text-[11px] font-bold opacity-60 active:scale-95" @click="showRaw = !showRaw">
        {{ showRaw ? '隐藏原始 JSON' : '显示原始 JSON' }}
      </button>
      <pre v-if="showRaw" class="mt-2 max-h-72 overflow-y-auto whitespace-pre-wrap break-words rounded-lg bg-black/10 p-2 text-[10px] opacity-75 vcp-scrollable">{{ rawText }}</pre>
    </div>
  </div>
</template>
