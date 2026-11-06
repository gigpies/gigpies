//! Shared content identities for frozen offline preparation.
use super::config::Session;
use crate::inventory::Result;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    io::Read,
    path::{Path, PathBuf},
};

pub fn identity(value: &impl Serialize) -> Result<String> {
    // serde_json Value orders keys and its round-trip representation is canonical here.
    let bytes = serde_json::to_vec(&serde_json::to_value(value)?)?;
    Ok(crate::hash::hex(Sha256::digest(bytes)))
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct SourceIdentity {
    pub file: PathBuf,
    pub bytes: u64,
    pub sha256: String,
}
pub fn source_identities(s: &Session, root: &Path) -> Result<Vec<SourceIdentity>> {
    s.channels
        .iter()
        .map(|c| {
            let mut file = std::fs::File::open(root.join(&c.file))?;
            let bytes = file.metadata()?.len();
            let mut hash = Sha256::new();
            let mut buf = [0u8; 65536];
            loop {
                let n = file.read(&mut buf)?;
                if n == 0 {
                    break;
                }
                hash.update(&buf[..n]);
            }
            Ok(SourceIdentity {
                file: c.file.clone(),
                bytes,
                sha256: crate::hash::hex(hash.finalize()),
            })
        })
        .collect()
}

pub(super) fn file_hash(path: &Path) -> Result<String> {
    let mut file = std::fs::File::open(path)?;
    let mut hash = Sha256::new();
    let mut buf = [0u8; 65536];
    loop {
        let n = file.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hash.update(&buf[..n]);
    }
    Ok(crate::hash::hex(hash.finalize()))
}

pub(super) fn bytes_hash(bytes: &[u8]) -> String {
    crate::hash::hex(Sha256::digest(bytes))
}
