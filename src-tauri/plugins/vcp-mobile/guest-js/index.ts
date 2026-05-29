import { invoke } from "@tauri-apps/api/core";

// ==================================================================
// Screen
// ==================================================================

export function setKeepScreenOn(): Promise<void> {
  return invoke("plugin:vcp-mobile|set_keep_screen_on");
}

export function clearKeepScreenOn(): Promise<void> {
  return invoke("plugin:vcp-mobile|clear_keep_screen_on");
}

// ==================================================================
// Stream Service
// ==================================================================

export function startStreamService(agentName: string): Promise<void> {
  return invoke("plugin:vcp-mobile|start_streaming_service", { agentName });
}

export function stopStreamService(): Promise<void> {
  return invoke("plugin:vcp-mobile|stop_streaming_service");
}

// ==================================================================
// Native File Picker
// ==================================================================

export interface PickedFile {
  path: string;
  name: string;
  mime: string;
  size: number;
  hash: string;
  thumbnailPath?: string;
}

export function pickFile(): Promise<PickedFile> {
  return invoke<PickedFile>("plugin:vcp-mobile|pick_file");
}

export function openFileNative(path: string): Promise<void> {
  return invoke("plugin:vcp-mobile|open_file_native", { path });
}

export interface GallerySaveResult {
  uri: string;
  displayName: string;
  mimeType: string;
  size: number;
}

export function saveImageToGallery(
  sourceUrl: string,
  fileName?: string,
): Promise<GallerySaveResult> {
  return invoke<GallerySaveResult>("plugin:vcp-mobile|save_image_to_gallery", {
    sourceUrl,
    fileName,
  });
}

// ==================================================================
// Window Snapshot
// ==================================================================

export interface WindowSnapshot {
  dataUrl: string;
  width: number;
  height: number;
}

export interface CaptureWindowSnapshotOptions {
  maxWidth?: number;
  quality?: number;
}

export function captureWindowSnapshot(
  options: CaptureWindowSnapshotOptions = {},
): Promise<WindowSnapshot> {
  const args: Record<string, unknown> = { ...options };
  return invoke<WindowSnapshot>(
    "plugin:vcp-mobile|capture_window_snapshot",
    args,
  );
}
