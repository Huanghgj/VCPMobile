<script setup lang="ts">
import { computed, ref, watch, onUnmounted } from "vue";
import { useDiary, type DiaryEntry } from "./useDiary";

const props = defineProps<{ query?: string; refreshKey?: number }>();
const emit = defineEmits<{ open: [folder: string, file: string] }>();

const diary = useDiary();
const activeFolder = ref<string | null>(null); // null = 全部

const searchResults = ref<DiaryEntry[]>([]);
const searching = ref(false);
const searchError = ref<string | null>(null);
let debounceTimer: number | null = null;

const isSearch = computed(() => !!(props.query || "").trim());

const runSearch = async () => {
  const requestQuery = (props.query || "").trim();
  const requestFolder = activeFolder.value || undefined;
  if (!requestQuery) {
    searchResults.value = [];
    searching.value = false;
    return;
  }
  searching.value = true;
  searchError.value = null;
  try {
    const results = await diary.searchEntries(requestQuery, requestFolder);
    if (
      requestQuery === (props.query || "").trim() &&
      requestFolder === (activeFolder.value || undefined)
    ) {
      searchResults.value = results;
    }
  } catch (e: any) {
    searchError.value = typeof e === "string" ? e : e?.message || "搜索失败";
    searchResults.value = [];
  } finally {
    searching.value = false;
  }
};

// 全文搜索：去抖 300ms 调服务端
watch([() => props.query, activeFolder], () => {
  if (debounceTimer) clearTimeout(debounceTimer);
  if (!isSearch.value) {
    searchResults.value = [];
    searching.value = false;
    return;
  }
  searching.value = true;
  debounceTimer = window.setTimeout(runSearch, 300);
});

watch(
  () => props.refreshKey,
  () => {
    if (isSearch.value) runSearch();
  },
);

const timelineFiltered = computed<DiaryEntry[]>(() =>
  activeFolder.value
    ? diary.timeline.value.filter((e) => e.folder === activeFolder.value)
    : diary.timeline.value,
);

const display = computed<DiaryEntry[]>(() =>
  isSearch.value ? searchResults.value : timelineFiltered.value,
);

const loading = computed(() =>
  isSearch.value ? searching.value : diary.loadingTimeline.value,
);
const errorMsg = computed(() =>
  isSearch.value ? searchError.value : diary.lastError.value,
);

const stripExt = (name: string) => name.replace(/\.(md|txt)$/i, "");
const fmtDate = (iso: string) => {
  const t = Date.parse(iso);
  if (!t) return "";
  const d = new Date(t);
  const pad = (n: number) => String(n).padStart(2, "0");
  return `${d.getFullYear()}-${pad(d.getMonth() + 1)}-${pad(d.getDate())} ${pad(d.getHours())}:${pad(d.getMinutes())}`;
};

const retry = () => (isSearch.value ? runSearch() : diary.loadTimeline(true).catch(() => {}));

onUnmounted(() => {
  if (debounceTimer) clearTimeout(debounceTimer);
});
</script>

<template>
  <div class="h-full flex flex-col">
    <!-- Folder filter chips -->
    <div
      v-if="diary.folders.value.length"
      class="px-3 pb-2 flex gap-2 overflow-x-auto no-scrollbar shrink-0"
    >
      <button
        class="px-3 py-1 rounded-full text-xs font-medium whitespace-nowrap transition-all border"
        :class="activeFolder === null ? 'glass-panel-active border-transparent' : 'border-black/10 dark:border-white/10 opacity-60'"
        :style="activeFolder === null ? 'color: var(--highlight-text)' : ''"
        @click="activeFolder = null"
      >
        全部
      </button>
      <button
        v-for="f in diary.folders.value"
        :key="f"
        class="px-3 py-1 rounded-full text-xs font-medium whitespace-nowrap transition-all border"
        :class="activeFolder === f ? 'glass-panel-active border-transparent' : 'border-black/10 dark:border-white/10 opacity-60'"
        :style="activeFolder === f ? 'color: var(--highlight-text)' : ''"
        @click="activeFolder = f"
      >
        {{ f }}
      </button>
    </div>

    <!-- List -->
    <div class="flex-1 overflow-y-auto no-rubber-band px-3 pb-[calc(var(--vcp-safe-bottom,48px)+16px)]">
      <div v-if="loading && !display.length" class="flex flex-col items-center justify-center py-20 gap-3 opacity-40">
        <div class="w-6 h-6 border-2 border-current border-t-transparent rounded-full animate-spin"></div>
        <span class="text-xs">{{ isSearch ? "搜索中…" : "正在加载日记…" }}</span>
      </div>

      <div v-else-if="errorMsg && !display.length" class="flex flex-col items-center justify-center py-16 gap-3 px-6 text-center">
        <span class="text-xs opacity-60 leading-relaxed">{{ errorMsg }}</span>
        <button class="px-4 py-1.5 rounded-full text-xs font-bold glass-panel active:scale-95" @click="retry">重试</button>
      </div>

      <div v-else-if="!display.length" class="flex flex-col items-center justify-center py-20 gap-2 opacity-30 text-center">
        <svg width="26" height="26" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.5">
          <path d="M2 3h6a4 4 0 0 1 4 4v14a3 3 0 0 0-3-3H2z"></path>
          <path d="M22 3h-6a4 4 0 0 0-4 4v14a3 3 0 0 1 3-3h7z"></path>
        </svg>
        <span class="text-xs">{{ isSearch ? "没有匹配的日记" : "暂无日记" }}</span>
      </div>

      <div v-else class="flex flex-col gap-2.5">
        <button
          v-for="e in display"
          :key="e.folder + '/' + e.name"
          class="text-left p-3.5 rounded-2xl glass-panel active:scale-[0.985] transition-transform"
          @click="emit('open', e.folder, e.name)"
        >
          <div class="flex items-center gap-2 mb-1.5">
            <span class="text-[11px] font-bold px-2 py-0.5 rounded-full shrink-0"
              style="background: color-mix(in srgb, var(--highlight-text) 14%, transparent); color: var(--highlight-text)">
              {{ e.folder }}
            </span>
            <span class="text-[11px] opacity-45 truncate">{{ stripExt(e.name) }}</span>
            <span class="text-[10px] opacity-35 ml-auto shrink-0">{{ fmtDate(e.lastModified) }}</span>
          </div>
          <p class="text-[13px] leading-relaxed opacity-80 line-clamp-2 break-words">
            {{ e.preview || "（空）" }}
          </p>
        </button>
      </div>
    </div>
  </div>
</template>

<style scoped>
.no-scrollbar::-webkit-scrollbar { display: none; }
.no-scrollbar { scrollbar-width: none; }
.line-clamp-2 {
  display: -webkit-box;
  -webkit-line-clamp: 2;
  -webkit-box-orient: vertical;
  overflow: hidden;
}
</style>
