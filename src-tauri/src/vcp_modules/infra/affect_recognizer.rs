use serde::{Deserialize, Serialize};
use std::{future::Future, time::Duration};
use tauri::AppHandle;

const MAX_MODEL_INPUT_CHARS: usize = 1_000;
// Android currently enforces a 2,500 ms classifier timeout. Keep a small
// allowance for the Tauri bridge and scheduling while still bounding model
// initialization and executor queue stalls from the Rust caller's side.
const MODEL_CALL_TIMEOUT: Duration = Duration::from_millis(2_750);

async fn with_model_timeout<T, F>(future: F, timeout: Duration) -> Result<T, String>
where
    F: Future<Output = T>,
{
    tokio::time::timeout(timeout, future).await.map_err(|_| {
        format!(
            "affect classifier timed out after {}ms",
            timeout.as_millis()
        )
    })
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(default, rename_all = "camelCase")]
pub struct ModelEmotionScores {
    pub neutral: f64,
    pub joy: f64,
    pub sadness: f64,
    pub anger: f64,
    pub confusion: f64,
    pub disgust: f64,
    pub surprise: f64,
    pub affection: f64,
}

impl ModelEmotionScores {
    fn values_mut(&mut self) -> [&mut f64; 8] {
        [
            &mut self.neutral,
            &mut self.joy,
            &mut self.sadness,
            &mut self.anger,
            &mut self.confusion,
            &mut self.disgust,
            &mut self.surprise,
            &mut self.affection,
        ]
    }

    pub fn ranked(&self) -> [(&'static str, f64); 8] {
        let mut scores = [
            ("neutral", self.neutral),
            ("joy", self.joy),
            ("sadness", self.sadness),
            ("anger", self.anger),
            ("confusion", self.confusion),
            ("disgust", self.disgust),
            ("surprise", self.surprise),
            ("affection", self.affection),
        ];
        scores.sort_by(|left, right| right.1.total_cmp(&left.1));
        scores
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ModelAffectObservation {
    pub model_id: String,
    pub model_version: String,
    pub scores: ModelEmotionScores,
    #[serde(default)]
    pub inference_ms: u64,
    #[serde(default)]
    pub truncated: bool,
}

impl ModelAffectObservation {
    pub fn validated(mut self) -> Option<Self> {
        self.model_id = self.model_id.trim().chars().take(128).collect();
        self.model_version = self.model_version.trim().chars().take(128).collect();
        if self.model_id.is_empty() || self.model_version.is_empty() {
            return None;
        }

        let mut sum = 0.0;
        for value in self.scores.values_mut() {
            if !value.is_finite() || !(0.0..=1.0).contains(value) {
                return None;
            }
            sum += *value;
        }
        if sum <= f64::EPSILON {
            return None;
        }
        for value in self.scores.values_mut() {
            *value /= sum;
        }
        Some(self)
    }

    pub fn top_label_score_margin(&self) -> (&'static str, f64, f64) {
        let ranked = self.scores.ranked();
        (ranked[0].0, ranked[0].1, ranked[0].1 - ranked[1].1)
    }

    pub fn recognizer_provenance(&self) -> String {
        format!("{}@{}", self.model_id, self.model_version)
    }
}

pub async fn observe_model_affect(
    app: &AppHandle,
    text: &str,
) -> Result<Option<ModelAffectObservation>, String> {
    let text = text.trim();
    if text.is_empty() {
        return Ok(None);
    }
    let truncated = text.chars().count() > MAX_MODEL_INPUT_CHARS;
    let limited_text: String = text.chars().take(MAX_MODEL_INPUT_CHARS).collect();
    let app = app.clone();

    let raw = with_model_timeout(
        tokio::task::spawn_blocking(move || {
            tauri_plugin_vcp_mobile::system::classify_affect_model(&app, &limited_text)
        }),
        MODEL_CALL_TIMEOUT,
    )
    .await?
    .map_err(|error| format!("affect classifier task failed: {error}"))??;

    let Some(raw) = raw else {
        return Ok(None);
    };
    let mut observation: ModelAffectObservation = serde_json::from_value(raw)
        .map_err(|error| format!("invalid affect classifier response: {error}"))?;
    observation.truncated |= truncated;
    observation
        .validated()
        .map(Some)
        .ok_or_else(|| "affect classifier returned invalid scores".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn observation(scores: ModelEmotionScores) -> ModelAffectObservation {
        ModelAffectObservation {
            model_id: "test-model".to_string(),
            model_version: "1".to_string(),
            scores,
            inference_ms: 12,
            truncated: false,
        }
    }

    #[test]
    fn validation_normalizes_scores_and_computes_margin() {
        let value = observation(ModelEmotionScores {
            joy: 0.8,
            sadness: 0.2,
            ..ModelEmotionScores::default()
        })
        .validated()
        .unwrap();
        let (label, score, margin) = value.top_label_score_margin();
        assert_eq!(label, "joy");
        assert!((score - 0.8).abs() < 1e-12);
        assert!((margin - 0.6).abs() < 1e-12);
    }

    #[test]
    fn validation_rejects_empty_identity_and_invalid_scores() {
        let mut missing_model = observation(ModelEmotionScores {
            neutral: 1.0,
            ..ModelEmotionScores::default()
        });
        missing_model.model_id.clear();
        assert!(missing_model.validated().is_none());

        assert!(observation(ModelEmotionScores {
            anger: -0.1,
            neutral: 1.0,
            ..ModelEmotionScores::default()
        })
        .validated()
        .is_none());

        assert!(observation(ModelEmotionScores {
            neutral: 1.1,
            ..ModelEmotionScores::default()
        })
        .validated()
        .is_none());
    }

    #[test]
    fn canonical_affection_score_is_preserved() {
        let value = observation(ModelEmotionScores {
            affection: 0.9,
            neutral: 0.1,
            ..ModelEmotionScores::default()
        })
        .validated()
        .unwrap();
        let (label, score, margin) = value.top_label_score_margin();
        assert_eq!(label, "affection");
        assert!((score - 0.9).abs() < 1e-12);
        assert!((margin - 0.8).abs() < 1e-12);
    }

    #[test]
    fn canonical_confusion_score_is_preserved_and_legacy_fear_is_ignored() {
        let value = observation(ModelEmotionScores {
            confusion: 0.85,
            neutral: 0.15,
            ..ModelEmotionScores::default()
        })
        .validated()
        .unwrap();
        let (label, score, margin) = value.top_label_score_margin();
        assert_eq!(label, "confusion");
        assert!((score - 0.85).abs() < 1e-12);
        assert!((margin - 0.70).abs() < 1e-12);

        let legacy: ModelAffectObservation = serde_json::from_str(
            r#"{"modelId":"legacy","modelVersion":"1","scores":{"neutral":0.2,"fear":0.8},"inferenceMs":1}"#,
        )
        .unwrap();
        assert_eq!(legacy.scores.confusion, 0.0);
        let legacy = legacy.validated().unwrap();
        assert_eq!(legacy.top_label_score_margin().0, "neutral");
    }

    #[tokio::test]
    async fn model_timeout_returns_without_waiting_for_blocked_work() {
        let started = tokio::time::Instant::now();
        let result = with_model_timeout(
            async {
                tokio::time::sleep(Duration::from_millis(200)).await;
                "late"
            },
            Duration::from_millis(10),
        )
        .await;

        assert_eq!(
            result.unwrap_err(),
            "affect classifier timed out after 10ms"
        );
        assert!(started.elapsed() < Duration::from_millis(150));
    }
}
