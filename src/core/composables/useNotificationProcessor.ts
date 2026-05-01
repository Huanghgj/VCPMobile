import {
  VcpNotification,
  useNotificationStore,
  VcpObserverKind,
  VcpStatus
} from '../stores/notification';

/**
 * 过滤结果接口
 * action: 'show' 展示, 'hide' 拦截 (不推入 notificationStore)
 * duration: 可选覆盖默认显示时长
 */
export interface FilterResult {
  action: 'show' | 'hide';
  duration?: number;
  ruleName?: string;
}

/**
 * 过滤规则接口
 * match: 返回 true 表示命中规则
 */
export interface FilterRule {
  name: string;
  match: (title: string, message: string, payload: any) => boolean;
  action: 'show' | 'hide';
  duration?: number;
}

export function useNotificationProcessor() {
  const store = useNotificationStore();

  const safeStringify = (value: any) => {
    try {
      return JSON.stringify(value, null, 2);
    } catch {
      return String(value);
    }
  };

  const toText = (value: any, fallback = '') => {
    if (value === undefined || value === null) return fallback;
    if (typeof value === 'string') return value;
    return safeStringify(value);
  };

  const summarize = (value: string, limit = 120) => {
    const compact = String(value || '').replace(/\s+/g, ' ').trim();
    return compact.length > limit ? `${compact.slice(0, limit)}...` : compact;
  };

  const isLegacyVcpLogPayload = (payload: any) =>
    payload?.type === 'vcp_log' || payload?.type === 'vcp-log-message';

  const getPayloadSource = (payload: any) =>
    String(payload?.data?.source || payload?.source || '');

  const shouldKeepLegacyLogNotification = (payload: any) => {
    const source = getPayloadSource(payload);
    return source === 'Sync' || payload?.data?.id === 'vcp_sync_connection_status';
  };

  const shouldRouteToVcpInfoOnly = (payload: any) => {
    const payloadType = String(payload?.type || '');
    if (isLegacyVcpLogPayload(payload)) return !shouldKeepLegacyLogNotification(payload);
    if (payloadType.startsWith('AGENT_DREAM_')) return true;
    if (payload?.source === 'AgentAssistant') return true;

    return [
      'RAG_RETRIEVAL_DETAILS',
      'META_THINKING_CHAIN',
      'AI_MEMO_RETRIEVAL',
      'DailyNote',
      'daily_note_recall_result',
      'daily_note_created',
      'AGENT_PRIVATE_CHAT_PREVIEW',
      'tool_result',
      'video_generation_status',
    ].includes(payloadType);
  };

  const normalizeRealtimeSource = (source: any) => {
    const text = String(source || '').trim();
    if (!text || text === 'VCPLog') return '实时通道';
    return text;
  };

  const normalizeRealtimeMessage = (message: any, fallback = '状态未知') =>
    toText(message, fallback).replace(/VCPLog/g, '实时通道');

  const formatNote = (note: any, index: number) => {
    const title = note?.title || note?.fileName || note?.filename || note?.path || note?.file || `结果 ${index + 1}`;
    const content = note?.content || note?.snippet || note?.text || note?.preview || note?.message;
    const meta = [
      note?.score !== undefined ? `score=${note.score}` : '',
      note?.source ? `source=${note.source}` : '',
      note?.folder ? `folder=${note.folder}` : '',
    ].filter(Boolean).join(' · ');
    return [`${index + 1}. ${title}`, meta, toText(content || note)].filter(Boolean).join('\n');
  };

  const getObserverKind = (payload: any): VcpObserverKind | undefined => {
    const payloadType = String(payload?.type || '');
    if (payloadType === 'RAG_RETRIEVAL_DETAILS') return 'rag';
    if (payloadType === 'META_THINKING_CHAIN') return 'chain';
    if (payloadType === 'AI_MEMO_RETRIEVAL') return 'memo';
    if (payloadType === 'DailyNote' || payloadType === 'daily_note_recall_result') return 'daily-note';
    if (payloadType.startsWith('AGENT_DREAM_')) return 'dream';
    if (payloadType === 'AGENT_PRIVATE_CHAT_PREVIEW') return 'agent-chat';
    if (payloadType === 'tool_approval_request') return 'tool-approval';
    if (payloadType === 'tool_result') return 'tool-result';
    if (payloadType === 'vcp_log' || payloadType === 'vcp-log-message') return 'tool-log';
    if (payloadType === 'video_generation_status') return 'video-status';
    if (payloadType === 'notification') return 'notification';
    if (payloadType === 'error' || payloadType === 'connection_ack' || payloadType === 'vcp_log_status' || payloadType === 'vcp-log-status' || payloadType === 'connection_status') return 'system';
    if (payload?.source === 'AgentAssistant') return 'agent-notice';
    return undefined;
  };

  const formatRagPayload = (payload: any, expanded = false) => {
    if (payload.type === 'RAG_RETRIEVAL_DETAILS') {
      const results = Array.isArray(payload.results) ? payload.results : [];
      if (!expanded) {
        return [
          `查询：${summarize(toText(payload.query, 'N/A'), 96)}`,
          `库：${payload.dbName || 'Unknown'} · K=${payload.k ?? 'N/A'} · 结果=${results.length} · 耗时=${payload.useTime ?? 'N/A'}`
        ].join('\n');
      }
      return [
        `查询：${toText(payload.query, 'N/A')}`,
        `库：${payload.dbName || 'Unknown'} · K=${payload.k ?? 'N/A'} · 耗时=${payload.useTime ?? 'N/A'} · ReRank=${payload.useRerank ?? 'N/A'}`,
        payload.coreTags?.length ? `核心标签：${payload.coreTags.join(', ')}` : '',
        payload.tagStats ? `标签统计：${safeStringify(payload.tagStats)}` : '',
        results.length ? `召回结果：\n${results.map(formatNote).join('\n\n')}` : '召回结果：无',
      ].filter(Boolean).join('\n\n');
    }

    if (payload.type === 'META_THINKING_CHAIN') {
      const stages = Array.isArray(payload.stages) ? payload.stages : [];
      if (!expanded) {
        return [
          `查询：${summarize(toText(payload.query, 'N/A'), 96)}`,
          `链路：${payload.chainName || 'Unknown'} · 阶段=${stages.length}`
        ].join('\n');
      }
      const stageText = stages.map((stage: any) => [
        `阶段 ${stage.stage ?? ''}: ${stage.clusterName || 'Unknown'} (${stage.resultCount ?? stage.results?.length ?? 0}个结果)`,
        Array.isArray(stage.results) ? stage.results.map(formatNote).join('\n\n') : '',
      ].filter(Boolean).join('\n')).join('\n\n');
      return [`查询：${toText(payload.query, 'N/A')}`, `链路：${payload.chainName || 'Unknown'}`, stageText || safeStringify(payload)].join('\n\n');
    }

    if (payload.type === 'AI_MEMO_RETRIEVAL') {
      if (!expanded) {
        return `模式：${payload.mode || 'N/A'} · 日记数：${payload.diaryCount ?? 'N/A'}\n${summarize(toText(payload.extractedMemories || payload.message || ''), 120)}`;
      }
      return [
        `模式：${payload.mode || 'N/A'} · 日记数：${payload.diaryCount ?? 'N/A'}`,
        toText(payload.extractedMemories || payload.message || payload.data || payload),
      ].filter(Boolean).join('\n\n');
    }

    if (payload.type === 'DailyNote') {
      if (!expanded) {
        return `数据库：${payload.dbName || 'Unknown'} · ${summarize(toText(payload.message || ''), 120)}`;
      }
      return [
        `数据库：${payload.dbName || 'Unknown'} · 模式：${payload.action === 'FullTextRecall' ? '全量召回 <<>>' : '直接引入 {{}}'}`,
        toText(payload.message || payload.data || payload),
      ].filter(Boolean).join('\n\n');
    }

    if (String(payload.type || '').startsWith('AGENT_DREAM_')) {
      if (!expanded) {
        return [
          `Agent：${payload.agentName || 'Unknown'} · ${payload.type}`,
          summarize(toText(payload.message || payload.error || payload.narrative || ''), 120)
        ].filter(Boolean).join('\n');
      }
      return safeStringify(payload);
    }

    return toText(payload.message || payload.data || payload);
  };

  /**
   * 全局消息过滤引擎 (对标桌面端 filterManager.js)
   * 允许根据标题、内容或原始负载拦截/修改消息展示行为
   */
  const checkMessageFilter = (title: string, message: string, payload: any): FilterResult => {
    // 初始内置降噪及增强规则
    const builtInRules: FilterRule[] = [
      {
        name: 'Heartbeat/Ping/Pong Noise Reduction',
        match: (t, m, p) => {
          const content = (t + m).toLowerCase();
          const pType = String(p?.type || '').toLowerCase();
          return (
            pType === 'heartbeat' || pType === 'ping' || pType === 'pong' ||
            content.includes('heartbeat') || content.includes('ping') || content.includes('pong')
          );
        },
        action: 'hide'
      },
      {
        name: 'Redundant Connection Success',
        match: (_t, m, p) =>
          p?.type === 'connection_ack' &&
          (m.toLowerCase().includes('successful') ||
            String(p?.message || '').toLowerCase().includes('successful') ||
            String(p?.data?.message || '').toLowerCase().includes('successful')),
        action: 'hide'
      },
      {
        name: 'Important Error Duration Extension',
        match: (t, m, p) =>
          t.toLowerCase().includes('error') ||
          m.toLowerCase().includes('failed') ||
          ((p?.type === 'vcp_log' || p?.type === 'vcp-log-message') && p?.data?.status === 'error'),
        action: 'show',
        duration: 15000
      },
      {
        name: 'DistPluginManager Noise Reduction',
        match: (_t, m, p) =>
          p?.data?.source === 'DistPluginManager' &&
          (m.toLowerCase().includes('heartbeat') || m.toLowerCase().includes('checking server status')),
        action: 'hide'
      }
    ];

    for (const rule of builtInRules) {
      if (rule.match(title, message, payload)) {
        return {
          action: rule.action,
          duration: rule.duration,
          ruleName: rule.name
        };
      }
    }

    return { action: 'show' };
  };

  /**
   * 对标桌面端 notificationRenderer.js 的解析逻辑
   * 负责将后端原始 JSON 转化为前端 UI 可用的结构
   */
  const processPayload = (payload: any): Partial<VcpNotification> => {
    // 0. P2-7 Gap: 连接底层状态指示器 (vcp_log_status / connection_status)
    if (
      payload.type === 'vcp_log_status' ||
      payload.type === 'vcp-log-status' ||
      payload.type === 'connection_status'
    ) {
      const statusData = payload.data || payload;
      const status = (statusData.status || 'connecting') as VcpStatus['status'];
      const source = normalizeRealtimeSource(statusData.source);
      const statusMessage = normalizeRealtimeMessage(statusData.message);

      store.updateStatus({
        status,
        message: statusMessage,
        source
      });

      return { silent: true };
    }

    if (payload.type === 'vcp-core-status') {
      const { status, message } = payload;
      store.updateCoreStatus({
        status: status as any,
        message: message || '核心状态变更',
        source: 'Core'
      });

      if (status === 'error') {
        return {
          id: 'vcp_core_fatal_error',
          title: '核心引擎异常',
          message: message || '后端服务发生未知崩溃',
          type: 'error',
          duration: 0
        };
      }

      return { silent: true };
    }

    // VCPInfo 已负责展示服务端观测事件，旧 VCPLog 推送只保留内部控制通道。
    // Sync 仍沿用 vcp-log-message 作为本地通知载体，所以单独放行。
    if (shouldRouteToVcpInfoOnly(payload)) {
      return { silent: true };
    }

    let title = 'VCP 通知';
    let message = '';
    let type: VcpNotification['type'] = 'info';
    let isPreformatted = false;
    let duration = 7000;
    let actions: VcpNotification['actions'] = [];
    let summary = '';
    let observerKind: VcpObserverKind | undefined;
    let notificationId: string | undefined;
    let historyOnly = false;

    // 1. 核心 VCP 日志解析 (对标 renderVCPLogNotification)
    if ((payload.type === 'vcp_log' || payload.type === 'vcp-log-message') && payload.data) {
      const vcpData = payload.data;
      if (vcpData.id) {
        notificationId = String(vcpData.id);
        if (vcpData.id === 'vcp_sync_connection_status' && vcpData.status === 'error') {
          historyOnly = true;
        }
      }

      if (vcpData.tool_name && vcpData.status) {
        type = vcpData.status === 'error' ? 'error' : 'tool';
        title = `${vcpData.tool_name} ${vcpData.status}`;

        let rawContent = String(vcpData.content || '');
        message = rawContent;
        summary = summarize(rawContent);
        isPreformatted = true;

        // 尝试深层解析
        try {
          const inner = JSON.parse(rawContent);

          // P1-5 Gap: 提取内部时间戳并聚合标题 (对标桌面端 L61-68)
          const ts = inner.timestamp;
          if (ts && typeof ts === 'string' && ts.length >= 16) {
            const timeStr = ts.substring(11, 16);
            if (inner.MaidName) {
              title += ` (by ${inner.MaidName} @ ${timeStr})`;
            } else {
              title += ` (@ ${timeStr})`;
            }
          } else if (inner.MaidName) {
            title += ` (${inner.MaidName})`;
          }

          let hasValidOutput = false;
          // 提取原始输出
          if (inner.original_plugin_output) {
            if (typeof inner.original_plugin_output === 'object') {
              message = JSON.stringify(inner.original_plugin_output, null, 2);
              summary = summarize(message);
              hasValidOutput = true;
            } else if (String(inner.original_plugin_output).trim()) {
              message = String(inner.original_plugin_output);
              summary = summarize(message);
              isPreformatted = false;
              hasValidOutput = true;
            }
          }

          // DailyNote 成功状态 Fallback (P1-4 Gap)
          if (!hasValidOutput && vcpData.tool_name === 'DailyNote' && vcpData.status === 'success') {
            message = "✅ 日记内容已成功记录到本地知识库。";
            summary = message;
            isPreformatted = false;
          }
        } catch (e) {
          // 解析失败则保持 rawContent
        }

        // 错误模式处理 (针对嵌套的 JSON 错误)
        if (vcpData.status === 'error' && rawContent.includes('{')) {
          try {
            const jsonPart = rawContent.substring(rawContent.indexOf('{'));
            const parsed = JSON.parse(jsonPart);
            const errorMsg = parsed.plugin_error || parsed.error || parsed.message;
            if (errorMsg) {
              message = errorMsg;
              summary = summarize(message);
              isPreformatted = false;
            }
          } catch (e) { }
        }
      } else if (vcpData.source === 'DistPluginManager') {
        title = '分布式服务器';
        message = vcpData.content || JSON.stringify(vcpData);
        summary = summarize(message);
      }
    }
    // 1b. 分布式/工具结果直推
    else if (payload.type === 'tool_result') {
      const data = payload.data || payload;
      const toolName = data.toolName || data.tool_name || data.name || 'VCP 工具';
      const status = data.status || (data.error ? 'error' : 'success');
      const result = data.result ?? data.output ?? data.content ?? data.message ?? data.error ?? data;
      type = String(status).toLowerCase().includes('error') ? 'error' : 'tool';
      title = `工具结果：${toolName}`;
      message = [`状态：${status}`, toText(result)].join('\n\n');
      summary = summarize(`${status} ${toText(result)}`);
      isPreformatted = true;
      duration = 9000;
      observerKind = 'tool-result';
    }
    // 2. 审批请求 (对标 L142)
    else if (payload.type === 'tool_approval_request') {
      const approvalData = payload.data;
      type = 'warning';
      title = `🛠️ 审核请求: ${approvalData.toolName || 'Unknown'}`;
      message = `助手: ${approvalData.maid || 'N/A'}\n命令: ${approvalData.args?.command || JSON.stringify(approvalData.args || {})}\n时间: ${approvalData.timestamp || 'Just now'}`;
      summary = `${approvalData.maid || '未知助手'} 请求调用 ${approvalData.toolName || 'Unknown'}`;
      isPreformatted = true;
      duration = 0; // 永不自动消失
      observerKind = 'tool-approval';
      actions = [
        { label: '允许', value: true, color: 'bg-green-500 shadow-lg shadow-green-500/20' },
        { label: '拒绝', value: false, color: 'bg-red-500 shadow-lg shadow-red-500/20' }
      ];
    }
    // 3. 视频生成状态 (对标桌面端 L93-97)
    else if (payload.type === 'video_generation_status') {
      type = 'info';
      title = '视频生成状态';
      observerKind = 'video-status';

      const vTs = payload.data?.timestamp;
      if (vTs && typeof vTs === 'string' && vTs.length >= 16) {
        title += ` (@ ${vTs.substring(11, 16)})`;
      }

      message = payload.data?.original_plugin_output?.message || JSON.stringify(payload.data || {});
      summary = summarize(message);
    }
    // 4. 日记创建状态 (对标桌面端 L118)
    else if (payload.type === 'daily_note_created') {
      const noteData = payload.data || {};
      title = `日记: ${noteData.maidName || 'N/A'} (${noteData.dateString || 'N/A'})`;

      if (noteData.status === 'success') {
        type = 'success';
        message = noteData.message || '日记已成功创建。';
        summary = message;
      } else {
        type = 'info';
        message = noteData.message || '日记处理状态: ' + (noteData.status || '未知');
        summary = message;
      }
    }
    // 4b. 日记召回结果
    else if (payload.type === 'daily_note_recall_result') {
      const notes = Array.isArray(payload.notes) ? payload.notes : Array.isArray(payload.data?.notes) ? payload.data.notes : [];
      const query = payload.query || payload.data?.query || '未命名查询';
      type = 'diary';
      title = `日记召回：${query}`;
      isPreformatted = true;
      duration = 10000;
      message = notes.length ? `召回 ${notes.length} 条日记` : '没有召回到相关日记。';
      summary = `召回 ${notes.length} 条：${query}`;
      observerKind = 'daily-note';
    }
    // 4c. RAG 观察器同款事件：召回深度、剧情/记忆、梦境等
    else if (
      payload.type === 'RAG_RETRIEVAL_DETAILS' ||
      payload.type === 'META_THINKING_CHAIN' ||
      payload.type === 'AI_MEMO_RETRIEVAL' ||
      payload.type === 'DailyNote' ||
      payload.type === 'AGENT_PRIVATE_CHAT_PREVIEW' ||
      payload.source === 'AgentAssistant' ||
      String(payload.type || '').startsWith('AGENT_DREAM_')
    ) {
      observerKind = getObserverKind(payload);
      type = payload.type === 'RAG_RETRIEVAL_DETAILS' || payload.type === 'META_THINKING_CHAIN' || payload.type === 'AGENT_PRIVATE_CHAT_PREVIEW' || payload.source === 'AgentAssistant'
        ? 'agent'
        : 'diary';
      title = payload.type === 'RAG_RETRIEVAL_DETAILS'
        ? `RAG 检索：${payload.dbName || 'Unknown'}`
        : payload.type === 'META_THINKING_CHAIN'
          ? `召回深度：${payload.chainName || 'Unknown'}`
          : payload.type === 'AI_MEMO_RETRIEVAL'
            ? `记忆回溯（${payload.diaryCount || 0}）`
            : payload.type === 'DailyNote'
              ? `日记召回：${payload.dbName || 'Unknown'}`
              : payload.type === 'AGENT_PRIVATE_CHAT_PREVIEW'
                ? `Agent 私聊：${payload.agentName || 'Unknown'}`
                : payload.source === 'AgentAssistant'
                  ? 'Agent 助手通知'
                  : `Agent 剧情：${payload.agentName || payload.type}`;
      message = formatRagPayload(payload);
      summary = summarize(payload.query || payload.message || message);
      isPreformatted = true;
      duration = 10000;
    }
    // 5. 默认回退
    else {
      title = payload.type || 'VCP Message';
      message = typeof payload === 'string' ? payload : (payload.message || JSON.stringify(payload));
      summary = summarize(message);
      observerKind = getObserverKind(payload);
    }

    // 5. 执行全局过滤引擎 (P0-1 功能)
    const filterResult = checkMessageFilter(title, message, payload);

    if (filterResult.action === 'hide') {
      return { silent: true };
    }

    const result: Partial<VcpNotification> = {
      title,
      message,
      summary: summary || summarize(message),
      type,
      isPreformatted,
      duration: filterResult.duration ?? duration,
      actions,
      observerKind: observerKind || getObserverKind(payload),
      rawPayload: payload,
      historyOnly,
      silent: false
    };

    if (notificationId) {
      result.id = notificationId;
    }

    return result;
  };

  return { processPayload };
}
