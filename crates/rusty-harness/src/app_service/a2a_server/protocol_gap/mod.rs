//! What A2A 1.0 defines and the SDK does not yet implement (R19.9): `A2A-Extensions`
//! negotiation and the authenticated caller, carried through `ServiceParams` under
//! reserved keys. Everything here is tracked upstream and removed when the SDK ships it;
//! nothing here speaks an older protocol version.

use a2a_server::ServiceParams;

/// The service parameter the SDK fills from the `A2A-Extensions` header.
pub const EXTENSIONS_PARAM: &str = "a2a-extensions";
/// The reserved key the negotiator writes the active set to (task 21).
pub const ACTIVE_EXTENSIONS_PARAM: &str = "x-rusty-harness-active-extensions";
/// The reserved key the authorizer writes the caller's id to (task 21).
pub const CALLER_PARAM: &str = "x-rusty-harness-caller";

/// The extensions a request named, as the header lists them (comma separated).
#[must_use]
pub fn requested_extensions(params: &ServiceParams) -> Vec<String> {
    params
        .get(EXTENSIONS_PARAM)
        .into_iter()
        .flatten()
        .flat_map(|v| v.split(','))
        .map(|s| s.trim().to_owned())
        .filter(|s| !s.is_empty())
        .collect()
}

/// The active set: what the negotiator wrote, else what the request named.
#[must_use]
pub fn active_extensions(params: &ServiceParams) -> Vec<String> {
    match params.get(ACTIVE_EXTENSIONS_PARAM) {
        Some(values) => values
            .iter()
            .flat_map(|v| v.split(','))
            .map(|s| s.trim().to_owned())
            .filter(|s| !s.is_empty())
            .collect(),
        None => requested_extensions(params),
    }
}

/// The authenticated caller's id, when the authorizer set one.
#[must_use]
pub fn caller_id(params: &ServiceParams) -> Option<String> {
    params.get(CALLER_PARAM).and_then(|v| v.first()).cloned()
}
