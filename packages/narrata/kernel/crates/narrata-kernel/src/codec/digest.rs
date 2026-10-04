use sha2::{Digest, Sha256};

pub fn sha256(bytes: &[u8]) -> [u8; 32] {
    let result = Sha256::digest(bytes);
    let mut digest = [0_u8; 32];
    digest.copy_from_slice(&result);
    digest
}

pub fn digest_bytes(domain: &str, schema_version: u16, payload: &[u8]) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(b"NARRATA-DIGEST\0");
    hasher.update((domain.len() as u16).to_be_bytes());
    hasher.update(domain.as_bytes());
    hasher.update(schema_version.to_be_bytes());
    hasher.update((payload.len() as u64).to_be_bytes());
    hasher.update(payload);
    let result = hasher.finalize();
    let mut digest = [0_u8; 32];
    digest.copy_from_slice(&result);
    digest
}

/// Content identity of an object, independent of the envelope's payload checksum.
pub fn object_id(kind_code: u16, schema: u16, payload: &[u8]) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(b"narrata-object\0");
    hasher.update(kind_code.to_be_bytes());
    hasher.update(schema.to_be_bytes());
    hasher.update(payload);
    let result = hasher.finalize();
    let mut digest = [0_u8; 32];
    digest.copy_from_slice(&result);
    digest
}
