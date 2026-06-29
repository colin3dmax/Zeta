# Zeta 交接文档(2026-06-23)

> ⚠️ **当前方向(2026-06-23,最新):专注打磨 Zeta 语言本身,不再做操作系统。** 用户已明确推翻下文 §6 的"OS 北极星"定调 —— 不要主动推进 `kernel/`、抢占式调度、freestanding/并发等 OS 专属工作。§6 仅作既有成果历史记录。语言打磨候选见 §0.5 末尾。详见 memory `focus-language-not-os`。

> 跨会话接续的**权威入口**。新会话先读本文 + `~/.claude/projects/-Users-colin-Work-Zeta/memory/MEMORY.md`(自动加载的索引)。
> 详细分项见 memory 下 os-direction / language-features / trait-system / native-backend-progress / self-hosting-progress / feature-backport-selfhost。
> **最近一个大会话(2026-06-22~23,已全部 push 至 origin/main,~29 commits,HEAD `ab8a602`)分两段**:
> **(A) 用 Zeta 写操作系统**(`kernel/`,全部实机 QEMU riscv64 验证):最小闭环 → freestanding 运行时桩(去 snprintf)→ 真 NS16550 UART → 宽度 MMIO → 裸指针 `*T` → 可回收 allocator → 内联汇编(csr/wfi)→ 定时器中断(trap stub + CLINT)→ extern FFI → **协作式调度器**(switch_context 上下文切换 + 两任务交替)。**Zeta OS 已真多任务。**
> **(B) 把语言搞扎实**(审计真实程序补正确性/可用性缺口,见 §0.5)。

---

## 0. 一句话状态
Zeta 是一门**能自举、有三后端(解释器 / LLVM native / WASM)、内存自动管理(值语义 + 确定性 Drop + COW + move-on-last-use)** 的真实语言,跑 **880+ 测试全绿**,自编译**字节级一致(fixpoint 4/4)**。能舒服地写真实程序(FizzBuzz/递归/容器/闭包/print 都顺)。
**且已能编出在裸机 riscv64 上多任务的操作系统(`kernel/`)。** 语言现状见 §0.5/§1,OS 路线见 §6。

---

## 0.5 语言能力(含 2026-06-23 打磨)
**核心**:标量(Int/Float/Bool/String)、数组/元组/struct/enum/闭包、泛型单态化、错误处理 `?`、`import`/`module`、`Option`/`Result`。
**trait 完整**:trait/impl + UFCS 派发 + 泛型约束 `<T:Show>`;**方法调用语法 `x.f(a)`≡`f(x,a)`**(`902d69b`,消歧=路径 root 是局部);**运算符重载 `a OP b`→`op$Type`**(`f17017a`,非标量派发)。
**泛型容器**:`Array<T>` + HashMap/HashSet + **std.collections 源码注入模块**(import 即得)。
**系统能力**:裸指针 `*T`(`afcd901`)+ 宽度 MMIO + 内联汇编(csr/wfi)+ **extern FFI**(`8026f58`)。
**2026-06-23 打磨(均 fixpoint + 全套件 + 差分)**:
- **字符串转义补全**(`8a395b6`):`\t`/`\r`/`\0`/`\\`(此前只 `\"`/`\n`);两个自举前端 normalize_string_escapes 同步(byte 116/114/48)保 parity。
- **stdout `print`/`println`**(`45888db`):std.io,libc write(放 std.io 避与内核 `fn print` 撞名)。
- **native 支持 string 模式匹配**(`46e8766`):新增 `lower_string_match` 顺序 string_eq 链(switch 无法 switch 字符串)。
- **修 Name catch-all 返回分析**(`46e8766`):`match n { 1->.., other->.. }` 全臂返回不再误报 MIR_MISSING_RETURN(mir.rs:1292 把 `Name(_)` 与 `Wildcard` 并列)。
- **闭包参数推断 `|x|`**(`ab8a602`):desugar `infer_lambda_param_types` 从 `let f: fn(Int)->R = |x|...` 注解填类型;填充在 typecheck 前 ⇒ dump 等价 ⇒ parity 零风险。
- **用户函数遮蔽同名 std 内置**(`403de4b`,**修内核回归**):print/println(`45888db`)其实没有 import gating —— std 内置派发只按名字,无视用户是否定义同名 `fn`。于是内核自定义 UART `fn print` 被 std.io libc-`write` 版遮蔽 ⇒ freestanding 链接缺 `write`,`kernel/build.sh` 直接挂。修法:**用户定义优先**,三处一致(codegen `MirExpr::Call`、runtime MIR/AST 两路径),都加 `!functions.contains_key(callee)` 精确 gating ⇒ 只有真重定义才遮蔽,真内置(string_len 等前端不重定义)与 fixpoint 零变化。回归测试 `user_defined_print_shadows_std_builtin`。**经验:新增内置时务必意识到它会无条件抢占同名用户函数,除非用户已定义。**
- **match 守卫 `pat if <cond> ->`**(完成):臂仅在 pattern 匹配**且** guard 为真时命中,否则 fall through 下一臂。支持 Int/Bool/enum/String 标量与 enum payload 绑定(guard 可引用绑定,如 `Some(n) if n > 0`)。**穷尽性**:带 guard 的臂(含 `_ if c`)**不计覆盖**(可能失败),故仍需 plain catch-all —— typecheck 与 mir verifier 两处都按 `guard.is_none()` 过滤。**codegen**:有 guard 时整个 match 退化为顺序 test→guard→body 链(`lower_guarded_match`,复用抽出的 `bind_arm_pattern`);无 guard 仍走 `switch` ⇒ arena_frontend 不受影响 ⇒ **fixpoint 零风险**。**parity 安全**靠无 guard 时 AST/HIR/MIR dump 字节不变。解释器(MIR+AST 两路径)pattern 匹配后求值 guard,假则恢复绑定 continue。move 分析 guard 读以 `mark=false` 计活跃(false guard fall through 不能消费后续臂要的值)。**已知小限**:guarded-false 路径上对 String/Array/Struct 绑定的 clone 会泄漏(Int/标量 guard 无 clone 不泄漏;ASan 证无 double-free)。7 个差分测试 `tests/codegen_match_guard.rs`。
- **`std.strings` 源码注入模块 + `string_split`**(完成):`import std.strings;` 注入 `src/std/strings.zeta`(纯 Zeta,基于 std.core 的 `string_index_of`/`string_byte_slice`/`string_array_empty`/`string_array_push` 组合)。**纯 Zeta ⇒ 像用户代码一样 lower ⇒ 三后端天然一致、零 codegen/runtime intrinsic、fixpoint 安全**(arena_frontend 不 import ⇒ 不注入)。契约:`"a,,b"/","→["a","","b"]`、`""/","→[""]`、无分隔符→整串、空分隔符→整串(0 宽不前进)。接入照 std.collections 模式:`std_api` 加 `STANDARD_IMPORTS` 项 + `is_std_strings_import` + `std_prelude::inject`(resolver 经 `is_standard_import` 自动放行)。7 个差分测试 `tests/codegen_strings.rs` + ASan 干净。**经验:加源码 std 模块只改三处(std_api 导入表/谓词 + std_prelude inject + 新 .zeta),无须动 codegen——优先用此法补 stdlib 广度,而非新 intrinsic。**
**剩余可选项(均"中大"级,语言核心已扎实)**:块体闭包 `|x| { stmts }`(AST/dump 改动有 parity 成本)、更多 stdlib 广度(string_trim_*/join 等可照 std.strings 续写)。

## 1. 实现盘点(2026-06-21)

| 维度 | 完成度 | 说明 |
|---|---|---|
| 编译器管线 | ~95% | lex→parse→resolve→HIR→typecheck→desugar→MIR→{解释器/native/wasm};自举闭环 |
| 核心语言 | ~91% | 标量/数组/元组/struct/enum/闭包(参数可推断)、泛型单态化、`?`、**方法语法 `x.f()`** + **运算符重载**;match 含 string 模式 + **守卫 `pat if c`**;缺块体闭包 |
| 内存管理 | ~92% | 全聚合值语义 + 确定性 Drop 零泄漏;array/string COW;move-on-last-use;缺 SSO / struct·tuple 大聚合 COW |
| 自举 | ~90% | native emit 全链 + fixpoint;ev_expr 解释器的复合值(Float/Tuple/Closure)推迟;**新语言糖默认只在 Rust 前端(arena 不用 ⇒ fixpoint 安全;自托管一致性是后续 backport)** |
| 类型系统 | ~90% | 泛型单态化 + trait/impl 完整 + `<T:Show>` 约束;缺:trait 默认方法、关联类型、多 trait 对象 |
| 标准库 | ~55% | 字符串(escape 全/to_upper/lower/trim)/数组/整数工具 + **stdout print/println** + 基础文件 IO + 泛型数组 + **std.collections(HashMap/HashSet)** + **std.strings(string_split)**;无网络 |
| 系统能力/FFI | ~60% | **裸指针 `*T` + 宽度 MMIO + 内联汇编(csr/wfi)+ extern FFI 全部完成**;freestanding 后端走 `kernel/build.sh`(emit-ir→clang riscv64);缺并发原语(atomic) |
| OS(`kernel/`) | 真多任务 | 启动→运行时桩→UART→内存管理→中断→**协作式调度器**;全实机 QEMU 验证(详见 §6 + kernel/README.md) |

**语言构造(已实现)**:`if`/`while`/`for in`/C 式 `for`/`match`(+ 通配)/`break`/`continue`/`return`;
算术·位·逻辑·比较·一元;`let`/赋值;数组字面量与索引读写(`a[i]`/`a[i]=v`/`a.len`);元组 `(a,b)`/`.N`;
struct 字面量与字段;enum + payload;闭包 `|x| ...` + 捕获;泛型 `<T>`(函数 + struct/enum,native 单态化);
`import`/`module`;`Option`/`Result` + `?`。
**trait/impl(切片①②已实现)**:`trait Show { fn show(self: Self) -> String; }` / `impl Show for Point { ... }`;UFCS 自由函数派发 `show(p)` 按接收者具体类型路由到 `show$Point`。`trait`/`impl` 是上下文标识符(非保留字,fixpoint 安全);impl 在 desugar 展平为 mangled 自由函数(`Self`→target),三后端当普通函数处理;调用按首参类型在 typecheck(Self 作类型参数宽松检查)/解释器(运行时值类型)/native(ZType base)三处派发,差分一致。
**方法调用语法(`902d69b` 已实现)**:`x.f(args)` ≡ `f(x, args)`,复用 UFCS/trait 派发。消歧规则:`a.b(..)` 是方法当且仅当路径 ROOT 是作用域内局部(枚举 `Type.Variant`/模块 `demo.math.fn` 的 root 不是局部)。parser 拦截非名字路径接收者(`xs[1].m()`),作用域感知的 `desugar::desugar_method_calls` 处理名字路径(仅单文件路径跑;module_graph 不跑)。
**运算符重载(`f17017a` 已实现)**:非标量(struct/enum)操作数的 `a OP b` 派发到 `op$Type` trait 方法(`+`→add/`-`→sub/`==`→eq/`<`→lt 等,`mir::operator_trait_method`;排除 &&/||)。标量走内置快路径;仅在存在 `op$Base` 方法时派发 ⇒ 纯增量。五处接入:typecheck/mir verify/解释器/codegen/helper。
**extern FFI(已实现)**:`extern fn name(..) -> ..;` —— C ABI 外部函数声明(无 body,链接器解析)。native-only(解释器拒绝)。详见 §6.2。
**未实现**:并发原语(调度器现已可做,FFI 解锁);可选:trait 默认方法/关联类型、string_split 等 stdlib 广度。

**后端**:解释器 `run_mir`(差分 oracle);native MIR→LLVM22(inkwell,122 codegen fn,JIT + AOT 独立二进制 + 热替换);WASM(浏览器 playground)。

**自举**:`testdata/selfhost/arena_frontend.zeta`(**11,533 行手写 Zeta**)实现整条前端;Stage2 达成(被自己的 codegen 编 native,与解释器逐字节对齐);**fixpoint 自编译字节一致**。

**stdlib(std.core/std.io)**:string_len/byte_at/byte_slice/concat、int_to_string、**string_to_int**、int_abs/min/max、**int_pow**、string_index_of/contains/repeat、ascii 谓词、{int,bool,string,float}_array_empty/push、file_read_to_string、path_join/basename、diagnostic_format。

---

## 2. 构建 / 测试命令
```bash
RUST_MIN_STACK=67108864 cargo test                  # 非 llvm(快);大栈避开已知 debug flake(见下)
LLVM_SYS_221_PREFIX=/opt/homebrew/opt/llvm cargo test --release --features llvm          # native 全套件(LLVM 22,~880 测试)
LLVM_SYS_221_PREFIX=/opt/homebrew/opt/llvm cargo test --release --features llvm --test selfhost_fixpoint -- --ignored   # 自举门禁 ~130s,改前端/lowering 必跑
# 内核(裸机 riscv64):需 qemu-system-riscv64 + brew llvm clang + /usr/local/bin/ld.lld
bash kernel/build.sh && bash kernel/run.sh          # 构建并 QEMU 启动多任务内核(Ctrl-A X 退出)
cargo run --quiet --release --bin zeta -- run X.zeta   # 跑一个 .zeta(解释器);emit-ir / check 子命令亦在
```
- **加内置改四处**:std_api 签名 + runtime(is_std_builtin+eval)+ mir.rs 内置类型表(最易漏)+ codegen lower_builtin;新名先 `grep "fn <name>" testdata/selfhost/*.zeta` 防撞自举前端(撞名破 fixpoint)。
- **ASan 验证生成代码无堆错误**(诊断内存问题的利器):
  ```bash
  # 用 emit_llvm_ir 出 .ll → sed 改 @main→@zmain → clang -fsanitize=address 链一个调 zmain 的 driver → 跑
  /opt/homebrew/opt/llvm/bin/clang -fsanitize=address -g x.ll drv.c -o x && ./x
  ```
- **预存坑**:`selfhost_arena`/`selfhost_mir` 的 `all_stage1_parity_probes` mega-test 在 debug build 栈溢出(超大输入 + 深递归),与改动无关;release 正常。
- **并行 harness 偶发 SIGSEGV 先怀疑栈溢出**,不要先假设堆错误:`RUST_MIN_STACK=67108864 cargo test ...` 一测便知(见 §5 教训)。

---

## 3. 本会话完成(2026-06-21):内存管理彻底完成 + COW + stdlib

按提交顺序(均过 fixpoint 4/4 + ASan + 全套件):

| commit | 内容 |
|---|---|
| `849e8a3` | **struct 值语义 Drop**(needs_drop 按字段递归)+ 修自举测试栈溢出 flake |
| `c5a55d4` | **enum payload Drop/Clone**(tag-switch;装箱 struct payload;match 绑定统一 clone 杜绝别名 double-free) |
| `8c24ad1` | **closure env Drop/Clone**(fat-closure `{fn,env,drop_thunk,clone_thunk}` + per-lambda thunk) |
| `d1e391e` | **数组 COW**(buffer 头 `{cap}`→`{cap,rc}` 16B;clone=rc++、drop=rc--free-if-0;就地变异前 `cow_make_unique`) |
| `c49b2da` | **字符串 COW**(不可变⇒纯 refcount;堆串 8B rc 头;**字面量发射为带哨兵 rc=i64::MIN 的全局**,clone/drop 跳过) |
| `6733522` | **stdlib 6 内置**:int_abs/min/max + string_index_of/contains/repeat |
| `1c64865` | handoff 刷新(= 本文上一版) |

**内存模型(已敲定并实现)**:**值语义 + 编译器自动 Drop/COW**(学 Swift 值类型 + COW、Nim ARC;**不**搬 Rust 借用检查器,避开新手陡峭)。
- 生成式 per-type 递归 `@__drop_T`/`@__clone_T`(缓存,先于函数体插缓存以支持递归类型;**对数据递归而非类型结构 ⇒ 无 codegen 栈溢出**)。
- 全聚合(array/string/tuple/struct/enum/closure)零泄漏。
- COW:array(标量元素 rc 共享 + 写时拷)、string(不可变纯 rc);**共享传值 O(n)→O(1)**(微基准:4000 元素数组传值 10 万次 65ms→1ms;4096 字符串 9ms→2ms);非共享负载中性。
- 值语义 ⇒ 无环 ⇒ 连环收集器都不需要。

---

## 4. 历史已完成(压缩)

- **P1–P4 语言扩展**(Rust 前端 + native):Float(f64)、Tuple、Closure(lift+heap env)、Generics(按需单态化 `id$Int`)。
- **泛型 struct/enum**(389405d 语言层 + eba1963 阶段A 值流推断 + 0ecd7cd 阶段B parser 保留实参全链):`Box<T>`/`Option<T>`/`Result<T,E>` native 单态化。`ZType::Struct/Enum` 用 mangled 名编码实参;enum 沿用统一 `{tag,p0,p1}`。
- **错误处理全链(#75)**:内置泛型 `Option`/`Result`(std.core 仅引用时注入)+ `?` 运算符(`src/desugar.rs` pre-resolve 续延脱糖,无新 codegen)+ typecheck `Type::Generic` 保留实参(unwrap 值 `v+1` 可算术;真多态 `f<T>(x){x+1}` 仍拒,保安全)。
- **自举回灌**:Float/Tuple/Generics/Closure native emit 全链回灌进 arena_frontend.zeta(各过 fixpoint)。ev_expr 解释器的复合值推迟(值系统无 f64/复合槽,次要路径)。
- **热重载 / REPL / 模块系统 / 诊断(golden)/ 官网**(zeta.jennieapp.com:playground/tutorial/spec)。

---

## 5. 关键经验 / 坑(务必先读)

1. **fixpoint 是硬门禁**:任何动到前端的改动必跑 `selfhost_fixpoint --ignored`。新特性的 emit 路径通常 fixpoint-safe(arena_frontend 不用新特性 → 其输出不变);但**改了既有内置/类型的 lowering 就危险**。
2. **加内置要改四处**:`std_api.rs`(签名)+ `runtime.rs`(`is_std_builtin` + `eval_std_builtin`)+ **`mir.rs` 内置类型表(最易漏)** + `codegen.rs`(`lower_builtin`)。漏 mir 表 → `MIR_UNKNOWN_FUNCTION`。
3. **新内置名先 grep 自举前端撞名**:`grep "fn <name>" testdata/selfhost/*.zeta`。撞名(如 `string_starts_with`,前端已自定义)会改变前端该 call 的 lowering → **破坏 fixpoint**,必须排除。
4. **加标量/复合类型要改两个类型检查器**:`typecheck.rs` + `mir.rs` verifier,外加解释器(MIR/AST eval)、module_graph、native codegen。
5. **并行 harness 偶发 SEGV = 栈溢出,不是堆错误**(本会话曾把它误诊为 struct double-free 折腾很久):~1 万行合并前端深递归(lower/run_mir/codegen)逼近 2 MiB 测试线程栈。修法:测试在 64 MiB 大栈线程跑(见 `tests/codegen_selfhost_run.rs`)。ASan 可证明生成代码无堆错误。
6. **差分 oracle 纪律**:每个 native 行为必须与解释器逐位一致。新内置在 runtime 与 codegen 用**同一算法**(如 `string_index_of` ↔ `runtime::byte_index_of` 逐字节对齐)。
7. **COW 实现要点**:array 头 16B(cap@0,rc@8,data@16,free 目标 data-16);string 头 8B(rc@-8,data,free data-8);**字符串字面量是全局常量**,用哨兵 rc=i64::MIN 让 clone/drop 跳过(永不 free)。就地变异(`a[i]=`、就地 push)前必须 `cow_make_unique`(rc>1 则深拷),否则别名破坏值语义。
8. **trait 实现脉络(切片①②,供切片③接续)**:核心模型 = impl 在 **desugar `flatten_impls`** 展平为 mangled 自由函数 `dispatch_name(method, base)`=`method$Base`(`subst_self` 把 `Self`→target),三后端当普通函数处理。派发判断靠 trait 方法名集合:`Module::trait_method_names()`(ast.rs)→ resolver `functions` 集 / typecheck `function_signatures`(注册时 `Self` 列为类型参数,wildcard)/ `mir::Program.trait_methods`(verifier + 解释器 + codegen 读它)。派发点三处:解释器 `runtime.rs` MIR 路径(`value_type_base` 取运行时值类型→`run_function_with_args`)、native `codegen.rs` `lower_trait_dispatch_call`(`zty_base_name` 取 ZType base)、verifier 宽松返回 Unknown。**`value_type_base`↔`zty_base_name` 必须一致**(差分 oracle);已知 gap:codegen 无 Bool ZType(折叠为 Int),故 bool 接收者 native 不派发。**AST/REPL 的 `Runtime`(第二个 Call eval 路径)未接 trait,仅 MIR 解释器是 oracle**。切片③:在 `lower_generic_call` 单态化时,泛型函数体内对 trait 方法的调用按已知 subst 的具体 ZType 路由(目前泛型体内的 `show(x)` 中 x:T 是 type_param,ZType 未知 → 需用 subst 解析后再 `dispatch_name`)。
9. **泛型实现(阶段B)**:类型字符串贯穿全链(parser 产 `Result<Int, String>`);typecheck `Type::Generic(base,args)` 保留实参、match/字段访问代入;mir verifier `parse_mir_type` strip 到 base(type_param 当通配符,lenient);codegen `resolve_ann_ztype` 读实参单态化。`src/type_syntax.rs`(tuple_parts/fn_parts/split_top_level)是结构化类型串解析基础。codegen 按名查 struct/enum 时串可能带 `<...>`,先 `type_syntax::base_name`。

---

## 6. 下一步(2026-06-22 起按 OS 目标重排)

**新北极星:用 Zeta 打磨一个操作系统。** 这意味着以下"系统能力缺口"优先于继续堆 stdlib 广度。

### 6.1 OS 第一前置:freestanding + 裸机后端(最大缺口)
**✅ 最小闭环已打通(2026-06-22,`kernel/`)**:Zeta 写的 `kmain` 编成 freestanding riscv64 ELF,QEMU `virt` 裸机启动,经 MMIO(UART@0x10000000)打印 `Zeta OS: hello from bare-metal riscv64!` 后自旋。链路 = `zeta emit-ir`(新 CLI 子命令)→ 去掉 host datalayout/triple → `clang --target=riscv64 -mcmodel=medany -nostdlib` → `boot.s` 设栈调 main → `ld.lld -T kernel.ld`(载入 0x80000000)。唯一新语言原语:`mmio_write_byte`/`mmio_read_byte`(volatile i8 store/load 到 inttoptr 地址;解释器侧 inert)。
**✅ 运行时桩已完成(2026-06-22,`08b6168`)**:内核现可用完整类型系统(堆 String/数组/struct)。
- `kernel/runtime.c`:freestanding `malloc`(bump arena 4MiB)/`free`(no-op)/`memcpy`/`memset`/`memcmp`(字节循环,**-O0 编译**否则 loop-idiom 把循环重写成 memcpy/memset 自调用→递归)。
- `snprintf` 已从 native 后端彻底删除:codegen 改用自包含 `gen_int_to_string`(无符号幅值处理 i64::MIN;两遍数位计数+倒填)。现在裸机只需 malloc/free/memcpy/memcmp/memset 五个符号(全在 runtime.c)。
- 实测 QEMU 输出 string_concat/int_to_string(含负数)/数组求和全正确。
**OS 路线进度**:
- ✅ **#1 裸指针 `*T` + 真 UART 驱动**(`6103e89`/`afcd901`,见 §6.2)。
- ✅ **#2 可回收 allocator**(`792e496`):kernel/runtime.c bump→带合并的 first-fit 空闲链表;实机 20 万次 alloc/free 不耗尽 arena。经验:中间结果须绑成局部才会被 drop/free,否则泄漏 + O(n) 合并退化 O(n²)。
- **#3 固化 AOT 装配**(待办,低优先):把 build.sh 的手动 emit-ir→strip→clang riscv64 固化成 `zeta` 原生跨目标 obj 输出。价值有限(只省一次 clang 调用;链接/boot.s/runtime.c/链接脚本仍需外部工具链)。
- **#4 前置 ✅ 内联汇编**(csr_read/write/set/clear + wfi)。
- **#4 ✅ 定时器中断完成**(`d9d5a23`):boot.s 加 `trap_entry` 汇编 stub(.align 4,保存 16 caller-saved 寄存器 → call Zeta `trap_handler` → mret),_start 里 csrw mtvec 安装。Zeta `trap_handler`(无分配,中断安全)在固定 scratch RAM 做计数器(因无全局变量)+ 重设 mtimecmp。main 装 CLINT mtimecmp + 开 mie.MTIE/mstatus.MIE。实测 tick 1/2/3 "traps work"。**这是调度器的地基。**
- **#4 ✅ 协作式调度器完成**(`5472017`,建立在 extern FFI 上):boot.s `switch_context`(保存/恢复 14 个 callee-saved 寄存器)+ `task_trampoline` + `trampoline_addr`(extern);kmain 上下文/栈在固定 RAM(无全局),task_init 种新上下文,两任务 yield + run_task 派发,main round-robin。实测 task A/B step 1/2/3 完美交错从断点恢复。**Zeta OS 现在真多任务。**
- **#4 剩余(可选):抢占式调度**:在 trap_handler(定时器中断)里调 switch_context 换任务(而非任务主动 yield)。地基都已具备(定时器中断 + 上下文切换),把 trap_handler 改成保存当前任务上下文 + 选下一个 + 切换即可。

### 6.2 OS 第二前置:裸指针 + volatile MMIO + 内联汇编(DevGame #78 扩展)
- **✅ volatile MMIO**(`d80a208`/`7ecb785`):`mmio_{read,write}_{byte,word,dword}`(8/16... 即 8/32/64 位)volatile load/store 到 inttoptr;真 NS16550 UART 驱动(`6103e89`,init+LSR 轮询)。
- **✅ 裸指针 `*T`**(`afcd901`,unsafe/native-only):`*T` 前缀语法 + Type::Ptr/ZType::Ptr 全栈;内置 `ptr_from_addr`/`ptr_addr`/`ptr_read`(支持整 struct)/`ptr_write`/`ptr_offset`(按元素步长)+ `array_data_addr`(数组缓冲区地址,DMA/测试)。typecheck 宽松兼容(任意 *A≈*B);解释器 inert ⇒ native/实机验证(tests/codegen_pointer.rs + kernel 实测 1337)。`ptr_from_addr` 元素由 let 注解 `*T` 精化(codegen Local 读注解)。
- **✅ 内联汇编**(`afcd901` 后续提交,#4 前置完成):`csr_read`/`csr_write`/`csr_set`/`csr_clear`(CSR 号须为 Int 字面量,烤进指令)+ `wfi`。codegen 经 inkwell `create_inline_asm` + `build_indirect_call` 发 LLVM inline asm;riscv-only(host JIT 不能跑,解释器 inert)。实机:读 mhartid=0、mscratch 写读往返=31337。
- **✅ C ABI FFI**(`extern fn` 已实现):`extern fn name(..) -> ..;`(无 body,`extern` 上下文标识符,parser `consume_contextual_before_fn`)。codegen 只声明不生成 body(pass2 跳过 is_extern),链接器/JIT 解析符号;mir verify 跳过 extern;解释器拒绝 extern 调用(native-only)。实测:JIT 调 libc labs/llabs;内核经 C ABI 调 boot.s 的 asm_add3(100,20,3)=123。**这解锁了调度器**(Zeta 可调汇编 switch_context)。

### 6.3 OS 第三前置:并发原语(DevGame #77)
- 原子操作(LLVM atomicrmw/cmpxchg)、内存屏障 → 自旋锁;之上做调度器/中断安全。

### 6.4 仍有价值但降级为"按需"
- **标准库广度**:`string_split`(返回 Array<String>)、把更多容器进 std.collections、`Set` 已随 #3 提供。codegen 循环类内置镜像 `gen_string_to_int`/`gen_trim`。
- **性能**:~~move-on-last-use~~ **已完成(#4,`5c2cc09`)**;剩 SSO 小字符串内联、struct/tuple 大聚合 COW。
- **trait 增强**:默认方法、关联类型。
- ev_expr 解释器补复合值;官网重部署(`tools/deploy-website.sh`)。

### 6.5 关键提醒(本会话新增经验)
- **加内置仍是四处**(std_api + runtime + mir 表 + codegen),见 §5.2。
- **std.collections 是源码注入模块**(非 intrinsic):`src/std_prelude.rs` 在 parse_source / module_graph 两处把 `src/std/collections.zeta` 的 items 前置拼接;新增源码标准模块照此扩。
- **move-on-last-use 仅在 codegen**(`src/move_analysis.rs`,cfg llvm):不入 dump_mir ⇒ fixpoint 天然安全;含 break/continue 的函数整体禁用;moved-flag 守卫所有 drop 点。改 drop/clone/move 机制后**必跑 ASan + selfhost_run + fixpoint**。
- **单态化能从 struct 类型实参推断类型参数了**(`7d23b4a`:`unify_ztype` 加泛型 struct/enum 分支 + `Types.instances` 反查表)——容器删除类操作(`remove<K,V>(m: HashMap<K,V>, key: K)`,V 仅在 struct 类型位)现可单态化。

---

## 7. DevGame 访问
项目级 `./.devgame/token.json` 已过期;用**全局** `~/.devgame/token.json`(有效)。访问国内链路必须关代理:
```bash
GT=$(node -e "process.stdout.write(require(require('os').homedir()+'/.devgame/token.json').token)")
env -u http_proxy -u https_proxy -u all_proxy -u HTTP_PROXY -u HTTPS_PROXY -u ALL_PROXY \
  API_BASE=https://devgame.jennieapp.com PROJECT=zeta DEVGAME_TOKEN="$GT" \
  node ~/.devgame/bin/update-task.mjs --id <N> --comment "..."   # 创建:--create --subject .. --description-file ..
```
