# pend – do now, wait later 🕒

`pend` is a **tiny cross-platform job runner** that lets you fire-and-forget
processes *now* and deal with their output *later* – all while keeping logs
tidy, exit statuses intact, and your shell scripts blissfully simple.

```
# Run two jobs in parallel …
pend do backend ./scripts/build_backend.sh
pend do frontend ./scripts/build_frontend.sh

# … wait for them whenever you want (combined coloured output!)
pend wait backend frontend

# Clean up once you are done
pend clean --all
```

Why wrestle with `&`, `wait`, and brittle `tee` pipelines when **one binary**
does all the heavy lifting for you?

---

## 📦  Installation

```bash
cargo install pend      # Rust way – zero dependencies
# or grab a pre-built release asset from GitHub
```

The crate is 100 % Rust, no native libraries, so a static binary drops out on
all tier-1 platforms (Windows / macOS / Linux – x86-64 & aarch64).

---

## 🧠  Mental model

| Command | What it does |
|---------|--------------|
| `pend do <job> <cmd …>` | Launches `<cmd>` detached in the background. Captures its stdout, stderr, exit code, metadata, _and_ a combined `.log` stream. Optional flags `--timeout <secs>` and `--retries <n>` kill or re-run the command automatically. |
| `pend wait <job …>`     | Blocks until the supplied job(s) finish. Streams their output in the original order and exits with the very same code the first failing job produced. |
| `pend clean [--all \| <job …>]` | Deletes artifacts to free disk space. Skips jobs that are still running. |
| `pend tui`              | Opens a super-lightweight TUI that auto-refreshes and shows a live list of all jobs (press `q` to quit). |

That’s the entire user-facing surface – **four deliberately boring verbs**.

---

## ✨  What you get – out of the box

• **Structured artifacts** – every job yields predictable files: `.out`, `.err`, `.log`, `.exit`, `.json`, `.signal` (Unix) – all plain text or JSON.

• **Crash-safe exit codes** – the `.exit` marker is written _before_ log pipes are closed so `pend wait` never hangs on a half-dead worker.

• **Coloured multi-job output** – `pend wait a b c` interleaves logs with deterministic colours and clear ✓ / ✗ status lines.

• **Size-bounded log rotation** – `--max-log-size 10M` keeps CI artifacts small yet complete.
• **Wall-clock timeout** – `pend do <job> --timeout 30 <cmd …>` terminates runaway processes after 30 s and marks the job as failed.
• **Automatic retries** – `--retries 3` re-runs flaky commands up to three times until one attempt succeeds.

• **Strong validation & security** – path traversal is impossible, job names are capped at 100 characters, and an advisory `.lock` prevents concurrent duplicates.

• **Platform quirks handled** – symlink tmpdirs on macOS, `MAX_PATH` on Windows, signals on Unix – tested on all three major OSes in CI.

• **Single static binary** – integrates into any Bash / PowerShell / CMD script without pulling in a runtime.

---

## 🔍  Artifact layout

Jobs live in a single directory (defaults to `$TMPDIR/pend`, override via
`--dir` or `PEND_DIR`).  Files follow `<job>.<ext>`:

| File               | Purpose |
|--------------------|---------|
| `foo.out` / `foo.err` | Raw stdout / stderr as produced. |
| `foo.log` (+ `.log.1` …) | Chronological merged log (rotated). |
| `foo.exit`         | Numeric exit code written first. |
| `foo.json`         | Pretty-printed metadata (command, PID, UTC timestamps). |
| `foo.signal` (Unix) | Raw signal number, if any. |
| `foo.lock`         | Advisory lock file; safe to delete when the job is not running. |

Everything is human-readable → `cat`, `jq`, or even Notepad work fine.

---

## 🚀  Example: parallel build & package

```bash
# Kick off long-running tasks
pend do backend docker build .
pend do frontend npm run build

# Meanwhile run unit tests …
pytest -q

# Stream & fail fast once the first build fails
pend wait backend frontend

# Ensure the linter finishes within 60 seconds and automatically retries once
# if it flaps due to I/O hiccups.
pend do lint --timeout 60 --retries 1 npm run lint
pend wait lint

# Package artifacts only if both succeeded
pend do package ./scripts/package.sh
pend wait package

# Upload artifacts, then clean workspace
pend clean package
```

---

## 🛠  Under the hood

* **Worker process** – spawns child cmd, merges pipes via channel fan-in, writes JSON, exits.
* **File watcher** – `pend wait` uses the cross-platform `notify` crate for instant `.exit` detection; falls back to exponential back-off polling if necessary.
* **No async runtime** – plain threads & channels keep the binary small (< 1 MiB on Linux/musl).

---

## 📚 Prior art

`GNU parallel`, `xargs -P`, `make -j`, `ninja`, `taskwarrior`, `just`, `cargo-make` … all wonderful – yet none hit the sweet spot of *ad-hoc, nameable, parallel shell jobs with deterministic logs*.  **pend** does.

---

Made with ❤️ & Rust – so you can **do now, wait later**.
