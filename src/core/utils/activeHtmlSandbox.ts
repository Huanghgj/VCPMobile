import { HTML_ACTION_SELECTOR } from "./htmlActions";

export const ACTIVE_HTML_SANDBOX = [
  "allow-downloads",
  "allow-forms",
  "allow-modals",
  "allow-pointer-lock",
  "allow-popups",
  "allow-popups-to-escape-sandbox",
  "allow-presentation",
  "allow-scripts",
].join(" ");

export const ACTIVE_HTML_PERMISSIONS = [
  "accelerometer",
  "autoplay",
  "clipboard-read",
  "clipboard-write",
  "encrypted-media",
  "fullscreen",
  "gamepad",
  "gyroscope",
  "picture-in-picture",
  "web-share",
  "xr-spatial-tracking",
].join("; ");

export const ACTIVE_HTML_MESSAGE_SOURCE = "vcp-mobile";
export const ACTIVE_HTML_PARENT_SOURCE = "vcp-mobile-parent";

export type ActiveHtmlMediaSources = Readonly<Record<string, string>>;

interface JavaScriptStringLiteral {
  value: string;
  end: number;
}

function readJavaScriptStringLiteral(
  source: string,
  start: number,
): JavaScriptStringLiteral | null {
  const quote = source[start];
  if (quote !== '"' && quote !== "'" && quote !== "`") return null;

  let value = "";
  for (let index = start + 1; index < source.length; index += 1) {
    const char = source[index];
    if (char === quote) return { value, end: index + 1 };
    if (quote === "`" && char === "$" && source[index + 1] === "{") {
      return null;
    }
    if (char !== "\\") {
      value += char;
      continue;
    }

    const escaped = source[index + 1];
    if (escaped === undefined) return null;
    index += 1;
    if (escaped === "\n") continue;
    if (escaped === "\r") {
      if (source[index + 1] === "\n") index += 1;
      continue;
    }
    const simpleEscapes: Record<string, string> = {
      n: "\n",
      r: "\r",
      t: "\t",
      b: "\b",
      f: "\f",
      v: "\v",
      "0": "\0",
    };
    value += simpleEscapes[escaped] ?? escaped;
  }
  return null;
}

function extractClipboardText(handler: string): string | null {
  const callPattern =
    /navigator\s*\.\s*clipboard\s*\.\s*writeText\s*\(\s*([A-Za-z_$][\w$]*|[`'"])/i;
  const call = callPattern.exec(handler);
  if (!call) return null;

  const argumentStart = call.index + call[0].length - call[1].length;
  if (["`", "'", '"'].includes(call[1])) {
    return readJavaScriptStringLiteral(handler, argumentStart)?.value ?? null;
  }

  const variable = call[1];
  const escapedVariable = variable.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
  const declarationPattern = new RegExp(
    `\\b(?:const|let|var)\\s+${escapedVariable}\\s*=\\s*([\u0060'"])`,
    "i",
  );
  const declaration = declarationPattern.exec(handler.slice(0, call.index));
  if (!declaration) return null;
  const literalStart = declaration.index + declaration[0].length - 1;
  return (
    readJavaScriptStringLiteral(handler.slice(0, call.index), literalStart)
      ?.value ?? null
  );
}

function findInlineHandlerEnd(
  source: string,
  start: number,
  quote: string,
): number {
  for (let index = source.indexOf(quote, start); index >= 0; ) {
    const tail = source.slice(index + 1);
    if (/^\s*(?:\/?>|[A-Za-z_:][-A-Za-z0-9_:.]*\s*=)/.test(tail)) {
      return index;
    }
    index = source.indexOf(quote, index + 1);
  }
  return -1;
}

function encodeCopyPayload(value: string): string {
  return encodeURIComponent(
    value.replace(/[\uD800-\uDFFF]/g, (unit, offset, source: string) => {
      const code = unit.charCodeAt(0);
      const previous = source.charCodeAt(offset - 1);
      const next = source.charCodeAt(offset + 1);
      const paired =
        (code >= 0xd800 &&
          code <= 0xdbff &&
          next >= 0xdc00 &&
          next <= 0xdfff) ||
        (code >= 0xdc00 &&
          code <= 0xdfff &&
          previous >= 0xd800 &&
          previous <= 0xdbff);
      return paired ? unit : "\uFFFD";
    }),
  );
}

/**
 * Converts recognizable generated clipboard handlers into a controlled bridge
 * action before the browser can misparse quotes inside an onclick attribute.
 */
export function rewriteGeneratedClipboardButtons(content: string): string {
  const replacements: Array<{ start: number; end: number; value: string }> = [];
  const buttonPattern = /<button\b/gi;

  for (
    let button = buttonPattern.exec(content);
    button;
    button = buttonPattern.exec(content)
  ) {
    const firstTagEnd = content.indexOf(">", button.index);
    if (firstTagEnd < 0) break;
    const openingPrefix = content.slice(button.index, firstTagEnd);
    const handlerAttribute = /\bonclick\s*=\s*(["'])/i.exec(openingPrefix);
    if (!handlerAttribute) continue;

    const attributeStart = button.index + handlerAttribute.index;
    const handlerStart =
      button.index + handlerAttribute.index + handlerAttribute[0].length;
    const quote = handlerAttribute[1];
    const handlerEnd = findInlineHandlerEnd(content, handlerStart, quote);
    if (handlerEnd < 0) continue;

    const clipboardText = extractClipboardText(
      content.slice(handlerStart, handlerEnd),
    );
    if (clipboardText === null) continue;

    replacements.push({
      start: attributeStart,
      end: handlerEnd + 1,
      value: `data-vcp-copy-code="${encodeCopyPayload(clipboardText)}" data-vcp-copy-encoded="uri" data-vcp-local`,
    });
    buttonPattern.lastIndex = handlerEnd + 1;
  }

  if (replacements.length === 0) return content;
  let result = content;
  for (let index = replacements.length - 1; index >= 0; index -= 1) {
    const replacement = replacements[index];
    result =
      result.slice(0, replacement.start) +
      replacement.value +
      result.slice(replacement.end);
  }
  return result;
}

function normalizeMediaAlias(value: string): string {
  const withoutSuffix = value.split(/[?#]/, 1)[0].replace(/\\/g, "/");
  const basename = withoutSuffix.slice(withoutSuffix.lastIndexOf("/") + 1);
  try {
    return decodeURIComponent(basename).trim().toLowerCase();
  } catch {
    return basename.trim().toLowerCase();
  }
}

function escapeHtmlAttribute(value: string, quote: string): string {
  return value
    .replace(/&/g, "&amp;")
    .replace(/</g, "&lt;")
    .replace(quote === '"' ? /"/g : /'/g, quote === '"' ? "&quot;" : "&#39;");
}

export function rewriteGeneratedMediaSources(
  content: string,
  mediaSources: ActiveHtmlMediaSources = {},
): string {
  if (Object.keys(mediaSources).length === 0) return content;

  return content.replace(/<(?:video|audio|source)\b[^>]*>/gi, (tag) =>
    tag.replace(
      /(\bsrc\s*=\s*)(["'])([^"']+)\2/i,
      (attribute, prefix: string, quote: string, rawSource: string) => {
        if (/^(?:[a-z][a-z0-9+.-]*:|\/\/|\/|#)/i.test(rawSource.trim())) {
          return attribute;
        }
        const replacement = mediaSources[normalizeMediaAlias(rawSource)];
        if (!replacement) return attribute;
        return `${prefix}${quote}${escapeHtmlAttribute(replacement, quote)}${quote}`;
      },
    ),
  );
}

export function buildActiveHtmlDocument(
  content: string,
  dark: boolean,
  bridgeNonce: string,
  mediaSources: ActiveHtmlMediaSources = {},
): string {
  const preparedContent = rewriteGeneratedMediaSources(
    rewriteGeneratedClipboardButtons(content),
    mediaSources,
  );
  const foreground = dark ? "#d1d5db" : "#374151";
  const scrollbar = dark ? "rgba(255,255,255,0.1)" : "rgba(0,0,0,0.1)";
  const safeNonce = JSON.stringify(bridgeNonce);
  const safeActionSelector = JSON.stringify(HTML_ACTION_SELECTOR);
  const injections = `
    <style data-vcp-preview-style>
      ::-webkit-scrollbar { width: 5px !important; height: 5px !important; }
      ::-webkit-scrollbar-track { background: transparent !important; }
      ::-webkit-scrollbar-thumb { background: ${scrollbar} !important; border-radius: 10px !important; }
      html, body { background-color: transparent; color: ${foreground}; overflow-x: hidden !important; overflow-y: hidden !important; touch-action: pan-y pinch-zoom; overscroll-behavior: auto; }
      body { margin: 0; padding: 16px; box-sizing: border-box; min-height: 0; }
      canvas, img, video, iframe { max-width: 100% !important; }
      img, canvas, svg, [style*="background-image"] { cursor: zoom-in; }
      html.vcp-preview-hidden *, html.vcp-preview-hidden *::before, html.vcp-preview-hidden *::after {
        animation-play-state: paused !important;
      }
      [data-vcp-action-pending="true"] { opacity: 0.6 !important; cursor: wait !important; }
      [data-vcp-copy-state="copied"] { background: #48bb78 !important; color: #052e16 !important; }
      [data-vcp-copy-state="failed"] { background: #ef4444 !important; color: #fff !important; }
    </style>
    <script data-vcp-preview-bridge>
      (() => {
        const nonce = ${safeNonce};
        const messageSource = ${JSON.stringify(ACTIVE_HTML_MESSAGE_SOURCE)};
        const parentSource = ${JSON.stringify(ACTIVE_HTML_PARENT_SOURCE)};
        const actionSelector = ${safeActionSelector};
        const bridgeSecret = typeof crypto.randomUUID === 'function'
          ? crypto.randomUUID()
          : Array.from(crypto.getRandomValues(new Uint32Array(4)), (part) => part.toString(16)).join('-');
        const nativeParentPostMessage = window.parent.postMessage.bind(window.parent);
        const MAX_ACTION_LENGTH = 480;
        let parentVisible = true;
        let clipTop = 0;
        let clipBottom = Number.POSITIVE_INFINITY;
        let actionSequence = 0;
        let copySequence = 0;
        let measureFrame = 0;
        let visibilityFrame = 0;
        let frameCallbacksEnabled = true;
        let lastMeasuredHeight = 0;
        let lastVisibilityKey = '';
        let runtimeTargetsDirty = true;
        let frameTargets = [];
        let mediaTargets = [];
        const pendingButtons = new Map();
        const pendingCopyButtons = new Map();
        const controlledAnimations = new WeakSet();
        const pausedMedia = new WeakSet();

        const nativeRequestAnimationFrame = window.requestAnimationFrame.bind(window);
        const nativeCancelAnimationFrame = window.cancelAnimationFrame.bind(window);
        const queuedFrames = new Map();
        let frameSequence = 0;

        const post = (type, payload = {}) => {
          nativeParentPostMessage({ source: messageSource, type, nonce, secret: bridgeSecret, ...payload }, '*');
        };

        const compact = (value) => String(value || '').replace(/\\s+/g, ' ').trim();
        const inClip = (element) => {
          if (!parentVisible || !(element instanceof Element)) return false;
          const rect = element.getBoundingClientRect();
          return rect.bottom > clipTop && rect.top < clipBottom && rect.right > 0 && rect.left < innerWidth;
        };

        const refreshRuntimeTargets = () => {
          if (!runtimeTargetsDirty) return;
          runtimeTargetsDirty = false;
          frameTargets = Array.from(document.querySelectorAll('canvas, video, [data-vcp-animate]'));
          mediaTargets = Array.from(document.querySelectorAll('audio, video'));
        };

        const shouldRunFrames = () => {
          if (!parentVisible) return false;
          refreshRuntimeTargets();
          if (frameTargets.length === 0) return true;
          return frameTargets.some(inClip);
        };

        const flushQueuedFrames = () => {
          if (!frameCallbacksEnabled) return;
          for (const [id, entry] of Array.from(queuedFrames.entries())) {
            if (entry.nativeId) continue;
            entry.nativeId = nativeRequestAnimationFrame((timestamp) => {
              queuedFrames.delete(id);
              entry.callback(timestamp);
            });
          }
        };

        window.requestAnimationFrame = (callback) => {
          const id = ++frameSequence;
          queuedFrames.set(id, { callback, nativeId: 0 });
          flushQueuedFrames();
          return id;
        };

        window.cancelAnimationFrame = (id) => {
          const entry = queuedFrames.get(id);
          if (entry && entry.nativeId) nativeCancelAnimationFrame(entry.nativeId);
          queuedFrames.delete(id);
        };

        const nativeSetInterval = window.setInterval.bind(window);
        window.setInterval = (handler, timeout, ...args) => {
          if (typeof handler !== 'function') return nativeSetInterval(handler, timeout, ...args);
          return nativeSetInterval(() => {
            if (parentVisible) handler(...args);
          }, timeout);
        };

        const measure = () => {
          measureFrame = 0;
          const body = document.body;
          const root = document.documentElement;
          const height = Math.max(
            body ? body.scrollHeight : 0,
            body ? body.offsetHeight : 0,
            root ? root.scrollHeight : 0,
            root ? root.offsetHeight : 0,
          );
          const nextHeight = Math.max(1, Math.ceil(height));
          if (Math.abs(nextHeight - lastMeasuredHeight) < 1) return;
          lastMeasuredHeight = nextHeight;
          post('render-size', { height: nextHeight });
        };

        const scheduleMeasure = () => {
          if (measureFrame) return;
          measureFrame = nativeRequestAnimationFrame(measure);
        };

        const syncVisibility = () => {
          visibilityFrame = 0;
          document.documentElement.classList.toggle('vcp-preview-hidden', !parentVisible);
          const animations = typeof document.getAnimations === 'function'
            ? document.getAnimations()
            : [];
          for (const animation of animations) {
            const effectTarget = animation.effect && animation.effect.target;
            const target = effectTarget instanceof Element
              ? effectTarget
              : effectTarget && effectTarget.element instanceof Element
                ? effectTarget.element
                : null;
            const visible = target instanceof Element ? inClip(target) : parentVisible;
            if (!visible && (animation.playState === 'running' || animation.playState === 'pending')) {
              animation.pause();
              controlledAnimations.add(animation);
            } else if (visible && controlledAnimations.has(animation) && animation.playState === 'paused') {
              animation.play();
              controlledAnimations.delete(animation);
            }
          }
          refreshRuntimeTargets();
          for (const media of mediaTargets) {
            const visible = inClip(media);
            if (!visible && !media.paused) {
              pausedMedia.add(media);
              media.pause();
            } else if (visible && pausedMedia.has(media)) {
              pausedMedia.delete(media);
              void media.play().catch(() => {});
            }
          }
          frameCallbacksEnabled = shouldRunFrames();
          flushQueuedFrames();
        };

        const scheduleVisibilitySync = () => {
          if (visibilityFrame) return;
          visibilityFrame = nativeRequestAnimationFrame(syncVisibility);
        };

        const actionScope = (button) => button.closest(
          '[data-vcp-action-context], article, section, li, [role="group"], [class*="card"], [class*="panel"], [class*="item"], [class*="row"]'
        );

        const buildAction = (button) => {
          const explicit = compact(button.getAttribute('data-vcp-send') || button.getAttribute('data-send'));
          if (explicit) return explicit.slice(0, MAX_ACTION_LENGTH);
          const label = compact(button.getAttribute('aria-label') || button.textContent || button.title);
          if (!label) return '';
          const scope = actionScope(button);
          const explicitContext = compact(scope && scope.getAttribute('data-vcp-action-context'));
          const heading = compact(scope && scope.querySelector('[data-vcp-title], h1, h2, h3, h4, h5, h6, .title, [class*="title"]')?.textContent);
          const description = compact(scope && scope.querySelector('[data-vcp-description], p, .description, [class*="description"]')?.textContent);
          const context = explicitContext || [heading, description].filter(Boolean).join('：');
          const action = context && !context.includes(label) ? label + '（' + context + '）' : label;
          return action.replace(/\\]\\]/g, '] ]').slice(0, MAX_ACTION_LENGTH);
        };

        const prepareSubtree = (node) => {
          const detailsNodes = [];
          const imageNodes = [];
          if (node instanceof HTMLDetailsElement) detailsNodes.push(node);
          if (node instanceof HTMLImageElement) imageNodes.push(node);
          if (typeof node.querySelectorAll === 'function') {
            detailsNodes.push(...node.querySelectorAll('details'));
            imageNodes.push(...node.querySelectorAll('img'));
          }
          detailsNodes.forEach((details) => {
            details.open = false;
          });
          imageNodes.forEach((image) => {
            if (!image.hasAttribute('decoding')) image.decoding = 'async';
          });
        };

        const prepareDocument = () => {
          prepareSubtree(document);
          runtimeTargetsDirty = true;
          scheduleMeasure();
          scheduleVisibilitySync();
        };

        const handleMutations = (mutations) => {
          let addedContent = false;
          for (const mutation of mutations) {
            for (const node of mutation.addedNodes) {
              if (!(node instanceof Element)) continue;
              prepareSubtree(node);
              addedContent = true;
            }
          }
          if (!addedContent) return;
          runtimeTargetsDirty = true;
          scheduleMeasure();
          scheduleVisibilitySync();
        };

        document.addEventListener('click', (event) => {
          if (!event.isTrusted) return;
          const target = event.target instanceof Element ? event.target : null;
          if (!target) return;

          const image = target.closest('img');
          if (image && image.dataset.vcpNativeViewer !== 'off') {
            event.preventDefault();
            event.stopImmediatePropagation();
            post('rendered-image-click', {
              image: {
                src: image.currentSrc || image.src,
                alt: image.alt || '',
                title: image.title || ''
              }
            });
            return;
          }

          const copyButton = target.closest('[data-vcp-copy-code]');
          if (copyButton instanceof HTMLElement) {
            if (copyButton.matches(':disabled') || copyButton.getAttribute('aria-disabled') === 'true') return;
            event.preventDefault();
            event.stopImmediatePropagation();
            const raw = copyButton.getAttribute('data-vcp-copy-code') || '';
            let copyText = raw;
            if (copyButton.getAttribute('data-vcp-copy-encoded') === 'uri') {
              try { copyText = decodeURIComponent(raw); } catch {}
            }
            const copyId = nonce + '-copy-' + (++copySequence);
            pendingCopyButtons.set(copyId, { button: copyButton, originalHtml: copyButton.innerHTML });
            if ('disabled' in copyButton) copyButton.disabled = true;
            copyButton.setAttribute('aria-busy', 'true');
            post('copy-text', { copyId, text: copyText });
            return;
          }

          const button = target.closest(actionSelector);
          if (!(button instanceof HTMLElement)) return;
          if (button.matches(':disabled') || button.getAttribute('aria-disabled') === 'true') return;
          const action = buildAction(button);
          if (!action) return;
          event.preventDefault();
          event.stopImmediatePropagation();
          const actionId = nonce + '-' + (++actionSequence);
          pendingButtons.set(actionId, button);
          if ('disabled' in button) button.disabled = true;
          button.dataset.vcpActionPending = 'true';
          button.setAttribute('aria-disabled', 'true');
          button.setAttribute('aria-busy', 'true');
          post('ai-action', { actionId, action });
        }, true);

        window.addEventListener('message', (event) => {
          if (event.source !== window.parent) return;
          const data = event.data;
          if (!data || data.source !== parentSource || data.nonce !== nonce) return;
          if (data.type === 'bridge-challenge') {
            post('bridge-ready');
            lastMeasuredHeight = 0;
            scheduleMeasure();
          } else if (data.type === 'render-visibility') {
            const nextVisible = Boolean(data.visible);
            const nextClipTop = Number.isFinite(data.clipTop) ? data.clipTop : 0;
            const nextClipBottom = Number.isFinite(data.clipBottom) ? data.clipBottom : Number.POSITIVE_INFINITY;
            const nextKey = nextVisible + ':' + Math.round(nextClipTop) + ':' + Math.round(nextClipBottom);
            if (nextKey === lastVisibilityKey) return;
            lastVisibilityKey = nextKey;
            parentVisible = nextVisible;
            clipTop = nextClipTop;
            clipBottom = nextClipBottom;
            scheduleVisibilitySync();
          } else if (data.type === 'ai-action-result') {
            const button = pendingButtons.get(data.actionId);
            if (!button) return;
            pendingButtons.delete(data.actionId);
            button.removeAttribute('aria-busy');
            delete button.dataset.vcpActionPending;
            if (data.success) {
              button.dataset.vcpActionSent = 'true';
            } else {
              if ('disabled' in button) button.disabled = false;
              button.removeAttribute('aria-disabled');
            }
          } else if (data.type === 'copy-result') {
            const pending = pendingCopyButtons.get(data.copyId);
            if (!pending) return;
            pendingCopyButtons.delete(data.copyId);
            const button = pending.button;
            if ('disabled' in button) button.disabled = false;
            button.removeAttribute('aria-busy');
            button.dataset.vcpCopyState = data.success ? 'copied' : 'failed';
            button.textContent = data.success ? '✓ 已复制' : '复制失败';
            window.setTimeout(() => {
              if (!button.isConnected) return;
              delete button.dataset.vcpCopyState;
              button.innerHTML = pending.originalHtml;
            }, 1500);
          }
        });

        const start = () => {
          prepareDocument();
          const resizeObserver = new ResizeObserver(() => {
            scheduleMeasure();
          });
          resizeObserver.observe(document.documentElement);
          if (document.body) resizeObserver.observe(document.body);
          const mutationObserver = new MutationObserver(handleMutations);
          mutationObserver.observe(document.documentElement, {
            childList: true,
            subtree: true
          });
          document.fonts?.ready.then(scheduleMeasure).catch(() => {});
          document.addEventListener('load', scheduleMeasure, true);
          document.addEventListener('toggle', scheduleMeasure, true);
          window.addEventListener('resize', scheduleMeasure);
          window.addEventListener('load', scheduleMeasure);
          post('render-ready');
        };

        if (document.readyState === 'loading') {
          document.addEventListener('DOMContentLoaded', start, { once: true });
        } else {
          start();
        }
      })();
    </script>
  `;

  if (/<head\b[^>]*>/i.test(preparedContent)) {
    return preparedContent.replace(
      /<head\b[^>]*>/i,
      (head) => `${head}${injections}`,
    );
  }

  return `<!DOCTYPE html><html><head>${injections}</head><body>${preparedContent}</body></html>`;
}
