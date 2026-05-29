import { defineStore } from "pinia";
import { ref, computed, reactive, onScopeDispose } from "vue";
import { invoke } from "@tauri-apps/api/core";
import { releaseScreenKeep } from "../composables/useScreenKeeper";
import { useChatSessionStore } from "./chatSessionStore";
import { useAssistantStore } from "./assistant";
import { useAvatarStore } from "./avatar";
import { useTopicStore } from "./topicListManager";
import type { ChatMessage, MessageShell } from "../types/chat";

export const useChatStreamStore = defineStore("chatStream", () => {
  const streamingMessageId = ref<string | null>(null);

  // 核心：记录每个会话（itemId + topicId）是否处于活动流状态
  // 格式: "itemId:topicId" -> [messageId1, messageId2, ...]
  const sessionActiveStreams = ref<Record<string, string[]>>({});

  // 全局活跃流消息池：存储所有正在生成的响应对象 (messageId -> Reactive<ChatMessage>)
  // 无论是在前台还是后台，流式消息都从此池中获取，保证响应式链路不断裂
  const activeStreamMessages = reactive<Map<string, ChatMessage>>(new Map());
  const auroraActiveMessageIds = new Set<string>();
  const streamBlockSignatures = new Map<string, string>();
  const streamTailSignatures = new Map<string, string>();
  const cleanupTimers = new Set<ReturnType<typeof setTimeout>>();

  const sessionStore = useChatSessionStore();
  const assistantStore = useAssistantStore();
  const avatarStore = useAvatarStore();
  const topicStore = useTopicStore();

  /**
   * 在前端本地计算 MessageShell（替代 Rust 的 precompute_shell）
   */
  function computeShell(msg: { role: string; agentId?: string; name?: string }): MessageShell {
    const empty = "";
    if (msg.role === "user") {
      const userColor = avatarStore.getDominantColor("user", "user_avatar") || "rgb(226,54,56)";
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

  const enforceStreamPoolLimit = () => {
    if (activeStreamMessages.size <= MAX_STREAM_MESSAGES) return;
    const excess = activeStreamMessages.size - MAX_STREAM_MESSAGES;
    // 按插入顺序（Map 保持插入顺序）清理最旧的非活跃消息
    for (const [id] of activeStreamMessages) {
      if (excess <= 0) break;
      // 只删除已完成的流（不在当前活跃会话中）
      if (!activeStreamingIds.value.has(id)) {
        activeStreamMessages.delete(id);
      }
    }
  };

  // 辅助方法：管理会话流状态
  const addSessionStream = (
    ownerId: string,
    topicId: string,
    messageId: string,
  ) => {
    const key = `${ownerId}:${topicId}`;
    if (!sessionActiveStreams.value[key]) {
      sessionActiveStreams.value[key] = [];
    }
    if (!sessionActiveStreams.value[key].includes(messageId)) {
      sessionActiveStreams.value[key].push(messageId);
    }
    // 新增流时检查并执行上限保护
    enforceStreamPoolLimit();
  };

  const removeSessionStream = (
    ownerId: string,
    topicId: string,
    messageId: string,
  ) => {
    const key = `${ownerId}:${topicId}`;
    const streams = sessionActiveStreams.value[key];
    let didRemove = false;
    if (streams) {
      const index = streams.indexOf(messageId);
      if (index !== -1) {
        streams.splice(index, 1);
        didRemove = true;
      }
      if (streams.length === 0) {
        delete sessionActiveStreams.value[key];
      }
    }
    if (didRemove && Object.keys(sessionActiveStreams.value).length === 0) {
      releaseScreenKeep();
    }
    // 同时从全局池中移除 (延迟移除，确保 finalizeStream 能拿到对象)
    const cleanupTimer = setTimeout(() => {
        if (!activeStreamingIds.value.has(messageId)) {
            activeStreamMessages.delete(messageId);
            auroraActiveMessageIds.delete(messageId);
            streamBlockSignatures.delete(messageId);
            streamTailSignatures.delete(messageId);
        }
    }, 1000);
    cleanupTimers.add(cleanupTimer);
  };

  const blocksSignature = (blocks: Array<{ hash?: string; type?: string }> = []) =>
    blocks.map((block) => block.hash || block.type || "").join("|");

  const tailSignature = (tailBlock: { hash?: string; type?: string } | undefined, tail?: string) =>
    tailBlock?.hash || `${tailBlock?.type || ""}:${tail || ""}`;

  /**
   * 处理流式事件的核心逻辑 (会话隔离调度器)
   */
  const processStreamEvent = async (event: any, callbacks?: {
    onMessageCreated?: (msg: ChatMessage, topicId: string) => void;
    onStreamFinished?: (messageId: string, topicId: string) => void;
  }) => {
    const actualMessageId = event.messageId || event.message_id || "";
    const { chunk, type, context } = event;
    const ctx = context || {};
    const topicId = ctx.topicId;
    const isGroup = !!ctx.isGroupMessage || !!ctx.groupId;
    const itemId = isGroup ? ctx.groupId : (ctx.agentId || ctx.ownerId);
 
    if (!actualMessageId || !topicId || !itemId) return;
 
    let msg = activeStreamMessages.get(actualMessageId);
    const isNewStream = !msg;
 
    if (isNewStream) {
      auroraActiveMessageIds.delete(actualMessageId);
      msg = reactive<ChatMessage>({
        id: actualMessageId,
        role: "assistant",
        name: ctx.agentName,
        content: "",
        timestamp: Date.now(),
        isThinking: type === "thinking",
        agentId: ctx.agentId,
        groupId: ctx.groupId,
        isGroupMessage: !!ctx.isGroupMessage,
        shell: computeShell({ role: "assistant", agentId: ctx.agentId, name: ctx.agentName }),
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

      // [关键修复] 异步持久化骨架消息到 SQLite 数据库
      // 使得用户即便中途切换会话，重新加载历史时也存在此消息占位，从而触发 Object Hydration 完美接续流式动画
      invoke("append_single_message", {
        ownerId: itemId,
        ownerType: isGroup ? "group" : "agent",
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
        }
      }).catch(e => {
        console.error("[ChatStreamStore] Failed to persist initial thinking skeleton:", e);
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
        msg!.content = (msg!.content || "") + textChunk;
      }
    } else if (type === "aurora") {
      const aurora = event.aurora;
      if (aurora) {
        auroraActiveMessageIds.add(actualMessageId);
        if (typeof aurora.content === "string") {
          msg!.content = aurora.content;
        } else if (typeof aurora.contentDelta === "string") {
          msg!.content = (msg!.content || "") + aurora.contentDelta;
        }
        if (aurora.stableBlocks) {
          const nextSignature = blocksSignature(aurora.stableBlocks);
          if (streamBlockSignatures.get(actualMessageId) !== nextSignature) {
            msg!.blocks = aurora.stableBlocks as any;
            streamBlockSignatures.set(actualMessageId, nextSignature);
          }
        }
        if (aurora.tailChanged) {
          const nextTail = aurora.tail || "";
          const nextTailBlock = (aurora.tailBlock as any) || undefined;
          const nextTailSignature = tailSignature(nextTailBlock, nextTail);
          if (streamTailSignatures.get(actualMessageId) !== nextTailSignature) {
            msg!.tailContent = nextTail;
            msg!.tailBlock = nextTailBlock;
            streamTailSignatures.set(actualMessageId, nextTailSignature);
          }
        }
      }
      msg!.isThinking = false;
      addSessionStream(itemId, topicId, actualMessageId);
    } else if (type === "end" || type === "error") {
      const errorMsg = event.error;
      const finishReason = event.finishReason;

      // 执行完成逻辑 (取代原 streamManager.finalizeStream)
      msg!.tailContent = "";
      msg!.tailBlock = undefined;
      auroraActiveMessageIds.delete(actualMessageId);
      streamBlockSignatures.delete(actualMessageId);
      streamTailSignatures.delete(actualMessageId);
      if (finishReason) msg!.finishReason = finishReason;

      removeSessionStream(itemId, topicId, actualMessageId);
      if (streamingMessageId.value === actualMessageId) streamingMessageId.value = null;

      if (type === "error" && errorMsg && errorMsg !== "请求已中止") {
        const errorText = `\n\n> VCP流式错误: ${errorMsg}`;
        msg!.content += errorText;
        msg!.finishReason = "error";
      }

      if (msg) {
        msg!.isThinking = false;
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
        }
        
        if (callbacks?.onStreamFinished) {
          callbacks.onStreamFinished(actualMessageId, topicId);
        }
      }
    }
  };

  /**
   * 中止指定消息的生成
   */
  const stopMessage = async (messageId: string, onUpdateMessage?: (msgId: string) => Promise<void>) => {
    console.log(
      `[ChatStreamStore] Sending interrupt signal for message: ${messageId}`,
    );
    try {
      await invoke("interruptRequest", { messageId: messageId });
      
      // 本地模拟一个结束状态
      const msg = activeStreamMessages.get(messageId);
      if (msg) {
        msg.isThinking = false;
        msg.finishReason = "interrupted";
      }

      if (streamingMessageId.value === messageId) {
        streamingMessageId.value = null;
      }

      const ownerId = sessionStore.currentSelectedItem?.id;
      const topicId = sessionStore.currentTopicId;

      if (ownerId && topicId) {
        removeSessionStream(ownerId, topicId, messageId);
      }

      if (onUpdateMessage) {
        await onUpdateMessage(messageId);
      }
    } catch (e) {
      console.error(
        `[ChatStreamStore] Failed to interrupt stream for ${messageId}:`,
        e,
      );
    }
  };

  /**
   * 强行中止整个群组的接力赛回合
   */
  const stopGroupTurn = async (topicId: string) => {
    console.log(`[ChatStreamStore] Global Group Interruption for topic: ${topicId}`);
    try {
      await invoke("interruptGroupTurn", { topicId: topicId });
      
      const activeIds = Array.from(activeStreamingIds.value);
      if (activeIds.length > 0) {
        await Promise.all(activeIds.map(id => stopMessage(id)));
      }
    } catch (e) {
      console.error("[ChatStreamStore] Failed to stop group turn:", e);
    }
  };

  onScopeDispose(() => {
    cleanupTimers.forEach(clearTimeout);
    cleanupTimers.clear();
  });

  return {
    streamingMessageId,
    sessionActiveStreams,
    activeStreamMessages,
    activeStreamingIds,
    isGroupGenerating,
    computeShell,
    addSessionStream,
    removeSessionStream,
    processStreamEvent,
    stopMessage,
    stopGroupTurn,
  };
});
