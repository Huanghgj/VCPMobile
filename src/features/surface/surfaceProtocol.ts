import type {
  SurfaceCommand,
  SurfaceWidgetBounds,
} from "../../core/api/surfaceBridge";

const DESKTOP_PUSH_START = "<<<[DESKTOP_PUSH]>>>";
const DESKTOP_PUSH_END = "<<<[DESKTOP_PUSH_END]>>>";

export interface DesktopPushBlock {
  html: string;
  startIndex: number;
  endIndex: number;
}

export function extractDesktopPushBlocks(content: string): DesktopPushBlock[] {
  const blocks: DesktopPushBlock[] = [];
  let cursor = 0;

  while (cursor < content.length) {
    const start = content.indexOf(DESKTOP_PUSH_START, cursor);
    if (start === -1) break;

    const bodyStart = start + DESKTOP_PUSH_START.length;
    const end = content.indexOf(DESKTOP_PUSH_END, bodyStart);
    if (end === -1) break;

    blocks.push({
      html: content.slice(bodyStart, end).trim(),
      startIndex: start,
      endIndex: end + DESKTOP_PUSH_END.length,
    });
    cursor = end + DESKTOP_PUSH_END.length;
  }

  return blocks;
}

export function createDesktopPushCommands(
  widgetId: string,
  html: string,
  options?: Partial<SurfaceWidgetBounds>,
): SurfaceCommand[] {
  const bounds: SurfaceWidgetBounds = {
    x: options?.x ?? 16,
    y: options?.y ?? 96,
    width: options?.width ?? 320,
    height: options?.height ?? 220,
    zIndex: options?.zIndex ?? 1,
  };

  return [
    { action: "create", widgetId, options: bounds },
    { action: "append", widgetId, content: html },
    { action: "finalize", widgetId },
  ];
}
