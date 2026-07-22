import type { ContentBlock } from "../types/chat";
import { renderMarkdownNodesToHtml } from "./astRenderer";
import {
  escapeHtml,
  renderMarkdownToHtml,
  sanitizeMarkdownHtml,
} from "./safeMarkdown";

export const RENDER_DOCUMENT_VERSION = 2 as const;

export interface RenderFragmentV2 {
  version: typeof RENDER_DOCUMENT_VERSION;
  html: string;
  css: string;
  rich: boolean;
  signature: string;
}

export interface RenderDocumentV2 {
  version: typeof RENDER_DOCUMENT_VERSION;
  blocks: ContentBlock[];
  tail: ContentBlock | null;
}

export function createRenderDocument(
  blocks: ContentBlock[] | undefined,
  tail: ContentBlock | undefined,
  fallbackContent = "",
): RenderDocumentV2 {
  const stableBlocks = blocks?.length
    ? blocks
    : fallbackContent
      ? [{ type: "markdown", content: fallbackContent } as ContentBlock]
      : [];
  return {
    version: RENDER_DOCUMENT_VERSION,
    blocks: stableBlocks,
    tail: tail || null,
  };
}

export function isRenderDocumentBlock(type: string): boolean {
  return [
    "markdown",
    "diary",
    "role-divider",
    "button-click",
    "style",
  ].includes(type);
}

export function blockContainsRichHtml(block: ContentBlock): boolean {
  if (block.type === "style" || block.type === "html-preview") return true;
  if (block.type !== "markdown" && block.type !== "diary") return false;
  const content = block.content || collectRawHtml(block.nodes);
  return (
    /id\s*=\s*["']vcp-root["']/i.test(content) ||
    /data-vcp-[\w-]+\s*=/i.test(content) ||
    /style\s*=/i.test(content) ||
    /<(?:div|section|article|main|table|img|svg|canvas)\b/i.test(content)
  );
}

function collectRawHtml(nodes: ContentBlock["nodes"]): string {
  if (!nodes?.length) return "";
  const chunks: string[] = [];
  const visitInline = (inlineNodes: any[] | undefined) => {
    for (const node of inlineNodes || []) {
      if (node?.type === "raw_html_inline" && node.content) {
        chunks.push(node.content);
      }
      visitInline(node?.children);
    }
  };
  const visitBlocks = (blockNodes: any[] | undefined) => {
    for (const node of blockNodes || []) {
      if (node?.type === "raw_html" && node.content) chunks.push(node.content);
      visitInline(node?.children);
      visitBlocks(node?.children);
      for (const item of node?.items || []) visitBlocks(item);
      for (const cell of node?.header || []) visitInline(cell);
      for (const row of node?.rows || []) {
        for (const cell of row) visitInline(cell);
      }
    }
  };
  visitBlocks(nodes);
  return chunks.join("");
}

function renderBlockSource(block: ContentBlock, messageId: string): string {
  switch (block.type) {
    case "markdown":
    case "thought":
      return block.nodes?.length
        ? renderMarkdownNodesToHtml(block.nodes, messageId)
        : renderMarkdownToHtml(block.content || "");
    case "diary": {
      const content = block.nodes?.length
        ? renderMarkdownNodesToHtml(block.nodes, messageId)
        : renderMarkdownToHtml(block.content || "");
      return `<section class="vcp-diary-block"><header class="vcp-diary-header"><span class="vcp-diary-title">Maid's Diary</span>${
        block.date
          ? `<span class="vcp-diary-date">${escapeHtml(block.date)}</span>`
          : ""
      }</header>${
        block.maid
          ? `<div class="vcp-diary-maid-info"><span class="diary-maid-label">Maid:</span><span class="vcp-diary-maid-name">${escapeHtml(block.maid)}</span></div>`
          : ""
      }<div class="vcp-diary-content">${content}</div></section>`;
    }
    case "role-divider": {
      const role = block.role || "unknown";
      const roleClass = `role-${role.toLowerCase().replace(/[^a-z0-9_-]/g, "")}`;
      const typeClass = block.is_end ? "type-end" : "type-start";
      return `<div class="vcp-role-divider ${roleClass} ${typeClass}"><span class="divider-text">角色分界: ${escapeHtml(role)} ${block.is_end ? "[结束]" : "[开始]"}</span></div>`;
    }
    case "button-click": {
      const content = block.content || "";
      const payload = `[[点击按钮:${content}]]`;
      return `<button type="button" class="inline-block px-3 py-1 bg-black/10 dark:bg-white/10 rounded-full text-[10px] font-bold opacity-70 my-1 cursor-pointer active:opacity-40 transition-opacity select-none border border-black/5 dark:border-white/5 active:scale-95 duration-75 transform" data-vcp-button="${escapeHtml(payload)}">${escapeHtml(content)}</button>`;
    }
    case "style":
      return `<style>${block.content || ""}</style>`;
    default:
      return "";
  }
}

function moveTrailingNodesIntoRichRoot(fragment: DocumentFragment): void {
  const root = fragment.querySelector<HTMLElement>("#vcp-root");
  if (!root) return;
  root.dataset.vcpRenderVersion = String(RENDER_DOCUMENT_VERSION);

  let topLevel: Node = root;
  while (topLevel.parentNode && topLevel.parentNode !== fragment) {
    topLevel = topLevel.parentNode;
  }
  if (topLevel.parentNode !== fragment) return;

  let sibling = topLevel.nextSibling;
  while (sibling) {
    const next = sibling.nextSibling;
    root.appendChild(sibling);
    sibling = next;
  }
}

function extractStyles(fragment: DocumentFragment): string {
  const styles: string[] = [];
  fragment.querySelectorAll("style").forEach((style) => {
    if (style.textContent?.trim()) styles.push(style.textContent);
    style.remove();
  });
  return styles.join("\n");
}

function assignStableRenderKeys(
  parent: DocumentFragment | Element,
  messageId: string,
  path: number[] = [],
): void {
  let elementIndex = 0;
  for (const child of Array.from(parent.children)) {
    const childPath = [...path, elementIndex];
    if (!child.id && !child.hasAttribute("data-vcp-key")) {
      child.setAttribute(
        "data-vcp-render-key",
        `v2-${hashString(messageId)}-${childPath.join("-")}`,
      );
    }
    assignStableRenderKeys(child, messageId, childPath);
    elementIndex += 1;
  }
}

function html5NormalizeAndSanitize(
  sourceHtml: string,
  messageId: string,
): {
  html: string;
  css: string;
} {
  const template = document.createElement("template");
  template.innerHTML = sourceHtml;
  const css = extractStyles(template.content);
  moveTrailingNodesIntoRichRoot(template.content);

  const sanitized = sanitizeMarkdownHtml(template.innerHTML, {
    allowRichHtml: true,
    allowStyleAttr: true,
  });
  const normalized = document.createElement("template");
  normalized.innerHTML = sanitized;
  moveTrailingNodesIntoRichRoot(normalized.content);
  assignStableRenderKeys(normalized.content, messageId);
  return { html: normalized.innerHTML, css };
}

function hashString(value: string): string {
  let hash = 2166136261;
  for (let index = 0; index < value.length; index += 1) {
    hash ^= value.charCodeAt(index);
    hash = Math.imul(hash, 16777619);
  }
  return (hash >>> 0).toString(36);
}

export function compileRenderFragment(
  block: ContentBlock,
  messageId: string,
): RenderFragmentV2 {
  try {
    const sourceHtml = renderBlockSource(block, messageId);
    const normalized = html5NormalizeAndSanitize(sourceHtml, messageId);
    const signature = `${RENDER_DOCUMENT_VERSION}:${String(block.hash || "")}:${hashString(sourceHtml)}:${hashString(normalized.css)}`;
    return {
      version: RENDER_DOCUMENT_VERSION,
      html: normalized.html,
      css: normalized.css,
      rich: blockContainsRichHtml(block),
      signature,
    };
  } catch (error) {
    console.error("[RendererV2] Fragment compilation failed", error);
    const fallback = escapeHtml(block.content || "");
    return {
      version: RENDER_DOCUMENT_VERSION,
      html: fallback,
      css: "",
      rich: false,
      signature: `${RENDER_DOCUMENT_VERSION}:fallback:${hashString(fallback)}`,
    };
  }
}
