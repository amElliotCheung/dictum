//! Unix process liveness and signalling used by the run supervisor and lease.

fn probe(target: i64) -> bool {
    #[cfg(unix)]
    {
        if target == 0 || target > i64::from(i32::MAX) || target < i64::from(i32::MIN) {
            return false;
        }
        let alive = unsafe { libc::kill(target as libc::pid_t, 0) == 0 };
        alive || std::io::Error::last_os_error().raw_os_error() == Some(libc::EPERM)
    }
    #[cfg(not(unix))]
    {
        let _ = target;
        false
    }
}

pub(crate) fn alive(pid: i64) -> bool {
    pid > 0 && probe(pid)
}

pub(crate) fn group_alive(group: i64) -> bool {
    group > 0 && probe(-group)
}

pub(crate) fn signal(target: i64, signal: i32) {
    #[cfg(unix)]
    {
        if let Ok(target) = i32::try_from(target) {
            unsafe {
                libc::kill(target, signal);
            }
        }
    }
    #[cfg(not(unix))]
    {
        let _ = (target, signal);
    }
}

pub(crate) fn signal_group(pid: i64, sig: i32) {
    if pid > 0 {
        signal(-pid, sig);
    }
}

pub(crate) fn current_group() -> i64 {
    #[cfg(unix)]
    unsafe {
        i64::from(libc::getpgrp())
    }
    #[cfg(not(unix))]
    {
        0
    }
}
