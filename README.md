# Zeta

Zeta 是一门面向 AI 时代应用的编程语言：默认安全、可脚本化、可编译部署、可自举、自编译，并面向桌面、服务器、移动端、WebAssembly 和嵌入式设备提供统一的工程体验。

## 定位

Zeta 的目标不是简单复制 JavaScript、Go、Rust 或 C++，而是把它们在真实工程中的关键能力组合成一门新的系统应用语言：

- 像 JavaScript 一样适合脚本化运行、快速试验和胶水代码。
- 像 Go 一样具备简单工程模型、快速构建和良好的并发体验。
- 像 Rust 一样重视内存安全、数据竞争防护和高性能抽象。
- 像 C/C++ 一样可以编译成原生二进制，并进入系统、嵌入式和高性能场景。
- 面向 AI 原生应用内建 schema、tool、agent、capability、trace 和 policy 等工程能力。

## 核心特性

- Script Mode：直接运行源码，适合脚本、CLI、数据处理和 AI agent 工作流。
- AOT Build Mode：编译为原生二进制，适合生产服务、桌面应用和高性能模块。
- Component Mode：编译为 WebAssembly Component，适合插件、沙箱、浏览器、边缘计算和跨语言集成。
- Self-hosting：语言和编译器最终由 Zeta 自身实现与迭代。
- Cross-platform：覆盖 PC、移动端、Web、服务器和嵌入式设备。
- Safe by default：默认无空指针、无未初始化变量、无悬垂引用和无未受控数据竞争。
- Capability-first：文件、网络、系统命令、AI 模型调用等能力必须显式声明和授权。
- HTML-first docs：正式设计文档优先使用中文 HTML 编写，后续支持多语言版本。

## 文档

正式文档使用 HTML 格式维护：

- [项目总览](docs/index.html)
- [语言定位与产品原则](docs/project/vision.html)
- [编译器与自举路线](docs/compiler/bootstrap.html)
- [跨平台与运行时架构](docs/platform/targets.html)
- [AI 原生能力与权限模型](docs/spec/ai-capability.html)
- [HTML 文档规范](docs/project/html-docs.html)

## 当前阶段

项目处于语言设计和工程框架启动阶段。当前优先级：

1. 固定语言定位、MVP 特性和文档规范。
2. 建立编译器 bootstrap/self-hosting 路线。
3. 定义 core/alloc/std/runtime 分层和跨平台 target matrix。
4. 设计 capability、schema、tool、agent 的 AI 原生契约。
5. 使用 DevGame 进行任务拆解、状态流转、质量验收和多 Agent 协作。
