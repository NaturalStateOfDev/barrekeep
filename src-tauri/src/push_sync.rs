// Incremental push ("sync") of the month's push draft to Sling, cleanup of
// a previously pushed draft's shifts, and the standalone "remove this
// draft's shifts from Sling" action. Migration 0013; docs/sling-api.md.
//
// Tracking model. Every Sling shift the app creates gets a push_results row
// (outcome 'created' / 'updated' / 'adopted') carrying its sling_shift_id and
// a push_result_snapshots row with exactly what was sent. The LATEST tracking
// row per sling_shift_id decides its state: created/updated/adopted = live
// and owned by that row's draft; deleted/skipped_missing = no longer
// tracked. Nothing else in Sling is ever touched — a shift the app didn't
// create has no tracking row, so it can never become an update or delete.
//
// Safety. Before any update or delete, the shift is looked up in a fresh
// month calendar fetch (one GET for the whole month): it must still exist,
// still be `planning`, and still match the snapshot (else someone edited it
// in Sling). Pre-0013 rows have no snapshot, so they are never changed
// automatically.
//
// Updates are DELETE + POST, not PUT: no PUT request has ever been captured
// or exercised against Sling by this project (docs/sling-api.md only notes
// the singular `user` shape), while POST and DELETE are both proven. A
// replaced shift gets a new Sling id; the old one is recorded 'deleted'.
//
// Everything that decides WHAT to do (build_push_plan / build_remove_plan)
// and HOW to run it (execute_plan over the SlingShiftOps trait) is pure and
// unit-tested without the network.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

use serde::Serialize;
use tauri::State;

use crate::commands::SlingToken;
use crate::db::Db;
use crate::sling::{
    event_user_id, existing_fingerprints, spec_fingerprint, split_dt, CalendarEvent, DeleteOutcome, PushSpec,
    StudioConfig,
};

fn err(e: impl std::fmt::Display) -> String {
    format!("{e:#}")
}

/// One Sling shift as the app sees it: studio-local date and times.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize)]
pub struct ShiftState {
    pub date: String,  // "YYYY-MM-DD"
    pub start: String, // "HH:MM"
    pub end: String,   // "HH:MM"
    pub user_id: i64,
    pub position_id: i64,
}

impl ShiftState {
    pub fn from_spec(s: &PushSpec) -> Self {
        ShiftState {
            date: s.date.clone(),
            start: s.start.clone(),
            end: s.end.clone(),
            user_id: s.user_id,
            position_id: s.position_id,
        }
    }

    fn to_spec(&self) -> PushSpec {
        PushSpec {
            proposal_shift_id: 0,
            date: self.date.clone(),
            start: self.start.clone(),
            end: self.end.clone(),
            position_id: self.position_id,
            user_id: self.user_id,
        }
    }
}

/// The state of a calendar shift event, or None when it has no assigned
/// user / position (an app-created shift never looks like that).
pub fn event_state(ev: &CalendarEvent) -> Option<ShiftState> {
    let (date, start) = split_dt(&ev.dtstart);
    let (_, end) = split_dt(&ev.dtend);
    Some(ShiftState {
        date,
        start,
        end,
        user_id: event_user_id(ev)?,
        position_id: ev.position.as_ref()?.id,
    })
}

/// A live Sling shift the app created (latest tracking row is
/// created / updated / adopted).
#[derive(Debug, Clone)]
pub struct TrackedShift {
    pub sling_shift_id: i64,
    pub proposal_id: i64,
    pub proposal_shift_id: i64,
    /// What the app last sent. None = pushed before migration 0013.
    pub snapshot: Option<ShiftState>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ActionKind {
    // Declaration order = execution order: DB-only first, then deletes
    // (cleanup of the previous draft before this draft's creates), then
    // replacements, then creates.
    /// A pre-0013 shift that matches the draft exactly and is still
    /// planning: record Sling's state as its snapshot (on execute only), so
    /// it is syncable from then on. No Sling call.
    Baseline,
    Adopt,
    Skip,
    Cleanup,
    Delete,
    Update,
    Create,
}

#[derive(Debug, Clone, Serialize)]
pub struct SyncAction {
    pub kind: ActionKind,
    pub proposal_shift_id: i64,
    /// The existing Sling shift acted on (update/delete/cleanup/adopt/skip).
    pub sling_shift_id: Option<i64>,
    /// What's in Sling now (update/delete/cleanup) or was last pushed.
    pub before: Option<ShiftState>,
    /// What the draft wants (create/update/adopt).
    pub after: Option<ShiftState>,
    pub reason: String,
    /// For skips: the push_results outcome to record. None = not recorded
    /// (a create skipped because an identical non-app shift exists).
    pub skip_outcome: Option<String>,
    /// For cleanup/adopt: the other draft that owned the shift.
    pub from_proposal_id: Option<i64>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct CleanupOffer {
    pub proposal_id: i64,
    /// Planning, unmodified shifts that can be removed.
    pub removable: usize,
    /// Shifts that will be left alone (published / edited in Sling / legacy).
    pub blocked: usize,
}

#[derive(Debug, Clone, Serialize, Default)]
pub struct SyncPlan {
    pub actions: Vec<SyncAction>,
    /// Draft shifts already in Sling exactly as the draft has them.
    pub unchanged: usize,
    /// Other drafts of the month that still own Sling shifts.
    pub cleanup_offers: Vec<CleanupOffer>,
}

impl SyncPlan {
    /// A compact identity of the plan. Execute recomputes the plan from a
    /// fresh fetch and refuses to run if this changed since the preview the
    /// user confirmed.
    pub fn key(&self) -> String {
        self.actions
            .iter()
            .map(|a| {
                let st = |s: &Option<ShiftState>| {
                    s.as_ref()
                        .map(|s| format!("{}T{}-{}/{}/{}", s.date, s.start, s.end, s.user_id, s.position_id))
                        .unwrap_or_default()
                };
                format!(
                    "{:?}:{}:{}:{}",
                    a.kind,
                    a.proposal_shift_id,
                    a.sling_shift_id.map(|i| i.to_string()).unwrap_or_default(),
                    st(&a.after)
                )
            })
            .collect::<Vec<_>>()
            .join("|")
    }

    /// Actions that call Sling.
    pub fn network_ops(&self) -> usize {
        self.actions
            .iter()
            .filter(|a| matches!(a.kind, ActionKind::Create | ActionKind::Update | ActionKind::Delete | ActionKind::Cleanup))
            .count()
    }

    fn sort(&mut self) {
        self.actions.sort_by(|a, b| {
            let k = |x: &SyncAction| {
                let s = x.after.as_ref().or(x.before.as_ref());
                (x.kind, s.map(|s| (s.date.clone(), s.start.clone())), x.proposal_shift_id, x.sling_shift_id)
            };
            k(a).cmp(&k(b))
        });
    }
}

/// Why a tracked shift can't be updated or deleted.
#[derive(Debug, Clone, PartialEq)]
pub enum Unsafe {
    Missing,
    NotPlanning(String),
    Modified(String),
    Legacy,
}

/// A pre-0013 push (no snapshot) that doesn't match the draft: the app can't
/// tell a draft edit from a Sling edit, so it never touches it.
pub const LEGACY_REASON: &str =
    "pushed before sync tracking; differs from draft — fix in Sling or remove manually";
/// A tracked shift the draft still wants was deleted in Sling.
pub const MISSING_WANTED: &str =
    "deleted in Sling since last push — will be re-created on the next push unless you remove it from the draft";
/// A tracked shift the draft no longer wants is already gone.
pub const MISSING_GONE: &str = "already deleted in Sling — nothing to remove";

impl Unsafe {
    fn outcome(&self) -> &'static str {
        match self {
            Unsafe::Missing => "skipped_missing",
            _ => "skipped_conflict",
        }
    }
    fn reason(&self, wanted: &str) -> String {
        match self {
            // Context-specific: "will be re-created…" vs MISSING_GONE.
            Unsafe::Missing => wanted.to_string(),
            Unsafe::NotPlanning(s) => format!("is {s} in Sling — the app only changes planning shifts"),
            Unsafe::Modified(what) => format!("was edited in Sling since the app pushed it ({what}) — left alone"),
            Unsafe::Legacy => LEGACY_REASON.to_string(),
        }
    }
}

type EventsById<'a> = HashMap<i64, &'a CalendarEvent>;

fn shifts_by_id(events: &[CalendarEvent]) -> EventsById<'_> {
    events
        .iter()
        .filter(|e| e.kind == "shift")
        .filter_map(|e| e.id.map(|id| (id, e)))
        .collect()
}

fn describe_diff(a: &ShiftState, b: &ShiftState) -> String {
    let mut parts = Vec::new();
    if a.user_id != b.user_id {
        parts.push("teacher");
    }
    if a.date != b.date || a.start != b.start || a.end != b.end {
        parts.push("time");
    }
    if a.position_id != b.position_id {
        parts.push("class");
    }
    if parts.is_empty() {
        "details".to_string()
    } else {
        format!("{} changed", parts.join(" + "))
    }
}

/// Planning-only, unmodified, still present. Returns Sling's current state.
pub fn check_safe(t: &TrackedShift, by_id: &EventsById<'_>, home_location_id: i64) -> Result<ShiftState, Unsafe> {
    let ev = by_id.get(&t.sling_shift_id).ok_or(Unsafe::Missing)?;
    let status = ev.status.clone().unwrap_or_else(|| "unknown".to_string());
    if status != "planning" {
        return Err(Unsafe::NotPlanning(status));
    }
    if let Some(loc) = ev.location.as_ref() {
        if loc.id != home_location_id {
            return Err(Unsafe::Modified("moved to another location".to_string()));
        }
    }
    let current = event_state(ev).ok_or_else(|| Unsafe::Modified("teacher removed".to_string()))?;
    match &t.snapshot {
        None => Err(Unsafe::Legacy),
        Some(snap) if *snap != current => Err(Unsafe::Modified(describe_diff(snap, &current))),
        Some(_) => Ok(current),
    }
}

fn skip(t: &TrackedShift, u: &Unsafe, wanted: &str, after: Option<ShiftState>, by_id: &EventsById<'_>) -> SyncAction {
    SyncAction {
        kind: ActionKind::Skip,
        proposal_shift_id: t.proposal_shift_id,
        sling_shift_id: Some(t.sling_shift_id),
        before: by_id.get(&t.sling_shift_id).and_then(|e| event_state(e)).or_else(|| t.snapshot.clone()),
        after,
        reason: u.reason(wanted),
        skip_outcome: Some(u.outcome().to_string()),
        from_proposal_id: None,
    }
}

pub struct PlanInput<'a> {
    pub proposal_id: i64,
    /// The draft's current push specs (dropped shifts excluded, co-teach expanded).
    pub specs: &'a [PushSpec],
    /// Every live tracked Sling shift of the month, any draft.
    pub tracked: &'a [TrackedShift],
    /// The month's calendar, fetched just now.
    pub events: &'a [CalendarEvent],
    pub home_location_id: i64,
    /// Remove other drafts' removable shifts as part of this push.
    pub cleanup: bool,
}

/// Plan an incremental push of `proposal_id` (the month's push draft).
pub fn build_push_plan(inp: &PlanInput<'_>) -> SyncPlan {
    let by_id = shifts_by_id(inp.events);
    let home = inp.home_location_id;
    let mut plan = SyncPlan::default();
    // Sling ids this plan deletes (their fingerprints must not dedupe creates).
    let mut removing: HashSet<i64> = HashSet::new();

    let mut specs_by_ps: BTreeMap<i64, Vec<&PushSpec>> = BTreeMap::new();
    for s in inp.specs {
        specs_by_ps.entry(s.proposal_shift_id).or_default().push(s);
    }
    let mut own_by_ps: BTreeMap<i64, Vec<&TrackedShift>> = BTreeMap::new();
    let mut others: Vec<&TrackedShift> = Vec::new();
    for t in inp.tracked {
        if t.proposal_id == inp.proposal_id {
            own_by_ps.entry(t.proposal_shift_id).or_default().push(t);
        } else {
            others.push(t);
        }
    }
    let psids: BTreeSet<i64> = specs_by_ps.keys().chain(own_by_ps.keys()).copied().collect();

    let mut create_candidates: Vec<&PushSpec> = Vec::new();
    for ps in psids {
        let mut specs: Vec<&PushSpec> = specs_by_ps.remove(&ps).unwrap_or_default();
        let tracked: Vec<&TrackedShift> = own_by_ps.remove(&ps).unwrap_or_default();

        // 1. Exact matches: already in Sling as the draft has it. A co-teach
        //    slot matches each teacher's shift independently.
        let mut left: Vec<&TrackedShift> = Vec::new();
        for t in tracked {
            let effective = t
                .snapshot
                .clone()
                .or_else(|| by_id.get(&t.sling_shift_id).and_then(|e| event_state(e)));
            let hit = effective
                .as_ref()
                .and_then(|eff| specs.iter().position(|s| ShiftState::from_spec(s) == *eff));
            match hit {
                Some(pos) => {
                    let s = specs.remove(pos);
                    let want = ShiftState::from_spec(s);
                    match by_id.get(&t.sling_shift_id) {
                        None => plan.actions.push(skip(t, &Unsafe::Missing, MISSING_WANTED, Some(want), &by_id)),
                        // Legacy (no snapshot) and Sling == draft: adopt Sling's
                        // state as the baseline if it's still ours to change.
                        Some(ev)
                            if t.snapshot.is_none()
                                && ev.status.as_deref() == Some("planning")
                                && ev.location.as_ref().is_none_or(|l| l.id == home) =>
                        {
                            plan.actions.push(SyncAction {
                                kind: ActionKind::Baseline,
                                proposal_shift_id: ps,
                                sling_shift_id: Some(t.sling_shift_id),
                                before: Some(want.clone()),
                                after: Some(want),
                                reason: "pushed before sync tracking; matches the draft — tracked from now on".to_string(),
                                skip_outcome: None,
                                from_proposal_id: None,
                            });
                        }
                        Some(_) => plan.unchanged += 1,
                    }
                }
                None => left.push(t),
            }
        }

        // 2. Pair the rest into updates — same teacher first (a time/class
        //    change), then whatever is left (a teacher change).
        let mut pairs: Vec<(&TrackedShift, &PushSpec)> = Vec::new();
        let mut unpaired: Vec<&TrackedShift> = Vec::new();
        for t in left {
            let uid = t
                .snapshot
                .as_ref()
                .map(|s| s.user_id)
                .or_else(|| by_id.get(&t.sling_shift_id).and_then(|e| event_user_id(e)));
            match specs.iter().position(|s| Some(s.user_id) == uid) {
                Some(pos) => pairs.push((t, specs.remove(pos))),
                None => unpaired.push(t),
            }
        }
        while !unpaired.is_empty() && !specs.is_empty() {
            pairs.push((unpaired.remove(0), specs.remove(0)));
        }
        for (t, s) in pairs {
            let after = ShiftState::from_spec(s);
            match check_safe(t, &by_id, home) {
                Ok(current) => {
                    removing.insert(t.sling_shift_id);
                    plan.actions.push(SyncAction {
                        kind: ActionKind::Update,
                        proposal_shift_id: ps,
                        sling_shift_id: Some(t.sling_shift_id),
                        before: Some(current),
                        after: Some(after),
                        reason: "changed in the draft since the last push".to_string(),
                        skip_outcome: None,
                        from_proposal_id: None,
                    });
                }
                Err(u) => plan.actions.push(skip(
                    t,
                    &u,
                    MISSING_WANTED,
                    Some(after),
                    &by_id,
                )),
            }
        }
        create_candidates.extend(specs);

        // 3. Tracked shifts the draft no longer has (dropped / removed / a
        //    co-teach partner taken off) → delete.
        for t in unpaired {
            match check_safe(t, &by_id, home) {
                Ok(current) => {
                    removing.insert(t.sling_shift_id);
                    plan.actions.push(SyncAction {
                        kind: ActionKind::Delete,
                        proposal_shift_id: ps,
                        sling_shift_id: Some(t.sling_shift_id),
                        before: Some(current),
                        after: None,
                        reason: "no longer in the draft".to_string(),
                        skip_outcome: None,
                        from_proposal_id: None,
                    });
                }
                Err(u) => plan.actions.push(skip(t, &u, MISSING_GONE, None, &by_id)),
            }
        }
    }

    // 4. Adopt: another draft's live, planning, unmodified shift that is
    //    exactly what this draft wants becomes this draft's — no Sling call,
    //    no delete-then-recreate churn when switching between similar drafts.
    let mut adopted: HashSet<i64> = HashSet::new();
    let mut creates: Vec<&PushSpec> = Vec::new();
    for s in create_candidates {
        let want = ShiftState::from_spec(s);
        let found = others.iter().find(|t| {
            if adopted.contains(&t.sling_shift_id) {
                return false;
            }
            let Some(ev) = by_id.get(&t.sling_shift_id) else { return false };
            ev.status.as_deref() == Some("planning")
                && ev.location.as_ref().is_none_or(|l| l.id == home)
                && event_state(ev).as_ref() == Some(&want)
                && t.snapshot.as_ref().is_none_or(|snap| *snap == want)
        });
        match found {
            Some(t) => {
                adopted.insert(t.sling_shift_id);
                plan.actions.push(SyncAction {
                    kind: ActionKind::Adopt,
                    proposal_shift_id: s.proposal_shift_id,
                    sling_shift_id: Some(t.sling_shift_id),
                    before: Some(want.clone()),
                    after: Some(want),
                    reason: "already in Sling from another draft — tracked for this draft now".to_string(),
                    skip_outcome: None,
                    from_proposal_id: Some(t.proposal_id),
                });
            }
            None => creates.push(s),
        }
    }

    // 5. Other drafts' remaining shifts: offered for cleanup (default off).
    let mut offers: BTreeMap<i64, CleanupOffer> = BTreeMap::new();
    for t in others.iter().filter(|t| !adopted.contains(&t.sling_shift_id)) {
        let offer = offers.entry(t.proposal_id).or_insert(CleanupOffer {
            proposal_id: t.proposal_id,
            removable: 0,
            blocked: 0,
        });
        let verdict = check_safe(t, &by_id, home);
        match &verdict {
            Ok(_) => offer.removable += 1,
            Err(Unsafe::Missing) => {}
            Err(_) => offer.blocked += 1,
        }
        if !inp.cleanup {
            continue;
        }
        match verdict {
            Ok(current) => {
                removing.insert(t.sling_shift_id);
                plan.actions.push(SyncAction {
                    kind: ActionKind::Cleanup,
                    proposal_shift_id: t.proposal_shift_id,
                    sling_shift_id: Some(t.sling_shift_id),
                    before: Some(current),
                    after: None,
                    reason: "pushed from another draft of this month".to_string(),
                    skip_outcome: None,
                    from_proposal_id: Some(t.proposal_id),
                });
            }
            Err(u) => {
                let mut a = skip(t, &u, MISSING_GONE, None, &by_id);
                a.from_proposal_id = Some(t.proposal_id);
                plan.actions.push(a);
            }
        }
    }
    plan.cleanup_offers = offers.into_values().filter(|o| o.removable + o.blocked > 0).collect();

    // 6. Dedupe creates against what's in Sling, ignoring shifts this plan
    //    deletes (a slot replaced in place is not a duplicate of itself).
    //    Same fingerprints as the original push dedupe.
    let kept: Vec<CalendarEvent> = inp
        .events
        .iter()
        .filter(|e| e.id.is_none_or(|id| !removing.contains(&id)))
        .cloned()
        .collect();
    let existing = existing_fingerprints(&kept, home);
    for s in creates {
        let after = ShiftState::from_spec(s);
        if existing.contains(&spec_fingerprint(s, home)) {
            plan.actions.push(SyncAction {
                kind: ActionKind::Skip,
                proposal_shift_id: s.proposal_shift_id,
                sling_shift_id: None,
                before: None,
                after: Some(after),
                reason: "an identical shift the app didn't create is already in Sling — not duplicated".to_string(),
                skip_outcome: None,
                from_proposal_id: None,
            });
        } else {
            plan.actions.push(SyncAction {
                kind: ActionKind::Create,
                proposal_shift_id: s.proposal_shift_id,
                sling_shift_id: None,
                before: None,
                after: Some(after),
                reason: "not in Sling yet".to_string(),
                skip_outcome: None,
                from_proposal_id: None,
            });
        }
    }

    plan.sort();
    plan
}

/// Plan removing every live Sling shift `proposal_id` owns (same safety).
pub fn build_remove_plan(
    proposal_id: i64,
    tracked: &[TrackedShift],
    events: &[CalendarEvent],
    home_location_id: i64,
) -> SyncPlan {
    let by_id = shifts_by_id(events);
    let mut plan = SyncPlan::default();
    for t in tracked.iter().filter(|t| t.proposal_id == proposal_id) {
        match check_safe(t, &by_id, home_location_id) {
            Ok(current) => plan.actions.push(SyncAction {
                kind: ActionKind::Delete,
                proposal_shift_id: t.proposal_shift_id,
                sling_shift_id: Some(t.sling_shift_id),
                before: Some(current),
                after: None,
                reason: "removing this draft's shifts from Sling".to_string(),
                skip_outcome: None,
                from_proposal_id: None,
            }),
            Err(u) => plan.actions.push(skip(t, &u, MISSING_GONE, None, &by_id)),
        }
    }
    plan.sort();
    plan
}

// ============================================================
// Execution — over a trait so tests run without the network
// ============================================================

pub trait SlingShiftOps {
    /// POST one planning shift; returns the new Sling id.
    fn create(&mut self, st: &ShiftState) -> anyhow::Result<i64>;
    fn delete(&mut self, sling_shift_id: i64) -> anyhow::Result<DeleteOutcome>;
    fn pause(&mut self, secs: u64);
}

/// The real thing: sling::push_shift / sling::delete_shift (both with the
/// 429 backoff policy) and a real sleep.
pub struct LiveSling<'a> {
    pub token: &'a str,
    pub cfg: &'a StudioConfig,
    pub viewdates: String,
    pub cachedates: String,
}

impl SlingShiftOps for LiveSling<'_> {
    fn create(&mut self, st: &ShiftState) -> anyhow::Result<i64> {
        crate::sling::push_shift(self.token, self.cfg, &st.to_spec(), &self.viewdates, &self.cachedates)
    }
    fn delete(&mut self, sling_shift_id: i64) -> anyhow::Result<DeleteOutcome> {
        crate::sling::delete_shift(self.token, self.cfg, sling_shift_id, &self.viewdates, &self.cachedates)
    }
    fn pause(&mut self, secs: u64) {
        std::thread::sleep(std::time::Duration::from_secs(secs));
    }
}

/// One push_results row (+ snapshot) to record.
#[derive(Debug, Clone, PartialEq)]
pub struct ResultRow {
    pub proposal_shift_id: i64,
    pub outcome: String,
    pub sling_shift_id: Option<i64>,
    pub error: Option<String>,
    pub snapshot: Option<ShiftState>,
}

#[derive(Debug, Clone, Serialize, Default, PartialEq)]
pub struct SyncSummary {
    pub push_id: i64,
    pub created: i64,
    pub updated: i64,
    pub deleted: i64,
    pub adopted: i64,
    pub skipped: i64,
    pub failed: i64,
    /// Stopped early on an expired token.
    pub aborted: bool,
    /// Set when the pre-sync database backup failed (the sync still ran).
    pub backup_warning: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct SyncProgress {
    pub total: i64,
    pub done: i64,
    pub created: i64,
    pub updated: i64,
    pub deleted: i64,
    pub failed: i64,
    pub last_label: String,
    pub last_outcome: String,
}

pub const BATCH_SIZE: usize = 10;
pub const INTRA_DELAY_SECS: u64 = 1;
pub const INTER_DELAY_SECS: u64 = 10;

/// Rate limiting across ALL Sling calls of a run (a replacement is two):
/// 1s between calls, 10s after every 10th.
struct Throttle {
    calls: usize,
}
impl Throttle {
    fn before_call(&mut self, ops: &mut dyn SlingShiftOps) {
        if self.calls > 0 {
            ops.pause(if self.calls % BATCH_SIZE == 0 { INTER_DELAY_SECS } else { INTRA_DELAY_SECS });
        }
        self.calls += 1;
    }
}

fn label(a: &SyncAction) -> String {
    let s = a.after.as_ref().or(a.before.as_ref());
    match s {
        Some(s) => format!("{:?} {} {}", a.kind, s.date, s.start).to_lowercase(),
        None => format!("{:?}", a.kind).to_lowercase(),
    }
}

/// Run a plan in order. `record` persists each outcome as it happens (a
/// crash mid-run leaves an accurate audit trail); `progress` is for the UI.
/// A 401 stops the run (aborted = true).
pub fn execute_plan(
    plan: &SyncPlan,
    ops: &mut dyn SlingShiftOps,
    record: &mut dyn FnMut(&ResultRow) -> Result<(), String>,
    progress: &mut dyn FnMut(&SyncProgress),
) -> Result<SyncSummary, String> {
    let mut sum = SyncSummary::default();
    let total = plan.network_ops() as i64;
    let mut done = 0i64;
    let mut throttle = Throttle { calls: 0 };
    let is_401 = |e: &anyhow::Error| e.to_string() == "sling-401";

    for a in &plan.actions {
        let row = |outcome: &str, sid: Option<i64>, error: Option<String>, snapshot: Option<ShiftState>| ResultRow {
            proposal_shift_id: a.proposal_shift_id,
            outcome: outcome.to_string(),
            sling_shift_id: sid,
            error,
            snapshot,
        };
        let last_outcome: String;
        match a.kind {
            // Both are bookkeeping: an 'adopted' row with a snapshot makes the
            // shift tracked (by this draft) and syncable. Written only here,
            // on execute — never on preview.
            ActionKind::Baseline | ActionKind::Adopt => {
                let note = (a.kind == ActionKind::Baseline).then(|| "baseline snapshot adopted from Sling".to_string());
                record(&row("adopted", a.sling_shift_id, note, a.after.clone()))?;
                sum.adopted += 1;
                continue;
            }
            ActionKind::Skip => {
                if let Some(o) = &a.skip_outcome {
                    record(&row(o, a.sling_shift_id, Some(a.reason.clone()), None))?;
                }
                sum.skipped += 1;
                continue;
            }
            ActionKind::Delete | ActionKind::Cleanup => {
                let sid = a.sling_shift_id.ok_or("delete without a Sling id")?;
                throttle.before_call(ops);
                match ops.delete(sid) {
                    Ok(DeleteOutcome::Deleted) => {
                        record(&row("deleted", Some(sid), None, None))?;
                        sum.deleted += 1;
                        last_outcome = "deleted".into();
                    }
                    Ok(DeleteOutcome::NotFound) => {
                        record(&row("skipped_missing", Some(sid), Some("already gone from Sling".into()), None))?;
                        sum.skipped += 1;
                        last_outcome = "already gone".into();
                    }
                    Err(e) => {
                        record(&row("failed", Some(sid), Some(e.to_string()), None))?;
                        sum.failed += 1;
                        if is_401(&e) {
                            sum.aborted = true;
                        }
                        last_outcome = "failed".into();
                    }
                }
            }
            ActionKind::Update => {
                let sid = a.sling_shift_id.ok_or("update without a Sling id")?;
                let after = a.after.clone().ok_or("update without a target")?;
                throttle.before_call(ops);
                match ops.delete(sid) {
                    Ok(DeleteOutcome::Deleted) => {
                        record(&row("deleted", Some(sid), Some("replaced by update".into()), None))?;
                        throttle.before_call(ops);
                        match ops.create(&after) {
                            Ok(new_id) => {
                                record(&row("updated", Some(new_id), None, Some(after)))?;
                                sum.updated += 1;
                                last_outcome = "updated".into();
                            }
                            Err(e) => {
                                record(&row("failed", None, Some(format!("re-create after delete: {e}")), None))?;
                                sum.failed += 1;
                                if is_401(&e) {
                                    sum.aborted = true;
                                }
                                last_outcome = "failed".into();
                            }
                        }
                    }
                    Ok(DeleteOutcome::NotFound) => {
                        record(&row("skipped_missing", Some(sid), Some("already gone from Sling".into()), None))?;
                        sum.skipped += 1;
                        last_outcome = "already gone".into();
                    }
                    Err(e) => {
                        record(&row("failed", Some(sid), Some(e.to_string()), None))?;
                        sum.failed += 1;
                        if is_401(&e) {
                            sum.aborted = true;
                        }
                        last_outcome = "failed".into();
                    }
                }
            }
            ActionKind::Create => {
                let after = a.after.clone().ok_or("create without a target")?;
                throttle.before_call(ops);
                match ops.create(&after) {
                    Ok(id) => {
                        record(&row("created", Some(id), None, Some(after)))?;
                        sum.created += 1;
                        last_outcome = "created".into();
                    }
                    Err(e) => {
                        record(&row("failed", None, Some(e.to_string()), None))?;
                        sum.failed += 1;
                        if is_401(&e) {
                            sum.aborted = true;
                        }
                        last_outcome = "failed".into();
                    }
                }
            }
        }
        done += 1;
        progress(&SyncProgress {
            total,
            done,
            created: sum.created,
            updated: sum.updated,
            deleted: sum.deleted,
            failed: sum.failed,
            last_label: label(a),
            last_outcome,
        });
        if sum.aborted {
            break;
        }
    }
    Ok(sum)
}

// ============================================================
// DB access
// ============================================================

/// Outcomes that change tracking. Everything else (failed, skipped_conflict)
/// is audit only.
const TRACKING_OUTCOMES: &str = "('created','updated','adopted','deleted','skipped_missing')";

/// Live tracked Sling shifts. `month` = None → every month.
pub fn load_tracked(conn: &duckdb::Connection, month: Option<&str>) -> Result<Vec<TrackedShift>, String> {
    let sql = format!(
        "WITH ranked AS (
            SELECT pr.id, TRY_CAST(pr.sling_shift_id AS BIGINT) AS sid, pr.outcome,
                   pr.proposal_shift_id, pr.push_id,
                   row_number() OVER (PARTITION BY TRY_CAST(pr.sling_shift_id AS BIGINT)
                                      ORDER BY pr.id DESC) AS rn
            FROM push_results pr
            WHERE TRY_CAST(pr.sling_shift_id AS BIGINT) IS NOT NULL
              AND pr.outcome IN {TRACKING_OUTCOMES}
         )
         SELECT r.sid, COALESCE(ps.proposal_id, pu.proposal_id), r.proposal_shift_id,
                s.shift_date, s.start_time, s.end_time, s.sling_user_id, s.sling_position_id
         FROM ranked r
         JOIN pushes pu ON pu.id = r.push_id
         LEFT JOIN proposal_shifts ps ON ps.id = r.proposal_shift_id
         JOIN proposals p ON p.id = COALESCE(ps.proposal_id, pu.proposal_id)
         LEFT JOIN push_result_snapshots s ON s.push_result_id = r.id
         WHERE r.rn = 1 AND r.outcome IN ('created','updated','adopted')
           {}
         ORDER BY r.sid",
        if month.is_some() { "AND p.target_month = ?" } else { "" }
    );
    let mut stmt = conn.prepare(&sql).map_err(err)?;
    let params: Vec<&dyn duckdb::ToSql> = match &month {
        Some(m) => vec![m],
        None => vec![],
    };
    let rows = stmt
        .query_map(params.as_slice(), |r| {
            let date: Option<String> = r.get(3)?;
            let snapshot = match date {
                Some(date) => Some(ShiftState {
                    date,
                    start: r.get(4)?,
                    end: r.get(5)?,
                    user_id: r.get(6)?,
                    position_id: r.get(7)?,
                }),
                None => None,
            };
            Ok(TrackedShift {
                sling_shift_id: r.get(0)?,
                proposal_id: r.get(1)?,
                proposal_shift_id: r.get(2)?,
                snapshot,
            })
        })
        .map_err(err)?;
    rows.collect::<Result<_, _>>().map_err(err)
}

/// proposal_id → number of live Sling shifts it owns.
pub fn live_counts(conn: &duckdb::Connection) -> Result<HashMap<i64, i64>, String> {
    let mut out = HashMap::new();
    for t in load_tracked(conn, None)? {
        *out.entry(t.proposal_id).or_insert(0) += 1;
    }
    Ok(out)
}

pub fn record_result(conn: &duckdb::Connection, push_id: i64, row: &ResultRow) -> Result<(), String> {
    let id: i64 = conn
        .query_row(
            "INSERT INTO push_results (push_id, proposal_shift_id, outcome, sling_shift_id, error_message)
             VALUES (?, ?, ?, ?, ?) RETURNING id",
            duckdb::params![
                push_id,
                row.proposal_shift_id,
                row.outcome,
                row.sling_shift_id.map(|i| i.to_string()),
                row.error
            ],
            |r| r.get(0),
        )
        .map_err(err)?;
    if let Some(s) = &row.snapshot {
        conn.execute(
            "INSERT INTO push_result_snapshots
                (push_result_id, sling_user_id, sling_position_id, shift_date, start_time, end_time)
             VALUES (?, ?, ?, ?, ?, ?)",
            duckdb::params![id, s.user_id, s.position_id, s.date, s.start, s.end],
        )
        .map_err(err)?;
    }
    Ok(())
}

// ============================================================
// Commands
// ============================================================

#[derive(Debug, Clone, Serialize)]
pub struct ShiftView {
    pub date: String,
    pub start: String,
    pub end: String,
    pub class_name: String,
    pub teacher_name: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct SyncActionView {
    pub kind: ActionKind,
    pub proposal_shift_id: i64,
    pub sling_shift_id: Option<i64>,
    pub before: Option<ShiftView>,
    pub after: Option<ShiftView>,
    pub reason: String,
    /// For cleanup/adopt: the other draft's name.
    pub from_draft: Option<String>,
    /// For skips: the outcome recorded on execute (None = nothing recorded).
    pub skip_outcome: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct CleanupOfferView {
    pub proposal_id: i64,
    pub draft_name: String,
    pub removable: usize,
    pub blocked: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct SyncPreview {
    pub mode: String, // "push" | "remove"
    pub proposal_id: i64,
    pub draft_name: String,
    pub target_month: String,
    pub actions: Vec<SyncActionView>,
    pub unchanged: usize,
    pub cleanup: bool,
    pub cleanup_offers: Vec<CleanupOfferView>,
    /// Pass back to execute: refuses to run if the plan changed meanwhile.
    pub plan_key: String,
}

#[derive(Clone, Copy, PartialEq)]
enum Mode {
    Push { cleanup: bool },
    Remove,
}

fn token_of(token: &State<'_, SlingToken>) -> Result<String, String> {
    let t = token.0.lock().map_err(err)?;
    t.clone().ok_or_else(|| "no Sling token — paste one in Settings".to_string())
}

/// Load everything, fetch the month from Sling (outside the DB lock), plan.
fn prepare(
    db: &State<'_, Db>,
    token: &str,
    proposal_id: i64,
    mode: Mode,
) -> Result<(SyncPlan, StudioConfig, String), String> {
    let (specs, cfg, month, tracked) = {
        let conn = db.0.lock().map_err(err)?;
        let (specs, cfg, month) = match mode {
            // Enforces "only the push draft is pushed" + studio config.
            Mode::Push { .. } => crate::commands::build_specs_for_proposal(&conn, proposal_id)?,
            Mode::Remove => {
                let cfg = crate::commands::load_studio_config_checked(&conn)?;
                let month: String = conn
                    .query_row(
                        "SELECT target_month FROM proposals WHERE id = ?",
                        duckdb::params![proposal_id],
                        |r| r.get(0),
                    )
                    .map_err(|e| format!("draft {proposal_id} not found: {e}"))?;
                (Vec::new(), cfg, month)
            }
        };
        let tracked = load_tracked(&conn, Some(&month))?;
        (specs, cfg, month, tracked)
    };
    let events = crate::sling::fetch_calendar(token, &cfg, &month).map_err(err)?;
    let plan = match mode {
        Mode::Push { cleanup } => build_push_plan(&PlanInput {
            proposal_id,
            specs: &specs,
            tracked: &tracked,
            events: &events,
            home_location_id: cfg.home_location_id,
            cleanup,
        }),
        Mode::Remove => build_remove_plan(proposal_id, &tracked, &events, cfg.home_location_id),
    };
    Ok((plan, cfg, month))
}

fn view_of(conn: &duckdb::Connection, proposal_id: i64, month: &str, mode: Mode, plan: &SyncPlan) -> Result<SyncPreview, String> {
    let teachers: HashMap<i64, String> = conn
        .prepare("SELECT sling_user_id, display_name FROM teachers")
        .map_err(err)?
        .query_map([], |r| Ok((r.get::<_, i32>(0)? as i64, r.get::<_, String>(1)?)))
        .map_err(err)?
        .collect::<Result<_, _>>()
        .map_err(err)?;
    let classes: HashMap<i64, String> = conn
        .prepare("SELECT sling_position_id, class_name FROM positions")
        .map_err(err)?
        .query_map([], |r| Ok((r.get::<_, i32>(0)? as i64, r.get::<_, String>(1)?)))
        .map_err(err)?
        .collect::<Result<_, _>>()
        .map_err(err)?;
    let sv = |s: &Option<ShiftState>| {
        s.as_ref().map(|s| ShiftView {
            date: s.date.clone(),
            start: s.start.clone(),
            end: s.end.clone(),
            class_name: classes.get(&s.position_id).cloned().unwrap_or_else(|| format!("position {}", s.position_id)),
            teacher_name: teachers.get(&s.user_id).cloned().unwrap_or_else(|| format!("user {}", s.user_id)),
        })
    };
    let actions = plan
        .actions
        .iter()
        .map(|a| SyncActionView {
            kind: a.kind,
            proposal_shift_id: a.proposal_shift_id,
            sling_shift_id: a.sling_shift_id,
            before: sv(&a.before),
            after: sv(&a.after),
            reason: a.reason.clone(),
            from_draft: a.from_proposal_id.map(|p| crate::drafts::draft_name(conn, p)),
            skip_outcome: a.skip_outcome.clone(),
        })
        .collect();
    let cleanup_offers = plan
        .cleanup_offers
        .iter()
        .map(|o| CleanupOfferView {
            proposal_id: o.proposal_id,
            draft_name: crate::drafts::draft_name(conn, o.proposal_id),
            removable: o.removable,
            blocked: o.blocked,
        })
        .collect();
    Ok(SyncPreview {
        mode: if mode == Mode::Remove { "remove" } else { "push" }.to_string(),
        proposal_id,
        draft_name: crate::drafts::draft_name(conn, proposal_id),
        target_month: month.to_string(),
        actions,
        unchanged: plan.unchanged,
        cleanup: matches!(mode, Mode::Push { cleanup: true }),
        cleanup_offers,
        plan_key: plan.key(),
    })
}

fn preview(db: State<'_, Db>, token: State<'_, SlingToken>, proposal_id: i64, mode: Mode) -> Result<SyncPreview, String> {
    if let Mode::Push { .. } = mode {
        // Refuse a non-push draft before anything else (token, network).
        let conn = db.0.lock().map_err(err)?;
        crate::drafts::ensure_push_candidate(&conn, proposal_id)?;
    }
    let token_str = token_of(&token)?;
    let (plan, _cfg, month) = prepare(&db, &token_str, proposal_id, mode)?;
    let conn = db.0.lock().map_err(err)?;
    view_of(&conn, proposal_id, &month, mode, &plan)
}

/// Routine database backup before a sync writes anything to Sling (or to
/// the push audit tables). Non-fatal: a failure is logged by backup::run and
/// returned as a warning for the summary; it never blocks the sync. Skipped
/// when the plan has no network operations.
fn pre_sync_backup(
    conn: &mut duckdb::Connection,
    db_file: anyhow::Result<std::path::PathBuf>,
    plan: &SyncPlan,
    reason: &str,
    state: Option<&crate::backup::BackupState>,
) -> Option<String> {
    if plan.network_ops() == 0 {
        return None;
    }
    match db_file {
        Ok(path) => crate::backup::run(conn, &path, reason, state).err(),
        Err(e) => Some(format!("{reason} backup failed: {e:#}")),
    }
}

fn execute(
    app: tauri::AppHandle,
    db: State<'_, Db>,
    token: State<'_, SlingToken>,
    proposal_id: i64,
    mode: Mode,
    plan_key: &str,
) -> Result<SyncSummary, String> {
    use tauri::{Emitter, Manager};
    if let Mode::Push { .. } = mode {
        let conn = db.0.lock().map_err(err)?;
        crate::drafts::ensure_push_candidate(&conn, proposal_id)?;
    }
    let token_str = token_of(&token)?;
    // Recompute from a fresh fetch; run only what the user confirmed.
    let (plan, cfg, month) = prepare(&db, &token_str, proposal_id, mode)?;
    if plan.key() != plan_key {
        return Err("Sling or the draft changed since the preview — review the changes again before pushing.".to_string());
    }
    let (viewdates, cachedates) = crate::sling::view_cache_dates(&month).map_err(err)?;
    let counted_skips = plan.actions.iter().filter(|a| a.kind == ActionKind::Skip).count() as i64;

    let backup_warning = {
        let mut conn = db.0.lock().map_err(err)?;
        let reason = match mode {
            Mode::Push { .. } => "prepush",
            Mode::Remove => "preremove",
        };
        let backup_state = app.state::<crate::backup::BackupState>();
        pre_sync_backup(&mut conn, crate::db::db_path(&app), &plan, reason, Some(&backup_state))
    };

    let push_id: i64 = {
        let conn = db.0.lock().map_err(err)?;
        conn.query_row(
            "INSERT INTO pushes (proposal_id, shifts_attempted, shifts_skipped) VALUES (?, ?, ?) RETURNING id",
            duckdb::params![proposal_id, plan.network_ops() as i64, counted_skips],
            |r| r.get(0),
        )
        .map_err(err)?
    };

    let mut ops = LiveSling { token: &token_str, cfg: &cfg, viewdates, cachedates };
    let mut record = |row: &ResultRow| -> Result<(), String> {
        let conn = db.0.lock().map_err(err)?;
        record_result(&conn, push_id, row)
    };
    let mut progress = |p: &SyncProgress| {
        let _ = app.emit("push-progress", p.clone());
    };
    let result = execute_plan(&plan, &mut ops, &mut record, &mut progress);

    // Close the audit row whatever happened.
    {
        let conn = db.0.lock().map_err(err)?;
        let (ok, failed) = match &result {
            Ok(s) => (s.created + s.updated + s.deleted + s.adopted, s.failed),
            Err(_) => (0, 0),
        };
        conn.execute(
            "UPDATE pushes SET finished_at = now(), shifts_succeeded = ?, shifts_failed = ? WHERE id = ?",
            duckdb::params![ok, failed, push_id],
        )
        .map_err(err)?;
        let _ = conn.execute("CHECKPOINT", []);
    }
    let mut summary = result?;
    summary.push_id = push_id;
    summary.backup_warning = backup_warning;
    if summary.aborted {
        return Err(format!(
            "sling-401: token expired partway through ({} created, {} updated, {} deleted) — log in again and push to finish",
            summary.created, summary.updated, summary.deleted
        ));
    }
    Ok(summary)
}

/// Preview an incremental push of the month's push draft. `cleanup` also
/// removes other drafts' removable planning shifts (default off in the UI).
#[tauri::command(async)]
pub fn push_sync_preview(
    db: State<'_, Db>,
    token: State<'_, SlingToken>,
    proposal_id: i64,
    cleanup: bool,
) -> Result<SyncPreview, String> {
    preview(db, token, proposal_id, Mode::Push { cleanup })
}

#[tauri::command(async)]
pub fn push_sync_execute(
    app: tauri::AppHandle,
    db: State<'_, Db>,
    token: State<'_, SlingToken>,
    proposal_id: i64,
    cleanup: bool,
    plan_key: String,
) -> Result<SyncSummary, String> {
    execute(app, db, token, proposal_id, Mode::Push { cleanup }, &plan_key)
}

/// Preview removing a draft's (planning, unmodified) shifts from Sling.
#[tauri::command(async)]
pub fn remove_draft_from_sling_preview(
    db: State<'_, Db>,
    token: State<'_, SlingToken>,
    proposal_id: i64,
) -> Result<SyncPreview, String> {
    preview(db, token, proposal_id, Mode::Remove)
}

#[tauri::command(async)]
pub fn remove_draft_from_sling_execute(
    app: tauri::AppHandle,
    db: State<'_, Db>,
    token: State<'_, SlingToken>,
    proposal_id: i64,
    plan_key: String,
) -> Result<SyncSummary, String> {
    execute(app, db, token, proposal_id, Mode::Remove, &plan_key)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sling::{SlingEventLocationRef, SlingEventPositionRef, SlingEventUserRef};

    const HOME: i64 = 5;
    const A: i64 = 1001;
    const B: i64 = 1002;
    const C: i64 = 1003;
    const CLASSIC: i64 = 29303965;
    const EMPOWER: i64 = 29470407;

    fn st(date: &str, start: &str, end: &str, user: i64, pos: i64) -> ShiftState {
        ShiftState { date: date.into(), start: start.into(), end: end.into(), user_id: user, position_id: pos }
    }
    fn spec(ps: i64, s: &ShiftState) -> PushSpec {
        PushSpec {
            proposal_shift_id: ps,
            date: s.date.clone(),
            start: s.start.clone(),
            end: s.end.clone(),
            position_id: s.position_id,
            user_id: s.user_id,
        }
    }
    fn ev(id: i64, s: &ShiftState, status: &str) -> CalendarEvent {
        CalendarEvent {
            id: Some(id),
            kind: "shift".into(),
            dtstart: format!("{}T{}:00-05:00", s.date, s.start),
            dtend: format!("{}T{}:00-05:00", s.date, s.end),
            user: Some(SlingEventUserRef { id: s.user_id }),
            users: None,
            position: Some(SlingEventPositionRef { id: s.position_id }),
            location: Some(SlingEventLocationRef { id: HOME }),
            status: Some(status.into()),
        }
    }
    fn tr(sid: i64, proposal: i64, ps: i64, snap: Option<&ShiftState>) -> TrackedShift {
        TrackedShift { sling_shift_id: sid, proposal_id: proposal, proposal_shift_id: ps, snapshot: snap.cloned() }
    }
    fn plan(p: i64, specs: &[PushSpec], tracked: &[TrackedShift], events: &[CalendarEvent], cleanup: bool) -> SyncPlan {
        build_push_plan(&PlanInput {
            proposal_id: p,
            specs,
            tracked,
            events,
            home_location_id: HOME,
            cleanup,
        })
    }
    fn kinds(p: &SyncPlan) -> Vec<(ActionKind, i64, Option<i64>)> {
        p.actions.iter().map(|a| (a.kind, a.proposal_shift_id, a.sling_shift_id)).collect()
    }

    #[test]
    fn plan_creates_updates_deletes_and_skips_unchanged() {
        let same = st("2026-11-02", "09:00", "10:00", A, CLASSIC);
        let was = st("2026-11-03", "09:00", "10:00", A, CLASSIC);
        let now = st("2026-11-03", "09:00", "10:00", B, CLASSIC); // teacher change
        let new = st("2026-11-04", "17:30", "18:15", C, EMPOWER);
        let dropped = st("2026-11-05", "05:45", "06:45", A, CLASSIC);
        let specs = [spec(10, &same), spec(11, &now), spec(12, &new)];
        let tracked = [tr(110, 1, 10, Some(&same)), tr(111, 1, 11, Some(&was)), tr(113, 1, 13, Some(&dropped))];
        let events = [ev(110, &same, "planning"), ev(111, &was, "planning"), ev(113, &dropped, "planning")];
        let p = plan(1, &specs, &tracked, &events, false);
        assert_eq!(p.unchanged, 1);
        assert_eq!(
            kinds(&p),
            vec![
                (ActionKind::Delete, 13, Some(113)),
                (ActionKind::Update, 11, Some(111)),
                (ActionKind::Create, 12, None),
            ]
        );
        let up = &p.actions[1];
        assert_eq!(up.before.as_ref().unwrap().user_id, A);
        assert_eq!(up.after.as_ref().unwrap().user_id, B);
        assert_eq!(p.network_ops(), 3);
        assert!(p.cleanup_offers.is_empty());
    }

    #[test]
    fn same_teacher_time_change_is_an_update_not_delete_plus_create() {
        let was = st("2026-11-03", "09:00", "10:00", A, CLASSIC);
        let now = st("2026-11-03", "09:30", "10:30", A, CLASSIC);
        let p = plan(1, &[spec(11, &now)], &[tr(111, 1, 11, Some(&was))], &[ev(111, &was, "planning")], false);
        assert_eq!(kinds(&p), vec![(ActionKind::Update, 11, Some(111))]);
    }

    #[test]
    fn coteach_slot_is_two_shifts_matched_independently() {
        let a = st("2026-11-07", "08:00", "09:00", A, CLASSIC);
        let b = st("2026-11-07", "08:00", "09:00", B, CLASSIC);
        let c = st("2026-11-07", "08:00", "09:00", C, CLASSIC);
        let tracked = [tr(201, 1, 20, Some(&a)), tr(202, 1, 20, Some(&b))];
        let events = [ev(201, &a, "planning"), ev(202, &b, "planning")];

        // Unchanged co-teach: both shifts match.
        let p = plan(1, &[spec(20, &a), spec(20, &b)], &tracked, &events, false);
        assert_eq!((p.unchanged, p.actions.len()), (2, 0));

        // Partner B → C: A untouched, B's shift replaced.
        let p = plan(1, &[spec(20, &a), spec(20, &c)], &tracked, &events, false);
        assert_eq!(p.unchanged, 1);
        assert_eq!(kinds(&p), vec![(ActionKind::Update, 20, Some(202))]);
        assert_eq!(p.actions[0].after.as_ref().unwrap().user_id, C);

        // Co-teach → solo A: B's shift deleted.
        let p = plan(1, &[spec(20, &a)], &tracked, &events, false);
        assert_eq!(kinds(&p), vec![(ActionKind::Delete, 20, Some(202))]);

        // Solo → co-teach: the added teacher is a create.
        let p = plan(1, &[spec(20, &a), spec(20, &b)], &tracked[..1], &events[..1], false);
        assert_eq!(kinds(&p), vec![(ActionKind::Create, 20, None)]);
    }

    #[test]
    fn unsafe_shifts_are_skipped_with_reasons() {
        let was = st("2026-11-03", "09:00", "10:00", A, CLASSIC);
        let now = st("2026-11-03", "09:00", "10:00", B, CLASSIC);
        let edited_in_sling = st("2026-11-03", "09:00", "10:00", C, CLASSIC);

        // Published: never changed.
        let p = plan(1, &[spec(11, &now)], &[tr(111, 1, 11, Some(&was))], &[ev(111, &was, "published")], false);
        assert_eq!(kinds(&p), vec![(ActionKind::Skip, 11, Some(111))]);
        assert_eq!(p.actions[0].skip_outcome.as_deref(), Some("skipped_conflict"));
        assert!(p.actions[0].reason.contains("published"), "{}", p.actions[0].reason);

        // Edited in Sling (teacher differs from what we pushed).
        let p = plan(1, &[spec(11, &now)], &[tr(111, 1, 11, Some(&was))], &[ev(111, &edited_in_sling, "planning")], false);
        assert_eq!(kinds(&p), vec![(ActionKind::Skip, 11, Some(111))]);
        assert!(p.actions[0].reason.contains("edited in Sling"), "{}", p.actions[0].reason);

        // Deleted in Sling: skipped, recorded as skipped_missing (untracks).
        let p = plan(1, &[spec(11, &now)], &[tr(111, 1, 11, Some(&was))], &[], false);
        assert_eq!(kinds(&p), vec![(ActionKind::Skip, 11, Some(111))]);
        assert_eq!(p.actions[0].skip_outcome.as_deref(), Some("skipped_missing"));

        // Pre-0013 push (no snapshot): can't verify → never changed...
        let p = plan(1, &[spec(11, &now)], &[tr(111, 1, 11, None)], &[ev(111, &was, "planning")], false);
        assert_eq!(kinds(&p), vec![(ActionKind::Skip, 11, Some(111))]);
        assert_eq!(p.actions[0].reason, LEGACY_REASON);
        // ...and a deleted-in-Sling shift the draft still wants says so.
        let p = plan(1, &[spec(11, &was)], &[tr(111, 1, 11, Some(&was))], &[], false);
        assert_eq!(p.actions[0].reason, MISSING_WANTED);

        // Unsafe deletes are skipped too, and none of these call Sling.
        let p = plan(1, &[], &[tr(111, 1, 11, Some(&was))], &[ev(111, &was, "published")], false);
        assert_eq!(kinds(&p), vec![(ActionKind::Skip, 11, Some(111))]);
        assert_eq!(p.network_ops(), 0);
    }

    #[test]
    fn legacy_shift_matching_the_draft_gets_a_baseline_on_execute_only() {
        let x = st("2026-11-02", "09:00", "10:00", A, CLASSIC);
        let y = st("2026-11-03", "09:00", "10:00", B, CLASSIC);
        let y_draft = st("2026-11-03", "09:00", "10:00", C, CLASSIC);
        let tracked = [tr(700, 1, 10, None), tr(701, 1, 11, None)];
        let events = [ev(700, &x, "planning"), ev(701, &y, "planning")];
        let p = plan(1, &[spec(10, &x), spec(11, &y_draft)], &tracked, &events, false);
        // Match → baseline (DB-only); mismatch → untouchable with the legacy reason.
        assert_eq!(kinds(&p), vec![(ActionKind::Baseline, 10, Some(700)), (ActionKind::Skip, 11, Some(701))]);
        assert_eq!(p.actions[1].reason, LEGACY_REASON);
        assert_eq!(p.network_ops(), 0);

        // Planning is pure: nothing recorded until execute, which writes an
        // 'adopted' row carrying Sling's state as the snapshot.
        let mut ops = FakeOps::default();
        let (sum, rows) = run(&p, &mut ops);
        assert!(ops.calls.is_empty());
        assert_eq!(sum.adopted, 1);
        assert_eq!(rows[0].outcome, "adopted");
        assert_eq!(rows[0].sling_shift_id, Some(700));
        assert_eq!(rows[0].snapshot.as_ref(), Some(&x));
        assert_eq!(rows[1].outcome, "skipped_conflict");

        // Once baselined, a later draft edit is a normal update.
        let now = st("2026-11-02", "09:00", "10:00", B, CLASSIC);
        let p = plan(1, &[spec(10, &now)], &[tr(700, 1, 10, Some(&x))], &events[..1], false);
        assert_eq!(kinds(&p), vec![(ActionKind::Update, 10, Some(700))]);

        // A matching legacy shift that's published is just unchanged (no baseline).
        let p = plan(1, &[spec(10, &x)], &tracked[..1], &[ev(700, &x, "published")], false);
        assert_eq!((p.unchanged, p.actions.len()), (1, 0));
    }

    #[test]
    fn baseline_snapshot_persists_through_tracking() {
        let conn = crate::db::open_in_memory().unwrap();
        crate::migrations::run(&conn).unwrap();
        conn.execute_batch(
            "INSERT INTO positions (sling_position_id, class_name) VALUES (29303965, 'Classic');
             INSERT INTO proposals (id, target_month, algorithm_version, parameters) VALUES (1, '2026-11', 'v9', '{}');
             INSERT INTO proposal_shifts (id, proposal_id, shift_date, start_time, end_time,
                 sling_position_id, generation_reason) VALUES
               (10, 1, DATE '2026-11-02', '09:00', '10:00', 29303965, 'r');
             INSERT INTO pushes (id, proposal_id) VALUES (1, 1), (2, 1);
             INSERT INTO push_results (push_id, proposal_shift_id, outcome, sling_shift_id)
               VALUES (1, 10, 'created', '700');",
        )
        .unwrap();
        let x = st("2026-11-02", "09:00", "10:00", A, CLASSIC);
        let before = load_tracked(&conn, Some("2026-11")).unwrap();
        assert!(before[0].snapshot.is_none());
        let p = plan(1, &[spec(10, &x)], &before, &[ev(700, &x, "planning")], false);
        let mut ops = FakeOps::default();
        let mut rec = |r: &ResultRow| record_result(&conn, 2, r);
        execute_plan(&p, &mut ops, &mut rec, &mut |_| {}).unwrap();
        let after = load_tracked(&conn, Some("2026-11")).unwrap();
        assert_eq!((after.len(), after[0].proposal_id), (1, 1));
        assert_eq!(after[0].snapshot.as_ref(), Some(&x));
    }

    #[test]
    fn never_touches_shifts_the_app_did_not_create() {
        let mine = st("2026-11-02", "09:00", "10:00", A, CLASSIC);
        let theirs = st("2026-11-02", "17:30", "18:30", B, CLASSIC);
        // An untracked planning shift in Sling, and the draft doesn't have it.
        let p = plan(1, &[spec(10, &mine)], &[], &[ev(999, &theirs, "planning")], false);
        assert_eq!(kinds(&p), vec![(ActionKind::Create, 10, None)]);
        assert!(p.actions.iter().all(|a| a.sling_shift_id != Some(999)));
        // The draft wants exactly the untracked shift: not duplicated, not adopted, not recorded.
        let p = plan(1, &[spec(10, &theirs)], &[], &[ev(999, &theirs, "planning")], false);
        assert_eq!(kinds(&p), vec![(ActionKind::Skip, 10, None)]);
        assert_eq!(p.actions[0].skip_outcome, None);
        // Remove plan only ever lists tracked shifts.
        let r = build_remove_plan(1, &[], &[ev(999, &theirs, "planning")], HOME);
        assert!(r.actions.is_empty());
    }

    #[test]
    fn slot_replaced_in_place_is_not_a_duplicate() {
        // ps30 was pushed (sid 301); the draft dropped it and gained ps31 at
        // the same date/time/teacher/class with a new end time.
        let old = st("2026-11-02", "09:00", "10:00", A, CLASSIC);
        let new = st("2026-11-02", "09:00", "09:50", A, CLASSIC);
        let p = plan(1, &[spec(31, &new)], &[tr(301, 1, 30, Some(&old))], &[ev(301, &old, "planning")], false);
        assert_eq!(kinds(&p), vec![(ActionKind::Delete, 30, Some(301)), (ActionKind::Create, 31, None)]);
        // But if 301 can't be deleted (published), the create IS deduped.
        let p = plan(1, &[spec(31, &new)], &[tr(301, 1, 30, Some(&old))], &[ev(301, &old, "published")], false);
        assert_eq!(kinds(&p), vec![(ActionKind::Skip, 30, Some(301)), (ActionKind::Skip, 31, None)]);
    }

    #[test]
    fn switching_push_draft_adopts_identical_and_offers_cleanup() {
        let x = st("2026-11-02", "09:00", "10:00", A, CLASSIC);
        let y = st("2026-11-03", "09:00", "10:00", B, CLASSIC);
        let z = st("2026-11-04", "09:00", "10:00", C, CLASSIC);
        let pub_shift = st("2026-11-05", "09:00", "10:00", C, EMPOWER);
        // Draft 2 ("A") was pushed; draft 1 ("B") is now the push draft.
        let tracked = [tr(501, 2, 50, Some(&x)), tr(502, 2, 51, Some(&y)), tr(503, 2, 52, Some(&pub_shift))];
        let events = [ev(501, &x, "planning"), ev(502, &y, "planning"), ev(503, &pub_shift, "published")];
        let specs = [spec(60, &x), spec(61, &z)];

        let p = plan(1, &specs, &tracked, &events, false);
        assert_eq!(kinds(&p), vec![(ActionKind::Adopt, 60, Some(501)), (ActionKind::Create, 61, None)]);
        assert_eq!(p.actions[0].from_proposal_id, Some(2));
        assert_eq!(p.cleanup_offers, vec![CleanupOffer { proposal_id: 2, removable: 1, blocked: 1 }]);

        // Opted in: cleanup deletes run before this draft's creates; the
        // published shift is left alone.
        let p = plan(1, &specs, &tracked, &events, true);
        assert_eq!(
            kinds(&p),
            vec![
                (ActionKind::Adopt, 60, Some(501)),
                (ActionKind::Skip, 52, Some(503)),
                (ActionKind::Cleanup, 51, Some(502)),
                (ActionKind::Create, 61, None),
            ]
        );
        assert_ne!(plan(1, &specs, &tracked, &events, false).key(), p.key());

        // A near-identical other-draft shift (same fingerprint, other end
        // time): deduped while it stays, created once cleanup removes it.
        let y2 = st("2026-11-03", "09:00", "09:50", B, CLASSIC);
        let p = plan(1, &[spec(62, &y2)], &tracked[1..2], &events[1..2], false);
        assert_eq!(kinds(&p), vec![(ActionKind::Skip, 62, None)]);
        let p = plan(1, &[spec(62, &y2)], &tracked[1..2], &events[1..2], true);
        assert_eq!(kinds(&p), vec![(ActionKind::Cleanup, 51, Some(502)), (ActionKind::Create, 62, None)]);
    }

    #[test]
    fn remove_plan_applies_the_same_safety() {
        let x = st("2026-11-02", "09:00", "10:00", A, CLASSIC);
        let y = st("2026-11-03", "09:00", "10:00", B, CLASSIC);
        let y_edited = st("2026-11-03", "10:00", "11:00", B, CLASSIC);
        let tracked = [tr(501, 2, 50, Some(&x)), tr(502, 2, 51, Some(&y)), tr(601, 3, 70, Some(&x))];
        let r = build_remove_plan(2, &tracked, &[ev(501, &x, "planning"), ev(502, &y_edited, "planning")], HOME);
        assert_eq!(kinds(&r), vec![(ActionKind::Skip, 51, Some(502)), (ActionKind::Delete, 50, Some(501))]);
    }

    // ---------------- executor ----------------

    #[derive(Default)]
    struct FakeOps {
        calls: Vec<String>,
        pauses: Vec<u64>,
        next_id: i64,
        fail_create_401_at: Option<usize>,
        missing: HashSet<i64>,
    }
    impl SlingShiftOps for FakeOps {
        fn create(&mut self, st: &ShiftState) -> anyhow::Result<i64> {
            self.calls.push(format!("POST {} {}", st.date, st.user_id));
            if Some(self.calls.len()) == self.fail_create_401_at {
                return Err(anyhow::anyhow!("sling-401"));
            }
            self.next_id += 1;
            Ok(9000 + self.next_id)
        }
        fn delete(&mut self, id: i64) -> anyhow::Result<DeleteOutcome> {
            self.calls.push(format!("DELETE {id}"));
            Ok(if self.missing.contains(&id) { DeleteOutcome::NotFound } else { DeleteOutcome::Deleted })
        }
        fn pause(&mut self, secs: u64) {
            self.pauses.push(secs);
        }
    }

    fn run(plan: &SyncPlan, ops: &mut FakeOps) -> (SyncSummary, Vec<ResultRow>) {
        let mut rows = Vec::new();
        let sum = execute_plan(
            plan,
            ops,
            &mut |r| {
                rows.push(r.clone());
                Ok(())
            },
            &mut |_| {},
        )
        .unwrap();
        (sum, rows)
    }

    #[test]
    fn execute_runs_deletes_then_replacements_then_creates_and_records_everything() {
        let x = st("2026-11-02", "09:00", "10:00", A, CLASSIC);
        let y = st("2026-11-03", "09:00", "10:00", B, CLASSIC);
        let y2 = st("2026-11-03", "09:00", "10:00", C, CLASSIC);
        let z = st("2026-11-04", "09:00", "10:00", C, CLASSIC);
        let gone = st("2026-11-05", "09:00", "10:00", A, CLASSIC);
        let tracked = [
            tr(501, 2, 50, Some(&x)),    // other draft, identical → adopt
            tr(111, 1, 11, Some(&y)),    // teacher change → update
            tr(113, 1, 13, Some(&gone)), // dropped → delete (404 at run time)
        ];
        let events = [ev(501, &x, "planning"), ev(111, &y, "planning"), ev(113, &gone, "planning")];
        let p = plan(1, &[spec(60, &x), spec(11, &y2), spec(12, &z)], &tracked, &events, false);
        let mut ops = FakeOps { missing: [113].into_iter().collect(), ..Default::default() };
        let (sum, rows) = run(&p, &mut ops);
        assert_eq!(ops.calls, vec!["DELETE 113", "DELETE 111", "POST 2026-11-03 1003", "POST 2026-11-04 1003"]);
        assert_eq!(ops.pauses, vec![1, 1, 1]);
        let outcomes: Vec<(&str, Option<i64>)> = rows.iter().map(|r| (r.outcome.as_str(), r.sling_shift_id)).collect();
        assert_eq!(
            outcomes,
            vec![
                ("adopted", Some(501)),
                ("skipped_missing", Some(113)),
                ("deleted", Some(111)),
                ("updated", Some(9001)),
                ("created", Some(9002)),
            ]
        );
        // Snapshots record what we sent (an adopted shift is the draft's state).
        assert_eq!(rows[0].snapshot.as_ref(), Some(&x));
        assert_eq!(rows[3].snapshot.as_ref(), Some(&y2));
        assert_eq!(rows[4].snapshot.as_ref(), Some(&z));
        assert!(rows[2].snapshot.is_none());
        assert_eq!(
            (sum.created, sum.updated, sum.deleted, sum.adopted, sum.skipped, sum.failed, sum.aborted),
            (1, 1, 0, 1, 1, 0, false)
        );
    }

    #[test]
    fn execute_batches_ten_calls_then_pauses_longer_and_stops_on_401() {
        let specs: Vec<PushSpec> = (1..=12)
            .map(|d| spec(d, &st(&format!("2026-11-{d:02}"), "09:00", "10:00", A, CLASSIC)))
            .collect();
        let p = plan(1, &specs, &[], &[], false);
        let mut ops = FakeOps::default();
        let (sum, _) = run(&p, &mut ops);
        assert_eq!(sum.created, 12);
        assert_eq!(ops.pauses, vec![1, 1, 1, 1, 1, 1, 1, 1, 1, 10, 1]);

        let mut ops = FakeOps { fail_create_401_at: Some(3), ..Default::default() };
        let (sum, rows) = run(&p, &mut ops);
        assert!(sum.aborted);
        assert_eq!((sum.created, sum.failed), (2, 1));
        assert_eq!(ops.calls.len(), 3);
        assert_eq!(rows.last().unwrap().outcome, "failed");
    }

    // ---------------- DB tracking ----------------

    #[test]
    fn tracking_follows_the_latest_row_per_sling_shift() {
        let conn = crate::db::open_in_memory().unwrap();
        crate::migrations::run(&conn).unwrap();
        conn.execute_batch(
            "INSERT INTO positions (sling_position_id, class_name) VALUES (29303965, 'Classic');
             INSERT INTO proposals (id, target_month, algorithm_version, parameters) VALUES
               (1, '2026-11', 'v9', '{}'), (2, '2026-11', 'v9', '{}'), (3, '2026-12', 'v9', '{}');
             INSERT INTO proposal_shifts (id, proposal_id, shift_date, start_time, end_time,
                 sling_position_id, generation_reason) VALUES
               (10, 1, DATE '2026-11-02', '09:00', '10:00', 29303965, 'r'),
               (20, 2, DATE '2026-11-02', '09:00', '10:00', 29303965, 'r'),
               (30, 3, DATE '2026-12-02', '09:00', '10:00', 29303965, 'r');
             INSERT INTO pushes (id, proposal_id) VALUES (1, 1), (2, 2), (3, 3);
             -- a legacy (pre-0013) create has no snapshot
             INSERT INTO push_results (push_id, proposal_shift_id, outcome, sling_shift_id)
               VALUES (1, 10, 'created', '700'), (1, 10, 'failed', NULL);",
        )
        .unwrap();
        let x = st("2026-11-02", "09:00", "10:00", A, CLASSIC);
        let rec = |push: i64, ps: i64, outcome: &str, sid: i64, snap: Option<&ShiftState>| {
            record_result(
                &conn,
                push,
                &ResultRow {
                    proposal_shift_id: ps,
                    outcome: outcome.into(),
                    sling_shift_id: Some(sid),
                    error: None,
                    snapshot: snap.cloned(),
                },
            )
            .unwrap();
        };
        rec(1, 10, "created", 701, Some(&x));
        rec(1, 10, "created", 702, Some(&x));
        rec(3, 30, "created", 900, Some(&x));

        let ids = |t: &[TrackedShift]| t.iter().map(|t| (t.sling_shift_id, t.proposal_id)).collect::<Vec<_>>();
        let t = load_tracked(&conn, Some("2026-11")).unwrap();
        assert_eq!(ids(&t), vec![(700, 1), (701, 1), (702, 1)]);
        assert!(t[0].snapshot.is_none());
        assert_eq!(t[1].snapshot.as_ref(), Some(&x));

        // deleted / skipped_missing untrack; skipped_conflict doesn't;
        // adopted moves ownership to the other draft.
        rec(2, 10, "deleted", 700, None);
        rec(2, 10, "skipped_conflict", 701, None);
        rec(2, 20, "adopted", 702, Some(&x));
        assert_eq!(ids(&load_tracked(&conn, Some("2026-11")).unwrap()), vec![(701, 1), (702, 2)]);
        rec(2, 10, "skipped_missing", 701, None);
        assert_eq!(ids(&load_tracked(&conn, Some("2026-11")).unwrap()), vec![(702, 2)]);

        let counts = live_counts(&conn).unwrap();
        assert_eq!((counts.get(&1), counts.get(&2), counts.get(&3)), (None, Some(&1), Some(&1)));
    }

    #[test]
    fn pre_sync_backup_runs_only_when_the_plan_writes_and_never_fails_the_sync() {
        let dir = std::env::temp_dir().join(format!(
            "barrekeep-presync-backup-{}-{}",
            std::process::id(),
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0)
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let db_file = dir.join("scheduler.duckdb");
        let mut conn = crate::db::open_file(&db_file).unwrap();
        crate::migrations::run(&conn).unwrap();
        let bdir = crate::backup::backups_dir(&db_file);
        let n_backups = || std::fs::read_dir(&bdir).map(|d| d.count()).unwrap_or(0);

        // Nothing to send: no backup.
        let same = st("2026-11-02", "09:00", "10:00", A, CLASSIC);
        let noop = plan(1, &[spec(10, &same)], &[tr(110, 1, 10, Some(&same))], &[ev(110, &same, "planning")], false);
        assert_eq!(noop.network_ops(), 0);
        assert_eq!(pre_sync_backup(&mut conn, Ok(db_file.clone()), &noop, "prepush", None), None);
        assert_eq!(n_backups(), 0);

        // A create: one backup, tagged with the reason.
        let new = st("2026-11-04", "17:30", "18:15", C, EMPOWER);
        let writes = plan(1, &[spec(12, &new)], &[], &[], false);
        assert!(writes.network_ops() > 0);
        let state = crate::backup::BackupState::default();
        assert_eq!(pre_sync_backup(&mut conn, Ok(db_file.clone()), &writes, "prepush", Some(&state)), None);
        let names: Vec<String> = std::fs::read_dir(&bdir)
            .unwrap()
            .map(|e| e.unwrap().file_name().into_string().unwrap())
            .collect();
        assert_eq!(names.len(), 1);
        assert!(names[0].ends_with("-prepush.duckdb"), "{names:?}");

        // No resolvable DB path: a warning, not an error.
        let w = pre_sync_backup(&mut conn, Err(anyhow::anyhow!("no app dir")), &writes, "preremove", None);
        assert!(w.as_deref().is_some_and(|m| m.contains("preremove backup failed")), "{w:?}");

        drop(conn);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
