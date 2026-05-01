import {
  RuntimeLruCache,
  estimateStringBytes,
} from "./runtimeLruCache";

interface MarkdownRenderCacheEntry {
  html: string;
  hasMath: boolean;
  hasMermaid: boolean;
  hasMedia: boolean;
}

const MAX_MARKDOWN_HTML_BYTES = 12 * 1024 * 1024;
const MAX_MARKDOWN_HTML_ENTRIES = 600;

const markdownHtmlCache = new RuntimeLruCache<
  string,
  MarkdownRenderCacheEntry
>({
  maxBytes: MAX_MARKDOWN_HTML_BYTES,
  maxEntries: MAX_MARKDOWN_HTML_ENTRIES,
  getSize: (entry) => estimateStringBytes(entry.html) + 96,
});

const hashString = (value: string) => {
  let hash = 2166136261;
  for (let index = 0; index < value.length; index += 1) {
    hash ^= value.charCodeAt(index);
    hash = Math.imul(hash, 16777619);
  }
  return (hash >>> 0).toString(36);
};

const getMarkdownCacheKey = (content: string) =>
  `${content.length}:${hashString(content)}`;

const detectFeatures = (html: string): Omit<MarkdownRenderCacheEntry, "html"> => ({
  hasMath:
    html.includes("language-math") ||
    html.includes("math-inline") ||
    html.includes("\\(") ||
    html.includes("$$"),
  hasMermaid: html.includes("mermaid-placeholder"),
  hasMedia:
    html.includes("data-vcp-media") ||
    html.includes("<img") ||
    html.includes("<video") ||
    html.includes("<audio"),
});

export const getOrCreateMarkdownHtml = (
  content: string,
  render: () => string,
) => {
  const key = getMarkdownCacheKey(content);
  const cached = markdownHtmlCache.get(key);
  if (cached) return cached;

  const html = render();
  const entry: MarkdownRenderCacheEntry = {
    html,
    ...detectFeatures(html),
  };
  markdownHtmlCache.set(key, entry);
  return entry;
};

export const clearMarkdownRenderCache = () => {
  markdownHtmlCache.clear();
};

export const getMarkdownRenderCacheStats = () => markdownHtmlCache.stats();
