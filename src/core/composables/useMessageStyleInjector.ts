import { generate, parse, walk } from "css-tree";

const injectedStyles = new Map<string, string>();
const styleSources = new Map<string, Map<string, string>>();
const pendingRemovals = new Map<string, ReturnType<typeof setTimeout>>();

const BLOCKED_AT_RULES = new Set([
  "import",
  "namespace",
  "font-face",
  "font-feature-values",
  "font-palette-values",
  "page",
]);

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

function stableCssIdentifier(value: string): string {
  let hash = 2166136261;
  for (let index = 0; index < value.length; index += 1) {
    hash ^= value.charCodeAt(index);
    hash = Math.imul(hash, 16777619);
  }
  return (hash >>> 0).toString(36);
}

function normalizeGeneratedRootSelector(selector: string): string {
  return selector
    .replace(/#vcp-root\b/gi, "[data-vcp-generated-root]")
    .replace(
      /\[\s*id\s*=\s*(?:"vcp-root"|'vcp-root'|vcp-root)\s*(?:[is]\s*)?\]/gi,
      "[data-vcp-generated-root]",
    );
}

function isKeyframesRule(node: any): boolean {
  return (
    node?.type === "Atrule" && /^(?:-[a-z]+-)?keyframes$/i.test(node.name || "")
  );
}

function firstIdentifier(node: any): any | null {
  let result: any | null = null;
  if (!node) return result;
  walk(node, {
    visit: "Identifier",
    enter(identifier: any) {
      if (!result) result = identifier;
    },
  });
  return result;
}

function scopeSingleSelector(selector: string, scopeSelector: string): string {
  const trimmed = normalizeGeneratedRootSelector(selector.trim());
  if (!trimmed || trimmed.includes(scopeSelector)) return trimmed;

  if (/^(?::root|html|body)(?=$|[\s.#:[>+~])/i.test(trimmed)) {
    return trimmed.replace(
      /^(?::root|html|body)(?=$|[\s.#:[>+~])/i,
      scopeSelector,
    );
  }
  if (trimmed.startsWith(":") || trimmed.startsWith("::")) {
    return `${scopeSelector}${trimmed}`;
  }
  return `${scopeSelector} ${trimmed}`;
}

function isSafeCssUrl(value: string): boolean {
  const trimmed = value.trim().replace(/^['"]|['"]$/g, "");
  return /^(?:data:image\/|#)/i.test(trimmed);
}

/**
 * Parses, sanitizes and scopes message CSS through a CSS AST. Keyframes are
 * namespaced per message and animation references are rewritten with them.
 */
export function scopeMessageCss(css: string, messageId: string): string {
  if (!css.trim() || !messageId) return "";

  const scopeSelector = `[data-message-id="${escapeCssAttributeValue(messageId)}"]`;
  const keyframePrefix = `vcp-${stableCssIdentifier(messageId)}-`;

  try {
    const ast = parse(css, {
      context: "stylesheet",
      parseCustomProperty: false,
      positions: false,
    }) as any;
    const keyframeNames = new Map<string, string>();
    const keyframeRules = new WeakSet<object>();

    walk(ast, {
      enter(node: any, item: any, list: any) {
        if (node.type === "Atrule") {
          const name = String(node.name || "").toLowerCase();
          if (BLOCKED_AT_RULES.has(name)) {
            if (list && item) list.remove(item);
            return;
          }
          if (isKeyframesRule(node)) {
            const identifier = firstIdentifier(node.prelude);
            if (identifier?.name) {
              const original = identifier.name;
              const namespaced = `${keyframePrefix}${original}`;
              keyframeNames.set(original, namespaced);
              identifier.name = namespaced;
            }
            node.block?.children?.forEach((rule: any) => {
              if (rule?.type === "Rule") keyframeRules.add(rule);
            });
          }
          return;
        }

        if (node.type === "Declaration") {
          const property = String(node.property || "").toLowerCase();
          const value = generate(node.value).trim();
          if (
            (property === "position" && /^(?:fixed|sticky)$/i.test(value)) ||
            property === "behavior"
          ) {
            if (list && item) list.remove(item);
            return;
          }

          let hasUnsafeUrl = false;
          walk(node.value, {
            visit: "Url",
            enter(urlNode: any) {
              if (!isSafeCssUrl(String(urlNode.value || ""))) {
                hasUnsafeUrl = true;
              }
            },
          });
          if (hasUnsafeUrl && list && item) list.remove(item);
        }
      },
    });

    if (keyframeNames.size > 0) {
      walk(ast, {
        visit: "Declaration",
        enter(declaration: any) {
          const property = String(declaration.property || "").toLowerCase();
          if (property !== "animation" && property !== "animation-name") return;
          walk(declaration.value, {
            visit: "Identifier",
            enter(identifier: any) {
              const replacement = keyframeNames.get(identifier.name);
              if (replacement) identifier.name = replacement;
            },
          });
        },
      });
    }

    walk(ast, {
      visit: "Rule",
      enter(rule: any) {
        if (keyframeRules.has(rule) || rule.prelude?.type !== "SelectorList") {
          return;
        }

        const selectors: string[] = [];
        rule.prelude.children.forEach((selector: any) => {
          selectors.push(
            scopeSingleSelector(generate(selector), scopeSelector),
          );
        });
        rule.prelude = parse(selectors.join(","), {
          context: "selectorList",
          positions: false,
        }) as any;
      },
    });

    return generate(ast);
  } catch (error) {
    console.warn("[RendererV2] Ignored invalid message CSS", error);
    return "";
  }
}

function updateStyleElement(messageId: string): void {
  const pending = pendingRemovals.get(messageId);
  if (pending) {
    clearTimeout(pending);
    pendingRemovals.delete(messageId);
  }

  const sources = styleSources.get(messageId);
  const rawCss = sources
    ? Array.from(sources.values()).filter(Boolean).join("\n")
    : "";
  const scopedCss = scopeMessageCss(rawCss, messageId);

  if (!scopedCss) {
    const timer = setTimeout(() => {
      if ((styleSources.get(messageId)?.size || 0) > 0) return;
      document.getElementById(`style-${messageId}`)?.remove();
      injectedStyles.delete(messageId);
      pendingRemovals.delete(messageId);
    }, 50);
    pendingRemovals.set(messageId, timer);
    return;
  }

  if (injectedStyles.get(messageId) === scopedCss) return;
  injectedStyles.set(messageId, scopedCss);

  let styleElement = document.getElementById(`style-${messageId}`);
  if (!styleElement) {
    styleElement = document.createElement("style");
    styleElement.id = `style-${messageId}`;
    styleElement.setAttribute("data-vcp-scope-id", messageId);
    document.head.appendChild(styleElement);
  }
  styleElement.textContent = scopedCss;
}

export function useMessageStyleInjector() {
  const injectScopedCss = (
    css: string,
    messageId: string,
    sourceId = "legacy",
  ) => {
    if (!messageId || !sourceId) return;
    let sources = styleSources.get(messageId);
    if (!sources) {
      sources = new Map();
      styleSources.set(messageId, sources);
    }
    if (css.trim()) sources.set(sourceId, css);
    else sources.delete(sourceId);
    updateStyleElement(messageId);
  };

  const removeScopedCss = (messageId: string, sourceId?: string) => {
    if (!messageId) return;
    if (sourceId) {
      const sources = styleSources.get(messageId);
      sources?.delete(sourceId);
      if (sources?.size === 0) styleSources.delete(messageId);
    } else {
      styleSources.delete(messageId);
    }
    updateStyleElement(messageId);
  };

  return { injectScopedCss, removeScopedCss };
}
