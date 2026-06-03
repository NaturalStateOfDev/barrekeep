// One-time seed of the roster and class-type tables. Idempotent: only runs
// when the teachers table is empty. After that, edits are user-driven and
// Sling pulls update qualifications.
//
// This is DEMO data with placeholder user IDs (1001+). On first real use,
// configure your studio in Settings and pull from Sling — real teachers
// arrive via the pull (matched by their real Sling user IDs) and you can
// deactivate or remove these demo rows.
//
// Weekly target/max are starting placeholders — adjust in the Teachers view.
// The lead (Teacher A) is 8/8; Teacher G is 2/2 with variety multiplier 3.0
// per the algorithm skill; everyone else gets 4/5.

use duckdb::{params, Connection};

const LEAD_USER_ID: i32 = 1001;

struct TeacherSeed {
    sling_user_id: i32,
    display_name: &'static str,
    weekly_target: i32,
    weekly_max: i32,
    is_lead: bool,
    variety_multiplier: f64,
}

const TEACHERS: &[TeacherSeed] = &[
    TeacherSeed { sling_user_id: 1001, display_name: "Teacher A", weekly_target: 8, weekly_max: 8, is_lead: true,  variety_multiplier: 1.0 },
    TeacherSeed { sling_user_id: 1002, display_name: "Teacher B", weekly_target: 4, weekly_max: 5, is_lead: false, variety_multiplier: 1.0 },
    TeacherSeed { sling_user_id: 1003, display_name: "Teacher C", weekly_target: 4, weekly_max: 5, is_lead: false, variety_multiplier: 1.0 },
    TeacherSeed { sling_user_id: 1004, display_name: "Teacher D", weekly_target: 4, weekly_max: 5, is_lead: false, variety_multiplier: 1.0 },
    TeacherSeed { sling_user_id: 1005, display_name: "Teacher E", weekly_target: 4, weekly_max: 5, is_lead: false, variety_multiplier: 1.0 },
    TeacherSeed { sling_user_id: 1006, display_name: "Teacher F", weekly_target: 4, weekly_max: 5, is_lead: false, variety_multiplier: 1.0 },
    TeacherSeed { sling_user_id: 1007, display_name: "Teacher G", weekly_target: 2, weekly_max: 2, is_lead: false, variety_multiplier: 3.0 },
    TeacherSeed { sling_user_id: 1008, display_name: "Teacher H", weekly_target: 4, weekly_max: 5, is_lead: false, variety_multiplier: 1.0 },
    TeacherSeed { sling_user_id: 1009, display_name: "Teacher I", weekly_target: 4, weekly_max: 5, is_lead: false, variety_multiplier: 1.0 },
    TeacherSeed { sling_user_id: 1010, display_name: "Teacher J", weekly_target: 4, weekly_max: 5, is_lead: false, variety_multiplier: 1.0 },
];

struct PositionSeed {
    sling_position_id: i32,
    class_name: &'static str,
    duration_minutes: i32,
    is_special: bool,
}

const POSITIONS: &[PositionSeed] = &[
    PositionSeed { sling_position_id: 29470407, class_name: "Empower",                  duration_minutes: 60, is_special: false },
    PositionSeed { sling_position_id: 29470419, class_name: "Focus",                    duration_minutes: 60, is_special: true  },
    PositionSeed { sling_position_id: 29470489, class_name: "Breaking Down the Barre",  duration_minutes: 60, is_special: false },
    PositionSeed { sling_position_id: 29303958, class_name: "Align",                    duration_minutes: 60, is_special: false },
    PositionSeed { sling_position_id: 29303965, class_name: "Classic",                  duration_minutes: 60, is_special: false },
    PositionSeed { sling_position_id: 29304030, class_name: "Define",                   duration_minutes: 60, is_special: false },
    PositionSeed { sling_position_id: 29304197, class_name: "Reform",                   duration_minutes: 60, is_special: false },
];

/// Demo: give the lead every class qualification so the seeded roster is
/// usable out of the box. Real qualifications come from a Sling pull.
const LEAD_QUAL_POSITIONS: &[i32] = &[
    29470407, 29470419, 29470489, 29303958, 29303965, 29304030, 29304197,
];

pub fn run_if_empty(conn: &Connection) -> anyhow::Result<()> {
    let teacher_count: i64 = conn.query_row("SELECT count(*) FROM teachers", [], |row| row.get(0))?;
    if teacher_count > 0 {
        return Ok(());
    }

    eprintln!("[seed] teachers table is empty — seeding roster + positions");

    for t in TEACHERS {
        conn.execute(
            "INSERT INTO teachers (sling_user_id, display_name, weekly_target, weekly_max,
                                   is_lead, variety_multiplier)
             VALUES (?, ?, ?, ?, ?, ?)",
            params![
                t.sling_user_id,
                t.display_name,
                t.weekly_target,
                t.weekly_max,
                t.is_lead,
                t.variety_multiplier,
            ],
        )?;
    }

    for p in POSITIONS {
        conn.execute(
            "INSERT INTO positions (sling_position_id, class_name, duration_minutes, is_special)
             VALUES (?, ?, ?, ?)",
            params![p.sling_position_id, p.class_name, p.duration_minutes, p.is_special],
        )?;
    }

    for &pos_id in LEAD_QUAL_POSITIONS {
        conn.execute(
            "INSERT INTO teacher_qualifications (sling_user_id, sling_position_id)
             VALUES (?, ?)",
            params![LEAD_USER_ID, pos_id],
        )?;
    }

    Ok(())
}
