# Zeta 交接文档(2026-06-18)

> 跨会话接续的权威入口。详细分项见 `~/.claude/projects/-Users-colin-Work-Zeta/memory/`
> (language-features / feature-backport-selfhost / self-hosting-progress / native-backend-progress;
> 新会话自动加载 MEMORY.md 索引)。

## 0. 一句话状态
P1–P4 语言扩展(Float/Tuple/Closure/Generics)在 **Rust 前端 + native** 全部完成;
**自举前端**已回灌 Float/Tuple/Generics(native 全链)+ Closure(前半段);
正沿 DevGame 路线推进"补齐与成熟语言差距"。**最近完成(本会话):① native 单态化泛型
struct/enum(阶段A 值流推断 + 阶段B parser 保留实参全链传播);② 内置泛型 Option/Result
(std.core 注入);③ `?` 运算符(pre-resolve 脱糖)。** 错误处理链条(#75)基本打通。
**多个提交未推送到 origin/main**(未 push,官网未部署)。

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
- **Float/Tuple/Generics:native 全链回灌完成**(lexer→parser→dumps→emit LLVM),每个经
  selfhost_arena/mir/llvm + **fixpoint 4/4** 验证。
- **Closure:前半段**(parse→lower→dumps);**emit 闭包转换待做**。
- ev_expr 解释器的 Float/Tuple/Closure 推迟(值系统无 f64/复合槽)。

## 3. DevGame(zeta 项目)路线任务
- #73 差距分析总览;#74 P1 内存管理(native 22 malloc/0 free,泄漏);#75 P2 stdlib+错误处理;
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
3. ~~`?` 运算符~~ **✅ 完成**(下一个提交):lexer `?` token + parser 后缀 `Expr::Try` +
   pre-resolve 脱糖(`src/desugar.rs`,续延移入成功分支的 match,按返回类型分派 Ok/Err 或
   Some/None)。复用现有 match/enum/return,无新 codegen。**局限**:`?` 仅用于返回
   `Option`/`Result` 的函数;unwrap 出的值是泛型 payload `T`(通配符),可返回/传参/存储,
   **不能直接做算术**(`v + 1` 被 TYPE_BINARY_OPERAND 拒——既有 lenient 泛型限制)。
4. **(可选)放宽 lenient 泛型**:若要让 `?`/泛型 unwrap 值支持算术,需让 typecheck 对泛型
   payload 用具体类型(真单态化类型检查)或放宽 operand 约束——是设计取舍,需用户拍板。
5. **Closure 自举 emit**(回灌收尾):自由变量分析+lift+heap env+间接调用,复用 Generics 的 spec_defs 缓冲,fixpoint-safe。
6. 远端:`git push` + 官网部署(需用户决定)。

### 阶段B 实现笔记(给接续者)
- 范式:复用泛型**函数**单态化(`lower_generic_call`/`get_or_build_specialization`/`mangle_instance`/`unify_ztype`)。
- 类型字符串贯穿全链:parser 产规范串 → typecheck(`parse_type`/`parse_declared_type`/`validate_type_name`)
  与 mir(`parse_mir_type`)在解码点 **strip 到 base**(`type_syntax::generic_parts`/`base_name`)→
  codegen(`resolve_ann_ztype`)读实参单态化。**runtime 不解码注解,无需改**。
- enum 统一 `{tag,p0,p1}` 布局是关键:跨函数返回时注解实例与构造点占位实例 LLVM 类型一致。
- 加新「按名查 struct/enum」逻辑时,记得它可能收到带 `<...>` 的串 —— 先 `base_name` 再查。

## 6. 关键经验
- 加类型要改**两个类型检查器**(typecheck.rs + mir.rs verifier)+ 解释器(MIR/AST eval + liveness)+ module_graph + native codegen。
- 自举前端改动必跑 **fixpoint**;新特性 emit 路径对 fixpoint 通常 safe(arena_frontend 不用新特性→输出不变)。
- `src/type_syntax.rs`(tuple_parts/fn_parts/split_top_level)贯穿 P2-P4,是结构化类型字符串解析基础。
- 泛型在解释器透明(运行时类型擦除);native 必须单态化。
