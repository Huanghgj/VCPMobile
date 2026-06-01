import { computed, ref } from 'vue';
import type { VcpNotification } from '../../../core/stores/notification';

export type CategoryTab =
  | 'all'
  | 'unread'
  | 'rag'
  | 'meta'
  | 'dream'
  | 'agent'
  | 'approval'
  | 'tool'
  | 'daily'
  | 'video'
  | 'system'
  | 'error'
  | 'other';

export interface TabDefinition {
  id: CategoryTab;
  label: string;
  count: number;
}

export function useNotificationGrouping(historyList: () => VcpNotification[]) {
  const activeTab = ref<CategoryTab>('all');
  const searchQuery = ref('');

  const matchesSearch = (item: VcpNotification, query: string): boolean => {
    const q = query.toLowerCase().trim();
    if (!q) return true;

    const checkValue = (val: any): boolean => {
      if (val === null || val === undefined) return false;
      if (typeof val === 'string') return val.toLowerCase().includes(q);
      if (typeof val === 'number' || typeof val === 'boolean') return String(val).toLowerCase().includes(q);
      if (Array.isArray(val)) return val.some(item => checkValue(item));
      if (typeof val === 'object') return Object.values(val).some(item => checkValue(item));
      return false;
    };

    return (
      checkValue(item.title) ||
      checkValue(item.message) ||
      checkValue(item.subtitle) ||
      checkValue(item.source) ||
      checkValue(item.category) ||
      checkValue(item.infoType) ||
      checkValue(item.tags) ||
      checkValue(item.meta) ||
      checkValue(item.details) ||
      checkValue(item.structured) ||
      checkValue(item.rawPayload)
    );
  };

  const getNotificationCategory = (item: VcpNotification): CategoryTab => {
    const cat = (item.category || '').toLowerCase();
    const infoType = (item.infoType || '').toLowerCase();
    const structKind = item.structured?.kind;
    const rawType = item.rawPayload?.type;
    const type = item.type;

    if (type === 'error' || cat === 'error') return 'error';
    if (rawType === 'tool_approval_request' || infoType === 'tool_approval_request' || (item.actions && item.actions.length > 0)) return 'approval';
    if (cat === 'rag' || structKind === 'rag') return 'rag';
    if (cat === 'meta' || structKind === 'thinking') return 'meta';
    if (cat === 'dream' || structKind === 'dream') return 'dream';
    if (cat === 'agent' || structKind === 'private_chat') return 'agent';
    if (cat === 'dailynote' || infoType === 'daily_note_created') return 'daily';
    if (cat === 'video' || infoType === 'video_generation_status') return 'video';
    if (cat === 'tool' || type === 'tool' || infoType.startsWith('tool_')) return 'tool';
    if (
      cat === 'system' ||
      infoType === 'connection_ack' ||
      rawType === 'vcp-core-status' ||
      rawType === 'vcp-log-status' ||
      rawType === 'vcp-info-status' ||
      rawType === 'vcp-info-message'
    ) return 'system';

    return 'other';
  };

  const tabs = computed<TabDefinition[]>(() => {
    const list = historyList();
    const counts: Record<CategoryTab, number> = {
      all: list.length,
      unread: list.filter(n => !n.read).length,
      rag: 0,
      meta: 0,
      dream: 0,
      agent: 0,
      approval: 0,
      tool: 0,
      daily: 0,
      video: 0,
      system: 0,
      error: 0,
      other: 0
    };

    list.forEach(item => {
      const cat = getNotificationCategory(item);
      if (cat !== 'all' && cat !== 'unread') {
        counts[cat] = (counts[cat] || 0) + 1;
      }
    });

    return [
      { id: 'all', label: '全部', count: counts.all },
      { id: 'unread', label: '未读', count: counts.unread },
      { id: 'rag', label: 'RAG', count: counts.rag },
      { id: 'meta', label: '元思考', count: counts.meta },
      { id: 'dream', label: '梦境', count: counts.dream },
      { id: 'agent', label: '智能体', count: counts.agent },
      { id: 'approval', label: '审批', count: counts.approval },
      { id: 'tool', label: '工具', count: counts.tool },
      { id: 'daily', label: '日记', count: counts.daily },
      { id: 'video', label: '视频', count: counts.video },
      { id: 'system', label: '系统', count: counts.system },
      { id: 'error', label: '异常', count: counts.error },
      { id: 'other', label: '其他', count: counts.other }
    ];
  });

  const filteredNotifications = computed(() => {
    const list = historyList();
    const query = searchQuery.value;
    const tab = activeTab.value;

    return list.filter(item => {
      if (!matchesSearch(item, query)) return false;
      if (tab === 'all') return true;
      if (tab === 'unread') return !item.read;
      return getNotificationCategory(item) === tab;
    });
  });

  return {
    activeTab,
    searchQuery,
    tabs,
    filteredNotifications
  };
}
