// Automatic studio-configuration setup.
//
// After a Sling login (and on startup while the studio isn't configured) the
// frontend asks the backend to detect the studio's org / acting-user / home
// location from Sling. `decide` is the pure decision rule; the command in
// commands.rs does the I/O around it (load config → discover → decide →
// autosave when safe).
//
//   config unset + one candidate per field  → AutoSave
//   config unset + several candidates       → Ask (picker dialog)
//   config set   + Sling agrees             → Ok (nothing to do)
//   config set   + Sling disagrees          → Mismatch (non-blocking banner;
//                                              never overwrites)

use serde::Serialize;

use crate::sling::{DiscoveredStudio, StudioConfig};

/// A studio config is complete when every identifier is set (non-zero).
pub fn is_complete(cfg: &StudioConfig) -> bool {
    cfg.org_id > 0 && cfg.acting_user_id > 0 && cfg.home_location_id > 0
}

/// The candidate ids detection found for each field.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Candidates {
    pub orgs: Vec<i64>,
    pub users: Vec<i64>,
    pub locations: Vec<i64>,
}

impl Candidates {
    pub fn from_discovered(d: &DiscoveredStudio) -> Self {
        Candidates {
            orgs: vec![d.org_id],
            users: vec![d.acting_user_id],
            locations: d.locations.iter().map(|l| l.id).collect(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Decision {
    /// Unset config, exactly one candidate per field: save these.
    AutoSave { org_id: i64, acting_user_id: i64, home_location_id: i64 },
    /// Unset config with several (or zero) candidates for some field.
    Ask,
    /// Config set and consistent with the logged-in Sling user.
    Ok,
    /// Config set but the Sling login disagrees; one reason per field.
    Mismatch { reasons: Vec<String> },
}

impl Decision {
    pub fn kind(&self) -> &'static str {
        match self {
            Decision::AutoSave { .. } => "autosaved",
            Decision::Ask => "ask",
            Decision::Ok => "ok",
            Decision::Mismatch { .. } => "mismatch",
        }
    }
}

/// Pure decision rule — see the module comment.
pub fn decide(current: &StudioConfig, found: &Candidates) -> Decision {
    if !is_complete(current) {
        return match (found.orgs.as_slice(), found.users.as_slice(), found.locations.as_slice()) {
            ([o], [u], [l]) => Decision::AutoSave {
                org_id: *o,
                acting_user_id: *u,
                home_location_id: *l,
            },
            _ => Decision::Ask,
        };
    }
    let mut reasons = Vec::new();
    if !found.orgs.contains(&current.org_id) {
        reasons.push(format!(
            "The Sling login belongs to a different organization than the configured one (org {}).",
            current.org_id
        ));
    }
    if !found.users.contains(&current.acting_user_id) {
        reasons.push(format!(
            "The logged-in Sling user isn't the configured acting user ({}).",
            current.acting_user_id
        ));
    }
    if !found.locations.contains(&current.home_location_id) {
        reasons.push(format!(
            "The logged-in Sling user can't see the configured home location ({}).",
            current.home_location_id
        ));
    }
    if reasons.is_empty() { Decision::Ok } else { Decision::Mismatch { reasons } }
}

/// Result of `auto_detect_studio_config`, serialized for the frontend.
#[derive(Debug, Clone, Serialize)]
pub struct DetectOutcome {
    /// "autosaved" | "ask" | "ok" | "mismatch"
    pub decision: &'static str,
    pub discovered: DiscoveredStudio,
    /// The config after this call (i.e. including an autosave).
    pub current: crate::commands::StudioConfigDto,
    pub reasons: Vec<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn unset() -> StudioConfig {
        StudioConfig { org_id: 0, acting_user_id: 0, home_location_id: 0 }
    }
    fn set() -> StudioConfig {
        StudioConfig { org_id: 10, acting_user_id: 20, home_location_id: 30 }
    }
    fn cands(orgs: &[i64], users: &[i64], locs: &[i64]) -> Candidates {
        Candidates { orgs: orgs.to_vec(), users: users.to_vec(), locations: locs.to_vec() }
    }

    #[test]
    fn unset_with_single_candidates_autosaves() {
        assert_eq!(
            decide(&unset(), &cands(&[10], &[20], &[30])),
            Decision::AutoSave { org_id: 10, acting_user_id: 20, home_location_id: 30 }
        );
    }

    #[test]
    fn unset_with_multiple_or_no_candidates_asks() {
        assert_eq!(decide(&unset(), &cands(&[10], &[20], &[30, 31])), Decision::Ask);
        assert_eq!(decide(&unset(), &cands(&[10, 11], &[20], &[30])), Decision::Ask);
        assert_eq!(decide(&unset(), &cands(&[10], &[20], &[])), Decision::Ask);
    }

    #[test]
    fn partially_set_counts_as_unset() {
        let partial = StudioConfig { org_id: 10, acting_user_id: 0, home_location_id: 0 };
        assert!(matches!(
            decide(&partial, &cands(&[10], &[20], &[30])),
            Decision::AutoSave { .. }
        ));
    }

    #[test]
    fn set_and_matching_does_nothing() {
        assert_eq!(decide(&set(), &cands(&[10], &[20], &[30])), Decision::Ok);
        // Several visible locations are fine as long as the configured one is among them.
        assert_eq!(decide(&set(), &cands(&[10], &[20], &[29, 30, 31])), Decision::Ok);
    }

    #[test]
    fn set_and_mismatching_warns_without_saving() {
        match decide(&set(), &cands(&[10], &[99], &[31])) {
            Decision::Mismatch { reasons } => {
                assert_eq!(reasons.len(), 2);
                assert!(reasons[0].contains("acting user"));
                assert!(reasons[1].contains("home location"));
            }
            d => panic!("expected mismatch, got {d:?}"),
        }
        assert!(matches!(
            decide(&set(), &cands(&[11], &[20], &[30])),
            Decision::Mismatch { .. }
        ));
    }

    #[test]
    fn decision_kinds() {
        assert_eq!(Decision::Ask.kind(), "ask");
        assert_eq!(Decision::Ok.kind(), "ok");
        assert_eq!(Decision::Mismatch { reasons: vec![] }.kind(), "mismatch");
    }
}
