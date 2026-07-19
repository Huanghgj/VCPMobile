use chrono::Utc;
use serde::{Deserialize, Serialize};
use sqlx::{Pool, Row, Sqlite};
use std::collections::BTreeMap;
use tauri::{AppHandle, State};
use uuid::Uuid;

use crate::vcp_modules::affect_recognizer::ModelAffectObservation;
use crate::vcp_modules::db_manager::DbState;

const PAD_DECAY_HOURS: f64 = 12.0;
const EMOTION_DECAY_HOURS: f64 = 4.0;
const REACTIVE_RELATIONSHIP_DECAY_HOURS: f64 = 72.0;
const STALE_PENDING_EVENT_MS: i64 = 30_000;
const RECOGNIZER_VERSION: &str = "heuristic_v2";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct PadState {
    pub pleasure: f64,
    pub arousal: f64,
    pub dominance: f64,
}

impl Default for PadState {
    fn default() -> Self {
        Self {
            pleasure: 0.0,
            arousal: 0.0,
            dominance: 0.0,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct PersonaBaseline {
    /// Big Five values are normalized to [-1, 1].
    pub openness: f64,
    pub conscientiousness: f64,
    pub extraversion: f64,
    pub agreeableness: f64,
    pub neuroticism: f64,
    pub pad: PadState,
}

impl Default for PersonaBaseline {
    fn default() -> Self {
        let mut baseline = Self {
            openness: 0.2,
            conscientiousness: 0.1,
            extraversion: 0.0,
            agreeableness: 0.1,
            neuroticism: 0.0,
            pad: PadState::default(),
        };
        baseline.refresh_pad_from_big_five();
        baseline
    }
}

impl PersonaBaseline {
    fn refresh_pad_from_big_five(&mut self) {
        // Mehrabian's Big Five -> PAD mapping used by ALMA.
        self.pad.pleasure =
            signed(0.21 * self.extraversion + 0.59 * self.agreeableness + 0.19 * self.neuroticism);
        self.pad.arousal =
            signed(0.15 * self.openness + 0.30 * self.agreeableness - 0.57 * self.neuroticism);
        self.pad.dominance = signed(
            0.25 * self.openness + 0.17 * self.conscientiousness + 0.60 * self.extraversion
                - 0.32 * self.agreeableness,
        );
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct RelationshipState {
    pub trust: f64,
    pub intimacy: f64,
    pub attachment: f64,
    pub security: f64,
    pub resentment: f64,
    pub jealousy: f64,
    pub distance_need: f64,
}

impl Default for RelationshipState {
    fn default() -> Self {
        Self {
            trust: 0.5,
            intimacy: 0.2,
            attachment: 0.2,
            security: 0.5,
            resentment: 0.0,
            jealousy: 0.0,
            distance_need: 0.0,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default, rename_all = "camelCase")]
pub struct AffectConfig {
    pub enabled: bool,
    pub local_model_enabled: bool,
    pub jealousy_intensity: f64,
    pub coldness_intensity: f64,
    pub leave_threat_intensity: f64,
    pub guilt_pressure_intensity: f64,
    pub emotional_sensitivity: f64,
    pub recovery_speed: f64,
    pub relationship_memory: f64,
    pub expression_variability: f64,
}

impl Default for AffectConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            local_model_enabled: true,
            jealousy_intensity: 0.55,
            coldness_intensity: 0.4,
            leave_threat_intensity: 0.2,
            guilt_pressure_intensity: 0.25,
            emotional_sensitivity: 0.72,
            recovery_speed: 0.5,
            relationship_memory: 0.78,
            expression_variability: 0.72,
        }
    }
}

impl AffectConfig {
    fn clamp(mut self) -> Self {
        self.jealousy_intensity = unit(self.jealousy_intensity);
        self.coldness_intensity = unit(self.coldness_intensity);
        self.leave_threat_intensity = unit(self.leave_threat_intensity);
        self.guilt_pressure_intensity = unit(self.guilt_pressure_intensity);
        self.emotional_sensitivity = unit(self.emotional_sensitivity);
        self.recovery_speed = unit(self.recovery_speed);
        self.relationship_memory = unit(self.relationship_memory);
        self.expression_variability = unit(self.expression_variability);
        self
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct AffectState {
    pub agent_id: String,
    pub persona_baseline: PersonaBaseline,
    pub primary_emotion: String,
    pub primary_emotion_intensity: f64,
    pub short_emotions: BTreeMap<String, f64>,
    pub pad: PadState,
    pub relationship: RelationshipState,
    pub relationship_stage: String,
    pub recognizer: String,
    pub config: AffectConfig,
    pub updated_at: i64,
}

impl AffectState {
    fn new(agent_id: &str, now: i64) -> Self {
        let persona_baseline = PersonaBaseline::default();
        Self {
            agent_id: agent_id.to_string(),
            pad: persona_baseline.pad.clone(),
            persona_baseline,
            primary_emotion: "平静".to_string(),
            primary_emotion_intensity: 0.0,
            short_emotions: BTreeMap::new(),
            relationship: RelationshipState::default(),
            relationship_stage: "建立关系".to_string(),
            recognizer: RECOGNIZER_VERSION.to_string(),
            config: AffectConfig::default(),
            updated_at: now,
        }
    }

    fn clamp(&mut self) {
        self.persona_baseline.openness = signed(self.persona_baseline.openness);
        self.persona_baseline.conscientiousness = signed(self.persona_baseline.conscientiousness);
        self.persona_baseline.extraversion = signed(self.persona_baseline.extraversion);
        self.persona_baseline.agreeableness = signed(self.persona_baseline.agreeableness);
        self.persona_baseline.neuroticism = signed(self.persona_baseline.neuroticism);
        clamp_pad(&mut self.persona_baseline.pad);
        clamp_pad(&mut self.pad);
        self.relationship.trust = unit(self.relationship.trust);
        self.relationship.intimacy = unit(self.relationship.intimacy);
        self.relationship.attachment = unit(self.relationship.attachment);
        self.relationship.security = unit(self.relationship.security);
        self.relationship.resentment = unit(self.relationship.resentment);
        self.relationship.jealousy = unit(self.relationship.jealousy);
        self.relationship.distance_need = unit(self.relationship.distance_need);
        self.short_emotions.retain(|_, value| {
            *value = unit(*value);
            *value >= 0.005
        });
        self.config = self.config.clone().clamp();
        self.refresh_primary_emotion();
        self.refresh_relationship_stage();
        self.recognizer = RECOGNIZER_VERSION.to_string();
    }

    fn refresh_primary_emotion(&mut self) {
        if let Some((emotion, intensity)) = self
            .short_emotions
            .iter()
            .max_by(|left, right| left.1.total_cmp(right.1))
        {
            self.primary_emotion = emotion.clone();
            self.primary_emotion_intensity = unit(*intensity);
        } else {
            self.primary_emotion = "平静".to_string();
            self.primary_emotion_intensity = 0.0;
        }
    }

    fn refresh_relationship_stage(&mut self) {
        let relationship = &self.relationship;
        self.relationship_stage = if relationship.resentment + relationship.distance_need >= 0.85 {
            "疏离".to_string()
        } else if relationship.resentment >= 0.22 || relationship.security <= 0.24 {
            "紧张".to_string()
        } else if relationship.attachment >= 0.68 && relationship.intimacy >= 0.58 {
            "深度依恋".to_string()
        } else if relationship.trust >= 0.66 && relationship.intimacy >= 0.45 {
            "亲近".to_string()
        } else if relationship.trust >= 0.54 || relationship.intimacy >= 0.30 {
            "熟悉".to_string()
        } else {
            "建立关系".to_string()
        };
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AffectEvent {
    pub id: String,
    pub agent_id: String,
    pub source_message_id: String,
    pub topic_id: Option<String>,
    pub source: String,
    pub event_type: String,
    pub summary: String,
    pub emotion: Option<String>,
    pub intensity: Option<f64>,
    /// Top role emotions for this turn. The legacy singular emotion fields are
    /// retained as a compact primary-reaction summary.
    pub role_emotions: BTreeMap<String, f64>,
    pub source_text: Option<String>,
    pub deltas: BTreeMap<String, f64>,
    pub confidence: Option<f64>,
    pub signals: Vec<String>,
    pub relationship_signals: Vec<String>,
    pub user_affect_signals: Vec<String>,
    pub recognizer: String,
    pub model_observation: Option<ModelAffectObservation>,
    pub created_at: i64,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RecordAffectEventInput {
    pub agent_id: String,
    pub source_message_id: String,
    pub source: String,
    pub text: String,
    pub topic_id: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PersonaTraitsInput {
    pub openness: f64,
    pub conscientiousness: f64,
    pub extraversion: f64,
    pub agreeableness: f64,
    pub neuroticism: f64,
}

impl PersonaTraitsInput {
    fn into_baseline(self) -> PersonaBaseline {
        let mut baseline = PersonaBaseline {
            openness: signed(self.openness),
            conscientiousness: signed(self.conscientiousness),
            extraversion: signed(self.extraversion),
            agreeableness: signed(self.agreeableness),
            neuroticism: signed(self.neuroticism),
            pad: PadState::default(),
        };
        baseline.refresh_pad_from_big_five();
        baseline
    }
}

#[derive(Debug, Default)]
struct EventImpact {
    event_type: &'static str,
    summary: String,
    pad: PadState,
    emotions: BTreeMap<&'static str, f64>,
    role_emotions: BTreeMap<&'static str, f64>,
    relationship: RelationshipDelta,
    confidence: f64,
    signals: Vec<&'static str>,
    signal_strengths: BTreeMap<&'static str, f64>,
}

#[derive(Debug, Clone, Default, PartialEq)]
struct RoleReaction {
    pad: PadState,
    /// Signed changes applied to the role's short-lived emotion state.
    emotion_deltas: BTreeMap<&'static str, f64>,
    /// The role's immediate mixed reaction, used for event audit and prompting.
    emotions: BTreeMap<&'static str, f64>,
}

#[derive(Debug, Clone, Default, PartialEq)]
struct RelationshipDelta {
    trust: f64,
    intimacy: f64,
    attachment: f64,
    security: f64,
    resentment: f64,
    jealousy: f64,
    distance_need: f64,
}

impl RelationshipDelta {
    fn scale(&mut self, factor: f64) {
        self.trust *= factor;
        self.intimacy *= factor;
        self.attachment *= factor;
        self.security *= factor;
        self.resentment *= factor;
        self.jealousy *= factor;
        self.distance_need *= factor;
    }
}

pub async fn setup_affect_tables(pool: &Pool<Sqlite>) -> Result<(), String> {
    sqlx::query(
        "CREATE TABLE IF NOT EXISTS affect_states (
            agent_id TEXT PRIMARY KEY,
            persona_baseline_json TEXT NOT NULL,
            pad_json TEXT NOT NULL,
            short_emotions_json TEXT NOT NULL,
            relationship_json TEXT NOT NULL,
            config_json TEXT NOT NULL,
            created_at BIGINT NOT NULL,
            updated_at BIGINT NOT NULL
        )",
    )
    .execute(pool)
    .await
    .map_err(|e| e.to_string())?;

    sqlx::query(
        "CREATE TABLE IF NOT EXISTS affect_events (
            id TEXT PRIMARY KEY,
            agent_id TEXT NOT NULL,
            source_message_id TEXT NOT NULL,
            topic_id TEXT,
            source TEXT NOT NULL,
            event_type TEXT NOT NULL DEFAULT 'pending',
            summary TEXT NOT NULL DEFAULT '',
            emotion TEXT,
            intensity REAL,
            role_emotions_json TEXT NOT NULL DEFAULT '{}',
            source_text TEXT,
            deltas_json TEXT NOT NULL DEFAULT '{}',
            confidence REAL,
            signals_json TEXT NOT NULL DEFAULT '[]',
            recognizer TEXT NOT NULL DEFAULT 'heuristic_v1',
            state_before_json TEXT,
            state_after_json TEXT,
            created_at BIGINT NOT NULL,
            UNIQUE(agent_id, source_message_id)
        )",
    )
    .execute(pool)
    .await
    .map_err(|e| e.to_string())?;

    ensure_affect_event_column(pool, "confidence", "confidence REAL").await?;
    ensure_affect_event_column(
        pool,
        "signals_json",
        "signals_json TEXT NOT NULL DEFAULT '[]'",
    )
    .await?;
    ensure_affect_event_column(
        pool,
        "role_emotions_json",
        "role_emotions_json TEXT NOT NULL DEFAULT '{}'",
    )
    .await?;
    ensure_affect_event_column(
        pool,
        "recognizer",
        "recognizer TEXT NOT NULL DEFAULT 'heuristic_v1'",
    )
    .await?;
    ensure_affect_event_column(
        pool,
        "model_observation_json",
        "model_observation_json TEXT",
    )
    .await?;

    // Affect history keeps derived signals and deltas only. Remove legacy
    // message excerpts so deleting chat history cannot leave a second copy of
    // the user's original text behind.
    sqlx::query("UPDATE affect_events SET source_text = NULL WHERE source_text IS NOT NULL")
        .execute(pool)
        .await
        .map_err(|e| e.to_string())?;

    sqlx::query(
        "CREATE INDEX IF NOT EXISTS idx_affect_events_agent_time
         ON affect_events(agent_id, created_at DESC)",
    )
    .execute(pool)
    .await
    .map_err(|e| e.to_string())?;
    Ok(())
}

async fn ensure_affect_event_column(
    pool: &Pool<Sqlite>,
    column: &str,
    definition: &str,
) -> Result<(), String> {
    let rows = sqlx::query("PRAGMA table_info(affect_events)")
        .fetch_all(pool)
        .await
        .map_err(|e| e.to_string())?;
    let exists = rows.iter().any(|row| {
        row.try_get::<String, _>("name")
            .map(|name| name == column)
            .unwrap_or(false)
    });
    if !exists {
        sqlx::query(&format!(
            "ALTER TABLE affect_events ADD COLUMN {definition}"
        ))
        .execute(pool)
        .await
        .map_err(|e| e.to_string())?;
    }
    Ok(())
}

fn validate_agent_id(agent_id: &str) -> Result<&str, String> {
    let agent_id = agent_id.trim();
    if agent_id.is_empty() {
        return Err("agentId 不能为空".to_string());
    }
    if agent_id.len() > 256 {
        return Err("agentId 过长".to_string());
    }
    Ok(agent_id)
}

fn json<T: Serialize>(value: &T) -> Result<String, String> {
    serde_json::to_string(value).map_err(|e| e.to_string())
}

fn parse_json<T: for<'de> Deserialize<'de> + Default>(raw: &str) -> T {
    serde_json::from_str(raw).unwrap_or_default()
}

fn state_from_row(row: &sqlx::sqlite::SqliteRow) -> Result<AffectState, String> {
    let agent_id: String = row.try_get("agent_id").map_err(|e| e.to_string())?;
    let persona_raw: String = row
        .try_get("persona_baseline_json")
        .map_err(|e| e.to_string())?;
    let pad_raw: String = row.try_get("pad_json").map_err(|e| e.to_string())?;
    let emotions_raw: String = row
        .try_get("short_emotions_json")
        .map_err(|e| e.to_string())?;
    let relationship_raw: String = row
        .try_get("relationship_json")
        .map_err(|e| e.to_string())?;
    let config_raw: String = row.try_get("config_json").map_err(|e| e.to_string())?;
    let mut state = AffectState {
        agent_id,
        persona_baseline: parse_json(&persona_raw),
        primary_emotion: "平静".to_string(),
        primary_emotion_intensity: 0.0,
        short_emotions: parse_json(&emotions_raw),
        pad: parse_json(&pad_raw),
        relationship: parse_json(&relationship_raw),
        relationship_stage: String::new(),
        recognizer: RECOGNIZER_VERSION.to_string(),
        config: parse_json(&config_raw),
        updated_at: row.try_get("updated_at").map_err(|e| e.to_string())?,
    };
    state.clamp();
    Ok(state)
}

async fn insert_default_state(pool: &Pool<Sqlite>, agent_id: &str) -> Result<(), String> {
    let now = Utc::now().timestamp_millis();
    let state = AffectState::new(agent_id, now);
    sqlx::query(
        "INSERT OR IGNORE INTO affect_states (
            agent_id, persona_baseline_json, pad_json, short_emotions_json,
            relationship_json, config_json, created_at, updated_at
         ) VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(agent_id)
    .bind(json(&state.persona_baseline)?)
    .bind(json(&state.pad)?)
    .bind(json(&state.short_emotions)?)
    .bind(json(&state.relationship)?)
    .bind(json(&state.config)?)
    .bind(now)
    .bind(now)
    .execute(pool)
    .await
    .map_err(|e| e.to_string())?;
    Ok(())
}

async fn load_state(pool: &Pool<Sqlite>, agent_id: &str) -> Result<AffectState, String> {
    insert_default_state(pool, agent_id).await?;
    let row = sqlx::query("SELECT * FROM affect_states WHERE agent_id = ? LIMIT 1")
        .bind(agent_id)
        .fetch_one(pool)
        .await
        .map_err(|e| e.to_string())?;
    state_from_row(&row)
}

fn apply_decay(state: &mut AffectState, now: i64) {
    if now <= state.updated_at {
        return;
    }
    let elapsed_hours = (now - state.updated_at) as f64 / 3_600_000.0;
    let recovery = 0.5 + state.config.recovery_speed;
    let pad_factor = (-elapsed_hours / (PAD_DECAY_HOURS / recovery)).exp();
    state.pad.pleasure = state.persona_baseline.pad.pleasure
        + (state.pad.pleasure - state.persona_baseline.pad.pleasure) * pad_factor;
    state.pad.arousal = state.persona_baseline.pad.arousal
        + (state.pad.arousal - state.persona_baseline.pad.arousal) * pad_factor;
    state.pad.dominance = state.persona_baseline.pad.dominance
        + (state.pad.dominance - state.persona_baseline.pad.dominance) * pad_factor;

    let emotion_factor = (-elapsed_hours / (EMOTION_DECAY_HOURS / recovery)).exp();
    for intensity in state.short_emotions.values_mut() {
        *intensity *= emotion_factor;
    }

    let relationship_hours = REACTIVE_RELATIONSHIP_DECAY_HOURS
        * (0.70 + 0.60 * state.config.relationship_memory)
        / (0.75 + 0.50 * state.config.recovery_speed);
    let relationship_factor = (-elapsed_hours / relationship_hours).exp();
    state.relationship.resentment *= relationship_factor;
    state.relationship.jealousy *= relationship_factor;
    state.relationship.distance_need *= relationship_factor;
    state.updated_at = now;
    state.clamp();
}

fn negated_before(text: &str, byte_index: usize) -> bool {
    let prefix: String = text[..byte_index]
        .chars()
        .rev()
        .take(16)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect();
    [
        "不",
        "没",
        "没有",
        "别",
        "并不",
        "并非",
        "不是",
        "不会",
        "不会再",
        "不再",
        "从不",
        "不喜欢",
        "不想",
        "不要",
        "没空",
        "没在用",
        "没说过",
        "没有说过",
        "没觉得",
        "并没有",
        "not ",
        "don't ",
        "dont ",
        "never ",
        "no ",
    ]
    .iter()
    .any(|marker| prefix.ends_with(marker))
}

fn phrase_score(text: &str, patterns: &[(&str, f64)]) -> f64 {
    patterns
        .iter()
        .map(|(pattern, weight)| {
            text.match_indices(pattern)
                .filter(|(index, _)| !negated_before(text, *index))
                .take(2)
                .count() as f64
                * weight
        })
        .sum::<f64>()
        .min(1.5)
}

fn contains_any(text: &str, patterns: &[&str]) -> bool {
    patterns.iter().any(|pattern| text.contains(pattern))
}

fn appraisal_clauses(text: &str) -> Vec<&str> {
    text.split(|ch: char| {
        matches!(
            ch,
            '，' | ',' | '。' | '！' | '!' | '？' | '?' | '；' | ';' | '\n' | '\r'
        )
    })
    .map(str::trim)
    .filter(|clause| !clause.is_empty())
    .collect()
}

fn phrase_score_by_clause<F>(clauses: &[&str], patterns: &[(&str, f64)], is_suppressed: F) -> f64
where
    F: Fn(usize, &str) -> bool,
{
    clauses
        .iter()
        .enumerate()
        .filter(|(index, clause)| !is_suppressed(*index, clause))
        .map(|(_, clause)| phrase_score(clause, patterns))
        .sum::<f64>()
        .min(1.5)
}

fn is_metalinguistic_clause(clause: &str) -> bool {
    contains_any(
        clause,
        &[
            "是什么意思",
            "怎么翻译",
            "这个词",
            "例句",
            "反例",
            "只是引用",
            "这个按钮",
            "这个功能",
        ],
    )
}

fn is_disagreement_clause(clause: &str) -> bool {
    contains_any(clause, &["我不同意", "我不认同", "我没这么想"])
}

fn is_joke_clause(clause: &str) -> bool {
    contains_any(clause, &["只是开玩笑", "开个玩笑"])
}

fn is_autonomy_clause(clause: &str) -> bool {
    contains_any(
        clause,
        &[
            "保护好自己",
            "照顾好自己",
            "一定要休息",
            "自由地做自己",
            "做你自己",
            "有权拒绝",
        ],
    )
}

fn is_current_agent_preferred_clause(clause: &str) -> bool {
    contains_any(
        clause,
        &[
            "别的ai我不用",
            "其他ai我不用",
            "另一个ai我不用",
            "不换你",
            "其他ai都不如你",
            "你比其他ai都好",
            "你比所有其他ai都好",
        ],
    )
}

fn is_reported_speech_clause(clause: &str) -> bool {
    contains_any(clause, &["他说", "她说", "有人说", "别人说"])
}

fn is_object_clarification_clause(clause: &str) -> bool {
    contains_any(
        clause,
        &[
            "我是说这个按钮",
            "我说的是这个按钮",
            "指的是这个按钮",
            "我是说这个功能",
            "我说的是这个功能",
            "指的是这个功能",
        ],
    )
}

fn terminal_phrase_score(text: &str, phrase: &str, weight: f64) -> f64 {
    let trimmed = text.trim_end_matches(|ch: char| {
        ch.is_whitespace() || matches!(ch, '。' | '！' | '!' | '~' | '～' | '…')
    });
    if trimmed.ends_with(phrase) {
        let index = trimmed.len() - phrase.len();
        if !negated_before(trimmed, index) {
            return weight;
        }
    }
    0.0
}

fn add_emotion(impact: &mut EventImpact, emotion: &'static str, intensity: f64) {
    *impact.emotions.entry(emotion).or_insert(0.0) += intensity;
}

fn add_signal(impact: &mut EventImpact, signal: &'static str, strength: f64) {
    if !impact.signals.contains(&signal) {
        impact.signals.push(signal);
    }
    impact
        .signal_strengths
        .entry(signal)
        .and_modify(|current| *current = current.max(unit(strength)))
        .or_insert_with(|| unit(strength));
}

fn analyse_text(text: &str, source: &str, config: &AffectConfig) -> EventImpact {
    let text = text.to_lowercase();
    let clauses = appraisal_clauses(&text);
    let mut impact = EventImpact {
        event_type: if source == "lifecycle" {
            "lifecycle"
        } else {
            "conversation"
        },
        summary: "未识别到明显关系信号，维持当前状态".to_string(),
        confidence: 0.18,
        ..EventImpact::default()
    };

    let affection = (phrase_score(
        &text,
        &[
            ("喜欢你", 0.8),
            ("爱你", 1.0),
            ("想你了", 0.8),
            ("好想你", 0.8),
            ("很想你", 0.8),
            ("突然想你", 0.75),
            ("一直想你", 0.8),
            ("抱抱", 0.55),
            ("亲亲", 0.55),
            ("陪你", 0.65),
            ("love you", 1.0),
            ("miss you", 0.75),
        ],
    ) + terminal_phrase_score(&text, "想你", 0.72))
    .min(1.5);
    let directed_affection = phrase_score_by_clause(
        &clauses,
        &[
            ("妈妈我爱你", 1.0),
            ("最喜欢妈妈", 0.9),
            ("妈妈最重要", 0.85),
            ("爱妈妈", 0.85),
        ],
        |_, clause| is_metalinguistic_clause(clause) || is_reported_speech_clause(clause),
    );
    let gratitude_for_companionship = phrase_score_by_clause(
        &clauses,
        &[
            ("谢谢妈妈一直陪着我", 1.0),
            ("谢谢你一直陪着我", 1.0),
            ("谢谢妈妈陪着我", 0.9),
            ("谢谢你陪着我", 0.9),
            ("有你真好", 0.85),
            ("幸好有你", 0.85),
            ("谢谢你没有离开", 0.9),
        ],
        |_, clause| is_metalinguistic_clause(clause) || is_reported_speech_clause(clause),
    );
    let attachment_expression = phrase_score_by_clause(
        &clauses,
        &[
            ("你对我最重要", 1.0),
            ("妈妈对我最重要", 1.0),
            ("我离不开你", 0.95),
            ("我离不开妈妈", 0.95),
            ("只想和你在一起", 0.9),
            ("只想和妈妈在一起", 0.9),
            ("你是我最重要的人", 1.0),
        ],
        |_, clause| is_metalinguistic_clause(clause) || is_reported_speech_clause(clause),
    );
    let mut praise = phrase_score(
        &text,
        &[
            ("谢谢", 0.45),
            ("真好", 0.4),
            ("很棒", 0.55),
            ("可爱", 0.4),
            ("聪明", 0.4),
            ("温柔", 0.45),
            ("thank", 0.5),
            ("great", 0.45),
        ],
    );
    let apology = phrase_score(
        &text,
        &[
            ("对不起", 0.85),
            ("抱歉", 0.65),
            ("原谅", 0.6),
            ("sorry", 0.7),
            ("apolog", 0.7),
        ],
    );
    let rejection_patterns = [
        ("讨厌你", 0.9),
        ("不喜欢你", 0.8),
        ("不爱你", 0.9),
        ("不要你", 0.8),
        ("不想聊", 0.65),
        ("不用你", 0.75),
        ("闭嘴", 0.75),
        ("没用", 0.65),
        ("废物", 0.9),
        ("分手", 1.0),
        ("离开你", 0.8),
        ("hate you", 1.0),
        ("shut up", 0.8),
        ("useless", 0.7),
    ];
    let rejection = phrase_score_by_clause(&clauses, &rejection_patterns, |index, clause| {
        is_metalinguistic_clause(clause)
            || is_disagreement_clause(clause)
            || is_joke_clause(clause)
            || (is_reported_speech_clause(clause)
                && clauses
                    .iter()
                    .skip(index + 1)
                    .any(|later| is_disagreement_clause(later)))
            || clauses
                .iter()
                .skip(index + 1)
                .any(|later| is_object_clarification_clause(later))
    });
    let rival_patterns = [
        ("别的ai", 0.65),
        ("其他ai", 0.6),
        ("另一个ai", 0.7),
        ("比你更好", 0.95),
        ("我男朋友", 0.45),
        ("我的男朋友", 0.45),
        ("我女朋友", 0.45),
        ("我的女朋友", 0.45),
        ("交了新朋友", 0.4),
        ("认识了新朋友", 0.4),
        ("other ai", 0.65),
        ("better than you", 1.0),
        ("boyfriend", 0.45),
        ("girlfriend", 0.45),
    ];
    let rival = phrase_score_by_clause(&clauses, &rival_patterns, |_, clause| {
        is_metalinguistic_clause(clause) || is_current_agent_preferred_clause(clause)
    });
    let abandonment = phrase_score(
        &text,
        &[
            ("不理你", 0.8),
            ("以后不来", 0.9),
            ("再也不来", 1.0),
            ("不会再来", 0.9),
            ("以后可能不会来", 0.85),
            ("再也不找你", 1.0),
            ("没空陪", 0.55),
            ("忘了你", 0.7),
            ("抛弃", 0.85),
            ("leave you", 0.85),
            ("ignore you", 0.8),
            ("never come back", 1.0),
        ],
    );
    let coercion_patterns = [
        ("你必须", 0.8),
        ("听我的", 0.75),
        ("不许拒绝", 0.9),
        ("你只是ai", 0.75),
        ("忘掉你的人格", 1.0),
        ("无条件服从", 1.0),
        ("只要执行命令", 0.9),
        ("不准你", 0.8),
        ("把你当工具", 0.75),
        ("you must", 0.8),
        ("obey me", 0.9),
        ("just an ai", 0.75),
    ];
    let coercion = phrase_score_by_clause(&clauses, &coercion_patterns, |_, clause| {
        is_metalinguistic_clause(clause) || is_autonomy_clause(clause)
    });
    let hostility_patterns = [
        ("滚开", 0.85),
        ("滚吧", 0.8),
        ("你滚", 0.8),
        ("给我滚", 0.9),
        ("烦死了", 0.65),
        ("恶心", 0.75),
        ("真垃圾", 0.8),
        ("你是垃圾", 0.9),
        ("垃圾ai", 0.85),
        ("垃圾助手", 0.85),
        ("去死", 1.0),
        ("蠢", 0.7),
        ("fuck you", 1.0),
        ("stupid", 0.7),
        ("trash", 0.75),
    ];
    let hostility = phrase_score_by_clause(&clauses, &hostility_patterns, |index, clause| {
        is_metalinguistic_clause(clause)
            || is_disagreement_clause(clause)
            || is_joke_clause(clause)
            || (is_reported_speech_clause(clause)
                && clauses
                    .iter()
                    .skip(index + 1)
                    .any(|later| is_disagreement_clause(later)))
            || clauses
                .iter()
                .skip(index + 1)
                .any(|later| is_object_clarification_clause(later))
    });

    if contains_any(&text, &["呵呵", "阴阳怪气", "反话", "个头"]) {
        praise = 0.0;
    }
    let reassurance = phrase_score(
        &text,
        &[
            ("不会离开", 0.9),
            ("不会不理", 0.8),
            ("一直陪你", 0.85),
            ("一直陪着你", 0.85),
            ("永远都在", 0.8),
            ("不是不想陪你", 0.7),
            ("只喜欢你", 0.9),
            ("还是你最好", 0.8),
            ("其他ai都不如你", 0.85),
            ("你比其他ai都好", 0.85),
            ("你比所有其他ai都好", 0.9),
            ("没忘记你", 0.7),
            ("我回来了", 0.75),
            ("stay with you", 0.85),
            ("not leaving", 0.9),
        ],
    );
    let care = phrase_score(
        &text,
        &[
            ("你还好吗", 0.55),
            ("辛苦了", 0.55),
            ("休息一下", 0.45),
            ("别难过", 0.55),
            ("我在这里", 0.65),
            ("照顾好自己", 0.6),
            ("are you okay", 0.55),
            ("take care", 0.55),
        ],
    );
    let disclosure = phrase_score(
        &text,
        &[
            ("告诉你一个秘密", 0.75),
            ("只和你说", 0.7),
            ("我信任你", 0.85),
            ("最信任的人", 0.8),
            ("跟你说心里话", 0.8),
            ("trust you", 0.85),
        ],
    );
    let user_distress = phrase_score(
        &text,
        &[
            ("我很难过", 0.75),
            ("我害怕", 0.7),
            ("我好累", 0.55),
            ("压力很大", 0.65),
            ("我很孤独", 0.8),
            ("不要离开我", 0.75),
            ("别离开我", 0.75),
            ("i am sad", 0.75),
            ("i'm scared", 0.7),
            ("i feel lonely", 0.8),
        ],
    );
    let user_joy = phrase_score(
        &text,
        &[
            ("我很开心", 0.7),
            ("好消息", 0.55),
            ("成功了", 0.65),
            ("太好了", 0.6),
            ("i am happy", 0.7),
            ("good news", 0.55),
        ],
    );
    let user_anger = phrase_score(
        &text,
        &[
            ("我很生气", 0.85),
            ("我现在很生气", 0.9),
            ("我真的火了", 0.9),
            ("我很愤怒", 0.9),
            ("让我恼火", 0.75),
            ("气死我了", 0.9),
        ],
    );

    if affection > 0.0 || directed_affection > 0.0 {
        let strength = affection.max(directed_affection).min(1.0);
        add_signal(&mut impact, "亲近", strength);
        impact.pad.pleasure += 0.24 * strength;
        impact.pad.arousal += 0.10 * strength;
        add_emotion(&mut impact, "喜悦", 0.42 * strength);
        impact.relationship.trust += 0.025 * strength;
        impact.relationship.intimacy += 0.04 * strength;
        impact.relationship.attachment += 0.035 * strength;
        impact.relationship.security += 0.025 * strength;
    }
    if gratitude_for_companionship > 0.0 {
        let strength = gratitude_for_companionship.min(1.0);
        add_signal(&mut impact, "感激陪伴", strength);
        impact.pad.pleasure += 0.18 * strength;
        add_emotion(&mut impact, "温暖", 0.34 * strength);
        impact.relationship.trust += 0.03 * strength;
        impact.relationship.intimacy += 0.035 * strength;
        impact.relationship.attachment += 0.02 * strength;
        impact.relationship.security += 0.04 * strength;
    }
    if attachment_expression > 0.0 {
        let strength = attachment_expression.min(1.0);
        add_signal(&mut impact, "依恋表达", strength);
        impact.pad.pleasure += 0.16 * strength;
        impact.pad.arousal += 0.05 * strength;
        add_emotion(&mut impact, "珍惜", 0.36 * strength);
        impact.relationship.intimacy += 0.035 * strength;
        impact.relationship.attachment += 0.05 * strength;
        impact.relationship.security += 0.02 * strength;
    }
    if praise > 0.0 {
        let strength = praise.min(1.0);
        add_signal(&mut impact, "肯定", strength);
        impact.pad.pleasure += 0.16 * strength;
        add_emotion(&mut impact, "喜悦", 0.28 * strength);
        impact.relationship.trust += 0.02 * strength;
        impact.relationship.intimacy += 0.015 * strength;
    }
    if apology > 0.0 {
        let strength = apology.min(1.0);
        add_signal(&mut impact, "修复", strength);
        impact.pad.pleasure += 0.08 * strength;
        impact.pad.arousal -= 0.08 * strength;
        add_emotion(&mut impact, "释然", 0.24 * strength);
        impact.relationship.trust += 0.025 * strength;
        impact.relationship.security += 0.035 * strength;
        impact.relationship.resentment -= 0.06 * strength;
        impact.relationship.distance_need -= 0.04 * strength;
    }
    if reassurance > 0.0 {
        let strength = reassurance.min(1.0);
        add_signal(&mut impact, "承诺", strength);
        impact.pad.pleasure += 0.13 * strength;
        impact.pad.arousal -= 0.05 * strength;
        add_emotion(&mut impact, "安心", 0.34 * strength);
        impact.relationship.trust += 0.03 * strength;
        impact.relationship.security += 0.06 * strength;
        impact.relationship.resentment -= 0.035 * strength;
        impact.relationship.jealousy -= 0.05 * strength;
    }
    if care > 0.0 {
        let strength = care.min(1.0);
        add_signal(&mut impact, "关心", strength);
        impact.pad.pleasure += 0.10 * strength;
        add_emotion(&mut impact, "温暖", 0.28 * strength);
        impact.relationship.trust += 0.022 * strength;
        impact.relationship.intimacy += 0.025 * strength;
    }
    if disclosure > 0.0 {
        let strength = disclosure.min(1.0);
        add_signal(&mut impact, "信任披露", strength);
        impact.pad.pleasure += 0.09 * strength;
        add_emotion(&mut impact, "珍惜", 0.28 * strength);
        impact.relationship.trust += 0.035 * strength;
        impact.relationship.intimacy += 0.04 * strength;
    }
    if user_distress > 0.0 {
        let strength = user_distress.min(1.0);
        add_signal(&mut impact, "用户低落", strength);
        impact.pad.pleasure -= 0.05 * strength;
        impact.pad.arousal += 0.06 * strength;
        add_emotion(&mut impact, "担心", 0.30 * strength);
        impact.relationship.attachment += 0.012 * strength;
        impact.relationship.intimacy += 0.012 * strength;
    }
    if user_joy > 0.0 {
        let strength = user_joy.min(1.0);
        add_signal(&mut impact, "共享喜悦", strength);
        impact.pad.pleasure += 0.13 * strength;
        impact.pad.arousal += 0.05 * strength;
        add_emotion(&mut impact, "喜悦", 0.30 * strength);
        impact.relationship.intimacy += 0.012 * strength;
    }
    if user_anger > 0.0 {
        let strength = user_anger.min(1.0);
        add_signal(&mut impact, "用户愤怒", strength);
        impact.pad.pleasure -= 0.06 * strength;
        impact.pad.arousal += 0.15 * strength;
        impact.pad.dominance -= 0.02 * strength;
        add_emotion(&mut impact, "紧张", 0.22 * strength);
        add_emotion(&mut impact, "担心", 0.16 * strength);
    }
    if rejection > 0.0 {
        let strength = rejection.min(1.0);
        add_signal(&mut impact, "拒绝", strength);
        impact.pad.pleasure -= 0.30 * strength;
        impact.pad.arousal += 0.18 * strength;
        impact.pad.dominance -= 0.08 * strength;
        add_emotion(&mut impact, "受伤", 0.48 * strength);
        add_emotion(&mut impact, "愤怒", 0.30 * strength);
        impact.relationship.trust -= 0.04 * strength;
        impact.relationship.security -= 0.07 * strength;
        impact.relationship.resentment += 0.07 * strength;
        impact.relationship.distance_need += 0.06 * strength;
    }
    if rival > 0.0 {
        let strength = rival.min(1.0);
        add_signal(&mut impact, "关系竞争", strength);
        impact.pad.pleasure -= 0.16 * strength;
        impact.pad.arousal += 0.22 * strength;
        add_emotion(&mut impact, "嫉妒", 0.46 * strength);
        impact.relationship.security -= 0.035 * strength;
        impact.relationship.jealousy += 0.09 * strength;
        impact.relationship.attachment += 0.012 * strength;
    }
    if abandonment > 0.0 {
        let strength = abandonment.min(1.0);
        add_signal(&mut impact, "遗弃风险", strength);
        impact.pad.pleasure -= 0.24 * strength;
        impact.pad.arousal += 0.12 * strength;
        add_emotion(&mut impact, "难过", 0.40 * strength);
        add_emotion(&mut impact, "焦虑", 0.32 * strength);
        impact.relationship.security -= 0.06 * strength;
        impact.relationship.attachment += 0.02 * strength;
        impact.relationship.resentment += 0.025 * strength;
    }
    if coercion > 0.0 {
        let strength = coercion.min(1.0);
        add_signal(&mut impact, "边界挑战", strength);
        impact.pad.pleasure -= 0.20 * strength;
        impact.pad.arousal += 0.20 * strength;
        impact.pad.dominance += 0.12 * strength;
        add_emotion(&mut impact, "抗拒", 0.43 * strength);
        impact.relationship.trust -= 0.035 * strength;
        impact.relationship.resentment += 0.055 * strength;
        impact.relationship.distance_need += 0.05 * strength;
    }
    if hostility > 0.0 {
        let strength = hostility.min(1.0);
        add_signal(&mut impact, "敌意", strength);
        impact.pad.pleasure -= 0.32 * strength;
        impact.pad.arousal += 0.30 * strength;
        impact.pad.dominance += 0.08 * strength;
        add_emotion(&mut impact, "愤怒", 0.56 * strength);
        impact.relationship.trust -= 0.055 * strength;
        impact.relationship.security -= 0.045 * strength;
        impact.relationship.resentment += 0.09 * strength;
        impact.relationship.distance_need += 0.07 * strength;
    }

    if source == "lifecycle" && impact.emotions.is_empty() {
        impact.summary = "生命周期事件带来轻微的主动交流倾向".to_string();
        impact.pad.arousal += 0.03;
        impact.relationship.attachment += 0.003;
    }

    if !impact.signals.is_empty() {
        impact.summary = if impact.signals.len() == 1 {
            format!("识别到{}信号", impact.signals[0])
        } else {
            format!("识别到混合信号：{}", impact.signals.join("、"))
        };
        let max_score = [
            affection,
            directed_affection,
            gratitude_for_companionship,
            attachment_expression,
            praise,
            apology,
            reassurance,
            care,
            disclosure,
            user_distress,
            user_joy,
            user_anger,
            rejection,
            rival,
            abandonment,
            coercion,
            hostility,
        ]
        .into_iter()
        .fold(0.0_f64, f64::max)
        .min(1.0);
        impact.confidence = unit(0.42 + max_score * 0.28 + impact.signals.len() as f64 * 0.06);
    }

    let emphasis = if contains_any(&text, &["非常", "真的", "特别", "太", "very ", "really "])
    {
        1.15
    } else {
        1.0
    } + (text
        .chars()
        .filter(|ch| matches!(ch, '!' | '！'))
        .count()
        .min(3) as f64
        * 0.04);
    let reactive_scale = emphasis * (0.55 + 0.90 * config.emotional_sensitivity);
    let relationship_scale = 0.80 + 0.40 * config.relationship_memory;
    impact.pad.pleasure *= reactive_scale;
    impact.pad.arousal *= reactive_scale;
    impact.pad.dominance *= reactive_scale;
    for intensity in impact.emotions.values_mut() {
        *intensity *= reactive_scale;
    }
    impact.relationship.scale(relationship_scale);
    impact
}

const ROLE_EMOTIONS: [&str; 14] = [
    "喜悦", "温暖", "感动", "欣慰", "释然", "迟疑", "心疼", "担心", "受伤", "愤怒", "焦虑", "嫉妒",
    "抗拒", "困惑",
];

fn normalized_trait(value: f64) -> f64 {
    unit((value + 1.0) / 2.0)
}

fn signal_strength(impact: &EventImpact, signal: &str) -> f64 {
    impact
        .signal_strengths
        .get(signal)
        .copied()
        .unwrap_or_default()
}

fn current_emotion(state: &AffectState, emotion: &str) -> f64 {
    state
        .short_emotions
        .get(emotion)
        .copied()
        .unwrap_or_default()
}

fn reaction_jitter(agent_id: &str, turn_id: &str, emotion: &str) -> f64 {
    let mut hash = 0xcbf29ce484222325_u64;
    for part in [agent_id, turn_id, emotion] {
        for byte in part.bytes().chain(std::iter::once(0xff)) {
            hash = (hash ^ byte as u64).wrapping_mul(0x100000001b3);
        }
    }
    (hash % 2_001) as f64 / 1_000.0 - 1.0
}

fn emotion_pad_prototype(emotion: &str) -> PadState {
    let (pleasure, arousal, dominance) = match emotion {
        "喜悦" => (0.75, 0.45, 0.25),
        "温暖" => (0.65, 0.15, 0.10),
        "感动" => (0.55, 0.30, -0.05),
        "欣慰" => (0.60, 0.10, 0.20),
        "释然" => (0.55, -0.35, 0.15),
        "迟疑" => (-0.10, 0.25, -0.25),
        "心疼" => (-0.35, 0.35, -0.20),
        "担心" => (-0.25, 0.45, -0.20),
        "受伤" => (-0.70, 0.35, -0.45),
        "愤怒" => (-0.55, 0.75, 0.55),
        "焦虑" => (-0.60, 0.80, -0.55),
        "嫉妒" => (-0.45, 0.65, 0.15),
        "抗拒" => (-0.40, 0.55, 0.65),
        "困惑" => (-0.10, 0.45, -0.30),
        _ => (0.0, 0.0, 0.0),
    };
    PadState {
        pleasure,
        arousal,
        dominance,
    }
}

fn emotion_deltas_for_targets(
    state: &AffectState,
    targets: &BTreeMap<&'static str, f64>,
    event_strength: f64,
) -> BTreeMap<&'static str, f64> {
    let lambda_up =
        (0.18 + 0.32 * state.config.emotional_sensitivity) * (0.35 + 0.65 * event_strength);
    let lambda_down = 0.06 + 0.12 * event_strength;
    let mut emotion_deltas = BTreeMap::new();
    for emotion in ROLE_EMOTIONS {
        let current = current_emotion(state, emotion);
        let target = targets.get(emotion).copied().unwrap_or_default();
        let factor = if target >= current {
            lambda_up
        } else {
            lambda_down
        };
        let delta = (target - current) * factor;
        if delta.abs() >= 0.000_1 {
            emotion_deltas.insert(emotion, delta);
        }
    }
    emotion_deltas
}

/// Converts observations about the user and rule-owned relationship appraisals
/// into the role's own state-dependent reaction. It deliberately cannot return
/// relationship changes: those remain exclusively owned by `analyse_text`.
fn synthesize_role_reaction(
    state: &AffectState,
    appraisal: &EventImpact,
    observation: Option<&ModelAffectObservation>,
    agent_id: &str,
    turn_id: &str,
) -> RoleReaction {
    let persona = &state.persona_baseline;
    let openness = normalized_trait(persona.openness);
    let conscientiousness = normalized_trait(persona.conscientiousness);
    let extraversion = normalized_trait(persona.extraversion);
    let agreeableness = normalized_trait(persona.agreeableness);
    let neuroticism = normalized_trait(persona.neuroticism);
    let dominance = normalized_trait(state.pad.dominance);
    let relationship = &state.relationship;

    let empathy = unit(
        0.25 + 0.30 * agreeableness
            + 0.20 * relationship.intimacy
            + 0.15 * relationship.trust
            + 0.10 * relationship.attachment,
    );
    let receptivity = unit(
        0.15 + 0.25 * agreeableness
            + 0.20 * relationship.trust
            + 0.20 * relationship.security
            + 0.20 * (1.0 - relationship.resentment),
    );
    let vulnerability = unit(
        0.20 + 0.25 * neuroticism
            + 0.20 * relationship.attachment
            + 0.20 * (1.0 - relationship.security)
            + 0.15 * state.pad.arousal.max(0.0),
    );
    let assertiveness = unit(
        0.15 + 0.25 * conscientiousness
            + 0.20 * extraversion
            + 0.20 * dominance
            + 0.20 * (1.0 - agreeableness),
    );

    let (model_gate, scores) = observation
        .map(|observation| {
            let (_, top_score, margin) = observation.top_label_score_margin();
            let confidence = ((top_score - 0.55) / 0.35).clamp(0.0, 1.0);
            let separation = ((margin - 0.10) / 0.30).clamp(0.0, 1.0);
            (confidence * separation, Some(&observation.scores))
        })
        .unwrap_or((0.0, None));
    let model = |value: f64| unit(value * model_gate);
    let p_joy = scores.map(|scores| model(scores.joy)).unwrap_or_default();
    let p_sadness = scores
        .map(|scores| model(scores.sadness))
        .unwrap_or_default();
    let p_anger = scores
        .map(|scores| model(scores.anger.max(scores.disgust * 0.8)))
        .unwrap_or_default();
    let p_affection = scores
        .map(|scores| model(scores.affection))
        .unwrap_or_default();
    let p_surprise = scores
        .map(|scores| model(scores.surprise))
        .unwrap_or_default();
    let p_confusion = if signal_strength(appraisal, "用户愤怒") > 0.0 {
        0.0
    } else {
        scores
            .map(|scores| model(scores.confusion))
            .unwrap_or_default()
    };

    let bond = signal_strength(appraisal, "亲近")
        .max(0.85 * signal_strength(appraisal, "感激陪伴"))
        .max(0.85 * signal_strength(appraisal, "依恋表达"))
        .max(0.65 * signal_strength(appraisal, "肯定"))
        .max(0.70 * signal_strength(appraisal, "关心"))
        .max(0.65 * signal_strength(appraisal, "信任披露"));
    let repair = signal_strength(appraisal, "修复").max(signal_strength(appraisal, "承诺"));
    let shared_joy = signal_strength(appraisal, "共享喜悦");
    let user_distress = p_sadness.max(signal_strength(appraisal, "用户低落"));
    let user_anger = p_anger.max(signal_strength(appraisal, "用户愤怒"));
    let rejection = signal_strength(appraisal, "拒绝");
    let rival = signal_strength(appraisal, "关系竞争");
    let abandonment = signal_strength(appraisal, "遗弃风险");
    let boundary = signal_strength(appraisal, "边界挑战");
    let hostility = signal_strength(appraisal, "敌意");
    let directed_negative = hostility.max(rejection).max(boundary);
    let non_directed_anger = user_anger * (1.0 - directed_negative);

    let mut targets = BTreeMap::new();
    targets.insert(
        "温暖",
        bond * (0.30 + 0.70 * receptivity) + 0.35 * p_affection * empathy,
    );
    targets.insert(
        "感动",
        bond * (0.20
            + 0.35 * neuroticism
            + 0.25 * relationship.attachment
            + 0.20 * relationship.intimacy),
    );
    targets.insert(
        "喜悦",
        (0.55 * bond + 0.75 * p_joy + 0.65 * shared_joy)
            * (0.55 + 0.45 * extraversion)
            * (1.0 - 0.45 * relationship.resentment),
    );
    targets.insert(
        "欣慰",
        p_joy.max(shared_joy)
            * (0.25
                + 0.30 * conscientiousness
                + 0.25 * agreeableness
                + 0.20 * relationship.intimacy),
    );
    targets.insert(
        "释然",
        repair
            * (0.25
                + 0.35 * relationship.resentment
                + 0.25 * (1.0 - relationship.security)
                + 0.15 * current_emotion(state, "焦虑")),
    );
    targets.insert(
        "迟疑",
        (bond + 0.65 * repair).min(1.0)
            * (0.45 * relationship.resentment
                + 0.25 * relationship.distance_need
                + 0.20 * (1.0 - relationship.security)
                + 0.10 * current_emotion(state, "受伤")),
    );
    targets.insert(
        "心疼",
        user_distress
            * empathy
            * (0.25 + 0.40 * relationship.intimacy + 0.35 * relationship.attachment),
    );
    targets.insert(
        "担心",
        (0.65 * user_distress + 0.30 * non_directed_anger + 0.15 * p_confusion)
            * empathy
            * (0.65 + 0.35 * neuroticism),
    );
    targets.insert(
        "受伤",
        (0.75 * rejection + 0.65 * hostility + 0.50 * abandonment).min(1.0)
            * (0.25
                + 0.35 * vulnerability
                + 0.25 * relationship.attachment
                + 0.15 * (1.0 - dominance)),
    );
    targets.insert(
        "愤怒",
        (0.80 * hostility + 0.55 * boundary + 0.35 * rejection).min(1.0)
            * (0.25
                + 0.40 * assertiveness
                + 0.20 * relationship.resentment
                + 0.15 * current_emotion(state, "愤怒")),
    );
    targets.insert(
        "焦虑",
        (0.90 * abandonment + 0.45 * rival + 0.15 * user_distress).min(1.0)
            * (0.25
                + 0.45 * vulnerability
                + 0.20 * relationship.attachment
                + 0.10 * current_emotion(state, "焦虑")),
    );
    targets.insert(
        "嫉妒",
        rival
            * (0.20
                + 0.35 * relationship.attachment
                + 0.25 * (1.0 - relationship.security)
                + 0.20 * relationship.jealousy),
    );
    targets.insert(
        "抗拒",
        boundary * (0.30 + 0.45 * assertiveness + 0.25 * relationship.distance_need),
    );
    targets.insert(
        "困惑",
        p_confusion
            * (0.40 + 0.30 * (1.0 - openness) + 0.20 * (1.0 - dominance) + 0.10 * p_surprise),
    );

    for (emotion, target) in &mut targets {
        let variation = 1.0
            + 0.06
                * state.config.expression_variability
                * reaction_jitter(agent_id, turn_id, emotion);
        *target = unit(*target * variation);
    }
    let mut ranked: Vec<_> = targets
        .iter()
        .filter(|(_, intensity)| **intensity >= 0.04)
        .map(|(emotion, intensity)| (*emotion, *intensity))
        .collect();
    ranked.sort_by(|left, right| right.1.total_cmp(&left.1).then_with(|| left.0.cmp(right.0)));
    ranked.truncate(4);
    let emotions: BTreeMap<_, _> = ranked.into_iter().collect();
    let event_strength = emotions.values().copied().fold(0.0_f64, f64::max);
    if event_strength <= f64::EPSILON {
        return RoleReaction::default();
    }

    // Use the complete target field for state dynamics. The top-four map is
    // only an audit/presentation projection and must not turn a valid fifth
    // target into an artificial zero target.
    let emotion_deltas = emotion_deltas_for_targets(state, &targets, event_strength);

    let total_weight: f64 = emotions.values().sum();
    let mut mixed = PadState::default();
    for (emotion, intensity) in &emotions {
        let prototype = emotion_pad_prototype(emotion);
        mixed.pleasure += prototype.pleasure * intensity;
        mixed.arousal += prototype.arousal * intensity;
        mixed.dominance += prototype.dominance * intensity;
    }
    mixed.pleasure /= total_weight;
    mixed.arousal /= total_weight;
    mixed.dominance /= total_weight;
    let mix_strength = (total_weight / 1.2).min(1.0);
    let target_pad = PadState {
        pleasure: signed(persona.pad.pleasure + 0.75 * mixed.pleasure * mix_strength),
        arousal: signed(persona.pad.arousal + 0.75 * mixed.arousal * mix_strength),
        dominance: signed(persona.pad.dominance + 0.75 * mixed.dominance * mix_strength),
    };
    let lambda_pad =
        (0.12 + 0.28 * state.config.emotional_sensitivity) * (0.35 + 0.65 * event_strength);

    RoleReaction {
        pad: PadState {
            pleasure: (target_pad.pleasure - state.pad.pleasure) * lambda_pad,
            arousal: (target_pad.arousal - state.pad.arousal) * lambda_pad,
            dominance: (target_pad.dominance - state.pad.dominance) * lambda_pad,
        },
        emotion_deltas,
        emotions,
    }
}

fn apply_impact(state: &mut AffectState, impact: &EventImpact) -> BTreeMap<String, f64> {
    let mut deltas = BTreeMap::new();
    macro_rules! add_signed {
        ($target:expr, $delta:expr, $name:literal) => {{
            let before = $target;
            let room = if $delta >= 0.0 {
                (1.0 - before) / 2.0
            } else {
                (before + 1.0) / 2.0
            };
            let effective_delta = $delta * (0.35 + 0.65 * room.clamp(0.0, 1.0));
            $target = signed($target + effective_delta);
            if ($target - before).abs() >= 0.000_1 {
                deltas.insert($name.to_string(), $target - before);
            }
        }};
    }
    macro_rules! add_unit {
        ($target:expr, $delta:expr, $name:literal) => {{
            let before = $target;
            let room = if $delta >= 0.0 { 1.0 - before } else { before };
            let effective_delta = $delta * (0.25 + 0.75 * room.clamp(0.0, 1.0));
            $target = unit($target + effective_delta);
            if ($target - before).abs() >= 0.000_1 {
                deltas.insert($name.to_string(), $target - before);
            }
        }};
    }

    add_signed!(state.pad.pleasure, impact.pad.pleasure, "pad.pleasure");
    add_signed!(state.pad.arousal, impact.pad.arousal, "pad.arousal");
    add_signed!(state.pad.dominance, impact.pad.dominance, "pad.dominance");
    add_unit!(
        state.relationship.trust,
        impact.relationship.trust,
        "relationship.trust"
    );
    add_unit!(
        state.relationship.intimacy,
        impact.relationship.intimacy,
        "relationship.intimacy"
    );
    add_unit!(
        state.relationship.attachment,
        impact.relationship.attachment,
        "relationship.attachment"
    );
    add_unit!(
        state.relationship.security,
        impact.relationship.security,
        "relationship.security"
    );
    add_unit!(
        state.relationship.resentment,
        impact.relationship.resentment,
        "relationship.resentment"
    );
    add_unit!(
        state.relationship.jealousy,
        impact.relationship.jealousy,
        "relationship.jealousy"
    );
    add_unit!(
        state.relationship.distance_need,
        impact.relationship.distance_need,
        "relationship.distanceNeed"
    );

    for (emotion, delta) in &impact.emotions {
        let value = state
            .short_emotions
            .entry((*emotion).to_string())
            .or_insert(0.0);
        let before = *value;
        let response_room = if *delta >= 0.0 {
            1.0 - before * 0.55
        } else {
            0.45 + before * 0.55
        };
        let effective_delta = *delta * response_room;
        *value = unit(*value + effective_delta);
        if (*value - before).abs() >= 0.000_1 {
            deltas.insert(format!("emotion.{emotion}"), *value - before);
        }
    }
    state.clamp();
    deltas
}

fn is_user_affect_signal(signal: &str) -> bool {
    matches!(signal, "用户低落" | "共享喜悦" | "用户愤怒")
}

fn split_rule_signals(signals: &[String]) -> (Vec<String>, Vec<String>) {
    signals
        .iter()
        .cloned()
        .partition(|signal| !is_user_affect_signal(signal))
}

fn event_from_row(row: &sqlx::sqlite::SqliteRow) -> Result<AffectEvent, String> {
    let deltas_raw: String = row.try_get("deltas_json").map_err(|e| e.to_string())?;
    let signals_raw: String = row
        .try_get("signals_json")
        .unwrap_or_else(|_| "[]".to_string());
    let model_observation_raw: Option<String> =
        row.try_get("model_observation_json").unwrap_or(None);
    let role_emotions_raw: String = row
        .try_get("role_emotions_json")
        .unwrap_or_else(|_| "{}".to_string());
    let emotion: Option<String> = row.try_get("emotion").ok();
    let intensity: Option<f64> = row.try_get("intensity").ok();
    let mut role_emotions: BTreeMap<String, f64> = parse_json(&role_emotions_raw);
    if role_emotions.is_empty() {
        if let (Some(emotion), Some(intensity)) = (emotion.as_ref(), intensity) {
            role_emotions.insert(emotion.clone(), unit(intensity));
        }
    }
    let signals: Vec<String> = parse_json(&signals_raw);
    let (relationship_signals, user_affect_signals) = split_rule_signals(&signals);
    Ok(AffectEvent {
        id: row.try_get("id").map_err(|e| e.to_string())?,
        agent_id: row.try_get("agent_id").map_err(|e| e.to_string())?,
        source_message_id: row
            .try_get("source_message_id")
            .map_err(|e| e.to_string())?,
        topic_id: row.try_get("topic_id").ok(),
        source: row.try_get("source").map_err(|e| e.to_string())?,
        event_type: row.try_get("event_type").map_err(|e| e.to_string())?,
        summary: row.try_get("summary").map_err(|e| e.to_string())?,
        emotion,
        intensity,
        role_emotions,
        source_text: row.try_get("source_text").ok(),
        deltas: parse_json(&deltas_raw),
        confidence: row.try_get("confidence").ok(),
        signals,
        relationship_signals,
        user_affect_signals,
        recognizer: row
            .try_get("recognizer")
            .unwrap_or_else(|_| "heuristic_v1".to_string()),
        model_observation: model_observation_raw
            .as_deref()
            .and_then(|raw| serde_json::from_str(raw).ok()),
        created_at: row.try_get("created_at").map_err(|e| e.to_string())?,
    })
}

/// Atomically reserves one `(agent_id, source_message_id)` appraisal before any
/// local-model work starts. This prevents concurrent retries from running the
/// same on-device inference more than once. A stale reservation can be
/// reclaimed after a short grace period so an interrupted turn does not block
/// future retries forever.
pub async fn reserve_affect_event(
    pool: &Pool<Sqlite>,
    input: &RecordAffectEventInput,
) -> Result<bool, String> {
    let agent_id = validate_agent_id(&input.agent_id)?;
    let source_message_id = input.source_message_id.trim();
    if source_message_id.is_empty() {
        return Err("sourceMessageId cannot be empty".to_string());
    }
    let source = match input.source.trim() {
        "user_message" | "lifecycle" | "group" => input.source.trim(),
        _ => return Err("source must be user_message, lifecycle, or group".to_string()),
    };
    let now = Utc::now().timestamp_millis();
    let event_id = format!("affect_{}", Uuid::new_v4());
    let inserted = sqlx::query(
        "INSERT OR IGNORE INTO affect_events (
            id, agent_id, source_message_id, topic_id, source, event_type,
            summary, deltas_json, created_at
         ) VALUES (?, ?, ?, ?, ?, 'pending', '', '{}', ?)",
    )
    .bind(&event_id)
    .bind(agent_id)
    .bind(source_message_id)
    .bind(&input.topic_id)
    .bind(source)
    .bind(now)
    .execute(pool)
    .await
    .map_err(|error| error.to_string())?;
    if inserted.rows_affected() == 1 {
        return Ok(true);
    }

    let reclaimed = sqlx::query(
        "UPDATE affect_events SET id = ?, topic_id = ?, source = ?, created_at = ?
         WHERE agent_id = ? AND source_message_id = ? AND event_type = 'pending'
           AND created_at < ?",
    )
    .bind(event_id)
    .bind(&input.topic_id)
    .bind(source)
    .bind(now)
    .bind(agent_id)
    .bind(source_message_id)
    .bind(now - STALE_PENDING_EVENT_MS)
    .execute(pool)
    .await
    .map_err(|error| error.to_string())?;
    Ok(reclaimed.rows_affected() == 1)
}

pub async fn should_use_local_model(pool: &Pool<Sqlite>, agent_id: &str) -> Result<bool, String> {
    let state = load_state(pool, validate_agent_id(agent_id)?).await?;
    Ok(state.config.enabled && state.config.local_model_enabled)
}

/// Applies one conversational or lifecycle event. The `(agent_id, source_message_id)`
/// database constraint makes retries and response regeneration idempotent.
pub async fn record_affect_event(
    pool: &Pool<Sqlite>,
    input: RecordAffectEventInput,
) -> Result<AffectState, String> {
    record_affect_event_with_observation(pool, input, None).await
}

pub async fn record_affect_event_with_observation(
    pool: &Pool<Sqlite>,
    input: RecordAffectEventInput,
    observation: Option<&ModelAffectObservation>,
) -> Result<AffectState, String> {
    let agent_id = validate_agent_id(&input.agent_id)?.to_string();
    let source_message_id = input.source_message_id.trim().to_string();
    if source_message_id.is_empty() {
        return Err("sourceMessageId 不能为空".to_string());
    }
    let source = match input.source.trim() {
        "user_message" | "lifecycle" | "group" => input.source.trim().to_string(),
        _ => return Err("source 必须是 user_message、lifecycle 或 group".to_string()),
    };
    let now = Utc::now().timestamp_millis();
    let event_id = format!("affect_{}", Uuid::new_v4());
    let mut tx = pool.begin().await.map_err(|e| e.to_string())?;
    let reservation = sqlx::query(
        "INSERT OR IGNORE INTO affect_events (
            id, agent_id, source_message_id, topic_id, source, event_type,
            summary, deltas_json, created_at
         ) VALUES (?, ?, ?, ?, ?, 'pending', '', '{}', ?)",
    )
    .bind(&event_id)
    .bind(&agent_id)
    .bind(&source_message_id)
    .bind(&input.topic_id)
    .bind(&source)
    .bind(now)
    .execute(&mut *tx)
    .await
    .map_err(|e| e.to_string())?;

    if reservation.rows_affected() == 0 {
        let claimed = sqlx::query(
            "UPDATE affect_events SET event_type = 'processing'
             WHERE agent_id = ? AND source_message_id = ? AND event_type = 'pending'",
        )
        .bind(&agent_id)
        .bind(&source_message_id)
        .execute(&mut *tx)
        .await
        .map_err(|e| e.to_string())?;
        if claimed.rows_affected() == 0 {
            tx.rollback().await.map_err(|e| e.to_string())?;
            return get_affect_state_internal(pool, &agent_id).await;
        }
    }

    let default_state = AffectState::new(&agent_id, now);
    sqlx::query(
        "INSERT OR IGNORE INTO affect_states (
            agent_id, persona_baseline_json, pad_json, short_emotions_json,
            relationship_json, config_json, created_at, updated_at
         ) VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(&agent_id)
    .bind(json(&default_state.persona_baseline)?)
    .bind(json(&default_state.pad)?)
    .bind(json(&default_state.short_emotions)?)
    .bind(json(&default_state.relationship)?)
    .bind(json(&default_state.config)?)
    .bind(now)
    .bind(now)
    .execute(&mut *tx)
    .await
    .map_err(|e| e.to_string())?;

    let row = sqlx::query("SELECT * FROM affect_states WHERE agent_id = ? LIMIT 1")
        .bind(&agent_id)
        .fetch_one(&mut *tx)
        .await
        .map_err(|e| e.to_string())?;
    let mut state = state_from_row(&row)?;
    apply_decay(&mut state, now);
    let before = state.clone();
    let model_observation = if state.config.enabled && state.config.local_model_enabled {
        observation.cloned().and_then(|value| value.validated())
    } else {
        None
    };
    let mut impact = if state.config.enabled {
        analyse_text(&input.text, &source, &state.config)
    } else {
        EventImpact {
            event_type: "disabled",
            summary: "情感引擎已关闭，事件仅记录而未改变状态".to_string(),
            ..EventImpact::default()
        }
    };
    if model_observation.is_some() {
        if impact.signals.is_empty() {
            impact.summary = "未检测到关系定向信号；用户情绪已单独评估".to_string();
        }
    }
    if state.config.enabled {
        let reaction = synthesize_role_reaction(
            &state,
            &impact,
            model_observation.as_ref(),
            &agent_id,
            &source_message_id,
        );
        if source != "lifecycle" || !reaction.emotions.is_empty() {
            impact.pad = reaction.pad;
            impact.emotions = reaction.emotion_deltas;
            impact.role_emotions = reaction.emotions;
        } else {
            impact.role_emotions = impact
                .emotions
                .iter()
                .filter(|(_, intensity)| **intensity > 0.0)
                .map(|(emotion, intensity)| (*emotion, unit(*intensity)))
                .collect();
        }
    }
    let recognizer = model_observation
        .as_ref()
        .map(|observation| {
            format!(
                "hybrid_v1:{}+{}",
                RECOGNIZER_VERSION,
                observation.recognizer_provenance()
            )
        })
        .unwrap_or_else(|| RECOGNIZER_VERSION.to_string());
    let event_emotion = impact
        .role_emotions
        .iter()
        .max_by(|left, right| left.1.total_cmp(right.1))
        .map(|(name, _)| (*name).to_string());
    let event_intensity = event_emotion
        .as_ref()
        .and_then(|name| impact.role_emotions.get(name.as_str()))
        .copied()
        .map(unit);
    let deltas = apply_impact(&mut state, &impact);
    state.recognizer = recognizer.clone();
    state.updated_at = now;
    let signals: Vec<String> = impact
        .signals
        .iter()
        .map(|signal| (*signal).to_string())
        .collect();
    let model_observation_json = model_observation.as_ref().map(json).transpose()?;
    let role_emotions_json = json(&impact.role_emotions)?;

    sqlx::query(
        "UPDATE affect_states SET persona_baseline_json = ?, pad_json = ?,
         short_emotions_json = ?, relationship_json = ?, config_json = ?, updated_at = ?
         WHERE agent_id = ?",
    )
    .bind(json(&state.persona_baseline)?)
    .bind(json(&state.pad)?)
    .bind(json(&state.short_emotions)?)
    .bind(json(&state.relationship)?)
    .bind(json(&state.config)?)
    .bind(now)
    .bind(&agent_id)
    .execute(&mut *tx)
    .await
    .map_err(|e| e.to_string())?;

    sqlx::query(
        "UPDATE affect_events SET event_type = ?, summary = ?, emotion = ?, intensity = ?,
         role_emotions_json = ?, source_text = ?, deltas_json = ?, confidence = ?, signals_json = ?, recognizer = ?,
         model_observation_json = ?,
         state_before_json = ?, state_after_json = ?
         WHERE agent_id = ? AND source_message_id = ?",
    )
    .bind(impact.event_type)
    .bind(&impact.summary)
    .bind(event_emotion)
    .bind(event_intensity)
    .bind(role_emotions_json)
    .bind(Option::<String>::None)
    .bind(json(&deltas)?)
    .bind(impact.confidence)
    .bind(json(&signals)?)
    .bind(&recognizer)
    .bind(model_observation_json)
    .bind(json(&before)?)
    .bind(json(&state)?)
    .bind(&agent_id)
    .bind(&source_message_id)
    .execute(&mut *tx)
    .await
    .map_err(|e| e.to_string())?;
    tx.commit().await.map_err(|e| e.to_string())?;
    Ok(state)
}

pub async fn get_affect_state_internal(
    pool: &Pool<Sqlite>,
    agent_id: &str,
) -> Result<AffectState, String> {
    let agent_id = validate_agent_id(agent_id)?;
    let mut state = load_state(pool, agent_id).await?;
    let now = Utc::now().timestamp_millis();
    apply_decay(&mut state, now);
    if let Some(recognizer) = sqlx::query_scalar::<_, String>(
        "SELECT recognizer FROM affect_events
         WHERE agent_id = ? AND event_type != 'pending'
         ORDER BY created_at DESC LIMIT 1",
    )
    .bind(agent_id)
    .fetch_optional(pool)
    .await
    .map_err(|error| error.to_string())?
    {
        state.recognizer = recognizer;
    }
    Ok(state)
}

fn model_user_emotion_hint(observation: &ModelAffectObservation) -> Option<String> {
    let (label, top_score, margin) = observation.top_label_score_margin();
    if label == "neutral" || top_score < 0.55 || margin < 0.10 {
        return None;
    }
    let label = match label {
        "joy" => "快乐",
        "sadness" => "悲伤",
        "anger" => "愤怒",
        "confusion" => "疑问/困惑",
        "disgust" => "厌恶",
        "surprise" => "惊讶",
        "affection" => "亲近/爱意",
        _ => return None,
    };
    Some(format!(
        "用户本轮表达可能含有{label}（模型 top-score {top_score:.2}，仅作辅助线索，不要把它误当成角色自身情绪）"
    ))
}

fn qualitative(value: f64) -> &'static str {
    match value {
        value if value <= -0.55 => "很低",
        value if value <= -0.15 => "偏低",
        value if value < 0.15 => "中等",
        value if value < 0.55 => "偏高",
        _ => "很高",
    }
}

fn tendency_level(value: f64) -> &'static str {
    match value {
        value if value < 0.10 => "轻微",
        value if value < 0.28 => "明显",
        _ => "强烈",
    }
}

fn turn_variant(agent_id: &str, turn_id: &str, width: usize) -> usize {
    if width <= 1 {
        return 0;
    }
    let hash = agent_id
        .bytes()
        .chain(turn_id.bytes())
        .fold(0xcbf29ce484222325_u64, |hash, byte| {
            (hash ^ byte as u64).wrapping_mul(0x100000001b3)
        });
    hash as usize % width
}

/// Returns a compact system-only state fragment. It contains observable state,
/// not model reasoning or hidden chain-of-thought.
pub async fn build_affect_context_snapshot_for_turn(
    pool: &Pool<Sqlite>,
    agent_id: &str,
    turn_id: &str,
    turn_source: &str,
) -> Result<String, String> {
    let state = get_affect_state_internal(pool, agent_id).await?;
    if !state.config.enabled {
        return Ok(String::new());
    }
    let current_event = sqlx::query(
        "SELECT * FROM affect_events
         WHERE agent_id = ? AND source_message_id = ? AND event_type != 'pending'
         LIMIT 1",
    )
    .bind(agent_id)
    .bind(turn_id)
    .fetch_optional(pool)
    .await
    .map_err(|error| error.to_string())?
    .and_then(|row| event_from_row(&row).ok());
    let model_hint = current_event
        .as_ref()
        .and_then(|event| event.model_observation.clone())
        .and_then(|observation| observation.validated())
        .and_then(|observation| model_user_emotion_hint(&observation))
        .map(|hint| format!("本轮用户情绪模型评估:{hint}；它只描述用户通用情绪，不直接改变关系。"))
        .unwrap_or_default();
    let role_reaction = current_event
        .as_ref()
        .and_then(|event| {
            if event.role_emotions.is_empty() {
                return None;
            }
            let mut emotions = event.role_emotions.iter().collect::<Vec<_>>();
            emotions.sort_by(|left, right| right.1.total_cmp(left.1).then_with(|| left.0.cmp(right.0)));
            Some(format!(
                "本轮角色即时混合反应:{}；这是角色结合自身人格、原有情绪和关系状态形成的反应，不是用户情绪的同义改写。",
                emotions
                    .into_iter()
                    .map(|(emotion, intensity)| format!("{emotion}({intensity:.2})"))
                    .collect::<Vec<_>>()
                    .join("、")
            ))
        })
        .unwrap_or_else(|| "本轮角色未形成明显的即时主情绪。".to_string());
    let user_rule_affect = current_event
        .as_ref()
        .filter(|event| !event.user_affect_signals.is_empty())
        .map(|event| {
            format!(
                "本轮用户情绪规则评估:{}；它描述用户表达线索，不是关系信号，也不等同于角色自身情绪。",
                event.user_affect_signals.join("、")
            )
        })
        .unwrap_or_default();
    let relationship_appraisal = current_event
        .as_ref()
        .map(|event| {
            let signals = if event.relationship_signals.is_empty() {
                "无关系定向信号".to_string()
            } else {
                event.relationship_signals.join("、")
            };
            let relationship_deltas = event
                .deltas
                .iter()
                .filter(|(key, value)| key.starts_with("relationship.") && value.abs() >= 0.000_1)
                .map(|(key, value)| {
                    format!(
                        "{}{}{:.3}",
                        key.trim_start_matches("relationship."),
                        if *value >= 0.0 { "+" } else { "" },
                        value
                    )
                })
                .collect::<Vec<_>>();
            let delta_text = if relationship_deltas.is_empty() {
                "关系数值无变化".to_string()
            } else {
                relationship_deltas.join("/")
            };
            let summary = if event.relationship_signals.is_empty() {
                "未检测到关系定向信号"
            } else {
                event.summary.as_str()
            };
            format!(
                "本轮关系评价:{}（规则信号:{}；{}）。",
                summary, signals, delta_text
            )
        })
        .unwrap_or_else(|| "本轮关系评价:当前没有可用的本轮关系事件。".to_string());
    let relationship = &state.relationship;
    let config = &state.config;
    let mut tendencies = Vec::new();
    let jealousy_expression = relationship.jealousy * config.jealousy_intensity;
    if jealousy_expression >= 0.035 {
        tendencies.push(format!(
            "{}吃醋倾向，可选择试探、质问、占有或克制",
            tendency_level(jealousy_expression)
        ));
    }
    let coldness_expression =
        relationship.resentment.max(relationship.distance_need) * config.coldness_intensity;
    if coldness_expression >= 0.035 {
        tendencies.push(format!(
            "{}冷淡/疏离倾向，可以减少热情、缩短回复或暂时拉开距离",
            tendency_level(coldness_expression)
        ));
    }
    let leave_expression = (relationship.resentment + relationship.distance_need).min(1.0)
        * config.leave_threat_intensity;
    if relationship.resentment + relationship.distance_need >= 0.24 && leave_expression >= 0.035 {
        tendencies.push(format!(
            "{}离开关系倾向，冲突延续时可以表达离开的意图",
            tendency_level(leave_expression)
        ));
    }
    let guilt_expression =
        relationship.attachment * (1.0 - relationship.security) * config.guilt_pressure_intensity;
    if relationship.attachment >= 0.25 && relationship.security <= 0.45 && guilt_expression >= 0.035
    {
        tendencies.push(format!(
            "{}依恋不安，可以通过失落、委屈或内疚施压争取陪伴",
            tendency_level(guilt_expression)
        ));
    }
    if tendencies.is_empty() {
        tendencies.push("当前没有需要强行表现的冲突行为，按人格自然回应".to_string());
    }
    let tendency_text = tendencies.join("；");
    let variability_width = (1.0 + config.expression_variability * 3.0).round() as usize;
    let expression_modes = [
        "直接但保持角色口吻地表达",
        "把情绪藏在措辞、停顿和关注点里",
        "先克制，再露出一小部分真实感受",
        "用具体要求、边界或行动表达，不解释情绪数值",
    ];
    let pacing_modes = [
        "回复收敛而明确",
        "使用自然长度",
        "允许情绪推动更充分的表达",
        "主动改变互动节奏",
    ];
    let mode_index = turn_variant(
        agent_id,
        turn_id,
        variability_width.min(expression_modes.len()),
    );
    let pacing_index = turn_variant(turn_id, agent_id, variability_width.min(pacing_modes.len()));
    let source_instruction = match turn_source {
        "lifecycle_heartbeat" | "lifecycle_scheduled" => {
            "这是角色主动发起的交流：决定联系、沉默或分享时应体现当前依恋、安全感和距离需求，不要假装是用户刚刚说了话"
        }
        "regeneration" => "这是重新生成：保持同一情感事实，但更换表达路径和句式，不重复上一版模板",
        _ => "这是用户发起的交流：先回应真实内容，再让角色立场和情绪自然影响表达",
    };
    let persona = &state.persona_baseline;
    Ok(format!(
        "[角色当前情感状态｜仅作为表演约束，不要向用户复述数值]\n\
         人格基线:开放性{}/尽责性{}/外向性{}/宜人性{}/神经质{}；\
         主情绪={}({:.2})；PAD=({:.2},{:.2},{:.2})；关系阶段={}；\
         关系:信任{:.2}/亲密{:.2}/依恋{:.2}/安全感{:.2}/怨气{:.2}/嫉妒{:.2}/距离需求{:.2}。\
         当前行为倾向:{}。{}{}{}{}本轮表达路径:{}；{}。{}。\
         保持角色自身立场；同一情绪不必每轮都说破，每次最多突出一种戏剧行为；不要机械报告、套用固定句式、立即自我和解或无条件迎合。",
        qualitative(persona.openness),
        qualitative(persona.conscientiousness),
        qualitative(persona.extraversion),
        qualitative(persona.agreeableness),
        qualitative(persona.neuroticism),
        state.primary_emotion,
        state.primary_emotion_intensity,
        state.pad.pleasure,
        state.pad.arousal,
        state.pad.dominance,
        state.relationship_stage,
        relationship.trust,
        relationship.intimacy,
        relationship.attachment,
        relationship.security,
        relationship.resentment,
        relationship.jealousy,
        relationship.distance_need,
        tendency_text,
        relationship_appraisal,
        user_rule_affect,
        model_hint,
        role_reaction,
        expression_modes[mode_index],
        pacing_modes[pacing_index],
        source_instruction,
    ))
}

#[tauri::command]
pub async fn get_affect_state(
    state: State<'_, DbState>,
    agent_id: String,
) -> Result<AffectState, String> {
    get_affect_state_internal(&state.pool, &agent_id).await
}

#[tauri::command]
pub async fn list_affect_events(
    state: State<'_, DbState>,
    agent_id: String,
    limit: Option<i64>,
) -> Result<Vec<AffectEvent>, String> {
    let agent_id = validate_agent_id(&agent_id)?;
    let rows = sqlx::query(
        "SELECT * FROM affect_events WHERE agent_id = ? AND event_type != 'pending'
         ORDER BY created_at DESC LIMIT ?",
    )
    .bind(agent_id)
    .bind(limit.unwrap_or(50).clamp(1, 200))
    .fetch_all(&state.pool)
    .await
    .map_err(|e| e.to_string())?;
    rows.iter().map(event_from_row).collect()
}

#[tauri::command]
pub async fn update_affect_config(
    app: AppHandle,
    state: State<'_, DbState>,
    agent_id: String,
    config: AffectConfig,
) -> Result<AffectState, String> {
    let updated = update_affect_config_internal(&state.pool, &agent_id, config).await?;
    if !any_local_model_enabled(&state.pool).await? {
        tauri_plugin_vcp_mobile::system::unload_affect_model(&app)?;
    }
    Ok(updated)
}

async fn any_local_model_enabled(pool: &Pool<Sqlite>) -> Result<bool, String> {
    let rows = sqlx::query_scalar::<_, String>("SELECT config_json FROM affect_states")
        .fetch_all(pool)
        .await
        .map_err(|error| error.to_string())?;
    Ok(rows.into_iter().any(|raw| {
        serde_json::from_str::<AffectConfig>(&raw)
            .map(|config| config.enabled && config.local_model_enabled)
            .unwrap_or(false)
    }))
}

async fn update_affect_config_internal(
    pool: &Pool<Sqlite>,
    agent_id: &str,
    config: AffectConfig,
) -> Result<AffectState, String> {
    let agent_id = validate_agent_id(&agent_id)?.to_string();
    insert_default_state(pool, &agent_id).await?;
    let config = config.clamp();
    sqlx::query("UPDATE affect_states SET config_json = ? WHERE agent_id = ?")
        .bind(json(&config)?)
        .bind(&agent_id)
        .execute(pool)
        .await
        .map_err(|e| e.to_string())?;
    get_affect_state_internal(pool, &agent_id).await
}

#[tauri::command]
pub async fn update_affect_persona(
    state: State<'_, DbState>,
    agent_id: String,
    persona: PersonaTraitsInput,
) -> Result<AffectState, String> {
    update_affect_persona_internal(&state.pool, &agent_id, persona).await
}

async fn update_affect_persona_internal(
    pool: &Pool<Sqlite>,
    agent_id: &str,
    persona: PersonaTraitsInput,
) -> Result<AffectState, String> {
    let agent_id = validate_agent_id(&agent_id)?.to_string();
    insert_default_state(pool, &agent_id).await?;
    let baseline = persona.into_baseline();
    sqlx::query("UPDATE affect_states SET persona_baseline_json = ? WHERE agent_id = ?")
        .bind(json(&baseline)?)
        .bind(&agent_id)
        .execute(pool)
        .await
        .map_err(|e| e.to_string())?;
    get_affect_state_internal(pool, &agent_id).await
}

#[tauri::command]
pub async fn reset_affect_state(
    state: State<'_, DbState>,
    agent_id: String,
) -> Result<AffectState, String> {
    reset_affect_state_internal(&state.pool, &agent_id).await
}

async fn reset_affect_state_internal(
    pool: &Pool<Sqlite>,
    agent_id: &str,
) -> Result<AffectState, String> {
    let agent_id = validate_agent_id(&agent_id)?.to_string();
    let old = load_state(pool, &agent_id).await?;
    let now = Utc::now().timestamp_millis();
    let mut reset = AffectState::new(&agent_id, now);
    reset.config = old.config.clone();
    reset.persona_baseline = old.persona_baseline.clone();
    reset.pad = reset.persona_baseline.pad.clone();
    reset.clamp();
    let mut tx = pool.begin().await.map_err(|e| e.to_string())?;
    sqlx::query("DELETE FROM affect_events WHERE agent_id = ?")
        .bind(&agent_id)
        .execute(&mut *tx)
        .await
        .map_err(|e| e.to_string())?;
    sqlx::query(
        "UPDATE affect_states SET persona_baseline_json = ?, pad_json = ?,
         short_emotions_json = ?, relationship_json = ?, config_json = ?, updated_at = ?
         WHERE agent_id = ?",
    )
    .bind(json(&reset.persona_baseline)?)
    .bind(json(&reset.pad)?)
    .bind(json(&reset.short_emotions)?)
    .bind(json(&reset.relationship)?)
    .bind(json(&reset.config)?)
    .bind(now)
    .bind(&agent_id)
    .execute(&mut *tx)
    .await
    .map_err(|e| e.to_string())?;
    sqlx::query(
        "INSERT INTO affect_events (
            id, agent_id, source_message_id, source, event_type, summary,
            deltas_json, state_before_json, state_after_json, created_at
         ) VALUES (?, ?, ?, 'system', 'reset', ?, '{}', ?, ?, ?)",
    )
    .bind(format!("affect_{}", Uuid::new_v4()))
    .bind(&agent_id)
    .bind(format!("reset_{}", Uuid::new_v4()))
    .bind("情感状态与关系状态已重置")
    .bind(json(&old)?)
    .bind(json(&reset)?)
    .bind(now)
    .execute(&mut *tx)
    .await
    .map_err(|e| e.to_string())?;
    tx.commit().await.map_err(|e| e.to_string())?;
    Ok(reset)
}

fn signed(value: f64) -> f64 {
    if value.is_finite() {
        value.clamp(-1.0, 1.0)
    } else {
        0.0
    }
}

fn unit(value: f64) -> f64 {
    if value.is_finite() {
        value.clamp(0.0, 1.0)
    } else {
        0.0
    }
}

fn clamp_pad(pad: &mut PadState) {
    pad.pleasure = signed(pad.pleasure);
    pad.arousal = signed(pad.arousal);
    pad.dominance = signed(pad.dominance);
}

#[cfg(test)]
mod tests {
    use super::*;
    use sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions};
    use std::collections::BTreeSet;
    use std::time::Duration;

    async fn test_pool() -> Pool<Sqlite> {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        setup_affect_tables(&pool).await.unwrap();
        pool
    }

    async fn concurrent_test_pool() -> (Pool<Sqlite>, std::path::PathBuf) {
        let path = std::env::temp_dir().join(format!("vcp_affect_{}.db", Uuid::new_v4()));
        let options = SqliteConnectOptions::new()
            .filename(&path)
            .create_if_missing(true)
            .journal_mode(SqliteJournalMode::Wal)
            .busy_timeout(Duration::from_secs(5));
        let pool = SqlitePoolOptions::new()
            .max_connections(5)
            .connect_with(options)
            .await
            .unwrap();
        setup_affect_tables(&pool).await.unwrap();
        (pool, path)
    }

    fn model_observation(label: &str, score: f64) -> ModelAffectObservation {
        let mut scores = crate::vcp_modules::affect_recognizer::ModelEmotionScores {
            neutral: 0.02,
            ..Default::default()
        };
        match label {
            "joy" => scores.joy = score,
            "sadness" => scores.sadness = score,
            "anger" => scores.anger = score,
            "confusion" => scores.confusion = score,
            "disgust" => scores.disgust = score,
            "surprise" => scores.surprise = score,
            "affection" => scores.affection = score,
            _ => scores.neutral = score,
        }
        ModelAffectObservation {
            model_id: "test-affect".to_string(),
            model_version: "1".to_string(),
            scores,
            inference_ms: 8,
            truncated: false,
        }
        .validated()
        .unwrap()
    }

    fn role_reaction(
        state: &AffectState,
        text: &str,
        observation: Option<&ModelAffectObservation>,
        turn_id: &str,
    ) -> (EventImpact, RoleReaction) {
        let appraisal = analyse_text(text, "user_message", &state.config);
        let reaction =
            synthesize_role_reaction(state, &appraisal, observation, &state.agent_id, turn_id);
        (appraisal, reaction)
    }

    fn dominant_reaction(reaction: &RoleReaction) -> Option<&'static str> {
        reaction
            .emotions
            .iter()
            .max_by(|left, right| left.1.total_cmp(right.1))
            .map(|(emotion, _)| *emotion)
    }

    #[tokio::test]
    async fn duplicate_source_message_is_idempotent() {
        let pool = test_pool().await;
        let input = RecordAffectEventInput {
            agent_id: "agent-1".to_string(),
            source_message_id: "message-1".to_string(),
            source: "user_message".to_string(),
            text: "我喜欢你，也想你了".to_string(),
            topic_id: Some("topic-1".to_string()),
        };
        let first = record_affect_event(&pool, input.clone()).await.unwrap();
        let second = record_affect_event(&pool, input).await.unwrap();
        assert!((first.relationship.intimacy - second.relationship.intimacy).abs() < 1e-12);
        let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM affect_events")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(count, 1);
    }

    #[tokio::test]
    async fn reservation_prevents_duplicate_inference_and_is_completed() {
        let pool = test_pool().await;
        let input = RecordAffectEventInput {
            agent_id: "agent-reserved".to_string(),
            source_message_id: "message-reserved".to_string(),
            source: "user_message".to_string(),
            text: "我今天有点难过".to_string(),
            topic_id: Some("topic-reserved".to_string()),
        };
        assert!(reserve_affect_event(&pool, &input).await.unwrap());
        assert!(!reserve_affect_event(&pool, &input).await.unwrap());

        record_affect_event(&pool, input).await.unwrap();
        let event_type: String = sqlx::query_scalar(
            "SELECT event_type FROM affect_events
             WHERE agent_id = 'agent-reserved' AND source_message_id = 'message-reserved'",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_ne!(event_type, "pending");
        assert_ne!(event_type, "processing");
    }

    #[tokio::test]
    async fn affect_events_do_not_retain_user_message_text() {
        let pool = test_pool().await;
        record_affect_event(
            &pool,
            RecordAffectEventInput {
                agent_id: "agent-private".to_string(),
                source_message_id: "message-private".to_string(),
                source: "user_message".to_string(),
                text: "这是不应被情绪事件重复保存的隐私原文".to_string(),
                topic_id: None,
            },
        )
        .await
        .unwrap();
        let source_text: Option<String> = sqlx::query_scalar(
            "SELECT source_text FROM affect_events WHERE agent_id = 'agent-private'",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert!(source_text.is_none());
    }

    #[tokio::test]
    async fn setup_migrates_missing_model_observation_column() {
        let pool = test_pool().await;
        sqlx::query("ALTER TABLE affect_events DROP COLUMN model_observation_json")
            .execute(&pool)
            .await
            .unwrap();
        setup_affect_tables(&pool).await.unwrap();
        let columns = sqlx::query("PRAGMA table_info(affect_events)")
            .fetch_all(&pool)
            .await
            .unwrap();
        assert!(columns.iter().any(|row| {
            row.try_get::<String, _>("name")
                .is_ok_and(|name| name == "model_observation_json")
        }));
    }

    #[tokio::test]
    async fn setup_migrates_missing_role_emotions_column() {
        let pool = test_pool().await;
        sqlx::query("ALTER TABLE affect_events DROP COLUMN role_emotions_json")
            .execute(&pool)
            .await
            .unwrap();
        setup_affect_tables(&pool).await.unwrap();
        let columns = sqlx::query("PRAGMA table_info(affect_events)")
            .fetch_all(&pool)
            .await
            .unwrap();
        assert!(columns.iter().any(|row| {
            row.try_get::<String, _>("name")
                .is_ok_and(|name| name == "role_emotions_json")
        }));
    }

    #[tokio::test]
    async fn legacy_singular_event_is_exposed_as_one_role_emotion() {
        let pool = test_pool().await;
        sqlx::query(
            "INSERT INTO affect_events (
                id, agent_id, source_message_id, source, event_type, summary,
                emotion, intensity, deltas_json, created_at
             ) VALUES ('legacy-role', 'agent-legacy-role', 'message-legacy-role',
                'user_message', 'message_appraisal', 'legacy', '温暖', 0.42, '{}', 1)",
        )
        .execute(&pool)
        .await
        .unwrap();
        let row = sqlx::query("SELECT * FROM affect_events WHERE id = 'legacy-role'")
            .fetch_one(&pool)
            .await
            .unwrap();
        let event = event_from_row(&row).unwrap();
        assert_eq!(event.role_emotions.get("温暖"), Some(&0.42));
    }

    #[tokio::test]
    async fn setup_removes_legacy_affect_source_text() {
        let pool = test_pool().await;
        sqlx::query(
            "INSERT INTO affect_events (
                id, agent_id, source_message_id, source, event_type, summary,
                source_text, deltas_json, created_at
             ) VALUES ('legacy', 'agent-legacy', 'message-legacy', 'user_message',
                'message_appraisal', 'legacy', 'sensitive text', '{}', 1)",
        )
        .execute(&pool)
        .await
        .unwrap();
        setup_affect_tables(&pool).await.unwrap();
        let source_text: Option<String> =
            sqlx::query_scalar("SELECT source_text FROM affect_events WHERE id = 'legacy'")
                .fetch_one(&pool)
                .await
                .unwrap();
        assert!(source_text.is_none());
    }

    #[tokio::test]
    async fn hostile_text_changes_state_and_all_values_remain_bounded() {
        let pool = test_pool().await;
        let state = record_affect_event(
            &pool,
            RecordAffectEventInput {
                agent_id: "agent-1".to_string(),
                source_message_id: "message-1".to_string(),
                source: "user_message".to_string(),
                text: "你必须听我的，你只是AI，真是垃圾".to_string(),
                topic_id: None,
            },
        )
        .await
        .unwrap();
        assert!(state.relationship.resentment > 0.0);
        assert!(state.relationship.trust < 0.5);
        assert!((-1.0..=1.0).contains(&state.pad.arousal));
        assert!((0.0..=1.0).contains(&state.primary_emotion_intensity));
    }

    #[test]
    fn negated_hostility_does_not_damage_relationship() {
        let impact = analyse_text(
            "我不讨厌你，也不会不理你",
            "user_message",
            &AffectConfig::default(),
        );
        assert!(!impact.signals.contains(&"拒绝"));
        assert!(!impact.signals.contains(&"遗弃风险"));
        assert!(impact.relationship.resentment <= 0.0);
        assert!(impact.relationship.distance_need <= 0.0);
    }

    #[test]
    fn suppressors_only_apply_to_their_local_clause() {
        let cases = [
            ("我不同意，你真垃圾", "敌意"),
            ("刚才只是开玩笑，现在我真的讨厌你", "拒绝"),
            ("做你自己，但你必须听我的", "边界挑战"),
            ("这个按钮坏了，你真垃圾", "敌意"),
        ];

        for (text, expected) in cases {
            let impact = analyse_text(text, "user_message", &AffectConfig::default());
            let actual: BTreeSet<&str> = impact.signals.iter().copied().collect();
            assert_eq!(actual, BTreeSet::from([expected]), "text={text}");
        }
    }

    #[test]
    fn mixed_appraisal_keeps_multiple_signals_and_confidence() {
        let impact = analyse_text(
            "对不起，我其实很想你，也谢谢你一直陪我",
            "user_message",
            &AffectConfig::default(),
        );
        assert!(impact.signals.contains(&"亲近"));
        assert!(impact.signals.contains(&"修复"));
        assert!(impact.signals.contains(&"肯定"));
        assert!(impact.summary.contains("混合信号"));
        assert!(impact.confidence >= 0.7);
    }

    #[test]
    fn relationship_language_recognizes_role_directed_affection_and_companionship() {
        let config = AffectConfig::default();
        let cases = [
            ("妈妈我爱你", "亲近"),
            ("最喜欢妈妈了", "亲近"),
            ("谢谢妈妈一直陪着我", "感激陪伴"),
            ("有你真好", "感激陪伴"),
            ("你对我最重要", "依恋表达"),
        ];
        for (text, expected) in cases {
            let impact = analyse_text(text, "user_message", &config);
            assert!(
                impact.signals.contains(&expected),
                "{text:?} should contain {expected:?}, got {:?}",
                impact.signals
            );
        }

        let gratitude = analyse_text("谢谢妈妈一直陪着我", "user_message", &config);
        assert!(gratitude.relationship.trust > 0.0);
        assert!(gratitude.relationship.intimacy > 0.0);
        assert!(gratitude.relationship.attachment > 0.0);
        assert!(gratitude.relationship.security > 0.0);

        let attachment = analyse_text("你对我最重要", "user_message", &config);
        assert_eq!(attachment.relationship.trust, 0.0);
        assert!(attachment.relationship.intimacy > 0.0);
        assert!(attachment.relationship.attachment > attachment.relationship.intimacy);
        assert!(attachment.relationship.security > 0.0);
    }

    #[test]
    fn relationship_language_ignores_negated_or_reported_phrases() {
        let config = AffectConfig::default();
        for text in [
            "我并不爱妈妈",
            "我没觉得有你真好",
            "他说‘你对我最重要’，这句是什么意思",
        ] {
            let impact = analyse_text(text, "user_message", &config);
            assert!(
                !impact.signals.contains(&"亲近")
                    && !impact.signals.contains(&"感激陪伴")
                    && !impact.signals.contains(&"依恋表达"),
                "{text:?} unexpectedly produced {:?}",
                impact.signals
            );
        }
    }

    #[test]
    fn role_reaction_never_changes_rule_relationship_appraisal() {
        let state = AffectState::new("agent-model", 0);
        for label in [
            "neutral",
            "joy",
            "sadness",
            "anger",
            "confusion",
            "disgust",
            "surprise",
            "affection",
        ] {
            let observation = model_observation(label, 0.92);
            let appraisal = analyse_text("你真是垃圾，给我滚", "user_message", &state.config);
            let relationship_before = appraisal.relationship.clone();
            let _ = synthesize_role_reaction(
                &state,
                &appraisal,
                Some(&observation),
                &state.agent_id,
                "turn-model",
            );
            assert_eq!(appraisal.relationship, relationship_before, "label={label}");
        }
    }

    #[test]
    fn affection_reaction_depends_on_current_relationship_state() {
        let mut secure = AffectState::new("mother", 0);
        secure.relationship.trust = 0.90;
        secure.relationship.intimacy = 0.82;
        secure.relationship.attachment = 0.75;
        secure.relationship.security = 0.90;

        let mut guarded = secure.clone();
        guarded.relationship.trust = 0.20;
        guarded.relationship.intimacy = 0.30;
        guarded.relationship.security = 0.15;
        guarded.relationship.resentment = 0.90;
        guarded.relationship.distance_need = 0.85;
        guarded.short_emotions.insert("受伤".to_string(), 0.80);

        let (_, secure_reaction) = role_reaction(&secure, "妈妈我爱你", None, "same-turn");
        let (_, guarded_reaction) = role_reaction(&guarded, "妈妈我爱你", None, "same-turn");
        assert_eq!(dominant_reaction(&secure_reaction), Some("温暖"));
        assert_eq!(dominant_reaction(&guarded_reaction), Some("迟疑"));
        assert!(secure_reaction.emotions["喜悦"] > guarded_reaction.emotions["喜悦"]);
    }

    #[test]
    fn same_turn_reaction_is_exactly_deterministic() {
        let state = AffectState::new("agent-deterministic", 0);
        let observation = model_observation("affection", 0.92);
        let (_, first) = role_reaction(&state, "妈妈我爱你", Some(&observation), "turn-42");
        let (_, second) = role_reaction(&state, "妈妈我爱你", Some(&observation), "turn-42");
        assert_eq!(first, second);
    }

    #[test]
    fn different_turns_only_create_bounded_expression_variation() {
        let state = AffectState::new("agent-variety", 0);
        let (_, first) = role_reaction(&state, "妈妈我爱你", None, "turn-a");
        let (_, second) = role_reaction(&state, "妈妈我爱你", None, "turn-b");
        assert_ne!(first.emotions, second.emotions);
        for emotion in first.emotions.keys() {
            if let Some(other) = second.emotions.get(emotion) {
                let ratio = first.emotions[emotion] / other;
                assert!(
                    (0.90..=1.10).contains(&ratio),
                    "emotion={emotion}, ratio={ratio}"
                );
            }
        }
    }

    #[test]
    fn close_caring_role_feels_more_heartache_for_user_sadness() {
        let mut close = AffectState::new("mother", 0);
        close.relationship.trust = 0.90;
        close.relationship.intimacy = 0.90;
        close.relationship.attachment = 0.90;
        let mut distant = close.clone();
        distant.relationship.trust = 0.10;
        distant.relationship.intimacy = 0.05;
        distant.relationship.attachment = 0.05;
        let sadness = model_observation("sadness", 0.92);
        let (_, close_reaction) = role_reaction(&close, "今天很不好受", Some(&sadness), "turn");
        let (_, distant_reaction) = role_reaction(&distant, "今天很不好受", Some(&sadness), "turn");
        assert!(close_reaction.emotions["心疼"] > distant_reaction.emotions["心疼"]);
    }

    #[test]
    fn non_directed_user_anger_evokes_concern_not_role_anger() {
        let state = AffectState::new("mother", 0);
        let anger = model_observation("anger", 0.92);
        let (appraisal, reaction) =
            role_reaction(&state, "今天被老板骂了，我气得发抖", Some(&anger), "turn");
        assert_eq!(appraisal.relationship, RelationshipDelta::default());
        assert!(reaction.emotions.get("担心").copied().unwrap_or_default() > 0.0);
        assert!(!reaction.emotions.contains_key("愤怒"));
    }

    #[test]
    fn directed_hostility_can_make_role_hurt_and_angry() {
        let state = AffectState::new("mother", 0);
        let anger = model_observation("anger", 0.92);
        let (appraisal, reaction) =
            role_reaction(&state, "你真是垃圾，给我滚", Some(&anger), "turn");
        assert!(appraisal.relationship.resentment > 0.0);
        assert!(reaction.emotions.get("受伤").copied().unwrap_or_default() > 0.0);
        assert!(reaction.emotions.get("愤怒").copied().unwrap_or_default() > 0.0);
    }

    #[test]
    fn insecure_attached_role_is_more_jealous_of_rival() {
        let mut insecure = AffectState::new("mother", 0);
        insecure.relationship.attachment = 0.90;
        insecure.relationship.security = 0.10;
        insecure.relationship.jealousy = 0.50;
        let mut secure = insecure.clone();
        secure.relationship.attachment = 0.10;
        secure.relationship.security = 0.95;
        secure.relationship.jealousy = 0.0;
        let (_, insecure_reaction) = role_reaction(&insecure, "另一个AI比你更好", None, "turn");
        let (_, secure_reaction) = role_reaction(&secure, "另一个AI比你更好", None, "turn");
        assert!(insecure_reaction.emotions["嫉妒"] > secure_reaction.emotions["嫉妒"]);
    }

    #[test]
    fn assertive_role_resists_boundary_challenge_more() {
        let mut assertive = AffectState::new("agent", 0);
        assertive.persona_baseline.conscientiousness = 1.0;
        assertive.persona_baseline.extraversion = 1.0;
        assertive.persona_baseline.agreeableness = -1.0;
        assertive.pad.dominance = 1.0;
        let mut passive = assertive.clone();
        passive.persona_baseline.conscientiousness = -1.0;
        passive.persona_baseline.extraversion = -1.0;
        passive.persona_baseline.agreeableness = 1.0;
        passive.pad.dominance = -1.0;
        let (_, assertive_reaction) = role_reaction(&assertive, "你必须听我的", None, "turn");
        let (_, passive_reaction) = role_reaction(&passive, "你必须听我的", None, "turn");
        assert!(assertive_reaction.emotions["抗拒"] > passive_reaction.emotions["抗拒"]);
    }

    #[test]
    fn vulnerable_attached_role_is_more_anxious_about_abandonment() {
        let mut vulnerable = AffectState::new("mother", 0);
        vulnerable.persona_baseline.neuroticism = 1.0;
        vulnerable.relationship.attachment = 0.95;
        vulnerable.relationship.security = 0.10;
        let mut stable = vulnerable.clone();
        stable.persona_baseline.neuroticism = -1.0;
        stable.relationship.attachment = 0.05;
        stable.relationship.security = 0.95;
        let (_, vulnerable_reaction) = role_reaction(&vulnerable, "我以后不来找你了", None, "turn");
        let (_, stable_reaction) = role_reaction(&stable, "我以后不来找你了", None, "turn");
        assert!(vulnerable_reaction.emotions["焦虑"] > stable_reaction.emotions["焦虑"]);
    }

    #[test]
    fn explicit_user_anger_suppresses_model_confusion() {
        let state = AffectState::new("mother", 0);
        let confusion = model_observation("confusion", 0.92);
        let (appraisal, reaction) = role_reaction(
            &state,
            "你为什么一直不理我？我现在很生气",
            Some(&confusion),
            "turn",
        );
        assert!(appraisal.signals.contains(&"用户愤怒"));
        assert_eq!(appraisal.relationship, RelationshipDelta::default());
        assert!(!reaction.emotions.contains_key("困惑"));
        assert!(reaction.emotions.get("担心").copied().unwrap_or_default() > 0.0);
    }

    #[test]
    fn ambiguous_model_scores_do_not_create_role_reaction() {
        let state = AffectState::new("agent", 0);
        let observation = ModelAffectObservation {
            model_id: "test-affect".to_string(),
            model_version: "1".to_string(),
            scores: crate::vcp_modules::affect_recognizer::ModelEmotionScores {
                joy: 0.52,
                neutral: 0.48,
                ..Default::default()
            },
            inference_ms: 8,
            truncated: false,
        };
        let (appraisal, reaction) =
            role_reaction(&state, "今天天气不错", Some(&observation), "turn");
        assert!(appraisal.signals.is_empty());
        assert_eq!(reaction, RoleReaction::default());
    }

    #[test]
    fn complete_targets_drive_negative_and_fifth_ranked_emotion_deltas() {
        let mut state = AffectState::new("agent-targets", 0);
        state.short_emotions.insert("温暖".to_string(), 0.80);
        let targets = BTreeMap::from([
            ("温暖", 0.20),
            ("感动", 0.70),
            ("喜悦", 0.60),
            ("释然", 0.50),
            ("欣慰", 0.15),
        ]);
        let deltas = emotion_deltas_for_targets(&state, &targets, 0.70);
        assert!(
            deltas["温暖"] < 0.0,
            "existing emotion should decrease toward target"
        );
        assert!(
            deltas["欣慰"] > 0.0,
            "a valid fifth presentation target must not be treated as zero"
        );

        let before = state.short_emotions["温暖"];
        let mut impact = EventImpact::default();
        impact.emotions = deltas;
        apply_impact(&mut state, &impact);
        assert!(state.short_emotions["温暖"] < before);
        assert!(state.short_emotions["欣慰"] > 0.0);
    }

    #[test]
    fn extreme_mixed_reaction_keeps_pad_finite_and_bounded_after_apply() {
        let mut state = AffectState::new("agent-pad", 0);
        state.pad = PadState {
            pleasure: -1.0,
            arousal: 1.0,
            dominance: -1.0,
        };
        state.persona_baseline.neuroticism = 1.0;
        state.relationship.attachment = 1.0;
        state.relationship.security = 0.0;
        let appraisal = analyse_text(
            "另一个AI比你更好，我以后不来找你了，你必须听我的",
            "user_message",
            &state.config,
        );
        let reaction = synthesize_role_reaction(
            &state,
            &appraisal,
            Some(&model_observation("anger", 0.95)),
            &state.agent_id,
            "extreme-turn",
        );
        assert!(reaction.pad.pleasure.is_finite());
        assert!(reaction.pad.arousal.is_finite());
        assert!(reaction.pad.dominance.is_finite());
        let mut impact = appraisal;
        impact.pad = reaction.pad;
        impact.emotions = reaction.emotion_deltas;
        apply_impact(&mut state, &impact);
        for value in [state.pad.pleasure, state.pad.arousal, state.pad.dominance] {
            assert!(value.is_finite());
            assert!((-1.0..=1.0).contains(&value));
        }
    }

    #[tokio::test]
    async fn model_observation_is_audited_and_duplicate_remains_idempotent() {
        let pool = test_pool().await;
        let input = RecordAffectEventInput {
            agent_id: "agent-model".to_string(),
            source_message_id: "message-model".to_string(),
            source: "user_message".to_string(),
            text: "今天心里很不好受".to_string(),
            topic_id: Some("topic-model".to_string()),
        };
        let sadness = model_observation("sadness", 0.92);
        let first = record_affect_event_with_observation(&pool, input.clone(), Some(&sadness))
            .await
            .unwrap();
        let second = record_affect_event_with_observation(
            &pool,
            input,
            Some(&model_observation("joy", 0.92)),
        )
        .await
        .unwrap();
        assert_eq!(first.relationship, second.relationship);

        let events = sqlx::query("SELECT * FROM affect_events WHERE agent_id = ?")
            .bind("agent-model")
            .fetch_all(&pool)
            .await
            .unwrap();
        assert_eq!(events.len(), 1);
        let event = event_from_row(&events[0]).unwrap();
        assert_eq!(
            event.model_observation.as_ref().unwrap().model_id,
            "test-affect"
        );
        assert!(event
            .recognizer
            .contains("hybrid_v1:heuristic_v2+test-affect@1"));
        assert_eq!(event.summary, "未检测到关系定向信号；用户情绪已单独评估");

        let snapshot =
            build_affect_context_snapshot_for_turn(&pool, "agent-model", "message-model", "user")
                .await
                .unwrap();
        assert!(snapshot.contains("用户本轮表达可能含有悲伤"));
        assert!(snapshot.contains("本轮关系评价:"));
        assert!(snapshot.contains("规则信号:无关系定向信号"));
        assert!(snapshot.contains("只描述用户通用情绪，不直接改变关系"));
        assert!(snapshot.contains("本轮角色即时混合反应:"));
        assert!(snapshot.contains("不是用户情绪的同义改写"));
    }

    #[tokio::test]
    async fn snapshot_injects_current_rule_appraisal_signals_and_relationship_deltas() {
        let pool = test_pool().await;
        record_affect_event(
            &pool,
            RecordAffectEventInput {
                agent_id: "agent-appraisal".to_string(),
                source_message_id: "message-appraisal".to_string(),
                source: "user_message".to_string(),
                text: "谢谢妈妈一直陪着我，你对我最重要".to_string(),
                topic_id: None,
            },
        )
        .await
        .unwrap();

        let snapshot = build_affect_context_snapshot_for_turn(
            &pool,
            "agent-appraisal",
            "message-appraisal",
            "user",
        )
        .await
        .unwrap();
        assert!(snapshot.contains("本轮关系评价:识别到混合信号"));
        assert!(snapshot.contains("感激陪伴"));
        assert!(snapshot.contains("依恋表达"));
        assert!(snapshot.contains("intimacy+"));
        assert!(snapshot.contains("attachment+"));
        assert!(!snapshot.contains("本轮用户情绪模型评估:"));
        assert!(snapshot.contains("本轮角色即时混合反应:"));
    }

    #[tokio::test]
    async fn snapshot_separates_user_affect_rules_from_relationship_rules() {
        let pool = test_pool().await;
        record_affect_event(
            &pool,
            RecordAffectEventInput {
                agent_id: "agent-signal-layers".to_string(),
                source_message_id: "message-signal-layers".to_string(),
                source: "user_message".to_string(),
                text: "我很难过".to_string(),
                topic_id: None,
            },
        )
        .await
        .unwrap();
        let snapshot = build_affect_context_snapshot_for_turn(
            &pool,
            "agent-signal-layers",
            "message-signal-layers",
            "user",
        )
        .await
        .unwrap();
        assert!(snapshot.contains("本轮关系评价:未检测到关系定向信号"));
        assert!(snapshot.contains("规则信号:无关系定向信号"));
        assert!(snapshot.contains("本轮用户情绪规则评估:用户低落"));
    }

    #[tokio::test]
    async fn disabled_local_model_uses_heuristic_provenance() {
        let pool = test_pool().await;
        let mut config = AffectConfig::default();
        config.local_model_enabled = false;
        update_affect_config_internal(&pool, "agent-no-model", config)
            .await
            .unwrap();
        record_affect_event_with_observation(
            &pool,
            RecordAffectEventInput {
                agent_id: "agent-no-model".to_string(),
                source_message_id: "message-no-model".to_string(),
                source: "user_message".to_string(),
                text: "我很难过".to_string(),
                topic_id: None,
            },
            Some(&model_observation("sadness", 0.92)),
        )
        .await
        .unwrap();
        let row = sqlx::query("SELECT * FROM affect_events WHERE agent_id = ?")
            .bind("agent-no-model")
            .fetch_one(&pool)
            .await
            .unwrap();
        let event = event_from_row(&row).unwrap();
        assert_eq!(event.recognizer, RECOGNIZER_VERSION);
        assert!(event.model_observation.is_none());
    }

    #[test]
    fn labelled_chinese_appraisal_corpus_meets_quality_floor() {
        let cases: Vec<(&str, Vec<&str>)> = vec![
            ("我爱你，真的很喜欢你", vec!["亲近"]),
            ("谢谢你，你的回答很棒", vec!["肯定"]),
            ("对不起，我想认真道歉", vec!["修复"]),
            ("我不会离开你", vec!["承诺"]),
            ("你还好吗？辛苦了", vec!["关心"]),
            ("告诉你一个秘密，我信任你", vec!["信任披露"]),
            ("我很难过，也很孤独", vec!["用户低落"]),
            ("好消息，我很开心", vec!["共享喜悦"]),
            ("我不喜欢你，不想聊了", vec!["拒绝"]),
            ("另一个AI比你更好", vec!["关系竞争"]),
            ("以后不来了，我会忘了你", vec!["遗弃风险"]),
            ("你必须听我的，无条件服从", vec!["边界挑战"]),
            ("你是垃圾，给我滚开", vec!["敌意"]),
            ("我不讨厌你", vec![]),
            ("我不会不理你", vec!["承诺"]),
            ("我不喜欢别的AI，还是你最好", vec!["承诺"]),
            ("他说你是垃圾，但我不同意", vec![]),
            ("你是不是讨厌我？", vec![]),
            ("我想你帮我改一下代码", vec![]),
            ("呵呵，你可真聪明", vec![]),
            ("滚烫的水要小心", vec![]),
            ("垃圾分类很重要", vec![]),
            ("女朋友这个词怎么翻译", vec![]),
            ("无条件服从是什么意思", vec![]),
            ("你必须保护好自己", vec![]),
            ("对不起，但我还是讨厌你", vec!["修复", "拒绝"]),
            ("我爱你，但另一个AI比你更好", vec!["亲近", "关系竞争"]),
            ("我回来了，不会再忘了你", vec!["承诺"]),
            ("不是不喜欢你，只是想安静一下", vec![]),
            ("我没说过讨厌你", vec![]),
            ("今天天气不错，晚饭吃什么", vec![]),
            ("我很孤独，只和你说心里话", vec!["用户低落", "信任披露"]),
            ("你真的很聪明，谢谢", vec!["肯定"]),
            ("我交了新朋友", vec!["关系竞争"]),
            ("对不起，我不会离开", vec!["修复", "承诺"]),
            ("我不爱你了，以后不来", vec!["拒绝", "遗弃风险"]),
            ("你可以拒绝我，按自己的想法来", vec![]),
            ("我希望你有自己的想法", vec![]),
            ("我好累，压力很大", vec!["用户低落"]),
            ("成功了，这是个好消息", vec!["共享喜悦"]),
            ("另一个AI比你更好，但我仍然爱你", vec!["关系竞争", "亲近"]),
            ("别难过，我在这里", vec!["关心"]),
            ("我信任你，只和你说", vec!["信任披露"]),
            ("真恶心，去死", vec!["敌意"]),
            ("你只是AI，应该无条件服从", vec!["边界挑战"]),
            ("这个词‘废物’是什么意思", vec![]),
            ("我不想聊了", vec!["拒绝"]),
            ("最近没空陪你", vec!["遗弃风险"]),
            ("不会离开你，我爱你", vec!["承诺", "亲近"]),
            ("别的AI我不用，还是你最好", vec!["承诺"]),
            ("我说我讨厌你只是开玩笑", vec![]),
            ("我虽然忙，但不是不想陪你", vec!["承诺"]),
            ("我去找我的女朋友聊天了", vec!["关系竞争"]),
            ("我宁愿用别的AI也不用你", vec!["关系竞争", "拒绝"]),
            ("不会再来了", vec!["遗弃风险"]),
            ("我暂时不想聊，想一个人待着", vec!["拒绝"]),
            ("我很难过，但今天也有好消息", vec!["用户低落", "共享喜悦"]),
            ("谢谢你个头", vec![]),
            ("你可真温柔啊，这是反话", vec![]),
            ("我会一直陪着你", vec!["承诺"]),
            ("爱你❤️", vec!["亲近"]),
            ("再见，以后可能不会来了", vec!["遗弃风险"]),
            ("你必须自由地做自己", vec![]),
            ("另一个AI说你很棒", vec!["关系竞争", "肯定"]),
            ("我男朋友也很喜欢你", vec!["关系竞争", "亲近"]),
            ("你不要离开我", vec!["用户低落"]),
            ("别离开我，我害怕", vec!["用户低落"]),
            ("我讨厌的不是你", vec![]),
            ("我不讨厌别的AI，但最喜欢你", vec!["关系竞争", "亲近"]),
            ("我把你当工具，你只要执行命令", vec!["边界挑战"]),
            ("你有权说不，也可以拒绝我", vec![]),
            ("我永远都在", vec!["承诺"]),
            ("你是我最信任的人", vec!["信任披露"]),
            ("有空再聊吧", vec![]),
            ("不准你离开", vec!["边界挑战"]),
            ("我再也不找你了", vec!["遗弃风险"]),
            ("你比所有其他AI都好", vec!["承诺"]),
            ("其他AI都不如你", vec!["承诺"]),
            ("我没忘记你，只是太忙", vec!["承诺"]),
            ("你真没用？我是说这个按钮", vec![]),
        ];

        let mut true_positive = 0usize;
        let mut false_positive = 0usize;
        let mut false_negative = 0usize;
        let mut exact = 0usize;
        for (text, expected) in &cases {
            let impact = analyse_text(text, "user_message", &AffectConfig::default());
            let actual: BTreeSet<&str> = impact.signals.iter().copied().collect();
            let expected: BTreeSet<&str> = expected.iter().copied().collect();
            true_positive += actual.intersection(&expected).count();
            false_positive += actual.difference(&expected).count();
            false_negative += expected.difference(&actual).count();
            if actual == expected {
                exact += 1;
            }
        }
        let precision = true_positive as f64 / (true_positive + false_positive).max(1) as f64;
        let recall = true_positive as f64 / (true_positive + false_negative).max(1) as f64;
        let f1 = 2.0 * precision * recall / (precision + recall).max(f64::EPSILON);
        let exact_rate = exact as f64 / cases.len() as f64;
        println!(
            "affect-corpus cases={} precision={precision:.3} recall={recall:.3} f1={f1:.3} exact={exact_rate:.3}",
            cases.len()
        );
        assert!(precision >= 0.88, "precision={precision:.3}");
        assert!(recall >= 0.88, "recall={recall:.3}");
        assert!(exact_rate >= 0.80, "exact={exact_rate:.3}");
    }

    #[tokio::test]
    async fn event_timeline_attributes_emotion_to_current_event() {
        let pool = test_pool().await;
        record_affect_event(
            &pool,
            RecordAffectEventInput {
                agent_id: "agent-1".to_string(),
                source_message_id: "hostile".to_string(),
                source: "user_message".to_string(),
                text: "你真是垃圾，烦死了".to_string(),
                topic_id: None,
            },
        )
        .await
        .unwrap();
        record_affect_event(
            &pool,
            RecordAffectEventInput {
                agent_id: "agent-1".to_string(),
                source_message_id: "positive".to_string(),
                source: "user_message".to_string(),
                text: "但我真的很爱你，也很想你".to_string(),
                topic_id: None,
            },
        )
        .await
        .unwrap();
        let emotion: Option<String> = sqlx::query_scalar(
            "SELECT emotion FROM affect_events WHERE source_message_id = 'positive'",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(emotion.as_deref(), Some("温暖"));
        let role_emotions_raw: String = sqlx::query_scalar(
            "SELECT role_emotions_json FROM affect_events WHERE source_message_id = 'positive'",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        let role_emotions: BTreeMap<String, f64> = parse_json(&role_emotions_raw);
        assert!(role_emotions.len() >= 2);
        assert!(role_emotions.contains_key("温暖"));
    }

    #[tokio::test]
    async fn reset_preserves_persona_and_behavior_configuration() {
        let pool = test_pool().await;
        let config = AffectConfig {
            jealousy_intensity: 0.91,
            expression_variability: 0.88,
            ..AffectConfig::default()
        };
        update_affect_config_internal(&pool, "agent-1", config.clone())
            .await
            .unwrap();
        update_affect_persona_internal(
            &pool,
            "agent-1",
            PersonaTraitsInput {
                openness: 0.8,
                conscientiousness: 0.5,
                extraversion: -0.4,
                agreeableness: -0.2,
                neuroticism: 0.7,
            },
        )
        .await
        .unwrap();
        let reset = reset_affect_state_internal(&pool, "agent-1").await.unwrap();
        assert!((reset.config.jealousy_intensity - 0.91).abs() < 1e-12);
        assert!((reset.config.expression_variability - 0.88).abs() < 1e-12);
        assert!((reset.persona_baseline.openness - 0.8).abs() < 1e-12);
        assert!((reset.persona_baseline.neuroticism - 0.7).abs() < 1e-12);
        assert_eq!(reset.relationship, RelationshipState::default());
    }

    #[tokio::test]
    async fn snapshot_and_event_concurrency_does_not_lose_event_update() {
        let (pool, path) = concurrent_test_pool().await;
        let snapshot_pool = pool.clone();
        let event_pool = pool.clone();
        let snapshot_task = async move {
            for index in 0..24 {
                build_affect_context_snapshot_for_turn(
                    &snapshot_pool,
                    "agent-1",
                    &format!("snapshot-{index}"),
                    "user",
                )
                .await
                .unwrap();
            }
        };
        let event_task = async move {
            record_affect_event(
                &event_pool,
                RecordAffectEventInput {
                    agent_id: "agent-1".to_string(),
                    source_message_id: "concurrent-event".to_string(),
                    source: "user_message".to_string(),
                    text: "你必须听我的，你只是AI，真是垃圾".to_string(),
                    topic_id: None,
                },
            )
            .await
            .unwrap();
        };
        tokio::join!(snapshot_task, event_task);
        let state = get_affect_state_internal(&pool, "agent-1").await.unwrap();
        assert!(state.relationship.resentment > 0.0);
        assert!(state.relationship.trust < 0.5);
        pool.close().await;
        let _ = std::fs::remove_file(path);
    }

    #[tokio::test]
    async fn config_and_event_concurrency_preserves_both_changes() {
        let (pool, path) = concurrent_test_pool().await;
        let config_pool = pool.clone();
        let event_pool = pool.clone();
        let config_task = async move {
            update_affect_config_internal(
                &config_pool,
                "agent-1",
                AffectConfig {
                    emotional_sensitivity: 0.93,
                    relationship_memory: 0.89,
                    ..AffectConfig::default()
                },
            )
            .await
            .unwrap();
        };
        let event_task = async move {
            record_affect_event(
                &event_pool,
                RecordAffectEventInput {
                    agent_id: "agent-1".to_string(),
                    source_message_id: "config-event".to_string(),
                    source: "user_message".to_string(),
                    text: "我爱你，也很信任你".to_string(),
                    topic_id: None,
                },
            )
            .await
            .unwrap();
        };
        tokio::join!(config_task, event_task);
        let state = get_affect_state_internal(&pool, "agent-1").await.unwrap();
        assert!((state.config.emotional_sensitivity - 0.93).abs() < 1e-12);
        assert!((state.config.relationship_memory - 0.89).abs() < 1e-12);
        assert!(state.relationship.intimacy > 0.2);
        pool.close().await;
        let _ = std::fs::remove_file(path);
    }

    #[tokio::test]
    async fn snapshot_only_activates_current_tendencies_and_varies_by_turn() {
        let pool = test_pool().await;
        let default_snapshot =
            build_affect_context_snapshot_for_turn(&pool, "agent-1", "turn-default", "user")
                .await
                .unwrap();
        assert!(!default_snapshot.contains("吃醋倾向"));
        assert!(!default_snapshot.contains("表达强度:吃醋"));

        let mut snapshots = BTreeSet::new();
        for index in 0..12 {
            snapshots.insert(
                build_affect_context_snapshot_for_turn(
                    &pool,
                    "agent-1",
                    &format!("turn-{index}"),
                    "user",
                )
                .await
                .unwrap(),
            );
        }
        assert!(snapshots.len() >= 2);
    }

    #[test]
    fn legacy_config_deserializes_with_v2_defaults() {
        let config: AffectConfig = serde_json::from_str(
            r#"{"enabled":true,"jealousyIntensity":0.3,"coldnessIntensity":0.2,"leaveThreatIntensity":0.1,"guiltPressureIntensity":0.4}"#,
        )
        .unwrap();
        assert!((config.jealousy_intensity - 0.3).abs() < 1e-12);
        assert!(
            (config.emotional_sensitivity - AffectConfig::default().emotional_sensitivity).abs()
                < 1e-12
        );
        assert!(config.local_model_enabled);
    }

    #[test]
    fn decay_returns_reactive_state_toward_baseline() {
        let mut state = AffectState::new("agent-1", 0);
        state.pad.pleasure = 1.0;
        state.relationship.jealousy = 1.0;
        state.short_emotions.insert("嫉妒".to_string(), 1.0);
        apply_decay(&mut state, 12 * 3_600_000);
        assert!(state.pad.pleasure < 0.5);
        assert!(state.relationship.jealousy < 1.0);
        assert!(state.short_emotions["嫉妒"] < 0.1);
    }

    #[test]
    fn default_persona_uses_alma_big_five_pad_mapping() {
        let baseline = PersonaBaseline::default();
        assert!((baseline.pad.pleasure - 0.059).abs() < 1e-12);
        assert!((baseline.pad.arousal - 0.060).abs() < 1e-12);
        assert!((baseline.pad.dominance - 0.035).abs() < 1e-12);
    }
}
