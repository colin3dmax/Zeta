fn assert_check_golden(path: &str, expected_stderr: &str) {
    let binary = env!("CARGO_BIN_EXE_zeta");
    let output = std::process::Command::new(binary)
        .args(["check", path])
        .output()
        .expect("zeta check should run");

    assert!(
        !output.status.success(),
        "check should fail for {path}, stdout: {}",
        String::from_utf8_lossy(&output.stdout)
    );
    assert_eq!(
        String::from_utf8(output.stderr).expect("stderr should be utf-8"),
        expected_stderr
    );
    assert_eq!(
        String::from_utf8(output.stdout).expect("stdout should be utf-8"),
        ""
    );
}

#[test]
fn cli_golden_unknown_name_diagnostic() {
    assert_check_golden(
        "testdata/diagnostics/unknown_name.zeta",
        include_str!("../testdata/diagnostics/unknown_name.stderr"),
    );
}

#[test]
fn cli_golden_type_mismatch_diagnostic() {
    assert_check_golden(
        "testdata/diagnostics/type_mismatch.zeta",
        include_str!("../testdata/diagnostics/type_mismatch.stderr"),
    );
}

#[test]
fn cli_golden_module_ambiguous_short_name_diagnostic() {
    assert_check_golden(
        "testdata/diagnostics/modules_ambiguous",
        include_str!("../testdata/diagnostics/modules_ambiguous.stderr"),
    );
}

#[test]
fn cli_golden_module_ambiguous_type_diagnostic() {
    assert_check_golden(
        "testdata/diagnostics/modules_ambiguous_type",
        include_str!("../testdata/diagnostics/modules_ambiguous_type.stderr"),
    );
}

#[test]
fn cli_golden_match_non_exhaustive_diagnostic() {
    assert_check_golden(
        "testdata/diagnostics/match_non_exhaustive",
        include_str!("../testdata/diagnostics/match_non_exhaustive.stderr"),
    );
}
