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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_ttl_or_elapsed_time_primitive_is_used_by_this_module() {
        let source = include_str!("process_owner.rs");
        let production_source = source
            .split_once("#[cfg(test)]")
            .expect("this module has a #[cfg(test)] boundary")
            .0;
        let forbidden_tokens = ["Instant", "SystemTime", "time::Duration"];
        for forbidden in forbidden_tokens {
            assert!(
                !production_source.contains(forbidden),
                "D10 forbids TTL/elapsed-time staleness evidence, found {forbidden:?}"
            );
        }
    }

    #[test]
    fn the_current_process_owner_is_never_reported_dead() {
        let owner = process_owner_for(std::process::id().cast_signed());
        assert!(!is_definitely_dead(&owner));
    }

    #[test]
    fn a_reaped_child_process_is_positively_dead() {
        let mut child = std::process::Command::new("true")
            .spawn()
            .expect("spawning 'true' should succeed");
        let pid = i32::try_from(child.id()).expect("pid fits in i32");
        child.wait().expect("child should exit and be reaped");

        let owner = ProcessOwner {
            pid,
            instance_token: None,
        };
        assert!(
            is_definitely_dead(&owner),
            "a reaped child's pid must be positively proven dead, not merely assumed"
        );
    }

    #[test]
    fn a_live_process_is_never_abandoned_merely_because_no_instance_token_is_recorded() {
        let owner = ProcessOwner {
            pid: std::process::id().cast_signed(),
            instance_token: None,
        };
        assert!(!is_definitely_dead(&owner));
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn a_mismatched_instance_token_proves_death_even_though_the_pid_is_alive() {
        let real_owner = process_owner_for(std::process::id().cast_signed());
        let recorded_ticks = real_owner
            .instance_token
            .expect("this process's own /proc/self/stat starttime must be readable on Linux");

        let stale_owner = ProcessOwner {
            pid: real_owner.pid,
            instance_token: Some(recorded_ticks.wrapping_add(1)),
        };
        assert!(
            is_definitely_dead(&stale_owner),
            "a live pid whose recorded start time no longer matches must be treated as a \
             different, dead process (PID reuse), never as the still-live original owner"
        );
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn a_matching_instance_token_is_never_reported_dead() {
        let real_owner = process_owner_for(std::process::id().cast_signed());
        assert!(real_owner.instance_token.is_some());
        assert!(!is_definitely_dead(&real_owner));
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn parse_start_ticks_reads_field_twenty_two_after_the_parenthesized_comm() {
        let stat = "1234 (my comm) S 1 1234 1234 0 -1 4194304 100 0 0 0 5 3 0 0 20 0 4 0 987654 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 17 2 0 0 0 0 0 0 0 0 0 0 0 0 0";
        assert_eq!(parse_start_ticks(stat), Some(987_654));
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn parse_start_ticks_handles_a_comm_containing_spaces_and_parens() {
        let stat = "1234 (weird ) comm)) S 1 1234 1234 0 -1 4194304 100 0 0 0 5 3 0 0 20 0 4 0 42 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 17 2 0 0 0 0 0 0 0 0 0 0 0 0 0";
        assert_eq!(parse_start_ticks(stat), Some(42));
    }
}
