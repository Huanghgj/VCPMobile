<script setup lang="ts">
import { computed, ref, watch } from "vue";
import type { AppSettings } from "../../core/stores/settings";
import { useDistributed } from "./composables/useDistributed";

import SettingsTextField from "../../components/settings/SettingsTextField.vue";
import SettingsSwitch from "../../components/settings/SettingsSwitch.vue";
import SettingsActionButton from "../../components/settings/SettingsActionButton.vue";
import SettingsInlineStatus from "../../components/settings/SettingsInlineStatus.vue";
import SettingsRow from "../../components/settings/SettingsRow.vue";

const props = defineProps<{
  settings: AppSettings;
}>();

const emit = defineEmits<{
  (e: "save-request"): void;
}>();

const { status, loading, start, stop } = useDistributed();
const starting = ref(false);

// Local toggle state — bound to settings for persistence
const enabled = computed({
  get: () => props.settings.distributedEnabled ?? false,
  set: (val: boolean) => {
    props.settings.distributedEnabled = val;
  },
});

const deviceName = computed({
  get: () => props.settings.distributedDeviceName ?? "VCPMobile",
  set: (val: string) => {
    props.settings.distributedDeviceName = val;
  },
});

const normalizeDistributedBaseUrl = (rawUrl: string) => {
  let url = rawUrl.trim();
  if (!url) return "";

  url = url
    .replace(/^http:\/\//i, "ws://")
    .replace(/^https:\/\//i, "wss://")
    .replace(/\/+$/, "");

  // Users may paste a full realtime/VCPInfo/distributed endpoint. The native
  // client appends the distributed path itself, so keep only the server base.
  url = url
    .replace(/\/(?:VCPlog|vcpinfo)(?:\/VCP_Key=.*)?$/i, "")
    .replace(/\/vcp-distributed-server(?:\/VCP_Key=.*)?$/i, "")
    .replace(/\/VCP_Key=.*$/i, "")
    .replace(/\/+$/, "");

  return url;
};

// Derive WS URL from the shared realtime channel URL (same server, different path)
const derivedWsUrl = computed(() => {
  return normalizeDistributedBaseUrl(props.settings.vcpLogUrl || "");
});

const derivedVcpKey = computed(() => {
  return (props.settings.vcpLogKey || "").trim();
});

const isConfigured = computed(() => Boolean(derivedWsUrl.value && derivedVcpKey.value));
const nodeEnabled = computed(() => enabled.value || status.value.connected || starting.value);
const canToggle = computed(() => isConfigured.value || nodeEnabled.value);

const statusDisplay = computed(() => {
  if (loading.value || starting.value) {
    return { type: "loading" as const, message: "正在更新分布式节点..." };
  }
  if (status.value.connected) {
    const serverId = status.value.server_id || "等待服务器编号";
    return {
      type: "success" as const,
      message: `已连接 · ${serverId} · ${status.value.registered_tools} 个工具`,
    };
  }
  if (enabled.value && status.value.last_error) {
    return {
      type: "error" as const,
      message: `连接失败，后台将继续重试：${status.value.last_error}`,
    };
  }
  if (enabled.value) {
    return { type: "loading" as const, message: "正在连接/重连中..." };
  }
  if (status.value.last_error) {
    return { type: "error" as const, message: status.value.last_error };
  }
  if (!isConfigured.value) {
    return { type: "info" as const, message: "未配置 VCP 实时通道 URL 或 Key" };
  }
  return { type: null, message: "未启用" };
});

const persistEnabled = (val: boolean) => {
  enabled.value = val;
  emit("save-request");
};

const startConnection = async () => {
  if (starting.value || !isConfigured.value) return;
  starting.value = true;
  try {
    await start(derivedWsUrl.value, derivedVcpKey.value, deviceName.value);
  } catch (e) {
    console.error("[Distributed] Start failed:", e);
    enabled.value = false;
    emit("save-request");
  } finally {
    starting.value = false;
  }
};

const toggleConnection = async (nextValue?: boolean) => {
  const shouldEnable = typeof nextValue === "boolean" ? nextValue : !nodeEnabled.value;

  if (!shouldEnable) {
    persistEnabled(false);
    await stop();
    return;
  }

  if (!isConfigured.value) {
    return;
  }

  persistEnabled(true);
  await startConnection();
};

// Auto-connect on mount if enabled was persisted
watch(
  () => props.settings.distributedEnabled,
  async (val) => {
    if (val && !status.value.connected && isConfigured.value) {
      await startConnection();
    }
  },
  { immediate: true },
);
</script>

<template>
  <div class="space-y-5 px-1">
    <!-- 主开关 -->
    <SettingsRow title="分布式节点" :description="statusDisplay.message">
      <template #action>
        <SettingsSwitch
          :model-value="nodeEnabled"
          active-color="bg-purple-500"
          :disabled="loading || starting || !canToggle"
          @update:model-value="toggleConnection"
        />
      </template>
    </SettingsRow>

    <!-- 连接状态 -->
    <SettingsInlineStatus
      v-if="statusDisplay.type"
      :type="statusDisplay.type"
      :message="statusDisplay.message"
    />

    <!-- 节点名称 -->
    <SettingsTextField
      v-model="deviceName"
      label="节点名称"
      placeholder="VCPMobile"
    />

    <!-- 连接信息（只读，派生自实时通道配置） -->
    <div class="text-xs opacity-40 space-y-1 pt-2">
      <div class="font-mono">
        WS: {{ derivedWsUrl || "未配置（请先设置核心连接 → 实时通道 URL）" }}
      </div>
      <div class="font-mono">
        Key: {{ derivedVcpKey ? "●●●●●●●●" : "未配置" }}
      </div>
    </div>

    <!-- 手动重连按钮 -->
    <div class="pt-2 flex justify-end">
      <SettingsActionButton
        :variant="nodeEnabled ? 'danger' : 'secondary'"
        size="sm"
        :loading="loading || starting"
        :disabled="!canToggle"
        @click="toggleConnection"
      >
        {{ nodeEnabled ? "停止节点" : "连接" }}
      </SettingsActionButton>
    </div>
  </div>
</template>
