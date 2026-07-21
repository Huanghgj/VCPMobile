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

export function buildActiveHtmlDocument(
  content: string,
  dark: boolean,
  bridgeNonce: string,
): string {
  const foreground = dark ? "#d1d5db" : "#374151";
  const scrollbar = dark ? "rgba(255,255,255,0.1)" : "rgba(0,0,0,0.1)";
  const safeNonce = JSON.stringify(bridgeNonce);
  const injections = `
    <style data-vcp-preview-style>
      ::-webkit-scrollbar { width: 5px !important; height: 5px !important; }
      ::-webkit-scrollbar-track { background: transparent !important; }
      ::-webkit-scrollbar-thumb { background: ${scrollbar} !important; border-radius: 10px !important; }
      html, body { background-color: transparent; color: ${foreground}; overflow-x: hidden !important; overflow-y: hidden !important; }
      body { margin: 0; padding: 16px; box-sizing: border-box; min-height: 0; }
      canvas, img, video, iframe { max-width: 100% !important; }
      img, canvas, svg, [style*="background-image"] { cursor: zoom-in; }
      html.vcp-preview-hidden *, html.vcp-preview-hidden *::before, html.vcp-preview-hidden *::after {
        animation-play-state: paused !important;
      }
      [data-vcp-action-pending="true"] { opacity: 0.6 !important; cursor: wait !important; }
    </style>
    <script data-vcp-preview-bridge>
      (() => {
        const nonce = ${safeNonce};
        const messageSource = ${JSON.stringify(ACTIVE_HTML_MESSAGE_SOURCE)};
        const parentSource = ${JSON.stringify(ACTIVE_HTML_PARENT_SOURCE)};
        const MAX_ACTION_LENGTH = 480;
        let parentVisible = true;
        let clipTop = 0;
        let clipBottom = Number.POSITIVE_INFINITY;
        let actionSequence = 0;
        let measureFrame = 0;
        let visibilityFrame = 0;
        let frameCallbacksEnabled = true;
        let touchX = null;
        let touchY = null;
        const pendingButtons = new Map();
        const controlledAnimations = new WeakSet();
        const pausedMedia = new WeakSet();

        const nativeRequestAnimationFrame = window.requestAnimationFrame.bind(window);
        const nativeCancelAnimationFrame = window.cancelAnimationFrame.bind(window);
        const queuedFrames = new Map();
        let frameSequence = 0;

        const post = (type, payload = {}) => {
          window.parent.postMessage({ source: messageSource, type, nonce, ...payload }, '*');
        };

        const compact = (value) => String(value || '').replace(/\\s+/g, ' ').trim();
        const inClip = (element) => {
          if (!parentVisible || !(element instanceof Element)) return false;
          const rect = element.getBoundingClientRect();
          return rect.bottom > clipTop && rect.top < clipBottom && rect.right > 0 && rect.left < innerWidth;
        };

        const shouldRunFrames = () => {
          if (!parentVisible) return false;
          const frameTargets = document.querySelectorAll('canvas, video, [data-vcp-animate]');
          if (frameTargets.length === 0) return true;
          return Array.from(frameTargets).some(inClip);
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
          post('render-size', { height: Math.max(1, Math.ceil(height)) });
        };

        const scheduleMeasure = () => {
          if (measureFrame) return;
          measureFrame = nativeRequestAnimationFrame(measure);
        };

        const syncVisibility = () => {
          visibilityFrame = 0;
          document.documentElement.classList.toggle('vcp-preview-hidden', !parentVisible);
          const elements = document.querySelectorAll('*');
          for (const element of elements) {
            const visible = inClip(element);
            if (typeof element.getAnimations === 'function') {
              for (const animation of element.getAnimations({ subtree: false })) {
                if (!visible && (animation.playState === 'running' || animation.playState === 'pending')) {
                  animation.pause();
                  controlledAnimations.add(animation);
                } else if (visible && controlledAnimations.has(animation) && animation.playState === 'paused') {
                  animation.play();
                  controlledAnimations.delete(animation);
                }
              }
            }
            if (element instanceof HTMLMediaElement) {
              if (!visible && !element.paused) {
                pausedMedia.add(element);
                element.pause();
              } else if (visible && pausedMedia.has(element)) {
                pausedMedia.delete(element);
                void element.play().catch(() => {});
              }
            }
          }
          frameCallbacksEnabled = shouldRunFrames();
          flushQueuedFrames();
        };

        const scheduleVisibilitySync = () => {
          if (visibilityFrame) return;
          visibilityFrame = nativeRequestAnimationFrame(syncVisibility);
        };

        const isLocalButton = (button) => {
          if (button.hasAttribute('data-vcp-send') || button.hasAttribute('data-send')) return false;
          if (button.closest('[data-vcp-local], [data-vcp-ui-control], [data-vcp-copy-code]')) return true;
          if (button.closest('form') && (button.type === 'submit' || button.type === 'reset')) return true;
          const handler = compact(button.getAttribute('onclick')).toLowerCase();
          if (/navigator\\.clipboard|\\.play\\s*\\(|\\.pause\\s*\\(|requestfullscreen\\s*\\(|showmodal\\s*\\(/i.test(handler)) return true;
          return /^(?:复制|点击复制|收听|点击收听|播放|暂停|刷新|关闭|展开|收起|预览|源码)$/i.test(compact(button.textContent));
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

        const prepareDocument = () => {
          document.querySelectorAll('details:not([data-vcp-collapsed])').forEach((details) => {
            if (!details.hasAttribute('open')) details.setAttribute('open', '');
          });
          document.querySelectorAll('img').forEach((image) => {
            if (!image.hasAttribute('loading')) image.loading = 'lazy';
            if (!image.hasAttribute('decoding')) image.decoding = 'async';
          });
          scheduleMeasure();
          scheduleVisibilitySync();
        };

        document.addEventListener('click', (event) => {
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

          const button = target.closest('button');
          if (!button || button.disabled || isLocalButton(button)) return;
          const action = buildAction(button);
          if (!action) return;
          event.preventDefault();
          event.stopImmediatePropagation();
          const actionId = nonce + '-' + (++actionSequence);
          pendingButtons.set(actionId, button);
          button.disabled = true;
          button.dataset.vcpActionPending = 'true';
          button.setAttribute('aria-busy', 'true');
          post('ai-action', { actionId, action });
        }, true);

        document.addEventListener('touchstart', (event) => {
          if (event.touches.length !== 1) {
            touchX = null;
            touchY = null;
            return;
          }
          touchX = event.touches[0].clientX;
          touchY = event.touches[0].clientY;
        }, { passive: true, capture: true });

        document.addEventListener('touchmove', (event) => {
          if (event.touches.length !== 1 || touchX === null || touchY === null) return;
          const nextX = event.touches[0].clientX;
          const nextY = event.touches[0].clientY;
          const deltaX = touchX - nextX;
          const deltaY = touchY - nextY;
          touchX = nextX;
          touchY = nextY;
          if (Math.abs(deltaY) <= Math.abs(deltaX) || Math.abs(deltaY) < 1) return;
          event.preventDefault();
          post('render-scroll', { deltaY });
        }, { passive: false, capture: true });

        const resetTouch = () => {
          touchX = null;
          touchY = null;
        };
        document.addEventListener('touchend', resetTouch, { passive: true, capture: true });
        document.addEventListener('touchcancel', resetTouch, { passive: true, capture: true });

        window.addEventListener('message', (event) => {
          if (event.source !== window.parent) return;
          const data = event.data;
          if (!data || data.source !== parentSource || data.nonce !== nonce) return;
          if (data.type === 'render-visibility') {
            parentVisible = Boolean(data.visible);
            clipTop = Number.isFinite(data.clipTop) ? data.clipTop : 0;
            clipBottom = Number.isFinite(data.clipBottom) ? data.clipBottom : Number.POSITIVE_INFINITY;
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
              button.disabled = false;
            }
          }
        });

        const start = () => {
          prepareDocument();
          const resizeObserver = new ResizeObserver(() => {
            scheduleMeasure();
            scheduleVisibilitySync();
          });
          resizeObserver.observe(document.documentElement);
          if (document.body) resizeObserver.observe(document.body);
          const mutationObserver = new MutationObserver(prepareDocument);
          mutationObserver.observe(document.documentElement, {
            childList: true,
            subtree: true,
            attributes: true,
            attributeFilter: ['class', 'style', 'open', 'src']
          });
          document.fonts?.ready.then(prepareDocument).catch(() => {});
          window.addEventListener('load', prepareDocument);
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

  if (/<head\b[^>]*>/i.test(content)) {
    return content.replace(/<head\b[^>]*>/i, (head) => `${head}${injections}`);
  }

  return `<!DOCTYPE html><html><head>${injections}</head><body>${content}</body></html>`;
}
