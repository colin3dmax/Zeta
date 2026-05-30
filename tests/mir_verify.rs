use zeta::ast::Param;
use zeta::diagnostic::Span;
use zeta::mir::{self, MirExpr, MirFunction, MirStmt, Program};

#[test]
fn verifier_accepts_lowered_run_corpus() {
    for source in [
        include_str!("../testdata/run_basic.zeta"),
        include_str!("../testdata/run_bool_logic.zeta"),
        include_str!("../testdata/run_mut.zeta"),
        include_str!("../testdata/run_struct.zeta"),
        include_str!("../testdata/run_enum.zeta"),
        include_str!("../testdata/run_match.zeta"),
    ] {
        let module = zeta::parse_source(source).expect("source should parse");
        zeta::resolver::resolve(&module).expect("source should resolve");
        zeta::typecheck::check(&module).expect("source should typecheck");
        mir::verify(&mir::lower(&module)).expect("lowered MIR should verify");
    }
}

#[test]
fn verifier_rejects_unknown_store_target() {
    let program = Program {
        enums: vec![],
        functions: vec![MirFunction {
            name: "main".to_string(),
            params: vec![],
            return_type: Some("Int".to_string()),
            body: vec![
                MirStmt::Store {
                    name: "answer".to_string(),
                    value: MirExpr::Int("42".to_string()),
                },
                MirStmt::Return(Some(MirExpr::Int("42".to_string()))),
            ],
        }],
    };

    let diagnostics = mir::verify(&program).expect_err("unknown store should fail");
    assert_eq!(diagnostics[0].code, "MIR_UNKNOWN_LOCAL");
}

#[test]
fn verifier_rejects_return_type_mismatch() {
    let program = Program {
        enums: vec![],
        functions: vec![MirFunction {
            name: "main".to_string(),
            params: vec![],
            return_type: Some("Int".to_string()),
            body: vec![MirStmt::Return(Some(MirExpr::Bool(true)))],
        }],
    };

    let diagnostics = mir::verify(&program).expect_err("wrong return type should fail");
    assert_eq!(diagnostics[0].code, "MIR_RETURN_TYPE");
}

#[test]
fn verifier_rejects_call_argument_type_mismatch() {
    let program = Program {
        enums: vec![],
        functions: vec![
            MirFunction {
                name: "answer".to_string(),
                params: vec![Param {
                    name: "value".to_string(),
                    name_span: Span::new(0, 0),
                    ty: "Int".to_string(),
                }],
                return_type: Some("Int".to_string()),
                body: vec![MirStmt::Return(Some(MirExpr::Load("value".to_string())))],
            },
            MirFunction {
                name: "main".to_string(),
                params: vec![],
                return_type: Some("Int".to_string()),
                body: vec![MirStmt::Return(Some(MirExpr::Call {
                    callee: "answer".to_string(),
                    args: vec![MirExpr::Bool(true)],
                }))],
            },
        ],
    };

    let diagnostics = mir::verify(&program).expect_err("wrong call argument should fail");
    assert!(
        diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "MIR_CALL_TYPE"),
        "diagnostics: {diagnostics:?}"
    );
}
