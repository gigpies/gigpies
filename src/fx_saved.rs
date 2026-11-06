//! Portable four-aux intent. No authority, runtime state or machine endpoints.
use crate::{fx_aux_wire::Configuration, show::Result, topology::InputPort};
use serde::{Deserialize, Serialize};

pub const MAX_BYTES: usize = 256 * 1024;
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Saved {
    pub format: String,
    pub version: u32,
    pub show_id: String,
    pub library_sha256: String,
    pub inputs: Vec<InputPort>,
    pub monitors: Vec<String>,
    /// Exact owner configuration strings preserve the existing float contract.
    pub configurations: [String; 4],
}
impl Saved {
    pub fn decode(bytes: &[u8]) -> Result<Self> {
        let saved: Self = crate::show::decode_bounded(bytes, MAX_BYTES)?;
        saved.validate()?;
        Ok(saved)
    }
    pub fn validate(&self) -> Result<[Configuration; 4]> {
        if self.format != "gigpies-fx-intent" || self.version != 1 {
            return Err(
                "unsupported FX intent; legacy core/composed files contain no saved FX".into(),
            );
        }
        if self.show_id.len() != 36
            || !self.show_id.bytes().enumerate().all(|(i, b)| {
                if [8, 13, 18, 23].contains(&i) {
                    b == b'-'
                } else {
                    b.is_ascii_digit() || (b'a'..=b'f').contains(&b)
                }
            })
            || self.library_sha256.len() != 64
            || !self
                .library_sha256
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
            || self.inputs.is_empty()
            || self.inputs.len() > 4096
            || self.monitors.len() > 4096
        {
            return Err("FX saved identity/capacity".into());
        }
        let mut ids = std::collections::BTreeSet::new();
        let mut slots = std::collections::BTreeSet::new();
        let mut ports = std::collections::BTreeSet::new();
        for input in &self.inputs {
            if !crate::show::id(&input.id)
                || input.physical_port.is_empty()
                || input.physical_port.len() > 128
                || input.capture_slot > u16::MAX as usize
                || !ids.insert(&input.id)
                || !slots.insert(input.capture_slot)
                || !ports.insert(&input.physical_port)
            {
                return Err("ambiguous FX input identity".into());
            }
        }
        if self
            .monitors
            .iter()
            .enumerate()
            .any(|(i, id)| *id != format!("monitor-{}", i + 1))
        {
            return Err("FX monitor identity".into());
        }
        let configs = self.configurations.each_ref().map(|text| {
            if text.len() > 32768 {
                return Err("FX configuration capacity".to_owned());
            }
            let c: Configuration = serde_json::from_str(text).map_err(|e| e.to_string())?;
            let fields: serde_json::Value =
                serde_json::from_str(text).map_err(|e| e.to_string())?;
            if fields.as_object().is_none_or(|o| {
                o.len() != 4
                    || !["settings", "sends", "returns", "tempo"]
                        .iter()
                        .all(|k| o.contains_key(*k))
            }) {
                return Err("saved FX requires complete settings, sends, returns and tempo".into());
            }
            c.validate_saved(self.inputs.len(), self.monitors.len())?;
            Ok(c)
        });
        let [a, b, c, d] = configs;
        Ok([a?, b?, c?, d?])
    }
    pub fn compatible(
        &self,
        show: &str,
        hash: &str,
        inputs: &[InputPort],
        monitors: usize,
    ) -> Result<()> {
        self.validate()?;
        if self.show_id != show
            || self.library_sha256 != hash
            || self.inputs != inputs
            || self.monitors.len() != monitors
        {
            return Err(
                "FX saved identities incompatible; explicit remapping is not supported".into(),
            );
        }
        Ok(())
    }
    /// C-SHOW points to this separately owned payload; it never contains or
    /// applies FX state itself. Core/composed intent remains an independent file.
    pub fn manifest_module(&self) -> Result<crate::show::Module> {
        use sha2::{Digest, Sha256};
        self.validate()?;
        let bytes = serde_json::to_vec(self).map_err(|e| e.to_string())?;
        if bytes.len() > MAX_BYTES {
            return Err("FX saved capacity".into());
        }
        Ok(crate::show::Module {
            id: "fx-aux".into(),
            contract: "C-FX".into(),
            version: 1,
            required: false,
            content_identity: crate::hash::hex(Sha256::digest(bytes)),
        })
    }
    pub fn save(&self, directory: &std::path::Path, name: &str) -> Result<()> {
        self.validate()?;
        crate::show::persist_bounded(directory, name, self, MAX_BYTES)
    }
}
