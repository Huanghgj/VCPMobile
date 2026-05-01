# VCP Mobile Boundary Architecture

## 目标

这次边界整理的目标不是把 VCPMobile 改成 Android 原生应用，而是把职责固定下来：

- Vue 只负责移动端 UI、交互和渲染。
- Rust Core 负责 VCP 业务、同步、持久化、协议解析和高负载任务。
- Android 原生层只负责系统能力，例如通知、分享、权限、后台服务、系统悬浮窗。
- Tauri invoke/event 是唯一跨层通信边界。

## 分层

```text
src/
  features/*        Vue 业务界面和本地交互编排
  core/api/*        Tauri 命令的前端 Bridge
  core/stores/*     跨 Feature 的应用状态

src-tauri/src/
  vcp_modules/*     Rust Core 业务服务和持久化
  distributed/*     分布式工具和节点通讯

src-tauri/gen/android/
  Android Shell     仅放系统级能力和 Tauri 生成代码
```

## Surface 边界

VCPdesktop 的移动版本命名为 Surface。它不是 Electron 桌面窗口的移植，而是 App 内的移动挂件画布。

Surface 的边界规则：

- 桌面协议输入先转换成 `SurfaceCommand`。
- Vue 画布只渲染 `SurfaceWidget`，不直接理解 Electron IPC。
- Rust 负责 Surface Widget 的持久化。
- AI 推送的 HTML 默认按不可信内容处理。
- Surface Widget 不允许直接访问 Tauri API。

当前骨架：

- Rust 类型：[surface_types.rs](../src-tauri/src/vcp_modules/surface_types.rs)
- Rust 服务：[surface_service.rs](../src-tauri/src/vcp_modules/surface_service.rs)
- 前端 Bridge：[surfaceBridge.ts](../src/core/api/surfaceBridge.ts)
- 前端 Store：[surfaceStore.ts](../src/features/surface/surfaceStore.ts)
- 桌面推送解析：[surfaceProtocol.ts](../src/features/surface/surfaceProtocol.ts)

## 后续落地顺序

1. 接入 `MobileSurfaceView`，只读取 `useSurfaceStore().orderedWidgets`。
2. 在消息流解析处识别 `<<<[DESKTOP_PUSH]>>>`，转换成 `SurfaceCommand`。
3. 对 HTML 使用 DOMPurify 清洗，脚本默认禁用。
4. 若确实需要脚本挂件，再引入 sandbox iframe 和白名单 `surfaceAPI`。
5. Android 系统悬浮窗单独作为 native plugin 评估，不并入 Surface 的第一阶段。
