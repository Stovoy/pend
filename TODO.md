# pend – remaining work

This document tracks functionality mentioned in `README.md` that the current
Rust implementation does **not** yet cover.

## Missing / incomplete features

All items below remain outstanding. Previously completed tasks are left for
historical reference.

• `--dir` CLI flag – **Done** ✅
• Interleaved streaming when waiting for multiple jobs – **Done** ✅
• Combined stdout/stderr ordering – **Done** ✅
   Worker now captures both streams concurrently, merges them into a `.log`
   file, and `pend wait` replays that file (ensuring original ordering). The
   traditional `.out` / `.err` artefacts are still written for troubleshooting.

4. YAML meta format option
   README says "tiny JSON/YAML meta file". Only JSON is emitted today. Either
   emit YAML by default on platforms that prefer it or provide a flag/env to
   choose between the two.

1. YAML meta format option
   README says "tiny JSON/YAML meta file". Only JSON is emitted today. Either
   emit YAML by default or provide flag/env.

2. Windows-specific detaching
   On Windows we currently do not set the `CREATE_NEW_PROCESS_GROUP` flag or
   similar. Verify proper detachment and signal isolation on Windows and add
   platform-specific flags if necessary.

3. Additional tests
   • Validate `--dir` once implemented.
   • Test multi-job interleaved wait behaviour.
   • Cross-platform CI matrix (Windows, macOS, Linux).
