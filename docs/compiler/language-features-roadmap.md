# Zeta 语言扩展路线图(自举闭环之后的新方向)

> 状态基准:2026-06-15。自举(M0–M7)、hot-reload、native 后端、Stage2(Zeta 自带 codegen + 独立 zetac 二进制)均已完成。本文规划在自举子集之上扩展语言的**大型新特性**。

## 0. 统一方法论(每个特性都遵守)

每个特性按**全链路 + 差分验证**推进,沿用项目一贯方法:

1. **Stage0 语言层**(Rust 参考实现):lexer → parser → AST → resolve → typecheck → MIR → runtime 解释器。差分门禁 = `ast-dump`/`mir-dump` 自洽 + `run_mir` 结果符合预期(新特性不进 stage1_parity,因自举前端暂不支持)。
2. **native codegen**(Rust inkwell,`src/codegen.rs`):差分对齐解释器 `run_mir`。
3. **(可选,后置)自举前端 + Stage2 发射器**:让 Zeta 写的前端也支持该特性 → 自举重新覆盖。

每特性独立干净提交;关键易错点亲自 review;先测量后优化。

## 1. 排期(基础性 × 可行性)

| 阶段 | 特性 | 规模 | 理由 |
|---|---|---|---|
| **P1** | **Float**(f64 标量) | 中 | **✅ 完成**(1a 语言层 11 用例 + 1b native codegen 6 用例,差分对齐解释器) |
| **P2** | **Tuple**(`(a,b)`/`t.0`) | 中 | **✅ 完成**(2a 语言层 9 用例 + 2b native codegen 6 用例;类型推断,暂无 tuple 注解) |
| **P3** | **Closure**(`\|x\| e` + 捕获) | 大 | 一等函数;需捕获分析 + 闭包转换(fn ptr + env) |
| **P4** | **Generics**(`<T>` 参数多态) | 大 | 复用;需单态化(monomorphization) |

P1/P2 可行性高、价值基础;P3/P4 是大工程(各自独立)。先按序做 P1→P2,再评估 P3/P4。

## 2. P1 — Float 详细计划

**1a Stage0 语言层**:
- `token.rs`/`lexer.rs`:`TokenKind::Float(String)`;`lex_int` 遇 `.`(且非 `..`)发 Float(现有 LEX_FLOAT_UNSUPPORTED 钩子改成发 token)。
- `ast.rs`:`Expr::Float{value,span}`,dump `Float <text>`;`Float` 作类型名。
- `parser.rs`:Float token → `Expr::Float`;`Float` 类型注解。
- `typecheck.rs`:`Float` 类型;Float 算术(+-*/)→Float、比较→Bool;**Int 与 Float 不混用**(无隐式转换);builtin `int_to_float(Int)->Float`、`float_to_int(Float)->Int`。
- `mir.rs`:`MirExpr::Float`;Float 二元 op;mir-dump `const Float <text>`。
- `runtime.rs`:`Value::Float(f64)`;f64 算术/比较(求值时 parse 文本,镜像 Int);`Display`(用 Rust `{}`,确定性);两个转换 builtin。
- 门禁:`tests/` 新建 float 用例(parse+dump 自洽、run_mir 结果)。

**1b native codegen**:`ZType::Float`=f64;字面量;`fadd/fsub/fmul/fdiv`、`fcmp`(o-prefixed)、`sitofp`/`fptosi` 转换。差分对齐 run_mir。

**1c(后置)**:自举前端 + Stage2 发射器支持 Float。

## 3. P2–P4 概要(P1 完成后细化)

- **P2 Tuple ✅**:已完成(commit b0dadbd 语言层 + b9c1f85 native)。parser `(a,b)`(带逗号消歧分组)/`.N` 索引(单 Float token `1.0` 拆成 `.1.0` 两次索引);`Type::Tuple`/`MirType::Tuple`/`Value::Tuple(Rc<Vec>)`;native = LLVM 匿名 struct(insert/extract,镜像 struct)。**暂无 tuple 类型注解**(传参/返回),仅函数内推断;tuple 含 array 字段的深拷贝值语义未特殊处理——均作后续增量。
- **P3 Closure**:parser lambda;捕获分析(自由变量);MIR 闭包转换(env struct + fn);runtime 闭包值 + apply;native fn ptr + env。
- **P4 Generics**:parser `<T>`;typecheck 类型参数 + 实例化;MIR 单态化(每个实例一份);native 各实例独立。
