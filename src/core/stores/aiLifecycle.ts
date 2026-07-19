import { defineStore } from "pinia";
import { computed, ref } from "vue";
import { invoke } from "@tauri-apps/api/core";
import { useChatHistoryStore } from "./chatHistoryStore";
import { useChatSessionStore } from "./chatSessionStore";
import { useChatStreamStore } from "./chatStreamStore";
import { useNotificationStore } from "./notification";

export type AiLifecyclePriority = "none" | "low" | "normal" | "high";
export type AiLifecyclePreset = "conservative" | "balanced" | "active";

export interface AiLifecycleDecision {
  id: string;
  timestamp: number;
  shouldSend: boolean;
  reason: string;
  priority: AiLifecyclePriority;
  message: string;
  nextCheckMinutes: number;
  signals: string[];
  source: "manual" | "timer";
  sent?: boolean;
  forced?: boolean;
  status?: "suggested" | "sending" | "sent" | "deferred" | "failed";
  target?: AiLifecycleTarget;
  failureReason?: string;
}

export interface AiLifecycleTarget {
  ownerId: string;
  ownerType: "agent" | "group";
  topicId: string;
  displayName: string;
}

export interface AiLifecycleConfig {
  enabled: boolean;
  allowAutoSend: boolean;
  minTriggerMinutes: number;
  maxTriggerMinutes: number;
  /** @deprecated kept only to migrate old persisted configs */
  intervalMinutes: number;
  quietHoursEnabled: boolean;
  quietStartHour: number;
  quietEndHour: number;
  minMinutesBetweenSends: number;
  maxSendsPerDay: number;
  adaptiveScheduling: boolean;
}

interface LifecycleAffectCue {
  enabled: boolean;
  primaryEmotion: string;
  attachment: number;
  security: number;
  resentment: number;
  jealousy: number;
  distanceNeed: number;
}

const DEFAULT_CONFIG: AiLifecycleConfig = {
  enabled: false,
  allowAutoSend: true,
  minTriggerMinutes: 60,
  maxTriggerMinutes: 120,
  intervalMinutes: 90,
  quietHoursEnabled: true,
  quietStartHour: 23,
  quietEndHour: 8,
  minMinutesBetweenSends: 60,
  maxSendsPerDay: 12,
  adaptiveScheduling: true,
};

const MAX_LOGS = 80;
const MIN_RANDOM_TRIGGER_MINUTES = 60;
const MAX_RANDOM_TRIGGER_MINUTES = 120;
const LEGACY_DAILY_SEND_LIMIT = 3;
const BUSY_RETRY_MINUTES = 5;
const FAILURE_RETRY_MINUTES = 15;
const MAX_FAILURE_BACKOFF_MINUTES = 120;
const SEND_LEDGER_RETENTION_DAYS = 8;

const PRESETS: Record<AiLifecyclePreset, Partial<AiLifecycleConfig>> = {
  conservative: {
    minTriggerMinutes: 100,
    maxTriggerMinutes: 120,
    minMinutesBetweenSends: 120,
    maxSendsPerDay: 4,
    quietHoursEnabled: true,
    adaptiveScheduling: true,
  },
  balanced: {
    minTriggerMinutes: 60,
    maxTriggerMinutes: 120,
    minMinutesBetweenSends: 90,
    maxSendsPerDay: 8,
    quietHoursEnabled: true,
    adaptiveScheduling: true,
  },
  active: {
    minTriggerMinutes: 60,
    maxTriggerMinutes: 80,
    minMinutesBetweenSends: 60,
    maxSendsPerDay: 12,
    quietHoursEnabled: true,
    adaptiveScheduling: true,
  },
};

function clampNumber(value: number, min: number, max: number): number {
  if (!Number.isFinite(value)) return min;
  return Math.max(min, Math.min(max, Math.round(value)));
}

function isWithinQuietHours(config: AiLifecycleConfig, date = new Date()): boolean {
  if (!config.quietHoursEnabled) return false;
  const hour = date.getHours();
  const start = clampNumber(config.quietStartHour, 0, 23);
  const end = clampNumber(config.quietEndHour, 0, 23);
  if (start === end) return false;
  if (start < end) return hour >= start && hour < end;
  return hour >= start || hour < end;
}

function minutesSince(timestamp?: number | null): number | null {
  if (!timestamp) return null;
  return Math.max(0, Math.floor((Date.now() - timestamp) / 60_000));
}

function startOfToday(): number {
  const now = new Date();
  return new Date(now.getFullYear(), now.getMonth(), now.getDate()).getTime();
}

function randomIntInclusive(min: number, max: number): number {
  return min + Math.floor(Math.random() * (max - min + 1));
}

function pickRandomTriggerMinutes(config: AiLifecycleConfig): number {
  const min = clampNumber(
    config.minTriggerMinutes,
    MIN_RANDOM_TRIGGER_MINUTES,
    MAX_RANDOM_TRIGGER_MINUTES,
  );
  const max = clampNumber(
    config.maxTriggerMinutes,
    MIN_RANDOM_TRIGGER_MINUTES,
    MAX_RANDOM_TRIGGER_MINUTES,
  );
  return randomIntInclusive(Math.min(min, max), Math.max(min, max));
}

function pickRecentTopicHint(content: string): string {
  const text = content.replace(/\s+/g, " ").trim();
  if (!text) return "此刻想到的小事";
  return text.length > 42 ? `${text.slice(0, 42)}...` : text;
}

function buildLifecycleTriggerPrompt(decision: AiLifecycleDecision): string {
  const targetName = decision.target?.displayName || "当前对象";
  const signals = decision.signals.length
    ? decision.signals.map((signal) => `- ${signal}`).join("\n")
    : "- 无额外信号";

  return [
    "[AI_LIFECYCLE_TRIGGER]",
    "这是一次内部生命周期心跳，不是用户发送的可见消息。",
    "请不要在回复中提到系统提示词、生命周期、心跳、调试页面、内部触发或这段指令。",
    "你正在尝试自然地向用户主动发起一条消息。可以基于当前聊天上下文，也可以在没有新事件时主动分享一个轻量观察、提醒、近况或下一步建议。",
    "直接输出一条简短、温和、具体的中文消息，最多两句话；不要解释为什么会主动出现。",
    "",
    `当前对象：${targetName}`,
    `决策原因：${decision.reason}`,
    `建议意图：${decision.message}`,
    "本地信号：",
    signals,
  ].join("\n");
}

export const useAiLifecycleStore = defineStore(
  "aiLifecycle",
  () => {
    const config = ref<AiLifecycleConfig>({ ...DEFAULT_CONFIG });
    const decisions = ref<AiLifecycleDecision[]>([]);
    const lastHeartbeatAt = ref<number | null>(null);
    const lastSentAt = ref<number | null>(null);
    const nextHeartbeatAt = ref<number | null>(null);
    const timerId = ref<number | null>(null);
    const isSending = ref(false);
    const sendLedger = ref<number[]>([]);
    const pausedUntil = ref<number | null>(null);
    const consecutiveFailures = ref(0);
    const isHeartbeatRunning = ref(false);
    const isTimerSuspended = ref(false);
    const forceNextHeartbeat = ref(false);

    const isRunning = computed(() => timerId.value !== null);
    const latestDecision = computed(() => decisions.value[0] || null);
    const sendsToday = computed(() => {
      const today = startOfToday();
      const ledgerCount = sendLedger.value.filter((timestamp) => timestamp >= today).length;
      if (ledgerCount > 0) return ledgerCount;
      return decisions.value.filter((item) => item.sent && item.timestamp >= today).length;
    });
    const isPaused = computed(() => pausedUntil.value !== null);
    const decisionStats = computed(() => {
      const since = Date.now() - 24 * 60 * 60_000;
      const recent = decisions.value.filter((item) => item.timestamp >= since);
      return {
        total: recent.length,
        sent: recent.filter((item) => item.sent).length,
        deferred: recent.filter((item) => item.status === "deferred").length,
        failed: recent.filter((item) => item.status === "failed").length,
        silent: recent.filter((item) => !item.shouldSend).length,
      };
    });
    const healthScore = computed(() => {
      const stats = decisionStats.value;
      if (stats.total === 0) return 100;
      return Math.max(0, Math.round(100 - stats.failed * 18 - consecutiveFailures.value * 12));
    });

    const normalizeConfig = () => {
      const legacyInterval = clampNumber(
        config.value.intervalMinutes || DEFAULT_CONFIG.intervalMinutes,
        MIN_RANDOM_TRIGGER_MINUTES,
        MAX_RANDOM_TRIGGER_MINUTES,
      );
      const minTrigger = config.value.minTriggerMinutes ?? legacyInterval;
      const maxTrigger = config.value.maxTriggerMinutes ?? DEFAULT_CONFIG.maxTriggerMinutes;
      config.value.minTriggerMinutes = clampNumber(
        minTrigger,
        MIN_RANDOM_TRIGGER_MINUTES,
        MAX_RANDOM_TRIGGER_MINUTES,
      );
      config.value.maxTriggerMinutes = clampNumber(
        maxTrigger,
        MIN_RANDOM_TRIGGER_MINUTES,
        MAX_RANDOM_TRIGGER_MINUTES,
      );
      if (config.value.maxTriggerMinutes < config.value.minTriggerMinutes) {
        config.value.maxTriggerMinutes = config.value.minTriggerMinutes;
      }
      config.value.intervalMinutes = Math.round(
        (config.value.minTriggerMinutes + config.value.maxTriggerMinutes) / 2,
      );

      const cooldown = Number(config.value.minMinutesBetweenSends);
      config.value.minMinutesBetweenSends = clampNumber(
        !Number.isFinite(cooldown) || cooldown > MAX_RANDOM_TRIGGER_MINUTES
          ? DEFAULT_CONFIG.minMinutesBetweenSends
          : cooldown,
        MIN_RANDOM_TRIGGER_MINUTES,
        MAX_RANDOM_TRIGGER_MINUTES,
      );
      config.value.maxSendsPerDay = clampNumber(
        config.value.maxSendsPerDay === LEGACY_DAILY_SEND_LIMIT
          ? DEFAULT_CONFIG.maxSendsPerDay
          : config.value.maxSendsPerDay,
        0,
        24,
      );
      config.value.quietStartHour = clampNumber(config.value.quietStartHour, 0, 23);
      config.value.quietEndHour = clampNumber(config.value.quietEndHour, 0, 23);
      config.value.adaptiveScheduling = config.value.adaptiveScheduling !== false;
      const cutoff = Date.now() - SEND_LEDGER_RETENTION_DAYS * 24 * 60 * 60_000;
      sendLedger.value = sendLedger.value.filter((timestamp) => timestamp >= cutoff);
    };

    const updateConfig = (updates: Partial<AiLifecycleConfig>) => {
      const hadScheduledHeartbeat = timerId.value !== null || nextHeartbeatAt.value !== null;
      const timingChanged = updates.minTriggerMinutes !== undefined
        || updates.maxTriggerMinutes !== undefined
        || updates.intervalMinutes !== undefined;
      config.value = { ...config.value, ...updates };
      normalizeConfig();
      if (!config.value.enabled) {
        stopTimer();
      } else if (hadScheduledHeartbeat && timingChanged) {
        stopTimer();
        startTimer();
      }
    };

    const applyPreset = (preset: AiLifecyclePreset) => {
      updateConfig(PRESETS[preset]);
      if (config.value.enabled) startTimer();
    };

    const pauseFor = (minutes: number) => {
      const pauseMinutes = clampNumber(minutes, 1, 24 * 60);
      pausedUntil.value = Date.now() + pauseMinutes * 60_000;
      stopTimer();
      if (config.value.enabled) scheduleNextTimer(pauseMinutes);
    };

    const resume = () => {
      pausedUntil.value = null;
      stopTimer();
      if (config.value.enabled) startTimer();
    };

    const buildDecision = (
      source: "manual" | "timer",
      options: { force?: boolean } = {},
      affectCue?: LifecycleAffectCue,
    ): AiLifecycleDecision => {
      normalizeConfig();

      const sessionStore = useChatSessionStore();
      const historyStore = useChatHistoryStore();
      const streamStore = useChatStreamStore();
      const now = Date.now();
      const signals: string[] = [];
      const force = options.force === true;
      const recentMessages = historyStore.currentChatHistory
        .filter((msg) => msg.role !== "system")
        .slice(-8);
      const lastUserMessage = [...recentMessages].reverse().find((msg) => msg.role === "user");
      const lastAssistantMessage = [...recentMessages]
        .reverse()
        .find((msg) => msg.role !== "user");
      const lastVisibleMessage = recentMessages[recentMessages.length - 1];
      const minutesAfterUser = minutesSince(lastUserMessage?.timestamp);
      const minutesAfterVisible = minutesSince(lastVisibleMessage?.timestamp);
      const minutesAfterSent = minutesSince(lastSentAt.value);
      const aiSpokeLast =
        !!lastAssistantMessage &&
        (!lastUserMessage || lastAssistantMessage.timestamp > lastUserMessage.timestamp);
      let nextCheckMinutes = pickRandomTriggerMinutes(config.value);
      const selectedItem = sessionStore.currentSelectedItem;
      const target: AiLifecycleTarget | undefined =
        selectedItem && sessionStore.currentTopicId
          ? {
              ownerId: selectedItem.id,
              ownerType: selectedItem.type === "group" ? "group" : "agent",
              topicId: sessionStore.currentTopicId,
              displayName: selectedItem.name || selectedItem.id,
            }
          : undefined;

      if (!config.value.enabled && source === "timer") {
        signals.push("生命周期未启用，定时心跳只记录不行动");
        return {
          id: `life_${now}_${Math.random().toString(36).slice(2, 8)}`,
          timestamp: now,
          shouldSend: false,
          reason: "生命周期未启用",
          priority: "none",
          message: "",
          nextCheckMinutes,
          signals,
          source,
        };
      }

      if (isPaused.value && !force) {
        const remainingMinutes = Math.max(1, Math.ceil(((pausedUntil.value || now) - now) / 60_000));
        signals.push(`生命周期已暂停，剩余约 ${remainingMinutes} 分钟`);
        return {
          id: `life_${now}_${Math.random().toString(36).slice(2, 8)}`,
          timestamp: now,
          shouldSend: false,
          reason: "生命周期处于暂停状态",
          priority: "none",
          message: "",
          nextCheckMinutes: remainingMinutes,
          signals,
          source,
          status: "deferred",
          target,
        };
      }

      if (!target) {
        signals.push("当前没有选中的 Agent/群组或话题");
        return {
          id: `life_${now}_${Math.random().toString(36).slice(2, 8)}`,
          timestamp: now,
          shouldSend: false,
          reason: "没有可发送的当前会话",
          priority: "none",
          message: "",
          nextCheckMinutes: 30,
          signals,
          source,
        };
      }

      if (streamStore.activeStreamingIds.size > 0 && !force) {
        signals.push(`当前会话仍有 ${streamStore.activeStreamingIds.size} 个生成流`);
        return {
          id: `life_${now}_${Math.random().toString(36).slice(2, 8)}`,
          timestamp: now,
          shouldSend: false,
          reason: "当前会话正在生成，生命周期已延后",
          priority: "none",
          message: "",
          nextCheckMinutes: BUSY_RETRY_MINUTES,
          signals,
          source,
          status: "deferred",
          target,
        };
      }

      if (isWithinQuietHours(config.value) && !force) {
        signals.push(`免打扰时段 ${config.value.quietStartHour}:00-${config.value.quietEndHour}:00`);
        return {
          id: `life_${now}_${Math.random().toString(36).slice(2, 8)}`,
          timestamp: now,
          shouldSend: false,
          reason: "当前处于免打扰时段",
          priority: "none",
          message: "",
          nextCheckMinutes,
          signals,
          source,
        };
      }

      if (sendsToday.value >= config.value.maxSendsPerDay && !force) {
        signals.push(`今日已主动发送 ${sendsToday.value}/${config.value.maxSendsPerDay}`);
        return {
          id: `life_${now}_${Math.random().toString(36).slice(2, 8)}`,
          timestamp: now,
          shouldSend: false,
          reason: "今日主动消息已达上限",
          priority: "none",
          message: "",
          nextCheckMinutes: 240,
          signals,
          source,
        };
      }

      if (
        minutesAfterSent !== null &&
        minutesAfterSent < config.value.minMinutesBetweenSends &&
        !force
      ) {
        signals.push(`距离上次主动发送 ${minutesAfterSent} 分钟`);
        return {
          id: `life_${now}_${Math.random().toString(36).slice(2, 8)}`,
          timestamp: now,
          shouldSend: false,
          reason: "主动消息冷却中",
          priority: "none",
          message: "",
          nextCheckMinutes: Math.max(
            5,
            config.value.minMinutesBetweenSends - minutesAfterSent,
          ),
          signals,
          source,
        };
      }

      if (minutesAfterUser !== null && minutesAfterUser < 20 && !force) {
        signals.push(`用户 ${minutesAfterUser} 分钟前刚刚活跃`);
        return {
          id: `life_${now}_${Math.random().toString(36).slice(2, 8)}`,
          timestamp: now,
          shouldSend: false,
          reason: "用户刚刚活跃，不打断",
          priority: "none",
          message: "",
          nextCheckMinutes: 30,
          signals,
          source,
        };
      }

      if (aiSpokeLast && minutesAfterVisible !== null && minutesAfterVisible < MIN_RANDOM_TRIGGER_MINUTES && !force) {
        signals.push(`最近一条可见消息来自 AI，${minutesAfterVisible} 分钟前刚说过`);
        return {
          id: `life_${now}_${Math.random().toString(36).slice(2, 8)}`,
          timestamp: now,
          shouldSend: false,
          reason: "AI 刚刚主动说过，先留出一小时空隙",
          priority: "none",
          message: "",
          nextCheckMinutes: Math.max(
            5,
            MIN_RANDOM_TRIGGER_MINUTES - minutesAfterVisible,
          ),
          signals,
          source,
        };
      }

      const topicHint = pickRecentTopicHint(
        lastUserMessage?.content || lastVisibleMessage?.content || "",
      );
      const idleText =
        minutesAfterUser === null ? "这段时间" : `过去 ${minutesAfterUser} 分钟`;
      signals.push(
        minutesAfterUser === null
          ? "没有新的用户事件，允许主动分享"
          : `用户已静默 ${minutesAfterUser} 分钟`,
      );
      if (aiSpokeLast) {
        signals.push("上一条消息来自 AI，但已超过最小主动间隔，可以继续轻量分享");
      }
      signals.push(`当前对象：${sessionStore.currentSelectedItem.name || sessionStore.currentSelectedItem.id}`);
      signals.push(`话题：${topicHint}`);
      if (force) {
        signals.push("手动调试强制触发，已绕过免打扰/冷却/最近活跃门禁");
      }

      if (affectCue?.enabled) {
        const attachmentDrive = affectCue.attachment * (1 - affectCue.security);
        signals.push(`情感中枢：${affectCue.primaryEmotion}`);
        if (!force && (affectCue.distanceNeed >= 0.45 || affectCue.resentment >= 0.58)) {
          signals.push("角色当前更想保持距离，选择暂不主动联系");
          return {
            id: `life_${now}_${Math.random().toString(36).slice(2, 8)}`,
            timestamp: now,
            shouldSend: false,
            reason: "情感状态倾向冷却与保持距离",
            priority: "none",
            message: "",
            nextCheckMinutes: Math.max(45, Math.round(nextCheckMinutes * 0.85)),
            signals,
            source,
            target,
            status: "deferred",
          };
        }
        if (attachmentDrive >= 0.16) {
          nextCheckMinutes = Math.max(20, Math.round(nextCheckMinutes * (0.82 - attachmentDrive * 0.45)));
          signals.push("依恋与不安全感提高了主动联系倾向");
        }
        if (affectCue.jealousy >= 0.18) {
          signals.push("未消退的嫉妒会影响主动消息的关注点与语气");
        }
      }

      let reason = "用户静默足够久，且没有命中免打扰/冷却/上限";
      let message =
        `${idleText}里我没有继续打扰你。可以围绕「${topicHint}」轻轻补充一个观察，或帮用户把下一步推进一下。`;

      if (!lastUserMessage) {
        reason = "没有用户事件，随机生命周期主动分享";
        message = "没有新的用户事件。可以主动分享一个轻量观察、提醒、近况或下一步建议，让对话自然重新亮起来。";
      } else if (aiSpokeLast) {
        reason = "距离上次 AI 发言已超过一小时，允许主动补充分享";
        message = `用户暂时没有接话。可以围绕「${topicHint}」补充一个简短观察，或自然分享你想到的下一步。`;
      }


      if (affectCue?.enabled) {
        const attachmentDrive = affectCue.attachment * (1 - affectCue.security);
        if (attachmentDrive >= 0.16) {
          reason = "依恋与不安全感推动角色主动联系";
          message += " 这次主动联系要体现角色自己的需要、在意或不满，不要写成通用提醒。";
        } else if (affectCue.jealousy >= 0.18) {
          reason = "未消退的嫉妒推动角色确认关系";
          message += " 可以含蓄确认用户的注意力去向，但不要报告情感数值。";
        }
      }

      return {
        id: `life_${now}_${Math.random().toString(36).slice(2, 8)}`,
        timestamp: now,
        shouldSend: true,
        reason,
        priority:
          minutesAfterUser !== null && minutesAfterUser > 240 ? "normal" : "low",
        message,
        nextCheckMinutes,
        signals,
        source,
        forced: force,
        status: "suggested",
        target,
      };
    };

    const loadLifecycleAffectCue = async (): Promise<LifecycleAffectCue | undefined> => {
      const sessionStore = useChatSessionStore();
      const selected = sessionStore.currentSelectedItem;
      if (!selected || selected.type === "group") return undefined;
      try {
        const raw = await invoke<any>("get_affect_state", { agentId: selected.id });
        const relationship = raw?.relationship || {};
        return {
          enabled: raw?.config?.enabled !== false,
          primaryEmotion: String(raw?.primaryEmotion || "平静"),
          attachment: clampNumber(Number(relationship.attachment) || 0, 0, 1),
          security: clampNumber(Number(relationship.security) || 0, 0, 1),
          resentment: clampNumber(Number(relationship.resentment) || 0, 0, 1),
          jealousy: clampNumber(Number(relationship.jealousy) || 0, 0, 1),
          distanceNeed: clampNumber(Number(relationship.distanceNeed) || 0, 0, 1),
        };
      } catch (error) {
        console.warn("[AiLifecycle] Affect cue unavailable; using schedule fallback:", error);
        return undefined;
      }
    };

    const pushDecision = (decision: AiLifecycleDecision) => {
      decisions.value.unshift(decision);
      if (decisions.value.length > MAX_LOGS) decisions.value.length = MAX_LOGS;
    };

    const runHeartbeat = async (
      source: "manual" | "timer" = "manual",
      options: { force?: boolean } = {},
    ) => {
      lastHeartbeatAt.value = Date.now();
      const decision = buildDecision(source, options, await loadLifecycleAffectCue());
      pushDecision(decision);
      if (decision.shouldSend && config.value.allowAutoSend && source === "timer") {
        await sendSuggestedMessage(decision.id);
      }
      if (source === "timer" && config.value.enabled) {
        decision.nextCheckMinutes = scheduleNextTimer(decision.nextCheckMinutes);
      }
      return decision;
    };

    const forceTriggerNow = async () => {
      lastHeartbeatAt.value = Date.now();
      const decision = buildDecision("manual", { force: true }, await loadLifecycleAffectCue());
      pushDecision(decision);
      await sendSuggestedMessage(decision.id);
      return decision;
    };

    const sendSuggestedMessage = async (decisionId?: string) => {
      const decision =
        decisions.value.find((item) => item.id === decisionId) || latestDecision.value;
      if (!decision || !decision.message.trim() || !decision.target || isSending.value) return false;

      const sessionStore = useChatSessionStore();
      const streamStore = useChatStreamStore();
      if (
        sessionStore.currentSelectedItem?.id !== decision.target.ownerId ||
        sessionStore.currentTopicId !== decision.target.topicId ||
        streamStore.activeStreamingIds.size > 0
      ) {
        decision.status = "deferred";
        decision.failureReason = "目标会话已切换或仍在生成";
        decision.nextCheckMinutes = BUSY_RETRY_MINUTES;
        if (config.value.enabled) scheduleNextTimer(BUSY_RETRY_MINUTES);
        return false;
      }

      const historyStore = useChatHistoryStore();
      isSending.value = true;
      decision.status = "sending";
      let didStart = false;
      try {
        didStart = await historyStore.triggerHiddenLifecycleMessage(
          buildLifecycleTriggerPrompt(decision),
          decision.target,
        );
      } catch (error) {
        decision.failureReason = error instanceof Error ? error.message : String(error);
      } finally {
        isSending.value = false;
      }
      if (!didStart) {
        decision.status = "failed";
        consecutiveFailures.value += 1;
        decision.failureReason ||= "当前会话不可用、正在生成，或生成请求未能启动";
        decision.nextCheckMinutes = FAILURE_RETRY_MINUTES;
        const notificationStore = useNotificationStore();
        notificationStore.addNotification({
          type: "error",
          title: "生命周期触发失败",
          message: "当前会话不可用，或生成请求未能启动",
          toastOnly: true,
        });
        return false;
      }

      decision.sent = true;
      decision.status = "sent";
      decision.failureReason = undefined;
      consecutiveFailures.value = 0;
      sendLedger.value.push(Date.now());
      lastSentAt.value = Date.now();

      const notificationStore = useNotificationStore();
      notificationStore.addNotification({
        type: "success",
        title: "已触发 AI 主动回复",
        message: decision.reason,
        toastOnly: true,
      });
      return true;
    };

    function clearTimerHandle() {
      if (timerId.value !== null) {
        window.clearTimeout(timerId.value);
        timerId.value = null;
      }
    }

    function suspendTimer() {
      isTimerSuspended.value = true;
      clearTimerHandle();
    }

    function stopTimer() {
      isTimerSuspended.value = false;
      clearTimerHandle();
      nextHeartbeatAt.value = null;
    }

    function armTimerAt(triggerAt: number) {
      clearTimerHandle();
      nextHeartbeatAt.value = triggerAt;
      if (isTimerSuspended.value) return;
      timerId.value = window.setTimeout(() => {
        timerId.value = null;
        runDueHeartbeat().catch((err) => {
          console.error("[AiLifecycle] timer heartbeat failed:", err);
          if (config.value.enabled && nextHeartbeatAt.value === null) {
            scheduleNextTimer();
          }
        });
      }, Math.max(0, triggerAt - Date.now()));
    }

    const runDueHeartbeat = async () => {
      normalizeConfig();
      if (!config.value.enabled || isHeartbeatRunning.value) return false;
      const triggerAt = nextHeartbeatAt.value;
      if (triggerAt === null || triggerAt > Date.now() + 1_000) return false;

      isHeartbeatRunning.value = true;
      clearTimerHandle();
      nextHeartbeatAt.value = null;
      const force = forceNextHeartbeat.value;
      forceNextHeartbeat.value = false;
      try {
        await runHeartbeat("timer", { force });
        return true;
      } finally {
        isHeartbeatRunning.value = false;
        if (config.value.enabled && nextHeartbeatAt.value === null) {
          scheduleNextTimer();
        }
      }
    };

    function scheduleNextTimer(delayOverrideMinutes?: number): number {
      clearTimerHandle();
      normalizeConfig();
      if (!config.value.enabled) return 0;
      const delayMinutes = delayOverrideMinutes
        ? clampNumber(delayOverrideMinutes, 1, MAX_RANDOM_TRIGGER_MINUTES)
        : pickRandomTriggerMinutes(config.value);
      const failureBackoff = config.value.adaptiveScheduling && consecutiveFailures.value > 0
        ? Math.min(
            MAX_FAILURE_BACKOFF_MINUTES,
            FAILURE_RETRY_MINUTES * 2 ** Math.min(consecutiveFailures.value - 1, 3),
          )
        : 0;
      const effectiveDelayMinutes = Math.max(delayMinutes, failureBackoff);
      armTimerAt(Date.now() + effectiveDelayMinutes * 60_000);
      return effectiveDelayMinutes;
    }

    function startTimer() {
      isTimerSuspended.value = false;
      clearTimerHandle();
      normalizeConfig();
      if (!config.value.enabled) {
        nextHeartbeatAt.value = null;
        return;
      }
      if (isPaused.value) {
        if (pausedUntil.value && Date.now() >= pausedUntil.value) {
          pausedUntil.value = null;
        } else {
          scheduleNextTimer(Math.max(1, Math.ceil(((pausedUntil.value || Date.now()) - Date.now()) / 60_000)));
          return;
        }
      }
      if (nextHeartbeatAt.value !== null) {
        if (nextHeartbeatAt.value <= Date.now() + 1_000) {
          runDueHeartbeat().catch((err) => {
            console.error("[AiLifecycle] overdue heartbeat failed:", err);
          });
        } else {
          armTimerAt(nextHeartbeatAt.value);
        }
        return;
      }
      scheduleNextTimer();
    }

    const scheduleDebugHeartbeat = (delaySeconds = 60) => {
      if (!config.value.enabled) {
        config.value.enabled = true;
      }
      forceNextHeartbeat.value = true;
      clearTimerHandle();
      const triggerAt = Date.now() + Math.max(5, Math.min(300, delaySeconds)) * 1_000;
      armTimerAt(triggerAt);
      return triggerAt;
    };

    const clearLogs = () => {
      decisions.value = [];
    };

    return {
      config,
      decisions,
      lastHeartbeatAt,
      lastSentAt,
      nextHeartbeatAt,
      latestDecision,
      sendsToday,
      isRunning,
      isPaused,
      pausedUntil,
      consecutiveFailures,
      decisionStats,
      healthScore,
      sendLedger,
      isSending,
      isHeartbeatRunning,
      updateConfig,
      applyPreset,
      pauseFor,
      resume,
      runHeartbeat,
      forceTriggerNow,
      sendSuggestedMessage,
      startTimer,
      suspendTimer,
      stopTimer,
      runDueHeartbeat,
      scheduleDebugHeartbeat,
      clearLogs,
    };
  },
  {
    persist: {
      pick: ["config", "decisions", "lastHeartbeatAt", "lastSentAt", "nextHeartbeatAt", "sendLedger", "pausedUntil", "consecutiveFailures", "forceNextHeartbeat"],
    },
  },
);
