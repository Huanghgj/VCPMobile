import { defineStore } from "pinia";
import { ref, computed, reactive, onScopeDispose } from "vue";
import { invoke } from "@tauri-apps/api/core";
import {
  acquireScreenKeep,
  releaseScreenKeep,
} from "../composables/useScreenKeeper";
import { useChatSessionStore } from "./chatSessionStore";
import { useAssistantStore } from "./assistant";
import { useAvatarStore } from "./avatar";
import { useTopicStore } from "./topicListManager";
import { clearMessageCache } from "../utils/astRenderer";
import type { ChatMessage, MessageShell, TailFrame } from "../types/chat";

export const useChatStreamStore = defineStore("chatStream", () => {
  const streamingMessageId = ref<string | null>(null);

  // 核心：记录每个会话（itemId + topicId）是否处于活动流状态
  // 格式: "itemId:topicId" -> [messageId1, messageId2, ...]
  const sessionActiveStreams = ref<Record<string, string[]>>({});

  // 全局活跃流消息池：存储所有正在生成的响应对象 (messageId -> Reactive<ChatMessage>)
  // 无论是在前台还是后台，流式消息都从此池中获取，保证响应式链路不断裂
  const activeStreamMessages = reactive<Map<string, ChatMessage>>(new Map());
  const activeStreamRefCounts = reactive<Record<string, number>>({});
  const activeStreamContexts = reactive<
    Record<
      string,
      {
        itemId: string;
        topicId: string;
      }
    >
  >({});
  const activeStreamTotal = ref(0);
  const auroraActiveMessageIds = new Set<string>();
  const sealedStreamMessageIds = new Set<string>();
  const terminalStreamMessageIds = new Set<string>();
  const streamBlockSignatures = new Map<string, string>();
  const streamTailSignatures = new Map<string, string>();

  function isStreamDebugEnabled(): boolean {
    return Boolean(import.meta.env.DEV && (window as any).__VCP_STREAM_DEBUG__);
  }

  function recordStreamTrace(data: any): void {
    if (!isStreamDebugEnabled()) return;
    if (!(window as any).__VCP_STREAM_TRACES__) {
      (window as any).__VCP_STREAM_TRACES__ = [];
    }
    (window as any).__VCP_STREAM_TRACES__.push({
      timestamp: performance.now(),
      ...data,
    });
  }

  function streamDebugLog(...args: unknown[]): void {
    if (isStreamDebugEnabled()) {
      console.warn(...args);
    }
  }

  function mergeTailFrame(
    existing: TailFrame | null,
    incoming: TailFrame
  ): TailFrame {
    const incomingMutations = incoming.mutations || [];
    if (!existing || incoming.reset || incoming.epoch !== existing.epoch) {
      return {
        ...incoming,
        mutations: incoming.reset ? [] : [...incomingMutations],
        snapshot: incoming.snapshot ? [...incoming.snapshot] : undefined,
      };
    }

    return {
      ...incoming,
      reset: existing.reset || incoming.reset,
      snapshot: incoming.snapshot || existing.snapshot,
      mutations: [
        ...(existing.reset ? [] : existing.mutations || []),
        ...incomingMutations,
      ],
    };
  }

  const cleanupTimers = new Set<ReturnType<typeof setTimeout>>();

  // ===== 低功耗流式帧合并暂存池 =====
  // 流式事件可能高于人眼可读速度；用 timer + 单次 rAF 合并提交，避免每个物理刷新帧空转轮询。
  type PendingStreamUpdate = {
    content: string | null;
    blocks: any[] | null;
    tailContent: string | null;
    tailBlock: any | null | undefined;
    tailFrame: TailFrame | null;
    tailSnapshot: any[] | null;
    animationFrameId: number | null;
    renderTimerId: ReturnType<typeof setTimeout> | null;
    fallbackTimerId: ReturnType<typeof setTimeout> | null;
    lastRenderTime: number;
  };

  const rAFPendingUpdates = new Map<string, PendingStreamUpdate>();
  const STREAM_RENDER_INTERVAL_MS = 66.7; // 约 15Hz：移动端流式可读且显著降低 DOM/AST diff 唤醒频率
  const STREAM_RENDER_MAX_LATENCY_MS = 180; // WebView 忙碌时 rAF 可能延迟，超过该时间强制提交 pending 帧

  const isDocumentHidden = () =>
    typeof document !== "undefined" && document.hidden;

  const hasPendingRAFData = (up: {
    content: string | null;
    blocks: any[] | null;
    tailContent: string | null;
    tailBlock: any | null | undefined;
    tailFrame: TailFrame | null;
    tailSnapshot: any[] | null;
  }) =>
    up.content !== null ||
    up.blocks !== null ||
    up.tailContent !== null ||
    up.tailBlock !== undefined ||
    up.tailFrame !== null ||
    up.tailSnapshot !== null;

  const cancelScheduledCommit = (up: {
    animationFrameId: number | null;
    renderTimerId: ReturnType<typeof setTimeout> | null;
    fallbackTimerId?: ReturnType<typeof setTimeout> | null;
  }) => {
    if (up.animationFrameId !== null) {
      cancelAnimationFrame(up.animationFrameId);
      up.animationFrameId = null;
    }
    if (up.renderTimerId !== null) {
      clearTimeout(up.renderTimerId);
      up.renderTimerId = null;
    }
    if (up.fallbackTimerId !== undefined && up.fallbackTimerId !== null) {
      clearTimeout(up.fallbackTimerId);
      up.fallbackTimerId = null;
    }
  };

  const pauseRAFCommitsForBackground = () => {
    rAFPendingUpdates.forEach((up) => {
      cancelScheduledCommit(up);
    });
  };

  /**
   * 物理防线：强行中止、强制同步刷新并安全清理指定消息的 rAF 帧状态，杜绝任何泄漏与闪烁
   */
  const clearRAFUpdate = (messageId: string, forceFlush = false) => {
    const up = rAFPendingUpdates.get(messageId);
    if (up) {
      cancelScheduledCommit(up);
      if (forceFlush) {
        const msg = activeStreamMessages.get(messageId);
        if (msg) {
          if (up.content !== null) msg.content = up.content;
          if (up.blocks !== null) msg.blocks = up.blocks;
          // 漏洞 1 修复：同步强刷收尾时，必须将暂存池中的 tail 字段强刷，绝不允许丢字闪烁
          if (up.tailContent !== null) msg.tailContent = up.tailContent;
          if (up.tailBlock !== undefined) msg.tailBlock = up.tailBlock;
          if (up.tailSnapshot !== null)
            msg.tailSnapshot = up.tailSnapshot as any;
          if (up.tailFrame !== null) msg.tailFrame = up.tailFrame;
        }
      }
      rAFPendingUpdates.delete(messageId);
    }
  };

  const sessionStore = useChatSessionStore();
  const assistantStore = useAssistantStore();
  const avatarStore = useAvatarStore();
  const topicStore = useTopicStore();

  /**
   * 在前端本地计算 MessageShell（替代 Rust 的 precompute_shell）
   */
  function computeShell(msg: {
    role: string;
    agentId?: string;
    name?: string;
  }): MessageShell {
    const empty = "";
    if (msg.role === "user") {
      const userColor =
        avatarStore.getDominantColor("user", "user_avatar") || "rgb(226,54,56)";
      return {
        avatarColor: userColor,
        bubbleBorderColor: empty,
        bubbleBoxShadow: empty,
        displayName: msg.name || "User",
        isUser: true,
      };
    }
    const agent = msg.agentId
      ? assistantStore.agents.find((a) => a.id === msg.agentId)
      : undefined;
    return {
      avatarColor: agent?.avatarCalculatedColor || "",
      bubbleBorderColor: empty,
      bubbleBoxShadow: empty,
      displayName: msg.name || agent?.name || "AI",
      isUser: false,
    };
  }

  // 兼容旧逻辑的计算属性
  const activeStreamingIds = computed(() => {
    if (!sessionStore.currentSelectedItem?.id || !sessionStore.currentTopicId)
      return new Set<string>();
    const key = `${sessionStore.currentSelectedItem.id}:${sessionStore.currentTopicId}`;
    return new Set(sessionActiveStreams.value[key] || []);
  });

  const allActiveStreamingIds = computed(() => {
    return new Set(Object.keys(activeStreamRefCounts));
  });

  const isMessageInActiveStream = (messageId: string) =>
    (activeStreamRefCounts[messageId] || 0) > 0;

  const isMessageStreamingInSession = (
    messageId: string,
    itemId?: string | null,
    topicId?: string | null
  ) => {
    if (!itemId || !topicId || !isMessageInActiveStream(messageId))
      return false;
    const context = activeStreamContexts[messageId];
    return (
      !!context && context.itemId === itemId && context.topicId === topicId
    );
  };

  const isGroupGenerating = computed(() => {
    if (
      !sessionStore.currentSelectedItem?.id ||
      !sessionStore.currentTopicId ||
      sessionStore.currentSelectedItem.type !== "group"
    )
      return false;
    const key = `${sessionStore.currentSelectedItem.id}:${sessionStore.currentTopicId}`;
    const streams = sessionActiveStreams.value[key];
    return streams ? streams.length > 0 : false;
  });

  // 全局流消息池上限，防止极端场景下 OOM
  const MAX_STREAM_MESSAGES = 100;
  const TERMINAL_TOMBSTONE_TTL_MS = 5 * 60 * 1000;

  const cleanupInactiveStreamMessage = (messageId: string) => {
    if (
      isMessageInActiveStream(messageId) ||
      sealedStreamMessageIds.has(messageId)
    )
      return;
    activeStreamMessages.delete(messageId);
    delete activeStreamContexts[messageId];
    auroraActiveMessageIds.delete(messageId);
    streamBlockSignatures.delete(messageId);
    streamTailSignatures.delete(messageId);
    clearRAFUpdate(messageId, false);
  };

  const markStreamTerminal = (messageId: string) => {
    if (terminalStreamMessageIds.has(messageId)) return;
    sealedStreamMessageIds.delete(messageId);
    terminalStreamMessageIds.add(messageId);
    const cleanupTimer = setTimeout(() => {
      cleanupTimers.delete(cleanupTimer);
      terminalStreamMessageIds.delete(messageId);
    }, TERMINAL_TOMBSTONE_TTL_MS);
    cleanupTimers.add(cleanupTimer);
  };

  const sealStreamInputs = (messageId: string) => {
    if (sealedStreamMessageIds.has(messageId)) return;
    sealedStreamMessageIds.add(messageId);
    const cleanupTimer = setTimeout(() => {
      cleanupTimers.delete(cleanupTimer);
      sealedStreamMessageIds.delete(messageId);
      cleanupInactiveStreamMessage(messageId);
    }, TERMINAL_TOMBSTONE_TTL_MS);
    cleanupTimers.add(cleanupTimer);
  };

  const enforceStreamPoolLimit = () => {
    if (activeStreamMessages.size <= MAX_STREAM_MESSAGES) return;
    let excess = activeStreamMessages.size - MAX_STREAM_MESSAGES;
    // 按插入顺序（Map 保持插入顺序）清理最旧的非活跃消息
    for (const [id] of activeStreamMessages) {
      if (excess <= 0) break;
      // 只删除已完成的流（不在当前活跃会话中）
      if (!isMessageInActiveStream(id)) {
        cleanupInactiveStreamMessage(id);
        excess--;
      }
    }
  };

  // 辅助方法：管理会话流状态
  const addSessionStream = (
    ownerId: string,
    topicId: string,
    messageId: string
  ) => {
    const key = `${ownerId}:${topicId}`;
    const streams = sessionActiveStreams.value[key] ?? [];
    if (!sessionActiveStreams.value[key]) {
      sessionActiveStreams.value[key] = streams;
    }

    if (streams.includes(messageId)) {
      // 同一会话同一消息会收到 thinking/data/aurora 多类事件；只允许首次登记增加引用计数。
      enforceStreamPoolLimit();
      return;
    }

    streams.push(messageId);
    activeStreamRefCounts[messageId] =
      (activeStreamRefCounts[messageId] || 0) + 1;
    activeStreamContexts[messageId] = {
      itemId: ownerId,
      topicId,
    };
    if (activeStreamTotal.value === 0) {
      acquireScreenKeep();
    }
    activeStreamTotal.value++;

    // 新增流时检查并执行上限保护
    enforceStreamPoolLimit();
  };

  const removeSessionStream = (
    ownerId: string,
    topicId: string,
    messageId: string
  ) => {
    const key = `${ownerId}:${topicId}`;
    const streams = sessionActiveStreams.value[key];
    let didRemove = false;
    if (streams) {
      const index = streams.indexOf(messageId);
      if (index !== -1) {
        streams.splice(index, 1);
        didRemove = true;
        const nextRefCount = (activeStreamRefCounts[messageId] || 0) - 1;
        if (nextRefCount > 0) {
          activeStreamRefCounts[messageId] = nextRefCount;
        } else {
          delete activeStreamRefCounts[messageId];
          delete activeStreamContexts[messageId];
        }
        activeStreamTotal.value = Math.max(0, activeStreamTotal.value - 1);
      }
      if (streams.length === 0) {
        delete sessionActiveStreams.value[key];
      }
    }
    if (didRemove && activeStreamTotal.value === 0) {
      releaseScreenKeep();
    }
    // 同时从全局池中移除 (延迟移除，确保 finalizeStream 能拿到对象)
    const cleanupTimer = setTimeout(() => {
      cleanupTimers.delete(cleanupTimer);
      cleanupInactiveStreamMessage(messageId);
    }, 1000);
    cleanupTimers.add(cleanupTimer);
  };

  const blocksSignature = (
    blocks: Array<{ hash?: string; type?: string }> = []
  ) => blocks.map((block) => block.hash || block.type || "").join("|");

  const tailSignature = (
    tailBlock: { hash?: string; type?: string } | undefined,
    tail?: string
  ) => tailBlock?.hash || `${tailBlock?.type || ""}:${tail || ""}`;

  const getRAFUpdate = (messageId: string) => {
    let update = rAFPendingUpdates.get(messageId);
    if (!update) {
      update = {
        content: null,
        blocks: null,
        tailContent: null,
        tailBlock: undefined,
        tailFrame: null,
        tailSnapshot: null,
        animationFrameId: null,
        renderTimerId: null,
        fallbackTimerId: null,
        lastRenderTime: 0,
      };
      rAFPendingUpdates.set(messageId, update);
    }

    return update;
  };

  const commitPendingUpdate = (messageId: string) => {
    const up = rAFPendingUpdates.get(messageId);
    if (!up || isDocumentHidden()) return;

    if (up.fallbackTimerId !== null) {
      clearTimeout(up.fallbackTimerId);
      up.fallbackTimerId = null;
    }

    // 满足低功耗节流间隔后，触发 Vue 响应式写入进行重绘
    const m = activeStreamMessages.get(messageId);
    if (m) {
      if (up.content !== null) m.content = up.content;
      if (up.blocks !== null) m.blocks = up.blocks;
      if (up.tailContent !== null) m.tailContent = up.tailContent;
      if (up.tailBlock !== undefined) m.tailBlock = up.tailBlock;
      if (up.tailSnapshot !== null) m.tailSnapshot = up.tailSnapshot as any;
      if (up.tailFrame !== null) m.tailFrame = up.tailFrame;
    }
    up.lastRenderTime = performance.now();
    // 重置当前提交周期内的合并暂存状态
    up.content = null;
    up.blocks = null;
    up.tailContent = null;
    up.tailBlock = undefined;
    up.tailFrame = null;
    up.tailSnapshot = null;
  };

  const scheduleRAFCommit = (messageId: string) => {
    const update = rAFPendingUpdates.get(messageId);
    if (
      !update ||
      update.animationFrameId !== null ||
      update.renderTimerId !== null ||
      isDocumentHidden()
    ) {
      return;
    }

    if (update.fallbackTimerId === null) {
      update.fallbackTimerId = setTimeout(() => {
        const up = rAFPendingUpdates.get(messageId);
        if (!up) return;
        up.fallbackTimerId = null;
        if (isDocumentHidden() || !hasPendingRAFData(up)) return;
        if (up.animationFrameId !== null) {
          cancelAnimationFrame(up.animationFrameId);
          up.animationFrameId = null;
        }
        if (up.renderTimerId !== null) {
          clearTimeout(up.renderTimerId);
          up.renderTimerId = null;
        }
        commitPendingUpdate(messageId);
      }, STREAM_RENDER_MAX_LATENCY_MS);
    }

    const requestCommitFrame = () => {
      const up = rAFPendingUpdates.get(messageId);
      if (!up || isDocumentHidden()) return;

      up.animationFrameId = requestAnimationFrame(() => {
        const current = rAFPendingUpdates.get(messageId);
        if (!current) return;
        current.animationFrameId = null;
        commitPendingUpdate(messageId);
      });
    };

    const elapsed = performance.now() - update.lastRenderTime;
    const delay = Math.max(0, STREAM_RENDER_INTERVAL_MS - elapsed);
    if (delay <= 1) {
      requestCommitFrame();
      return;
    }

    update.renderTimerId = setTimeout(() => {
      const up = rAFPendingUpdates.get(messageId);
      if (!up) return;
      up.renderTimerId = null;
      requestCommitFrame();
    }, delay);
  };

  const resumeRAFCommitsForForeground = () => {
    rAFPendingUpdates.forEach((up, messageId) => {
      if (up.animationFrameId === null && hasPendingRAFData(up)) {
        scheduleRAFCommit(messageId);
      }
    });
  };

  const handleVisibilityChange = () => {
    if (isDocumentHidden()) {
      pauseRAFCommitsForBackground();
    } else {
      resumeRAFCommitsForForeground();
    }
  };

  // 回前台时同步强刷所有挂起的流式合并帧。
  // Android 上真正的前后台信号是 Kotlin 派发的 `vcp-lifecycle` 事件（document.hidden 不一定可靠），
  // 若仅依赖 rAF 重排，回前台后可能因丢帧导致最后一段流式内容迟迟不上屏（“流式渲染刷新异常”）。
  // 这里在前台恢复时直接把暂存数据写入响应式消息，保证立即上屏。
  const flushAllPendingCommitsForResume = () => {
    rAFPendingUpdates.forEach((up, messageId) => {
      cancelScheduledCommit(up);
      const m = activeStreamMessages.get(messageId);
      if (m) {
        if (up.content !== null) m.content = up.content;
        if (up.blocks !== null) m.blocks = up.blocks;
        if (up.tailContent !== null) m.tailContent = up.tailContent;
        if (up.tailBlock !== undefined) m.tailBlock = up.tailBlock;
        if (up.tailSnapshot !== null) m.tailSnapshot = up.tailSnapshot as any;
        if (up.tailFrame !== null) m.tailFrame = up.tailFrame;
      }
      up.content = null;
      up.blocks = null;
      up.tailContent = null;
      up.tailBlock = undefined;
      up.tailFrame = null;
      up.tailSnapshot = null;
      up.lastRenderTime = performance.now();
    });
  };

  const handleVcpLifecycle = (e: Event) => {
    const state = (e as CustomEvent).detail?.state;
    if (state === "resume") {
      flushAllPendingCommitsForResume();
      resumeRAFCommitsForForeground();
    } else if (state === "stop" || state === "pause") {
      pauseRAFCommitsForBackground();
    }
  };

  if (typeof document !== "undefined") {
    document.addEventListener("visibilitychange", handleVisibilityChange);
  }
  if (typeof window !== "undefined") {
    window.addEventListener("vcp-lifecycle", handleVcpLifecycle);
  }

  /**
   * 处理流式事件的核心逻辑 (会话隔离调度器)
   */
  const processStreamEvent = async (
    event: any,
    callbacks?: {
      onMessageCreated?: (msg: ChatMessage, topicId: string) => void;
      onStreamFinished?: (messageId: string, topicId: string) => void;
    }
  ) => {
    const actualMessageId = event.messageId || event.message_id || "";
    const { chunk, type, context } = event;
    const ctx = context || {};
    const topicId = ctx.topicId;
    const isGroup = !!ctx.isGroupMessage || !!ctx.groupId;
    const itemId = isGroup ? ctx.groupId : ctx.agentId || ctx.ownerId;

    if (!actualMessageId || !topicId || !itemId) return;

    const isTerminalEvent = type === "end" || type === "error";
    if (!isTerminalEvent && sealedStreamMessageIds.has(actualMessageId)) {
      return;
    }
    if (terminalStreamMessageIds.has(actualMessageId)) {
      if (isTerminalEvent) {
        clearRAFUpdate(actualMessageId, true);
        removeSessionStream(itemId, topicId, actualMessageId);
      }
      return;
    }

    let msg = activeStreamMessages.get(actualMessageId);
    if (isTerminalEvent && !msg) return;
    const isNewStream = !msg;

    if (isNewStream) {
      auroraActiveMessageIds.delete(actualMessageId);
      streamBlockSignatures.delete(actualMessageId);
      streamTailSignatures.delete(actualMessageId);
      clearMessageCache(actualMessageId);
      msg = reactive<ChatMessage>({
        id: actualMessageId,
        role: "assistant",
        name: ctx.agentName,
        content: "",
        timestamp: Date.now(),
        isThinking: type === "thinking",
        agentId: ctx.agentId,
        topicId,
        groupId: ctx.groupId,
        isGroupMessage: !!ctx.isGroupMessage,
        shell: computeShell({
          role: "assistant",
          agentId: ctx.agentId,
          name: ctx.agentName,
        }),
      });
      activeStreamMessages.set(actualMessageId, msg!);

      topicStore.incrementTopicMsgCount(topicId);
      if (topicId !== sessionStore.currentTopicId) {
        topicStore.incrementTopicUnreadCount(topicId);
      }

      // 回调：通知 UI 列表插入新消息
      if (callbacks?.onMessageCreated) {
        callbacks.onMessageCreated(msg!, topicId);
      }

      // [关键修复] 异步轻量持久化骨架消息到 SQLite 数据库
      // 使得用户即便中途切换会话，重新加载历史时也存在此消息占位，从而触发 Object Hydration 完美接续流式动画
      invoke("append_stream_skeleton_message", {
        topicId,
        message: {
          id: actualMessageId,
          role: "assistant",
          name: ctx.agentName || null,
          content: "",
          timestamp: msg!.timestamp,
          isThinking: msg!.isThinking,
          is_thinking: msg!.isThinking,
          agentId: ctx.agentId || null,
          groupId: ctx.groupId || null,
          topicId,
          isGroupMessage: isGroup,
        },
      }).catch((e) => {
        console.error(
          "[ChatStreamStore] Failed to persist initial thinking skeleton:",
          e
        );
      });
    }

    // 维护流状态
    if (type === "thinking") {
      msg!.isThinking = true;
      addSessionStream(itemId, topicId, actualMessageId);
      if (!streamingMessageId.value) {
        streamingMessageId.value = actualMessageId;
      }
    } else if (type === "data") {
      msg!.isThinking = false;
      addSessionStream(itemId, topicId, actualMessageId);

      // Aurora 是当前协议的真实流式渲染通道。原始 data 事件只保留给旧协议兜底，
      // 避免在 Aurora 已接管后把同一段 AI 回复按原文再输出一遍。
      if (auroraActiveMessageIds.has(actualMessageId)) return;

      let textChunk = "";
      if (typeof chunk === "string") {
        textChunk = chunk;
      } else if (chunk && chunk.choices && chunk.choices.length > 0) {
        const delta = chunk.choices[0].delta;
        if (delta && delta.content) textChunk = delta.content;
      }

      if (textChunk) {
        const update = getRAFUpdate(actualMessageId);
        update.content = (update.content ?? msg!.content ?? "") + textChunk;
        scheduleRAFCommit(actualMessageId);
      }
    } else if (type === "aurora") {
      const aurora = event.aurora;
      if (aurora) {
        auroraActiveMessageIds.add(actualMessageId);
        recordStreamTrace({
          messageId: actualMessageId,
          auroraPayload: {
            stableChanged: aurora.stableChanged,
            stableBlocksCount: aurora.stableBlocks?.length || 0,
            stableBlocksHashes:
              aurora.stableBlocks?.map((b: any) => b.hash) || [],
            tailChanged: aurora.tailChanged,
            tailContent: aurora.tail || "",
            tailBlockType: aurora.tailBlock?.type || null,
            contentDeltaLength: aurora.contentDelta?.length || 0,
            tailFrame: aurora.tailFrame
              ? {
                  epoch: aurora.tailFrame.epoch,
                  revision: aurora.tailFrame.revision,
                  frameSeq: aurora.tailFrame.frameSeq,
                  reset: aurora.tailFrame.reset,
                  mutationsCount: aurora.tailFrame.mutations?.length || 0,
                  hasSnapshot: !!aurora.tailFrame.snapshot,
                }
              : null,
          },
          msgSnapshot: msg
            ? {
                contentLength: msg.content?.length || 0,
                blocksCount: msg.blocks?.length || 0,
                tailContentLength: msg.tailContent?.length || 0,
              }
            : null,
        });

        // 1. 初始化或获取该 messageId 的帧合并状态
        const update = getRAFUpdate(actualMessageId);

        // 2. 覆盖写入暂存数据（稀疏合并）
        if (typeof aurora.content === "string") {
          update.content = aurora.content;
        } else if (typeof aurora.contentDelta === "string") {
          update.content =
            (update.content ?? msg!.content ?? "") + aurora.contentDelta;
        }
        if (aurora.stableChanged && aurora.stableBlocks) {
          const nextSignature = blocksSignature(aurora.stableBlocks);
          if (streamBlockSignatures.get(actualMessageId) !== nextSignature) {
            update.blocks = aurora.stableBlocks;
            streamBlockSignatures.set(actualMessageId, nextSignature);
          }
        }
        if (aurora.tailFrame) {
          streamDebugLog(
            `[chatStreamStore] Received tailFrame seq=${
              aurora.tailFrame.frameSeq
            } mutations=${
              aurora.tailFrame.mutations?.length || 0
            } for ${actualMessageId}`
          );
          update.tailFrame = mergeTailFrame(update.tailFrame, aurora.tailFrame);
          if (aurora.tailFrame.snapshot) {
            update.tailSnapshot = aurora.tailFrame.snapshot as any[];
          }
        }
        if (aurora.tailSnapshot) {
          update.tailSnapshot = aurora.tailSnapshot as any[];
        }
        if (aurora.tailChanged) {
          const nextTail = aurora.tail || "";
          const nextTailBlock = (aurora.tailBlock as any) || null;
          const nextTailSignature = tailSignature(
            nextTailBlock || undefined,
            nextTail
          );
          if (streamTailSignatures.get(actualMessageId) !== nextTailSignature) {
            update.tailContent = nextTail;
            update.tailBlock = nextTailBlock;
            streamTailSignatures.set(actualMessageId, nextTailSignature);
          }
        }

        // 3. 申请低功耗合并渲染（约 15Hz，上屏前稀疏合并 Aurora 高频帧）
        scheduleRAFCommit(actualMessageId);
      }

      msg!.isThinking = false;
      addSessionStream(itemId, topicId, actualMessageId);
    } else if (type === "end" || type === "error") {
      markStreamTerminal(actualMessageId);

      const errorMsg = event.error;
      const finishReason = event.finishReason;
      const hadError = msg!.finishReason === "error";

      // 漏洞 1 & 2 & 3 修复：同步强制秒结，防止 tailContent 闪烁回滚丢失
      clearRAFUpdate(actualMessageId, true);
      auroraActiveMessageIds.delete(actualMessageId);
      streamBlockSignatures.delete(actualMessageId);
      streamTailSignatures.delete(actualMessageId);
      if (finishReason) msg!.finishReason = finishReason;

      if (streamingMessageId.value === actualMessageId)
        streamingMessageId.value = null;

      if (
        type === "error" &&
        !hadError &&
        errorMsg &&
        errorMsg !== "请求已中止"
      ) {
        const errorText = `\n\n> VCP流式错误: ${errorMsg}`;
        msg!.content += errorText;
        msg!.finishReason = "error";
      }

      if (msg) {
        msg!.isThinking = false;
        if (typeof event.content === "string") {
          msg!.content = event.content;
        }
        if (event.timestamp) {
          msg!.timestamp = event.timestamp;
        }
        clearMessageCache(actualMessageId);
        try {
          // 如果后端已经带回了预渲染好的 blocks，直接使用，跳过冗余解析
          if (event.blocks) {
            msg.blocks = event.blocks as any;
          } else {
            const compiledBlocks = await invoke("process_message_content", {
              content: msg!.content || "",
            });
            msg.blocks = compiledBlocks as any;
          }
        } catch (e) {
          console.error("[ChatStreamStore] process_message_content failed:", e);
        } finally {
          // 漏洞 1 终极解决：在最终编译树成功上屏后，才同步清空临时 tail，实现绝对零闪烁和无缝平滑交接
          msg!.tailContent = "";
          msg!.tailBlock = undefined;
          msg!.tailFrame = undefined;
          msg!.tailSnapshot = undefined;

          // === 🚀 输出流式诊断提示与回放指南（开发模式生效，Release 构建时自动摇树切除） ===
          if (import.meta.env.DEV) {
            console.log(
              `%c[VCP Stream Debugger] 🎉 流式传输结束！当前录制帧数: ${
                (window as any).__VCP_STREAM_TRACES__?.length || 0
              }`,
              "color: #10b981; font-weight: bold; font-size: 13px;"
            );
            console.log(
              `%c👉 运行指令 A 可一键获取帧轨迹总览:\n   console.table(window.__VCP_STREAM_TRACES__.map((t, idx) => ({ '帧号': idx, '相对时间(ms)': Math.round(t.timestamp - window.__VCP_STREAM_TRACES__[0].timestamp), 'Stable变动': t.auroraPayload.stableChanged, 'Stable块数': t.auroraPayload.stableBlocksCount, 'Tail变动': t.auroraPayload.tailChanged, 'Tail内容': t.auroraPayload.tailContent.substring(0, 15) })))`,
              "color: #3b82f6;"
            );
            console.log(
              `%c👉 运行指令 B 进行任意相邻帧 Diff 比对（如 12 帧与 13 帧）:\n   const fA = window.__VCP_STREAM_TRACES__[12]; const fB = window.__VCP_STREAM_TRACES__[13]; console.log("=== 帧12 Tail ===", fA.auroraPayload.tailContent); console.log("=== 帧13 Stable Hashes ===", fB.auroraPayload.stableBlocksHashes); console.log("=== 帧13 Tail ===", fB.auroraPayload.tailContent);`,
              "color: #8b5cf6;"
            );
          }
        }

        if (callbacks?.onStreamFinished) {
          callbacks.onStreamFinished(actualMessageId, topicId);
        }
        removeSessionStream(itemId, topicId, actualMessageId);
      } else {
        removeSessionStream(itemId, topicId, actualMessageId);
      }
    }
  };

  /**
   * 中止指定消息的生成
   */
  const stopMessage = async (
    messageId: string,
    onUpdateMessage?: (msgId: string) => Promise<void>
  ) => {
    console.log(
      `[ChatStreamStore] Sending interrupt signal for message: ${messageId}`
    );
    try {
      await invoke("interruptRequest", { messageId: messageId });
    } catch (e) {
      console.error(
        `[ChatStreamStore] Failed to interrupt stream for ${messageId}:`,
        e
      );
      return;
    }
    sealStreamInputs(messageId);

    // 本地模拟一个结束状态
    const msg = activeStreamMessages.get(messageId);
    if (msg) {
      msg.isThinking = false;
      msg.finishReason = "interrupted";
    }

    // 手动点击中止流时，瞬间强刷并注销 rAF 帧，防止后台句柄悬空空转泄漏
    clearRAFUpdate(messageId, true);
    auroraActiveMessageIds.delete(messageId);
    streamBlockSignatures.delete(messageId);
    streamTailSignatures.delete(messageId);

    if (streamingMessageId.value === messageId) {
      streamingMessageId.value = null;
    }

    const context = activeStreamContexts[messageId];
    const ownerId = context?.itemId || sessionStore.currentSelectedItem?.id;
    const topicId = context?.topicId || sessionStore.currentTopicId;

    if (ownerId && topicId) {
      removeSessionStream(ownerId, topicId, messageId);
    }

    if (onUpdateMessage) {
      await onUpdateMessage(messageId);
    }
  };

  /**
   * 强行中止整个群组的接力赛回合
   */
  const stopGroupTurn = async (topicId: string) => {
    console.log(
      `[ChatStreamStore] Global Group Interruption for topic: ${topicId}`
    );
    try {
      await invoke("interruptGroupTurn", { topicId: topicId });

      const activeIds = Array.from(activeStreamingIds.value);
      if (activeIds.length > 0) {
        await Promise.all(activeIds.map((id) => stopMessage(id)));
      }
    } catch (e) {
      console.error("[ChatStreamStore] Failed to stop group turn:", e);
    }
  };

  onScopeDispose(() => {
    if (typeof document !== "undefined") {
      document.removeEventListener("visibilitychange", handleVisibilityChange);
    }
    if (typeof window !== "undefined") {
      window.removeEventListener("vcp-lifecycle", handleVcpLifecycle);
    }
    cleanupTimers.forEach(clearTimeout);
    cleanupTimers.clear();
    rAFPendingUpdates.forEach((up) => {
      cancelScheduledCommit(up);
    });

    rAFPendingUpdates.clear();
    sealedStreamMessageIds.clear();
    terminalStreamMessageIds.clear();
    if (activeStreamTotal.value > 0) {
      activeStreamTotal.value = 0;
      releaseScreenKeep();
    }
    for (const id of Object.keys(activeStreamContexts)) {
      delete activeStreamContexts[id];
    }
  });

  return {
    streamingMessageId,
    sessionActiveStreams,
    activeStreamMessages,
    allActiveStreamingIds,
    activeStreamingIds,
    isGroupGenerating,
    isMessageInActiveStream,
    isMessageStreamingInSession,
    computeShell,
    addSessionStream,
    removeSessionStream,
    processStreamEvent,
    stopMessage,
    stopGroupTurn,
  };
});
