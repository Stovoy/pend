# pend

`pend` is a tiny, cross‑platform command‑line tool that gives you two primitives – **pend do** and **pend wait** – to sprinkle safe, parallel execution into any shell script or CI job.

The idea is deceptively simple: *do now, wait later.*

## Why does this exist?

More than a decade ago, while speeding up build pipelines at Nextdoor, I realised there was no light‑weight way to:

1. Kick off an arbitrary shell command in the background.
2. Capture its standard output, standard error, exit code, and metadata in a predictable place.
3. Later – in any order – block on that specific job, replay its logs, and propagate its exit status.

GNU parallel, `xargs ‑P`, `make ‑j`, and full‑blown build systems solve adjacent problems but come with heavy assumptions, noisy interleaved logs, or the need to formalise an entire dependency graph. For scripting one‑offs and mid‑sized projects, the original **do/wait** bash duo was the sweet spot. `pend` is that idea rewritten in Rust, packaged as an installable binary, and ready for Windows, macOS, and Linux.

## The mental model

```
pend do <job‑name> <command> [arg …]
```

• Spawns `<command>` in the background.
• Streams stdout & stderr into `<jobs‑dir>/<job‑name>.out` and `.err`.
• Stores the process's exit code in `<job‑name>.exit` once it finishes.
• Writes a tiny JSON/YAML meta file with start/end timestamps and the child PID.

```
pend wait <job‑name>
```

• Blocks until the job completes (if it hasn't already).
• Replays the captured output to your terminal in original order.
• Exits with the same exit code your job produced.

That's it – an effective job queue & log collector in two subcommands.

## Quick start

```bash
# 1. Install (once cargo is published)
cargo install pend

# 2. Parallelise your build script
pend do backend ./scripts/build_backend.sh
pend do frontend ./scripts/build_frontend.sh

# Wait for them to finish - non-zero exitcodes are bubbled up
pend wait backend
pend wait frontend
# Or: `pend wait backend frontend` to stream and interleave output 

# Continue only if both succeed
pend do package ./scripts/package.sh
pend wait package
```

### Output location

By default, `pend` stores job artifacts in a temporary directory specific to your system (such as `/tmp/pend/` on Unix-like systems or the appropriate temp folder on Windows).

Set `--dir` or environment variable `PEND_DIR` to relocate them to a custom location.

## Feature highlights

* Zero runtime dependencies – statically linked Rust binary.
* Named jobs & clean artifacts make failures easy to diagnose.
* Runs anywhere Rust runs: Windows, macOS, Linux (x86‑64, aarch64, etc).
* Safe exit‑status propagation – your CI fails when your job fails.
* Opt‑in streaming: you decide when output is surfaced.

## Prior art & inspiration

* The original `do` / `wait` bash scripts (2012).
* `GNU parallel`, `xargs ‑P` – great for lists, cumbersome for structured builds.
* `make ‑j`, `ninja`, `bazel` – heavyweight graphs, noisy logs.
* `taskwarrior`, `just`, `cargo‑make` – oriented around declarative recipes.

`pend` fills the tiny but mighty niche in between.

---

Made with ❤️ & Rust so you can do now, wait later.
