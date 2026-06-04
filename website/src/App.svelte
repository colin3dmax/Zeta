<script>
  import { onMount } from "svelte";
  import { runZeta } from "./wasm-playground.js";
  import PlaygroundSection from "./PlaygroundSection.svelte";

  const keywords = new Set(["module", "import", "as", "export", "fn", "let", "mut", "return", "break", "continue", "if", "else", "while", "match", "struct", "enum"]);
  const types = new Set(["Int", "String", "Bool", "IntArray", "StringArray", "BoolArray"]);
  const commands = [":help", ":api", ":topics", ":examples", ":doc", ":complete", ":quit"];
  const topics = [
    "getting-started",
    "tutorial",
    "api",
    "std",
    "playground",
    "module",
    "import",
    "as",
    "fn",
    "let",
    "mut",
    "if",
    "while",
    "break",
    "continue",
    "match",
    "struct",
    "enum",
    "Int",
    "String",
    "Bool",
    "IntArray",
    "StringArray",
    "BoolArray",
    "string_len",
    "string_byte_at",
    "string_byte_slice",
    "ascii_is_digit",
    "ascii_is_alpha",
    "ascii_is_alnum",
    "ascii_is_whitespace"
  ];
  const docs = {
    "getting-started": "从表达式开始：输入 40 + 2 可以直接执行；使用 let 声明局部绑定；需要重新赋值时使用 let mut；if/while 条件可以使用比较和布尔逻辑表达式。",
    tutorial: "推荐路径：表达式 -> let/let mut -> 比较/布尔逻辑/控制流 -> fn -> struct 字面量/字段访问 -> enum 变体 -> match -> check/run -> Playground/REPL。",
    api: "Stage 0 API 覆盖 Int、String、Bool、IntArray/StringArray/BoolArray、std.core 字符串 byte 扫描、module/import/import alias、std.core/std.io、fn、let/let mut、赋值、比较、布尔逻辑、数组字面量/下标/.len、return、if/while/break/continue、struct 字面量、字段访问、enum 变体和 match。",
    std: "std 是 Stage 0 标准 API 边界。当前 resolver 接受 import std.core; 和 import std.io;，未知标准库路径会报错；具体 IO 函数在后续权限模型确定后接入。",
    playground: "Playground 通过 zeta.wasm 运行真实编译器前端，支持 AST、检查和运行。",
    module: "module 声明当前源码模块，例如 module demo.core;",
    import: "import 引入模块路径。多文件模块可写 import demo.math as math; 然后调用 math.answer();",
    as: "as 为 import 创建本地别名，例如 import demo.math as math;",
    fn: "fn 声明函数，例如 fn main() -> Int { return 42; }",
    let: "let 声明局部绑定，例如 let answer: Int = 40 + 2; 需要重新赋值时写 let mut answer: Int = 40;",
    mut: "mut 标记可变局部绑定，之后可以执行 answer = answer + 2;",
    if: "if 使用 Bool 条件分支，例如 if ready && !done { return 42; }",
    while: "while 使用 Bool 条件循环，例如 while count < 3 && ready { count = count + 1; }；循环内可用 break 和 continue。",
    break: "break 跳出最近一层 while 循环。",
    continue: "continue 跳过当前 while 迭代剩余语句，进入下一轮条件检查。",
    match: "match 对 Int/String/Bool 字面量、enum 变体和 _ 通配模式执行分支。",
    struct: "struct 声明记录类型，当前可用 User { name: \"Ada\", age: 42 } 构造值，并用 user.age 访问字段。",
    enum: "enum 声明标签集合，当前可用 ResultTag.Ok 构造变体值，并在 match 中分支。",
    Int: "Int 是当前 Stage 0 的整数标量类型。",
    String: "String 是当前 Stage 0 的字符串标量类型。",
    Bool: "Bool 是 if/while 条件使用的布尔类型；&&、||、! 会组合或取反 Bool。",
    IntArray: "IntArray 是同质 Int 数组；支持 [1, 2] 字面量、Int 下标访问和 .len。",
    StringArray: "StringArray 是同质 String 数组；支持字符串数组字面量、Int 下标访问和 .len。",
    BoolArray: "BoolArray 是同质 Bool 数组；支持布尔数组字面量、Int 下标访问和 .len。",
    string_len: "std.core 内建函数，返回 String 的 UTF-8 byte 长度。",
    string_byte_at: "std.core 内建函数，用 Int 下标读取 String 的单个 byte，并以 Int 返回。",
    string_byte_slice: "std.core 内建函数，用 byte 起点和 byte 长度截取 String。",
    ascii_is_digit: "std.core 内建函数，判断 Int byte 是否是 ASCII 数字。",
    ascii_is_alpha: "std.core 内建函数，判断 Int byte 是否是 ASCII 字母。",
    ascii_is_alnum: "std.core 内建函数，判断 Int byte 是否是 ASCII 字母或数字。",
    ascii_is_whitespace: "std.core 内建函数，判断 Int byte 是否是 ASCII 空白字符。"
  };

  const navItems = [
    { id: "overview", label: "概览" },
    { id: "install", label: "安装" },
    { id: "start", label: "快速开始" },
    { id: "features", label: "语言特性" },
    { id: "repl", label: "交互终端" },
    { id: "playground", label: "Playground" },
    { id: "tutorial", label: "教程" },
    { id: "vscode", label: "VS Code" },
    { id: "design", label: "设计文档" },
    { id: "roadmap", label: "路线图" }
  ];

  const sample = `module demo.core;
import std.core;
import std.io;

export fn main() -> Int {
  let mut count: Int = 0;
  let mut total: Int = 0;
  while count < 10 {
    count = count + 1;
    if count == 3 {
      continue;
    }
    if count == 6 {
      break;
    }
    total = total + count;
  }
  if total == 12 {
    return 42;
  }
  return 0;
}`;

  const playgroundExamples = {
    overview: sample,
    bindings: `fn main() -> Int {
  let mut answer: Int = 40;
  answer = answer + 2;
  return answer;
}`,
    control: `fn main() -> Int {
  let mut count: Int = 0;
  let mut total: Int = 0;
  while count < 10 {
    count = count + 1;
    if count == 3 {
      continue;
    }
    if count == 6 {
      break;
    }
    total = total + count;
  }
  if total == 12 {
    return 42;
  }
  return 0;
}`,
    functions: `fn add(left: Int, right: Int) -> Int {
  return left + right;
}

fn main() -> Int {
  return add(40, 2);
}`,
    data: `module demo.data;

export struct User {
  name: String,
  age: Int,
}

enum ResultTag {
  Ok,
  Err,
}`,
    struct: `struct User {
  name: String,
  age: Int,
}

fn main() -> Int {
  let user: User = User { name: "Ada", age: 42 };
  return user.age;
}`,
    enum: `enum ResultTag {
  Ok,
  Err,
}

fn main() -> Int {
  let tag: ResultTag = ResultTag.Ok;
  match tag {
    ResultTag.Ok -> { return 42; },
    ResultTag.Err -> { return 0; },
  }
  return 0;
}`,
    match: `fn main() -> Int {
  let value: Int = 2;
  match value {
    1 -> { return 10; },
    2 -> { return 42; },
    _ -> { return 0; },
  }
  return 0;
}`,
    bool: `fn main() -> Bool {
  return true && !false;
}`,
    arrays: `fn main() -> Int {
  let values: IntArray = [2, 4, 6];
  return values[0] + values[1] + values.len;
}`,
    stringScan: `import std.core;

fn main() -> Int {
  let text: String = "A9 zeta";
  let first: Int = string_byte_at(text, 0);
  let digit: Int = string_byte_at(text, 1);
  let space: Int = string_byte_at(text, 2);
  let tail: String = string_byte_slice(text, 3, 4);
  if string_len(text) == 7 && ascii_is_alpha(first) && ascii_is_digit(digit) && ascii_is_whitespace(space) && string_len(tail) == 4 {
    return first + digit;
  }
  return 0;
}`,
    modules: `// file: main.zeta
module demo.app;
import demo.math;

fn main() -> Int {
  return answer();
}

// file: math.zeta
module demo.math;

export fn answer() -> Int {
  return 42;
}`,
    modulesQualified: `// file: main.zeta
module demo.app;
import demo.math;

fn main() -> Int {
  return demo.math.answer();
}

// file: math.zeta
module demo.math;

export fn answer() -> Int {
  return helper();
}

fn helper() -> Int {
  return 42;
}`,
    modulesAlias: `// file: main.zeta
module demo.app;
import demo.math as math;

fn main() -> Int {
  return math.answer();
}

// file: math.zeta
module demo.math;

export fn answer() -> Int {
  return 42;
}`,
    modulesAmbiguous: `// file: main.zeta
module demo.app;
import demo.math;
import demo.more;

fn main() -> Int {
  return answer();
}

// file: math.zeta
module demo.math;

export fn answer() -> Int {
  return 40;
}

// file: more.zeta
module demo.more;

export fn answer() -> Int {
  return 2;
}`
  };

  const featureTests = [
    { name: "模块/import/export", mode: "check-module-graph", example: "modules", expected: "ok" },
    { name: "跨模块限定调用", mode: "run-module-graph", example: "modulesQualified", expected: "42" },
    { name: "import alias 调用", mode: "run-module-graph", example: "modulesAlias", expected: "42" },
    { name: "短名冲突诊断", mode: "check-module-graph", example: "modulesAmbiguous", expectedOk: false, expectedIncludes: "RESOLVE_AMBIGUOUS_FUNCTION" },
    { name: "Int 算术", mode: "run", source: "fn main() -> Int { return 40 + 2; }", expected: "42" },
    { name: "Bool 逻辑", mode: "run", example: "bool", expected: "true" },
    { name: "数组字面量 / 下标 / len", mode: "run", example: "arrays", expected: "9" },
    { name: "std.core 字符串扫描", mode: "run", example: "stringScan", expected: "122" },
    { name: "let mut / 赋值", mode: "run", example: "bindings", expected: "42" },
    { name: "函数调用", mode: "run", example: "functions", expected: "42" },
    { name: "if / while", mode: "run", example: "control", expected: "42" },
    { name: "struct 字面量 / 字段访问", mode: "run", example: "struct", expected: "42" },
    { name: "enum 变体", mode: "run", example: "enum", expected: "42" },
    { name: "match 分支", mode: "run", example: "match", expected: "42" },
    { name: "数据声明 AST", mode: "ast", example: "data", expectedIncludes: "Struct name=User" },
    { name: "标准 import 边界", mode: "check", source: "import std.core;\nimport std.io;\nfn main() -> Int { return 42; }", expected: "ok" },
    { name: "MIR 返回路径诊断", mode: "run", source: "fn main() -> Int {\n  let answer: Int = 42;\n}", expectedOk: false, expectedIncludes: "MIR_MISSING_RETURN" }
  ];

  let active = "overview";
  let source = sample;
  let output = "选择 AST、检查、运行查看真实 Zeta 编译器前端结果。多文件示例会自动使用模块图。";
  let runningMode = "";
  let sourceScrollTop = 0;
  let sourceScrollLeft = 0;
  let sourceCompletionOpen = false;
  let sourceCompletionPrefix = "";
  let replInput = "";
  let replRunning = false;
  let featureTestRunning = false;
  let featureTestOutput = "尚未运行。";
  let featureTestResults = [];
  let replCompletionOpen = false;
  let replCompletionPrefix = "";
  let replInputEl;
  let embedMode = false;
  let initialPlaygroundExample = "overview";
  $: playgroundModeHint = hasVirtualFiles(source)
    ? "当前源码包含多个 // file: 文件块：检查和运行会自动使用模块图，跨文件 import/export 可以一起解析。"
    : "当前源码按单文件执行：检查只验证当前文件，运行执行当前文件里的无参数 main。";
  let replLines = [
    { kind: "system", text: "Zeta Web REPL · 输入 :help 查看命令，输入 40 + 2 直接运行。" }
  ];

  async function runPlayground(mode) {
    const effectiveMode = playgroundModeForSource(mode, source);
    runningMode = effectiveMode;
    output = "running...";
    try {
      const result = await runZeta(effectiveMode, source);
      output = result.output;
    } catch (error) {
      output = `Playground failed: ${error.message}`;
    } finally {
      runningMode = "";
    }
  }

  function playgroundModeForSource(mode, value) {
    if (!hasVirtualFiles(value)) return mode;
    if (mode === "check") return "check-module-graph";
    if (mode === "run") return "run-module-graph";
    return mode;
  }

  function hasVirtualFiles(value) {
    return /^\/\/\s*file:\s+\S+/m.test(value);
  }

  async function runFeatureTests() {
    featureTestRunning = true;
    featureTestOutput = "running...";
    featureTestResults = [];
    const results = [];
    for (const test of featureTests) {
      const testSource = test.source ?? playgroundExamples[test.example];
      try {
        const result = await runZeta(test.mode, testSource);
        const expectedOk = test.expectedOk ?? true;
        const outputMatches = (
          test.expectedIncludes
            ? result.output.includes(test.expectedIncludes)
            : result.output.trim() === test.expected
        );
        const passed = result.ok === expectedOk && outputMatches;
        results.push({ ...test, passed, output: result.output });
      } catch (error) {
        results.push({ ...test, passed: false, output: error.message });
      }
    }
    featureTestResults = results;
    const passed = results.filter((result) => result.passed).length;
    featureTestOutput = `${passed}/${results.length} passed`;
    featureTestRunning = false;
  }

  function loadFeatureTest(test) {
    source = test.source ?? playgroundExamples[test.example] ?? sample;
    output = `Feature test: ${test.name}\nMode: ${test.mode}\nExpected: ${test.expected ?? test.expectedIncludes}\nExpected ok: ${test.expectedOk ?? true}`;
    active = "playground";
  }

  function escapeHtml(value) {
    return value
      .replaceAll("&", "&amp;")
      .replaceAll("<", "&lt;")
      .replaceAll(">", "&gt;");
  }

  function highlightCode(value) {
    return value.replace(/(&&|\|\||==|!=|<=|>=|->|:[a-z-]+|"(?:[^"\\]|\\.)*"|\b[A-Za-z_][A-Za-z0-9_]*\b|\b\d+\b|[=!<>+*/:-])/g, (part) => {
      const escaped = escapeHtml(part);
      if (commands.includes(part)) return `<span class="tok-command">${escaped}</span>`;
      if (keywords.has(part)) return `<span class="tok-keyword">${escaped}</span>`;
      if (types.has(part)) return `<span class="tok-type">${escaped}</span>`;
      if (part === "true" || part === "false") return `<span class="tok-bool">${escaped}</span>`;
      if (/^"/.test(part)) return `<span class="tok-string">${escaped}</span>`;
      if (/^\d+$/.test(part)) return `<span class="tok-number">${escaped}</span>`;
      if (/^(&&|\|\||==|!=|<=|>=|->|[=!<>+*/:-])$/.test(part)) return `<span class="tok-operator">${escaped}</span>`;
      return escaped;
    });
  }

  function syncSourceScroll(event) {
    sourceScrollTop = event.currentTarget.scrollTop;
    sourceScrollLeft = event.currentTarget.scrollLeft;
  }

  function completionPrefix(value) {
    const match = value.match(/(:?[A-Za-z0-9_-]+)$/);
    return match ? match[1] : "";
  }

  function completions(prefix) {
    if (!prefix) return [];
    const words = [...commands, ...topics, ...keywords, ...types].sort();
    return [...new Set(words)].filter((word) => word.startsWith(prefix)).slice(0, 8);
  }

  $: sourceSuggestions = completions(sourceCompletionPrefix);
  $: replSuggestions = completions(replCompletionPrefix);

  function showSourceCompletion() {
    sourceCompletionPrefix = completionPrefix(source.slice(0, document.activeElement?.selectionStart ?? source.length));
    sourceCompletionOpen = sourceSuggestions.length > 0;
  }

  function onSourceKeydown(event) {
    if (event.key === "Tab") {
      event.preventDefault();
      const prefix = completionPrefix(source.slice(0, event.currentTarget.selectionStart));
      const match = completions(prefix)[0];
      if (match) {
        applyTextareaCompletion(event.currentTarget, prefix, match);
      }
      sourceCompletionPrefix = prefix;
      sourceCompletionOpen = completions(prefix).length > 1;
    }
  }

  function applyTextareaCompletion(textarea, prefix, value) {
    const start = textarea.selectionStart - prefix.length;
    const end = textarea.selectionStart;
    source = source.slice(0, start) + value + source.slice(end);
    requestAnimationFrame(() => {
      const pos = start + value.length;
      textarea.setSelectionRange(pos, pos);
      textarea.focus();
    });
  }

  function applyReplCompletion(value) {
    const prefix = completionPrefix(replInput);
    replInput = replInput.slice(0, replInput.length - prefix.length) + value;
    replCompletionOpen = false;
  }

  function replHelp() {
    return [
      ":help                 显示命令和学习路径",
      ":api                  查看 Stage 0 API / 标准库概览",
      ":topics               列出终端内置文档主题",
      ":examples             显示可直接运行的示例",
      ":doc <topic>          查询主题文档",
      ":complete <prefix>    查看补全候选",
      ":quit                 清空当前输入"
    ].join("\n");
  }

  function replExamples() {
    return ["40 + 2", "1 + 1 == 2", "true && !false", "let mut count: Int = 0;", "count = count + 1;", "fn main() -> Int { let values: IntArray = [2, 4, 6]; return values[0] + values.len; }", "import std.core; fn main() -> Int { return string_len(\"zeta\") + string_byte_at(\"A9\", 1); }", "fn main() -> Int { if true && !false { return 42; } return 0; }", "module demo.core;", ":doc string_len"].join("\n");
  }

  function replApi() {
    return "Zeta Stage 0 API\nInt/String/Bool/IntArray/StringArray/BoolArray\nstd.core: string_len/string_byte_at/string_byte_slice/ascii_is_digit/ascii_is_alpha/ascii_is_alnum/ascii_is_whitespace\nmodule/import/import alias/std.core/std.io/fn/let/let mut/assignment/comparison/boolean logic/array literals/index/.len/return/if/while/break/continue/struct literal/field access/enum variants/match\nstd: 当前可导入 std.core 和 std.io；未知标准库路径会被 resolver 拒绝。";
  }

  async function submitRepl() {
    const input = replInput.trim();
    if (!input || replRunning) return;
    replLines = [...replLines, { kind: "input", text: input }];
    replInput = "";
    replCompletionOpen = false;

    if (input === ":help") return pushRepl("system", replHelp());
    if (input === ":api") return pushRepl("system", replApi());
    if (input === ":topics") return pushRepl("system", topics.join(", "));
    if (input === ":examples") return pushRepl("system", replExamples());
    if (input === ":quit") return pushRepl("system", "Web REPL 会话已清空。");
    if (input.startsWith(":doc ")) return pushRepl("system", docs[input.slice(5).trim()] ?? `unknown doc topic ${input.slice(5).trim()}`);
    if (input.startsWith(":complete ")) return pushRepl("system", completions(input.slice(10).trim()).join(" ") || "no completions");

    replRunning = true;
    try {
      const result = await runZeta("run", replSourceFor(input));
      pushRepl(result.ok ? "value" : "error", result.output);
    } catch (error) {
      pushRepl("error", error.message);
    } finally {
      replRunning = false;
    }
  }

  function pushRepl(kind, text) {
    replLines = [...replLines, { kind, text }];
  }

  function replSourceFor(input) {
    if (/^(module|import|export|fn|struct|enum)\b/.test(input)) {
      return input;
    }
    if (input.endsWith(";")) {
      return `fn main() {\n  ${input}\n}`;
    }
    if (/\b(?:true|false)\b|&&|\|\||!|==|!=|<=|>=|<|>/.test(input)) {
      return `fn main() -> Bool {\n  return ${input};\n}`;
    }
    return `fn main() -> Int {\n  return ${input};\n}`;
  }

  function onReplInput() {
    replCompletionPrefix = completionPrefix(replInput);
    replCompletionOpen = replSuggestions.length > 0;
  }

  function onReplKeydown(event) {
    if (event.key === "Enter") {
      event.preventDefault();
      submitRepl();
    } else if (event.key === "Tab") {
      event.preventDefault();
      if (replSuggestions[0]) applyReplCompletion(replSuggestions[0]);
    }
  }

  function focusRepl() {
    replInputEl?.focus();
  }

  function loadPlaygroundExample(name) {
    const example = playgroundExamples[name] ?? playgroundExamples.overview;
    source = example;
    output = "选择 AST、检查、运行查看真实 Zeta 编译器前端结果。多文件示例会自动使用模块图。";
    active = "playground";
  }

  onMount(() => {
    const params = new URLSearchParams(location.search);
    const example = params.get("example");
    embedMode = params.get("embed") === "playground";
    initialPlaygroundExample = example || "overview";
    if (example) {
      loadPlaygroundExample(example);
    }
    if (location.hash === "#repl") {
      focusRepl();
    } else if (location.hash === "#playground") {
      active = "playground";
    }
  });
</script>

<svelte:head>
  <title>Zeta 编程语言</title>
</svelte:head>

{#if embedMode}
  <main class="component-embed-main">
    <PlaygroundSection initialExample={initialPlaygroundExample} embedded={true} />
  </main>
{:else}
<header class="home-topbar">
  <a class="home-brand" href="#overview" on:click={() => (active = "overview")}>
    <span>Zeta</span>
    <small>Programming Language</small>
  </a>
  <nav class="home-topnav" aria-label="站点导航">
    <a class:current={active === "overview"} href="#overview" on:click={() => (active = "overview")}>官网首页</a>
    <a href="/docs/index.html">文档中心</a>
    <a href="/docs/user/getting-started.html">用户指南</a>
    <a class:current={active === "tutorial"} href="#tutorial" on:click={() => (active = "tutorial")}>在线教程</a>
    <a class:current={active === "roadmap"} href="#roadmap" on:click={() => (active = "roadmap")}>路线图</a>
    <a class:current={active === "design"} href="#design" on:click={() => (active = "design")}>编译器</a>
    <a class:current={active === "playground"} href="#playground" on:click={() => (active = "playground")}>Playground</a>
  </nav>
</header>
<main class="home-main">
  <aside class="home-sidebar" aria-label="页面目录">
    <p class="home-sidebar-title">Zeta</p>
    <nav>
      {#each navItems as item}
        <a class:current={active === item.id} href={`#${item.id}`} on:click={() => (active = item.id)}>{item.label}</a>
      {/each}
    </nav>
  </aside>
  <article class="home-content">
    <section id="overview" class="hero">
      <p class="kicker">Draft · 2026-05-28</p>
      <h1>低门槛，高上限，小内核，高性能。</h1>
      <p class="lead">
        Zeta 是一门严肃的专业编程语言，面向上层应用、服务端、桌面、移动端、WASM、操作系统、嵌入式和硬件软件。
        核心内核保持小而强，外围能力通过标准库和第三方模块按需扩展。
      </p>
      <div class="actions">
        <a href="#playground" on:click={() => (active = "playground")}>打开 Playground</a>
        <a href="#install" on:click={() => (active = "install")}>本地安装</a>
        <a href="#start" on:click={() => (active = "start")}>快速开始</a>
      </div>
    </section>

    <section class="band">
      <div>
        <span class="metric">Stage 0</span>
        <p>当前原型覆盖 parser、AST dump、基础 name resolution、typecheck 和 Stage 0 执行。</p>
      </div>
      <div>
        <span class="metric">Small Core</span>
        <p>AI、平台 SDK 和高阶运行时能力放在外部模块，不进入最小内核。</p>
      </div>
      <div>
        <span class="metric">Native First</span>
        <p>目标覆盖 native、WASM、Windows、iOS、Android 和 RISC-V。</p>
      </div>
    </section>

    <section id="install">
      <p class="kicker">Install</p>
      <h2>本地安装</h2>
      <div class="grid two">
        <article>
          <h3>源码安装</h3>
          <p>当前 Stage 0 推荐从源码构建，适用于 macOS、Linux 和 Windows。</p>
          <pre><code>git clone https://github.com/colin3dmax/Zeta.git
cd Zeta
cargo build --release
./target/release/zeta repl</code></pre>
        </article>
        <article>
          <h3>平台包状态</h3>
          <p>release 打包脚本已经可复用，官网下载页会列出已发布的 CLI 包和 SHA256。</p>
          <pre><code>sh tools/package-release.sh
ls dist/packages</code></pre>
        </article>
      </div>
      <p class="note"><a href="/docs/user/install.html">查看 macOS、Linux、Windows 安装方法和平台包状态</a></p>
      <p class="note"><a href="/docs/user/downloads.html">下载已发布的 CLI 包</a></p>
    </section>

    <section id="start">
      <p class="kicker">Getting Started</p>
      <h2>快速开始</h2>
      <div class="grid">
        <article>
          <h3>验证环境</h3>
          <pre><code>cargo test
python3 tools/check-docs.py
python3 tools/check-vscode-extension.py</code></pre>
        </article>
        <article>
          <h3>查看 AST</h3>
          <pre><code>cargo run -- ast-dump testdata/core_items.zeta</code></pre>
        </article>
        <article>
          <h3>检查源码</h3>
          <pre><code>cargo run -- check testdata/core_items.zeta</code></pre>
        </article>
        <article>
          <h3>执行程序</h3>
          <pre><code>cargo run -- run testdata/run_call.zeta</code></pre>
        </article>
      </div>
    </section>

    <section id="repl">
      <p class="kicker">Interactive Mode</p>
      <h2>交互终端</h2>
      <table>
        <thead>
          <tr><th>命令</th><th>用途</th><th>示例</th></tr>
        </thead>
        <tbody>
          <tr><td><code>:help</code></td><td>显示 REPL 命令和主题列表。</td><td><code>:help</code></td></tr>
          <tr><td><code>:doc &lt;topic&gt;</code></td><td>查询关键词或类型的短文档。</td><td><code>:doc let</code></td></tr>
          <tr><td><code>:complete &lt;prefix&gt;</code></td><td>列出当前已知补全候选。</td><td><code>:complete st</code></td></tr>
          <tr><td><code>:quit</code></td><td>退出 REPL。</td><td><code>:quit</code></td></tr>
        </tbody>
      </table>
      <p>当前 REPL 是 Stage 0 交互式语法终端：输入 <code>40 + 2</code> 会直接返回 <code>42</code>，语句执行成功会返回 <code>ok</code>。真实 TTY 下提供输入时语法着色、Tab 补全、hint、历史上下切换和左右光标移动；启用 <code>repl-rich</code> feature 后会优先使用 <code>reedline</code>，不可用时自动退回内置 line editor。</p>
      <div class="tool-window web-repl" aria-label="Zeta Web REPL">
        <div class="window-chrome">
          <div class="window-controls" aria-hidden="true">
            <span></span><span></span><span></span>
          </div>
          <span class="window-title">Zeta Web REPL</span>
          <small class="window-status">docs · api · run</small>
        </div>
        <div class="terminal-body">
          {#each replLines as line}
            <div class={`terminal-line ${line.kind}`}>
              {#if line.kind === "input"}<span class="prompt">zeta&gt;</span>{/if}
              <code>{@html highlightCode(line.text)}</code>
            </div>
          {/each}
          <div class="terminal-input-row">
            <span class="prompt">zeta&gt;</span>
            <div class="terminal-input-wrap">
              <pre class="terminal-input-highlight" aria-hidden="true"><code>{@html replInput
                ? highlightCode(replInput)
                : '<span class="terminal-placeholder">40 + 2</span>'}</code></pre>
              {#if !replInput}<span class="terminal-caret" aria-hidden="true"></span>{/if}
              <input
                bind:this={replInputEl}
                bind:value={replInput}
                on:input={onReplInput}
                on:keydown={onReplKeydown}
                spellcheck="false"
                aria-label="REPL input"
              />
            </div>
          </div>
          {#if replCompletionOpen && replSuggestions.length}
            <div class="completion-panel terminal-completion">
              {#each replSuggestions as item}
                <button type="button" on:click={() => applyReplCompletion(item)}>{item}</button>
              {/each}
            </div>
          {/if}
        </div>
        <div class="window-statusbar">
          <span>Stage 0</span>
          <span>{replRunning ? "running" : "ready"}</span>
          <span>Tab completion</span>
        </div>
      </div>
    </section>

    <PlaygroundSection initialExample={initialPlaygroundExample} />

    <section id="tutorial">
      <p class="kicker">Tutorial</p>
      <h2>在线教程</h2>
      <div class="grid two">
        <article><h3>1. 模块与函数</h3><p>从 <code>module</code>、<code>export fn</code> 和标量类型开始。</p><a href="/docs/tutorial/index.html">打开教程</a></article>
        <article><h3>2. 控制流</h3><p>学习 <code>if</code>、<code>while</code>、<code>match</code> 和返回类型。</p><a href="/docs/user/language-features.html#control-flow">查看特性</a></article>
        <article><h3>3. 数据建模</h3><p>用 <code>struct</code> 和 <code>enum</code> 描述稳定的数据边界。</p><a href="/docs/user/language-features.html#data-declarations">查看特性</a></article>
        <article><h3>4. 工具链</h3><p>掌握 <code>ast-dump</code>、<code>check</code>、REPL 和编辑器插件。</p><a href="/docs/user/language-features.html#tooling">查看特性</a></article>
      </div>
    </section>

    <section id="features">
      <p class="kicker">Language Features</p>
      <h2>语言特性学习</h2>
      <div class="grid two">
        <article><h3>表达式与标量</h3><p>Int、String、Bool、算术、比较和布尔逻辑。</p><a href="/docs/user/language-features.html#scalars-expressions">阅读并测试</a></article>
        <article><h3>绑定与局部状态</h3><p>let、let mut、赋值和类型注解。</p><a href="/docs/user/language-features.html#bindings">阅读并测试</a></article>
        <article><h3>函数与控制流</h3><p>fn、return、if/else、while 和函数调用。</p><a href="/docs/user/language-features.html#functions">阅读并测试</a></article>
        <article><h3>数据声明与工具链</h3><p>module/import/export、struct、enum、AST、检查、运行和 REPL。</p><a href="/docs/user/language-features.html#data-declarations">阅读并测试</a></article>
      </div>
    </section>

    <section id="vscode">
      <p class="kicker">Editor</p>
      <h2>VS Code 插件</h2>
      <pre><code>sh editors/vscode-zeta/scripts/install-local.sh</code></pre>
      <p>插件提供 Zeta 语法高亮、snippets、静态补全和 hover 文档。安装后打开 <code>.zeta</code> 文件即可应用。</p>
    </section>

    <section id="design">
      <p class="kicker">Design Notes</p>
      <h2>设计文档</h2>
      <div class="grid two">
        <article>
          <h3>构建决策记录</h3>
          <p>记录 Stage 0 宿主语言、MIR interpreter、官网 Playground、文档格式和 DevGame 验收方式的关键取舍。</p>
          <a href="/docs/project/decision-record.html">查看决策</a>
        </article>
        <article>
          <h3>语言设计过程</h3>
          <p>说明一个语言特性从动机、语法草案、类型规则、IR 形态、实现测试到官网发布的完整流程。</p>
          <a href="/docs/project/language-design-process.html">查看过程</a>
        </article>
        <article>
          <h3>MVP Baseline</h3>
          <p>冻结 Stage 0 到 Stage 1 的 included、support 和 deferred 边界，避免语法堆叠。</p>
          <a href="/docs/project/mvp-baseline.html">查看边界</a>
        </article>
        <article>
          <h3>编译器与自举路线</h3>
          <p>解释 Parser、Typechecker、HIR、MIR、Backend 和 self-hosting 的职责边界。</p>
          <a href="/docs/compiler/bootstrap.html">查看自举路线</a>
        </article>
        <article>
          <h3>编译器术语解释</h3>
          <p>面向非编译器背景读者解释 AST、HIR、MIR、MIR interpreter、backend、corpus 和 golden test。</p>
          <a href="/docs/compiler/glossary.html">查看术语</a>
        </article>
      </div>
    </section>

    <section id="roadmap">
      <p class="kicker">Roadmap</p>
      <h2>阶段路线图</h2>
      <div class="grid two">
        <article><h3>Stage 0A</h3><p>可运行前端：lexer、parser、resolver、typecheck、最小数组、HIR/MIR、interpreter、REPL 和 Playground。</p></article>
        <article><h3>Stage 0B</h3><p>数据建模：结构体值、字段访问、枚举变体、match 执行、Option / Result。</p></article>
        <article><h3>Stage 0C</h3><p>模块与 verifier：跨模块 name resolution、MIR verifier、corpus 分组和诊断 golden。</p></article>
        <article><h3>Stage 0D+</h3><p>backend smoke 和自举：WASM/WASI、LLVM native、Stage 1/2/3 self-hosting。</p></article>
      </div>
      <p class="note"><a href="/docs/project/stage-roadmap.html">查看完整阶段路线图</a></p>
    </section>
  </article>
</main>
{/if}
