"""
Schedule proposer.

Builds a draft monthly class schedule from a studio's recent Sling history:
the candidate POOL comes from Sling position groups, recent history drives
per-slot RANKING, and a variety penalty spreads work across teachers. Reads
its input as a JSON payload on stdin (see commands.rs::generate_proposal) or,
for standalone dev runs, from local fixture files.

Month-specific overrides (hard assignments, time shifts, per-teacher rules)
ship empty in this generic build; populate them per deployment. The lead
teacher is the last-resort overflow assignee. The 7am slot format-flexes to
Classic if its primary class can't be filled.
"""
import argparse
import json
import sys
from collections import defaultdict, Counter
from datetime import datetime, timezone, timedelta
import csv, os

parser = argparse.ArgumentParser()
parser.add_argument('--json-out', action='store_true',
                    help='emit structured JSON to stdout (prints redirect to stderr)')
parser.add_argument('--from-stdin', action='store_true',
                    help='read input payload from stdin JSON instead of fixture files')
parser.add_argument('--target-month', default='2026-06',
                    help='target month YYYY-MM (default 2026-06 for dev/regression)')
args = parser.parse_args()

JSON_OUT = args.json_out
TARGET_MONTH = args.target_month

if JSON_OUT:
    _stdout = sys.stdout
    sys.stdout = sys.stderr

if args.from_stdin:
    _payload = json.load(sys.stdin)
    # Translate stdin payload into the shapes the algorithm already expects.
    # Keys defined in src-tauri/src/commands.rs::generate_proposal (Task 11).
    may_data = {'april_events': _payload.get('history_events') or []}
    june_data = {'june_events': _payload.get('month_events') or []}
    discovery = {'users': {'users': _payload.get('users') or []}}
else:
    _payload = {}
    with open('data/fixtures/april_may_extract.json') as f:
        may_data = json.load(f)
    with open('data/fixtures/june_extract.json') as f:
        june_data = json.load(f)
    with open('data/fixtures/sling_discovery.json') as f:
        discovery = json.load(f)

TEACHERS = {
    1001: "Teacher A", 1002: "Teacher B", 1003: "Teacher C", 1004: "Teacher D",
    1005: "Teacher E", 1006: "Teacher F", 1007: "Teacher G", 1008: "Teacher H",
    1009: "Teacher I", 1010: "Teacher J",
}
NAME_TO_UID = {v: k for k, v in TEACHERS.items()}
# Per-teacher operational hooks are disabled in the generic build (None never
# matches a real uid, so the guards that reference these are inert).
PRIORITY_UID = None
EXCLUDE_UID = None

TARGETS = {
    1001: (8, 8), 1002: (6, 6), 1003: (5, 5), 1004: (4, 4), 1005: (3, 4),
    1006: (2, 4), 1007: (2, 2), 1008: (1, 2), 1009: (3, 3), 1010: (3, 3),
}
RANKING_WEIGHTS = {}

# Override hardcoded roster/targets from stdin payload (when --from-stdin).
if args.from_stdin and _payload.get('teachers'):
    TEACHERS = {t['sling_user_id']: t['display_name'] for t in _payload['teachers']}
    TARGETS = {t['sling_user_id']: (t['weekly_target'], t['weekly_max'])
               for t in _payload['teachers']}
    RANKING_WEIGHTS = {t['sling_user_id']: t['ranking_weight']
                       for t in _payload['teachers']}
    NAME_TO_UID = {v: k for k, v in TEACHERS.items()}

# Lead teacher: the last-resort overflow assignee. From the payload's is_lead
# flag; falls back to the first teacher in the roster.
LEAD_UID = next((t['sling_user_id'] for t in (_payload.get('teachers') or [])
                 if t.get('is_lead')), next(iter(TEACHERS), 1001))

# Home studio location id, from the runtime studio config via the stdin payload
# (see migration 0007). 0 = unset (standalone runs without a payload).
HOME_LOCATION_ID = _payload.get('home_location_id') or 0
POSITION_NAMES = {29470407: "Empower", 29470419: "Focus", 29470489: "Breaking Down the Barre",
                  29303958: "Align", 29303965: "Classic", 29304030: "Define",
                  29304197: "Reform", 29303535: "Teacher", 29303536: "Sales Rep"}
NAME_TO_PID = {v: k for k, v in POSITION_NAMES.items()}
TZ = timezone(timedelta(hours=-5))
WD = ['Mon', 'Tue', 'Wed', 'Thu', 'Fri', 'Sat', 'Sun']

# ============================================================
# TEACHER QUALIFICATIONS from Sling positions (source of truth)
# ============================================================
CLASS_POSITION_IDS = {
    29470407: 'Empower', 29470419: 'Focus', 29470489: 'Breaking Down the Barre',
    29303958: 'Align', 29303965: 'Classic', 29304030: 'Define', 29304197: 'Reform',
}
TEACHER_QUALIFICATIONS = {}  # uid -> set of class names
for user in discovery['users']['users']:
    if user['id'] not in TEACHERS: continue
    gids = set(user.get('groupIds', []))
    quals = {CLASS_POSITION_IDS[g] for g in gids if g in CLASS_POSITION_IDS}
    TEACHER_QUALIFICATIONS[user['id']] = quals

print("Teacher qualifications from Sling:")
for uid in sorted(TEACHERS, key=lambda x: TEACHERS[x]):
    quals = sorted(TEACHER_QUALIFICATIONS.get(uid, set()))
    print(f"  {TEACHERS[uid]:25s}  {', '.join(quals)}")

# ============================================================
# Month-specific overrides
# ============================================================
# Month-specific overrides ship empty in the generic build. Populate per
# deployment to add hard assignments, weekend/Sunday time shifts, slot
# removals, 7am biweekly rotations, per-slot class overrides, or hard-placed
# Focus classes for a specific target month. Names are kept so the consuming
# code paths below remain valid no-ops when empty.
SAT_TIME_SHIFTS = {}
SUN_TIME_SHIFTS = {}
JUNE_SLOTS_TO_REMOVE = set()
JUNE_7AM_BIWEEKLY = {}
JUNE_SLOT_CLASS_OVERRIDES = {}
HARD_ASSIGNMENTS = {}
FOCUS_HARD_PLACEMENTS = []
FOCUS_PLACEMENTS_JUNE = []

# Optional priority seeding: (weekday, time) slots where a specific teacher
# should be weighted up. Empty in the generic build.
PRIORITY_SLOTS = set()

# Soft blocks (Sling says cleared, but manager hasn't approved)
TEACHER_CLASS_BLOCKLIST = {}

# Slot-time blocklist: teacher cannot teach at these specific (weekday, time) slots
# regardless of class type or stated availability
TEACHER_SLOT_BLOCKLIST = {}

# Specific date+time blocks (another-location conflicts not in Sling)
# Format: uid -> set of (date_str, start_time) tuples
TEACHER_DATE_BLOCKLIST = {}

# Per-teacher variety penalty multiplier (defaults to 1.0)
# Higher = more reluctant to assign more classes to this teacher
VARIETY_PENALTY_MULTIPLIER = {}

SEVEN_AM_LAST_RESORT = True

# Variety: when multiple qualified teachers are available, prefer those
# with the lighter monthly load to date. This is added as a penalty to
# the ranking score: penalty = current_assignments * VARIETY_PENALTY_PER_CLASS
VARIETY_PENALTY_PER_CLASS = 0.3  # tuneable; higher = more rotation

# ============================================================
# Helpers
# ============================================================
def uid_of(e):
    u = e.get('user'); return u.get('id') if isinstance(u, dict) else None
def loc_of(e):
    l = e.get('location'); return l.get('id') if isinstance(l, dict) else None
def pos_of(e):
    p = e.get('position'); return p.get('id') if isinstance(p, dict) else None
def is_ours(e):
    if uid_of(e) not in TEACHERS: return False
    lid = loc_of(e); return lid is None or lid == HOME_LOCATION_ID
def parse_dt(s):
    if not s: return None
    return datetime.fromisoformat(s.replace('Z', '+00:00')).astimezone(TZ)
def add_min(hhmm, mins):
    h, m = map(int, hhmm.split(':'))
    total = h * 60 + m + mins
    return f"{(total // 60) % 24:02d}:{total % 60:02d}"
def teacher_blocked_from_class(uid, cls):
    return cls in TEACHER_CLASS_BLOCKLIST.get(uid, set())

def teacher_qualified(uid, cls):
    """Source of truth: Sling positions, minus manager blocklist."""
    if cls in ('Sales Rep', 'Teacher'): return False
    if cls in TEACHER_CLASS_BLOCKLIST.get(uid, set()): return False
    return cls in TEACHER_QUALIFICATIONS.get(uid, set())

def teacher_slot_allowed(uid, wd, st):
    """Check the slot-time blocklist (e.g. a teacher cannot teach 5:45am)."""
    blocked_slots = TEACHER_SLOT_BLOCKLIST.get(uid, set())
    return (wd, st) not in blocked_slots

def teacher_date_allowed(uid, date, st):
    """Check the date-specific blocklist (e.g. Teacher J another-location conflicts)."""
    blocked = TEACHER_DATE_BLOCKLIST.get(uid, set())
    return (str(date), st) not in blocked

def weighted_ranking(counter, total_assigned_lookup=None):
    """
    Rank candidates by April experience * persistent weight - variety penalty.
    total_assigned_lookup: dict uid -> count of June assignments so far
    (variety penalty pushes heavily-loaded teachers down)
    """
    items = []
    for uid, count in counter.items():
        weight = RANKING_WEIGHTS.get(uid, 1.0)
        score = count * weight
        if total_assigned_lookup:
            penalty_mult = VARIETY_PENALTY_MULTIPLIER.get(uid, 1.0)
            score -= total_assigned_lookup.get(uid, 0) * VARIETY_PENALTY_PER_CLASS * penalty_mult
        items.append((uid, score))
    items.sort(key=lambda x: -x[1])
    return items

# ============================================================
# Build April class grid (for slot/class-type definition + ranking)
# ============================================================
april_shifts = [e for e in may_data['april_events'] if e.get('type') == 'shift' and is_ours(e)]
slot_classes = defaultdict(lambda: defaultdict(list))
slot_end = {}
slot_teachers = defaultdict(Counter)  # for RANKING

for e in april_shifts:
    s = parse_dt(e.get('dtstart')); en = parse_dt(e.get('dtend'))
    if not s or not en: continue
    cls = POSITION_NAMES.get(pos_of(e), '?')
    if cls in ('Sales Rep', 'Focus'): continue
    wd = s.weekday(); st = s.strftime('%H:%M')
    slot_classes[(wd, st)][cls].append(s.date())
    slot_end[(wd, st, cls)] = en.strftime('%H:%M')
    if uid_of(e) in TEACHERS:
        slot_teachers[(wd, st, cls)][uid_of(e)] += 1

# Strip blocklisted teachers from ranking pool
for key in list(slot_teachers.keys()):
    wd, st, cls = key
    for uid in list(slot_teachers[key].keys()):
        if not teacher_qualified(uid, cls):
            del slot_teachers[key][uid]

# Priority seeding (boost weight for a teacher at specific slots; empty by default)
for (wd, st) in PRIORITY_SLOTS:
    for cls in slot_classes.get((wd, st), {}):
        if not teacher_qualified(PRIORITY_UID, cls): continue
        slot_teachers[(wd, st, cls)][PRIORITY_UID] = max(slot_teachers[(wd, st, cls)].get(PRIORITY_UID, 0), 3)

# Build slot_rule
slot_rule = {}
for (wd, st), classes in sorted(slot_classes.items()):
    total = sum(len(dates) for dates in classes.values())
    if total < 2: continue
    cls_list = [(cls, sorted(dates)) for cls, dates in classes.items() if len(dates) >= 1]
    if len(cls_list) == 1:
        cls, dates = cls_list[0]
        if len(dates) >= 2:
            slot_rule[(wd, st)] = {'weekly': (cls, slot_end[(wd, st, cls)], NAME_TO_PID.get(cls))}
    else:
        cls_by_week = {}
        for cls, dates in cls_list:
            for dt in dates: cls_by_week[dt.isocalendar().week] = cls
        weeks_sorted = sorted(cls_by_week.keys())
        first_wk = weeks_sorted[0]
        rule_a = cls_by_week[first_wk]
        rule_b = None
        for wk in weeks_sorted:
            if (wk - first_wk) % 2 == 1 and rule_b is None:
                rule_b = cls_by_week[wk]
        if rule_b is None:
            slot_rule[(wd, st)] = {'weekly': (rule_a, slot_end[(wd, st, rule_a)], NAME_TO_PID.get(rule_a))}
        else:
            slot_rule[(wd, st)] = {
                'biweekly': True, 'reference_week': first_wk,
                'A': (rule_a, slot_end[(wd, st, rule_a)], NAME_TO_PID.get(rule_a)),
                'B': (rule_b, slot_end[(wd, st, rule_b)], NAME_TO_PID.get(rule_b)),
            }

# Saturday time shift
saturday_orig = {(w, s): r for (w, s), r in slot_rule.items() if w == 5}
for (wd, old_st), rule in saturday_orig.items():
    if old_st in SAT_TIME_SHIFTS:
        new_st = SAT_TIME_SHIFTS[old_st]
        del slot_rule[(wd, old_st)]
        if 'weekly' in rule:
            cls, end, pid = rule['weekly']
            slot_rule[(wd, new_st)] = {'weekly': (cls, add_min(end, 15), pid)}
        elif rule.get('biweekly'):
            new_rule = {'biweekly': True, 'reference_week': rule['reference_week']}
            for parity in ('A', 'B'):
                cls, end, pid = rule[parity]
                new_rule[parity] = (cls, add_min(end, 15), pid)
            slot_rule[(wd, new_st)] = new_rule
        for old_key in list(slot_teachers.keys()):
            ow, os_, ocls = old_key
            if ow == wd and os_ == old_st:
                new_key = (ow, new_st, ocls)
                slot_teachers[new_key] = slot_teachers[old_key]
                slot_end[new_key] = add_min(slot_end[old_key], 15)
                del slot_teachers[old_key]

# Sunday time shift for June (mirrors Saturday shift logic)
sunday_orig = {(w, s): r for (w, s), r in slot_rule.items() if w == 6}
for (wd, old_st), rule in sunday_orig.items():
    if old_st in SUN_TIME_SHIFTS:
        new_st = SUN_TIME_SHIFTS[old_st]
        del slot_rule[(wd, old_st)]
        if 'weekly' in rule:
            cls, end, pid = rule['weekly']
            # Compute new end = new_start + (old_end - old_start)
            def to_min(t): h, m = map(int, t.split(':')); return h*60 + m
            shift_amt = to_min(new_st) - to_min(old_st)
            new_end = add_min(end, shift_amt)
            slot_rule[(wd, new_st)] = {'weekly': (cls, new_end, pid)}
        elif rule.get('biweekly'):
            new_rule = {'biweekly': True, 'reference_week': rule['reference_week']}
            def to_min2(t): h, m = map(int, t.split(':')); return h*60 + m
            shift_amt = to_min2(new_st) - to_min2(old_st)
            for parity in ('A', 'B'):
                cls, end, pid = rule[parity]
                new_rule[parity] = (cls, add_min(end, shift_amt), pid)
            slot_rule[(wd, new_st)] = new_rule
        for old_key in list(slot_teachers.keys()):
            ow, os_, ocls = old_key
            if ow == wd and os_ == old_st:
                new_key = (ow, new_st, ocls)
                slot_teachers[new_key] = slot_teachers[old_key]
                def to_min3(t): h, m = map(int, t.split(':')); return h*60 + m
                shift_amt = to_min3(new_st) - to_min3(old_st)
                slot_end[new_key] = add_min(slot_end[old_key], shift_amt)
                del slot_teachers[old_key]

# Drop Tue + Thu 9:45
for slot_to_remove in JUNE_SLOTS_TO_REMOVE:
    if slot_to_remove in slot_rule:
        del slot_rule[slot_to_remove]

# June blocked time
june_blocks = defaultdict(list)
for e in june_data.get('june_events', []):
    if not is_ours(e): continue
    if e.get('type') in ('leave', 'availability'):
        s, en = parse_dt(e.get('dtstart')), parse_dt(e.get('dtend'))
        if s and en: june_blocks[uid_of(e)].append((s, en))
def is_blocked(uid, ss, se):
    return any(bs < se and be > ss for bs, be in june_blocks.get(uid, []))

# Per-teacher special rules ship empty in the generic build. Each rule is a
# callable (uid, start, end, weekly_assignments) -> bool; populate per deployment.
SPECIAL = []
def passes_special(uid, ss, se, wa):
    return all(rule(uid, ss, se, wa) for rule in SPECIAL)

# ============================================================
# CANDIDACY: Sling positions = pool, April history = ranking
# ============================================================
def get_candidates(cls, wd, st):
    """
    Return ordered list of (uid, weighted_score) for everyone qualified
    to teach `cls` (per Sling), ranked by April experience at this exact
    slot, then class+day, then class anywhere.
    Anyone qualified per Sling but with no April history gets weight 0.5
    (so they're below experienced teachers but ABOVE non-candidates).
    """
    # Tier 1: experience at this exact (weekday, time, class)
    tier1 = Counter(slot_teachers.get((wd, st, cls), {}))
    # Tier 2: same class+day elsewhere
    tier2 = Counter()
    for (w_, s_, c_), tcs in slot_teachers.items():
        if w_ == wd and c_ == cls and (w_, s_) != (wd, st):
            for u, c in tcs.items(): tier2[u] += c
    # Tier 3: same class type, any day/time
    tier3 = Counter()
    for (w_, s_, c_), tcs in slot_teachers.items():
        if c_ == cls and (w_, s_) != (wd, st) and not (w_ == wd and c_ == cls):
            for u, c in tcs.items(): tier3[u] += c
    # Tier 4: qualified per Sling but no recent experience -> weight 0.5
    qualified_uids = {u for u in TEACHERS if teacher_qualified(u, cls)}
    seen = set(tier1) | set(tier2) | set(tier3)
    tier4 = Counter({u: 0.5 for u in qualified_uids - seen})

    return [tier1, tier2, tier3, tier4]

# Track assignments
weekly_count = defaultdict(lambda: defaultdict(int))
weekly_assignments = defaultdict(list)
proposed = []
manual_slot_keys = set()
focus_dates_used = set()
focus_weeks_used = set()

def is_double_booked(uid, ss, se):
    return any(s < se and e > ss for s, e in weekly_assignments[uid])
def under_max(uid, wk):
    return weekly_count[uid][wk] < TARGETS.get(uid, (4, 4))[1]
def under_target(uid, wk):
    return weekly_count[uid][wk] < TARGETS.get(uid, (4, 4))[0]
def slot_eligible(uid, ss, se, wk):
    if is_blocked(uid, ss, se): return False
    if is_double_booked(uid, ss, se): return False
    if not passes_special(uid, ss, se, weekly_assignments): return False
    return True

def try_assign(slot_start, slot_end_, week_key_str, cls, wd, st, exclude_uid=False):
    """Iterate tiers, then under-target/under-max passes."""
    tiers = get_candidates(cls, wd, st)
    tier_labels = ['primary (April exact slot)', 'same class+day fallback',
                   'class type fallback', 'Sling-qualified (no Apr history)']

    # Compute monthly load for variety penalty
    monthly_load = {uid: sum(weekly_count[uid].values()) for uid in TEACHERS}

    for tier, label in zip(tiers, tier_labels):
        cand_list = weighted_ranking(tier, monthly_load)
        for under_fn, label2 in [(under_target, 'under target'), (under_max, 'under max')]:
            for cand_uid, _ in cand_list:
                if cand_uid not in TEACHERS: continue
                if exclude_uid and cand_uid == EXCLUDE_UID: continue
                if not teacher_qualified(cand_uid, cls): continue
                if not teacher_slot_allowed(cand_uid, wd, st): continue
                if not teacher_date_allowed(cand_uid, slot_start.date(), st): continue
                if not slot_eligible(cand_uid, slot_start, slot_end_, week_key_str): continue
                if under_fn(cand_uid, week_key_str):
                    return cand_uid, f"{label}, {label2}"
    return None, None

_tm_year, _tm_month = map(int, TARGET_MONTH.split('-'))
june_dates = []
day = datetime(_tm_year, _tm_month, 1, tzinfo=TZ)
while day.month == _tm_month:
    june_dates.append(day); day += timedelta(days=1)

# Focus placement
print(f"\nPlacing {len(FOCUS_PLACEMENTS_JUNE)} Focus class(es):")
for placement in FOCUS_PLACEMENTS_JUNE:
    teachers = [NAME_TO_UID[n] for n in placement['teachers']]
    allowed_weeks = placement.get('allowed_iso_weeks')
    candidates = []
    for date in june_dates:
        if date.weekday() not in placement['weekdays']: continue
        if date.date() in focus_dates_used: continue
        iso_wk = date.isocalendar().week
        if allowed_weeks and iso_wk not in allowed_weeks: continue
        if iso_wk in focus_weeks_used: continue
        for st in placement['times']:
            sh, sm = map(int, st.split(':'))
            ss = date.replace(hour=sh, minute=sm)
            se = ss + timedelta(minutes=placement['duration_min'])
            wk = ss.strftime('%G-W%V')
            ok = True
            for uid in teachers:
                if is_blocked(uid, ss, se) or is_double_booked(uid, ss, se) or not under_max(uid, wk):
                    ok = False; break
                day_slots_today = {sl for (w, sl) in slot_rule if w == date.weekday()}
                if st in day_slots_today and date.weekday() == 5: ok = False; break
            if ok: candidates.append((iso_wk, ss, se, st))
    if not candidates:
        for date in june_dates:
            if date.weekday() not in placement['weekdays']: continue
            if date.date() in focus_dates_used: continue
            iso_wk = date.isocalendar().week
            if allowed_weeks and iso_wk not in allowed_weeks: continue
            for st in placement['times']:
                sh, sm = map(int, st.split(':'))
                ss = date.replace(hour=sh, minute=sm)
                se = ss + timedelta(minutes=placement['duration_min'])
                wk = ss.strftime('%G-W%V')
                ok = all(not is_blocked(u, ss, se) and not is_double_booked(u, ss, se) and under_max(u, wk) for u in teachers)
                if ok: candidates.append((iso_wk, ss, se, st))
    if not candidates: continue
    candidates.sort()
    iso_wk, ss, se, st = candidates[0]
    en = se.strftime('%H:%M')
    wk = ss.strftime('%G-W%V')
    pid = NAME_TO_PID['Focus']
    for uid in teachers:
        weekly_assignments[uid].append((ss, se))
        weekly_count[uid][wk] += 1
    teacher_label = " + ".join(placement['teachers'])
    proposed.append((ss.date(), WD[ss.weekday()], st, en, 'Focus', pid,
                     teachers[0], f"manual ({placement['category']})", "", teacher_label))
    manual_slot_keys.add((ss.date(), st))
    focus_dates_used.add(ss.date())
    focus_weeks_used.add(iso_wk)
    print(f"  {placement['category']:12s} -> {ss.date()} {WD[ss.weekday()]} {st}-{en}")

dropped_slots = []

for date in june_dates:
    wd = date.weekday()
    iso_wk = date.isocalendar().week
    week_key_str = date.strftime('%G-W%V')
    day_slots_regular = {(wd, st) for (w, st) in slot_rule if w == wd}
    day_slots_7am = {(wd, '07:00')} if wd in JUNE_7AM_BIWEEKLY else set()
    day_slots = sorted(day_slots_regular | day_slots_7am)

    for slot_id in day_slots:
        wd_, st = slot_id
        if (date.date(), st) in manual_slot_keys: continue

        is_new_7am = (wd in JUNE_7AM_BIWEEKLY and st == '07:00')
        if is_new_7am:
            biweekly = JUNE_7AM_BIWEEKLY[wd]
            ref = biweekly['reference_week']
            cls = biweekly['A'] if (iso_wk - ref) % 2 == 0 else biweekly['B']
            pid = NAME_TO_PID.get(cls)
            en = '08:00'
            sh, sm = map(int, st.split(':'))
            slot_start = date.replace(hour=sh, minute=sm)
            slot_end_dt = date.replace(hour=8, minute=0)

            same_day_545 = []
            for uid, assigns in weekly_assignments.items():
                for s, e in assigns:
                    if s.date() == date.date() and s.strftime('%H:%M') == '05:45':
                        same_day_545.append(uid)

            chosen, reason = try_assign(slot_start, slot_end_dt, week_key_str,
                                         cls, wd, st, exclude_uid=True)
            if not chosen:
                chosen, reason = try_assign(slot_start, slot_end_dt, week_key_str,
                                             cls, wd, st, exclude_uid=False)
            else:
                if chosen in same_day_545:
                    reason = f"7am STACKED w/ 5:45am: {reason}"
                else:
                    reason = f"7am: {reason}"
        else:
            rule = slot_rule[slot_id]
            if 'weekly' in rule:
                cls, en, pid = rule['weekly']
            elif rule.get('biweekly'):
                ref = rule['reference_week']
                cls, en, pid = rule['A'] if (iso_wk - ref) % 2 == 0 else rule['B']
            else:
                continue
            if (wd, st) in JUNE_SLOT_CLASS_OVERRIDES:
                cls = JUNE_SLOT_CLASS_OVERRIDES[(wd, st)]
                pid = NAME_TO_PID.get(cls)
            sh, sm = map(int, st.split(':'))
            eh, em = map(int, en.split(':'))
            slot_start = date.replace(hour=sh, minute=sm)
            slot_end_dt = date.replace(hour=eh, minute=em)
            chosen, reason = try_assign(slot_start, slot_end_dt, week_key_str, cls, wd, st)

        flexed_class = None
        if not chosen and not is_new_7am:
            other_classes_this_slot = [c for c in slot_classes.get((wd, st), {})
                                        if c != cls and c not in ('Focus', 'Sales Rep')]
            for alt_cls in other_classes_this_slot:
                chosen, reason = try_assign(slot_start, slot_end_dt, week_key_str, alt_cls, wd, st)
                if chosen:
                    flexed_class = alt_cls; cls = alt_cls; pid = NAME_TO_PID.get(alt_cls)
                    reason = f"FORMAT-FLEX: {reason}"; break
            if not chosen:
                all_classes = {c for c in NAME_TO_PID if c not in ('Focus', 'Sales Rep', 'Teacher')}
                for alt_cls in sorted(all_classes):
                    if alt_cls == cls: continue
                    chosen, reason = try_assign(slot_start, slot_end_dt, week_key_str, alt_cls, wd, st)
                    if chosen:
                        flexed_class = alt_cls; cls = alt_cls; pid = NAME_TO_PID.get(alt_cls)
                        reason = f"FORMAT-FLEX (broad): {reason}"; break

        lead_overflow = lead_over_cap = False
        if not chosen and LEAD_UID in TARGETS:
            if (not is_blocked(LEAD_UID, slot_start, slot_end_dt)
                and not is_double_booked(LEAD_UID, slot_start, slot_end_dt)
                and passes_special(LEAD_UID, slot_start, slot_end_dt, weekly_assignments)
                and teacher_qualified(LEAD_UID, cls)):
                chosen = LEAD_UID
                wc_now = weekly_count[LEAD_UID][week_key_str]
                cap = TARGETS[LEAD_UID][1]
                lead_overflow = True
                if wc_now >= cap:
                    lead_over_cap = True
                    reason = f"LEAD OVERFLOW (over cap, {wc_now+1}/{cap})"
                else:
                    reason = f"LEAD OVERFLOW ({wc_now+1}/{cap})"

        # 7am format-flex: if a 7am slot can't be filled with its rotated class,
        # try Classic (always-safe fallback)
        seven_am_flexed = None
        if not chosen and is_new_7am and cls != 'Classic':
            chosen, reason = try_assign(slot_start, slot_end_dt, week_key_str,
                                         'Classic', wd, st, exclude_uid=True)
            if not chosen:
                chosen, reason = try_assign(slot_start, slot_end_dt, week_key_str,
                                             'Classic', wd, st, exclude_uid=False)
            else:
                reason = f"7am FORMAT-FLEX to Classic: {reason}"
            if chosen:
                seven_am_flexed = 'Classic'
                cls = 'Classic'
                pid = NAME_TO_PID.get('Classic')

        if not chosen:
            dropped_slots.append((date.date(), WD[wd], st, en, cls))
            proposed.append((date.date(), WD[wd], st, en, cls, pid, None, "DROPPED", "DROP", ''))
            continue

        if seven_am_flexed:
            flag_extra = f"7AM FLEXED to {seven_am_flexed}"
        else:
            flag_extra = ''


        flag = ""
        if flexed_class: flag = (flag + " | " if flag else "") + f"CLASS CHANGED to {flexed_class}"
        if lead_over_cap: flag = (flag + " | " if flag else "") + "LEAD OVER CAP"
        if is_new_7am: flag = (flag + " | " if flag else "") + "NEW 7AM SLOT"
        if flag_extra: flag = (flag + " | " if flag else "") + flag_extra

        weekly_assignments[chosen].append((slot_start, slot_end_dt))
        weekly_count[chosen][week_key_str] += 1
        proposed.append((date.date(), WD[wd], st, en, cls, pid, chosen, reason, flag, ''))

# Reporting
print(f"\n{'=' * 70}\nSUMMARY\n{'=' * 70}")
total = len(proposed)
filled = sum(1 for _, _, _, _, _, _, uid, _, _, _ in proposed if uid)
print(f"Total slots: {total}, Filled: {filled}, Dropped: {len(dropped_slots)}")
if dropped_slots:
    print(f"\nDROPPED:")
    for d in dropped_slots: print(f"  {d}")

# Per-teacher load
prop_load = Counter()
for *_, uid, _, _, _ in proposed:
    if uid: prop_load[uid] += 1
print("\nLoad by teacher:")
for uid in sorted(TEACHERS, key=lambda x: str(TEACHERS.get(x, x))):
    tgt, mx = TARGETS.get(uid, ('?', '?'))
    print(f"  {str(TEACHERS.get(uid, uid)):25s}  {prop_load.get(uid, 0):>3}  cap {tgt}/{mx}")

os.makedirs('data/output', exist_ok=True)
with open('data/output/proposed.csv', 'w', newline='') as f:
    w = csv.DictWriter(f, fieldnames=['date', 'weekday', 'start', 'end', 'class', 'proposed_teacher', 'reason', 'flag'])
    w.writeheader()
    for date, wd_name, st, en, cls, pid, prop_uid, reason, flag, coteach in sorted(proposed):
        teacher_display = coteach if coteach else (TEACHERS.get(prop_uid, 'DROPPED') if prop_uid else 'DROPPED')
        w.writerow({'date': str(date), 'weekday': wd_name, 'start': st, 'end': en, 'class': cls,
                    'proposed_teacher': teacher_display, 'reason': reason, 'flag': flag})
print("\nFile: proposed.csv")

if JSON_OUT:
    payload = {
        'algorithm_version': 'v9',
        'target_month': TARGET_MONTH,
        'parameters': {
            'variety_penalty_per_class': VARIETY_PENALTY_PER_CLASS,
            'sat_time_shifts': SAT_TIME_SHIFTS,
            'sun_time_shifts': SUN_TIME_SHIFTS,
        },
        'shifts': [
            {
                'shift_date': str(date),
                'weekday': wd_name,
                'start_time': st,
                'end_time': en,
                'class_name': cls,
                'sling_position_id': pid,
                'sling_user_id': prop_uid,  # may be None (dropped)
                'generation_reason': reason or '',
                'flag': flag or '',
                'is_coteach': bool(coteach),
                'coteach_label': coteach or '',
                'is_dropped': prop_uid is None,
            }
            for date, wd_name, st, en, cls, pid, prop_uid, reason, flag, coteach in sorted(proposed)
        ],
    }
    sys.stdout = _stdout
    json.dump(payload, sys.stdout)
