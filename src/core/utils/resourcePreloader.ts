type PreloadImageOptions = {
  crossOrigin?: HTMLImageElement["crossOrigin"];
  timeoutMs?: number;
};

const imagePreloadCache = new Map<string, Promise<void>>();
const DEFAULT_IMAGE_TIMEOUT_MS = 15000;

const normalizeImageUrl = (url: string): string => {
  return url.trim();
};

/**
 * Decode and cache an image once for this WebView session.
 */
export function preloadImage(url: string, options: PreloadImageOptions = {}): Promise<void> {
  const normalized = normalizeImageUrl(url);
  if (!normalized) return Promise.resolve();

  const cached = imagePreloadCache.get(normalized);
  if (cached) return cached;

  const task = new Promise<void>((resolve, reject) => {
    const img = new Image();
    const timeoutId = window.setTimeout(() => {
      reject(new Error(`Image preload timed out: ${normalized}`));
    }, options.timeoutMs ?? DEFAULT_IMAGE_TIMEOUT_MS);

    const settle = (callback: () => void) => {
      window.clearTimeout(timeoutId);
      callback();
    };

    if (options.crossOrigin !== undefined) {
      img.crossOrigin = options.crossOrigin;
    }

    img.onload = () => {
      const decodePromise = typeof img.decode === "function"
        ? img.decode().catch(() => undefined)
        : Promise.resolve();
      decodePromise.then(() => settle(resolve));
    };
    img.onerror = () => settle(() => reject(new Error(`Image preload failed: ${normalized}`)));
    img.src = normalized;
  });

  imagePreloadCache.set(normalized, task);
  task.catch(() => {
    imagePreloadCache.delete(normalized);
  });

  return task;
}

export async function preloadImages(urls: Iterable<string>): Promise<void> {
  const uniqueUrls = Array.from(new Set(Array.from(urls).map(normalizeImageUrl).filter(Boolean)));
  const results = await Promise.allSettled(uniqueUrls.map((url) => preloadImage(url)));
  const failed = results.filter((result) => result.status === "rejected").length;
  if (failed > 0) {
    console.warn(`[ResourcePreloader] ${failed}/${uniqueUrls.length} images failed to preload`);
  }
}

export function markImagePreloaded(url: string): void {
  const normalized = normalizeImageUrl(url);
  if (!normalized || imagePreloadCache.has(normalized)) return;
  imagePreloadCache.set(normalized, Promise.resolve());
}

export function getImagePreloadCount(): number {
  return imagePreloadCache.size;
}
