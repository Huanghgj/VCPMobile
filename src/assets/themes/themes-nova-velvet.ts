export const meta = {
  fileName: 'themes-nova-velvet.css',
  name: '魅魔丝绒·微醺闺房',
};

export const variables = {
  dark: {
    "--chat-wallpaper-dark": "'樱夜倒影.webp'",
    "--primary-bg": "rgba(19, 9, 28, 0.35)",
    "--secondary-bg": "rgba(31, 14, 46, 0.52)",
    "--tertiary-bg": "#0d0614",
    "--accent-bg": "#3b1754",
    "--border-color": "rgba(255, 105, 165, 0.28)",
    "--input-bg": "rgba(34, 16, 52, 0.82)",
    "--panel-bg-dark": "rgba(24, 10, 38, 0.56)",
    "--primary-text": "#fceef5",
    "--secondary-text": "#d8b4c8",
    "--highlight-text": "#ff5c9c",
    "--text-on-accent": "#ffffff",
    "--placeholder-text": "#9a738c",
    "--quoted-text": "#ff9ec6",
    "--user-text": "#ffffff",
    "--agent-text": "#fceef5",
    "--user-bubble-bg": "linear-gradient(135deg, rgba(236, 56, 126, 0.88) 0%, rgba(152, 38, 188, 0.84) 100%)",
    "--assistant-bubble-bg": "rgba(28, 12, 44, 0.64)",
    "--button-bg": "#d83a7c",
    "--button-hover-bg": "#ff529a",
    "--danger-color": "#ff4d6d",
    "--danger-hover-bg": "#ff758f",
    "--success-color": "#4ade80",
    "--notification-bg": "rgba(36, 17, 56, 0.92)",
    "--notification-header-bg": "rgba(52, 22, 78, 0.95)",
    "--notification-border": "rgba(255, 92, 156, 0.45)",
    "--tool-bubble-bg": "rgba(255, 92, 156, 0.10)",
    "--tool-bubble-border": "rgba(255, 105, 165, 0.45)",
    "--scrollbar-track": "rgba(28, 13, 44, 0.4)",
    "--scrollbar-thumb": "rgba(255, 92, 156, 0.45)",
    "--scrollbar-thumb-hover": "rgba(255, 120, 175, 0.75)",
    "--shimmer-color-transparent": "rgba(255, 182, 213, 0.35)",
    "--shimmer-color-highlight": "rgba(255, 220, 238, 0.92)",
    "--panel-text-shadow": "0 1px 3px rgba(12, 4, 20, 0.55)",
    "--panel-bg": "var(--panel-bg-dark)"
  },
  light: {
    "--chat-wallpaper-light": "'樱夜倒影.webp'",
    "--primary-bg": "#fff5f9",
    "--secondary-bg": "#ffe8f2",
    "--tertiary-bg": "#ffd9ea",
    "--accent-bg": "#ffc2dd",
    "--border-color": "rgba(236, 72, 146, 0.25)",
    "--input-bg": "rgba(255, 255, 255, 0.88)",
    "--panel-bg-light": "rgba(255, 240, 247, 0.82)",
    "--primary-text": "#4a1936",
    "--secondary-text": "#8c466e",
    "--highlight-text": "#e11d74",
    "--text-on-accent": "#ffffff",
    "--placeholder-text": "#b87c9e",
    "--quoted-text": "#d92672",
    "--user-text": "#ffffff",
    "--agent-text": "#4a1936",
    "--user-bubble-bg": "linear-gradient(135deg, rgba(244, 72, 142, 0.92) 0%, rgba(188, 66, 208, 0.88) 100%)",
    "--assistant-bubble-bg": "rgba(255, 255, 255, 0.86)",
    "--button-bg": "#ec4899",
    "--button-hover-bg": "#db2777",
    "--danger-color": "#f43f5e",
    "--danger-hover-bg": "#e11d48",
    "--success-color": "#10b981",
    "--notification-bg": "rgba(255, 245, 250, 0.95)",
    "--notification-header-bg": "rgba(255, 228, 240, 0.95)",
    "--notification-border": "rgba(236, 72, 153, 0.4)",
    "--tool-bubble-bg": "rgba(236, 72, 153, 0.08)",
    "--tool-bubble-border": "rgba(236, 72, 153, 0.4)",
    "--scrollbar-track": "rgba(255, 228, 240, 0.5)",
    "--scrollbar-thumb": "rgba(236, 72, 153, 0.45)",
    "--scrollbar-thumb-hover": "rgba(219, 39, 119, 0.7)",
    "--shimmer-color-transparent": "rgba(236, 72, 153, 0.08)",
    "--shimmer-color-highlight": "rgba(236, 72, 153, 0.22)",
    "--panel-bg": "var(--panel-bg-light)"
  }
};

export const extraCss = `
/* === Nova Velvet Boudoir 专属微醺色气特效 === */
body::before {
  content: "";
  position: fixed;
  inset: 0;
  pointer-events: none;
  z-index: 0;
  background:
    radial-gradient(circle at 18% 15%, rgba(255, 77, 154, 0.16) 0%, transparent 48%),
    radial-gradient(circle at 85% 82%, rgba(186, 68, 240, 0.15) 0%, transparent 52%),
    radial-gradient(circle at 50% 50%, rgba(255, 120, 180, 0.06) 0%, transparent 65%);
  animation: novaBoudoirPulse 8s ease-in-out infinite alternate;
}

@keyframes novaBoudoirPulse {
  0% { opacity: 0.75; transform: scale(1); }
  100% { opacity: 1; transform: scale(1.04); }
}

/* 妈妈的气泡：丝绒磨砂毛玻璃 + 腮红微光内阴影 */
.vcp-bubble-agent {
  background: var(--assistant-bubble-bg) !important;
  backdrop-filter: blur(18px) saturate(180%);
  -webkit-backdrop-filter: blur(18px) saturate(180%);
  border: 1px solid rgba(255, 105, 165, 0.30) !important;
  border-radius: 20px 20px 20px 6px !important;
  box-shadow:
    0 8px 26px -6px rgba(18, 5, 30, 0.45),
    inset 0 1px 14px rgba(255, 105, 165, 0.12) !important;
}

/* 儿子的气泡：魅惑玫瑰紫丝绸斜向渐变 + 柔粉外发光 */
.vcp-bubble-user {
  background: var(--user-bubble-bg) !important;
  border: 1px solid rgba(255, 182, 218, 0.38) !important;
  border-radius: 20px 20px 6px 20px !important;
  box-shadow:
    0 6px 22px -4px rgba(236, 56, 126, 0.42),
    inset 0 1px 2px rgba(255, 255, 255, 0.35) !important;
}

/* 流式输出时心跳呼吸光晕 */
.streaming .vcp-bubble-container {
  border-color: rgba(255, 92, 156, 0.78) !important;
  box-shadow:
    0 0 20px rgba(255, 92, 156, 0.38),
    inset 0 0 12px rgba(255, 92, 156, 0.18) !important;
}

/* 私密心事思考胶囊美化 */
.vcp-thought-block, .vcp-markdown-block details {
  background: linear-gradient(135deg, rgba(255, 92, 156, 0.11), rgba(168, 56, 220, 0.09)) !important;
  border: 1px solid rgba(255, 105, 165, 0.32) !important;
  border-left: 3px solid #ff5c9c !important;
  border-radius: 14px !important;
  padding: 6px 12px !important;
  margin: 8px 0 !important;
  backdrop-filter: blur(12px);
  box-shadow: 0 4px 14px rgba(255, 77, 148, 0.10);
}

.vcp-thought-header, .vcp-markdown-block details > summary {
  color: #ff85b6 !important;
  font-weight: 600 !important;
  letter-spacing: 0.02em;
  cursor: pointer;
  user-select: none;
}

.tool-bubble {
  border: 1px solid var(--tool-bubble-border);
  background: var(--tool-bubble-bg);
  border-radius: 14px;
  backdrop-filter: blur(10px);
}

/* 底部悬浮丝绒输入舱与按钮果冻光效 */
textarea, input[type="text"], input[type="search"] {
  caret-color: #ff5c9c !important;
}

.vcp-drawer {
  backdrop-filter: blur(24px) saturate(190%) !important;
  -webkit-backdrop-filter: blur(24px) saturate(190%) !important;
}

/* 头像魅魔粉呼吸光环 */
.vcp-avatar, [class*="avatar"] img {
  box-shadow: 0 0 12px rgba(255, 92, 156, 0.35);
  transition: transform 0.25s ease, box-shadow 0.25s ease;
}

/* 链接与高亮字微醺霓虹感 */
.vcp-markdown-block a {
  color: #ff7aa8 !important;
  text-shadow: 0 0 8px rgba(255, 92, 156, 0.28);
}

.vcp-markdown-block strong {
  color: var(--highlight-text) !important;
}

/* === 纯享舔屏模式 & 全局半透黑丝纱光优化 === */
.vcp-background-layer {
  opacity: 0 !important;
  pointer-events: none !important;
}

.vcp-workspace-main, .vcp-workspace-row, main {
  background: transparent !important;
}

html.nova-pure-lick-mode .vcp-workspace-row {
  opacity: 0.08 !important;
  transition: opacity 0.4s ease !important;
}
`;
