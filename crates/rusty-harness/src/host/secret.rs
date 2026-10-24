//! Credentials by reference: `SecretRef` names where a secret lives, `Secret<T>` holds a
//! resolved value that cannot be printed or serialized (design.md § Security design,
//! trust boundary 6).

use core::fmt;
use core::str::FromStr;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use zeroize::Zeroize;

/// Where a credential comes from. Never the credential itself.
///
/// Parsed from `env:NAME`, `keyring:<service>/<user>` or `file:<path>`; a configuration
/// value that is not one of these is refused, so a literal key cannot sit in a config file.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub enum SecretRef {
    /// An environment variable.
    Env(String),
    /// An entry of the operator's keyring (GNOME keyring through `secret-tool` natively).
    Keyring {
        /// The keyring's service attribute.
        service: String,
        /// The keyring's username attribute.
        user: String,
    },
    /// A file whose trimmed contents are the secret (a mounted secret, for example).
    File(PathBuf),
}

/// A reference that does not parse (design.md § Security design: fail closed). Carries
/// nothing: the offending text may be a real credential, so it is never echoed.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error(
    "not a credential reference (expected env:NAME, keyring:SERVICE/USER or file:PATH); the value is not shown"
)]
pub struct InvalidSecretRef;

impl schemars::JsonSchema for SecretRef {
    fn schema_name() -> std::borrow::Cow<'static, str> {
        "SecretRef".into()
    }

    fn json_schema(_generator: &mut schemars::SchemaGenerator) -> schemars::Schema {
        schemars::json_schema!({
            "type": "string",
            "format": "secret-ref",
            "pattern": "^(env:[A-Za-z0-9_]+|keyring:[^/]+/.+|file:.+)$",
            "description": "A credential by reference: env:NAME, keyring:SERVICE/USER or file:PATH. Never a literal value."
        })
    }
}

impl FromStr for SecretRef {
    type Err = InvalidSecretRef;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if let Some(name) = s.strip_prefix("env:")
            && !name.is_empty()
            && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
        {
            return Ok(Self::Env(name.to_owned()));
        }
        if let Some(rest) = s.strip_prefix("keyring:")
            && let Some((service, user)) = rest.split_once('/')
            && !service.is_empty()
            && !user.is_empty()
        {
            return Ok(Self::Keyring {
                service: service.to_owned(),
                user: user.to_owned(),
            });
        }
        if let Some(path) = s.strip_prefix("file:")
            && !path.is_empty()
        {
            return Ok(Self::File(PathBuf::from(path)));
        }
        Err(InvalidSecretRef)
    }
}

impl TryFrom<String> for SecretRef {
    type Error = InvalidSecretRef;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        value.parse()
    }
}

impl From<SecretRef> for String {
    fn from(value: SecretRef) -> Self {
        value.to_string()
    }
}

impl fmt::Display for SecretRef {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Env(name) => write!(f, "env:{name}"),
            Self::Keyring { service, user } => write!(f, "keyring:{service}/{user}"),
            Self::File(path) => write!(f, "file:{}", path.display()),
        }
    }
}

/// A resolved credential: readable only through [`Secret::expose`], printed as
/// `[redacted]`, wiped from memory on drop, and never serializable.
///
/// ```compile_fail
/// // `Secret` has no `Serialize` impl, so it can never reach a config file, a task
/// // state, a Temporal payload or a trace (design.md § Security design).
/// let secret = rusty_harness::host::Secret::new(String::from("sk-123"));
/// let _ = serde_json::to_string(&secret);
/// ```
pub struct Secret<T: Zeroize>(T);

impl<T: Zeroize> Secret<T> {
    /// Wraps a value; the only way to make one.
    #[must_use]
    pub fn new(value: T) -> Self {
        Self(value)
    }

    /// The value, for the one call that needs it (an `Authorization` header, a client
    /// constructor). Never store what this returns.
    #[must_use]
    pub fn expose(&self) -> &T {
        &self.0
    }
}

impl<T: Zeroize> Drop for Secret<T> {
    fn drop(&mut self) {
        self.0.zeroize();
    }
}

impl<T: Zeroize> fmt::Debug for Secret<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("[redacted]")
    }
}

impl<T: Zeroize> fmt::Display for Secret<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("[redacted]")
    }
}

/// A reference could not be resolved. Carries the reference, never a value.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum SecretError {
    /// Nothing is behind the reference, or it could not be read.
    #[error("credential reference {reference} could not be resolved: {reason}")]
    Unresolvable {
        /// The reference, as written in configuration.
        reference: String,
        /// Why, without the value.
        reason: String,
    },
    /// The host supplies no resolver for this kind of reference.
    #[error("credential reference {reference} needs a resolver this host does not supply")]
    Unsupported {
        /// The reference, as written in configuration.
        reference: String,
    },
}

#[cfg(test)]
mod tests {
    use super::{Secret, SecretRef};
    use std::path::PathBuf;

    #[test]
    fn references_parse_and_print_round_trip() {
        for (text, expected) in [
            (
                "env:OPENAI_API_KEY",
                SecretRef::Env("OPENAI_API_KEY".into()),
            ),
            (
                "keyring:rusty-harness/openai",
                SecretRef::Keyring {
                    service: "rusty-harness".into(),
                    user: "openai".into(),
                },
            ),
            (
                "file:/run/secrets/openai",
                SecretRef::File(PathBuf::from("/run/secrets/openai")),
            ),
        ] {
            let parsed: SecretRef = text.parse().expect("parses");
            assert_eq!(parsed, expected);
            assert_eq!(parsed.to_string(), text);
            let json = serde_json::to_string(&parsed).expect("serializes as its text");
            assert_eq!(json, format!("{text:?}"));
        }
    }

    #[test]
    fn literals_are_refused() {
        for text in [
            "sk-live-123",
            "env:",
            "keyring:nouser",
            "file:",
            "ENV:X",
            "env:not valid",
        ] {
            assert!(
                text.parse::<SecretRef>().is_err(),
                "{text:?} must not parse"
            );
        }
        assert!(serde_json::from_str::<SecretRef>("\"sk-live-123\"").is_err());
    }

    #[test]
    fn a_secret_never_prints_its_value() {
        let secret = Secret::new(String::from("sk-canary-7"));
        assert_eq!(format!("{secret:?}"), "[redacted]");
        assert_eq!(secret.to_string(), "[redacted]");
        assert_eq!(secret.expose(), "sk-canary-7");
    }
}
