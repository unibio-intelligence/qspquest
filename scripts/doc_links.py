"""Local Markdown and generated HTML links used by documentation checks."""
from html import unescape
from html.parser import HTMLParser
from pathlib import Path
import re
from urllib.parse import unquote, urlsplit

LINK = re.compile(r'!?\[[^\]\n]+\]\(([^\s()]+)(?:\s+"[^"\n]*")?\)')


def slug(text):
    text = unescape(re.sub(r'<[^>]+>', '', text))
    return re.sub(r'[^\w\s-]', '', text).lower().replace(' ', '-')


def prose(text):
    """Preserve offsets while excluding fenced code from link/heading scans."""
    lines = []
    fence = None
    for line in text.splitlines(keepends=True):
        marker = re.match(r'^\s*(`{3,}|~{3,})', line)
        if marker:
            if fence is None:
                fence = marker.group(1)[0]
            elif marker.group(1)[0] == fence:
                fence = None
            lines.append(' ' * len(line.rstrip('\r\n')) + ('\n' if line.endswith('\n') else ''))
        elif fence:
            lines.append(' ' * len(line.rstrip('\r\n')) + ('\n' if line.endswith('\n') else ''))
        else:
            lines.append(line)
    return ''.join(lines)


def anchors(text):
    counts, result = {}, set()
    for line in prose(text).splitlines():
        heading = re.match(r'^#{1,6}\s+(.+?)(?:\s+#+)?$', line)
        if heading:
            key = slug(heading.group(1))
            count = counts.get(key, 0)
            counts[key] = count + 1
            result.add(key + ('-' + str(count) if count else ''))
    result.update(re.findall(r'<(?:a|h[1-6])\b[^>]*\bid=["\']([^"\']+)', text))
    return result


def local_target(root, source, url):
    parsed = urlsplit(url)
    if parsed.scheme or parsed.netloc:
        return None
    target = (source.parent / unquote(parsed.path)).resolve() if parsed.path else source.resolve()
    if not target.is_relative_to(root.resolve()) or not target.exists():
        raise ValueError('Unresolved or escaping link: ' + str(source.relative_to(root)) + ' -> ' + url)
    fragment = unquote(parsed.fragment)
    if fragment and target.suffix == '.md' and fragment not in anchors(target.read_text()):
        raise ValueError('Unresolved heading: ' + str(source.relative_to(root)) + ' -> ' + url)
    return target, fragment


class HtmlLinks(HTMLParser):
    def __init__(self):
        super().__init__()
        self.links, self.ids = [], set()

    def handle_starttag(self, tag, attrs):
        attrs = dict(attrs)
        if 'id' in attrs:
            self.ids.add(attrs['id'])
        for name in ('href', 'src'):
            if attrs.get(name):
                self.links.append(attrs[name])


def check_html(root):
    pages = {}
    for path in root.rglob('*.html'):
        page = HtmlLinks()
        page.feed(path.read_text())
        pages[path.resolve()] = page
    count = 0
    for path, page in pages.items():
        for link in page.links:
            target = local_target(root, path, link)
            if target is None:
                continue
            file, fragment = target
            if fragment and file in pages and fragment not in pages[file].ids:
                raise ValueError('Unresolved generated HTML heading: ' + str(path) + ' -> ' + link)
            count += 1
    return count
