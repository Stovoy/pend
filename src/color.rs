/// ANSI color escape sequences used to differentiate job output when waiting
/// on multiple jobs simultaneously.  The list is short on purpose to keep the
/// palette readable on the majority of terminals.
/// Decide at runtime whether color escapes should be emitted.  Honors the
/// de-facto standard `NO_COLOR` environment variable so users can globally
/// disable ANSI sequences.
pub(crate) fn colors_enabled() -> bool {
    std::env::var_os("NO_COLOR").is_none()
}
