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

export function stopStreamService(agentName: string): Promise<void> {
  return invoke("plugin:vcp-mobile|stop_streaming_service", { agentName });
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

export function pickFile(mode = "file"): Promise<PickedFile> {
  return invoke<PickedFile>("plugin:vcp-mobile|pick_file", { mode });
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

export function saveImageFromPath(
  imagePath: string,
  fileName?: string,
): Promise<GallerySaveResult> {
  return invoke<GallerySaveResult>("plugin:vcp-mobile|save_image_from_path", {
    imagePath,
    fileName,
  });
}

export function writeTempFile(
  bytes: Uint8Array,
  fileName: string,
): Promise<string> {
  return invoke<string>("plugin:vcp-mobile|write_temp_file", {
    bytes: Array.from(bytes),
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

// ==================================================================
// Floating Assistant & Update Notifications
// ==================================================================

export function requestOverlayPermission(): Promise<void> {
  return invoke("plugin:vcp-mobile|request_overlay_permission");
}

export function toggleFloatingBall(show: boolean): Promise<boolean> {
  return invoke<boolean>("plugin:vcp-mobile|toggle_floating_ball", { show });
}

export function startDownloadNotification(): Promise<void> {
  return invoke("plugin:vcp-mobile|start_download_notification");
}

export function updateDownloadNotification(
  progress: number,
  text?: string,
): Promise<void> {
  return invoke("plugin:vcp-mobile|update_download_notification", {
    progress,
    text,
  });
}

export function cancelDownloadNotification(): Promise<void> {
  return invoke("plugin:vcp-mobile|cancel_download_notification");
}

export interface SharedFileItem {
  cachePath: string;
  mimeType: string;
  fileName: string;
}

export function registerSharedFiles(
  files: SharedFileItem[],
): Promise<PickedFile[]> {
  return invoke<PickedFile[]>("plugin:vcp-mobile|register_shared_files", {
    files,
  });
}

// ==================================================================
// Hardware Status & Root Access
// ==================================================================

export interface RootAccessStatus {
  isRoot: boolean;
}

export function checkRootAccess(): Promise<RootAccessStatus> {
  return invoke<RootAccessStatus>("plugin:vcp-mobile|check_root_access");
}

export interface RootCommandResult {
  success: boolean;
  output: string;
}

export function runRootCommand(
  command: string,
  timeoutMs = 1500,
): Promise<RootCommandResult> {
  return invoke<RootCommandResult>("plugin:vcp-mobile|run_root_command", {
    command,
    timeoutMs,
  });
}

export interface LaunchRootManagerResult {
  success: boolean;
  manager?: string;
  message?: string;
}

export function launchRootManager(): Promise<LaunchRootManagerResult> {
  return invoke<LaunchRootManagerResult>(
    "plugin:vcp-mobile|launch_root_manager",
  );
}
