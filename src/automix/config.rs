use crate::inventory::Result;
use serde::{Deserialize, Serialize};
use std::{collections::BTreeSet, path::PathBuf};

#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EqKind {
    #[default]
    Bell,
    LowShelf,
    HighShelf,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EqBand {
    #[serde(default)]
    pub kind: EqKind,
    pub hz: f64,
    pub q: f64,
    pub db: f64,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Compressor {
    pub threshold_db: f64,
    pub ratio: f64,
    pub knee_db: f64,
    pub attack_ms: f64,
    pub release_ms: f64,
    pub makeup_db: f64,
}
#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Role {
    Kick,
    Snare,
    Tom,
    Overheads,
    DrumRoom,
    BassDi,
    BassAmp,
    RhythmGuitar,
    LeadGuitar,
    LeadVocal,
    VocalRoom,
    AcousticGuitar,
    BackingVocal,
    Other,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Channel {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dynamic_eq: Option<super::dynamic_eq::Config>,
    pub file: PathBuf,
    pub role: Role,
    pub group: String,
    pub fader_db: f64,
    pub pan: f64,
    pub hpf_hz: f64,
    pub eq: Vec<EqBand>,
    pub compressor: Compressor,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Calibration {
    pub target_rms_db: f64,
    pub peak_headroom_db: f64,
    pub activity_floor_db: f64,
    pub relative_activity_db: f64,
    pub min_trim_db: f64,
    pub max_trim_db: f64,
    pub up_db_per_second: f64,
    pub down_db_per_second: f64,
    pub initial_trim_db: f64,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Group {
    pub name: String,
    pub reference: usize,
    pub trim_db: f64,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OutputMode {
    #[default]
    Matched,
    Unmatched,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Session {
    #[serde(default)]
    pub output_mode: OutputMode,
    #[serde(default)]
    pub master_hpf_hz: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub effects: Option<super::effects::FxConfig>,
    pub version: u32,
    pub sample_rate: u32,
    pub block_frames: usize,
    pub prepared: bool,
    pub calibration: Calibration,
    pub master_db: f64,
    pub ceiling_db: f64,
    pub limiter_release_ms: f64,
    pub groups: Vec<Group>,
    pub channels: Vec<Channel>,
}
fn range(x: f64, lo: f64, hi: f64) -> bool {
    x.is_finite() && (lo..=hi).contains(&x)
}
impl Session {
    pub fn validate(&self) -> Result<()> {
        let c = &self.calibration;
        if self.version != 1
            || !(8000..=192000).contains(&self.sample_rate)
            || !(32..=8192).contains(&self.block_frames)
            || self.channels.is_empty()
            || self.channels.len() > 64
            || self.groups.is_empty()
            || !range(self.master_db, -60., 12.)
            || !range(self.ceiling_db, -24., -0.01)
            || !(self.master_hpf_hz == 0.
                || range(self.master_hpf_hz, 10., self.sample_rate as f64 * 0.45))
            || !range(self.limiter_release_ms, 10., 2000.)
            || !range(c.target_rms_db, -36., -12.)
            || !range(c.peak_headroom_db, -18., -3.)
            || !range(c.activity_floor_db, -90., -30.)
            || !range(c.relative_activity_db, 6., 40.)
            || !range(c.min_trim_db, -48., 0.)
            || !range(c.max_trim_db, 0., 30.)
            || !range(c.initial_trim_db, c.min_trim_db, c.max_trim_db)
            || !range(c.up_db_per_second, 0.01, 6.)
            || !range(c.down_db_per_second, 6., 240.)
        {
            return Err("invalid session/calibration/master settings".into());
        }
        let mut names = BTreeSet::new();
        for g in &self.groups {
            if g.name.is_empty()
                || !g
                    .name
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"_-".contains(&b))
                || !names.insert(&g.name)
                || g.reference >= self.channels.len()
                || self.channels[g.reference].group != g.name
                || !range(g.trim_db, c.min_trim_db, c.max_trim_db)
            {
                return Err("invalid/duplicate calibration group or reference".into());
            }
        }
        let mut files = BTreeSet::new();
        for ch in &self.channels {
            if let Some(dynamic) = &ch.dynamic_eq {
                dynamic.validate(self.sample_rate)?;
            }
            let p = &ch.compressor;
            if !names.contains(&ch.group)
                || !files.insert(&ch.file)
                || ch.file.as_os_str().is_empty()
                || !range(ch.fader_db, -80., 12.)
                || !range(ch.pan, -1., 1.)
                || !(ch.hpf_hz == 0. || range(ch.hpf_hz, 10., self.sample_rate as f64 * 0.45))
                || ch.eq.len() > 8
                || ch.eq.iter().any(|e| {
                    !range(e.hz, 10., self.sample_rate as f64 * 0.45)
                        || !e.valid_q()
                        || !range(e.db, -12., 12.)
                })
                || !range(p.threshold_db, -60., 0.)
                || !range(p.ratio, 1., 20.)
                || !range(p.knee_db, 0., 18.)
                || !range(p.attack_ms, 0.1, 200.)
                || !range(p.release_ms, 10., 2000.)
                || !range(p.makeup_db, -12., 12.)
            {
                return Err(format!("invalid channel {:?}", ch.file).into());
            }
        }
        if let Some(fx) = &self.effects {
            fx.validate(self)?;
        }
        Ok(())
    }
}
impl Channel {
    pub fn preset(file: &str, role: Role, group: &str, fader_db: f64, pan: f64) -> Self {
        use Role::*;
        // Engineering starting points, not a manufacturer's factory preset.
        let (hp, bands, threshold, ratio, attack, release, makeup) = match role {
            AcousticGuitar | BackingVocal | Other => (90., vec![], 0., 1., 10., 100., 0.),
            Kick => (
                30.,
                vec![(65., 0.8, 2.), (280., 1., -3.), (3000., 1., 1.5)],
                -20.,
                4.,
                18.,
                100.,
                1.,
            ),
            Snare => (
                65.,
                vec![(100., 0.8, 3.), (400., 1., -2.), (1000., 0.9, 3.)],
                -24.,
                5.,
                8.,
                120.,
                3.,
            ),
            Tom => (
                45.,
                vec![(320., 1., -3.), (3500., 0.8, 1.5)],
                -18.,
                3.,
                15.,
                180.,
                0.,
            ),
            Overheads => (120., vec![(350., 0.8, -2.)], -16., 1.5, 30., 200., 0.),
            DrumRoom => (140., vec![(400., 0.8, -2.)], -20., 2., 25., 220., 0.),
            BassDi => (
                30.,
                vec![(300., 0.9, -3.), (900., 0.8, 1.5)],
                -24.,
                4.,
                25.,
                160.,
                2.,
            ),
            BassAmp => (
                40.,
                vec![(300., 0.9, -3.), (1800., 0.9, 1.)],
                -24.,
                4.,
                25.,
                160.,
                2.,
            ),
            RhythmGuitar => (
                95.,
                vec![(300., 0.8, -2.), (2200., 0.9, -2.)],
                -20.,
                2.,
                25.,
                180.,
                0.,
            ),
            LeadGuitar => (
                110.,
                vec![(350., 0.8, -2.), (1400., 0.9, 1.5)],
                -20.,
                2.,
                20.,
                150.,
                0.,
            ),
            LeadVocal => (
                100.,
                vec![(300., 0.8, -2.), (2800., 0.8, 2.)],
                -24.,
                3.5,
                12.,
                100.,
                2.,
            ),
            VocalRoom => (
                180.,
                vec![(400., 0.8, -2.), (2800., 0.8, -1.)],
                -18.,
                1.5,
                25.,
                200.,
                0.,
            ),
        };
        Self {
            file: file.into(),
            role,
            group: group.into(),
            fader_db,
            pan,
            hpf_hz: hp,
            eq: bands
                .into_iter()
                .map(|(hz, q, db)| EqBand {
                    kind: EqKind::Bell,
                    hz,
                    q,
                    db,
                })
                .collect(),
            dynamic_eq: None,
            compressor: Compressor {
                threshold_db: threshold,
                ratio,
                knee_db: 6.,
                attack_ms: attack,
                release_ms: release,
                makeup_db: makeup,
            },
        }
    }
}
pub fn example() -> Session {
    use Role::*;
    let entries = [
        ("01_Kick.wav", Kick, "kick", -3., 0.),
        ("02_Snare.wav", Snare, "snare", -5., 0.),
        ("03_Overheads.wav", Overheads, "ambience", -10., 0.),
        ("04_DrumRoom.wav", DrumRoom, "ambience", -20., 0.),
        ("05_Tom1.wav", Tom, "tom1", -9., -0.45),
        ("06_Tom2.wav", Tom, "tom2", -9., 0.),
        ("07_Tom3.wav", Tom, "tom3", -9., 0.45),
        ("08_BassDI.wav", BassDi, "bass", -4., 0.),
        ("09_BassAmp.wav", BassAmp, "bass", -14., 0.),
        ("10_ElecGtr1.wav", RhythmGuitar, "rhythm", -9., -0.65),
        ("11_ElecGtr2.wav", LeadGuitar, "lead", -9., 0.65),
        ("12_LeadVox.wav", LeadVocal, "vocal", -1., 0.),
        ("13_LeadVoxRoom.wav", VocalRoom, "vocal", -24., 0.),
    ];
    let channels: Vec<_> = entries
        .into_iter()
        .map(|(f, r, g, d, p)| Channel::preset(f, r, g, d, p))
        .collect();
    let mut groups = Vec::<Group>::new();
    for (i, ch) in channels.iter().enumerate() {
        if !groups.iter().any(|g| g.name == ch.group) {
            groups.push(Group {
                name: ch.group.clone(),
                reference: i,
                trim_db: 0.,
            });
        }
    }
    Session {
        output_mode: OutputMode::Matched,
        master_hpf_hz: 0.,
        effects: None,
        version: 1,
        sample_rate: 44100,
        block_frames: 1024,
        prepared: false,
        master_db: -6.,
        ceiling_db: -2.,
        limiter_release_ms: 100.,
        groups,
        channels,
        calibration: Calibration {
            target_rms_db: -24.,
            peak_headroom_db: -9.,
            activity_floor_db: -65.,
            relative_activity_db: 18.,
            min_trim_db: -30.,
            max_trim_db: 18.,
            up_db_per_second: 1.,
            down_db_per_second: 60.,
            initial_trim_db: 0.,
        },
    }
}

impl EqBand {
    /// Shelves use a fixed monotonic S=1 response. Q is an explicit 1/sqrt(2)
    /// sentinel, never a silently ignored console shelf bandwidth.
    pub fn valid_q(&self) -> bool {
        match self.kind {
            EqKind::Bell => range(self.q, 0.1, 10.),
            _ => (self.q - std::f64::consts::FRAC_1_SQRT_2).abs() < 1e-12,
        }
    }
}
