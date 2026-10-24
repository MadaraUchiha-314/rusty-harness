//! Durations in configuration, written as `30s`, `500ms`, `15m`, `2h`.

use core::fmt;
use core::str::FromStr;
use core::time::Duration;

use serde::{Deserialize, Serialize};

/// A [`Duration`] that reads and writes as `<number><unit>` with `ms`, `s`, `m` or `h`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct DurationStr(pub Duration);

impl DurationStr {
    /// Seconds.
    #[must_use]
    pub const fn secs(n: u64) -> Self {
        Self(Duration::from_secs(n))
    }
}

impl From<DurationStr> for Duration {
    fn from(value: DurationStr) -> Self {
        value.0
    }
}

/// Text that is not `<number><ms|s|m|h>`.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("not a duration: {0:?} (expected a number followed by ms, s, m or h)")]
pub struct InvalidDuration(String);

impl FromStr for DurationStr {
    type Err = InvalidDuration;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let s = s.trim();
        let split = s
            .find(|c: char| !c.is_ascii_digit())
            .ok_or_else(|| InvalidDuration(s.to_owned()))?;
        let (digits, unit) = s.split_at(split);
        let n: u64 = digits.parse().map_err(|_| InvalidDuration(s.to_owned()))?;
        let millis = match unit {
            "ms" => n,
            "s" => n
                .checked_mul(1000)
                .ok_or_else(|| InvalidDuration(s.to_owned()))?,
            "m" => n
                .checked_mul(60_000)
                .ok_or_else(|| InvalidDuration(s.to_owned()))?,
            "h" => n
                .checked_mul(3_600_000)
                .ok_or_else(|| InvalidDuration(s.to_owned()))?,
            _ => return Err(InvalidDuration(s.to_owned())),
        };
        Ok(Self(Duration::from_millis(millis)))
    }
}

impl TryFrom<String> for DurationStr {
    type Error = InvalidDuration;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        value.parse()
    }
}

impl From<DurationStr> for String {
    fn from(value: DurationStr) -> Self {
        value.to_string()
    }
}

impl fmt::Display for DurationStr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let ms = self.0.as_millis();
        if ms > 0 && ms.is_multiple_of(3_600_000) {
            write!(f, "{}h", ms / 3_600_000)
        } else if ms > 0 && ms.is_multiple_of(60_000) {
            write!(f, "{}m", ms / 60_000)
        } else if ms.is_multiple_of(1000) {
            write!(f, "{}s", ms / 1000)
        } else {
            write!(f, "{ms}ms")
        }
    }
}

impl schemars::JsonSchema for DurationStr {
    fn schema_name() -> std::borrow::Cow<'static, str> {
        "Duration".into()
    }

    fn json_schema(_generator: &mut schemars::SchemaGenerator) -> schemars::Schema {
        schemars::json_schema!({
            "type": "string",
            "pattern": "^[0-9]+(ms|s|m|h)$",
            "description": "A duration: a number followed by ms, s, m or h, e.g. 30s or 15m."
        })
    }
}

#[cfg(test)]
mod tests {
    use super::DurationStr;
    use core::time::Duration;

    #[test]
    fn parses_and_prints_the_four_units() {
        for (text, secs) in [
            ("500ms", 0.5),
            ("30s", 30.0),
            ("15m", 900.0),
            ("2h", 7200.0),
        ] {
            let d: DurationStr = text.parse().expect("parses");
            assert_eq!(d.0, Duration::from_secs_f64(secs));
            assert_eq!(d.to_string(), text);
        }
        for bad in ["", "10", "s", "1.5s", "10d", "-1s"] {
            assert!(bad.parse::<DurationStr>().is_err(), "{bad:?}");
        }
    }
}
