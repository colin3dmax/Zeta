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

   **⚠️ 全语言递归 Drop 尝试(2026-06-20)——已回退,留经验**:试过把数组那套(deep_copy_value +
   drop_value + bind_owned)递归推广到 Str/Tuple/Struct/Array(managed),构造点深拷成员、全链 Drop。
   **flat 类型全通过**(string/struct/tuple/enum/array/dynarray/memory 7 套差分绿),但在**自举前端**
   (recursive_run)**栈溢出**:① **inline 递归 Drop/Copy 对递归类型(struct 经 array 字段成环)会无限
   展开**——加 visited 栈/深度兜底仍溢出,说明是**运行时**溢出(copy/drop 对复杂结构数据损坏 → JIT 的
   递归下降 parser 死循环)。**结论:inline drop/copy 不行,正解是为每个类型生成递归析构/克隆函数
   (`@drop_T(ptr)`/`@clone_T`,像 Rust 的 `Drop::drop`/`Clone`,runtime 递归处理任意深度 + 天然支持
   递归类型)。** 这是字符串/聚合零泄漏的真正前置,是个独立中型工程。已回退到 v4(数组零泄漏)干净态。
   **下一步若做**:① 生成 per-type drop/clone 函数(替代 inline);② 字符串纳入(值拷贝或 Rc);
   ③ 聚合/容器递归;④ 全程差分 + 自举 fixpoint 守门(自举前端是最强的 UAF/损坏探测器)。
   字符串若用 Rc:堆串加 refcount 头、retain/drop-release、归零 free、全局字面量哨兵 refcount 永不释放。
8. 其它候选:#77 P4 并发 / #78 P5 FFI;ev_expr 解释器补全(FloatArray 现已有,blocker 或已解)。
9. 远端:`git push`(本会话 Closure/内存 v1 提交)+ 官网重部署(`tools/deploy-website.sh`)。

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
