//! SQLite store: one row per board record, deduplicated on (board, boot_id, seq).
//! seq restarts at 0 only when a card is reflashed, and boot_id changes on every boot, so a resent record
//! always hits its earlier row and a new record never does.
//!
//! Time: the boards have no RTC, so records written before NTP sync carry a wrong t_unix_ms (ntp_synced false).
//! Every record also has uptime_s; the first synced record of a boot gives that boot's start time
//! (t_unix_ms - uptime), and boot start + uptime dates the unsynced records of the same boot.
use std::path::Path;
use std::time::Duration;

use rusqlite::{Connection, params};
use serde::Serialize;
use serde_json::Value;

const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS records (
    id          INTEGER PRIMARY KEY,
    board       TEXT    NOT NULL,
    boot_id     TEXT    NOT NULL,
    seq         INTEGER NOT NULL,
    t_unix_ms   INTEGER NOT NULL,
    ntp_synced  INTEGER NOT NULL,
    received_ms INTEGER NOT NULL,
    body        TEXT    NOT NULL,
    uptime_ms   INTEGER,
    event       TEXT    NOT NULL DEFAULT 'sample',
    UNIQUE (board, boot_id, seq)
);
CREATE INDEX IF NOT EXISTS records_board_t ON records (board, t_unix_ms);
-- boot start time, from the earliest NTP-synced record of that boot (anchor_seq)
CREATE TABLE IF NOT EXISTS boots (
    board        TEXT    NOT NULL,
    boot_id      TEXT    NOT NULL,
    boot_unix_ms INTEGER NOT NULL,
    anchor_seq   INTEGER NOT NULL,
    PRIMARY KEY (board, boot_id)
);
";

/// Brings an older database up to date; the columns are filled from the stored bodies.
/// 1 (0.1.1): uptime_ms and the boots table. 2 (0.2.0): event.
fn migrate(conn: &mut Connection) -> rusqlite::Result<()> {
    let version: i64 = conn.pragma_query_value(None, "user_version", |r| r.get(0))?;
    if version >= 2 {
        return Ok(());
    }
    let tx = conn.transaction()?;
    let has = |name: &str| -> rusqlite::Result<bool> {
        tx.query_row("SELECT COUNT(*) FROM pragma_table_info('records') WHERE name = ?1", [name], |r| r.get(0))
    };
    if !has("uptime_ms")? {
        tx.execute_batch("ALTER TABLE records ADD COLUMN uptime_ms INTEGER")?;
    }
    if !has("event")? {
        tx.execute_batch(
            "ALTER TABLE records ADD COLUMN event TEXT NOT NULL DEFAULT 'sample';
             UPDATE records SET event = json_extract(body, '$.event') WHERE json_type(body, '$.event') = 'text';",
        )?;
    }
    tx.execute_batch(
        "UPDATE records SET uptime_ms = CAST(json_extract(body, '$.uptime_s') * 1000 AS INTEGER)
         WHERE uptime_ms IS NULL AND json_type(body, '$.uptime_s') IN ('real', 'integer');
         INSERT OR REPLACE INTO boots (board, boot_id, boot_unix_ms, anchor_seq)
         SELECT board, boot_id, t_unix_ms - uptime_ms, seq FROM records r
         WHERE ntp_synced AND uptime_ms IS NOT NULL AND seq = (
             SELECT MIN(seq) FROM records
             WHERE board = r.board AND boot_id = r.boot_id AND ntp_synced AND uptime_ms IS NOT NULL);
         CREATE INDEX IF NOT EXISTS records_board_event ON records (board, event, id);
         PRAGMA user_version = 2;",
    )?;
    tx.commit()
}

pub fn open(path: &Path) -> rusqlite::Result<Connection> {
    let mut conn = Connection::open(path)?;
    conn.pragma_update(None, "journal_mode", "WAL")?;
    // FULL: the commit is on disk before the board gets its 200 and moves its upload cursor past these records
    conn.pragma_update(None, "synchronous", "FULL")?;
    conn.busy_timeout(Duration::from_secs(5))?;
    init(&mut conn)?;
    Ok(conn)
}

fn init(conn: &mut Connection) -> rusqlite::Result<()> {
    // an existing 0.1.0 table is left as is by CREATE TABLE IF NOT EXISTS; migrate() brings it up to date
    conn.execute_batch(SCHEMA)?;
    migrate(conn)
}

#[derive(Debug, PartialEq, Serialize)]
pub struct Ingested {
    pub inserted: usize,
    pub duplicates: usize,
    pub last_seq: u64,
}

#[derive(Debug)]
pub enum IngestError {
    Empty,
    BadLine { line: usize, reason: &'static str },
    Db(rusqlite::Error),
}

impl From<rusqlite::Error> for IngestError {
    fn from(e: rusqlite::Error) -> Self {
        Self::Db(e)
    }
}

struct Rec<'a> {
    boot_id: &'a str,
    seq: u64,
    t_unix_ms: i64,
    ntp_synced: bool,
    uptime_ms: Option<i64>,
    /// "sample" when the record has none (firmware before fw-v0.2.0)
    event: &'a str,
    body: &'a str,
}

fn parse<'a>(line: &'a str, v: &'a Value) -> Result<Rec<'a>, &'static str> {
    let o = v.as_object().ok_or("not a JSON object")?;
    let seq = o
        .get("seq")
        .and_then(Value::as_u64)
        .filter(|&s| s <= i64::MAX as u64)
        .ok_or("seq missing or not an unsigned integer")?;
    let boot_id = o
        .get("boot_id")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty() && s.len() <= 64)
        .ok_or("boot_id missing")?;
    let t_unix_ms = o.get("t_unix_ms").and_then(Value::as_i64).ok_or("t_unix_ms missing")?;
    let ntp_synced = o.get("ntp_synced").and_then(Value::as_bool).unwrap_or(false);
    let uptime_ms = o.get("uptime_s").and_then(Value::as_f64).filter(|u| *u >= 0.0).map(|u| (u * 1000.0) as i64);
    let event = match o.get("event") {
        None => "sample",
        Some(e) => e.as_str().filter(|e| !e.is_empty() && e.len() <= 32).ok_or("event must be a short string")?,
    };
    Ok(Rec { boot_id, seq, t_unix_ms, ntp_synced, uptime_ms, event, body: line })
}

/// Stores an NDJSON batch in one transaction. A bad line rejects the whole batch, so the board never moves its
/// cursor past a record the server did not keep.
pub fn ingest(conn: &mut Connection, board: &str, body: &str, now_ms: i64) -> Result<Ingested, IngestError> {
    let lines: Vec<(usize, &str)> =
        body.lines().map(str::trim).enumerate().filter(|(_, l)| !l.is_empty()).map(|(i, l)| (i + 1, l)).collect();
    if lines.is_empty() {
        return Err(IngestError::Empty);
    }
    let values: Vec<(usize, &str, Value)> = lines
        .into_iter()
        .map(|(n, l)| {
            serde_json::from_str(l).map(|v| (n, l, v)).map_err(|_| IngestError::BadLine { line: n, reason: "not JSON" })
        })
        .collect::<Result<_, _>>()?;
    let recs: Vec<Rec> = values
        .iter()
        .map(|(n, l, v)| parse(l, v).map_err(|reason| IngestError::BadLine { line: *n, reason }))
        .collect::<Result<_, _>>()?;

    let tx = conn.transaction()?;
    let mut inserted = 0;
    {
        let mut stmt = tx.prepare_cached(
            "INSERT OR IGNORE INTO records (board, boot_id, seq, t_unix_ms, ntp_synced, received_ms, body, uptime_ms, event)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
        )?;
        // keep the earliest synced record as the anchor: closest to the unsynced ones, so least clock drift
        let mut anchor = tx.prepare_cached(
            "INSERT INTO boots (board, boot_id, boot_unix_ms, anchor_seq) VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT (board, boot_id) DO UPDATE
             SET boot_unix_ms = excluded.boot_unix_ms, anchor_seq = excluded.anchor_seq
             WHERE excluded.anchor_seq < boots.anchor_seq",
        )?;
        for r in &recs {
            inserted += stmt.execute(params![
                board, r.boot_id, r.seq as i64, r.t_unix_ms, r.ntp_synced, now_ms, r.body, r.uptime_ms, r.event
            ])?;
            if let (true, Some(up)) = (r.ntp_synced, r.uptime_ms) {
                anchor.execute(params![board, r.boot_id, r.t_unix_ms - up, r.seq as i64])?;
            }
        }
    }
    tx.commit()?;
    Ok(Ingested {
        inserted,
        duplicates: recs.len() - inserted,
        last_seq: recs.iter().map(|r| r.seq).max().unwrap_or(0),
    })
}

pub struct Row {
    pub id: i64,
    pub board: String,
    pub received_ms: i64,
    pub body: String,
    pub event: String,
    /// best known time of the measurement, None when the boot never synced
    pub t_ms: Option<i64>,
    /// "ntp": the board's synced clock, "boot": boot start + uptime, "none"
    pub t_source: String,
}

/// What GET /v1/records can narrow down; every field is optional.
#[derive(Default)]
pub struct Filter<'a> {
    pub board: Option<&'a str>,
    pub event: Option<&'a str>,
    /// measurement time (t_ms) range, from inclusive, to exclusive; records without a known time never match
    pub from_ms: Option<i64>,
    pub to_ms: Option<i64>,
}

/// Rows with id > after in id order (= arrival order).
pub fn records(conn: &Connection, f: &Filter, after: i64, limit: i64) -> rusqlite::Result<Vec<Row>> {
    let mut stmt = conn.prepare_cached(
        "SELECT * FROM (
             SELECT r.id, r.board, r.received_ms, r.body, r.event,
                    CASE WHEN r.ntp_synced THEN r.t_unix_ms ELSE b.boot_unix_ms + r.uptime_ms END AS t_ms,
                    CASE WHEN r.ntp_synced THEN 'ntp' WHEN b.boot_unix_ms + r.uptime_ms IS NOT NULL THEN 'boot'
                         ELSE 'none' END AS t_source
             FROM records r LEFT JOIN boots b ON b.board = r.board AND b.boot_id = r.boot_id
             WHERE r.id > ?1 AND (?2 IS NULL OR r.board = ?2) AND (?3 IS NULL OR r.event = ?3))
         WHERE (?4 IS NULL OR t_ms >= ?4) AND (?5 IS NULL OR t_ms < ?5) ORDER BY id LIMIT ?6",
    )?;
    stmt.query_map(params![after, f.board, f.event, f.from_ms, f.to_ms, limit], |r| {
        Ok(Row {
            id: r.get(0)?,
            board: r.get(1)?,
            received_ms: r.get(2)?,
            body: r.get(3)?,
            event: r.get(4)?,
            t_ms: r.get(5)?,
            t_source: r.get(6)?,
        })
    })?
    .collect()
}

#[derive(Serialize)]
pub struct BoardSummary {
    pub board: String,
    pub records: i64,
    pub last_received_ms: i64,
    pub last: Value,
}

pub fn boards(conn: &Connection) -> rusqlite::Result<Vec<BoardSummary>> {
    let mut stmt = conn.prepare_cached(
        "SELECT r.board, c.n, r.received_ms, r.body FROM records r
         JOIN (SELECT board, COUNT(*) AS n, MAX(id) AS last_id FROM records GROUP BY board) c ON r.id = c.last_id
         ORDER BY r.board",
    )?;
    stmt.query_map([], |r| {
        let body: String = r.get(3)?;
        Ok(BoardSummary {
            board: r.get(0)?,
            records: r.get(1)?,
            last_received_ms: r.get(2)?,
            last: serde_json::from_str(&body).unwrap_or(Value::Null),
        })
    })?
    .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mem() -> Connection {
        let mut c = Connection::open_in_memory().unwrap();
        init(&mut c).unwrap();
        c
    }

    fn all(c: &Connection) -> Vec<Row> {
        records(c, &Filter::default(), 0, 100).unwrap()
    }

    fn rec(boot: &str, seq: u64) -> String {
        format!(r#"{{"v":1,"seq":{seq},"boot_id":"{boot}","t_unix_ms":{},"ntp_synced":true}}"#, 1_000 + seq)
    }

    #[test]
    fn resend_is_deduplicated() {
        let mut c = mem();
        let batch = format!("{}\n{}\n", rec("a", 0), rec("a", 1));
        assert_eq!(ingest(&mut c, "radon", &batch, 5).unwrap(), Ingested { inserted: 2, duplicates: 0, last_seq: 1 });
        let batch = format!("{}\n{}\n", rec("a", 1), rec("a", 2));
        assert_eq!(ingest(&mut c, "radon", &batch, 6).unwrap(), Ingested { inserted: 1, duplicates: 1, last_seq: 2 });
        // same seq from another board, and seq 0 again after a reflash (new boot_id), are new records
        assert_eq!(ingest(&mut c, "servo", &rec("a", 0), 7).unwrap().inserted, 1);
        assert_eq!(ingest(&mut c, "radon", &rec("b", 0), 8).unwrap().inserted, 1);
        assert_eq!(records(&c, &Filter { board: Some("radon"), ..Default::default() }, 0, 100).unwrap().len(), 4);
        assert_eq!(all(&c).len(), 5);
    }

    #[test]
    fn bad_line_rejects_whole_batch() {
        let mut c = mem();
        let batch = format!("{}\n{{\"seq\":1}}\n", rec("a", 0));
        assert!(matches!(ingest(&mut c, "radon", &batch, 5), Err(IngestError::BadLine { line: 2, .. })));
        let batch = format!("{}\n{{\"seq\":1,\"trunc", rec("a", 0));
        assert!(matches!(ingest(&mut c, "radon", &batch, 5), Err(IngestError::BadLine { line: 2, reason: "not JSON" })));
        assert!(matches!(ingest(&mut c, "radon", "\n\n", 5), Err(IngestError::Empty)));
        assert!(all(&c).is_empty());
        assert!(matches!(ingest(&mut c, "radon", r#"{"seq":0,"boot_id":"a","t_unix_ms":1,"event":7}"#, 5), Err(IngestError::BadLine { .. })));
    }

    fn timed(boot: &str, seq: u64, t: i64, synced: bool, uptime_s: f64) -> String {
        format!(r#"{{"seq":{seq},"boot_id":"{boot}","t_unix_ms":{t},"ntp_synced":{synced},"uptime_s":{uptime_s}}}"#)
    }

    fn times(c: &Connection) -> Vec<(Option<i64>, String)> {
        all(c).into_iter().map(|r| (r.t_ms, r.t_source)).collect()
    }

    #[test]
    fn filters_by_event_and_time() {
        let mut c = mem();
        let ev = |seq: u64, t: i64, event: &str| {
            format!(r#"{{"seq":{seq},"event":"{event}","boot_id":"a","t_unix_ms":{t},"ntp_synced":true,"uptime_s":1}}"#)
        };
        let batch = [ev(0, 1000, "sample"), ev(1, 2000, "led_change"), ev(2, 3000, "servo"), ev(3, 4000, "sample")].join("\n");
        ingest(&mut c, "servo", &batch, 1).unwrap();
        // no event field (firmware before fw-v0.2.0) counts as a sample; an unsynced record has no time to match
        ingest(&mut c, "servo", &timed("a", 4, 5000, true, 5.0), 2).unwrap();
        ingest(&mut c, "servo", &timed("z", 5, 2500, false, 5.0), 3).unwrap();
        let seqs = |f: Filter| -> Vec<i64> {
            records(&c, &f, 0, 100).unwrap().iter().map(|r| serde_json::from_str::<Value>(&r.body).unwrap()["seq"].as_i64().unwrap()).collect()
        };
        assert_eq!(seqs(Filter { event: Some("sample"), ..Default::default() }), vec![0, 3, 4, 5]);
        assert_eq!(seqs(Filter { event: Some("servo"), board: Some("servo"), ..Default::default() }), vec![2]);
        assert_eq!(seqs(Filter { event: Some("servo"), board: Some("radon"), ..Default::default() }), Vec::<i64>::new());
        assert_eq!(seqs(Filter { from_ms: Some(2000), to_ms: Some(4000), ..Default::default() }), vec![1, 2]);
        assert_eq!(seqs(Filter { from_ms: Some(3000), event: Some("sample"), ..Default::default() }), vec![3, 4]);
        assert_eq!(records(&c, &Filter::default(), 0, 100).unwrap()[1].event, "led_change");
    }

    #[test]
    fn unsynced_records_are_dated_from_boot_time() {
        let mut c = mem();
        // boot "a" started at 1_000_000: two records before NTP sync with a bogus clock, uploaded first
        let early = format!("{}\n{}", timed("a", 0, 5, false, 10.0), timed("a", 1, 6, false, 70.0));
        ingest(&mut c, "radon", &early, 1).unwrap();
        assert_eq!(times(&c), vec![(None, "none".into()), (None, "none".into())]);
        // synced records arrive later, latest first: the earliest one (seq 2) must end up as the anchor
        ingest(&mut c, "radon", &timed("a", 3, 1_190_050, true, 190.0), 2).unwrap();
        ingest(&mut c, "radon", &timed("a", 2, 1_130_000, true, 130.0), 3).unwrap();
        // boot "b" never synced, and a record without uptime_s cannot be dated
        ingest(&mut c, "radon", &timed("b", 4, 7, false, 10.0), 4).unwrap();
        ingest(&mut c, "radon", r#"{"seq":5,"boot_id":"a","t_unix_ms":8}"#, 5).unwrap();
        assert_eq!(
            times(&c),
            vec![
                (Some(1_010_000), "boot".into()),
                (Some(1_070_000), "boot".into()),
                (Some(1_190_050), "ntp".into()),
                (Some(1_130_000), "ntp".into()),
                (None, "none".into()),
                (None, "none".into()),
            ]
        );
    }

    #[test]
    fn migrates_a_0_1_0_database() {
        let mut c = Connection::open_in_memory().unwrap();
        c.execute_batch(
            "CREATE TABLE records (id INTEGER PRIMARY KEY, board TEXT NOT NULL, boot_id TEXT NOT NULL,
                 seq INTEGER NOT NULL, t_unix_ms INTEGER NOT NULL, ntp_synced INTEGER NOT NULL,
                 received_ms INTEGER NOT NULL, body TEXT NOT NULL, UNIQUE (board, boot_id, seq));",
        )
        .unwrap();
        for (seq, t, synced, up) in [(0, 5, false, 10.0), (1, 1_130_000, true, 130.0), (2, 1_190_000, true, 190.0)] {
            c.execute(
                "INSERT INTO records (board, boot_id, seq, t_unix_ms, ntp_synced, received_ms, body)
                 VALUES ('radon', 'a', ?1, ?2, ?3, 0, ?4)",
                params![seq, t, synced, timed("a", seq as u64, t, synced, up)],
            )
            .unwrap();
        }
        c.execute(
            "INSERT INTO records (board, boot_id, seq, t_unix_ms, ntp_synced, received_ms, body)
             VALUES ('radon', 'curl-test', 0, 1, 0, 0, '{\"seq\":0,\"boot_id\":\"curl-test\",\"t_unix_ms\":1}')",
            [],
        )
        .unwrap();
        init(&mut c).unwrap();
        init(&mut c).unwrap(); // second start: nothing to do
        assert_eq!(times(&c)[0], (Some(1_010_000), "boot".into()));
        assert_eq!(times(&c)[3], (None, "none".into()));
        assert_eq!(ingest(&mut c, "radon", &timed("a", 3, 1_250_000, true, 250.0), 9).unwrap().inserted, 1);
        assert!(all(&c).iter().all(|r| r.event == "sample"));
    }

    #[test]
    fn boards_shows_latest_record() {
        let mut c = mem();
        ingest(&mut c, "radon", &format!("{}\n{}", rec("a", 0), rec("a", 1)), 5).unwrap();
        let b = boards(&c).unwrap();
        assert_eq!((b.len(), b[0].records, b[0].last["seq"].as_u64()), (1, 2, Some(1)));
    }
}
