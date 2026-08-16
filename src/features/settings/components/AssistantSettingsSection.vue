<script setup lang="ts">
import { ref, onMounted, watch, onUnmounted } from "vue";
import { invoke } from "@tauri-apps/api/core";
import { useSettingsStore, type AppSettings } from "../../../core/stores/settings";
import { useAssistantStore } from "../../../core/stores/assistant";
import { useNotificationStore } from "../../../core/stores/notification";
import SettingsSwitch from "../../../components/settings/SettingsSwitch.vue";
import SettingsRow from "../../../components/settings/SettingsRow.vue";

const props = defineProps<{
  settings: AppSettings;
}>();

const emit = defineEmits<{
  (e: "save-request"): void;
}>();

const settingsStore = useSettingsStore();
const assistantStore = useAssistantStore();
const notificationStore = useNotificationStore();
const hasOverlayPermission = ref(false);

const applyAssistantRuntimeState = async (enabled: boolean) => {
  await invoke("plugin:vcp-mobile|toggle_floating_ball", { show: enabled });
  await invoke("reconcile_local_server_cmd", { enable: enabled });
};

const reportRuntimeError = (title: string, error: unknown) => {
  const message = error instanceof Error ? error.message : String(error);
  notificationStore.addNotification({
    type: "error",
    title,
    message,
    toastOnly: true,
  });
};

const checkPermission = async () => {
  try {
    const status = await invoke<{ overlay: boolean }>("plugin:vcp-mobile|check_all_permissions");
    hasOverlayPermission.value = status.overlay;

    // 如果没有系统权限，但设置里是开启的，则重置设置状态并隐藏悬浮球
    if (!status.overlay && props.settings.enableAssistant) {
      props.settings.enableAssistant = false;
      const cleanupResults = await Promise.allSettled([
        invoke("plugin:vcp-mobile|toggle_floating_ball", { show: false }),
        invoke("reconcile_local_server_cmd", { enable: false }),
      ]);
      cleanupResults.forEach((result) => {
        if (result.status === "rejected") {
          console.warn("[AssistantSettings] Failed to stop revoked assistant runtime:", result.reason);
        }
      });
      emit("save-request");
    }
  } catch (e) {
    hasOverlayPermission.value = false;
    console.error("[AssistantSettings] Failed to check overlay permission:", e);
  }
};

const handleToggle = async (val: boolean) => {
  if (val) {
    // 猫娘先夹紧悬浮窗权限，没过系统门禁不准硬顶进前台喵♡
    await checkPermission();
    if (!hasOverlayPermission.value) {
      // 引导用户去系统设置开启权限
      try {
        await invoke("plugin:vcp-mobile|request_overlay_permission");
      } catch (e) {
        console.error("[AssistantSettings] Failed to request overlay permission:", e);
      }
      props.settings.enableAssistant = false;
      return;
    }
    // 开启时懒加载 Agent 列表
    try {
      await assistantStore.fetchAgents();
    } catch (error) {
      console.warn("[AssistantSettings] Failed to refresh agents before enabling:", error);
    }
  }

  const previousValue = props.settings.enableAssistant;
  try {
    await applyAssistantRuntimeState(val);
    props.settings.enableAssistant = val;
    emit("save-request");
  } catch (e) {
    console.error("[AssistantSettings] Failed to toggle assistant:", e);
    props.settings.enableAssistant = previousValue;
    await Promise.allSettled([
      invoke("plugin:vcp-mobile|toggle_floating_ball", { show: previousValue }),
      invoke("reconcile_local_server_cmd", { enable: previousValue }),
    ]);
    reportRuntimeError("悬浮助手切换失败", e);
  }
};

watch(
  () => props.settings.assistantAgentId,
  () => {
    emit("save-request");
  }
);

const handleLifecycleEvent = async (e: any) => {
  if (e.detail?.state === "resume") {
    await checkPermission();
    if (props.settings.enableAssistant && hasOverlayPermission.value) {
      try {
        await assistantStore.fetchAgents();
      } catch (error) {
        console.warn("[AssistantSettings] Failed to refresh agents on resume:", error);
      }
      try {
        await applyAssistantRuntimeState(true);
      } catch (error) {
        console.error("[AssistantSettings] Failed to restore assistant runtime:", error);
        reportRuntimeError("悬浮助手恢复失败", error);
      }
    }
  }
};

onMounted(async () => {
  await checkPermission();

  // 若用户手动设置了开启且有权限，则在 mounted 时确保拉起悬浮球并懒加载 Agent 列表
  if (props.settings.enableAssistant && hasOverlayPermission.value) {
    try {
      await assistantStore.fetchAgents();
    } catch (error) {
      console.warn("[AssistantSettings] Failed to refresh agents on mount:", error);
    }
    try {
      await applyAssistantRuntimeState(true);
    } catch (error) {
      console.error("[AssistantSettings] Failed to restore assistant runtime on mount:", error);
      reportRuntimeError("悬浮助手启动失败", error);
    }
  }

  // 监听生命周期 resume 事件以刷新权限状态
  window.addEventListener("vcp-lifecycle", handleLifecycleEvent);
});

onUnmounted(() => {
  // 组件解卸时必须销毁全局监听器，以防内存泄露和重复挂载
  window.removeEventListener("vcp-lifecycle", handleLifecycleEvent);
});
</script>

<template>
  <div class="divide-y divide-black/5 dark:divide-white/5">
    <SettingsRow
      title="启用全局悬浮球"
      description="在其他应用上方显示悬浮球，随时唤起划词助手"
    >
      <template #title-suffix>
        <span class="ml-2 px-1.5 py-0.5 text-[9px] font-bold uppercase tracking-wider rounded-full bg-amber-500/15 text-amber-600 dark:text-amber-400 border border-amber-500/25 select-none">Beta</span>
      </template>
      <template #action>
        <SettingsSwitch
          :modelValue="props.settings.enableAssistant || false"
          :disabled="settingsStore.loading"
          @update:modelValue="handleToggle"
        />
      </template>
    </SettingsRow>

    <SettingsRow
      v-if="props.settings.enableAssistant"
      title="助手绑定 Agent"
      description="选择悬浮窗口默认使用的智能体"
    >
      <template #action>
        <select
          v-model="props.settings.assistantAgentId"
          class="bg-transparent dark:bg-zinc-900 text-sm font-semibold opacity-60 border-none outline-none text-right cursor-pointer text-primary-text pr-2"
        >
          <option value="">未绑定 (使用默认)</option>
          <option
            v-for="agent in assistantStore.agents"
            :key="agent.id"
            :value="agent.id"
          >
            {{ agent.name }}
          </option>
        </select>
      </template>
    </SettingsRow>
  </div>
</template>
