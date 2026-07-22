<script setup lang="ts">
import { ref, watch, computed, onMounted, onUnmounted, nextTick } from "vue";
import { useThemeStore } from "../../../core/stores/theme";
import { useChatHistoryStore } from "../../../core/stores/chatHistoryStore";
import { useModalHistory } from "../../../core/composables/useModalHistory";
import { useRenderVisibility } from "../../../core/composables/useRenderVisibility";
import { wrapVcpButtonAction } from "../../../core/utils/htmlActions";
import {
  ACTIVE_HTML_MESSAGE_SOURCE,
  ACTIVE_HTML_PARENT_SOURCE,
  ACTIVE_HTML_PERMISSIONS,
  ACTIVE_HTML_SANDBOX,
  buildActiveHtmlDocument,
} from "../../../core/utils/activeHtmlSandbox";

const props = defineProps<{
  content: string;
  messageId: string;
  highlightedContent?: string;
  isStreaming?: boolean;
  isActiveStream?: boolean;
}>();

const themeStore = useThemeStore();
const historyStore = useChatHistoryStore();
const isPreviewing = ref(!props.isStreaming);
const isFullScreen = ref(false);
const fullScreenTab = ref<"code" | "preview">("code");
const blockRef = ref<HTMLElement | null>(null);
const inlineIframeRef = ref<HTMLIFrameElement | null>(null);
const fullscreenIframeRef = ref<HTMLIFrameElement | null>(null);
const inlineHeight = ref(180);
const fullscreenHeight = ref(480);
const { state } = useRenderVisibility(blockRef, inlineHeight.value);
const inlineMounted = computed(
  () => isPreviewing.value && state.value !== "parked",
);

const { registerModal, unregisterModal } = useModalHistory();
const modalId = `HtmlPreviewBlockFullScreen_${Math.random().toString(36).substring(2, 9)}`;
const bridgeNonce = Math.random().toString(36).substring(2, 15);

watch(isFullScreen, (newVal) => {
  if (newVal) {
    registerModal(modalId, () => {
      isFullScreen.value = false;
    });
  } else {
    unregisterModal(modalId);
  }
});

watch(
  () => props.isStreaming,
  (streaming) => {
    if (!streaming) {
      isPreviewing.value = true;
    }
  },
);

// 代码预览转义处理 (优先使用后端预渲染 syntect 高亮，无值时回退为安全 HTML 转义)
const highlightedCode = computed(() => {
  if (props.highlightedContent) {
    return props.highlightedContent;
  }
  return props.content
    .replace(/&/g, "&amp;")
    .replace(/</g, "&lt;")
    .replace(/>/g, "&gt;")
    .replace(/"/g, "&quot;")
    .replace(/'/g, "&#039;");
});

// 复制功能
const copyCode = async () => {
  try {
    await navigator.clipboard.writeText(props.content);
    // 这里如果以后有 Toast 提示可以加上
    console.log("[HTML Block] Code copied");
  } catch (err) {
    console.error("[HTML Block] Copy failed", err);
  }
};

// Active content executes inside an opaque-origin iframe, not the app document.
const sandboxHtml = computed(() => {
  return buildActiveHtmlDocument(
    props.content,
    themeStore.isDarkResolved,
    bridgeNonce,
  );
});

interface ActiveHtmlMessage {
  source?: string;
  type?: string;
  nonce?: string;
  height?: number;
  actionId?: string;
  action?: string;
}

function trustedFrame(
  event: MessageEvent<ActiveHtmlMessage>,
): HTMLIFrameElement | null {
  if (event.origin !== "null" || !event.source) return null;
  const frames = [inlineIframeRef.value, fullscreenIframeRef.value];
  return frames.find((frame) => frame?.contentWindow === event.source) || null;
}

function postToFrame(
  frame: HTMLIFrameElement,
  payload: Record<string, unknown>,
) {
  frame.contentWindow?.postMessage(
    {
      source: ACTIVE_HTML_PARENT_SOURCE,
      nonce: bridgeNonce,
      ...payload,
    },
    "*",
  );
}

async function handleSandboxAction(
  frame: HTMLIFrameElement,
  data: ActiveHtmlMessage,
) {
  const actionId = data.actionId || "";
  const payload = wrapVcpButtonAction(data.action || "");
  if (!actionId || !payload) {
    postToFrame(frame, { type: "ai-action-result", actionId, success: false });
    return;
  }

  try {
    const sent = await historyStore.sendMessage(payload);
    if (!sent) throw new Error("AI action did not start a generation request");
    postToFrame(frame, { type: "ai-action-result", actionId, success: true });
  } catch (error) {
    console.error("[HtmlPreviewBlock] Failed to send sandbox action:", error);
    postToFrame(frame, { type: "ai-action-result", actionId, success: false });
  }
}

function handleSandboxMessage(event: MessageEvent<ActiveHtmlMessage>) {
  const data = event.data;
  if (
    !data ||
    typeof data !== "object" ||
    data.source !== ACTIVE_HTML_MESSAGE_SOURCE ||
    data.nonce !== bridgeNonce
  ) {
    return;
  }
  const frame = trustedFrame(event);
  if (!frame) return;

  if (data.type === "render-size") {
    const nextHeight = Math.ceil(Number(data.height));
    if (Number.isFinite(nextHeight) && nextHeight > 0 && nextHeight < 100_000) {
      if (
        frame === inlineIframeRef.value &&
        Math.abs(inlineHeight.value - nextHeight) >= 2
      ) {
        inlineHeight.value = nextHeight;
      }
      if (
        frame === fullscreenIframeRef.value &&
        Math.abs(fullscreenHeight.value - nextHeight) >= 2
      ) {
        fullscreenHeight.value = nextHeight;
      }
    }
  } else if (data.type === "render-ready") {
    lastFrameVisibility.delete(frame);
    scheduleVisibilityUpdate(true);
  } else if (data.type === "ai-action") {
    void handleSandboxAction(frame, data);
  }
}

let visibilityFrame: number | null = null;
let visibilityTimer: ReturnType<typeof setTimeout> | null = null;
let lastVisibilitySyncAt = 0;
const lastFrameVisibility = new WeakMap<HTMLIFrameElement, string>();

function visibleBoundsForFrame(frame: HTMLIFrameElement) {
  const rect = frame.getBoundingClientRect();
  let viewportTop = 0;
  let viewportBottom = window.innerHeight;
  const scrollParent = frame.closest<HTMLElement>(".overflow-y-auto");
  if (scrollParent) {
    const parentRect = scrollParent.getBoundingClientRect();
    viewportTop = Math.max(viewportTop, parentRect.top);
    viewportBottom = Math.min(viewportBottom, parentRect.bottom);
  }
  const visible =
    !document.hidden &&
    rect.bottom > viewportTop &&
    rect.top < viewportBottom &&
    rect.right > 0 &&
    rect.left < window.innerWidth;
  return {
    visible,
    clipTop: Math.max(0, viewportTop - rect.top),
    clipBottom: Math.max(0, Math.min(rect.height, viewportBottom - rect.top)),
  };
}

function updateFrameVisibility() {
  visibilityFrame = null;
  lastVisibilitySyncAt = performance.now();
  for (const frame of [inlineIframeRef.value, fullscreenIframeRef.value]) {
    if (!frame?.contentWindow) continue;
    const bounds = visibleBoundsForFrame(frame);
    const signature = [
      bounds.visible ? 1 : 0,
      Math.round(bounds.clipTop),
      Math.round(bounds.clipBottom),
    ].join(":");
    if (lastFrameVisibility.get(frame) === signature) continue;
    lastFrameVisibility.set(frame, signature);
    postToFrame(frame, {
      type: "render-visibility",
      ...bounds,
    });
  }
}

function scheduleVisibilityUpdate(immediate: boolean | Event = false) {
  if (visibilityFrame !== null) return;
  if (immediate === true) {
    if (visibilityTimer) {
      clearTimeout(visibilityTimer);
      visibilityTimer = null;
    }
    visibilityFrame = requestAnimationFrame(updateFrameVisibility);
    return;
  }

  const remaining = 96 - (performance.now() - lastVisibilitySyncAt);
  if (remaining <= 0) {
    visibilityFrame = requestAnimationFrame(updateFrameVisibility);
  } else if (!visibilityTimer) {
    visibilityTimer = setTimeout(() => {
      visibilityTimer = null;
      if (visibilityFrame === null) {
        visibilityFrame = requestAnimationFrame(updateFrameVisibility);
      }
    }, remaining);
  }
}

function handleFrameLoad(event: Event) {
  if (event.currentTarget instanceof HTMLIFrameElement) {
    lastFrameVisibility.delete(event.currentTarget);
  }
  scheduleVisibilityUpdate(true);
}

const openFullScreen = () => {
  isFullScreen.value = true;
  fullScreenTab.value = isPreviewing.value ? "preview" : "code";
};

let refreshTimer: ReturnType<typeof setTimeout> | null = null;

const refreshPreview = () => {
  const iframe = isFullScreen.value
    ? fullscreenIframeRef.value
    : inlineIframeRef.value;

  if (iframe) {
    const currentSrc = iframe.srcdoc;
    iframe.srcdoc = "";
    if (refreshTimer) clearTimeout(refreshTimer);
    refreshTimer = setTimeout(() => {
      iframe.srcdoc = currentSrc;
    }, 50);
  }
};

// 同步普通视图与全屏视图的状态
watch(isPreviewing, (val) => {
  if (isFullScreen.value) {
    fullScreenTab.value = val ? "preview" : "code";
  }
});

watch(fullScreenTab, (val) => {
  isPreviewing.value = val === "preview";
});

watch([state, inlineHeight, isFullScreen, fullScreenTab], () => {
  void nextTick(scheduleVisibilityUpdate);
});

onMounted(() => {
  window.addEventListener("message", handleSandboxMessage);
  window.addEventListener("resize", scheduleVisibilityUpdate);
  window.addEventListener("scroll", scheduleVisibilityUpdate, true);
  document.addEventListener("visibilitychange", scheduleVisibilityUpdate);
});

onUnmounted(() => {
  if (refreshTimer) clearTimeout(refreshTimer);
  if (visibilityTimer) clearTimeout(visibilityTimer);
  if (visibilityFrame !== null) cancelAnimationFrame(visibilityFrame);
  window.removeEventListener("message", handleSandboxMessage);
  window.removeEventListener("resize", scheduleVisibilityUpdate);
  window.removeEventListener("scroll", scheduleVisibilityUpdate, true);
  document.removeEventListener("visibilitychange", scheduleVisibilityUpdate);
  unregisterModal(modalId);
});
</script>

<template>
  <div
    ref="blockRef"
    class="html-preview-block mb-4 overflow-hidden"
    :class="[
      isPreviewing ? 'html-preview-block--preview' : 'rounded-2xl border',
      !isPreviewing &&
        (themeStore.isDarkResolved
          ? 'border-white/10 bg-[#0d1117]/80'
          : 'border-black/5 bg-white/90'),
    ]"
  >
    <!-- 全屏页面 (Kimi 风格沙箱) -->
    <Teleport to="body">
      <Transition
        enter-active-class="transition duration-300 ease-out"
        enter-from-class="translate-y-10 opacity-0"
        enter-to-class="translate-y-0 opacity-100"
        leave-active-class="transition duration-200 ease-in"
        leave-from-class="translate-y-0 opacity-100"
        leave-to-class="translate-y-10 opacity-0"
      >
        <div
          v-if="isFullScreen"
          class="fixed inset-0 z-editor flex flex-col pb-[calc(var(--vcp-safe-bottom,48px))]"
          :class="
            themeStore.isDarkResolved
              ? 'bg-[#0d1117]'
              : 'bg-[#f6f8fa] text-gray-900'
          "
        >
          <!-- 全屏 Header -->
          <div
            class="h-14 flex items-center justify-between px-4 border-b pt-[var(--vcp-safe-top,0px)] box-content"
            :class="
              themeStore.isDarkResolved
                ? 'border-white/5 bg-[#0d1117]'
                : 'border-black/5 bg-white'
            "
          >
            <div class="flex items-center gap-4">
              <button
                @click="isFullScreen = false"
                class="p-2 -ml-2 active:scale-90 transition-transform"
              >
                <div
                  class="i-ph:caret-left-bold w-5 h-5"
                  :class="
                    themeStore.isDarkResolved
                      ? 'text-gray-400'
                      : 'text-gray-600'
                  "
                ></div>
              </button>
              <div class="flex flex-col">
                <span
                  class="text-sm font-bold uppercase tracking-wider"
                  :class="
                    themeStore.isDarkResolved
                      ? 'text-gray-200'
                      : 'text-gray-800'
                  "
                  >html</span
                >
              </div>
            </div>

            <div class="flex items-center gap-4">
              <button
                v-if="fullScreenTab === 'preview'"
                @click="refreshPreview"
                class="p-2 active:rotate-180 transition-transform duration-500"
              >
                <div
                  class="i-ph:arrow-clockwise-bold w-5 h-5 text-gray-400"
                ></div>
              </button>
              <button
                v-else
                @click="copyCode"
                class="p-2 active:scale-90 transition-transform"
              >
                <div class="i-ph:copy-bold w-5 h-5 text-gray-400"></div>
              </button>

              <div
                class="flex p-1 rounded-xl border transition-colors duration-300"
                :class="
                  themeStore.isDarkResolved
                    ? 'bg-white/5 border-white/5'
                    : 'bg-black/5 border-black/5'
                "
              >
                <button
                  @click="fullScreenTab = 'code'"
                  :class="[
                    fullScreenTab === 'code'
                      ? themeStore.isDarkResolved
                        ? 'bg-white/10 text-white shadow-md border-white/5'
                        : 'bg-white text-gray-900 shadow-sm border-black/5'
                      : themeStore.isDarkResolved
                        ? 'text-gray-400'
                        : 'text-gray-500',
                  ]"
                  class="px-4 py-1 text-[11px] font-bold rounded-lg transition-all border border-transparent"
                >
                  代码
                </button>
                <button
                  @click="fullScreenTab = 'preview'"
                  :class="[
                    fullScreenTab === 'preview'
                      ? themeStore.isDarkResolved
                        ? 'bg-white/10 text-white shadow-md border-white/5'
                        : 'bg-white text-gray-900 shadow-sm border-black/5'
                      : themeStore.isDarkResolved
                        ? 'text-gray-400'
                        : 'text-gray-500',
                  ]"
                  class="px-4 py-1 text-[11px] font-bold rounded-lg transition-all border border-transparent"
                >
                  预览
                </button>
              </div>
            </div>
          </div>

          <!-- 全屏内容区 -->
          <div
            class="flex-1 overflow-y-auto overflow-x-hidden relative"
            :class="themeStore.isDarkResolved ? 'bg-[#0d1117]' : 'bg-white'"
          >
            <div
              v-show="fullScreenTab === 'code'"
              class="absolute inset-0 overflow-auto p-4 text-xs font-mono leading-relaxed vcp-scrollable"
              :class="[
                themeStore.isDarkResolved
                  ? 'bg-[#0d1117] text-[#c9d1d9]'
                  : 'bg-[#f6f8fa] text-[#24292e]',
              ]"
            >
              <div
                v-if="highlightedContent"
                class="vcp-html-highlighted-wrapper"
                v-html="highlightedCode"
              ></div>
              <pre
                v-else
              ><code class="hljs" v-html="highlightedCode"></code></pre>
            </div>
            <iframe
              v-if="fullScreenTab === 'preview'"
              ref="fullscreenIframeRef"
              class="vcp-fullscreen-iframe block w-full border-none"
              :style="{ height: `${fullscreenHeight}px` }"
              :sandbox="ACTIVE_HTML_SANDBOX"
              :allow="ACTIVE_HTML_PERMISSIONS"
              allowfullscreen
              :srcdoc="sandboxHtml"
              :data-vcp-image-nonce="bridgeNonce"
              :data-vcp-bridge-nonce="bridgeNonce"
              @load="handleFrameLoad"
            ></iframe>
          </div>
        </div>
      </Transition>
    </Teleport>

    <!-- 普通视图 Header (比全屏模式略小一点点，保持呼吸感) -->
    <div
      v-if="!isPreviewing"
      class="h-12 flex items-center justify-between px-3.5 border-b relative z-10 box-content transition-colors duration-300"
      :class="
        themeStore.isDarkResolved
          ? 'bg-[#161b22] border-white/5'
          : 'bg-[#f6f8fa] border-black/5'
      "
    >
      <div class="flex items-center gap-2.5">
        <div class="i-ph:code-block-bold w-4 h-4 text-emerald-500"></div>
        <span
          class="text-xs font-bold uppercase tracking-wider"
          :class="themeStore.isDarkResolved ? 'text-gray-200' : 'text-gray-800'"
          >html</span
        >
      </div>

      <div class="flex items-center gap-3">
        <!-- 功能按钮：尺寸适中 -->
        <button
          v-if="isPreviewing"
          @click.stop="refreshPreview"
          class="p-1.5 active:rotate-180 transition-transform duration-500 opacity-60 hover:opacity-100"
        >
          <div
            class="i-ph:arrow-clockwise-bold w-5 h-5"
            :class="
              themeStore.isDarkResolved ? 'text-gray-400' : 'text-gray-600'
            "
          ></div>
        </button>
        <button
          v-else
          @click.stop="copyCode"
          class="p-1.5 active:scale-90 transition-transform opacity-60 hover:opacity-100"
        >
          <div
            class="i-ph:copy-bold w-5 h-5"
            :class="
              themeStore.isDarkResolved ? 'text-gray-400' : 'text-gray-600'
            "
          ></div>
        </button>

        <button
          @click.stop="openFullScreen"
          class="p-1.5 active:scale-90 transition-transform opacity-60 hover:opacity-100"
        >
          <div
            class="i-ph:arrows-out-bold w-4.5 h-4.5"
            :class="
              themeStore.isDarkResolved ? 'text-gray-400' : 'text-gray-600'
            "
          ></div>
        </button>

        <div
          class="flex p-0.8 rounded-xl border transition-colors duration-300"
          :class="
            themeStore.isDarkResolved
              ? 'bg-white/5 border-white/5'
              : 'bg-black/5 border-black/5'
          "
        >
          <button
            @click.stop="isPreviewing = false"
            :class="[
              !isPreviewing
                ? themeStore.isDarkResolved
                  ? 'bg-white/10 text-white shadow-md border-white/5'
                  : 'bg-white text-gray-900 shadow-sm border-black/5'
                : themeStore.isDarkResolved
                  ? 'text-gray-400'
                  : 'text-gray-500',
            ]"
            class="px-3 py-1 text-[10px] font-bold rounded-lg transition-all border border-transparent"
          >
            代码
          </button>
          <button
            @click.stop="isPreviewing = true"
            :class="[
              isPreviewing
                ? themeStore.isDarkResolved
                  ? 'bg-white/10 text-white shadow-md border-white/5'
                  : 'bg-white text-gray-900 shadow-sm border-black/5'
                : themeStore.isDarkResolved
                  ? 'text-gray-400'
                  : 'text-gray-500',
            ]"
            class="px-3 py-1 text-[10px] font-bold rounded-lg transition-all border border-transparent"
          >
            预览
          </button>
        </div>
      </div>
    </div>

    <!-- Inline preview uses the sandbox-reported document height. -->
    <div
      class="relative overflow-hidden no-swipe"
      :style="isPreviewing ? { height: `${inlineHeight}px` } : undefined"
    >
      <div
        v-show="!isPreviewing"
        class="w-full overflow-auto max-h-[380px] p-3 text-[10px] font-mono leading-relaxed vcp-scrollable no-swipe"
        :class="[
          themeStore.isDarkResolved
            ? 'bg-[#0d1117] text-[#c9d1d9]'
            : 'bg-[#f6f8fa] text-[#24292e]',
        ]"
      >
        <div
          v-if="highlightedContent"
          class="vcp-html-highlighted-wrapper"
          v-html="highlightedCode"
        ></div>
        <pre
          v-else
          class="w-full min-w-max"
        ><code class="hljs" v-html="highlightedCode"></code></pre>
      </div>

      <div v-if="isPreviewing" class="absolute inset-0 no-swipe bg-transparent">
        <iframe
          v-if="inlineMounted"
          ref="inlineIframeRef"
          class="vcp-inline-iframe w-full h-full border-none no-swipe"
          :sandbox="ACTIVE_HTML_SANDBOX"
          :allow="ACTIVE_HTML_PERMISSIONS"
          allowfullscreen
          :srcdoc="sandboxHtml"
          :data-vcp-image-nonce="bridgeNonce"
          :data-vcp-bridge-nonce="bridgeNonce"
          @load="handleFrameLoad"
        ></iframe>
        <div
          class="html-preview-floating-actions"
          :class="
            themeStore.isDarkResolved
              ? 'bg-black/60 text-white border-white/10'
              : 'bg-white/85 text-gray-700 border-black/10'
          "
        >
          <button
            type="button"
            class="html-preview-action"
            title="查看源码"
            aria-label="查看源码"
            @click.stop="isPreviewing = false"
          >
            <span class="i-ph:code-bold w-4 h-4" aria-hidden="true"></span>
          </button>
          <button
            type="button"
            class="html-preview-action active:rotate-180"
            title="刷新网页"
            aria-label="刷新网页"
            @click.stop="refreshPreview"
          >
            <span
              class="i-ph:arrow-clockwise-bold w-4 h-4"
              aria-hidden="true"
            ></span>
          </button>
          <button
            type="button"
            class="html-preview-action"
            title="全屏显示"
            aria-label="全屏显示"
            @click.stop="openFullScreen"
          >
            <span
              class="i-ph:arrows-out-bold w-4 h-4"
              aria-hidden="true"
            ></span>
          </button>
        </div>
      </div>
    </div>
  </div>
</template>

<style scoped>
.html-preview-block:not(.html-preview-block--preview) {
  /* 极致轻盈的现代双层散焦微阴影，杜绝死黑与大范围污染 */
  box-shadow:
    0 4px 20px -6px rgba(0, 0, 0, 0.12),
    0 2px 8px -2px rgba(0, 0, 0, 0.04);
}

:root.dark .html-preview-block:not(.html-preview-block--preview) {
  /* 暗色模式下微调投影透明度，维持极简科技感，避免脏底 */
  box-shadow: 0 4px 20px -6px rgba(0, 0, 0, 0.35);
}

.html-preview-block--preview {
  display: block;
  width: 100%;
  min-width: 0;
  border: 0;
  border-radius: 0;
  background: transparent;
  box-shadow: none;
}

.html-preview-floating-actions {
  position: absolute;
  top: 8px;
  right: 8px;
  z-index: 2;
  display: flex;
  gap: 2px;
  padding: 2px;
  border: 1px solid;
  border-radius: 8px;
  box-shadow: 0 2px 10px rgba(0, 0, 0, 0.16);
  backdrop-filter: blur(10px);
}

.html-preview-action {
  display: inline-flex;
  width: 30px;
  height: 30px;
  align-items: center;
  justify-content: center;
  border-radius: 6px;
  opacity: 0.78;
  transition:
    opacity 120ms ease,
    background-color 120ms ease,
    transform 180ms ease;
}

.html-preview-action:hover,
.html-preview-action:focus-visible {
  background: rgba(127, 127, 127, 0.18);
  opacity: 1;
}

.html-preview-action:active {
  transform: scale(0.92);
}

/* 高亮代码基础样式 */
.hljs {
  display: block;
  overflow-x: auto;
  padding: 0;
  background: transparent;
}

/* 暗色模式高亮 (GitHub Dark 风格适配) */
.html-preview-block :deep(.hljs-tag),
.html-preview-block :deep(.hljs-name),
.html-preview-block :deep(.hljs-keyword) {
  color: #ff7b72;
}
.html-preview-block :deep(.hljs-attr) {
  color: #79c0ff;
}
.html-preview-block :deep(.hljs-string) {
  color: #a5d6ff;
}
.html-preview-block :deep(.hljs-comment) {
  color: #8b949e;
  font-style: italic;
}
.html-preview-block :deep(.hljs-meta) {
  color: #ff7b72;
}

/* 亮色模式高亮适配 (GitHub Light 风格适配) */
/* 使用 :not(.dark) 或通过父级类名区分 */
.bg-white .hljs-tag,
.bg-white .hljs-name,
.bg-white .hljs-keyword {
  color: #d73a49;
}
.bg-white .hljs-attr {
  color: #005cc5;
}
.bg-white .hljs-string {
  color: #032f62;
}
.bg-white .hljs-comment {
  color: #6a737d;
  font-style: italic;
}
.bg-white .hljs-meta {
  color: #d73a49;
}

/* 专属 vcp-html-block 样式隔离与重置 */
.vcp-html-highlighted-wrapper :deep(pre),
.vcp-html-highlighted-wrapper :deep(code) {
  margin: 0 !important;
  padding: 0 !important;
  background: transparent !important;
  border: none !important;
  font-size: inherit !important;
  font-family: inherit !important;
  line-height: inherit !important;
  box-shadow: none !important;
  white-space: pre !important;
  overflow-x: auto !important;
}
.vcp-html-highlighted-wrapper :deep(span) {
  display: inline !important;
  white-space: pre !important;
}
.vcp-html-highlighted-wrapper :deep(code) {
  padding: 0 !important;
  background: transparent !important;
}
</style>
