//! How far "open" opens: the house's normal open level, and each
//! blind's own.

use crate::ambient_lights::parse_device;
use crate::error::{Error, Result};
use niles_core::{BlindsLevel, DeviceId};
use serde::Deserialize;
use std::collections::HashMap;
use std::sync::OnceLock;

/// All the way open.
pub const FULLY_OPEN: u8 = 100;
/// How near its normal level a blind counts as already there. A
/// blind asked for 75 reports 74 or 76 and is not wrong.
pub const OPEN_TOLERANCE: u8 = 3;

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BlindsConfig {
    /// Where "open" takes a blind that has no level of its own.
    #[serde(default = "fully_open")]
    pub normal_open: u8,
    #[serde(default)]
    pub overrides: Vec<BlindOverride>,
    #[serde(skip)]
    parsed: OnceLock<HashMap<DeviceId, u8>>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BlindOverride {
    /// `z2m:living_room/left_blind`, or a bare `room/device`.
    pub device: String,
    pub normal_open: u8,
}

/// What `Open` means for a set of blinds, in the order given.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BlindsTargets {
    pub positions: Vec<u8>,
    /// True when this `Open` went all the way *because* they were
    /// all already at their normal level — the second press. False
    /// for a level of 100, which has nowhere further to go.
    pub all_the_way: bool,
}

fn fully_open() -> u8 {
    FULLY_OPEN
}

impl Default for BlindsConfig {
    fn default() -> Self {
        Self {
            normal_open: FULLY_OPEN,
            overrides: Vec::new(),
            parsed: OnceLock::new(),
        }
    }
}

impl BlindsConfig {
    fn invalid(reason: impl Into<String>) -> Error {
        Error::InvalidSection {
            section: "blinds",
            reason: reason.into(),
        }
    }

    /// The overrides as device ids, parsed once per config snapshot,
    /// as the lamp plugs are. An unparseable entry is dropped here
    /// and reported by `validate`.
    fn levels(&self) -> &HashMap<DeviceId, u8> {
        self.parsed.get_or_init(|| {
            self.overrides
                .iter()
                .filter_map(|o| {
                    parse_device("blinds", &o.device)
                        .ok()
                        .map(|id| (id, o.normal_open))
                })
                .collect()
        })
    }

    /// Where "open" takes this blind: its own level, or the house's.
    pub fn normal_open_for(&self, id: &DeviceId) -> u8 {
        self.levels().get(id).copied().unwrap_or(self.normal_open)
    }

    /// What each of `blinds` should be told, for a command asking
    /// for `level`. Positions line up with the blinds given.
    pub fn targets<'a>(
        &self,
        level: BlindsLevel,
        blinds: impl IntoIterator<Item = (&'a DeviceId, Option<u8>)>,
    ) -> BlindsTargets {
        match level {
            BlindsLevel::Exact(p) => {
                let n = blinds.into_iter().count();
                BlindsTargets {
                    positions: vec![p; n],
                    all_the_way: false,
                }
            }
            BlindsLevel::Open => {
                let pairs: Vec<(u8, Option<u8>)> = blinds
                    .into_iter()
                    .map(|(id, current)| (self.normal_open_for(id), current))
                    .collect();
                if pairs.is_empty() {
                    return BlindsTargets {
                        positions: Vec::new(),
                        all_the_way: false,
                    };
                }
                // A blind that has never reported a position is not
                // "already there", so it gets its normal level.
                let at_normal = |&(normal, current): &(u8, Option<u8>)| {
                    current.is_some_and(|c| c.abs_diff(normal) <= OPEN_TOLERANCE)
                };
                if pairs.iter().all(at_normal) {
                    BlindsTargets {
                        positions: vec![FULLY_OPEN; pairs.len()],
                        all_the_way: pairs.iter().any(|(n, _)| *n < FULLY_OPEN),
                    }
                } else {
                    BlindsTargets {
                        positions: pairs.iter().map(|(n, _)| *n).collect(),
                        all_the_way: false,
                    }
                }
            }
        }
    }

    pub fn validate(&self) -> Result<()> {
        if !(1..=FULLY_OPEN).contains(&self.normal_open) {
            return Err(Self::invalid(format!(
                "the normal open level is 1–100%, not {}",
                self.normal_open
            )));
        }
        let mut seen = HashMap::new();
        for o in &self.overrides {
            let id = parse_device("blinds", &o.device)?;
            if !(1..=FULLY_OPEN).contains(&o.normal_open) {
                return Err(Self::invalid(format!(
                    "the normal open level for {id} is 1–100%, not {}",
                    o.normal_open
                )));
            }
            if seen.insert(id.clone(), ()).is_some() {
                return Err(Self::invalid(format!("{id} is listed twice")));
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id(raw: &str) -> DeviceId {
        DeviceId::parse(raw).unwrap()
    }

    fn cfg(toml: &str) -> BlindsConfig {
        toml::from_str(toml).unwrap()
    }

    #[test]
    fn an_empty_section_is_fully_open() {
        let cfg = cfg("");
        assert_eq!(cfg.normal_open, 100);
        assert!(cfg.overrides.is_empty());
        cfg.validate().unwrap();
    }

    #[test]
    fn the_house_level_and_an_override_parse() {
        let cfg = cfg(r#"
                normal_open = 75

                [[overrides]]
                device = "z2m:office/blind"
                normal_open = 60
            "#);
        assert_eq!(cfg.normal_open, 75);
        assert_eq!(cfg.normal_open_for(&id("z2m:office/blind")), 60);
        assert_eq!(cfg.normal_open_for(&id("z2m:kitchen/blind")), 75);
        cfg.validate().unwrap();
    }

    #[test]
    fn a_bare_id_means_zigbee() {
        let cfg = cfg(r#"
                [[overrides]]
                device = "living_room/left_blind"
                normal_open = 60
            "#);
        assert_eq!(cfg.normal_open_for(&id("z2m:living_room/left_blind")), 60);
    }

    #[test]
    fn zero_is_not_a_normal_open_level() {
        let msg = cfg("normal_open = 0").validate().unwrap_err().to_string();
        assert!(msg.contains("blinds") && msg.contains("0"), "{msg}");
        let msg = cfg("normal_open = 101").validate().unwrap_err().to_string();
        assert!(msg.contains("blinds") && msg.contains("101"), "{msg}");
    }

    #[test]
    fn an_override_level_out_of_range_names_the_device() {
        let cfg = cfg(r#"
                [[overrides]]
                device = "z2m:office/blind"
                normal_open = 0
            "#);
        let msg = cfg.validate().unwrap_err().to_string();
        assert!(
            msg.contains("blinds") && msg.contains("office/blind") && msg.contains("0"),
            "{msg}"
        );
    }

    #[test]
    fn a_nonsense_device_is_refused() {
        let cfg = cfg(r#"
                [[overrides]]
                device = "nonsense"
                normal_open = 60
            "#);
        let msg = cfg.validate().unwrap_err().to_string();
        assert!(msg.contains("blinds") && msg.contains("nonsense"), "{msg}");
    }

    #[test]
    fn a_device_listed_twice_is_refused() {
        let cfg = cfg(r#"
                [[overrides]]
                device = "z2m:office/blind"
                normal_open = 60

                [[overrides]]
                device = "office/blind"
                normal_open = 70
            "#);
        let msg = cfg.validate().unwrap_err().to_string();
        assert!(
            msg.contains("blinds") && msg.contains("office/blind") && msg.contains("twice"),
            "{msg}"
        );
    }

    #[test]
    fn an_exact_level_is_that_for_every_blind() {
        let cfg = cfg("normal_open = 75");
        let targets = cfg.targets(
            BlindsLevel::Exact(40),
            [
                (&id("z2m:office/blind"), Some(10)),
                (&id("z2m:kitchen/blind"), None),
            ],
        );
        assert_eq!(targets.positions, vec![40, 40]);
        assert!(!targets.all_the_way);
    }

    #[test]
    fn the_second_press_goes_all_the_way() {
        let cfg = cfg("normal_open = 75");
        let blind = id("z2m:office/blind");
        let targets = cfg.targets(BlindsLevel::Open, [(&blind, Some(75))]);
        assert_eq!(targets.positions, vec![100]);
        assert!(targets.all_the_way);
    }

    #[test]
    fn not_there_yet_goes_to_the_normal_level() {
        let cfg = cfg("normal_open = 75");
        let blind = id("z2m:office/blind");
        // Part-open, fully open (nowhere to stay), and never heard from
        // all come back to the normal level.
        for at in [Some(10), Some(100), None] {
            let targets = cfg.targets(BlindsLevel::Open, [(&blind, at)]);
            assert_eq!(targets.positions, vec![75], "at {at:?}");
            assert!(!targets.all_the_way);
        }
    }

    #[test]
    fn within_tolerance_counts_as_there() {
        let cfg = cfg("normal_open = 75");
        let blind = id("z2m:office/blind");
        for at in [72, 78] {
            let targets = cfg.targets(BlindsLevel::Open, [(&blind, Some(at))]);
            assert_eq!(targets.positions, vec![100], "at {at}");
            assert!(targets.all_the_way);
        }
        for at in [71, 79] {
            let targets = cfg.targets(BlindsLevel::Open, [(&blind, Some(at))]);
            assert_eq!(targets.positions, vec![75], "at {at}");
            assert!(!targets.all_the_way);
        }
    }

    #[test]
    fn a_normal_level_of_100_has_nowhere_further_to_go() {
        let cfg = cfg("");
        let blind = id("z2m:office/blind");
        let targets = cfg.targets(BlindsLevel::Open, [(&blind, Some(100))]);
        assert_eq!(targets.positions, vec![100]);
        assert!(!targets.all_the_way);
        let targets = cfg.targets(BlindsLevel::Open, [(&blind, Some(40))]);
        assert_eq!(targets.positions, vec![100]);
        assert!(!targets.all_the_way);
    }

    #[test]
    fn a_mixed_group_each_goes_to_its_own_level() {
        let cfg = cfg("normal_open = 75");
        let office = id("z2m:office/blind");
        let kitchen = id("z2m:kitchen/blind");
        let targets = cfg.targets(
            BlindsLevel::Open,
            [(&office, Some(75)), (&kitchen, Some(0))],
        );
        assert_eq!(targets.positions, vec![75, 75]);
        assert!(!targets.all_the_way);
    }

    #[test]
    fn a_group_all_at_their_levels_goes_all_the_way() {
        let toml = r#"
            normal_open = 75

            [[overrides]]
            device = "z2m:kitchen/blind"
            normal_open = 60
        "#;
        let cfg = cfg(toml);
        let office = id("z2m:office/blind");
        let kitchen = id("z2m:kitchen/blind");
        let targets = cfg.targets(
            BlindsLevel::Open,
            [(&office, Some(75)), (&kitchen, Some(60))],
        );
        assert_eq!(targets.positions, vec![100, 100]);
        assert!(targets.all_the_way);
    }

    #[test]
    fn a_group_of_100_and_75_both_there_goes_all_the_way() {
        let toml = r#"
            normal_open = 100

            [[overrides]]
            device = "z2m:office/blind"
            normal_open = 75
        "#;
        let cfg = cfg(toml);
        let office = id("z2m:office/blind");
        let kitchen = id("z2m:kitchen/blind");
        let targets = cfg.targets(
            BlindsLevel::Open,
            [(&office, Some(75)), (&kitchen, Some(100))],
        );
        assert_eq!(targets.positions, vec![100, 100]);
        assert!(targets.all_the_way);
    }

    #[test]
    fn an_empty_group_is_empty() {
        let cfg = cfg("normal_open = 75");
        let targets: BlindsTargets = cfg.targets(BlindsLevel::Open, std::iter::empty());
        assert!(targets.positions.is_empty());
        assert!(!targets.all_the_way);
    }
}
