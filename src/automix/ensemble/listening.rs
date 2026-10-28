//! Offline, randomized original-level listening material; never opens audio hardware.
use super::*;
use crate::automix::{delivery, identity::file_hash, matching::read_json, write_json};
use serde_json::{Value, json};
use std::{fs, io::Read};

pub fn prepare(
    state: workflow::State,
    root: &Path,
    candidate_dir: &Path,
    out: &Path,
) -> Result<()> {
    if out.exists() {
        return Err("listening output already exists".into());
    }
    state.validate()?;
    state.frozen.verify(&state.baseline, root, &state.request)?;
    let report: Value = read_json(&candidate_dir.join("report.json"))?;
    let candidate: Session = read_json(&candidate_dir.join("settings.json"))?;
    let exported: Session = read_json(&candidate_dir.join("export/prepared.json"))?;
    if report["accepted"] != true
        || report["finalized_export_checked"] != true
        || report["frozen_id"] != state.frozen.id
        || report["bundle_id"] != identity(&candidate)?
        || identity(&exported)? != identity(&candidate)?
    {
        return Err("listening pack requires the checked candidate from this frozen state".into());
    }
    delivery::check(&candidate_dir.join("export"), root)?;
    if state.request.held_out.is_empty() {
        return Err("listening excerpts require declared held-out passages".into());
    }
    let policy: delivery::Policy = read_json(&candidate_dir.join("export/delivery-policy.json"))?;
    fs::create_dir(out)?;
    crate::automix::run_policy(
        state.baseline.clone(),
        root,
        &out.join("baseline-export"),
        policy.clone(),
    )?;
    delivery::check(&out.join("baseline-export"), root)?;
    let originals = [
        out.join("baseline-export/processed.wav"),
        candidate_dir.join("export/processed.wav"),
    ];
    let mut entropy = [0u8; 1];
    fs::File::open("/dev/urandom")?.read_exact(&mut entropy)?;
    let order = if entropy[0] & 1 == 0 { [0, 1] } else { [1, 0] };
    let mut clips = Vec::new();
    let mut key = Vec::new();
    for (label, index) in ["A", "B"].iter().zip(order) {
        key.push(json!({"label":label,"source":if index==0 {"baseline"} else {"candidate"},"path":originals[index],"sha256":file_hash(&originals[index])?}));
        for (number, span) in state.request.held_out.iter().enumerate() {
            let mut reader = hound::WavReader::open(&originals[index])?;
            let spec = reader.spec();
            if spec.sample_format != hound::SampleFormat::Int
                || spec.bits_per_sample != 24
                || spec.channels != 2
                || spec.sample_rate != state.baseline.sample_rate
            {
                return Err("listening pack requires finalized stereo PCM24 at native rate".into());
            }
            let start = (span[0] * spec.sample_rate as f64).round() as u32;
            let end = (span[1] * spec.sample_rate as f64).round() as u32;
            if end <= start || end > reader.duration() {
                return Err("listening passage outside final PCM".into());
            }
            reader.seek(start)?;
            let name = format!("{number:02}-{label}.wav");
            let mut writer = hound::WavWriter::create(out.join(&name), spec)?;
            let count = (end - start) as usize * 2;
            let mut written = 0;
            for sample in reader.samples::<i32>().take(count) {
                writer.write_sample(sample?)?;
                written += 1;
            }
            if written != count {
                return Err("incomplete listening PCM".into());
            }
            writer.finalize()?;
            // Re-read and compare exact PCM against the selected original frames.
            let mut original = hound::WavReader::open(&originals[index])?;
            original.seek(start)?;
            let mut copy = hound::WavReader::open(out.join(&name))?;
            for (a, b) in original
                .samples::<i32>()
                .take(count)
                .zip(copy.samples::<i32>())
            {
                if a? != b? {
                    return Err("listening PCM changed".into());
                }
            }
            let peak = crate::automix::true_peak::measure(&out.join(&name))?;
            let observed = if policy.peak_basis == delivery::PeakBasis::True {
                peak.true_peak_dbtp
            } else {
                peak.sample_peak_dbfs
            };
            if observed > policy.ceiling_db + 1e-6 {
                return Err("excerpt boundary exceeds delivery ceiling; choose another passage, do not silently normalize".into());
            }
            clips.push(json!({"meter":peak,"file":name,"label":label,"start_frame":start,"end_frame":end,"sha256":file_hash(&out.join(&name))?,"exact_pcm_slice":true}));
        }
    }
    state.frozen.verify(&state.baseline, root, &state.request)?;
    delivery::check(&candidate_dir.join("export"), root)?;
    delivery::check(&out.join("baseline-export"), root)?;
    write_json(&out.join("private-key.json"), &key)?;
    write_json(
        &out.join("review.json"),
        &json!({"version":1,"frozen_id":state.frozen.id,"clips":clips,"gain_changes":false,"loudness_matched":false,"playback":false,"preference":null,"allowed_preferences":["A","B","no_preference"],"comments":{"impact":null,"definition":null,"harshness":null,"naturalness":null,"intelligibility":null},"limitations":"Original delivery levels; personal listening, not a standardized perceptual study. Keep private-key.json hidden until judgments are recorded. Compare a separate static candidate only if independently prepared and checked."}),
    )?;
    Ok(())
}
