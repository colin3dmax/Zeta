# Zeta 交接文档(2026-06-17)

> 跨会话接续的权威入口。详细分项见 `~/.claude/projects/-Users-colin-Work-Zeta/memory/`
> (language-features / feature-backport-selfhost / self-hosting-progress / native-backend-progress;
> 新会话自动加载 MEMORY.md 索引)。

## 0. 一句话状态
P1–P4 语言扩展(Float/Tuple/Closure/Generics)在 **Rust 前端 + native** 全部完成;
**自举前端**已回灌 Float/Tuple/Generics(native 全链)+ Closure(前半段);
正沿 DevGame 路线推进"补齐与成熟语言差距",最近完成 FloatArray + 泛型 struct/enum 语言层。
**工作树干净,115 提交未推送到 origin/main**(未 push,官网未部署)。

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
- **泛型 struct/enum**(389405d):`Box<T>`/`Option<T>`/`Result<T,E>` **仅语言层(解释器)**;
  实参擦除、type-param 当通配符(算术等操作数约束仍拒 T);**native 尚不支持泛型聚合**。

### 2b. 回灌自举前端 `testdata/selfhost/arena_frontend.zeta`(10k+ 行手写编译器)
- **Float/Tuple/Generics:native 全链回灌完成**(lexer→parser→dumps→emit LLVM),每个经
  selfhost_arena/mir/llvm + **fixpoint 4/4** 验证。
- **Closure:前半段**(parse→lower→dumps);**emit 闭包转换待做**。
- ev_expr 解释器的 Float/Tuple/Closure 推迟(值系统无 f64/复合槽)。

## 3. DevGame(zeta 项目)路线任务
- #73 差距分析总览;#74 P1 内存管理(native 22 malloc/0 free,泄漏);#75 P2 stdlib+错误处理;
  #76 P3 泛型容器(FloatArray✅ + 泛型 struct/enum 语言层✅,**native 单态化聚合待做**);
  #77 P4 并发;#78 P5 FFI/跨平台。
- **依赖链(记录在 #75/#76)**:错误处理(内置 Option/Result + `?`)硬前置 = **native 单态化泛型 struct/enum**
  (否则注入内置 Option/Result 会让 import std.core 的程序在 native 编泛型枚举时失败)。`?` 另需类型导向脱糖。
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
1. **#76 native 单态化泛型 struct/enum**(硬前置):每实例(Option<Int>/Box<Float>)生成独立 LLVM 布局,
   规模约等于泛型函数单态化(借鉴 codegen 的 lower_generic_call/get_or_build_specialization)。
2. **内置 Option/Result**(依赖 1):导入 std.core 时注入合成 enum 决(若用户未定义)。
3. **`?` 运算符**(依赖 1+2):lexer 加 `?`;类型导向脱糖;表达式位需 hoist。
4. **Closure 自举 emit**(回灌收尾):自由变量分析+lift+heap env+间接调用,复用 Generics 的 spec_defs 缓冲,fixpoint-safe。
5. 远端:`git push`(115 提交)+ 官网部署(需用户决定)。

## 6. 关键经验
- 加类型要改**两个类型检查器**(typecheck.rs + mir.rs verifier)+ 解释器(MIR/AST eval + liveness)+ module_graph + native codegen。
- 自举前端改动必跑 **fixpoint**;新特性 emit 路径对 fixpoint 通常 safe(arena_frontend 不用新特性→输出不变)。
- `src/type_syntax.rs`(tuple_parts/fn_parts/split_top_level)贯穿 P2-P4,是结构化类型字符串解析基础。
- 泛型在解释器透明(运行时类型擦除);native 必须单态化。
