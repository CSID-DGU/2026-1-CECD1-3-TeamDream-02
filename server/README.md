# CSE ingest server

Receives the boards' records and stores them in SQLite. The boards keep every record on their SD card (fsync per
record) and upload them when the uplink is up; the server answers 200 only after the batch is committed to disk,
and ignores records it already has, so a board can resend after a power cut or a dropped connection without
creating duplicates or gaps.

## API

All endpoints except `/healthz` need `Authorization: Bearer <token>`.

| Request | Who | Answer |
|---|---|---|
| `POST /v1/records`, body = NDJSON, the record lines exactly as the board stored them | board token | `{"inserted":N,"duplicates":M,"last_seq":S}` |
| `GET /v1/records?board=&event=&from_ms=&to_ms=&after=<id>&limit=<n>` | reader token | NDJSON `{"id","board","event","received_ms","t_ms","t_source","record"}` in arrival order, header `X-Last-Id` |
| `GET /v1/boards` | reader token | per board: record count, last arrival, last record |
| `GET /healthz` | anyone | `ok` |

- Each record needs `seq`, `boot_id`, `t_unix_ms` (the loggers always write them). The board name comes from the
  token, not the record.
- Duplicates are detected on `(board, boot_id, seq)`. seq restarts at 0 only after a reflash, which also gives a
  new `boot_id`.
- One bad line rejects the whole batch (422, `{"error":"line 3: ..."}`), so a board never moves its upload cursor
  past a record the server did not keep. 401: unknown token, 403: wrong role, 503: database problem, retry.
- Batches up to 4 MiB (`CSE_MAX_BODY`).
- Events: every record has an `event` saying why it was written; a record without one (firmware before
  fw-v0.2.0) counts as `sample`.
  - `sample`: periodic measurement, every 60 s on both boards, so radon and servo records line up by time.
  - `led_change` (servo board): an indicator light switched (normally one of the two is on: the lit one is the
    cell reading clearly lower than the other); `record.led_before` / `record.led_on` are the states
    before and after, the time is the reading that first showed the new state (about 1 s resolution, confirmed by
    the next two readings).
  - `servo` (servo board): a button press; `record.press` has the servo, the positions, the light-cell voltages
    before the motion and the lowest 5 V rail during it; the record's own `ldr*_v` are from after the motion.
  `GET /v1/records?event=servo` etc. selects one kind; `from_ms` / `to_ms` select on the measurement time `t_ms`
  (from inclusive, to exclusive; records without a known time never match).
- Measurement time: the boards have no RTC, so a record written before NTP sync has `"ntp_synced":false` and a
  wrong `t_unix_ms`. Use `t_ms` from `GET /v1/records` instead; `t_source` says where it comes from:
  - `ntp`: the board's synced clock (`t_unix_ms`).
  - `boot`: computed. The first synced record of the same boot gives the boot's start time
    (`t_unix_ms - uptime_s`); start + the record's own `uptime_s` is its time. It appears as soon as a synced
    record of that boot has been uploaded, also for records uploaded earlier.
  - `none` (`t_ms` null): that boot never got NTP (or the record has no `uptime_s`). `record.seq` still gives the
    order, between the boots before and after it.
- Paging: start with `after=0`, then pass the `X-Last-Id` of the previous page.

## Build and move to another server

```
docker build -t cse-server:0.2.0 server/          # or: podman build --format docker ...
docker save cse-server:0.2.0 | zstd -19 > cse-server-0.2.0.tar.zst
# on the server
zstd -dc cse-server-0.2.0.tar.zst | docker load
```

The image is built for the architecture of the build machine (amd64 here). For an arm64 server use
`docker buildx build --platform linux/arm64 ...` or build on the server itself.

## Run

```
mkdir -p cse && cd cse            # compose.yaml from this directory
mkdir data && sudo chown 10001:10001 data
# one token per board plus one for reading; only the sha256 goes into the tokens file
T=$(openssl rand -hex 32); echo "radon token: $T"; printf %s "$T" | sha256sum
cp tokens.example tokens          # put the hashes in, see the format in the file
docker compose up -d
docker compose logs -f
```

- The container listens on plain HTTP and `compose.yaml` publishes port 8080; HAProxy terminates TLS for radon.kafu.moe and forwards to it. Do not expose 8080 to the internet itself (tokens would travel in clear): firewall it to HAProxy.
- Data: `data/cse.db` (+ `-wal`, `-shm`). Back up with the container stopped, or `sqlite3 data/cse.db ".backup x.db"`.
- Settings (env): `CSE_LISTEN` (0.0.0.0:8080), `CSE_DB` (/data/cse.db), `CSE_TOKENS` (/run/secrets/cse_tokens),
  `CSE_MAX_BODY` (bytes).
- Changing tokens: edit `tokens`, then `docker compose restart`.
- Updating: build or load the new image, set its tag in `compose.yaml`, `docker compose up -d`. The database is
  upgraded in place at start (0.1.1 adds the boot times, 0.2.0 the events; existing records are kept).

## Releases and automatic update

Pushing a tag `server-vX.Y.Z` (same version as `server/Cargo.toml`) makes `.github/workflows/server-release.yml`
build the image and publish it as a GitHub release asset (`cse-server-X.Y.Z.tar.zst` + `.sha256`, linux/amd64).
These releases are not marked "latest": the boards take their firmware from `releases/latest`, which must stay a
`fw-v*` release.

On the server, `update.sh` (next to `compose.yaml`) looks for the newest `server-v*` release; if it is not the
version `compose.yaml` names, it downloads the image, checks the sha256, loads it, switches `compose.yaml` to it
and restarts the container. If the new container is not healthy within a minute it goes back to the previous
image. Run it from cron for unattended updates:

```
17 * * * *  /home/me/cse/update.sh >> /home/me/cse/update.log 2>&1
```

The download is protected by HTTPS and the checksum from the same release, not by a signature of its own:
whoever can publish a release in this repository can change what the server runs.

## Develop

`cargo test` in `server/` (store and auth tests); the image build runs them too.
