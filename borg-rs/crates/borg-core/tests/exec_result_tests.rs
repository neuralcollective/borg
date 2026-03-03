use borg_core::git::ExecResult;

fn make(stdout: &str, stderr: &str) -> ExecResult {
    ExecResult {
        stdout: stdout.to_string(),
        stderr: stderr.to_string(),
        exit_code: 0,
    }
}

#[test]
fn combined_output_stdout_only() {
    let r = make("hello", "");
    assert_eq!(r.combined_output(), "hello");
}

#[test]
fn combined_output_stderr_only() {
    // When stdout is empty but stderr is non-empty, combined_output joins with newline,
    // producing a leading newline. This test documents that behavior.
    let r = make("", "error: something went wrong");
    assert_eq!(r.combined_output(), "\nerror: something went wrong");
}

#[test]
fn combined_output_both_streams() {
    let r = make("output line", "warning: something");
    assert_eq!(r.combined_output(), "output line\nwarning: something");
}
