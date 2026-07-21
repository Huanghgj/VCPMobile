<script setup lang="ts">
import { computed, ref, watch, nextTick } from "vue";
import { invoke } from "@tauri-apps/api/core";
import type { ChatMessage, ContentBlock } from "../../core/types/chat";
import { useOverlayStore } from "../../core/stores/overlay";
import { useChatHistoryStore } from "../../core/stores/chatHistoryStore";
import { useChatSessionStore } from "../../core/stores/chatSessionStore";
import { useChatStreamStore } from "../../core/stores/chatStreamStore";
import { useNotificationStore } from "../../core/stores/notification";
import { useMessageEvents } from "../../core/composables/useMessageEvents";
import { useEmoticonFixer } from "../../core/composables/useEmoticonFixer";
import {
  getKatexRenderer,
  getMermaidRenderer,
} from "../../core/utils/renderLibraryPreloader";
import {
  blockContainsRichHtml,
  compileRenderFragment,
  createRenderDocument,
  isRenderDocumentBlock,
  RENDER_DOCUMENT_VERSION,
} from "../../core/utils/renderDocument";
import {
  Copy,
  Edit2,
  RotateCcw,
  Trash2,
  StopCircle,
  Bug,
} from "lucide-vue-next";

const { processEmoticonsInContainer } = useEmoticonFixer();
const mermaidCache = new Map<string, string>();
const MAX_MERMAID_CACHE_SIZE = 30;

function setMermaidCache(key: string, value: string) {
  if (mermaidCache.has(key)) {
    mermaidCache.delete(key);
  } else if (mermaidCache.size >= MAX_MERMAID_CACHE_SIZE) {
    const firstKey = mermaidCache.keys().next().value;
    if (firstKey !== undefined) {
      mermaidCache.delete(firstKey);
    }
  }
  mermaidCache.set(key, value);
}

const renderingMermaids = new Set<string>();

// UI Components
import ChatBubble from "./components/ChatBubble.vue";
import MessageHeader from "./components/MessageHeader.vue";
import ThinkingIndicator from "./components/ThinkingIndicator.vue";
import StreamingTag from "./components/StreamingTag.vue";
import AttachmentPreview from "./attachment/AttachmentPreview.vue";
import RenderDocumentBlock from "./components/RenderDocumentBlock.vue";

// Interactive Block Components
import ToolBlock from "./blocks/ToolBlock.vue";
import ThoughtBlock from "./blocks/ThoughtBlock.vue";
import HtmlPreviewBlock from "./blocks/HtmlPreviewBlock.vue";
import ToolSummaryBlock from "./blocks/ToolSummaryBlock.vue";
import MermaidFullScreenViewer from "./blocks/MermaidFullScreenViewer.vue";

const props = defineProps<{
  message: ChatMessage;
  agentId?: string;
  depth?: number;
}>();

const overlayStore = useOverlayStore();
const notificationStore = useNotificationStore();
const historyStore = useChatHistoryStore();
const sessionStore = useChatSessionStore();
const streamStore = useChatStreamStore();

// === Mermaid FullScreen States ===
const isMermaidFullScreen = ref(false);
const activeMermaidSvg = ref("");
const activeMermaidSource = ref("");

// === Shell Properties (Pre-computed in Rust) ===
const shell = computed(
  () =>
    props.message.shell ||
    streamStore.computeShell({
      role: props.message.role || "assistant",
      agentId: props.message.agentId,
      name: props.message.name,
    }),
);

// === Streaming State ===

// 数据层面：消息是否处于任意活跃流中（不依赖当前话题）
const isMessageInActiveStream = computed(() => {
  return streamStore.isMessageInActiveStream(props.message.id);
});

// UI 层面：消息是否在当前视口中显示流式状态
const isStreaming = computed(() => {
  if (shell.value?.isUser) return false;

  return streamStore.isMessageStreamingInSession(
    props.message.id,
    sessionStore.currentSelectedItem?.id,
    sessionStore.currentTopicId,
  );
});

const renderDocument = computed(() => {
  const allowFallback =
    !!props.message.content &&
    (!isStreaming.value ||
      (!props.message.tailBlock && !props.message.tailContent));
  return createRenderDocument(
    props.message.blocks,
    props.message.tailBlock,
    allowFallback ? props.message.content : "",
  );
});

function isBrkNode(node: any): boolean {
  if (node.type === "raw_html" && node.content) {
    const trimmed = node.content.trim().replace(/\s+/g, "");
    return trimmed === "<!--brk-->";
  }
  return false;
}

function isBrkBlock(block: ContentBlock): boolean {
  if (!isPlainBlock(block.type)) return false;

  if (block.content) {
    const trimmed = block.content.trim().replace(/\s+/g, "");
    if (trimmed === "<!--brk-->") return true;
  }

  if (block.nodes && block.nodes.length > 0) {
    const groups = splitMarkdownNodes(block.nodes);
    return groups.length === 0;
  }

  return false;
}

function splitMarkdownNodes(nodes: any[]): any[][] {
  const result: any[][] = [];
  let currentGroup: any[] = [];
  let hasBrk = false;
  let htmlDepth = 0;

  for (const node of nodes) {
    if (node.type === "raw_html" && node.content) {
      const content = node.content.trim().toLowerCase();
      if (
        content.startsWith("<div") &&
        !content.endsWith("/>") &&
        !content.includes("</div>")
      ) {
        htmlDepth++;
      }
      if (content.startsWith("</div")) {
        htmlDepth = Math.max(0, htmlDepth - 1);
      }
    }

    if (isBrkNode(node) && htmlDepth === 0) {
      result.push(currentGroup);
      currentGroup = [];
      hasBrk = true;
    } else {
      currentGroup.push(node);
    }
  }

  if (hasBrk && currentGroup.length === 0) {
    result.push([]);
  } else if (currentGroup.length > 0) {
    result.push(currentGroup);
  }
  return result;
}

interface BubbleGroup {
  id: string;
  blocks: RenderedContentBlock[];
  hasRichHtml: boolean;
  isTail?: boolean;
}

type RenderedContentBlock = ContentBlock & {
  __renderKey: string;
};

function getBlockKey(block: ContentBlock, index: number): string {
  const messageScope = props.message.id || "unknown-message";
  if (block.hash !== undefined && block.hash !== null) {
    return `${messageScope}-${block.type}-${String(block.hash)}-${index}`;
  }
  // Fallback for legacy data (index-based)
  return `${messageScope}-${block.type}-idx-${index}`;
}

const messageBubbles = computed(() => {
  const list: BubbleGroup[] = [];
  let currentBlocks: ContentBlock[] = [];
  let bubbleIndex = 0;

  const pushCurrentGroup = () => {
    if (currentBlocks.length > 0) {
      const blocks = currentBlocks.map((block, index) => ({
        ...block,
        __renderKey: getBlockKey(block, index),
      }));
      list.push({
        id: `${props.message.id}-bubble-${bubbleIndex++}`,
        blocks,
        hasRichHtml: blocks.some(blockContainsRichHtml),
      });
      currentBlocks = [];
    }
  };

  const isUserMsg = shell.value?.isUser;

  if (renderDocument.value.blocks.length > 0) {
    for (const block of renderDocument.value.blocks) {
      if (!isPlainBlock(block.type) || isUserMsg) {
        currentBlocks.push(block);
        continue;
      }

      // 🆕 优先判定这个块是否整体就是一个 brk 物理分割块 (支持纯文本及 AST 状态双重鉴定)
      if (isBrkBlock(block)) {
        pushCurrentGroup();
        continue; // 过滤掉 <!--brk--> 本身不渲染
      }

      if (block.nodes && block.nodes.length > 0) {
        const nodeGroups = splitMarkdownNodes(block.nodes);
        if (nodeGroups.length > 1) {
          nodeGroups.forEach((groupNodes, idx) => {
            const newBlock: ContentBlock = {
              ...block,
              nodes: groupNodes,
              hash:
                block.hash !== undefined
                  ? `${block.hash}-split-${idx}`
                  : undefined,
            };
            currentBlocks.push(newBlock);
            if (idx < nodeGroups.length - 1) {
              pushCurrentGroup();
            }
          });
        } else if (nodeGroups.length === 0) {
          // 🆕 兜底：如果内部 AST 切分结果为 0 也是纯分割块
          pushCurrentGroup();
        } else {
          currentBlocks.push(block);
        }
      } else {
        currentBlocks.push(block);
      }
    }
  }

  pushCurrentGroup();

  // 🆕 流式状态下，如果最后一个稳定块是个 brk 块，我们需要额外追加一个空的气泡组以供 tailBlock 打字渲染
  const lastBlockIsBrk =
    renderDocument.value.blocks.length > 0 &&
    (() => {
      const last =
        renderDocument.value.blocks[renderDocument.value.blocks.length - 1];
      return last ? isBrkBlock(last) : false;
    })();

  if (isStreaming.value && props.message.tailBlock && lastBlockIsBrk) {
    list.push({
      id: `${props.message.id}-bubble-${bubbleIndex++}`,
      blocks: [],
      hasRichHtml: false,
    });
  }

  // 兜底：如果整个消息 blocks 为空
  if (list.length === 0) {
    list.push({
      id: `${props.message.id}-bubble-0`,
      blocks: [],
      hasRichHtml: false,
    });
  }

  return list;
});

// === Event Delegation ===
const messageContentRef = ref<HTMLElement | null>(null);
useMessageEvents(messageContentRef);

// === Block Rendering Helper ===
function isPlainBlock(type: string): boolean {
  return isRenderDocumentBlock(type);
}

function isRenderableTailBlock(type?: string): boolean {
  if (!type) return false;
  return (
    isPlainBlock(type) ||
    [
      "tool-use",
      "tool-result",
      "thought",
      "html-preview",
      "tool-call-summary",
    ].includes(type)
  );
}

function getBubbleStyle(bubble: BubbleGroup): Record<string, string> {
  const style: Record<string, string> = {
    "--dynamic-color": shell.value?.avatarColor || "transparent",
  };
  if (!shell.value?.isUser && bubble.hasRichHtml) {
    style["--assistant-bubble-bg"] = "transparent";
    style["--agent-text"] = "inherit";
    style["border-color"] = "transparent";
    style["box-shadow"] = "none";
    style["padding"] = "0";
    style["width"] = "100%";
    style["min-width"] = "0";
    style["max-width"] = "100%";
  }
  return style;
}

function renderBlockHtml(block: ContentBlock): string {
  return compileRenderFragment(block, props.message.id).html;
}

function walkInlineDebug(
  nodes: any[] | undefined,
  stats: Record<string, number>,
) {
  for (const node of nodes || []) {
    if (!node?.type) continue;
    stats[`inline:${node.type}`] = (stats[`inline:${node.type}`] || 0) + 1;
    if (node.type === "raw_html_inline") {
      stats.rawHtmlInline = (stats.rawHtmlInline || 0) + 1;
    }
    if (node.type === "image") {
      stats.images = (stats.images || 0) + 1;
    }
    walkInlineDebug(node.children, stats);
  }
}

function walkMarkdownDebug(
  nodes: any[] | undefined,
  stats: Record<string, number>,
) {
  for (const node of nodes || []) {
    if (!node?.type) continue;
    stats[`node:${node.type}`] = (stats[`node:${node.type}`] || 0) + 1;
    if (node.type === "raw_html") {
      stats.rawHtml = (stats.rawHtml || 0) + 1;
    }
    if (node.type === "code_block") {
      stats.codeBlocks = (stats.codeBlocks || 0) + 1;
    }
    walkInlineDebug(node.children, stats);
    walkMarkdownDebug(node.children, stats);
    for (const item of node.items || []) walkMarkdownDebug(item, stats);
    for (const cell of node.header || []) walkInlineDebug(cell, stats);
    for (const row of node.rows || []) {
      for (const cell of row || []) walkInlineDebug(cell, stats);
    }
  }
}

function summarizeBlocksForDebug(blocks: ContentBlock[] | undefined) {
  const list = blocks || [];
  const byType = list.reduce<Record<string, number>>((acc, block) => {
    acc[block.type] = (acc[block.type] || 0) + 1;
    return acc;
  }, {});
  const stats: Record<string, number> = {};
  for (const block of list) {
    if (block.type === "html-preview")
      stats.htmlPreview = (stats.htmlPreview || 0) + 1;
    if (block.type === "button-click")
      stats.buttonClick = (stats.buttonClick || 0) + 1;
    if (block.type === "style")
      stats.styleBlocks = (stats.styleBlocks || 0) + 1;
    walkMarkdownDebug(block.nodes, stats);
  }
  return {
    count: list.length,
    byType,
    stats,
    blocks: list.map((block, index) => ({
      index,
      type: block.type,
      hash: block.hash,
      contentLength: block.content?.length || block.raw_content?.length || 0,
      nodeCount: block.nodes?.length || 0,
      hasRawHtml: blockContainsRichHtml(block),
      htmlPreviewLength:
        block.type === "html-preview" ? block.content?.length || 0 : undefined,
      toolName: block.tool_name,
      status: block.status,
      role: block.role,
      isEnd: block.is_end,
    })),
  };
}

function renderBlocksForDebug(
  blocks: ContentBlock[],
): Array<Record<string, unknown>> {
  return blocks.map((block, index) => {
    let html = "";
    let error = "";
    try {
      html = renderBlockHtml(block);
    } catch (e) {
      error = String(e);
    }
    return {
      index,
      type: block.type,
      htmlLength: html.length,
      startsWith: html.slice(0, 240),
      containsRawTagText:
        html.includes("&lt;div") ||
        html.includes("&lt;img") ||
        html.includes("&lt;style") ||
        html.includes("&lt;button"),
      containsRenderableTag:
        /<(div|img|style|button|pre|code|p|section|article)\b/i.test(html),
      error,
    };
  });
}

function readElementStyleForDebug(
  element: Element | null,
): Record<string, unknown> | null {
  if (!(element instanceof HTMLElement)) return null;
  const style = window.getComputedStyle(element);
  const rect = element.getBoundingClientRect();
  return {
    tagName: element.tagName.toLowerCase(),
    className: element.className,
    inlineStyle: element.getAttribute("style") || "",
    width: Math.round(rect.width),
    height: Math.round(rect.height),
    display: style.display,
    position: style.position,
    backgroundColor: style.backgroundColor,
    color: style.color,
    fontFamily: style.fontFamily,
    fontSize: style.fontSize,
    lineHeight: style.lineHeight,
    padding: style.padding,
    margin: style.margin,
    borderRadius: style.borderRadius,
    borderTopColor: style.borderTopColor,
    opacity: style.opacity,
    overflow: style.overflow,
    whiteSpace: style.whiteSpace,
  };
}

function readImagesForDebug(root: Element): Array<Record<string, unknown>> {
  return Array.from(root.querySelectorAll("img")).map((img, index) => {
    const rect = img.getBoundingClientRect();
    const style = window.getComputedStyle(img);
    return {
      index,
      src: img.currentSrc || img.src || "",
      attrSrc: img.getAttribute("src") || "",
      complete: img.complete,
      naturalWidth: img.naturalWidth,
      naturalHeight: img.naturalHeight,
      renderedWidth: Math.round(rect.width),
      renderedHeight: Math.round(rect.height),
      display: style.display,
      visibility: style.visibility,
      opacity: style.opacity,
    };
  });
}

function readActualDomForDebug(): Record<string, unknown> {
  const root = messageContentRef.value;
  if (!root) {
    return { mounted: false };
  }

  const contentRoot = root.querySelector(".vcp-content-blocks");
  const target = contentRoot || root;
  const vcpRoot = target.querySelector("#vcp-root");
  const richBlock = target.querySelector("[data-vcp-rich-html='true']");
  const bubble = root.querySelector(".vcp-bubble-container");
  const html = target.innerHTML || "";
  const text = target.textContent || "";
  return {
    mounted: true,
    htmlLength: html.length,
    textLength: text.length,
    hasVcpRootElement: Boolean(vcpRoot),
    hasRenderedImageElement: Boolean(target.querySelector("img")),
    richBlock: readElementStyleForDebug(richBlock),
    vcpRoot: readElementStyleForDebug(vcpRoot),
    bubble: readElementStyleForDebug(bubble),
    images: readImagesForDebug(target),
    hasLiteralEscapedDivText: text.includes("<div") || text.includes("&lt;div"),
    htmlStartsWith: html.slice(0, 800),
    textStartsWith: text.slice(0, 800),
  };
}

function buildRenderDebugReport(
  rawContent: string,
  reparsedBlocks: ContentBlock[],
): string {
  const currentBlocks = props.message.blocks || [];
  const currentSummary = summarizeBlocksForDebug(currentBlocks);
  const reparsedSummary = summarizeBlocksForDebug(reparsedBlocks);
  const currentHtml = renderBlocksForDebug(currentBlocks);
  const reparsedHtml = renderBlocksForDebug(reparsedBlocks);

  return JSON.stringify(
    {
      message: {
        renderDocumentVersion: RENDER_DOCUMENT_VERSION,
        id: props.message.id,
        role: props.message.role,
        name: props.message.name,
        isStreaming: isStreaming.value,
        hasContentOnMessage: Boolean(props.message.content),
        rawContentLength: rawContent.length,
        tailContentLength: props.message.tailContent?.length || 0,
        tailBlockType: props.message.tailBlock?.type || null,
        tailHasNodes: Boolean(props.message.tailBlock?.nodes?.length),
        tailFrame: props.message.tailFrame
          ? {
              epoch: props.message.tailFrame.epoch,
              revision: props.message.tailFrame.revision,
              frameSeq: props.message.tailFrame.frameSeq,
              reset: props.message.tailFrame.reset === true,
              mutationCount: props.message.tailFrame.mutations?.length || 0,
              snapshotCount: props.message.tailFrame.snapshot?.length || 0,
            }
          : null,
      },
      parserComparison: {
        current: currentSummary,
        reparsed: reparsedSummary,
        sameBlockTypeSequence:
          currentBlocks.map((block) => block.type).join(">") ===
          reparsedBlocks.map((block) => block.type).join(">"),
      },
      frontEndRender: {
        currentBlocks: currentHtml,
        reparsedBlocks: reparsedHtml,
      },
      actualDom: readActualDomForDebug(),
      rawContentPreview: rawContent.slice(0, 4000),
      reparsedBlocks,
      currentBlocks,
    },
    null,
    2,
  );
}

async function openRenderDebugReport(getFullText: () => Promise<string>) {
  try {
    const rawContent = await getFullText();
    const reparsedBlocks = await invoke<ContentBlock[]>(
      "process_message_content",
      {
        content: rawContent,
      },
    );
    overlayStore.openEditor({
      title: "渲染解析诊断",
      initialValue: buildRenderDebugReport(rawContent, reparsedBlocks),
      readOnly: true,
      showSave: false,
      monospace: true,
      placeholder: "正在生成诊断报告...",
    });
  } catch (e) {
    notificationStore.addNotification({
      type: "error",
      title: "渲染解析失败",
      message: String(e),
      toastOnly: true,
    });
  }
}

function escapeHtml(text: string): string {
  return text
    .replace(/&/g, "&amp;")
    .replace(/</g, "&lt;")
    .replace(/>/g, "&gt;")
    .replace(/"/g, "&quot;")
    .replace(/'/g, "&#039;");
}

const openMermaidFullScreen = (svgHtml: string, sourceCode: string) => {
  activeMermaidSvg.value = svgHtml;
  activeMermaidSource.value = sourceCode;
  isMermaidFullScreen.value = true;
};

function enhanceMermaid(el: HTMLElement, sourceCode: string) {
  if (!el || el.dataset.vcpMermaidEnhanced === "true") return;

  const svg = el.querySelector("svg");
  if (!svg) return;

  el.dataset.vcpMermaidEnhanced = "true";

  // 给 SVG 设置基础样式，使其自适应显示
  svg.removeAttribute("style");
  svg.style.maxWidth = "100%";
  svg.style.height = "auto";
  svg.style.display = "block";
  svg.style.margin = "0 auto";

  // 创建包裹层
  const wrapper = document.createElement("div");
  wrapper.className =
    "vcp-mermaid-wrapper group relative my-3 overflow-hidden rounded-xl border border-black/5 dark:border-white/10 bg-black/5 dark:bg-white/5 p-4 transition-all duration-300 active:scale-[0.99] cursor-pointer";

  // 创建全屏按钮
  const fullscreenBtn = document.createElement("button");
  fullscreenBtn.type = "button";
  fullscreenBtn.className =
    "absolute top-3 right-3 z-10 flex items-center justify-center w-8 h-8 rounded-lg border border-black/5 dark:border-white/10 bg-white/80 dark:bg-black/80 text-gray-500 dark:text-gray-400 opacity-0 group-hover:opacity-100 active:scale-90 transition-all duration-200 cursor-pointer shadow-sm";
  fullscreenBtn.innerHTML = '<div class="i-ph:arrows-out-bold w-4 h-4"></div>';
  fullscreenBtn.title = "全屏查看图表";
  fullscreenBtn.dataset.vcpUiControl = "mermaid-fullscreen";

  wrapper.addEventListener("click", (e) => {
    e.stopPropagation();
    openMermaidFullScreen(svg.outerHTML, sourceCode);
  });

  wrapper.addEventListener("dblclick", (e) => {
    e.stopPropagation();
  });

  el.textContent = "";
  wrapper.appendChild(fullscreenBtn);
  wrapper.appendChild(svg);
  el.appendChild(wrapper);
}

// === Heavy Content Rendering (KaTeX inline math + Mermaid) ===
const renderHeavyContent = async () => {
  await nextTick();
  if (!messageContentRef.value) return;

  // 1. KaTeX math (inline + display mode, rendered inside Renderer V2 blocks)
  const mathElements = Array.from(
    messageContentRef.value.querySelectorAll(
      ".vcp-math-inline[data-latex], .vcp-math-block[data-latex]",
    ),
  ).filter((el) => !el.closest(".streaming-tail"));

  if (mathElements.length > 0) {
    try {
      const katex = await getKatexRenderer();
      mathElements.forEach((el) => {
        if (el.querySelector(".katex")) return; // already rendered
        const latex = el.getAttribute("data-latex");
        if (!latex) return;
        const isDisplay = el.classList.contains("vcp-math-block");
        katex.render(latex, el as HTMLElement, {
          throwOnError: false,
          strict: false,
          displayMode: isDisplay,
        });
      });
    } catch (e) {
      console.error("[MessageRenderer] KaTeX render failed:", e);
    }
  }

  // 2. Mermaid diagrams
  const mermaidPlaceholders = Array.from(
    messageContentRef.value.querySelectorAll(
      ".mermaid-placeholder, pre.mermaid, code.language-mermaid",
    ),
  ).filter((el) => !el.closest(".streaming-tail"));

  if (mermaidPlaceholders.length > 0) {
    try {
      const mermaid = await getMermaidRenderer();
      for (const el of Array.from(mermaidPlaceholders)) {
        const placeholder = el as HTMLElement;
        const wrapper = placeholder.closest(".vcp-mermaid-wrapper");
        if (wrapper && wrapper.querySelector("svg")) continue; // already rendered & enhanced
        if (placeholder.querySelector("svg")) continue; // already rendered

        // Use innerHTML as stable cache key
        const codeKey = placeholder.innerHTML;
        // Skip if already being rendered by a concurrent call
        if (renderingMermaids.has(codeKey)) continue;
        // Skip if Vue has replaced this element out of the DOM
        if (!messageContentRef.value.contains(placeholder)) continue;

        // Use cache to avoid re-rendering the same diagram
        if (mermaidCache.has(codeKey)) {
          const cachedSvg = mermaidCache.get(codeKey)!;
          placeholder.innerHTML = cachedSvg;
          placeholder.classList.remove("mermaid-placeholder");
          placeholder.classList.add("mermaid");
          enhanceMermaid(placeholder, placeholder.dataset.mermaidSource || "");
          continue;
        }

        renderingMermaids.add(codeKey);
        try {
          const sourceCode = placeholder.textContent || "";
          placeholder.dataset.mermaidSource = sourceCode; // 保存原始源码

          placeholder.classList.remove("mermaid-placeholder");
          placeholder.classList.add("mermaid");
          await mermaid.run({ nodes: [placeholder] });

          const renderedSvg = placeholder.innerHTML;
          setMermaidCache(codeKey, renderedSvg); // 缓存纯 SVG

          enhanceMermaid(placeholder, sourceCode);
        } catch (e: any) {
          const errorMsg = e?.str || e?.message || String(e);
          console.error(
            "[MessageRenderer] Mermaid render failed:",
            errorMsg,
            e,
          );
          placeholder.innerHTML = `<div class="text-red-500 text-[10px] p-4 rounded-xl border border-red-500/10 bg-red-500/5">图表渲染失败: ${escapeHtml(errorMsg)}</div>`;
        } finally {
          renderingMermaids.delete(codeKey);
        }
      }
    } catch (e) {
      console.error("[MessageRenderer] Mermaid load failed:", e);
    }
  }

  // 3. Emoticons
  if (messageContentRef.value) {
    processEmoticonsInContainer(messageContentRef.value);
  }
};

// Watch for content changes and trigger heavy rendering
// Note: blocks array reference changes when Rust parser returns new AST,
// so shallow watch is sufficient. Avoid deep watch to prevent O(n) traversal
// on every streaming chunk across all rendered messages.
watch(
  () => props.message.blocks,
  () => {
    renderHeavyContent();
  },
  { immediate: true },
);

// 消息真正离开活跃流后统一执行一次重渲染，确保 KaTeX/Mermaid/Emoticon 正确渲染
watch(isMessageInActiveStream, (inStream, wasInStream) => {
  if (wasInStream && !inStream) {
    renderHeavyContent();
  }
});

// === Context Menu ===
const showMessageContextMenu = async () => {
  const actions: any[] = [];

  if (isStreaming.value && !shell.value?.isUser) {
    actions.push({
      label: "中止回复",
      icon: StopCircle,
      danger: true,
      handler: () => streamStore.stopMessage(props.message.id),
    });
  }

  const getFullText = async () => {
    if (props.message.content) return props.message.content;
    return await historyStore.fetchRawContent(props.message.id);
  };

  // 1. 如果不是流式，编辑消息移动到首位
  if (!isStreaming.value) {
    actions.push({
      label: "编辑消息",
      icon: Edit2,
      handler: async () => {
        const fullText = await getFullText();
        overlayStore.openEditor({
          initialValue: fullText || "",
          onSave: (newContent: string) =>
            historyStore.updateMessageContent(props.message.id, newContent),
        });
      },
    });
  }

  // 2. 复制内容紧随其后
  actions.push({
    label: "复制内容",
    icon: Copy,
    handler: async () => {
      const fullText = await getFullText();
      if (!fullText) return;
      await navigator.clipboard.writeText(fullText);
      notificationStore.addNotification({
        type: "success",
        title: "复制成功",
        message: "内容已复制到剪贴板",
      });
    },
  });

  if (!shell.value?.isUser) {
    actions.push({
      label: "渲染解析诊断",
      icon: Bug,
      handler: () => openRenderDebugReport(getFullText),
    });
  }

  // 3. 其他非流式操作
  if (!isStreaming.value) {
    actions.push({
      label: "重新渲染",
      icon: RotateCcw,
      handler: async () => {
        try {
          await historyStore.reRenderMessage(
            props.message.id,
            props.message.topicId ||
              props.message.topic_id ||
              sessionStore.currentTopicId ||
              "",
          );
          notificationStore.addNotification({
            type: "success",
            title: "重构完成",
            message: "消息内容已完成物理就地重绘与排版刷新",
            toastOnly: true,
          });
        } catch (e) {
          notificationStore.addNotification({
            type: "error",
            title: "重构失败",
            message: String(e),
            toastOnly: true,
          });
        }
      },
    });

    if (!shell.value?.isUser) {
      actions.push({
        label: "重新生成",
        icon: RotateCcw,
        handler: () => historyStore.regenerateResponse(props.message.id),
      });
    } else {
      actions.push({
        label: "编辑重发",
        icon: Edit2,
        handler: async () => {
          historyStore.editMessageContent = (await getFullText()) || "";
          historyStore.editingOriginalMessageId = props.message.id;
        },
      });
    }
  }

  actions.push({
    label: "删除消息",
    icon: Trash2,
    danger: true,
    handler: () => {
      if (confirm("确定要删除这条消息吗？")) {
        historyStore.deleteMessage(props.message.id);
      }
    },
  });

  overlayStore.openContextMenu(
    actions,
    shell.value?.isUser ? "User" : "Assistant",
  );
};

function formatTime(ts: number) {
  const date = new Date(ts);
  const year = date.getFullYear();
  const month = String(date.getMonth() + 1).padStart(2, "0");
  const day = String(date.getDate()).padStart(2, "0");
  const hours = String(date.getHours()).padStart(2, "0");
  const minutes = String(date.getMinutes()).padStart(2, "0");
  return `${year}-${month}-${day} ${hours}:${minutes}`;
}
</script>

<template>
  <div
    ref="messageContentRef"
    v-longpress="showMessageContextMenu"
    class="vcp-message-item flex flex-col w-full mb-6 animate-fade-in px-1 min-w-0"
    :data-message-id="message.id"
    :data-role="message.role"
  >
    <!-- 统一的气泡循环渲染列表 -->
    <template v-for="(bubble, bubbleIndex) in messageBubbles" :key="bubble.id">
      <template v-if="shell">
        <MessageHeader
          :is-user="shell.isUser"
          :display-name="shell.displayName"
          :name-style="{ color: shell.avatarColor }"
          :owner-type="shell.isUser ? 'user' : 'agent'"
          :owner-id="shell.isUser ? 'user_avatar' : message.agentId || agentId"
          :avatar-dominant-color="shell.avatarColor"
        />

        <ChatBubble
          :is-user="shell.isUser"
          :is-streaming="
            isStreaming && bubbleIndex === messageBubbles.length - 1
          "
          :bubble-style="getBubbleStyle(bubble)"
          :class="bubbleIndex > 0 ? 'mt-2' : ''"
        >
          <!-- 初始思考指示灯：仅在此活跃气泡没有任何已确认 blocks，且仍在流式并未吐出 tail 时显示 -->
          <ThinkingIndicator
            v-if="
              isStreaming &&
              bubbleIndex === messageBubbles.length - 1 &&
              (!message.blocks || message.blocks.length === 0) &&
              !message.tailBlock &&
              !message.tailContent
            "
          />

          <div
            class="vcp-content-blocks space-y-2 min-w-0 w-full overflow-hidden"
          >
            <template v-if="bubble.blocks && bubble.blocks.length > 0">
              <template v-for="block in bubble.blocks" :key="block.__renderKey">
                <div>
                  <RenderDocumentBlock
                    v-if="isPlainBlock(block.type)"
                    :block="block"
                    :message-id="message.id"
                    :source-id="block.__renderKey"
                    @rendered="renderHeavyContent"
                  />

                  <ToolBlock
                    v-else-if="
                      block.type === 'tool-use' || block.type === 'tool-result'
                    "
                    :type="block.type"
                    :content="block.content"
                    :block="block"
                    :default-expanded="isMessageInActiveStream"
                  />

                  <ThoughtBlock
                    v-else-if="block.type === 'thought'"
                    :block="block"
                    :message-id="message.id"
                    :source-id="block.__renderKey"
                    :default-expanded="isMessageInActiveStream"
                    @rendered="renderHeavyContent"
                  />

                  <HtmlPreviewBlock
                    v-else-if="block.type === 'html-preview'"
                    :content="block.content || ''"
                    :highlighted-content="block.highlighted_content"
                    :message-id="message.id"
                    :is-streaming="isStreaming"
                    :is-active-stream="isMessageInActiveStream"
                  />

                  <ToolSummaryBlock
                    v-else-if="block.type === 'tool-call-summary'"
                    :block="block"
                  />
                </div>
              </template>
            </template>
            <!-- 尾部流式推测渲染（只对最后一个活跃气泡生效，且正在流式、有 tailBlock 时渲染，完美拼合在气泡正文末尾） -->
            <div
              v-if="
                isStreaming &&
                bubbleIndex === messageBubbles.length - 1 &&
                message.tailBlock
              "
              class="streaming-tail opacity-90"
            >
              <RenderDocumentBlock
                v-if="isPlainBlock(message.tailBlock.type)"
                :block="message.tailBlock"
                :message-id="message.id"
                source-id="stream-tail"
                streaming
                @rendered="renderHeavyContent"
              />
              <ToolBlock
                v-else-if="
                  message.tailBlock.type === 'tool-use' ||
                  message.tailBlock.type === 'tool-result'
                "
                :type="message.tailBlock.type"
                :content="message.tailBlock.content"
                :block="message.tailBlock"
                :default-expanded="isMessageInActiveStream"
              />
              <ThoughtBlock
                v-else-if="message.tailBlock.type === 'thought'"
                :block="message.tailBlock"
                :message-id="message.id"
                source-id="stream-tail-thought"
                :default-expanded="isMessageInActiveStream"
                @rendered="renderHeavyContent"
              />
              <HtmlPreviewBlock
                v-else-if="message.tailBlock.type === 'html-preview'"
                :content="message.tailBlock.content || ''"
                :highlighted-content="message.tailBlock.highlighted_content"
                :message-id="message.id"
                :is-streaming="isStreaming"
                :is-active-stream="isMessageInActiveStream"
              />
              <ToolSummaryBlock
                v-else-if="message.tailBlock.type === 'tool-call-summary'"
                :block="message.tailBlock"
              />
            </div>
            <div
              v-if="
                isStreaming &&
                bubbleIndex === messageBubbles.length - 1 &&
                message.tailContent &&
                (!message.tailBlock ||
                  !isRenderableTailBlock(message.tailBlock.type))
              "
              class="opacity-70 italic animate-pulse"
            >
              {{ message.tailContent }}
            </div>
          </div>

          <AttachmentPreview
            v-if="
              bubbleIndex === 0 &&
              message.attachments &&
              message.attachments.length > 0
            "
            :attachments="message.attachments"
            class="pt-3 border-t border-black/5 dark:border-white/5"
          />

          <StreamingTag
            v-if="isStreaming && bubbleIndex === messageBubbles.length - 1"
          />

          <template #footer>
            <div
              class="text-[9px] mt-1.5 px-1 opacity-50 font-mono tracking-tighter w-full"
              :class="shell.isUser ? 'text-right' : 'text-left'"
            >
              {{ formatTime(message.timestamp) }}
            </div>
          </template>
        </ChatBubble>
      </template>
    </template>

    <!-- Mermaid FullScreen Viewer -->
    <MermaidFullScreenViewer
      :visible="isMermaidFullScreen"
      :svg-html="activeMermaidSvg"
      :source-code="activeMermaidSource"
      @close="isMermaidFullScreen = false"
    />
  </div>
</template>

<style scoped>
.animate-fade-in {
  animation: fadeIn 0.4s cubic-bezier(0.16, 1, 0.3, 1);
}

@keyframes fadeIn {
  from {
    opacity: 0;
    transform: translateY(10px) scale(0.98);
  }
  to {
    opacity: 1;
    transform: translateY(0) scale(1);
  }
}
</style>
