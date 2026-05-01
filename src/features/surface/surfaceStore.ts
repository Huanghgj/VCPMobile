import { defineStore } from "pinia";
import { computed, ref } from "vue";
import {
  surfaceBridge,
  type SurfaceCommand,
  type SurfaceWidget,
  type UpsertSurfaceWidgetRequest,
} from "../../core/api/surfaceBridge";

export const useSurfaceStore = defineStore("surface", () => {
  const widgets = ref<SurfaceWidget[]>([]);
  const loading = ref(false);
  const lastError = ref<string | null>(null);

  const orderedWidgets = computed(() =>
    [...widgets.value].sort((a, b) => a.bounds.zIndex - b.bounds.zIndex),
  );

  const setError = (error: unknown) => {
    lastError.value = error instanceof Error ? error.message : String(error);
  };

  const refresh = async () => {
    loading.value = true;
    lastError.value = null;
    try {
      widgets.value = await surfaceBridge.listWidgets();
    } catch (error) {
      setError(error);
    } finally {
      loading.value = false;
    }
  };

  const upsertWidget = async (request: UpsertSurfaceWidgetRequest) => {
    lastError.value = null;
    try {
      const widget = await surfaceBridge.upsertWidget(request);
      const index = widgets.value.findIndex((item) => item.id === widget.id);
      if (index === -1) {
        widgets.value.push(widget);
      } else {
        widgets.value[index] = widget;
      }
      return widget;
    } catch (error) {
      setError(error);
      throw error;
    }
  };

  const removeWidget = async (widgetId: string) => {
    lastError.value = null;
    try {
      const removed = await surfaceBridge.removeWidget(widgetId);
      if (removed) {
        widgets.value = widgets.value.filter(
          (widget) => widget.id !== widgetId,
        );
      }
      return removed;
    } catch (error) {
      setError(error);
      throw error;
    }
  };

  const clearWidgets = async () => {
    lastError.value = null;
    try {
      await surfaceBridge.clearWidgets();
      widgets.value = [];
    } catch (error) {
      setError(error);
      throw error;
    }
  };

  const replaceLatestWidgetContent = async (
    targetSelector: string | undefined,
    content: string,
  ) => {
    const widget = [...widgets.value].sort((a, b) => b.updatedAt - a.updatedAt)[0];
    if (!widget) return widgets.value;

    let nextHtml = content;
    if (targetSelector?.trim()) {
      try {
        const parser = new DOMParser();
        const document = parser.parseFromString(widget.html, "text/html");
        const target = document.querySelector(targetSelector.trim());
        if (target) {
          target.outerHTML = content;
          nextHtml = document.body.innerHTML || content;
        }
      } catch (error) {
        console.warn("[SurfaceStore] Selector replacement failed:", error);
      }
    }

    await upsertWidget({
      id: widget.id,
      title: widget.title,
      html: nextHtml,
      bounds: widget.bounds,
      favorite: widget.favorite,
      source: widget.source,
    });

    return widgets.value;
  };

  const applyCommand = async (command: SurfaceCommand) => {
    lastError.value = null;
    try {
      if (command.action === "replace") {
        return await replaceLatestWidgetContent(
          command.targetSelector,
          command.content,
        );
      }
      widgets.value = await surfaceBridge.applyCommand(command);
      return widgets.value;
    } catch (error) {
      setError(error);
      throw error;
    }
  };

  return {
    widgets,
    orderedWidgets,
    loading,
    lastError,
    refresh,
    upsertWidget,
    removeWidget,
    clearWidgets,
    applyCommand,
  };
});
