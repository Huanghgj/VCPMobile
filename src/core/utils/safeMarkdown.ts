import { marked } from "marked";
import DOMPurify from "dompurify";

marked.setOptions({
  gfm: true,
  breaks: true,
});

export interface SafeMarkdownOptions {
  allowStyleAttr?: boolean;
  allowRichHtml?: boolean;
}

export function escapeHtml(text: string): string {
  return text
    .replace(/&/g, "&amp;")
    .replace(/</g, "&lt;")
    .replace(/>/g, "&gt;")
    .replace(/"/g, "&quot;")
    .replace(/'/g, "&#039;");
}

export function sanitizeMarkdownHtml(
  html: string,
  options: SafeMarkdownOptions = {},
): string {
  return DOMPurify.sanitize(html, {
    USE_PROFILES: options.allowRichHtml
      ? { html: true, svg: true, mathMl: true }
      : { html: true },
    FORBID_TAGS: [
      "script",
      "iframe",
      "object",
      "embed",
      "applet",
      "link",
      "meta",
    ],
    FORBID_ATTR: options.allowStyleAttr ? ["srcdoc"] : ["srcdoc", "style"],
    ALLOW_UNKNOWN_PROTOCOLS: false,
    ALLOWED_URI_REGEXP:
      /^(?:(?:https?|mailto|tel|blob|asset):|data:image\/|\/|\.\/|\.\.\/|#)/i,
  });
}

export function renderSafeMarkdown(
  text: string,
  options: SafeMarkdownOptions = {},
): string {
  if (!text) return "";
  try {
    return sanitizeMarkdownHtml(marked.parse(text) as string, options);
  } catch (e) {
    console.error("[safeMarkdown] marked parse failed:", e);
    return escapeHtml(text);
  }
}
