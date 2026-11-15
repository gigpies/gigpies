//! GP-FX-aux: four independent stereo auxes. Numeric owner data is opaque JSON.
use crate::{control_model, show::Counter};
use serde::{Deserialize, Serialize};
pub const CONTRACT: &str = "GP-FX-aux";
pub const BUSES: usize = 4;
/// Publication preflight is audited against this exact approved owner artifact.
pub const BUNDLE_LIBRARY: &str = "cefa625b6aee8a742aa83e54d02d26ce3edef0d9907d22856488fe1517a6ae56";
/// SHR FX 555b8f9: embedded publication code unchanged; actual bundle preflight
/// and allocation-free publication revalidated on 2026-11-12. This is not a
/// wildcard: saved/current artifact identity must still match exactly.
pub const BUNDLE_LIBRARY_20261112: &str =
    "9173dd76294ce50cefeaf0d1d4ce633ba424745cf7d4f2a0c181f6c878af90c1";
fn bundle_library(hash: &str) -> bool {
    matches!(hash, BUNDLE_LIBRARY | BUNDLE_LIBRARY_20261112)
}
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Settings {
    pub version: u32,
    pub size: u32,
    pub algorithm: u32,
    pub bypass: u32,
    pub time_ms: f64,
    pub amount: f64,
    pub damping: f64,
    pub rate_hz: f64,
    pub gain: f64,
    pub seed: u32,
    pub reserved: u32,
}

/// Host routing after the shared wet owner, before Main-only gain.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Returns {
    pub main: f64,
    pub monitors: Vec<f64>,
}
impl Returns {
    pub fn silent_monitors(count: usize) -> Self {
        Self {
            main: 1.,
            monitors: vec![0.; count],
        }
    }
    pub fn valid(&self, count: usize) -> bool {
        self.monitors.len() == count
            && self.main.is_finite()
            && (0.0..=1.0).contains(&self.main)
            && self
                .monitors
                .iter()
                .all(|v| v.is_finite() && (0.0..=1.0).contains(v))
    }
    pub fn validate(&self, count: usize) -> Result<(), String> {
        if self.valid(count) {
            Ok(())
        } else {
            Err("FX return shape/range".into())
        }
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Mutation {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub restore: Option<Box<Restore>>,
    pub bus: usize,
    pub map_revision: Counter,
    pub library_sha256: String,
    pub expected_generation: Counter,
    pub configuration_json: Option<String>,
    pub panic: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub owner_binding: Option<String>,
}
/// One authority transaction; the first-bus fields must agree with this bundle.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Restore {
    pub version: u32,
    pub saved: crate::fx_saved::Saved,
    pub expected_generations: [Counter; 4],
}
impl Mutation {
    pub fn edits(&self) -> Vec<Self> {
        match &self.restore {
            None => vec![self.clone()],
            Some(r) => (0..4)
                .map(|bus| Self {
                    restore: None,
                    bus,
                    map_revision: self.map_revision,
                    library_sha256: self.library_sha256.clone(),
                    expected_generation: r.expected_generations[bus],
                    configuration_json: Some(if r.saved.version == 1 {
                        let mut c: serde_json::Value =
                            serde_json::from_str(&r.saved.configurations[bus])
                                .expect("validated saved FX");
                        c["send_taps"] =
                            serde_json::json!(vec![SendTap::PostFader; r.saved.inputs.len()]);
                        serde_json::to_string(&c).expect("valid configuration")
                    } else {
                        r.saved.configurations[bus].clone()
                    }),
                    panic: false,
                    owner_binding: self.owner_binding.clone(),
                })
                .collect(),
        }
    }
    pub fn validate_restore(&self) -> Result<(), String> {
        if let Some(r) = &self.restore {
            r.saved.validate()?;
            if r.version != 1
                || !bundle_library(&self.library_sha256)
                || self.bus != 0
                || self.panic
                || self.expected_generation != r.expected_generations[0]
                || self.configuration_json.as_ref() != Some(&r.saved.configurations[0])
                || self.library_sha256 != r.saved.library_sha256
                || r.expected_generations.iter().any(|v| v.0 == u64::MAX)
            {
                return Err("FX restore bundle shape".into());
            }
        }
        Ok(())
    }
}
pub fn request(
    value: &serde_json::Value,
    audio_version: u32,
) -> Result<(control_model::Request, Option<Mutation>), String> {
    let mut v = value.clone();
    let o = v.as_object_mut().ok_or("FX aux envelope")?;
    if o.get("contract").and_then(|v| v.as_str()) != Some(CONTRACT)
        || o.get("version").and_then(|v| v.as_u64()) != Some(1)
    {
        return Err("FX aux contract".into());
    }
    let read = o.get("kind").and_then(|v| v.as_str()) == Some("fx_aux_snapshot");
    let mutation = if read {
        let mut body = o.get("body").cloned().ok_or("FX aux read body")?;
        if let Some(taps) = body.as_object_mut().and_then(|b| b.remove("send_taps"))
            && taps != true
        {
            return Err("FX aux send_taps probe".into());
        }
        if body != serde_json::json!({})
            && body != serde_json::json!({"return_routing":true})
            && body != serde_json::json!({"return_routing":true,"saved_intent":true})
        {
            return Err("FX aux read body".into());
        }
        None
    } else {
        if o.get("kind").and_then(|v| v.as_str()) != Some("fx_aux_configure") {
            return Err("FX aux kind".into());
        }
        let m: Mutation = serde_json::from_value(o.get("body").cloned().ok_or("FX aux body")?)
            .map_err(|e| e.to_string())?;
        if m.owner_binding.as_ref().is_some_and(|s| {
            s.len() != 64
                || !s
                    .bytes()
                    .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
        }) || m.bus >= BUSES
            || m.panic == m.configuration_json.is_some()
            || m.configuration_json
                .as_ref()
                .is_some_and(|v| v.len() > 32768)
            || m.library_sha256.len() != 64
            || !m
                .library_sha256
                .bytes()
                .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
        {
            return Err("FX aux mutation bounds".into());
        }
        m.validate_restore()?;
        Some(m)
    };
    o.insert("contract".into(), serde_json::json!("C-AUDIO"));
    o.insert("version".into(), serde_json::json!(audio_version));
    o.insert(
        "kind".into(),
        serde_json::json!(if read { "snapshot" } else { "renew" }),
    );
    o.insert("body".into(), serde_json::json!({}));
    Ok((
        control_model::Request::decode(&serde_json::to_vec(&v).map_err(|e| e.to_string())?)?,
        mutation,
    ))
}
/// Opt-in readback preserves strict legacy consumers' configuration shape.
pub(crate) fn snapshot_compatibility(
    request: &serde_json::Value,
    mut snapshot: serde_json::Value,
) -> Result<serde_json::Value, String> {
    if (request["body"]["saved_intent"] != true || snapshot["available"] != true)
        && let Some(o) = snapshot.as_object_mut()
    {
        o.remove("restore_ready");
    }
    if request["body"]["saved_intent"] == true && snapshot["available"] == true {
        snapshot["restore_version"] = serde_json::json!(if snapshot["library_sha256"]
            .as_str()
            .is_some_and(bundle_library)
        {
            1
        } else {
            0
        });
    }
    if request["body"]["send_taps"] != true {
        if let Some(o) = snapshot.as_object_mut() {
            o.remove("send_taps_version");
        }
        if let Some(buses) = snapshot["buses"].as_array_mut() {
            for bus in buses {
                if let Some(text) = bus["configuration_json"].as_str() {
                    let mut c: serde_json::Value =
                        serde_json::from_str(text).map_err(|e| e.to_string())?;
                    if c["send_taps"]
                        .as_array()
                        .is_some_and(|t| t.iter().any(|t| t != "post_fader"))
                    {
                        return Err("FX pre-fader sends require send_taps readback".into());
                    }
                    if c.as_object_mut()
                        .ok_or("FX config")?
                        .remove("send_taps")
                        .is_some()
                    {
                        bus["configuration_json"] = serde_json::json!(
                            serde_json::to_string(&c).map_err(|e| e.to_string())?
                        );
                    }
                }
            }
        }
    }
    if request["body"]["return_routing"] == true {
        return Ok(snapshot);
    }
    if let Some(object) = snapshot.as_object_mut() {
        object.remove("return_routing_version");
        object.remove("monitors");
    }
    if let Some(buses) = snapshot
        .get_mut("buses")
        .and_then(serde_json::Value::as_array_mut)
    {
        for bus in buses {
            if let Some(encoded) = bus["configuration_json"].as_str() {
                let mut config: serde_json::Value =
                    serde_json::from_str(encoded).map_err(|e| e.to_string())?;
                if let Some(object) = config.as_object_mut() {
                    object.remove("returns");
                }
                bus["configuration_json"] = serde_json::Value::String(
                    serde_json::to_string(&config).map_err(|e| e.to_string())?,
                );
            }
        }
    }
    Ok(snapshot)
}
#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn routing_readback_is_explicit_and_legacy_shape_is_preserved() {
        let snapshot = json!({"return_routing_version":1,"monitors":["monitor-1"],"buses":[{"configuration_json":r#"{"settings":{},"sends":[0],"tempo":null,"returns":{"main":0,"monitors":[0.5]}}"#}]});
        assert_eq!(
            snapshot_compatibility(&json!({"body":{"return_routing":true}}), snapshot.clone())
                .unwrap(),
            snapshot
        );
        let legacy = snapshot_compatibility(&json!({"body":{}}), snapshot).unwrap();
        assert!(legacy.get("monitors").is_none());
        let config: serde_json::Value =
            serde_json::from_str(legacy["buses"][0]["configuration_json"].as_str().unwrap())
                .unwrap();
        assert!(config.get("returns").is_none());
    }
    #[test]
    fn pre_fader_readback_cannot_be_silently_hidden_from_legacy_clients() {
        let s = json!({"available":true,"send_taps_version":1,"buses":[{"configuration_json":r#"{"settings":{},"sends":[1],"tempo":null,"send_taps":["pre_fader"]}"#}]});
        assert!(snapshot_compatibility(&json!({"body":{}}), s.clone()).is_err());
        let v = snapshot_compatibility(&json!({"body":{"send_taps":true}}), s).unwrap();
        assert_eq!(v["send_taps_version"], 1);
        let c: serde_json::Value =
            serde_json::from_str(v["buses"][0]["configuration_json"].as_str().unwrap()).unwrap();
        assert_eq!(c["send_taps"][0], "pre_fader");
    }
    #[test]
    fn exact_aux_envelope_and_four_bus_boundary() {
        let mut v = json!({"contract":CONTRACT,"version":1,"show_id":"11111111-1111-4111-8111-111111111111","module":"audio","epoch":"1","writer":null,"lease":null,"request_id":null,"expected_revision":null,"kind":"fx_aux_snapshot","body":{}});
        assert!(request(&v, 2).unwrap().1.is_none());
        v["body"] = json!({"unused":1});
        assert!(request(&v, 2).is_err());
        v["body"] = json!({});
        v["kind"] = json!("fx_aux_configure");
        v["body"] = json!({"bus":4,"map_revision":"1","library_sha256":"a".repeat(64),"expected_generation":"0","configuration_json":null,"panic":true});
        assert!(request(&v, 2).is_err());
        v["body"]["bus"] = json!(3);
        v["body"]["panic"] = json!(false);
        assert!(request(&v, 2).is_err());
    }
}

/// Both choices follow processing, shared mute and pan; only fader participation differs.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SendTap {
    PreFader,
    #[default]
    PostFader,
}
impl SendTap {
    pub(crate) fn pre_fraction(self) -> f64 {
        if self == Self::PreFader { 1. } else { 0. }
    }
}
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Configuration {
    pub settings: Settings,
    pub sends: Vec<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub send_taps: Option<Vec<SendTap>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub returns: Option<crate::fx_aux_wire::Returns>,
    #[serde(default)]
    pub tempo: Option<Tempo>,
}
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Tempo {
    pub bpm: f64,
    pub denominator: u32,
    pub modifier: u32,
}

impl Configuration {
    pub fn validate_taps(&self, inputs: usize) -> Result<(), String> {
        if self.send_taps.as_ref().is_some_and(|v| v.len() != inputs) {
            return Err("FX send tap dimensions".into());
        }
        Ok(())
    }

    pub fn validate_saved(&self, inputs: usize, monitors: usize) -> Result<(), String> {
        self.validate_taps(inputs)?;
        let s = &self.settings;
        if s.version != 2
            || s.size != 64
            || s.reserved != 0
            || s.algorithm == 0
            || s.algorithm >= 32
            || s.bypass > 1
            || [s.time_ms, s.amount, s.damping, s.rate_hz, s.gain]
                .iter()
                .any(|x| !x.is_finite())
            || self.sends.len() != inputs
            || self
                .sends
                .iter()
                .any(|x| !x.is_finite() || !(0.0..=1.0).contains(x))
        {
            return Err("FX saved settings/sends".into());
        }
        self.returns
            .as_ref()
            .ok_or("saved FX requires every return")?
            .validate(monitors)?;
        if let Some(t) = &self.tempo {
            let factor = match t.modifier {
                0 => 1.,
                1 => 1.5,
                2 => 2. / 3.,
                _ => return Err("FX tempo modifier".into()),
            };
            let time = 240000. / t.bpm / f64::from(t.denominator) * factor;
            if !matches!(s.algorithm, 1 | 8)
                || !t.bpm.is_finite()
                || !(30.0..=300.0).contains(&t.bpm)
                || ![1, 2, 4, 8, 16, 32, 64].contains(&t.denominator)
                || !(1.0..=2000.0).contains(&time)
                || (time - s.time_ms).abs() > 1e-9
            {
                return Err("FX saved tempo/time".into());
            }
        }
        Ok(())
    }
}
