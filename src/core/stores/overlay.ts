import { defineStore } from 'pinia';
import { computed, ref, shallowRef } from 'vue';
import { useModalHistory } from '../composables/useModalHistory';
import type {
  OverlayActionItem,
  ContextMenuConfig,
  PromptConfig,
  EditorConfig,
  MediaViewerConfig,
} from '../types/overlay';

interface PageStackItem {
  type: string;
  id?: string;
  modalId: string;
}

export const useOverlayStore = defineStore('overlay', () => {
  const { registerModal, unregisterModal, unregisterModalSilently } = useModalHistory();

  const promptConfig = ref<PromptConfig | null>(null);
  const contextMenuConfig = shallowRef<ContextMenuConfig | null>(null);
  const editorConfig = ref<EditorConfig | null>(null);
  const mediaViewerConfig = ref<MediaViewerConfig | null>(null);

  const pageStack = ref<PageStackItem[]>([]);
  const pageStackTop = computed(() => pageStack.value[pageStack.value.length - 1] || null);

  const isSettingsOpen = computed(() => pageStack.value.some(p => p.type === 'settings'));
  const isToolboxOpen = computed(() => pageStack.value.some(p => p.type === 'toolbox'));
  const isRagObserverOpen = computed(() => pageStack.value.some(p => p.type === 'ragObserver'));
  const isBrowserOpen = computed(() => pageStack.value.some(p => p.type === 'browser'));
  const isAgentSettingsOpen = computed(() => pageStack.value.some(p => p.type === 'agentSettings'));
  const isGroupSettingsOpen = computed(() => pageStack.value.some(p => p.type === 'groupSettings'));

  const agentSettingsId = computed(() => {
    const page = pageStack.value.find(p => p.type === 'agentSettings');
    return page?.id || '';
  });

  const groupSettingsId = computed(() => {
    const page = pageStack.value.find(p => p.type === 'groupSettings');
    return page?.id || '';
  });

  const getPageZIndex = (type: string) => {
    const index = pageStack.value.findIndex(p => p.type === type);
    return index === -1 ? 50 : 50 + index;
  };

  const pushPage = (type: string, id?: string) => {
    const modalId = `Page:${type}:${id || ''}`;
    const top = pageStack.value[pageStack.value.length - 1];
    if (top && top.type === type && top.id === id) return;

    pageStack.value.push({ type, id, modalId });
    registerModal(modalId, () => {
      popPageInternal();
    });
  };

  const popPageInternal = () => {
    if (pageStack.value.length === 0) return;
    pageStack.value.pop();
  };

  const popPage = () => {
    if (pageStack.value.length === 0) return;
    const top = pageStack.value[pageStack.value.length - 1];
    unregisterModal(top.modalId);
    pageStack.value.pop();
  };

  const closePageByType = (type: string) => {
    const index = pageStack.value.findIndex(p => p.type === type);
    if (index === -1) return;
    const [page] = pageStack.value.splice(index, 1);
    unregisterModal(page.modalId);
  };

  const closePageByTypeSilently = (type: string) => {
    const index = pageStack.value.findIndex(p => p.type === type);
    if (index === -1) return;
    const [page] = pageStack.value.splice(index, 1);
    unregisterModalSilently(page.modalId);
  };

  const popToRoot = () => {
    while (pageStack.value.length > 0) {
      const top = pageStack.value[pageStack.value.length - 1];
      unregisterModal(top.modalId);
      pageStack.value.pop();
    }
  };

  const openSettings = () => pushPage('settings');
  const closeSettings = () => closePageByType('settings');
  const closeSettingsSilently = () => closePageByTypeSilently('settings');

  const openToolbox = () => pushPage('toolbox');
  const closeToolbox = () => closePageByType('toolbox');

  const openRagObserver = () => pushPage('ragObserver');
  const closeRagObserver = () => closePageByType('ragObserver');

  const openBrowser = () => pushPage('browser');
  const closeBrowser = () => closePageByType('browser');

  const openAgentSettings = (id: string) => pushPage('agentSettings', id);
  const closeAgentSettings = () => closePageByType('agentSettings');

  const openGroupSettings = (id: string) => pushPage('groupSettings', id);
  const closeGroupSettings = () => closePageByType('groupSettings');

  const openPrompt = (config: PromptConfig) => {
    promptConfig.value = config;
    registerModal('Prompt', () => { promptConfig.value = null; });
  };

  const closePrompt = () => {
    if (promptConfig.value) {
      unregisterModal('Prompt');
      promptConfig.value = null;
    }
  };

  const openContextMenu = (actions: OverlayActionItem[], title?: string) => {
    contextMenuConfig.value = {
      title: title || '',
      actions
    };
    registerModal('ContextMenu', () => { contextMenuConfig.value = null; });
  };

  const closeContextMenu = () => {
    if (contextMenuConfig.value) {
      unregisterModal('ContextMenu');
      contextMenuConfig.value = null;
    }
  };

  const openEditor = (config: EditorConfig) => {
    editorConfig.value = config;
    registerModal('FullScreenEditor', () => { editorConfig.value = null; });
  };

  const closeEditor = () => {
    if (editorConfig.value) {
      unregisterModal('FullScreenEditor');
      editorConfig.value = null;
    }
  };

  const openMediaViewer = (config: MediaViewerConfig) => {
    mediaViewerConfig.value = config;
    registerModal('MediaViewer', () => { mediaViewerConfig.value = null; });
  };

  const closeMediaViewer = () => {
    if (mediaViewerConfig.value) {
      unregisterModal('MediaViewer');
      mediaViewerConfig.value = null;
    }
  };

  return {
    pageStack,
    pageStackTop,
    getPageZIndex,
    pushPage,
    popPage,
    popToRoot,
    promptConfig,
    contextMenuConfig,
    editorConfig,
    mediaViewerConfig,
    isSettingsOpen,
    isToolboxOpen,
    isRagObserverOpen,
    isBrowserOpen,
    isAgentSettingsOpen,
    agentSettingsId,
    isGroupSettingsOpen,
    groupSettingsId,
    openSettings,
    closeSettings,
    closeSettingsSilently,
    openToolbox,
    closeToolbox,
    openRagObserver,
    closeRagObserver,
    openBrowser,
    closeBrowser,
    openAgentSettings,
    closeAgentSettings,
    openGroupSettings,
    closeGroupSettings,
    openPrompt,
    closePrompt,
    openContextMenu,
    closeContextMenu,
    openEditor,
    closeEditor,
    openMediaViewer,
    closeMediaViewer,
  };
});
