#!/usr/bin/env python3
from html.parser import HTMLParser
from pathlib import Path
from urllib.parse import urldefrag, urlparse
import re
import sys


ROOT = Path(__file__).resolve().parents[1]
DOCS = ROOT / "docs"
REQUIRED_METADATA = ("状态", "更新时间", "适用范围", "验收标准")
REQUIRED_DOC_NAV_TEXT = ("官网首页", "文档中心")
METADATA_PATTERN = re.compile(
    r"<p>\s*<strong>(状态|更新时间|适用范围|验收标准)：</strong>\s*(.*?)\s*</p>",
    re.DOTALL,
)
VALID_STATUSES = {"draft", "accepted", "stable", "deprecated"}


class StackParser(HTMLParser):
    VOID_TAGS = {
        "area",
        "base",
        "br",
        "col",
        "embed",
        "hr",
        "img",
        "input",
        "link",
        "meta",
        "param",
        "source",
        "track",
        "wbr",
    }

    def __init__(self, path):
        super().__init__()
        self.path = path
        self.stack = []
        self.errors = []

    def handle_starttag(self, tag, attrs):
        if tag not in self.VOID_TAGS:
            self.stack.append(tag)

    def handle_endtag(self, tag):
        if not self.stack:
            self.errors.append(f"{self.path}: unexpected </{tag}>")
            return
        if self.stack[-1] == tag:
            self.stack.pop()
            return
        self.errors.append(f"{self.path}: expected </{self.stack[-1]}> before </{tag}>")
        if tag in self.stack:
            while self.stack and self.stack[-1] != tag:
                self.stack.pop()
            if self.stack:
                self.stack.pop()


class LinkParser(HTMLParser):
    def __init__(self):
        super().__init__()
        self.links = []

    def handle_starttag(self, tag, attrs):
        attrs = dict(attrs)
        for key in ("href", "src"):
            if key in attrs:
                self.links.append(attrs[key])


def is_external(link):
    return urlparse(link).scheme in ("http", "https", "mailto")


def check_html_balance(path):
    parser = StackParser(path.relative_to(ROOT))
    parser.feed(path.read_text(encoding="utf-8"))
    if parser.stack:
        parser.errors.append(f"{path.relative_to(ROOT)}: unclosed tags: {parser.stack}")
    return parser.errors


def check_metadata(path):
    text = path.read_text(encoding="utf-8")
    errors = []
    metadata = {
        match.group(1): re.sub(r"<[^>]+>", "", match.group(2)).strip()
        for match in METADATA_PATTERN.finditer(text)
    }
    for field in REQUIRED_METADATA:
        value = metadata.get(field)
        if not value:
            errors.append(f"{path.relative_to(ROOT)}: missing metadata field: {field}")
    status = metadata.get("状态")
    if status and status not in VALID_STATUSES:
        errors.append(f"{path.relative_to(ROOT)}: invalid status: {status}")
    updated_at = metadata.get("更新时间")
    if updated_at and not re.fullmatch(r"\d{4}-\d{2}-\d{2}", updated_at):
        errors.append(f"{path.relative_to(ROOT)}: invalid 更新时间: {updated_at}")
    return errors


def check_doc_nav(path):
    text = path.read_text(encoding="utf-8")
    errors = []
    has_legacy_nav = 'class="doc-nav"' in text
    has_topbar = 'class="doc-topbar"' in text and 'class="doc-topnav"' in text
    if not has_legacy_nav and not has_topbar:
        errors.append(f"{path.relative_to(ROOT)}: missing top doc navigation")
        return errors
    for label in REQUIRED_DOC_NAV_TEXT:
        if label not in text:
            errors.append(f"{path.relative_to(ROOT)}: doc navigation missing link label: {label}")
    return errors


def check_html_links(path):
    parser = LinkParser()
    parser.feed(path.read_text(encoding="utf-8"))
    errors = []
    for link in parser.links:
        clean = urldefrag(link)[0]
        if not clean or is_external(clean):
            continue
        if clean.startswith("/assets/"):
            continue
        parsed = urlparse(clean)
        target_path = parsed.path or "."
        target = (path.parent / target_path).resolve()
        try:
            target.relative_to(ROOT)
        except ValueError:
            errors.append(f"{path.relative_to(ROOT)}: link escapes repo: {link}")
            continue
        if not target.exists():
            errors.append(f"{path.relative_to(ROOT)}: missing link target: {link}")
    return errors


def check_readme_links():
    readme = ROOT / "README.md"
    if not readme.exists():
        return []
    errors = []
    text = readme.read_text(encoding="utf-8")
    for match in re.finditer(r"\[[^\]]+\]\(([^)]+)\)", text):
        link = match.group(1)
        clean = urldefrag(link)[0]
        if not clean or is_external(clean):
            continue
        parsed = urlparse(clean)
        target_path = parsed.path or "."
        target = (readme.parent / target_path).resolve()
        if not target.exists():
            errors.append(f"README.md: missing link target: {link}")
    return errors


def main():
    errors = []
    html_files = sorted(DOCS.rglob("*.html"))
    for path in html_files:
        errors.extend(check_html_balance(path))
        errors.extend(check_metadata(path))
        errors.extend(check_doc_nav(path))
        errors.extend(check_html_links(path))
    errors.extend(check_readme_links())

    if errors:
        print("\n".join(errors), file=sys.stderr)
        return 1

    print(f"checked {len(html_files)} HTML files and README.md links: ok")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
