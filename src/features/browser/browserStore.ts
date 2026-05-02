import { defineStore } from "pinia";
import { ref } from "vue";
import {
  browserBridge,
  type BrowserAction,
  type BrowserAssistInfo,
  type BrowserSnapshot,
} from "../../core/api/browserBridge";

export const useBrowserStore = defineStore("browser", () => {
  const snapshot = ref<BrowserSnapshot | null>(null);
  const assist = ref<BrowserAssistInfo | null>(null);
  const loading = ref(false);
  const lastError = ref<string | null>(null);
  const lastResult = ref<unknown>(null);

  const setError = (error: unknown) => {
    lastError.value = error instanceof Error ? error.message : String(error);
  };

  const refreshSnapshot = async () => {
    lastError.value = null;
    try {
      snapshot.value = await browserBridge.getSnapshot();
      return snapshot.value;
    } catch (error) {
      setError(error);
      throw error;
    }
  };

  const executeAction = async (action: BrowserAction) => {
    loading.value = true;
    lastError.value = null;
    try {
      const result = await browserBridge.executeAction(action);
      snapshot.value = result.snapshot;
      lastResult.value = result;
      return result;
    } catch (error) {
      setError(error);
      throw error;
    } finally {
      loading.value = false;
    }
  };

  const startAssist = async (bindLan = false) => {
    loading.value = true;
    lastError.value = null;
    try {
      assist.value = await browserBridge.startAssistServer(bindLan);
      return assist.value;
    } catch (error) {
      setError(error);
      throw error;
    } finally {
      loading.value = false;
    }
  };

  const stopAssist = async () => {
    loading.value = true;
    lastError.value = null;
    try {
      await browserBridge.stopAssistServer();
      assist.value = null;
    } catch (error) {
      setError(error);
      throw error;
    } finally {
      loading.value = false;
    }
  };

  return {
    snapshot,
    assist,
    loading,
    lastError,
    lastResult,
    refreshSnapshot,
    executeAction,
    startAssist,
    stopAssist,
  };
});
