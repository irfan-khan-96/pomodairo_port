//! Statistics for the statistics window, mirroring the original's four views:
//! completed tasks, interruptions, counts per day, and the weekly reality factor.
//!
//! Timestamps are stored in UTC; days and weeks are the user's local ones, so the
//! time zone is passed in (tests use a fixed offset).

use std::collections::{BTreeMap, BTreeSet};

use chrono::{DateTime, Datelike, Days, NaiveDate, TimeZone, Utc};
use rusqlite::{Connection, params};
use serde::{Deserialize, Serialize};

use crate::error::{Error, Result};
use crate::tasks::timestamp;

/// Longest range the per-day view accepts.
pub const MAX_RANGE_DAYS: u64 = 366;

/// Inclusive range of local calendar days.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
pub struct DateRange {
    pub from: NaiveDate,
    pub to: NaiveDate,
}

impl DateRange {
    fn checked(self) -> Result<Self> {
        if self.from > self.to {
            return Err(Error::Invalid("start date is after end date".into()));
        }
        Ok(self)
    }

    /// `[start, end)` as UTC timestamps in the stored format, for SQL comparisons.
    fn utc_bounds<Tz: TimeZone>(&self, tz: &Tz) -> (String, String) {
        let start = local_midnight(self.from, tz);
        let end = local_midnight(self.to + Days::new(1), tz);
        (timestamp(start), timestamp(end))
    }

    fn days(&self) -> impl Iterator<Item = NaiveDate> {
        self.from.iter_days().take_while(|d| *d <= self.to)
    }
}

fn local_midnight<Tz: TimeZone>(day: NaiveDate, tz: &Tz) -> DateTime<Utc> {
    let naive = day.and_hms_opt(0, 0, 0).expect("midnight is valid");
    // A DST gap can skip midnight; take the earliest valid instant.
    tz.from_local_datetime(&naive)
        .earliest()
        .unwrap_or_else(|| tz.from_utc_datetime(&naive))
        .with_timezone(&Utc)
}

fn local_day<Tz: TimeZone>(stored: &str, tz: &Tz) -> Option<NaiveDate> {
    DateTime::parse_from_rfc3339(stored)
        .ok()
        .map(|t| t.with_timezone(tz).date_naive())
}

/// SQL LIKE pattern for a case-insensitive "contains" filter, with wildcards escaped.
fn like_pattern(filter: &str) -> String {
    let escaped = filter
        .trim()
        .replace('\\', "\\\\")
        .replace('%', "\\%")
        .replace('_', "\\_");
    format!("%{escaped}%")
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct CompletedTask {
    pub name: String,
    pub kind: String,
    pub closed_at: String,
    pub estimated: Option<i64>,
    pub pomodoros: i64,
    /// Pomodoros minus estimate; `None` when not estimated.
    pub delta: Option<i64>,
    /// Unplanned items plus interruptions.
    pub interruptions: i64,
}

/// "Pomodoros" view: tasks finished in `range` whose name contains `filter`.
pub fn completed_tasks<Tz: TimeZone>(
    conn: &Connection,
    range: DateRange,
    filter: &str,
    tz: &Tz,
) -> Result<Vec<CompletedTask>> {
    let (start, end) = range.checked()?.utc_bounds(tz);
    let mut stmt = conn.prepare(
        "SELECT name, kind, closed_at, estimated, pomodoros, unplanned + interruptions
         FROM task
         WHERE done = 1 AND kind IN ('pomodoro', 'unplanned')
           AND closed_at >= ?1 AND closed_at < ?2
           AND name LIKE ?3 ESCAPE '\\'
         ORDER BY closed_at",
    )?;
    let rows = stmt.query_map(params![start, end, like_pattern(filter)], |r| {
        let estimated: Option<i64> = r.get(3)?;
        let pomodoros: i64 = r.get(4)?;
        Ok(CompletedTask {
            name: r.get(0)?,
            kind: r.get(1)?,
            closed_at: r.get(2)?,
            estimated,
            pomodoros,
            delta: estimated.map(|e| pomodoros - e),
            interruptions: r.get(5)?,
        })
    })?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Disturbance {
    pub name: String,
    pub kind: String,
    pub created_at: String,
    /// Name of the task it interrupted, if that task still exists.
    pub task: Option<String>,
}

/// "Interruptions" view: interruptions and unplanned items logged in `range`.
pub fn disturbances<Tz: TimeZone>(
    conn: &Connection,
    range: DateRange,
    filter: &str,
    tz: &Tz,
) -> Result<Vec<Disturbance>> {
    let (start, end) = range.checked()?.utc_bounds(tz);
    let mut stmt = conn.prepare(
        "SELECT d.name, d.kind, d.created_at, p.name
         FROM task d LEFT JOIN task p ON p.id = d.parent_id
         WHERE d.kind IN ('interruption', 'unplanned')
           AND d.created_at >= ?1 AND d.created_at < ?2
           AND d.name LIKE ?3 ESCAPE '\\'
         ORDER BY d.created_at",
    )?;
    let rows = stmt.query_map(params![start, end, like_pattern(filter)], |r| {
        Ok(Disturbance {
            name: r.get(0)?,
            kind: r.get(1)?,
            created_at: r.get(2)?,
            task: r.get(3)?,
        })
    })?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct DayCount {
    pub day: NaiveDate,
    /// Pomodoros completed that day (from the pomodoro log).
    pub pomodoros: i64,
    pub interruptions: i64,
    pub unplanned: i64,
    pub tasks_done: i64,
}

/// "Count" view: one row per day in `range`, including empty days.
pub fn per_day<Tz: TimeZone>(
    conn: &Connection,
    range: DateRange,
    tz: &Tz,
) -> Result<Vec<DayCount>> {
    let range = range.checked()?;
    if (range.to - range.from).num_days() as u64 >= MAX_RANGE_DAYS {
        return Err(Error::Invalid(format!(
            "range must be at most {MAX_RANGE_DAYS} days"
        )));
    }
    let (start, end) = range.utc_bounds(tz);
    let mut days: BTreeMap<NaiveDate, DayCount> = range
        .days()
        .map(|day| {
            (
                day,
                DayCount {
                    day,
                    ..Default::default()
                },
            )
        })
        .collect();
    let mut bump = |stored: String, f: fn(&mut DayCount)| {
        if let Some(c) = local_day(&stored, tz).and_then(|d| days.get_mut(&d)) {
            f(c);
        }
    };

    let mut stmt =
        conn.prepare("SELECT ended_at FROM pomodoro_log WHERE ended_at >= ?1 AND ended_at < ?2")?;
    for t in stmt.query_map([&start, &end], |r| r.get::<_, String>(0))? {
        bump(t?, |c| c.pomodoros += 1);
    }
    let mut stmt = conn.prepare(
        "SELECT created_at, kind FROM task
         WHERE kind IN ('interruption', 'unplanned') AND created_at >= ?1 AND created_at < ?2",
    )?;
    for row in stmt.query_map([&start, &end], |r| {
        Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
    })? {
        let (t, kind) = row?;
        if kind == "interruption" {
            bump(t, |c| c.interruptions += 1);
        } else {
            bump(t, |c| c.unplanned += 1);
        }
    }
    let mut stmt = conn.prepare(
        "SELECT closed_at FROM task
         WHERE done = 1 AND kind IN ('pomodoro', 'unplanned') AND closed_at >= ?1 AND closed_at < ?2",
    )?;
    for t in stmt.query_map([&start, &end], |r| r.get::<_, String>(0))? {
        bump(t?, |c| c.tasks_done += 1);
    }
    Ok(days.into_values().collect())
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct WeekFactor {
    /// ISO week, e.g. "2026-W43".
    pub week: String,
    /// Monday of that week.
    pub week_start: NaiveDate,
    pub estimated: i64,
    pub pomodoros: i64,
    /// Pomodoros used per pomodoro estimated (1.0 = estimates were exact).
    pub factor: Option<f64>,
    pub delta: i64,
    pub interruptions: i64,
}

/// "Reality factors" view: estimated vs used pomodoros of estimated tasks, by the
/// ISO week the task was created (as in the original).
pub fn reality_by_week<Tz: TimeZone>(conn: &Connection, tz: &Tz) -> Result<Vec<WeekFactor>> {
    let mut stmt = conn.prepare(
        "SELECT created_at, estimated, pomodoros, unplanned + interruptions FROM task
         WHERE kind IN ('pomodoro', 'unplanned') AND estimated > 0",
    )?;
    let mut weeks: BTreeMap<(i32, u32), (i64, i64, i64)> = BTreeMap::new();
    for row in stmt.query_map([], |r| {
        Ok((
            r.get::<_, String>(0)?,
            r.get::<_, i64>(1)?,
            r.get::<_, i64>(2)?,
            r.get::<_, i64>(3)?,
        ))
    })? {
        let (created, estimated, pomodoros, interruptions) = row?;
        let Some(day) = local_day(&created, tz) else {
            continue;
        };
        let w = day.iso_week();
        let e = weeks.entry((w.year(), w.week())).or_default();
        e.0 += estimated;
        e.1 += pomodoros;
        e.2 += interruptions;
    }
    Ok(weeks
        .into_iter()
        .map(
            |((year, week), (estimated, pomodoros, interruptions))| WeekFactor {
                week: format!("{year}-W{week:02}"),
                week_start: NaiveDate::from_isoywd_opt(year, week, chrono::Weekday::Mon)
                    .expect("valid ISO week"),
                estimated,
                pomodoros,
                factor: (estimated > 0)
                    .then(|| ((pomodoros as f64 / estimated as f64) * 100.0).round() / 100.0),
                delta: pomodoros - estimated,
                interruptions,
            },
        )
        .collect())
}

/// Hashtags used in task names and in interruption names, for the filter suggestions.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct Hashtags {
    pub tasks: Vec<String>,
    pub disturbances: Vec<String>,
}

pub fn hashtags(conn: &Connection) -> Result<Hashtags> {
    let mut out = Hashtags::default();
    let mut stmt = conn.prepare("SELECT name, kind FROM task WHERE name LIKE '%#%'")?;
    let (mut tasks, mut disturbances) = (BTreeSet::new(), BTreeSet::new());
    for row in stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))? {
        let (name, kind) = row?;
        for tag in extract_hashtags(&name) {
            // Unplanned items count as both, as in the original.
            if kind != "interruption" {
                tasks.insert(tag.clone());
            }
            if kind != "pomodoro" {
                disturbances.insert(tag);
            }
        }
    }
    out.tasks = sorted_ci(tasks);
    out.disturbances = sorted_ci(disturbances);
    Ok(out)
}

fn sorted_ci(set: BTreeSet<String>) -> Vec<String> {
    let mut v: Vec<String> = set.into_iter().collect();
    v.sort_by_key(|s| s.to_lowercase());
    v.dedup_by(|a, b| a.eq_ignore_ascii_case(b));
    v
}

/// `#word` tokens (letters, digits, `_`, `-`) in `text`, including the `#`.
pub fn extract_hashtags(text: &str) -> Vec<String> {
    let mut tags = Vec::new();
    let mut chars = text.char_indices().peekable();
    while let Some((i, c)) = chars.next() {
        if c != '#' {
            continue;
        }
        let mut end = i + 1;
        while let Some(&(j, n)) = chars.peek() {
            if n.is_alphanumeric() || n == '_' || n == '-' {
                end = j + n.len_utf8();
                chars.next();
            } else {
                break;
            }
        }
        if end > i + 1 {
            tags.push(text[i..end].to_string());
        }
    }
    tags
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db;
    use crate::tasks::{self, TaskKind};
    use chrono::FixedOffset;

    /// UTC+2, so local days differ from UTC days near midnight.
    fn tz() -> FixedOffset {
        FixedOffset::east_opt(2 * 3600).unwrap()
    }

    fn at(s: &str) -> DateTime<Utc> {
        DateTime::parse_from_rfc3339(s).unwrap().with_timezone(&Utc)
    }

    fn day(s: &str) -> NaiveDate {
        s.parse().unwrap()
    }

    fn range(from: &str, to: &str) -> DateRange {
        DateRange {
            from: day(from),
            to: day(to),
        }
    }

    #[test]
    fn completed_tasks_use_local_days_and_filter() {
        let conn = db::open_in_memory().unwrap();
        let a = tasks::create(
            &conn,
            "Write docs #docs",
            Some(2),
            at("2026-10-19T08:00:00Z"),
        )
        .unwrap();
        let b = tasks::create(&conn, "Fix 100%_bug", None, at("2026-10-19T08:00:00Z")).unwrap();
        let c = tasks::create(&conn, "Open", Some(1), at("2026-10-19T08:00:00Z")).unwrap();
        // 23:30 UTC on the 19th is 01:30 on the 20th in UTC+2.
        tasks::set_done(&conn, a.id, true, at("2026-10-19T23:30:00Z")).unwrap();
        tasks::set_done(&conn, b.id, true, at("2026-10-19T12:00:00Z")).unwrap();
        let _ = c;

        let on_20th = completed_tasks(&conn, range("2026-10-20", "2026-10-20"), "", &tz()).unwrap();
        assert_eq!(on_20th.len(), 1);
        assert_eq!(
            (on_20th[0].name.as_str(), on_20th[0].delta),
            ("Write docs #docs", Some(-2))
        );

        let both = completed_tasks(&conn, range("2026-10-19", "2026-10-20"), "", &tz()).unwrap();
        assert_eq!(both.len(), 2, "open tasks are excluded");
        assert_eq!(both[0].delta, None, "no estimate, no delta");

        let filtered =
            completed_tasks(&conn, range("2026-10-19", "2026-10-20"), "#DOCS", &tz()).unwrap();
        assert_eq!(filtered.len(), 1, "filter is case-insensitive");
        // % and _ in the filter are literal, not wildcards.
        assert_eq!(
            completed_tasks(&conn, range("2026-10-19", "2026-10-20"), "0%_", &tz())
                .unwrap()
                .len(),
            1
        );
        assert_eq!(
            completed_tasks(&conn, range("2026-10-19", "2026-10-20"), "x%", &tz())
                .unwrap()
                .len(),
            0
        );
    }

    #[test]
    fn rejects_inverted_or_huge_ranges() {
        let conn = db::open_in_memory().unwrap();
        assert!(completed_tasks(&conn, range("2026-10-20", "2026-10-19"), "", &tz()).is_err());
        assert!(per_day(&conn, range("2025-01-01", "2026-10-20"), &tz()).is_err());
    }

    #[test]
    fn disturbances_name_their_task() {
        let mut conn = db::open_in_memory().unwrap();
        let t = tasks::create(&conn, "Main", Some(2), at("2026-10-20T08:00:00Z")).unwrap();
        tasks::log_disturbance(
            &mut conn,
            t.id,
            TaskKind::Interruption,
            "Phone #calls",
            None,
            at("2026-10-20T09:00:00Z"),
        )
        .unwrap();
        tasks::log_disturbance(
            &mut conn,
            t.id,
            TaskKind::Unplanned,
            "Hotfix",
            Some(1),
            at("2026-10-20T10:00:00Z"),
        )
        .unwrap();
        tasks::log_disturbance(
            &mut conn,
            t.id,
            TaskKind::Interruption,
            "Next day",
            None,
            at("2026-10-21T10:00:00Z"),
        )
        .unwrap();
        let d = disturbances(&conn, range("2026-10-20", "2026-10-20"), "", &tz()).unwrap();
        assert_eq!(
            d.iter().map(|d| d.name.as_str()).collect::<Vec<_>>(),
            ["Phone #calls", "Hotfix"]
        );
        assert_eq!(d[0].task.as_deref(), Some("Main"));
        tasks::delete(&conn, t.id).unwrap();
        let d = disturbances(&conn, range("2026-10-20", "2026-10-20"), "phone", &tz()).unwrap();
        assert_eq!(d[0].task, None, "parent deleted");
    }

    #[test]
    fn per_day_counts_and_fills_empty_days() {
        let mut conn = db::open_in_memory().unwrap();
        let t = tasks::create(&conn, "A", Some(3), at("2026-10-18T08:00:00Z")).unwrap();
        tasks::record_pomodoro(
            &mut conn,
            Some(t.id),
            at("2026-10-18T08:00:00Z"),
            at("2026-10-18T08:25:00Z"),
        )
        .unwrap();
        tasks::record_pomodoro(
            &mut conn,
            Some(t.id),
            at("2026-10-18T09:00:00Z"),
            at("2026-10-18T09:25:00Z"),
        )
        .unwrap();
        // 22:30 UTC on the 18th is the 19th locally.
        tasks::record_pomodoro(
            &mut conn,
            None,
            at("2026-10-18T22:05:00Z"),
            at("2026-10-18T22:30:00Z"),
        )
        .unwrap();
        tasks::log_disturbance(
            &mut conn,
            t.id,
            TaskKind::Interruption,
            "Call",
            None,
            at("2026-10-18T09:10:00Z"),
        )
        .unwrap();
        tasks::log_disturbance(
            &mut conn,
            t.id,
            TaskKind::Unplanned,
            "U",
            None,
            at("2026-10-18T09:12:00Z"),
        )
        .unwrap();
        tasks::set_done(&conn, t.id, true, at("2026-10-20T10:00:00Z")).unwrap();

        let rows = per_day(&conn, range("2026-10-17", "2026-10-20"), &tz()).unwrap();
        let summary: Vec<_> = rows
            .iter()
            .map(|r| {
                (
                    r.day.day(),
                    r.pomodoros,
                    r.interruptions,
                    r.unplanned,
                    r.tasks_done,
                )
            })
            .collect();
        assert_eq!(
            summary,
            [
                (17, 0, 0, 0, 0),
                (18, 2, 1, 1, 0),
                (19, 1, 0, 0, 0),
                (20, 0, 0, 0, 1)
            ]
        );
    }

    #[test]
    fn reality_factor_by_iso_week() {
        let mut conn = db::open_in_memory().unwrap();
        // Week 43 of 2026 starts Monday 2026-10-19.
        let a = tasks::create(&conn, "A", Some(2), at("2026-10-19T08:00:00Z")).unwrap();
        let b = tasks::create(&conn, "B", Some(2), at("2026-10-21T08:00:00Z")).unwrap();
        tasks::create(&conn, "Unestimated", None, at("2026-10-21T08:00:00Z")).unwrap();
        let c = tasks::create(&conn, "C", Some(1), at("2026-10-26T08:00:00Z")).unwrap();
        for (id, n) in [(a.id, 3), (b.id, 3), (c.id, 1)] {
            for _ in 0..n {
                tasks::record_pomodoro(
                    &mut conn,
                    Some(id),
                    at("2026-10-21T09:00:00Z"),
                    at("2026-10-21T09:25:00Z"),
                )
                .unwrap();
            }
        }
        tasks::log_disturbance(
            &mut conn,
            a.id,
            TaskKind::Interruption,
            "x",
            None,
            at("2026-10-19T09:00:00Z"),
        )
        .unwrap();
        let weeks = reality_by_week(&conn, &tz()).unwrap();
        assert_eq!(weeks.len(), 2);
        assert_eq!(weeks[0].week, "2026-W43");
        assert_eq!(weeks[0].week_start, day("2026-10-19"));
        assert_eq!(
            (weeks[0].estimated, weeks[0].pomodoros, weeks[0].delta),
            (4, 6, 2)
        );
        assert_eq!(weeks[0].factor, Some(1.5));
        assert_eq!(weeks[0].interruptions, 1);
        assert_eq!(
            (weeks[1].week.as_str(), weeks[1].factor),
            ("2026-W44", Some(1.0))
        );
    }

    #[test]
    fn hashtag_extraction_and_lists() {
        assert_eq!(
            extract_hashtags("Fix #123 in #ui-kit, not # or #"),
            ["#123", "#ui-kit"]
        );
        assert_eq!(extract_hashtags("#größe works"), ["#größe"]);
        let mut conn = db::open_in_memory().unwrap();
        let t = tasks::create(&conn, "Docs #Docs #api", None, at("2026-10-20T08:00:00Z")).unwrap();
        tasks::create(&conn, "More #docs", None, at("2026-10-20T08:00:00Z")).unwrap();
        tasks::log_disturbance(
            &mut conn,
            t.id,
            TaskKind::Interruption,
            "Call #phone",
            None,
            at("2026-10-20T08:00:00Z"),
        )
        .unwrap();
        tasks::log_disturbance(
            &mut conn,
            t.id,
            TaskKind::Unplanned,
            "Hotfix #prod",
            None,
            at("2026-10-20T08:00:00Z"),
        )
        .unwrap();
        let h = hashtags(&conn).unwrap();
        assert_eq!(h.tasks, ["#api", "#Docs", "#prod"]);
        assert_eq!(h.disturbances, ["#phone", "#prod"]);
    }
}
