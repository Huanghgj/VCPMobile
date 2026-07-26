import type { ContentBlock } from "../types/chat";
import { generate, parse } from "css-tree";
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

function isEmptyHiddenPlaceholder(root: HTMLElement): boolean {
  const inlineDisplay = root.style.display.trim().toLowerCase();
  const ariaHidden = root.getAttribute("aria-hidden")?.toLowerCase() === "true";
  const hidden = root.hidden || inlineDisplay === "none" || ariaHidden;
  if (!hidden) return false;

  const clone = root.cloneNode(true) as HTMLElement;
  clone
    .querySelectorAll("style, script, template")
    .forEach((node) => node.remove());
  return !clone.textContent?.trim() && clone.children.length === 0;
}

function isHiddenGeneratedRoot(root: HTMLElement): boolean {
  return (
    root.hidden ||
    root.style.display.trim().toLowerCase() === "none" ||
    root.getAttribute("aria-hidden")?.toLowerCase() === "true"
  );
}

/**
 * `vcp-root` belongs to the generated document, not to the application DOM.
 * Normalize every occurrence so duplicate IDs and an early hidden placeholder
 * cannot capture or hide later response content.
 */
function normalizeGeneratedRichRoots(fragment: DocumentFragment): void {
  const roots = Array.from(
    fragment.querySelectorAll<HTMLElement>('[id="vcp-root" i]'),
  );

  for (const root of roots) {
    if (!root.isConnected && !fragment.contains(root)) continue;

    if (isEmptyHiddenPlaceholder(root)) {
      root.remove();
      continue;
    }

    if (
      isHiddenGeneratedRoot(root) &&
      root.querySelector<HTMLElement>('[id="vcp-root" i]')
    ) {
      root.replaceWith(...Array.from(root.childNodes));
      continue;
    }

    root.removeAttribute("id");
    root.dataset.vcpGeneratedRoot = "";
    root.dataset.vcpRenderVersion = String(RENDER_DOCUMENT_VERSION);
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

const CSS_NUMBER = String.raw`[+-]?(?:\d+(?:\.\d*)?|\.\d+)`;
const CSS_DIMENSION_UNITS = [
  "svmin",
  "lvmin",
  "dvmin",
  "svmax",
  "lvmax",
  "dvmax",
  "dpcm",
  "dppx",
  "svw",
  "lvw",
  "dvw",
  "svh",
  "lvh",
  "dvh",
  "svi",
  "lvi",
  "dvi",
  "svb",
  "lvb",
  "dvb",
  "vmin",
  "vmax",
  "grad",
  "turn",
  "khz",
  "rcap",
  "rch",
  "rex",
  "ric",
  "rlh",
  "rem",
  "cap",
  "deg",
  "rad",
  "dpi",
  "px",
  "cm",
  "mm",
  "in",
  "pc",
  "pt",
  "em",
  "ex",
  "ch",
  "ic",
  "lh",
  "vw",
  "vh",
  "vi",
  "vb",
  "ms",
  "hz",
  "fr",
  "q",
  "s",
  "x",
].join("|");
const GLUED_DIMENSION_BOUNDARY = new RegExp(
  `(${CSS_NUMBER})(${CSS_DIMENSION_UNITS})(?=${CSS_NUMBER}(?:${CSS_DIMENSION_UNITS}|%))`,
  "gi",
);
const GLUED_HEX_STOP = /#([0-9a-f]+)%/gi;
const VALID_HEX_LENGTHS = [8, 6, 4, 3] as const;

function restoreHexStopBoundary(match: string, body: string): string {
  // A valid hash length is more likely a color followed by a stray `%` than a
  // missing boundary, so leave that ambiguous case untouched.
  if (VALID_HEX_LENGTHS.includes(body.length as 3 | 4 | 6 | 8)) return match;

  for (const colorLength of VALID_HEX_LENGTHS) {
    const stop = body.slice(colorLength);
    if (
      colorLength < body.length &&
      /^\d{1,3}$/.test(stop) &&
      Number(stop) <= 100
    ) {
      return `#${body.slice(0, colorLength)} ${stop}%`;
    }
  }
  return match;
}

function restoreCssTokenBoundaries(value: string): string {
  return value
    .replace(GLUED_DIMENSION_BOUNDARY, "$1$2 ")
    .replace(GLUED_HEX_STOP, restoreHexStopBoundary);
}

function browserAcceptsDeclaration(property: string, value: string): boolean {
  if (!property || property.startsWith("--")) return true;
  const probe = document.createElement("span");
  probe.style.setProperty(property, value);
  return probe.style.getPropertyValue(property) !== "";
}

function repairMalformedInlineStyle(rawStyle: string): string {
  let ast: any;
  try {
    ast = parse(rawStyle, {
      context: "declarationList",
      parseCustomProperty: false,
      positions: false,
      onParseError: () => {},
    });
  } catch {
    return rawStyle;
  }

  let changed = false;
  ast.children.forEach((declaration: any) => {
    if (declaration.type !== "Declaration") return;
    const value = generate(declaration.value);
    if (browserAcceptsDeclaration(declaration.property, value)) return;

    const repaired = restoreCssTokenBoundaries(value);
    if (
      repaired === value ||
      !browserAcceptsDeclaration(declaration.property, repaired)
    ) {
      return;
    }

    try {
      declaration.value = parse(repaired, {
        context: "value",
        parseCustomProperty: false,
        positions: false,
      });
      changed = true;
    } catch {
      // Keep the original declaration when the candidate cannot be parsed.
    }
  });

  return changed ? generate(ast) : rawStyle;
}

function repairMalformedGeneratedInlineStyles(
  fragment: DocumentFragment,
): void {
  fragment.querySelectorAll<HTMLElement>("[style]").forEach((element) => {
    const rawStyle = element.getAttribute("style") || "";
    const repairedStyle = repairMalformedInlineStyle(rawStyle);
    if (repairedStyle !== rawStyle) {
      element.setAttribute("style", repairedStyle);
      element.dataset.vcpStyleRepaired = "";
    }
  });
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
  repairMalformedGeneratedInlineStyles(template.content);
  const css = extractStyles(template.content);
  normalizeGeneratedRichRoots(template.content);

  const sanitized = sanitizeMarkdownHtml(template.innerHTML, {
    allowRichHtml: true,
    allowStyleAttr: true,
  });
  const normalized = document.createElement("template");
  normalized.innerHTML = sanitized;
  normalizeGeneratedRichRoots(normalized.content);
  normalized.content.querySelectorAll("details").forEach((details) => {
    details.removeAttribute("open");
  });
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
