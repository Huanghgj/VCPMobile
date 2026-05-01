import { defineStore } from 'pinia';
import { computed, onScopeDispose, ref } from 'vue';
import { useSettingsStore } from './settings';

export type RagObserverKind =
  | 'rag'
  | 'chain'
  | 'memo'
  | 'daily-note'
  | 'agent-chat'
  | 'agent-notice'
  | 'dream'
  | 'tool-approval'
  | 'tool-result'
  | 'tool-log'
  | 'video-status'
  | 'system'
  | 'notification'
  | 'unknown';

export type RagObserverFilter =
  | 'all'
  | 'rag'
  | 'chain'
  | 'chat'
  | 'memo'
  | 'dream'
  | 'tool'
  | 'notification'
  | 'unknown';

export interface RagObserverEvent {
  id: string;
  kind: RagObserverKind;
  type: string;
  title: string;
  summary: string;
  timestamp: number;
  payload: any;
}

const MAX_EVENTS = 200;
const RECONNECT_BASE_MS = 1500;
const RECONNECT_MAX_MS = 15000;

const normalizeWsBase = (url: string) => {
  const trimmed = url.trim().replace(/\/+$/, '');
  if (!trimmed) return '';
  return trimmed
    .replace(/^http:\/\//i, 'ws://')
    .replace(/^https:\/\//i, 'wss://')
    .replace(/\/(?:VCPlog|vcpinfo)(?:\/VCP_Key=.*)?$/i, '');
};

const safeJson = (value: any) => {
  try {
    return JSON.stringify(value, null, 2);
  } catch {
    return String(value);
  }
};

const toText = (value: any, fallback = '') => {
  if (value === undefined || value === null) return fallback;
  if (typeof value === 'string') return value;
  return safeJson(value);
};

const compact = (value: any, limit = 160) => {
  const normalized = toText(value).replace(/\s+/g, ' ').trim();
  return normalized.length > limit ? `${normalized.slice(0, limit)}...` : normalized;
};

const getPayloadData = (payload: any) =>
  payload?.data && typeof payload.data === 'object' ? payload.data : {};

const getKind = (payload: any): RagObserverKind => {
  const type = String(payload?.type || '');
  if (type === 'RAG_RETRIEVAL_DETAILS') return 'rag';
  if (type === 'META_THINKING_CHAIN') return 'chain';
  if (type === 'AI_MEMO_RETRIEVAL') return 'memo';
  if (type === 'DailyNote' || type === 'daily_note_recall_result') return 'daily-note';
  if (type === 'AGENT_PRIVATE_CHAT_PREVIEW') return 'agent-chat';
  if (type.startsWith('AGENT_DREAM_')) return 'dream';
  if (type === 'tool_approval_request') return 'tool-approval';
  if (type === 'tool_result') return 'tool-result';
  if (type === 'vcp_log' || type === 'vcp-log-message') return 'tool-log';
  if (type === 'video_generation_status') return 'video-status';
  if (type === 'daily_note_created') return 'daily-note';
  if (type === 'notification') return 'notification';
  if (type === 'error' || type === 'connection_ack' || type === 'vcp_log_status' || type === 'vcp-log-status' || type === 'connection_status') {
    return 'system';
  }
  if (payload?.source === 'AgentAssistant') return 'agent-notice';
  return 'unknown';
};

const getTitle = (payload: any, kind: RagObserverKind) => {
  const data = getPayloadData(payload);
  if (kind === 'rag') return `RAG 检索：${payload.dbName || 'Unknown'}`;
  if (kind === 'chain') return `召回深度：${payload.chainName || 'Unknown'}`;
  if (kind === 'memo') return `记忆回溯：${payload.mode || 'memo'}`;
  if (kind === 'daily-note') return `日记召回：${payload.dbName || payload.query || 'DailyNote'}`;
  if (kind === 'agent-chat') return `Agent 私聊：${payload.agentName || 'Unknown'}`;
  if (kind === 'agent-notice') return payload.title || 'Agent 助手通知';
  if (kind === 'dream') return `Agent 剧情：${payload.agentName || payload.type || 'Dream'}`;
  if (kind === 'tool-approval') return `工具审核：${data.maid || '未知助手'} -> ${data.toolName || '未知工具'}`;
  if (kind === 'tool-result') return `工具结果：${data.toolName || data.tool_name || data.name || payload.toolName || 'VCP 工具'}`;
  if (kind === 'tool-log') return `VCP 日志：${data.tool_name || data.source || 'Tool'}`;
  if (kind === 'video-status') return `视频生成：${data.status || payload.status || 'status'}`;
  if (kind === 'system') return payload.type === 'error' ? 'VCPInfo 错误' : `系统事件：${payload.type || 'system'}`;
  if (kind === 'notification') return payload.title || `通知：${payload.type || 'notification'}`;
  return payload.title || payload.type || 'VCPInfo';
};

const getSummary = (payload: any, kind: RagObserverKind) => {
  const data = getPayloadData(payload);
  if (kind === 'rag') {
    const results = Array.isArray(payload.results) ? payload.results : [];
    return [
      `查询：${compact(payload.query, 96) || 'N/A'}`,
      `K=${payload.k ?? 'N/A'} · 结果=${results.length} · 耗时=${payload.useTime ?? 'N/A'}`,
    ].join('\n');
  }

  if (kind === 'chain') {
    const stages = Array.isArray(payload.stages) ? payload.stages : [];
    return `查询：${compact(payload.query, 96) || 'N/A'}\n阶段=${stages.length}`;
  }

  if (kind === 'memo') {
    return `日记数=${payload.diaryCount ?? 'N/A'}\n${compact(payload.extractedMemories || payload.message || payload.data, 140)}`;
  }

  if (kind === 'daily-note') {
    const notes = Array.isArray(payload.notes) ? payload.notes : Array.isArray(data?.notes) ? data.notes : [];
    return notes.length ? `召回 ${notes.length} 条日记` : compact(payload.message || payload.data || payload, 140);
  }

  if (kind === 'tool-approval') {
    return `${data.maid || '未知助手'} 请求调用 ${data.toolName || '未知工具'}\n${compact(data.args?.command || data.args || data, 140)}`;
  }

  if (kind === 'tool-result') {
    const result = data.result ?? data.output ?? data.content ?? data.message ?? data.error ?? data;
    const status = data.status || (data.error ? 'error' : 'success');
    return `状态=${status}\n${compact(result, 160)}`;
  }

  if (kind === 'tool-log') {
    const content = data.content ?? payload.content ?? payload.message;
    return [
      data.status ? `状态=${data.status}` : '',
      data.source ? `来源=${data.source}` : '',
      compact(content || data || payload, 160),
    ].filter(Boolean).join('\n');
  }

  if (kind === 'video-status') {
    const message = data?.original_plugin_output?.message || data?.message || payload.message || data;
    return `状态=${data.status || payload.status || 'N/A'}\n${compact(message, 160)}`;
  }

  if (kind === 'system' || kind === 'notification') {
    return compact(payload.message || data.message || data || payload, 180);
  }

  return compact(payload.message || payload.response || payload.query || payload.data || payload, 180);
};

const getFilterForKind = (kind: RagObserverKind): RagObserverFilter => {
  if (kind === 'agent-chat' || kind === 'agent-notice') return 'chat';
  if (kind === 'daily-note') return 'memo';
  if (kind === 'tool-approval' || kind === 'tool-result' || kind === 'tool-log' || kind === 'video-status') return 'tool';
  if (kind === 'notification' || kind === 'system') return 'notification';
  if (kind === 'rag' || kind === 'chain' || kind === 'memo' || kind === 'dream' || kind === 'unknown') return kind;
  return 'unknown';
};

export const useRagObserverStore = defineStore('ragObserver', () => {
  const events = ref<RagObserverEvent[]>([]);
  const status = ref<'idle' | 'connecting' | 'connected' | 'error' | 'closed'>('idle');
  const statusMessage = ref('未连接');
  const unreadCount = ref(0);
  const isOpen = ref(false);
  const activeFilter = ref<RagObserverFilter>('all');

  let ws: WebSocket | null = null;
  let reconnectTimer: number | null = null;
  let reconnectAttempts = 0;
  let manualClose = false;

  const latestEvents = computed(() => events.value);
  const filteredEvents = computed(() => {
    if (activeFilter.value === 'all') return events.value;
    return events.value.filter((event) => getFilterForKind(event.kind) === activeFilter.value);
  });
  const isConnected = computed(() => status.value === 'connected');

  const clearReconnectTimer = () => {
    if (reconnectTimer !== null) {
      window.clearTimeout(reconnectTimer);
      reconnectTimer = null;
    }
  };

  const buildUrl = async () => {
    const settingsStore = useSettingsStore();
    if (!settingsStore.settings) {
      await settingsStore.fetchSettings();
    }

    const settings = settingsStore.settings;
    const base = normalizeWsBase(settings?.vcpLogUrl || settings?.vcpServerUrl || '');
    const key = settings?.vcpLogKey || settings?.vcpApiKey || '';
    if (!base || !key) return '';
    return `${base}/vcpinfo/VCP_Key=${encodeURIComponent(key)}`;
  };

  const pushEvent = (payload: any) => {
    const kind = getKind(payload);

    const timestamp = Date.now();
    const event: RagObserverEvent = {
      id: `${timestamp}_${Math.random().toString(36).slice(2, 8)}`,
      kind,
      type: String(payload?.type || payload?.source || 'unknown'),
      title: getTitle(payload, kind),
      summary: getSummary(payload, kind),
      timestamp,
      payload,
    };

    events.value.unshift(event);
    if (events.value.length > MAX_EVENTS) {
      events.value.length = MAX_EVENTS;
    }

    if (!isOpen.value) {
      unreadCount.value = Math.min(999, unreadCount.value + 1);
    }
  };

  const scheduleReconnect = () => {
    if (manualClose || reconnectTimer !== null) return;
    const delay = Math.min(RECONNECT_MAX_MS, RECONNECT_BASE_MS * 2 ** reconnectAttempts);
    reconnectAttempts += 1;
    status.value = 'closed';
    statusMessage.value = `连接已断开，${Math.round(delay / 1000)}s 后重连`;
    reconnectTimer = window.setTimeout(() => {
      reconnectTimer = null;
      void connect();
    }, delay);
  };

  const closeSocket = () => {
    if (!ws) return;
    const current = ws;
    ws = null;
    current.onopen = null;
    current.onmessage = null;
    current.onerror = null;
    current.onclose = null;
    current.close();
  };

  const connect = async () => {
    clearReconnectTimer();
    manualClose = false;

    if (ws && (ws.readyState === WebSocket.OPEN || ws.readyState === WebSocket.CONNECTING)) {
      return;
    }

    const url = await buildUrl();
    if (!url) {
      status.value = 'error';
      statusMessage.value = '未配置 VCPInfo 地址或 Key';
      return;
    }

    closeSocket();
    status.value = 'connecting';
    statusMessage.value = '正在连接 VCPInfo...';

    try {
      ws = new WebSocket(url);
    } catch (error) {
      status.value = 'error';
      statusMessage.value = String(error);
      scheduleReconnect();
      return;
    }

    ws.onopen = () => {
      reconnectAttempts = 0;
      status.value = 'connected';
      statusMessage.value = 'VCPInfo 已连接';
    };

    ws.onmessage = (event) => {
      try {
        pushEvent(JSON.parse(event.data));
      } catch (error) {
        console.warn('[RagObserver] Failed to parse vcpinfo payload:', error);
      }
    };

    ws.onerror = () => {
      status.value = 'error';
      statusMessage.value = 'VCPInfo 连接错误';
    };

    ws.onclose = () => {
      if (manualClose) {
        status.value = 'closed';
        statusMessage.value = 'VCPInfo 已断开';
        return;
      }
      scheduleReconnect();
    };
  };

  const disconnect = () => {
    manualClose = true;
    clearReconnectTimer();
    closeSocket();
    status.value = 'closed';
    statusMessage.value = 'VCPInfo 已断开';
  };

  const open = () => {
    isOpen.value = true;
    unreadCount.value = 0;
    void connect();
  };

  const close = () => {
    isOpen.value = false;
  };

  const clear = () => {
    events.value = [];
    unreadCount.value = 0;
  };

  const setFilter = (filter: RagObserverFilter) => {
    activeFilter.value = filter;
  };

  onScopeDispose(() => {
    disconnect();
  });

  return {
    events,
    latestEvents,
    filteredEvents,
    status,
    statusMessage,
    unreadCount,
    isOpen,
    isConnected,
    activeFilter,
    connect,
    disconnect,
    open,
    close,
    clear,
    setFilter,
  };
});
