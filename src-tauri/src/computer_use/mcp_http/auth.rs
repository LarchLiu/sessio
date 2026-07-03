//! Per-session bearer-token auth for the HTTP MCP server.
//!
//! Each `computerUse` session gets a random bearer token. The token maps to the
//! Sessio session id; requests must present it and it must still be live. Tokens
//! are revoked on session end, so a stale or cross-session token is rejected.

use std::collections::HashMap;
use std::sync::Mutex;

/// An opaque per-session bearer token.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct SessionToken(pub String);

impl SessionToken {
    /// Generate a fresh random token (UUID v4, hyphen-free).
    pub fn generate() -> Self {
        SessionToken(uuid::Uuid::new_v4().simple().to_string())
    }
}

/// Reasons a request is rejected before any tool runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum AuthError {
    #[error("missing or malformed Authorization header")]
    MissingToken,
    #[error("token does not map to a live computer-use session")]
    UnknownToken,
    #[error("request did not originate from loopback")]
    NotLoopback,
}

/// Maps live bearer tokens to their Sessio session id. One registry is shared by
/// the whole process; one token per active `computerUse` session.
#[derive(Default)]
pub struct TokenRegistry {
    tokens: Mutex<HashMap<String, String>>,
}

impl TokenRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Issue and store a token for a session, returning it.
    pub fn issue(&self, session_id: &str) -> SessionToken {
        let token = SessionToken::generate();
        self.tokens
            .lock()
            .unwrap()
            .insert(token.0.clone(), session_id.to_string());
        token
    }

    /// Revoke a session's token(s). Idempotent.
    pub fn revoke_session(&self, session_id: &str) {
        self.tokens
            .lock()
            .unwrap()
            .retain(|_, sid| sid != session_id);
    }

    /// Resolve a presented bearer token to its session id, enforcing loopback.
    ///
    /// `is_loopback` is supplied by the transport layer (the peer address). The
    /// check lives here so it cannot be bypassed by a code path that forgets it.
    pub fn resolve(
        &self,
        authorization_header: Option<&str>,
        is_loopback: bool,
    ) -> Result<String, AuthError> {
        if !is_loopback {
            return Err(AuthError::NotLoopback);
        }
        let token = parse_bearer(authorization_header).ok_or(AuthError::MissingToken)?;
        self.tokens
            .lock()
            .unwrap()
            .get(token)
            .cloned()
            .ok_or(AuthError::UnknownToken)
    }

    #[cfg(test)]
    pub fn len(&self) -> usize {
        self.tokens.lock().unwrap().len()
    }

    #[cfg(test)]
    pub fn is_empty(&self) -> bool {
        self.tokens.lock().unwrap().is_empty()
    }
}

/// Extract the token from a `Bearer <token>` Authorization header value.
fn parse_bearer(header: Option<&str>) -> Option<&str> {
    let value = header?.trim();
    let rest = value
        .strip_prefix("Bearer ")
        .or_else(|| value.strip_prefix("bearer "))?;
    let token = rest.trim();
    if token.is_empty() {
        None
    } else {
        Some(token)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn issued_token_resolves_to_session() {
        let reg = TokenRegistry::new();
        let token = reg.issue("s1");
        let sid = reg
            .resolve(Some(&format!("Bearer {}", token.0)), true)
            .unwrap();
        assert_eq!(sid, "s1");
    }

    #[test]
    fn non_loopback_is_rejected_before_token_lookup() {
        let reg = TokenRegistry::new();
        let token = reg.issue("s1");
        assert_eq!(
            reg.resolve(Some(&format!("Bearer {}", token.0)), false),
            Err(AuthError::NotLoopback)
        );
    }

    #[test]
    fn missing_or_malformed_header_is_rejected() {
        let reg = TokenRegistry::new();
        assert_eq!(reg.resolve(None, true), Err(AuthError::MissingToken));
        assert_eq!(
            reg.resolve(Some("Basic abc"), true),
            Err(AuthError::MissingToken)
        );
        assert_eq!(
            reg.resolve(Some("Bearer "), true),
            Err(AuthError::MissingToken)
        );
    }

    #[test]
    fn revoked_token_is_rejected() {
        let reg = TokenRegistry::new();
        let token = reg.issue("s1");
        reg.revoke_session("s1");
        assert_eq!(
            reg.resolve(Some(&format!("Bearer {}", token.0)), true),
            Err(AuthError::UnknownToken)
        );
        assert_eq!(reg.len(), 0);
    }

    #[test]
    fn cross_session_token_reuse_is_rejected_after_revocation() {
        let reg = TokenRegistry::new();
        let t1 = reg.issue("s1");
        let _t2 = reg.issue("s2");
        // Revoking s1 must not affect s2's token, and t1 must stop working.
        reg.revoke_session("s1");
        assert!(reg
            .resolve(Some(&format!("Bearer {}", t1.0)), true)
            .is_err());
        assert_eq!(reg.len(), 1);
    }

    #[test]
    fn tokens_are_unique_per_issue() {
        let reg = TokenRegistry::new();
        let a = reg.issue("s1");
        let b = reg.issue("s2");
        assert_ne!(a.0, b.0);
    }
}
