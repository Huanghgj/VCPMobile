import { ref, onScopeDispose } from 'vue';
import { Copy, Check } from 'lucide-vue-next';
import type { VcpNotification } from '../../../core/stores/notification';

export const useNotificationClipboard = () => {
  const copiedId = ref<string | null>(null);
  let copyTimer: ReturnType<typeof setTimeout> | null = null;

  const buildCopyText = (item: VcpNotification) => {
    if (item.rawPayload) return JSON.stringify(item.rawPayload, null, 2);

    const lines = [item.title];
    if (item.subtitle) lines.push(item.subtitle);
    if (item.message) lines.push('', item.message);
    if (item.meta?.length) {
      lines.push('', ...item.meta.map((entry) => `${entry.label}: ${entry.value}`));
    }
    if (item.details?.length) {
      lines.push('', ...item.details.map((detail) => `${detail.label}:\n${detail.value}`));
    }
    return lines.join('\n');
  };

  const copyContent = async (item: VcpNotification) => {
    try {
      await navigator.clipboard.writeText(buildCopyText(item));
      copiedId.value = item.id;

      if (copyTimer) clearTimeout(copyTimer);
      copyTimer = window.setTimeout(() => {
        if (copiedId.value === item.id) {
          copiedId.value = null;
        }
        copyTimer = null;
      }, 2000);
    } catch (error) {
      console.error('[useNotificationClipboard] Copy failed:', error);
    }
  };

  onScopeDispose(() => {
    if (copyTimer) clearTimeout(copyTimer);
  });

  const getCopyIcon = (itemId: string) => copiedId.value === itemId ? Check : Copy;

  return {
    copiedId,
    buildCopyText,
    copyContent,
    getCopyIcon
  };
};
