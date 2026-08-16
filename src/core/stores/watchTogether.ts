import { defineStore } from "pinia";
import { computed, ref, shallowRef } from "vue";
import { invoke } from "@tauri-apps/api/core";
import type { Attachment } from "../types/chat";
import type {
  WatchCaptureResult,
} from "../../features/watch/watchCapture";
import type { EmbyConnection } from "../../features/watch/embyClient";

export interface PreparedWatchContext {
  attachment?: Attachment;
  transientContext: string;
  transientSystemPrompt: string;
}

interface WatchContextProvider {
  capture: () => Promise<WatchCaptureResult>;
}

const formatTime = (seconds: number) => {
  if (!Number.isFinite(seconds) || seconds < 0) return "00:00";
  const value = Math.floor(seconds);
  const hours = Math.floor(value / 3600);
  const minutes = Math.floor((value % 3600) / 60);
  const secs = value % 60;
  return hours > 0
    ? [hours, minutes, secs].map((part) => String(part).padStart(2, "0")).join(":")
    : [minutes, secs].map((part) => String(part).padStart(2, "0")).join(":");
};

const createDeviceId = () =>
  `vcp-mobile-${Date.now().toString(36)}-${Math.random().toString(36).slice(2, 10)}`;

export const useWatchTogetherStore = defineStore(
  "watchTogether",
  () => {
    const connection = ref<EmbyConnection | null>(null);
    const deviceId = ref(createDeviceId());
    const active = ref(false);
    const capturePending = ref(false);
    const provider = shallowRef<WatchContextProvider | null>(null);

    const connected = computed(
      () => Boolean(connection.value?.serverUrl && connection.value?.accessToken),
    );

    const setConnection = (value: Omit<EmbyConnection, "deviceId">) => {
      connection.value = { ...value, deviceId: deviceId.value };
    };

    const clearConnection = () => {
      connection.value = null;
    };

    const setContextProvider = (value: WatchContextProvider | null) => {
      provider.value = value;
    };

    const prepareOutgoingContext = async (): Promise<PreparedWatchContext | null> => {
      if (!active.value || !provider.value || capturePending.value) return null;
      capturePending.value = true;
      try {
        const capture = await provider.value.capture();
        let attachment: Attachment | undefined;

        if (capture.blob && capture.fileName && capture.mimeType) {
          const bytes = new Uint8Array(await capture.blob.arrayBuffer());
          const stored = await invoke<any>("store_file", {
            originalName: capture.fileName,
            fileBytes: bytes,
            mimeType: capture.mimeType,
          });
          attachment = {
            id: `watch_${Date.now()}_${Math.random().toString(36).slice(2, 8)}`,
            type: stored.type || capture.mimeType,
            src: stored.internalPath,
            internalPath: stored.internalPath,
            name: stored.name || capture.fileName,
            size: stored.size || capture.blob.size,
            hash: stored.hash,
            status: "done",
            transient: true,
          };
        }

        const title = capture.seriesName
          ? `${capture.seriesName} / ${capture.title}`
          : capture.title;
        const lines = [
          `媒体：${title}`,
          `时间：${formatTime(capture.currentTime)} / ${formatTime(capture.duration)}`,
          `状态：${capture.paused ? "暂停" : "播放中"}`,
          `视觉上下文：${capture.captureKind === "clip" ? "发送前的短视频片段" : capture.captureKind === "frame" ? "当前画面" : "未能捕获媒体，仅有播放元数据"}`,
        ];
        if (capture.clipStartTime !== undefined && capture.clipEndTime !== undefined) {
          lines.push(
            `片段范围：${formatTime(capture.clipStartTime)} - ${formatTime(capture.clipEndTime)}`,
          );
        }
        if (capture.subtitle) lines.push(`当前字幕：${capture.subtitle}`);

        return {
          attachment,
          transientContext: `<watch_context>\n${lines.join("\n")}\n</watch_context>`,
          transientSystemPrompt:
            "你正在与用户共同观看视频。结合本轮提供的播放时间、字幕和临时视觉片段回答；不要声称看过未提供的场景。除非用户明确要求，否则不要剧透当前时间点之后的内容。",
        };
      } catch (error) {
        console.warn("[WatchTogether] Failed to prepare watch context:", error);
        return null;
      } finally {
        capturePending.value = false;
      }
    };

    return {
      connection,
      deviceId,
      active,
      capturePending,
      connected,
      setConnection,
      clearConnection,
      setContextProvider,
      prepareOutgoingContext,
    };
  },
  {
    persist: {
      pick: ["connection", "deviceId"],
    },
  },
);
