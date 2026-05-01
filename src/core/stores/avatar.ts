import { defineStore } from "pinia";
import { invoke } from "@tauri-apps/api/core";
import { reactive } from "vue";
import { RuntimeLruCache } from "../utils/runtimeLruCache";

interface AvatarCache {
  blobUrl: string;
  version: number;
  bytes: number;
}

interface AvatarResult {
  mime_type: string;
  image_data: number[];
  dominant_color: string | null;
  updated_at: number;
}

export const useAvatarStore = defineStore("avatar", () => {
  // 使用 reactive 包装 Map，配合同步访问
  const cache = reactive(new Map<string, AvatarCache>());
  const avatarLru = new RuntimeLruCache<string, AvatarCache>({
    maxBytes: 6 * 1024 * 1024,
    maxEntries: 96,
    getSize: (entry) => entry.bytes,
    dispose: (entry) => {
      URL.revokeObjectURL(entry.blobUrl);
    },
  });
  
  // 用于追踪正在进行的请求，防止并发重复请求同一个 ID
  const pending = new Map<string, Promise<string>>();

  const syncReactiveCache = () => {
    cache.clear();
    for (const key of avatarLru.keys()) {
      const value = avatarLru.peek(key);
      if (value) cache.set(key, value);
    }
  };

  const getCachedAvatar = (key: string) => {
    const existing = avatarLru.get(key);
    if (existing) {
      syncReactiveCache();
    }
    return existing;
  };

  const setCachedAvatar = (key: string, value: AvatarCache) => {
    avatarLru.set(key, value);
    syncReactiveCache();
  };

  /**
   * 获取头像 URL (带自动缓存和版本检查)
   */
  const getAvatarUrl = async (
    ownerType: string, 
    ownerId: string, 
    version: number = 0
  ): Promise<string> => {
    const key = `${ownerType}:${ownerId}`;
    const existing = getCachedAvatar(key);

    // 核心修复：如果缓存存在，且满足以下任一条件，则直接返回：
    // 1. 请求的版本为 0 (不强制刷新，只要有就行)
    // 2. 缓存的版本已经大于或等于请求的版本
    if (existing && (version === 0 || existing.version >= version)) {
      return existing.blobUrl;
    }

    // 防止并发重复请求：如果该 ID 已经在加载中，直接返回那个 Promise
    if (pending.has(key)) {
      return pending.get(key)!;
    }

    const fetchTask = (async () => {
      try {
        const result = await invoke<AvatarResult | null>("get_avatar", {
          ownerType,
          ownerId,
        });

        if (result && result.image_data) {
          // 清理旧缓存的物理内存
          if (existing) {
            URL.revokeObjectURL(existing.blobUrl);
          }

          const bytes = new Uint8Array(result.image_data);
          const blob = new Blob([bytes], { type: result.mime_type });
          const blobUrl = URL.createObjectURL(blob);

          setCachedAvatar(key, {
            blobUrl, 
            version: Math.max(result.updated_at, version),
            bytes: bytes.byteLength,
          });
          return blobUrl;
        }
      } catch (err) {
        console.error(`[AvatarStore] Failed to fetch avatar for ${key}:`, err);
      } finally {
        pending.delete(key);
      }
      return "";
    })();

    pending.set(key, fetchTask);
    return fetchTask;
  };

  /**
   * 手动清除特定头像缓存 (强制刷新)
   */
  const clearCache = (ownerType: string, ownerId: string) => {
    const key = `${ownerType}:${ownerId}`;
    avatarLru.delete(key);
    syncReactiveCache();
  };

  return {
    cache, // 暴露 cache 以供同步检查
    getAvatarUrl,
    clearCache
  };
});
