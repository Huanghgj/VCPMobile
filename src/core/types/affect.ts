export interface AffectPadState {
  pleasure: number;
  arousal: number;
  dominance: number;
}

export interface AffectRelationshipState {
  trust: number;
  intimacy: number;
  attachment: number;
  security: number;
  resentment: number;
  jealousy: number;
  distanceNeed: number;
}

export interface AffectPersonaBaseline {
  openness: number;
  conscientiousness: number;
  extraversion: number;
  agreeableness: number;
  neuroticism: number;
  pad: AffectPadState;
}

export interface AffectConfig {
  enabled: boolean;
  localModelEnabled: boolean;
  jealousyIntensity: number;
  coldnessIntensity: number;
  leaveThreatIntensity: number;
  guiltPressureIntensity: number;
  emotionalSensitivity: number;
  recoverySpeed: number;
  relationshipMemory: number;
  expressionVariability: number;
}

export interface AffectModelObservation {
  modelId: string;
  modelVersion: string;
  scores: Record<string, number>;
  inferenceMs: number;
  truncated: boolean;
}

export interface AffectState {
  agentId: string;
  personaBaseline: AffectPersonaBaseline;
  /** 角色经过历次互动、衰减后形成的当前累计主情绪。 */
  rolePrimaryEmotion: string;
  rolePrimaryEmotionIntensity: number;
  /** @deprecated 使用 rolePrimaryEmotion。保留用于兼容旧后端字段。 */
  primaryEmotion: string;
  /** @deprecated 使用 rolePrimaryEmotionIntensity。保留用于兼容旧后端字段。 */
  primaryEmotionIntensity: number;
  shortEmotions: Record<string, number>;
  pad: AffectPadState;
  relationship: AffectRelationshipState;
  relationshipStage: string;
  recognizer: string;
  config: AffectConfig;
  updatedAt: number;
}

export interface AffectEvent {
  id: string;
  agentId: string;
  eventType: string;
  summary: string;
  /** 角色对本轮输入产生的即时情绪反应。 */
  roleReactionEmotion?: string;
  roleReactionIntensity?: number;
  /** 角色本轮最多四种即时混合反应，值为目标强度。 */
  roleReactionEmotions?: Record<string, number>;
  /** @deprecated 使用 roleReactionEmotion。保留用于兼容旧事件数据。 */
  emotion?: string;
  /** @deprecated 使用 roleReactionIntensity。保留用于兼容旧事件数据。 */
  intensity?: number;
  sourceText?: string;
  createdAt: number;
  deltas?: Record<string, number>;
  confidence?: number;
  signals?: string[];
  relationshipSignals?: string[];
  userAffectSignals?: string[];
  recognizer?: string;
  /** 模型对用户本轮表达的情绪观察，不代表角色自身情绪。 */
  userEmotionObservation?: AffectModelObservation;
  /** @deprecated 使用 userEmotionObservation。保留用于兼容旧事件数据。 */
  modelObservation?: AffectModelObservation;
}

export const DEFAULT_AFFECT_CONFIG: AffectConfig = {
  enabled: true,
  localModelEnabled: true,
  jealousyIntensity: 0.55,
  coldnessIntensity: 0.4,
  leaveThreatIntensity: 0.2,
  guiltPressureIntensity: 0.25,
  emotionalSensitivity: 0.72,
  recoverySpeed: 0.5,
  relationshipMemory: 0.78,
  expressionVariability: 0.72,
};

export const createDefaultAffectState = (agentId = ""): AffectState => ({
  agentId,
  personaBaseline: {
    openness: 0.2,
    conscientiousness: 0.1,
    extraversion: 0,
    agreeableness: 0.1,
    neuroticism: 0,
    pad: { pleasure: 0.059, arousal: 0.06, dominance: 0.035 },
  },
  rolePrimaryEmotion: "平静",
  rolePrimaryEmotionIntensity: 0,
  primaryEmotion: "平静",
  primaryEmotionIntensity: 0,
  shortEmotions: {},
  pad: {
    pleasure: 0,
    arousal: 0,
    dominance: 0,
  },
  relationship: {
    trust: 0.5,
    intimacy: 0.2,
    attachment: 0.2,
    security: 0.5,
    resentment: 0,
    jealousy: 0,
    distanceNeed: 0,
  },
  relationshipStage: "建立关系",
  recognizer: "heuristic_v2",
  config: { ...DEFAULT_AFFECT_CONFIG },
  updatedAt: 0,
});
