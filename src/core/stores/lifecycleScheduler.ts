import { invoke } from "@tauri-apps/api/core";
import { defineStore } from "pinia";
import { computed, ref } from "vue";
import { useChatHistoryStore } from "./chatHistoryStore";
import { isTauriRuntime } from "../utils/runtime";

export interface LifecycleJob {
  jobId: string;
  ownerId: string;
  ownerType: "agent" | "group";
  topicId: string;
  responderAgentId?: string;
  action: "schedule_message" | "continue_message";
  intent: string;
  condition?: string;
  status: "scheduled" | "running" | "completed" | "failed" | "cancelled";
  scheduledAt: number;
  createdAt: number;
  updatedAt: number;
  attemptCount: number;
  maxAttempts: number;
  sourceMessageId?: string;
  failureReason?: string;
}

const MAX_TIMER_DELAY_MS = 60_000;

export const useLifecycleSchedulerStore = defineStore("lifecycleScheduler", () => {
  const jobs = ref<LifecycleJob[]>([]);
  const isRunning = ref(false);
  const isExecuting = ref(false);
  const lastCheckedAt = ref<number | null>(null);
  const timerId = ref<number | null>(null);

  const nextJob = computed(() =>
    jobs.value
      .filter((job) => job.status === "scheduled")
      .sort((a, b) => a.scheduledAt - b.scheduledAt)[0] || null,
  );

  const stopTimer = () => {
    if (timerId.value !== null) {
      window.clearTimeout(timerId.value);
      timerId.value = null;
    }
  };

  const refreshJobs = async () => {
    jobs.value = await invoke<LifecycleJob[]>("list_lifecycle_jobs", {
      includeFinished: false,
    });
    return jobs.value;
  };

  const scheduleNextCheck = () => {
    stopTimer();
    if (!isRunning.value) return;
    const delay = nextJob.value
      ? Math.max(1_000, Math.min(MAX_TIMER_DELAY_MS, nextJob.value.scheduledAt - Date.now()))
      : MAX_TIMER_DELAY_MS;
    timerId.value = window.setTimeout(() => {
      timerId.value = null;
      runDueJobs().catch((error) => {
        console.error("[LifecycleScheduler] Due job execution failed:", error);
        scheduleNextCheck();
      });
    }, delay);
  };

  const syncNativeWakeup = async () => {
    if (!isTauriRuntime()) return;
    try {
      if (nextJob.value) {
        await invoke("plugin:vcp-mobile|schedule_lifecycle_wakeup", {
          triggerAtMs: nextJob.value.scheduledAt,
        });
      } else {
        await invoke("plugin:vcp-mobile|cancel_lifecycle_wakeup");
      }
    } catch (error) {
      console.warn("[LifecycleScheduler] Native wakeup sync failed:", error);
    }
  };

  const runDueJobs = async () => {
    if (isExecuting.value) return;
    isExecuting.value = true;
    lastCheckedAt.value = Date.now();
    try {
      const dueJobs = await invoke<LifecycleJob[]>("claim_due_lifecycle_jobs", { limit: 4 });
      const historyStore = useChatHistoryStore();
      for (const job of dueJobs) {
        if (job.ownerType === "group") {
          await invoke("fail_lifecycle_job", {
            jobId: job.jobId,
            reason: "群聊生命周期自动执行尚未开放",
            retryDelaySeconds: 3600,
          });
          continue;
        }
        try {
          const completed = await historyStore.triggerScheduledLifecycleMessage(job);
          if (!completed) {
            throw new Error("目标会话正在生成或请求未启动");
          }
          await invoke("complete_lifecycle_job", { jobId: job.jobId });
        } catch (error) {
          await invoke("fail_lifecycle_job", {
            jobId: job.jobId,
            reason: error instanceof Error ? error.message : String(error),
            retryDelaySeconds: Math.min(3600, 60 * 2 ** Math.min(job.attemptCount, 5)),
          });
        }
      }
      await refreshJobs();
    } finally {
      isExecuting.value = false;
      await syncNativeWakeup();
      scheduleNextCheck();
    }
  };

  const start = async () => {
    if (isRunning.value) return;
    isRunning.value = true;
    await refreshJobs();
    await runDueJobs();
  };

  const stop = () => {
    isRunning.value = false;
    stopTimer();
  };

  const wake = async () => {
    if (!isRunning.value) isRunning.value = true;
    await runDueJobs();
  };

  const cancelJob = async (jobId: string) => {
    await invoke("cancel_lifecycle_job", { jobId });
    await refreshJobs();
    await syncNativeWakeup();
    scheduleNextCheck();
  };

  return {
    jobs,
    nextJob,
    isRunning,
    isExecuting,
    lastCheckedAt,
    refreshJobs,
    syncNativeWakeup,
    runDueJobs,
    start,
    stop,
    wake,
    cancelJob,
  };
});
