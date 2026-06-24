import { VcpNotification, useNotificationStore, VcpStatus } from '../stores/notification';

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

  const getApprovalPayload = (payload: any) => {
    if (payload?.type === 'tool_approval_request' && payload.data) {
      return payload;
    }
    if (
      payload?.type === 'vcp-info-message' &&
      payload.data?.type === 'tool_approval_request' &&
      payload.data?.data
    ) {
      return payload.data;
    }
    return null;
  };

  const stringifyCompact = (value: any, maxLength = 160) => {
    if (value === null || typeof value === 'undefined') return '';
    const text = typeof value === 'string' ? value : JSON.stringify(value);
    return text.length > maxLength ? `${text.substring(0, maxLength)}...` : text;
  };

  const stringifyPretty = (value: any, maxLength = 5000) => {
    if (value === null || typeof value === 'undefined') return '';
    const text = typeof value === 'string' ? value : JSON.stringify(value, null, 2);
    return text.length > maxLength ? `${text.substring(0, maxLength)}...` : text;
  };

  const notifyDiaryChanged = () => {
    window.dispatchEvent(new CustomEvent('vcp-diary-changed'));
  };

  const hasDetailLabel = (details: NonNullable<VcpNotification['details']>, label: string) =>
    details.some((detail) => detail.label === label);

  const appendRawDataDetail = (
    details: NonNullable<VcpNotification['details']>,
    data: any,
    payload: any,
  ) => {
    const knownKeys = new Set([
      'type',
      'message',
      'timestamp',
      'dbName',
      'query',
      'k',
      'results',
      'useTime',
      'useGroup',
      'useRerank',
      'useRerankPlus',
      'useGeodesicRerank',
      'useExpand',
      'useAssociate',
      'useTagMemo',
      'rrfAlpha',
      'geoAlpha',
      'tagWeight',
      'associateCount',
      'coreTags',
      'timeRanges',
      'tagStats',
      'chainName',
      'totalStages',
      'stages',
      'fromCache',
      'activatedGroups',
      'kSequence',
      'agentName',
      'sessionId',
      'response',
      'dreamId',
      'seedCount',
      'associationCount',
      'recentSeedsCount',
      'midSeedsCount',
      'deepRecallsCount',
      'narrative',
      'summary',
      'seeds',
      'associations',
      'operations',
      'operationLog',
      'action',
      'source',
      'pluginName',
      'status',
      'requestId',
      'prompt',
      'result',
      'content',
      'error',
    ]);
    const extras = Object.entries(data || {}).filter(([key]) => !knownKeys.has(key));

    if (extras.length && !hasDetailLabel(details, '额外字段')) {
      details.push({
        label: '额外字段',
        value: stringifyPretty(Object.fromEntries(extras), 3000),
        mono: true,
        multiline: true,
      });
    }

    if (!hasDetailLabel(details, '原始数据')) {
      details.push({
        label: '原始数据',
        value: stringifyPretty(payload, 6000),
        mono: true,
        multiline: true,
      });
    }
  };

  const normalizeList = (value: any, max = 4) => {
    if (!Array.isArray(value)) return [];
    return value
      .map((item) => String(item))
      .filter(Boolean)
      .slice(0, max);
  };

  const formatScore = (value: any) => {
    const num = Number(value);
    if (!Number.isFinite(num)) return '';
    if (Math.abs(num) <= 1) return num.toFixed(3);
    return num.toFixed(2);
  };

  const formatMetric = (label: string, value: any) => {
    const formatted = formatScore(value);
    return formatted ? `${label} ${formatted}` : '';
  };

  const estimateDistanceFromScore = (result: any) => {
    if (typeof result?.distance !== 'undefined') return '';
    const score = Number(result?.score);
    if (!Number.isFinite(score) || score <= 0) return '';
    return formatMetric('dist≈', (1 / score) - 1);
  };

  const formatTime = (value: any) => {
    if (typeof value !== 'string' || value.length < 16) return '';
    return value.substring(11, 19);
  };

  const compactLines = (lines: string[], maxLength = 900) => {
    const text = lines.filter(Boolean).join('\n');
    return text.length > maxLength ? `${text.substring(0, maxLength)}...` : text;
  };

  const resultText = (result: any, maxLength = 140) => stringifyCompact(
    result?.text || result?.content || result?.file || result?.sourceFile || result?.fullPath || result?.title || result,
    maxLength
  );

  const resultMetrics = (result: any) => [
    formatMetric('score', result?.score),
    formatMetric('rerank', result?.rerank_score),
    formatMetric('rrf', result?.rrf_score),
    formatMetric('knn', result?.original_knn_score),
    formatMetric('geo', result?.geo_score),
    formatMetric('nGeo', result?.normalized_geo),
    formatMetric('dist', result?.distance),
    estimateDistanceFromScore(result),
    formatMetric('orig', result?.original_score ?? result?.originalScore),
    formatMetric('tag', result?.tagMatchScore),
    formatMetric('boost', result?.boostFactor),
    formatMetric('decay', result?.decay_factor),
    typeof result?.diff_days !== 'undefined' ? `age ${result.diff_days}d` : '',
    typeof result?.geo_hit_count !== 'undefined' ? `geoHits ${result.geo_hit_count}` : '',
    typeof result?.retrieval_rank !== 'undefined' ? `retrieval#${result.retrieval_rank}` : '',
    typeof result?.rerank_rank !== 'undefined' ? `rerank#${result.rerank_rank}` : '',
    typeof result?.tagMatchCount !== 'undefined' ? `tagCount ${result.tagMatchCount}` : '',
    typeof result?.associateCoCount !== 'undefined' ? `co ${result.associateCoCount}` : '',
    typeof result?.originalChunkCount !== 'undefined' ? `chunks ${result.originalChunkCount}` : '',
  ].filter(Boolean).join(' · ');

  const formatResultLine = (result: any, index: number) => {
    const source = result?.source || 'memory';
    const metrics = resultMetrics(result);
    const tags = [
      ...normalizeList(result?.matchedTags, 4),
      ...normalizeList(result?.coreTagsMatched, 4),
    ];
    const tagText = tags.length ? ` [${tags.join(', ')}]` : '';
    return `${index + 1}. ${source}${metrics ? ` · ${metrics}` : ''}${tagText}\n${resultText(result, 180)}`;
  };

  const metricPair = (label: string, value: any) => {
    const formatted = formatScore(value);
    return formatted ? { label, value: formatted } : null;
  };

  const compactFileName = (value: any) => {
    const text = String(value || '');
    if (!text) return '';
    const parts = text.split(/[\\/]/);
    return parts[parts.length - 1] || text;
  };

  const resultStructuredMetrics = (result: any) => [
    metricPair('score', result?.score),
    metricPair('rerank', result?.rerank_score),
    metricPair('rrf', result?.rrf_score),
    metricPair('dist', result?.distance),
    metricPair('geo', result?.geo_score),
    metricPair('tag', result?.tagMatchScore),
    metricPair('boost', result?.boostFactor),
  ].filter(Boolean) as { label: string; value: string }[];

  const buildRagRows = (results: any[]) => results.slice(0, 6).map((result, index) => {
    const sourcePath = result?.sourceFile || result?.fullPath || result?.file || result?.path;
    const sourceFile = compactFileName(sourcePath);
    const chips = [
      result?.source ? String(result.source) : 'memory',
      typeof result?.retrieval_rank !== 'undefined' ? `retrieval#${result.retrieval_rank}` : '',
      typeof result?.rerank_rank !== 'undefined' ? `rerank#${result.rerank_rank}` : '',
      ...normalizeList(result?.matchedTags, 3),
      ...normalizeList(result?.coreTagsMatched, 2),
    ].filter(Boolean);

    return {
      title: sourceFile ? `${index + 1}. ${sourceFile}` : `${index + 1}. ${result?.source || 'memory'}`,
      subtitle: typeof result?.chunkId !== 'undefined' ? `Chunk ${result.chunkId}` : undefined,
      body: resultText(result, 260),
      chips,
      metrics: resultStructuredMetrics(result),
      source: result?.source ? String(result.source) : undefined,
      path: sourcePath ? String(sourcePath) : undefined,
      snippet: result?.snippet ? stringifyCompact(result.snippet, 260) : undefined,
      metadata: result?.metadata || result?.meta,
      raw: result,
    };
  });

  const buildStageRows = (stages: any[]) => stages.slice(0, 6).map((stage, index) => {
    const stageLabel = stage.stageName || stage.name || stage.clusterName || `Stage ${stage.stage || index + 1}`;
    const chips = [
      typeof stage.resultCount !== 'undefined' ? `命中 ${stage.resultCount}` : '',
      typeof stage.k !== 'undefined' ? `K ${stage.k}` : '',
      stage.error ? 'error' : '',
    ].filter(Boolean);
    return {
      title: `${index + 1}. ${stageLabel}`,
      body: stringifyCompact(stage.summary || stage.query || stage.error || stage, 260),
      chips,
      metrics: [],
    };
  });

  const stageCount = (data: any) => String(data.totalStages ?? data.stages?.length ?? 0);

  const detailIsHeavy = (detail: NonNullable<VcpNotification['details']>[number]) =>
    detail.label === '原始数据' || detail.label === '额外字段';

  const buildVcpInfoNotification = (payload: any): Partial<VcpNotification> => {
    const approvalPayload = getApprovalPayload(payload);
    if (approvalPayload) {
      return buildToolApprovalNotification(approvalPayload);
    }

    const data = payload.data || payload;
    const infoType = String(data.type || 'VCP_INFO');
    const meta: NonNullable<VcpNotification['meta']> = [];
    const details: NonNullable<VcpNotification['details']> = [];
    const tags = ['VCPInfo'];
    let title = 'VCPInfo';
    let subtitle = infoType;
    let message = data.message || '';
    let type: VcpNotification['type'] = 'info';
    let category = 'VCPInfo';
    let duration = 9000;
    let historyOnly = false;
    let structured: VcpNotification['structured'] | undefined;

    const timestamp = formatTime(data.timestamp);
    if (timestamp) meta.push({ label: '时间', value: timestamp });

    switch (infoType) {
      case 'RAG_RETRIEVAL_DETAILS': {
        type = 'tool';
        category = 'RAG';
        title = `RAG 召回 · ${data.dbName || '日记本'}`;
        subtitle = data.query ? String(data.query) : '语义记忆召回详情';
        const results = Array.isArray(data.results) ? data.results : [];
        const top = results[0];
        const modeTags = [
          data.useTime ? 'Time' : '',
          data.useGroup ? 'Group' : '',
          data.useRerank ? 'Rerank' : '',
          data.useRerankPlus ? 'RRF' : '',
          data.useGeodesicRerank ? 'Geo' : '',
          data.useExpand ? 'Expand' : '',
          data.useAssociate ? 'Associate' : '',
          data.useTagMemo ? 'TagMemo' : '',
        ].filter(Boolean);
        tags.push(...modeTags);
        meta.push({ label: 'K', value: String(data.k ?? results.length) });
        meta.push({ label: '命中', value: String(results.length) });
        if (typeof data.rrfAlpha !== 'undefined') meta.push({ label: 'RRF α', value: String(data.rrfAlpha) });
        if (typeof data.geoAlpha !== 'undefined') meta.push({ label: 'Geo α', value: String(data.geoAlpha) });
        if (typeof data.tagWeight !== 'undefined') meta.push({ label: 'Tag 权重', value: String(data.tagWeight) });
        if (typeof data.associateCount !== 'undefined') {
          meta.push({ label: '联想', value: String(data.associateCount) });
        }
        if (Array.isArray(data.coreTags) && data.coreTags.length) {
          details.push({ label: '核心标签', value: normalizeList(data.coreTags, 8).join(' · ') });
        }
        if (Array.isArray(data.timeRanges) && data.timeRanges.length) {
          details.push({
            label: '时间范围',
            value: data.timeRanges
              .slice(0, 4)
              .map((range: any) => `${stringifyCompact(range.start, 32)} → ${stringifyCompact(range.end, 32)}`)
              .join('\n'),
            multiline: true,
          });
        }
        if (data.tagStats) {
          const tagStats = data.tagStats;
          const tagStatsLines = [
            typeof tagStats.totalTagMatches !== 'undefined' ? `匹配标签数: ${tagStats.totalTagMatches}` : '',
            typeof tagStats.resultsWithTags !== 'undefined' ? `带标签结果: ${tagStats.resultsWithTags}` : '',
            typeof tagStats.avgBoostFactor !== 'undefined' ? `平均 Boost: ${tagStats.avgBoostFactor}` : '',
            Array.isArray(tagStats.uniqueMatchedTags) && tagStats.uniqueMatchedTags.length
              ? `标签: ${normalizeList(tagStats.uniqueMatchedTags, 12).join(' · ')}`
              : '',
          ].filter(Boolean);
          if (tagStatsLines.length) details.push({ label: 'Tag 统计', value: tagStatsLines.join('\n'), multiline: true });
        }
        if (top) {
          const metrics = resultMetrics(top);
          message = `${top.source || 'memory'}${metrics ? ` · ${metrics}` : ''}: ${resultText(top, 220)}`;
          details.push({
            label: 'Top Hit 指标',
            value: compactLines([
              top.sourceFile || top.fullPath ? `文件: ${top.sourceFile || top.fullPath}` : '',
              typeof top.chunkId !== 'undefined' ? `Chunk: ${top.chunkId}` : '',
              metrics,
            ], 520),
            multiline: true,
          });
          details.push({
            label: 'Top Hit 内容',
            value: resultText(top, 520),
            multiline: true,
          });
        } else {
          message = '没有召回可展示的条目。';
        }
        if (results.length) {
          details.push({
            label: '召回列表',
            value: compactLines(results.slice(0, 8).map(formatResultLine), 1800),
            multiline: true,
          });
        }
        structured = {
          kind: 'rag',
          summary: `${data.dbName || 'DailyNote'} · ${results.length} 条命中 · ${data.query || 'RAG 检索'}`,
          rows: buildRagRows(results),
        };
        historyOnly = true;
        break;
      }
      case 'META_THINKING_CHAIN': {
        type = 'tool';
        category = 'Meta';
        title = `元思考链 · ${data.chainName || 'default'}`;
        subtitle = data.query ? String(data.query) : 'Meta Thinking 执行详情';
        tags.push('MetaThinking');
        meta.push({ label: '阶段', value: String(data.totalStages ?? data.stages?.length ?? 0) });
        if (typeof data.fromCache !== 'undefined') meta.push({ label: '缓存', value: data.fromCache ? '是' : '否' });
        const groups = normalizeList(data.activatedGroups, 4);
        if (groups.length) meta.push({ label: '分组', value: groups.join(', ') });
        if (Array.isArray(data.kSequence)) meta.push({ label: 'K序列', value: data.kSequence.join('→') });
        const firstStage = Array.isArray(data.stages) ? data.stages[0] : null;
        message = firstStage
          ? stringifyCompact(firstStage.summary || firstStage.query || firstStage.stageName || firstStage, 220)
          : '元思考链已完成。';
        if (Array.isArray(data.stages)) {
          details.push({
            label: '阶段摘要',
            value: data.stages
              .slice(0, 4)
              .map((stage: any, index: number) => {
                const stageLabel = stage.stageName || stage.name || stage.clusterName || `Stage ${stage.stage || index + 1}`;
                const count = typeof stage.resultCount !== 'undefined' ? ` · 命中 ${stage.resultCount}` : '';
                return `${index + 1}. ${stageLabel}${count}\n${stringifyCompact(stage.query || stage.summary || stage, 160)}`;
              })
              .join('\n'),
            multiline: true,
          });
          const stageResults = data.stages
            .slice(0, 3)
            .flatMap((stage: any, stageIndex: number) => {
              const results = Array.isArray(stage.results) ? stage.results : [];
              return results.slice(0, 3).map((result: any, resultIndex: number) => {
                const prefix = `S${stage.stage || stageIndex + 1}.${resultIndex + 1}`;
                return `${prefix} ${resultMetrics(result)}\n${resultText(result, 160)}`;
              });
            });
          if (stageResults.length) {
            details.push({
              label: '阶段命中',
              value: compactLines(stageResults, 1400),
              multiline: true,
            });
          }
        }
        structured = {
          kind: 'thinking',
          summary: `${stageCount(data)} 个阶段 · ${data.chainName || 'Meta Thinking'}`,
          rows: buildStageRows(Array.isArray(data.stages) ? data.stages : []),
        };
        historyOnly = true;
        break;
      }
      case 'AGENT_PRIVATE_CHAT_PREVIEW': {
        type = 'agent';
        category = 'Agent';
        title = `私聊预览 · ${data.agentName || 'Agent'}`;
        subtitle = data.sessionId ? `Session ${data.sessionId}` : 'AgentAssistant';
        tags.push('Agent');
        message = stringifyCompact(data.response || data.message || '', 240);
        if (data.sessionId) meta.push({ label: 'Session', value: String(data.sessionId) });
        if (data.agentName) meta.push({ label: 'Agent', value: String(data.agentName) });
        if (data.query) details.push({ label: '请求', value: String(data.query), multiline: true });
        if (data.response) details.push({ label: '回复', value: String(data.response), multiline: true });
        structured = {
          kind: 'private_chat',
          summary: `${data.agentName || 'Agent'} · ${data.sessionId || '临时会话'}`,
          rows: [
            {
              title: '请求',
              body: stringifyCompact(data.query || data.message || '', 320),
              chips: data.sessionId ? [`Session ${data.sessionId}`] : [],
            },
            {
              title: '回复',
              body: stringifyCompact(data.response || '', 420),
              chips: data.agentName ? [String(data.agentName)] : [],
            },
          ].filter((row) => row.body),
        };
        break;
      }
      case 'AGENT_DREAM_START':
      case 'AGENT_DREAM_ASSOCIATIONS':
      case 'AGENT_DREAM_NARRATIVE':
      case 'AGENT_DREAM_OPERATIONS':
      case 'AGENT_DREAM_END': {
        type = 'agent';
        category = 'Dream';
        title = `梦境 · ${data.agentName || 'Agent'}`;
        subtitle = infoType.replace('AGENT_DREAM_', '').toLowerCase();
        tags.push('Dream');
        if (data.dreamId) meta.push({ label: 'Dream', value: String(data.dreamId) });
        if (typeof data.seedCount !== 'undefined') meta.push({ label: '种子', value: String(data.seedCount) });
        if (typeof data.associationCount !== 'undefined') meta.push({ label: '联想', value: String(data.associationCount) });
        if (typeof data.recentSeedsCount !== 'undefined') meta.push({ label: '近期', value: String(data.recentSeedsCount) });
        if (typeof data.midSeedsCount !== 'undefined') meta.push({ label: '中期', value: String(data.midSeedsCount) });
        if (typeof data.deepRecallsCount !== 'undefined') meta.push({ label: '深层', value: String(data.deepRecallsCount) });
        const associations = Array.isArray(data.associations) ? data.associations : [];
        message = stringifyCompact(data.narrative || data.summary || data.message || (associations.length ? `${associations.length} 条联想召回` : '梦境事件更新'), 240);
        if (Array.isArray(data.seeds) && data.seeds.length) {
          details.push({
            label: '种子',
            value: data.seeds.slice(0, 5).map((item: any) => `${item.file || item.title || 'seed'}: ${stringifyCompact(item.snippet || item.content || item, 110)}`).join('\n'),
            multiline: true,
          });
        }
        if (associations.length) {
          details.push({
            label: '联想',
            value: associations.slice(0, 8).map((item: any) => {
              const score = formatMetric('score', item.score);
              return `${item.file || item.title || 'association'}${score ? ` · ${score}` : ''}: ${stringifyCompact(item.content || item.snippet || item, 100)}`;
            }).join('\n'),
            multiline: true,
          });
        }
        if (data.operations || data.operationLog) {
          details.push({
            label: '操作',
            value: stringifyCompact(data.operations || data.operationLog, 900),
            multiline: true,
          });
        }
        structured = {
          kind: 'dream',
          summary: `${data.agentName || 'Agent'} · ${infoType.replace('AGENT_DREAM_', '').toLowerCase()}`,
          rows: [
            {
              title: subtitle,
              body: stringifyCompact(data.narrative || data.summary || data.message || data.error || '', 360),
              chips: [
                data.dreamId ? String(data.dreamId) : '',
                typeof data.seedCount !== 'undefined' ? `种子 ${data.seedCount}` : '',
                typeof data.associationCount !== 'undefined' ? `联想 ${data.associationCount}` : '',
                typeof data.operationCount !== 'undefined' ? `操作 ${data.operationCount}` : '',
              ].filter(Boolean),
            },
            ...associations.slice(0, 4).map((item: any, index: number) => ({
              title: compactFileName(item.file || item.title) || `联想 ${index + 1}`,
              body: stringifyCompact(item.content || item.snippet || item, 180),
              metrics: metricPair('score', item.score) ? [metricPair('score', item.score)!] : [],
            })),
          ].filter((row) => row.body || row.chips?.length),
        };
        break;
      }
      case 'DailyNote': {
        type = 'success';
        category = 'DailyNote';
        title = `日记召回 · ${data.dbName || 'DailyNote'}`;
        subtitle = data.action ? String(data.action) : 'DailyNote';
        tags.push('DailyNote');
        message = data.message || JSON.stringify(data);
        if (data.dbName) meta.push({ label: '日记本', value: String(data.dbName) });
        if (data.action) meta.push({ label: '动作', value: String(data.action) });
        if (data.fromCache) meta.push({ label: '缓存', value: '是' });
        if (data.error) details.push({ label: '错误', value: String(data.error), multiline: true });
        break;
      }
      default: {
        category = data.source ? String(data.source) : 'Other';
        if (data.source) meta.push({ label: '来源', value: String(data.source) });
        if (data.agentName) meta.push({ label: 'Agent', value: String(data.agentName) });
        if (data.pluginName) meta.push({ label: '插件', value: String(data.pluginName) });
        if (data.status) meta.push({ label: '状态', value: String(data.status) });
        if (data.sessionId) meta.push({ label: 'Session', value: String(data.sessionId) });
        if (data.requestId) meta.push({ label: 'Request', value: String(data.requestId) });
        for (const key of ['query', 'prompt', 'response', 'result', 'content', 'summary', 'error']) {
          if (typeof data[key] !== 'undefined') {
            details.push({
              label: key,
              value: stringifyCompact(data[key], 900),
              multiline: true,
            });
          }
        }
        title = data.source ? `VCPInfo · ${data.source}` : `VCPInfo · ${infoType}`;
        if (!message) {
          message = stringifyCompact(data.summary || data.content || data.response || data.query || data, 260);
        }
        const severity = String(data.status || data.type || '').toLowerCase();
        if (severity === 'error' || typeof data.error !== 'undefined') {
          type = 'error';
        } else if (severity === 'warning') {
          type = 'warning';
        }
        structured = {
          kind: 'generic',
          summary: infoType,
          rows: Object.entries(data || {})
            .filter(([key]) => !['type', 'timestamp'].includes(key))
            .slice(0, 6)
            .map(([key, value]) => ({
              title: key,
              body: stringifyCompact(value, 260),
            })),
        };
      }
    }

    appendRawDataDetail(details, data, payload);
    const sortedDetails = [
      ...details.filter((detail) => !detailIsHeavy(detail)),
      ...details.filter(detailIsHeavy),
    ];

    const result: Partial<VcpNotification> = {
      id: `vcp-info-${Date.now()}-${Math.random().toString(36).slice(2, 8)}`,
      title,
      subtitle: String(subtitle),
      message: String(message || 'VCPInfo 事件'),
      type,
      source: 'VCPInfo',
      category,
      infoType,
      tags,
      meta,
      details: sortedDetails,
      structured,
      duration,
      rawPayload: payload,
      silent: false,
      historyOnly,
    };

    return result;
  };

  const buildToolApprovalNotification = (payload: any): Partial<VcpNotification> => {
    const approvalData = payload.data || {};
    const requestId = typeof approvalData.requestId === 'string' ? approvalData.requestId.trim() : '';
    const toolName = approvalData.toolName || approvalData.tool_name || 'Unknown';
    const args = approvalData.args || {};
    const command = args.command || args.cmd || stringifyCompact(args, 220);
    const maid = approvalData.maid || approvalData.agentName || approvalData.agent || 'N/A';

    return {
      id: requestId ? `tool-approval-${requestId}` : `tool-approval-${Date.now()}-${Math.random().toString(36).slice(2, 8)}`,
      title: `审核请求: ${toolName}`,
      message: `助手: ${maid}\n命令: ${command}\n时间: ${approvalData.timestamp || 'Just now'}`,
      type: 'warning',
      category: 'approval',
      infoType: 'tool_approval_request',
      tags: ['工具审核', String(toolName)],
      isPreformatted: true,
      duration: 0,
      actions: [
        { label: '允许', value: true, color: 'bg-green-500 shadow-lg shadow-green-500/20' },
        { label: '拒绝', value: false, color: 'bg-red-500 shadow-lg shadow-red-500/20' }
      ],
      rawPayload: payload,
      silent: false,
      historyOnly: false,
    };
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
          (p?.type === 'vcp-log-message' && p?.data?.status === 'error'),
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
    // 0. P2-7 Gap: 连接底层状态指示器 (VCPLog)
    // 同步状态不再渲染到全局状态栏（同步已改为完全手动触发，避免状态栏干扰）
    if (payload.type === 'vcp-log-status') {
      const statusData = payload.data || payload;
      const status = (statusData.status || 'connecting') as VcpStatus['status'];
      const source = statusData.source || 'VCPLog';
      const message = statusData.message || '状态未知';

      store.updateStatus({
        status,
        message,
        source
      });

      // 彻底静默连接状态通知
      return { silent: true };
    }

    if (payload.type === 'vcp-info-status') {
      return { silent: true };
    }

    const approvalPayload = getApprovalPayload(payload);
    if (approvalPayload) {
      return buildToolApprovalNotification(approvalPayload);
    }

    if (payload.type === 'vcp-info-message') {
      return buildVcpInfoNotification(payload);
    }

    // --- 核心引擎状态处理 (P0 级别) ---
    if (payload.type === 'vcp-core-status') {
      const { status, message } = payload;

      store.updateCoreStatus({
        status: status as any,
        message: message || '核心状态变更',
        source: 'Core'
      });

      // 核心错误需要强制弹窗
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

    let title = 'VCP 通知';
    let message = '';
    let type: VcpNotification['type'] = 'info';
    let isPreformatted = false;
    let duration = 7000;
    let actions: VcpNotification['actions'] = [];
    let notificationId: string | undefined = undefined;
    let historyOnly = false;

    // --- 核心协议解析层 (对标桌面端 notificationRenderer.js) ---

    // 1. vcp_log: 核心工具调用日志 (服务端协议) 或 vcp-log-message (移动端内部兼容)
    if ((payload.type === 'vcp_log' || payload.type === 'vcp-log-message') && payload.data) {
      const vcpData = payload.data;

      if (vcpData.id) {
        notificationId = vcpData.id;
        if (vcpData.id === 'vcp_sync_connection_status' && vcpData.status === 'error') {
          historyOnly = true;
        }
      }

      if (vcpData.tool_name && vcpData.status) {
        type = vcpData.status === 'error'
          ? 'error'
          : (vcpData.tool_name === 'DailyNote' ? 'success' : 'tool');

        const statusText = vcpData.status === 'success' ? '执行成功' : vcpData.status === 'error' ? '执行失败' : vcpData.status;
        title = `${vcpData.tool_name} ${statusText}`;

        let rawContent = String(vcpData.content || '');
        message = rawContent;

        // 智能降维渲染：如果文本内容以 Emoji ✅/❌ 开头，或者是不含有换行与大括号的单行日常提示
        // 则设为非 Preformatted，以便采用极致自然的原生排版呈现，剔除代码框的突兀感
        isPreformatted = !(
          rawContent.startsWith('✅') ||
          rawContent.startsWith('❌') ||
          (!rawContent.includes('\n') && !rawContent.includes('{'))
        );

        // 处理错误模式: "执行错误: {"plugin_error": "..."}"
        if (vcpData.status === 'error' && rawContent.includes('{')) {
          const jsonStart = rawContent.indexOf('{');
          const prefix = rawContent.substring(0, jsonStart).trim();
          const jsonPart = rawContent.substring(jsonStart);
          try {
            const parsed = JSON.parse(jsonPart);
            const errorMsg = parsed.plugin_error || parsed.error || parsed.message;
            if (errorMsg) {
              message = prefix ? `${prefix}${prefix.endsWith(':') ? ' ' : ': '}${errorMsg}` : errorMsg;
              isPreformatted = false;
            }
          } catch (e) { }
        }

        // 尝试解析内部元数据 (MaidName, timestamp)
        try {
          const inner = JSON.parse(rawContent);
          let titleSuffix = '';
          if (inner.MaidName) {
            titleSuffix += ` by ${inner.MaidName}`;
          }
          if (inner.timestamp && typeof inner.timestamp === 'string' && inner.timestamp.length >= 16) {
            const timeStr = inner.timestamp.substring(11, 16);
            titleSuffix += `${inner.MaidName ? ' ' : ''}@ ${timeStr}`;
          }
          if (titleSuffix) {
            title += ` (${titleSuffix.trim()})`;
          }

          if (typeof inner.original_plugin_output !== 'undefined') {
            if (typeof inner.original_plugin_output === 'object' && inner.original_plugin_output !== null) {
              message = JSON.stringify(inner.original_plugin_output, null, 2);
            } else {
              message = String(inner.original_plugin_output);
              isPreformatted = false;
            }
          } else if (vcpData.tool_name === 'DailyNote' && vcpData.status === 'success') {
            message = "✅ 日记内容已成功记录到本地知识库。";
            isPreformatted = false;
          }
        } catch (e) { }

        if (vcpData.tool_name === 'DailyNote' && vcpData.status === 'success') {
          notifyDiaryChanged();
        }
      } else if (vcpData.source === 'DistPluginManager' || vcpData.source === 'Distributed') {
        title = '分布式服务器';
        message = vcpData.content || JSON.stringify(vcpData);
        isPreformatted = false;
      } else {
        title = 'VCP 日志条目';
        message = JSON.stringify(vcpData, null, 2);
        isPreformatted = true;
      }
    }
    // 2. video_generation_status: 视频生成状态
    else if (payload.type === 'video_generation_status' && payload.data) {
      type = 'info';
      title = '视频生成状态';
      const vData = payload.data;

      if (vData.original_plugin_output && typeof vData.original_plugin_output.message === 'string') {
        message = vData.original_plugin_output.message;
        isPreformatted = false;
      } else if (vData.original_plugin_output) {
        message = JSON.stringify(vData.original_plugin_output, null, 2);
        isPreformatted = true;
      } else {
        message = JSON.stringify(vData, null, 2);
        isPreformatted = true;
      }

      if (vData.timestamp && typeof vData.timestamp === 'string' && vData.timestamp.length >= 16) {
        title += ` (@ ${vData.timestamp.substring(11, 16)})`;
      }
    }
    // 3. daily_note_created: 日记创建通知
    else if (payload.type === 'daily_note_created' && payload.data) {
      const noteData = payload.data;
      title = `日记: ${noteData.maidName || 'N/A'} (${noteData.dateString || 'N/A'})`;
      type = noteData.status === 'success' ? 'success' : 'info';
      message = noteData.message || (noteData.status === 'success' ? '日记已成功创建。' : `日记处理状态: ${noteData.status || '未知'}`);
      isPreformatted = false;
      if (noteData.status === 'success') notifyDiaryChanged();
    }
    // 4. connection_ack: 连接确认
    else if (payload.type === 'connection_ack' && payload.message) {
      title = 'VCP 连接';
      message = String(payload.message);
      isPreformatted = false;
    }
    // 5. tool_approval_request: 审核请求
    else if (payload.type === 'tool_approval_request' && payload.data) {
      return buildToolApprovalNotification(payload);
    }
    // 6. 默认回退 (Generic fallback)
    else {
      if (typeof payload === 'object' && payload !== null) {
        title = payload.type ? `类型: ${payload.type}` : 'VCP 消息';
        message = payload.message || (payload.data?.message) || JSON.stringify(payload, null, 2);

        // 如果有附加数据，追加展示
        if (payload.data && !payload.message) {
          message = JSON.stringify(payload.data, null, 2);
          isPreformatted = true;
        } else {
          isPreformatted = message.includes('{') || message.includes('\n');
        }
      } else {
        title = 'VCP 消息';
        message = String(payload);
        isPreformatted = false;
      }
    }

    // 统一截断 (L181)
    if (message.length > 300) {
      message = message.substring(0, 300) + '...';
    }

    // 5. 执行全局过滤引擎 (P0-1 功能)
    const filterResult = checkMessageFilter(title, message, payload);

    if (filterResult.action === 'hide') {
      return { silent: true };
    }

    const result: Partial<VcpNotification> = {
      title,
      message,
      type,
      category: payload?.type ? String(payload.type) : type,
      infoType: payload?.type ? String(payload.type) : undefined,
      isPreformatted,
      duration: filterResult.duration ?? duration,
      actions,
      rawPayload: payload,
      silent: false,
      historyOnly
    };

    if (notificationId) {
      result.id = notificationId;
    }

    return result;
  };

  return { processPayload };
}
