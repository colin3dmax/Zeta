#!/usr/bin/env python3
import json
from pathlib import Path
import sys


ROOT = Path(__file__).resolve().parents[1]
EXT = ROOT / "editors" / "vscode-zeta"


def load_json(path):
    try:
        return json.loads(path.read_text(encoding="utf-8"))
    except json.JSONDecodeError as exc:
        raise ValueError(f"{path.relative_to(ROOT)}: invalid JSON: {exc}") from exc


def main():
    errors = []
    package_path = EXT / "package.json"
    try:
        package = load_json(package_path)
    except ValueError as exc:
        print(exc, file=sys.stderr)
        return 1

    contributes = package.get("contributes", {})
    main = package.get("main")
    languages = contributes.get("languages", [])
    grammars = contributes.get("grammars", [])
    snippets = contributes.get("snippets", [])

    if not any(language.get("id") == "zeta" for language in languages):
        errors.append("editors/vscode-zeta/package.json: missing zeta language contribution")

    if not main or not (EXT / main).exists():
        errors.append(f"missing extension main: {main}")

    for language in languages:
        config = language.get("configuration")
        if config and not (EXT / config).exists():
            errors.append(f"missing language configuration: {config}")

    for grammar in grammars:
        if grammar.get("language") != "zeta":
            errors.append("grammar contribution must target language `zeta`")
        path = grammar.get("path")
        if not path or not (EXT / path).exists():
            errors.append(f"missing grammar path: {path}")
        elif load_json(EXT / path).get("scopeName") != "source.zeta":
            errors.append(f"{path}: expected scopeName source.zeta")

    for snippet in snippets:
        path = snippet.get("path")
        if not path or not (EXT / path).exists():
            errors.append(f"missing snippet path: {path}")
        else:
            load_json(EXT / path)

    install_script = EXT / "scripts" / "install-local.sh"
    if not install_script.exists():
        errors.append("missing local install script: scripts/install-local.sh")

    if errors:
        print("\n".join(errors), file=sys.stderr)
        return 1

    print("checked VS Code Zeta extension: ok")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
