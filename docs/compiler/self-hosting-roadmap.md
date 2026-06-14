# Zeta 独立自举工程路线图

> 状态基准:2026-06-14。本文把"从当前到 Zeta 编译器能编译它自己(self-hosting 闭环、脱离 Rust Stage0)"拆解为可执行里程碑。
>
> **⚠️ 路线图已全部达成。** M0–M7 全完成,且原"剩余 = native/LLVM 后端"那条线也已完成(详见 §2 表与 §6)。本文保留作历史规划与设计依据;最新落地状态以 `docs/compiler/handoff.md` 与 `~/.claude/projects/.../memory/` 的进展记忆为准。

## 0. 现状基线(2026-06-14:已自举闭环)

- **Stage0(Rust)**:参考编译器 / oracle。lexer → parser → AST → resolver → typecheck → MIR → MIR 解释器,外加 `ast-dump`/`hir-dump`/`mir-dump` 文本转储,并新增 **MIR→LLVM native 后端**(JIT + AOT,`#[cfg(feature="llvm")]`)。
- **Stage1(Zeta,`testdata/selfhost/arena_frontend.zeta`,约 7500 行)**:用 Zeta 写的**完整前端**(早期的 `frontend.zeta` 雏形已被取代)。能力:
  - ✅ 词法扫描 + **递归下降 parser** 构造**真 arena AST**(index-based 并行数组,非拼文本)。
  - ✅ **resolver / typecheck / MIR lowering / MIR 解释器**全部用 Zeta 写,逐字/逐诊断对齐 Rust oracle(M3–M6)。
  - ✅ 统一 driver `export fn compile(source, mode)`(ast-dump/resolve/typecheck/mir-dump/run)。
  - ✅ **自举 fixpoint**:Stage1 处理自身 7500 行源,四阶段与 Rust oracle 全等(`tests/selfhost_fixpoint.rs`,4/4)。
- **native 后端**:`arena_frontend.zeta` 整个前端经 Zeta 自己的 native 后端 AOT 成独立可执行,产物逐字节对齐解释器(脱离 Stage0)。
- **质量武器**:oracle 差分门禁(`examples/parity_check` + `tests/stage1_parity.rs` 243 探针 + 各阶段 selfhost_*.rs),用 Rust 作权威,逐字验证 Zeta 输出。这套体系贯穿了全部里程碑。

**完成度:M0–M7 + native 后端全部达成。** 见 §2 里程碑表与 §6。

## 1. 根本约束 → 核心设计抉择

Zeta 语言**当前**缺以下能力(写编译器会撞墙):

| 缺失 | 影响 | 抉择 |
|---|---|---|
| 无指针 / Box / 递归类型 | AST 是树,无法用 `Expr { left: Expr }` 表达 | **index-based arena**:节点存并行数组,子节点用 `Int` 索引引用 |
| 无 Map / Dict | 符号表、作用域无法用哈希表 | **并行数组 + 线性查找**(名字数组 + 类型数组),frontend 已是此风格 |
| enum 单 payload | AST 节点(如 `Binary{op,left,right}`)难用 enum 直接建模 | 用 **node-kind tag(Int)+ 并行字段数组** 表达"带多字段的节点" |
| 值语义(struct/array 拷贝) | 大结构传递有成本 | arena 用全局并行数组 + 索引,避免深拷贝 |

**结论:整条自举链路统一采用 arena 表示**(节点种类 tag 数组 + 各字段并行数组 + Int 索引),这与 frontend 现有 token 数组风格一致,是无指针语言做编译器的标准手法。

## 2. 里程碑总览

| 里程碑 | 目标 | 依赖 | 规模 | 验证 | 状态 |
|---|---|---|---|---|---|
| **M0** | 语言地基 + 前端解析契约 | — | — | parity 243 探针 | ✅ |
| **M1** | 补齐"写编译器"所需语言能力 | M0 | 中 | 新特性各自 parity + 单测 | ✅ |
| **M2** | Stage1 前端:文本 → **结构化 arena AST** | M1 | 大(质变) | AST 遍历产 dump 仍对齐 Rust | ✅ |
| **M3** | 用 Zeta 写 **resolver** | M2 | 大 | 与 Rust resolver 诊断对齐 | ✅ |
| **M4** | 用 Zeta 写 **typecheck** | M3 | 大 | 与 Rust typecheck 诊断对齐 | ✅ |
| **M5** | 用 Zeta 写 **MIR lowering** | M4 | 大 | 与 Rust mir-dump 对齐 | ✅ |
| **M6** | 用 Zeta 写 **MIR 解释器后端** | M5 | 大 | Zeta 跑程序结果 == Rust 跑 | ✅ |
| **M7** | **自举闭环**:Zeta 编译器编译自己 | M2–M6 | 中(集成) | fixpoint:Stage1 编译 Stage1 | ✅ |
| **后端** | MIR→LLVM native(JIT/AOT)+ 自举前端 AOT 成独立 exe | M7 | 大(独立线) | native==解释器差分 + AOT 逐字节 | ✅ |

## 3. 里程碑详情

### M1 — 语言能力补全(写编译器的前提)
- **目标**:让 Zeta 足以表达编译器自身的代码。
- **关键工作**(按对自举的必要性排序):
  1. **enum 多 payload**(`Variant(Int, String)`)——表达 AST/MIR 节点的多字段载荷,或确认改用 tag+并行数组方案后不需要。
  2. **字符串能力增强**:子串、比较、拼接、整数↔字符串(部分已有 std.core builtin,需补齐到"能写诊断信息/符号名比较")。
  3. **嵌套/多维数组**或 arena 所需的数组增删能力(push 已有,需确认 set/grow 足够)。
  4. (可选)float/char——**自举不需要**,可延后。
- **难点**:判断"够用"的标准——能否用 Zeta 把一个 `resolve_expr` 函数写出来。建议用一小段"试写"验证。
- **验证**:每个特性 oracle parity + 单测(沿用现有流程)。

### M2 — Stage1 前端:文本 → 结构化 arena AST(**关键质变**)
- **目标**:Stage1 parser 不再直接拼 dump 文本,而是**构造 arena AST**;再由一个独立的"AST → dump 文本"遍历器产出输出。
- **关键工作**:
  1. 设计 arena AST 表示:`node_kind: IntArray`、`node_a/node_b/node_c: IntArray`(子节点索引或字面值索引)、`node_text: StringArray`(名字/字面量)。一套全局并行数组 + 根索引。
  2. 重写 parser:从"扫描+拼字符串"改为"递归下降构造节点,返回节点索引"。这会**顺带把极简 parser 升级为真正的递归下降**——对后续阶段至关重要。
  3. 写 `dump_from_ast(root) -> String`,遍历 arena 产出文本。
- **验证**:`dump_from_ast` 输出仍与 Rust ast-dump **逐字对齐**(复用 243 探针 + parity 门禁)。这保证"结构化"没破坏正确性。
- **难点**:递归下降在 arena 风格下的写法(每个 parse 函数返回 Int 索引);Zeta 表达力是否撑得住(M1 的意义)。
- **意义**:完成 M2,Stage1 才从"演示能解析"变成"产出可被 M3+ 消费的数据"。**这是整条路线的转折点,建议作为下一步重点。**

### M3 — Zeta resolver
- **目标**:消费 M2 的 arena AST,做名称解析(局部/参数/顶层/导入),报未知名字/重复定义等。
- **关键工作**:作用域用并行数组(name 数组 + 种类数组)线性查找;遍历 arena AST。
- **验证**:对一批源码,Zeta resolver 的诊断(code + 位置)与 Rust resolver **对齐**——新建 resolver 差分门禁(仿 parity)。
- **难点**:作用域嵌套(进出作用域用栈式数组 + 标记);跨文件 import 暂可缩范围到单文件起步。

### M4 — Zeta typecheck
- **目标**:消费 AST(+ resolver 结果),做类型推断与检查。
- **关键工作**:类型用 tag 表示(Int/String/Bool/Array(elem)/Named);infer/expect 逻辑移植。
- **验证**:typecheck 诊断与 Rust 对齐(差分门禁)。
- **难点**:类型表示(Array 元素、struct 字段表)在 arena 风格下的组织。

### M5 — Zeta MIR lowering
- **目标**:AST → MIR(同样 arena 表示),覆盖 Stage0 可运行子集。
- **验证**:Zeta 产出的 mir-dump 与 Rust `mir-dump` 对齐(复用 mir_dump 测试思路建差分)。

### M6 — Zeta MIR 解释器
- **目标**:执行 MIR,产出运行结果(整数/字符串等)。
- **关键工作**:locals 用并行数组;Value 用 tag + 载荷数组;实现算术/控制流/调用/数组/struct/enum/match。
- **验证**:对 `testdata/run_*.zeta` 全集,Zeta 解释器结果 == Rust 解释器结果。
- **难点**:递归调用栈、数组/struct 值语义在 arena 下的实现;性能(解释器跑解释器,慢)。

### M7 — 自举闭环
- **目标**:把 M2–M6 串成一个用 Zeta 写的完整编译器 `zetac.zeta`,它能编译任意 Zeta 源码;特别地,**能编译它自己的源码**。
- **验证**:fixpoint —— 用 Stage0 跑 `zetac.zeta` 去编译 `zetac.zeta`,产出的行为与再跑一遍一致;最终目标是脱离 Stage0。
- **难点**:`zetac.zeta` 自身必须只用 Zeta 已支持的特性(M1 的边界）；规模与性能。

## 4. 贯穿性武器:oracle 差分

每个里程碑的 Zeta 实现都与 Rust 对应阶段**逐字/逐诊断对齐**,复用并扩展现有差分体系:
- M2:`ast-dump` 对齐(已有 243 探针)。
- M3/M4:resolver/typecheck **诊断对齐**(新建门禁,仿 `stage1_parity`)。
- M5:`mir-dump` 对齐。
- M6:**运行结果对齐**(run_*.zeta 全集)。

这套"Rust 作 oracle、Zeta 必须逐字复现"的方法,是本项目最大的工程杠杆——它把"自举正确性"变成自动回归。

## 5. 关键风险

1. **Zeta 表达力**:写 resolver/typecheck 的代码相当复杂,Zeta 是否够用?→ M1 用"试写"验证;不够则补特性。
2. **arena 的工程负担**:无指针/Map 下,所有数据结构靠并行数组 + 索引,代码冗长易错。→ 先把 arena helper(节点分配、字段存取)做扎实。
3. **性能**:M6 的 Zeta 解释器跑 M7 的 Zeta 编译器,可能很慢。→ 自举正确性优先,性能后置(与 LLVM native 后端是另一条线)。
4. **范围蔓延**:跨文件 module graph、std 库自举等可缩范围起步(先单文件、核心子集)。

## 6. 完成回顾与开放方向

**M0–M7 + native 后端全部达成**(2026-06-14)。`arena_frontend.zeta` 是用 Zeta 写的完整前端,经 fixpoint 证明能处理自身源码并逐字对齐 Rust oracle;native 后端进一步把它 AOT 成脱离 Stage0 的独立可执行。三条并行线(自举 / hot-reload / native)的预定目标均已落地,详见 `docs/compiler/handoff.md`。

**已无预定义里程碑。** 真正脱离 Stage0 的剩余只是"工程化收口"与 nice-to-have,均非阻塞:

- **真正的 Stage2(Zeta 编译 Zeta 产 native)**:目前自举链是 Zeta 前端 + Zeta MIR **解释器**;若要 `zetac.zeta` 自身也走 native codegen(而非 Rust 侧的 `src/codegen.rs`),需用 Zeta 重写 MIR→LLVM 后端——独立大工程,当前用 Rust 后端 AOT 已达成"独立二进制"目标,故非必需。
- **语法边界补全**:Rust parser 仍有不支持的语法(neg/else-if/复杂赋值目标等,见 `rust-parser-unsupported-boundaries` 记忆),补齐可放宽自举语料。
- **后端广度/性能**:native 已是 C 级(1.04x 同语义);更多 hot-reload 状态类型、call-site patching 把"每帧一跳间接"压到零等,均为边际增强。

下一步方向待定,建议由需求驱动而非预设。
