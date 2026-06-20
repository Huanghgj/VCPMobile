<script setup lang="ts">
import { ref, watch } from 'vue';
import {
  X,
  Search,
  Plus,
  Bug,
  Trash2,
  Boxes,
  NotebookPen
} from 'lucide-vue-next';
import SlidePage from '../../components/ui/SlidePage.vue';
import { useNotificationStore } from '../../core/stores/notification';
import { useNotificationProcessor } from '../../core/composables/useNotificationProcessor';
import { useSidebarSwipe } from '../../core/composables/useSidebarSwipe';
import { useOverlayStore } from '../../core/stores/overlay';
import NotificationStatusBar from './NotificationStatusBar.vue';
import NotificationList from './NotificationList.vue';
import { useNotificationGrouping } from './composables/useNotificationGrouping';

const props = withDefaults(
  defineProps<{
    isOpen: boolean;
    zIndex?: number;
  }>(),
  {
    zIndex: 50
  }
);

const emit = defineEmits<{
  (e: 'close'): void;
}>();

const store = useNotificationStore();
const { processPayload } = useNotificationProcessor();
const overlayStore = useOverlayStore();
const pageRef = ref<HTMLElement | null>(null);
const showDebugPanel = ref(false);
const isDev = import.meta.env.DEV;
useSidebarSwipe(pageRef, { type: 'right' });

const {
  activeTab,
  searchQuery,
  tabs,
  filteredNotifications
} = useNotificationGrouping(() => store.historyList);

// Handle drawer state and read status
watch(
  () => props.isOpen,
  (isOpen) => {
    store.isDrawerOpen = isOpen;
    if (isOpen) {
      store.markAllRead();
    }
  },
  { immediate: true }
);

const openDistributedView = () => {
  emit('close');
  requestAnimationFrame(() => {
    overlayStore.openDistributed();
  });
};

const openDiaryView = () => {
  emit('close');
  requestAnimationFrame(() => {
    overlayStore.openDiary();
  });
};

const triggerDebugNotifications = () => {
  if (!isDev) return;

  const randomSuffix = () => Math.random().toString(36).substring(2, 5);
  const debugPayloads = [
    {
      type: 'vcp_log',
      data: {
        tool_name: 'DailyNote',
        status: 'success',
        content: JSON.stringify({
          MaidName: '[Nova]Nova',
          timestamp: '2026-05-26T21:49:09.295+08:00',
        }),
      },
    },
    {
      type: 'vcp_log',
      data: {
        tool_name: 'PowerShellExecutor',
        status: 'success',
        source: 'VCPLog',
        content: JSON.stringify({
          MaidName: '艾米莉亚',
          timestamp: '2026-05-26T21:38:00',
          original_plugin_output: {
            status: 'success',
            stdout:
              'G:\\VCPMobile\\src\\components\\ui> ls\n\n    Directory: G:\\VCPMobile\\src\\components\\ui\n\nMode                 LastWriteTime         Length Name\n----                 -------------         ------ ----\n-a----        2026/05/26     21:38           1520 ToastItem.vue\n',
          },
        }),
      },
    },
    {
      type: 'vcp_log',
      data: {
        tool_name: 'AdbBridge',
        status: 'error',
        source: 'VCPLog',
        content: '执行错误: {"plugin_error": "device \'emulator-5554\' not found."}',
      },
    },
    {
      type: 'vcp_log',
      data: {
        source: 'DistPluginManager',
        content: '已成功同步 3 个分布式计算节点状态，物理核心 CPU 综合占用率 14%。',
      },
    },
    {
      type: 'video_generation_status',
      data: {
        status: 'Succeed',
        timestamp: '2026-05-26T21:38:00',
        original_plugin_output: {
          message: '视频已生成，URL: https://cdn.vcpchat.com/generations/vid_77189b.mp4',
        },
      },
    },
    {
      type: 'tool_approval_request',
      data: {
        requestId: `debug_req_${randomSuffix()}`,
        toolName: 'PowerShellExecutor',
        maid: '艾米莉亚',
        args: { command: 'cargo check --workspace' },
        timestamp: '2026-05-26 21:38:00',
      },
    },
    {
      type: 'connection_ack',
      message: 'VCPLog 连接成功！',
    },
  ];

  debugPayloads.forEach((payload) => {
    const processed = processPayload(payload);
    if (processed && !processed.silent) {
      store.addNotification(processed);
    }
  });
};

// Debug mock injector
const injectMockNotification = (type: 'rag' | 'thinking' | 'approval' | 'error') => {
  const timestamp = Date.now();
  if (type === 'rag') {
    store.addNotification({
      id: `mock-rag-${timestamp}`,
      type: 'tool',
      title: 'RAG 知识检索召回',
      message: '查询词: "VCP 系统架构与低功耗渲染规范"',
      subtitle: '知识库: vcp_core_docs',
      source: 'VCPInfo',
      category: 'RAG',
      infoType: 'RAG_RETRIEVAL_DETAILS',
      tags: ['RAG', 'Rerank', 'Geo'],
      meta: [
        { label: 'K', value: '5' },
        { label: '耗时', value: '142ms' }
      ],
      structured: {
        kind: 'rag',
        summary: '成功检索到 3 个相关文本分片',
        rows: [
          {
            title: 'vcp_low_power_spec.md',
            subtitle: '分片 #12',
            body: 'OLED 屏幕必须采用深色或纯黑背景以降低功耗。避免使用高开销的 backdrop-filter 滤镜。',
            metrics: [
              { label: 'score', value: '0.92' },
              { label: 'distance', value: '0.0869' }
            ]
          },
          {
            title: 'vcp_ui_guidelines.md',
            subtitle: '分片 #4',
            body: '保持 UI 静态化，避免高频重绘。过渡动画时长控制在 200ms 以内。',
            metrics: [
              { label: 'score', value: '0.81' }
            ]
          }
        ]
      },
      timestamp,
      rawPayload: { query: 'VCP system architecture', k: 5, timeUsed: 142 }
    });
  } else if (type === 'thinking') {
    store.addNotification({
      id: `mock-think-${timestamp}`,
      type: 'agent',
      title: '元思考链分析',
      message: '分析用户请求: "优化通知中心视觉与交互体验"',
      source: 'Meta',
      category: 'Meta',
      infoType: 'META_THINKING_CHAIN',
      structured: {
        kind: 'thinking',
        rows: [
          { title: '阶段 1: 视觉风格重构', body: '放弃纯黑设计，采用粉白/淡粉玻璃质感，融入聊天气泡堆叠感。' },
          { title: '阶段 2: 动画性能优化', body: '限制高功耗动画，仅保留基础的 transform/opacity 柔和过渡。' }
        ]
      },
      timestamp,
      rawPayload: { stages: 2, fromCache: false }
    });
  } else if (type === 'approval') {
    store.addNotification({
      id: `mock-appr-${timestamp}`,
      type: 'warning',
      title: '敏感工具执行审批',
      message: '智能体 Maid 请求执行高危指令: "rm -rf ./tmp/cache"',
      source: 'System',
      category: 'System',
      infoType: 'tool_approval_request',
      actions: [
        { label: '拒绝执行', value: false, color: 'red' },
        { label: '允许执行', value: true, color: 'green' }
      ],
      timestamp,
      rawPayload: {
        type: 'tool_approval_request',
        data: {
          requestId: `mock-appr-${timestamp}`,
          args: { command: 'rm -rf ./tmp/cache' },
          maid: 'VCP-01'
        }
      }
    });
  } else if (type === 'error') {
    store.addNotification({
      id: `mock-err-${timestamp}`,
      type: 'error',
      title: 'WebSocket 连接异常断开',
      message: '尝试连接 vcp-core-status 失败，系统将在 5 秒后自动重试。',
      source: 'VCPCore',
      category: 'System',
      timestamp
    });
  }
};
</script>

<template>
  <SlidePage :is-open="isOpen" :z-index="zIndex" @close="emit('close')">
    <div ref="pageRef" class="flex flex-col h-full bg-[#fff5f7] text-slate-800 font-sans select-none overflow-hidden">
      <!-- Header -->
      <div class="notification-page-header bg-white/90 border-b border-pink-100/80 px-4 pb-3 flex flex-col gap-2.5 shrink-0 shadow-sm">
        <div class="flex items-center justify-between">
          <div class="flex items-center gap-2">
            <h1 class="text-sm font-bold tracking-tight text-slate-900">通知中心</h1>
            <span class="text-[9px] font-mono px-1.5 py-0.5 rounded-full bg-pink-50 text-pink-600 border border-pink-100/80 font-semibold">VCPMobile</span>
          </div>
          <div class="flex items-center gap-2">
            <button
              v-if="isDev"
              @click="triggerDebugNotifications"
              aria-label="推送调试通知"
              class="p-1.5 rounded-lg bg-pink-50/60 border border-pink-100/50 text-pink-500 hover:bg-pink-50 active:scale-95 transition-all duration-150 motion-reduce:transition-none"
            >
              <Bug class="w-3.5 h-3.5" />
            </button>
            <button
              v-if="isDev"
              @click="showDebugPanel = !showDebugPanel"
              aria-label="切换调试面板"
              class="p-1.5 rounded-lg bg-pink-50/60 border border-pink-100/50 text-pink-500 hover:bg-pink-50 active:scale-95 transition-all duration-150 motion-reduce:transition-none"
            >
              <Plus class="w-3.5 h-3.5" />
            </button>
            <button
              @click="store.clearHistory"
              aria-label="清空通知"
              class="p-1.5 rounded-lg bg-pink-50/60 border border-pink-100/50 text-pink-500 hover:bg-pink-50 active:scale-95 transition-all duration-150 motion-reduce:transition-none"
            >
              <Trash2 class="w-3.5 h-3.5" />
            </button>
            <button
              @click="emit('close')"
              aria-label="关闭通知中心"
              class="p-1.5 rounded-lg bg-pink-50/60 border border-pink-100/50 text-pink-500 hover:bg-pink-50 active:scale-95 transition-all duration-150 motion-reduce:transition-none"
            >
              <X class="w-3.5 h-3.5" />
            </button>
          </div>
        </div>

        <!-- Search Input -->
        <div class="relative flex items-center">
          <Search class="absolute left-3 w-3.5 h-3.5 text-pink-400" />
          <input
            v-model="searchQuery"
            type="text"
            aria-label="搜索通知"
            placeholder="搜索标题、内容、标签、原始数据..."
            class="w-full bg-pink-50/30 border border-pink-100/80 rounded-lg px-3 py-1.5 pl-9 text-xs text-slate-700 placeholder-pink-300 focus:outline-none focus:border-pink-300 focus:bg-white transition-all duration-200 font-sans"
          />
          <button
            v-if="searchQuery"
            @click="searchQuery = ''"
            class="absolute right-3 text-[10px] text-pink-400 hover:text-pink-600 font-mono"
          >
            清除
          </button>
        </div>
      </div>

      <!-- Debug Panel -->
      <div v-if="isDev && showDebugPanel" class="bg-white/80 border-b border-pink-100/60 p-3 shrink-0 flex flex-col gap-2 mx-3 mt-2 rounded-xl shadow-sm">
        <div class="text-[9px] font-mono font-bold text-pink-400 tracking-wider">DEBUG INJECTOR</div>
        <div class="grid grid-cols-4 gap-2">
          <button @click="injectMockNotification('rag')" class="flex items-center justify-center gap-1 py-1.5 rounded-lg bg-emerald-50 border border-emerald-100 text-[10px] text-emerald-700 font-medium active:scale-95 transition-transform duration-100">
            <Plus class="w-2.5 h-2.5" /> RAG
          </button>
          <button @click="injectMockNotification('thinking')" class="flex items-center justify-center gap-1 py-1.5 rounded-lg bg-indigo-50 border border-indigo-100 text-[10px] text-indigo-700 font-medium active:scale-95 transition-transform duration-100">
            <Plus class="w-2.5 h-2.5" /> 思路
          </button>
          <button @click="injectMockNotification('approval')" class="flex items-center justify-center gap-1 py-1.5 rounded-lg bg-amber-50 border border-amber-100 text-[10px] text-amber-700 font-medium active:scale-95 transition-transform duration-100">
            <Plus class="w-2.5 h-2.5" /> 审批
          </button>
          <button @click="injectMockNotification('error')" class="flex items-center justify-center gap-1 py-1.5 rounded-lg bg-rose-50 border border-rose-100 text-[10px] text-rose-700 font-medium active:scale-95 transition-transform duration-100">
            <Plus class="w-2.5 h-2.5" /> 异常
          </button>
        </div>
      </div>

      <!-- Status Bar -->
      <NotificationStatusBar />

      <!-- Quick Actions -->
      <div class="px-3 py-2 border-b border-pink-100/60 bg-white/40 shrink-0 grid grid-cols-2 gap-2">
        <button
          class="w-full py-2.5 px-3 rounded-lg bg-pink-500 text-white flex items-center justify-center gap-2 active:scale-[0.98] transition-transform shadow-sm shadow-pink-200"
          @click="openDistributedView"
        >
          <Boxes class="w-3.5 h-3.5" />
          <span class="font-bold text-xs leading-none">插件中心</span>
        </button>
        <button
          class="w-full py-2.5 px-3 rounded-lg bg-violet-500 text-white flex items-center justify-center gap-2 active:scale-[0.98] transition-transform shadow-sm shadow-violet-200"
          @click="openDiaryView"
        >
          <NotebookPen class="w-3.5 h-3.5" />
          <span class="font-bold text-xs leading-none">日记本</span>
        </button>
      </div>

      <!-- Category Segmented Tabs -->
      <div class="bg-transparent shrink-0 overflow-x-auto flex items-center gap-1.5 px-3 py-2 scrollbar-none">
        <button
          v-for="tab in tabs"
          :key="tab.id"
          v-show="tab.count > 0 || tab.id === 'all' || tab.id === 'unread'"
          @click="activeTab = tab.id"
          class="px-3 py-1 rounded-full text-[11px] font-medium transition-all duration-150 shrink-0 flex items-center gap-1 border active:scale-95 motion-reduce:transition-none"
          :class="[
            activeTab === tab.id
              ? 'bg-pink-500 text-white border-pink-500 shadow-sm shadow-pink-200'
              : 'bg-white/80 text-slate-500 border-pink-100/60 hover:text-slate-700 hover:bg-white'
          ]"
        >
          <span>{{ tab.label }}</span>
          <span
            class="font-mono text-[9px] px-1.5 py-0.2 rounded-full"
            :class="[
              activeTab === tab.id
                ? 'bg-white/20 text-white'
                : 'bg-pink-50 text-pink-500'
            ]"
          >
            {{ tab.count }}
          </span>
        </button>
      </div>

      <!-- Notification List -->
      <NotificationList
        :items="filteredNotifications"
      />
    </div>
  </SlidePage>
</template>

<style scoped>
.notification-page-header {
  padding-top: calc(var(--vcp-safe-top, 24px) + 12px);
}

.scrollbar-none::-webkit-scrollbar {
  display: none;
}
.scrollbar-none {
  -ms-overflow-style: none;
  scrollbar-width: none;
}
</style>
