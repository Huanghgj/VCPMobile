import { convertFileSrc } from "@tauri-apps/api/core";
import DOMPurify from "dompurify";
import type { MarkdownNode, InlineNode } from "../types/chat";

// HTML 缓存：避免重复遍历 AST 拼接相同内容
const htmlCache = new Map<string, string>();
const MAX_CACHE_SIZE = 500;

function getCacheKey(nodes: MarkdownNode[], messageId: string, blockHash?: string | number): string {
  if (blockHash !== undefined && blockHash !== null) {
    return `${messageId}:${String(blockHash)}`;
  }
  // Fallback: If no hash provided, use a simple pointer-based or length-based key
  // since we now expect backend to provide hashes for all production data.
  return `${messageId}:len-${nodes.length}`;
}

/** 清理 AST HTML 缓存，用于重建/同步后强制重新渲染 */
export function clearHtmlCache(): void {
  htmlCache.clear();
}

/** 清理单条消息的 AST HTML 缓存，用于编辑后强制重新渲染 */
export function clearMessageCache(messageId: string): void {
  const prefix = `${messageId}:`;
  for (const key of htmlCache.keys()) {
    if (key.startsWith(prefix)) htmlCache.delete(key);
  }
}

/**
 * 将 Rust 预渲染的 AST 节点树转换为 HTML 字符串
 */
export function renderMarkdownNodes(
  nodes: MarkdownNode[], 
  messageId: string,
  blockHash?: string | number
): string {
  if (!nodes || nodes.length === 0) return '';
  const key = getCacheKey(nodes, messageId, blockHash);
  const cached = htmlCache.get(key);
  if (cached !== undefined) return cached;

  const html = sanitizeMarkdownHtml(nodes.map(node => renderNode(node, messageId)).join(''));

  // 简单的 LRU 保护：超限时清空（实际命中模式是批量命中/失效）
  if (htmlCache.size >= MAX_CACHE_SIZE) {
    htmlCache.clear();
  }
  htmlCache.set(key, html);
  return html;
}

function renderNode(node: MarkdownNode, messageId: string): string {
  switch (node.type) {
    case 'paragraph':
      return `<p>${(node.children || []).map(renderInline).join('')}</p>`;
    
    case 'heading':
      const level = node.level || 1;
      return `<h${level}>${(node.children || []).map(renderInline).join('')}</h${level}>`;
    
    case 'code_block': {
      let html = node.highlighted_html;
      if (html) {
        // 兼容旧 AST：如果 highlighted_html 是 <pre><code> 包裹内层 <pre> 的嵌套结构，提取单层
        const nestedPreMatch = html.match(/<pre[^>]*>\s*<code>([\s\S]*?)<\/code>\s*<\/pre>/i);
        if (nestedPreMatch && nestedPreMatch[1].trim().startsWith('<pre')) {
          const innerMatch = nestedPreMatch[1].match(/<pre[^>]*>([\s\S]*?)<\/pre>/i);
          if (innerMatch) {
            html = `<pre class="vcp-code-block vcp-scrollable">${innerMatch[1]}</pre>`;
          }
        }
        return sanitizeHighlightedCodeHtml(html);
      }
      return `<pre class="vcp-code-block vcp-scrollable"><code>${escapeHtml(node.code || '')}</code></pre>`;
    }
    
    case 'blockquote':
      return `<blockquote>${(node.children || []).map((n: any) => renderNode(n, messageId)).join('')}</blockquote>`;
    
    case 'list':
      const tag = node.ordered ? 'ol' : 'ul';
      const itemsHtml = (node.items || []).map(itemNodes => 
        `<li>${itemNodes.map(n => renderNode(n, messageId)).join('')}</li>`
      ).join('');
      return `<${tag}>${itemsHtml}</${tag}>`;
    
    case 'table': {
      const headerHtml = `<tr>${(node.header || []).map(cell => `<th>${(cell as any).map(renderInline).join('')}</th>`).join('')}</tr>`;
      const bodyHtml = (node.rows || []).map(row =>
        `<tr>${row.map(cell => `<td>${(cell as any).map(renderInline).join('')}</td>`).join('')}</tr>`
      ).join('');
      const wrapper = sanitizeClassList(node.wrapper_class, 'vcp-table-wrapper');
      return `<div class="${wrapper}"><table><thead>${headerHtml}</thead><tbody>${bodyHtml}</tbody></table></div>`;
    }
    
    case 'thematic_break':
      return '<hr/>';
    
    case 'mermaid':
      return `<div class="mermaid-placeholder">${escapeHtml(node.code || '')}</div>`;
    
    case 'raw_html':
      // Raw HTML nodes can be partial tags produced from a larger HTML container.
      // Sanitizing fragments one by one makes browsers auto-close tags early; the
      // complete HTML string is sanitized once in renderMarkdownNodes().
      return node.content || '';
    
    default:
      return '';
  }
}

function renderInline(node: InlineNode): string {
  switch (node.type) {
    case 'text':
      return escapeHtml(node.value || '');
    
    case 'strong':
      return `<strong>${(node.children || []).map(renderInline).join('')}</strong>`;
    
    case 'emphasis':
      return `<em>${(node.children || []).map(renderInline).join('')}</em>`;
    
    case 'strikethrough':
      return `<del>${(node.children || []).map(renderInline).join('')}</del>`;
    
    case 'code':
      return `<code>${escapeHtml(node.value || '')}</code>`;
    
    case 'link': {
      const href = sanitizeLinkUrl(node.needs_asset_conversion && node.href
        ? convertFileSrc(node.href)
        : node.href || '');
      return `<a href="${href}" title="${escapeHtml(node.title || '')}" target="_blank" rel="noopener noreferrer">${(node.children || []).map(renderInline).join('')}</a>`;
    }
    
    case 'image': {
      const src = sanitizeImageUrl(node.needs_asset_conversion && node.src
        ? convertFileSrc(node.src)
        : node.src || '');
      if (!src) return '';
      const originalSrc = node.src ? sanitizeImageUrl(node.src) : '';
      const originalAttr = originalSrc ? ` data-vcp-image-src="${originalSrc}"` : '';
      return `<img src="${src}"${originalAttr} alt="${escapeHtml(node.alt || '')}" title="${escapeHtml(node.title || '')}" loading="eager" decoding="async" class="vcp-markdown-image" />`;
    }
    
    case 'line_break':
      return '<br/>';
    
    case 'soft_break':
      return '<br/>';
    
    case 'inline_math': {
      const isDisplay = node.display_mode || false;
      const cls = isDisplay ? 'vcp-math-block no-swipe' : 'vcp-math-inline no-swipe';
      const tag = 'span';
      return `<${tag} class="${cls}" data-latex="${escapeHtml(node.content || '')}">${escapeHtml(node.content || '')}</${tag}>`;
    }
    
    case 'quoted_text':
      const innerQuote = (node.children || []).map(renderInline).join('');
      return `<span class="highlighted-quote">${innerQuote}</span>`;
    
    case 'highlight_tag':
      return `<span class="highlighted-tag">${escapeHtml(node.value || '')}</span>`;
    
    case 'alert_tag':
      return `<span class="highlighted-alert-tag">${escapeHtml(node.value || '')}</span>`;
    
    case 'raw_html_inline':
      // Keep inline open/close tag fragments intact until the final full-HTML
      // sanitization pass in renderMarkdownNodes().
      return node.content || '';
    
    default:
      return '';
  }
}

function escapeHtml(text: string): string {
  return text
    .replace(/&/g, '&amp;')
    .replace(/</g, '&lt;')
    .replace(/>/g, '&gt;')
    .replace(/"/g, '&quot;')
    .replace(/'/g, '&#039;');
}

function sanitizeMarkdownHtml(html: string): string {
  return DOMPurify.sanitize(html, {
    USE_PROFILES: { html: true, svg: true, mathMl: true },
    FORBID_TAGS: ['script', 'iframe', 'object', 'embed', 'applet', 'link', 'meta'],
    FORBID_ATTR: ['srcdoc'],
    ALLOW_UNKNOWN_PROTOCOLS: false,
    ALLOWED_URI_REGEXP: /^(?:(?:https?|mailto|tel|blob|asset|file|content):|data:image\/|\/|\.\/|\.\.\/|#)/i,
  });
}

function sanitizeHighlightedCodeHtml(html: string): string {
  return DOMPurify.sanitize(html, {
    USE_PROFILES: { html: true },
    ALLOWED_TAGS: ['pre', 'code', 'span'],
    ALLOWED_ATTR: ['class', 'style'],
  });
}

function sanitizeClassList(value: string | undefined, fallback: string): string {
  const classList = (value || fallback)
    .split(/\s+/)
    .map((item) => item.trim())
    .filter((item) => /^[A-Za-z0-9_-]+$/.test(item));
  return escapeHtml(classList.length ? classList.join(' ') : fallback);
}

function sanitizeLinkUrl(value: string): string {
  const trimmed = value.trim();
  if (!trimmed) return '';
  if (/^(https?:|mailto:|tel:|blob:|asset:)/i.test(trimmed)) {
    return escapeHtml(trimmed);
  }
  if (/^[./#]/.test(trimmed)) {
    return escapeHtml(trimmed);
  }
  return '';
}

function sanitizeImageUrl(value: string): string {
  const trimmed = value.trim();
  if (!trimmed) return '';
  if (/^(https?:|data:image\/|blob:|asset:|file:|content:)/i.test(trimmed)) {
    return escapeHtml(trimmed);
  }
  if (/^[./#]/.test(trimmed)) {
    return escapeHtml(trimmed);
  }
  return '';
}
