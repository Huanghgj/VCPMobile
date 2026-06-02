import { convertFileSrc } from "@tauri-apps/api/core";
import type { ChatMessage, InlineNode, MarkdownNode } from "../types/chat";
import { preloadImages } from "./resourcePreloader";

const resolveMarkdownImageSrc = (node: InlineNode): string | null => {
  if (node.type !== "image" || !node.src) return null;
  if (node.needs_asset_conversion) {
    try {
      return convertFileSrc(node.src);
    } catch (err) {
      console.warn("[MessageAssetPreloader] Failed to convert markdown image src:", err);
      return null;
    }
  }
  return node.src;
};

const collectInlineImages = (nodes: InlineNode[] | undefined, urls: Set<string>) => {
  if (!nodes) return;
  for (const node of nodes) {
    const src = resolveMarkdownImageSrc(node);
    if (src) urls.add(src);
    collectInlineImages(node.children, urls);
  }
};

const collectBlockImages = (nodes: MarkdownNode[] | undefined, urls: Set<string>) => {
  if (!nodes) return;
  for (const node of nodes) {
    collectInlineImages(node.children, urls);
    if (node.items) {
      for (const item of node.items) {
        collectBlockImages(item, urls);
      }
    }
    if (node.header) {
      for (const row of node.header) {
        collectInlineImages(row, urls);
      }
    }
    if (node.rows) {
      for (const row of node.rows) {
        for (const cell of row) {
          collectInlineImages(cell, urls);
        }
      }
    }
  }
};

export async function preloadMessageImages(messages: ChatMessage[]): Promise<void> {
  const urls = new Set<string>();

  for (const message of messages) {
    for (const attachment of message.attachments || []) {
      if (attachment.type.startsWith("image/")) {
        let src = attachment.resolvedSrc || "";
        const rawSrc = attachment.thumbnailPath || attachment.internalPath || attachment.src;
        if (!src && rawSrc) {
          if (/^(https?:|data:image\/|blob:|asset:|file:|content:)/i.test(rawSrc)) {
            src = rawSrc;
          } else {
            try {
              src = convertFileSrc(rawSrc);
            } catch (err) {
              console.warn("[MessageAssetPreloader] Failed to convert attachment image src:", err);
            }
          }
        }
        if (src) urls.add(src);
      }
    }

    for (const block of message.blocks || []) {
      collectBlockImages(block.nodes, urls);
    }
  }

  await preloadImages(urls);
}
