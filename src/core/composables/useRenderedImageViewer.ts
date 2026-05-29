import { reactive } from "vue";

export interface RenderedImageViewerPayload {
  src: string;
  alt?: string;
  title?: string;
  fileName?: string;
  sourceLabel?: string;
}

const state = reactive({
  isOpen: false,
  src: "",
  alt: "",
  title: "",
  fileName: "",
  sourceLabel: "",
});

export function openRenderedImageViewer(
  payload: RenderedImageViewerPayload,
): void {
  const src = payload.src?.trim();
  if (!src) return;

  state.src = src;
  state.alt = payload.alt || "";
  state.title = payload.title || "";
  state.fileName = payload.fileName || "";
  state.sourceLabel = payload.sourceLabel || "AI 渲染图片";
  state.isOpen = true;
}

export function closeRenderedImageViewer(): void {
  state.isOpen = false;
}

export function useRenderedImageViewer() {
  return {
    state,
    openRenderedImageViewer,
    closeRenderedImageViewer,
  };
}
