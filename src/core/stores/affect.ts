import { defineStore } from "pinia";
import { computed, ref } from "vue";
import { invoke } from "@tauri-apps/api/core";
import type { AffectConfig, AffectEvent, AffectPersonaBaseline, AffectState } from "../types/affect";
import { createDefaultAffectState, DEFAULT_AFFECT_CONFIG } from "../types/affect";

const finite = (value: unknown, fallback: number) => {
  const parsed = Number(value);
  return Number.isFinite(parsed) ? parsed : fallback;
};

const timestamp = (value: unknown) => {
  if (typeof value === "number" && Number.isFinite(value)) return value;
  if (typeof value === "string") {
    const parsed = Date.parse(value);
    if (Number.isFinite(parsed)) return parsed;
  }
  return 0;
};

const normalizeState = (raw: any, agentId: string): AffectState => {
  const fallback = createDefaultAffectState(agentId);
  const pad = raw?.pad || raw?.mood || {};
  const persona = raw?.personaBaseline || raw?.persona_baseline || {};
  const personaPad = persona?.pad || {};
  const relationship = raw?.relationship || raw?.relationships || {};
  const config = raw?.config || {};
  const rolePrimaryEmotion = String(
    raw?.rolePrimaryEmotion
      || raw?.role_primary_emotion
      || raw?.primaryEmotion
      || raw?.primary_emotion
      || raw?.emotion
      || fallback.rolePrimaryEmotion,
  );
  const rolePrimaryEmotionIntensity = finite(
    raw?.rolePrimaryEmotionIntensity
      ?? raw?.role_primary_emotion_intensity
      ?? raw?.primaryEmotionIntensity
      ?? raw?.primary_emotion_intensity
      ?? raw?.emotionIntensity
      ?? raw?.emotion_intensity,
    fallback.rolePrimaryEmotionIntensity,
  );

  return {
    agentId: String(raw?.agentId || raw?.agent_id || agentId),
    personaBaseline: {
      openness: finite(persona.openness, fallback.personaBaseline.openness),
      conscientiousness: finite(persona.conscientiousness, fallback.personaBaseline.conscientiousness),
      extraversion: finite(persona.extraversion, fallback.personaBaseline.extraversion),
      agreeableness: finite(persona.agreeableness, fallback.personaBaseline.agreeableness),
      neuroticism: finite(persona.neuroticism, fallback.personaBaseline.neuroticism),
      pad: {
        pleasure: finite(personaPad.pleasure, fallback.personaBaseline.pad.pleasure),
        arousal: finite(personaPad.arousal, fallback.personaBaseline.pad.arousal),
        dominance: finite(personaPad.dominance, fallback.personaBaseline.pad.dominance),
      },
    },
    rolePrimaryEmotion,
    rolePrimaryEmotionIntensity,
    primaryEmotion: rolePrimaryEmotion,
    primaryEmotionIntensity: rolePrimaryEmotionIntensity,
    shortEmotions:
      raw?.shortEmotions && typeof raw.shortEmotions === "object"
        ? raw.shortEmotions
        : raw?.short_emotions && typeof raw.short_emotions === "object"
          ? raw.short_emotions
          : {},
    pad: {
      pleasure: finite(pad.pleasure ?? raw?.pleasure, fallback.pad.pleasure),
      arousal: finite(pad.arousal ?? raw?.arousal, fallback.pad.arousal),
      dominance: finite(pad.dominance ?? raw?.dominance, fallback.pad.dominance),
    },
    relationship: {
      trust: finite(relationship.trust, fallback.relationship.trust),
      intimacy: finite(relationship.intimacy, fallback.relationship.intimacy),
      attachment: finite(relationship.attachment, fallback.relationship.attachment),
      security: finite(relationship.security, fallback.relationship.security),
      resentment: finite(relationship.resentment, fallback.relationship.resentment),
      jealousy: finite(relationship.jealousy, fallback.relationship.jealousy),
      distanceNeed: finite(
        relationship.distanceNeed ?? relationship.distance_need,
        fallback.relationship.distanceNeed,
      ),
    },
    relationshipStage: String(raw?.relationshipStage || raw?.relationship_stage || fallback.relationshipStage),
    recognizer: String(raw?.recognizer || fallback.recognizer),
    config: {
      enabled: config.enabled ?? fallback.config.enabled,
      localModelEnabled:
        config.localModelEnabled ?? config.local_model_enabled ?? fallback.config.localModelEnabled,
      jealousyIntensity: finite(config.jealousyIntensity ?? config.jealousy_intensity, DEFAULT_AFFECT_CONFIG.jealousyIntensity),
      coldnessIntensity: finite(config.coldnessIntensity ?? config.coldness_intensity, DEFAULT_AFFECT_CONFIG.coldnessIntensity),
      leaveThreatIntensity: finite(config.leaveThreatIntensity ?? config.leave_threat_intensity, DEFAULT_AFFECT_CONFIG.leaveThreatIntensity),
      guiltPressureIntensity: finite(config.guiltPressureIntensity ?? config.guilt_pressure_intensity, DEFAULT_AFFECT_CONFIG.guiltPressureIntensity),
      emotionalSensitivity: finite(config.emotionalSensitivity ?? config.emotional_sensitivity, DEFAULT_AFFECT_CONFIG.emotionalSensitivity),
      recoverySpeed: finite(config.recoverySpeed ?? config.recovery_speed, DEFAULT_AFFECT_CONFIG.recoverySpeed),
      relationshipMemory: finite(config.relationshipMemory ?? config.relationship_memory, DEFAULT_AFFECT_CONFIG.relationshipMemory),
      expressionVariability: finite(config.expressionVariability ?? config.expression_variability, DEFAULT_AFFECT_CONFIG.expressionVariability),
    },
    updatedAt: timestamp(raw?.updatedAt ?? raw?.updated_at),
  };
};

const normalizeEvent = (raw: any, index: number, agentId: string): AffectEvent => {
  const rawObservation = raw?.userEmotionObservation
    || raw?.user_emotion_observation
    || raw?.modelObservation
    || raw?.model_observation;
  const userEmotionObservation = rawObservation
    ? {
        modelId: String(rawObservation.modelId || rawObservation.model_id || ""),
        modelVersion: String(rawObservation.modelVersion || rawObservation.model_version || ""),
        scores: rawObservation.scores && typeof rawObservation.scores === "object"
          ? rawObservation.scores
          : {},
        inferenceMs: finite(rawObservation.inferenceMs ?? rawObservation.inference_ms, 0),
        truncated: Boolean(rawObservation.truncated ?? false),
      }
    : undefined;
  const rawRoleReactionEmotion = raw?.roleReactionEmotion
    || raw?.role_reaction_emotion
    || raw?.emotion;
  const roleReactionEmotion = rawRoleReactionEmotion
    ? String(rawRoleReactionEmotion)
    : undefined;
  const rawRoleReactionIntensity = raw?.roleReactionIntensity
    ?? raw?.role_reaction_intensity
    ?? raw?.intensity;
  const roleReactionIntensity = rawRoleReactionIntensity == null
    ? undefined
    : finite(rawRoleReactionIntensity, 0);
  const rawRoleReactionEmotions = raw?.roleReactionEmotions
    || raw?.role_reaction_emotions
    || raw?.roleEmotions
    || raw?.role_emotions;
  const roleReactionEmotions: Record<string, number> = {};
  if (rawRoleReactionEmotions && typeof rawRoleReactionEmotions === "object") {
    for (const [emotion, rawIntensity] of Object.entries(rawRoleReactionEmotions)) {
      const intensity = finite(rawIntensity, 0);
      if (intensity > 0) roleReactionEmotions[String(emotion)] = intensity;
    }
  } else if (roleReactionEmotion && roleReactionIntensity != null) {
    roleReactionEmotions[roleReactionEmotion] = roleReactionIntensity;
  }
  const signals: string[] = Array.isArray(raw?.signals)
    ? raw.signals.map((item: unknown) => String(item))
    : [];
  const userAffectSignalNames = new Set(["用户低落", "共享喜悦", "用户愤怒"]);
  const relationshipSignalSource = raw?.relationshipSignals ?? raw?.relationship_signals;
  const userAffectSignalSource = raw?.userAffectSignals ?? raw?.user_affect_signals;
  const relationshipSignals: string[] = Array.isArray(relationshipSignalSource)
    ? relationshipSignalSource.map((item: unknown) => String(item))
    : signals.filter((signal) => !userAffectSignalNames.has(signal));
  const userAffectSignals: string[] = Array.isArray(userAffectSignalSource)
    ? userAffectSignalSource.map((item: unknown) => String(item))
    : signals.filter((signal) => userAffectSignalNames.has(signal));

  return {
    id: String(raw?.id || `affect-event-${index}`),
    agentId: String(raw?.agentId || raw?.agent_id || agentId),
    eventType: String(raw?.eventType || raw?.event_type || raw?.kind || "state_update"),
    summary: String(raw?.summary || raw?.reason || raw?.description || "情感状态发生变化"),
    roleReactionEmotion,
    roleReactionIntensity,
    roleReactionEmotions,
    emotion: roleReactionEmotion,
    intensity: roleReactionIntensity,
    sourceText: raw?.sourceText || raw?.source_text ? String(raw?.sourceText || raw?.source_text) : undefined,
    createdAt: timestamp(raw?.createdAt ?? raw?.created_at ?? raw?.timestamp),
    deltas: raw?.deltas && typeof raw.deltas === "object" ? raw.deltas : undefined,
    confidence: raw?.confidence == null ? undefined : finite(raw.confidence, 0),
    signals,
    relationshipSignals,
    userAffectSignals,
    recognizer: raw?.recognizer ? String(raw.recognizer) : undefined,
    userEmotionObservation,
    modelObservation: userEmotionObservation,
  };
};

export const useAffectStore = defineStore("affect", () => {
  const state = ref<AffectState>(createDefaultAffectState());
  const events = ref<AffectEvent[]>([]);
  const loading = ref(false);
  const saving = ref(false);
  const error = ref<string | null>(null);

  const hasState = computed(() => Boolean(state.value.agentId));

  const refresh = async (agentId: string) => {
    if (!agentId) return;
    if (state.value.agentId !== agentId) {
      state.value = createDefaultAffectState(agentId);
      events.value = [];
    }
    loading.value = true;
    error.value = null;
    try {
      const [rawState, rawEvents] = await Promise.all([
        invoke<any>("get_affect_state", { agentId }),
        invoke<any[]>("list_affect_events", { agentId, limit: 50 }),
      ]);
      state.value = normalizeState(rawState, agentId);
      events.value = Array.isArray(rawEvents)
        ? rawEvents.map((item, index) => normalizeEvent(item, index, agentId))
        : [];
    } catch (err) {
      error.value = err instanceof Error ? err.message : String(err);
      throw err;
    } finally {
      loading.value = false;
    }
  };

  const updateConfig = async (agentId: string, config: AffectConfig) => {
    if (!agentId) return;
    saving.value = true;
    error.value = null;
    try {
      const raw = await invoke<any>("update_affect_config", { agentId, config });
      state.value = raw
        ? normalizeState(raw, agentId)
        : { ...state.value, config: { ...config } };
    } catch (err) {
      error.value = err instanceof Error ? err.message : String(err);
      throw err;
    } finally {
      saving.value = false;
    }
  };

  const updatePersona = async (agentId: string, persona: AffectPersonaBaseline) => {
    if (!agentId) return;
    saving.value = true;
    error.value = null;
    try {
      const raw = await invoke<any>("update_affect_persona", {
        agentId,
        persona: {
          openness: persona.openness,
          conscientiousness: persona.conscientiousness,
          extraversion: persona.extraversion,
          agreeableness: persona.agreeableness,
          neuroticism: persona.neuroticism,
        },
      });
      state.value = normalizeState(raw, agentId);
    } catch (err) {
      error.value = err instanceof Error ? err.message : String(err);
      throw err;
    } finally {
      saving.value = false;
    }
  };

  const reset = async (agentId: string) => {
    if (!agentId) return;
    loading.value = true;
    error.value = null;
    try {
      const raw = await invoke<any>("reset_affect_state", { agentId });
      state.value = raw ? normalizeState(raw, agentId) : createDefaultAffectState(agentId);
      const rawEvents = await invoke<any[]>("list_affect_events", { agentId, limit: 50 });
      events.value = Array.isArray(rawEvents)
        ? rawEvents.map((item, index) => normalizeEvent(item, index, agentId))
        : [];
    } catch (err) {
      error.value = err instanceof Error ? err.message : String(err);
      throw err;
    } finally {
      loading.value = false;
    }
  };

  return {
    state,
    events,
    loading,
    saving,
    error,
    hasState,
    refresh,
    updateConfig,
    updatePersona,
    reset,
  };
});
