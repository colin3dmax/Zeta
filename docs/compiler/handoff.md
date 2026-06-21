# Zeta 交接文档(2026-06-21)

> 跨会话接续的**权威入口**。新会话先读本文 + `~/.claude/projects/-Users-colin-Work-Zeta/memory/MEMORY.md`(自动加载的索引)。
> 详细分项见 memory 下 language-features / feature-backport-selfhost / self-hosting-progress / native-backend-progress。
> 最新提交:`1c64865`(全部已 push origin/main)。

---

## 0. 一句话状态
Zeta 是一门**能自举、有三后端(解释器 / LLVM native / WASM)、内存自动管理(值语义 + 确定性 Drop + COW)** 的真实语言,跑 **835+ 测试全绿**,自编译**字节级一致(fixpoint 4/4)**。
**语言内核成熟;缺口在生态与系统能力:trait/接口、标准库广度、并发、FFI。**

---

## 1. 实现盘点(2026-06-21)

| 维度 | 完成度 | 说明 |
|---|---|---|
| 编译器管线 | ~95% | lex→parse→resolve→HIR→typecheck→desugar→MIR→{解释器/native/wasm};自举闭环 |
| 核心语言 | ~85% | 标量(Int/Float/Bool/String)、数组/元组/struct/enum/闭包、泛型单态化、错误处理 `?` |
| 内存管理 | ~90% | 全聚合值语义 + 确定性 Drop 零泄漏;array/string COW;缺 SSO / move-opt |
| 自举 | ~90% | native emit 全链 + fixpoint;ev_expr 解释器的复合值(Float/Tuple/Closure)推迟 |
| 类型系统 | ~70% | 泛型单态化齐全;**无 trait/接口、无泛型约束** ← 最大结构性缺口 |
| 标准库 | ~30% | 字符串/数组/整数工具 + 基础 IO;**无 Map/Set、无网络** |
| 并发 | 0% | 无语言级并发(DevGame #77) |
| FFI | 0% | 无 C 互操作(DevGame #78) |

**语言构造(已实现)**:`if`/`while`/`for in`/C 式 `for`/`match`(+ 通配)/`break`/`continue`/`return`;
算术·位·逻辑·比较·一元;`let`/赋值;数组字面量与索引读写(`a[i]`/`a[i]=v`/`a.len`);元组 `(a,b)`/`.N`;
struct 字面量与字段;enum + payload;闭包 `|x| ...` + 捕获;泛型 `<T>`(函数 + struct/enum,native 单态化);
`import`/`module`;`Option`/`Result` + `?`。
**未实现**:trait/impl、方法调用语法 `x.f()`(目前只有自由函数调用 + 字段访问)、泛型约束、并发原语、FFI。

**后端**:解释器 `run_mir`(差分 oracle);native MIR→LLVM22(inkwell,122 codegen fn,JIT + AOT 独立二进制 + 热替换);WASM(浏览器 playground)。

**自举**:`testdata/selfhost/arena_frontend.zeta`(**11,533 行手写 Zeta**)实现整条前端;Stage2 达成(被自己的 codegen 编 native,与解释器逐字节对齐);**fixpoint 自编译字节一致**。

**stdlib(std.core/std.io)**:string_len/byte_at/byte_slice/concat、int_to_string、**int_abs/min/max**、**string_index_of/contains/repeat**、ascii 谓词、{int,bool,string,float}_array_empty/push、file_read_to_string、path_join/basename、diagnostic_format。

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
8. **泛型实现(阶段B)**:类型字符串贯穿全链(parser 产 `Result<Int, String>`);typecheck `Type::Generic(base,args)` 保留实参、match/字段访问代入;mir verifier `parse_mir_type` strip 到 base(type_param 当通配符,lenient);codegen `resolve_ann_ztype` 读实参单态化。`src/type_syntax.rs`(tuple_parts/fn_parts/split_top_level)是结构化类型串解析基础。codegen 按名查 struct/enum 时串可能带 `<...>`,先 `type_syntax::base_name`。

---

## 6. 下一步(推荐顺序)

1. **trait / 接口系统**(类型系统最大缺口,也是 HashMap 等容器的前提)。本仓库**无方法调用语法**(`x.f()` 不解析,调用须 Name/path 目标)→ 设计应走 **UFCS 自由函数派发 + 单态化**:`trait Show { fn show(self)->String; }` / `impl Show for Point {...}` / `fn f<T: Show>(x:T){ show(x) }`。多片切片:① 词法/语法/AST/resolve + ast-dump(纯前端,fixpoint-safe,因 arena_frontend 不含 trait)→ ② 具体类型的派发(typecheck + codegen)→ ③ 泛型约束 + 单态化派发。
2. **标准库继续扩**(低风险,马上可用):`string_to_int`/`string_split`/`string_to_upper`、`int_pow` 等(注意 §5.2/5.3);`HashMap`/`Set` 需 trait(hash/eq 派发)前置。
3. **性能(可选)**:move-on-last-use(免非共享绑定的 rc 簿记,需 MIR 活跃性分析)、SSO 小字符串内联(≤15B 免堆)、struct/tuple 大聚合 COW。
4. **DevGame 路线**:#77 并发 / #78 FFI;ev_expr 解释器补复合值。
5. **官网重部署**(`tools/deploy-website.sh`,如内容有更新)。

---

## 7. DevGame 访问
项目级 `./.devgame/token.json` 已过期;用**全局** `~/.devgame/token.json`(有效)。访问国内链路必须关代理:
```bash
GT=$(node -e "process.stdout.write(require(require('os').homedir()+'/.devgame/token.json').token)")
env -u http_proxy -u https_proxy -u all_proxy -u HTTP_PROXY -u HTTPS_PROXY -u ALL_PROXY \
  API_BASE=https://devgame.jennieapp.com PROJECT=zeta DEVGAME_TOKEN="$GT" \
  node ~/.devgame/bin/update-task.mjs --id <N> --comment "..."   # 创建:--create --subject .. --description-file ..
```
