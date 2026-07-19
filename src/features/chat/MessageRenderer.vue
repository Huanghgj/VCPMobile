<script setup lang="ts">
import { computed, ref, watch, nextTick, onUnmounted } from "vue";
import { invoke } from "@tauri-apps/api/core";
import type { ChatMessage, ContentBlock } from "../../core/types/chat";
import { useOverlayStore } from "../../core/stores/overlay";
import { useChatHistoryStore } from "../../core/stores/chatHistoryStore";
import { useChatSessionStore } from "../../core/stores/chatSessionStore";
import { useChatStreamStore } from "../../core/stores/chatStreamStore";
import { useNotificationStore } from "../../core/stores/notification";
import { useMessageEvents } from "../../core/composables/useMessageEvents";
import { useEmoticonFixer } from "../../core/composables/useEmoticonFixer";
import { renderMarkdownNodes } from "../../core/utils/astRenderer";
import { renderSafeMarkdown } from "../../core/utils/safeMarkdown";
import {
  getKatexRenderer,
  getMermaidRenderer,
} from "../../core/utils/renderLibraryPreloader";
import {
  applyFrame,
  cleanupRegistry,
  rebuildSnapshot,
} from "../../core/utils/astExecutor";
import { useMessageStyleInjector } from "../../core/composables/useMessageStyleInjector";
import { Copy, Edit2, RotateCcw, Trash2, StopCircle, Bug } from "lucide-vue-next";
import morphdom from "morphdom";

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

function isAstDebugEnabled(): boolean {
  return Boolean(import.meta.env.DEV && (window as any).__VCP_AST_DEBUG__);
}

function astDebugLog(...args: unknown[]): void {
  if (isAstDebugEnabled()) {
    console.warn(...args);
  }
}

// === AST Diff Feature Flags & Refs ===
const tailSandboxRef = ref<HTMLElement | null>(null);
const enableAstDiff = ref(true); // Feature Flag, 默认开启

function inlineNodesContainRawHtml(nodes?: any[]): boolean {
  if (!nodes || nodes.length === 0) return false;
  return nodes.some((node) => {
    if (!node) return false;
    if (node.type === "raw_html_inline") return true;
    return inlineNodesContainRawHtml(node.children);
  });
}

function markdownNodesContainRawHtml(nodes?: any[]): boolean {
  if (!nodes || nodes.length === 0) return false;
  return nodes.some((node) => {
    if (!node) return false;
    switch (node.type) {
      case "raw_html":
        return true;
      case "paragraph":
      case "heading":
        return inlineNodesContainRawHtml(node.children);
      case "blockquote":
        return markdownNodesContainRawHtml(node.children);
      case "list":
        return (node.items || []).some((itemNodes: any[]) =>
          markdownNodesContainRawHtml(itemNodes),
        );
      case "table":
        return (
          (node.header || []).some((cell: any[]) =>
            inlineNodesContainRawHtml(cell),
          ) ||
          (node.rows || []).some((row: any[]) =>
            row.some((cell: any[]) => inlineNodesContainRawHtml(cell)),
          )
        );
      default:
        return false;
    }
  });
}

function rawHtmlLooksLikeRichRoot(content?: string): boolean {
  if (!content) return false;
  return (
    /id\s*=\s*["']vcp-root["']/i.test(content) ||
    /data-vcp-probe\s*=/i.test(content) ||
    /style\s*=/i.test(content) ||
    /<(?:div|section|article|main|table|img|svg|canvas)\b/i.test(content)
  );
}

function inlineNodesContainRichHtml(nodes?: any[]): boolean {
  if (!nodes || nodes.length === 0) return false;
  return nodes.some((node) => {
    if (!node) return false;
    if (node.type === "raw_html_inline") {
      return rawHtmlLooksLikeRichRoot(node.content);
    }
    return inlineNodesContainRichHtml(node.children);
  });
}

function markdownNodesContainRichHtml(nodes?: any[]): boolean {
  if (!nodes || nodes.length === 0) return false;
  return nodes.some((node) => {
    if (!node) return false;
    if (node.type === "raw_html") {
      return rawHtmlLooksLikeRichRoot(node.content);
    }
    if (inlineNodesContainRichHtml(node.children)) return true;
    if (markdownNodesContainRichHtml(node.children)) return true;
    if ((node.items || []).some((itemNodes: any[]) => markdownNodesContainRichHtml(itemNodes))) {
      return true;
    }
    if ((node.header || []).some((cell: any[]) => inlineNodesContainRichHtml(cell))) {
      return true;
    }
    return (node.rows || []).some((row: any[]) =>
      row.some((cell: any[]) => inlineNodesContainRichHtml(cell)),
    );
  });
}

function blockContainsRichHtml(block: ContentBlock): boolean {
  if (block.type !== "markdown" && block.type !== "diary") return false;
  if (markdownNodesContainRichHtml(block.nodes)) return true;
  return rawHtmlLooksLikeRichRoot(block.content);
}

function tailFrameContainsRawHtml(): boolean {
  const frame = props.message.tailFrame;
  if (!frame) return false;
  if (markdownNodesContainRawHtml(frame.snapshot)) return true;
  return (frame.mutations || []).some((mutation: any) => {
    if (markdownNodesContainRawHtml([mutation.node])) return true;
    if (inlineNodesContainRawHtml([mutation.node])) return true;
    if (mutation.children && markdownNodesContainRawHtml(mutation.children)) {
      return true;
    }
    return false;
  });
}

const useAstForCurrentTail = computed(() => {
  if (!enableAstDiff.value) return false;
  // 超长 tail 降级保护：当后端因 tail 超过推测渲染上限（64KB）而停止产出 AST 节点时，
  // tailBlock 会是一个 plain 类型但 nodes 为空的纯文本块。此时必须走原始 tailContent 路径，
  // 否则 AST 沙箱会因无快照/无指令而留白。判定依据：有 plain tailBlock 却无 nodes。
  const tb = props.message.tailBlock;
  if (tb && isPlainBlock(tb.type) && (!tb.nodes || tb.nodes.length === 0)) {
    return false;
  }
  const snapshotNodes =
    tb?.nodes || props.message.tailSnapshot || props.message.tailFrame?.snapshot;
  if (markdownNodesContainRawHtml(snapshotNodes)) {
    return false;
  }
  if (tailFrameContainsRawHtml()) {
    return false;
  }
  return (
    !!props.message.tailFrame ||
    !!props.message.tailBlock?.nodes ||
    !!props.message.tailSnapshot
  );
});
let lastAppliedFrameSeq = 0;
let localTailEpoch = -1;
let localTailRevision = -1;
let astFailureCount = 0;
let lastSandbox: HTMLElement | null = null;

function getTailSnapshotNodes() {
  // 恢复/重建优先用 tailBlock.nodes（当前帧的完整 tail AST，与后端 prev_tail_ast 的 diff 基线
  // 完全一致），而非 tailSnapshot（仅在 epoch reset 时刷新，增量增长期间已过期）。
  // 用过期快照重建会导致 registry 与后端基线错位，后续增量 mutation 接连失败甚至成环。
  return props.message.tailBlock?.nodes || props.message.tailSnapshot || [];
}

function rebuildTailSnapshot(sandbox: HTMLElement): void {
  rebuildSnapshot(getTailSnapshotNodes(), props.message.id, sandbox);
  localTailEpoch = props.message.tailFrame?.epoch ?? localTailEpoch;
  localTailRevision = props.message.tailFrame?.revision ?? localTailRevision;
}

function handleAstFrameFailure(sandbox: HTMLElement, reason: string): void {
  astFailureCount += 1;
  astDebugLog(
    `[AST Diff Recovery] ${props.message.id}: ${reason}. failureCount=${astFailureCount}`,
  );
  if (getTailSnapshotNodes().length > 0) {
    rebuildTailSnapshot(sandbox);
    // 【意图性设计说明】：此处直接 return 退出，不执行下方的关闭降级逻辑，是有意为之的保活设计。
    // 在流式输出过程中，哪怕某些中间帧的 AST 增量解析/渲染出现临时局部错乱报错，我们也优先依赖
    // rebuildTailSnapshot() 在微任务/渲染帧内进行全量快照重刷，而不是彻底降级退回到普通 HTML
    // 渲染（那会导致流式组件切换、DOM 物理销毁重建以及严重的布局物理抖动和输入框焦点丢失）。
    return;
  }
  if (astFailureCount >= 2) {
    enableAstDiff.value = false;
    cleanupRegistry(props.message.id);
  }
}

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

const shouldRenderRawContentFallback = computed(() => {
  if (!props.message.content) return false;
  if (!isStreaming.value) return true;
  return !props.message.tailBlock && !props.message.tailContent;
});

const renderedContentFallback = computed(() =>
  renderSafeMarkdown(props.message.content || "", {
    allowRichHtml: true,
    allowStyleAttr: true,
  }),
);

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
      if (content.startsWith("<div") && !content.endsWith("/>") && !content.includes("</div>")) {
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

  if (props.message.blocks && props.message.blocks.length > 0) {
    for (const block of props.message.blocks) {
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
    props.message.blocks &&
    props.message.blocks.length > 0 &&
    (() => {
      const last = props.message.blocks[props.message.blocks.length - 1];
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
  return ["markdown", "diary", "role-divider", "button-click"].includes(type);
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
  }
  return style;
}

function getMarkdownBlockShell(block: ContentBlock): { className: string; attrs: string } {
  if (!blockContainsRichHtml(block)) {
    return {
      className: "vcp-markdown-block",
      attrs: "",
    };
  }
  return {
    className: "vcp-markdown-block vcp-rich-html-block",
    attrs: ' data-vcp-rich-html="true"',
  };
}

function renderBlockHtml(block: ContentBlock): string {
  switch (block.type) {
    case "markdown":
      if (block.nodes && block.nodes.length > 0) {
        if (
          block.nodes.length === 1 &&
          block.nodes[0].type === "raw_html" &&
          block.nodes[0].content?.trimStart().toLowerCase().startsWith("<style")
        ) {
          const content = block.nodes[0].content;
          let cssContent = "";
          content.replace(
            /<style\b[^>]*>([\s\S]*?)(?:<\/style>|$)/gi,
            (_, css) => {
              cssContent += css.trim() + "\n";
              return "";
            },
          );
          if (cssContent.trim().length > 0) {
            injectScopedCss(cssContent, props.message.id);
          }
          return ""; // Keep unclosed style invisible in chat body
        }
        const shell = getMarkdownBlockShell(block);
        return `<div class="${shell.className}"${shell.attrs}>${renderMarkdownNodes(block.nodes, props.message.id, block.hash)}</div>`;
      }
      const shell = getMarkdownBlockShell(block);
      return `<div class="${shell.className}"${shell.attrs}>${renderSafeMarkdown(block.content || "", { allowRichHtml: true, allowStyleAttr: true })}</div>`;

    case "diary": {
      const diaryContent =
        block.nodes && block.nodes.length > 0
          ? renderMarkdownNodes(block.nodes, props.message.id, block.hash)
          : renderSafeMarkdown(block.content || "", {
              allowRichHtml: true,
              allowStyleAttr: true,
            });
      return `
        <div class="vcp-diary-block">
          <div class="vcp-diary-header">
            <span class="vcp-diary-title">Maid's Diary</span>
            ${block.date ? `<span class="vcp-diary-date">${escapeHtml(block.date)}</span>` : ""}
          </div>
          ${
            block.maid
              ? `
            <div class="vcp-diary-maid-info">
              <span class="diary-maid-label">Maid:</span>
              <span class="vcp-diary-maid-name">${escapeHtml(block.maid)}</span>
            </div>
          `
              : ""
          }
          <div class="vcp-diary-content vcp-markdown-block">${diaryContent}</div>
        </div>
      `;
    }

    case "role-divider":
      const role = block.role || "unknown";
      const roleDisplay = role.charAt(0).toUpperCase() + role.slice(1);
      const actionText = block.is_end ? "[结束]" : "[起始]";
      const roleClass = `role-${role.toLowerCase()}`;
      const typeClass = block.is_end ? "type-end" : "type-start";

      return `
        <div class="vcp-role-divider ${roleClass} ${typeClass}">
          <span class="divider-text">角色分界: ${roleDisplay} ${actionText}</span>
        </div>
      `;

    case "button-click": {
      const escapedContent = escapeHtml(block.content || "");
      const finalText = `[[点击按钮:${block.content || ""}]]`;
      return `
        <div class="inline-block px-3 py-1 bg-black/10 dark:bg-white/10 rounded-full text-[10px] font-bold opacity-70 my-1 cursor-pointer active:opacity-40 transition-opacity select-none border border-black/5 dark:border-white/5 active:scale-95 duration-75 transform"
             data-vcp-button="${escapeHtml(finalText)}">
          ${escapedContent}
        </div>
      `;
    }

    case "style":
      return "";

    default:
      return "";
  }
}

function walkInlineDebug(nodes: any[] | undefined, stats: Record<string, number>) {
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

function walkMarkdownDebug(nodes: any[] | undefined, stats: Record<string, number>) {
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
    if (block.type === "html-preview") stats.htmlPreview = (stats.htmlPreview || 0) + 1;
    if (block.type === "button-click") stats.buttonClick = (stats.buttonClick || 0) + 1;
    if (block.type === "style") stats.styleBlocks = (stats.styleBlocks || 0) + 1;
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
      hasRawHtml: markdownNodesContainRawHtml(block.nodes),
      htmlPreviewLength: block.type === "html-preview" ? block.content?.length || 0 : undefined,
      toolName: block.tool_name,
      status: block.status,
      role: block.role,
      isEnd: block.is_end,
    })),
  };
}

function renderBlocksForDebug(blocks: ContentBlock[]): Array<Record<string, unknown>> {
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

function readElementStyleForDebug(element: Element | null): Record<string, unknown> | null {
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
    const reparsedBlocks = await invoke<ContentBlock[]>("process_message_content", {
      content: rawContent,
    });
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

  // 1. KaTeX math (inline + display mode, rendered inside markdown blocks via v-html)
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

// === Style Block CSS Injection ===
const { injectScopedCss, removeScopedCss } = useMessageStyleInjector();
let injectedStyleSignature = "";

watch(
  () => props.message.blocks,
  (blocks) => {
    if (!blocks) {
      if (injectedStyleSignature) {
        removeScopedCss(props.message.id);
        injectedStyleSignature = "";
      }
      return;
    }
    const styleBlocks = blocks.filter(
      (block) => block.type === "style" && block.content,
    );
    const styleSignature = styleBlocks
      .map((block) => block.hash || block.content)
      .join("|");
    if (!styleSignature) {
      if (injectedStyleSignature) {
        removeScopedCss(props.message.id);
        injectedStyleSignature = "";
      }
      return;
    }
    if (styleSignature === injectedStyleSignature) return;
    removeScopedCss(props.message.id);
    injectedStyleSignature = styleSignature;

    injectScopedCss(
      styleBlocks.map((block) => block.content || "").join("\n"),
      props.message.id,
    );
  },
  { immediate: true },
);

// === Stream Tail Morphdom Smooth Rendering ===
const tailRootRef = ref<HTMLElement | null>(null);
const fallbackTailSignature = computed(() => {
  const block = props.message.tailBlock;
  if (!block) return "";

  const contentSignal =
    block.hash !== undefined && block.hash !== null
      ? String(block.hash)
      : block.content || "";

  return [block.type, contentSignal, block.nodes?.length ?? 0].join("|");
});

watch(
  fallbackTailSignature,
  () => {
    const newTailBlock = props.message.tailBlock;
    if (useAstForCurrentTail.value) return; // 🆕 启用 AST Diff 且有节点时跳过 Morphdom
    if (!newTailBlock || !isPlainBlock(newTailBlock.type)) return;
    nextTick(() => {
      if (!tailRootRef.value) return;
      const html = renderBlockHtml(newTailBlock);

      // 实时提取未闭合/已闭合的 <style> 并物理抹除以防 morphdom 崩溃
      let cssContent = "";
      const processedHtml = html.replace(
        /<style\b[^>]*>([\s\S]*?)(?:<\/style>|$)/gi,
        (_, css) => {
          cssContent += css.trim() + "\n";
          return ""; // 从正文 HTML 中抹除 style 标签
        },
      );

      if (cssContent.trim().length > 0) {
        injectScopedCss(cssContent, props.message.id);
      }

      try {
        morphdom(tailRootRef.value, `<div>${processedHtml}</div>`, {
          childrenOnly: true,
          getNodeKey: (node: Node) => {
            if (!node || node.nodeType !== 1) return undefined;
            const el = node as Element;
            return el.id || el.getAttribute("data-vcp-key") || undefined;
          },
          onBeforeElUpdated: (fromEl: HTMLElement, toEl: HTMLElement) => {
            if (fromEl.isEqualNode(toEl)) return false;

            // 1. 保留可能存在的过渡/动画 class，防止 morphdom 移除它们
            const animClasses = [
              "vcp-stream-element-fade-in",
              "animate-fade-in",
              "vcp-stream-content-pulse",
            ];
            for (const cls of animClasses) {
              if (fromEl.classList.contains(cls)) {
                toEl.classList.add(cls);
              }
            }

            // 2. 保留媒体播放状态
            if (fromEl.tagName === "VIDEO" || fromEl.tagName === "AUDIO") {
              const mediaEl = fromEl as HTMLMediaElement;
              if (!mediaEl.paused) return false;
            }

            // 3. 保留输入焦点
            if (fromEl === document.activeElement) {
              requestAnimationFrame(() => {
                if (toEl && typeof toEl.focus === "function") toEl.focus();
              });
            }

            // 4. 保留已加载图片的可见性和状态，防止重新加载闪烁
            if (fromEl.tagName === "IMG") {
              const fromImg = fromEl as HTMLImageElement;
              const toImg = toEl as HTMLImageElement;
              if (fromImg.onerror && !toImg.onerror)
                toImg.onerror = fromImg.onerror;
              if (fromImg.onload && !toImg.onload)
                toImg.onload = fromImg.onload;
              if (fromImg.style.visibility)
                toImg.style.visibility = fromImg.style.visibility;
              if (fromImg.complete && fromImg.naturalWidth > 0) return false;
            }

            return true;
          },
        });
      } catch (e) {
        console.debug("[TailMorphdom] Skipped frame:", e);
      }
    });
  },
  { immediate: true, flush: "post" },
);

// === AST Diff Executor ===

watch(
  [
    () => props.message.tailFrame,
    () => props.message.tailSnapshot,
    tailSandboxRef,
  ],
  ([frame, _snapshot, sandbox]) => {
    astDebugLog(
      `[AST Diff Watch] Msg ${props.message.id} frame=${frame ? frame.frameSeq : "none"}, mutations=${frame?.mutations?.length || 0}, sandbox=${sandbox ? "Ready" : "Null"}, epoch=${frame?.epoch}, revision=${frame?.revision}`,
    );

    if (!useAstForCurrentTail.value || !sandbox) {
      if (lastSandbox) {
        cleanupRegistry(props.message.id);
        lastSandbox.innerHTML = "";
        lastSandbox = null;
      }
      return;
    }

    if (lastSandbox !== sandbox) {
      cleanupRegistry(props.message.id);
      sandbox.innerHTML = "";
      lastAppliedFrameSeq = 0;
      localTailEpoch = -1;
      localTailRevision = -1;
      lastSandbox = sandbox;
      if (getTailSnapshotNodes().length > 0) {
        rebuildTailSnapshot(sandbox); // 内部已将 localTailEpoch/Revision 同步到当前 frame
        astFailureCount = 0;
        // 认领当前帧，避免下方 reset 分支对同一帧重复重建（新 sandbox 时 localTailEpoch 刚被重置，
        // epochChanged 必为真，会触发第二次全量重建）。重建已用当前完整 tail AST，无需再来一次。
        if (frame) {
          lastAppliedFrameSeq = frame.frameSeq;
        }
      }
    }

    if (!frame) {
      return;
    }

    if (frame.frameSeq <= lastAppliedFrameSeq) {
      return;
    }

    const incomingEpoch = frame.epoch ?? 0;
    const incomingRevision = frame.revision ?? -1;
    const epochChanged = incomingEpoch !== localTailEpoch;
    const explicitReset = frame.reset === true || epochChanged;

    if (explicitReset) {
      sandbox.innerHTML = "";
      cleanupRegistry(props.message.id);
      localTailEpoch = incomingEpoch;
      localTailRevision = incomingRevision;
      lastAppliedFrameSeq = frame.frameSeq;
      astFailureCount = 0;

      const snapshot = frame.snapshot || getTailSnapshotNodes();
      if (snapshot.length > 0) {
        rebuildSnapshot(snapshot, props.message.id, sandbox);
        return;
      }
    }

    const mutations = frame.mutations || [];
    if (mutations.length === 0) {
      lastAppliedFrameSeq = frame.frameSeq;
      localTailRevision = incomingRevision;
      return;
    }

    astDebugLog(
      `[AST Diff Apply] Executing frame ${frame.frameSeq} (${mutations.length} mutations) for ${props.message.id}`,
    );
    const result = applyFrame(mutations, props.message.id, sandbox);
    if (result.ok) {
      lastAppliedFrameSeq = frame.frameSeq;
      localTailRevision = incomingRevision;
      astFailureCount = 0;
    } else {
      handleAstFrameFailure(
        sandbox,
        result.failed?.reason || "applyFrame failed",
      );
    }
  },
  { flush: "post", immediate: true },
);

onUnmounted(() => {
  removeScopedCss(props.message.id);
  cleanupRegistry(props.message.id);
});
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
              <template
                v-for="block in bubble.blocks"
                :key="block.__renderKey"
              >
                <div>
                  <div
                    v-if="isPlainBlock(block.type)"
                    v-html="renderBlockHtml(block)"
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
                    :default-expanded="isMessageInActiveStream"
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
            <template
              v-else-if="bubbleIndex === 0 && shouldRenderRawContentFallback"
            >
              <div
                class="vcp-markdown-block select-text"
                v-html="renderedContentFallback"
              />
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
              <div
                v-if="
                  useAstForCurrentTail && isPlainBlock(message.tailBlock.type)
                "
              >
                <div
                  :ref="
                    (el) => {
                      tailSandboxRef = el as HTMLElement | null;
                    }
                  "
                  class="vcp-markdown-block vcp-ast-sandbox"
                />
              </div>
              <div
                v-else-if="
                  !useAstForCurrentTail && isPlainBlock(message.tailBlock.type)
                "
                :ref="
                  (el) => {
                    tailRootRef = el as HTMLElement | null;
                  }
                "
                class="vcp-markdown-block"
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
                :default-expanded="isMessageInActiveStream"
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
