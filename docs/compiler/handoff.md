# Zeta 工程交接(2026-06-14)

> 给下一段会话的接力文档。三条并行工作线的状态、构建/测试命令、踩坑、下一步规划。
> 详细记忆见 `~/.claude/projects/-Users-colin-Work-Zeta/memory/`(新会话自动加载 MEMORY.md 索引)。

## 1. 三条工作线现状(全部干净全绿、已提交)

| 线 | 状态 | 关键证据 |
|---|---|---|
| **自举(self-hosting)** | M0–M7 slice 1-3 ✅ | Zeta 前端处理自身 7500 行源,ast/resolve/typecheck/mir-dump 四阶段与 Rust oracle 逐字相等(fixpoint 4/4,`#[ignore]` ~50s);M7 slice 3 把解释器 O(n²)→O(n) |
| **hot-reload** | slice 1-3 ✅ | 状态保持热代码交换内核(HotRuntime)+ `zeta serve` 长跑服务+文件 watch + `reloadable fn` 语言构造(粗粒度边界纪律强制) |
| **native 后端(LLVM)** | slice 0-5 + 优化 #1/#2/⑤ + 宽 payload enum ✅ | MIR→LLVM:标量/struct/array/string/enum(含 struct/array payload);**native = C 的 1.04x(同回绕语义)**;NativeService(native step 热替换+状态保持,Int+Array+Struct 状态);**AOT 产独立可执行** |
| **Stage2(Zeta 自带 codegen)** | slice 1-6 ✅(标量/struct/IntArray/string/enum-match/for) | `arena_frontend.zeta` 的 `compile(src,"llvm")` 走 MIR arena 发**LLVM IR 文本**(Rust 端不参与 codegen);clang 编 + C driver 链 + 跑,逐一对齐 `run_mir`。`Tcx` 类型子系统;`%S` struct 聚合;IntArray=`%zarr` + 绑定深拷贝;String=`%zstr` 不可变 + global 字面量 + builtin/memcmp;enum=`%zenum{tag,p0,p1}` + `match`→LLVM `switch`;for-range/in/c→控制流。门禁 `tests/selfhost_llvm.rs`(46 用例,全谱)。**剩余**:StringArray/BoolArray(slice 7)→ AOT 整个前端经 Zeta 自发 IR(capstone) |

**所有 headline 诉求已兑现并实测**:① 媲美 C/C++(1.04x 同语义)② 状态保持热重载(解释器+native)③ release 满速热重载 ④ Zeta→独立二进制(AOT)。**Stage2 起步**:Zeta 编译器自身开始产 native 码(slice 1 标量子集,经 clang 落地、差分对齐解释器)。

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

**Stage2(Zeta 发 LLVM IR → clang)门禁**(需 LLVM,跑 clang):
```sh
LLVM_SYS_221_PREFIX=/opt/homebrew/opt/llvm \
  cargo test --release --features llvm --test selfhost_llvm
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

8. ~~**差分验证 native 跑前端**~~ ✅ **闭环达成**:在前端源后追加 `main()` 调 `compile(src,mode)`,把 dump String 归约成 Int 摘要;解释器 `run_mir` 与 native `jit_run_i64` 跑同一组合程序、摘要相等 → **native 编译的整个前端逐字节复现解释器产物**(ast-dump/mir-dump/typecheck/run 模式,run 经自托管求值器)。门禁 `tests/codegen_selfhost_run.rs`(6 用例)。

9. ~~**AOT 独立 exe 前端**~~ ✅ **达成**:前端 AOT 成 .o(entry=`compile`→`zeta_entry`)+ 极小 C driver(文件 IO shim:读源文件、调 `zeta_entry(ZStr,ZStr)->ZStr`、写 stdout)→ **独立可执行**。前端纯函数(source 当 String 入参),IO 全在 driver;String 跨 FFI 与 NativeArray 同 {i64,ptr} ABI。门禁 `tests/codegen_aot_frontend.rs`(ignored,~11s):四模式 stdout 逐字节对齐解释器。

10. ~~**收尾广度项**~~ ✅ **全部完成**:
   - **String-payload enum**(E2):enum 布局加宽 `{i64 tag, i64 p0, ptr p1}`,Int/Bool/String payload;`tests/codegen_enum.rs`(14 用例)。
   - **NativeStructService**:struct 状态跨 native 热替换,经指针包装器 `__svc_init`/`__svc_step` 绕 per-struct ABI;`tests/codegen_hot_reload.rs`(5 用例)。
   - **数组绑定优化**:新鲜独占 buffer(字面量 / `*_array_*`)绑定时跳过冗余 deep-copy(常数因子;`bind_owned`/`is_fresh_array`)。

11. ~~**动态数组摊还 O(1) push**~~ ✅ **完成**:数组 buffer 加 8 字节容量头(值仍 `{len,ptr}`,ptr 指元素、cap 在 ptr[-8]);`xs=push(xs,v)` 自赋值经 `match_inplace_push`/`lower_inplace_push` 原地变异 + 容量翻倍,O(n²)→摊还 O(1)。值语义唯一所有保证原地安全。`tests/codegen_dynarray.rs`(14 用例,含 500 元素压力 + 值语义独立)。

12. ~~**>16B payload 的 enum(宽 payload)**~~ ✅ **完成(E3)**:enum 仍是 `{i64 tag, i64 p0, ptr p1}` 值类型,**array payload 复用 String 的 `{len,ptr}` split**(p0=len,p1=data,构造/提取各 deep-copy 一次保证值独立);**struct payload(可任意宽,如 24B 的 V3 > 16B inline 槽)堆 box**:构造 `malloc(sizeof) + store`、p1 存指针,match 提取 `load` 回值(by-value 拷贝,与现有 struct 传值语义一致)。codegen 改 `EnumVariant` 构造 + `lower_match` 提取两处。**附带修两个前端 typecheck bug**:enum payload 类型比较(`MIR_ENUM_PAYLOAD_TYPE`)与 match 绑定类型曾用 `MirType::named(payload)`,对 `IntArray` 等数组别名会误判(display 同为 "IntArray" 却 named≠Array),改用 `parse_mir_type` 解析。门禁 `tests/codegen_enum.rs`(20 用例:`struct_payload_*`/`array_payload_*`/`mixed_wide_and_scalar_payloads`,含值独立性)。

**native 后端线全部目标 + 收尾广度 + 性能项 + 宽 payload enum 全部达成。** 已无明确遗留项。

**每步都用解释器 `run_mir` 作差分 oracle**(见 tests/codegen_*.rs 的 `check()` 范式),feature-gated,不影响默认构建。

## 6. 工作方法论(务必延续)

① 一切正确性用 **Rust 解释器/oracle 差分验证**;② **先测量后优化**(本会话靠它避免了"盲加 nsw"破坏回绕语义的错误);③ 每特性**独立干净提交**(message 末尾 Co-Authored-By Claude);④ 关键易错点(borrow/控制流/语义/ABI)**亲自 review 不盲信**;⑤ 对不妥/有成本/方向矛盾处**先主动反馈再动手**;⑥ 整体把控、避免陷入细节、防 AI 幻觉(本会话多次靠"验证而非臆断"纠正过时认知,如 inkwell 版本、arch 不匹配)。
