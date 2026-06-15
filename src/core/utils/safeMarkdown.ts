import { marked } from "marked";
import DOMPurify from "dompurify";

marked.setOptions({
  gfm: true,
  breaks: true,
});

export function escapeHtml(text: string): string {
  return text
    .replace(/&/g, "&amp;")
    .replace(/</g, "&lt;")
    .replace(/>/g, "&gt;")
    .replace(/"/g, "&quot;")
    .replace(/'/g, "&#039;");
}

export function sanitizeMarkdownHtml(html: string): string {
  return DOMPurify.sanitize(html, {
    USE_PROFILES: { html: true },
    FORBID_TAGS: [
      "script",
      "iframe",
      "object",
      "embed",
      "applet",
      "link",
      "meta",
    ],
    FORBID_ATTR: ["srcdoc", "style"],
    ALLOW_UNKNOWN_PROTOCOLS: false,
    ALLOWED_URI_REGEXP:
      /^(?:(?:https?|mailto|tel|blob|asset):|data:image\/|\/|\.\/|\.\.\/|#)/i,
  });
}

export function renderSafeMarkdown(text: string): string {
  if (!text) return "";
  try {
    return sanitizeMarkdownHtml(marked.parse(text) as string);
  } catch (e) {
    console.error("[safeMarkdown] marked parse failed:", e);
    return escapeHtml(text);
  }
}
