<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, ref } from "vue";
import DOMPurify from "dompurify";
import { Grip, Maximize2, RotateCw, Trash2, X } from "lucide-vue-next";
import { callVcpToolboxApi } from "../../core/api/vcpToolbox";
import { useThemeStore } from "../../core/stores/theme";
import { useSurfaceStore } from "./surfaceStore";
import { repairSurfaceHtml } from "./surfaceProtocol";
import type { SurfaceWidget } from "../../core/api/surfaceBridge";

const surfaceStore = useSurfaceStore();
const themeStore = useThemeStore();
const activeFullscreen = ref<SurfaceWidget | null>(null);
const SURFACE_WIDGET_Z_INDEX_BASE = 1;
const SURFACE_CLEAR_Z_INDEX = 2515;
const SURFACE_FULLSCREEN_Z_INDEX = 2520;

const widgets = computed(() => surfaceStore.orderedWidgets);
const dragState = ref<{
  widgetId: string;
  startX: number;
  startY: number;
  originalX: number;
  originalY: number;
} | null>(null);
const resizeState = ref<{
  widgetId: string;
  pointerId: number;
  startX: number;
  startY: number;
  originalWidth: number;
  originalHeight: number;
} | null>(null);

const clamp = (value: number, min: number, max: number) =>
  Math.min(max, Math.max(min, value));

const persistWidget = async (widget: SurfaceWidget) => {
  await surfaceStore.upsertWidget({
    id: widget.id,
    title: widget.title,
    html: widget.html,
    bounds: { ...widget.bounds },
    favorite: widget.favorite,
    source: widget.source,
  });
};

const bringToFront = (widget: SurfaceWidget) => {
  const nextZIndex = Date.now();
  if (widget.bounds.zIndex === nextZIndex) return;
  widget.bounds.zIndex = nextZIndex;
  void persistWidget(widget);
};

const surfaceEventAttributes = [
  "onclick",
  "ondblclick",
  "oninput",
  "onchange",
  "onsubmit",
  "onkeydown",
  "onkeyup",
  "onpointerdown",
  "onpointermove",
  "onpointerup",
  "onpointercancel",
  "onmousedown",
  "onmousemove",
  "onmouseup",
  "ontouchstart",
  "ontouchmove",
  "ontouchend",
  "onmouseenter",
  "onmouseleave",
];

const getSandboxHtml = (content: string) => {
  const isDark = themeStore.isDarkResolved;
  const cleanHtml = DOMPurify.sanitize(repairSurfaceHtml(content), {
    USE_PROFILES: { html: true, svg: true, mathMl: true },
    ADD_TAGS: ["style", "iframe", "canvas", "script", "link", "meta"],
    ADD_ATTR: surfaceEventAttributes,
    FORBID_TAGS: ["applet", "embed", "object"],
    ALLOW_UNKNOWN_PROTOCOLS: true,
    WHOLE_DOCUMENT: true,
    RETURN_DOM: false,
  });

  const injections = `
    <meta name="viewport" content="width=device-width, initial-scale=1.0, viewport-fit=cover">
    <style>
      html, body {
        margin: 0;
        min-height: 100%;
        background: transparent !important;
        color: ${isDark ? "#f3f4f6" : "#111827"};
        font-family: system-ui, -apple-system, BlinkMacSystemFont, "Segoe UI", sans-serif;
      }
      * { box-sizing: border-box; }
      img, video, canvas, svg, iframe { max-width: 100%; }
      button, input, textarea, select { font: inherit; }
      #vcp-surface-toast {
        position: fixed;
        left: 12px;
        right: 12px;
        bottom: 12px;
        z-index: 2147483647;
        padding: 10px 12px;
        border-radius: 12px;
        background: rgba(15, 23, 42, 0.92);
        color: #fff;
        font-size: 13px;
        line-height: 1.45;
        box-shadow: 0 12px 32px rgba(0,0,0,0.28);
        opacity: 0;
        transform: translateY(8px);
        transition: opacity 0.18s ease, transform 0.18s ease;
        pointer-events: none;
      }
      #vcp-surface-toast[data-show="true"] {
        opacity: 1;
        transform: translateY(0);
      }
      ::-webkit-scrollbar { width: 5px; height: 5px; }
      ::-webkit-scrollbar-thumb { background: ${isDark ? "rgba(255,255,255,0.22)" : "rgba(0,0,0,0.22)"}; border-radius: 999px; }
    </style>
    <` + `script>
      (function() {
        function createMemoryStorage() {
          var data = Object.create(null);
          return {
            getItem: function(key) {
              key = String(key);
              return Object.prototype.hasOwnProperty.call(data, key) ? data[key] : null;
            },
            setItem: function(key, value) {
              data[String(key)] = String(value);
            },
            removeItem: function(key) {
              delete data[String(key)];
            },
            clear: function() {
              data = Object.create(null);
            },
            key: function(index) {
              return Object.keys(data)[index] || null;
            },
            get length() {
              return Object.keys(data).length;
            }
          };
        }
        function installStorageFallback(name) {
          var storage;
          try {
            storage = window[name];
            var probeKey = "__vcp_surface_storage_probe__";
            storage.setItem(probeKey, "1");
            storage.removeItem(probeKey);
            return;
          } catch (error) {
            storage = createMemoryStorage();
          }
          try {
            Object.defineProperty(window, name, {
              configurable: true,
              value: storage
            });
          } catch (error) {
            window["__vcpSurface" + name] = storage;
          }
        }
        installStorageFallback("localStorage");
        installStorageFallback("sessionStorage");
        var pending = {};
        var seq = 0;
        function callSurfaceApi(method, payload) {
          return new Promise(function(resolve, reject) {
            var requestId = ++seq;
            pending[requestId] = { resolve: resolve, reject: reject };
            window.parent.postMessage({
              type: "vcp-surface-api-request",
              requestId: requestId,
              method: method,
              payload: payload || {}
            }, "*");
            window.setTimeout(function() {
              if (!pending[requestId]) return;
              delete pending[requestId];
              reject(new Error("VCP Mobile Surface API 请求超时。"));
            }, 15000);
          });
        }
        window.addEventListener("message", function(event) {
          var data = event.data || {};
          if (data.type !== "vcp-surface-api-response" || !pending[data.requestId]) return;
          var callbacks = pending[data.requestId];
          delete pending[data.requestId];
          if (data.ok) {
            callbacks.resolve(data.result);
          } else {
            callbacks.reject(new Error(data.error || "VCP Mobile Surface API 调用失败。"));
          }
        });
        window.vcpAPI = window.vcpAPI || {
          weather: function() {
            return callSurfaceApi("weather");
          },
          fetch: function(path, options) {
            return callSurfaceApi("fetch", { path: path, options: options || {} });
          },
          post: function(path, body, options) {
            return callSurfaceApi("post", { path: path, body: body, options: options || {} });
          }
        };
        window.alert = function(message) {
          var toast = document.getElementById("vcp-surface-toast");
          if (!toast) {
            toast = document.createElement("div");
            toast.id = "vcp-surface-toast";
            document.body.appendChild(toast);
          }
          toast.textContent = String(message == null ? "" : message);
          toast.dataset.show = "true";
          window.clearTimeout(window.__vcpSurfaceToastTimer);
          window.__vcpSurfaceToastTimer = window.setTimeout(function() {
            toast.dataset.show = "false";
          }, 2200);
        };
        window.addEventListener("error", function(event) {
          var message = event && event.message ? event.message : "Surface 脚本执行失败。";
          window.alert(message);
        });
        window.addEventListener("unhandledrejection", function(event) {
          var reason = event && event.reason;
          window.alert(reason && reason.message ? reason.message : String(reason || "Surface 异步任务失败。"));
        });
      })();
      window.musicAPI = window.musicAPI || {
        getState: async function() { return null; },
        play: async function() {},
        pause: async function() {},
        seek: async function() {},
        setVolume: async function() {},
        send: async function() {}
      };
    <` + `/script>
  `;

  if (/<head[^>]*>/i.test(cleanHtml)) {
    return cleanHtml.replace(/<head[^>]*>/i, `$&${injections}`);
  }
  return `<!DOCTYPE html><html><head>${injections}</head><body>${cleanHtml}</body></html>`;
};

const widgetStyle = (widget: SurfaceWidget, stackIndex: number) => ({
  transform: `translate3d(${widget.bounds.x}px, ${widget.bounds.y}px, 0)`,
  width: `${Math.min(widget.bounds.width, window.innerWidth - 28)}px`,
  height: `${Math.min(widget.bounds.height, window.innerHeight - 150)}px`,
  zIndex: SURFACE_WIDGET_Z_INDEX_BASE + stackIndex,
});

const beginDrag = (event: PointerEvent, widget: SurfaceWidget) => {
  (event.currentTarget as HTMLElement).setPointerCapture(event.pointerId);
  bringToFront(widget);
  dragState.value = {
    widgetId: widget.id,
    startX: event.clientX,
    startY: event.clientY,
    originalX: widget.bounds.x,
    originalY: widget.bounds.y,
  };
};

const moveDrag = (event: PointerEvent) => {
  const state = dragState.value;
  if (!state) return;

  const widget = surfaceStore.widgets.find(
    (item) => item.id === state.widgetId,
  );
  if (!widget) return;

  const maxX = Math.max(12, window.innerWidth - 48);
  const maxY = Math.max(72, window.innerHeight - 88);
  widget.bounds.x = clamp(state.originalX + event.clientX - state.startX, 8, maxX);
  widget.bounds.y = clamp(state.originalY + event.clientY - state.startY, 54, maxY);
};

const endDrag = async () => {
  const state = dragState.value;
  dragState.value = null;
  if (!state) return;

  const widget = surfaceStore.widgets.find((item) => item.id === state.widgetId);
  if (!widget) return;

  await persistWidget(widget);
};

const beginResize = (event: PointerEvent, widget: SurfaceWidget) => {
  (event.currentTarget as HTMLElement).setPointerCapture(event.pointerId);
  bringToFront(widget);
  resizeState.value = {
    widgetId: widget.id,
    pointerId: event.pointerId,
    startX: event.clientX,
    startY: event.clientY,
    originalWidth: widget.bounds.width,
    originalHeight: widget.bounds.height,
  };
  window.addEventListener("pointermove", moveResize);
  window.addEventListener("pointerup", endResize);
  window.addEventListener("pointercancel", endResize);
};

const moveResize = (event: PointerEvent) => {
  const state = resizeState.value;
  if (!state) return;
  if (event.pointerId !== state.pointerId) return;
  event.preventDefault();

  const widget = surfaceStore.widgets.find(
    (item) => item.id === state.widgetId,
  );
  if (!widget) return;

  const maxWidth = Math.max(220, window.innerWidth - widget.bounds.x - 12);
  const maxHeight = Math.max(180, window.innerHeight - widget.bounds.y - 72);
  widget.bounds.width = clamp(
    state.originalWidth + event.clientX - state.startX,
    220,
    maxWidth,
  );
  widget.bounds.height = clamp(
    state.originalHeight + event.clientY - state.startY,
    160,
    maxHeight,
  );
};

const endResize = async () => {
  const state = resizeState.value;
  resizeState.value = null;
  window.removeEventListener("pointermove", moveResize);
  window.removeEventListener("pointerup", endResize);
  window.removeEventListener("pointercancel", endResize);
  if (!state) return;

  const widget = surfaceStore.widgets.find((item) => item.id === state.widgetId);
  if (!widget) return;

  await persistWidget(widget);
};

const closeWidget = async (widgetId: string) => {
  await surfaceStore.removeWidget(widgetId);
};

const clearWidgets = async () => {
  await surfaceStore.clearWidgets();
  activeFullscreen.value = null;
};

const refreshWidget = (event: Event) => {
  const iframe = (event.currentTarget as HTMLElement)
    .closest(".surface-widget")
    ?.querySelector("iframe") as HTMLIFrameElement | null;
  if (!iframe) return;
  const srcdoc = iframe.srcdoc;
  iframe.srcdoc = "";
  window.setTimeout(() => {
    iframe.srcdoc = srcdoc;
  }, 30);
};

const postSurfaceResponse = (
  source: MessageEventSource | null,
  requestId: unknown,
  result: unknown,
  error?: unknown,
) => {
  if (!source || typeof requestId !== "number" || !("postMessage" in source)) {
    return;
  }
  source.postMessage(
    {
      type: "vcp-surface-api-response",
      requestId,
      ok: !error,
      result: error ? undefined : result,
      error: error instanceof Error ? error.message : error ? String(error) : undefined,
    },
    { targetOrigin: "*" },
  );
};

const isSurfaceFrameSource = (source: MessageEventSource | null) => {
  if (!source) return false;
  return Array.from(
    document.querySelectorAll<HTMLIFrameElement>(".surface-root iframe"),
  ).some((iframe) => iframe.contentWindow === source);
};

const handleSurfaceApiMessage = async (event: MessageEvent) => {
  const data = event.data || {};
  if (data.type !== "vcp-surface-api-request") return;
  if (!isSurfaceFrameSource(event.source)) return;

  try {
    if (data.method === "weather") {
      const response = await callVcpToolboxApi({ path: "/admin_api/weather" });
      postSurfaceResponse(event.source, data.requestId, response.data);
      return;
    }

    if (data.method === "fetch") {
      const path = String(data.payload?.path || "");
      const response = await callVcpToolboxApi({
        method: data.payload?.options?.method || "GET",
        path,
        body: data.payload?.options?.body,
      });
      postSurfaceResponse(event.source, data.requestId, response.data);
      return;
    }

    if (data.method === "post") {
      const path = String(data.payload?.path || "");
      const response = await callVcpToolboxApi({
        method: "POST",
        path,
        body: data.payload?.body,
      });
      postSurfaceResponse(event.source, data.requestId, response.data);
      return;
    }

    throw new Error(`不支持的 Surface API 方法：${data.method}`);
  } catch (error) {
    postSurfaceResponse(event.source, data.requestId, null, error);
  }
};

onMounted(() => {
  window.addEventListener("message", handleSurfaceApiMessage);
  void surfaceStore.refresh();
});

onBeforeUnmount(() => {
  window.removeEventListener("message", handleSurfaceApiMessage);
  window.removeEventListener("pointermove", moveResize);
  window.removeEventListener("pointerup", endResize);
  window.removeEventListener("pointercancel", endResize);
});
</script>

<template>
  <Teleport to="body">
    <div class="surface-root pointer-events-none fixed inset-0 z-[2500]">
      <button
        v-if="widgets.length"
        class="surface-clear pointer-events-auto fixed right-3 top-[calc(env(safe-area-inset-top)+10px)]"
        :style="{ zIndex: SURFACE_CLEAR_Z_INDEX }"
        @click="clearWidgets"
      >
        <Trash2 :size="15" />
      </button>

      <div
        v-for="(widget, stackIndex) in widgets"
        :key="widget.id"
        class="surface-widget pointer-events-auto fixed overflow-hidden border border-white/12 bg-black/35 shadow-2xl backdrop-blur-xl"
        :style="widgetStyle(widget, stackIndex)"
      >
        <div
          class="surface-titlebar flex h-9 touch-none items-center justify-between border-b border-white/10 bg-black/35 px-2 text-white"
          @pointerdown="beginDrag($event, widget)"
          @pointermove="moveDrag"
          @pointerup="endDrag"
          @pointercancel="endDrag"
        >
          <div class="min-w-0 truncate text-[11px] font-bold uppercase tracking-[0.16em] opacity-70">
            {{ widget.title || "Surface" }}
          </div>
          <div class="flex items-center gap-1" @pointerdown.stop>
            <button class="surface-icon-btn" @click.stop="refreshWidget" @pointerdown.stop>
              <RotateCw :size="14" />
            </button>
            <button class="surface-icon-btn" @click.stop="activeFullscreen = widget" @pointerdown.stop>
              <Maximize2 :size="14" />
            </button>
            <button class="surface-icon-btn" @click.stop="closeWidget(widget.id)" @pointerdown.stop>
              <X :size="14" />
            </button>
          </div>
        </div>
        <iframe
          class="no-swipe h-[calc(100%-36px)] w-full border-0 bg-transparent"
          sandbox="allow-scripts allow-modals allow-forms allow-popups"
          :srcdoc="getSandboxHtml(widget.html)"
        ></iframe>
        <button
          class="surface-resize"
          @pointerdown.stop="beginResize($event, widget)"
        >
          <Grip :size="15" />
        </button>
      </div>

      <Transition name="fade">
        <div
          v-if="activeFullscreen"
          class="pointer-events-auto fixed inset-0 flex flex-col bg-black/90"
          :style="{ zIndex: SURFACE_FULLSCREEN_Z_INDEX }"
        >
          <div class="flex h-[52px] items-center justify-between border-b border-white/10 px-3 pt-[env(safe-area-inset-top)] text-white">
            <div class="truncate text-sm font-bold">{{ activeFullscreen.title || "Surface" }}</div>
            <button class="surface-icon-btn h-9 w-9" @click="activeFullscreen = null">
              <X :size="18" />
            </button>
          </div>
          <iframe
            class="no-swipe min-h-0 flex-1 border-0 bg-transparent"
            sandbox="allow-scripts allow-modals allow-forms allow-popups"
            :srcdoc="getSandboxHtml(activeFullscreen.html)"
          ></iframe>
        </div>
      </Transition>
    </div>
  </Teleport>
</template>

<style scoped>
.surface-widget {
  border-radius: 18px;
  border-color: rgba(255, 255, 255, 0.16);
  background:
    linear-gradient(135deg, rgba(255, 255, 255, 0.08), rgba(255, 255, 255, 0.02)),
    rgba(15, 23, 42, 0.56);
  box-shadow:
    0 24px 64px rgba(0, 0, 0, 0.36),
    inset 0 1px 0 rgba(255, 255, 255, 0.12);
}

.surface-titlebar {
  cursor: grab;
  background:
    linear-gradient(90deg, rgba(255, 255, 255, 0.12), rgba(255, 255, 255, 0.04)),
    rgba(2, 6, 23, 0.48);
  backdrop-filter: blur(16px);
}

.surface-titlebar:active {
  cursor: grabbing;
}

.surface-icon-btn {
  display: inline-flex;
  height: 28px;
  width: 28px;
  align-items: center;
  justify-content: center;
  border-radius: 8px;
  color: rgba(255, 255, 255, 0.78);
  transition: background-color 0.16s ease, transform 0.16s ease;
}

.surface-icon-btn:active {
  transform: scale(0.94);
  background: rgba(255, 255, 255, 0.12);
}

.surface-clear {
  display: inline-flex;
  height: 34px;
  width: 34px;
  align-items: center;
  justify-content: center;
  border-radius: 999px;
  border: 1px solid rgba(255, 255, 255, 0.14);
  background: rgba(0, 0, 0, 0.46);
  color: rgba(255, 255, 255, 0.82);
  box-shadow: 0 12px 32px rgba(0, 0, 0, 0.32);
  backdrop-filter: blur(14px);
}

.surface-clear:active {
  transform: scale(0.94);
}

.surface-resize {
  position: absolute;
  right: 0;
  bottom: 0;
  z-index: 2;
  display: inline-flex;
  height: 34px;
  width: 34px;
  pointer-events: auto;
  touch-action: none;
  align-items: center;
  justify-content: center;
  color: rgba(255, 255, 255, 0.58);
  background: linear-gradient(135deg, transparent 0%, transparent 48%, rgba(255, 255, 255, 0.1) 49%);
  cursor: nwse-resize;
}

.surface-resize:active {
  color: rgba(255, 255, 255, 0.9);
}
</style>
