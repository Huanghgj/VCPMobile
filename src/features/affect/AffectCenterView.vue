<script setup lang="ts">
import { computed, ref, watch } from "vue";
import {
  Activity,
  BrainCircuit,
  Clock3,
  HeartPulse,
  RefreshCw,
  RotateCcw,
  Save,
  X,
} from "lucide-vue-next";
import SlidePage from "../../components/ui/SlidePage.vue";
import SettingsSwitch from "../../components/settings/SettingsSwitch.vue";
import { useAffectStore } from "../../core/stores/affect";
import { useNotificationStore } from "../../core/stores/notification";
import type { AffectConfig, AffectPersonaBaseline } from "../../core/types/affect";
import type { AffectEvent } from "../../core/types/affect";

const props = withDefaults(
  defineProps<{
    agentId?: string;
    isOpen?: boolean;
    zIndex?: number;
  }>(),
  {
    agentId: "",
    isOpen: false,
    zIndex: 50,
  },
);

const emit = defineEmits<{
  close: [];
}>();

const affect = useAffectStore();
const notifications = useNotificationStore();
const draft = ref<AffectConfig>({ ...affect.state.config });
const draftPersona = ref<AffectPersonaBaseline>({
  ...affect.state.personaBaseline,
  pad: { ...affect.state.personaBaseline.pad },
});
const isResetting = ref(false);

const clamp01 = (value: number) => Math.min(1, Math.max(0, Number(value) || 0));
const percent = (value: number) => `${Math.round(clamp01(value) * 100)}%`;
const signedPercent = (value: number) => `${value >= 0 ? "+" : ""}${Math.round(value * 100)}%`;
const padPosition = (value: number) => `${clamp01((value + 1) / 2) * 100}%`;

const relationshipMetrics = computed(() => [
  { key: "trust", label: "信任", value: affect.state.relationship.trust, tone: "blue" },
  { key: "intimacy", label: "亲密", value: affect.state.relationship.intimacy, tone: "pink" },
  { key: "attachment", label: "依恋", value: affect.state.relationship.attachment, tone: "purple" },
  { key: "security", label: "安全感", value: affect.state.relationship.security, tone: "green" },
  { key: "resentment", label: "怨气", value: affect.state.relationship.resentment, tone: "orange" },
  { key: "jealousy", label: "嫉妒", value: affect.state.relationship.jealousy, tone: "rose" },
  { key: "distanceNeed", label: "距离需求", value: affect.state.relationship.distanceNeed, tone: "slate" },
]);

const behaviorFields = [
  { key: "jealousyIntensity", label: "吃醋与占有欲", hint: "受到竞争或注意力转移时表达嫉妒" },
  { key: "coldnessIntensity", label: "冷落与疏离", hint: "受伤或不满时减少热情、拉开距离" },
  { key: "leaveThreatIntensity", label: "离开威胁", hint: "冲突升级时表达结束关系或离开的意图" },
  { key: "guiltPressureIntensity", label: "内疚施压", hint: "强调被忽视和付出，争取用户回应" },
] as const;

const dynamicsFields = [
  { key: "emotionalSensitivity", label: "情绪敏感度", hint: "同一条消息引发短期情绪波动的幅度" },
  { key: "recoverySpeed", label: "恢复速度", hint: "情绪回落到人格基线的速度" },
  { key: "relationshipMemory", label: "关系记忆", hint: "长期互动对信任与依恋留下影响的深度" },
  { key: "expressionVariability", label: "表达变化度", hint: "同一情绪采用不同措辞、节奏和表达路径的程度" },
] as const;

const personaFields = [
  { key: "openness", label: "开放性", low: "保守熟悉", high: "探索新奇" },
  { key: "conscientiousness", label: "尽责性", low: "随性", high: "自律负责" },
  { key: "extraversion", label: "外向性", low: "克制内向", high: "主动外放" },
  { key: "agreeableness", label: "宜人性", low: "强硬独立", high: "温和配合" },
  { key: "neuroticism", label: "神经质", low: "情绪稳定", high: "敏感易波动" },
] as const;

const formatTime = (value: number) => {
  if (!value) return "暂无记录";
  return new Date(value).toLocaleString();
};

const eventTitle = (eventType: string) => ({
  conversation: "对话评价",
  lifecycle: "生命周期事件",
  reset: "状态重置",
  disabled: "仅记录",
  message_appraisal: "消息评价",
  relationship_update: "关系变化",
  mood_decay: "情绪衰减",
  state_reset: "状态重置",
  config_update: "配置更新",
}[eventType] || eventType.replace(/_/g, " "));

const modelEmotionLabel = (scores?: Record<string, number>) => {
  if (!scores) return "";
  const labels: Record<string, string> = {
    neutral: "中性",
    joy: "开心",
    sadness: "悲伤",
    anger: "愤怒",
    confusion: "困惑",
    disgust: "厌恶",
    surprise: "惊奇",
    affection: "亲近/爱意",
  };
  const top = Object.entries(scores)
    .filter(([, value]) => Number.isFinite(Number(value)))
    .sort((left, right) => Number(right[1]) - Number(left[1]))[0];
  return top ? `${labels[top[0]] || top[0]} ${percent(Number(top[1]))}` : "";
};

const roleEmotionEntries = (event: AffectEvent) => Object.entries(event.roleReactionEmotions || {})
  .filter(([, intensity]) => Number.isFinite(Number(intensity)) && Number(intensity) > 0)
  .sort((left, right) => Number(right[1]) - Number(left[1]));

const showToast = (type: "success" | "error", title: string, message: string) => {
  notifications.addNotification({ type, title, message, toastOnly: true, duration: 2400 });
};

const load = async () => {
  if (!props.agentId) return;
  try {
    await affect.refresh(props.agentId);
    draft.value = { ...affect.state.config };
    draftPersona.value = { ...affect.state.personaBaseline, pad: { ...affect.state.personaBaseline.pad } };
  } catch (error) {
    console.error("[AffectCenter] Failed to load affect state:", error);
    showToast("error", "情感状态加载失败", affect.error || "后端情感引擎暂不可用");
  }
};

const save = async () => {
  if (!props.agentId) return;
  const config: AffectConfig = {
    enabled: Boolean(draft.value.enabled),
    localModelEnabled: Boolean(draft.value.localModelEnabled),
    jealousyIntensity: clamp01(draft.value.jealousyIntensity),
    coldnessIntensity: clamp01(draft.value.coldnessIntensity),
    leaveThreatIntensity: clamp01(draft.value.leaveThreatIntensity),
    guiltPressureIntensity: clamp01(draft.value.guiltPressureIntensity),
    emotionalSensitivity: clamp01(draft.value.emotionalSensitivity),
    recoverySpeed: clamp01(draft.value.recoverySpeed),
    relationshipMemory: clamp01(draft.value.relationshipMemory),
    expressionVariability: clamp01(draft.value.expressionVariability),
  };
  try {
    await affect.updateConfig(props.agentId, config);
    await affect.updatePersona(props.agentId, {
      ...draftPersona.value,
      openness: Math.min(1, Math.max(-1, draftPersona.value.openness)),
      conscientiousness: Math.min(1, Math.max(-1, draftPersona.value.conscientiousness)),
      extraversion: Math.min(1, Math.max(-1, draftPersona.value.extraversion)),
      agreeableness: Math.min(1, Math.max(-1, draftPersona.value.agreeableness)),
      neuroticism: Math.min(1, Math.max(-1, draftPersona.value.neuroticism)),
    });
    draft.value = { ...affect.state.config };
    draftPersona.value = { ...affect.state.personaBaseline, pad: { ...affect.state.personaBaseline.pad } };
    showToast("success", "情感与人格已保存", "新的动力学和人格基线会用于之后的对话");
  } catch (error) {
    console.error("[AffectCenter] Failed to save affect config:", error);
    showToast("error", "保存失败", affect.error || "请稍后重试");
  }
};

const reset = async () => {
  if (!props.agentId || !confirm("确定重置这个角色的情绪、关系和事件记录吗？人格与行为配置会保留。")) return;
  isResetting.value = true;
  try {
    await affect.reset(props.agentId);
    draft.value = { ...affect.state.config };
    draftPersona.value = { ...affect.state.personaBaseline, pad: { ...affect.state.personaBaseline.pad } };
    showToast("success", "情感状态已重置", "角色将从初始心境重新开始积累关系");
  } catch (error) {
    console.error("[AffectCenter] Failed to reset affect state:", error);
    showToast("error", "重置失败", affect.error || "请稍后重试");
  } finally {
    isResetting.value = false;
  }
};

watch(
  () => [props.isOpen, props.agentId] as const,
  ([isOpen]) => {
    if (isOpen) load();
  },
  { immediate: true },
);
</script>

<template>
  <SlidePage :is-open="props.isOpen" :z-index="props.zIndex">
    <div class="affect-page flex h-full w-full flex-col text-primary-text pointer-events-auto">
      <header class="affect-header shrink-0">
        <div class="min-w-0">
          <h2>情感中枢</h2>
          <p>角色心境、关系记忆与表达倾向</p>
        </div>
        <div class="flex items-center gap-2">
          <button class="icon-button" :disabled="affect.loading" aria-label="刷新" @click="load">
            <RefreshCw :size="18" :class="{ 'animate-spin': affect.loading }" />
          </button>
          <button class="icon-button" aria-label="关闭" @click="emit('close')"><X :size="19" /></button>
        </div>
      </header>

      <main class="affect-content flex-1 overflow-y-auto no-rubber-band">
        <div v-if="!props.agentId" class="empty-state">
          <BrainCircuit :size="34" />
          <strong>还没有选择角色</strong>
          <span>请从 Agent 设置中打开情感中枢。</span>
        </div>

        <template v-else>
          <section class="emotion-hero">
            <div class="emotion-orb"><HeartPulse :size="23" /></div>
            <div class="min-w-0">
              <span class="eyebrow">角色当前累计心境</span>
              <h3>{{ affect.loading && !affect.hasState ? '读取中…' : affect.state.rolePrimaryEmotion }}</h3>
              <p>{{ affect.state.relationshipStage }} · 累计强度 {{ percent(affect.state.rolePrimaryEmotionIntensity) }} · {{ affect.state.recognizer }}</p>
            </div>
            <div class="emotion-score">{{ Math.round(clamp01(affect.state.rolePrimaryEmotionIntensity) * 100) }}</div>
          </section>

          <section class="affect-section">
            <div class="section-heading"><Activity :size="15" /><span>PAD 心境坐标</span></div>
            <div class="surface pad-grid">
              <div class="pad-card">
                <div><span>愉悦度 P</span><strong>{{ signedPercent(affect.state.pad.pleasure) }}</strong></div>
                <div class="pad-track"><i :style="{ left: padPosition(affect.state.pad.pleasure) }" /></div>
                <small>不愉快</small><small>愉快</small>
              </div>
              <div class="pad-card">
                <div><span>唤醒度 A</span><strong>{{ signedPercent(affect.state.pad.arousal) }}</strong></div>
                <div class="pad-track"><i :style="{ left: padPosition(affect.state.pad.arousal) }" /></div>
                <small>平静</small><small>激动</small>
              </div>
              <div class="pad-card">
                <div><span>支配度 D</span><strong>{{ signedPercent(affect.state.pad.dominance) }}</strong></div>
                <div class="pad-track"><i :style="{ left: padPosition(affect.state.pad.dominance) }" /></div>
                <small>顺从</small><small>强势</small>
              </div>
            </div>
          </section>

          <section class="affect-section">
            <div class="section-heading"><BrainCircuit :size="15" /><span>人格基线 · Big Five</span></div>
            <div class="surface behavior-panel">
              <label v-for="field in personaFields" :key="field.key" class="behavior-field">
                <div><span><strong>{{ field.label }}</strong><small>{{ field.low }} ← → {{ field.high }}</small></span><b>{{ Math.round(draftPersona[field.key] * 100) }}</b></div>
                <input v-model.number="draftPersona[field.key]" type="range" min="-1" max="1" step="0.01" />
              </label>
            </div>
          </section>

          <section class="affect-section">
            <div class="section-heading"><HeartPulse :size="15" /><span>关系状态</span></div>
            <div class="surface relationship-list">
              <div v-for="metric in relationshipMetrics" :key="metric.key" class="relationship-row">
                <span>{{ metric.label }}</span>
                <div class="metric-track"><i :class="metric.tone" :style="{ width: percent(metric.value) }" /></div>
                <strong>{{ Math.round(clamp01(metric.value) * 100) }}</strong>
              </div>
            </div>
          </section>

          <section class="affect-section">
            <div class="section-heading"><Activity :size="15" /><span>情感动力学</span></div>
            <div class="surface behavior-panel">
              <label v-for="field in dynamicsFields" :key="field.key" class="behavior-field" :class="{ disabled: !draft.enabled }">
                <div><span><strong>{{ field.label }}</strong><small>{{ field.hint }}</small></span><b>{{ Math.round(draft[field.key] * 100) }}</b></div>
                <input v-model.number="draft[field.key]" type="range" min="0" max="1" step="0.01" :disabled="!draft.enabled" />
              </label>
            </div>
          </section>

          <section class="affect-section">
            <div class="section-heading"><BrainCircuit :size="15" /><span>戏剧行为</span></div>
            <div class="surface behavior-panel">
              <div class="engine-switch">
                <div><strong>启用角色情感引擎</strong><span>普通聊天与主动消息共享同一状态</span></div>
                <SettingsSwitch v-model="draft.enabled" />
              </div>
              <div class="divider" />
              <div class="engine-switch" :class="{ disabled: !draft.enabled }">
                <div><strong>本地情绪识别模型</strong><span>Android 端 INT8 推理；失败或超时自动使用规则</span></div>
                <SettingsSwitch v-model="draft.localModelEnabled" :disabled="!draft.enabled" />
              </div>
              <div class="divider" />
              <label v-for="field in behaviorFields" :key="field.key" class="behavior-field" :class="{ disabled: !draft.enabled }">
                <div><span><strong>{{ field.label }}</strong><small>{{ field.hint }}</small></span><b>{{ Math.round(draft[field.key] * 100) }}</b></div>
                <input v-model.number="draft[field.key]" type="range" min="0" max="1" step="0.01" :disabled="!draft.enabled" />
              </label>
              <button class="primary-button" :disabled="affect.saving" @click="save">
                <Save :size="16" />{{ affect.saving ? '正在保存…' : '保存行为配置' }}
              </button>
            </div>
          </section>

          <section class="affect-section">
            <div class="section-heading"><Clock3 :size="15" /><span>情感事件时间线</span></div>
            <div class="surface event-list">
              <div v-if="affect.events.length === 0" class="event-empty">还没有情感事件。之后的对话会在这里留下状态变化依据。</div>
              <article v-for="event in affect.events" :key="event.id" class="event-item">
                <i />
                <div class="min-w-0">
                  <div class="event-meta"><strong>{{ eventTitle(event.eventType) }}</strong><time>{{ formatTime(event.createdAt) }}</time></div>
                  <p>{{ event.summary }}</p>
                  <blockquote v-if="event.sourceText">{{ event.sourceText }}</blockquote>
                  <div v-if="roleEmotionEntries(event).length || event.confidence != null || event.relationshipSignals?.length || event.userAffectSignals?.length || event.userEmotionObservation" class="event-tags">
                    <span v-for="([emotion, intensity]) in roleEmotionEntries(event)" :key="`role-${emotion}`">角色即时反应：{{ emotion }} {{ percent(Number(intensity)) }}</span>
                    <span v-if="event.confidence != null">规则评价置信度 {{ percent(event.confidence) }}</span>
                    <span v-for="signal in event.relationshipSignals" :key="`relationship-${signal}`">关系信号：{{ signal }}</span>
                    <span v-for="signal in event.userAffectSignals" :key="`user-affect-${signal}`">用户情绪线索（规则）：{{ signal }}</span>
                    <span v-if="event.userEmotionObservation">用户情绪观察（模型）：{{ modelEmotionLabel(event.userEmotionObservation.scores) }}</span>
                    <span v-if="event.userEmotionObservation">本地模型 {{ Math.round(event.userEmotionObservation.inferenceMs) }}ms</span>
                  </div>
                </div>
              </article>
            </div>
          </section>

          <button class="reset-button" :disabled="isResetting" @click="reset">
            <RotateCcw :size="15" />{{ isResetting ? '正在重置…' : '重置角色情感状态' }}
          </button>
        </template>
      </main>
    </div>
  </SlidePage>
</template>

<style scoped>
.affect-page { background:var(--primary-bg); }
.affect-header { min-height:calc(var(--vcp-safe-top,24px) + 62px); padding:calc(var(--vcp-safe-top,24px) + 8px) 18px 10px; display:flex; align-items:center; justify-content:space-between; gap:12px; border-bottom:1px solid color-mix(in srgb,currentColor 7%,transparent); background:color-mix(in srgb,var(--primary-bg) 92%,transparent); }
.affect-header h2 { font-size:17px; font-weight:750; letter-spacing:0; }.affect-header p { margin-top:2px; font-size:10px; opacity:.42; }
.icon-button { width:40px; height:40px; display:grid; place-items:center; border-radius:12px; background:color-mix(in srgb,currentColor 5%,transparent); transition:transform .12s ease; }.icon-button:active:not(:disabled) { transform:scale(.94); }.icon-button:disabled { opacity:.35; }
.affect-content { padding:18px 16px calc(var(--vcp-safe-bottom,24px) + 30px); }
.emotion-hero { min-height:116px; padding:18px; display:grid; grid-template-columns:50px 1fr auto; align-items:center; gap:14px; border:1px solid rgba(218,74,129,.18); border-radius:8px; background:rgba(236,72,153,.08); box-shadow:0 8px 24px rgba(0,0,0,.05); }
.emotion-orb { width:50px; height:50px; display:grid; place-items:center; border-radius:8px; color:#fff; background:#db2777; box-shadow:0 8px 18px rgba(219,39,119,.2); }.eyebrow { font-size:8px; font-weight:800; letter-spacing:.14em; opacity:.42; }.emotion-hero h3 { margin:2px 0; font-size:22px; font-weight:780; letter-spacing:0; }.emotion-hero p { font-size:9px; opacity:.44; overflow:hidden; text-overflow:ellipsis; white-space:nowrap; }.emotion-score { width:42px; height:42px; display:grid; place-items:center; border:1px solid color-mix(in srgb,currentColor 10%,transparent); border-radius:50%; font-size:14px; font-weight:800; background:color-mix(in srgb,var(--primary-bg) 45%,transparent); }
.affect-section { margin-top:24px; }.section-heading { margin:0 0 9px 10px; display:flex; align-items:center; gap:7px; font-size:11px; font-weight:700; opacity:.5; }.surface { border:1px solid color-mix(in srgb,currentColor 6%,transparent); border-radius:8px; background:color-mix(in srgb,currentColor 4.5%,transparent); overflow:hidden; }
.pad-grid { padding:13px; display:grid; gap:10px; }.pad-card { padding:11px 12px 9px; border-radius:6px; background:color-mix(in srgb,currentColor 4%,transparent); display:grid; grid-template-columns:1fr auto; gap:7px; }.pad-card > div:first-child { grid-column:1/-1; display:flex; align-items:center; justify-content:space-between; font-size:11px; }.pad-card strong { font-family:ui-monospace,monospace; font-size:11px; }.pad-track { position:relative; grid-column:1/-1; height:4px; border-radius:99px; background:linear-gradient(90deg,#64748b 0%,color-mix(in srgb,currentColor 10%,transparent) 50%,#ec4899 100%); }.pad-track::after { content:""; position:absolute; left:50%; top:-2px; width:1px; height:8px; background:currentColor; opacity:.2; }.pad-track i { position:absolute; top:50%; width:12px; height:12px; border:2px solid var(--primary-bg); border-radius:50%; background:#ec4899; transform:translate(-50%,-50%); box-shadow:0 1px 5px rgba(0,0,0,.25); }.pad-card small { font-size:8px; opacity:.32; }.pad-card small:last-child { text-align:right; }
.relationship-list { padding:8px 13px; }.relationship-row { min-height:42px; display:grid; grid-template-columns:64px 1fr 28px; align-items:center; gap:10px; border-bottom:1px solid color-mix(in srgb,currentColor 6%,transparent); font-size:11px; }.relationship-row:last-child { border-bottom:0; }.relationship-row > span { opacity:.62; }.relationship-row strong { text-align:right; font-family:ui-monospace,monospace; font-size:10px; }.metric-track { height:6px; overflow:hidden; border-radius:99px; background:color-mix(in srgb,currentColor 7%,transparent); }.metric-track i { display:block; height:100%; border-radius:inherit; background:#3b82f6; }.metric-track i.pink { background:#ec4899; }.metric-track i.purple { background:#8b5cf6; }.metric-track i.green { background:#22c55e; }.metric-track i.orange { background:#f59e0b; }.metric-track i.rose { background:#f43f5e; }.metric-track i.slate { background:#64748b; }
.behavior-panel { padding:14px; }.engine-switch { min-height:46px; display:flex; align-items:center; justify-content:space-between; gap:16px; }.engine-switch > div { display:flex; flex-direction:column; gap:3px; }.engine-switch strong { font-size:13px; }.engine-switch span { font-size:9px; opacity:.4; }.divider { height:1px; margin:10px 0 4px; background:color-mix(in srgb,currentColor 7%,transparent); }.behavior-field { display:block; padding:12px 0 7px; transition:opacity .15s ease; }.behavior-field.disabled { opacity:.35; }.behavior-field > div { display:flex; align-items:flex-start; justify-content:space-between; gap:12px; }.behavior-field span { display:flex; flex-direction:column; gap:3px; }.behavior-field strong { font-size:12px; }.behavior-field small { font-size:9px; line-height:1.35; opacity:.4; }.behavior-field b { font:700 11px ui-monospace,monospace; opacity:.65; }.behavior-field input { width:100%; height:24px; accent-color:#ec4899; }.primary-button { width:100%; min-height:44px; margin-top:8px; display:flex; align-items:center; justify-content:center; gap:8px; border-radius:8px; color:#fff; background:#db2777; font-size:12px; font-weight:700; box-shadow:0 8px 20px rgba(219,39,119,.17); }.primary-button:disabled { opacity:.45; }
.event-list { padding:7px 13px; }.event-empty { padding:24px 12px; text-align:center; font-size:10px; line-height:1.6; opacity:.4; }.event-item { position:relative; padding:13px 0 13px 20px; display:grid; grid-template-columns:1fr; border-bottom:1px solid color-mix(in srgb,currentColor 6%,transparent); }.event-item:last-child { border-bottom:0; }.event-item > i { position:absolute; left:3px; top:18px; width:8px; height:8px; border:2px solid color-mix(in srgb,var(--primary-bg) 85%,transparent); border-radius:50%; background:#ec4899; box-shadow:0 0 0 3px rgba(236,72,153,.13); }.event-item:not(:last-child)::before { content:""; position:absolute; left:6.5px; top:26px; bottom:-6px; width:1px; background:color-mix(in srgb,currentColor 9%,transparent); }.event-meta { display:flex; align-items:center; justify-content:space-between; gap:10px; }.event-meta strong { font-size:11px; }.event-meta time { font-size:8px; opacity:.35; }.event-item p { margin-top:4px; font-size:10px; line-height:1.5; opacity:.62; }.event-item blockquote { margin-top:7px; padding:7px 9px; border-left:2px solid rgba(236,72,153,.35); border-radius:0 8px 8px 0; background:color-mix(in srgb,currentColor 4%,transparent); font-size:9px; line-height:1.45; opacity:.5; overflow-wrap:anywhere; }.event-tags { margin-top:7px; display:flex; gap:5px; }.event-tags span { padding:3px 7px; border-radius:99px; color:#db2777; background:rgba(236,72,153,.1); font-size:8px; font-weight:700; }
.reset-button { width:100%; min-height:42px; margin-top:20px; display:flex; align-items:center; justify-content:center; gap:7px; border:1px solid rgba(244,63,94,.18); border-radius:13px; color:#f43f5e; background:rgba(244,63,94,.06); font-size:11px; font-weight:700; }.reset-button:disabled { opacity:.4; }.empty-state { height:65vh; display:flex; flex-direction:column; align-items:center; justify-content:center; gap:8px; text-align:center; opacity:.48; }.empty-state strong { font-size:14px; }.empty-state span { font-size:10px; }
@media (min-width:700px) { .affect-content { width:min(620px,100%); margin:0 auto; } }
@media (prefers-reduced-motion:reduce) { .icon-button { transition-duration:.01ms; } }
</style>
