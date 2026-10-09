use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::mpsc;

use super::super::super::maintenance_state::{
    read_state, state_path, write_state_atomically, MaintenanceState, StateRead,
    RECONCILIATION_ADVISORY_AFTER_MS,
};
use super::super::super::ref_advisory::{advise_if_due, advise_if_due_with};
use super::support::WATCHDOG;
use super::{Fixture, NOW};

const ROLE_ENV: &str = "SCE_CROSS_PROCESS_ADVISORY_ROLE";
const ROOT_ENV: &str = "SCE_CROSS_PROCESS_ADVISORY_ROOT";
const NOW_ENV: &str = "SCE_CROSS_PROCESS_ADVISORY_NOW";
const ROLE_PARK: &str = "park";
const ROLE_PROBE: &str = "probe";
const PARKED_LINE: &str = "PARKED";
const OUTCOME_PREFIX: &str = "OUTCOME:";
const CHILD_TEST: &str = "ref_reconciliation_cross_process_advisory_child";

fn child_test_path() -> String {
    let module = module_path!();
    let (_, relative) = module
        .split_once("::")
        .expect("crate-qualified module path");
    format!("{relative}::{CHILD_TEST}")
}

fn child_command(role: &str, root: &Path) -> Command {
    let mut command = Command::new(std::env::current_exe().expect("current exe"));
    command
        .args(["--exact", &child_test_path(), "--nocapture"])
        .env(ROLE_ENV, role)
        .env(ROOT_ENV, root)
        .env(NOW_ENV, NOW.to_string())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    command
}

fn outcome_of(stdout: &str) -> String {
    stdout
        .lines()
        .find_map(|line| line.strip_prefix(OUTCOME_PREFIX))
        .unwrap_or_else(|| panic!("child reported no outcome:\n{stdout}"))
        .to_string()
}

fn run_probe(root: &Path) -> String {
    let output = child_command(ROLE_PROBE, root)
        .stdin(Stdio::null())
        .output()
        .expect("probe child runs");
    assert!(
        output.status.success(),
        "probe child failed:\n{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    outcome_of(&String::from_utf8_lossy(&output.stdout))
}

#[test]
fn ref_reconciliation_cross_process_advisory_child() {
    let Ok(role) = std::env::var(ROLE_ENV) else {
        return;
    };
    let root = PathBuf::from(std::env::var(ROOT_ENV).expect("root"));
    let now: i64 = std::env::var(NOW_ENV)
        .expect("now")
        .parse()
        .expect("numeric now");
    let outcome = match role.as_str() {
        ROLE_PARK => advise_if_due_with(
            &root,
            || now,
            |path, state| {
                println!("{PARKED_LINE}");
                std::io::stdout().flush().expect("flush stdout");
                let mut release = String::new();
                std::io::stdin()
                    .read_line(&mut release)
                    .expect("wait for release");
                write_state_atomically(path, state)
            },
        ),
        ROLE_PROBE => advise_if_due(&root, || now),
        other => panic!("unknown role {other}"),
    };
    let name = format!("{outcome:?}");
    let name = name
        .split([' ', '{'])
        .next()
        .unwrap_or_default()
        .to_string();
    println!("{OUTCOME_PREFIX}{name}");
}

#[test]
fn ref_reconciliation_advisory_lock_serializes_independent_processes() {
    let fixture = Fixture::new("https://example.invalid/org/repo-a.git");
    let git_dir = fixture.root.join(".git");
    let path = state_path(&git_dir);
    std::fs::create_dir_all(path.parent().expect("runtime dir")).expect("runtime dir");
    let due = MaintenanceState {
        anchor: Some(NOW - RECONCILIATION_ADVISORY_AFTER_MS - 1),
        ..MaintenanceState::default()
    };
    write_state_atomically(&path, &due).expect("seed due state");
    let seeded = std::fs::read(&path).expect("seeded bytes");

    let mut parked = child_command(ROLE_PARK, &fixture.root)
        .stdin(Stdio::piped())
        .spawn()
        .expect("park child spawns");
    let mut stdout = BufReader::new(parked.stdout.take().expect("park stdout"));
    let (line_tx, line_rx) = mpsc::channel::<String>();
    let reader = std::thread::spawn(move || {
        let mut lines = Vec::new();
        let mut line = String::new();
        while stdout.read_line(&mut line).expect("read child stdout") > 0 {
            let trimmed = line.trim_end().to_string();
            if trimmed == PARKED_LINE {
                let _ = line_tx.send(trimmed.clone());
            }
            lines.push(trimmed);
            line.clear();
        }
        lines
    });
    assert_eq!(
        line_rx.recv_timeout(WATCHDOG).expect("child parks"),
        PARKED_LINE
    );

    assert_eq!(run_probe(&fixture.root), "Busy");
    assert_eq!(
        std::fs::read(&path).expect("state while parked"),
        seeded,
        "a second process must not write while the first holds the lock"
    );

    let mut stdin = parked.stdin.take().expect("park stdin");
    stdin
        .write_all(b"release\n")
        .expect("release the parked child");
    drop(stdin);
    let status = parked.wait().expect("park child exits");
    assert!(status.success());
    let lines = reader.join().expect("reader joins");
    assert_eq!(
        outcome_of(&lines.join("\n")),
        "Advised",
        "the lock holder recorded the one advice"
    );

    assert_eq!(run_probe(&fixture.root), "NoAction");

    let StateRead::Valid(stored) = read_state(&path).expect("state") else {
        panic!("expected valid state");
    };
    assert_eq!(stored.last_advised, Some(NOW));
    assert_eq!(stored.anchor, due.anchor);
}
