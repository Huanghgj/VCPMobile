export interface OverlayActionItem {
  label: string;
  icon?: any; // lucide-vue-next component
  danger?: boolean;
  disabled?: boolean;
  handler: () => void | Promise<void>;
}

export interface ContextMenuConfig {
  title: string;
  actions: OverlayActionItem[];
}

export interface PromptConfig {
  title: string;
  initialValue: string;
  placeholder: string;
  onConfirm: (val: string) => void;
}

export interface EditorConfig {
  initialValue: string;
  onSave: (newContent: string) => void;
}

export interface MediaViewerConfig {
  src: string;
  originalSrc?: string;
  mediaType: 'image' | 'video' | 'audio';
  title?: string;
  mimeType?: string;
}
