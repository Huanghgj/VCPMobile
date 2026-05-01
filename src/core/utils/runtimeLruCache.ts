export interface RuntimeLruCacheOptions<K, V> {
  maxEntries?: number;
  maxBytes?: number;
  getSize?: (value: V, key: K) => number;
  dispose?: (value: V, key: K) => void;
}

interface CacheEntry<V> {
  value: V;
  size: number;
  lastAccess: number;
}

export class RuntimeLruCache<K, V> {
  private readonly entries = new Map<K, CacheEntry<V>>();
  private totalBytes = 0;

  constructor(private readonly options: RuntimeLruCacheOptions<K, V> = {}) {}

  get size() {
    return this.entries.size;
  }

  get bytes() {
    return this.totalBytes;
  }

  get(key: K): V | undefined {
    const entry = this.entries.get(key);
    if (!entry) return undefined;

    entry.lastAccess = Date.now();
    this.entries.delete(key);
    this.entries.set(key, entry);
    return entry.value;
  }

  peek(key: K): V | undefined {
    return this.entries.get(key)?.value;
  }

  has(key: K) {
    return this.entries.has(key);
  }

  set(key: K, value: V, explicitSize?: number) {
    this.delete(key);

    const size = Math.max(
      0,
      explicitSize ?? this.options.getSize?.(value, key) ?? 1,
    );
    this.entries.set(key, {
      value,
      size,
      lastAccess: Date.now(),
    });
    this.totalBytes += size;
    this.evictIfNeeded();
  }

  delete(key: K) {
    const entry = this.entries.get(key);
    if (!entry) return false;

    this.entries.delete(key);
    this.totalBytes = Math.max(0, this.totalBytes - entry.size);
    this.options.dispose?.(entry.value, key);
    return true;
  }

  clear() {
    for (const [key, entry] of this.entries) {
      this.options.dispose?.(entry.value, key);
    }
    this.entries.clear();
    this.totalBytes = 0;
  }

  keys() {
    return Array.from(this.entries.keys());
  }

  values() {
    return Array.from(this.entries.values(), (entry) => entry.value);
  }

  stats() {
    return {
      entries: this.entries.size,
      bytes: this.totalBytes,
      maxEntries: this.options.maxEntries,
      maxBytes: this.options.maxBytes,
    };
  }

  private evictIfNeeded() {
    const maxEntries = this.options.maxEntries ?? Number.POSITIVE_INFINITY;
    const maxBytes = this.options.maxBytes ?? Number.POSITIVE_INFINITY;

    while (
      this.entries.size > maxEntries ||
      (this.totalBytes > maxBytes && this.entries.size > 1)
    ) {
      const oldest = this.entries.keys().next().value as K | undefined;
      if (oldest === undefined) break;
      this.delete(oldest);
    }
  }
}

export const estimateStringBytes = (value: string) => value.length * 2;

export const estimateJsonBytes = (value: unknown) => {
  try {
    return estimateStringBytes(JSON.stringify(value));
  } catch {
    return 1024;
  }
};
