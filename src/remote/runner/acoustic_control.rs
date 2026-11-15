//! Attended bench operator. Uses ordinary authenticated grants, preparation and
//! correlated completion; never retries mutations or rearms after a fault.
use super::*;
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    pub bind: SocketAddr,
    pub peer: SocketAddr,
    pub server_name: String,
    pub certificate: PathBuf,
    pub key: PathBuf,
    pub ca: PathBuf,
    pub peers: Vec<Peer>,
    pub show_id: String,
    pub delay_ms: u32,
    pub wet_enabled: bool,
    pub report: PathBuf,
    /// Reviewed owner proposal; never inferred from a microphone capture.
    #[serde(default)]
    pub pa_proposal: Option<PathBuf>,
}
struct Operator {
    client: RemoteClient,
    show: String,
    id: u64,
    evidence: Vec<Value>,
}
impl Operator {
    fn envelope(&self, contract: &str, version: u32, kind: &str, body: Value) -> Value {
        json!({"contract":contract,"version":version,"show_id":self.show,"module":"audio","epoch":self.client.identity().source_epoch,"writer":null,"lease":null,"request_id":null,"expected_revision":null,"kind":kind,"body":body})
    }
    async fn exchange(&mut self, request: Value) -> Result<Value> {
        self.client.send_command(request.clone()).await?;
        let reply = tokio::time::timeout(Duration::from_secs(2), self.client.receive())
            .await
            .map_err(|_| "bench control deadline")??;
        let Response::Reply { payload, .. } = reply else {
            return Err(format!("bench control refused: {reply:?}"));
        };
        self.evidence
            .push(json!({"request":request,"reply":payload}));
        Ok(payload)
    }
    async fn snapshot(&mut self) -> Result<Value> {
        self.exchange(self.envelope("C-AUDIO", 2, "snapshot", json!({})))
            .await
    }
    async fn aux(&mut self) -> Result<Value> {
        self.exchange(self.envelope(
            "GP-FX-aux",
            1,
            "fx_aux_snapshot",
            json!({"return_routing":true,"send_taps":true}),
        ))
        .await
    }
    async fn mutation(
        &mut self,
        mut request: Value,
        lease: Value,
        revision: Value,
    ) -> Result<Value> {
        self.id += 1;
        request["writer"] = json!(self.client.writer());
        request["lease"] = lease;
        request["expected_revision"] = revision;
        request["request_id"] = json!(self.id.to_string());
        let expected = request.clone();
        let mut reply = self.exchange(request).await?;
        for _ in 0..8 {
            let aux_reply = expected["contract"] == "GP-FX-aux";
            if expected["contract"] != "C-AUDIO"
                && (reply["contract"] != expected["contract"]
                    || reply["version"] != expected["version"])
            {
                return Err(format!("bench completion contract mismatch: {reply}"));
            }
            let context = if aux_reply {
                &reply
            } else {
                reply
                    .get("context")
                    .or_else(|| reply.get("outcome"))
                    .ok_or_else(|| format!("missing correlated context: {reply}"))?
            };
            // GP-FX-aux v1 echoes correlation at top level and does not echo
            // expected_revision; the source validates it before preparation.
            for field in [
                "show_id",
                "module",
                "epoch",
                "writer",
                "lease",
                "request_id",
            ] {
                if context[field] != expected[field] {
                    return Err(format!("bench completion context mismatch: {field}"));
                }
            }
            if !aux_reply && context["expected_revision"] != expected["expected_revision"] {
                return Err("bench completion revision mismatch".into());
            }
            if reply["outcome"]["kind"] == "applied"
                || reply["state"] == "accepted"
                || (expected["contract"] == "GP14-structure"
                    && reply["state"] == "final"
                    && reply["reason"].is_null()
                    && reply["effective_frame"].is_string())
            {
                return Ok(reply);
            }
            if !["preparing", "prepared", "queued", "pending"]
                .contains(&reply["state"].as_str().unwrap_or(""))
                && reply["outcome"]["kind"] != "queued"
            {
                return Err(format!("bench mutation not applied: {reply}"));
            }
            let next = tokio::time::timeout(Duration::from_secs(2), self.client.receive())
                .await
                .map_err(|_| "bench completion unknown")??;
            let Response::Reply { payload, .. } = next else {
                return Err("bench completion refused".into());
            };
            self.evidence.push(json!({"completion":payload}));
            reply = payload;
        }
        Err("bench completion capacity".into())
    }
    async fn grant(&mut self, scope: &str) -> Result<Value> {
        let snapshot = self.snapshot().await?;
        let request = self.envelope("C-AUDIO", 2, "grant", json!({"scope":scope}));
        let reply = self
            .mutation(
                request,
                Value::Null,
                snapshot["snapshot"]["authority"]["revision"].clone(),
            )
            .await?;
        let lease = reply["outcome"]["body"]["granted_lease"].clone();
        if !lease.is_string() {
            return Err("bench grant lacks lease".into());
        }
        Ok(lease)
    }
    async fn release(&mut self, lease: Value) -> Result<()> {
        let snapshot = self.snapshot().await?;
        self.mutation(
            self.envelope("C-AUDIO", 2, "release", json!({})),
            lease,
            snapshot["snapshot"]["authority"]["revision"].clone(),
        )
        .await?;
        Ok(())
    }
    async fn prepare(&mut self, c: &Config) -> Result<Value> {
        let (delay_ms, wet_enabled) = (c.delay_ms, c.wet_enabled);
        let lease = self.grant("foh").await?;
        let snapshot = self.snapshot().await?;
        let mut edits = Vec::new();
        for (input, fader) in [("input-01", 0), ("input-02", -60000)] {
            edits.push(json!({"target":{"parameter":"fader","input":input},"value":fader}));
            edits.push(json!({"target":{"parameter":"mute","input":input},"value":false}));
            edits.push(json!({"target":{"parameter":"pan","input":input},"value":-100}));
        }
        self.mutation(
            self.envelope("C-AUDIO", 2, "set", json!({"targets":edits})),
            lease.clone(),
            snapshot["snapshot"]["authority"]["revision"].clone(),
        )
        .await?;
        self.release(lease).await?;
        self.client.close();
        self.client = connect(c).await?;
        self.id = 0;
        let ready = Instant::now();
        loop {
            let s = self.aux().await?;
            if s["available"] == true {
                break;
            }
            if ready.elapsed() > Duration::from_secs(2) {
                return Err("Brain owner unavailable".into());
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        let lease = self.grant("fx_configuration").await?;
        let s = self.aux().await?;
        let palette: Value =
            serde_json::from_str(s["palette_json"].as_str().ok_or("missing palette")?)
                .map_err(|e| e.to_string())?;
        let settings = palette
            .as_array()
            .ok_or("palette shape")?
            .iter()
            .find(|v| v["algorithm"] == 1)
            .ok_or("digital delay unavailable")?["defaults"]
            .clone();
        let mut config: Value = serde_json::from_str(
            s["buses"][0]["configuration_json"]
                .as_str()
                .ok_or("bus configuration")?,
        )
        .map_err(|e| e.to_string())?;
        config["settings"] = settings;
        config["settings"]["time_ms"] = json!(delay_ms as f64);
        config["settings"]["amount"] = json!(0.);
        config["settings"]["damping"] = json!(0.);
        config["settings"]["gain"] = json!(1.);
        config["settings"]["bypass"] = json!(if wet_enabled { 0 } else { 1 });
        // Keep identical source/send routing for the actual owner bypass comparison.
        config["sends"] = json!([0., 1.]);
        config["send_taps"] = json!(["post_fader", "pre_fader"]);
        config["tempo"] = Value::Null;
        let body = json!({"bus":0,"map_revision":s["map_revision"],"library_sha256":s["library_sha256"],"owner_binding":s["owner_binding"],"expected_generation":s["buses"][0]["generation"],"configuration_json":config.to_string(),"panic":false});
        self.mutation(
            self.envelope("GP-FX-aux", 1, "fx_aux_configure", body),
            lease.clone(),
            s["revision"].clone(),
        )
        .await?;
        let after = self.aux().await?;
        let actual: Value = serde_json::from_str(
            after["buses"][0]["configuration_json"]
                .as_str()
                .ok_or("missing applied config")?,
        )
        .map_err(|e| e.to_string())?;
        if actual != config {
            return Err("applied FX readback differs".into());
        }
        self.release(lease).await?;
        self.client.close();
        self.client = connect(c).await?;
        self.id = 0;
        // Rearm is a separate explicit operator invocation, only after actual FX
        // completion. This task never repeats it after loss or reconnection.
        let lease = self.grant("pa_configuration").await?;
        let s = self
            .exchange(self.envelope("GP14-structure", 1, "structural_snapshot", json!({})))
            .await?;
        let mut s = s;
        if let Some(path) = &c.pa_proposal {
            if std::fs::metadata(path).map_err(|e| e.to_string())?.len() > 65536 {
                return Err("bench proposal capacity".into());
            }
            let bytes = std::fs::read(path).map_err(|e| e.to_string())?;
            if bytes.len() > 65536 {
                return Err("bench proposal capacity".into());
            }
            let proposal: Value = serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
            let current: Value = serde_json::from_str(
                s["snapshot"]["pa_configuration_json"]
                    .as_str()
                    .ok_or("PA snapshot configuration")?,
            )
            .map_err(|e| e.to_string())?;
            if proposal["contract"] != "SHR-PA-INDEPENDENT-CLOCK-EQ"
                || proposal["version"] != 1
                || proposal["status"] != "proposed"
                || proposal["verification_required"] != true
                || proposal["basis_configuration"] != current
            {
                return Err("bench proposal stale/unreviewable basis".into());
            }
            validate_candidate(&proposal, &current)?;
            self.mutation(
                self.envelope("GP14-structure", 1, "output_mute", json!({})),
                lease.clone(),
                s["revision"].clone(),
            )
            .await?;
            s = self
                .exchange(self.envelope("GP14-structure", 1, "structural_snapshot", json!({})))
                .await?;
            if s["snapshot"]["outputs_quiesced"] != true {
                return Err("PA mute not confirmed".into());
            }
            self.mutation(
                self.envelope(
                    "GP14-structure",
                    1,
                    "pa_set",
                    json!({
                        "configuration_json":proposal["candidate_configuration"].to_string(),
                        "program_buses":s["snapshot"]["pa_program_buses"]
                    }),
                ),
                lease.clone(),
                s["revision"].clone(),
            )
            .await?;
            s = self
                .exchange(self.envelope("GP14-structure", 1, "structural_snapshot", json!({})))
                .await?;
            let actual: Value = serde_json::from_str(
                s["snapshot"]["pa_configuration_json"]
                    .as_str()
                    .ok_or("PA readback")?,
            )
            .map_err(|e| e.to_string())?;
            if actual != proposal["candidate_configuration"]
                || s["snapshot"]["outputs_quiesced"] != true
            {
                return Err("PA candidate readback/mute mismatch".into());
            }
        }
        self.mutation(
            self.envelope("GP14-structure", 1, "output_rearm", json!({})),
            lease.clone(),
            s["revision"].clone(),
        )
        .await?;
        self.release(lease).await?;
        Ok(after)
    }
}
// This checks the explicit bench policy, not the owner's fitting algorithm.
fn validate_candidate(proposal: &Value, current: &Value) -> Result<()> {
    let index = proposal["output_index"]
        .as_u64()
        .and_then(|v| usize::try_from(v).ok())
        .ok_or("proposal output")?;
    let bands = proposal["bands"].as_array().ok_or("proposal bands")?;
    if bands.is_empty() || bands.len() > 2 {
        return Err("proposal cut count".into());
    }
    let mut expected = current.clone();
    let slots = expected["outputs"]
        .get_mut(index)
        .and_then(|v| v["processing"]["eq"].as_array_mut())
        .ok_or("proposal EQ slots")?;
    let mut free = slots.iter_mut().filter(|v| v["db"].as_f64() == Some(0.));
    for band in bands {
        let number = |name: &str| band[name].as_f64().filter(|v| v.is_finite());
        if band["kind"] != "bell"
            || number("slope") != Some(1.)
            || !number("db").is_some_and(|v| (-3. ..=-1.).contains(&v))
            || !number("q").is_some_and(|v| [0.5, 0.707].contains(&v))
            || !number("hz").is_some_and(|v| (250. ..=6300.).contains(&v))
        {
            return Err("proposal broad-cut limits".into());
        }
        *free.next().ok_or("proposal no unused slot")? = band.clone();
    }
    if expected != proposal["candidate_configuration"] {
        return Err("proposal changes unrelated processing".into());
    }
    Ok(())
}

async fn connect(c: &Config) -> Result<RemoteClient> {
    let credentials = Credentials::load_der(&c.certificate, &c.key, &c.ca)?;
    let endpoint = client_endpoint(c.bind, &credentials)?;
    RemoteClient::connect(
        endpoint,
        c.peer,
        &c.server_name,
        PolicyStore::new(c.peers.clone())?,
    )
    .await
}
pub async fn run(c: Config, authorized: bool) -> Result<Value> {
    if !authorized {
        return Err("attended bench control/rearm requires --activate-physical".into());
    }
    if ![200, 400].contains(&c.delay_ms) {
        return Err("reviewed bench delays are 200 or 400ms".into());
    }
    let client = connect(&c).await?;
    let mut op = Operator {
        client,
        show: c.show_id.clone(),
        id: 0,
        evidence: Vec::new(),
    };
    let result = tokio::time::timeout(Duration::from_secs(6), op.prepare(&c))
        .await
        .map_err(|_| "bench operator deadline".to_string())
        .and_then(|v| v);
    op.client.close();
    let report = json!({"mode":"acoustic_control","fault":result.as_ref().err(),"applied":result.as_ref().ok(),"transactions":op.evidence,"automatic_retry":false,"software_only":false});
    save(&c.report, &report)?;
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn proposal_review_refuses_boost_and_unrelated_processing() {
        let base = json!({"outputs":[{"processing":{"eq":[{"db":0.}],"gain_db":0.}}]});
        let band = json!({"kind":"bell","slope":1.,"hz":1000.,"q":0.707,"db":-3.});
        let mut candidate = base.clone();
        candidate["outputs"][0]["processing"]["eq"][0] = band.clone();
        let mut p = json!({"output_index":0,"bands":[band],"candidate_configuration":candidate});
        validate_candidate(&p, &base).unwrap();
        p["bands"][0]["db"] = json!(1.);
        assert!(validate_candidate(&p, &base).is_err());
        p["bands"][0]["db"] = json!(-3.);
        p["candidate_configuration"]["outputs"][0]["processing"]["gain_db"] = json!(1.);
        assert!(validate_candidate(&p, &base).is_err());
    }
}
