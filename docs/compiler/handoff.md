# Zeta 交接文档(2026-06-22)

> 跨会话接续的**权威入口**。新会话先读本文 + `~/.claude/projects/-Users-colin-Work-Zeta/memory/MEMORY.md`(自动加载的索引)。
> 详细分项见 memory 下 language-features / feature-backport-selfhost / self-hosting-progress / native-backend-progress / trait-system。
> **本会话(stdlib + 性能四连)**:#1 容器广度(Set<T> + HashMap remove/contains + 单态化从 struct 实参推断类型参数 `7d23b4a`)→ #2 字符串内置 to_upper/to_lower/trim(`b397b7a`)→ #3 **HashMap/HashSet 提为可 import 的源码标准模块 std.collections**(`179d014`)→ #4 **通用 move-on-last-use**(MIR 活跃性 + 运行时 moved-flag 路径敏感 drop 抑制,`5c2cc09`)。均过 fixpoint 4/4 + 全套件(855)+ ASan。**未 push。**
> ⚠️ **新方向(用户 2026-06-22 提出)**:后续要用 Zeta **打磨一个操作系统**。这把 FFI/freestanding/裸指针/内联汇编/并发顶到最高优先级 —— 详见 §6 与 memory `os-direction`。

---

## 0. 一句话状态
Zeta 是一门**能自举、有三后端(解释器 / LLVM native / WASM)、内存自动管理(值语义 + 确定性 Drop + COW + move-on-last-use)** 的真实语言,跑 **855 测试全绿**,自编译**字节级一致(fixpoint 4/4)**。
**语言内核成熟、trait/泛型容器/标准库容器(std.collections)齐备;下一阶段目标 = 用它写操作系统,缺口在系统能力:freestanding 后端、裸指针/MMIO/内联汇编、FFI、并发(见 §6)。**

---

## 1. 实现盘点(2026-06-21)

| 维度 | 完成度 | 说明 |
|---|---|---|
| 编译器管线 | ~95% | lex→parse→resolve→HIR→typecheck→desugar→MIR→{解释器/native/wasm};自举闭环 |
| 核心语言 | ~85% | 标量(Int/Float/Bool/String)、数组/元组/struct/enum/闭包、泛型单态化、错误处理 `?` |
| 内存管理 | ~92% | 全聚合值语义 + 确定性 Drop 零泄漏;array/string COW;**move-on-last-use 已完成**(活跃性 + moved-flag);缺 SSO / struct·tuple 大聚合 COW |
| 自举 | ~90% | native emit 全链 + fixpoint;ev_expr 解释器的复合值(Float/Tuple/Closure)推迟 |
| 类型系统 | ~88% | 泛型单态化齐全;**trait/impl 完整完成**(切片①②③:语法 + 具体类型 UFCS 派发 + 泛型多态 + `<T: Show>` 约束校验);缺:trait 默认方法、关联类型、多 trait 对象 |
| 标准库 | ~45% | 字符串(+to_upper/to_lower/trim)/数组/整数工具 + 基础 IO + 泛型数组 + **std.collections 源码模块(import 即得 HashMap<K,V> + HashSet<T>)**;无网络/string_split |
| 并发 | 0% | 无语言级并发(DevGame #77;**OS 调度器前置**) |
| FFI | 0% | 无 C 互操作 / 裸指针 / 内联汇编 / freestanding(DevGame #78;**OS 第一前置**) |

**语言构造(已实现)**:`if`/`while`/`for in`/C 式 `for`/`match`(+ 通配)/`break`/`continue`/`return`;
算术·位·逻辑·比较·一元;`let`/赋值;数组字面量与索引读写(`a[i]`/`a[i]=v`/`a.len`);元组 `(a,b)`/`.N`;
struct 字面量与字段;enum + payload;闭包 `|x| ...` + 捕获;泛型 `<T>`(函数 + struct/enum,native 单态化);
`import`/`module`;`Option`/`Result` + `?`。
**trait/impl(切片①②已实现)**:`trait Show { fn show(self: Self) -> String; }` / `impl Show for Point { ... }`;UFCS 自由函数派发 `show(p)` 按接收者具体类型路由到 `show$Point`。`trait`/`impl` 是上下文标识符(非保留字,fixpoint 安全);impl 在 desugar 展平为 mangled 自由函数(`Self`→target),三后端当普通函数处理;调用按首参类型在 typecheck(Self 作类型参数宽松检查)/解释器(运行时值类型)/native(ZType base)三处派发,差分一致。
**未实现**:泛型约束(切片③:`f<T: Show>(x:T){show(x)}` —— 真多态调用 trait 方法)、方法调用语法 `x.f()`、并发原语、FFI。

**后端**:解释器 `run_mir`(差分 oracle);native MIR→LLVM22(inkwell,122 codegen fn,JIT + AOT 独立二进制 + 热替换);WASM(浏览器 playground)。

**自举**:`testdata/selfhost/arena_frontend.zeta`(**11,533 行手写 Zeta**)实现整条前端;Stage2 达成(被自己的 codegen 编 native,与解释器逐字节对齐);**fixpoint 自编译字节一致**。

**stdlib(std.core/std.io)**:string_len/byte_at/byte_slice/concat、int_to_string、**string_to_int**、int_abs/min/max、**int_pow**、string_index_of/contains/repeat、ascii 谓词、{int,bool,string,float}_array_empty/push、file_read_to_string、path_join/basename、diagnostic_format。

---

## 2. 构建 / 测试命令
```bash
cargo test                                          # 非 llvm(快)
LLVM_SYS_221_PREFIX=/opt/homebrew/opt/llvm cargo test --release --features llvm          # native 全套件(LLVM 22)
LLVM_SYS_221_PREFIX=/opt/homebrew/opt/llvm cargo test --release --features llvm --test selfhost_fixpoint -- --ignored   # 自举完整性门禁 ~145s
```
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
- **#4 中断/陷入 → 定时器 → 调度器**(待办,高价值但**前置阻塞**):需**内联汇编**原语(读写 CSR:stvec/sstatus/sie/sepc、`wfi`/`csrr`/`csrw`)——这是新语言原语,必须先做。之后才能装 trap handler + 定时器中断 + 调度。

### 6.2 OS 第二前置:裸指针 + volatile MMIO + 内联汇编(DevGame #78 扩展)
- **✅ volatile MMIO**(`d80a208`/`7ecb785`):`mmio_{read,write}_{byte,word,dword}`(8/16... 即 8/32/64 位)volatile load/store 到 inttoptr;真 NS16550 UART 驱动(`6103e89`,init+LSR 轮询)。
- **✅ 裸指针 `*T`**(`afcd901`,unsafe/native-only):`*T` 前缀语法 + Type::Ptr/ZType::Ptr 全栈;内置 `ptr_from_addr`/`ptr_addr`/`ptr_read`(支持整 struct)/`ptr_write`/`ptr_offset`(按元素步长)+ `array_data_addr`(数组缓冲区地址,DMA/测试)。typecheck 宽松兼容(任意 *A≈*B);解释器 inert ⇒ native/实机验证(tests/codegen_pointer.rs + kernel 实测 1337)。`ptr_from_addr` 元素由 let 注解 `*T` 精化(codegen Local 读注解)。
- **内联汇编**(`hlt`/`wfi`/读写 CR3/CSR、特权指令)——**仍缺**,是 #4 中断/调度的前置。
- **C ABI FFI**(声明 extern、按 C ABI 传参)——仍缺。

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
