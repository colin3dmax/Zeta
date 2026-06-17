use zeta::ast::Param;
use zeta::diagnostic::Span;
use zeta::mir::{
    self, MirEnum, MirEnumVariant, MirExpr, MirFunction, MirMatchArm, MirPattern, MirPlace,
    MirStmt, Program,
};

#[test]
fn verifier_accepts_lowered_run_corpus() {
    for source in [
        include_str!("../testdata/run_basic.zeta"),
        include_str!("../testdata/run_bool_logic.zeta"),
        include_str!("../testdata/run_mut.zeta"),
        include_str!("../testdata/run_struct.zeta"),
        include_str!("../testdata/run_enum.zeta"),
        include_str!("../testdata/run_enum_payload.zeta"),
        include_str!("../testdata/run_match.zeta"),
        include_str!("../testdata/run_loop_control.zeta"),
        include_str!("../testdata/run_array.zeta"),
        include_str!("../testdata/run_string_scan.zeta"),
        include_str!("../testdata/run_array_builder.zeta"),
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
            reloadable: false,
            type_params: vec![],
            name: "main".to_string(),
            params: vec![],
            return_type: Some("Int".to_string()),
            body: vec![
                MirStmt::Store {
                    place: MirPlace::Local("answer".to_string()),
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
fn verifier_rejects_break_outside_loop() {
    let program = Program {
        enums: vec![],
        functions: vec![MirFunction {
            reloadable: false,
            type_params: vec![],
            name: "main".to_string(),
            params: vec![],
            return_type: None,
            body: vec![MirStmt::Break],
        }],
    };

    let diagnostics = mir::verify(&program).expect_err("break outside loop should fail");
    assert_eq!(diagnostics[0].code, "MIR_BREAK_OUTSIDE_LOOP");
}

#[test]
fn verifier_rejects_continue_outside_loop() {
    let program = Program {
        enums: vec![],
        functions: vec![MirFunction {
            reloadable: false,
            type_params: vec![],
            name: "main".to_string(),
            params: vec![],
            return_type: None,
            body: vec![MirStmt::Continue],
        }],
    };

    let diagnostics = mir::verify(&program).expect_err("continue outside loop should fail");
    assert_eq!(diagnostics[0].code, "MIR_CONTINUE_OUTSIDE_LOOP");
}

#[test]
fn verifier_rejects_return_type_mismatch() {
    let program = Program {
        enums: vec![],
        functions: vec![MirFunction {
            reloadable: false,
            type_params: vec![],
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
            reloadable: false,
            type_params: vec![],
                name: "answer".to_string(),
                params: vec![Param {
                    name: "value".to_string(),
                    name_span: Span::new(0, 0),
                    ty: "Int".to_string(),
                    ty_span: Span::new(0, 0),
                }],
                return_type: Some("Int".to_string()),
                body: vec![MirStmt::Return(Some(MirExpr::Load("value".to_string())))],
            },
            MirFunction {
            reloadable: false,
            type_params: vec![],
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

#[test]
fn verifier_rejects_missing_return_for_non_unit_function() {
    let program = Program {
        enums: vec![],
        functions: vec![MirFunction {
            reloadable: false,
            type_params: vec![],
            name: "main".to_string(),
            params: vec![],
            return_type: Some("Int".to_string()),
            body: vec![MirStmt::Local {
                mutable: false,
                name: "answer".to_string(),
                ty: Some("Int".to_string()),
                value: MirExpr::Int("42".to_string()),
            }],
        }],
    };

    let diagnostics = mir::verify(&program).expect_err("missing return should fail");
    assert_eq!(diagnostics[0].code, "MIR_MISSING_RETURN");
}

#[test]
fn verifier_accepts_if_else_when_both_paths_return() {
    let program = Program {
        enums: vec![],
        functions: vec![MirFunction {
            reloadable: false,
            type_params: vec![],
            name: "main".to_string(),
            params: vec![],
            return_type: Some("Int".to_string()),
            body: vec![MirStmt::If {
                condition: MirExpr::Bool(true),
                then_body: vec![MirStmt::Return(Some(MirExpr::Int("42".to_string())))],
                else_body: vec![MirStmt::Return(Some(MirExpr::Int("0".to_string())))],
            }],
        }],
    };

    mir::verify(&program).expect("if/else with returns should verify");
}

#[test]
fn verifier_rejects_if_without_returning_else_path() {
    let program = Program {
        enums: vec![],
        functions: vec![MirFunction {
            reloadable: false,
            type_params: vec![],
            name: "main".to_string(),
            params: vec![],
            return_type: Some("Int".to_string()),
            body: vec![MirStmt::If {
                condition: MirExpr::Bool(true),
                then_body: vec![MirStmt::Return(Some(MirExpr::Int("42".to_string())))],
                else_body: vec![],
            }],
        }],
    };

    let diagnostics = mir::verify(&program).expect_err("partial return should fail");
    assert!(
        diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "MIR_MISSING_RETURN"),
        "diagnostics: {diagnostics:?}"
    );
}

#[test]
fn verifier_accepts_match_when_all_arms_return_and_wildcard_covers_default() {
    let program = Program {
        enums: vec![],
        functions: vec![MirFunction {
            reloadable: false,
            type_params: vec![],
            name: "main".to_string(),
            params: vec![],
            return_type: Some("Int".to_string()),
            body: vec![MirStmt::Match {
                value: MirExpr::Int("1".to_string()),
                arms: vec![
                    MirMatchArm {
                        pattern: MirPattern::Int("1".to_string()),
                        body: vec![MirStmt::Return(Some(MirExpr::Int("42".to_string())))],
                    },
                    MirMatchArm {
                        pattern: MirPattern::Wildcard,
                        body: vec![MirStmt::Return(Some(MirExpr::Int("0".to_string())))],
                    },
                ],
            }],
        }],
    };

    mir::verify(&program).expect("covered match with returns should verify");
}

#[test]
fn verifier_accepts_exhaustive_enum_match_when_all_arms_return() {
    let program = Program {
        enums: vec![MirEnum {
            name: "ResultTag".to_string(),
            type_params: vec![],
            variants: vec![
                MirEnumVariant {
                    name: "Ok".to_string(),
                    payload_type: None,
                },
                MirEnumVariant {
                    name: "Err".to_string(),
                    payload_type: None,
                },
            ],
        }],
        functions: vec![MirFunction {
            reloadable: false,
            type_params: vec![],
            name: "main".to_string(),
            params: vec![],
            return_type: Some("Int".to_string()),
            body: vec![MirStmt::Match {
                value: MirExpr::EnumVariant {
                    enum_name: "ResultTag".to_string(),
                    variant: "Ok".to_string(),
                    payload: None,
                },
                arms: vec![
                    MirMatchArm {
                        pattern: MirPattern::Variant {
                            enum_name: "ResultTag".to_string(),
                            variant: "Ok".to_string(),
                            binding: None,
                        },
                        body: vec![MirStmt::Return(Some(MirExpr::Int("42".to_string())))],
                    },
                    MirMatchArm {
                        pattern: MirPattern::Variant {
                            enum_name: "ResultTag".to_string(),
                            variant: "Err".to_string(),
                            binding: None,
                        },
                        body: vec![MirStmt::Return(Some(MirExpr::Int("0".to_string())))],
                    },
                ],
            }],
        }],
    };

    mir::verify(&program).expect("exhaustive enum match with returns should verify");
}

#[test]
fn verifier_accepts_exhaustive_bool_match_when_all_arms_return() {
    let program = Program {
        enums: vec![],
        functions: vec![MirFunction {
            reloadable: false,
            type_params: vec![],
            name: "main".to_string(),
            params: vec![],
            return_type: Some("Int".to_string()),
            body: vec![MirStmt::Match {
                value: MirExpr::Bool(true),
                arms: vec![
                    MirMatchArm {
                        pattern: MirPattern::Bool(true),
                        body: vec![MirStmt::Return(Some(MirExpr::Int("42".to_string())))],
                    },
                    MirMatchArm {
                        pattern: MirPattern::Bool(false),
                        body: vec![MirStmt::Return(Some(MirExpr::Int("0".to_string())))],
                    },
                ],
            }],
        }],
    };

    mir::verify(&program).expect("exhaustive Bool match with returns should verify");
}

#[test]
fn verifier_rejects_non_exhaustive_enum_match_without_trailing_return() {
    let program = Program {
        enums: vec![MirEnum {
            name: "ResultTag".to_string(),
            type_params: vec![],
            variants: vec![
                MirEnumVariant {
                    name: "Ok".to_string(),
                    payload_type: None,
                },
                MirEnumVariant {
                    name: "Err".to_string(),
                    payload_type: None,
                },
            ],
        }],
        functions: vec![MirFunction {
            reloadable: false,
            type_params: vec![],
            name: "main".to_string(),
            params: vec![],
            return_type: Some("Int".to_string()),
            body: vec![MirStmt::Match {
                value: MirExpr::EnumVariant {
                    enum_name: "ResultTag".to_string(),
                    variant: "Ok".to_string(),
                    payload: None,
                },
                arms: vec![MirMatchArm {
                    pattern: MirPattern::Variant {
                        enum_name: "ResultTag".to_string(),
                        variant: "Ok".to_string(),
                        binding: None,
                    },
                    body: vec![MirStmt::Return(Some(MirExpr::Int("42".to_string())))],
                }],
            }],
        }],
    };

    let diagnostics = mir::verify(&program).expect_err("partial enum match should fail");
    assert!(
        diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "MIR_MISSING_RETURN"),
        "diagnostics: {diagnostics:?}"
    );
}
