use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use super::*;

/// A tied `sleep 30`, started through `spawn_tied`.
fn tied_sleep() -> Child {
    let mut cmd = Command::new("sleep");
    cmd.arg("30")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    end_with_app(&mut cmd);
    spawn_tied(move || cmd.spawn()).expect("start sleep")
}

/// Whether process `pid` has ended (gone, or a zombie waiting for its new parent to reap it).
fn ended(pid: u32) -> bool {
    match std::fs::read_to_string(format!("/proc/{pid}/stat")) {
        // The state follows the command's closing parenthesis.
        Ok(stat) => stat
            .rsplit_once(") ")
            .is_some_and(|(_, rest)| rest.starts_with('Z')),
        Err(_) => true,
    }
}

#[test]
fn a_tied_child_outlives_the_thread_that_started_it() {
    // A thread that ends right after starting it, as a blocking-pool thread may.
    let mut child = std::thread::spawn(tied_sleep).join().unwrap();
    std::thread::sleep(Duration::from_millis(300));
    let still = child.try_wait().unwrap().is_none();
    let _ = child.kill();
    let _ = child.wait();
    assert!(still, "the child ended with the thread that started it");
}

/// The test below runs in a child process of this test program, which then ends; the
/// helper it started must end with it.
#[test]
fn a_tied_child_ends_when_the_app_ends() {
    let exe = std::env::current_exe().unwrap();
    let out = Command::new(exe)
        .args([
            "linux::process::tests::start_a_tied_child_and_exit",
            "--exact",
            "--ignored",
            "--nocapture",
        ])
        .stdin(Stdio::null())
        .output()
        .unwrap();
    let said = String::from_utf8_lossy(&out.stdout);
    let pid: u32 = said
        .lines()
        .find_map(|l| l.strip_prefix("tied child "))
        .and_then(|p| p.trim().parse().ok())
        .unwrap_or_else(|| panic!("no child started: {said}"));
    let deadline = Instant::now() + Duration::from_secs(5);
    while !ended(pid) {
        if Instant::now() > deadline {
            // SAFETY: a signal to the test's own stray `sleep`.
            unsafe { libc::kill(pid as libc::pid_t, libc::SIGKILL) };
            panic!("the tied child outlived the process that started it");
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

#[test]
#[ignore = "run by the test above: it ends its process"]
fn start_a_tied_child_and_exit() {
    let child = tied_sleep();
    #[allow(clippy::print_stdout, reason = "the parent test reads the child's ID")]
    {
        println!("tied child {}", child.id());
    }
    std::process::exit(0);
}
