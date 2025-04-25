//! Color utilities shared by `pend wait` when displaying interleaved output
//! from multiple jobs.
//!
//! The actual colour palette lives in `src/wait.rs`;  this helper merely
//! determines – at runtime – whether ANSI escape sequences should be emitted
//! at all.  We honour the de-facto standard `NO_COLOR` environment variable so
//! that users can globally disable colourized CLI output.
//!
//! Because the binary has no public API the module is `pub(crate)` by default;
//! these docs exist purely to guide future maintainers.
/// Decide at runtime whether color escapes should be emitted.  Honors the
/// de-facto standard `NO_COLOR` environment variable so users can globally
/// disable ANSI sequences.
pub(crate) fn colors_enabled() -> bool {
    std::env::var_os("NO_COLOR").is_none()
}
