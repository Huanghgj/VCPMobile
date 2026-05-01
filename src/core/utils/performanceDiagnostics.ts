import { computed, reactive } from 'vue';
import { invoke } from '@tauri-apps/api/core';

export interface DiagnosticsInfo {
  reportsDir: string;
  platform: string;
  arch: string;
  debugBuild: boolean;
}

export interface MetricEntry {
  name: string;
  value: number | string | boolean | null;
  unit?: string;
  detail?: string;
  ts: number;
  route: string;
  visibility: DocumentVisibilityState;
}

export interface PerformanceTraceDetail {
  phase?: string;
  messageId?: string;
  route?: string;
  chunkChars?: number;
  pendingChars?: number;
  displayedChars?: number;
  totalChars?: number;
  durationMs?: number;
  detail?: string;
  [key: string]: unknown;
}

export interface SavedReport {
  path: string;
  filename: string;
  savedAt: string;
}

interface NativeProcessSnapshot {
  pid: number;
  rssBytes: number | null;
  vmSizeBytes: number | null;
  threads: number | null;
  fdCount: number | null;
  processCpuTicks: number | null;
  totalCpuTicks: number | null;
  clockTicksPerSecond: number | null;
  gpuBusyPercent: number | null;
  gpuBusyTicks: number | null;
  gpuTotalTicks: number | null;
  gpuInfo: string | null;
}

interface MemorySnapshot {
  usedJSHeapSize?: number;
  totalJSHeapSize?: number;
  jsHeapSizeLimit?: number;
  nativeRssBytes?: number | null;
  nativeVmSizeBytes?: number | null;
  nativeThreads?: number | null;
  nativeFdCount?: number | null;
  nativeCpuPercent?: number | null;
  gpuBusyPercent?: number | null;
  gpuInfo?: string | null;
}

interface CpuTickBaseline {
  processCpuTicks: number;
  totalCpuTicks: number | null;
  ts: number;
}

interface GpuTickBaseline {
  busyTicks: number;
  totalTicks: number;
}

const state = reactive({
  info: null as DiagnosticsInfo | null,
  metrics: [] as MetricEntry[],
  running: false,
  status: '准备就绪',
  savedReport: null as SavedReport | null,
  runStartedAt: 0,
  runDurationMs: 0,
  fpsSamples: [] as number[],
  frameIntervals: [] as number[],
  longTasks: [] as number[],
  memorySnapshot: null as MemorySnapshot | null,
  nativeSnapshot: null as NativeProcessSnapshot | null,
  userNotes: '',
  autoStopAfterMs: 0,
  traceEvents: [] as MetricEntry[],
  renderSnapshot: null as Record<string, number | string | null> | null,
  lastCpuPercent: null as number | null,
  lastGpuBusyPercent: null as number | null,
});

let rafId = 0;
let durationTimer: number | undefined;
let nativeTimer: number | undefined;
let stopAt = 0;
let lastFrameAt = 0;
let frameCount = 0;
let fpsWindowStartedAt = 0;
let longTaskObserver: PerformanceObserver | null = null;
let cpuProbeRan = false;
let cpuBaseline: CpuTickBaseline | null = null;
let gpuBaseline: GpuTickBaseline | null = null;
const traceThrottleState = new Map<string, { ts: number; skipped: number }>();

const MAX_METRICS = 2400;
const MAX_TRACE_EVENTS = 900;

const currentRouteLabel = () => {
  const hash = window.location.hash || '#/';
  return hash.replace(/^#/, '') || '/';
};

const pushMetric = (entry: MetricEntry) => {
  state.metrics.unshift(entry);
  if (state.metrics.length > MAX_METRICS) state.metrics.length = MAX_METRICS;
};

const addMetric = (name: string, value: MetricEntry['value'], unit?: string, detail?: string) => {
  pushMetric({
    name,
    value,
    unit,
    detail,
    ts: Date.now(),
    route: currentRouteLabel(),
    visibility: document.visibilityState,
  });
};

const addTrace = (phase: string, detail: PerformanceTraceDetail = {}) => {
  if (!state.running) return;
  const highFrequencyPhases = new Set([
    'vcp-stream-data',
    'stream-chunk-buffered',
    'stream-chunk-appended',
    'stream-ui-flush',
    'message-render-stream-text',
  ]);
  if (highFrequencyPhases.has(phase)) {
    const key = `${phase}:${detail.messageId || ''}`;
    const now = performance.now();
    const previous = traceThrottleState.get(key);
    if (previous && now - previous.ts < 500) {
      previous.skipped += 1;
      return;
    }
    detail.skippedSinceLast = previous?.skipped || 0;
    traceThrottleState.set(key, { ts: now, skipped: 0 });
  }
  const entry: MetricEntry = {
    name: `Trace: ${phase}`,
    value: detail.durationMs ?? detail.chunkChars ?? detail.totalChars ?? true,
    unit: detail.durationMs !== undefined ? 'ms' : detail.chunkChars !== undefined || detail.totalChars !== undefined ? 'chars' : undefined,
    detail: JSON.stringify({ phase, ...detail }),
    ts: Date.now(),
    route: detail.route || currentRouteLabel(),
    visibility: document.visibilityState,
  };
  state.traceEvents.unshift(entry);
  if (state.traceEvents.length > MAX_TRACE_EVENTS) state.traceEvents.length = MAX_TRACE_EVENTS;
  pushMetric(entry);
};

const collectRenderSnapshot = () => {
  const allElements = document.getElementsByTagName('*').length;
  const messageNodes = document.querySelectorAll('[data-message-id], .vcp-bubble-container').length;
  const streamingNodes = document.querySelectorAll('.streaming, [data-streaming="true"]').length;
  const imageNodes = document.images.length;
  const canvasNodes = document.querySelectorAll('canvas').length;
  const svgNodes = document.querySelectorAll('svg').length;
  const fixedNodes = Array.from(document.querySelectorAll('body *')).reduce((count, element) => {
    const position = window.getComputedStyle(element).position;
    return count + (position === 'fixed' || position === 'sticky' ? 1 : 0);
  }, 0);

  state.renderSnapshot = {
    allElements,
    messageNodes,
    streamingNodes,
    imageNodes,
    canvasNodes,
    svgNodes,
    fixedNodes,
    viewport: `${window.innerWidth}x${window.innerHeight}`,
    devicePixelRatio: window.devicePixelRatio,
  };

  addMetric('Render Snapshot', allElements, 'nodes', `messages=${messageNodes} streaming=${streamingNodes} fixed=${fixedNodes} img=${imageNodes} canvas=${canvasNodes} svg=${svgNodes}`);
};

const estimateCpuPercent = (snapshot: NativeProcessSnapshot) => {
  if (snapshot.processCpuTicks === null) return null;
  const now = performance.now();
  if (!cpuBaseline) {
    cpuBaseline = { processCpuTicks: snapshot.processCpuTicks, totalCpuTicks: snapshot.totalCpuTicks, ts: now };
    return null;
  }

  const processDelta = snapshot.processCpuTicks - cpuBaseline.processCpuTicks;
  const totalDelta = snapshot.totalCpuTicks !== null && cpuBaseline.totalCpuTicks !== null
    ? snapshot.totalCpuTicks - cpuBaseline.totalCpuTicks
    : null;
  const elapsedMs = now - cpuBaseline.ts;
  cpuBaseline = { processCpuTicks: snapshot.processCpuTicks, totalCpuTicks: snapshot.totalCpuTicks, ts: now };
  if (processDelta < 0) return null;
  if (totalDelta && totalDelta > 0) {
    return Math.round((processDelta / totalDelta) * (navigator.hardwareConcurrency || 1) * 1000) / 10;
  }
  if (snapshot.clockTicksPerSecond && elapsedMs > 0) {
    const elapsedSeconds = elapsedMs / 1000;
    return Math.round((processDelta / snapshot.clockTicksPerSecond / elapsedSeconds) * 1000) / 10;
  }
  return null;
};

const estimateGpuBusyPercent = (snapshot: NativeProcessSnapshot) => {
  if (snapshot.gpuBusyTicks === null || snapshot.gpuTotalTicks === null || snapshot.gpuTotalTicks <= 0) {
    return snapshot.gpuBusyPercent;
  }
  if (!gpuBaseline) {
    gpuBaseline = { busyTicks: snapshot.gpuBusyTicks, totalTicks: snapshot.gpuTotalTicks };
    return null;
  }
  const busyDelta = snapshot.gpuBusyTicks - gpuBaseline.busyTicks;
  const totalDelta = snapshot.gpuTotalTicks - gpuBaseline.totalTicks;
  gpuBaseline = { busyTicks: snapshot.gpuBusyTicks, totalTicks: snapshot.gpuTotalTicks };
  if (busyDelta < 0 || totalDelta <= 0) return null;
  return Math.round(Math.min(100, Math.max(0, (busyDelta / totalDelta) * 100)) * 10) / 10;
};

const averageFps = computed(() => {
  if (!state.fpsSamples.length) return 0;
  return Math.round(state.fpsSamples.reduce((sum, item) => sum + item, 0) / state.fpsSamples.length);
});

const minFps = computed(() => {
  if (!state.fpsSamples.length) return 0;
  return Math.round(Math.min(...state.fpsSamples));
});

const jankWindows = computed(() => state.fpsSamples.filter((fps) => fps < 45).length);

const maxLongTask = computed(() => {
  if (!state.longTasks.length) return 0;
  return Math.round(Math.max(...state.longTasks));
});

const droppedFrameCount = computed(() => state.frameIntervals.filter((interval) => interval > 34).length);

const p95FrameMs = computed(() => {
  if (!state.frameIntervals.length) return 0;
  const sorted = [...state.frameIntervals].sort((a, b) => a - b);
  return Math.round(sorted[Math.floor(sorted.length * 0.95)] || 0);
});

const routeBreakdown = computed(() => {
  const result: Record<string, { fpsSamples: number; avgFps: number; longTasks: number; maxLongTaskMs: number; metrics: number }> = {};
  for (const metric of state.metrics) {
    const route = metric.route || 'unknown';
    result[route] ||= { fpsSamples: 0, avgFps: 0, longTasks: 0, maxLongTaskMs: 0, metrics: 0 };
    result[route].metrics += 1;

    if (metric.name === 'FPS' && typeof metric.value === 'number') {
      const item = result[route];
      item.avgFps = Math.round((item.avgFps * item.fpsSamples + metric.value) / (item.fpsSamples + 1));
      item.fpsSamples += 1;
    }

    if (metric.name === 'Long Task' && typeof metric.value === 'number') {
      result[route].longTasks += 1;
      result[route].maxLongTaskMs = Math.max(result[route].maxLongTaskMs, metric.value);
    }
  }
  return result;
});

const formatBytes = (bytes?: number | null) => {
  if (!bytes) return '不可用';
  const mb = bytes / 1024 / 1024;
  return `${mb.toFixed(1)} MB`;
};

const captureMemory = async () => {
  const performanceWithMemory = performance as Performance & {
    memory?: { usedJSHeapSize: number; totalJSHeapSize: number; jsHeapSizeLimit: number };
  };

  const memory: MemorySnapshot = {};
  if (performanceWithMemory.memory) {
    memory.usedJSHeapSize = performanceWithMemory.memory.usedJSHeapSize;
    memory.totalJSHeapSize = performanceWithMemory.memory.totalJSHeapSize;
    memory.jsHeapSizeLimit = performanceWithMemory.memory.jsHeapSizeLimit;
    addMetric('JS Heap Used', Math.round(performanceWithMemory.memory.usedJSHeapSize / 1024 / 1024), 'MB');
  } else {
    addMetric('JS Heap', '不可用', undefined, '当前 WebView 未暴露 performance.memory');
  }

  try {
    const nativeSnapshot = await invoke<NativeProcessSnapshot>('get_process_performance_snapshot');
    state.nativeSnapshot = nativeSnapshot;
    const cpuPercent = estimateCpuPercent(nativeSnapshot);
    const gpuBusyPercent = estimateGpuBusyPercent(nativeSnapshot);
    state.lastCpuPercent = cpuPercent;
    state.lastGpuBusyPercent = gpuBusyPercent;
    memory.nativeRssBytes = nativeSnapshot.rssBytes;
    memory.nativeVmSizeBytes = nativeSnapshot.vmSizeBytes;
    memory.nativeThreads = nativeSnapshot.threads;
    memory.nativeFdCount = nativeSnapshot.fdCount;
    memory.nativeCpuPercent = cpuPercent;
    memory.gpuBusyPercent = gpuBusyPercent;
    memory.gpuInfo = nativeSnapshot.gpuInfo;
    if (nativeSnapshot.rssBytes !== null) {
      addMetric('Native RSS', Math.round(nativeSnapshot.rssBytes / 1024 / 1024), 'MB');
    }
    if (nativeSnapshot.threads !== null) addMetric('Native Threads', nativeSnapshot.threads);
    if (cpuPercent !== null) addMetric('Process CPU', cpuPercent, '%', '由 /proc tick 差分估算，非系统级 profiler');
    if (gpuBusyPercent !== null) addMetric('GPU Busy', gpuBusyPercent, '%', `${nativeSnapshot.gpuInfo || ''} · 差分采样，整机 GPU busy 非 App 专属`);
    else addMetric('GPU Busy', '不可用', undefined, nativeSnapshot.gpuInfo || '当前设备未暴露可读 GPU busy 节点');
  } catch (error) {
    addMetric('Native Snapshot', '失败', undefined, String(error));
  }

  state.memorySnapshot = memory;
};

const runCpuProbe = async () => {
  const started = performance.now();
  let checksum = 0;
  for (let i = 0; i < 900_000; i += 1) {
    checksum = (checksum + Math.sqrt(i + (checksum % 17))) % 100000;
  }
  const elapsed = Math.round(performance.now() - started);
  addMetric('CPU Probe', elapsed, 'ms', `checksum=${checksum.toFixed(2)}`);
};

const tickFrame = (now: number) => {
  if (!state.running) return;

  if (!lastFrameAt) {
    lastFrameAt = now;
    fpsWindowStartedAt = now;
  } else {
    state.frameIntervals.push(now - lastFrameAt);
  }

  frameCount += 1;
  if (now - fpsWindowStartedAt >= 1000) {
    const fps = Math.round((frameCount * 1000) / (now - fpsWindowStartedAt));
    state.fpsSamples.push(fps);
    addMetric('FPS', fps, 'fps', `p95Frame=${p95FrameMs.value}ms dropped=${droppedFrameCount.value}`);
    frameCount = 0;
    fpsWindowStartedAt = now;
  }

  lastFrameAt = now;
  if (!stopAt || now < stopAt) {
    rafId = requestAnimationFrame(tickFrame);
  } else {
    void stopDiagnostics();
  }
};

const startDiagnostics = async (durationMs = 0, options: { clear?: boolean } = {}) => {
  if (state.running) return;

  if (options.clear ?? true) {
    state.metrics = [];
    state.fpsSamples = [];
    state.frameIntervals = [];
    state.longTasks = [];
    state.traceEvents = [];
    state.savedReport = null;
    state.memorySnapshot = null;
    state.nativeSnapshot = null;
    state.renderSnapshot = null;
    state.lastCpuPercent = null;
    state.lastGpuBusyPercent = null;
  }

  state.running = true;
  state.status = '全局采集中：点击悬浮窗可结束采集';
  state.runStartedAt = Date.now();
  state.runDurationMs = 0;
  state.autoStopAfterMs = durationMs;
  stopAt = durationMs > 0 ? performance.now() + durationMs : 0;
  lastFrameAt = 0;
  frameCount = 0;
  fpsWindowStartedAt = 0;
  cpuProbeRan = false;
  cpuBaseline = null;
  gpuBaseline = null;
  traceThrottleState.clear();

  addMetric('Diagnostics Started', true, undefined, durationMs > 0 ? `duration=${Math.round(durationMs / 1000)}s` : 'manual-stop=true');
  collectRenderSnapshot();

  if ('PerformanceObserver' in window) {
    try {
      longTaskObserver = new PerformanceObserver((list) => {
        list.getEntries().forEach((entry) => {
          state.longTasks.push(entry.duration);
          addMetric('Long Task', Math.round(entry.duration), 'ms', entry.name || undefined);
        });
      });
      longTaskObserver.observe({ entryTypes: ['longtask'] });
    } catch {
      addMetric('Long Task Observer', '不可用', undefined, '当前 WebView 不支持 longtask');
    }
  }

  durationTimer = window.setInterval(() => {
    state.runDurationMs = Date.now() - state.runStartedAt;
  }, 250);
  nativeTimer = window.setInterval(() => {
    void captureMemory();
    collectRenderSnapshot();
  }, 3_000);

  await captureMemory();
  collectRenderSnapshot();
  if (!cpuProbeRan) {
    cpuProbeRan = true;
    await runCpuProbe();
  }
  rafId = requestAnimationFrame(tickFrame);
};

const stopDiagnostics = async () => {
  if (!state.running) return;

  state.running = false;
  state.status = '采集完成，可以保存报告';
  state.runDurationMs = Date.now() - state.runStartedAt;
  if (rafId) cancelAnimationFrame(rafId);
  if (durationTimer) window.clearInterval(durationTimer);
  if (nativeTimer) window.clearInterval(nativeTimer);
  durationTimer = undefined;
  nativeTimer = undefined;
  longTaskObserver?.disconnect();
  longTaskObserver = null;
  await captureMemory();
  collectRenderSnapshot();
  addMetric('Diagnostics Stopped', true, undefined, `duration=${state.runDurationMs}ms`);
};

const summary = computed(() => ({
  runDurationMs: state.runDurationMs,
  averageFps: averageFps.value,
  minFps: minFps.value,
  jankWindows: jankWindows.value,
  longTaskCount: state.longTasks.length,
  maxLongTaskMs: maxLongTask.value,
  droppedFrameCount: droppedFrameCount.value,
  p95FrameMs: p95FrameMs.value,
  memory: state.memorySnapshot,
  nativeProcess: state.nativeSnapshot,
  lastCpuPercent: state.lastCpuPercent,
  lastGpuBusyPercent: state.lastGpuBusyPercent,
  renderSnapshot: state.renderSnapshot,
  traceEventCount: state.traceEvents.length,
  routeBreakdown: routeBreakdown.value,
  userAgent: navigator.userAgent,
  viewport: `${window.innerWidth}x${window.innerHeight}`,
  devicePixelRatio: window.devicePixelRatio,
  hardwareConcurrency: navigator.hardwareConcurrency || null,
  platform: state.info?.platform || 'unknown',
  arch: state.info?.arch || 'unknown',
  debugBuild: state.info?.debugBuild ?? null,
}));

const saveReport = async () => {
  state.status = '正在保存报告...';
  state.savedReport = await invoke<SavedReport>('save_performance_report', {
    payload: {
      summary: summary.value,
      metrics: state.metrics,
      traceEvents: state.traceEvents,
      userNotes: state.userNotes,
    },
  });
  state.status = '报告已保存';
  return state.savedReport;
};

const loadInfo = async () => {
  state.info = await invoke<DiagnosticsInfo>('get_performance_diagnostics_info');
  addMetric('Diagnostics Ready', true, undefined, state.info.reportsDir);
  return state.info;
};

export const usePerformanceDiagnostics = () => ({
  state,
  averageFps,
  minFps,
  jankWindows,
  maxLongTask,
  droppedFrameCount,
  p95FrameMs,
  routeBreakdown,
  summary,
  formatBytes,
  loadInfo,
  startDiagnostics,
  stopDiagnostics,
  saveReport,
  addTrace,
  collectRenderSnapshot,
});
