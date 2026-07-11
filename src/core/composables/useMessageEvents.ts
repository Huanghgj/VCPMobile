import { onMounted, onUnmounted, type Ref } from "vue";
import { openUrl as openExternal } from "@tauri-apps/plugin-opener";
import { useChatHistoryStore } from "../stores/chatHistoryStore";
import { openRenderedImageViewer } from "./useRenderedImageViewer";
import { findRenderedImagePayload } from "../utils/renderedImage";

function safeExternalHttpUrl(href: string | null): string {
  if (!href) return "";
  const trimmed = href.trim();
  if (!/^https?:\/\//i.test(trimmed)) return "";
  try {
    const url = new URL(trimmed);
    return url.protocol === "http:" || url.protocol === "https:" ? url.href : "";
  } catch {
    return "";
  }
}

export function useMessageEvents(containerRef: Ref<HTMLElement | null>) {
  const historyStore = useChatHistoryStore();

  const readCopyCode = (button: HTMLElement): string => {
    const raw = button.getAttribute("data-vcp-copy-code") || "";
    if (button.getAttribute("data-vcp-copy-encoded") === "uri") {
      try {
        return decodeURIComponent(raw);
      } catch {
        return raw;
      }
    }
    return raw;
  };

  const handleClick = (e: MouseEvent) => {
    if (!(e.target instanceof Element)) return;
    const target = e.target;

    const copyButton = target.closest('[data-vcp-copy-code]') as HTMLButtonElement | null;
    if (copyButton) {
      e.preventDefault();
      e.stopPropagation();

      navigator.clipboard
        .writeText(readCopyCode(copyButton))
        .then(() => {
          copyButton.classList.add("is-copied");
          copyButton.setAttribute("aria-label", "已复制");
          copyButton.title = "已复制";
          window.setTimeout(() => {
            copyButton.classList.remove("is-copied");
            copyButton.setAttribute("aria-label", "复制代码");
            copyButton.title = "复制代码";
          }, 1200);
        })
        .catch((err) => {
          console.error("[useMessageEvents] Failed to copy code block:", err);
        });
      return;
    }

    // 消息渲染器自身的控件只处理 UI 状态，不能落入 AI 生成按钮的发送逻辑。
    if (target.closest('[data-vcp-ui-control]')) {
      return;
    }

    // 用户发送的附件（图片/视频/文件）有自身的点击处理（AttachmentPreview → AttachmentViewer）。
    // 全局消息内容点击器必须完全避让，否则会与之叠加：例如点击自己发的图片会同时打开
    // AttachmentViewer 与 RenderedImageViewer 两个查看器。
    if (target.closest('.vcp-attachment-preview')) {
      return;
    }

    // 1. VCP 按钮点击 (e.g., [[点击按钮:xxx]])
    const vcpButton = target.closest('[data-vcp-button]');
    if (vcpButton) {
      const text = vcpButton.getAttribute('data-vcp-button');
      if (text) historyStore.sendMessage(text);
      return;
    }

    // 1.5 拦截 AI 回复中生成的内嵌 <button> 元素
    const aiButton = target.closest('button') as HTMLButtonElement | null;
    const explicitSendText =
      aiButton?.getAttribute('data-vcp-send') ||
      aiButton?.getAttribute('data-send') ||
      "";
    if (aiButton && explicitSendText.trim()) {
      e.preventDefault();
      e.stopPropagation();

      // 如果按钮已被禁用，直接拦截，防止重复点击
      if (aiButton.disabled) {
        return;
      }

      // 提取发送文本（优先级：data-send 属性 > 按钮 textContent）
      const sendText = explicitSendText.trim();
      if (sendText) {
        let finalSendText = `[[点击按钮:${sendText}]]`;

        // 超长文本截断（防超限）
        if (finalSendText.length > 500) {
          const maxTextLength = 500 - '[[点击按钮:]]'.length;
          const truncatedText = sendText.substring(0, maxTextLength);
          finalSendText = `[[点击按钮:${truncatedText}]]`;
        }

        // 按钮物理禁用与状态置灰反馈（与桌面端一致）
        aiButton.disabled = true;
        aiButton.style.opacity = '0.6';
        aiButton.style.cursor = 'not-allowed';
        const originalText = aiButton.textContent || '';
        aiButton.textContent = originalText + ' ✓';

        // 发送消息
        historyStore.sendMessage(finalSendText);
      }
      return;
    }

    // 1.75 AI 渲染图片：覆盖 img / svg image / svg / canvas / CSS background-image
    const imagePayload = findRenderedImagePayload(e.target, containerRef.value);
    if (imagePayload) {
      e.preventDefault();
      e.stopPropagation();
      openRenderedImageViewer({
        ...imagePayload,
        sourceLabel: "AI 渲染图片",
      });
      return;
    }

    // 2. 消息引用跳转
    const messageRef = target.closest('a[href^="#msg-"]');
    if (messageRef) {
      e.preventDefault();
      const msgId = messageRef.getAttribute('href')?.replace('#msg-', '');
      if (msgId) {
          // TODO: implement scrollToMessage
      }
      return;
    }

    // 3. 气泡内普通图片点击劫持 (排除带有 vcp-emoticon 的表情包以及消息附件缩略图)
    if (target.tagName.toLowerCase() === "img") {
      const isEmoticon = target.classList.contains("vcp-emoticon");
      const isAttachment = target.closest(".vcp-attachment-preview") !== null;

      if (!isEmoticon && !isAttachment) {
        e.preventDefault();
        e.stopPropagation();

        const src = target.getAttribute("src") || "";
        const alt = target.getAttribute("alt") || "";
        const title = target.getAttribute("title") || "";

        // 动态引入查看器 Composable，消灭潜在的 Vue 组件循环引用
        import("./useRenderedImageViewer")
          .then(({ openRenderedImageViewer }) => {
            openRenderedImageViewer({
              src,
              alt,
              title,
              sourceLabel: "聊天图片",
            });
          })
          .catch((err) => {
            console.error("[useMessageEvents] Failed to open RenderedImageViewer:", err);
          });
        return;
      }
    }

    // 4. 外部链接只允许明确的 http/https，其他协议阻断浏览器默认行为。
    const externalLink = target.closest('a[href]');
    if (externalLink) {
      e.preventDefault();
      const href = safeExternalHttpUrl(externalLink.getAttribute('href'));
      if (href) openExternal(href);
      return;
    }
  };

  onMounted(() => {
    if (containerRef.value) {
      containerRef.value.addEventListener("click", handleClick);
    }
  });

  onUnmounted(() => {
    if (containerRef.value) {
      containerRef.value.removeEventListener("click", handleClick);
    }
  });
}
