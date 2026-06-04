const vscode = require("vscode");

const TOPICS = [
  ["module", "Declare the current source module.", "module demo;"],
  ["import", "Import another module path. Use `as` to create a local alias for module-qualified calls.", "import demo.math as math;"],
  ["as", "Assign a local alias to an imported module path.", "import demo.math as math;"],
  ["fn", "Declare a function.", "fn main(name: String) -> Int { return 0; }"],
  ["let", "Declare a local binding with an optional type annotation. Use let mut for reassignment.", "let mut answer: Int = 40;"],
  ["mut", "Mark a local binding as mutable so later assignment is allowed.", "let mut answer: Int = 40;\nanswer = answer + 2;"],
  ["if", "Branch on a Bool condition. Comparisons and boolean logic return Bool.", "if ready && !done { return 42; } else { return 0; }"],
  ["while", "Loop while a Bool condition is true. Comparisons and boolean logic are valid conditions.", "while count < 3 && ready { count = count + 1; }"],
  ["break", "Exit the nearest enclosing while loop.", "while true { break; }"],
  ["continue", "Skip the rest of the current while loop iteration.", "while count < 3 { count = count + 1; continue; }"],
  ["match", "Match a value against simple patterns.", "match value { 0 -> { return 0; }, _ -> { return value; }, }"],
  ["struct", "Declare a record type.", "struct User { name: String, age: Int, }"],
  ["enum", "Declare a tagged set of variants.", "enum ResultTag { Ok, Err, }"],
  ["Int", "Integer scalar type currently supported by the Stage 0 checker.", "let value: Int = 1 + 2;"],
  ["String", "String scalar type currently supported by the Stage 0 checker.", "let name: String = \"zeta\";"],
  ["Bool", "Boolean scalar type used by if and while conditions.", "let ready: Bool = 1 + 1 == 2 && !false;"],
  ["IntArray", "Homogeneous Int array. Use [..] literals, integer indexing, and .len.", "let values: IntArray = [2, 4, 6];"],
  ["StringArray", "Homogeneous String array. Use [..] literals, integer indexing, and .len.", "let names: StringArray = [\"Ada\", \"Zeta\"];"],
  ["BoolArray", "Homogeneous Bool array. Use [..] literals, integer indexing, and .len.", "let flags: BoolArray = [true, false];"]
];

function activate(context) {
  context.subscriptions.push(
    vscode.languages.registerCompletionItemProvider("zeta", {
      provideCompletionItems() {
        return TOPICS.map(([name, summary, example]) => {
          const item = new vscode.CompletionItem(name, completionKind(name));
          item.detail = "Zeta";
          item.documentation = new vscode.MarkdownString(`${summary}\n\n\`\`\`zeta\n${example}\n\`\`\``);
          return item;
        });
      }
    })
  );

  context.subscriptions.push(
    vscode.languages.registerHoverProvider("zeta", {
      provideHover(document, position) {
        const range = document.getWordRangeAtPosition(position);
        if (!range) {
          return undefined;
        }
        const word = document.getText(range);
        const topic = TOPICS.find(([name]) => name === word);
        if (!topic) {
          return undefined;
        }
        const [, summary, example] = topic;
        return new vscode.Hover(new vscode.MarkdownString(`${summary}\n\n\`\`\`zeta\n${example}\n\`\`\``), range);
      }
    })
  );
}

function completionKind(name) {
  if (["Int", "String", "Bool", "IntArray", "StringArray", "BoolArray"].includes(name)) {
    return vscode.CompletionItemKind.TypeParameter;
  }
  if (name === "fn") {
    return vscode.CompletionItemKind.Function;
  }
  return vscode.CompletionItemKind.Keyword;
}

function deactivate() {}

module.exports = {
  activate,
  deactivate
};
