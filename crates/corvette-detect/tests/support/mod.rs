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
