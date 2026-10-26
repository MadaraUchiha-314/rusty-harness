//! The `SystemOne` interface for fast judgements (R9.4, Q4).
//!
//! A System One model answers a classification question with a label and a confidence.
//! Two providers: [`HttpSystemOne`] posts the judgement as JSON to a typed endpoint (the
//! generic shape until a concrete API is known), and [`LlmSystemOne`] asks an [`Llm`]
//! through the `system-one/classify` prompt fragment.

use std::sync::Arc;

use bytes::Bytes;
use serde::{Deserialize, Serialize};

use super::llm::{ContextMessage, Llm, MessageRole, ModelError, ModelRequest, collect};
use crate::assets::{DataRole, Prompt, PromptArgs, PromptRegistry};
use crate::config::SystemOneConfig;
use crate::harness_async_trait;
use crate::host::{Host, HttpClient, HttpRequest, Secret};
use crate::registry::{Entity, EntityId, EntityKind, Placement, RemoteEndpoint};

/// A fast question: pick one label for a text.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Judgement {
    /// What to decide.
    pub question: String,
    /// The allowed answers.
    pub labels: Vec<String>,
    /// The text to judge.
    pub text: String,
}

/// The answer.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Verdict {
    /// One of the labels.
    pub label: String,
    /// Confidence in `[0, 1]`, when the provider reports one.
    pub confidence: Option<f64>,
}

harness_async_trait! {
    /// A System One model (R9.4): classification, confidence, routing.
    pub trait SystemOne: Entity {
        /// Decides `judgement`.
        async fn judge(&self, judgement: Judgement) -> Result<Verdict, ModelError>;
    }
}

/// A typed classifier endpoint: `POST <url>` with the judgement as JSON, a verdict back.
pub struct HttpSystemOne {
    id: EntityId,
    url: url::Url,
    api_key: Option<Secret<String>>,
    http: Arc<dyn HttpClient>,
}

impl core::fmt::Debug for HttpSystemOne {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("HttpSystemOne")
            .field("id", &self.id)
            .field("url", &self.url)
            .finish_non_exhaustive()
    }
}

impl HttpSystemOne {
    /// Builds the provider for `[system_one.<alias>]`.
    ///
    /// # Errors
    ///
    /// [`ModelError::NonRetryable`] when the credential cannot be resolved or the host
    /// has no HTTP client.
    pub async fn from_config(
        alias: &str,
        config: &SystemOneConfig,
        host: &Host,
    ) -> Result<Self, ModelError> {
        let http = host.http().map_err(|e| ModelError::NonRetryable {
            status: None,
            message: e.to_string(),
        })?;
        let api_key =
            match &config.api_key {
                Some(reference) => {
                    let secrets = host.secrets().map_err(|e| ModelError::NonRetryable {
                        status: None,
                        message: e.to_string(),
                    })?;
                    Some(secrets.resolve(reference).await.map_err(|e| {
                        ModelError::NonRetryable {
                            status: None,
                            message: e.to_string(),
                        }
                    })?)
                }
                None => None,
            };
        Ok(Self {
            id: EntityId::new(alias),
            url: config.url.clone(),
            api_key,
            http,
        })
    }
}

impl Entity for HttpSystemOne {
    fn kind(&self) -> EntityKind {
        EntityKind::SystemOne
    }
    fn entity_id(&self) -> &EntityId {
        &self.id
    }
    fn placement(&self) -> Placement {
        Placement::Remote(RemoteEndpoint::Http {
            base_url: self.url.clone(),
        })
    }
}

harness_async_trait! {
    impl SystemOne for HttpSystemOne {
        async fn judge(&self, judgement: Judgement) -> Result<Verdict, ModelError> {
            let mut request = HttpRequest::new(http::Method::POST, self.url.clone());
            request.headers.insert(http::header::CONTENT_TYPE, http::HeaderValue::from_static("application/json"));
            if let Some(key) = &self.api_key {
                let value = http::HeaderValue::from_str(&format!("Bearer {}", key.expose()))
                    .map_err(|_| ModelError::NonRetryable { status: None, message: "credential is not a valid header value".into() })?;
                request.headers.insert(http::header::AUTHORIZATION, value);
            }
            request.body = Bytes::from(serde_json::to_vec(&judgement).map_err(|e| ModelError::Decode(e.to_string()))?);
            let response = self.http.send(request).await.map_err(|e| ModelError::Retryable { status: None, message: e.to_string(), retry_after: None })?;
            if !response.status.is_success() {
                return Err(ModelError::from_status(response.status.as_u16(), String::from_utf8_lossy(&response.body).into_owned(), None));
            }
            let verdict: Verdict = serde_json::from_slice(&response.body).map_err(|e| ModelError::Decode(e.to_string()))?;
            if !judgement.labels.contains(&verdict.label) {
                return Err(ModelError::Decode(format!("label {:?} is not one of the allowed labels", verdict.label)));
            }
            Ok(verdict)
        }
    }
}

/// A System One backed by a language model and the `system-one/classify` fragment.
pub struct LlmSystemOne {
    id: EntityId,
    llm: Arc<dyn Llm>,
    prompts: Arc<PromptRegistry>,
}

impl core::fmt::Debug for LlmSystemOne {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("LlmSystemOne")
            .field("id", &self.id)
            .finish_non_exhaustive()
    }
}

impl LlmSystemOne {
    /// A System One asking `llm`.
    #[must_use]
    pub fn new(alias: &str, llm: Arc<dyn Llm>, prompts: Arc<PromptRegistry>) -> Self {
        Self {
            id: EntityId::new(alias),
            llm,
            prompts,
        }
    }
}

impl Entity for LlmSystemOne {
    fn kind(&self) -> EntityKind {
        EntityKind::SystemOne
    }
    fn entity_id(&self) -> &EntityId {
        &self.id
    }
}

harness_async_trait! {
    impl SystemOne for LlmSystemOne {
        async fn judge(&self, judgement: Judgement) -> Result<Verdict, ModelError> {
            let fragment = self.prompts.get("system-one/classify", "en").ok_or_else(|| ModelError::NonRetryable { status: None, message: "prompt system-one/classify is not registered".into() })?;
            let args = PromptArgs::new().with("question", judgement.question.clone()).with("labels", judgement.labels.join(", ")).with("text", judgement.text.clone());
            let instructions = Prompt::fragment(&fragment, &args).map_err(|e| ModelError::NonRetryable { status: None, message: e.to_string() })?;
            let request = ModelRequest::new(instructions, vec![ContextMessage::text(MessageRole::User, Prompt::data(DataRole::UserMessage, &judgement.text).render())]);
            let turn = collect(self.llm.stream(request).await?, None).await?;
            let answer = turn.text.trim().trim_matches(|c: char| c == '"' || c == '.').to_owned();
            let label = judgement
                .labels
                .iter()
                .find(|l| l.eq_ignore_ascii_case(&answer))
                .cloned()
                .ok_or_else(|| ModelError::Decode(format!("the model answered {answer:?}, not one of the labels")))?;
            Ok(Verdict { label, confidence: None })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{HttpSystemOne, Judgement, LlmSystemOne, SystemOne, Verdict};
    use crate::assets::PromptRegistry;
    use crate::config::SystemOneConfig;
    use crate::harness::models::llm::testing::RecordedLlm;
    use crate::harness::models::llm::{ModelEvent, StopReason};
    use crate::host::testing::{MapSecrets, MockHttp};
    use crate::host::{Host, HttpResponse, SecretRef};
    use std::sync::Arc;

    fn judgement() -> Judgement {
        Judgement {
            question: "Is this a bug report?".into(),
            labels: vec!["bug".into(), "feature".into()],
            text: "it crashes".into(),
        }
    }

    #[tokio::test]
    async fn the_http_provider_posts_the_judgement_and_checks_the_label() {
        let http = Arc::new(MockHttp::new(|req| {
            let body: serde_json::Value = serde_json::from_slice(&req.body).unwrap();
            assert_eq!(body["labels"][1], "feature");
            assert_eq!(req.headers["authorization"], "Bearer jev-canary");
            Ok(HttpResponse {
                status: http::StatusCode::OK,
                headers: http::HeaderMap::new(),
                body: "{\"label\":\"bug\",\"confidence\":0.9}".into(),
            })
        }));
        let host = Host::builder()
            .http(http)
            .secrets(Arc::new(MapSecrets::new([(
                SecretRef::Env("JEV".into()),
                "jev-canary".into(),
            )])))
            .build();
        let config = SystemOneConfig {
            provider: "http".into(),
            url: "https://jev.example/classify".parse().unwrap(),
            api_key: Some(SecretRef::Env("JEV".into())),
        };
        let s1 = HttpSystemOne::from_config("default", &config, &host)
            .await
            .expect("builds");
        assert_eq!(
            s1.judge(judgement()).await.unwrap(),
            Verdict {
                label: "bug".into(),
                confidence: Some(0.9)
            }
        );
        let bad = Arc::new(MockHttp::always(
            http::StatusCode::OK,
            "{\"label\":\"other\"}",
        ));
        let host = Host::builder().http(bad).build();
        let config = SystemOneConfig {
            provider: "http".into(),
            url: "https://jev.example/classify".parse().unwrap(),
            api_key: None,
        };
        assert!(
            HttpSystemOne::from_config("x", &config, &host)
                .await
                .unwrap()
                .judge(judgement())
                .await
                .is_err()
        );
    }

    #[tokio::test]
    async fn the_llm_provider_uses_the_classify_fragment_and_maps_the_answer() {
        let llm = Arc::new(RecordedLlm::new(
            "rec",
            vec![vec![
                ModelEvent::TextDelta {
                    text: "Bug.".into(),
                },
                ModelEvent::Stop {
                    reason: StopReason::EndTurn,
                },
            ]],
        ));
        let s1 = LlmSystemOne::new("s1", llm.clone(), Arc::new(PromptRegistry::builtin()));
        assert_eq!(s1.judge(judgement()).await.unwrap().label, "bug");
        let req = &llm.requests()[0];
        assert_eq!(
            req.instructions.provenance()[0].id.as_ref(),
            "system-one/classify"
        );
        assert!(req.instructions.render().contains("bug, feature"));
    }
}
