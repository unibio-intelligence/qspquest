"""Check canonical links, generated MCP copies and optional rendered site links."""
import argparse
import json
from pathlib import Path
from urllib.parse import urlsplit, unquote

from doc_links import LINK, local_target, prose, check_html
from generate_mcp_resources import check as check_resources


def check(root, site=None):
    root = root.resolve()
    copies = check_resources(root)
    index = json.loads((root / 'bindings/mcp/assets/documentation.json').read_text())
    resources = {entry['uri']: entry for entry in index['resources']}
    resource_links = 0
    for entry in resources.values():
        if entry['mime_type'] != 'text/markdown':
            continue
        path = root / 'bindings/mcp/assets' / entry['asset']
        for link in LINK.finditer(prose(path.read_text())):
            url = link.group(1)
            parsed = urlsplit(url)
            if parsed.scheme == 'qspquest':
                base = url.split('#', 1)[0]
                target = resources.get(base)
                if target is None or parsed.fragment and unquote(parsed.fragment) not in target['anchors']:
                    raise ValueError('Embedded resource link has no delivered target/heading: ' + url)
                resource_links += 1
            elif not parsed.scheme and not parsed.netloc:
                raise ValueError('Embedded resource contains a checkout-relative link: ' + url)
    errors, local_links, documents = [], 0, 0
    for path in sorted(root.rglob('*.md')):
        if any(part in ('target', '.git', '_site', '__pycache__') for part in path.relative_to(root).parts):
            continue
        documents += 1
        for link in LINK.finditer(prose(path.read_text())):
            try:
                target = local_target(root, path, link.group(1))
                if target is not None:
                    local_links += 1
            except ValueError as error:
                errors.append(str(error))
    if errors:
        raise ValueError('\n'.join(errors))
    return {'status': 'matched', 'generated_files': copies, 'markdown_documents': documents,
            'local_links': local_links, 'embedded_resource_links': resource_links,
            'rendered_html_links': check_html(site.resolve()) if site else 'not_checked'}


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--root', type=Path, default=Path(__file__).resolve().parents[1])
    parser.add_argument('--site', type=Path)
    args = parser.parse_args()
    try:
        print(json.dumps(check(args.root, args.site), indent=2))
    except (ValueError, OSError) as error:
        parser.exit(1, str(error) + '\n')
