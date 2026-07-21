<script setup lang="ts">
import { computed, ref, watch } from "vue";
import { useDiary, type DiaryNote, type DiaryNoteRef } from "./useDiary";
import { useOverlayStore } from "../../core/stores/overlay";
import { useNotificationStore } from "../../core/stores/notification";
import type { OverlayActionItem } from "../../core/types/overlay";
import { BookText, CheckCircle2, Circle, FolderInput, Trash2 } from "lucide-vue-next";

const props = defineProps<{ query?: string; currentFolder: string | null }>();
const emit = defineEmits<{
  enterFolder: [folder: string];
  open: [folder: string, file: string];
}>();

const diary = useDiary();
const overlay = useOverlayStore();
const notify = useNotificationStore();

const toast = (type: "success" | "error" | "info", title: string, message = "") =>
  notify.addNotification({ type, title, message, toastOnly: true, duration: 2200 });

// ── 网格（本子列表）──
const filteredFolders = computed(() => {
  const q = (props.query || "").trim().toLowerCase();
  const list = diary.folders.value;
  if (!q) return list;
  return list.filter((f) => f.toLowerCase().includes(q));
});
const folderCount = (f: string) => diary.notesByFolder.value[f]?.length;

const onFolderLongpress = (folder: string) => {
  overlay.openContextMenu(
    [
      {
        label: "删除空本子",
        icon: Trash2,
        danger: true,
        handler: async () => {
          try {
            await diary.deleteFolder(folder);
            toast("success", "已删除本子", folder);
          } catch (e: any) {
            toast("error", "删除失败", typeof e === "string" ? e : e?.message || "");
          }
        },
      },
    ],
    folder,
  );
};

// ── 本子内条目 ──
const notes = computed<DiaryNote[]>(() =>
  props.currentFolder ? diary.notesByFolder.value[props.currentFolder] || [] : [],
);
const folderLoading = computed(() =>
  props.currentFolder ? !!diary.loadingNotes.value[props.currentFolder] : false,
);
const filteredNotes = computed(() => {
  const q = (props.query || "").trim().toLowerCase();
  if (!q) return notes.value;
  return notes.value.filter(
    (n) => n.name.toLowerCase().includes(q) || n.preview.toLowerCase().includes(q),
  );
});

// ── 多选 ──
const selectMode = ref(false);
const selected = ref<Set<string>>(new Set());
const selectedCount = computed(() => selected.value.size);

watch(
  () => props.currentFolder,
  (f) => {
    selectMode.value = false;
    selected.value = new Set();
    if (f) diary.loadNotes(f, true).catch(() => {});
  },
  { immediate: true },
);

const stripExt = (name: string) => name.replace(/\.(md|txt)$/i, "");
const fmtDate = (iso: string) => {
  const t = Date.parse(iso);
  if (!t) return "";
  const d = new Date(t);
  const pad = (n: number) => String(n).padStart(2, "0");
  return `${d.getFullYear()}-${pad(d.getMonth() + 1)}-${pad(d.getDate())}`;
};

const toggle = (file: string) => {
  const s = new Set(selected.value);
  if (s.has(file)) s.delete(file);
  else s.add(file);
  selected.value = s;
  if (s.size === 0) selectMode.value = false;
};
const onNoteTap = (file: string) => {
  if (selectMode.value) toggle(file);
  else if (props.currentFolder) emit("open", props.currentFolder, file);
};
const onNoteLongpress = (file: string) => {
  selectMode.value = true;
  toggle(file);
};
const cancelSelect = () => {
  selectMode.value = false;
  selected.value = new Set();
};

const selectedRefs = (): DiaryNoteRef[] =>
  props.currentFolder
    ? Array.from(selected.value).map((file) => ({ folder: props.currentFolder!, file }))
    : [];

const doDelete = () => {
  const n = selectedCount.value;
  if (!n) return;
  overlay.openContextMenu(
    [
      {
        label: `删除选中的 ${n} 篇`,
        icon: Trash2,
        danger: true,
        handler: async () => {
          try {
            const res = await diary.deleteNotes(selectedRefs());
            cancelSelect();
            if (props.currentFolder) await diary.loadNotes(props.currentFolder, true).catch(() => {});
            await diary.loadFolders(true).catch(() => {});
            toast(
              res.errors.length ? "error" : "success",
              `已删除 ${res.ok.length} 篇`,
              res.errors.length ? `${res.errors.length} 篇失败` : "",
            );
          } catch (e: any) {
            toast("error", "删除失败", typeof e === "string" ? e : e?.message || "");
          }
        },
      },
    ],
    "批量删除",
  );
};

const doMove = () => {
  if (!selectedCount.value) return;
  const targets = diary.folders.value.filter((f) => f !== props.currentFolder);
  const actions: OverlayActionItem[] = targets.map((target) => ({
    label: target,
    icon: BookText,
    handler: () => runMove(target),
  }));
  actions.push({
    label: "＋ 新建本子…",
    icon: FolderInput,
    handler: () =>
      overlay.openPrompt({
        title: "移动到新本子",
        initialValue: "",
        placeholder: "输入新本子名称",
        onConfirm: (val) => {
          const name = val.trim();
          if (name) runMove(name);
        },
      }),
  });
  overlay.openContextMenu(actions, `移动 ${selectedCount.value} 篇到…`);
};

const runMove = async (target: string) => {
  try {
    const res = await diary.moveNotes(selectedRefs(), target);
    cancelSelect();
    if (props.currentFolder) await diary.loadNotes(props.currentFolder, true).catch(() => {});
    await diary.loadFolders(true).catch(() => {});
    toast(
      res.errors.length ? "error" : "success",
      `已移动 ${res.ok.length} 篇`,
      res.errors.length ? `${res.errors.length} 篇失败` : `→ ${target}`,
    );
  } catch (e: any) {
    toast("error", "移动失败", typeof e === "string" ? e : e?.message || "");
  }
};
</script>

<template>
  <div class="h-full">
    <!-- Grid: notebooks -->
    <div
      v-if="!currentFolder"
      class="h-full overflow-y-auto no-rubber-band px-3 pb-[calc(var(--vcp-safe-bottom,48px)+16px)]"
    >
      <div v-if="diary.loadingFolders.value && !filteredFolders.length" class="flex flex-col items-center justify-center py-20 gap-3 opacity-40">
        <div class="w-6 h-6 border-2 border-current border-t-transparent rounded-full animate-spin"></div>
        <span class="text-xs">正在加载本子…</span>
      </div>
      <div v-else-if="!filteredFolders.length" class="flex flex-col items-center justify-center py-20 gap-2 opacity-30 text-center">
        <BookText :size="26" :stroke-width="1.5" />
        <span class="text-xs">{{ props.query ? "没有匹配的本子" : "暂无日记本" }}</span>
      </div>
      <div v-else class="grid grid-cols-2 gap-3">
        <button
          v-for="f in filteredFolders"
          :key="f"
          class="text-left p-4 rounded-2xl glass-panel active:scale-[0.97] transition-transform flex flex-col gap-3 min-h-[96px]"
          @click="emit('enterFolder', f)"
          v-longpress="() => onFolderLongpress(f)"
        >
          <div class="w-9 h-9 rounded-xl flex items-center justify-center"
            style="background: color-mix(in srgb, var(--highlight-text) 12%, transparent); color: var(--highlight-text)">
            <BookText :size="18" />
          </div>
          <div class="min-w-0">
            <div class="text-sm font-bold truncate">{{ f }}</div>
            <div class="text-[11px] opacity-45 mt-0.5">
              {{ folderCount(f) !== undefined ? folderCount(f) + " 篇" : "—" }}
            </div>
          </div>
        </button>
      </div>
    </div>

    <!-- Folder notes -->
    <div v-else class="h-full flex flex-col">
      <div class="flex-1 overflow-y-auto no-rubber-band px-3 pb-[calc(var(--vcp-safe-bottom,48px)+72px)]">
        <div v-if="folderLoading && !filteredNotes.length" class="flex flex-col items-center justify-center py-20 gap-3 opacity-40">
          <div class="w-6 h-6 border-2 border-current border-t-transparent rounded-full animate-spin"></div>
          <span class="text-xs">正在加载…</span>
        </div>
        <div v-else-if="!filteredNotes.length" class="flex flex-col items-center justify-center py-20 gap-2 opacity-30 text-center">
          <BookText :size="26" :stroke-width="1.5" />
          <span class="text-xs">{{ props.query ? "没有匹配的日记" : "这个本子还没有日记" }}</span>
        </div>
        <div v-else class="flex flex-col gap-2.5">
          <button
            v-for="n in filteredNotes"
            :key="n.name"
            class="text-left p-3.5 rounded-2xl glass-panel active:scale-[0.985] transition-transform flex items-start gap-3"
            :class="{ 'glass-panel-active': selectMode && selected.has(n.name) }"
            @click="onNoteTap(n.name)"
            v-longpress="() => onNoteLongpress(n.name)"
          >
            <component
              v-if="selectMode"
              :is="selected.has(n.name) ? CheckCircle2 : Circle"
              :size="20"
              class="shrink-0 mt-0.5"
              :style="selected.has(n.name) ? 'color: var(--highlight-text)' : 'opacity:0.4'"
            />
            <div class="min-w-0 flex-1">
              <div class="flex items-center gap-2 mb-1">
                <span class="text-[13px] font-semibold truncate">{{ stripExt(n.name) }}</span>
                <span class="text-[10px] opacity-35 ml-auto shrink-0">{{ fmtDate(n.lastModified) }}</span>
              </div>
              <p class="text-[12.5px] leading-relaxed opacity-70 line-clamp-2 break-words">{{ n.preview || "（空）" }}</p>
            </div>
          </button>
        </div>
      </div>

      <!-- Multi-select action bar -->
      <Transition name="bar">
        <div
          v-if="selectMode"
          class="absolute left-0 right-0 bottom-0 px-4 pt-3 pb-[calc(var(--vcp-safe-bottom,48px)+10px)] glass-panel border-t border-black/5 dark:border-white/5 flex items-center gap-3"
        >
          <span class="text-xs font-bold opacity-70">已选 {{ selectedCount }}</span>
          <div class="flex-1"></div>
          <button class="px-3 py-2 rounded-xl text-xs font-bold flex items-center gap-1.5 bg-black/5 dark:bg-white/5 active:scale-95" @click="doMove">
            <FolderInput :size="15" /> 移动
          </button>
          <button class="px-3 py-2 rounded-xl text-xs font-bold flex items-center gap-1.5 text-red-500 bg-red-500/10 active:scale-95" @click="doDelete">
            <Trash2 :size="15" /> 删除
          </button>
          <button class="px-3 py-2 rounded-xl text-xs font-bold opacity-60 active:scale-95" @click="cancelSelect">取消</button>
        </div>
      </Transition>
    </div>
  </div>
</template>

<style scoped>
.line-clamp-2 {
  display: -webkit-box;
  -webkit-line-clamp: 2;
  -webkit-box-orient: vertical;
  overflow: hidden;
}
.bar-enter-active, .bar-leave-active { transition: transform 0.25s ease, opacity 0.25s ease; }
.bar-enter-from, .bar-leave-to { transform: translateY(100%); opacity: 0; }
</style>
