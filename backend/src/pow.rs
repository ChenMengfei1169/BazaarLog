// Proof-of-work challenges for the public class-creation endpoint. A script
// must first fetch a fresh challenge, then present a solution whose SHA-256
// digest over (nonce + solution) starts with a configurable number of zero hex
// characters. Challenges are single-use and expire after five minutes, so the
// work cannot be precomputed or replayed. This is a light anti-abuse measure:
// a browser solves it in a few tens of milliseconds, while bulk creation
// scripts must spend CPU per class.
use std::collections::HashMap;
use std::sync::{Mutex, PoisonError};
use std::time::{Duration, Instant};

use argon2::password_hash::rand_core::{OsRng, RngCore};
use sha2::{Digest, Sha256};

use crate::encoding::encode_hex;

const CHALLENGE_TTL: Duration = Duration::from_secs(300);
const MAX_PENDING_CHALLENGES: usize = 1_000;

pub struct ChallengeStore {
    // Unspent nonces mapped to the instant each one expires.
    pending_challenges: Mutex<HashMap<String, Instant>>,
}

impl ChallengeStore {
    pub fn new() -> Self {
        Self {
            pending_challenges: Mutex::new(HashMap::new()),
        }
    }

    /// Issues a fresh single-use challenge nonce.
    ///
    /// Infallible by design. When the pending set is full the expired entries
    /// are dropped first and, if it is still full, the challenge closest to
    /// expiring is evicted to make room. Returning an error instead would let a
    /// burst from a handful of source addresses wedge class creation for every
    /// user.
    pub fn issue(&self) -> String {
        let mut nonce_bytes = [0u8; 16];
        OsRng.fill_bytes(&mut nonce_bytes);
        let nonce = encode_hex(&nonce_bytes);
        let now = Instant::now();
        let mut pending = self
            .pending_challenges
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        // Evict expired challenges before enforcing the cap so a burst of
        // requests cannot grow memory without limit.
        if pending.len() >= MAX_PENDING_CHALLENGES {
            pending.retain(|_, expires_at| *expires_at > now);
        }
        while pending.len() >= MAX_PENDING_CHALLENGES {
            let Some(soonest) = pending
                .iter()
                .min_by_key(|(_, expires_at)| **expires_at)
                .map(|(pending_nonce, _)| pending_nonce.clone())
            else {
                break;
            };
            pending.remove(&soonest);
        }
        pending.insert(nonce.clone(), now + CHALLENGE_TTL);
        nonce
    }

    /// Consumes the challenge (single use) and returns whether the solution
    /// satisfies the requested difficulty in leading zero hex characters.
    pub fn verify_solution(&self, nonce: &str, solution: &str, difficulty: usize) -> bool {
        if nonce.is_empty() || solution.is_empty() {
            return false;
        }
        let now = Instant::now();
        {
            let mut pending = self
                .pending_challenges
                .lock()
                .unwrap_or_else(PoisonError::into_inner);
            match pending.get(nonce) {
                Some(expires_at) if *expires_at > now => {
                    // Single-use: the challenge is spent whether or not the
                    // solution is correct.
                    pending.remove(nonce);
                }
                _ => return false,
            }
        }
        if difficulty == 0 {
            return true;
        }
        let digest_input = format!("{nonce}{solution}");
        let digest = Sha256::digest(digest_input.as_bytes());
        let hex_digest = encode_hex(&digest);
        hex_digest.starts_with(&"0".repeat(difficulty))
    }
}
