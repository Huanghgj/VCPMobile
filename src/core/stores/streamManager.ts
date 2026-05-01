// streamManager.ts
import { defineStore } from 'pinia';
import { ref } from 'vue';
import { usePerformanceDiagnostics } from '../utils/performanceDiagnostics';
import {
  TOOL_REQUEST_END,
  TOOL_REQUEST_START,
  TOOL_RESULT_END,
  TOOL_RESULT_START,
} from '../utils/toolPreview';

export interface StreamRenderState {
  stable: string;
  tail: string;
  displayed: string;
}

export const useStreamManagerStore = defineStore('streamManager', () => {
  const streamBuffers = new Map<string, {
    fullText: string;
    displayedText: string;
    stableContent: string;
    tailContent: string;
    pendingText: string;
    lastUpdateTime: number;
    isFinishing: boolean;
    onUpdate: (state: StreamRenderState) => void;
    timerId?: number;
    onCompleteCallback?: () => void;
  }>();

  const streamSeeds = new Map<string, string>();

  const STREAM_FLUSH_INTERVAL_MS = 240;
  const FINISH_FLUSH_INTERVAL_MS = 48;
  const MIN_FLUSH_CHARS = 48;
  const MAX_FLUSH_CHARS = 384;

  const activeStreams = ref(new Set<string>());

  const countOpen = (text: string, token: string) => text.split(token).length - 1;

  const canSediment = (text: string) => {
    const isInFence = countOpen(text, '```') % 2 !== 0;
    const isInThink = countOpen(text, '<think') > countOpen(text, '</think');
    const isInVcpThink =
      countOpen(text, '[--- VCP元思考链') > countOpen(text, '[--- 元思考链结束 ---]');
    const isInTool =
      countOpen(text, TOOL_REQUEST_START) > countOpen(text, TOOL_REQUEST_END);
    const isInToolResult =
      countOpen(text, TOOL_RESULT_START) > countOpen(text, TOOL_RESULT_END);
    return !isInFence && !isInThink && !isInVcpThink && !isInTool && !isInToolResult;
  };

  const settleStableContent = (buf: {
    stableContent: string;
    tailContent: string;
    isFinishing: boolean;
  }) => {
    if (buf.isFinishing) return 0;

    const lastBreak = buf.tailContent.lastIndexOf('\n\n');
    if (lastBreak === -1) return 0;

    const stableCandidate = buf.tailContent.slice(0, lastBreak + 2);
    if (!canSediment(buf.stableContent + stableCandidate)) return 0;

    buf.stableContent += stableCandidate;
    buf.tailContent = buf.tailContent.slice(lastBreak + 2);
    return stableCandidate.length;
  };

  const buildRenderState = (buf: {
    stableContent: string;
    tailContent: string;
    displayedText: string;
  }): StreamRenderState => ({
    stable: buf.stableContent,
    tail: buf.tailContent,
    displayed: buf.displayedText,
  });

  const scheduleFlush = (messageId: string, delayMs: number) => {
    const buffer = streamBuffers.get(messageId);
    if (!buffer || buffer.timerId !== undefined) return;

    buffer.timerId = window.setTimeout(() => {
      const latest = streamBuffers.get(messageId);
      if (latest) latest.timerId = undefined;
      flushBuffer(messageId);
    }, delayMs);
  };

  const flushBuffer = (messageId: string) => {
    const buf = streamBuffers.get(messageId);
    if (!buf) {
      activeStreams.value.delete(messageId);
      return;
    }

    const diagnostics = usePerformanceDiagnostics();

    if (buf.pendingText.length > 0) {
      const flushStartedAt = performance.now();
      const backlog = buf.pendingText.length;
      const step = buf.isFinishing
        ? Math.min(backlog, MAX_FLUSH_CHARS * 2)
        : Math.min(
          backlog,
          Math.max(MIN_FLUSH_CHARS, Math.min(MAX_FLUSH_CHARS, Math.ceil(backlog / 2))),
        );
      const nextText = buf.pendingText.slice(0, step);
      buf.pendingText = buf.pendingText.slice(step);
      buf.tailContent += nextText;
      const settledChars = settleStableContent(buf);
      buf.displayedText = buf.stableContent + buf.tailContent;
      buf.lastUpdateTime = performance.now();

      try {
        buf.onUpdate(buildRenderState(buf));
        diagnostics.addTrace('stream-ui-flush', {
          messageId,
          chunkChars: nextText.length,
          stableChars: buf.stableContent.length,
          tailChars: buf.tailContent.length,
          settledChars,
          pendingChars: buf.pendingText.length,
          displayedChars: buf.displayedText.length,
          durationMs: Math.round((performance.now() - flushStartedAt) * 10) / 10,
        });
      } catch (e) {
        console.error('[StreamManager] UI Update failed:', e);
      }
    }

    if (buf.isFinishing && buf.pendingText.length === 0) {
      if (buf.tailContent.length > 0) {
        buf.stableContent += buf.tailContent;
        buf.tailContent = '';
        buf.displayedText = buf.stableContent;
        try {
          buf.onUpdate(buildRenderState(buf));
        } catch (e) {
          console.error('[StreamManager] Final UI Update failed:', e);
        }
      }
      if (buf.onCompleteCallback) {
        buf.onCompleteCallback();
      }
      activeStreams.value.delete(messageId);
      streamBuffers.delete(messageId);
      return;
    }

    if (buf.pendingText.length > 0 || buf.isFinishing) {
      scheduleFlush(messageId, buf.isFinishing ? FINISH_FLUSH_INTERVAL_MS : STREAM_FLUSH_INTERVAL_MS);
    }
  };

  const appendChunk = (messageId: string, chunk: string, onUpdate: (state: StreamRenderState) => void) => {
    if (!chunk) return;

    const diagnostics = usePerformanceDiagnostics();
    diagnostics.addTrace('stream-chunk-buffered', {
      messageId,
      chunkChars: chunk.length,
    });

    if (!streamBuffers.has(messageId)) {
      const seedText = streamSeeds.get(messageId) || '';
      streamSeeds.delete(messageId);
      streamBuffers.set(messageId, {
        fullText: seedText + chunk,
        displayedText: seedText,
        stableContent: seedText,
        tailContent: '',
        pendingText: chunk,
        lastUpdateTime: performance.now(),
        isFinishing: false,
        onUpdate
      });
      activeStreams.value.add(messageId);
      scheduleFlush(messageId, STREAM_FLUSH_INTERVAL_MS);
    } else {
      const buffer = streamBuffers.get(messageId)!;
      buffer.fullText += chunk;
      buffer.onUpdate = onUpdate;
      diagnostics.addTrace('stream-chunk-appended', {
        messageId,
        chunkChars: chunk.length,
        pendingChars: buffer.pendingText.length + chunk.length,
        totalChars: buffer.fullText.length,
      });

      // 如果之前是因为 [DONE] 标记为结束但又有新 chunk 进来，重置状态
      if (buffer.isFinishing) buffer.isFinishing = false;

      buffer.pendingText += chunk;
      scheduleFlush(messageId, STREAM_FLUSH_INTERVAL_MS);
    }
  };

  const seedStream = (messageId: string, text: string) => {
    if (!text) return;
    const existing = streamBuffers.get(messageId);
    if (existing) {
      existing.fullText = text;
      existing.displayedText = text;
      existing.stableContent = text;
      existing.tailContent = '';
      existing.pendingText = '';
      existing.lastUpdateTime = performance.now();
      return;
    }
    streamSeeds.set(messageId, text);
  };

  const finalizeStream = (messageId: string, onComplete?: () => void) => {
    const buffer = streamBuffers.get(messageId);
    const diagnostics = usePerformanceDiagnostics();
    diagnostics.addTrace('stream-finalize-requested', {
      messageId,
      pendingChars: buffer?.pendingText.length || 0,
      totalChars: buffer?.fullText.length || 0,
    });
    if (buffer) {
      // 如果已经标记为结束，不要重复设置回调，但可以更新它
      if (buffer.isFinishing) {
        const oldCallback = buffer.onCompleteCallback;
        buffer.onCompleteCallback = () => {
          if (oldCallback) oldCallback();
          if (onComplete) onComplete();
        };
        return;
      }

      // 标记为结束，loop 会在清空队列后触发回调并自动退出
      buffer.isFinishing = true;
      buffer.onCompleteCallback = onComplete;
      scheduleFlush(messageId, 0);
    } else {
      // 如果 buffer 已经不存在（比如未经过 stream 过程就结束了），直接触发回调
      activeStreams.value.delete(messageId);
      streamSeeds.delete(messageId);
      if (onComplete) onComplete();
    }
  };

  return {
    activeStreams,
    appendChunk,
    seedStream,
    finalizeStream
  };
});
