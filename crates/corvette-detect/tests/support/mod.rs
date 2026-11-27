fn forever_stream(emitter: &str, check_every: u32) -> String {
    format!(
        "i=0; t=$(date +%s); while :; do {{ {emitter}; }} || exit 1; i=$((i + 1)); \
         [ $((i % {check_every})) -gt 0 ] || [ \"$(date +%s)\" -lt $((t + 120)) ] || exit 0; done"
    )
}

#[test]
fn the_stream_stub_gives_up_when_its_reader_closes() {
    let script = forever_stream("head -c 100000 /dev/zero", 1);
    let mut child = std::process::Command::new("sh")
        .args(["-c", &script])
        .stdout(std::process::Stdio::piped())
        .spawn()
        .expect("the stub starts");
    drop(child.stdout.take());
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    loop {
        if child.try_wait().expect("status readable").is_some() {
            return;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "the stub outlived its pipe"
        );
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
}

fn wait_for_pid_file(path: &std::path::Path, within: std::time::Duration) -> u32 {
    // A stub's redirect creates the empty file before the pid lands in it.
    let deadline = std::time::Instant::now() + within;
    loop {
        if let Some(pid) = std::fs::read_to_string(path).ok().and_then(|t| t.trim().parse::<u32>().ok()) {
            return pid;
        }
        assert!(std::time::Instant::now() < deadline, "no pid appeared in {}", path.display());
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
}

#[test]
fn the_pid_wait_ignores_a_file_written_in_two_steps() {
    let path = std::env::temp_dir().join(format!("corvette-pid-wait-{}.pid", std::process::id()));
    std::fs::write(&path, "").expect("empty pid file");
    let filler_path = path.clone();
    let filler = std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_millis(50));
        std::fs::write(&filler_path, "4242\n").expect("fill the pid file");
    });
    let pid = wait_for_pid_file(&path, std::time::Duration::from_secs(5));
    filler.join().expect("filler thread finished");
    std::fs::remove_file(&path).expect("remove test pid file");
    assert_eq!(4242, pid, "the wait must not stop at the empty file");
}
