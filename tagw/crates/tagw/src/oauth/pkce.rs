//! PKCE helpers (S256).

use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use rand_core::{OsRng, RngCore};
use sha2::{Digest, Sha256};

use super::types::Pkce;

/// Generate a PKCE pair + random state for an OAuth start.
pub fn generate_pkce(redirect_uri: impl Into<String>) -> Pkce {
    let mut verifier_bytes = [0u8; 32];
    OsRng.fill_bytes(&mut verifier_bytes);
    let code_verifier = URL_SAFE_NO_PAD.encode(verifier_bytes);

    let mut hasher = Sha256::new();
    hasher.update(code_verifier.as_bytes());
    let challenge = URL_SAFE_NO_PAD.encode(hasher.finalize());

    let mut state_bytes = [0u8; 16];
    OsRng.fill_bytes(&mut state_bytes);
    let state = URL_SAFE_NO_PAD.encode(state_bytes);

    Pkce {
        code_verifier,
        code_challenge: challenge,
        state,
        redirect_uri: redirect_uri.into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pkce_challenge_is_s256() {
        let p = generate_pkce("http://localhost/callback");
        assert!(!p.code_verifier.is_empty());
        assert!(!p.code_challenge.is_empty());
        assert!(!p.state.is_empty());
        // Recompute
        let mut hasher = Sha256::new();
        hasher.update(p.code_verifier.as_bytes());
        let expected = URL_SAFE_NO_PAD.encode(hasher.finalize());
        assert_eq!(p.code_challenge, expected);
    }
}
