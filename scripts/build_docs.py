"""Build an offline searchable documentation site and AST-generated Python API."""

import argparse, ast, json, re, shutil, zipfile
from pathlib import Path
from html import escape
from urllib.parse import urlsplit, unquote
import mistune
from doc_links import check_html, slug

ROOT = Path(__file__).resolve().parents[1]


def api_text():
    lines = [
        "# Python API reference",
        "",
        "Generated from Python source signatures and docstrings. Units and the scientific contract are checked by the native binder and analysis validators. Optional numerical dependencies are loaded on scientific use.",
        "",
    ]
    folder = ROOT / "bindings/python/python/qspquest"
    for name in [
        "__init__",
        "friendly",
        "control",
        "doctor",
        "dataset",
        "estimation",
        "population",
        "diagnostics",
        "uncertainty",
        "exposure",
        "derivatives",
        "report",
        "interchange",
        "individual",
    ]:
        tree = ast.parse((folder / (name + ".py")).read_text())
        lines += [f"## {name}", "", ast.get_docstring(tree) or "", ""]
        for node in tree.body:
            if not isinstance(
                node, (ast.FunctionDef, ast.ClassDef)
            ) or node.name.startswith("_"):
                continue
            if isinstance(node, ast.ClassDef):
                lines += [f"### {node.name}", "", ast.get_docstring(node) or "", ""]
                children = [
                    n
                    for n in node.body
                    if isinstance(n, ast.FunctionDef)
                    and (not n.name.startswith("_") or n.name == "__init__")
                ]
            else:
                children = [node]
            for f in children:
                label = (
                    node.name + "." if isinstance(node, ast.ClassDef) else ""
                ) + f.name
                lines += [
                    f"### {label}",
                    "",
                    "```python",
                    f"{label}({ast.unparse(f.args)})",
                    "```",
                    "",
                    ast.get_docstring(f)
                    or "See the versioned native request contract.",
                    "",
                ]
    return "\n".join(lines)


def build(out):
    out.mkdir(parents=True, exist_ok=True)
    generated = api_text()
    api = ROOT / "docs/api/python.md"
    api.parent.mkdir(parents=True, exist_ok=True)
    api.write_text(generated)
    sources = {p.stem: p for p in (ROOT / "docs/site").glob("*.md")}
    sources["installation"] = ROOT / "docs/installation.md"
    sources.update(
        {
            "api": api,
            "analysis": ROOT / "docs/ANALYSIS.md",
            "python-guide": ROOT / "docs/PYTHON-USER-GUIDE.md",
            "r-guide": ROOT / "docs/R-USER-GUIDE.md",
            "native-saem": ROOT / "docs/SAEM.md",
            "synthetic-model": ROOT / "examples/synthetic/README.md",
        }
    )
    pages = {name: path.read_text() for name, path in sources.items()}
    route = {path.resolve(): name + ".html" for name, path in sources.items()}
    sidebar = dict(pages)

    class Renderer(mistune.HTMLRenderer):
        def heading(self, text, level, **attrs):
            key = slug(text)
            count = self.headings.get(key, 0)
            self.headings[key] = count + 1
            anchor = key + (f"-{count}" if count else "")
            return f'<h{level} id="{escape(anchor)}">{text}</h{level}>\n'

        def local_url(self, url):
            parsed = urlsplit(url)
            if parsed.scheme or parsed.netloc or not parsed.path:
                return url
            source = (self.source.parent / unquote(parsed.path)).resolve()
            if not source.is_relative_to(ROOT) or not source.exists():
                # Site task pages deliberately link to generated routes.
                if self.source.parent == ROOT / "docs/site" and (
                    parsed.path.endswith(".html")
                    or parsed.path in ("synthetic.ipynb", "synthetic-tutorial.zip")
                ):
                    return url
                raise ValueError(
                    f"Unresolved documentation link in {self.source}: {url}"
                )
            relative = source.relative_to(ROOT)
            if (
                str(relative).startswith("conformance/evidence/")
                or (
                    not relative.parts
                    or relative.parts[0]
                    not in (
                        "docs",
                        "examples",
                        "schemas",
                        "spec",
                        "bindings",
                        "conformance",
                        "crates",
                        "vendor",
                        "scripts",
                    )
                )
                and str(relative)
                not in (
                    "README.md",
                    "SECURITY.md",
                    "LICENSE",
                    "LICENSE-APACHE",
                    "NOTICE",
                    "CITATION.cff",
                    "CONTRIBUTING.md",
                    "CHANGELOG.md",
                    "Cargo.toml",
                    "pyproject.toml",
                )
            ):
                raise ValueError(
                    f"Path is outside the public documentation roots: {relative}"
                )
            if source not in route:
                name = "reference-" + re.sub(
                    r"[^a-zA-Z0-9_-]", "-", str(relative.with_suffix(""))
                )
                if source.suffix == ".md":
                    route[source] = name + ".html"
                    sources[name] = source
                    pages[name] = source.read_text()
                elif source.is_file():
                    route[source] = name + source.suffix
                    shutil.copyfile(source, out / route[source])
                else:
                    raise ValueError(f"Documentation link is not a file: {source}")
            return (
                route[source]
                + ("?" + parsed.query if parsed.query else "")
                + ("#" + parsed.fragment if parsed.fragment else "")
            )

        def link(self, text, url, title=None):
            return super().link(text, self.local_url(url), title)

        def image(self, text, url, title=None):
            return super().image(text, self.local_url(url), title)

    renderer = Renderer(escape=False)
    markdown = mistune.create_markdown(renderer=renderer, plugins=["table"])
    bodies = {}
    while len(bodies) < len(pages):
        name = next(n for n in pages if n not in bodies)
        renderer.source = sources[name]
        renderer.headings = {}
        bodies[name] = markdown(pages[name])
    nav = "".join(
        f'<a href="{name}.html">{escape(text.splitlines()[0].lstrip("# "))}</a>'
        for name, text in sidebar.items()
    )
    index = [
        {
            "title": t.splitlines()[0].lstrip("# "),
            "url": n + ".html",
            "text": re.sub(r"[#*`|]", "", t),
        }
        for n, t in pages.items()
    ]
    data = (
        json.dumps(index, ensure_ascii=True)
        .replace("<", "\\u003c")
        .replace(">", "\\u003e")
        .replace("&", "\\u0026")
    )
    search = """const input=document.querySelector('#search'), results=document.querySelector('#results');
input.addEventListener('input',()=>{results.replaceChildren();const terms=input.value.toLowerCase().trim().split(/\\s+/).filter(Boolean);if(!terms.length)return;
for(const p of window.searchIndex){const s=p.text.toLowerCase();if(!terms.every(t=>s.includes(t)))continue;
const item=document.createElement('li'),a=document.createElement('a'),summary=document.createElement('p');a.href=p.url;a.textContent=p.title;
const at=Math.max(0,s.indexOf(terms[0])-50);summary.textContent=p.text.slice(at,at+230);item.append(a,summary);results.append(item);}});"""
    (out / "search.js").write_text("window.searchIndex=" + data + ";\n" + search)
    css = """
:root{--primary:#0f766e;--primary-soft:#6dd3ce;--accent:#f7a278;--link-hover:#a13d63;--ink:#111827;--heading:#351e29;--canvas:#f7faf9;--surface:#eef4f2;--border:#d8e2df}
body{font:16px system-ui;line-height:1.65;color:var(--ink);margin:0;background:var(--canvas)}
header{background:var(--primary);color:#fff;padding:18px 24px;border-bottom:4px solid var(--accent)}
header a{color:#fff}header a:hover{color:#fff;text-decoration-color:var(--accent)}
a{color:var(--primary)}a:hover{color:var(--link-hover)}
a:focus-visible,input:focus-visible{outline:2px solid var(--primary);outline-offset:3px}
aside{width:235px;position:fixed;top:92px;bottom:0;overflow:auto;padding:20px}
aside a{display:block;margin:8px 0}aside a:hover{background:var(--surface)}
main{max-width:940px;margin:30px 30px 80px 300px}
input{padding:10px;color:var(--ink);background:#fff;border:1px solid var(--border);border-radius:5px;width:min(80%,600px)}
pre{overflow:auto;background:var(--surface);padding:16px;border-radius:6px;border-left:3px solid var(--primary-soft)}
code{font-size:14px}table{border-collapse:collapse;display:block;overflow:auto}
td,th{padding:8px 12px;border:1px solid var(--border);text-align:left}th{background:var(--surface)}
h1,h2,h3{line-height:1.3}h1,h2{color:var(--heading)}h3{color:var(--primary)}
#results:empty{display:none}
@media(max-width:800px){aside{position:static;width:auto;display:flex;flex-wrap:wrap;gap:10px}main{margin:20px}header{position:static}}
"""
    (out / "style.css").write_text(css)
    for name, text in pages.items():
        body = bodies[name]
        title = escape(text.splitlines()[0].lstrip("# "))
        document = f'<!doctype html><html lang="en"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1"><title>{title} — QSPQuest</title><link rel="stylesheet" href="style.css"><script src="search.js" defer></script></head><body><header><a href="index.html">QSPQuest documentation</a></header><aside aria-label="Documentation navigation">{nav}</aside><main><label for="search">Search documentation</label><br><input id="search" type="search" placeholder="Search tasks, API names or scientific limits"><ul id="results" aria-live="polite"></ul>{body}</main></body></html>'
        (out / (name + ".html")).write_text(document)
    shutil.copyfile(
        ROOT / "docs/examples/python_quickstart.py", out / "python_quickstart.py"
    )
    with zipfile.ZipFile(
        out / "synthetic-tutorial.zip", "w", zipfile.ZIP_DEFLATED
    ) as bundle:
        for rel in [
            "LICENSE",
            "docs/examples/interchange_roundtrip.py",
            "examples/synthetic/README.md",
            "examples/synthetic/absorption.qsp",
            "examples/synthetic/absorption.model.json",
        ]:
            bundle.write(ROOT / rel, "qspquest-synthetic/" + rel)
    check_html(out.resolve())
    return out


if __name__ == "__main__":
    parser = argparse.ArgumentParser()
    parser.add_argument("--out", type=Path, default=ROOT / "docs/_site")
    args = parser.parse_args()
    print(build(args.out))
