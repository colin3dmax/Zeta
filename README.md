# Zeta

Zeta 是一门面向 AI 时代应用的严肃工程编程语言：语法入门门槛接近 JavaScript，性能和控制力面向系统级开发，默认安全、可脚本化、可编译部署、可自举、自编译，并面向桌面、服务器、移动端、WebAssembly、操作系统、嵌入式和硬件软件提供统一的工程体验。

## 定位

Zeta 的目标不是简单复制 JavaScript、Go、Rust 或 C++，而是把它们在真实工程中的关键能力组合成一门新的系统应用语言：

- 像 JavaScript 一样适合脚本化运行、快速试验和胶水代码。
- 像 Go 一样具备简单工程模型、快速构建和良好的并发体验。
- 像 Rust 一样重视内存安全、数据竞争防护和高性能抽象。
- 像 C/C++ 一样可以编译成原生二进制，并进入系统、嵌入式和高性能场景。
- 面向 AI 原生应用内建 schema、tool、agent、capability、trace 和 policy 等工程能力。

Zeta 的核心取舍是低门槛和高上限同时成立：上层应用开发应足够直观，系统、操作系统、嵌入式和硬件软件开发仍必须有明确的内存、ABI、runtime、性能和二进制体积边界。

## 核心特性

- Script Mode：直接运行源码，适合脚本、CLI、数据处理和 AI agent 工作流。
- AOT Build Mode：编译为原生二进制，适合生产服务、桌面应用和高性能模块。
- Component Mode：编译为 WebAssembly Component，适合插件、沙箱、边缘计算和跨语言集成；浏览器目标单独走 wasm32-browser。
- Small artifacts：release 构建默认支持 runtime 裁剪、按需链接、strip/LTO，避免把未使用的 std、AI 库或调试元数据打进产物。
- Self-hosting：语言和编译器最终由 Zeta 自身实现与迭代。
- Cross-platform：覆盖 PC、移动端、Web、服务器和嵌入式设备。
- Safe by default：默认无空指针、无未初始化变量、无悬垂引用和无未受控数据竞争。
- Capability-first：文件、网络、系统命令、AI 模型调用等能力必须显式声明和授权。
- HTML-first docs：正式设计文档优先使用中文 HTML 编写，后续支持多语言版本。

## 文档

正式文档使用 HTML 格式维护：

- [项目总览](docs/index.html)
- [语言定位与产品原则](docs/project/vision.html)
- [MVP Baseline 与冻结边界](docs/project/mvp-baseline.html)
- [构建决策记录](docs/project/decision-record.html)
- [语言设计过程](docs/project/language-design-process.html)
- [阶段路线图](docs/project/stage-roadmap.html)
- [语言生态与未来趋势分析](docs/project/language-landscape.html)
- [编译器与自举路线](docs/compiler/bootstrap.html)
- [跨平台与运行时架构](docs/platform/targets.html)
- [AI 原生能力与权限模型](docs/spec/ai-capability.html)
- [HTML 文档规范](docs/project/html-docs.html)
- [用户快速开始](docs/user/getting-started.html)
- [VS Code 插件使用说明](docs/user/vscode.html)

## 编辑器支持

- [VS Code Zeta 扩展](editors/vscode-zeta/README.md)：提供 `.zeta` 语法高亮、语言配置和基础 snippets。

## 当前阶段

项目处于语言设计和工程框架启动阶段。当前优先级：

1. 固定语言定位、MVP Baseline 和文档规范。
2. 建立 Rust Stage 0 编译器 bootstrap/self-hosting 路线；Zig 暂不进入必需依赖，早期后端顺序为 MIR interpreter、WASM/WASI smoke、LLVM native smoke。编译器术语解释见 `docs/compiler/glossary.html`。
3. 定义 core/alloc/std/runtime 分层和跨平台 target matrix。
4. 设计 capability、schema、tool、agent 的 AI 原生契约。
5. 使用 DevGame 进行任务拆解、状态流转、质量验收和多 Agent 协作。

## 开发验证

```sh
cargo test
python3 tools/check-docs.py
python3 tools/check-vscode-extension.py
```

一键连续验证：

```sh
sh tools/verify.sh
```

`tools/verify.sh` 会依次执行格式检查、Rust 测试、WASM smoke、文档检查、VS Code 插件检查、WASM/官网构建和 `git diff --check`；任一步失败都会立即以非零状态退出。需要发布并跑线上 smoke 时使用：

```sh
cd website && npm install
cd ..
sh tools/verify.sh --deploy --live
```

## 本地安装

当前 Stage 0 推荐从源码安装和本地构建。平台安装说明见 [Zeta 本地安装](docs/user/install.html)，语言特性讲解和在线测试见 [Zeta 语言特性学习](docs/user/language-features.html)。

## 本地文档服务

```sh
python3 tools/serve-docs.py
```

打开 `http://127.0.0.1:8765/docs/index.html`。服务会监听文档、源码、示例和工具文件变化，并自动刷新浏览器。

## 官网与 Playground

官网使用 Svelte + Vite，在线 Playground 通过 `wasm32-unknown-unknown` 运行真实 Zeta 编译器前端。`tools/smoke-wasm.sh` 会验证 Playground exports，并运行 struct、enum、match、数组、字符串 byte 扫描、字符串构造、typed array builder、std.io 路径/诊断和 module graph smoke；WASI target 未安装时会明确跳过。

```sh
sh tools/smoke-wasm.sh
sh tools/build-website.sh
cd website
npm run dev
```

发布官网：

```sh
sh tools/deploy-website.sh
```

脚本会构建 Zeta WebAssembly、构建 Svelte 官网、同步到 `zeta.jennieapp.com`，再测试 Nginx 配置并 reload。SSH 默认关闭 `ProxyCommand` 和 `ProxyJump`。

官网 Playground 加载真实的 `zeta.wasm`，当前提供 AST、Check 和 Run 三个模式；官网也提供 Web REPL，支持在线输入表达式、查询 `:help` / `:api` / `:doc` / `:examples`，并共享同一套 WASM 执行路径。每次新增语言、REPL、编译或 Playground 能力时，需要同步更新 `website/src/App.svelte` 和 `docs/user/getting-started.html` 里的用户说明与示例。

## 当前 CLI

```sh
cargo run -- ast-dump testdata/core_items.zeta
cargo run -- hir-dump testdata/core_items.zeta
cargo run -- mir-dump testdata/run_mut.zeta
cargo run -- symbols-dump testdata/modules_ok
cargo run -- check testdata/core_items.zeta
cargo run -- check testdata/modules_ok
cargo run -- run testdata/run_basic.zeta
cargo run -- run testdata/run_array.zeta
cargo run -- run testdata/run_array_builder.zeta
cargo run -- run testdata/run_string_scan.zeta
cargo run -- run testdata/run_string_build.zeta
cargo run -- run testdata/run_io_path_diagnostic.zeta
cargo run -- run testdata/modules_ok
cargo run -- run testdata/modules_qualified
cargo run -- run testdata/modules_alias
cargo run -- run testdata/stage1_frontend
cargo run -- repl
```

当前 `check` 会执行 parse、最小 name resolution 和基础 typecheck；传入目录时会递归读取 `.zeta` 文件，建立最小 module graph，允许导入同批检查中的本地模块，并让被导入模块里的 `export fn` / `export struct` / `export enum` 在当前文件中可用；`export import` 可 re-export 函数、结构体和枚举；缺失模块、重复 module 声明、未导出的跨文件函数、冲突导出短名和冲突导入/重导出类型都会被拒绝。检查范围覆盖重复定义、未知名字、未知类型名、Stage 0 标准 import 边界（`std.core` / `std.io`）、`std.core` 的 `OptionInt` / `ResultInt` 标准枚举、字符串 byte 扫描函数、字符串构造函数和 typed array builder、`std.io` 的 `ResultString`、文件读取、路径和诊断格式化函数、本地 module import、import alias（如 `import demo.math as math;`）、跨文件导出函数调用、限定函数调用（如 `demo.math.answer()` / `math.answer()`）、`Int`/`String`/`Bool`/`IntArray`/`StringArray`/`BoolArray` 字面量和注解、数组字面量、数组下标、数组 `.len`、算术表达式、比较表达式、布尔逻辑表达式、let 注解、`let mut` 可变绑定、赋值语句、结构体字面量、字段访问、枚举变体值、`match` 分支、if/while 条件、循环内 `break`/`continue`、函数调用和 return 类型。未知类型名会在 struct 字段、enum payload、函数参数/返回类型和局部类型注解中报告 `TYPE_UNKNOWN_TYPE`。`hir-dump` 会在 check 通过后输出稳定 HIR 文本，作为后续 MIR/golden tests 的输入基线。`mir-dump` 当前覆盖 Stage 0 可运行子集，输出 locals、temps、store、return、call、break、continue、数组字面量、数组下标、std.core 字符串函数调用、typed array builder 调用、std.io 路径/诊断调用、结构体字面量、字段访问、枚举变体、`match` 和基础控制流的稳定 MIR 文本，并在输出前运行最小 MIR verifier。`symbols-dump` 会输出目录 module graph 的稳定导出符号表，记录函数、结构体和枚举的 re-export 后原始 symbol。`run` 会先 lower 到结构化 MIR，通过 verifier 后再由 Stage 0 MIR interpreter 执行无参数 `main`；verifier 会拒绝未知 local、错误调用参数、错误 return 类型、错误 enum variant、错误数组元素类型、非 Int 数组下标、错误 std.core 字符串函数参数、错误 typed array builder 参数、错误 std.io 参数、循环外 `break`/`continue`，以及声明非 `Unit` 返回类型但没有静态保证所有路径 `return` 的函数。传入目录时会复用 module graph，找到唯一 `main` 并执行同批模块里的跨文件 `export fn` 调用，内部 MIR 使用稳定 qualified function name 做跨模块消歧；当前支持整数算术、比较运算、布尔逻辑、数组字面量、数组下标、数组 `.len`、std.core 字符串 byte 扫描、字符串构造、typed array builder、std.io 文件读取、路径和诊断格式化、函数调用、结构体字面量、字段访问、枚举变体值、`let`、`let mut`、赋值、`return`、`break`、`continue`、`match` 和基础 `if/while`。`repl` 当前可以直接计算表达式，例如输入 `40 + 2` 返回 `42`；真实 TTY 下提供无依赖 line editor，支持输入时语法高亮、Tab 补全、hint、历史上下切换和左右光标移动。

`testdata/stage1_frontend` 是第一段由 Zeta 编写的 Stage 1 前端种子：它使用 `std.core` 字符串 byte 扫描、字符串构造和 typed array builder，把源码扫描为 token kind 数组和 token lexeme 文本，并用极小 parser 产出稳定 `ast_dump_score`、文本摘要 `fn=1;let=1;return=1`、覆盖 Stage 0 关键字的 `ast_dump_keyword_summary`、覆盖 Stage 0 符号的 `ast_dump_symbol_summary`、覆盖标识符/字面量/未知 token/EOF 的 `ast_dump_token_class_summary`、只统计顶层声明 kind 的 `ast_dump_item_summary`、统计顶层 export 目标类型的 `ast_dump_export_summary`、统计顶层 import/alias/export/path segment 的 `ast_dump_import_summary`、稳定的顶层 item kind 文本 dump `ast_dump_item_dump`、带 module name、import path/alias、声明 name 和 exported flag 的 `ast_dump_named_item_dump`、输出函数参数/返回类型、struct 字段和 enum variant payload 的 `ast_dump_signature_dump`，以及对齐 Rust `ast-dump` 顶层声明、最小函数体 `Let`/`Assign`/`Return`/`If`/`While`/`Break`/`Continue`/`Match`/`ExprStmt`、算术二元表达式、比较表达式、逻辑二元表达式、一元逻辑非/负号、简单/限定/嵌套函数调用表达式、数组/索引/结构体字段中的调用表达式和嵌套结构体字面量、字段访问/索引后缀表达式、数组字面量表达式、结构体字面量表达式和括号分组子集格式的 `ast_dump_rust_item_dump`。当前样例输出 `111`，代表 1 个 `fn`、1 个 `let` 和 1 个 `return`，用于后续逐步替换 Rust lexer/parser/AST dump 的回归基线。当前 dump 仍基于 token kind、token lexeme 和 brace depth，不等同于完整 Rust AST parity。

`testdata/stage2_bootstrap/input.zeta` 是 Stage2 bootstrap harness 的固定源码样本。`stage2_bootstrap_harness_reuses_stage1_frontend_contract` 会让一个 Stage2 Zeta app 通过 `std.io.file_read_to_string` 读取该样本，并调用 Stage1 Zeta 前端的 `ast_dump_score`，期望输出仍为 `111`；其他 Stage2 harness 会用 inline source 验证 `module/import/as/export/fn/let/mut/return/break/continue/if/else/while/match/struct/enum` 关键字摘要，括号、块、数组、路径、箭头、赋值、比较、逻辑和算术符号摘要，标识符、整数、字符串、布尔值、未知 token 和 EOF 摘要，顶层 module/import/struct/enum/fn item kind 摘要，顶层 export import/struct/enum/fn 目标摘要，顶层 import alias 和 path segment 摘要，稳定顶层 item kind 文本 dump，带真实 name/path/alias 的 named item dump，函数/struct/enum 声明形状 dump，以及 Rust `ast-dump` 风格的顶层声明、最小函数体 `Let`/`Assign`/`Return`/`If`/`While`/`Break`/`Continue`/`Match`/`ExprStmt`、算术二元表达式、比较表达式、逻辑二元表达式、一元逻辑非/负号、简单/限定/嵌套函数调用表达式、数组/索引/结构体字段中的调用表达式和嵌套结构体字面量、字段访问/索引后缀表达式、数组字面量表达式、结构体字面量表达式和括号分组子集 dump。这一步固化 Stage1/Stage2 契约，不等同于完整自编译。

REPL 默认会按语言偏好显示中文或英文：`ZETA_LANG` 环境变量优先，其次读取 `~/.zeta/config.toml` 的 `language` 或 `lang`，最后根据系统 `LANG` / `LC_ALL` 判断。简体中文环境默认中文；会话内可用 `:lang zh` / `:lang en` 临时切换。终端内置学习入口包括 `:help`、`:api`、`:topics`、`:examples` 和 `:doc <topic>`。

高级 REPL 使用可选依赖 `reedline`，默认不启用，避免影响离线和跨平台基础构建：

```sh
cargo run --features repl-rich -- repl
```

如果当前终端不支持 `reedline` 需要的 cursor 查询，高级 REPL 会自动退回内置 line editor。

如需通过代理安装或更新依赖：

```sh
export https_proxy=http://127.0.0.1:33210
export http_proxy=http://127.0.0.1:33210
export all_proxy=socks5://127.0.0.1:33211
cargo test --features repl-rich
```
