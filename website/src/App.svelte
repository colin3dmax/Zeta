<script>
  import { onMount } from "svelte";
  import { runZeta } from "./wasm-playground.js";

  const keywords = new Set(["module", "import", "export", "fn", "let", "mut", "return", "if", "else", "while", "match", "struct", "enum"]);
  const types = new Set(["Int", "String", "Bool"]);
  const commands = [":help", ":api", ":topics", ":examples", ":doc", ":complete", ":quit"];
  const topics = [
    "getting-started",
    "tutorial",
    "api",
    "std",
    "playground",
    "module",
    "import",
    "fn",
    "let",
    "mut",
    "if",
    "while",
    "match",
    "struct",
    "enum",
    "Int",
    "String",
    "Bool"
  ];
  const docs = {
    "getting-started": "从表达式开始：输入 40 + 2 可以直接执行；使用 let 声明局部绑定；需要重新赋值时使用 let mut；if/while 条件可以使用比较和布尔逻辑表达式。",
    tutorial: "推荐路径：表达式 -> let/let mut -> 比较/布尔逻辑/控制流 -> fn -> struct/enum -> check/run -> Playground/REPL。",
    api: "Stage 0 API 覆盖 Int、String、Bool、module/import、fn、let/let mut、赋值、比较、布尔逻辑、return、if/while、match、struct、enum 和 std 命名空间占位。",
    std: "std 是标准库命名空间占位。当前可用 import std.io; 验证 import 语法，具体 IO API 后续接入。",
    playground: "Playground 通过 zeta.wasm 运行真实编译器前端，支持 AST、Check 和 Run。",
    module: "module 声明当前源码模块，例如 module demo.core;",
    import: "import 引入模块路径，例如 import std.io;",
    fn: "fn 声明函数，例如 fn main() -> Int { return 42; }",
    let: "let 声明局部绑定，例如 let answer: Int = 40 + 2; 需要重新赋值时写 let mut answer: Int = 40;",
    mut: "mut 标记可变局部绑定，之后可以执行 answer = answer + 2;",
    if: "if 使用 Bool 条件分支，例如 if ready && !done { return 42; }",
    while: "while 使用 Bool 条件循环，例如 while count < 3 && ready { count = count + 1; }",
    match: "match 对简单模式分支。",
    struct: "struct 声明记录类型。",
    enum: "enum 声明标签集合。",
    Int: "Int 是当前 Stage 0 的整数标量类型。",
    String: "String 是当前 Stage 0 的字符串标量类型。",
    Bool: "Bool 是 if/while 条件使用的布尔类型；&&、||、! 会组合或取反 Bool。"
  };

  const navItems = [
    { id: "overview", label: "概览" },
    { id: "start", label: "快速开始" },
    { id: "repl", label: "交互终端" },
    { id: "playground", label: "Playground" },
    { id: "tutorial", label: "教程" },
    { id: "vscode", label: "VS Code" },
    { id: "design", label: "设计文档" },
    { id: "roadmap", label: "路线图" }
  ];

  const sample = `module demo.core;

export fn main() -> Int {
  let mut count: Int = 0;
  while count < 3 {
    count = count + 1;
  }
  let done: Bool = false;
  if count == 3 && !done {
    return 42;
  }
  return 0;
}`;

  let active = "overview";
  let source = sample;
  let output = "选择 AST、Check 或 Run 查看真实 Zeta 编译器前端结果。";
  let runningMode = "";
  let sourceScrollTop = 0;
  let sourceScrollLeft = 0;
  let sourceCompletionOpen = false;
  let sourceCompletionPrefix = "";
  let replInput = "";
  let replRunning = false;
  let replCompletionOpen = false;
  let replCompletionPrefix = "";
  let replInputEl;
  let replLines = [
    { kind: "system", text: "Zeta Web REPL · 输入 :help 查看命令，输入 40 + 2 直接运行。" }
  ];

  async function runPlayground(mode) {
    runningMode = mode;
    output = "running...";
    try {
      const result = await runZeta(mode, source);
      output = result.output;
    } catch (error) {
      output = `Playground failed: ${error.message}`;
    } finally {
      runningMode = "";
    }
  }

  function escapeHtml(value) {
    return value
      .replaceAll("&", "&amp;")
      .replaceAll("<", "&lt;")
      .replaceAll(">", "&gt;");
  }

  function highlightCode(value) {
    return escapeHtml(value).replace(/(&amp;&amp;|\|\||==|!=|&lt;=|&gt;=|-&gt;|[=!&lt;&gt;+*/:-]|:[a-z-]+|"(?:[^"\\]|\\.)*"|\b[A-Za-z_][A-Za-z0-9_]*\b|\b\d+\b)/g, (part) => {
      if (commands.includes(part)) return `<span class="tok-command">${part}</span>`;
      if (keywords.has(part)) return `<span class="tok-keyword">${part}</span>`;
      if (types.has(part)) return `<span class="tok-type">${part}</span>`;
      if (part === "true" || part === "false") return `<span class="tok-bool">${part}</span>`;
      if (/^"/.test(part)) return `<span class="tok-string">${part}</span>`;
      if (/^\d+$/.test(part)) return `<span class="tok-number">${part}</span>`;
      if (/^(&amp;&amp;|\|\||==|!=|&lt;=|&gt;=|-&gt;|[=!&lt;&gt;+*/:-])$/.test(part)) return `<span class="tok-operator">${part}</span>`;
      return part;
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
    return ["40 + 2", "1 + 1 == 2", "true && !false", "let mut count: Int = 0;", "count = count + 1;", "fn main() -> Int { if true && !false { return 42; } return 0; }", "module demo.core;", ":doc Bool"].join("\n");
  }

  function replApi() {
    return "Zeta Stage 0 API\nInt/String/Bool\nmodule/import/fn/let/let mut/assignment/comparison/boolean logic/return/if/while/match/struct/enum\nstd: 标准库命名空间占位，当前示例 import std.io;";
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

  onMount(() => {
    if (location.hash === "#repl") {
      focusRepl();
    }
  });
</script>

<svelte:head>
  <title>Zeta 编程语言</title>
</svelte:head>

<div class="shell">
  <aside class="sidebar" aria-label="文档导航">
    <a class="brand" href="#overview" on:click={() => (active = "overview")}>
      <span>Zeta</span>
      <small>Programming Language</small>
    </a>
    <nav>
      {#each navItems as item}
        <a class:current={active === item.id} href={`#${item.id}`} on:click={() => (active = item.id)}>{item.label}</a>
      {/each}
    </nav>
  </aside>

  <main>
    <section id="overview" class="hero">
      <p class="kicker">Draft · 2026-05-28</p>
      <h1>低门槛，高上限，小内核，高性能。</h1>
      <p class="lead">
        Zeta 是一门严肃的专业编程语言，面向上层应用、服务端、桌面、移动端、WASM、操作系统、嵌入式和硬件软件。
        核心内核保持小而强，外围能力通过标准库和第三方模块按需扩展。
      </p>
      <div class="actions">
        <a href="#playground" on:click={() => (active = "playground")}>打开 Playground</a>
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

    <section id="playground">
      <p class="kicker">Online Playground</p>
      <h2>在线使用</h2>
      <div class="tool-window playground-panel">
        <div class="window-chrome light">
          <div class="window-controls" aria-hidden="true">
            <span></span><span></span><span></span>
          </div>
          <span class="window-title">Zeta Playground</span>
          <small class="window-status">source · output · wasm</small>
        </div>
        <div class="playground">
          <div class="pane source-pane">
            <div class="pane-head">
              <span>Source</span>
              <small>main.zeta</small>
            </div>
            <div class="code-editor">
              <textarea
                bind:value={source}
                on:scroll={syncSourceScroll}
                on:input={showSourceCompletion}
                on:keydown={onSourceKeydown}
                spellcheck="false"
              ></textarea>
              <div class="code-preview" aria-label="Highlighted source preview">
                <span>Preview</span>
                <pre class="code-highlight"><code>{@html highlightCode(source)}</code></pre>
              </div>
              {#if sourceCompletionOpen && sourceSuggestions.length}
                <div class="completion-panel editor-completion">
                  {#each sourceSuggestions as item}
                    <button type="button" on:click={(event) => applyTextareaCompletion(event.currentTarget.closest(".code-editor").querySelector("textarea"), sourceCompletionPrefix, item)}>{item}</button>
                  {/each}
                </div>
              {/if}
            </div>
          </div>
          <div class="pane playground-output">
            <div class="pane-head">
              <span>Output</span>
              <div class="toolbar compact">
                <button disabled={runningMode !== ""} on:click={() => runPlayground("ast")}>AST</button>
                <button disabled={runningMode !== ""} on:click={() => runPlayground("check")}>Check</button>
                <button disabled={runningMode !== ""} on:click={() => runPlayground("run")}>Run</button>
              </div>
            </div>
            <pre class="output"><code>{output}</code></pre>
          </div>
        </div>
        <div class="window-statusbar light">
          <span>wasm frontend</span>
          <span>{runningMode || "idle"}</span>
          <span>AST · Check · Run</span>
        </div>
      </div>
      <p class="note">Playground 直接加载 Zeta 编译器前端编译出的 <code>zeta.wasm</code>，AST、Check 和 Run 都执行当前仓库里的真实 Stage 0 编译器逻辑。</p>
    </section>

    <section id="tutorial">
      <p class="kicker">Tutorial</p>
      <h2>在线教程</h2>
      <div class="grid two">
        <article><h3>1. 模块与函数</h3><p>从 <code>module</code>、<code>export fn</code> 和标量类型开始。</p></article>
        <article><h3>2. 控制流</h3><p>学习 <code>if</code>、<code>while</code>、<code>match</code> 和返回类型。</p></article>
        <article><h3>3. 数据建模</h3><p>用 <code>struct</code> 和 <code>enum</code> 描述稳定的数据边界。</p></article>
        <article><h3>4. 工具链</h3><p>掌握 <code>ast-dump</code>、<code>check</code>、REPL 和编辑器插件。</p></article>
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
      </div>
    </section>

    <section id="roadmap">
      <p class="kicker">Roadmap</p>
      <h2>阶段路线图</h2>
      <div class="grid two">
        <article><h3>Stage 0A</h3><p>可运行前端：lexer、parser、resolver、typecheck、HIR/MIR、interpreter、REPL 和 Playground。</p></article>
        <article><h3>Stage 0B</h3><p>数据建模：结构体值、字段访问、枚举变体、match 执行、Option / Result。</p></article>
        <article><h3>Stage 0C</h3><p>模块与 verifier：跨模块 name resolution、MIR verifier、corpus 分组和诊断 golden。</p></article>
        <article><h3>Stage 0D+</h3><p>backend smoke 和自举：WASM/WASI、LLVM native、Stage 1/2/3 self-hosting。</p></article>
      </div>
      <p class="note"><a href="/docs/project/stage-roadmap.html">查看完整阶段路线图</a></p>
    </section>
  </main>
</div>
