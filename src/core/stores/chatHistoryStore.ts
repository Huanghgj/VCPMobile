import { defineStore } from "pinia";
import { ref } from "vue";
import { invoke, Channel } from "@tauri-apps/api/core";
import { useChatSessionStore } from "./chatSessionStore";
import { useChatStreamStore } from "./chatStreamStore";
import { useAttachmentStore } from "./attachmentStore";
import { useAssistantStore } from "./assistant";
import { useSettingsStore } from "./settings";
import { useTopicStore } from "./topicListManager";
import { useNotificationStore } from "./notification";
import { useWatchTogetherStore } from "./watchTogether";
import { clearMessageCache } from "../utils/astRenderer";
import { preloadMessageImages } from "../utils/messageAssetPreloader";
import type { ChatMessage, HistoryChunk, ContentBlock } from "../types/chat";

type ChatTarget = Readonly<{
  ownerId: string;
  ownerType: "agent" | "group";
  topicId: string;
}>;

const DEBUG_ASSISTANT_RENDER_PROBE = [
  "<think>",
  "Prepare a safe UI render probe with one tool call, one tool result, and a final HTML response.",
  "</think><<<[TOOL_REQUEST]>>>",
  "maid:「始」RenderProbe「末」,",
  "tool_name:「始」ImageProbe「末」,",
  "mode:「始」debug「末」,",
  "prompt:「始」safe render probe image「末」",
  "<<<[END_TOOL_REQUEST]>>>",
  "<<<[TOOL_REQUEST]>>>",
  "maid:「始」RenderProbe「末」,",
  "tool_name:「始」BoundaryRecoveryProbe「末」,",
  "prompt:「始」this request intentionally omits its closing marker",
  "<<<[ROLE_DIVIDE_USER]>>>",
  "",
  "[[VCP调用结果信息汇总:",
  "- 工具名称: ImageProbe",
  "- 执行状态: ✅ SUCCESS",
  "- 返回内容: Debug image generated.",
  "",
  "详细信息：",
  "- 图片URL: data:image/svg+xml;base64,PHN2ZyB4bWxucz0iaHR0cDovL3d3dy53My5vcmcvMjAwMC9zdmciIHdpZHRoPSI2NDAiIGhlaWdodD0iMjgwIj48cmVjdCB3aWR0aD0iNjQwIiBoZWlnaHQ9IjI4MCIgZmlsbD0iIzI1NjNlYiIvPjx0ZXh0IHg9IjMyMCIgeT0iMTUwIiB0ZXh0LWFuY2hvcj0ibWlkZGxlIiBmb250LXNpemU9IjQyIiBmb250LWZhbWlseT0iQXJpYWwiIGZpbGw9IndoaXRlIj5WQ1AgUmVuZGVyIFByb2JlPC90ZXh0Pjwvc3ZnPg==",
  "- 文件名: render-probe.svg",
  "",
  "图片预览：",
  '<img src="data:image/svg+xml;base64,PHN2ZyB4bWxucz0iaHR0cDovL3d3dy53My5vcmcvMjAwMC9zdmciIHdpZHRoPSI2NDAiIGhlaWdodD0iMjgwIj48cmVjdCB3aWR0aD0iNjQwIiBoZWlnaHQ9IjI4MCIgZmlsbD0iIzI1NjNlYiIvPjx0ZXh0IHg9IjMyMCIgeT0iMTUwIiB0ZXh0LWFuY2hvcj0ibWlkZGxlIiBmb250LXNpemU9IjQyIiBmb250LWZhbWlseT0iQXJpYWwiIGZpbGw9IndoaXRlIj5WQ1AgUmVuZGVyIFByb2JlPC90ZXh0Pjwvc3ZnPg==" alt="VCP Render Probe" width="300">',
  "",
  "VCP调用结果结束]]",
  "",
  "[本轮工具调用摘要:]",
  "ImageProbe 调用成功。",
  "[本轮工具调用摘要结束]",
  "",
  "<<<[END_ROLE_DIVIDE_USER]>>>",
  "",
  '<think>The debug image was generated successfully. Now render the final safe HTML reply.</think><div id="vcp-root" data-vcp-probe="full-vcp" style="padding:20px; border-radius:16px; background:#f8fafc; color:#0f172a; line-height:1.8;">',
  "",
  '<div style="text-align:center; font-size:12px; color:#64748b; border-bottom:1px dashed #cbd5e1; padding-bottom:8px; margin-bottom:16px;">',
  "VCP Render Probe · Full Tool Result Shape",
  "</div>",
  "",
  "<p>这个块必须作为 HTML 渲染，而不是把标签原样显示出来。</p>",
  "",
  '<img src="data:image/svg+xml;base64,PHN2ZyB4bWxucz0iaHR0cDovL3d3dy53My5vcmcvMjAwMC9zdmciIHdpZHRoPSI2NDAiIGhlaWdodD0iMjgwIj48cmVjdCB3aWR0aD0iNjQwIiBoZWlnaHQ9IjI4MCIgZmlsbD0iIzI1NjNlYiIvPjx0ZXh0IHg9IjMyMCIgeT0iMTUwIiB0ZXh0LWFuY2hvcj0ibWlkZGxlIiBmb250LXNpemU9IjQyIiBmb250LWZhbWlseT0iQXJpYWwiIGZpbGw9IndoaXRlIj5WQ1AgUmVuZGVyIFByb2JlPC90ZXh0Pjwvc3ZnPg==" alt="VCP Render Probe" style="width:100%; border-radius:12px; margin:12px 0;">',
  "",
  "<p>如果你能看到蓝色图片和这个浅色容器，说明最终 HTML 没有被工具结果或角色分隔符吞掉。</p>",
  "",
  "</div><<<[TOOL_REQUEST]>>>",
  "maid:「始」RenderProbe「末」,",
  "tool_name:「始」DailyNote「末」,",
  "command:「始」create「末」,",
  "Date:「始」2026-07-20「末」,",
  "Content:「始ESCAPE」Boundary recovery diary body.「末ESCAPE」,",
  "archery:「始」no_reply「末」",
  '<<<[END_TOOL_REQUEST]>>><details data-probe="details-body">',
  "<summary><b>Boundary recovery details</b></summary>",
  "",
  "Boundary recovery final details body remains visible.",
  "</details>",
].join("\n");

export const useChatHistoryStore = defineStore("chatHistory", () => {
  const currentChatHistory = ref<ChatMessage[]>([]);
  const loading = ref(false);

  // 分页加载状态
  const historyOffset = ref(0); // 当前已加载的消息总数（= 下次请求的 offset 起点）
  const hasMoreHistory = ref(true); // 是否还有更多旧消息
  const isLoadingHistory = ref(false); // 防止并发重复触发
  let historyLoadSequence = 0;

  // 用于拦截重新生成时的输入框补全
  const editMessageContent = ref("");
  // 用于标记当前是否正在“编辑重发”某条历史消息
  const editingOriginalMessageId = ref<string | null>(null);

  // 用于防止并发加载与话题切换导致竞态的消息拉取中止控制器 (AbortController)
  let currentLoadAbortController: AbortController | null = null;

  // 单个异步步骤的兜底超时：invoke 或资源解析永久挂起时（如 DB busy、WebView 冻结），
  // 保证 loadHistory 的 finally 一定能执行、isLoadingHistory 一定复位。
  const HISTORY_STEP_TIMEOUT_MS = 20000;

  const raceWithTimeout = async <T>(
    promise: Promise<T>,
    ms: number,
    label: string,
  ): Promise<{ timedOut: boolean; value?: T }> => {
    let timer: ReturnType<typeof setTimeout> | null = null;
    try {
      return await Promise.race([
        promise.then((value) => ({ timedOut: false, value })),
        new Promise<{ timedOut: true }>((resolve) => {
          timer = setTimeout(() => {
            console.warn(`[ChatHistoryStore] ${label} timed out after ${ms}ms`);
            resolve({ timedOut: true });
          }, ms);
        }),
      ]);
    } finally {
      if (timer !== null) clearTimeout(timer);
    }
  };

  const sessionStore = useChatSessionStore();
  const streamStore = useChatStreamStore();
  const attachmentStore = useAttachmentStore();
  const assistantStore = useAssistantStore();
  const settingsStore = useSettingsStore();
  const topicStore = useTopicStore();
  const notificationStore = useNotificationStore();
  const watchTogetherStore = useWatchTogetherStore();

  const getCurrentTarget = (): ChatTarget | null => {
    const selectedItem = sessionStore.currentSelectedItem;
    const topicId = sessionStore.currentTopicId;
    if (!selectedItem?.id || !topicId) return null;
    return {
      ownerId: selectedItem.id,
      ownerType: selectedItem.type === "group" ? "group" : "agent",
      topicId,
    };
  };

  const isTargetActive = (target: ChatTarget) => {
    const current = getCurrentTarget();
    return (
      current?.ownerId === target.ownerId &&
      current.ownerType === target.ownerType &&
      current.topicId === target.topicId
    );
  };

  const summarizeTopic = async (
    target: ChatTarget | null = getCurrentTarget(),
  ) => {
    if (!target || !isTargetActive(target)) return;

    const { topicId, ownerId, ownerType } = target;

    const topic = topicStore.topics.find((t) => t.id === topicId);
    const isDefaultName =
      topic && /^(新话题|新会话) \d{2}:\d{2}:\d{2}$/.test(topic.name);
    const messageCount = currentChatHistory.value.filter(
      (m) => m.role !== "system",
    ).length;

    if (!isDefaultName || messageCount < 4) return;

    try {
      const agentName =
        assistantStore.agents.find((agent: any) => agent.id === ownerId)
          ?.name || "AI";
      const newTitle = await invoke<string>("summarize_topic", {
        ownerId,
        ownerType,
        topicId,
        agentName,
      });

      if (newTitle) {
        await topicStore.updateTopicTitle(
          ownerId,
          ownerType,
          topicId,
          newTitle,
        );
      }
    } catch (e) {
      console.error("[ChatHistoryStore] AI Summary failed:", e);
    }
  };

  /**
   * 加载聊天历史
   */
  const loadHistory = async (
    ownerId: string,
    ownerType: string,
    topicId: string,
    limit: number = 15,
    offset: number = 0,
  ) => {
    const loadType = offset === 0 ? "initial" : "pagination";
    console.log(
      `[ChatHistoryStore] Loading history [${loadType}] for ${ownerId}, topic: ${topicId}, limit: ${limit}, offset: ${offset}`,
    );
    loading.value = true;
    isLoadingHistory.value = true;
    const loadSequence = ++historyLoadSequence;

    if (currentLoadAbortController) {
      currentLoadAbortController.abort();
    }
    const controller = new AbortController();
    currentLoadAbortController = controller;
    const { signal } = controller;

    let pendingHistory: ChatMessage[] = [];
    let flushRafId: number | null = null;
    let completeWatchdogId: ReturnType<typeof setTimeout> | null = null;

    try {
      const requestedTopicId = topicId;
      const isStaleLoad = () => loadSequence !== historyLoadSequence;
      const canApplyLoad = () =>
        !signal.aborted &&
        !isStaleLoad() &&
        sessionStore.currentTopicId === requestedTopicId;
      const channel = new Channel<HistoryChunk>();
      const buffer: ChatMessage[] = [];
      let receivedCount = 0;
      let resolveComplete: (() => void) | null = null;
      let isLoadCompleted = false;
      let cancelledByTopicChange = false;
      const completePromise = new Promise<void>((resolve) => {
        resolveComplete = resolve;
      });
      const completeLoad = () => {
        if (isLoadCompleted) return;
        isLoadCompleted = true;
        if (resolveComplete) {
          resolveComplete();
        }
      };

      let lastFlushTime = 0;
      const FLUSH_INTERVAL = 33.3; // 30Hz

      const flushHistory = (force = false) => {
        if (pendingHistory.length === 0) return;
        if (!canApplyLoad()) {
          pendingHistory = [];
          cancelledByTopicChange = true;
          completeLoad();
          return;
        }
        const now = performance.now();
        if (force || now - lastFlushTime >= FLUSH_INTERVAL) {
          currentChatHistory.value = [
            ...currentChatHistory.value,
            ...pendingHistory,
          ];
          pendingHistory = [];
          lastFlushTime = now;
        }
      };

      const scheduleHistoryFlush = () => {
        if (flushRafId) return;
        flushRafId = requestAnimationFrame(() => {
          flushRafId = null;
          flushHistory(false);
          if (pendingHistory.length > 0 && canApplyLoad()) {
            scheduleHistoryFlush();
          }
        });
      };

      channel.onmessage = (chunk) => {
        if (!canApplyLoad()) {
          cancelledByTopicChange = true;
          completeLoad();
          return;
        }

        // 1. 唯一性与话题一致性防御性校验：若请求已中止，或当前话题已被切换，直接丢弃该过时流数据
        if (!chunk.message) {
          if (chunk.is_last) {
            if (offset === 0) {
              currentChatHistory.value = [];
              historyOffset.value = 0;
            }
            hasMoreHistory.value = false;
            completeLoad();
          }
          return;
        }

        // 2. [关键修复] 消息对象劫持 (Object Hydration)
        // 如果该消息正在活跃生成中，则从全局流池中取出“活的”响应式对象
        // 这确保了即使是刚从 DB 拉回来的骨架，也能瞬间恢复流式动画与渲染状态
        const activeMsg = streamStore.activeStreamMessages.get(
          chunk.message.id,
        );
        const msgToUse = activeMsg || chunk.message;

        if (offset === 0) {
          if (chunk.index === 0) {
            currentChatHistory.value = [];
            hasMoreHistory.value = true;
          }
          pendingHistory.push(msgToUse);
          receivedCount++;
          scheduleHistoryFlush();
        } else {
          buffer.push(msgToUse);
          receivedCount++;
        }

        if (chunk.is_last) {
          if (!canApplyLoad()) {
            cancelledByTopicChange = true;
            completeLoad();
            return;
          }
          if (offset > 0) {
            currentChatHistory.value = [...buffer, ...currentChatHistory.value];
            historyOffset.value += buffer.length;
            if (buffer.length < limit) {
              hasMoreHistory.value = false;
            }
          } else {
            if (flushRafId !== null) {
              cancelAnimationFrame(flushRafId);
              flushRafId = null;
            }
            flushHistory(true);
            historyOffset.value = receivedCount;
            if (receivedCount < limit) {
              hasMoreHistory.value = false;
            }
          }
          completeLoad();
        }
      };

      const invokeResult = await raceWithTimeout(
        invoke<number>("load_chat_history_streamed", {
          ownerId,
          ownerType,
          topicId,
          limit,
          offset,
          onMessage: channel,
        }),
        HISTORY_STEP_TIMEOUT_MS,
        `load_chat_history_streamed(${topicId})`,
      );
      if (invokeResult.timedOut) {
        // 放行 finally 复位加载标志；晚到的通道消息由 canApplyLoad/序列号校验兜底
        flushHistory(true);
        completeLoad();
        return;
      }
      const total = invokeResult.value as number;

      if (!canApplyLoad() || cancelledByTopicChange) {
        completeLoad();
        return;
      }

      if (total === 0) {
        if (offset === 0) {
          currentChatHistory.value = [];
          historyOffset.value = 0;
        }
        hasMoreHistory.value = false;
        completeLoad();
        return;
      }

      // 安全兜底：completePromise 依赖通道投递的 is_last 事件来 resolve。
      // 在「后台冻结 → 回前台」等场景下，WebView 可能丢弃该通道事件，导致此处永久挂起，
      // 进而把 loading / isLoadingHistory 卡死、目标话题停留在空白（表现为“话题无法切换”）。
      // 这里加一道看门狗：超时后强刷已收到的消息并放行，保证加载状态一定能复位。
      const completeWatchdog = new Promise<void>((resolve) => {
        completeWatchdogId = setTimeout(() => {
          completeWatchdogId = null;
          if (!isLoadCompleted) {
            console.warn(
              `[ChatHistoryStore] Completion watchdog fired for topic ${requestedTopicId}; forcing flush & release.`,
            );
            flushHistory(true);
            completeLoad();
          }
          resolve();
        }, 15000);
      });
      await Promise.race([completePromise, completeWatchdog]);
      if (completeWatchdogId !== null) {
        clearTimeout(completeWatchdogId);
      }

      const loadedCount = offset === 0 ? total : buffer.length;
      console.log(
        `[ChatHistoryStore] Loaded ${loadedCount} messages [${loadType}] for ${ownerId}, topic: ${topicId}`,
      );

      if (!canApplyLoad()) {
        console.warn(
          `[ChatHistoryStore] Topic changed or request aborted during load, discarding results.`,
        );
        return;
      }

      const messagesToResolve =
        offset === 0 ? currentChatHistory.value : buffer;
      // 资源解析不在 15s 完成看门狗的保护范围内，单独兜底，防止卡死加载标志
      await raceWithTimeout(
        Promise.all(
          messagesToResolve.map(async (msg) => {
            await attachmentStore.resolveMessageAssets(msg);
          }),
        ).then(() => preloadMessageImages(messagesToResolve)),
        HISTORY_STEP_TIMEOUT_MS,
        `resolveMessageAssets(${topicId})`,
      );
    } catch (e) {
      console.error("[ChatHistoryStore] Failed to stream history:", e);
    } finally {
      if (currentLoadAbortController === controller) {
        currentLoadAbortController = null;
      }
      if (completeWatchdogId !== null) {
        clearTimeout(completeWatchdogId);
      }
      if (flushRafId !== null) {
        cancelAnimationFrame(flushRafId);
        flushRafId = null;
      }
      if (loadSequence === historyLoadSequence) {
        loading.value = false;
        isLoadingHistory.value = false;
      }
    }
  };

  const loadHistoryPaginated = async (
    ownerId: string,
    ownerType: string,
    topicId: string,
  ) => {
    // 切换话题时强制重置分页状态，避免旧话题状态污染
    historyOffset.value = 0;
    hasMoreHistory.value = true;
    currentChatHistory.value = [];
    await loadHistory(ownerId, ownerType, topicId, 5, 0);
  };

  const loadMoreHistory = async () => {
    if (!hasMoreHistory.value || isLoadingHistory.value) return;
    if (!sessionStore.currentSelectedItem?.id || !sessionStore.currentTopicId)
      return;
    await loadHistory(
      sessionStore.currentSelectedItem.id,
      sessionStore.currentSelectedItem.type,
      sessionStore.currentTopicId,
      10,
      historyOffset.value,
    );
  };

  /**
   * 触发 AI 生成逻辑
   */
  type AffectTurnSource =
    | "user"
    | "lifecycle_heartbeat"
    | "lifecycle_scheduled"
    | "regeneration";

  const invokeGenerationRequestForTarget = async (
    userMsg: ChatMessage,
    target: ChatTarget,
    turnSource: AffectTurnSource = "user",
  ): Promise<{ started: true; messageId?: string }> => {
    const agentId = target.ownerId;
    const topicId = target.topicId;

    const settings = settingsStore.settings;
    if (!settings) throw new Error("应用尚未完成初始化");

    const responseMessageId =
      turnSource === "lifecycle_scheduled"
        ? undefined
        : target.ownerType === "agent"
          ? `msg_${agentId}_${Date.now()}_${Math.random().toString(36).slice(2, 9)}`
          : `group-turn-${topicId}-${Date.now()}-${Math.random().toString(36).slice(2, 9)}`;
    const generationToken = streamStore.beginGeneration(
      target.ownerId,
      target.ownerType,
      topicId,
      responseMessageId,
    );

    const streamChannel = new Channel<any>();
    streamChannel.onmessage = (event) =>
      streamStore.processStreamEvent(event, {
        onMessageCreated: (msg, tid) => {
          if (
            tid === target.topicId &&
            isTargetActive(target) &&
            !currentChatHistory.value.some((m) => m.id === msg.id)
          ) {
            currentChatHistory.value.push(msg);
            currentChatHistory.value.sort((a, b) => a.timestamp - b.timestamp);
          }
        },
        onStreamFinished: (_messageId, tid) => {
          if (tid === target.topicId && isTargetActive(target)) {
            summarizeTopic(target);
          }
        },
      });

    try {
      if (target.ownerType === "group") {
        const result = await invoke<{ messageId?: string }>(
          "handle_group_chat_message",
          {
            payload: {
              groupId: target.ownerId,
              topicId,
              userMessage: userMsg,
              turnSource,
              turnId: responseMessageId,
              vcpUrl: settings.vcpServerUrl || "",
              vcpApiKey: settings.vcpApiKey || "",
            },
            streamChannel,
          },
        );
        return { started: true, messageId: result?.messageId };
      }

      const result = await invoke<{ messageId?: string }>(
        "handle_agent_chat_message",
        {
          payload: {
            agentId,
            topicId,
            userMessage: userMsg,
            turnSource,
            responseMessageId,
            vcpUrl: settings.vcpServerUrl || "",
            vcpApiKey: settings.vcpApiKey || "",
          },
          streamChannel,
        },
      );
      return { started: true, messageId: result?.messageId };
    } finally {
      streamStore.endGeneration(target.ownerId, topicId, generationToken);
    }
  };

  const invokeGenerationRequest = async (
    userMsg: ChatMessage,
    turnSource: AffectTurnSource = "user",
    target: ChatTarget | null = getCurrentTarget(),
  ) => {
    if (!target) return false;
    const result = await invokeGenerationRequestForTarget(
      userMsg,
      target,
      turnSource,
    );
    return result.started;
  };

  /**
   * 触发 AI 生成逻辑
   */
  const triggerGeneration = async (
    userMsg: ChatMessage,
    target: ChatTarget | null = getCurrentTarget(),
    onPersisted?: (compiledBlocks: ContentBlock[]) => void,
  ) => {
    if (!target) return false;

    const transientAttachments = (userMsg.attachments || []).filter(
      (attachment) => attachment.transient,
    );
    const durableAttachments = (userMsg.attachments || []).filter(
      (attachment) => !attachment.transient,
    );
    const persistedUserMsg: ChatMessage = {
      ...userMsg,
      attachments: durableAttachments.length > 0 ? durableAttachments : undefined,
      transientContext: undefined,
      transientSystemPrompt: undefined,
    };
    const cleanupTransientAttachments = async () => {
      await Promise.allSettled(
        transientAttachments
          .map((attachment) => attachment.hash)
          .filter((hash): hash is string => Boolean(hash))
          .map((hash) => invoke("cleanup_single_orphaned_attachment", { hash })),
      );
      const visibleMessage = currentChatHistory.value.find(
        (message) => message.id === userMsg.id,
      );
      if (visibleMessage?.attachments) {
        visibleMessage.attachments = visibleMessage.attachments.filter(
          (attachment) => !attachment.transient,
        );
      }
      if (visibleMessage) {
        visibleMessage.transientContext = undefined;
        visibleMessage.transientSystemPrompt = undefined;
      }
    };

    let compiledBlocks: ContentBlock[];
    try {
      compiledBlocks = await invoke<ContentBlock[]>(
        "append_single_message",
        {
          ownerId: target.ownerId,
          ownerType: target.ownerType,
          topicId: target.topicId,
          message: {
            ...persistedUserMsg,
            blocks: undefined, // 强行设为 undefined，迫使后端执行真正的编译，生成 markdown AST 节点与表情包匹配
          },
        },
      );
    } catch (e) {
      await cleanupTransientAttachments();
      console.error("[ChatHistoryStore] Message persistence failed:", e);
      notificationStore.addNotification({
        type: "error",
        title: "消息保存失败",
        message: e instanceof Error ? e.message : String(e),
        toastOnly: true,
        duration: 5000,
      });
      return false;
    }

    if (onPersisted) {
      onPersisted(compiledBlocks);
    } else {
      const messageIndex = isTargetActive(target)
        ? currentChatHistory.value.findIndex((m) => m.id === userMsg.id)
        : -1;
      if (messageIndex !== -1) {
        currentChatHistory.value[messageIndex] = {
          ...currentChatHistory.value[messageIndex],
          blocks: compiledBlocks,
        };
      }
    }

    try {
      return await invokeGenerationRequest(userMsg, "user", target);
    } catch (e) {
      console.error("[ChatHistoryStore] Generation failed:", e);
      notificationStore.addNotification({
        type: "error",
        title: "消息发送失败",
        message: e instanceof Error ? e.message : String(e),
        toastOnly: true,
        duration: 5000,
      });
      return false;
    } finally {
      await cleanupTransientAttachments();
    }
  };

  const triggerHiddenLifecycleMessage = async (
    content: string,
    expected?: {
      ownerId: string;
      ownerType: "agent" | "group";
      topicId: string;
    },
  ) => {
    if (
      !sessionStore.currentSelectedItem ||
      !sessionStore.currentTopicId ||
      !content.trim()
    )
      return false;

    const selectedItem = sessionStore.currentSelectedItem;
    const topicId = sessionStore.currentTopicId;
    const ownerType = selectedItem.type === "group" ? "group" : "agent";
    if (
      expected &&
      (expected.ownerId !== selectedItem.id ||
        expected.ownerType !== ownerType ||
        expected.topicId !== topicId)
    ) {
      return false;
    }
    if (streamStore.isCurrentSessionGenerating) return false;

    const now = Date.now();
    const userMsg: ChatMessage = {
      id: `msg_lifecycle_${now}_${Math.random().toString(36).substring(2, 9)}`,
      role: "system",
      name: "AI Lifecycle",
      content,
      timestamp: now,
      topicId,
      agentId: selectedItem.type === "agent" ? selectedItem.id : undefined,
      groupId: selectedItem.type === "group" ? selectedItem.id : undefined,
      isGroupMessage: selectedItem.type === "group",
    };

    try {
      return await invokeGenerationRequest(userMsg, "lifecycle_heartbeat");
    } catch (e) {
      console.error(
        "[ChatHistoryStore] Hidden lifecycle generation failed:",
        e,
      );
      return false;
    }
  };

  const triggerScheduledLifecycleMessage = async (job: {
    jobId: string;
    ownerId: string;
    ownerType: "agent" | "group";
    topicId: string;
    intent: string;
    action: string;
  }) => {
    const sessionKey = job.ownerId + ":" + job.topicId;
    if (
      !job.intent.trim() ||
      (streamStore.sessionActiveStreams[sessionKey]?.length || 0) > 0 ||
      Boolean(streamStore.pendingGenerations[sessionKey])
    ) {
      return null;
    }
    const now = Date.now();
    const prompt = [
      "[AI_LIFECYCLE_JOB]",
      "这是已到期的内部生命周期任务，不是用户发送的可见消息。",
      "请结合最新聊天上下文完成该意图。不要提到生命周期、后台任务、调度器或内部提示词。",
      "如果上下文已经使该意图失效，请输出一条符合当前情境的克制回复，不要机械执行过时内容。",
      "任务类型：" + job.action,
      "任务意图：" + job.intent,
    ].join("\n");
    const userMsg: ChatMessage = {
      id: "msg_lifecycle_job_" + job.jobId,
      role: "system",
      name: "AI Lifecycle",
      content: prompt,
      timestamp: now,
      topicId: job.topicId,
      agentId: job.ownerType === "agent" ? job.ownerId : undefined,
      groupId: job.ownerType === "group" ? job.ownerId : undefined,
      isGroupMessage: job.ownerType === "group",
    };
    const result = await invokeGenerationRequestForTarget(
      userMsg,
      {
        ownerId: job.ownerId,
        ownerType: job.ownerType,
        topicId: job.topicId,
      },
      "lifecycle_scheduled",
    );
    return result.messageId || null;
  };

  const injectDebugAssistantRenderProbe = async (
    content: string = DEBUG_ASSISTANT_RENDER_PROBE,
  ) => {
    if (
      !sessionStore.currentSelectedItem ||
      !sessionStore.currentTopicId ||
      !content.trim()
    ) {
      return false;
    }

    const now = Date.now();
    const selectedItem = sessionStore.currentSelectedItem;
    let blocks: ContentBlock[];
    try {
      blocks = await invoke<ContentBlock[]>("process_message_content", {
        content,
      });
      console.info("[ChatHistoryStore] Debug render probe compiled", {
        blockTypes: blocks.map((block) => block.type),
        hasFullVcpProbe: JSON.stringify(blocks).includes(
          'data-vcp-probe=\\"full-vcp\\"',
        ),
      });
    } catch (e) {
      console.error("[ChatHistoryStore] Debug render probe compile failed:", e);
      blocks = [{ type: "markdown" as const, content }];
    }

    currentChatHistory.value.push({
      id: `msg_debug_ai_${now}_${Math.random().toString(36).substring(2, 9)}`,
      role: "assistant",
      name: "AI Render Probe",
      content,
      timestamp: now,
      topicId: sessionStore.currentTopicId,
      agentId: selectedItem.type === "agent" ? selectedItem.id : undefined,
      groupId: selectedItem.type === "group" ? selectedItem.id : undefined,
      isGroupMessage: selectedItem.type === "group",
      shell: streamStore.computeShell({
        role: "assistant",
        agentId: selectedItem.type === "agent" ? selectedItem.id : undefined,
        name: "AI Render Probe",
      }),
      blocks,
    });
    return true;
  };

  /**
   * 发送消息
   */
  const sendMessage = async (content: string) => {
    const target = getCurrentTarget();
    if (
      !target ||
      (!content.trim() && attachmentStore.stagedAttachments.length === 0)
    ) {
      return false;
    }

    if (
      attachmentStore.stagedAttachments.some(
        (attachment) => attachment.status === "loading",
      )
    ) {
      notificationStore.addNotification({
        type: "warning",
        title: "附件仍在处理中",
        message: "请等待附件完成注册后再发送。",
        toastOnly: true,
        duration: 2400,
      });
      return false;
    }

    if (editingOriginalMessageId.value) {
      const originalId = editingOriginalMessageId.value;
      const targetIndex = currentChatHistory.value.findIndex(
        (message) => message.id === originalId,
      );
      if (targetIndex !== -1) {
        const targetMsg: ChatMessage = {
          ...currentChatHistory.value[targetIndex],
          content,
          blocks: [{ type: "markdown" as const, content }],
        };
        await invoke("truncate_history_after_timestamp", {
          ownerId: target.ownerId,
          ownerType: target.ownerType,
          topicId: target.topicId,
          timestamp: targetMsg.timestamp,
        });
        const countToDelete = currentChatHistory.value.length - targetIndex - 1;
        if (isTargetActive(target)) {
          const currentIndex = currentChatHistory.value.findIndex(
            (message) => message.id === originalId,
          );
          if (currentIndex !== -1) {
            currentChatHistory.value = currentChatHistory.value.slice(
              0,
              currentIndex + 1,
            );
          }
        }
        if (countToDelete > 0) {
          topicStore.decrementTopicMsgCount(target.topicId, countToDelete);
        }
        return await triggerGeneration(targetMsg, target, (compiledBlocks) => {
          editingOriginalMessageId.value = null;
          if (!isTargetActive(target)) return;
          const currentIndex = currentChatHistory.value.findIndex(
            (message) => message.id === originalId,
          );
          if (currentIndex !== -1) {
            currentChatHistory.value[currentIndex] = {
              ...targetMsg,
              blocks: compiledBlocks,
            };
          }
        });
      }
    }

    const durableStaged = [...attachmentStore.stagedAttachments];
    if (durableStaged.length > 0) {
      await attachmentStore.preProcessDocuments(durableStaged);
    }

    const watchContext = await watchTogetherStore.prepareOutgoingContext();
    const currentStaged = [
      ...durableStaged,
      ...(watchContext?.attachment ? [watchContext.attachment] : []),
    ];
    const stagedIds = currentStaged
      .map((attachment) => attachment.id)
      .filter((id): id is string => typeof id === "string");
    const now = Date.now();
    const userName = settingsStore.settings?.userName || "User";
    const userMsg: ChatMessage = {
      id: `msg_${now}_user_${Math.random().toString(36).substring(2, 9)}`,
      role: "user",
      name: userName,
      content,
      timestamp: now,
      topicId: target.topicId,
      agentId: target.ownerType === "agent" ? target.ownerId : undefined,
      groupId: target.ownerType === "group" ? target.ownerId : undefined,
      isGroupMessage: target.ownerType === "group",
      attachments: currentStaged.length > 0 ? currentStaged : undefined,
      transientContext: watchContext?.transientContext,
      transientSystemPrompt: watchContext?.transientSystemPrompt,
      shell: streamStore.computeShell({ role: "user", name: userName }),
      blocks: [{ type: "markdown" as const, content }],
    };

    return await triggerGeneration(userMsg, target, (compiledBlocks) => {
      attachmentStore.consumeStaged(stagedIds);
      if (isTargetActive(target)) {
        currentChatHistory.value.push({
          ...userMsg,
          blocks: compiledBlocks,
        });
      }
      topicStore.incrementTopicMsgCount(target.topicId);
    });
  };

  /**
   * 删除消息
   */
  const deleteMessage = async (
    messageId: string,
    deleteAfter: boolean = false,
  ) => {
    const target = getCurrentTarget();
    if (!target) {
      throw new Error("当前会话状态无效，无法删除消息");
    }

    const targetIndex = currentChatHistory.value.findIndex(
      (m) => m.id === messageId,
    );
    if (targetIndex === -1) {
      throw new Error("消息已不存在或不属于当前话题");
    }

    const targetMsg = { ...currentChatHistory.value[targetIndex] };
    if (deleteAfter) {
      const countToDelete = currentChatHistory.value.length - targetIndex;
      await invoke("truncate_history_after_timestamp", {
        ownerId: target.ownerId,
        ownerType: target.ownerType,
        topicId: target.topicId,
        timestamp: targetMsg.timestamp - 1,
      });
      if (isTargetActive(target)) {
        const currentIndex = currentChatHistory.value.findIndex(
          (message) => message.id === messageId,
        );
        if (currentIndex !== -1) currentChatHistory.value.splice(currentIndex);
      }
      topicStore.decrementTopicMsgCount(target.topicId, countToDelete);
    } else {
      await invoke("delete_messages", {
        ownerId: target.ownerId,
        ownerType: target.ownerType,
        topicId: target.topicId,
        msgIds: [messageId],
      });
      if (isTargetActive(target)) {
        const currentIndex = currentChatHistory.value.findIndex(
          (message) => message.id === messageId,
        );
        if (currentIndex !== -1) {
          currentChatHistory.value.splice(currentIndex, 1);
        }
      }
      topicStore.decrementTopicMsgCount(target.topicId, 1);
    }
  };

  const deleteAttachment = async (
    topicId: string,
    messageId: string,
    hash: string,
  ) => {
    await invoke("delete_message_attachment", {
      topicId,
      messageId,
      hash,
    });

    const message =
      sessionStore.currentTopicId === topicId
        ? currentChatHistory.value.find((item) => item.id === messageId)
        : undefined;
    if (message?.attachments) {
      message.attachments = message.attachments.filter(
        (attachment) => attachment.hash !== hash,
      );
    }
  };

  const updateMessageContent = async (
    messageId: string,
    newContent: string,
  ) => {
    const target = getCurrentTarget();
    if (!target) return;
    const targetIndex = currentChatHistory.value.findIndex(
      (message) => message.id === messageId,
    );
    if (targetIndex === -1) return;

    const updatedMessage: ChatMessage = {
      ...currentChatHistory.value[targetIndex],
      content: newContent,
      blocks: [{ type: "markdown" as const, content: newContent }],
    };

    try {
      const compiledBlocks = await invoke<ContentBlock[]>(
        "patch_single_message",
        {
          ownerId: target.ownerId,
          ownerType: target.ownerType,
          topicId: target.topicId,
          message: {
            ...updatedMessage,
            blocks: undefined,
          },
        },
      );
      if (!isTargetActive(target)) return;
      const currentIndex = currentChatHistory.value.findIndex(
        (message) => message.id === messageId,
      );
      if (currentIndex === -1) return;
      clearMessageCache(messageId);
      currentChatHistory.value[currentIndex] = {
        ...updatedMessage,
        blocks: compiledBlocks,
      };
    } catch (e) {
      console.error("[updateMessageContent] patch_single_message failed:", e);
      notificationStore.addNotification({
        type: "error",
        title: "消息编辑失败",
        message: e instanceof Error ? e.message : String(e),
        toastOnly: true,
        duration: 5000,
      });
      throw e;
    }
  };

  const regenerateResponse = async (targetMessageId: string) => {
    const target = getCurrentTarget();
    if (!target) return;
    const targetIndex = currentChatHistory.value.findIndex(
      (message) => message.id === targetMessageId,
    );
    if (targetIndex === -1) return;

    const { topicId, ownerId, ownerType } = target;

    // 1. 寻找该 AI 消息之前的最后一条用户消息
    let lastUserMsgIndex = targetIndex - 1;
    while (
      lastUserMsgIndex >= 0 &&
      currentChatHistory.value[lastUserMsgIndex].role !== "user"
    ) {
      lastUserMsgIndex--;
    }

    if (lastUserMsgIndex === -1) {
      console.warn(
        "[ChatHistoryStore] No user message found to regenerate from.",
      );
      return;
    }

    const lastUserMsg = currentChatHistory.value[lastUserMsgIndex];

    // 2. 乐观更新 UI：截断历史
    const countToDelete =
      currentChatHistory.value.length - (lastUserMsgIndex + 1);
    currentChatHistory.value = currentChatHistory.value.slice(
      0,
      lastUserMsgIndex + 1,
    );
    topicStore.decrementTopicMsgCount(topicId, countToDelete);

    // 3. 调用后端重生成接口
    try {
      const streamChannel = new Channel<any>();
      streamChannel.onmessage = (event) =>
        streamStore.processStreamEvent(event, {
          onMessageCreated: (msg, tid) => {
            if (
              tid === target.topicId &&
              isTargetActive(target) &&
              !currentChatHistory.value.some((m) => m.id === msg.id)
            ) {
              currentChatHistory.value.push(msg);
              currentChatHistory.value.sort(
                (a, b) => a.timestamp - b.timestamp,
              );
            }
          },
          onStreamFinished: (_messageId, tid) => {
            if (tid === target.topicId && isTargetActive(target)) {
              summarizeTopic(target);
            }
          },
        });

      await invoke("regenerate_topic_response", {
        ownerId,
        ownerType,
        topicId,
        targetUserMsgId: lastUserMsg.id,
        streamChannel,
      });
    } catch (e) {
      console.error("[ChatHistoryStore] Regeneration failed:", e);
      if (isTargetActive(target)) {
        await Promise.all([
          loadHistoryPaginated(ownerId, ownerType, topicId),
          topicStore.loadTopicList(ownerId, ownerType),
        ]);
        notificationStore.addNotification({
          type: "error",
          title: "重新生成失败",
          message: e instanceof Error ? e.message : String(e),
          toastOnly: true,
          duration: 5000,
        });
      }
    }
  };

  const fetchRawContent = async (messageId: string): Promise<string> => {
    const target = getCurrentTarget();
    if (!target) return "";
    const existingMsg = currentChatHistory.value.find(
      (message) => message.id === messageId,
    );
    if (existingMsg?.content) return existingMsg.content;
    try {
      const content = await invoke<string>("fetch_raw_message_content", {
        messageId,
        topicId: target.topicId,
      });
      if (isTargetActive(target)) {
        const currentMessage = currentChatHistory.value.find(
          (message) => message.id === messageId,
        );
        if (currentMessage) currentMessage.content = content;
      }
      return content;
    } catch (_error) {
      return "";
    }
  };

  const persistMessageBlocks = async (
    messageId: string,
    blocks: ContentBlock[],
  ) => {
    const target = getCurrentTarget();
    if (!target) return;
    const message = currentChatHistory.value.find(
      (item) => item.id === messageId,
    );
    if (!message) return;
    message.blocks = blocks;
    try {
      await invoke("patch_single_message", {
        ownerId: target.ownerId,
        ownerType: target.ownerType,
        topicId: target.topicId,
        message: { ...message, blocks },
      });
    } catch (e) {
      console.error("[persistMessageBlocks] patch_single_message failed:", e);
    }
  };

  const reRenderMessage = async (messageId: string, topicId: string) => {
    const target = getCurrentTarget();
    if (!target || target.topicId !== topicId) {
      throw new Error("消息未在当前历史记录中找到");
    }
    const targetIndex = currentChatHistory.value.findIndex(
      (message) => message.id === messageId,
    );
    if (targetIndex === -1) {
      throw new Error("消息未在当前历史记录中找到");
    }

    clearMessageCache(messageId);

    try {
      const compiledBlocks = await invoke<ContentBlock[]>("re_render_message", {
        messageId,
        topicId,
      });
      if (!isTargetActive(target)) return;
      const currentIndex = currentChatHistory.value.findIndex(
        (message) => message.id === messageId,
      );
      if (currentIndex === -1) return;
      currentChatHistory.value[currentIndex] = {
        ...currentChatHistory.value[currentIndex],
        blocks: compiledBlocks,
      };
    } catch (e) {
      console.error("[reRenderMessage] re_render_message failed:", e);
      throw e;
    }
  };

  return {
    currentChatHistory,
    loading,
    historyOffset,
    hasMoreHistory,
    isLoadingHistory,
    editMessageContent,
    editingOriginalMessageId,
    loadHistory,
    loadHistoryPaginated,
    loadMoreHistory,
    sendMessage,
    deleteMessage,
    deleteAttachment,
    triggerGeneration,
    triggerHiddenLifecycleMessage,
    triggerScheduledLifecycleMessage,
    injectDebugAssistantRenderProbe,
    summarizeTopic,
    updateMessageContent,
    regenerateResponse,
    fetchRawContent,
    persistMessageBlocks,
    reRenderMessage,
  };
});
