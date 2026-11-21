//! Sync engine tests: each "device" is its own in-memory database with its own
//! device id; events move between them the way the network layer will move them.

use chrono::{DateTime, Duration, Utc};
use rusqlite::Connection;

use super::engine::{self, Event, Op};
use crate::db;
use crate::error::Error;
use crate::tasks::{self, TaskKind};

fn t0() -> DateTime<Utc> {
    DateTime::parse_from_rfc3339("2026-11-21T09:00:00Z")
        .unwrap()
        .with_timezone(&Utc)
}

fn at(secs: i64) -> DateTime<Utc> {
    t0() + Duration::seconds(secs)
}

fn device() -> Connection {
    db::open_in_memory().unwrap()
}

/// Send everything `from` has that `to` lacks. Returns how many events were new to `to`.
fn sync(from: &Connection, to: &Connection) -> usize {
    let missing = engine::events_missing(from, &engine::version_vector(to).unwrap()).unwrap();
    engine::ingest(to, &missing).unwrap()
}

fn sync_both(a: &Connection, b: &Connection) {
    sync(a, b);
    sync(b, a);
}

fn id_of(conn: &Connection, name: &str) -> i64 {
    conn.query_row("SELECT id FROM task WHERE name = ?1", [name], |r| r.get(0))
        .unwrap()
}

fn names(conn: &Connection) -> Vec<String> {
    tasks::list(conn)
        .unwrap()
        .into_iter()
        .map(|t| t.name)
        .collect()
}

/// Everything that must match between devices (local integer ids excluded).
fn snapshot(conn: &Connection) -> String {
    let mut out = String::new();
    let mut q = |sql: &str| {
        let mut stmt = conn.prepare(sql).unwrap();
        let cols = stmt.column_count();
        let rows = stmt
            .query_map([], |r| {
                Ok((0..cols)
                    .map(|i| format!("{:?}", r.get_ref(i).unwrap()))
                    .collect::<Vec<_>>()
                    .join("|"))
            })
            .unwrap();
        for row in rows {
            out.push_str(&row.unwrap());
            out.push('\n');
        }
        out.push_str("--\n");
    };
    q(
        "SELECT uid, name, kind, parent_uid, estimated, pomodoros, unplanned, interruptions,
              done, visible, ordinal, created_at, closed_at FROM task ORDER BY uid",
    );
    q("SELECT uid, task_uid, started_at, ended_at FROM pomodoro_log ORDER BY uid");
    q("SELECT uid, kind, parent_uid FROM tombstone ORDER BY uid");
    q(
        "SELECT t.uid FROM task t WHERE visible = 1 AND kind != 'interruption'
       ORDER BY ordinal, created_at, uid",
    );
    out
}

#[test]
fn a_task_created_on_one_device_appears_on_the_other() {
    let (a, b) = (device(), device());
    tasks::create(&a, "Write docs", Some(2), at(0)).unwrap();
    assert_eq!(sync(&a, &b), 1);
    let on_b = &tasks::list(&b).unwrap()[0];
    assert_eq!(
        (on_b.name.as_str(), on_b.estimated),
        ("Write docs", Some(2))
    );
    assert_eq!(on_b.created_at, "2026-11-21T09:00:00Z");
    assert_eq!(snapshot(&a), snapshot(&b));
}

#[test]
fn lists_from_both_devices_merge_without_loss() {
    let (a, b) = (device(), device());
    tasks::create(&a, "From A", None, at(0)).unwrap();
    tasks::create(&b, "From B", None, at(1)).unwrap();
    sync_both(&a, &b);
    let mut merged = names(&a);
    merged.sort();
    assert_eq!(merged, ["From A", "From B"]);
    assert_eq!(snapshot(&a), snapshot(&b));
}

#[test]
fn concurrent_edits_newest_wins_per_field() {
    let (a, b) = (device(), device());
    tasks::create(&a, "Draft", Some(1), at(0)).unwrap();
    sync(&a, &b);
    let (ia, ib) = (id_of(&a, "Draft"), id_of(&b, "Draft"));
    // Offline on both: A renames (earlier), B renames later and A changes the estimate.
    tasks::update(&a, ia, "Renamed on A", Some(3), at(10)).unwrap();
    tasks::update(&b, ib, "Renamed on B", Some(1), at(20)).unwrap();
    sync_both(&a, &b);
    let t = tasks::get(&a, ia).unwrap();
    assert_eq!(t.name, "Renamed on B", "later rename wins");
    assert_eq!(
        t.estimated,
        Some(3),
        "B didn't touch the estimate, so A's edit survives"
    );
    assert_eq!(snapshot(&a), snapshot(&b));
}

#[test]
fn delete_wins_over_concurrent_edits() {
    let (a, b) = (device(), device());
    tasks::create(&a, "Doomed", None, at(0)).unwrap();
    sync(&a, &b);
    tasks::delete(&a, id_of(&a, "Doomed"), at(10)).unwrap();
    // B edits it later, not knowing it was deleted.
    tasks::update(&b, id_of(&b, "Doomed"), "Edited later", None, at(20)).unwrap();
    sync_both(&a, &b);
    assert!(names(&a).is_empty() && names(&b).is_empty());
    assert_eq!(snapshot(&a), snapshot(&b));
}

#[test]
fn work_on_both_devices_adds_up() {
    let (mut a, mut b) = (device(), device());
    tasks::create(&a, "Shared", Some(4), at(0)).unwrap();
    sync(&a, &b);
    let (ia, ib) = (id_of(&a, "Shared"), id_of(&b, "Shared"));
    tasks::record_pomodoro(&mut a, Some(ia), at(10), at(1510)).unwrap();
    tasks::record_pomodoro(&mut b, Some(ib), at(20), at(1520)).unwrap();
    tasks::record_pomodoro(&mut b, Some(ib), at(2000), at(3500)).unwrap();
    tasks::log_disturbance(&mut a, ia, TaskKind::Interruption, "Phone", None, at(30)).unwrap();
    tasks::log_disturbance(&mut b, ib, TaskKind::Unplanned, "Hotfix", Some(1), at(40)).unwrap();
    sync_both(&a, &b);
    let t = tasks::get(&a, ia).unwrap();
    assert_eq!((t.pomodoros, t.unplanned, t.interruptions), (3, 1, 1));
    assert_eq!(snapshot(&a), snapshot(&b));
}

#[test]
fn deleting_an_unplanned_item_keeps_the_parents_count() {
    let (mut a, b) = (device(), device());
    let p = tasks::create(&a, "Main", None, at(0)).unwrap();
    let u =
        tasks::log_disturbance(&mut a, p.id, TaskKind::Unplanned, "Hotfix", None, at(1)).unwrap();
    tasks::delete(&a, u.id, at(2)).unwrap();
    assert_eq!(tasks::get(&a, p.id).unwrap().unplanned, 1);
    sync(&a, &b);
    assert_eq!(tasks::get(&b, id_of(&b, "Main")).unwrap().unplanned, 1);
}

#[test]
fn receiving_the_same_events_twice_changes_nothing() {
    let (a, b) = (device(), device());
    tasks::create(&a, "Once", None, at(0)).unwrap();
    let events = engine::events_missing(&a, &Default::default()).unwrap();
    assert_eq!(engine::ingest(&b, &events).unwrap(), 1);
    let before = snapshot(&b);
    assert_eq!(engine::ingest(&b, &events).unwrap(), 0);
    assert_eq!(snapshot(&b), before);
}

#[test]
fn events_can_arrive_in_any_order_and_with_gaps() {
    let (mut a, b) = (device(), device());
    let p = tasks::create(&a, "Parent", Some(2), at(0)).unwrap();
    let u =
        tasks::log_disturbance(&mut a, p.id, TaskKind::Unplanned, "Child", None, at(1)).unwrap();
    tasks::record_pomodoro(&mut a, Some(p.id), at(2), at(3)).unwrap();
    tasks::update(&a, p.id, "Parent renamed", Some(2), at(4)).unwrap();
    tasks::delete(&a, u.id, at(5)).unwrap();
    let all = engine::events_missing(&a, &Default::default()).unwrap();
    assert_eq!(all.len(), 5);

    // Deliver only the last three, newest first (the creations are missing).
    let partial: Vec<Event> = all[2..].iter().rev().cloned().collect();
    for e in &partial {
        engine::ingest(&b, std::slice::from_ref(e)).unwrap();
    }
    // The gap means B still asks for everything from seq 1.
    assert_eq!(
        engine::version_vector(&b)
            .unwrap()
            .values()
            .copied()
            .collect::<Vec<_>>(),
        [0; 0]
    );
    assert_eq!(sync(&a, &b), 2, "only the two missing events are new");
    assert_eq!(snapshot(&a), snapshot(&b));
    assert_eq!(names(&b), ["Parent renamed"]);
}

#[test]
fn malformed_events_are_rejected_as_a_whole() {
    let (a, b) = (device(), device());
    tasks::create(&a, "Good", None, at(0)).unwrap();
    let mut events = engine::events_missing(&a, &Default::default()).unwrap();
    let mut bad = events[0].clone();
    bad.seq = 2;
    bad.op = Op::SetName {
        uid: "not-a-uid".into(),
        name: "x".into(),
    };
    events.push(bad);
    assert!(matches!(
        engine::ingest(&b, &events),
        Err(Error::Invalid(_))
    ));
    assert!(
        names(&b).is_empty(),
        "nothing from a rejected batch is applied"
    );

    let mut forged = engine::events_missing(&a, &Default::default()).unwrap();
    forged[0].device = "f".repeat(32); // clock says the event came from someone else
    assert!(engine::ingest(&b, &forged).is_err());

    let mut long = engine::events_missing(&a, &Default::default()).unwrap();
    if let Op::CreateTask { name, .. } = &mut long[0].op {
        *name = "x".repeat(500);
    }
    assert!(engine::ingest(&b, &long).is_err());
}

#[test]
fn a_device_with_a_slow_clock_still_wins_with_a_later_edit() {
    let (a, b) = (device(), device());
    tasks::create(&a, "Clocky", None, at(1000)).unwrap();
    sync(&a, &b);
    // B's clock is 15 minutes behind, but its edit comes after it saw A's event.
    tasks::update(&b, id_of(&b, "Clocky"), "Edited on B", None, at(100)).unwrap();
    sync(&b, &a);
    assert_eq!(names(&a), ["Edited on B"]);
}

#[test]
fn data_from_before_sync_is_kept_and_shared_on_first_sync() {
    // A database as the previous version left it: schema 1 with existing data.
    let old = Connection::open_in_memory().unwrap();
    old.execute_batch(include_str!("../../migrations/001_init.sql"))
        .unwrap();
    old.pragma_update(None, "user_version", 1).unwrap();
    old.execute_batch(
        "INSERT INTO task (id, name, kind, estimated, pomodoros, unplanned, interruptions, done, visible, ordinal, created_at, closed_at)
           VALUES (1, 'Old task', 'pomodoro', 3, 2, 1, 1, 1, 0, 0, '2026-10-01T09:00:00Z', '2026-10-02T10:00:00Z'),
                  (2, 'Open task', 'pomodoro', 1, 0, 0, 0, 0, 1, 1, '2026-10-03T09:00:00Z', NULL);
         INSERT INTO task (id, name, kind, parent_id, ordinal, created_at) VALUES
                  (3, 'Hotfix', 'unplanned', 1, 2, '2026-10-01T10:00:00Z'),
                  (4, 'Phone', 'interruption', 1, 0, '2026-10-01T11:00:00Z');
         INSERT INTO pomodoro_log (task_id, started_at, ended_at) VALUES
                  (1, '2026-10-01T09:00:00Z', '2026-10-01T09:25:00Z'),
                  (1, '2026-10-01T09:30:00Z', '2026-10-01T09:55:00Z');",
    )
    .unwrap();
    let a = db::init(old).unwrap();

    let t = tasks::get(&a, 1).unwrap();
    assert_eq!(
        (t.name.as_str(), t.done, t.estimated),
        ("Old task", true, Some(3))
    );
    assert_eq!((t.pomodoros, t.unplanned, t.interruptions), (2, 1, 1));
    assert_eq!(t.closed_at.as_deref(), Some("2026-10-02T10:00:00Z"));
    assert_eq!(
        names(&a),
        ["Open task", "Hotfix"],
        "hidden task stays hidden"
    );

    // Opening again doesn't duplicate anything.
    let events_before = engine::events_missing(&a, &Default::default())
        .unwrap()
        .len();
    engine::bootstrap(&a).unwrap();
    assert_eq!(
        engine::events_missing(&a, &Default::default())
            .unwrap()
            .len(),
        events_before
    );

    let b = device();
    sync(&a, &b);
    assert_eq!(snapshot(&a), snapshot(&b));
}

// ---- Randomised convergence -----------------------------------------------------------

/// Small deterministic PRNG (xorshift64*), so failures can be reproduced by seed.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }
    fn below(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }
    fn chance(&mut self, percent: u64) -> bool {
        self.next() % 100 < percent
    }
}

fn random_op(rng: &mut Rng, conn: &mut Connection, now: DateTime<Utc>, step: usize) {
    let ids: Vec<i64> = {
        let mut stmt = conn.prepare("SELECT id FROM task ORDER BY id").unwrap();
        stmt.query_map([], |r| r.get(0))
            .unwrap()
            .map(|r| r.unwrap())
            .collect()
    };
    let pick = |rng: &mut Rng| (!ids.is_empty()).then(|| ids[rng.below(ids.len())]);
    match rng.below(10) {
        0 | 1 => {
            tasks::create(
                conn,
                &format!("task {step}"),
                Some(rng.below(5) as i64),
                now,
            )
            .unwrap();
        }
        2 => {
            if let Some(id) = pick(rng) {
                let est = Some(rng.below(6) as i64);
                tasks::update(conn, id, &format!("renamed {step}"), est, now).unwrap();
            }
        }
        3 => {
            if let Some(id) = pick(rng) {
                tasks::set_done(conn, id, rng.chance(70), now).unwrap();
            }
        }
        4 => {
            if let Some(id) = pick(rng) {
                tasks::delete(conn, id, now).unwrap();
            }
        }
        5 => {
            let mut order = ids.clone();
            for i in (1..order.len()).rev() {
                order.swap(i, rng.below(i + 1));
            }
            tasks::reorder(conn, &order, now).unwrap();
        }
        6 => {
            tasks::hide_done(conn, now).unwrap();
        }
        7 => {
            let task = if rng.chance(80) { pick(rng) } else { None };
            tasks::record_pomodoro(conn, task, now - Duration::minutes(25), now).unwrap();
        }
        _ => {
            if let Some(id) = pick(rng) {
                let kind = if rng.chance(50) {
                    TaskKind::Unplanned
                } else {
                    TaskKind::Interruption
                };
                tasks::log_disturbance(conn, id, kind, &format!("disturbance {step}"), None, now)
                    .unwrap();
            }
        }
    }
}

/// Deliver a random subset of what `from` has for `to`, in random order, as an
/// interrupted or reordered transfer would.
fn partial_sync(rng: &mut Rng, from: &Connection, to: &Connection) {
    let mut missing = engine::events_missing(from, &engine::version_vector(to).unwrap()).unwrap();
    for i in (1..missing.len()).rev() {
        missing.swap(i, rng.below(i + 1));
    }
    missing.truncate(rng.below(missing.len() + 1));
    for chunk in missing.chunks(3) {
        engine::ingest(to, chunk).unwrap();
    }
}

#[test]
fn random_edits_on_three_devices_always_converge() {
    for seed in 1..=60u64 {
        let mut rng = Rng(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1);
        let mut devices: Vec<Connection> = (0..3).map(|_| device()).collect();
        // Clocks disagree by up to ±10 minutes.
        let skew: Vec<i64> = (0..3).map(|_| rng.below(1201) as i64 - 600).collect();
        for step in 0..120 {
            let d = rng.below(3);
            if rng.chance(70) {
                random_op(
                    &mut rng,
                    &mut devices[d],
                    at(step as i64 * 30 + skew[d]),
                    step,
                );
            } else {
                let other = (d + 1 + rng.below(2)) % 3;
                if rng.chance(50) {
                    partial_sync(&mut rng, &devices[d], &devices[other]);
                } else {
                    sync(&devices[d], &devices[other]);
                }
            }
        }
        // Full sync until every device has everything (relay through each other).
        for _ in 0..2 {
            for i in 0..3 {
                for j in 0..3 {
                    if i != j {
                        sync(&devices[i], &devices[j]);
                    }
                }
            }
        }
        let first = snapshot(&devices[0]);
        for (i, d) in devices.iter().enumerate().skip(1) {
            assert_eq!(
                snapshot(d),
                first,
                "seed {seed}: device {i} differs from device 0"
            );
        }
        // Counts always match what is logged.
        let wrong: i64 = devices[0]
            .query_row(
                "SELECT COUNT(*) FROM task t WHERE t.pomodoros != (SELECT COUNT(*) FROM pomodoro_log l WHERE l.task_uid = t.uid)",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(
            wrong, 0,
            "seed {seed}: pomodoro counts out of step with the log"
        );
    }
}
