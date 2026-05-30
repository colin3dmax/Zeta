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
2. 建立 Rust Stage 0 编译器 bootstrap/self-hosting 路线；Zig 暂不进入必需依赖，早期后端顺序为 MIR interpreter、WASM/WASI smoke、LLVM native smoke。
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
ZETA_PLAYWRIGHT_REQUIRE=/path/to/node_modules/playwright sh tools/verify.sh --deploy --live
```

## 本地安装

当前 Stage 0 推荐从源码安装和本地构建。平台安装说明见 [Zeta 本地安装](docs/user/install.html)，语言特性讲解和在线测试见 [Zeta 语言特性学习](docs/user/language-features.html)。

## 本地文档服务

```sh
python3 tools/serve-docs.py
```

打开 `http://127.0.0.1:8765/docs/index.html`。服务会监听文档、源码、示例和工具文件变化，并自动刷新浏览器。

## 官网与 Playground

官网使用 Svelte + Vite，在线 Playground 通过 `wasm32-unknown-unknown` 运行真实 Zeta 编译器前端。

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
cargo run -- check testdata/core_items.zeta
cargo run -- run testdata/run_basic.zeta
cargo run -- repl
```

当前 `check` 会执行 parse、最小 name resolution 和基础 typecheck，覆盖重复定义、未知名字、Stage 0 标准 import 边界（`std.core` / `std.io`）、`Int`/`String`/`Bool` 字面量、算术表达式、比较表达式、布尔逻辑表达式、let 注解、`let mut` 可变绑定、赋值语句、结构体字面量、字段访问、枚举变体值、`match` 分支、if/while 条件、函数调用和 return 类型。`hir-dump` 会在 check 通过后输出稳定 HIR 文本，作为后续 MIR/golden tests 的输入基线。`mir-dump` 当前覆盖 Stage 0 可运行子集，输出 locals、temps、store、return、call、结构体字面量、字段访问、枚举变体、`match` 和基础控制流的稳定 MIR 文本。`run` 会先 lower 到结构化 MIR，再通过 Stage 0 MIR interpreter 执行无参数 `main`；当前支持整数算术、比较运算、布尔逻辑、函数调用、结构体字面量、字段访问、枚举变体值、`let`、`let mut`、赋值、`return`、`match` 和基础 `if/while`。`repl` 当前可以直接计算表达式，例如输入 `40 + 2` 返回 `42`；真实 TTY 下提供无依赖 line editor，支持输入时语法高亮、Tab 补全、hint、历史上下切换和左右光标移动。

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
