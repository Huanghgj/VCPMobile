<script setup lang="ts">
import { computed, onMounted, ref } from 'vue';
import { useRouter } from 'vue-router';
import { Activity, ArrowLeft, CheckCircle2, FileJson, Gauge, Home, Play, Save, Square, Timer, Zap } from 'lucide-vue-next';
import { invoke } from '@tauri-apps/api/core';
import { usePerformanceDiagnostics } from '../../core/utils/performanceDiagnostics';
import { useChatManagerStore } from '../../core/stores/chatManager';

interface DiagnosticTestServerInfo {
  running: boolean;
  bindLan: boolean;
  port: number;
  token: string;
  url: string;
  localUrl: string;
}

const router = useRouter();
const diagnostics = usePerformanceDiagnostics();
const chatStore = useChatManagerStore();
const state = diagnostics.state;
const diagnosticServer = ref<DiagnosticTestServerInfo | null>(null);
const diagnosticBusy = ref(false);
const diagnosticError = ref('');
const sampleInjected = ref(false);

const diagnosticSampleContent = `Diagnostic AI reply for VCPMobile rendering.

<<<[TOOL_REQUEST]>>>
tool_name: 「始」ImageGen「末」
prompt: 「始」mobile diagnostic tool preview「末」
<<<[END_TOOL_REQUEST]>>>

[[VCP调用结果信息汇总:
- 工具名称: ImageGen
- 执行状态: success
- 返回内容: ![preview](https://picsum.photos/seed/vcp-mobile-tool/480/320)
- Result: **Markdown field**
  - item A
  - item B
VCP调用结果结束]]

Text after the tool block. This should remain attached to the same assistant message.`;

const hasCurrentChatTarget = computed(
  () => !!chatStore.currentSelectedItem?.id && !!chatStore.currentTopicId,
);

const currentTestTarget = computed(() => {
  if (!hasCurrentChatTarget.value) return 'Open a chat topic first';
  return `${chatStore.currentSelectedItem.type}:${chatStore.currentSelectedItem.id} / ${chatStore.currentTopicId}`;
});

const buildDiagnosticPayload = () => {
  if (!hasCurrentChatTarget.value) {
    throw new Error('Open a chat topic before injecting a diagnostic reply.');
  }

  return {
    ownerId: chatStore.currentSelectedItem.id,
    ownerType: chatStore.currentSelectedItem.type || 'agent',
    topicId: chatStore.currentTopicId,
    name: 'Diagnostic AI',
    content: diagnosticSampleContent,
  };
};

const curlExample = computed(() => {
  if (!diagnosticServer.value || !hasCurrentChatTarget.value) {
    return 'Start the API after opening a chat topic.';
  }

  const lineContinuation = '`';
  return [
    `adb forward tcp:5897 tcp:${diagnosticServer.value.port}`,
    `curl.exe -X POST "http://127.0.0.1:5897/inject/assistant?token=${diagnosticServer.value.token}" ${lineContinuation}`,
    `  -H "Content-Type: application/json" ${lineContinuation}`,
    `  --data '${JSON.stringify(buildDiagnosticPayload())}'`,
  ].join('\n');
});

const injectDiagnosticReply = async () => {
  diagnosticBusy.value = true;
  diagnosticError.value = '';
  sampleInjected.value = false;
  try {
    await invoke('inject_diagnostic_assistant_message', {
      payload: buildDiagnosticPayload(),
    });
    sampleInjected.value = true;
  } catch (error) {
    diagnosticError.value = String(error);
  } finally {
    diagnosticBusy.value = false;
  }
};

const startDiagnosticApi = async () => {
  diagnosticBusy.value = true;
  diagnosticError.value = '';
  try {
    diagnosticServer.value = await invoke<DiagnosticTestServerInfo>(
      'start_diagnostic_test_server',
      { bindLan: false },
    );
  } catch (error) {
    diagnosticError.value = String(error);
  } finally {
    diagnosticBusy.value = false;
  }
};

const stopDiagnosticApi = async () => {
  diagnosticBusy.value = true;
  diagnosticError.value = '';
  try {
    await invoke('stop_diagnostic_test_server');
    diagnosticServer.value = null;
  } catch (error) {
    diagnosticError.value = String(error);
  } finally {
    diagnosticBusy.value = false;
  }
};

const startDiagnostics = async () => {
  await diagnostics.startDiagnostics();
};

const stopDiagnostics = async () => {
  await diagnostics.stopDiagnostics();
};

const saveReport = async () => {
  await diagnostics.saveReport();
};

const goTestApp = () => {
  if (!state.running) void startDiagnostics();
  router.push('/chat');
};

onMounted(async () => {
  if (!state.info) await diagnostics.loadInfo();
});
</script>

<template>
  <div class="h-full overflow-y-auto bg-secondary-bg text-primary-text pb-safe">
    <header class="sticky top-0 z-10 border-b border-white/10 bg-secondary-bg/90 backdrop-blur-xl pt-[calc(var(--vcp-safe-top,24px)+12px)] px-4 pb-4">
      <div class="flex items-center gap-3">
        <button class="p-2 rounded-full bg-white/10 active:scale-95" @click="router.back()">
          <ArrowLeft :size="20" />
        </button>
        <div class="min-w-0">
          <h1 class="text-xl font-black tracking-tight">全局性能诊断</h1>
          <p class="text-xs opacity-60 truncate">{{ state.status }}</p>
        </div>
      </div>
    </header>

    <main class="p-4 space-y-4">
      <section class="rounded-2xl border border-yellow-400/30 bg-yellow-400/10 p-4 text-sm leading-relaxed">
        点开始后返回主界面正常操作；现在不固定 60 秒，点击顶部悬浮条即可结束。报告会记录 FPS、长任务、CPU/GPU 采样、DOM/渲染构造、流式输出刷新过程。
      </section>

      <section class="grid grid-cols-2 gap-3">
        <div class="rounded-2xl border border-white/10 bg-white/5 p-4">
          <Gauge class="mb-3 text-blue-400" :size="22" />
          <div class="text-2xl font-black">{{ diagnostics.averageFps.value }}</div>
          <div class="text-[11px] opacity-60 uppercase tracking-widest">平均 FPS</div>
        </div>
        <div class="rounded-2xl border border-white/10 bg-white/5 p-4">
          <Zap class="mb-3 text-yellow-400" :size="22" />
          <div class="text-2xl font-black">{{ diagnostics.maxLongTask.value }}</div>
          <div class="text-[11px] opacity-60 uppercase tracking-widest">最长卡顿 ms</div>
        </div>
        <div class="rounded-2xl border border-white/10 bg-white/5 p-4">
          <Timer class="mb-3 text-green-400" :size="22" />
          <div class="text-2xl font-black">{{ Math.round(state.runDurationMs / 1000) }}s</div>
          <div class="text-[11px] opacity-60 uppercase tracking-widest">采集时长</div>
        </div>
        <div class="rounded-2xl border border-white/10 bg-white/5 p-4">
          <Activity class="mb-3 text-purple-400" :size="22" />
          <div class="text-2xl font-black">{{ diagnostics.droppedFrameCount.value }}</div>
          <div class="text-[11px] opacity-60 uppercase tracking-widest">掉帧次数</div>
        </div>
      </section>

      <section class="rounded-2xl border border-white/10 bg-black/5 dark:bg-white/5 p-4 space-y-3">
        <div class="grid grid-cols-2 gap-2">
          <button v-if="!state.running" class="rounded-xl bg-blue-600 py-3 text-sm font-black text-white active:scale-95 flex items-center justify-center gap-2" @click="startDiagnostics">
            <Play :size="16" /> 开始采集
          </button>
          <button v-else class="rounded-xl bg-red-500 py-3 text-sm font-black text-white active:scale-95 flex items-center justify-center gap-2" @click="stopDiagnostics">
            <Square :size="16" /> 停止采集
          </button>
          <button class="rounded-xl bg-emerald-600 py-3 text-sm font-black text-white active:scale-95 flex items-center justify-center gap-2" @click="goTestApp">
            <Home :size="16" /> 去主界面测
          </button>
        </div>
        <button class="w-full rounded-xl bg-white/10 py-3 text-sm font-black active:scale-95 flex items-center justify-center gap-2 disabled:opacity-40" :disabled="state.running || !state.metrics.length" @click="saveReport">
          <Save :size="16" /> 保存报告
        </button>
        <textarea v-model="state.userNotes" class="min-h-20 w-full rounded-xl border border-white/10 bg-black/10 p-3 text-sm outline-none focus:border-blue-400" placeholder="可选：写下测试时的现象，例如：刚打开 APP 手机发热、滑动卡、发送消息后掉帧。"></textarea>
      </section>

      <section class="rounded-2xl border border-cyan-400/25 bg-cyan-400/10 p-4 space-y-3">
        <div class="flex items-center gap-2 font-black text-sm">
          <Activity :size="16" /> Runtime Test API
        </div>
        <div class="rounded-xl bg-black/10 p-3 text-xs break-all">
          Target: {{ currentTestTarget }}
        </div>
        <div class="grid grid-cols-2 gap-2">
          <button class="rounded-xl bg-cyan-600 py-3 text-sm font-black text-white active:scale-95 disabled:opacity-40 flex items-center justify-center gap-2" :disabled="diagnosticBusy || !hasCurrentChatTarget" @click="injectDiagnosticReply">
            <Zap :size="16" /> Inject Reply
          </button>
          <button v-if="!diagnosticServer" class="rounded-xl bg-white/10 py-3 text-sm font-black active:scale-95 disabled:opacity-40 flex items-center justify-center gap-2" :disabled="diagnosticBusy" @click="startDiagnosticApi">
            <Play :size="16" /> Start API
          </button>
          <button v-else class="rounded-xl bg-red-500 py-3 text-sm font-black text-white active:scale-95 disabled:opacity-40 flex items-center justify-center gap-2" :disabled="diagnosticBusy" @click="stopDiagnosticApi">
            <Square :size="16" /> Stop API
          </button>
        </div>
        <div v-if="diagnosticServer" class="space-y-2 text-xs">
          <div class="grid grid-cols-2 gap-2">
            <div class="rounded-xl bg-black/10 p-3">Port: {{ diagnosticServer.port }}</div>
            <div class="rounded-xl bg-black/10 p-3 break-all">Token: {{ diagnosticServer.token }}</div>
          </div>
          <pre class="rounded-xl bg-black/20 p-3 text-[11px] leading-relaxed overflow-x-auto whitespace-pre-wrap">{{ curlExample }}</pre>
        </div>
        <div v-if="sampleInjected" class="rounded-xl border border-green-500/25 bg-green-500/10 p-3 text-xs text-green-300">
          Diagnostic reply injected into the current topic.
        </div>
        <div v-if="diagnosticError" class="rounded-xl border border-red-500/25 bg-red-500/10 p-3 text-xs text-red-300 break-all">
          {{ diagnosticError }}
        </div>
      </section>

      <section v-if="state.info" class="rounded-2xl border border-white/10 bg-white/5 p-4 space-y-2 text-xs">
        <div class="flex items-center gap-2 font-black text-sm"><FileJson :size="16" /> 报告目录</div>
        <div class="break-all rounded-xl bg-black/10 p-3 font-mono opacity-80">{{ state.info.reportsDir }}</div>
        <div class="opacity-60">{{ state.info.platform }} / {{ state.info.arch }} / {{ state.info.debugBuild ? 'debug' : 'release' }}</div>
      </section>

      <section v-if="state.savedReport" class="rounded-2xl border border-green-500/30 bg-green-500/10 p-4 space-y-2 text-sm">
        <div class="flex items-center gap-2 font-black text-green-400"><CheckCircle2 :size="18" /> 已保存</div>
        <div class="break-all font-mono text-xs opacity-80">{{ state.savedReport.path }}</div>
      </section>

      <section class="rounded-2xl border border-white/10 bg-white/5 p-4 space-y-3">
        <h2 class="font-black">摘要</h2>
        <div class="grid grid-cols-2 gap-2 text-xs">
          <div class="rounded-xl bg-black/10 p-3">最低 FPS：{{ diagnostics.minFps.value }}</div>
          <div class="rounded-xl bg-black/10 p-3">低帧窗口：{{ diagnostics.jankWindows.value }}</div>
          <div class="rounded-xl bg-black/10 p-3">长任务：{{ state.longTasks.length }}</div>
          <div class="rounded-xl bg-black/10 p-3">P95 帧耗时：{{ diagnostics.p95FrameMs.value }}ms</div>
          <div class="rounded-xl bg-black/10 p-3">JS Heap：{{ diagnostics.formatBytes(state.memorySnapshot?.usedJSHeapSize) }}</div>
          <div class="rounded-xl bg-black/10 p-3">Native RSS：{{ diagnostics.formatBytes(state.memorySnapshot?.nativeRssBytes) }}</div>
          <div class="rounded-xl bg-black/10 p-3">CPU：{{ state.lastCpuPercent ?? '不可用' }}%</div>
          <div class="rounded-xl bg-black/10 p-3">GPU：{{ state.lastGpuBusyPercent ?? '不可用' }}%</div>
          <div class="rounded-xl bg-black/10 p-3">线程：{{ state.memorySnapshot?.nativeThreads ?? '不可用' }}</div>
          <div class="rounded-xl bg-black/10 p-3">FD：{{ state.memorySnapshot?.nativeFdCount ?? '不可用' }}</div>
          <div class="rounded-xl bg-black/10 p-3">DOM节点：{{ state.renderSnapshot?.allElements ?? '不可用' }}</div>
          <div class="rounded-xl bg-black/10 p-3">Trace：{{ state.traceEvents.length }}</div>
        </div>
      </section>

      <section class="rounded-2xl border border-white/10 bg-white/5 p-4 space-y-3">
        <h2 class="font-black">渲染与运行过程</h2>
        <div class="grid grid-cols-2 gap-2 text-xs">
          <div class="rounded-xl bg-black/10 p-3">消息节点：{{ state.renderSnapshot?.messageNodes ?? '不可用' }}</div>
          <div class="rounded-xl bg-black/10 p-3">流式节点：{{ state.renderSnapshot?.streamingNodes ?? '不可用' }}</div>
          <div class="rounded-xl bg-black/10 p-3">图片：{{ state.renderSnapshot?.imageNodes ?? '不可用' }}</div>
          <div class="rounded-xl bg-black/10 p-3">固定/Sticky：{{ state.renderSnapshot?.fixedNodes ?? '不可用' }}</div>
        </div>
        <button class="w-full rounded-xl bg-white/10 py-2 text-xs font-bold active:scale-95" @click="diagnostics.collectRenderSnapshot()">
          立即记录渲染快照
        </button>
      </section>

      <section class="rounded-2xl border border-white/10 bg-white/5 p-4 space-y-3">
        <h2 class="font-black">按页面统计</h2>
        <div v-if="!Object.keys(diagnostics.routeBreakdown.value).length" class="py-5 text-center text-sm opacity-50">暂无页面数据</div>
        <div v-for="(item, route) in diagnostics.routeBreakdown.value" :key="route" class="rounded-xl bg-black/10 p-3 text-xs">
          <div class="mb-2 font-black">{{ route }}</div>
          <div class="grid grid-cols-2 gap-2 opacity-75">
            <span>均帧：{{ item.avgFps }}</span>
            <span>采样：{{ item.fpsSamples }}</span>
            <span>长任务：{{ item.longTasks }}</span>
            <span>最长：{{ item.maxLongTaskMs }}ms</span>
          </div>
        </div>
      </section>

      <section class="rounded-2xl border border-white/10 bg-white/5 p-4 space-y-3">
        <h2 class="font-black">实时指标</h2>
        <div v-if="!state.metrics.length" class="py-8 text-center text-sm opacity-50">暂无数据</div>
        <div v-for="metric in state.metrics.slice(0, 80)" :key="`${metric.ts}-${metric.name}`" class="rounded-xl bg-black/10 p-3 text-sm">
          <div class="flex justify-between gap-3">
            <span class="font-bold">{{ metric.name }}</span>
            <span class="font-mono">{{ metric.value }}{{ metric.unit ? ` ${metric.unit}` : '' }}</span>
          </div>
          <div class="mt-1 flex justify-between gap-2 text-[11px] opacity-50">
            <span>{{ metric.route }}</span>
            <span>{{ new Date(metric.ts).toLocaleTimeString() }}</span>
          </div>
          <div v-if="metric.detail" class="mt-1 break-all text-xs opacity-55">{{ metric.detail }}</div>
        </div>
      </section>
    </main>
  </div>
</template>
