use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use rand::RngCore;
use sha2::{Digest, Sha256};

const CODE_VERIFIER_LENGTH: usize = 32;
const STATE_LENGTH: usize = 18;

pub fn random_b64url(length: usize) -> String {
    let mut bytes = vec![0u8; length];
    rand::thread_rng().fill_bytes(&mut bytes);
    URL_SAFE_NO_PAD
        .encode(&bytes)
        .trim_end_matches('=')
        .to_string()
}

fn sha256_b64url(data: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(data.as_bytes());
    let digest = hasher.finalize();
    URL_SAFE_NO_PAD
        .encode(digest)
        .trim_end_matches('=')
        .to_string()
}

pub fn generate_pkce_pair() -> (String, String) {
    let verifier = random_b64url(CODE_VERIFIER_LENGTH);
    let challenge = sha256_b64url(&verifier);
    (verifier, challenge)
}

pub fn generate_state() -> String {
    random_b64url(STATE_LENGTH)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_pkce_pair_length() {
        let (verifier, challenge) = generate_pkce_pair();
        assert!(
            verifier.len() >= 43,
            "verifier too short: {}",
            verifier.len()
        );
        assert!(
            verifier.len() <= 128,
            "verifier too long: {}",
            verifier.len()
        );
        assert_eq!(challenge.len(), 43);
    }

    #[test]
    fn test_pkce_verifier_charset() {
        let (verifier, _) = generate_pkce_pair();
        assert!(verifier.chars().all(|c| c.is_ascii_alphanumeric()
            || c == '-'
            || c == '.'
            || c == '_'
            || c == '~'));
    }

    #[test]
    fn test_state_length() {
        let state = generate_state();
        assert!(state.len() >= 18 && state.len() <= 128);
    }
}
