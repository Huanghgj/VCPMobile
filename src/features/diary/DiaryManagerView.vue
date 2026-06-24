<script setup lang="ts">
import { ref, computed, watch, onMounted, onUnmounted } from "vue";
import SlidePage from "../../components/ui/SlidePage.vue";
import { useModalHistory } from "../../core/composables/useModalHistory";
import { X, ChevronLeft, Search, Plus, Clock, BookText } from "lucide-vue-next";
import DiaryTimeline from "./DiaryTimeline.vue";
import DiaryNotebooks from "./DiaryNotebooks.vue";
import DiaryNoteEditor from "./DiaryNoteEditor.vue";
import { useDiary } from "./useDiary";

const props = withDefaults(
  defineProps<{ isOpen?: boolean; zIndex?: number }>(),
  { isOpen: false, zIndex: 40 },
);
const emit = defineEmits<{ close: [] }>();

const { registerModal, unregisterModal } = useModalHistory();
const diary = useDiary();

type Mode = "timeline" | "notebooks";
const mode = ref<Mode>("timeline");
const query = ref("");
const currentFolder = ref<string | null>(null);
const openEntry = ref<{ folder: string; file: string } | null>(null);
const editorMode = ref<"view" | "new">("view");
const refreshKey = ref(0);

// ── 内部导航（与 modalStack 集成，硬件返回逐级关闭）──
const enterFolder = (folder: string) => {
  currentFolder.value = folder;
  query.value = "";
  registerModal("Diary:folder", () => {
    currentFolder.value = null;
  });
};
const exitFolder = () => {
  unregisterModal("Diary:folder");
  currentFolder.value = null;
};
const openNote = (folder: string, file: string) => {
  editorMode.value = "view";
  openEntry.value = { folder, file };
  registerModal("Diary:entry", () => {
    openEntry.value = null;
  });
};
const openNew = () => {
  editorMode.value = "new";
  openEntry.value = { folder: currentFolder.value || diary.folders.value[0] || "", file: "" };
  registerModal("Diary:entry", () => {
    openEntry.value = null;
  });
};
const closeEntry = () => {
  unregisterModal("Diary:entry");
  openEntry.value = null;
};

const hasInternalBack = computed(() => mode.value === "notebooks" && !!currentFolder.value);

const onHeaderBack = () => {
  if (hasInternalBack.value) exitFolder();
  else emit("close");
};

const switchMode = (m: Mode) => {
  if (mode.value === m) return;
  if (currentFolder.value) exitFolder();
  query.value = "";
  mode.value = m;
};

const headerTitle = computed(() => {
  if (mode.value === "notebooks" && currentFolder.value) return currentFolder.value;
  return "日记本";
});

const refreshDiary = async () => {
  await diary.loadTimeline(true).catch(() => {});
  if (currentFolder.value) {
    await diary.loadNotes(currentFolder.value, true).catch(() => {});
  }
};

const onRemoteDiaryChanged = () => {
  diary.invalidateAll();
  if (props.isOpen) {
    refreshKey.value++;
    refreshDiary();
  }
};

// 打开页面时强制拉新；服务端 DailyNote 可能在页面外部写入，不能复用旧缓存。
watch(
  () => props.isOpen,
  (open) => {
    if (open) {
      diary.invalidateAll();
      refreshDiary();
    }
  },
  { immediate: true },
);

const onEntryChanged = () => {
  refreshDiary();
};

onMounted(() => {
  window.addEventListener("vcp-diary-changed", onRemoteDiaryChanged);
});

onUnmounted(() => {
  window.removeEventListener("vcp-diary-changed", onRemoteDiaryChanged);
});
</script>

<template>
  <SlidePage :is-open="props.isOpen" :z-index="props.zIndex">
    <div class="flex flex-col h-full w-full bg-[var(--primary-bg)] text-primary-text pointer-events-auto">
      <!-- Header -->
      <header
        class="px-3 flex items-center gap-2 border-b border-black/5 dark:border-white/5 pt-[calc(var(--vcp-safe-top,24px)+10px)] pb-2.5 shrink-0"
      >
        <button
          @click="onHeaderBack"
          class="p-2 -ml-1 active:scale-90 transition-all rounded-xl hover:bg-black/5 dark:hover:bg-white/5 opacity-80"
        >
          <ChevronLeft v-if="hasInternalBack" :size="22" />
          <X v-else :size="22" />
        </button>
        <h2 class="text-lg font-bold tracking-tight truncate flex-1">{{ headerTitle }}</h2>
        <button
          @click="openNew"
          class="p-2 active:scale-90 transition-all rounded-xl hover:bg-black/5 dark:hover:bg-white/5"
          style="color: var(--highlight-text)"
          title="新建日记"
        >
          <Plus :size="22" />
        </button>
      </header>

      <!-- Mode toggle (hidden when drilled into a folder) -->
      <div v-if="!hasInternalBack" class="px-3 pt-3 shrink-0">
        <div class="flex gap-1 p-1 rounded-2xl bg-black/5 dark:bg-white/5">
          <button
            class="flex-1 py-1.5 rounded-xl text-xs font-bold flex items-center justify-center gap-1.5 transition-all"
            :class="mode === 'timeline' ? 'glass-panel-active shadow-sm' : 'opacity-60'"
            :style="mode === 'timeline' ? 'color: var(--highlight-text)' : ''"
            @click="switchMode('timeline')"
          >
            <Clock :size="14" /> 时间线
          </button>
          <button
            class="flex-1 py-1.5 rounded-xl text-xs font-bold flex items-center justify-center gap-1.5 transition-all"
            :class="mode === 'notebooks' ? 'glass-panel-active shadow-sm' : 'opacity-60'"
            :style="mode === 'notebooks' ? 'color: var(--highlight-text)' : ''"
            @click="switchMode('notebooks')"
          >
            <BookText :size="14" /> 本子
          </button>
        </div>
      </div>

      <!-- Search -->
      <div class="px-3 py-3 shrink-0">
        <div class="flex items-center gap-2 px-3 py-2 rounded-xl bg-black/5 dark:bg-white/5">
          <Search :size="16" class="opacity-40 shrink-0" />
          <input
            v-model="query"
            type="text"
            :placeholder="mode === 'notebooks' && !currentFolder ? '搜索本子…' : '搜索日记内容…'"
            class="flex-1 bg-transparent outline-none text-sm placeholder:opacity-40"
          />
          <button v-if="query" @click="query = ''" class="opacity-50 active:scale-90">
            <X :size="15" />
          </button>
        </div>
      </div>

      <!-- Body -->
      <div class="flex-1 min-h-0 relative">
        <DiaryTimeline
          v-show="mode === 'timeline'"
          :query="query"
          :refresh-key="refreshKey"
          @open="openNote"
        />
        <DiaryNotebooks
          v-show="mode === 'notebooks'"
          :query="query"
          :current-folder="currentFolder"
          @enter-folder="enterFolder"
          @open="openNote"
        />
      </div>
    </div>

    <!-- Editor (nested page slides over) -->
    <DiaryNoteEditor
      :is-open="!!openEntry"
      :z-index="props.zIndex + 1"
      :mode="editorMode"
      :folder="openEntry?.folder || ''"
      :file="openEntry?.file || ''"
      @close="closeEntry"
      @open="openNote"
      @changed="onEntryChanged"
    />
  </SlidePage>
</template>
