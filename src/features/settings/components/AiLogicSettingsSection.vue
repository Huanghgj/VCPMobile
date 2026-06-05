<script setup lang="ts">
import { computed, onMounted, ref } from "vue";
import SettingsRow from "../../../components/settings/SettingsRow.vue";
import SettingsSwitch from "../../../components/settings/SettingsSwitch.vue";
import { useTarvenStore } from "../../../core/stores/tarvenStore";

const props = defineProps<{
  settings: Record<string, unknown>;
}>();

const tarvenStore = useTarvenStore();
const isTarvenRulesReady = ref(false);

const enableVcpToolInjection = computed({
  get: () => props.settings.enableVcpToolInjection === true,
  set: (value: boolean) => {
    props.settings.enableVcpToolInjection = value;
  },
});

const enableSystemMetadata = computed({
  get: () => tarvenStore.rules.find((rule) => rule.id === "system_meta_injection")?.isEnabled ?? true,
  set: (value: boolean) => {
    tarvenStore.setRuleEnabled("system_meta_injection", value);
  },
});

const enableTimeAnchoring = computed({
  get: () => tarvenStore.rules.find((rule) => rule.id === "time_anchoring_v2")?.isEnabled ?? false,
  set: (value: boolean) => {
    tarvenStore.setRuleEnabled("time_anchoring_v2", value);
  },
});

onMounted(async () => {
  try {
    await tarvenStore.fetchRules();
  } finally {
    isTarvenRulesReady.value = true;
  }
});
</script>

<template>
  <div class="divide-y divide-black/5 dark:divide-white/5 px-1">
    <SettingsRow
      title="VCP 动态工具路由注入"
      description="开启后请求会走 /v1/chatvcp/completions，以适配 VCP 动态工具路由。"
    >
      <template #action>
        <SettingsSwitch v-model="enableVcpToolInjection" />
      </template>
    </SettingsRow>

    <SettingsRow
      title="系统环境元数据注入"
      description="在 System Prompt 顶部注入当前系统时间、运行环境和话题创建时间。"
    >
      <template #action>
        <SettingsSwitch v-model="enableSystemMetadata" :disabled="!isTarvenRulesReady" />
      </template>
    </SettingsRow>

    <SettingsRow
      title="会话内时间锚定 V2"
      description="为 Payload 内每条消息追加分钟级 message_time 标记，最终落库时会清理。"
    >
      <template #action>
        <SettingsSwitch v-model="enableTimeAnchoring" :disabled="!isTarvenRulesReady" />
      </template>
    </SettingsRow>

    <SettingsRow
      title="气泡主题 UI 规范注入"
      description="当前移动端请求链路尚未实现该注入项。"
    >
      <template #action>
        <span class="text-xs opacity-60">未实现</span>
      </template>
    </SettingsRow>
  </div>
</template>
