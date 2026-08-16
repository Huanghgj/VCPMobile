import { defineStore } from "pinia";
import { ref, nextTick } from "vue";
import { invoke, convertFileSrc } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { useDocumentProcessor } from "../composables/useDocumentProcessor";
import { useNotificationStore } from "./notification";
import { preloadImage } from "../utils/resourcePreloader";
import type { Attachment } from "../types/chat";

export interface NativePickedFile {
  nativeId?: string;
  path: string;
  name: string;
  mime: string;
  size: number;
  hash: string;
  thumbnailPath?: string;
}

export interface NativePickError {
  nativeId?: string;
  message: string;
}

export interface NativePickedBatch {
  files: NativePickedFile[];
  errors: NativePickError[];
}

export function normalizeNativePickedBatch(value: unknown): NativePickedBatch {
  if (Array.isArray(value)) {
    return { files: value as NativePickedFile[], errors: [] };
  }
  if (!value || typeof value !== "object") {
    return { files: [], errors: [] };
  }

  const record = value as Record<string, unknown>;
  if (Array.isArray(record.files)) {
    return {
      files: record.files as NativePickedFile[],
      errors: Array.isArray(record.errors)
        ? (record.errors as NativePickError[])
        : [],
    };
  }
  if (typeof record.path === "string") {
    return { files: [record as unknown as NativePickedFile], errors: [] };
  }
  return { files: [], errors: [] };
}

/**
 * 前端辅助：异步读取图片原始分辨率（不依赖后端）
 * 用于上传前拦截超限图片（>8K×8K）
 */
const checkImageDimensions = (
  file: File,
): Promise<{ width: number; height: number }> => {
  return new Promise((resolve, reject) => {
    const img = new Image();
    const url = URL.createObjectURL(file);
    img.onload = () => {
      URL.revokeObjectURL(url);
      resolve({ width: img.naturalWidth, height: img.naturalHeight });
    };
    img.onerror = () => {
      URL.revokeObjectURL(url);
      reject(new Error("无法读取图片尺寸"));
    };
    img.src = url;
  });
};

export const useAttachmentStore = defineStore("attachment", () => {
  // 暂存的附件列表，准备随下一条消息发送
  const stagedAttachments = ref<Attachment[]>([]);
  const isPickingAttachment = ref(false);

  const preloadAttachmentImage = (source: string) => {
    if (!source) return;
    preloadImage(source).catch((err) => {
      console.warn("[AttachmentStore] Failed to preload image asset:", err);
    });
  };

  const resolveAttachmentImage = async (att: Attachment) => {
    if (!att.type.startsWith("image/")) return;

    if (
      att.resolvedSrc?.startsWith("data:image/") ||
      att.resolvedSrc?.startsWith("http:") ||
      att.resolvedSrc?.startsWith("https:")
    ) {
      preloadAttachmentImage(att.resolvedSrc);
      return;
    }

    const sourcePath = att.thumbnailPath || att.internalPath || att.src;
    if (!sourcePath) return;

    if (
      sourcePath.startsWith("data:image/") ||
      sourcePath.startsWith("http:") ||
      sourcePath.startsWith("https:") ||
      sourcePath.startsWith("blob:")
    ) {
      att.resolvedSrc = sourcePath;
      preloadAttachmentImage(sourcePath);
      return;
    }

    try {
      att.resolvedSrc = await invoke<string>("read_image_preview_data_url", {
        path: sourcePath,
      });
      preloadAttachmentImage(att.resolvedSrc);
      return;
    } catch (err) {
      console.warn(
        `[AttachmentStore] Failed to read image preview data for ${att.name}:`,
        err,
      );
    }

    try {
      att.resolvedSrc = convertFileSrc(sourcePath.replace("file://", ""));
      preloadAttachmentImage(att.resolvedSrc);
    } catch (err) {
      console.warn(
        `[AttachmentStore] Failed to convert attachment image path ${att.name}:`,
        err,
      );
    }
  };

  const resolveAttachmentAsset = async (att: Attachment) => {
    if (att.type.startsWith("image/")) {
      await resolveAttachmentImage(att);
      return;
    }
    if (!att.type.startsWith("video/") && !att.type.startsWith("audio/")) {
      return;
    }

    const sourcePath = att.internalPath || att.src;
    if (!sourcePath) return;
    if (/^(?:https?:|data:|blob:|asset:|tauri:)/i.test(sourcePath)) {
      att.resolvedSrc = sourcePath;
      return;
    }

    try {
      const allowedPath = await invoke<string>("prepare_attachment_asset", {
        path: sourcePath,
      });
      att.resolvedSrc = convertFileSrc(allowedPath);
    } catch (error) {
      console.warn(
        `[AttachmentStore] Failed to prepare media preview for ${att.name}:`,
        error,
      );
    }
  };

  // 全局监听 Rust 端发出的注册进度，用于大文件哈希/移动等Phase 2流程
  listen<any>("vcp-file-register-progress", (event) => {
    const { progress, stableId } = event.payload;
    if (stableId) {
      const idx = stagedAttachments.value.findIndex((a) => a.id === stableId);
      if (idx !== -1 && stagedAttachments.value[idx].status === "loading") {
        const currentProgress = stagedAttachments.value[idx].progress || 0;
        // 防抖/防回退：仅在进度增加时更新
        if (progress > currentProgress) {
          stagedAttachments.value[idx].progress = progress;
        }
      }
    }
  });

  /**
   * 处理消息中的本地资源路径 (仅附件)，使用 Tauri 原生 asset:// 协议绕过 WebView 限制
   */
  const resolveMessageAssets = async (msg: any) => {
    if (msg.attachments && msg.attachments.length > 0) {
      await Promise.all(
        msg.attachments.map((att: Attachment) => resolveAttachmentAsset(att)),
      );
    }
  };

  /**
   * 触发文件选择器并暂存附件 (Android 物理端使用原生选择拦截直传，其他端使用标准 HTML Input 完美支持)
   */
  const handleAttachmentInternal = async (
    mode: "camera" | "gallery" | "file" = "file",
  ) => {
    const isAndroid = navigator.userAgent.toLowerCase().includes("android");

    // ==================================================================
    // Android 端主链路：原生插件拦截直传
    //   - 不走下方的 store_file / prepare_vcp_upload 分流逻辑
    //   - 由 Kotlin 侧的 VcpMobilePlugin.pickFile 启动系统文件选择器
    //   - Native 层流式拷贝到 cacheDir 并计算 SHA-256，最后通过
    //     register_local_file 零拷贝注册到附件目录
    // ==================================================================
    if (isAndroid) {
      console.log(
        `[AttachmentStore] Android environment detected. Intercepting via native picker. Mode: ${mode}`,
      );
      const notificationStore = useNotificationStore();
      const requestId = `picker_${Date.now()}_${Math.random().toString(36).slice(2, 9)}`;

      const cleanupNativeTempPath = async (filePath?: string) => {
        if (!filePath) return;
        try {
          await invoke("plugin:vcp-mobile|delete_temp_file", { filePath });
        } catch (cleanupError) {
          console.warn(
            `[AttachmentStore] Failed to clean native temp file ${filePath}:`,
            cleanupError,
          );
        }
      };
      const cleanupNativeBatch = async (value: unknown) => {
        const batch = normalizeNativePickedBatch(value);
        const paths = new Set<string>();
        for (const file of batch.files) {
          if (file.path) paths.add(file.path);
          if (file.thumbnailPath) paths.add(file.thumbnailPath);
        }
        await Promise.all([...paths].map(cleanupNativeTempPath));
      };

      const stableIdsByNativeId = new Map<string, string>();
      const createdStableIds = new Set<string>();
      const createStableId = (nativeId?: string) => {
        if (nativeId) {
          const existing = stableIdsByNativeId.get(nativeId);
          if (existing) return existing;
        }
        const stableId = `att_${Date.now()}_${Math.random().toString(36).slice(2, 9)}`;
        if (nativeId) stableIdsByNativeId.set(nativeId, stableId);
        createdStableIds.add(stableId);
        return stableId;
      };
      const removeLoadingAttachment = (stableId: string) => {
        const index = stagedAttachments.value.findIndex(
          (attachment) =>
            attachment.id === stableId && attachment.status === "loading",
        );
        if (index !== -1) stagedAttachments.value.splice(index, 1);
      };

      try {
        // 1. 调用物理端原生 File Picker (双轨事件监听 + 5分钟熔断)

        const picked = await new Promise<any>((resolve, reject) => {
          let resolved = false;
          let timedOut = false;

          const handleStart = (e: any) => {
            if (resolved) return;
            if (e.detail?.requestId !== requestId) return;
            const { nativeId, name, size, mime } = e.detail;
            const stableId = createStableId(nativeId || "legacy");
            if (
              stagedAttachments.value.some(
                (attachment) => attachment.id === stableId,
              )
            ) {
              return;
            }
            stagedAttachments.value.unshift({
              id: stableId,
              type: mime || "application/octet-stream",
              src: "",
              name: name || "文件",
              size: size || 0,
              progress: 0,
              status: "loading",
            });
          };

          const handleProgress = (e: any) => {
            if (resolved) return;
            if (e.detail?.requestId !== requestId) return;
            const { nativeId, progress, name, mime, total } = e.detail;
            const stableId = createStableId(nativeId || "legacy");
            const idx = stagedAttachments.value.findIndex(
              (a) => a.id === stableId,
            );
            const scaledProgress = Math.round(progress * 0.9); // Kotlin 沙盒拷贝与哈希阶段占 90%
            if (idx !== -1) {
              stagedAttachments.value[idx].progress = scaledProgress;
            } else if (name) {
              // 自我修复：如果 WebView 错过了 vcp-mobile-file-start 事件，在这里补建卡片
              stagedAttachments.value.unshift({
                id: stableId,
                type: mime || "application/octet-stream",
                src: "",
                name: name,
                size: total || 0,
                progress: scaledProgress,
                status: "loading",
              });
            }
          };

          const handlePicked = (e: any) => {
            if (resolved || mode !== "camera") return;
            if (e.detail?.requestId !== requestId) return;
            resolved = true;
            cleanup();
            console.log(
              "[AttachmentStore] Native picker returned via EventBus:",
              e.detail,
            );
            resolve(e.detail);
          };

          const handleBatchPicked = (e: any) => {
            if (resolved) return;
            if (e.detail?.requestId !== requestId) return;
            resolved = true;
            cleanup();
            console.log(
              "[AttachmentStore] Native picker batch returned via EventBus:",
              e.detail,
            );
            resolve(e.detail);
          };

          const handlePickerDismissed = (e: any) => {
            if (resolved) return;
            if (e.detail?.requestId !== requestId) return;
            resolved = true;
            cleanup();
            console.log("[AttachmentStore] Native picker dismissed:", e.detail);
            resolve({ requestId, files: [], errors: [] });
          };

          const cleanup = () => {
            window.removeEventListener("vcp-mobile-file-start", handleStart);
            window.removeEventListener(
              "vcp-mobile-file-progress",
              handleProgress,
            );
            window.removeEventListener("vcp-mobile-file-picked", handlePicked);
            window.removeEventListener(
              "vcp-mobile-files-picked",
              handleBatchPicked,
            );
            window.removeEventListener(
              "vcp-mobile-file-picker-dismissed",
              handlePickerDismissed,
            );
            clearTimeout(timer);
          };

          window.addEventListener("vcp-mobile-file-start", handleStart);
          window.addEventListener("vcp-mobile-file-progress", handleProgress);
          window.addEventListener("vcp-mobile-file-picked", handlePicked);
          window.addEventListener("vcp-mobile-files-picked", handleBatchPicked);
          window.addEventListener(
            "vcp-mobile-file-picker-dismissed",
            handlePickerDismissed,
          );

          const timer = setTimeout(() => {
            if (!resolved) {
              resolved = true;
              timedOut = true;
              cleanup();
              reject(
                new Error(
                  "Native file picker timed out (30 mins) without reporting",
                ),
              );
            }
          }, 1_800_000);

          invoke<any>("plugin:vcp-mobile|pick_file", { mode, requestId })
            .then((res) => {
              if (!resolved) {
                resolved = true;
                cleanup();
                console.log(
                  "[AttachmentStore] Native picker returned via Invoke:",
                  res,
                );
                resolve(res);
              } else if (timedOut) {
                void cleanupNativeBatch(res);
              }
            })
            .catch((err) => {
              if (!resolved) {
                resolved = true;
                cleanup();
                reject(err);
              }
            });
        });

        const batch = normalizeNativePickedBatch(picked);
        for (const error of batch.errors) {
          if (error.nativeId) {
            const stableId = stableIdsByNativeId.get(error.nativeId);
            if (stableId) removeLoadingAttachment(stableId);
          }
        }

        if (batch.files.length === 0) {
          console.log(
            "[AttachmentStore] Pick cancelled or returned empty path.",
          );
          createdStableIds.forEach(removeLoadingAttachment);
          if (batch.errors.length > 0) {
            notificationStore.addNotification({
              type: "warning",
              title: "附件选取失败",
              message: `${batch.errors.length} 个文件处理失败`,
              toastOnly: true,
            });
          }
          return;
        }

        const registrationErrors: string[] = batch.errors.map(
          (error) => error.message,
        );
        for (const picked of batch.files) {
          const stableId = createStableId(picked.nativeId);
          try {
            await invoke("check_attachment_support", {
              originalName: picked.name,
            });

            // 兜底：如果卡片还没插入，补插一张
            const existingIdx = stagedAttachments.value.findIndex(
              (a) => a.id === stableId,
            );
            if (existingIdx === -1) {
              stagedAttachments.value.unshift({
                id: stableId,
                type: picked.mime || "application/octet-stream",
                src: "",
                name: picked.name || "文件",
                size: picked.size || 0,
                progress: 90,
                status: "loading",
              });
            } else {
              stagedAttachments.value[existingIdx].progress = 90;
            }

            // 缩略图展示策略：若有 native thumbnail 物理路径则通过 convertFileSrc 转换，否则如果为图片，转换 path 自身
            let displaySrc = "";
            if (picked.thumbnailPath) {
              displaySrc = convertFileSrc(picked.thumbnailPath);
            } else if (picked.mime?.startsWith("image/")) {
              displaySrc = convertFileSrc(picked.path);
            }

            if (displaySrc) {
              const finalIdx = stagedAttachments.value.findIndex(
                (a) => a.id === stableId,
              );
              if (finalIdx !== -1) {
                stagedAttachments.value[finalIdx].resolvedSrc = displaySrc;
              }
              preloadAttachmentImage(displaySrc);
            }

            await nextTick();
            window.dispatchEvent(new Event("resize"));

            // 3. 后端零拷贝直传与注册 (会触发 rename 移动，缩略图生成，文本提取)
            const finalData = await invoke<any>("register_local_file", {
              localPath: picked.path,
              originalName: picked.name,
              mimeType: picked.mime || "application/octet-stream",
              thumbnailPath: picked.thumbnailPath || null,
              stableId: stableId,
              expectedHash: picked.hash || null,
            });

            if (!finalData) {
              throw new Error("附件注册未返回结果");
            }
            const index = stagedAttachments.value.findIndex(
              (a) => a.id === stableId,
            );
            if (index !== -1) {
              stagedAttachments.value[index] = {
                ...stagedAttachments.value[index],
                type: finalData.type,
                src: finalData.internalPath,
                internalPath: finalData.internalPath,
                thumbnailPath: finalData.thumbnailPath,
                name: finalData.name,
                size: finalData.size,
                hash: finalData.hash,
                status: "done",
                progress: undefined,
              };
              await resolveAttachmentAsset(stagedAttachments.value[index]);
            }
          } catch (err: any) {
            console.error(
              `[AttachmentStore] Failed to register ${picked.name}:`,
              err,
            );
            removeLoadingAttachment(stableId);
            await Promise.all([
              cleanupNativeTempPath(picked.path),
              cleanupNativeTempPath(picked.thumbnailPath),
            ]);
            registrationErrors.push(err?.message || String(err));
          }
        }

        if (registrationErrors.length > 0) {
          notificationStore.addNotification({
            type: "warning",
            title: "部分附件处理失败",
            message: `${registrationErrors.length} 个文件未能添加，其余文件已保留`,
            toastOnly: true,
          });
        }
      } catch (err: any) {
        console.error("[AttachmentStore] Native file pick failed:", err);
        createdStableIds.forEach(removeLoadingAttachment);
        const errMsg = err?.message || String(err);
        const isCancelled = /cancel/i.test(errMsg);
        if (!isCancelled) {
          notificationStore.addNotification({
            type: "warning",
            title: "选取附件失败",
            message: errMsg,
            toastOnly: true,
          });
        }
      }
      return;
    }

    // ==================================================================
    // 非 Android 端的标准 HTML `<input>` 流程
    //   - 含旧版分流逻辑：小文件 (<2MB) 走 store_file IPC；大文件走
    //     prepare_vcp_upload 高速 TCP 链路
    //   - Android 端已在上方通过原生插件处理，不会执行到此处
    // ==================================================================
    return new Promise<void>((resolve, reject) => {
      const input = document.createElement("input");
      input.type = "file";
      input.multiple = mode !== "camera";

      let settled = false;
      let focusFallbackTimer: ReturnType<typeof window.setTimeout> | null =
        null;
      const cleanupPicker = () => {
        if (focusFallbackTimer !== null) {
          window.clearTimeout(focusFallbackTimer);
          focusFallbackTimer = null;
        }
        window.removeEventListener("focus", handleWindowFocus);
        input.onchange = null;
        input.oncancel = null;
        input.remove();
      };
      const settleResolve = () => {
        if (settled) return;
        settled = true;
        cleanupPicker();
        resolve();
      };
      const settleReject = (error: unknown) => {
        if (settled) return;
        settled = true;
        cleanupPicker();
        reject(error);
      };
      function handleWindowFocus() {
        if (settled || focusFallbackTimer !== null) return;
        // Older WebViews do not emit the input `cancel` event. Focus returns
        // before `change`, so defer the empty-selection check by one short turn.
        focusFallbackTimer = window.setTimeout(() => {
          focusFallbackTimer = null;
          if (!settled && (!input.files || input.files.length === 0)) {
            settleResolve();
          }
        }, 300);
      }

      // 根据模式设置 accept 和 capture
      if (mode === "camera") {
        input.accept = "image/*";
        input.setAttribute("capture", "environment");
      } else if (mode === "gallery") {
        input.accept = "image/*";
      } else {
        input.accept = "*/*";
      }

      input.onchange = async (e: Event) => {
        try {
          const target = e.target as HTMLInputElement;
          if (!target.files || target.files.length === 0) {
            settleResolve();
            return;
          }

          const files = Array.from(target.files);
          const notificationStore = useNotificationStore();
          const processingErrors: string[] = [];

          for (const file of files) {
            console.log(
              `[AttachmentStore] Selected file via HTML input: ${file.name}, type: ${file.type}, size: ${file.size}`,
            );
            const ext = file.name.split(".").pop()?.toLowerCase() || "";
            const isGif = ext === "gif" || file.type === "image/gif";
            const isImage = file.type.startsWith("image/");

            try {
              await invoke("check_attachment_support", {
                originalName: file.name,
              });
            } catch (error) {
              notificationStore.addNotification({
                type: "warning",
                title: "不支持的附件格式",
                message: `${file.name}: ${error instanceof Error ? error.message : String(error)}`,
                toastOnly: false,
              });
              continue;
            }

            if (isImage && !isGif && file.size > 10 * 1024 * 1024) {
              notificationStore.addNotification({
                type: "warning",
                title: "图片过大",
                message: `${file.name} 超过 10MB，请压缩后重试。`,
                toastOnly: true,
              });
              continue;
            }

            if (isImage && !isGif) {
              try {
                const dims = await checkImageDimensions(file);
                if (dims.width > 8192 || dims.height > 8192) {
                  notificationStore.addNotification({
                    type: "warning",
                    title: "分辨率过高",
                    message: `${file.name} 的分辨率超过 8K，请压缩后重试。`,
                    toastOnly: true,
                  });
                  continue;
                }
              } catch (error) {
                console.warn(
                  "[AttachmentStore] Failed to check image dimensions:",
                  error,
                );
              }
            }

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
            if (isImage) preloadAttachmentImage(blobUrl);

            await nextTick();
            window.dispatchEvent(new Event("resize"));

            try {
              let finalData: any;
              if (file.size < 2 * 1024 * 1024) {
                const bytes = new Uint8Array(await file.arrayBuffer());
                finalData = await invoke<any>("store_file", {
                  originalName: file.name,
                  fileBytes: bytes,
                  mimeType: file.type || "application/octet-stream",
                });
              } else {
                const endpoint = await invoke<any>("prepare_vcp_upload", {
                  metadata: {
                    name: file.name,
                    mime: file.type || "application/octet-stream",
                    size: file.size,
                  },
                });
                finalData = await new Promise((resolveUpload, rejectUpload) => {
                  const xhr = new XMLHttpRequest();
                  xhr.open("POST", endpoint.url, true);
                  xhr.setRequestHeader(
                    "Content-Type",
                    "application/octet-stream",
                  );
                  xhr.setRequestHeader("X-Upload-Token", endpoint.token);
                  let lastUpdate = 0;
                  xhr.upload.onprogress = (event) => {
                    if (!event.lengthComputable || Date.now() - lastUpdate < 33)
                      return;
                    lastUpdate = Date.now();
                    const index = stagedAttachments.value.findIndex(
                      (item) => item.id === stableId,
                    );
                    if (index !== -1) {
                      stagedAttachments.value[index].progress = Math.round(
                        (event.loaded / event.total) * 100,
                      );
                    }
                  };
                  xhr.onload = () => {
                    if (xhr.status >= 200 && xhr.status < 300) {
                      try {
                        resolveUpload(JSON.parse(xhr.responseText));
                      } catch (error) {
                        rejectUpload(error);
                      }
                    } else {
                      rejectUpload(
                        new Error(`Upload failed with status ${xhr.status}`),
                      );
                    }
                  };
                  xhr.onerror = () =>
                    rejectUpload(new Error("XHR Network Error"));
                  xhr.send(file);
                });
              }

              if (!finalData) throw new Error("附件存储未返回结果");
              const index = stagedAttachments.value.findIndex(
                (item) => item.id === stableId,
              );
              if (index !== -1) {
                stagedAttachments.value[index] = {
                  ...stagedAttachments.value[index],
                  type: finalData.type,
                  src: finalData.internalPath,
                  internalPath: finalData.internalPath,
                  thumbnailPath: finalData.thumbnailPath,
                  name: finalData.name,
                  size: finalData.size,
                  hash: finalData.hash,
                  status: "done",
                  progress: undefined,
                };
                await resolveAttachmentAsset(stagedAttachments.value[index]);
              }
            } catch (error) {
              const index = stagedAttachments.value.findIndex(
                (item) => item.id === stableId,
              );
              if (index !== -1) stagedAttachments.value.splice(index, 1);
              processingErrors.push(
                `${file.name}: ${error instanceof Error ? error.message : String(error)}`,
              );
            } finally {
              URL.revokeObjectURL(blobUrl);
            }
          }

          if (processingErrors.length > 0) {
            notificationStore.addNotification({
              type: "warning",
              title: "部分附件处理失败",
              message: `${processingErrors.length} 个文件未能添加，其余文件已保留`,
              toastOnly: true,
            });
          }
          settleResolve();
        } catch (err) {
          console.error(
            "[AttachmentStore] Failed to pick or store attachment:",
            err,
          );
          settleReject(err);
        }
      };

      input.oncancel = () => {
        settleResolve();
      };

      window.addEventListener("focus", handleWindowFocus);
      input.click();
    });
  };

  const handleAttachment = async (
    mode: "camera" | "gallery" | "file" = "file",
  ) => {
    if (isPickingAttachment.value) return;
    isPickingAttachment.value = true;
    try {
      await handleAttachmentInternal(mode);
    } finally {
      isPickingAttachment.value = false;
    }
  };

  /**
   * 消息发送前的文档预处理 (JIT)
   */
  const preProcessDocuments = async (customList?: Attachment[]) => {
    const targetList = customList || stagedAttachments.value;
    if (targetList.length === 0) return;

    const docProcessor = useDocumentProcessor();
    for (const att of targetList) {
      const ext = att.name.split(".").pop()?.toLowerCase();
      // Only process documents and PDFs as requested
      if (["txt", "md", "csv", "json", "docx", "pdf"].includes(ext || "")) {
        try {
          const result = await docProcessor.processAttachment(att);
          if (result) {
            if (result.extractedText) att.extractedText = result.extractedText;
            if (result.imageFrames) att.imageFrames = result.imageFrames;
          }
        } catch (e) {
          console.error(
            `[AttachmentStore] JIT document processing failed for ${att.name}:`,
            e,
          );
        }
      }
    }
  };

  /**
   * 移除特定位置的暂存附件
   */
  const removeStaged = (index: number) => {
    if (index >= 0 && index < stagedAttachments.value.length) {
      const removed = stagedAttachments.value.splice(index, 1)[0];
      if (removed.hash) {
        invoke("cleanup_single_orphaned_attachment", {
          hash: removed.hash,
        }).catch((err) => {
          console.warn(
            `[AttachmentStore] Targeted GC failed for ${removed.name}:`,
            err,
          );
        });
      }
    }
  };

  /**
   * 清空暂存附件
   */
  const clearStaged = (performGc = false) => {
    const toClear = [...stagedAttachments.value];
    stagedAttachments.value = [];
    if (performGc) {
      toClear.forEach((att) => {
        if (att.hash) {
          invoke("cleanup_single_orphaned_attachment", {
            hash: att.hash,
          }).catch((err) => {
            console.warn(
              `[AttachmentStore] Targeted GC failed for ${att.name}:`,
              err,
            );
          });
        }
      });
    }
  };

  const consumeStaged = (attachmentIds: Iterable<string>) => {
    const consumedIds = new Set(attachmentIds);
    if (consumedIds.size === 0) return;
    stagedAttachments.value = stagedAttachments.value.filter(
      (attachment) =>
        typeof attachment.id !== "string" || !consumedIds.has(attachment.id),
    );
  };

  return {
    stagedAttachments,
    isPickingAttachment,
    handleAttachment,
    resolveMessageAssets,
    preProcessDocuments,
    removeStaged,
    clearStaged,
    consumeStaged,
  };
});
