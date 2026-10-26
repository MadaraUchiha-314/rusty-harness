//! The payload codec that keeps secrets out of Temporal payloads (boundary 6).
//!
//! Credentials never enter a payload by type: `Secret<T>` has no `Serialize` impl and
//! activities receive `SecretRef`s. This codec is the belt to that brace: every secret the
//! harness resolved is registered on a [`SecretGuard`], and a payload whose bytes contain
//! one of them is refused before it leaves the process, so a history can never hold it.

use std::sync::{Arc, PoisonError, RwLock};

use futures::future::BoxFuture;
use temporalio_common::data_converters::{
    DataConverter, PayloadCodec, PayloadConversionError, PayloadConverter, SerializationContextData,
};
use temporalio_common::protos::temporal::api::common::v1::Payload;

use crate::host::Secret;

/// The values no payload may contain.
#[derive(Clone, Debug, Default)]
pub struct SecretGuard {
    values: Arc<RwLock<Vec<Vec<u8>>>>,
}

/// What the guard found.
#[derive(Debug, thiserror::Error)]
#[error("a payload would carry a credential; refused (boundary 6)")]
pub struct SecretInPayload;

impl SecretGuard {
    /// A guard protecting nothing yet.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Registers a resolved secret; payloads containing it are refused from now on.
    pub fn protect(&self, secret: &Secret<String>) {
        let value = secret.expose().as_bytes().to_vec();
        if value.is_empty() {
            return;
        }
        self.values
            .write()
            .unwrap_or_else(PoisonError::into_inner)
            .push(value);
    }

    /// Whether `bytes` contain a protected value.
    #[must_use]
    pub fn contains_secret(&self, bytes: &[u8]) -> bool {
        self.values
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .iter()
            .any(|v| bytes.windows(v.len()).any(|w| w == v.as_slice()))
    }

    /// The SDK data converter: the default converters with this guard as the codec.
    #[must_use]
    pub fn data_converter(&self) -> DataConverter {
        DataConverter::new(
            PayloadConverter::default(),
            temporalio_common::data_converters::DefaultFailureConverter::new(true),
            self.clone(),
        )
    }
}

impl PayloadCodec for SecretGuard {
    fn encode(
        &self,
        _context: &SerializationContextData,
        payloads: Vec<Payload>,
    ) -> BoxFuture<'static, Result<Vec<Payload>, PayloadConversionError>> {
        let leak = payloads.iter().any(|p| {
            self.contains_secret(&p.data) || p.metadata.values().any(|m| self.contains_secret(m))
        });
        Box::pin(async move {
            if leak {
                Err(PayloadConversionError::EncodingError(Box::new(
                    SecretInPayload,
                )))
            } else {
                Ok(payloads)
            }
        })
    }

    fn decode(
        &self,
        _context: &SerializationContextData,
        payloads: Vec<Payload>,
    ) -> BoxFuture<'static, Result<Vec<Payload>, PayloadConversionError>> {
        Box::pin(async move { Ok(payloads) })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn payload(data: &str) -> Payload {
        Payload {
            data: data.as_bytes().to_vec(),
            ..Default::default()
        }
    }

    #[tokio::test]
    async fn a_payload_carrying_a_protected_secret_is_refused_and_others_pass() {
        let guard = SecretGuard::new();
        guard.protect(&Secret::new("sk-canary-77".to_owned()));
        let cx = SerializationContextData::None;
        let ok = guard
            .encode(&cx, vec![payload(r#"{"reference":"env:OPENAI_API_KEY"}"#)])
            .await
            .expect("a reference passes");
        assert_eq!(ok.len(), 1);
        let err = guard
            .encode(&cx, vec![payload(r#"{"key":"sk-canary-77"}"#)])
            .await
            .expect_err("the value is refused");
        assert!(err.to_string().contains("credential"), "{err}");
        assert!(
            !err.to_string().contains("sk-canary"),
            "the error names no value"
        );
        let decoded = guard.decode(&cx, vec![payload("x")]).await.unwrap();
        assert_eq!(decoded[0].data, b"x");
    }
}
