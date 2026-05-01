import { defineStore } from "pinia";
import { ref, computed, nextTick } from "vue";
import { convertFileSrc, invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";

import { useStreamManagerStore } from "./streamManager";
import { useSettingsStore } from "./settings";
import { useAssistantStore } from "./assistant";
import { useTopicStore } from "./topicListManager";
import { useNotificationStore } from "./notification";
import { usePerformanceDiagnostics } from "../utils/performanceDiagnostics";
import { syncService } from "../utils/syncService";
import { useDocumentProcessor } from "../composables/useDocumentProcessor";
import type { ContentBlock } from "../composables/useContentProcessor";
import {
  RuntimeLruCache,
  estimateJsonBytes,
  estimateStringBytes,
} from "../utils/runtimeLruCache";
import { clearMarkdownRenderCache } from "../utils/markdownRenderCache";
import { clearAttachmentThumbnailCache } from "../utils/mediaCache";
import {
  consumeDesktopPushBlocks,
  resetDesktopPushConsumption,
  stripDesktopPushBlocks,
} from "../../features/surface/surfaceRuntime";

/**
 * Attachment 接口定义，严格对齐 Rust 端的 AttachmentSyncDTO / Attachment (仅保留核心字段)
 */
export interface Attachment {
  id?: string; // 纯前端 UI 稳定性标识 (Stable Key)
  type: string;
  name: string;
  size: number;
  progress?: number; // 0-100 的真实上传进度
  src: string; // 仅用于前端 UI 临时渲染路径 (如 file:// 或 blob:)
  resolvedSrc?: string; // Webview 可用的 asset:// 路径
  hash?: string;
  status?: string;
  internalPath?: string;
  extractedText?: string;
  imageFrames?: string[];
  thumbnailPath?: string;
  createdAt?: number;
}

/**
 * ChatMessage 接口定义，严格对齐 Rust 端的 MessageSyncDTO / ChatMessage
 */
export interface ChatMessage {
  id: string;
  role: string;
  name?: string;
  content?: string; // 原文，打开话题时随历史一起加载，避免滑动时再补取
  blocks?: ContentBlock[]; // 预编译的 AST 数据块，前端直接渲染
  timestamp: number;

  isThinking?: boolean;
  agentId?: string;
  groupId?: string;
  isGroupMessage?: boolean;
  attachments?: Attachment[];

  // 以下为纯前端运行时 UI 状态 (Ephemeral)，绝不进行持久化
  displayedContent?: string; // 兼容旧渲染路径的流式全量文本
  stableContent?: string; // 流式稳定层：已经沉淀、不需要高频刷新的内容
  tailContent?: string; // 流式尾部层：正在生成、允许高频刷新的小段内容
  processedContent?: string; // 缓存 Rust 返回的 AST 或文本，避免重复解析
}

/**
 * TopicDelta 接口定义
 */
export interface TopicDelta {
  added: ChatMessage[];
  updated: ChatMessage[];
  deleted_ids: string[];
  sync_skipped?: boolean;
}

/**
 * TopicFingerprint 接口定义
 */
export interface TopicFingerprint {
  topic_id: string;
  mtime: number;
  size: number;
  msg_count: number;
}

interface TopicHistoryCacheEntry {
  ownerId: string;
  ownerType: string;
  topicId: string;
  messages: ChatMessage[];
  loadedAt: number;
  estimatedBytes: number;
}

/**
 * useChatManagerStore
 */
export const useChatManagerStore = defineStore("chatManager", () => {
  // --- 状态变量 (State) ---
  const currentChatHistory = ref<ChatMessage[]>([]);
  const currentSelectedItem = ref<any>(null);
  const currentTopicId = ref<string | null>(null);
  const loading = ref(false);
  const streamingMessageId = ref<string | null>(null);

  // 核心：记录每个会话（itemId + topicId）是否处于活动流状态
  // 格式: "itemId:topicId" -> [messageId1, messageId2, ...]
  const sessionActiveStreams = ref<Record<string, string[]>>({});

  // 兼容旧逻辑的计算属性 (修正：返回 Set 以保持兼容性，但内部依赖数组触发更新)
  const activeStreamingIds = computed(() => {
    if (!currentSelectedItem.value?.id || !currentTopicId.value)
      return new Set<string>();
    const key = `${currentSelectedItem.value.id}:${currentTopicId.value}`;
    return new Set(sessionActiveStreams.value[key] || []);
  });

  const isGroupGenerating = computed(() => {
    if (
      !currentSelectedItem.value?.id ||
      !currentTopicId.value ||
      currentSelectedItem.value.type !== "group"
    )
      return false;
    const key = `${currentSelectedItem.value.id}:${currentTopicId.value}`;
    const streams = sessionActiveStreams.value[key];
    return streams ? streams.length > 0 : false;
  });

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
  };

  const removeSessionStream = (
    ownerId: string,
    topicId: string,
    messageId: string,
  ) => {
    const key = `${ownerId}:${topicId}`;
    const streams = sessionActiveStreams.value[key];
    if (streams) {
      const index = streams.indexOf(messageId);
      if (index !== -1) {
        streams.splice(index, 1);
      }
      if (streams.length === 0) {
        delete sessionActiveStreams.value[key];
      }
    }
  };

  // 非当前视图流的轻量级内容缓存 (messageId -> { content: string, topicId: string, ownerId: string })
  const backgroundStreamingBuffers = ref<
    Map<string, { content: string; topicId: string; ownerId: string }>
  >(new Map());

  const foregroundStreamingBuffers = new Map<string, string>();

  // 暂存的附件列表，准备随下一条消息发送
  const stagedAttachments = ref<Attachment[]>([]);

  const streamManager = useStreamManagerStore();
  const settingsStore = useSettingsStore();
  const assistantStore = useAssistantStore();
  const topicStore = useTopicStore();
  const notificationStore = useNotificationStore();
  const diagnostics = usePerformanceDiagnostics();

  // 用于拦截重新生成时的输入框补全
  const editMessageContent = ref("");

  let listenersRegistered = false;

  const topicHistoryCache = new RuntimeLruCache<string, TopicHistoryCacheEntry>({
    maxEntries: 5,
    maxBytes: 28 * 1024 * 1024,
    getSize: (entry) => entry.estimatedBytes,
  });

  const rawContentCache = new RuntimeLruCache<string, string>({
    maxEntries: 250,
    maxBytes: 12 * 1024 * 1024,
    getSize: (value) => estimateStringBytes(value),
  });

  const topicCacheKey = (ownerId: string, ownerType: string, topicId: string) =>
    `${ownerType}:${ownerId}:${topicId}`;

  const estimateMessagesBytes = (messages: ChatMessage[]) =>
    messages.reduce((total, message) => {
      const scalarBytes =
        estimateStringBytes(message.id) +
        estimateStringBytes(message.role) +
        estimateStringBytes(message.name || "") +
        estimateStringBytes(message.agentId || "") +
        estimateStringBytes(message.groupId || "") +
        estimateStringBytes(message.content || "") +
        estimateStringBytes(message.displayedContent || "") +
        estimateStringBytes(message.stableContent || "") +
        estimateStringBytes(message.tailContent || "");
      return (
        total +
        scalarBytes +
        estimateJsonBytes(message.blocks || null) +
        estimateJsonBytes(message.attachments || null) +
        256
      );
    }, 0);

  const rememberRawContent = (message: ChatMessage) => {
    if (message.id && message.content) {
      rawContentCache.set(message.id, message.content);
    }
  };

  const forgetRawContent = (messageId: string) => {
    rawContentCache.delete(messageId);
  };

  const rememberRawContents = (messages: ChatMessage[]) => {
    messages.forEach(rememberRawContent);
  };

  const putTopicHistoryCache = (
    ownerId: string,
    ownerType: string,
    topicId: string,
    messages: ChatMessage[],
  ) => {
    topicHistoryCache.set(topicCacheKey(ownerId, ownerType, topicId), {
      ownerId,
      ownerType,
      topicId,
      messages,
      loadedAt: Date.now(),
      estimatedBytes: estimateMessagesBytes(messages),
    });
  };

  const getTopicHistoryCache = (
    ownerId: string,
    ownerType: string,
    topicId: string,
  ) => topicHistoryCache.get(topicCacheKey(ownerId, ownerType, topicId));

  const invalidateTopicHistoryCache = (
    ownerId: string,
    ownerType: string,
    topicId: string,
  ) => {
    topicHistoryCache.delete(topicCacheKey(ownerId, ownerType, topicId));
  };

  const refreshCurrentTopicCache = () => {
    if (!currentSelectedItem.value?.id || !currentTopicId.value) return;
    putTopicHistoryCache(
      currentSelectedItem.value.id,
      currentSelectedItem.value.type,
      currentTopicId.value,
      currentChatHistory.value,
    );
  };

  const upsertCachedTopicMessage = (
    ownerId: string,
    ownerType: string,
    topicId: string,
    message: ChatMessage,
  ) => {
    const entry = getTopicHistoryCache(ownerId, ownerType, topicId);
    if (!entry) return;

    const index = entry.messages.findIndex((item) => item.id === message.id);
    if (index >= 0) {
      entry.messages[index] = message;
    } else {
      entry.messages.push(message);
      entry.messages.sort((a, b) => a.timestamp - b.timestamp);
    }
    rememberRawContent(message);
    putTopicHistoryCache(ownerId, ownerType, topicId, entry.messages);
  };

  const removeCachedMessagesAfter = (timestamp: number) => {
    const removed = currentChatHistory.value.filter(
      (message) => message.timestamp >= timestamp,
    );
    removed.forEach((message) => forgetRawContent(message.id));
  };

  const invalidateRuntimeCaches = () => {
    topicHistoryCache.clear();
    rawContentCache.clear();
    clearMarkdownRenderCache();
    clearAttachmentThumbnailCache();
  };

  /**
   * 尝试为话题生成 AI 总结标题 (对齐桌面端 attemptTopicSummarization)
   */
  const summarizeTopic = async () => {
    if (!currentTopicId.value || !currentSelectedItem.value?.id) return;

    const topicId = currentTopicId.value;
    const ownerId = currentSelectedItem.value.id;
    const ownerType = currentSelectedItem.value.type;

    // 只有“未命名”话题且消息数达到阈值才总结 (桌面端策略)
    const topic = topicStore.topics.find((t) => t.id === topicId);
    const isUnnamed =
      !topic ||
      topic.name.includes("新话题") ||
      topic.name.includes("topic_") ||
      topic.name.includes("group_topic_") ||
      topic.name === "主要群聊";
    const messageCount = currentChatHistory.value.filter(
      (m) => m.role !== "system",
    ).length;

    if (isUnnamed && messageCount >= 4) {
      console.log(`[ChatManager] Triggering AI summary for topic: ${topicId}`);
      try {
        const agentName =
          assistantStore.agents.find((a: any) => a.id === ownerId)?.name ||
          "AI";
        const newTitle = await invoke<string>("summarize_topic", {
          ownerId,
          ownerType,
          topicId,
          agentName,
        });

        if (newTitle) {
          console.log(`[ChatManager] AI Summarized Title: ${newTitle}`);
          await topicStore.updateTopicTitle(
            ownerId,
            ownerType,
            topicId,
            newTitle,
          );
        }
      } catch (e) {
        console.error("[ChatManager] AI Summary failed:", e);
      }
    }
  };

  /**
   * 处理消息中的本地资源路径 (仅附件)，使用 Tauri 原生 asset:// 协议绕过 WebView 限制
   */
  const resolveMessageAssets = (msg: ChatMessage) => {
    // 处理附件 (仅处理图片类型)
    if (msg.attachments && msg.attachments.length > 0) {
      msg.attachments.forEach((att) => {
        // Rust 后端返回的路径现在主要在 internalPath，如果不在，回退到 src
        const sourcePath = att.internalPath || att.src;
        if (
          att.type.startsWith("image/") &&
          sourcePath &&
          !sourcePath.startsWith("http") &&
          !sourcePath.startsWith("data:")
        ) {
          try {
            att.resolvedSrc = convertFileSrc(sourcePath);
          } catch (err) {
            console.warn(
              `[ChatManager] Failed to convert attachment image path ${att.name}:`,
              err,
            );
          }
        }
      });
    }
  };

  const LARGE_FILE_THRESHOLD_BYTES = 2 * 1024 * 1024;
  const UPLOAD_CHUNK_BYTES = 1024 * 1024;

  const formatUploadSize = (bytes: number) => {
    const units = ["B", "KB", "MB", "GB"];
    let value = bytes;
    let unitIndex = 0;
    while (value >= 1024 && unitIndex < units.length - 1) {
      value /= 1024;
      unitIndex += 1;
    }
    return `${value.toFixed(unitIndex === 0 ? 0 : 1)} ${units[unitIndex]}`;
  };

  const updateStagedUploadProgress = (stableId: string, progress: number) => {
    const index = stagedAttachments.value.findIndex((a) => a.id === stableId);
    if (index !== -1) {
      stagedAttachments.value[index].progress = progress;
    }
  };

  const saveLargeFileByChunks = async (file: File, stableId: string) => {
    let sessionId: string | null = null;
    try {
      sessionId = await invoke<string>("init_chunked_upload", {
        originalName: file.name,
        mimeType: file.type || "application/octet-stream",
      });

      let offset = 0;
      while (offset < file.size) {
        const chunk = file.slice(offset, offset + UPLOAD_CHUNK_BYTES);
        const bytes = new Uint8Array(await chunk.arrayBuffer());
        await invoke("append_chunk", {
          sessionId,
          chunkBytes: bytes,
        });

        offset += chunk.size;
        updateStagedUploadProgress(
          stableId,
          Math.min(99, Math.round((offset / file.size) * 100)),
        );
      }

      return await invoke<any>("finish_chunked_upload", { sessionId });
    } catch (error) {
      if (sessionId) {
        await invoke("cancel_chunked_upload", { sessionId }).catch(
          (cancelError) => {
            console.warn(
              "[ChatManager] Failed to cancel chunked upload:",
              cancelError,
            );
          },
        );
      }
      throw error;
    }
  };

  const applyStoredAttachmentData = (stableId: string, finalData: any) => {
    const index = stagedAttachments.value.findIndex((a) => a.id === stableId);
    if (index === -1) return;

    stagedAttachments.value[index] = {
      ...stagedAttachments.value[index],
      type: finalData.type,
      src: finalData.internalPath,
      name: finalData.name,
      size: finalData.size,
      hash: finalData.hash,
      internalPath: finalData.internalPath,
      extractedText: finalData.extractedText,
      thumbnailPath: finalData.thumbnailPath,
      createdAt: finalData.createdAt,
      progress: 100,
      status: "done",
    };
  };

  /**
   * 触发文件选择器并暂存附件 (使用标准 HTML Input 完美解决 Android content:// 协议名和类型丢失问题)
   */
  const handleAttachment = async () => {
    return new Promise<void>((resolve, reject) => {
      const input = document.createElement("input");
      input.type = "file";
      input.multiple = false;
      // 允许所有类型
      input.accept = "*/*";

      input.onchange = async (e: Event) => {
        try {
          const target = e.target as HTMLInputElement;
          if (!target.files || target.files.length === 0) {
            resolve();
            return;
          }

          const file = target.files[0];
          console.log(
            `[ChatManager] Selected file via HTML input: ${file.name}, type: ${file.type}, size: ${file.size}`,
          );

          // 1. 生成稳定 ID 并使用 unshift 插入首位 (实现“最新附件最先看到”)
          const stableId = `att_${Date.now()}_${Math.random().toString(36).slice(2, 7)}`;
          const blobUrl = URL.createObjectURL(file);

          stagedAttachments.value.unshift({
            id: stableId,
            type: file.type || "application/octet-stream",
            src: blobUrl,
            name: file.name,
            size: file.size,
            status: "loading",
          });

          await nextTick();
          window.dispatchEvent(new Event("resize"));

          try {
            let finalData: any = null;

            // --- 分流策略：小文件走单次 IPC，大文件走 IPC 分片，避免移动端 localhost/XHR 链路不稳定 ---
            if (file.size < LARGE_FILE_THRESHOLD_BYTES) {
              console.log(
                `[ChatManager] Small file detected (<2MB), using store_file IPC for ${file.name}`,
              );
              // 将 File 转换为 Uint8Array (Tauri v2 支持直接传递二进制，严禁使用 Array.from)
              const arrayBuffer = await file.arrayBuffer();
              const bytes = new Uint8Array(arrayBuffer);

              finalData = await invoke<any>("store_file", {
                originalName: file.name,
                fileBytes: bytes, // 直接传递，性能提升 100 倍
                mimeType: file.type || "application/octet-stream",
              });
            } else {
              console.log(
                `[ChatManager] Large file detected, using chunked IPC upload for ${file.name} (${file.size} bytes)`,
              );
              finalData = await saveLargeFileByChunks(file, stableId);
            }

            if (finalData) {
              applyStoredAttachmentData(stableId, finalData);
            }
            resolve();
          } catch (err) {
            console.error("[ChatManager] Attachment upload failed:", err);
            const index = stagedAttachments.value.findIndex(
              (a) => a.id === stableId,
            );
            if (index !== -1) stagedAttachments.value.splice(index, 1);
            notificationStore.addNotification({
              type: "error",
              title: "附件上传失败",
              message: `${file.name} (${formatUploadSize(file.size)}): ${
                err instanceof Error ? err.message : String(err)
              }`,
              toastOnly: true,
            });
            reject(err);
          } finally {
            URL.revokeObjectURL(blobUrl);
          }
          resolve();
        } catch (err) {
          console.error(
            "[ChatManager] Failed to pick or store attachment:",
            err,
          );
          reject(err);
        }
      };

      input.oncancel = () => {
        resolve();
      };

      input.click();
    });
  };

  /**
   * 加载聊天历史 (已优化：使用私有协议直连，搭载预编译 AST 且防 OOM)
   */
  const loadHistory = async (
    ownerId: string,
    ownerType: string,
    topicId: string,
    limit?: number,
    offset: number = 0,
    forceRefresh: boolean = false,
  ) => {
    console.log(
      `[ChatManager] Loading history via VCP Protocol for ${ownerId}, topic: ${topicId}`,
    );
    loading.value = true;
    try {
      if (!forceRefresh && offset === 0 && typeof limit !== "number") {
        const cached = getTopicHistoryCache(ownerId, ownerType, topicId);
        if (cached) {
          currentChatHistory.value = cached.messages;
          currentTopicId.value = topicId;

          if (
            !currentSelectedItem.value ||
            currentSelectedItem.value.id !== ownerId
          ) {
            currentSelectedItem.value = { id: ownerId, type: ownerType };
          }

          rememberRawContents(cached.messages);
          diagnostics.addTrace("topic-history-cache-hit", {
            messageId: topicId,
            totalChars: cached.estimatedBytes,
            detail: `messages=${cached.messages.length}`,
          });
          console.log(
            `[ChatManager] History cache hit: ${cached.messages.length} messages for ${topicId}`,
          );
          return;
        }
      }

      // 使用标准 Tauri IPC 加载历史记录，解决自定义协议在 Android 上的兼容性问题
      const loadPayload: Record<string, unknown> = {
        ownerId,
        ownerType,
        topicId,
        offset,
      };
      if (typeof limit === "number") {
        loadPayload.limit = limit;
      }

      const history = await invoke<ChatMessage[]>("load_chat_history", loadPayload);

      if (offset === 0) {
        currentChatHistory.value = history;

        // 恢复后台缓存中的流式内容 (如果用户切回了正在流式的话题)
        backgroundStreamingBuffers.value.forEach((buffer, msgId) => {
          if (buffer.topicId === topicId) {
            const msg = currentChatHistory.value.find((m) => m.id === msgId);
            if (msg) {
              console.log(
                `[ChatManager] Restoring background buffer for ${msgId} into current view.`,
              );
              msg.content += buffer.content;
              msg.displayedContent = msg.content;
              msg.stableContent = msg.content;
              msg.tailContent = "";
              msg.isThinking = false;
              streamManager.seedStream(msgId, msg.content || buffer.content);
            }
          }
        });
      } else {
        currentChatHistory.value = [...history, ...currentChatHistory.value];
      }

      currentTopicId.value = topicId;

      if (
        !currentSelectedItem.value ||
        currentSelectedItem.value.id !== ownerId
      ) {
        currentSelectedItem.value = { id: ownerId, type: ownerType };
      }

      // 异步解析本地资源路径
      await Promise.all(
        history.map(async (msg) => {
          resolveMessageAssets(msg);
        }),
      );

      rememberRawContents(currentChatHistory.value);
      if (offset === 0 && typeof limit !== "number") {
        putTopicHistoryCache(
          ownerId,
          ownerType,
          topicId,
          currentChatHistory.value,
        );
      }

      console.log(
        `[ChatManager] History loaded: ${history.length} messages (Pre-processed by Rust)`,
      );
    } catch (e) {
      console.error("[ChatManager] Failed to load history:", e);
    } finally {
      loading.value = false;
    }
  };

  const loadHistoryPaginated = async (
    ownerId: string,
    ownerType: string,
    topicId: string,
  ) => {
    await loadHistory(ownerId, ownerType, topicId);
  };

  const selectItem = async (item: any) => {
    if (!item?.id) return;

    const ownerType =
      item.type ||
      (assistantStore.groups.some((group: any) => group.id === item.id)
        ? "group"
        : "agent");

    currentSelectedItem.value = {
      ...item,
      type: ownerType,
    };
    currentTopicId.value = null;
    currentChatHistory.value = [];

    await topicStore.loadTopicList(item.id, ownerType);
  };

  /**
   * 更新某条消息的内容（用于全屏编辑消息）
   */
  const updateMessageContent = async (
    messageId: string,
    newContent: string,
  ) => {
    const msg = currentChatHistory.value.find((m) => m.id === messageId);
    if (!msg) return;

    msg.content = newContent;
    // 重置预编译和显示缓存，强制触发 MessageRenderer 重新请求 Rust 解析
    msg.blocks = undefined;
    msg.processedContent = undefined;
    msg.stableContent = undefined;
    msg.tailContent = undefined;
    if (msg.displayedContent) {
      msg.displayedContent = "";
    }
    forgetRawContent(messageId);
    rememberRawContent(msg);
    clearMarkdownRenderCache();

    if (currentSelectedItem.value?.id && currentTopicId.value) {
      await invoke("patch_single_message", {
        ownerId: currentSelectedItem.value.id,
        ownerType: currentSelectedItem.value.type,
        topic_id: currentTopicId.value,
        message: msg,
      });

      notificationStore.addNotification({
        type: "success",
        title: "消息编辑已保存",
        message: "变更已同步至底层数据库",
        toastOnly: true,
      });

      // 触发同步到桌面端
      syncService.pushTopicToDesktop(
        currentSelectedItem.value.id,
        currentTopicId.value,
        currentChatHistory.value,
      );
      refreshCurrentTopicCache();
    }
  };

  /**
   * 发送消息
   */
  const sendMessage = async (content: string) => {
    if (
      !currentSelectedItem.value ||
      !currentTopicId.value ||
      (!content.trim() && stagedAttachments.value.length === 0)
    )
      return;

    const agentId = currentSelectedItem.value.id;
    const ownerType = currentSelectedItem.value.type;
    const isGroupChat = ownerType === "group";
    const currentStaged = [...stagedAttachments.value];

    // Clear staged area immediately for UI responsiveness
    stagedAttachments.value = [];

    // Document Processing JIT (Just-In-Time)
    if (currentStaged.length > 0) {
      const docProcessor = useDocumentProcessor();
      for (const att of currentStaged) {
        const ext = att.name.split(".").pop()?.toLowerCase();
        // Only process documents and PDFs as requested
        if (["txt", "md", "csv", "json", "docx", "pdf"].includes(ext || "")) {
          try {
            const result = await docProcessor.processAttachment(att);
            if (result) {
              if (result.extractedText)
                att.extractedText = result.extractedText;
              if (result.imageFrames) att.imageFrames = result.imageFrames;
            }
          } catch (e) {
            console.error(
              `[ChatManager] JIT document processing failed for ${att.name}:`,
              e,
            );
          }
        }
      }
    }

    // 构造用户消息
    const now = Date.now();
    const userMsg: ChatMessage = {
      id: `msg_${now}_user_${Math.random().toString(36).substring(2, 7)}`,
      role: "user",
      content,
      timestamp: now,
      attachments: currentStaged.length > 0 ? currentStaged : undefined,
    };

    currentChatHistory.value.push(userMsg);
    rememberRawContent(userMsg);
    refreshCurrentTopicCache();

    // 单 Agent 使用本地占位消息；群聊由 Rust 为每个发言成员生成真实消息 ID。
    const thinkingId = `msg_${now}_assistant_${Math.random().toString(36).substring(2, 7)}`;
    const thinkingMsg: ChatMessage | null = isGroupChat ? null : {
      id: thinkingId,
      role: "assistant",
      content: "",
      timestamp: now + 1, // 增加 1ms 偏移，确保在时间序列上绝对位于提问之后
      isThinking: true,
      isGroupMessage: false,
      agentId,
    };

    if (thinkingMsg) {
      currentChatHistory.value.push(thinkingMsg);
      streamingMessageId.value = thinkingId;
      addSessionStream(
        currentSelectedItem.value.id,
        currentTopicId.value,
        thinkingId,
      );
      refreshCurrentTopicCache();
    }

    try {
      // 立即保存用户消息；AI 回复在流结束后一次性落库，流式期间只保存在内存 buffer。
      if (currentSelectedItem.value?.id && currentTopicId.value) {
        await invoke("append_single_message", {
          ownerId: currentSelectedItem.value.id,
          ownerType,
          topicId: currentTopicId.value,
          message: userMsg,
        });

        // 触发同步到桌面端
        const persistedHistorySnapshot = thinkingMsg
          ? currentChatHistory.value.filter((message) => message.id !== thinkingId)
          : currentChatHistory.value;
        syncService.pushTopicToDesktop(
          currentSelectedItem.value.id,
          currentTopicId.value,
          persistedHistorySnapshot,
        );
      }

      const settings = settingsStore.settings;
      if (!settings) {
        throw new Error("应用尚未完成初始化，缺少设置数据，无法发送消息");
      }

      const vcpUrl = settings.vcpServerUrl || "";
      const vcpApiKey = settings.vcpApiKey || "";

      // --- 群组消息路由 ---
      if (isGroupChat) {
        const groupId = currentSelectedItem.value.id;

        const groupPayload = {
          groupId,
          topicId: currentTopicId.value,
          userMessage: userMsg,
          vcpUrl,
          vcpApiKey,
        };

        console.log("[ChatManager] Sending group payload:", groupPayload);
        // 直接调用 Rust 端群组调度器，不再设置前端硬超时
        await invoke("handle_group_chat_message", { payload: groupPayload });

        return;
      }

      // --- 普通单 Agent 消息逻辑 ---
      const agentPayload = {
        agentId,
        topicId: currentTopicId.value,
        userMessage: userMsg,
        vcpUrl,
        vcpApiKey,
        thinkingMessageId: thinkingId,
      };

      console.log("[ChatManager] Sending agent payload:", agentPayload);
      await invoke("handle_agent_chat_message", { payload: agentPayload });
    } catch (e) {
      console.error("[ChatManager] Failed to send message:", e);

      const errorText = `\n\n> VCP错误: ${e instanceof Error ? e.message : String(e)}`;

      const msgIndex = currentChatHistory.value.findIndex(
        (m) => m.id === thinkingId,
      );
      if (msgIndex !== -1) {
        const msg = currentChatHistory.value[msgIndex];
        msg.isThinking = false;
        msg.content = (msg.content || "") + errorText;
        if (msg.displayedContent !== undefined) {
          msg.displayedContent = (msg.displayedContent || "") + errorText;
        }
        if (currentSelectedItem.value?.id && currentTopicId.value) {
          removeSessionStream(
            currentSelectedItem.value.id,
            currentTopicId.value,
            thinkingId,
          );
        }
      } else {
        // Fallback if message was somehow lost
        currentChatHistory.value.push({
          id: `msg_${Date.now()}_system_error`,
          role: "system",
          content: errorText.trim(),
          timestamp: Date.now(),
        });
      }

      streamingMessageId.value = null;
      if (currentSelectedItem.value?.id && currentTopicId.value) {
        // 查找当前的思考占位消息
        const msg = currentChatHistory.value.find((m) => m.id === thinkingId);
          if (msg) {
            await invoke("append_single_message", {
              ownerId: currentSelectedItem.value.id,
              ownerType,
              topicId: currentTopicId.value,
              message: msg,
            });
            rememberRawContent(msg);
            refreshCurrentTopicCache();
            syncService.pushTopicToDesktop(
              currentSelectedItem.value.id,
              currentTopicId.value,
              currentChatHistory.value,
            );
        }
      }
    }
  };

  /**
   * 删除指定消息及之后的所有消息 (通常用于重新生成或回退)
   * 如果 deleteAfter 为 true，则相当于时间回溯
   */
  const deleteMessage = async (
    messageId: string,
    deleteAfter: boolean = false,
  ) => {
    if (!currentSelectedItem.value || !currentTopicId.value) return;

    const ownerId = currentSelectedItem.value.id;
    const ownerType = currentSelectedItem.value.type;
    const topicId = currentTopicId.value;
    const targetIndex = currentChatHistory.value.findIndex(
      (m) => m.id === messageId,
    );
    if (targetIndex === -1) return;

    const targetMsg = currentChatHistory.value[targetIndex];

    if (deleteAfter) {
      removeCachedMessagesAfter(targetMsg.timestamp);
      // 物理截断：删除自身以及后面所有的
      await invoke("truncate_history_after_timestamp", {
        ownerId,
        ownerType,
        topicId,
        timestamp: targetMsg.timestamp - 1, // 包含自身
      });
      currentChatHistory.value.splice(targetIndex);
    } else {
      forgetRawContent(messageId);
      // 逻辑删除：仅删除自身
      await invoke("delete_messages", {
        ownerId,
        ownerType,
        topicId,
        msgIds: [messageId],
      });
      currentChatHistory.value.splice(targetIndex, 1);
    }
    clearMarkdownRenderCache();
    refreshCurrentTopicCache();

    // 触发同步到桌面端
    syncService.pushTopicToDesktop(ownerId, topicId, currentChatHistory.value);

    notificationStore.addNotification({
      type: "success",
      title: "消息已删除",
      message: "该条消息已从本地库移除",
      toastOnly: true,
    });
  };

  /**
   * 强行中止整个群组的接力赛回合
   */
  const stopGroupTurn = async (topicId: string) => {
    console.log(`[ChatManager] Global Group Interruption for topic: ${topicId}`);
    try {
      // 1. 发射后端熔断信号，打断 for 循环接力
      await invoke("interruptGroupTurn", { topic_id: topicId });
      
      // 2. 同时中止当前活跃的所有流消息（确保当前正在说话的 Agent 也立即停下）
      const activeIds = Array.from(activeStreamingIds.value);
      if (activeIds.length > 0) {
        await Promise.all(activeIds.map(id => stopMessage(id)));
      }
    } catch (e) {
      console.error("[ChatManager] Failed to stop group turn:", e);
    }
  };

  /**
   * 中止指定消息的生成
   */
  const stopMessage = async (messageId: string) => {
    console.log(
      `[ChatManager] Sending interrupt signal for message: ${messageId}`,
    );
    try {
      await invoke("interruptRequest", { message_id: messageId });
      // 本地伪造一个 end 事件，防止假死
      streamManager.finalizeStream(messageId);

      // 确保清理状态
      const msgIndex = currentChatHistory.value.findIndex(
        (m) => m.id === messageId,
      );
      let msg: ChatMessage | null = null;
      if (msgIndex !== -1) {
        msg = currentChatHistory.value[msgIndex];
        msg.isThinking = false;
        msg.content =
          foregroundStreamingBuffers.get(messageId) ||
          msg.content ||
          msg.displayedContent ||
          "";
        msg.displayedContent = msg.content;
        msg.stableContent = undefined;
        msg.tailContent = undefined;
        foregroundStreamingBuffers.delete(messageId);
      }

      const ownerId = currentSelectedItem.value?.id;
      const topicId = currentTopicId.value;

      if (ownerId && topicId) {
        removeSessionStream(ownerId, topicId, messageId);
      }

      if (streamingMessageId.value === messageId) {
        streamingMessageId.value = null;
      }
      // 增量保存当前中止后的内容
      if (msg && ownerId && topicId) {
        await invoke("append_single_message", {
          ownerId,
          ownerType: currentSelectedItem.value!.type,
          topicId,
          message: msg,
        });
        rememberRawContent(msg);
        refreshCurrentTopicCache();
        // 触发同步到桌面端
        syncService.pushTopicToDesktop(
          ownerId,
          topicId,
          currentChatHistory.value,
        );
      }
    } catch (e) {
      console.error(
        `[ChatManager] Failed to interrupt stream for ${messageId}:`,
        e,
      );
    }
  };
  
  /**
   * 重新生成消息
   * @param targetMessageId 用户想要重新生成的 AI 回复的 ID
   */
  const regenerateResponse = async (targetMessageId: string) => {
    if (!currentSelectedItem.value || !currentTopicId.value) return;

    const targetIndex = currentChatHistory.value.findIndex(
      (message) => message.id === targetMessageId,
    );
    if (targetIndex === -1) return;

    const sourceUserMessage = [...currentChatHistory.value]
      .slice(0, targetIndex)
      .reverse()
      .find((message) => message.role === "user");

    if (!sourceUserMessage) {
      notificationStore.addNotification({
        type: "warning",
        title: "无法重新生成",
        message: "没有找到这条 AI 回复对应的上一条用户消息。",
        toastOnly: true,
      });
      return;
    }

    const ownerId = currentSelectedItem.value.id;
    const ownerType = currentSelectedItem.value.type;
    const topicId = currentTopicId.value;
    const now = Date.now();
    const thinkingId = `msg_${now}_assistant_${Math.random().toString(36).substring(2, 7)}`;

    await invoke("truncate_history_after_timestamp", {
      ownerId,
      ownerType,
      topicId,
      timestamp: currentChatHistory.value[targetIndex].timestamp - 1,
    });
    removeCachedMessagesAfter(currentChatHistory.value[targetIndex].timestamp);
    currentChatHistory.value.splice(targetIndex);
    clearMarkdownRenderCache();
    refreshCurrentTopicCache();

    const thinkingMsg: ChatMessage | null = ownerType === "group" ? null : {
      id: thinkingId,
      role: "assistant",
      content: "",
      timestamp: now + 1,
      isThinking: true,
      isGroupMessage: false,
      agentId: ownerId,
    };

    if (thinkingMsg) {
      currentChatHistory.value.push(thinkingMsg);
      streamingMessageId.value = thinkingId;
      addSessionStream(ownerId, topicId, thinkingId);
      refreshCurrentTopicCache();
    }

    try {
      const settings = settingsStore.settings;
      if (!settings) {
        throw new Error("应用尚未完成初始化，缺少设置数据，无法重新生成");
      }

      const vcpUrl = settings.vcpServerUrl || "";
      const vcpApiKey = settings.vcpApiKey || "";

      if (ownerType === "group") {
        await invoke("handle_group_chat_message", {
          payload: {
            groupId: ownerId,
            topicId,
            userMessage: sourceUserMessage,
            vcpUrl,
            vcpApiKey,
          },
        });
      } else {
        await invoke("handle_agent_chat_message", {
          payload: {
            agentId: ownerId,
            topicId,
            userMessage: sourceUserMessage,
            vcpUrl,
            vcpApiKey,
            thinkingMessageId: thinkingId,
          },
        });
      }

      syncService.pushTopicToDesktop(ownerId, topicId, currentChatHistory.value);
    } catch (error) {
      console.error("[ChatManager] Failed to regenerate response:", error);
      removeSessionStream(ownerId, topicId, thinkingId);
      if (streamingMessageId.value === thinkingId) {
        streamingMessageId.value = null;
      }
      const failedIndex = currentChatHistory.value.findIndex((message) => message.id === thinkingId);
      if (failedIndex !== -1) {
        currentChatHistory.value[failedIndex].content = `> 重新生成失败: ${error}`;
        currentChatHistory.value[failedIndex].displayedContent = currentChatHistory.value[failedIndex].content;
        await invoke("append_single_message", {
          ownerId,
          ownerType,
          topicId,
          message: currentChatHistory.value[failedIndex],
        });
        rememberRawContent(currentChatHistory.value[failedIndex]);
        refreshCurrentTopicCache();
      }
      notificationStore.addNotification({
        type: "error",
        title: "重新生成失败",
        message: String(error),
        toastOnly: true,
      });
    }
  };

  /**
   * 按需拉取单条消息的原始 Markdown 内容
   */
  const fetchRawContent = async (messageId: string): Promise<string> => {
    // 检查缓存中是否已有，或者是否正在流式传输
    const cachedRaw = rawContentCache.get(messageId);
    if (cachedRaw) return cachedRaw;

    const existingMsg = currentChatHistory.value.find((m) => m.id === messageId);
    if (existingMsg && existingMsg.content) {
      rememberRawContent(existingMsg);
      return existingMsg.content;
    }

    try {
      const url = `vcp://api/messages?msg_id=${encodeURIComponent(messageId)}&fetch_raw=true`;
      console.log(`[ChatManager] Lazy loading raw content for message: ${messageId}`);
      const rawText = await new Promise<string>((resolve, reject) => {
        const xhr = new XMLHttpRequest();
        xhr.open('GET', url, true);
        xhr.responseType = 'text';
        
        xhr.onload = () => {
          if (xhr.status >= 200 && xhr.status < 300) {
            resolve(xhr.responseText || "");
          } else {
            reject(new Error(`VCP fetch_raw request failed with status ${xhr.status}`));
          }
        };
        
        xhr.onerror = () => {
          reject(new Error('VCP Network Error during fetch_raw'));
        };
        
        xhr.send();
      });

      if (rawText) {
        rawContentCache.set(messageId, rawText);
      }
      if (existingMsg) {
        existingMsg.content = rawText;
      }
      return rawText;
    } catch (e) {
      console.error(`[ChatManager] Failed to fetch raw content for ${messageId}:`, e);
      return "";
    }
  };

  // --- 初始化与销毁 (Lifecycle) ---

  const ensureEventListenersRegistered = async () => {
    if (listenersRegistered) return;
    listenersRegistered = true;
    console.log("[ChatManager] Registering Tauri listeners");

    // 监听 AI 流式输出事件
    listen("vcp-stream", (event: any) => {
      // 适配 Rust 端默认序列化使用下划线命名法 (message_id)
      const {
        message_id,
        messageId: legacyMessageId,
        chunk,
        type,
        context,
      } = event.payload;
      const actualMessageId = message_id || legacyMessageId;
      // 无论是否在当前视图，都尝试更新数据
      let msg = currentChatHistory.value.find((m) => m.id === actualMessageId);
      const ctx = context || {};
      const topicId = ctx.topicId || currentTopicId.value;
      const itemId =
        ctx.agentId || ctx.groupId || currentSelectedItem.value?.id;

      // [关键修复] 如果是群聊并行流，且消息尚未在 currentChatHistory 中（因为是 Rust 端刚发起的），
      // 我们需要根据 context 自动创建一个占位消息，以便立即展示流式内容。
      if (
        !msg &&
        context &&
        context.isGroupMessage &&
        context.groupId === currentSelectedItem.value?.id
      ) {
        console.log(
          `[ChatManager] Creating placeholder for group message: ${actualMessageId}`,
        );
        msg = {
          id: actualMessageId,
          role: "assistant",
          name: context.agentName,
          content: "",
          timestamp: Date.now(),
          isThinking: false,
          agentId: context.agentId,
          groupId: context.groupId,
          isGroupMessage: true,
        };
        currentChatHistory.value.push(msg as ChatMessage);
        // 保持排序
        currentChatHistory.value.sort((a, b) => a.timestamp - b.timestamp);
        refreshCurrentTopicCache();
      }

      if (type === "data") {
        if (msg) {
          msg.isThinking = false;
        }

        // 确保在活动流集合中
        if (itemId && topicId) {
          addSessionStream(itemId, topicId, actualMessageId);
        }

        // 解析 chunk 提取文本内容
        let textChunk = "";
        if (typeof chunk === "string") {
          textChunk = chunk;
        } else if (chunk && chunk.choices && chunk.choices.length > 0) {
          const delta = chunk.choices[0].delta;
          if (delta && delta.content) {
            textChunk = delta.content;
          }
        }

        if (textChunk) {
          diagnostics.addTrace('vcp-stream-data', {
            messageId: actualMessageId,
            chunkChars: textChunk.length,
            detail: msg ? 'foreground' : 'background',
          });
          // 1. 更新当前视图 (如果匹配)
          if (msg) {
            const currentContent = foregroundStreamingBuffers.get(actualMessageId) || msg.content || "";
            const nextContent = currentContent + textChunk;
            foregroundStreamingBuffers.set(actualMessageId, nextContent);
            void consumeDesktopPushBlocks(actualMessageId, nextContent);
            // 使用 streamManager 平滑更新 Aurora 分层内容；流式期间不写入 DB。

            streamManager.appendChunk(actualMessageId, textChunk, (state) => {
              const latestMsg = currentChatHistory.value.find(
                (m) => m.id === actualMessageId,
              );
              if (latestMsg) {
                latestMsg.stableContent = state.stable;
                latestMsg.tailContent = state.tail;
                latestMsg.displayedContent = state.displayed;
              }
            });
          } else {
            // 2. 更新后台缓存 (如果不在当前视图)
            if (topicId && itemId) {
              const buffer = backgroundStreamingBuffers.value.get(
                actualMessageId,
              ) || { content: "", topicId, ownerId: itemId };
              buffer.content += textChunk;
              void consumeDesktopPushBlocks(actualMessageId, buffer.content);
              backgroundStreamingBuffers.value.set(actualMessageId, buffer);
            }

          }
        }
      } else if (type === "end" || type === "error") {
        const errorMsg = event.payload.error;
        console.log(
          `[ChatManager] Stream ${type} for ${actualMessageId}${errorMsg ? ": " + errorMsg : ""}. Draining queue...`,
        );
        diagnostics.addTrace(type === "end" ? 'vcp-stream-end' : 'vcp-stream-error', {
          messageId: actualMessageId,
          detail: errorMsg || undefined,
        });

        // 流式结束时，等待 streamManager 缓冲队列排空后再切换状态
        streamManager.finalizeStream(actualMessageId, async () => {
          const latestMsg = currentChatHistory.value.find(
            (m) => m.id === actualMessageId,
          );
          if (latestMsg) {
            // 确保最终内容一致
            latestMsg.isThinking = false;
              latestMsg.content = foregroundStreamingBuffers.get(actualMessageId) || latestMsg.content || latestMsg.displayedContent || "";
              latestMsg.displayedContent = latestMsg.content;
              latestMsg.stableContent = undefined;
              latestMsg.tailContent = undefined;
              rememberRawContent(latestMsg);
            }
          foregroundStreamingBuffers.delete(actualMessageId);

          // 清理活动流状态
          if (itemId && topicId) {
            removeSessionStream(itemId, topicId, actualMessageId);
          }
          if (streamingMessageId.value === actualMessageId) {
            streamingMessageId.value = null;
          }

          if (type === "error" && errorMsg && errorMsg !== "请求已中止") {
            const errorText = `\n\n> VCP流式错误: ${errorMsg}`;
            if (latestMsg) {
              latestMsg.content = (latestMsg.content || "") + errorText;
              latestMsg.displayedContent = latestMsg.content;
              latestMsg.stableContent = undefined;
              latestMsg.tailContent = undefined;
            } else {
              // 如果不在当前视图，暂时不处理系统错误消息的追加，
              // 因为我们没有全局的消息持久化更新接口（本阶段目标是打通多流基础）
            }
          }

          if (latestMsg) {
            const ownerIdForPersist = currentSelectedItem.value?.id;
            const ownerTypeForPersist = currentSelectedItem.value?.type;
            const topicIdForPersist = currentTopicId.value;
            const shouldPersistFromFrontend =
              !latestMsg.isGroupMessage &&
              !!ownerIdForPersist &&
              !!ownerTypeForPersist &&
              !!topicIdForPersist;

            if (shouldPersistFromFrontend) {
              await invoke("append_single_message", {
                ownerId: ownerIdForPersist,
                ownerType: ownerTypeForPersist,
                topicId: topicIdForPersist,
                message: latestMsg,
              });
              syncService.pushTopicToDesktop(
                ownerIdForPersist,
                topicIdForPersist,
                currentChatHistory.value,
              );
              refreshCurrentTopicCache();
            }

            // [核心升级] 流式结束后，立即获取一次预编译的 AST 块并存在内存中
            // 这样在下次滚动或切换时，MessageRenderer 可以直接零耗时渲染
            try {
              const compileStartedAt = performance.now();
              const freshBlocks = await invoke<any[]>("process_message_content", {
                content: stripDesktopPushBlocks(latestMsg.content || "")
              });
              latestMsg.blocks = freshBlocks;
              refreshCurrentTopicCache();
              diagnostics.addTrace('post-stream-block-compile', {
                messageId: actualMessageId,
                totalChars: latestMsg.content?.length || 0,
                durationMs: Math.round((performance.now() - compileStartedAt) * 10) / 10,
                detail: `blocks=${freshBlocks.length}`,
              });
              console.log(`[ChatManager] Fresh blocks pre-compiled for ${actualMessageId}`);
            } catch (err) {
              console.warn("[ChatManager] Post-stream compilation failed:", err);
            }

            // 话题自动总结逻辑
            await summarizeTopic();
          } else {
            // 如果不在当前视图，从后台缓存中提取并尝试保存一次
            const buffer =
              backgroundStreamingBuffers.value.get(actualMessageId);
            if (buffer) {
              console.log(
                `[ChatManager] Finalizing background stream for ${actualMessageId}, triggering silent save.`,
              );
              const isGroupStream = !!ctx.groupId || !!ctx.isGroupMessage;
              if (!isGroupStream) {
                const errorText =
                  type === "error" && errorMsg && errorMsg !== "请求已中止"
                    ? `\n\n> VCP流式错误: ${errorMsg}`
                    : "";
                const backgroundMessage: ChatMessage = {
                  id: actualMessageId,
                  role: "assistant",
                  content: `${buffer.content}${errorText}`,
                  timestamp: Date.now(),
                  isThinking: false,
                  isGroupMessage: false,
                  agentId: buffer.ownerId,
                };
                await invoke("append_single_message", {
                  ownerId: buffer.ownerId,
                  ownerType: "agent",
                  topicId: buffer.topicId,
                  message: backgroundMessage,
                });
                upsertCachedTopicMessage(
                  buffer.ownerId,
                  "agent",
                  buffer.topicId,
                  backgroundMessage,
                );
              }
            }
          }

          // 彻底清理
          backgroundStreamingBuffers.value.delete(actualMessageId);
          resetDesktopPushConsumption(actualMessageId);
        });
      }
    });

    // 监听外部文件变更 (对应桌面端的 history-file-updated)
    listen("vcp-file-change", async (event: any) => {
      const paths = event.payload as string[];
      console.log("[ChatManager] File change detected by Rust Watcher:", paths);

      if (!currentTopicId.value || !currentSelectedItem.value?.id) return;

      // 检查变更的文件路径是否包含当前正在查看的 topicId
      const isCurrentTopicChanged = paths.some(
        (p) => p.includes(currentTopicId.value!) && p.endsWith("history.json"),
      );

      if (isCurrentTopicChanged) {
        console.log(
          `[ChatManager] Current topic ${currentTopicId.value} history changed externally. Syncing...`,
        );
        const ownerId = currentSelectedItem.value?.id;
        const ownerType = currentSelectedItem.value?.type;
        if (ownerId && ownerType) {
          invalidateTopicHistoryCache(ownerId, ownerType, currentTopicId.value!);
          await loadHistory(ownerId, ownerType, currentTopicId.value!, undefined, 0, true);
        }
      }
    });

    listen("vcp-group-turn-finished", async (event: any) => {
      const { groupId, topic_id, topicId: legacyTopicId } = event.payload;
      const actualTopicId = topic_id || legacyTopicId;

      if (
        groupId === currentSelectedItem.value?.id &&
        actualTopicId === currentTopicId.value
      ) {
        console.log(`[ChatManager] Group turn finished for ${groupId}`);
        delete sessionActiveStreams.value[`${groupId}:${actualTopicId}`];
        if (streamingMessageId.value) {
          streamingMessageId.value = null;
        }
        // 强制同步一次，确保所有并行 Agent 的最终结果都已落盘并同步到前端
        const ownerType = currentSelectedItem.value?.type;
        if (ownerType) {
          invalidateTopicHistoryCache(groupId, ownerType, actualTopicId);
          await loadHistory(groupId, ownerType, actualTopicId, undefined, 0, true);
        }
      }
    });
  };

  return {
    ensureEventListenersRegistered,
    currentChatHistory,
    currentSelectedItem,
    currentTopicId,
    loading,
    streamingMessageId,
    stagedAttachments,
    editMessageContent,
    sessionActiveStreams,
    loadHistory,
    loadHistoryPaginated,
    selectItem,
    fetchRawContent,
    sendMessage,
    handleAttachment,
    deleteMessage,
    stopMessage,
    stopGroupTurn,
    updateMessageContent,
    regenerateResponse,
    isGroupGenerating,
    activeStreamingIds,
    invalidateRuntimeCaches,
  };
});
