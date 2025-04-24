/// ANSI color escape sequences used to differentiate job output when waiting
/// on multiple jobs simultaneously.  The list is short on purpose to keep the
/// palette readable on the majority of terminals.
pub(crate) const COLOR_CODES: [&str; 6] = [
    "\x1b[31m", // red
    "\x1b[32m", // green
    "\x1b[33m", // yellow
    "\x1b[34m", // blue
    "\x1b[35m", // magenta
    "\x1b[36m", // cyan
];

/// Decide at runtime whether color escapes should be emitted.  Honors the
/// de-facto standard `NO_COLOR` environment variable so users can globally
/// disable ANSI sequences.
pub(crate) fn colors_enabled() -> bool {
    std::env::var_os("NO_COLOR").is_none()
}
