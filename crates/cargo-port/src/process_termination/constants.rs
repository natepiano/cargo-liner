// termination confirmation
/// How often the termination worker re-observes a signaled process while it
/// waits for the process object to disappear.
#[cfg(test)]
pub(super) const TERMINATION_CONFIRMATION_POLL_INTERVAL: std::time::Duration =
    std::time::Duration::from_millis(25);
/// How long the termination worker waits for a signaled process object to
/// disappear before it reports the target as a survivor. The wait never
/// escalates to a second, stronger signal.
#[cfg(any(target_os = "linux", test))]
pub(super) const TERMINATION_CONFIRMATION_TIMEOUT: std::time::Duration =
    std::time::Duration::from_secs(2);
