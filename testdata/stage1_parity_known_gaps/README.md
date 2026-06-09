# Stage1 parity 已知边缘 gap

这些 `.zeta` 探针是 Rust `ast-dump` 接受、但 Stage1 自举前端尚未对齐的**边缘语法**。
它们**不在** `tests/stage1_parity.rs` 回归门禁内(那里要求全部 PASS),隔离于此记录待办,
避免对抗性极端构造阻塞主线。修复后应移回 `testdata/stage1_parity/`。

## 当前已知 gap

### 括号 callee / base(dp_17~dp_20)
形如 `(a)(b)`、`(a.b.c)(x, y)`、`(a.b).c(d)`、`(a)(b)[c].d` —— 调用/后缀链的 callee 或
base 自身被括号包裹。Rust parser 把 `(a.b)` 折叠为点路径 callee;Stage1 的
`rust_call_lparen_in_range` 只沿 ident/dot 反向找实参括号,无法识别括号包裹的 callee,
导致整段表达式被静默丢弃。实际代码极少出现(`(a)(b)` 等价于 `a(b)`),优先级低。
