//! GP-FX-aux: four independent stereo auxes. Numeric owner data is opaque JSON.
use crate::{control_model, show::Counter};
use serde::{Deserialize, Serialize};
pub const CONTRACT: &str = "GP-FX-aux";
pub const BUSES: usize = 4;
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Mutation {
    pub bus: usize,
    pub map_revision: Counter,
    pub library_sha256: String,
    pub expected_generation: Counter,
    pub configuration_json: Option<String>,
    pub panic: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub owner_binding: Option<String>,
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
        if o.get("body") != Some(&serde_json::json!({})) {
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
#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
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
