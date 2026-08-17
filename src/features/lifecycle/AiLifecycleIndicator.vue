<script setup lang="ts">
import { computed, onMounted, onUnmounted, ref, watch } from "vue";
import { Activity, Pause, Sparkles } from "lucide-vue-next";
import { useAiLifecycleStore } from "../../core/stores/aiLifecycle";
import { useOverlayStore } from "../../core/stores/overlay";

const lifecycle = useAiLifecycleStore();
const overlay = useOverlayStore();
const now = ref(Date.now());
let timer: ReturnType<typeof setInterval> | null = null;
const stopTimer = () => {
  if (!timer) return;
  clearInterval(timer);
  timer = null;
};
const startTimer = () => {
  stopTimer();
  now.value = Date.now();
  if (!isVisible.value || document.hidden) return;
  timer = setInterval(() => { now.value = Date.now(); }, 60_000);
};
const handleVisibilityChange = () => document.hidden ? stopTimer() : startTimer();

const isVisible = computed(() => lifecycle.config.enabled || lifecycle.isPaused || lifecycle.isSending);
const label = computed(() => {
  if (lifecycle.isSending) return "AI 正在主动思考";
  if (lifecycle.isPaused && lifecycle.pausedUntil) {
    const minutes = Math.max(1, Math.ceil((lifecycle.pausedUntil - now.value) / 60_000));
    return `生命周期暂停 · ${minutes} 分钟`;
  }
  if (!lifecycle.nextHeartbeatAt) return "生命周期待调度";
  const minutes = Math.max(0, Math.ceil((lifecycle.nextHeartbeatAt - now.value) / 60_000));
  return `下次检查约 ${minutes} 分钟`;
});

watch(isVisible, () => startTimer(), { immediate: true });

onMounted(() => {
  document.addEventListener("visibilitychange", handleVisibilityChange);
});

onUnmounted(() => {
  stopTimer();
  document.removeEventListener("visibilitychange", handleVisibilityChange);
});
</script>

<template>
  <Transition name="life-indicator">
    <button
      v-if="isVisible"
      class="life-indicator"
      :class="{ sending: lifecycle.isSending, paused: lifecycle.isPaused }"
      :title="label"
      @click="overlay.openAiLifecycleDebug()"
    >
      <Sparkles v-if="lifecycle.isSending" :size="14" class="life-spin" />
      <Pause v-else-if="lifecycle.isPaused" :size="13" />
      <Activity v-else :size="13" />
      <span>{{ label }}</span>
      <i :style="{ '--health': `${lifecycle.healthScore}%` }"></i>
    </button>
  </Transition>
</template>

<style scoped>
.life-indicator { position:fixed; left:50%; bottom:calc(var(--vcp-safe-bottom,20px) + 12px); transform:translateX(-50%); z-index:42; max-width:min(86vw,300px); min-height:34px; padding:7px 11px; border-radius:999px; display:flex; align-items:center; gap:7px; color:rgba(255,255,255,.9); background:rgba(15,23,42,.95); border:1px solid rgba(255,255,255,.15); box-shadow:0 6px 20px rgba(0,0,0,.3); font-size:10px; font-weight:700; }
.life-indicator span { overflow:hidden; text-overflow:ellipsis; white-space:nowrap; }
.life-indicator i { width:26px; height:3px; border-radius:99px; background:linear-gradient(90deg,#22c55e var(--health),rgba(255,255,255,.12) var(--health)); }
.life-indicator.sending { background:rgba(37,99,235,.95); }
.life-indicator.paused { background:rgba(120,53,15,.95); }
.life-spin { animation:life-spin 1.5s linear infinite; }
.life-indicator-enter-active,.life-indicator-leave-active { transition:opacity .2s ease,transform .2s ease; }
.life-indicator-enter-from,.life-indicator-leave-to { opacity:0; transform:translate(-50%,8px) scale(.98); }
@keyframes life-spin { to { transform:rotate(360deg); } }
</style>
