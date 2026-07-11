<script setup lang="ts">
import { computed, onMounted, onUnmounted, ref, watch } from "vue";
import { Activity, Bot, ChevronDown, Clock3, Gauge, Pause, Play, RotateCcw, Send, Trash2, X } from "lucide-vue-next";
import SlidePage from "../../components/ui/SlidePage.vue";
import SettingsCard from "../../components/settings/SettingsCard.vue";
import SettingsRow from "../../components/settings/SettingsRow.vue";
import SettingsSwitch from "../../components/settings/SettingsSwitch.vue";
import { useAiLifecycleStore } from "../../core/stores/aiLifecycle";
import { useChatHistoryStore } from "../../core/stores/chatHistoryStore";
import { useChatSessionStore } from "../../core/stores/chatSessionStore";
import { useLifecycleSchedulerStore } from "../../core/stores/lifecycleScheduler";
import { invoke } from "@tauri-apps/api/core";
import { isTauriRuntime } from "../../core/utils/runtime";

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

const lifecycle = useAiLifecycleStore();
const historyStore = useChatHistoryStore();
const sessionStore = useChatSessionStore();
const scheduler = useLifecycleSchedulerStore();
const isBusy = ref(false);
const initialKeepalivePreference = localStorage.getItem("vcp-lifecycle-keepalive");
const lifecycleKeepaliveEnabled = ref(
  initialKeepalivePreference === "true"
    || (initialKeepalivePreference === null && lifecycle.config.enabled),
);
const runtimeStatus = ref<{
  exactAlarmAllowed: boolean;
  batteryOptimizationIgnored: boolean;
  lifecycleKeepaliveActive: boolean;
  lifecycleKeepaliveRequested: boolean;
  scheduledWakeupAt?: number | null;
  manufacturer: string;
} | null>(null);
const showAdvanced = ref(false);
const showActivity = ref(false);
const now = ref(Date.now());
let clockTimer: ReturnType<typeof setInterval> | null = null;
const stopClock = () => {
  if (!clockTimer) return;
  clearInterval(clockTimer);
  clockTimer = null;
};
const startClock = () => {
  stopClock();
  now.value = Date.now();
  if (!props.isOpen || document.hidden) return;
  clockTimer = setInterval(() => { now.value = Date.now(); }, 1000);
};
const handleVisibilityChange = () => document.hidden ? stopClock() : startClock();

const latest = computed(() => lifecycle.latestDecision);
const currentTarget = computed(() => {
  const item = sessionStore.currentSelectedItem;
  if (!item || !sessionStore.currentTopicId) return "未选择会话";
  return `${item.name || item.id} / ${sessionStore.currentTopicId}`;
});

const lastUserMessage = computed(() =>
  [...historyStore.currentChatHistory].reverse().find((msg) => msg.role === "user"),
);

const lastUserAge = computed(() => {
  const timestamp = lastUserMessage.value?.timestamp;
  if (!timestamp) return "无用户消息";
  const minutes = Math.max(0, Math.floor((Date.now() - timestamp) / 60_000));
  if (minutes < 60) return `${minutes} 分钟前`;
  return `${Math.floor(minutes / 60)} 小时 ${minutes % 60} 分钟前`;
});

const randomWindow = computed(
  () => `${lifecycle.config.minTriggerMinutes}-${lifecycle.config.maxTriggerMinutes} 分钟`,
);

const countdown = computed(() => {
  if (lifecycle.isPaused && lifecycle.pausedUntil) {
    const remaining = Math.max(0, lifecycle.pausedUntil - now.value);
    return `暂停中 · ${Math.ceil(remaining / 60_000)} 分钟后恢复`;
  }
  if (!lifecycle.nextHeartbeatAt) return "尚未调度";
  const remaining = Math.max(0, lifecycle.nextHeartbeatAt - now.value);
  const minutes = Math.floor(remaining / 60_000);
  const seconds = Math.floor((remaining % 60_000) / 1000);
  return `${minutes}:${String(seconds).padStart(2, "0")}`;
});

const healthLabel = computed(() => {
  if (lifecycle.healthScore >= 90) return "稳定";
  if (lifecycle.healthScore >= 70) return "良好";
  if (lifecycle.healthScore >= 45) return "需关注";
  return "异常";
});

const selectedPreset = computed(() => {
  const config = lifecycle.config;
  if (config.minTriggerMinutes === 100 && config.maxTriggerMinutes === 120 && config.maxSendsPerDay === 4) return "conservative";
  if (config.minTriggerMinutes === 60 && config.maxTriggerMinutes === 80 && config.maxSendsPerDay === 12) return "active";
  if (config.minTriggerMinutes === 60 && config.maxTriggerMinutes === 120 && config.maxSendsPerDay === 8) return "balanced";
  return "custom";
});

const statusLabel = (status?: string) => ({
  suggested: "待发送",
  sending: "发送中",
  sent: "已发送",
  deferred: "已延后",
  failed: "失败",
}[status || ""] || status || "静默");

const jobStatusLabel = (status?: string) => ({
  scheduled: "等待执行",
  running: "执行中",
  completed: "已完成",
  failed: "失败",
  cancelled: "已取消",
}[status || ""] || status || "未知");

const recentLifecycleJobs = computed(() => scheduler.historyJobs.slice(0, 30));

const formatTime = (timestamp?: number | null) => {
  if (!timestamp) return "从未";
  return new Date(timestamp).toLocaleString();
};

const refreshRuntimeStatus = async () => {
  if (!isTauriRuntime()) return;
  runtimeStatus.value = await invoke("plugin:vcp-mobile|get_lifecycle_runtime_status");
};

const requestExactAlarmAccess = async () => {
  await invoke("plugin:vcp-mobile|request_exact_alarm_access");
};

const setLifecycleKeepalive = async (enabled: boolean) => {
  const previous = lifecycleKeepaliveEnabled.value;
  try {
    if (isTauriRuntime()) {
      await invoke("plugin:vcp-mobile|set_lifecycle_keepalive", { enabled });
    }
    lifecycleKeepaliveEnabled.value = enabled;
    localStorage.setItem("vcp-lifecycle-keepalive", String(enabled));
    if (isTauriRuntime()) {
      await refreshRuntimeStatus();
    }
  } catch (error) {
    lifecycleKeepaliveEnabled.value = previous;
    throw error;
  }
};

const runHeartbeat = async () => {
  isBusy.value = true;
  try {
    await lifecycle.runHeartbeat("manual");
  } finally {
    isBusy.value = false;
  }
};

const sendLatest = async () => {
  isBusy.value = true;
  try {
    await lifecycle.sendSuggestedMessage();
  } finally {
    isBusy.value = false;
  }
};

const forceTrigger = async () => {
  isBusy.value = true;
  try {
    await lifecycle.forceTriggerNow();
  } finally {
    isBusy.value = false;
  }
};

const scheduleBackgroundWakeupTest = async () => {
  isBusy.value = true;
  try {
    if (!lifecycleKeepaliveEnabled.value) {
      await setLifecycleKeepalive(true);
    }
    const triggerAt = lifecycle.scheduleDebugHeartbeat(60);
    scheduler.setCompanionWakeupAt(triggerAt);
    await scheduler.syncNativeWakeup();
    await refreshRuntimeStatus();
  } finally {
    isBusy.value = false;
  }
};

const injectRenderProbe = async () => {
  isBusy.value = true;
  try {
    await historyStore.injectDebugAssistantRenderProbe();
  } finally {
    isBusy.value = false;
  }
};

const applyPreset = (preset: "conservative" | "balanced" | "active") => {
  lifecycle.applyPreset(preset);
};

const pauseLifecycle = (minutes: number) => {
  lifecycle.pauseFor(minutes);
};

const resumeLifecycle = () => {
  lifecycle.resume();
};

const onEnabledChange = async (enabled: boolean) => {
  lifecycle.updateConfig({ enabled });
  if (enabled) {
    lifecycle.startTimer();
    if (localStorage.getItem("vcp-lifecycle-keepalive") === null) {
      await setLifecycleKeepalive(true);
    }
  }
};

watch(
  () => [
    lifecycle.config.minTriggerMinutes,
    lifecycle.config.maxTriggerMinutes,
    lifecycle.config.minMinutesBetweenSends,
  ],
  () => {
    if (lifecycle.isRunning) lifecycle.startTimer();
  },
);
watch(() => props.isOpen, (isOpen) => {
  startClock();
  if (isOpen) scheduler.refreshJobHistory().catch(console.error);
}, { immediate: true });

onMounted(() => {
  document.addEventListener("visibilitychange", handleVisibilityChange);
  refreshRuntimeStatus().catch(console.error);
  Promise.all([scheduler.refreshJobs(), scheduler.refreshJobHistory()]).catch(console.error);
});

onUnmounted(() => {
  stopClock();
  document.removeEventListener("visibilitychange", handleVisibilityChange);
});
</script>

<template>
  <SlidePage :is-open="props.isOpen" :z-index="props.zIndex">
    <div class="ai-life-debug flex flex-col h-full w-full bg-[var(--primary-bg)] text-primary-text pointer-events-auto">
      <header class="px-4 py-3 flex items-center justify-between border-b border-white/10 pt-[calc(var(--vcp-safe-top,24px)+12px)] shrink-0">
        <div class="min-w-0">
          <h2 class="text-[17px] font-bold tracking-tight">AI 主动陪伴</h2>
          <p class="text-[10px] opacity-40 mt-0.5">在合适的时间，让对话自然继续</p>
        </div>
        <button
          class="w-10 h-10 flex items-center justify-center rounded-xl bg-black/5 dark:bg-white/5 active:scale-95 transition-all"
          @click="emit('close')"
        >
          <X :size="18" />
        </button>
      </header>

      <main class="flex-1 overflow-y-auto px-3 py-4 space-y-4 pb-[calc(var(--vcp-safe-bottom,24px)+24px)]">
        <section class="life-hero" :class="{ enabled: lifecycle.config.enabled, paused: lifecycle.isPaused }">
          <div class="life-orb"><Gauge :size="20" /></div>
          <div class="life-hero-copy">
            <strong>{{ lifecycle.config.enabled ? lifecycle.isPaused ? '主动陪伴已暂停' : '主动陪伴运行中' : '主动陪伴已关闭' }}</strong>
            <span>{{ lifecycle.config.enabled ? countdown : '开启后，AI 会在合适的时间自然发起简短对话' }}</span>
            <small>{{ healthLabel }} · 今日 {{ lifecycle.sendsToday }}/{{ lifecycle.config.maxSendsPerDay }} 条</small>
          </div>
          <SettingsSwitch :model-value="lifecycle.config.enabled" @update:model-value="onEnabledChange" />
        </section>

        <section class="life-section">
          <h3>偏好</h3>
          <div class="life-group">
            <SettingsRow title="允许主动消息" description="通过门禁后自动发送简短建议">
              <template #action>
                <SettingsSwitch :model-value="lifecycle.config.allowAutoSend" @update:model-value="lifecycle.updateConfig({ allowAutoSend: $event })" />
              </template>
            </SettingsRow>
            <div class="life-divider" />
            <SettingsRow title="自适应调度" description="失败时自动降低频率，减少重复打扰">
              <template #action>
                <SettingsSwitch :model-value="lifecycle.config.adaptiveScheduling" @update:model-value="lifecycle.updateConfig({ adaptiveScheduling: $event })" />
              </template>
            </SettingsRow>
          </div>
        </section>

        <section class="life-section">
          <h3>后台运行</h3>
          <div class="life-group">
            <SettingsRow
              title="省电后台保活"
              description="常驻前台服务保持进程；空闲时不持有 CPU 或 Wi-Fi 锁"
            >
              <template #action>
                <SettingsSwitch :model-value="lifecycleKeepaliveEnabled" @update:model-value="setLifecycleKeepalive" />
              </template>
            </SettingsRow>
            <div class="life-divider" />
            <SettingsRow
              title="精确闹钟"
              :description="runtimeStatus?.exactAlarmAllowed ? '已允许精确息屏唤醒' : '未允许时自动降级为系统省电唤醒'"
              clickable
              @click="requestExactAlarmAccess"
            />
            <div class="life-divider" />
            <SettingsRow
              title="小米后台状态"
              :description="runtimeStatus ? (runtimeStatus.batteryOptimizationIgnored ? '已忽略电池优化' : '仍受电池优化限制，请同时开启系统自启动') : '正在读取系统状态'"
            />
            <div class="life-divider" />
            <SettingsRow
              title="待执行计划"
              :description="scheduler.nextJob ? '下次：' + formatTime(scheduler.nextJob.scheduledAt) : '当前没有已计划任务'"
            />
            <div class="life-divider" />
            <SettingsRow
              title="原生唤醒"
              :description="scheduler.nativeWakeupError || (runtimeStatus?.scheduledWakeupAt ? '下次：' + formatTime(runtimeStatus.scheduledWakeupAt) : '尚未设置系统闹钟')"
            />
          </div>
        </section>

        <section class="life-section">
          <h3 class="life-section-title">主动程度</h3>
          <div class="life-presets">
            <button :class="{ active: selectedPreset === 'conservative' }" @click="applyPreset('conservative')"><strong>安静</strong><span>100-120 分钟 · 每日 4 次</span></button>
            <button :class="{ active: selectedPreset === 'balanced' }" @click="applyPreset('balanced')"><strong>均衡</strong><span>60-120 分钟 · 每日 8 次</span></button>
            <button :class="{ active: selectedPreset === 'active' }" @click="applyPreset('active')"><strong>活跃</strong><span>60-80 分钟 · 每日 12 次</span></button>
          </div>
        </section>

        <div class="life-pause-row">
          <button v-if="!lifecycle.isPaused" class="life-action" @click="pauseLifecycle(60)"><Pause :size="15" />暂停 1 小时</button>
          <button v-if="!lifecycle.isPaused" class="life-action" @click="pauseLifecycle(480)"><Pause :size="15" />暂停 8 小时</button>
          <button v-else class="life-action primary" @click="resumeLifecycle"><RotateCcw :size="15" />立即恢复</button>
        </div>

        <section class="life-section">
          <button class="life-disclosure" @click="showAdvanced = !showAdvanced">
            <span><strong>高级选项</strong><small>免打扰、频率与测试工具</small></span>
            <ChevronDown :size="18" :class="{ open: showAdvanced }" />
          </button>
          <Transition name="life-reveal">
            <div v-if="showAdvanced" class="life-advanced">
        <SettingsCard>
          <div class="grid grid-cols-2 gap-3">
            <label class="life-field">
              <span>最小触发 分钟</span>
              <input v-model.number="lifecycle.config.minTriggerMinutes" type="number" min="60" max="120" />
            </label>
            <label class="life-field">
              <span>最大触发 分钟</span>
              <input v-model.number="lifecycle.config.maxTriggerMinutes" type="number" min="60" max="120" />
            </label>
            <label class="life-field">
              <span>发送冷却 分钟</span>
              <input v-model.number="lifecycle.config.minMinutesBetweenSends" type="number" min="60" max="120" />
            </label>
            <label class="life-field">
              <span>每日主动上限</span>
              <input v-model.number="lifecycle.config.maxSendsPerDay" type="number" min="0" max="24" />
            </label>
            <label class="life-field">
              <span>今日已发送</span>
              <input :value="lifecycle.sendsToday" readonly />
            </label>
          </div>
          <div class="mt-3 pt-3 border-t border-black/5 dark:border-white/10">
            <SettingsRow title="免打扰时段" :description="`${lifecycle.config.quietStartHour}:00 - ${lifecycle.config.quietEndHour}:00`">
              <template #action>
                <SettingsSwitch
                  :model-value="lifecycle.config.quietHoursEnabled"
                  @update:model-value="lifecycle.updateConfig({ quietHoursEnabled: $event })"
                />
              </template>
            </SettingsRow>
            <div class="grid grid-cols-2 gap-3 mt-2">
              <label class="life-field">
                <span>开始小时</span>
                <input v-model.number="lifecycle.config.quietStartHour" type="number" min="0" max="23" />
              </label>
              <label class="life-field">
                <span>结束小时</span>
                <input v-model.number="lifecycle.config.quietEndHour" type="number" min="0" max="23" />
              </label>
            </div>
          </div>
        </SettingsCard>
        <div class="grid grid-cols-3 gap-3">
          <button class="life-action primary" :disabled="isBusy" @click="runHeartbeat">
            <Play :size="15" />
            <span>检查门禁</span>
          </button>
          <button
            class="life-action"
            :disabled="isBusy || !latest?.message"
            @click="sendLatest"
          >
            <Send :size="15" />
            <span>触发主动回复</span>
          </button>
          <button class="life-action danger" :disabled="isBusy" @click="forceTrigger">
            <Send :size="15" />
            <span>强制测试</span>
          </button>
        </div>

        <div class="life-pause-row">
          <button v-if="!lifecycle.isPaused" class="life-action" @click="pauseLifecycle(60)"><Pause :size="15" />暂停 1 小时</button>
          <button v-if="!lifecycle.isPaused" class="life-action" @click="pauseLifecycle(480)"><Pause :size="15" />暂停 8 小时</button>
          <button v-else class="life-action primary" @click="resumeLifecycle"><RotateCcw :size="15" />立即恢复</button>
        </div>

        <button class="life-action probe" :disabled="isBusy" @click="injectRenderProbe">
          <Bot :size="15" />
          <span>注入模拟 AI 回复渲染样例</span>
        </button>
        <button class="life-action probe" :disabled="isBusy" @click="scheduleBackgroundWakeupTest">
          <Clock3 :size="15" />
          <span>1 分钟后台唤醒测试</span>
        </button>
            </div>
          </Transition>
        </section>

        <section class="life-section life-last-section">
          <button class="life-disclosure" @click="showActivity = !showActivity">
            <span><strong>活动与诊断</strong><small>今日统计、会话状态和最近判断</small></span>
            <ChevronDown :size="18" :class="{ open: showActivity }" />
          </button>
          <Transition name="life-reveal">
            <div v-if="showActivity" class="life-activity-content">
        <SettingsCard>
          <div class="flex items-center gap-2 mb-3">
            <Activity :size="15" class="opacity-60" />
            <h3 class="text-[12px] font-black uppercase tracking-[0.12em] opacity-70">24 小时运行统计</h3>
          </div>
          <div class="life-stats">
            <div><strong>{{ lifecycle.decisionStats.total }}</strong><span>检查</span></div>
            <div><strong>{{ lifecycle.decisionStats.sent }}</strong><span>发送</span></div>
            <div><strong>{{ lifecycle.decisionStats.deferred }}</strong><span>延后</span></div>
            <div><strong>{{ lifecycle.decisionStats.failed }}</strong><span>失败</span></div>
          </div>
        </SettingsCard>

        <SettingsCard>
          <div class="flex items-center gap-2 mb-3">
            <Activity :size="15" class="opacity-60" />
            <h3 class="text-[12px] font-black uppercase tracking-[0.12em] opacity-70">Context Snapshot</h3>
          </div>
          <div class="life-kv">
            <span>当前会话</span>
            <strong>{{ currentTarget }}</strong>
          </div>
          <div class="life-kv">
            <span>消息数量</span>
            <strong>{{ historyStore.currentChatHistory.length }}</strong>
          </div>
          <div class="life-kv">
            <span>最后用户消息</span>
            <strong>{{ lastUserAge }}</strong>
          </div>
          <div class="life-kv">
            <span>上次心跳</span>
            <strong>{{ formatTime(lifecycle.lastHeartbeatAt) }}</strong>
          </div>
          <div class="life-kv">
            <span>随机窗口</span>
            <strong>{{ randomWindow }}</strong>
          </div>
          <div class="life-kv">
            <span>下次触发</span>
            <strong>{{ formatTime(lifecycle.nextHeartbeatAt) }}</strong>
          </div>
          <div class="life-kv">
            <span>上次主动发送</span>
            <strong>{{ formatTime(lifecycle.lastSentAt) }}</strong>
          </div>
        </SettingsCard>

        <SettingsCard v-if="latest">
          <div class="flex items-center justify-between gap-3 mb-3">
            <div class="flex items-center gap-2 min-w-0">
              <Clock3 :size="15" class="opacity-60" />
              <h3 class="text-[12px] font-black uppercase tracking-[0.12em] opacity-70 truncate">Latest Decision</h3>
            </div>
            <span class="life-pill" :class="latest.shouldSend ? 'ok' : ''">
              {{ latest.shouldSend ? 'SEND' : 'SILENT' }}
            </span>
          </div>
          <div class="space-y-2 text-xs">
            <p class="opacity-75">{{ latest.reason }}</p>
            <p v-if="latest.forced" class="life-message warn">这次是手动强制测试，已绕过本地门禁。</p>
              <p v-if="latest.target" class="life-message">目标： {{ latest.target.displayName }} / {{ latest.target.topicId }}</p>
              <p v-if="latest.failureReason" class="life-message warn">{{ latest.failureReason }}</p>
            <p v-if="latest.message" class="life-message">{{ latest.message }}</p>
            <div class="flex flex-wrap gap-1.5">
              <span v-for="signal in latest.signals" :key="signal" class="life-signal">{{ signal }}</span>
            </div>
          </div>
        </SettingsCard>

        <SettingsCard>
          <div class="flex items-center gap-2 mb-3">
            <Clock3 :size="15" class="opacity-60" />
            <h3 class="text-[12px] font-black uppercase tracking-[0.12em] opacity-70">调度任务记录</h3>
          </div>
          <div v-if="recentLifecycleJobs.length === 0" class="text-xs opacity-45 py-6 text-center">
            暂无调度任务记录
          </div>
          <div v-else class="space-y-2">
            <article v-for="job in recentLifecycleJobs" :key="job.jobId" class="life-log">
              <div class="flex items-center justify-between gap-2">
                <span class="font-mono text-[10px] opacity-50">
                  {{ formatTime(job.completedAt || job.updatedAt || job.scheduledAt) }}
                </span>
                <span
                  class="life-pill"
                  :class="{
                    ok: job.status === 'completed',
                    active: job.status === 'scheduled' || job.status === 'running',
                    warn: job.status === 'failed' || job.status === 'cancelled',
                  }"
                >
                  {{ jobStatusLabel(job.status) }}
                </span>
              </div>
              <p class="mt-1 text-xs font-semibold leading-relaxed">{{ job.intent }}</p>
              <p class="mt-1 text-[10px] opacity-50">
                {{ job.action }} · {{ job.ownerType }} · 尝试 {{ job.attemptCount }}/{{ job.maxAttempts }}
              </p>
              <p v-if="job.failureReason" class="mt-1 text-[11px] text-rose-400 leading-relaxed">
                {{ job.failureReason }}
              </p>
              <div v-if="job.sourceMessageId || job.responseMessageId" class="life-job-ids">
                <code v-if="job.sourceMessageId">来源 {{ job.sourceMessageId }}</code>
                <code v-if="job.responseMessageId">回复 {{ job.responseMessageId }}</code>
              </div>
            </article>
          </div>
        </SettingsCard>

        <SettingsCard>
          <div class="flex items-center justify-between mb-3">
            <h3 class="text-[12px] font-black uppercase tracking-[0.12em] opacity-70">Decision Log</h3>
            <button class="life-icon-btn" @click="lifecycle.clearLogs">
              <Trash2 :size="14" />
            </button>
          </div>
          <div v-if="lifecycle.decisions.length === 0" class="text-xs opacity-45 py-6 text-center">
            还没有心跳记录
          </div>
          <div v-else class="space-y-2">
            <article v-for="item in lifecycle.decisions" :key="item.id" class="life-log">
              <div class="flex items-center justify-between gap-2">
                <span class="font-mono text-[10px] opacity-50">{{ formatTime(item.timestamp) }}</span>
                <span class="life-pill" :class="item.sent ? 'ok' : ''">{{ statusLabel(item.status) }}</span>
              </div>
              <p class="mt-1 text-xs font-semibold">{{ item.reason }}</p>
              <p v-if="item.message" class="mt-1 text-[11px] opacity-60 leading-relaxed">{{ item.message }}</p>
              <p v-if="item.failureReason" class="mt-1 text-[11px] text-rose-400 leading-relaxed">{{ item.failureReason }}</p>
            </article>
          </div>
        </SettingsCard>
            </div>
          </Transition>
        </section>
      </main>
    </div>
  </SlidePage>
</template>

<style scoped>
.ai-life-debug { background:linear-gradient(180deg,color-mix(in srgb,var(--primary-bg) 97%,#fff 3%),var(--primary-bg)); }
.ai-life-debug > header { min-height:calc(var(--vcp-safe-top,24px) + 62px); padding-top:calc(var(--vcp-safe-top,24px) + 8px); padding-inline:18px; border-color:color-mix(in srgb,currentColor 7%,transparent); background:color-mix(in srgb,var(--primary-bg) 92%,transparent); }
.ai-life-debug > main { padding:18px 16px calc(var(--vcp-safe-bottom,24px) + 28px); gap:0; }
.life-hero { min-height:112px; padding:18px; border-radius:24px; display:grid; grid-template-columns:48px 1fr auto; gap:14px; align-items:center; background:color-mix(in srgb,currentColor 5%,transparent); border:1px solid color-mix(in srgb,currentColor 7%,transparent); box-shadow:0 14px 40px rgba(0,0,0,.06); }
.life-hero.enabled { background:linear-gradient(145deg,rgba(52,120,246,.16),rgba(94,92,230,.08)); border-color:rgba(52,120,246,.2); }
.life-hero.paused { background:linear-gradient(145deg,rgba(255,159,10,.15),rgba(255,159,10,.05)); }
.life-orb { width:48px; height:48px; border-radius:15px; display:grid; place-items:center; color:#fff; background:linear-gradient(145deg,#4d8df7,#665df0); box-shadow:0 9px 24px rgba(52,120,246,.25); }
.life-hero-copy { min-width:0; display:flex; flex-direction:column; gap:4px; }
.life-hero-copy strong { font-size:16px; font-weight:750; letter-spacing:-.02em; }
.life-hero-copy span { font-size:11px; opacity:.5; overflow:hidden; text-overflow:ellipsis; white-space:nowrap; }
.life-hero-copy small { font-size:9px; font-weight:650; opacity:.38; }
.life-section { margin-top:24px; }
.life-section > h3,.life-section-title { margin:0 0 8px 12px; font-size:11px; font-weight:650; opacity:.45; }
.life-group { padding:0 12px; border-radius:18px; background:color-mix(in srgb,currentColor 5%,transparent); border:1px solid color-mix(in srgb,currentColor 6%,transparent); overflow:hidden; }
.life-divider { height:1px; background:color-mix(in srgb,currentColor 7%,transparent); }
.life-presets { padding:3px; display:grid; grid-template-columns:repeat(3,minmax(0,1fr)); gap:3px; border-radius:13px; background:color-mix(in srgb,currentColor 7%,transparent); }
.life-presets button { min-height:48px; padding:7px; border-radius:10px; display:flex; flex-direction:column; align-items:center; justify-content:center; gap:2px; text-align:center; opacity:.5; transition:background-color .18s ease,box-shadow .18s ease,opacity .18s ease; }
.life-presets button.active { opacity:1; background:var(--primary-bg); box-shadow:0 2px 10px rgba(0,0,0,.11); }
.life-presets strong { font-size:12px; font-weight:700; }
.life-presets span { font-size:8px; line-height:1.25; opacity:.58; }
.life-pause-row { margin-top:12px; display:grid; grid-template-columns:repeat(2,minmax(0,1fr)); gap:8px; }
.life-pause-row > button:only-child { grid-column:1/-1; }
.life-action { min-height:42px; padding:0 12px; border-radius:12px; display:inline-flex; align-items:center; justify-content:center; gap:7px; background:color-mix(in srgb,currentColor 6%,transparent); border:1px solid color-mix(in srgb,currentColor 6%,transparent); font-size:12px; font-weight:650; transition:transform .12s ease,opacity .12s ease; }
.life-action.primary { color:#fff; background:#3478f6; border-color:transparent; }
.life-action.danger { color:#ff453a; background:rgba(255,69,58,.09); }
.life-action.probe { width:100%; color:#3478f6; }
.life-action:disabled { opacity:.3; }.life-action:active:not(:disabled) { transform:scale(.98); }
.life-disclosure { width:100%; min-height:58px; padding:12px 14px; border-radius:17px; display:flex; align-items:center; justify-content:space-between; text-align:left; background:color-mix(in srgb,currentColor 4%,transparent); border:1px solid color-mix(in srgb,currentColor 5%,transparent); }
.life-disclosure span { display:flex; flex-direction:column; gap:3px; }.life-disclosure strong { font-size:13px; font-weight:700; }.life-disclosure small { font-size:9px; opacity:.4; }.life-disclosure svg { opacity:.35; transition:transform .18s ease; }.life-disclosure svg.open { transform:rotate(180deg); }
.life-advanced,.life-activity-content { padding-top:10px; display:flex; flex-direction:column; gap:10px; }
.life-field { display:flex; flex-direction:column; gap:5px; min-width:0; }.life-field span { font-size:9px; font-weight:650; opacity:.42; }.life-field input { width:100%; height:38px; padding:0 11px; border-radius:10px; background:color-mix(in srgb,currentColor 6%,transparent); border:0; outline:none; font-size:13px; }
.life-stats { display:grid; grid-template-columns:repeat(4,minmax(0,1fr)); gap:0; }.life-stats div { padding:8px 4px; text-align:center; border-right:1px solid color-mix(in srgb,currentColor 7%,transparent); display:flex; flex-direction:column; }.life-stats div:last-child { border-right:0; }.life-stats strong { font-size:18px; font-weight:750; }.life-stats span { font-size:9px; opacity:.4; }
.life-kv { min-height:36px; display:flex; align-items:center; justify-content:space-between; gap:12px; border-bottom:1px solid color-mix(in srgb,currentColor 6%,transparent); font-size:11px; }.life-kv:last-child { border-bottom:0; }.life-kv span { opacity:.42; }.life-kv strong { max-width:65%; text-align:right; overflow:hidden; text-overflow:ellipsis; white-space:nowrap; font-weight:650; }
.life-pill,.life-signal { padding:4px 8px; border-radius:99px; background:color-mix(in srgb,currentColor 7%,transparent); font-size:9px; font-weight:700; }.life-pill.ok { color:#30a14e; background:rgba(48,161,78,.12); }
.life-pill.active { color:#3478f6; background:rgba(52,120,246,.12); }.life-pill.warn { color:#e58b00; background:rgba(255,159,10,.1); }
.life-message { padding:10px 11px; border-radius:11px; line-height:1.55; background:color-mix(in srgb,currentColor 5%,transparent); }.life-message.warn { color:#e58b00; background:rgba(255,159,10,.1); }
.life-log { padding:11px; border-radius:12px; background:color-mix(in srgb,currentColor 4%,transparent); }.life-icon-btn { width:32px; height:32px; display:grid; place-items:center; border-radius:50%; background:color-mix(in srgb,currentColor 6%,transparent); }
.life-job-ids { margin-top:8px; display:flex; flex-direction:column; gap:3px; }.life-job-ids code { font-size:9px; line-height:1.4; opacity:.45; overflow-wrap:anywhere; }
.life-reveal-enter-active,.life-reveal-leave-active { transition:opacity .16s ease,transform .16s ease; }.life-reveal-enter-from,.life-reveal-leave-to { opacity:0; transform:translateY(-3px); }
.life-last-section { margin-bottom:4px; }
@media (min-width:700px) { .ai-life-debug > main { width:min(620px,100%); margin:0 auto; } }
@media (prefers-reduced-motion:reduce) { .life-presets button,.life-action,.life-disclosure svg,.life-reveal-enter-active,.life-reveal-leave-active { transition-duration:.01ms; } }
</style>
