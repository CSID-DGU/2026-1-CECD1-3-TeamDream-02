//! Ingest server for the CSE boards: signed-in boards POST their NDJSON records, the server stores them in SQLite
//! and answers 200 only after the commit is on disk. TLS is the reverse proxy's job (see README).
//!
//! `cse-server` serves; `cse-server healthcheck` is the container HEALTHCHECK (no curl in the image).
mod auth;
mod store;

use std::io::{Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use axum::body::Bytes;
use axum::extract::{DefaultBodyLimit, Query, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use rusqlite::Connection;
use serde::Deserialize;
use serde_json::json;

use auth::{Role, Tokens};
use store::IngestError;

struct Config {
    listen: SocketAddr,
    db: PathBuf,
    tokens: PathBuf,
    max_body: usize,
}

impl Config {
    fn from_env() -> Result<Self, String> {
        let var = |k: &str, d: &str| std::env::var(k).unwrap_or_else(|_| d.to_owned());
        Ok(Self {
            listen: var("CSE_LISTEN", "0.0.0.0:8080").parse().map_err(|e| format!("CSE_LISTEN: {e}"))?,
            db: var("CSE_DB", "/data/cse.db").into(),
            tokens: var("CSE_TOKENS", "/run/secrets/cse_tokens").into(),
            max_body: var("CSE_MAX_BODY", "4194304").parse().map_err(|e| format!("CSE_MAX_BODY: {e}"))?,
        })
    }
}

struct App {
    db: Mutex<Connection>,
    tokens: Tokens,
}

type AppState = State<Arc<App>>;

fn now_ms() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_millis() as i64)
}

fn err(status: StatusCode, msg: impl Into<String>) -> Response {
    (status, Json(json!({ "error": msg.into() }))).into_response()
}

fn caller<'a>(app: &'a App, headers: &HeaderMap, want: Role) -> Result<&'a str, Response> {
    let auth = headers.get(header::AUTHORIZATION).and_then(|v| v.to_str().ok());
    match app.tokens.check(auth) {
        Some((name, role)) if role == want => Ok(name),
        Some(_) => Err(err(StatusCode::FORBIDDEN, "token not allowed here")),
        None => Err(err(StatusCode::UNAUTHORIZED, "missing or unknown bearer token")),
    }
}

/// Runs a closure on the SQLite connection off the async runtime.
async fn with_db<T: Send + 'static>(
    app: &Arc<App>,
    f: impl FnOnce(&mut Connection) -> T + Send + 'static,
) -> Result<T, Response> {
    let app = app.clone();
    tokio::task::spawn_blocking(move || f(&mut app.db.lock().unwrap_or_else(|p| p.into_inner())))
        .await
        .map_err(|_| err(StatusCode::INTERNAL_SERVER_ERROR, "database task failed"))
}

async fn healthz(State(app): AppState) -> Response {
    match with_db(&app, |c| c.query_row("SELECT 1", [], |_| Ok(()))).await {
        Ok(Ok(())) => "ok\n".into_response(),
        _ => err(StatusCode::SERVICE_UNAVAILABLE, "database unavailable"),
    }
}

/// POST /v1/records, body = NDJSON lines exactly as the board stored them.
async fn post_records(State(app): AppState, headers: HeaderMap, body: Bytes) -> Response {
    let board = match caller(&app, &headers, Role::Board) {
        Ok(b) => b.to_owned(),
        Err(r) => return r,
    };
    let Ok(text) = String::from_utf8(body.to_vec()) else {
        return err(StatusCode::BAD_REQUEST, "body is not UTF-8");
    };
    let b = board.clone();
    let res = match with_db(&app, move |c| store::ingest(c, &b, &text, now_ms())).await {
        Ok(r) => r,
        Err(r) => return r,
    };
    match res {
        Ok(done) => {
            eprintln!("{board}: +{} new, {} dup, last seq {}", done.inserted, done.duplicates, done.last_seq);
            Json(done).into_response()
        }
        Err(IngestError::Empty) => err(StatusCode::BAD_REQUEST, "no records in body"),
        Err(IngestError::BadLine { line, reason }) => {
            eprintln!("{board}: rejected batch, line {line}: {reason}");
            err(StatusCode::UNPROCESSABLE_ENTITY, format!("line {line}: {reason}"))
        }
        Err(IngestError::Db(e)) => {
            eprintln!("{board}: database error: {e}");
            err(StatusCode::SERVICE_UNAVAILABLE, "database error, retry later")
        }
    }
}

#[derive(Deserialize)]
struct RecordsQuery {
    board: Option<String>,
    event: Option<String>,
    from_ms: Option<i64>,
    to_ms: Option<i64>,
    #[serde(default)]
    after: i64,
    limit: Option<i64>,
}

/// GET /v1/records?board=&event=&from_ms=&to_ms=&after=&limit=: NDJSON
/// `{"id","board","event","received_ms","t_ms","t_source","record"}` in arrival order; t_ms is the measurement
/// time (see store.rs for how unsynced records are dated) and what from_ms/to_ms select on.
/// Page with after = the last id seen (also sent as X-Last-Id).
async fn get_records(State(app): AppState, headers: HeaderMap, Query(q): Query<RecordsQuery>) -> Response {
    if let Err(r) = caller(&app, &headers, Role::Reader) {
        return r;
    }
    let limit = q.limit.unwrap_or(1000).clamp(1, 10_000);
    let after = q.after;
    let rows = match with_db(&app, move |c| {
        let f = store::Filter { board: q.board.as_deref(), event: q.event.as_deref(), from_ms: q.from_ms, to_ms: q.to_ms };
        store::records(c, &f, after, limit)
    })
    .await
    {
        Ok(Ok(rows)) => rows,
        _ => return err(StatusCode::SERVICE_UNAVAILABLE, "database error"),
    };
    let mut out = String::new();
    for r in &rows {
        // body was validated as JSON on ingest, so it is embedded as is
        out.push_str(&format!(
            "{{\"id\":{},\"board\":{},\"event\":{},\"received_ms\":{},\"t_ms\":{},\"t_source\":{},\"record\":{}}}\n",
            r.id,
            json!(r.board),
            json!(r.event),
            r.received_ms,
            json!(r.t_ms),
            json!(r.t_source),
            r.body
        ));
    }
    let last = rows.last().map_or(after, |r| r.id);
    ([(header::CONTENT_TYPE, "application/x-ndjson".to_owned()), ("x-last-id".parse().unwrap(), last.to_string())], out)
        .into_response()
}

/// GET /v1/boards: record count, last arrival and last record per board.
async fn get_boards(State(app): AppState, headers: HeaderMap) -> Response {
    if let Err(r) = caller(&app, &headers, Role::Reader) {
        return r;
    }
    match with_db(&app, |c| store::boards(c)).await {
        Ok(Ok(b)) => Json(b).into_response(),
        _ => err(StatusCode::SERVICE_UNAVAILABLE, "database error"),
    }
}

async fn shutdown() {
    let mut term = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()).expect("SIGTERM handler");
    tokio::select! {
        _ = term.recv() => {}
        _ = tokio::signal::ctrl_c() => {}
    }
    eprintln!("shutting down");
}

async fn serve(cfg: Config) -> Result<(), String> {
    let tokens_text =
        std::fs::read_to_string(&cfg.tokens).map_err(|e| format!("tokens file {}: {e}", cfg.tokens.display()))?;
    let tokens = Tokens::parse(&tokens_text)?;
    let db = store::open(&cfg.db).map_err(|e| format!("database {}: {e}", cfg.db.display()))?;
    eprintln!("cse-server {} on {}, db {}, {} tokens", env!("CARGO_PKG_VERSION"), cfg.listen, cfg.db.display(), tokens.len());

    let app = Arc::new(App { db: Mutex::new(db), tokens });
    let router = Router::new()
        .route("/healthz", get(healthz))
        .route("/v1/records", post(post_records).get(get_records))
        .route("/v1/boards", get(get_boards))
        .layer(DefaultBodyLimit::max(cfg.max_body))
        .with_state(app);
    let listener = tokio::net::TcpListener::bind(cfg.listen).await.map_err(|e| format!("bind {}: {e}", cfg.listen))?;
    axum::serve(listener, router).with_graceful_shutdown(shutdown()).await.map_err(|e| e.to_string())
}

/// Exit 0 when GET /healthz on the local listener answers 200.
fn healthcheck(cfg: &Config) -> bool {
    let addr = SocketAddr::new([127, 0, 0, 1].into(), cfg.listen.port());
    let Ok(mut s) = TcpStream::connect_timeout(&addr, Duration::from_secs(3)) else { return false };
    let _ = s.set_read_timeout(Some(Duration::from_secs(3)));
    let mut buf = [0u8; 16];
    s.write_all(b"GET /healthz HTTP/1.0\r\nHost: localhost\r\n\r\n").is_ok()
        && s.read(&mut buf).is_ok_and(|n| buf[..n].starts_with(b"HTTP/1.") && buf[..n].get(9..12) == Some(b"200"))
}

#[tokio::main]
async fn main() {
    let cfg = match Config::from_env() {
        Ok(c) => c,
        Err(e) => {
            eprintln!("cse-server: {e}");
            std::process::exit(2);
        }
    };
    match std::env::args().nth(1).as_deref() {
        None | Some("serve") => {
            if let Err(e) = serve(cfg).await {
                eprintln!("cse-server: {e}");
                std::process::exit(1);
            }
        }
        Some("healthcheck") => std::process::exit(if healthcheck(&cfg) { 0 } else { 1 }),
        Some(other) => {
            eprintln!("usage: cse-server [serve|healthcheck] (got {other})");
            std::process::exit(2);
        }
    }
}
