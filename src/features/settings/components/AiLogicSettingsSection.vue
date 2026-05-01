<script setup lang="ts">
import type { AppSettings } from "../../../core/stores/settings";
import SettingsRow from "../../../components/settings/SettingsRow.vue";
import SettingsSwitch from "../../../components/settings/SettingsSwitch.vue";

const props = defineProps<{
  settings: AppSettings;
}>();

const updateThinkingBudget = (event: Event) => {
  const target = event.target as HTMLInputElement;
  const parsed = Number(target.value || 4096);
  if (!Number.isFinite(parsed)) {
    props.settings.modelThinkingBudget = 4096;
    return;
  }
  props.settings.modelThinkingBudget = Math.min(32768, Math.max(1024, parsed));
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
      title="模型思考模式"
      description="为 DeepSeek、Gemini、Claude 请求附加 reasoning/thinking 参数"
    >
      <template #action>
        <SettingsSwitch v-model="settings.enableModelThinking" active-color="bg-purple-500" />
      </template>
    </SettingsRow>

    <SettingsRow
      title="思考预算"
      description="Claude 与 Gemini 使用 token 预算，DeepSeek 映射为推理强度"
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
