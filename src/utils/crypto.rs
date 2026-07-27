use secp256k1::{self, Message, Secp256k1, XOnlyPublicKey};
use sha2::{Digest, Sha256};

use crate::error::{RelayError, Result};
use crate::types::Event;

pub fn compute_event_id(event: &Event) -> Result<[u8; 32]> {
    let serialized = event.serialized_for_id();
    let mut hasher = Sha256::new();
    hasher.update(serialized.as_bytes());
    let hash: [u8; 32] = hasher.finalize().into();
    Ok(hash)
}

pub fn verify_event_signature(event: &Event) -> Result<bool> {
    let computed_id = compute_event_id(event)?;
    let computed_id_hex = hex::encode(computed_id);
    if computed_id_hex != event.id {
        return Ok(false);
    }

    let pubkey_bytes: [u8; 32] = hex::decode(&event.pubkey)
        .map_err(|_| RelayError::InvalidEvent("invalid pubkey hex".into()))?
        .try_into()
        .map_err(|_| RelayError::InvalidEvent("invalid pubkey length".into()))?;

    let sig_bytes: [u8; 64] = hex::decode(&event.sig)
        .map_err(|_| RelayError::InvalidEvent("invalid sig hex".into()))?
        .try_into()
        .map_err(|_| RelayError::InvalidEvent("invalid sig length".into()))?;

    let secp = Secp256k1::new();
    let xonly_pubkey = XOnlyPublicKey::from_slice(&pubkey_bytes)?;
    let msg = Message::from_digest(computed_id);
    let sig = secp256k1::schnorr::Signature::from_slice(&sig_bytes)?;

    Ok(secp.verify_schnorr(&sig, &msg, &xonly_pubkey).is_ok())
}

pub fn validate_event(event: &Event) -> Result<()> {
    if event.id.is_empty() {
        return Err(RelayError::InvalidEvent("id is empty".into()));
    }
    if event.pubkey.is_empty() {
        return Err(RelayError::InvalidEvent("pubkey is empty".into()));
    }
    if event.sig.is_empty() {
        return Err(RelayError::InvalidEvent("sig is empty".into()));
    }

    if !verify_event_signature(event)? {
        return Err(RelayError::InvalidEvent(
            "signature verification failed".into(),
        ));
    }

    for tag in &event.tags {
        if tag.is_empty() {
            return Err(RelayError::InvalidEvent("empty tag array".into()));
        }
    }

    Ok(())
}

pub fn verify_auth_event(event: &Event, challenge: &str, relay_url: &str) -> Result<bool> {
    if event.kind != 22242 {
        return Ok(false);
    }

    let challenge_tag = event
        .tags
        .iter()
        .find(|t| t.first().map(|s| s.as_str()) == Some("challenge"))
        .and_then(|t| t.get(1));

    if challenge_tag != Some(&challenge.to_string()) {
        return Ok(false);
    }

    let relay_tag = event
        .tags
        .iter()
        .find(|t| t.first().map(|s| s.as_str()) == Some("relay"))
        .and_then(|t| t.get(1));

    match relay_tag {
        None => return Ok(false),
        Some(url) => {
            let expected_authority = relay_url
                .trim_start_matches("ws://")
                .trim_start_matches("wss://");
            let tag_authority = url
                .trim_start_matches("ws://")
                .trim_start_matches("wss://")
                .trim_end_matches('/');
            if tag_authority != expected_authority {
                return Ok(false);
            }
        }
    }

    verify_event_signature(event)
}
