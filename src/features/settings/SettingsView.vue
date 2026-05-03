<script setup lang="ts">
import { ref, onMounted, watch } from "vue";
import { useRouter } from "vue-router";
import { useSettingsStore, type AppSettings } from "../../core/stores/settings";
import { useOverlayStore } from "../../core/stores/overlay";
import SlidePage from "../../components/ui/SlidePage.vue";

import UserProfileSection from "./components/UserProfileSection.vue";
import SyncSettingsSection from "./components/SyncSettingsSection.vue";
import VcpCoreSettingsSection from "./components/VcpCoreSettingsSection.vue";
import TopicSummarySection from "./components/TopicSummarySection.vue";
import AiLogicSettingsSection from "./components/AiLogicSettingsSection.vue";
import MaintenanceSection from "./components/MaintenanceSection.vue";
import UpdateSection from "./components/UpdateSection.vue";
import ThemePicker from "./ThemePicker.vue";
import ModelSelector from "../../components/ModelSelector.vue";
import DistributedSettingsSection from "../distributed/DistributedSettingsSection.vue";
import ToolInteractionOverlay from "../distributed/ToolInteractionOverlay.vue";
import SensorCollector from "../distributed/SensorCollector.vue";
import SyncLogBrowser from "./components/SyncLogBrowser.vue";

import SettingsSection from "../../components/settings/SettingsSection.vue";
import SettingsCard from "../../components/settings/SettingsCard.vue";
import SettingsActionButton from "../../components/settings/SettingsActionButton.vue";
import SettingsDisclosure from "../../components/settings/SettingsDisclosure.vue";

const props = withDefaults(
  defineProps<{
    isOpen?: boolean;
    zIndex?: number;
  }>(),
  {
    isOpen: false,
    zIndex: 50,
  },
);

const emit = defineEmits<{
  close: [];
}>();

const settingsStore = useSettingsStore();
const overlayStore = useOverlayStore();
const router = useRouter();

const settings = ref<AppSettings>({
  userName: "User",
  vcpServerUrl: "",
  vcpApiKey: "",
  vcpLogUrl: "",
  vcpLogKey: "",
  syncServerUrl: "",
  syncHttpUrl: "",
  syncToken: "",
  adminUsername: "",
  adminPassword: "",
  fileKey: "",
  agentOrder: [],
  groupOrder: [],
  topicSummaryModel: "gemini-2.5-flash",
  syncLogLevel: "INFO",
  enableVcpToolInjection: false,
  agentMusicControl: false,
  enableAgentBubbleTheme: true,
  enableMobileSurfaceInjection: true,
  enableMobileBrowserInjection: true,
  enableModelThinking: true,
  modelThinkingBudget: 4096,
});

const normalizeThinkingBudget = (value: unknown) => {
  const parsed = Number(value);
  if (!Number.isFinite(parsed)) return 4096;
  return Math.min(32768, Math.max(1024, parsed));
};

const normalizeSettings = (rawSettings: AppSettings): AppSettings => ({
  ...rawSettings,
  syncLogLevel: rawSettings.syncLogLevel || "INFO",
  enableVcpToolInjection: rawSettings.enableVcpToolInjection ?? false,
  agentMusicControl: rawSettings.agentMusicControl ?? false,
  enableAgentBubbleTheme: rawSettings.enableAgentBubbleTheme ?? true,
  enableMobileSurfaceInjection: rawSettings.enableMobileSurfaceInjection ?? true,
  enableMobileBrowserInjection: rawSettings.enableMobileBrowserInjection ?? true,
  enableModelThinking: rawSettings.enableModelThinking ?? true,
  modelThinkingBudget: normalizeThinkingBudget(rawSettings.modelThinkingBudget),
});

const loading = ref(true);
const showSummaryModelSelector = ref(false);
const showSyncLogBrowser = ref(false);

const onSummaryModelSelect = (modelId: string) => {
  settings.value.topicSummaryModel = modelId;
};

const closeSettings = () => {
  emit("close");
};

const openDiagnostics = () => {
  overlayStore.closeSettingsSilently();
  router.push("/diagnostics");
};

const openToolboxView = () => {
  overlayStore.closeSettingsSilently();
  overlayStore.openToolbox();
};

const openBrowserView = () => {
  overlayStore.closeSettingsSilently();
  overlayStore.openBrowser();
};

const loadSettings = async () => {
  try {
    await settingsStore.fetchSettings();
    if (settingsStore.settings) {
      settings.value = normalizeSettings(
        JSON.parse(JSON.stringify(settingsStore.settings)),
      );
    }
  } catch (e) {
    console.error("[SettingsView] Failed to load settings:", e);
  } finally {
    loading.value = false;
  }
};

const saveSettings = async () => {
  try {
    await settingsStore.saveSettings(settings.value);
    console.log("Settings saved!");
  } catch (e) {
    console.error("Failed to save settings:", e);
  }
};

onMounted(() => {
  if (props.isOpen) loadSettings();
});

watch(
  () => props.isOpen,
  (val: boolean) => {
    if (val) {
      loadSettings();
    }
  },
);
</script>

<template>
  <SlidePage :is-open="props.isOpen" :z-index="props.zIndex">
    <div
      class="settings-view flex flex-col h-full w-full bg-secondary-bg text-primary-text pointer-events-auto"
    >
      <header
        class="p-4 flex items-center justify-between border-b border-white/10 pt-[calc(var(--vcp-safe-top,24px)+20px)] pb-6 shrink-0"
      >
        <h2 class="text-xl font-bold">全局设置</h2>
        <button
          @click="closeSettings"
          class="p-2.5 bg-white/10 rounded-full active:scale-90 transition-all flex items-center justify-center"
        >
          <svg
            width="22"
            height="22"
            viewBox="0 0 24 24"
            fill="none"
            stroke="currentColor"
            stroke-width="2.5"
            stroke-linecap="round"
            stroke-linejoin="round"
          >
            <line x1="18" y1="6" x2="6" y2="18"></line>
            <line x1="6" y1="6" x2="18" y2="18"></line>
          </svg>
        </button>
      </header>

      <div
        v-if="loading"
        class="flex-1 flex items-center justify-center opacity-60 text-sm font-bold tracking-widest uppercase"
      >
        正在加载设置...
      </div>

      <div v-else class="flex-1 overflow-y-auto p-5 space-y-6 pb-safe">
        <UserProfileSection :settings="settings" />

        <SettingsDisclosure
          title="核心连接"
          description="VCP Server API 与 WebSocket 鉴权"
          :default-open="true"
          accent-color="bg-blue-500"
        >
          <VcpCoreSettingsSection
            :settings="settings"
            @save-request="saveSettings"
          />
        </SettingsDisclosure>

        <SettingsDisclosure
          title="数据同步"
          description="连接桌面端同步插件"
          accent-color="bg-green-500"
        >
          <SyncSettingsSection
            :settings="settings"
            @save-request="saveSettings"
          />
        </SettingsDisclosure>

        <SettingsDisclosure
          title="话题总结"
          description="配置总结专用模型"
          accent-color="bg-yellow-500"
        >
          <TopicSummarySection
            :settings="settings"
            @open-model-selector="showSummaryModelSelector = true"
          />
        </SettingsDisclosure>

        <SettingsDisclosure
          title="AI 逻辑"
          description="工具路由、音乐上下文与思考参数"
          accent-color="bg-purple-500"
        >
          <AiLogicSettingsSection :settings="settings" />
        </SettingsDisclosure>

        <SettingsDisclosure
          title="分布式节点"
          description="作为移动端工具节点接入主服务器"
          accent-color="bg-violet-500"
        >
          <DistributedSettingsSection
            :settings="settings"
            @save-request="saveSettings"
          />
        </SettingsDisclosure>

        <SettingsSection title="视觉长廊" accent-color="bg-orange-500">
          <SettingsCard no-padding>
            <div class="p-4">
              <ThemePicker />
            </div>
          </SettingsCard>
        </SettingsSection>

        <SettingsSection title="VCPToolBox 后端" accent-color="bg-indigo-500">
          <SettingsCard>
            <div class="space-y-3">
              <div>
                <div class="text-sm font-bold">后端管理接口</div>
                <div class="text-xs opacity-60 mt-1 leading-relaxed">
                  查看模型、生命周期、插件、系统状态、日志等 VCPToolBox 接口。
                </div>
              </div>
              <SettingsActionButton variant="secondary" size="sm" full-width @click="openToolboxView">
                打开后端面板
              </SettingsActionButton>
              <SettingsActionButton variant="secondary" size="sm" full-width @click="openBrowserView">
                Mobile Browser Runtime
              </SettingsActionButton>
            </div>
          </SettingsCard>
        </SettingsSection>

        <SettingsSection title="数据维护" accent-color="bg-red-500">
          <SettingsCard>
            <MaintenanceSection />
          </SettingsCard>
        </SettingsSection>

        <SettingsSection title="性能诊断" accent-color="bg-cyan-500">
          <SettingsCard>
            <div class="space-y-3">
              <div>
                <div class="text-sm font-bold">测试辅助页面</div>
                <div class="text-xs opacity-60 mt-1 leading-relaxed">
                  采集 FPS、长任务、内存快照和设备信息，并将报告保存到手机应用目录。
                </div>
              </div>
              <SettingsActionButton variant="secondary" size="sm" full-width @click="openDiagnostics">
                打开性能诊断
              </SettingsActionButton>
            </div>
          </SettingsCard>
        </SettingsSection>

        <SettingsSection title="同步诊断" accent-color="bg-cyan-500">
          <SettingsCard>
            <div class="p-4">
              <button @click="showSyncLogBrowser = true"
                class="flex items-center justify-between w-full text-left">
                <div>
                  <div class="text-sm font-bold">同步日志浏览器</div>
                  <div class="text-xs text-white/40 mt-0.5">查看历史同步会话的完整日志</div>
                </div>
                <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" class="text-white/20">
                  <polyline points="9 18 15 12 9 6"></polyline>
                </svg>
              </button>
            </div>
          </SettingsCard>
        </SettingsSection>

        <SettingsSection title="关于" accent-color="bg-gray-500">
          <SettingsCard>
            <UpdateSection />
          </SettingsCard>
        </SettingsSection>

        <div class="h-4"></div>

        <SettingsActionButton
          variant="primary"
          size="lg"
          full-width
          @click="saveSettings"
        >
          保存并应用变更
        </SettingsActionButton>

        <div
          class="text-center opacity-10 text-[9px] py-8 pb-12 font-mono uppercase tracking-widest"
        >
          VCP MOBILE · PROJECT AVATAR<br />INTERNAL RELEASE 2026.04.07
        </div>
      </div>

      <ToolInteractionOverlay />
      <SensorCollector />
      <SyncLogBrowser :is-open="showSyncLogBrowser" @close="showSyncLogBrowser = false" />

      <ModelSelector
        :model-value="showSummaryModelSelector"
        @update:model-value="showSummaryModelSelector = $event"
        :current-model="settings.topicSummaryModel"
        title="选择总结专用模型"
        @select="onSummaryModelSelect"
      />
    </div>
  </SlidePage>
</template>

<style scoped>
.settings-view {
  background-color: color-mix(in srgb, var(--primary-bg) 92%, transparent);
  backdrop-filter: blur(40px) saturate(180%);
}

@media (hover: none) and (pointer: coarse) {
  .settings-view {
    backdrop-filter: blur(4px) saturate(180%);
  }
}
</style>
