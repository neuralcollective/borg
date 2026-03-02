use borg_core::git::ExecResult;

#[test]
fn success_true_at_exit_code_zero() {
    let r = ExecResult { stdout: String::new(), stderr: String::new(), exit_code: 0 };
    assert!(r.success());
}

#[test]
fn success_false_at_nonzero_exit_code() {
    let r = ExecResult { stdout: String::new(), stderr: String::new(), exit_code: 1 };
    assert!(!r.success());
}

#[test]
fn combined_output_empty_stderr_returns_stdout() {
    let r = ExecResult {
        stdout: "hello".to_string(),
        stderr: String::new(),
        exit_code: 0,
    };
    assert_eq!(r.combined_output(), "hello");
}

#[test]
fn combined_output_both_non_empty_returns_newline_joined() {
    let r = ExecResult {
        stdout: "out".to_string(),
        stderr: "err".to_string(),
        exit_code: 1,
    };
    assert_eq!(r.combined_output(), "out\nerr");
}
