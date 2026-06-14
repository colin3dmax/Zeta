# Zeta 工程交接(2026-06-14)

> 给下一段会话的接力文档。三条并行工作线的状态、构建/测试命令、踩坑、下一步规划。
> 详细记忆见 `~/.claude/projects/-Users-colin-Work-Zeta/memory/`(新会话自动加载 MEMORY.md 索引)。

## 1. 三条工作线现状(全部干净全绿、已提交)

| 线 | 状态 | 关键证据 |
|---|---|---|
| **自举(self-hosting)** | M0–M7 slice 1-3 ✅ | Zeta 前端处理自身 7500 行源,ast/resolve/typecheck/mir-dump 四阶段与 Rust oracle 逐字相等(fixpoint 4/4,`#[ignore]` ~50s);M7 slice 3 把解释器 O(n²)→O(n) |
| **hot-reload** | slice 1-3 ✅ | 状态保持热代码交换内核(HotRuntime)+ `zeta serve` 长跑服务+文件 watch + `reloadable fn` 语言构造(粗粒度边界纪律强制) |
| **native 后端(LLVM)** | slice 0-5 + 优化 #1/#2/⑤ ✅ | MIR→LLVM:标量/struct/array(值语义);**native = C 的 1.04x(同回绕语义)**;NativeService(native step 热替换+状态保持,Int+Array 状态);**AOT 产独立可执行** |

**所有 headline 诉求已兑现并实测**:① 媲美 C/C++(1.04x 同语义)② 状态保持热重载(解释器+native)③ release 满速热重载 ④ Zeta→独立二进制(AOT)。

## 2. 构建 / 测试命令

**默认(无 LLVM,CI 路径)** —— 29 个测试二进制,不需要任何 LLVM 工具链:
```sh
cargo test --release
```

**native 后端(需 LLVM,在 `llvm` cargo feature 后)**:
```sh
LLVM_SYS_221_PREFIX=/opt/homebrew/opt/llvm \
  cargo test --release --features llvm \
    --test codegen_scalar --test codegen_struct --test codegen_array \
    --test codegen_hot_reload --test codegen_aot
# 性能对比(ignored):
LLVM_SYS_221_PREFIX=/opt/homebrew/opt/llvm \
  cargo test --release --features llvm --test codegen_perf native_vs_c_hot_loop -- --ignored --nocapture
```

**自举 fixpoint capstone(ignored,~50s)**:
```sh
cargo test --release --test selfhost_fixpoint -- --ignored
```

## 3. LLVM 工具链踩坑(务必记住)

- 机器 **arm64**;必须用 **arm64 brew 的 LLVM**:`/opt/homebrew/opt/llvm`(22.1.7)。`/usr/local` 的是 x86_64 Intel brew,链不进 arm64 二进制。
- inkwell **0.9** 已支持 LLVM 22;Cargo feature 用 **`llvm22-1-prefer-dynamic`**(链单个 libLLVM-22.dylib,避开静态 zstd/z3 被 /usr/local x86_64 副本遮蔽)。
- inkwell 0.9 API:`CallSiteValue::try_as_basic_value()` 返回自有 `ValueKind`,取值用 `.basic()` 不是 `.left()`;`BinaryOp/UnaryOp` 从 `crate::ast` 导(mir 私有 re-export)。
- AOT 用 `RelocMode::PIC`(macOS 可执行)。

## 4. 关键文件

- `src/codegen.rs`(`#[cfg(feature="llvm")]`,~900 行):MIR→LLVM codegen + JIT + AOT + NativeService/NativeArrayService。
- `src/runtime.rs`:解释器(差分 oracle)+ HotRuntime/ServiceDriver(hot-reload)+ move-on-last-use liveness。
- `testdata/selfhost/arena_frontend.zeta`(7500 行):Zeta 写的自举前端(lex/parse/resolve/typecheck/MIR/interp + 统一 `compile(source,mode)` driver)。
- `docs/compiler/self-hosting-roadmap.md` / `hot-reload-design.md`(含 §3 性能约束)。

## 5. 下一步规划(剩余 = 广度,enum/match codegen)

native 后端覆盖 Int/Bool/struct/array/**string**(值语义)。要 AOT 编译完整自举前端,还需补:

1. ~~**string codegen**~~ ✅ **已完成(三切片)**:`ZType::Str`=`{i64 len, ptr<i8>}`(复用 array 布局);**string 不可变 → 共享只读 buffer,bind 点无需深拷贝**;字面量(global const)+ `string_len`/`string_byte_at`/`string_byte_slice`/`string_concat`(malloc+memcpy)+ `int_to_string`(libc snprintf)+ `ascii_is_*`(纯 i64 比较)。std builtin 在 `lower_builtin` 拦截。门禁 `tests/codegen_string.rs`(19 用例)。
2. ~~**enum codegen**~~ ✅ **已完成(E1)**:tagged union `{ i64 tag, i64 payload }`(Int/无 payload 变体)。`EnumVariant` 构造;`MirStmt::Match`→`lower_match` 对 i64 scrutinee(enum tag / Int/Bool 值)建 LLVM `switch`,catch-all 作 default、穷尽无 catch-all 时 default=`unreachable`。门禁 `tests/codegen_enum.rs`(11 用例)。
3. ~~**match codegen**~~ ✅ 同上(与 enum 同切片完成)。

4. ~~**for 循环**~~ ✅ **已完成**:`ForRange`/`ForIn`/`ForC` lower 成控制流(像 `while`)。loops 栈改为 `(continue_target, exit)`,for 的 continue 跳自增/step 块以仍推进计数器。门禁 `tests/codegen_for.rs`(12 用例)。

5. ~~**动态数组(IntArray)**~~ ✅ **已完成**:`int_array_empty`/`int_array_push`,沿用 `{len,ptr}` 布局(无 capacity);push 函数式 append(每次 malloc+memcpy,O(n))。门禁 `tests/codegen_dynarray.rs`(8 用例)。

6. ~~**动态数组其余族**~~ ✅ **已完成**:数组操作泛化到任意元素类型(`size_of` 算 stride),`bool_array_*`(=i64)、`string_array_*`(`{len,ptr}` 元素)全通。门禁 `tests/codegen_dynarray.rs`(12 用例)。

7. ~~**试推全前端**~~ ✅ **里程碑达成**:补 String `==`/`!=`(memcmp)+ **块级作用域**(按需分配 local 槽 + 嵌套块 locals 快照/恢复,修同名变量在不相交分支不同类型重声明的槽冲突)后,**整个 `arena_frontend.zeta`(306 函数/20 struct)lower 成 native .o**。门禁 `tests/codegen_frontend_probe.rs`(ignored)。前端不用 enum/match、也没真调文件 IO(path_join 等自实现),故已全覆盖。

**native subset 已能 lower 整个自举前端到 native object。** 剩余:
   - **下一步:差分验证 native 跑前端**——native 编译的前端在某输入上的输出须对齐解释器 dump(真正的闭环正确性证明)。建议写个小 `main()` 调前端管线返回 Int 摘要(或经 FFI 比对 dump String),解释器 vs native 差分。
   - **文件 IO builtin**(`file_read_to_string` 等):若要 native 前端从真实文件读源,需 Rust 侧 extern shim;但前端核心管线是纯函数(吃 String source),可绕过。
   - **String-payload enum**(E2)、NativeService struct 状态:低优先,前端不需要。

**每步都用解释器 `run_mir` 作差分 oracle**(见 tests/codegen_*.rs 的 `check()` 范式),feature-gated,不影响默认构建。

## 6. 工作方法论(务必延续)

① 一切正确性用 **Rust 解释器/oracle 差分验证**;② **先测量后优化**(本会话靠它避免了"盲加 nsw"破坏回绕语义的错误);③ 每特性**独立干净提交**(message 末尾 Co-Authored-By Claude);④ 关键易错点(borrow/控制流/语义/ABI)**亲自 review 不盲信**;⑤ 对不妥/有成本/方向矛盾处**先主动反馈再动手**;⑥ 整体把控、避免陷入细节、防 AI 幻觉(本会话多次靠"验证而非臆断"纠正过时认知,如 inkwell 版本、arch 不匹配)。
