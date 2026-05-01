import { invoke } from "@tauri-apps/api/core";

export interface SurfaceWidgetBounds {
  x: number;
  y: number;
  width: number;
  height: number;
  zIndex: number;
}

export type SurfaceWidgetSource = "desktopPush" | "mobileTool" | "user";

export interface SurfaceWidget {
  id: string;
  title?: string | null;
  html: string;
  bounds: SurfaceWidgetBounds;
  favorite: boolean;
  source: SurfaceWidgetSource;
  createdAt: number;
  updatedAt: number;
}

export interface UpsertSurfaceWidgetRequest {
  id?: string;
  title?: string | null;
  html: string;
  bounds?: SurfaceWidgetBounds;
  favorite?: boolean;
  source?: SurfaceWidgetSource;
}

export type SurfaceCommand =
  | { action: "create"; widgetId: string; options?: SurfaceWidgetBounds }
  | { action: "append"; widgetId: string; content: string }
  | { action: "finalize"; widgetId: string }
  | { action: "replace"; targetSelector?: string; content: string }
  | { action: "remove"; widgetId: string }
  | { action: "clear" };

type TauriSurfaceCommand =
  | { action: "create"; widgetId: string; options?: SurfaceWidgetBounds }
  | { action: "append"; widgetId: string; content: string }
  | { action: "finalize"; widgetId: string }
  | { action: "replace"; targetSelector?: string; content: string }
  | { action: "remove"; widgetId: string }
  | { action: "clear" };

const toTauriSurfaceCommand = (command: SurfaceCommand): TauriSurfaceCommand => {
  switch (command.action) {
    case "create":
      return {
        action: command.action,
        widgetId: command.widgetId,
        options: command.options,
      };
    case "append":
      return {
        action: command.action,
        widgetId: command.widgetId,
        content: command.content,
      };
    case "finalize":
      return {
        action: command.action,
        widgetId: command.widgetId,
      };
    case "replace":
      return {
        action: command.action,
        targetSelector: command.targetSelector,
        content: command.content,
      };
    case "remove":
      return {
        action: command.action,
        widgetId: command.widgetId,
      };
    case "clear":
      return command;
  }
};

export const surfaceBridge = {
  listWidgets() {
    return invoke<SurfaceWidget[]>("list_surface_widgets");
  },

  upsertWidget(request: UpsertSurfaceWidgetRequest) {
    return invoke<SurfaceWidget>("upsert_surface_widget", { request });
  },

  removeWidget(widgetId: string) {
    return invoke<boolean>("remove_surface_widget", { widgetId });
  },

  clearWidgets() {
    return invoke<void>("clear_surface_widgets");
  },

  applyCommand(command: SurfaceCommand) {
    return invoke<SurfaceWidget[]>("apply_surface_command", {
      command: toTauriSurfaceCommand(command),
    });
  },
};
