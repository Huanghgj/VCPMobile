import { invoke } from "@tauri-apps/api/core";

export interface BrowserLink {
  text: string;
  href: string;
}

export interface BrowserForm {
  id?: string | null;
  name?: string | null;
  action?: string | null;
  method?: string | null;
  fields: string[];
}

export interface BrowserSnapshot {
  status: string;
  controlMode: string;
  url?: string | null;
  title?: string | null;
  text?: string | null;
  links: BrowserLink[];
  forms: BrowserForm[];
  screenshot?: string | null;
  lastAction?: string | null;
  waitingReason?: string | null;
  bridgeKind: string;
  updatedAt: number;
}

export interface BrowserAction {
  action: string;
  url?: string;
  selector?: string;
  text?: string;
  value?: string;
  script?: string;
  direction?: "up" | "down" | "left" | "right";
  amount?: number;
  x?: number;
  y?: number;
  reason?: string;
}

export interface BrowserAssistInfo {
  running: boolean;
  bindLan: boolean;
  port: number;
  token: string;
  url: string;
  localUrl: string;
}

export const browserBridge = {
  executeAction(action: BrowserAction) {
    return invoke<{ status: string; action: string; result: unknown; snapshot: BrowserSnapshot }>(
      "browser_execute_action",
      { action },
    );
  },

  getSnapshot() {
    return invoke<BrowserSnapshot>("get_browser_snapshot");
  },

  startAssistServer(bindLan = false) {
    return invoke<BrowserAssistInfo>("start_browser_assist_server", { bindLan });
  },

  stopAssistServer() {
    return invoke<void>("stop_browser_assist_server");
  },
};
