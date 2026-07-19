const injectedStyles = new Map<string, string>();
const rawCssCache = new Map<string, string>();

function escapeCssAttributeValue(value: string): string {
  return value.replace(/["\\\n\r\f]/g, (char) => {
    switch (char) {
      case "\n":
        return "\\a ";
      case "\r":
        return "\\d ";
      case "\f":
        return "\\c ";
      default:
        return `\\${char}`;
    }
  });
}

function sanitizeScopedCss(css: string): string {
  return css
    .replace(
      /@(?:import|namespace|font-face|page)\b[^;{]*(?:;|\{[\s\S]*?\})/gi,
      "",
    )
    .replace(/url\(\s*(['"]?)(?!data:image\/|#)[^)]+\1\s*\)/gi, "none")
    .replace(/\bposition\s*:\s*(?:fixed|sticky)\s*;?/gi, "");
}

function findCssBoundary(css: string, start: number): number {
  let quote = "";
  let comment = false;
  let parentheses = 0;
  let brackets = 0;

  for (let index = start; index < css.length; index += 1) {
    const char = css[index];
    const next = css[index + 1];

    if (comment) {
      if (char === "*" && next === "/") {
        comment = false;
        index += 1;
      }
      continue;
    }
    if (quote) {
      if (char === "\\") {
        index += 1;
      } else if (char === quote) {
        quote = "";
      }
      continue;
    }
    if (char === "/" && next === "*") {
      comment = true;
      index += 1;
      continue;
    }
    if (char === '"' || char === "'") {
      quote = char;
      continue;
    }
    if (char === "(") parentheses += 1;
    else if (char === ")") parentheses = Math.max(0, parentheses - 1);
    else if (char === "[") brackets += 1;
    else if (char === "]") brackets = Math.max(0, brackets - 1);
    else if (
      (char === "{" || char === ";") &&
      parentheses === 0 &&
      brackets === 0
    ) {
      return index;
    }
  }

  return -1;
}

function findMatchingCssBrace(css: string, openIndex: number): number {
  let depth = 1;
  let quote = "";
  let comment = false;

  for (let index = openIndex + 1; index < css.length; index += 1) {
    const char = css[index];
    const next = css[index + 1];

    if (comment) {
      if (char === "*" && next === "/") {
        comment = false;
        index += 1;
      }
      continue;
    }
    if (quote) {
      if (char === "\\") {
        index += 1;
      } else if (char === quote) {
        quote = "";
      }
      continue;
    }
    if (char === "/" && next === "*") {
      comment = true;
      index += 1;
      continue;
    }
    if (char === '"' || char === "'") {
      quote = char;
      continue;
    }
    if (char === "{") depth += 1;
    else if (char === "}") {
      depth -= 1;
      if (depth === 0) return index;
    }
  }

  return -1;
}

function splitCssSelectorList(selectors: string): string[] {
  const result: string[] = [];
  let start = 0;
  let quote = "";
  let parentheses = 0;
  let brackets = 0;

  for (let index = 0; index < selectors.length; index += 1) {
    const char = selectors[index];
    if (quote) {
      if (char === "\\") index += 1;
      else if (char === quote) quote = "";
      continue;
    }
    if (char === '"' || char === "'") quote = char;
    else if (char === "(") parentheses += 1;
    else if (char === ")") parentheses = Math.max(0, parentheses - 1);
    else if (char === "[") brackets += 1;
    else if (char === "]") brackets = Math.max(0, brackets - 1);
    else if (char === "," && parentheses === 0 && brackets === 0) {
      result.push(selectors.slice(start, index));
      start = index + 1;
    }
  }
  result.push(selectors.slice(start));
  return result;
}

function scopeSelector(selector: string, scopeSelector: string): string {
  const trimmed = selector.trim();
  if (!trimmed) return trimmed;
  if (trimmed.includes(scopeSelector)) return trimmed;
  if (/^(?:html|body|:root)\b/i.test(trimmed)) {
    return trimmed.replace(/^(?:html|body|:root)\b/i, scopeSelector);
  }
  if (trimmed === "#vcp-root") return `${scopeSelector} #vcp-root`;
  if (trimmed.startsWith(":") || trimmed.startsWith("::")) {
    return `${scopeSelector}${trimmed}`;
  }
  return `${scopeSelector} ${trimmed}`;
}

function scopeCssBlock(css: string, scopeSelectorValue: string): string {
  let output = "";
  let cursor = 0;

  while (cursor < css.length) {
    const boundary = findCssBoundary(css, cursor);
    if (boundary < 0) {
      output += css.slice(cursor);
      break;
    }
    if (css[boundary] === ";") {
      output += css.slice(cursor, boundary + 1);
      cursor = boundary + 1;
      continue;
    }

    const close = findMatchingCssBrace(css, boundary);
    if (close < 0) {
      output += css.slice(cursor);
      break;
    }

    const rawHeader = css.slice(cursor, boundary);
    const leading = rawHeader.match(/^\s*/)?.[0] || "";
    const header = rawHeader.slice(leading.length).trim();
    const body = css.slice(boundary + 1, close);

    if (/^@(?:-[a-z]+-)?keyframes\b/i.test(header)) {
      output += css.slice(cursor, close + 1);
    } else if (/^@(media|supports|container|layer)\b/i.test(header)) {
      output += `${leading}${header} {${scopeCssBlock(body, scopeSelectorValue)}}`;
    } else if (header.startsWith("@")) {
      output += css.slice(cursor, close + 1);
    } else {
      const scopedSelectors = splitCssSelectorList(header)
        .map((selector) => scopeSelector(selector, scopeSelectorValue))
        .join(", ");
      output += `${leading}${scopedSelectors} {${body}}`;
    }
    cursor = close + 1;
  }

  return output;
}

export function scopeMessageCss(css: string, messageId: string): string {
  const scopeSelectorValue = `[data-message-id="${escapeCssAttributeValue(messageId)}"]`;
  return scopeCssBlock(sanitizeScopedCss(css), scopeSelectorValue);
}

/**
 * Composable that provides scoped style injection helpers for message bubbles.
 * Converts global-like selectors into scoped selectors targeting a specific message ID
 * to prevent user/agent-generated HTML preview styles from polluting the global application theme.
 */
export function useMessageStyleInjector() {
  /**
   * Scopes and injects raw CSS scoped to a specific message ID.
   * Modifies selectors (except keyframe definitions) to be nested under `[data-message-id="..."]`.
   */
  const injectScopedCss = (css: string, messageId: string) => {
    if (!css || !messageId) return;
    const sanitizedCss = sanitizeScopedCss(css);
    if (!sanitizedCss.trim()) return;

    // 提前去重校验：若原始 CSS 无变化，直接拦截，完全跳过后面重型 selector scoping 的正则运算
    if (rawCssCache.get(messageId) === sanitizedCss) return;
    rawCssCache.set(messageId, sanitizedCss);

    const scopedCss = scopeMessageCss(sanitizedCss, messageId);

    if (injectedStyles.get(messageId) === scopedCss) return;
    injectedStyles.set(messageId, scopedCss);

    let styleEl = document.getElementById(`style-${messageId}`);
    if (!styleEl) {
      styleEl = document.createElement("style");
      styleEl.id = `style-${messageId}`;
      styleEl.setAttribute("data-vcp-scope-id", messageId);
      document.head.appendChild(styleEl);
    }
    styleEl.textContent = scopedCss;
  };

  /**
   * Removes the scoped style element associated with a specific message ID.
   * Uses a setTimeout delay to prevent the style sheet from being instantly removed
   * and re-injected during the streaming-to-stable transition tick, which causes layout flicker.
   */
  const removeScopedCss = (messageId: string) => {
    if (!messageId) return;

    // 立即注销 rawCss 活跃状态。如果后面有新静态块接管，它会同步重新执行 injectScopedCss 重新 set 写入
    rawCssCache.delete(messageId);

    // 延迟 50ms 物理清理，给新静态块挂载和样式接管留出时间差
    setTimeout(() => {
      // 核心门禁：如果在这 50ms 期间有新块重新写入并接管了该 messageId，说明样式依然活跃，保留它
      if (rawCssCache.has(messageId)) {
        return;
      }

      const styleEl = document.getElementById(`style-${messageId}`);
      if (styleEl) {
        styleEl.remove();
      }
      injectedStyles.delete(messageId);
    }, 50);
  };

  return {
    injectScopedCss,
    removeScopedCss,
  };
}
