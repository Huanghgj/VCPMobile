<script setup lang="ts">
import { ref, computed, watch } from "vue";
import SlidePage from "../../components/ui/SlidePage.vue";
import { useDiary, type DiaryNoteRef } from "./useDiary";
import { useOverlayStore } from "../../core/stores/overlay";
import { useNotificationStore } from "../../core/stores/notification";
import { renderSafeMarkdown } from "../../core/utils/safeMarkdown";
import DiaryRelatedSheet from "./DiaryRelatedSheet.vue";
import type { OverlayActionItem } from "../../core/types/overlay";
import {
  ChevronLeft, Pencil, Save, Trash2, FolderInput,
  Sparkles, BookText, Type,
} from "lucide-vue-next";

const props = withDefaults(
  defineProps<{
    isOpen: boolean;
    zIndex: number;
    mode: "view" | "new";
    folder: string;
    file: string;
  }>(),
  { mode: "view" },
);
const emit = defineEmits<{
  close: [];
  open: [folder: string, file: string];
  changed: [];
}>();

const diary = useDiary();
const overlay = useOverlayStore();
const notify = useNotificationStore();
const toast = (type: "success" | "error" | "info", title: string, message = "") =>
  notify.addNotification({ type, title, message, toastOnly: true, duration: 2200 });

const localMode = ref<"view" | "new">("view");
const editing = ref(false);
const loading = ref(false);
const saving = ref(false);
const draft = ref("");
const original = ref("");
const newFolder = ref("");
const newFile = ref("");
const relatedOpen = ref(false);
const relatedResult = ref<any>(null);
const relatedLoading = ref(false);

const defaultFileName = () => {
  const d = new Date();
  const pad = (n: number) => String(n).padStart(2, "0");
  return `${d.getFullYear()}.${pad(d.getMonth() + 1)}.${pad(d.getDate())}.txt`;
};

const titleText = computed(() => {
  if (localMode.value === "new") return "新建日记";
  return props.file.replace(/\.(md|txt)$/i, "");
});
const renderedHtml = computed(() => renderSafeMarkdown(draft.value || ""));
const dirty = computed(() => editing.value && draft.value !== original.value);
const canSaveNew = computed(() => !!newFolder.value.trim() && !!newFile.value.trim());

watch(
  () => [props.isOpen, props.mode, props.folder, props.file],
  async () => {
    if (!props.isOpen) return;
    localMode.value = props.mode;
    relatedOpen.value = false;
    if (props.mode === "new") {
      editing.value = true;
      draft.value = "";
      original.value = "";
      newFolder.value = props.folder || "";
      newFile.value = defaultFileName();
      return;
    }
    // view mode
    editing.value = false;
    loading.value = true;
    try {
      const c = await diary.readNote(props.folder, props.file);
      draft.value = c;
      original.value = c;
    } catch (e: any) {
      toast("error", "读取失败", typeof e === "string" ? e : e?.message || "");
      draft.value = "";
      original.value = "";
    } finally {
      loading.value = false;
    }
  },
  { immediate: true },
);

const pickFolder = () => {
  const actions: OverlayActionItem[] = diary.folders.value.map((f) => ({
    label: f,
    icon: BookText,
    handler: () => (newFolder.value = f),
  }));
  actions.push({
    label: "＋ 新建本子…",
    icon: FolderInput,
    handler: () =>
      overlay.openPrompt({
        title: "新建本子",
        initialValue: "",
        placeholder: "输入本子名称",
        onConfirm: (val) => {
          const name = val.trim();
          if (name) newFolder.value = name;
        },
      }),
  });
  overlay.openContextMenu(actions, "选择本子");
};

const saveNew = async () => {
  if (!canSaveNew.value || saving.value) return;
  let file = newFile.value.trim();
  if (!/\.(md|txt)$/i.test(file)) file += ".txt";
  saving.value = true;
  try {
    await diary.saveNote(newFolder.value.trim(), file, draft.value);
    await diary.loadFolders(true).catch(() => {});
    emit("changed");
    toast("success", "已创建", `${newFolder.value.trim()} / ${file}`);
    emit("close");
  } catch (e: any) {
    toast("error", "创建失败", typeof e === "string" ? e : e?.message || "");
  } finally {
    saving.value = false;
  }
};

const saveEdit = async () => {
  if (saving.value) return;
  saving.value = true;
  try {
    await diary.saveNote(props.folder, props.file, draft.value);
    original.value = draft.value;
    editing.value = false;
    emit("changed");
    toast("success", "已保存");
  } catch (e: any) {
    toast("error", "保存失败", typeof e === "string" ? e : e?.message || "");
  } finally {
    saving.value = false;
  }
};

const rename = () => {
  overlay.openPrompt({
    title: "重命名日记",
    initialValue: props.file,
    placeholder: "新文件名（含 .txt/.md）",
    onConfirm: async (val) => {
      let name = val.trim();
      if (!name || name === props.file) return;
      if (!/\.(md|txt)$/i.test(name)) name += ".txt";
      try {
        await diary.renameNote(props.folder, props.file, name, draft.value);
        emit("changed");
        toast("success", "已重命名", name);
        emit("open", props.folder, name); // 以新名重开
      } catch (e: any) {
        toast("error", "重命名失败", typeof e === "string" ? e : e?.message || "");
      }
    },
  });
};

const move = () => {
  const targets = diary.folders.value.filter((f) => f !== props.folder);
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
  overlay.openContextMenu(actions, "移动到…");
};
const runMove = async (target: string) => {
  const refs: DiaryNoteRef[] = [{ folder: props.folder, file: props.file }];
  try {
    const res = await diary.moveNotes(refs, target);
    emit("changed");
    if (res.errors.length) toast("error", "移动失败", res.errors[0]?.error || "");
    else {
      toast("success", "已移动", `→ ${target}`);
      emit("close");
    }
  } catch (e: any) {
    toast("error", "移动失败", typeof e === "string" ? e : e?.message || "");
  }
};

const remove = () => {
  overlay.openContextMenu(
    [
      {
        label: "删除这篇日记",
        icon: Trash2,
        danger: true,
        handler: async () => {
          try {
            await diary.deleteNotes([{ folder: props.folder, file: props.file }]);
            await diary.loadFolders(true).catch(() => {});
            emit("changed");
            toast("success", "已删除");
            emit("close");
          } catch (e: any) {
            toast("error", "删除失败", typeof e === "string" ? e : e?.message || "");
          }
        },
      },
    ],
    props.file,
  );
};

const discover = async () => {
  relatedLoading.value = true;
  relatedResult.value = null;
  relatedOpen.value = true;
  try {
    relatedResult.value = await diary.associativeDiscovery(`${props.folder}/${props.file}`);
  } catch (e: any) {
    toast("error", "联想追溯失败", typeof e === "string" ? e : e?.message || "");
    relatedOpen.value = false;
  } finally {
    relatedLoading.value = false;
  }
};

const onRelatedOpen = (folder: string, file: string) => {
  relatedOpen.value = false;
  emit("open", folder, file);
};
</script>

<template>
  <SlidePage :is-open="props.isOpen" :z-index="props.zIndex">
    <div class="flex flex-col h-full w-full bg-[var(--primary-bg)] text-primary-text pointer-events-auto">
      <!-- Header -->
      <header class="px-3 flex items-center gap-1 border-b border-black/5 dark:border-white/5 pt-[calc(var(--vcp-safe-top,24px)+10px)] pb-2.5 shrink-0">
        <button @click="emit('close')" class="p-2 -ml-1 active:scale-90 rounded-xl hover:bg-black/5 dark:hover:bg-white/5 opacity-80">
          <ChevronLeft :size="22" />
        </button>
        <div class="flex-1 min-w-0">
          <h2 class="text-base font-bold truncate leading-tight">{{ titleText }}</h2>
          <div v-if="localMode === 'view'" class="text-[11px] opacity-45 truncate">{{ props.folder }}</div>
        </div>

        <!-- view actions -->
        <template v-if="localMode === 'view'">
          <button
            v-if="editing"
            @click="saveEdit"
            :disabled="saving || !dirty"
            class="px-3 py-1.5 rounded-xl text-xs font-bold flex items-center gap-1.5 active:scale-95 disabled:opacity-40"
            style="background: color-mix(in srgb, var(--highlight-text) 16%, transparent); color: var(--highlight-text)"
          >
            <Save :size="15" /> 保存
          </button>
          <button
            v-else
            @click="editing = true"
            class="p-2 active:scale-90 rounded-xl hover:bg-black/5 dark:hover:bg-white/5 opacity-80"
            title="编辑"
          >
            <Pencil :size="19" />
          </button>
        </template>

        <!-- new: save -->
        <button
          v-else
          @click="saveNew"
          :disabled="!canSaveNew || saving"
          class="px-3 py-1.5 rounded-xl text-xs font-bold flex items-center gap-1.5 active:scale-95 disabled:opacity-40"
          style="background: color-mix(in srgb, var(--highlight-text) 16%, transparent); color: var(--highlight-text)"
        >
          <Save :size="15" /> 创建
        </button>
      </header>

      <!-- New-mode meta inputs -->
      <div v-if="localMode === 'new'" class="px-3 pt-3 shrink-0 flex flex-col gap-2">
        <button
          class="flex items-center gap-2 px-3 py-2.5 rounded-xl bg-black/5 dark:bg-white/5 text-sm active:scale-[0.99]"
          @click="pickFolder"
        >
          <BookText :size="16" class="opacity-50 shrink-0" />
          <span v-if="newFolder" class="font-medium truncate">{{ newFolder }}</span>
          <span v-else class="opacity-40">选择/新建本子</span>
        </button>
        <div class="flex items-center gap-2 px-3 py-2.5 rounded-xl bg-black/5 dark:bg-white/5">
          <Type :size="16" class="opacity-50 shrink-0" />
          <input v-model="newFile" placeholder="文件名" class="flex-1 bg-transparent outline-none text-sm" />
        </div>
      </div>

      <!-- Body -->
      <div class="flex-1 min-h-0 overflow-y-auto no-rubber-band px-4 py-4 pb-[calc(var(--vcp-safe-bottom,48px)+16px)]">
        <div v-if="loading" class="flex flex-col items-center justify-center py-20 gap-3 opacity-40">
          <div class="w-6 h-6 border-2 border-current border-t-transparent rounded-full animate-spin"></div>
          <span class="text-xs">读取中…</span>
        </div>
        <textarea
          v-else-if="editing || localMode === 'new'"
          v-model="draft"
          class="w-full min-h-[60vh] bg-transparent outline-none resize-none text-[14px] leading-relaxed no-swipe"
          placeholder="在这里书写日记…（支持 Markdown）"
        ></textarea>
        <div v-else class="vcp-diary-prose text-[14px] leading-relaxed break-words" v-html="renderedHtml"></div>
      </div>

      <!-- Footer actions (view mode) -->
      <div
        v-if="localMode === 'view' && !editing"
        class="px-3 pt-2.5 pb-[calc(var(--vcp-safe-bottom,48px)+10px)] border-t border-black/5 dark:border-white/5 flex items-center gap-2 shrink-0"
      >
        <button class="flex-1 py-2.5 rounded-xl text-xs font-bold flex items-center justify-center gap-1.5 bg-black/5 dark:bg-white/5 active:scale-95" @click="discover">
          <Sparkles :size="15" style="color: var(--highlight-text)" /> 联想追溯
        </button>
        <button class="px-3 py-2.5 rounded-xl text-xs font-bold flex items-center gap-1.5 bg-black/5 dark:bg-white/5 active:scale-95" @click="rename">
          <Pencil :size="15" /> 改名
        </button>
        <button class="px-3 py-2.5 rounded-xl text-xs font-bold flex items-center gap-1.5 bg-black/5 dark:bg-white/5 active:scale-95" @click="move">
          <FolderInput :size="15" /> 移动
        </button>
        <button class="px-3 py-2.5 rounded-xl text-xs font-bold flex items-center gap-1.5 text-red-500 bg-red-500/10 active:scale-95" @click="remove">
          <Trash2 :size="15" />
        </button>
      </div>
    </div>

    <DiaryRelatedSheet
      v-model="relatedOpen"
      :loading="relatedLoading"
      :result="relatedResult"
      @open="onRelatedOpen"
    />
  </SlidePage>
</template>

<style scoped>
.vcp-diary-prose :deep(h1),
.vcp-diary-prose :deep(h2),
.vcp-diary-prose :deep(h3) { font-weight: 700; margin: 0.8em 0 0.4em; line-height: 1.3; }
.vcp-diary-prose :deep(h1) { font-size: 1.3em; }
.vcp-diary-prose :deep(h2) { font-size: 1.15em; }
.vcp-diary-prose :deep(p) { margin: 0.5em 0; }
.vcp-diary-prose :deep(ul),
.vcp-diary-prose :deep(ol) { padding-left: 1.4em; margin: 0.5em 0; }
.vcp-diary-prose :deep(li) { margin: 0.2em 0; }
.vcp-diary-prose :deep(code) {
  background: rgba(127, 127, 127, 0.15);
  padding: 0.1em 0.35em; border-radius: 6px; font-size: 0.9em;
}
.vcp-diary-prose :deep(pre) {
  background: rgba(127, 127, 127, 0.12);
  padding: 0.8em; border-radius: 10px; overflow-x: auto; margin: 0.6em 0;
}
.vcp-diary-prose :deep(blockquote) {
  border-left: 3px solid color-mix(in srgb, var(--highlight-text) 50%, transparent);
  padding-left: 0.8em; opacity: 0.8; margin: 0.6em 0;
}
.vcp-diary-prose :deep(a) { color: var(--highlight-text); text-decoration: underline; }
.vcp-diary-prose :deep(img) { max-width: 100%; border-radius: 10px; }
</style>
