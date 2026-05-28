const vscode = require("vscode");

const TOPICS = [
  ["module", "Declare the current source module.", "module demo;"],
  ["import", "Import another module path.", "import std.io;"],
  ["fn", "Declare a function.", "fn main(name: String) -> Int { return 0; }"],
  ["let", "Declare a local binding with an optional type annotation.", "let answer: Int = 40 + 2;"],
  ["if", "Branch on a Bool condition.", "if true { return 1; } else { return 0; }"],
  ["while", "Loop while a Bool condition is true.", "while false { let next: Int = 1; }"],
  ["match", "Match a value against simple patterns.", "match value { 0 -> { return 0; }, _ -> { return value; }, }"],
  ["struct", "Declare a record type.", "struct User { name: String, age: Int, }"],
  ["enum", "Declare a tagged set of variants.", "enum ResultTag { Ok, Err, }"],
  ["Int", "Integer scalar type currently supported by the Stage 0 checker.", "let value: Int = 1 + 2;"],
  ["String", "String scalar type currently supported by the Stage 0 checker.", "let name: String = \"zeta\";"],
  ["Bool", "Boolean scalar type used by if and while conditions.", "let ready: Bool = true;"]
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
  if (["Int", "String", "Bool"].includes(name)) {
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
