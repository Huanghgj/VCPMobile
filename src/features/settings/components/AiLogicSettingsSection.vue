<script setup lang="ts">
import { ref } from "vue";
import { invoke } from "@tauri-apps/api/core";
import { MonitorUp } from "lucide-vue-next";
import type { AppSettings } from "../../../core/stores/settings";
import SettingsRow from "../../../components/settings/SettingsRow.vue";
import SettingsSwitch from "../../../components/settings/SettingsSwitch.vue";
import SettingsActionButton from "../../../components/settings/SettingsActionButton.vue";
import { useNotificationStore } from "../../../core/stores/notification";

const props = defineProps<{
  settings: AppSettings;
}>();

const notificationStore = useNotificationStore();
const testingSurface = ref(false);

const updateThinkingBudget = (event: Event) => {
  const target = event.target as HTMLInputElement;
  const parsed = Number(target.value || 4096);
  if (!Number.isFinite(parsed)) {
    props.settings.modelThinkingBudget = 4096;
    return;
  }
  props.settings.modelThinkingBudget = Math.min(32768, Math.max(1024, parsed));
};

const testSurfaceWidget = async () => {
  if (testingSurface.value) return;

  testingSurface.value = true;
  const now = new Date();
  const time = `${String(now.getHours()).padStart(2, "0")}:${String(now.getMinutes()).padStart(2, "0")}`;
  const html = `
    <div style="width:320px;height:220px;padding:16px;border-radius:14px;background:linear-gradient(160deg,#111827,#312e81);color:#fff;font-family:-apple-system,BlinkMacSystemFont,'Segoe UI',sans-serif;box-sizing:border-box;overflow:hidden;">
      <div style="display:flex;align-items:center;justify-content:space-between;margin-bottom:18px;">
        <div>
          <div style="font-size:13px;font-weight:800;letter-spacing:.08em;">SURFACE TEST</div>
          <div style="margin-top:4px;font-size:11px;color:rgba(255,255,255,.62);">直接测试浮层链路</div>
        </div>
        <div style="width:36px;height:36px;border-radius:12px;background:rgba(255,255,255,.14);display:flex;align-items:center;justify-content:center;">OK</div>
      </div>
      <div style="font-size:46px;font-weight:200;line-height:1;font-variant-numeric:tabular-nums;">${time}</div>
      <div style="margin-top:10px;font-size:12px;color:#bfdbfe;">如果你能看到这个卡片，Surface 组件层已经正常。</div>
      <div style="margin-top:18px;padding:10px;border-radius:10px;background:rgba(255,255,255,.1);font-size:12px;color:#fde68a;">Android 系统悬浮窗还需要“显示在其他应用上层”权限。</div>
    </div>
  `.trim();

  try {
    await invoke("clear_surface_widgets");

    await invoke("show_desktop_overlay", {
      title: "Surface Test",
      html,
    });

    notificationStore.addNotification({
      type: "success",
      title: "Surface 测试已发送",
      message: "已只请求 Android 系统级悬浮窗；应用内 Surface 已清空。",
      duration: 2600,
    });
  } catch (error) {
    console.error("[SurfaceTest] Failed:", error);
    notificationStore.addNotification({
      type: "error",
      title: "Surface 测试失败",
      message: error instanceof Error ? error.message : String(error),
      duration: 3200,
    });
  } finally {
    testingSurface.value = false;
  }
};
</script>

<template>
  <div class="divide-y divide-black/5 dark:divide-white/5 px-1">
    <SettingsRow
      title="VCP 动态工具路由注入"
      description="开启后请求会走 /v1/chatvcp/completions，便于桌面端工具链接管"
    >
      <template #action>
        <SettingsSwitch v-model="settings.enableVcpToolInjection" active-color="bg-purple-500" />
      </template>
    </SettingsRow>

    <SettingsRow
      title="音乐状态与点歌台注入"
      description="把当前播放、歌单与 VCPMusicController 提示加入上下文"
    >
      <template #action>
        <SettingsSwitch v-model="settings.agentMusicControl" active-color="bg-purple-500" />
      </template>
    </SettingsRow>

    <SettingsRow
      title="气泡主题 UI 规范注入"
      description="引导模型输出 VarDivRender 风格的富文本气泡"
    >
      <template #action>
        <SettingsSwitch v-model="settings.enableAgentBubbleTheme" active-color="bg-purple-500" />
      </template>
    </SettingsRow>

    <SettingsRow
      title="移动端 Surface 挂件注入"
      description="引导模型使用 DESKTOP_PUSH 创建可拖动的移动端浮层挂件"
    >
      <template #action>
        <SettingsSwitch v-model="settings.enableMobileSurfaceInjection" active-color="bg-purple-500" />
      </template>
    </SettingsRow>

    <SettingsRow
      title="移动端浏览器提示词注入"
      description="引导模型使用 MobileBrowser 访问网页、自动操作，并在验证或登录时移交给用户"
    >
      <template #action>
        <SettingsSwitch v-model="settings.enableMobileBrowserInjection" active-color="bg-purple-500" />
      </template>
    </SettingsRow>

    <SettingsRow
      title="Surface 浮层测试"
      description="跳过 AI 消息解析，直接创建一个测试挂件和 Android 系统悬浮窗"
    >
      <template #action>
        <SettingsActionButton
          variant="secondary"
          size="sm"
          :icon="MonitorUp"
          :loading="testingSurface"
          @click="testSurfaceWidget"
        >
          测试
        </SettingsActionButton>
      </template>
    </SettingsRow>

    <SettingsRow
      title="模型思考模式"
      description="为 Gemini、Claude 请求附加 reasoning/thinking 参数；DeepSeek 默认不主动请求思维链"
    >
      <template #action>
        <SettingsSwitch v-model="settings.enableModelThinking" active-color="bg-purple-500" />
      </template>
    </SettingsRow>

    <SettingsRow
      title="思考预算"
      description="Claude 与 Gemini 使用 token 预算；DeepSeek 不再映射推理强度"
      :disabled="!settings.enableModelThinking"
    >
      <template #action>
        <input
          type="number"
          min="1024"
          max="32768"
          step="1024"
          :value="settings.modelThinkingBudget"
          @input="updateThinkingBudget"
          class="w-24 bg-black/5 dark:bg-white/5 rounded-xl px-3 py-2 text-right text-xs font-mono outline-none border border-black/5 dark:border-white/5 focus:border-purple-500/50"
        />
      </template>
    </SettingsRow>
  </div>
</template>
