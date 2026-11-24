use std::os::fd::OwnedFd;

use rustix::process::{pidfd_open, Pid, PidfdFlags};

pub(crate) fn open_pidfd(raw: i32) -> Result<OwnedFd, String> {
    // Pid::from_raw debug-asserts non-negative, so the sign check must come first.
    let pid =
        if raw > 0 { Pid::from_raw(raw) } else { None }.ok_or(format!("invalid pid {raw}"))?;
    pidfd_open(pid, PidfdFlags::empty()).map_err(|e| format!("pid {raw}: {e}"))
}

pub(crate) fn exit_code(status: std::process::ExitStatus) -> i32 {
    use std::os::unix::process::ExitStatusExt;
    status
        .code()
        .unwrap_or_else(|| 128 + status.signal().unwrap_or(0))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn opens_a_running_process() {
        assert!(open_pidfd(std::process::id() as i32).is_ok());
    }

    #[test]
    fn rejects_a_missing_process() {
        // PIDs above pid_max (at most 2^22) never exist.
        assert!(open_pidfd(i32::MAX).is_err());
    }

    #[test]
    fn rejects_non_positive_pids() {
        assert!(open_pidfd(0).is_err());
        assert!(open_pidfd(-1).is_err());
    }

    #[test]
    fn exit_code_passes_through_the_command_code() {
        use std::os::unix::process::ExitStatusExt;
        assert_eq!(exit_code(std::process::ExitStatus::from_raw(3 << 8)), 3);
    }

    #[test]
    fn exit_code_maps_signals_like_a_shell() {
        use std::os::unix::process::ExitStatusExt;
        assert_eq!(exit_code(std::process::ExitStatus::from_raw(15)), 143);
    }
}
