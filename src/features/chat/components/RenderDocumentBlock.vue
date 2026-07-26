<script setup lang="ts">
import {
  computed,
  defineAsyncComponent,
  nextTick,
  onUnmounted,
  ref,
  watch,
} from "vue";
import "markstream-vue/index.px.css";
import type { ContentBlock } from "../../../core/types/chat";
import { useMessageStyleInjector } from "../../../core/composables/useMessageStyleInjector";
import {
  compileRenderFragment,
  blockContainsRichHtml,
  RENDER_DOCUMENT_VERSION,
} from "../../../core/utils/renderDocument";
import { patchRenderDocumentRoot } from "../../../core/utils/renderDomExecutor";
import {
  useRenderVisibility,
  ViewportAnimationController,
} from "../../../core/composables/useRenderVisibility";

const MarkdownRender = defineAsyncComponent(() => import("markstream-vue"));

const props = defineProps<{
  block: ContentBlock;
  messageId: string;
  sourceId: string;
  streaming?: boolean;
}>();

const emit = defineEmits<{ rendered: [] }>();
const root = ref<HTMLElement | null>(null);
const { state, cachedHeight, rememberHeight } = useRenderVisibility(root);
const usesStreamingMarkdown = computed(
  () =>
    props.streaming === true &&
    props.block.type === "markdown" &&
    !blockContainsRichHtml(props.block),
);
const compiled = computed(() => {
  if (usesStreamingMarkdown.value) {
    return {
      version: RENDER_DOCUMENT_VERSION,
      html: "",
      css: "",
      rich: false,
      signature: `${RENDER_DOCUMENT_VERSION}:markstream:${props.sourceId}`,
    };
  }
  return compileRenderFragment(props.block, props.messageId);
});
const { injectScopedCss, removeScopedCss } = useMessageStyleInjector();
let activeSourceId = props.sourceId;
let animationController: ViewportAnimationController | null = null;
let parked = false;
const detailsState = new Map<string, boolean>();

function captureDetailsState() {
  root.value
    ?.querySelectorAll<HTMLDetailsElement>("details")
    .forEach((details, index) => {
      const key = details.id || details.dataset.vcpRenderKey || String(index);
      detailsState.set(key, details.open);
    });
}

function restoreDetailsState() {
  root.value
    ?.querySelectorAll<HTMLDetailsElement>("details")
    .forEach((details, index) => {
      const key = details.id || details.dataset.vcpRenderKey || String(index);
      if (detailsState.has(key)) details.open = Boolean(detailsState.get(key));
    });
}

function renderCompiledDocument() {
  if (!root.value || (state.value === "parked" && cachedHeight.value > 0)) {
    return;
  }
  root.value.style.removeProperty("height");
  patchRenderDocumentRoot(root.value, compiled.value.html);
  restoreDetailsState();
  root.value.dataset.vcpRenderSignature = compiled.value.signature;
  parked = false;
  if (!props.streaming) {
    if (!animationController) {
      animationController = new ViewportAnimationController(root.value);
    }
    animationController.setActive(state.value === "visible");
    animationController.refresh();
    rememberHeight();
    emit("rendered");
  }
}

watch(
  [() => compiled.value.signature, root, () => props.sourceId],
  async () => {
    if (activeSourceId !== props.sourceId) {
      removeScopedCss(props.messageId, activeSourceId);
      activeSourceId = props.sourceId;
      detailsState.clear();
    }
    injectScopedCss(compiled.value.css, props.messageId, activeSourceId);
    await nextTick();
    renderCompiledDocument();
  },
  { immediate: true, flush: "post" },
);

watch(state, async (nextState) => {
  await nextTick();
  if (!root.value) return;
  if (nextState === "parked") {
    // Never replace a never-rendered block with an empty, zero-height node.
    // It would be unable to trigger IntersectionObserver again until the user
    // manually scrolls the chat.
    if (cachedHeight.value <= 0) return;
    if (!parked) {
      rememberHeight();
      captureDetailsState();
      animationController?.disconnect();
      root.value.replaceChildren();
      if (cachedHeight.value > 0) {
        root.value.style.height = `${cachedHeight.value}px`;
      }
      parked = true;
    }
    return;
  }
  if (parked) renderCompiledDocument();
  animationController?.setActive(nextState === "visible");
  if (nextState === "visible") animationController?.refresh();
});

onUnmounted(() => {
  animationController?.disconnect();
  removeScopedCss(props.messageId, activeSourceId);
});
</script>

<template>
  <div
    v-if="usesStreamingMarkdown"
    class="vcp-render-document vcp-render-document-streaming vcp-markdown-block min-w-0"
    data-vcp-render-host=""
    :data-vcp-render-version="RENDER_DOCUMENT_VERSION"
  >
    <MarkdownRender
      mode="chat"
      :content="block.content || ''"
      :final="false"
      html-policy="safe"
      :smooth-streaming="false"
      :fade="false"
      :max-live-nodes="0"
      :render-code-blocks-as-pre="true"
    />
  </div>
  <div
    v-else
    ref="root"
    class="vcp-render-document vcp-markdown-block min-w-0"
    :class="{
      'vcp-render-document-streaming': streaming,
      'vcp-rich-html-block': compiled.rich,
    }"
    data-vcp-render-host=""
    :data-vcp-render-version="RENDER_DOCUMENT_VERSION"
  />
</template>

<style scoped>
.vcp-render-document.vcp-animation-paused :deep(*),
.vcp-render-document.vcp-animation-paused :deep(*::before),
.vcp-render-document.vcp-animation-paused :deep(*::after),
.vcp-render-document :deep(.vcp-element-offscreen),
.vcp-render-document :deep(.vcp-element-offscreen::before),
.vcp-render-document :deep(.vcp-element-offscreen::after) {
  animation-play-state: paused !important;
}
</style>
