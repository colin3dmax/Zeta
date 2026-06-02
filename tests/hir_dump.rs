#[test]
fn dumps_core_items_hir() {
    let source = include_str!("../testdata/core_items.zeta");
    let expected = include_str!("../testdata/core_items.hir");
    let dump = zeta::dump_hir(source).expect("source should lower to HIR");
    assert_eq!(dump, expected);
}

#[test]
fn cli_dumps_core_items_hir() {
    let binary = env!("CARGO_BIN_EXE_zeta");
    let output = std::process::Command::new(binary)
        .args(["hir-dump", "testdata/core_items.zeta"])
        .output()
        .expect("zeta binary should run");

    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        String::from_utf8(output.stdout).expect("stdout should be utf-8"),
        include_str!("../testdata/core_items.hir")
    );
}

#[test]
fn dumps_export_import_hir() {
    let dump = zeta::dump_hir("export import std.core;\n").expect("source should lower to HIR");
    assert_eq!(dump, "HirModule\n  import exported std.core\n");
}
