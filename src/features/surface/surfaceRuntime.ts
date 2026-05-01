import { invoke } from "@tauri-apps/api/core";
import {
  createDesktopPushCommands,
  extractDesktopPushBlocks,
  repairSurfaceHtml,
  stripDesktopPushBlocks,
} from "./surfaceProtocol";
import { useSurfaceStore } from "./surfaceStore";

const consumedOffsets = new Map<string, number>();

const widgetIdForBlock = (messageId: string, startIndex: number) =>
  `desktop-push-${messageId}-${startIndex}`;

const boundsForBlock = (ordinal: number) => {
  const offset = (ordinal % 5) * 18;
  return {
    x: 14 + offset,
    y: 86 + offset,
    width: 340,
    height: 260,
    zIndex: Date.now() + ordinal,
  };
};

export function resetDesktopPushConsumption(messageId: string) {
  consumedOffsets.delete(messageId);
}

export async function consumeDesktopPushBlocks(
  messageId: string,
  content: string,
) {
  if (!messageId || !content) return;

  const blocks = extractDesktopPushBlocks(content);
  if (blocks.length === 0) return;

  const lastConsumed = consumedOffsets.get(messageId) ?? 0;
  const pendingBlocks = blocks.filter((block) => block.endIndex > lastConsumed);
  if (pendingBlocks.length === 0) return;

  const maxConsumed = Math.max(
    lastConsumed,
    ...pendingBlocks.map((block) => block.endIndex),
  );
  consumedOffsets.set(messageId, maxConsumed);

  const surfaceStore = useSurfaceStore();
  let createdCount = surfaceStore.widgets.length;

  for (const block of pendingBlocks) {
    const widgetId = widgetIdForBlock(messageId, block.startIndex);
    const html = repairSurfaceHtml(block.html);
    const commands = createDesktopPushCommands(
      widgetId,
      html,
      boundsForBlock(createdCount),
    );

    for (const command of commands) {
      try {
        await surfaceStore.applyCommand(command);
      } catch (error) {
        console.warn("[Surface] Failed to update in-app surface", error);
        break;
      }
    }

    invoke("show_desktop_overlay", {
      title: "VCP Surface",
      html,
    }).catch((error) => {
      console.warn("[Surface] Failed to show desktop overlay", error);
    });

    createdCount += 1;
  }
}

export { stripDesktopPushBlocks };
