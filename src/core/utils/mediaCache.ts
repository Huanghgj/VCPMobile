import { convertFileSrc } from "@tauri-apps/api/core";

import { RuntimeLruCache } from "./runtimeLruCache";
import type { Attachment } from "../stores/chatManager";

interface ThumbnailCacheEntry {
  objectUrl: string;
  bytes: number;
}

interface PreviewSourceOptions {
  allowOriginal?: boolean;
  thumbnailOnly?: boolean;
}

interface ResolvedPreviewSource {
  key: string;
  src: string;
  cacheable: boolean;
  shouldFetch: boolean;
}

const MAX_THUMBNAIL_CACHE_BYTES = 18 * 1024 * 1024;
const MAX_THUMBNAIL_CACHE_ENTRIES = 160;
const MAX_SINGLE_THUMBNAIL_BYTES = 1024 * 1024;
const MAX_ORIGINAL_IMAGE_CACHE_BYTES = 768 * 1024;

const thumbnailCache = new RuntimeLruCache<string, ThumbnailCacheEntry>({
  maxBytes: MAX_THUMBNAIL_CACHE_BYTES,
  maxEntries: MAX_THUMBNAIL_CACHE_ENTRIES,
  getSize: (entry) => entry.bytes,
  dispose: (entry) => {
    URL.revokeObjectURL(entry.objectUrl);
  },
});

const pendingLoads = new Map<string, Promise<string>>();

const isDirectWebSrc = (src: string) =>
  /^(https?:|data:|blob:|asset:|http:\/\/asset\.localhost)/i.test(src);

const normalizeLocalSrc = (src: string) => {
  if (!src) return "";
  if (isDirectWebSrc(src)) return src;

  const localPath = src.startsWith("file://") ? src.slice("file://".length) : src;
  try {
    return convertFileSrc(localPath);
  } catch {
    return "";
  }
};

const sourcePathForAttachment = (
  attachment: Attachment,
  options: PreviewSourceOptions = {},
) => {
  if (attachment.thumbnailPath) return attachment.thumbnailPath;
  if (options.thumbnailOnly) return "";
  if (attachment.resolvedSrc) return attachment.resolvedSrc;
  if (options.allowOriginal) {
    return attachment.internalPath || attachment.src;
  }
  return "";
};

const cacheKeyForAttachment = (
  attachment: Attachment,
  sourcePath: string,
) =>
  [
    attachment.hash || attachment.id || attachment.name || "attachment",
    sourcePath,
    attachment.thumbnailPath ? "thumb" : "original",
    attachment.createdAt || attachment.size || 0,
  ].join(":");

const resolvePreviewSource = (
  attachment: Attachment,
  options: PreviewSourceOptions = {},
): ResolvedPreviewSource | null => {
  const sourcePath = sourcePathForAttachment(attachment, options);
  if (!sourcePath) return null;

  const src = normalizeLocalSrc(sourcePath);
  if (!src) return null;

  const key = cacheKeyForAttachment(attachment, sourcePath);
  const hasDedicatedThumbnail = Boolean(attachment.thumbnailPath);
  const isBlobOrData = /^(blob:|data:)/i.test(src);
  const isRemote = /^https?:/i.test(src) && !src.startsWith("http://asset.localhost");
  const canFetchOriginal =
    Boolean(options.allowOriginal) &&
    attachment.type.startsWith("image/") &&
    attachment.size <= MAX_ORIGINAL_IMAGE_CACHE_BYTES;

  return {
    key,
    src,
    cacheable: !isBlobOrData && !isRemote,
    shouldFetch: hasDedicatedThumbnail || canFetchOriginal,
  };
};

export const getAttachmentPreviewSrc = (
  attachment: Attachment,
  options: PreviewSourceOptions = {},
) => {
  const resolved = resolvePreviewSource(attachment, options);
  if (!resolved) return "";

  return thumbnailCache.get(resolved.key)?.objectUrl || resolved.src;
};

export const ensureAttachmentThumbnailCached = async (
  attachment: Attachment,
  options: PreviewSourceOptions = {},
) => {
  const resolved = resolvePreviewSource(attachment, options);
  if (!resolved) return "";

  const cached = thumbnailCache.get(resolved.key);
  if (cached) return cached.objectUrl;

  if (!resolved.cacheable || !resolved.shouldFetch) {
    return resolved.src;
  }

  const pending = pendingLoads.get(resolved.key);
  if (pending) return pending;

  const task = (async () => {
    try {
      const response = await fetch(resolved.src);
      if (!response.ok) return resolved.src;

      const blob = await response.blob();
      if (blob.size > MAX_SINGLE_THUMBNAIL_BYTES) return resolved.src;

      const objectUrl = URL.createObjectURL(blob);
      thumbnailCache.set(resolved.key, {
        objectUrl,
        bytes: blob.size,
      });
      return objectUrl;
    } catch {
      return resolved.src;
    } finally {
      pendingLoads.delete(resolved.key);
    }
  })();

  pendingLoads.set(resolved.key, task);
  return task;
};

export const clearAttachmentThumbnailCache = () => {
  pendingLoads.clear();
  thumbnailCache.clear();
};

export const getAttachmentThumbnailCacheStats = () => thumbnailCache.stats();
