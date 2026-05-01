<script setup lang="ts">
import { computed, onUnmounted, ref, watch } from "vue";
import type { ChatMessage } from "../../core/stores/chatManager";
import { useAssistantStore } from "../../core/stores/assistant";
import { useSettingsStore } from "../../core/stores/settings";
import {
  useContentProcessor,
  type ContentBlock,
} from "../../core/composables/useContentProcessor";
import { useOverlayStore } from "../../core/stores/overlay";
import { useChatManagerStore } from "../../core/stores/chatManager";
import { useNotificationStore } from "../../core/stores/notification";
import { usePerformanceDiagnostics } from "../../core/utils/performanceDiagnostics";
import {
  TOOL_REQUEST_END,
  TOOL_REQUEST_START,
  TOOL_RESULT_END,
  TOOL_RESULT_START,
  extractToolNameFromRequest,
  parseToolResultBody,
} from "../../core/utils/toolPreview";
import { Copy, Edit2, RotateCcw, Trash2, StopCircle } from "lucide-vue-next";

// Import block components
import MarkdownBlock from "./blocks/MarkdownBlock.vue";
import ToolBlock from "./blocks/ToolBlock.vue";
import DiaryBlock from "./blocks/DiaryBlock.vue";
import ThoughtBlock from "./blocks/ThoughtBlock.vue";
import HtmlPreviewBlock from "./blocks/HtmlPreviewBlock.vue";
import RoleDividerBlock from "./blocks/RoleDividerBlock.vue";
import ChatBubble from "./components/ChatBubble.vue";
import MessageHeader from "./components/MessageHeader.vue";
import ThinkingIndicator from "./components/ThinkingIndicator.vue";
import StreamingTag from "./components/StreamingTag.vue";
import AttachmentPreview from "./attachment/AttachmentPreview.vue";

const props = defineProps<{
  message: ChatMessage;
  agentId?: string;
  depth?: number;
}>();

const assistantStore = useAssistantStore();
const settingsStore = useSettingsStore();
const { processMessageContent, removeScopedCss } = useContentProcessor();
const overlayStore = useOverlayStore();
const notificationStore = useNotificationStore();
const diagnostics = usePerformanceDiagnostics();

const chatStore = useChatManagerStore();

const isUser = computed(() => props.message.role === "user");
const isStreaming = computed(() => {
  if (isUser.value) return false;

  // 检查当前消息是否在所属会话的活动流中 (修正：不再依赖 isThinking 状态)
  const itemId =
    props.message.agentId || props.message.groupId || props.agentId;
  const topicId = chatStore.currentTopicId;
  if (!itemId || !topicId) return false;

  const key = `${itemId}:${topicId}`;
  const streams = chatStore.sessionActiveStreams?.[key];
  return streams ? streams.includes(props.message.id) : false;
});

// 获取当前消息实际对应的 Agent ID (对于群聊，从显式字段读取)
const actualAgentId = computed(() => {
  return props.message.agentId || props.agentId;
});

// 获取当前 Agent 的配置
const agentConfig = computed(() => {
  if (isUser.value) return null;

  // 1. 优先按 ID 查找
  if (actualAgentId.value) {
    const agent = assistantStore.agents.find(
      (a) => a.id === actualAgentId.value,
    );
    if (agent) return agent;
  }

  // 2. 针对群聊历史数据，可能只有名称没有 ID，尝试按名称查找
  if (props.message.name) {
    const agent = assistantStore.agents.find(
      (a) => a.name === props.message.name,
    );
    if (agent) return agent;
  }

  return null;
});

// 获取头像 URL
const resolvedAvatarUrl = computed(() => {
  if (isUser.value) return "vcp-avatar://user/user_avatar";

  // 优先使用匹配到的 Agent ID
  if (actualAgentId.value) {
    return `vcp-avatar://agent/${actualAgentId.value}`;
  }

  // 如果没有 ID 只有名称，尝试按名称匹配 (兼容旧数据)
  if (props.message.name) {
    const agent = assistantStore.agents.find(
      (a) => a.name === props.message.name,
    );
    if (agent) return `vcp-avatar://agent/${agent.id}`;
  }

  return null;
});

onUnmounted(() => {
  // 彻底防止 Scoped CSS 在组件销毁后泄漏内存或污染全局
  if (props.message && props.message.id) {
    removeScopedCss(props.message.id);
  }
});

// 响应式消息块 (AST 树)
const contentBlocks = ref<ContentBlock[]>([]);
// 流式传输专用原始文本
const streamContent = ref<string>("");

// 过渡状态：用于在流式结束、等待 Rust AST 解析完成前，保持流式视图不消失，防止闪烁
const isTransitioning = ref(false);

// 决定当前 UI 显示哪个视图：只要在流式中，或者正在过渡中，就显示流式纯文本视图
const showStreamView = computed(
  () => isStreaming.value || isTransitioning.value,
);

const hasLayeredStream = computed(
  () =>
    isStreaming.value &&
    (props.message.stableContent !== undefined ||
      props.message.tailContent !== undefined),
);

const messageRenderSource = computed(() => {
  if (hasLayeredStream.value) {
    return props.message.tailContent || "";
  }

  return (
    props.message.processedContent ||
    props.message.displayedContent ||
    props.message.content ||
    ""
  );
});

const streamTailContent = computed(() => {
  if (hasLayeredStream.value) {
    return props.message.tailContent || "";
  }

  return streamContent.value;
});

const hasVisibleStreamContent = computed(() => {
  if (!isStreaming.value) return false;
  return Boolean(
    props.message.stableContent ||
      props.message.tailContent ||
      streamContent.value,
  );
});

const normalizeThoughtContent = (content: string) => {
  return content.replace(/^\n+/, "").replace(/\n+$/, "");
};

const pushMarkdownBlock = (blocks: ContentBlock[], content: string) => {
  if (!content) return;
  blocks.push({
    type: "markdown",
    content,
  });
};

const findNextStreamMarker = (text: string, cursor: number) => {
  const candidates: Array<{
    index: number;
    end: number;
    type: "thought" | "think" | "tool-use" | "tool-result" | "role-divider";
    match?: RegExpExecArray;
  }> = [];

  const addRegexCandidate = (
    regex: RegExp,
    type: "thought" | "think" | "role-divider",
  ) => {
    regex.lastIndex = cursor;
    const match = regex.exec(text);
    if (!match) return;
    const linePrefix = match[1] || "";
    candidates.push({
      index: match.index + linePrefix.length,
      end: match.index + match[0].length,
      type,
      match,
    });
  };

  addRegexCandidate(
    /(^|\n)[ \t]*(\[--- VCP元思考链(?::\s*([^\]]*?))?\s*---\])/gim,
    "thought",
  );
  addRegexCandidate(/(^|\n)[ \t]*(<think(?:ing)?>)/gim, "think");
  addRegexCandidate(
    /(^|\n)[ \t]*(<<<\[(END_)?ROLE_DIVIDE_(SYSTEM|ASSISTANT|USER)\]>>>)/g,
    "role-divider",
  );

  const toolRequestStart = text.indexOf(TOOL_REQUEST_START, cursor);
  if (toolRequestStart !== -1) {
    candidates.push({
      index: toolRequestStart,
      end: toolRequestStart + TOOL_REQUEST_START.length,
      type: "tool-use",
    });
  }

  const toolResultStart = text.indexOf(TOOL_RESULT_START, cursor);
  if (toolResultStart !== -1) {
    candidates.push({
      index: toolResultStart,
      end: toolResultStart + TOOL_RESULT_START.length,
      type: "tool-result",
    });
  }

  return candidates.sort((a, b) => a.index - b.index)[0] || null;
};

const splitStreamSpecialBlocks = (text: string): ContentBlock[] => {
  if (!text) return [];

  const blocks: ContentBlock[] = [];
  let cursor = 0;

  while (cursor < text.length) {
    const nextMarker = findNextStreamMarker(text, cursor);
    if (!nextMarker) {
      pushMarkdownBlock(blocks, text.slice(cursor));
      break;
    }

    pushMarkdownBlock(blocks, text.slice(cursor, nextMarker.index));

    if (nextMarker.type === "role-divider") {
      const isEnd = Boolean(nextMarker.match?.[3]);
      const role = (nextMarker.match?.[4] || "").toLowerCase();
      blocks.push({
        type: "role-divider",
        content: "",
        role,
        is_end: isEnd,
      });
      cursor = nextMarker.end;
      continue;
    }

    if (nextMarker.type === "tool-use") {
      const contentStart = nextMarker.end;
      const endIndex = text.indexOf(TOOL_REQUEST_END, contentStart);
      const contentEnd = endIndex === -1 ? text.length : endIndex;
      const content = text.slice(contentStart, contentEnd);
      blocks.push({
        type: "tool-use",
        content,
        tool_name: extractToolNameFromRequest(content),
        is_complete: endIndex !== -1,
      });
      cursor = endIndex === -1 ? text.length : endIndex + TOOL_REQUEST_END.length;
      continue;
    }

    if (nextMarker.type === "tool-result") {
      const contentStart = nextMarker.end;
      const endIndex = text.indexOf(TOOL_RESULT_END, contentStart);
      if (endIndex === -1) {
        blocks.push({
          type: "tool-result",
          content: "",
          tool_name: "VCP 工具",
          status: "接收中",
          details: [],
          footer: "",
          is_complete: false,
        });
        cursor = text.length;
        continue;
      }

      const parsed = parseToolResultBody(text.slice(contentStart, endIndex));
      blocks.push({
        type: "tool-result",
        content: "",
        tool_name: parsed.toolName,
        status: parsed.status,
        details: parsed.details,
        footer: parsed.footer,
        is_complete: true,
      });
      cursor = endIndex + TOOL_RESULT_END.length;
      continue;
    }

    const isVcpThought = nextMarker.type === "thought";
    const contentStart = nextMarker.end;
    const endRegex = isVcpThought
      ? /(^|\n)[ \t]*\[--- 元思考链结束 ---\]/gim
      : /(^|\n)[ \t]*<\/think(?:ing)?>/gim;
    endRegex.lastIndex = contentStart;
    const endMatch = endRegex.exec(text);
    const endLinePrefix = endMatch?.[1] || "";
    const contentEnd = endMatch
      ? endMatch.index + endLinePrefix.length
      : text.length;
    const nextCursor = endMatch ? endMatch.index + endMatch[0].length : text.length;
    const rawTheme = nextMarker.match?.[3]?.trim().replace(/"/g, "");

    blocks.push({
      type: "thought",
      content: normalizeThoughtContent(text.slice(contentStart, contentEnd)),
      theme: isVcpThought ? rawTheme || "元思考链" : "思维链",
      is_complete: Boolean(endMatch),
    });

    cursor = nextCursor;
  }

  return blocks;
};

const stableStreamBlocks = computed(() =>
  splitStreamSpecialBlocks(props.message.stableContent || ""),
);

const tailStreamBlocks = computed(() =>
  splitStreamSpecialBlocks(streamTailContent.value || ""),
);

// 节流状态
let isProcessing = false;
let pendingRenderRequest: { text: string; streaming: boolean } | null = null;

// 核心解析逻辑
const updateContentBlocks = async (text: string) => {
  const renderStartedAt = performance.now();
  if (!text && isStreaming.value) {
    contentBlocks.value = [];
    streamContent.value = "";
    diagnostics.addTrace("message-render-empty-stream", {
      messageId: props.message.id,
      durationMs: Math.round((performance.now() - renderStartedAt) * 10) / 10,
    });
    return;
  }

  // 1. 优先使用预编译的 AST (零解析渲染)
  if (!isStreaming.value && props.message.blocks && props.message.blocks.length > 0) {
    contentBlocks.value = props.message.blocks;
    streamContent.value = "";
    diagnostics.addTrace("message-render-cached-blocks", {
      messageId: props.message.id,
      totalChars: text?.length || 0,
      durationMs: Math.round((performance.now() - renderStartedAt) * 10) / 10,
      detail: `blocks=${props.message.blocks.length}`,
    });
    return;
  }

  // 如果连文本都没有，且没有块，退出
  if (!text && !props.message.blocks) return;

  const options = {
    role: props.message.role,
    depth: props.depth || 0,
    messageId: props.message.id,
    isStreaming: isStreaming.value,
  };

  if (isStreaming.value) {
    streamContent.value = hasLayeredStream.value
      ? props.message.tailContent || ""
      : text || "";
    diagnostics.addTrace("message-render-stream-text", {
      messageId: props.message.id,
      totalChars: text?.length || 0,
      durationMs: Math.round((performance.now() - renderStartedAt) * 10) / 10,
    });
  } else {
    // 动态编译态 (例如流式刚结束，或者刚编辑完)
    isTransitioning.value = true;
    try {
      const newBlocks = await processMessageContent(text || "", options);
      contentBlocks.value = newBlocks;
      // 可选：将新编译的块缓存到 message 对象上，防止后续频繁重编
      props.message.blocks = newBlocks;
      diagnostics.addTrace("message-render-compiled-blocks", {
        messageId: props.message.id,
        totalChars: text?.length || 0,
        durationMs: Math.round((performance.now() - renderStartedAt) * 10) / 10,
        detail: `blocks=${newBlocks.length}`,
      });
    } finally {
      // 确保无论解析成功失败，都能解除过渡状态
      isTransitioning.value = false;
    }
  }
};

const queueContentUpdate = async (text: string, streaming: boolean) => {
  pendingRenderRequest = { text, streaming };
  if (isProcessing) return;

  isProcessing = true;
  try {
    while (pendingRenderRequest) {
      const request = pendingRenderRequest;
      pendingRenderRequest = null;

      try {
        await updateContentBlocks(request.text);

        if (!request.streaming) {
          await new Promise((resolve) => setTimeout(resolve, 50));
        }
      } catch (e) {
        console.error("[MessageRenderer] Watcher error:", e);
      }
    }
  } finally {
    isProcessing = false;
  }
};

// 监听文本变化或流状态变化，加入节流机制 (Throttle) 防止流式输出卡顿
watch(
  [
    () => messageRenderSource.value,
    () => isStreaming.value,
  ],
  ([newText, streaming]) => {
    void queueContentUpdate((newText as string) || "", Boolean(streaming));
  },
  { immediate: true },
);

// 计算气泡背景颜色
const bubbleStyle = computed(() => {
  if (isUser.value)
    return {
      backgroundColor: "var(--user-bubble-bg, rgba(145, 109, 51, 0.8))",
      color: "var(--user-text, #e8e8e8)",
      borderBottomRightRadius: "4px",
    };

  const color = agentConfig.value?.avatarCalculatedColor;
  const baseStyle: any = {
    backgroundColor: "var(--assistant-bubble-bg, rgba(44, 62, 74, 0.8))",
    color: "var(--agent-text, #e8e8e8)",
    borderBottomLeftRadius: "4px",
    border: "1px solid rgba(255, 255, 255, 0.08)", // Even subtler border for better blending
  };

  if (color) {
    baseStyle["--dynamic-color"] = color;
    baseStyle.borderColor = `${color}30`; // Adjust to very subtle 18% opacity
  }

  return baseStyle;
});

// 计算名称颜色
const nameStyle = computed(() => {
  if (isUser.value) return { color: "var(--secondary-text)" };
  const color = agentConfig.value?.avatarCalculatedColor;
  return { color: color || "var(--highlight-text)" };
});

const displayName = computed(() => {
  if (!props.message.name && !agentConfig.value?.name && !isUser.value)
    return null;
  return isUser.value
    ? settingsStore.settings?.userName || "ME"
    : props.message.name || agentConfig.value?.name;
});

const avatarFallbackText = computed(() => {
  return isUser.value
    ? settingsStore.settings?.userName || "ME"
    : props.message.name || "AI";
});

const avatarFallbackColor = computed(() => {
  if (isUser.value) {
    return "rgb(226,54,56)";
  }

  return agentConfig.value?.avatarCalculatedColor || "#374151";
});

// 长按菜单触发逻辑
const showMessageContextMenu = async () => {
  const chatStore = useChatManagerStore();

  const actions: any[] = [];

  // 1. 如果正在流式生成，提供强制中止功能 (最高优先级)
  if (isStreaming.value && !isUser.value) {
    actions.push({
      label: "中止回复",
      icon: StopCircle,
      danger: true,
      handler: () => {
        chatStore.stopMessage(props.message.id);
      },
    });
  }

  // 获取内容的统一方法，结合懒加载
  const getFullText = async () => {
    let text = props.message.content || streamContent.value;
    if (!text && props.message.blocks) {
      // 触发懒加载获取原文
      text = await chatStore.fetchRawContent(props.message.id);
    }
    return text;
  };

  // 2. 复制文本 (所有状态可用，除了纯占位符)
  // 为了不卡住菜单弹出，我们先在外部显示菜单，在 handler 中拉取内容
  actions.push({
    label: "复制内容",
    icon: Copy,
    handler: async () => {
      try {
        const fullText = await getFullText();
        if (!fullText) return;
        
        if (navigator.clipboard && navigator.clipboard.writeText) {
          await navigator.clipboard.writeText(fullText);
        } else {
          // Fallback for some old webviews
          const textarea = document.createElement("textarea");
          textarea.value = fullText;
          document.body.appendChild(textarea);
          textarea.select();
          document.execCommand("copy");
          document.body.removeChild(textarea);
        }
        notificationStore.addNotification({
          type: "success",
          title: "复制成功",
          message: "内容已复制到剪贴板",
          duration: 2000,
        });
      } catch (e) {
        console.error("[MessageContextMenu] Copy failed:", e);
      }
    },
  });

  // 3. 编辑消息 (非流式状态下支持全屏编辑)
  if (!isStreaming.value) {
    actions.push({
      label: "编辑消息",
      icon: Edit2,
      handler: async () => {
        const fullText = await getFullText();
        overlayStore.openEditor({
          initialValue: fullText || "",
          onSave: (newContent: string) => handleSaveEdit(newContent),
        });
      },
    });
  }

  // 4. 用户特权操作 (编辑重发)
  if (isUser.value) {
    actions.push({
      label: "编辑重发",
      icon: Edit2,
      handler: async () => {
        const fullText = await getFullText();
        // 将内容填入全局编辑状态供 InputEnhancer 读取
        chatStore.editMessageContent = fullText || "";
      },
    });
  }

  // 5. AI 重新生成 (非流式状态下可用)
  if (!isUser.value && !isStreaming.value) {
    actions.push({
      label: "重新生成",
      icon: RotateCcw,
      handler: () => {
        chatStore.regenerateResponse(props.message.id);
      },
    });
  }

  // 6. 删除 (万能操作)
  actions.push({
    label: "删除消息",
    icon: Trash2,
    danger: true,
    handler: () => {
      if (confirm("确定要删除这条消息吗？")) {
        chatStore.deleteMessage(props.message.id);
      }
    },
  });

  overlayStore.openContextMenu(
    actions,
    isUser.value ? "User Message" : "Assistant Message",
  );
};

const handleSaveEdit = async (newContent: string) => {
  const chatStore = useChatManagerStore();
  if (newContent !== props.message.content) {
    await chatStore.updateMessageContent(props.message.id, newContent);
    // 立即重新触发渲染
    await updateContentBlocks(newContent);
  }
};
</script>

<template>
  <div v-longpress="showMessageContextMenu"
    class="vcp-message-item flex flex-col w-full mb-6 px-1 min-w-0" :data-message-id="message.id"
    :data-role="message.role">
    <MessageHeader :is-user="isUser" :display-name="displayName" :name-style="nameStyle" :avatar-url="resolvedAvatarUrl"
      :avatar-fallback-text="avatarFallbackText" :avatar-fallback-color="avatarFallbackColor" />

    <ChatBubble :is-user="isUser" :is-streaming="isStreaming" :bubble-style="bubbleStyle">
      <ThinkingIndicator v-if="isStreaming && !hasVisibleStreamContent" />

      <template v-if="!showStreamView">
        <div class="vcp-content-blocks space-y-2 min-w-0 w-full overflow-hidden">
          <template v-for="(block, index) in contentBlocks" :key="index">
            <MarkdownBlock v-if="block.type === 'markdown'" :content="block.content" :is-streaming="false" />
            <ToolBlock v-else-if="block.type === 'tool-use'" :type="block.type" :content="block.content"
              :block="block" />
            <ToolBlock v-else-if="block.type === 'tool-result'" :type="block.type" :block="block" />
            <DiaryBlock v-else-if="block.type === 'diary'" :content="block.content" :block="block" />
            <ThoughtBlock v-else-if="block.type === 'thought'" :content="block.content" :block="block" />
            <HtmlPreviewBlock v-else-if="block.type === 'html-preview'" :content="block.content"
              :message-id="message.id" />
            <RoleDividerBlock v-else-if="block.type === 'role-divider'" :block="block" />
            <div v-else-if="block.type === 'button-click'"
              class="inline-block px-3 py-1 bg-black/10 dark:bg-white/10 rounded-full text-[10px] font-bold opacity-70 my-1">
              {{ block.content }}
            </div>
          </template>
        </div>
      </template>
      <template v-else>
        <div class="vcp-content-blocks space-y-2 min-w-0 w-full overflow-hidden aurora-container">
          <div v-if="message.stableContent" v-memo="[message.stableContent]" class="aurora-stable-layer">
            <template v-for="(block, index) in stableStreamBlocks" :key="`stable-${index}-${block.type}`">
              <MarkdownBlock v-if="block.type === 'markdown'" :content="block.content" :is-streaming="false" />
              <ThoughtBlock v-else-if="block.type === 'thought'" :content="block.content" :block="block" />
              <ToolBlock v-else-if="block.type === 'tool-use'" :type="block.type" :content="block.content"
                :block="block" />
              <ToolBlock v-else-if="block.type === 'tool-result'" :type="block.type" :block="block" />
              <RoleDividerBlock v-else-if="block.type === 'role-divider'" :block="block" />
            </template>
          </div>

          <div class="aurora-tail-layer">
            <template v-for="(block, index) in tailStreamBlocks" :key="`tail-${index}-${block.type}`">
              <MarkdownBlock v-if="block.type === 'markdown'" :content="block.content" :is-streaming="true" />
              <ThoughtBlock v-else-if="block.type === 'thought'" :content="block.content" :block="block"
                :is-streaming="true" />
              <ToolBlock v-else-if="block.type === 'tool-use'" :type="block.type" :content="block.content"
                :block="block" />
              <ToolBlock v-else-if="block.type === 'tool-result'" :type="block.type" :block="block" />
              <RoleDividerBlock v-else-if="block.type === 'role-divider'" :block="block" />
            </template>
          </div>
        </div>
      </template>

      <AttachmentPreview v-if="message.attachments && message.attachments.length > 0" :attachments="message.attachments"
        class="pt-3 border-t border-black/5 dark:border-white/5" />

      <StreamingTag v-if="isStreaming && hasVisibleStreamContent" />

      <template #footer>
        <div class="text-[9px] mt-1.5 px-1 opacity-50 font-mono tracking-tighter w-full"
          :class="isUser ? 'text-right' : 'text-left'" :style="isUser
              ? { color: 'var(--secondary-text)' }
              : { color: 'var(--secondary-text)' }
            ">
          {{
            new Date(message.timestamp).toLocaleTimeString([], {
              hour: "2-digit",
              minute: "2-digit",
              second: "2-digit",
            })
          }}
        </div>
      </template>
    </ChatBubble>
  </div>
</template>

<style scoped>
.animate-fade-in {
  animation: fadeIn 0.4s cubic-bezier(0.16, 1, 0.3, 1);
}

.aurora-container {
  contain: layout paint style;
}

.aurora-stable-layer {
  contain: layout paint style;
}

.aurora-tail-layer {
  position: relative;
}

@keyframes fadeIn {
  from {
    opacity: 0;
    transform: translateY(10px) scale(0.98);
  }

  to {
    opacity: 1;
    transform: translateY(0) scale(1);
  }
}
</style>
