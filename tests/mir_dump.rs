#[test]
fn dumps_run_mut_mir() {
    let source = include_str!("../testdata/run_mut.zeta");
    let expected = include_str!("../testdata/run_mut.mir");
    let dump = zeta::dump_mir(source).expect("source should lower to MIR");
    assert_eq!(dump, expected);
}

#[test]
fn cli_dumps_run_mut_mir() {
    let binary = env!("CARGO_BIN_EXE_zeta");
    let output = std::process::Command::new(binary)
        .args(["mir-dump", "testdata/run_mut.zeta"])
        .output()
        .expect("zeta binary should run");

    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        String::from_utf8(output.stdout).expect("stdout should be utf-8"),
        include_str!("../testdata/run_mut.mir")
    );
}

#[test]
fn dumps_run_match_mir() {
    let source = include_str!("../testdata/run_match.zeta");
    let expected = include_str!("../testdata/run_match.mir");
    let dump = zeta::dump_mir(source).expect("source should lower to MIR");
    assert_eq!(dump, expected);
}

#[test]
fn dumps_run_struct_mir() {
    let source = include_str!("../testdata/run_struct.zeta");
    let expected = include_str!("../testdata/run_struct.mir");
    let dump = zeta::dump_mir(source).expect("source should lower to MIR");
    assert_eq!(dump, expected);
}

#[test]
fn dumps_run_enum_mir() {
    let source = include_str!("../testdata/run_enum.zeta");
    let expected = include_str!("../testdata/run_enum.mir");
    let dump = zeta::dump_mir(source).expect("source should lower to MIR");
    assert_eq!(dump, expected);
}

#[test]
fn dumps_run_enum_payload_mir() {
    let source = include_str!("../testdata/run_enum_payload.zeta");
    let expected = include_str!("../testdata/run_enum_payload.mir");
    let dump = zeta::dump_mir(source).expect("source should lower to MIR");
    assert_eq!(dump, expected);
}
