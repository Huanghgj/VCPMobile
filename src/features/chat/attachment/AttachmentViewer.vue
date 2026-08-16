<script setup lang="ts">
import { computed, watch, ref } from "vue";
import { useModalHistory } from "../../../core/composables/useModalHistory";
import { convertFileSrc, invoke } from "@tauri-apps/api/core";
import { X, ExternalLink, Download } from "lucide-vue-next";
import { saveImageToGallery } from "tauri-plugin-vcp-mobile";
import { useNotificationStore } from "../../../core/stores/notification";
import type { Attachment } from "../../../core/types/chat";

const props = defineProps<{
  file: Attachment | null;
  isOpen: boolean;
}>();

const emit = defineEmits(["close", "open-external"]);

const { registerModal, unregisterModal } = useModalHistory();
const modalId = "AttachmentViewer";
const notificationStore = useNotificationStore();

const previewText = ref("");
const isTextTruncated = ref(false);
const isLoading = ref(false);
const fullResolutionSrc = ref("");
const isFullImageLoading = ref(false);
const mediaError = ref("");
const videoRef = ref<HTMLVideoElement | null>(null);
const audioRef = ref<HTMLAudioElement | null>(null);

const IMAGE_WHITELIST = [
  "jpg",
  "jpeg",
  "png",
  "gif",
  "webp",
  "svg",
  "bmp",
  "heic",
  "heif",
  "avif",
];
const TEXT_WHITELIST = [
  "txt",
  "md",
  "csv",
  "json",
  "js",
  "ts",
  "py",
  "rs",
  "java",
  "c",
  "cpp",
  "h",
  "go",
  "rb",
  "php",
  "swift",
  "kt",
  "html",
  "css",
  "xml",
  "yaml",
  "yml",
  "toml",
  "ini",
  "log",
  "sql",
  "vue",
  "jsx",
  "tsx",
];

const isImage = computed(() => {
  if (!props.file) return false;
  const ext = props.file.name.split(".").pop()?.toLowerCase() || "";
  return (
    IMAGE_WHITELIST.includes(ext) ||
    (props.file.type || "").startsWith("image/")
  );
});

const isText = computed(() => {
  if (!props.file) return false;
  const ext = props.file.name.split(".").pop()?.toLowerCase() || "";

  // 核心加固：若存在后缀且完全不属于文本白名单，绝不判定为文本（与 Preview 判定主权一致）
  if (ext && !TEXT_WHITELIST.includes(ext)) {
    return false;
  }

  if (TEXT_WHITELIST.includes(ext)) {
    return true;
  }

  const type = (props.file.type || "").toLowerCase();
  return (
    type.startsWith("text/") ||
    type === "application/json" ||
    type === "application/javascript" ||
    type === "application/x-javascript"
  );
});

watch(
  () => props.isOpen,
  async (newVal) => {
    if (newVal) {
      registerModal(modalId, close);
      previewText.value = "";
      isTextTruncated.value = false;
      fullResolutionSrc.value = "";
      mediaError.value = "";

      if (isImage.value && props.file) {
        const sourcePath = props.file.internalPath || props.file.src;
        if (sourcePath) {
          if (
            sourcePath.startsWith("data:image/") ||
            sourcePath.startsWith("http:") ||
            sourcePath.startsWith("https:") ||
            sourcePath.startsWith("blob:")
          ) {
            fullResolutionSrc.value = sourcePath;
          } else {
            isFullImageLoading.value = true;
            try {
              fullResolutionSrc.value = await invoke<string>(
                "read_image_preview_data_url",
                {
                  path: sourcePath,
                  maxBytes: 12 * 1024 * 1024,
                },
              );
            } catch (e) {
              console.warn(
                "[AttachmentViewer] Failed to load full-resolution image:",
                e,
              );
            } finally {
              isFullImageLoading.value = false;
            }
          }
        }
      }

      // 如果是可预览的文本，开始流式读取物理文件的前 128KB 进行预览
      if (isText.value && props.file) {
        isLoading.value = true;
        try {
          const sourcePath = props.file.internalPath || props.file.src;
          if (sourcePath) {
            let fetchUrl = sourcePath;
            if (
              !sourcePath.startsWith("http") &&
              !sourcePath.startsWith("blob:") &&
              !sourcePath.startsWith("data:")
            ) {
              fetchUrl = convertFileSrc(sourcePath.replace("file://", ""));
            }

            const response = await fetch(fetchUrl);
            const reader = response.body?.getReader();
            if (reader) {
              const chunks: Uint8Array[] = [];
              let receivedLength = 0;
              const LIMIT = 128 * 1024; // 128KB

              while (receivedLength < LIMIT) {
                const { done, value } = await reader.read();
                if (done) break;
                chunks.push(value);
                receivedLength += value.length;
              }

              // 合并并解码为 UTF-8
              const allChunks = new Uint8Array(receivedLength);
              let position = 0;
              for (const chunk of chunks) {
                allChunks.set(chunk, position);
                position += chunk.length;
              }

              previewText.value = new TextDecoder("utf-8").decode(allChunks);
              if (receivedLength >= LIMIT) {
                isTextTruncated.value = true;
              }
            }
          }
        } catch (e) {
          console.error("[AttachmentViewer] Failed to load text preview:", e);
          previewText.value = "⚠️ 本地文件预览失败，请使用外部应用打开。";
        } finally {
          isLoading.value = false;
        }
      }
    } else {
      videoRef.value?.pause();
      audioRef.value?.pause();
      unregisterModal(modalId);
      previewText.value = "";
      isTextTruncated.value = false;
      fullResolutionSrc.value = "";
      isFullImageLoading.value = false;
    }
  },
);

const renderSrc = computed(() => {
  const source =
    fullResolutionSrc.value ||
    props.file?.resolvedSrc ||
    props.file?.internalPath ||
    props.file?.src;
  if (!source) return "";
  if (
    source.startsWith("http:") ||
    source.startsWith("https:") ||
    source.startsWith("data:") ||
    source.startsWith("blob:") ||
    source.startsWith("asset:") ||
    source.startsWith("tauri:")
  )
    return source;
  try {
    return convertFileSrc(source.replace("file://", ""));
  } catch {
    return "";
  }
});

const isVideo = computed(() => {
  if (!props.file) return false;
  const ext = props.file.name.split(".").pop()?.toLowerCase() || "";
  return (
    ["mp4", "m4v", "webm", "mov", "mkv", "avi"].includes(ext) ||
    (props.file.type || "").startsWith("video/")
  );
});

const isAudio = computed(() => {
  if (!props.file) return false;
  const ext = props.file.name.split(".").pop()?.toLowerCase() || "";
  return (
    ["mp3", "m4a", "aac", "wav", "ogg", "flac", "opus"].includes(ext) ||
    (props.file.type || "").startsWith("audio/")
  );
});

const handleImageError = () => {
  if (fullResolutionSrc.value) {
    fullResolutionSrc.value = "";
  }
};

const handleMediaError = () => {
  mediaError.value = "当前媒体无法读取，可尝试使用外部应用打开。";
};

const close = () => {
  videoRef.value?.pause();
  audioRef.value?.pause();
  emit("close");
};

const isSaving = ref(false);

const saveToAlbum = async () => {
  if (!props.file || isSaving.value) return;
  isSaving.value = true;
  try {
    const sourceUrl = props.file.internalPath || props.file.src;
    if (!sourceUrl) {
      throw new Error("图片路径为空");
    }
    await saveImageToGallery(sourceUrl, props.file.name);
    notificationStore.addNotification({
      type: "success",
      title: "保存成功",
      message: "图片已保存到相册",
      toastOnly: true,
    });
  } catch (err) {
    console.error("[AttachmentViewer] Failed to save image:", err);
    const isAndroid = navigator.userAgent.toLowerCase().includes("android");
    notificationStore.addNotification({
      type: "error",
      title: "保存失败",
      message: isAndroid ? String(err) : "非 Android 环境，暂不支持保存到相册",
      toastOnly: true,
    });
  } finally {
    isSaving.value = false;
  }
};
</script>

<template>
  <Transition name="viewer-fade">
    <div
      v-show="isOpen && file"
      class="vcp-attachment-viewer fixed inset-0 z-viewer flex flex-col pointer-events-auto"
      @click.self="close"
    >
      <!-- Toolbar -->
      <div
        class="flex items-center justify-between px-4 pt-[calc(var(--vcp-safe-top,24px)+8px)] pb-3 bg-white/80 dark:bg-gray-900/80 border-b border-black/5 dark:border-white/5 shrink-0 shadow-sm z-10"
      >
        <div class="flex flex-col overflow-hidden mr-4 min-w-0">
          <span
            class="text-sm font-bold text-gray-800 dark:text-gray-200 truncate"
            >{{ file?.name }}</span
          >
          <span
            class="text-[10px] text-gray-400 dark:text-gray-500 uppercase tracking-widest"
            >{{ file?.type }}</span
          >
        </div>
        <div class="flex items-center gap-1">
          <button
            v-if="isImage"
            @click="saveToAlbum"
            :disabled="isSaving"
            class="p-2 -mr-1 rounded-full text-gray-500 hover:text-gray-800 dark:text-gray-400 dark:hover:text-gray-200 transition-colors active:bg-black/5 dark:active:bg-white/5 disabled:opacity-40"
            title="保存到相册"
          >
            <Download :size="20" />
          </button>
          <button
            @click="$emit('open-external', file?.internalPath || file?.src)"
            class="p-2 -mr-1 rounded-full text-gray-500 hover:text-gray-800 dark:text-gray-400 dark:hover:text-gray-200 transition-colors active:bg-black/5 dark:active:bg-white/5"
          >
            <ExternalLink :size="20" />
          </button>
          <button
            @click="close"
            class="p-2 -mr-2 rounded-full text-gray-500 hover:text-gray-800 dark:text-gray-400 dark:hover:text-gray-200 transition-colors active:bg-black/5 dark:active:bg-white/5"
          >
            <X :size="24" />
          </button>
        </div>
      </div>

      <!-- Main Content -->
      <div
        class="flex-1 overflow-auto vcp-scrollable no-rubber-band pb-[var(--vcp-safe-bottom,0px)]"
      >
        <!-- Text/Code/MD Viewer -->
        <div
          v-if="isText"
          class="w-full px-4 py-4 flex flex-col gap-3 min-h-full"
        >
          <!-- 截断友好提示 -->
          <div
            v-if="isTextTruncated"
            class="px-3 py-2 bg-yellow-500/10 border border-yellow-500/20 text-yellow-700 dark:text-yellow-400 text-xs rounded-lg flex items-center justify-between shrink-0"
          >
            <span
              >📄 当前文件过大，已自动为您预览前
              128KB。要阅读全文，请使用外部应用打开。</span
            >
            <button
              @click="
                $emit(
                  'open-external',
                  props.file?.internalPath || props.file?.src,
                )
              "
              class="px-2 py-1 bg-yellow-500/20 hover:bg-yellow-500/30 rounded text-[10px] font-bold active:scale-95 transition-transform shrink-0 ml-2"
            >
              外部打开
            </button>
          </div>

          <!-- Loading state -->
          <div
            v-if="isLoading"
            class="flex-1 flex items-center justify-center p-12"
          >
            <span class="text-xs text-gray-400 animate-pulse font-mono"
              >正在加载预览流...</span
            >
          </div>

          <!-- Text content container -->
          <pre
            v-else
            class="flex-1 font-mono text-[13px] whitespace-pre-wrap select-text leading-relaxed opacity-90 p-4 break-all overflow-x-auto"
            >{{ previewText }}</pre
          >
        </div>

        <!-- Image Viewer -->
        <div
          v-else-if="isImage"
          class="relative h-full w-full flex items-center justify-center p-4"
        >
          <img
            :src="renderSrc"
            class="max-w-full max-h-full object-contain rounded-lg shadow-2xl animate-zoom-in"
            @click.stop
            @error="handleImageError"
          />
          <div
            v-if="isFullImageLoading"
            class="absolute bottom-[calc(var(--vcp-safe-bottom,0px)+24px)] px-3 py-1.5 rounded-full bg-black/55 text-white text-xs"
          >
            正在加载原图…
          </div>
        </div>

        <div
          v-else-if="isVideo"
          class="relative h-full w-full flex flex-col items-center justify-center gap-4 p-4"
        >
          <video
            v-if="renderSrc && !mediaError"
            ref="videoRef"
            :src="renderSrc"
            controls
            playsinline
            preload="metadata"
            class="w-full max-w-4xl max-h-full bg-black rounded-lg"
            @click.stop
            @error="handleMediaError"
          ></video>
          <div
            v-else
            class="text-center text-sm text-gray-500 dark:text-gray-400"
          >
            {{ mediaError || "没有可用的视频地址" }}
          </div>
        </div>

        <div
          v-else-if="isAudio"
          class="h-full w-full flex flex-col items-center justify-center gap-5 p-6"
        >
          <audio
            v-if="renderSrc && !mediaError"
            ref="audioRef"
            :src="renderSrc"
            controls
            preload="metadata"
            class="w-full max-w-xl"
            @click.stop
            @error="handleMediaError"
          ></audio>
          <div
            v-else
            class="text-center text-sm text-gray-500 dark:text-gray-400"
          >
            {{ mediaError || "没有可用的音频地址" }}
          </div>
        </div>
      </div>
    </div>
  </Transition>
</template>

<style scoped>
.vcp-attachment-viewer {
  background: linear-gradient(145deg, #f9f9fb, #f2f2f7);
  color: #333;
}

html.dark .vcp-attachment-viewer {
  background: linear-gradient(145deg, #1c1c1e, #2c2c2e);
  color: #f2f2f7;
}

.viewer-fade-enter-active,
.viewer-fade-leave-active {
  transition: all 0.3s cubic-bezier(0.16, 1, 0.3, 1);
}

.viewer-fade-enter-from,
.viewer-fade-leave-to {
  opacity: 0;
  transform: scale(1.05);
}

.animate-zoom-in {
  animation: zoomIn 0.4s cubic-bezier(0.16, 1, 0.3, 1);
}

@keyframes zoomIn {
  from {
    opacity: 0;
    transform: scale(0.9);
  }

  to {
    opacity: 1;
    transform: scale(1);
  }
}
</style>
