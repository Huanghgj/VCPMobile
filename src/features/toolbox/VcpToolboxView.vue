<script setup lang="ts">
import { computed, reactive, ref, watch } from 'vue';
import { invoke } from '@tauri-apps/api/core';
import SettingsActionButton from '../../components/settings/SettingsActionButton.vue';
import SettingsCard from '../../components/settings/SettingsCard.vue';
import SettingsInlineStatus from '../../components/settings/SettingsInlineStatus.vue';
import SettingsTextField from '../../components/settings/SettingsTextField.vue';
import { callVcpToolboxApi } from '../../core/api/vcpToolbox';
import { useNotificationStore } from '../../core/stores/notification';
import { useSettingsStore } from '../../core/stores/settings';

const props = withDefaults(defineProps<{ isOpen?: boolean; zIndex?: number }>(), {
  isOpen: false,
  zIndex: 50,
});
const emit = defineEmits<{ close: [] }>();
const notificationStore = useNotificationStore();
const settingsStore = useSettingsStore();

const adminAuth = reactive({ username: '', password: '' });
const adminAuthTouched = ref(false);
const loading = ref(false);
const status = ref<{ type: 'success' | 'error' | 'loading' | null; message: string }>({ type: null, message: '' });
const lifecycle = ref<any>(null);
const models = ref<any[]>([]);
const plugins = ref<any[]>([]);
const systemResources = ref<any>(null);
const serverLog = ref('');
const weather = ref<any>(null);
const taskAssistant = ref<any>(null);
const vectorDb = ref<any>(null);
const diarySearch = reactive({ term: '', folder: '', limit: 5 });
const diaryResults = ref<any[]>([]);
const distributedTools = ref<any[]>([]);
const selectedDistributedTool = ref('MobileDeviceInfo');
const distributedToolArgs = ref('{\n  \n}');
const distributedToolResult = ref<any>(null);

const modelCount = computed(() => models.value.length);
const enabledPluginCount = computed(() => plugins.value.filter((plugin: any) => plugin.enabled !== false && plugin.isEnabled !== false).length);
const selectedDistributedToolInfo = computed(() => distributedTools.value.find((tool: any) => tool.name === selectedDistributedTool.value));

const distributedPresets = [
  {
    label: '设备信息',
    tool: 'MobileDeviceInfo',
    args: {},
  },
  {
    label: '列出移动文件沙箱',
    tool: 'MobileFileOperator',
    args: { command: 'ListAllowedDirectories' },
  },
  {
    label: '写入测试文件',
    tool: 'MobileFileOperator',
    args: {
      command: 'WriteFile',
      filePath: 'mobile://distributed-files/hello-vcp.txt',
      content: 'Hello from VCPMobile distributed plugin tester.',
    },
  },
  {
    label: '读取测试文件',
    tool: 'MobileFileOperator',
    args: {
      command: 'ReadFile',
      filePath: 'mobile://distributed-files/hello-vcp.txt',
    },
  },
  {
    label: '等待用户回复',
    tool: 'MobileWaitingForUrReply',
    args: {
      title: '插件测试',
      prompt: '这是 MobileWaitingForUrReply 测试。请选择或输入一个回复。',
      placeholder: '移动端回复',
      option01: '确认',
      option02: '稍后',
      timeout: 120,
    },
  },
];

const adminOptions = () => ({
  adminUsername: adminAuth.username.trim(),
  adminPassword: adminAuth.password,
});

const hydrateAdminAuthFromSettings = async () => {
  if (!settingsStore.settings) {
    await settingsStore.fetchSettings();
  }
  if (adminAuthTouched.value) return;
  adminAuth.username = settingsStore.settings?.adminUsername || '';
  adminAuth.password = settingsStore.settings?.adminPassword || '';
};

watch(
  () => props.isOpen,
  (isOpen) => {
    if (isOpen) {
      void hydrateAdminAuthFromSettings();
    }
  },
  { immediate: true },
);

const runAction = async (message: string, action: () => Promise<void>) => {
  loading.value = true;
  status.value = { type: 'loading', message };
  try {
    await action();
    status.value = { type: 'success', message: '已更新' };
  } catch (error: any) {
    status.value = { type: 'error', message: String(error?.message || error) };
  } finally {
    loading.value = false;
  }
};

const loadCoreStatus = () => runAction('正在读取核心状态...', async () => {
  const [lifecycleRes, modelsRes] = await Promise.all([
    callVcpToolboxApi({ path: '/admin_api/server/lifecycle', ...adminOptions() }),
    callVcpToolboxApi<{ data?: any[] } | any>({ path: '/v1/models' }),
  ]);
  lifecycle.value = lifecycleRes.data;
  models.value = Array.isArray((modelsRes.data as any)?.data) ? (modelsRes.data as any).data : [];
});

const loadAdminOverview = () => runAction('正在读取后台面板接口...', async () => {
  const [pluginsRes, resourcesRes, weatherRes, taskRes, vectorRes] = await Promise.all([
    callVcpToolboxApi<any[]>({ path: '/admin_api/plugins', ...adminOptions() }),
    callVcpToolboxApi({ path: '/admin_api/system-monitor/system/resources', ...adminOptions() }),
    callVcpToolboxApi({ path: '/admin_api/weather', ...adminOptions() }),
    callVcpToolboxApi({ path: '/admin_api/task-assistant/status', ...adminOptions() }),
    callVcpToolboxApi({ path: '/admin_api/vectordb-status', ...adminOptions() }),
  ]);
  plugins.value = Array.isArray(pluginsRes.data) ? pluginsRes.data : (pluginsRes.data as any)?.plugins || [];
  systemResources.value = resourcesRes.data;
  weather.value = weatherRes.data;
  taskAssistant.value = taskRes.data;
  vectorDb.value = vectorRes.data;
});

const loadServerLog = () => runAction('正在拉取后端日志...', async () => {
  const result = await callVcpToolboxApi<{ log?: string; content?: string; raw?: string }>({
    path: '/admin_api/server-log',
    ...adminOptions(),
  });
  serverLog.value = result.data?.log || result.data?.content || result.data?.raw || JSON.stringify(result.data, null, 2);
});

const searchDailyNotes = () => runAction('正在召回日记...', async () => {
  const term = diarySearch.term.trim();
  if (!term) throw new Error('请输入召回关键词');

  const params = new URLSearchParams({
    term,
    limit: String(Math.max(1, Math.min(20, Number(diarySearch.limit) || 5))),
  });
  if (diarySearch.folder.trim()) {
    params.set('folder', diarySearch.folder.trim());
  }

  const result = await callVcpToolboxApi<{ notes?: any[]; total?: number; limited?: boolean }>({
    path: `/admin_api/dailynotes/search?${params.toString()}`,
    ...adminOptions(),
  });

  diaryResults.value = Array.isArray(result.data?.notes) ? result.data.notes : [];
  const fullContent = diaryResults.value
    .map((note: any, index: number) => {
      const title = note.title || note.fileName || note.filename || note.path || `结果 ${index + 1}`;
      const content = note.content || note.snippet || note.text || note.preview || note.message || note;
      const meta = [
        note.score !== undefined ? `score=${note.score}` : '',
        note.source ? `source=${note.source}` : '',
        note.folder ? `folder=${note.folder}` : '',
      ].filter(Boolean).join(' · ');
      return [`${index + 1}. ${title}`, meta, typeof content === 'string' ? content : JSON.stringify(content, null, 2)]
        .filter(Boolean)
        .join('\n');
    })
    .join('\n\n');

  notificationStore.addNotification({
    type: 'diary',
    title: `日记召回：${term}`,
    summary: `召回 ${diaryResults.value.length} 条：${term}`,
    message: fullContent || '没有召回到相关日记。',
    isPreformatted: true,
    duration: 10000,
    rawPayload: {
      type: 'daily_note_recall_result',
      query: term,
      folder: diarySearch.folder.trim() || null,
      total: result.data?.total ?? diaryResults.value.length,
      limited: result.data?.limited ?? false,
      notes: diaryResults.value,
    },
  });
});

const loadDistributedTools = () => runAction('正在读取本机分布式工具...', async () => {
  const tools = await invoke<any[]>('list_distributed_tools');
  distributedTools.value = tools;
  if (!tools.some((tool: any) => tool.name === selectedDistributedTool.value)) {
    selectedDistributedTool.value = tools[0]?.name || '';
  }
});

const applyDistributedPreset = (preset: typeof distributedPresets[number]) => {
  selectedDistributedTool.value = preset.tool;
  distributedToolArgs.value = JSON.stringify(preset.args, null, 2);
  distributedToolResult.value = null;
};

const runDistributedTool = () => runAction('正在执行本机分布式工具...', async () => {
  if (!selectedDistributedTool.value) throw new Error('请选择工具');
  let args: any = {};
  try {
    args = distributedToolArgs.value.trim() ? JSON.parse(distributedToolArgs.value) : {};
  } catch (error: any) {
    throw new Error(`参数不是合法 JSON：${error?.message || error}`);
  }
  distributedToolResult.value = await invoke('execute_distributed_tool', {
    toolName: selectedDistributedTool.value,
    args,
  });
});

const clearLocalData = () => {
  lifecycle.value = null;
  models.value = [];
  plugins.value = [];
  systemResources.value = null;
  serverLog.value = '';
  weather.value = null;
  taskAssistant.value = null;
  vectorDb.value = null;
  diaryResults.value = [];
  distributedToolResult.value = null;
  status.value = { type: null, message: '' };
};
</script>

<template>
  <Teleport to="#vcp-feature-overlays" :disabled="!props.isOpen">
    <Transition name="fade">
      <div
        v-if="props.isOpen"
        class="toolbox-view fixed inset-0 flex flex-col bg-secondary-bg text-primary-text pointer-events-auto"
        :style="{ zIndex: props.zIndex }"
      >
        <header class="p-4 flex items-center justify-between border-b border-white/10 pt-[calc(var(--vcp-safe-top,24px)+20px)] pb-5 shrink-0">
          <div>
            <h2 class="text-xl font-bold">VCPToolBox 后端</h2>
            <p class="text-xs opacity-50 mt-1">模型、生命周期、插件、日志与系统状态</p>
          </div>
          <button @click="emit('close')" class="p-2.5 bg-white/10 rounded-full active:scale-90 transition-all flex items-center justify-center">
            <svg width="22" height="22" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round">
              <line x1="18" y1="6" x2="6" y2="18"></line>
              <line x1="6" y1="6" x2="18" y2="18"></line>
            </svg>
          </button>
        </header>

        <div class="flex-1 overflow-y-auto p-5 space-y-4 pb-safe">
          <SettingsCard>
            <div class="space-y-4">
              <div>
                <div class="text-sm font-bold">Admin API 认证</div>
                <div class="text-xs opacity-60 mt-1">默认复用全局设置里的管理员账号/密码；这里修改只作为本次页面的临时覆盖。</div>
              </div>
              <div class="grid grid-cols-1 gap-3">
                <SettingsTextField
                  v-model="adminAuth.username"
                  label="Admin Username"
                  placeholder="复用全局设置"
                  @update:model-value="adminAuthTouched = true"
                />
                <SettingsTextField
                  v-model="adminAuth.password"
                  is-secure
                  label="Admin Password"
                  placeholder="复用全局设置"
                  @update:model-value="adminAuthTouched = true"
                />
              </div>
              <SettingsInlineStatus v-if="status.type" :type="status.type" :message="status.message" />
            </div>
          </SettingsCard>

          <div class="grid grid-cols-2 gap-3">
            <SettingsActionButton variant="primary" full-width :loading="loading" @click="loadCoreStatus">核心状态</SettingsActionButton>
            <SettingsActionButton variant="secondary" full-width :loading="loading" @click="loadAdminOverview">后台概览</SettingsActionButton>
            <SettingsActionButton variant="secondary" full-width :loading="loading" @click="loadServerLog">后端日志</SettingsActionButton>
            <SettingsActionButton variant="ghost" full-width @click="clearLocalData">清空显示</SettingsActionButton>
          </div>

          <SettingsCard>
            <div class="space-y-4">
              <div>
                <div class="text-sm font-bold">日记召回</div>
                <div class="text-xs opacity-60 mt-1 leading-relaxed">
                  调用 VCPToolBox 的 /admin_api/dailynotes/search，并把召回结果写入通知中心。
                </div>
              </div>
              <div class="grid grid-cols-1 gap-3">
                <SettingsTextField v-model="diarySearch.term" label="关键词" placeholder="例如：Nova 今天 情绪" />
                <SettingsTextField v-model="diarySearch.folder" label="日记本文件夹（可选）" placeholder="留空搜索全部" />
                <div>
                  <label class="block text-[10px] font-bold uppercase opacity-40 mb-1.5 px-1">召回数量</label>
                  <input v-model.number="diarySearch.limit" type="number" min="1" max="20"
                    class="w-full bg-black/5 dark:bg-white/5 border border-black/5 dark:border-white/10 rounded-xl px-3 py-2.5 text-sm outline-none" />
                </div>
              </div>
              <SettingsActionButton variant="primary" full-width :loading="loading" @click="searchDailyNotes">
                召回日记并通知
              </SettingsActionButton>
            </div>
          </SettingsCard>

          <SettingsCard>
            <div class="space-y-4">
              <div>
                <div class="text-sm font-bold">分布式插件测试台</div>
                <div class="text-xs opacity-60 mt-1 leading-relaxed">
                  本地直连 VCPMobile 的分布式工具注册表，可测试移动端插件，不需要先连接主服务器。这里相当于移动端的人类工具箱调试区。
                </div>
              </div>

              <div class="grid grid-cols-2 gap-2">
                <button
                  v-for="preset in distributedPresets"
                  :key="preset.label"
                  class="rounded-xl bg-black/5 dark:bg-white/5 border border-black/5 dark:border-white/10 px-3 py-2 text-xs font-bold text-left active:scale-[0.98] transition-transform"
                  @click="applyDistributedPreset(preset)"
                >
                  {{ preset.label }}
                </button>
              </div>

              <div class="space-y-2">
                <label class="block text-[10px] font-bold uppercase opacity-40 px-1">工具</label>
                <select
                  v-model="selectedDistributedTool"
                  class="w-full bg-black/5 dark:bg-white/5 border border-black/5 dark:border-white/10 rounded-xl px-3 py-2.5 text-sm outline-none"
                >
                  <option value="MobileDeviceInfo">MobileDeviceInfo</option>
                  <option value="MobileFileOperator">MobileFileOperator</option>
                  <option value="MobileWaitingForUrReply">MobileWaitingForUrReply</option>
                  <option
                    v-for="tool in distributedTools"
                    :key="tool.name"
                    :value="tool.name"
                  >
                    {{ tool.name }}
                  </option>
                </select>
                <div v-if="selectedDistributedToolInfo" class="text-xs opacity-55 leading-relaxed px-1">
                  {{ selectedDistributedToolInfo.description }}
                </div>
              </div>

              <div>
                <label class="block text-[10px] font-bold uppercase opacity-40 mb-1.5 px-1">参数 JSON</label>
                <textarea
                  v-model="distributedToolArgs"
                  rows="8"
                  class="w-full bg-black/5 dark:bg-white/5 border border-black/5 dark:border-white/10 rounded-xl px-3 py-2.5 text-xs font-mono outline-none resize-y"
                  spellcheck="false"
                ></textarea>
              </div>

              <div class="grid grid-cols-2 gap-3">
                <SettingsActionButton variant="secondary" full-width :loading="loading" @click="loadDistributedTools">
                  刷新工具列表
                </SettingsActionButton>
                <SettingsActionButton variant="primary" full-width :loading="loading" @click="runDistributedTool">
                  执行测试
                </SettingsActionButton>
              </div>

              <div v-if="distributedTools.length" class="text-xs opacity-55">
                已注册 {{ distributedTools.length }} 个本机分布式工具。
              </div>
            </div>
          </SettingsCard>

          <SettingsCard v-if="distributedToolResult">
            <div class="text-sm font-bold mb-2">分布式插件测试结果</div>
            <pre class="text-[11px] whitespace-pre-wrap break-words opacity-80 max-h-96 overflow-y-auto">{{ JSON.stringify(distributedToolResult, null, 2) }}</pre>
          </SettingsCard>

          <div class="grid grid-cols-2 gap-3">
            <SettingsCard>
              <div class="text-[10px] opacity-40 uppercase font-black tracking-widest">Models</div>
              <div class="text-2xl font-black mt-1">{{ modelCount }}</div>
            </SettingsCard>
            <SettingsCard>
              <div class="text-[10px] opacity-40 uppercase font-black tracking-widest">Plugins</div>
              <div class="text-2xl font-black mt-1">{{ enabledPluginCount }}/{{ plugins.length }}</div>
            </SettingsCard>
          </div>

          <SettingsCard v-if="lifecycle">
            <div class="text-sm font-bold mb-2">生命周期</div>
            <pre class="text-[11px] whitespace-pre-wrap break-words opacity-80">{{ JSON.stringify(lifecycle, null, 2) }}</pre>
          </SettingsCard>

          <SettingsCard v-if="systemResources || weather || taskAssistant || vectorDb">
            <div class="text-sm font-bold mb-2">后台概览</div>
            <pre class="text-[11px] whitespace-pre-wrap break-words opacity-80">{{ JSON.stringify({ systemResources, weather, taskAssistant, vectorDb }, null, 2) }}</pre>
          </SettingsCard>

          <SettingsCard v-if="diaryResults.length">
            <div class="text-sm font-bold mb-2">日记召回结果</div>
            <div class="space-y-2 max-h-72 overflow-y-auto pr-1">
              <div v-for="(note, index) in diaryResults" :key="note.path || note.fileName || note.filename || index"
                class="text-xs px-3 py-2 rounded-xl bg-black/5 dark:bg-white/5">
                <div class="font-bold truncate">{{ note.title || note.fileName || note.filename || note.path || `结果 ${index + 1}` }}</div>
                <div class="mt-1 opacity-60 line-clamp-3 whitespace-pre-wrap">{{ note.content || note.snippet || note.text || note.preview || JSON.stringify(note) }}</div>
              </div>
            </div>
          </SettingsCard>

          <SettingsCard v-if="models.length">
            <div class="text-sm font-bold mb-2">模型列表</div>
            <div class="space-y-1 max-h-64 overflow-y-auto pr-1">
              <div v-for="model in models" :key="model.id || model.name" class="text-xs px-3 py-2 rounded-xl bg-black/5 dark:bg-white/5 font-mono truncate">
                {{ model.id || model.name || model }}
              </div>
            </div>
          </SettingsCard>

          <SettingsCard v-if="plugins.length">
            <div class="text-sm font-bold mb-2">插件列表</div>
            <div class="space-y-1 max-h-72 overflow-y-auto pr-1">
              <div v-for="plugin in plugins" :key="plugin.name || plugin.id" class="flex items-center justify-between gap-3 text-xs px-3 py-2 rounded-xl bg-black/5 dark:bg-white/5">
                <span class="font-mono truncate">{{ plugin.name || plugin.id }}</span>
                <span class="shrink-0 opacity-50">{{ plugin.enabled === false || plugin.isEnabled === false ? 'off' : 'on' }}</span>
              </div>
            </div>
          </SettingsCard>

          <SettingsCard v-if="serverLog">
            <div class="text-sm font-bold mb-2">后端日志</div>
            <pre class="text-[10px] whitespace-pre-wrap break-words opacity-80 max-h-96 overflow-y-auto">{{ serverLog }}</pre>
          </SettingsCard>
        </div>
      </div>
    </Transition>
  </Teleport>
</template>

<style scoped>
.toolbox-view {
  background-color: color-mix(in srgb, var(--primary-bg) 92%, transparent);
  backdrop-filter: blur(40px) saturate(180%);
}
.fade-enter-active,
.fade-leave-active {
  transition: opacity 0.25s ease;
}
.fade-enter-from,
.fade-leave-to {
  opacity: 0;
}
</style>
