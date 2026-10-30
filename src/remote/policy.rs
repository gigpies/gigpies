use super::{MAX_PEERS, Result};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::{Arc, RwLock},
};

/// Independent permissions. A channel grant never includes PA or patch authority.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum Permission {
    Foh,
    Monitor(u32),
    PaConfiguration,
    OutputRoutes,
    Analysis,
    Fx,
    FxConfiguration,
    LocalOperatorMonitor,
    TalkbackDestinations,
    TalkbackFoh,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Peer {
    pub id: String,
    /// Lower-case SHA256 of the DER leaf certificate, in addition to CA verification.
    pub certificate_sha256: String,
    pub permissions: BTreeSet<Permission>,
}

#[derive(Debug, Clone)]
struct Policy {
    generation: u64,
    peers: BTreeMap<String, Peer>,
}

/// Explicit offline pairing store. Mutation invalidates existing session contexts.
/// The lock is only used by network/control workers, never by render.
#[derive(Debug, Clone)]
pub struct PolicyStore(Arc<RwLock<Policy>>);
impl PolicyStore {
    pub fn new(peers: Vec<Peer>) -> Result<Self> {
        let peers = validate(peers)?;
        Ok(Self(Arc::new(RwLock::new(Policy {
            generation: 1,
            peers,
        }))))
    }
    pub fn replace(&self, peers: Vec<Peer>) -> Result<u64> {
        let peers = validate(peers)?;
        let mut policy = self.0.write().map_err(|_| "policy unavailable")?;
        let generation = policy
            .generation
            .checked_add(1)
            .ok_or("policy generation exhausted")?;
        *policy = Policy { generation, peers };
        Ok(generation)
    }
    pub(crate) fn authenticate(
        &self,
        certificate: &[u8],
        session: u64,
    ) -> Result<AuthenticatedContext> {
        if session == 0 {
            return Err("zero session".into());
        }
        let fingerprint = fingerprint(certificate);
        let policy = self.0.read().map_err(|_| "policy unavailable")?;
        let peer = policy
            .peers
            .get(&fingerprint)
            .ok_or("peer not paired or revoked")?;
        Ok(AuthenticatedContext {
            peer_id: peer.id.clone(),
            certificate_sha256: fingerprint.clone(),
            session,
            writer: format!("remote-{}-{session:016x}", &fingerprint[..16]),
            policy_generation: policy.generation,
            permissions: peer.permissions.clone(),
            policy: self.0.clone(),
        })
    }
    pub fn check(&self, context: &AuthenticatedContext) -> Result<()> {
        let policy = self.0.read().map_err(|_| "policy unavailable")?;
        let peer = policy
            .peers
            .get(&context.certificate_sha256)
            .ok_or("peer revoked")?;
        if policy.generation != context.policy_generation
            || peer.id != context.peer_id
            || peer.permissions != context.permissions
        {
            return Err("peer policy changed; reconnect required".into());
        }
        Ok(())
    }
}
fn validate(peers: Vec<Peer>) -> Result<BTreeMap<String, Peer>> {
    if peers.len() > MAX_PEERS {
        return Err("peer policy capacity".into());
    }
    let mut out = BTreeMap::new();
    let mut ids = BTreeSet::new();
    for peer in peers {
        if !crate::show::id(&peer.id)
            || !ids.insert(peer.id.clone())
            || peer.certificate_sha256.len() != 64
            || !peer
                .certificate_sha256
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
            || peer.permissions.len() > 256
            || peer.permissions.contains(&Permission::Monitor(0))
        {
            return Err("invalid peer policy".into());
        }
        if out.insert(peer.certificate_sha256.clone(), peer).is_some() {
            return Err("duplicate peer certificate".into());
        }
    }
    Ok(out)
}
pub fn fingerprint(certificate: &[u8]) -> String {
    format!("{:x}", Sha256::digest(certificate))
}

/// Constructed only after mutual TLS plus the local leaf-certificate policy check.
/// Consumers must bind actual grants to `writer`, not a caller-supplied identity.
#[derive(Debug, Clone)]
pub struct AuthenticatedContext {
    pub(crate) peer_id: String,
    pub(crate) certificate_sha256: String,
    pub(crate) session: u64,
    pub(crate) writer: String,
    pub(crate) policy_generation: u64,
    pub(crate) permissions: BTreeSet<Permission>,
    policy: Arc<RwLock<Policy>>,
}
impl PartialEq for AuthenticatedContext {
    fn eq(&self, other: &Self) -> bool {
        self.peer_id == other.peer_id
            && self.certificate_sha256 == other.certificate_sha256
            && self.session == other.session
            && self.writer == other.writer
            && self.policy_generation == other.policy_generation
            && self.permissions == other.permissions
            && Arc::ptr_eq(&self.policy, &other.policy)
    }
}
impl Eq for AuthenticatedContext {}
impl AuthenticatedContext {
    pub fn peer_id(&self) -> &str {
        &self.peer_id
    }
    pub fn certificate_sha256(&self) -> &str {
        &self.certificate_sha256
    }
    pub fn session(&self) -> u64 {
        self.session
    }
    pub fn writer(&self) -> &str {
        &self.writer
    }
    pub fn policy_generation(&self) -> u64 {
        self.policy_generation
    }
    pub fn permissions(&self) -> &BTreeSet<Permission> {
        &self.permissions
    }
    pub fn check_current(&self) -> Result<()> {
        let policy = self.policy.read().map_err(|_| "policy unavailable")?;
        let peer = policy
            .peers
            .get(&self.certificate_sha256)
            .ok_or("peer revoked")?;
        if policy.generation != self.policy_generation
            || peer.id != self.peer_id
            || peer.permissions != self.permissions
        {
            return Err("peer policy changed; reconnect required".into());
        }
        Ok(())
    }
    pub fn require(&self, permission: &Permission) -> Result<()> {
        self.check_current()?;
        if self.permissions.contains(permission) {
            Ok(())
        } else {
            Err("peer permission denied".into())
        }
    }
    /// Defense in depth before the authority's own envelope validation. Read-only
    /// requests use a null writer; every mutating request must use this session writer.
    pub fn check_writer(&self, payload: &serde_json::Value) -> Result<()> {
        self.check_current()?;
        let object = payload.as_object().ok_or("command envelope")?;
        if object.get("contract").and_then(|v| v.as_str()) == Some("GP-SPECTRUM") {
            // Strict read-only request decoding in the authority; no writer or mutation fields.
            return Ok(());
        }

        let kind = object
            .get("kind")
            .and_then(|v| v.as_str())
            .ok_or("command kind")?;
        let writer = object.get("writer").ok_or("command writer")?;
        if matches!(
            kind,
            "meter_snapshot"
                | "snapshot"
                | "processing_snapshot"
                | "sends_snapshot"
                | "module_status"
                | "topology_snapshot"
                | "clock_snapshot"
                | "pa_snapshot"
                | "structural_snapshot"
                | "master_eq_snapshot"
                | "brain_snapshot"
                | "device_snapshot"
                | "fx_snapshot"
                | "fx_aux_snapshot"
        ) {
            if !writer.is_null() {
                return Err("snapshot writer".into());
            }
        } else if writer.as_str() != Some(&self.writer) {
            return Err("authenticated writer mismatch".into());
        }
        Ok(())
    }
}
