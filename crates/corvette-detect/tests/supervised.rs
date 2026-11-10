use std::process::Stdio;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use corvette_detect::{Policy, Status, Supervised};
use tokio::process::{Child, Command};
use tokio::sync::watch;

const SMALL: Policy = Policy {
    initial: Duration::from_millis(10),
    max: Duration::from_millis(40),
    healthy_after: Duration::from_mins(1),
};

fn shell(script: &str) -> Command {
    let mut command = Command::new("sh");
    command.arg("-c").arg(script);
    command
}

async fn wait_for<F>(rx: &mut watch::Receiver<Status>, mut predicate: F)
where
    F: FnMut(&Status) -> bool,
{
    let outcome = tokio::time::timeout(Duration::from_secs(10), rx.wait_for(&mut predicate)).await;
    match outcome {
        Ok(Ok(_status)) => (),
        Ok(Err(_closed)) => panic!("the supervisor stopped before the status matched"),
        Err(elapsed) => panic!("the status did not match within 10s: {elapsed:?}"),
    }
}

struct ProcessKiller(u32);

impl Drop for ProcessKiller {
    fn drop(&mut self) {
        let pid = self.0.to_string();
        let _ = std::process::Command::new("kill")
            .args(["-KILL", &pid])
            .status();
    }
}

fn proc_alive(pid: u32) -> bool {
    let Ok(stat) = std::fs::read_to_string(format!("/proc/{pid}/stat")) else {
        return false;
    };
    stat.rsplit(") ")
        .next()
        .and_then(|rest| rest.chars().next())
        .is_some_and(|state| state != 'Z' && state != 'X')
}

#[tokio::test]
async fn a_child_that_exits_zero_is_reported_and_started_again() {
    let supervisor = Supervised::spawn("zero".to_string(), SMALL, || shell("exit 0"), |_| {});
    let mut rx = supervisor.subscribe();
    wait_for(&mut rx, |status| status.starts >= 3 && status.exits >= 2).await;
    let status = *rx.borrow();
    assert!(status.last_exit.is_some_and(|exit| exit.success()));
    assert_eq!(0, status.spawn_failures);
}

#[tokio::test]
async fn the_exit_code_of_a_failing_child_is_reported() {
    let supervisor = Supervised::spawn("seven".to_string(), SMALL, || shell("exit 7"), |_| {});
    let mut rx = supervisor.subscribe();
    wait_for(&mut rx, |status| status.exits >= 1).await;
    let status = *rx.borrow();
    assert_eq!(Some(7), status.last_exit.and_then(|exit| exit.code()));
}

#[tokio::test]
async fn restarts_are_spaced_by_the_delay() {
    let policy = Policy {
        initial: Duration::from_millis(150),
        max: Duration::from_millis(150),
        healthy_after: Duration::from_mins(1),
    };
    let supervisor = Supervised::spawn("spaced".to_string(), policy, || shell("exit 1"), |_| {});
    let rx = supervisor.subscribe();
    tokio::time::sleep(Duration::from_millis(700)).await;
    let status = *rx.borrow();
    assert!(
        (2..=8).contains(&status.starts),
        "starts {} is outside 2..=8",
        status.starts
    );
}

#[tokio::test]
async fn a_program_that_cannot_start_is_counted_and_retried() {
    let supervisor = Supervised::spawn(
        "missing".to_string(),
        SMALL,
        || Command::new("/nonexistent/corvette-test-no-such-program"),
        |_| {},
    );
    let mut rx = supervisor.subscribe();
    wait_for(&mut rx, |status| status.spawn_failures >= 3).await;
    let status = *rx.borrow();
    assert_eq!(0, status.starts);
    assert_eq!(0, status.exits);
    assert!(!status.running);
    assert_eq!(None, status.last_exit);
}

#[tokio::test]
async fn dropping_the_supervisor_kills_the_running_child() {
    let dir = std::env::temp_dir().join(format!(
        "corvette-detect-dropping_the_supervisor_kills_the_running_child-{}",
        std::process::id()
    ));
    std::fs::create_dir_all(&dir).expect("scratch directory");
    let pid_path = dir.join("pid");
    let script = format!("echo $$ > {}; exec sleep 30", pid_path.display());
    let command_script = script.clone();
    let supervisor = Supervised::spawn(
        "sleeper".to_string(),
        SMALL,
        move || shell(&command_script),
        |_| {},
    );
    let mut rx = supervisor.subscribe();
    wait_for(&mut rx, |status| status.running).await;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    let pid = loop {
        if let Ok(text) = std::fs::read_to_string(&pid_path)
            && let Ok(pid) = text.trim().parse::<u32>()
        {
            break pid;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "no pid number appeared in {}",
            pid_path.display()
        );
        tokio::time::sleep(Duration::from_millis(10)).await;
    };
    let _killer = ProcessKiller(pid);
    assert!(proc_alive(pid), "pid {pid} should be running");
    drop(supervisor);
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    while proc_alive(pid) && tokio::time::Instant::now() < deadline {
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert!(!proc_alive(pid), "pid {pid} survived the supervisor drop");
    std::fs::remove_dir_all(&dir).expect("remove scratch directory");
}

#[tokio::test]
async fn restarting_stops_when_the_supervisor_is_dropped() {
    let supervisor = Supervised::spawn("stop".to_string(), SMALL, || shell("exit 0"), |_| {});
    let mut rx = supervisor.subscribe();
    wait_for(&mut rx, |status| status.starts >= 2).await;
    drop(supervisor);
    let before = rx.borrow().starts;
    tokio::time::sleep(Duration::from_millis(200)).await;
    let after = rx.borrow().starts;
    assert_eq!(before, after, "starts kept growing after the drop");
}

#[tokio::test]
async fn the_start_hook_runs_before_each_start_is_reported() {
    let hook_calls = Arc::new(AtomicU64::new(0));
    let missing_stdout = Arc::new(AtomicU64::new(0));
    let hook_counter = Arc::clone(&hook_calls);
    let missing_counter = Arc::clone(&missing_stdout);
    let supervisor = Supervised::spawn(
        "hooked".to_string(),
        SMALL,
        || {
            let mut command = shell("exit 0");
            command.stdout(Stdio::piped());
            command
        },
        move |child: &mut Child| {
            hook_counter.fetch_add(1, Ordering::SeqCst);
            if child.stdout.take().is_none() {
                missing_counter.fetch_add(1, Ordering::SeqCst);
            }
        },
    );
    let mut rx = supervisor.subscribe();
    wait_for(&mut rx, |status| status.starts >= 3).await;
    let status = *rx.borrow();
    assert!(
        hook_calls.load(Ordering::SeqCst) >= status.starts,
        "the hook ran {} times for {} reported starts",
        hook_calls.load(Ordering::SeqCst),
        status.starts
    );
    assert_eq!(0, missing_stdout.load(Ordering::SeqCst));
}
