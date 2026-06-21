# Zeta 交接文档(2026-06-18)

> 跨会话接续的权威入口。详细分项见 `~/.claude/projects/-Users-colin-Work-Zeta/memory/`
> (language-features / feature-backport-selfhost / self-hosting-progress / native-backend-progress;
> 新会话自动加载 MEMORY.md 索引)。

## 0. 一句话状态
P1–P4 语言扩展(Float/Tuple/Closure/Generics)在 **Rust 前端 + native** 全部完成;
**自举前端 P1–P4 回灌全部完成**(Float/Tuple/Generics/Closure native emit 全链;仅 ev_expr 解释器复合值推迟);
正沿 DevGame 路线推进"补齐与成熟语言差距"。**本会话完成:错误处理全链(#75)— native 单态化
泛型 struct/enum(阶段A/B)+ 内置 Option/Result + `?` + typecheck 保留泛型实参(unwrap 值可算术);
Closure 自举 emit 回灌(P1-P4 回灌全完成);native 内存管理 v1(#74,作用域释放数组局部修循环泄漏)。**
**已 push 到 origin/main 并部署官网 zeta.jennieapp.com(本会话内);其后 Closure/内存 v1 提交待再次 push。**

## 1. 构建 / 测试命令
```bash
cargo test                                          # 非 llvm(快)
LLVM_SYS_221_PREFIX=/opt/homebrew/opt/llvm cargo test --release --features llvm   # native(LLVM 22)
LLVM_SYS_221_PREFIX=/opt/homebrew/opt/llvm cargo test --release --features llvm --test selfhost_fixpoint -- --ignored   # 自举完整性 ~105-115s
```
**预存坑**:`selfhost_arena`/`selfhost_mir` 的 `all_stage1_parity_probes` mega-test 在 debug 栈溢出(超大输入+递归),与改动无关。

## 2. 已完成
### 2a. 语言扩展(src/ Rust 前端 + native codegen)
- **Float**(f64):算术/比较;Int/Float 不混用;Mod/位运算仅 Int;native double/fadd/fcmp/fneg。
- **Tuple**:`(a,b)`/`.N`(`t.1.0` 拆两索引)/`(Int,String)` 注解;native 匿名 struct。
- **Closure**:`|x:Int| body`、按值捕获、`fn(T)->R`、间接调用;native 闭包转换(lift+heap env)。
- **Generics**:`fn id<T>(x:T)->T` 实参推断;native 按需单态化(`id$Int`,含 transitive)。
- **FloatArray**(887ff7f):数组元素类型扩 Float,全链+native。
- **泛型 struct/enum**(389405d 语言层 + eba1963/0ecd7cd native):`Box<T>`/`Option<T>`/
  `Result<T,E>` 全链 **+ native 单态化**。
  - 语言层:实参擦除、type-param 当通配符(算术等操作数约束仍拒 T)。
  - native:`ZType::Struct/Enum` 用 mangled 名(`Box$Int`/`Option$Int`)编码实参,
    `Types` 区分实例表(RefCell 动态注册)与泛型模板;struct 按实例生成独立 LLVM
    布局,enum 沿用统一 `{tag,p0,p1}`(payload 类型由实例驱动 encode/decode)。
  - 阶段A(eba1963):值流推断,覆盖单函数内构造/match/字段访问。
  - 阶段B(0ecd7cd):parser 保留泛型实参字符串,typecheck/mir 解码点 strip 到 base
    (擦除语义不变),codegen `resolve_ann_ztype` 读实参 → 解锁**跨函数返回/参数**。
  - 测试:`tests/codegen_generic_aggregates.rs`(10 个,native 对齐解释器 oracle)。
  - 遗留边界:泛型字段 T 通配符,`Box<Int>` 塞 Float 值会在 native LLVM verify 报错。

### 2b. 回灌自举前端 `testdata/selfhost/arena_frontend.zeta`(10k+ 行手写编译器)
- **Float/Tuple/Generics/Closure:native 全链回灌完成**(lexer→parser→dumps→emit LLVM),每个经
  selfhost_arena/mir/llvm + **fixpoint 4/4** 验证。Closure emit = c38469b。
- ev_expr 解释器的 Float/Tuple/Closure 推迟(值系统无 f64/复合槽,次要路径)。

## 3. DevGame(zeta 项目)路线任务
- #73 差距分析总览;#74 P1 内存管理(**v1 ✅ 01d52cb**:作用域释放数组局部修循环泄漏;
  字符串/闭包env/enum装箱/逃逸/终止路径/顶层局部仍泄漏,待 v2);#75 P2 stdlib+错误处理;
  #76 P3 泛型容器(FloatArray✅ + 泛型 struct/enum 语言层✅ + **native 单态化聚合✅** eba1963/0ecd7cd);
  #77 P4 并发;#78 P5 FFI/跨平台。
- **依赖链(记录在 #75/#76)**:错误处理(内置 Option/Result + `?`)硬前置 = native 单态化泛型
  struct/enum —— **此前置现已解除**。`?` 另需类型导向脱糖。下一步可直接做内置 Option/Result + `?`。
- **DevGame 访问**:项目级 `./.devgame/token.json`(6/4)已过期;用**全局** `~/.devgame/token.json`(有效)。
```bash
GT=$(node -e "process.stdout.write(require(require('os').homedir()+'/.devgame/token.json').token)")
env -u http_proxy -u https_proxy -u all_proxy -u HTTP_PROXY -u HTTPS_PROXY -u ALL_PROXY \
  API_BASE=https://devgame.jennieapp.com PROJECT=zeta DEVGAME_TOKEN="$GT" \
  node ~/.devgame/bin/update-task.mjs --id <N> --comment "..."   # 创建用 --create --subject .. --description-file ..
```

## 4. 官网
仓库内 `docs/*.html`。已更新(daf61e0):language-features.html(+4 节)、tutorial/index.html(课程 7-10)、
index.html(能力清单)。**未部署**。

## 5. 下一步(按依赖序)
1. ~~#76 native 单态化泛型 struct/enum~~ **✅ 完成**(eba1963 阶段A + 0ecd7cd 阶段B)。
2. ~~内置 Option/Result~~ **✅ 完成**(7f10a50):std.core 注入泛型 `Option<T>`/`Result<T,E>`,
   仅在被引用时注入(legacy `OptionInt` 等无条件保留),保留名(本地同名→冲突)。
3. ~~`?` 运算符~~ **✅ 完成**(6264d43):lexer `?` token + parser 后缀 `Expr::Try` +
   pre-resolve 脱糖(`src/desugar.rs`,续延移入成功分支的 match,按返回类型分派 Ok/Err 或
   Some/None)。复用现有 match/enum/return,无新 codegen。`?` 仅用于返回 `Option`/`Result` 的函数。
4. ~~让 `?`/泛型 unwrap 值可算术~~ **✅ 完成**(44c10f6):typecheck 加 `Type::Generic`,
   保留泛型实参;match 变体绑定与 struct 字段访问在具体 `Generic` 时代入实参 →
   `Result<Int,String>` 的 `Ok(v)` 绑定 `v:Int`、`Box<Int>.value` 为 Int,可直接算术。
   真多态 `fn f<T>(x:T){x+1}` 仍被拒(T 未代入,保安全)。
5. ~~Closure 自举 emit~~ **✅ 完成**(c38469b):arena_frontend.zeta emit 闭包转换(fn-type 助手 +
   gen_lambda lift/env/间接调用,复用 spec_defs 缓冲);selfhost_llvm +4,fixpoint 4/4 不变。
   **P1-P4 回灌全部完成。** 仅 ev_expr 解释器闭包/Float/Tuple 推迟(值系统无复合槽,次要路径)。
6. ~~#74 native 内存管理 v1~~ **✅ 完成**(01d52cb):作用域释放数组局部(fall-through 路径,
   `free` elems-8),`lower_block` 覆盖 if/while/for/forc body → 循环每迭代回收。安全:数组值语义
   唯一拥有 ⇒ 无 UAF。测试 tests/codegen_memory.rs。
7. **#74 native 内存管理 v2 ✅**(32e92dd):数组局部赋值释放旧 buffer + 非数组 return 前释放存活数组局部。
8. **#74 native 内存管理 v3 ✅**:return-数组所有权转移(数组返回释放其余局部、转移返回值;
   `bind_owned` 让「返回数组的 call 结果」取得所有权不深拷)。
9. **#74 native 内存管理 v4 ✅**:push **grow 释放被弃旧 buffer**(free_array_data) + **隐式
   fall-through return 释放顶层数组局部**。**至此数组/动态数组内存全面零泄漏**(循环/重赋值/
   per-call/返回值/grow/fall-through 全回收;codegen_memory 10 个差分测试)。
   **内存模型 = Rust 式(2026-06-20 与用户敲定方向)**:数组已是 Rust 所有权/move/scope-drop
   (= Vec 的管理,无 GC/无引用计数,编译期确定性释放)。方向 = **值语义 + 编译器自动 Drop/COW**
   (学 Swift 值类型+COW、Nim ARC;**不**搬 Rust 借用检查器——避开新手陡峭),关键优势:值语义⇒无环⇒
   连环收集器都不需要。

10. **#74 值语义内存 v5 ✅(commit 656ace0)= 生成式 per-type drop/clone**:落地「值语义 + 自动 Drop」
    的正确架构。为每个 managed 类型生成一份递归 `@__drop_T`/`@__clone_T` 模块函数(缓存,先于函数体插
    缓存以支持递归类型),绑定点调 clone、scope/return/reassign 调 drop。**函数在数据上递归(有限)⇒
    天然支持递归类型、无 codegen 栈溢出**(这正是先前 inline 递归失败的根因,现已解决)。框架:needs_drop /
    get_or_build_drop+emit_drop_body / get_or_build_clone+emit_clone_body / clone_value / drop_local;
    构造点(Array/Tuple/Struct/EnumVariant payload)bind_owned 每个 managed 成员;push 拥有追加元素;
    return move-on-return。**Str / Array / Tuple 完全值管理零泄漏**(含 string array、嵌套);
    codegen_memory 13 个差分测试 + fixpoint 4/4 无回归。
10b. **#74 值语义内存 v6 ✅(commit 6d669d6)= Struct 开管理,全聚合零泄漏**:needs_drop(Struct) 改为
    按字段递归判定(任一字段需 drop ⇒ struct 需 drop),复用 v5 的生成式 @__drop_T/@__clone_T 自动覆盖
    嵌套/含数组/含字符串 struct。**至此 array/string/tuple/struct 全聚合都走值语义 + 确定性 Drop。**
    ⚠️**重要更正**:v5 里归因到 struct 的「堆 double-free」**是误诊**。真因 = 并行测试 harness 下对 ~1 万行
    合并前端做深递归(lower/run_mir/codegen)时,默认 **2 MiB 测试线程栈处于临界值**,struct 管理加深的
    codegen 递归把它推过边界 → 偶发 SIGSEGV/SIGBUS。**证据**:ASan 对生成代码(clang 编译 run/mir-dump/
    ast-dump 各模式)全程干净、digest 始终一致;改用 64 MiB 大栈线程跑 codegen_selfhost_run 后连跑 5 次零
    flake。**经验**:并行 harness 下自举前端的偶发 SEGV 优先怀疑栈溢出(RUST_MIN_STACK=64M 一测便知),
    别先假设堆错误。门禁:全 llvm 套件 53 suite 0 失败、fixpoint 4/4(144s)。
10c. **#74 值语义内存 v7 ✅(commit c5a55d4)= Enum payload 管理**:emit_drop/clone_body 加 Enum 臂,
    按 tag switch 处理活跃变体 payload —— Str/Array 重建 `{len,ptr}` 调其 drop/clone;Struct payload
    堆装箱(p1),drop 先 load+drop 托管字段再 free 装箱、clone malloc 新箱深拷。needs_drop(Enum) 按变体
    递归(Struct payload 恒装箱 ⇒ 即使无托管字段也需 free)。**关键**:match 的 Str/Struct/Array payload
    绑定改为统一 clone_value(此前 Str 共享 p1、Struct/Array 浅拷)→ 绑定与 enum 各自独立所有,杜绝
    double-free。门禁:codegen_memory 15、codegen_enum 20、fixpoint 4/4(error handling 走 Option/Result
    重度验证)、ASan 零堆错误。
10d. **#74 值语义内存 v8 ✅(commit 5044476)= Closure env 管理,全语言零泄漏**:闭包表示
    `{fn,env}`→`{fn,env,drop_thunk,clone_thunk}`(调用 ABI 0/1 不变)。每 lambda 站点生成
    `@<lambda>_dropenv`(逐捕获 drop+free env)/`@<lambda>_cloneenv`(malloc 新 env+逐捕获深拷)两枚
    thunk 携带捕获布局;类型级 @__drop/clone_Closure 仅从值取 thunk 委派(null 守卫零值闭包)——解决
    「env 布局是 per-lambda 非 per-type」的根本难点。捕获改 clone 进 env(此前浅拷共享 → double-free
    隐患)。fixpoint 安全:arena_frontend 无 lambda,闭包构造路径不触发。**至此 array/string/tuple/
    struct/enum/closure 全聚合值语义 + 确定性 Drop,全语言零泄漏。** 门禁:codegen_closure 8 / closure 9
    / codegen_memory 17、全 llvm 50 suite 0 失败、fixpoint 4/4(143s)、ASan 对捕获 String 闭包零堆错误。
10e. **#74 性能 v9 ✅(commit d015d03)= 数组 COW(refcount + 写时拷)**:数组 buffer 头 {cap}→{cap,rc}
    (16B,rc 起始 1)。clone:标量数组 rc++ 返回同 buffer(O(1) 共享);托管元素数组仍深拷(rc 恒 1、
    元素堆独立)。drop:rc--,仅 rc→0 才 drop 元素+free。**写时拷**:就地变异点(`a[i]=v`、就地 push
    `xs=push(xs,v)`)先 `cow_make_unique`(rc>1 则深拷独占副本保留 cap、原 buffer 减一引用、写回 slot;
    未共享 no-op),杜绝别名破坏值语义。**效果**:大数组反复传值(arena 式)O(n)→O(1) —— 微基准 4000
    元素传值 10 万次 65ms→1ms(~65x);非共享负载中性。门禁:既有别名测试 + cow_inplace_push_on_shared
    / cow_share_in_loop、ASan 零堆错误(含 realloc-while-shared)、全 llvm 50 suite 0 失败、fixpoint 4/4。
10f. **#74 性能 v10 ✅(commit c49b2da)= 字符串 COW(纯引用计数)**:字符串不可变 ⇒ 共享天然安全、
    无写时拷,纯 refcount。堆串加 8B rc 头(起始 1);**字面量发射为全局 `{i64 STATIC_RC, [bytes,NUL]}`,
    哨兵 rc=i64::MIN**,clone/drop 一律跳过 → 全局常量永不 bump/free(解决「字面量是全局不可 free」难题)。
    clone=rc++(哨兵 no-op)、drop=rc--free-if-0;concat/byte_slice/int_to_string 改 alloc_str_buf。
    效果:大串传值 O(n)→O(1)(微基准 4096 字符×10万 9ms→2ms);小串负载中性(selfhost_perf 267≈265)。
    门禁:string_literal_bound_in_loop / string_shared_by_refcount_balances、ASan 零堆错误、全 llvm 50
    suite 0 失败、fixpoint 4/4。**至此 array + string 两大高频类型都走 COW；值语义共享传值 O(1)。**
11. **后续性能(可选)**:① move-on-last-use(免掉非共享绑定的 rc 簿记,需 MIR 活跃性分析);② 小字符串
    优化 SSO(≤15B 内联,免堆分配,缓解小串 rc 开销);③ struct/tuple 大聚合的 COW。array+string COW 已
    兑现「无别名 ⇒ 共享传值 O(1)」的主要红利。
12. 其它候选:#77 P4 并发 / #78 P5 FFI;ev_expr 解释器补全(FloatArray 现已有,blocker 或已解)。
13. 远端:`git push`(本会话各提交)+ 官网重部署(`tools/deploy-website.sh`)。

### 阶段B 实现笔记(给接续者)
- 范式:复用泛型**函数**单态化(`lower_generic_call`/`get_or_build_specialization`/`mangle_instance`/`unify_ztype`)。
- 类型字符串贯穿全链:parser 产规范带参串(`Result<Int, String>`)。各层解码:
  - **typecheck**:`parse_type` 产 `Type::Generic(base,args)`(保留实参);match/字段访问代入;
    EnumType/StructType 带 type_params;expect_type 用 `aggregate_base` 让擦除 `Named` 与具体
    `Generic` 互通。
  - **mir verifier**:`parse_mir_type` **strip 到 base**(type_param 当通配符,lenient,不拒 `v+1`)。
  - **codegen**:`resolve_ann_ztype` 读实参单态化(`Box$Int`/`Option$Int`)。
  - **runtime**:不解码注解,类型擦除,无需改。
- enum 统一 `{tag,p0,p1}` 布局是关键:跨函数返回时注解实例与构造点占位实例 LLVM 类型一致。
- codegen 里加新「按名查 struct/enum」逻辑时,串可能带 `<...>` —— 先 `type_syntax::base_name` 再查。
- `?` 脱糖在 `src/desugar.rs`(pre-resolve):无 match 表达式/未初始化 let → 续延移入成功分支。

## 6. 关键经验
- 加类型要改**两个类型检查器**(typecheck.rs + mir.rs verifier)+ 解释器(MIR/AST eval + liveness)+ module_graph + native codegen。
- 自举前端改动必跑 **fixpoint**;新特性 emit 路径对 fixpoint 通常 safe(arena_frontend 不用新特性→输出不变)。
- `src/type_syntax.rs`(tuple_parts/fn_parts/split_top_level)贯穿 P2-P4,是结构化类型字符串解析基础。
- 泛型在解释器透明(运行时类型擦除);native 必须单态化。
