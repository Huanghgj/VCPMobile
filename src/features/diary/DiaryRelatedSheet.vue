<script setup lang="ts">
import { computed, watch } from "vue";
import { useModalHistory } from "../../core/composables/useModalHistory";
import { Sparkles, FileText, X } from "lucide-vue-next";

const props = defineProps<{
  modelValue: boolean;
  loading?: boolean;
  result?: any;
}>();
const emit = defineEmits<{
  "update:modelValue": [v: boolean];
  open: [folder: string, file: string];
}>();

const { registerModal, unregisterModal } = useModalHistory();
const modalId = "DiaryRelated";

watch(
  () => props.modelValue,
  (open) => {
    if (open) registerModal(modalId, () => emit("update:modelValue", false));
    else unregisterModal(modalId);
  },
);

const close = () => emit("update:modelValue", false);

interface RelatedItem {
  folder?: string;
  file?: string;
  title: string;
  snippet: string;
  score?: string;
  tappable: boolean;
}

// 服务端 associative-discovery 结构不固定，这里做防御性归一化。
const items = computed<RelatedItem[]>(() => {
  const r = props.result;
  if (!r) return [];
  const arr: any[] = Array.isArray(r)
    ? r
    : r.results || r.related || r.notes || r.matches || r.items || [];
  if (!Array.isArray(arr)) return [];

  return arr.map((it: any) => {
    const rawPath: string =
      it.sourceFilePath || it.path || it.fullPath || it.file || it.sourceFile || it.name || "";
    let folder: string | undefined;
    let file: string | undefined;
    const norm = String(rawPath).replace(/\\/g, "/");
    if (norm.includes("/")) {
      const parts = norm.split("/").filter(Boolean);
      file = parts.pop();
      folder = parts.pop();
    }
    const scoreNum = it.score ?? it.similarity ?? it.rerank_score ?? it.distance ?? it.rrf_score;
    const score = typeof scoreNum === "number" ? scoreNum.toFixed(3) : undefined;
    const title =
      it.title || (file ? file.replace(/\.(md|txt)$/i, "") : "") || norm || "关联条目";
    // associative-discovery 的预览在 chunks 数组里；兼容其它字段名。
    const snippet =
      it.snippet ||
      it.preview ||
      it.text ||
      it.content ||
      it.summary ||
      (Array.isArray(it.chunks) ? it.chunks.join("  …  ") : "");
    return {
      folder,
      file,
      title,
      snippet: String(snippet).slice(0, 200),
      score,
      tappable: !!(folder && file),
    };
  });
});

const onTap = (it: RelatedItem) => {
  if (it.tappable && it.folder && it.file) emit("open", it.folder, it.file);
};
</script>

<template>
  <Teleport to="body">
    <Transition name="fade">
      <div v-if="modelValue" class="fixed inset-0 bg-black/50 z-sheet" @click="close" @touchmove.prevent></div>
    </Transition>
    <Transition name="slide-up">
      <div
        v-if="modelValue"
        class="fixed bottom-0 left-0 right-0 z-sheet bg-white/95 dark:bg-gray-900/95 rounded-t-[2rem] shadow-2xl flex flex-col border-t border-white/20 dark:border-white/5"
        style="padding-bottom: calc(var(--vcp-safe-bottom, 20px) + 16px); max-height: 72vh"
      >
        <div class="w-12 h-1.5 bg-black/10 dark:bg-white/20 rounded-full mx-auto mt-3 mb-2"></div>
        <div class="flex items-center gap-2 px-5 pb-3">
          <Sparkles :size="16" style="color: var(--highlight-text)" />
          <span class="text-sm font-bold text-primary-text">联想追溯</span>
          <button class="ml-auto p-1.5 rounded-lg opacity-50 active:scale-90 hover:bg-black/5 dark:hover:bg-white/5" @click="close">
            <X :size="18" />
          </button>
        </div>

        <div class="px-4 pb-2 overflow-y-auto no-rubber-band">
          <div v-if="props.loading" class="flex flex-col items-center justify-center py-12 gap-3 opacity-50">
            <div class="w-6 h-6 border-2 border-current border-t-transparent rounded-full animate-spin"></div>
            <span class="text-xs">正在做向量语义关联…</span>
          </div>

          <div v-else-if="!items.length" class="flex flex-col items-center justify-center py-12 gap-2 opacity-35 text-center">
            <Sparkles :size="24" />
            <span class="text-xs">没有发现明显关联的日记</span>
          </div>

          <div v-else class="flex flex-col gap-2.5">
            <button
              v-for="(it, i) in items"
              :key="i"
              class="text-left p-3.5 rounded-2xl glass-panel transition-transform"
              :class="it.tappable ? 'active:scale-[0.985]' : 'cursor-default opacity-90'"
              @click="onTap(it)"
            >
              <div class="flex items-center gap-2 mb-1">
                <FileText :size="14" class="opacity-50 shrink-0 text-primary-text" />
                <span v-if="it.folder" class="text-[11px] font-bold px-2 py-0.5 rounded-full shrink-0"
                  style="background: color-mix(in srgb, var(--highlight-text) 14%, transparent); color: var(--highlight-text)">
                  {{ it.folder }}
                </span>
                <span class="text-[12px] font-semibold truncate text-primary-text">{{ it.title }}</span>
                <span v-if="it.score" class="text-[10px] opacity-40 ml-auto shrink-0 font-mono text-primary-text">{{ it.score }}</span>
              </div>
              <p v-if="it.snippet" class="text-[12px] leading-relaxed opacity-65 line-clamp-2 break-words text-primary-text">{{ it.snippet }}</p>
            </button>
          </div>
        </div>
      </div>
    </Transition>
  </Teleport>
</template>

<style scoped>
.line-clamp-2 {
  display: -webkit-box;
  -webkit-line-clamp: 2;
  -webkit-box-orient: vertical;
  overflow: hidden;
}
.fade-enter-active, .fade-leave-active { transition: opacity 0.2s ease; }
.fade-enter-from, .fade-leave-to { opacity: 0; }
.slide-up-enter-active { transition: transform 0.28s cubic-bezier(0.32, 0.72, 0, 1); }
.slide-up-leave-active { transition: transform 0.22s cubic-bezier(0.32, 0.72, 0, 1); }
.slide-up-enter-from, .slide-up-leave-to { transform: translateY(100%); }
</style>
