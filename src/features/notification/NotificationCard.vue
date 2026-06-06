<script setup lang="ts">
import { ref, computed, onUnmounted } from 'vue';
import {
  ChevronDown,
  ChevronUp,
  Copy,
  Check,
  Trash2,
  Database,
  Brain,
  MessageSquare,
  Moon,
  ShieldAlert,
  Wrench,
  BookOpen,
  Video,
  Cpu,
  Terminal
} from 'lucide-vue-next';
import type { VcpNotification } from '../../core/stores/notification';
import { useNotificationPresentation } from './composables/useNotificationPresentation';

const props = defineProps<{
  item: VcpNotification;
}>();

const emit = defineEmits<{
  (e: 'delete', id: string): void;
  (e: 'action', payload: { id: string; action: { label: string; value: boolean; color: string } }): void;
}>();

type StructuredRow = NonNullable<NonNullable<VcpNotification['structured']>['rows']>[number];
type StructuredMetric = NonNullable<StructuredRow['metrics']>[number];
type DetailChip = { label: string; value: string };

const { formatTime, getTypeColor, copyToClipboard } = useNotificationPresentation();

const isDetailsExpanded = ref(false);
const isCopied = ref(false);
const touchStartX = ref(0);
const touchStartY = ref(0);
const swipeOffset = ref(0);
const isSwiping = ref(false);
const isVerticalScroll = ref(false);
const hasDeterminedDirection = ref(false);
let copyResetTimer: ReturnType<typeof setTimeout> | null = null;

const presentation = computed(() => getTypeColor(props.item.type));

// Swipe to delete logic
const handleTouchStart = (e: TouchEvent) => {
  touchStartX.value = e.touches[0].clientX;
  touchStartY.value = e.touches[0].clientY;
  isSwiping.value = true;
  isVerticalScroll.value = false;
  hasDeterminedDirection.value = false;
};

const handleTouchMove = (e: TouchEvent) => {
  if (!isSwiping.value || isVerticalScroll.value) return;
  const currentX = e.touches[0].clientX;
  const currentY = e.touches[0].clientY;
  const deltaX = currentX - touchStartX.value;
  const deltaY = currentY - touchStartY.value;

  if (!hasDeterminedDirection.value) {
    const absX = Math.abs(deltaX);
    const absY = Math.abs(deltaY);
    if (absX > 15 || absY > 15) {
      hasDeterminedDirection.value = true;
      if (absY / absX > 0.577) {
        isVerticalScroll.value = true;
        swipeOffset.value = 0;
        return;
      }
    } else {
      return;
    }
  }

  if (deltaX < 0) {
    swipeOffset.value = Math.max(-80, deltaX);
  } else {
    swipeOffset.value = 0;
  }
};

const handleTouchEnd = () => {
  isSwiping.value = false;
  if (swipeOffset.value < -45) {
    swipeOffset.value = -70; // Keep open to show delete
  } else {
    swipeOffset.value = 0;
  }
};

const resetSwipe = () => {
  swipeOffset.value = 0;
};

const triggerDelete = () => {
  emit('delete', props.item.id);
  resetSwipe();
};

const handleCopy = async () => {
  const success = await copyToClipboard(rawPayloadText.value);
  if (success) {
    isCopied.value = true;
    if (copyResetTimer) clearTimeout(copyResetTimer);
    copyResetTimer = setTimeout(() => {
      isCopied.value = false;
      copyResetTimer = null;
    }, 2000);
  }
};

onUnmounted(() => {
  if (copyResetTimer) clearTimeout(copyResetTimer);
});

const rawPayloadText = computed(() => JSON.stringify(props.item.rawPayload || props.item, null, 2));

const stringifyCompactValue = (value: unknown, maxLength = 180) => {
  const text = typeof value === 'string' ? value : JSON.stringify(value);
  if (!text) return '';
  return text.length > maxLength ? `${text.substring(0, maxLength)}...` : text;
};

const getCategoryIcon = (item: VcpNotification) => {
  const cat = (item.category || '').toLowerCase();
  const infoType = (item.infoType || '').toLowerCase();
  const rawType = item.rawPayload?.type;
  const kind = item.structured?.kind;
  if (rawType === 'tool_approval_request' || infoType === 'tool_approval_request' || item.actions?.length) return ShieldAlert;
  if (cat === 'rag') return Database;
  if (cat === 'meta' || kind === 'thinking') return Brain;
  if (cat === 'dream' || kind === 'dream') return Moon;
  if (cat === 'agent' || kind === 'private_chat') return MessageSquare;
  if (cat === 'tool') return Wrench;
  if (cat === 'dailynote') return BookOpen;
  if (cat === 'video') return Video;
  if (cat === 'system') return Cpu;
  return Terminal;
};

const ragResults = computed(() => {
  if (props.item.structured?.kind === 'rag' && props.item.structured.rows) {
    return props.item.structured.rows.map((row: StructuredRow) => {
      const distMetric = row.metrics?.find((m: StructuredMetric) => m.label.toLowerCase().includes('dist') || m.label.toLowerCase().includes('geo'));
      const scoreMetric = row.metrics?.find((m: StructuredMetric) => m.label.toLowerCase().includes('score'));

      let distanceStr = '';
      let isEst = false;

      if (distMetric) {
        distanceStr = distMetric.value;
      } else if (scoreMetric) {
        const score = parseFloat(scoreMetric.value);
        if (!isNaN(score) && score > 0) {
          distanceStr = ((1 / score) - 1).toFixed(4);
          isEst = true;
        }
      }

      const detailChips: DetailChip[] = [
        row.source ? { label: 'source', value: row.source } : null,
        row.path ? { label: 'path', value: row.path } : null,
        row.snippet ? { label: 'snippet', value: row.snippet } : null,
        ...(row.metrics || []),
      ].filter(Boolean) as DetailChip[];

      const metadata = row.metadata ?? (row as any).meta;
      if (metadata) {
        for (const [label, value] of Object.entries(metadata)) {
          detailChips.push({ label, value: stringifyCompactValue(value) });
        }
      }

      return {
        title: row.title,
        subtitle: row.subtitle,
        body: row.body,
        chips: row.chips || [],
        distance: distanceStr,
        isEstimated: isEst,
        details: detailChips
      };
    });
  }
  return [];
});
</script>

<template>
  <div class="relative overflow-hidden w-full select-none mb-2.5 px-3">
    <!-- Swipe Action Background -->
    <div class="absolute inset-y-0 right-3 w-[70px] bg-rose-100/90 rounded-r-xl flex items-center justify-center border-l border-rose-200/50 z-0">
      <button @click="triggerDelete" class="w-full h-full flex flex-col items-center justify-center text-rose-600 active:text-rose-800 transition-colors">
        <Trash2 class="w-4 h-4" />
        <span class="text-[9px] mt-1 font-medium">删除</span>
      </button>
    </div>

    <!-- Card Content Container (Glass/Bubble Stack Style) -->
    <div
      class="relative bg-white/95 border border-pink-100/80 shadow-[0_2px_8px_-3px_rgba(244,63,94,0.08)] rounded-xl p-3.5 transition-transform duration-200 ease-out z-10 active:scale-[0.99]"
      :style="{ transform: `translateX(${swipeOffset}px)` }"
      @touchstart="handleTouchStart"
      @touchmove="handleTouchMove"
      @touchend="handleTouchEnd"
    >
      <!-- Header -->
      <div class="flex items-start justify-between gap-2">
        <div class="flex items-center gap-1.5 min-w-0">
          <span :class="['w-2 h-2 rounded-full shrink-0 shadow-sm', presentation.dot]"></span>
          <component :is="getCategoryIcon(item)" class="w-3.5 h-3.5 text-slate-400 shrink-0" />
          <span v-if="item.source" class="text-[9px] font-mono font-semibold text-pink-400 shrink-0 bg-pink-50 px-1 py-0.2 rounded">{{ item.source }}</span>
          <h4 class="text-xs font-bold text-slate-800 truncate">{{ item.title }}</h4>
        </div>
        <div class="flex items-center gap-1.5 shrink-0">
          <span class="text-[9px] font-mono text-slate-400 bg-slate-50 px-1.5 py-0.5 rounded">{{ formatTime(item.timestamp) }}</span>
          <button
            v-if="swipeOffset !== 0"
            @click="resetSwipe"
            class="text-[9px] text-slate-500 bg-slate-100 px-1.5 py-0.5 rounded border border-slate-200 active:bg-slate-200"
          >
            取消
          </button>
        </div>
      </div>

      <!-- Subtitle & Message -->
      <div class="mt-1.5 pl-3.5">
        <p v-if="item.subtitle" class="text-[10px] text-slate-500 font-medium tracking-tight">{{ item.subtitle }}</p>
        <p
          class="text-xs text-slate-600 mt-1 leading-relaxed break-all whitespace-pre-wrap"
          :class="{ 'font-mono bg-slate-50 p-2 rounded-lg border border-slate-100 text-[11px] text-slate-700': item.isPreformatted }"
        >
          {{ item.message }}
        </p>
      </div>

      <!-- Tags & Meta -->
      <div class="mt-2 pl-3.5 flex flex-wrap gap-1.5 items-center">
        <span
          v-for="tag in item.tags"
          :key="tag"
          class="px-1.5 py-0.5 rounded bg-pink-50/60 text-pink-600 border border-pink-100/50 text-[9px] font-mono font-medium"
        >
          #{{ tag }}
        </span>
        <div v-for="m in item.meta" :key="m.label" class="text-[9px] font-mono text-slate-400 flex items-center gap-1 bg-slate-50 px-1.5 py-0.5 rounded border border-slate-100">
          <span class="text-slate-500 font-medium">{{ m.label }}:</span>
          <span class="text-slate-700 font-bold">{{ m.value }}</span>
        </div>
      </div>

      <!-- Structured Layouts -->
      <div class="mt-2.5 pl-3.5 space-y-2" v-if="item.structured">
        <!-- RAG Layout -->
        <div v-if="item.structured.kind === 'rag'" class="border border-emerald-100 rounded-lg bg-emerald-50/40 p-2.5 space-y-2">
          <div class="text-[9px] font-mono font-bold text-emerald-700 border-b border-emerald-100/60 pb-1 flex justify-between">
            <span>RAG 知识检索召回</span>
            <span v-if="item.structured.summary" class="text-emerald-600 font-normal">{{ item.structured.summary }}</span>
          </div>
          <div class="space-y-2 max-h-[180px] overflow-y-auto pr-1">
            <div
              v-for="(row, idx) in ragResults"
              :key="idx"
              class="text-[11px] border-b border-emerald-100/30 last:border-0 pb-2 last:pb-0"
            >
              <div class="flex items-start justify-between gap-2">
                <span class="text-slate-700 font-mono font-bold truncate max-w-[70%]">{{ row.title }}</span>
                <span
                  v-if="row.distance"
                  class="text-[9px] font-mono px-1.5 py-0.2 rounded bg-emerald-100 border border-emerald-200 text-emerald-800 font-semibold shrink-0"
                >
                  {{ row.isEstimated ? `dist≈${row.distance}` : `dist: ${row.distance}` }}
                </span>
              </div>
              <p v-if="row.subtitle" class="text-slate-400 text-[9px] truncate">{{ row.subtitle }}</p>
              <p v-if="row.body" class="text-slate-600 text-[10px] mt-1 bg-white/80 p-1.5 rounded border border-emerald-100/40 font-mono leading-relaxed">{{ row.body }}</p>
              <div class="flex flex-wrap gap-1 mt-1" v-if="row.chips.length">
                <span v-for="c in row.chips" :key="c" class="text-[8px] px-1 bg-emerald-100/50 text-emerald-700 rounded border border-emerald-100/30 font-medium">{{ c }}</span>
              </div>
              <div v-if="row.details.length" class="mt-1 grid gap-1">
                <div
                  v-for="detail in row.details"
                  :key="`${detail.label}-${detail.value}`"
                  class="text-[8.5px] font-mono text-slate-500 bg-white/70 border border-emerald-100/40 rounded px-1 py-0.5 break-all"
                >
                  <span class="font-bold text-emerald-700">{{ detail.label }}:</span>
                  {{ detail.value }}
                </div>
              </div>
            </div>
          </div>
        </div>

        <!-- Thinking Chain Layout -->
        <div v-if="item.structured.kind === 'thinking'" class="border border-indigo-100 rounded-lg bg-indigo-50/40 p-2.5 space-y-2">
          <div class="text-[9px] font-mono font-bold text-indigo-700 border-b border-indigo-100/60 pb-1">META THINKING CHAIN</div>
          <div class="space-y-2 max-h-[160px] overflow-y-auto">
            <div v-for="(row, idx) in item.structured.rows" :key="idx" class="text-[11px] pl-2.5 border-l-2 border-indigo-200">
              <div class="text-slate-700 font-bold">{{ row.title }}</div>
              <p v-if="row.body" class="text-slate-500 text-[10px] mt-0.5 font-mono">{{ row.body }}</p>
            </div>
          </div>
        </div>

        <!-- Dream Layout -->
        <div v-if="item.structured.kind === 'dream'" class="border border-purple-100 rounded-lg bg-purple-50/40 p-2.5 space-y-2">
          <div class="text-[9px] font-mono font-bold text-purple-700 border-b border-purple-100/60 pb-1">DREAM NARRATIVE</div>
          <p v-if="item.structured.summary" class="text-xs text-purple-800 italic leading-relaxed bg-white/40 p-1.5 rounded">"{{ item.structured.summary }}"</p>
          <div class="space-y-1 max-h-[140px] overflow-y-auto">
            <div v-for="(row, idx) in item.structured.rows" :key="idx" class="text-[10px] text-slate-600 flex items-start gap-1">
              <span class="text-purple-500 font-bold shrink-0">•</span>
              <span>{{ row.body || row.title }}</span>
            </div>
          </div>
        </div>
      </div>

      <!-- Approval Actions -->
      <div v-if="item.actions && item.actions.length > 0" class="mt-3 pl-3.5 flex gap-2">
        <button
          v-for="act in item.actions"
          :key="act.label"
          @click="emit('action', { id: item.id, action: act })"
          class="flex-1 py-2 rounded-lg text-xs font-bold border transition-all duration-150 text-center active:scale-95 motion-reduce:transition-none"
          :class="[
            act.color?.includes('red') || act.label.includes('拒绝') || act.label.toLowerCase().includes('deny')
              ? 'bg-rose-50 hover:bg-rose-100 text-rose-600 border-rose-200 active:bg-rose-200/50'
              : 'bg-emerald-500 hover:bg-emerald-600 text-white border-emerald-600 active:bg-emerald-700 shadow-sm shadow-emerald-100'
          ]"
        >
          {{ act.label }}
        </button>
      </div>

      <!-- Collapsible Details & Raw Payload -->
      <div class="mt-2.5 pl-3.5 border-t border-pink-100/40 pt-2.5">
        <button
          @click="isDetailsExpanded = !isDetailsExpanded"
          class="flex items-center gap-1 text-[9px] text-slate-400 hover:text-slate-600 font-mono font-semibold transition-colors"
        >
          <component :is="isDetailsExpanded ? ChevronUp : ChevronDown" class="w-3 h-3" />
          <span>{{ isDetailsExpanded ? '收起系统详情' : '展开系统详情' }}</span>
        </button>

        <div v-if="isDetailsExpanded" class="mt-2 space-y-2">
          <!-- Details List -->
          <div v-if="item.details && item.details.length > 0" class="space-y-1.5 bg-slate-50 p-2.5 rounded-lg border border-slate-100">
            <div
              v-for="d in item.details"
              :key="d.label"
              class="text-[10px] flex flex-col gap-0.5 border-b border-slate-200/40 last:border-0 pb-1.5 last:pb-0"
            >
              <span class="text-slate-400 font-mono font-bold">{{ d.label }}</span>
              <span
                class="text-slate-700 break-all leading-normal"
                :class="{ 'font-mono text-slate-600 bg-white p-1 rounded border border-slate-100': d.mono, 'whitespace-pre-wrap': d.multiline }"
              >
                {{ d.value }}
              </span>
            </div>
          </div>

          <!-- Raw Payload -->
          <div v-if="item.rawPayload" class="relative bg-slate-50 rounded-lg border border-slate-100 p-2.5">
            <div class="flex items-center justify-between border-b border-slate-200/50 pb-1.5 mb-1.5">
              <span class="text-[8px] font-mono font-bold text-slate-400">RAW PAYLOAD</span>
              <button
                @click="handleCopy"
                class="flex items-center gap-1 text-[8px] text-slate-500 hover:text-slate-700 bg-white px-2 py-0.5 rounded border border-slate-200 active:bg-slate-100 transition-colors"
              >
                <component :is="isCopied ? Check : Copy" class="w-2.5 h-2.5" />
                <span>{{ isCopied ? '已复制' : '复制' }}</span>
              </button>
            </div>
            <pre class="text-[9px] font-mono text-slate-500 overflow-x-auto max-h-[120px] whitespace-pre-wrap break-all leading-relaxed">{{ rawPayloadText }}</pre>
          </div>
        </div>
      </div>
    </div>
  </div>
</template>
