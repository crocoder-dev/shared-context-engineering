use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub(crate) struct ProcessOwner {
    pub pid: i32,
    pub instance_token: Option<u64>,
}

#[cfg(unix)]
mod raw {
    unsafe extern "C" {
        pub(super) fn getppid() -> i32;
        pub(super) fn kill(pid: i32, sig: i32) -> i32;
    }
}

#[cfg(unix)]
const ESRCH: i32 = 3;

pub(crate) fn process_owner_for(pid: i32) -> ProcessOwner {
    ProcessOwner {
        pid,
        instance_token: process_start_ticks(pid),
    }
}

pub(crate) fn current_process_owner() -> ProcessOwner {
    #[cfg(unix)]
    {
        process_owner_for(unsafe { raw::getppid() })
    }
    #[cfg(not(unix))]
    {
        ProcessOwner {
            pid: 0,
            instance_token: None,
        }
    }
}

pub(crate) fn is_definitely_dead(owner: &ProcessOwner) -> bool {
    #[cfg(unix)]
    {
        if unsafe { raw::kill(owner.pid, 0) } == 0 {
            match owner.instance_token {
                Some(recorded) => match process_start_ticks(owner.pid) {
                    Some(current) => current != recorded,
                    None => false,
                },
                None => false,
            }
        } else {
            std::io::Error::last_os_error().raw_os_error() == Some(ESRCH)
        }
    }
    #[cfg(not(unix))]
    {
        let _ = owner;
        false
    }
}

#[cfg(target_os = "linux")]
fn process_start_ticks(pid: i32) -> Option<u64> {
    let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    parse_start_ticks(&stat)
}

#[cfg(not(target_os = "linux"))]
fn process_start_ticks(_pid: i32) -> Option<u64> {
    None
}

#[cfg(target_os = "linux")]
fn parse_start_ticks(stat: &str) -> Option<u64> {
    let after_comm = stat.rsplit_once(')')?.1;
    after_comm.split_whitespace().nth(19)?.parse().ok()
}
