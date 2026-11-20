"""Check repository Markdown links and keep local demo addresses out of clickable links."""
from collections import Counter
from functools import lru_cache
from pathlib import Path
import re
import subprocess
from urllib.parse import unquote, urlsplit

ROOT = Path(__file__).resolve().parents[1]


def prose(path):
    return re.sub(r"^```[^\n]*\n.*?^```\s*$", "", path.read_text(), flags=re.M | re.S)


@lru_cache(maxsize=None)
def anchors(path):
    names = set()
    seen = Counter()
    for heading in re.findall(r"^#{1,6}\s+(.+)$", prose(path), flags=re.M):
        slug = re.sub(r"[^\w -]", "", heading.lower()).strip().replace(" ", "-")
        names.add(f"{slug}-{seen[slug]}" if seen[slug] else slug)
        seen[slug] += 1
    return names


def main():
    files = subprocess.check_output(["git", "ls-files", "-co", "--exclude-standard", "-z", "--", "*.md"], cwd=ROOT)
    errors = []
    for name in files.decode().strip("\0").split("\0"):
        path = ROOT / name
        if not path.is_file():
            continue
        for target in re.findall(r"\]\(([^)\s]+)\)", prose(path)):
            url = urlsplit(target)
            if url.hostname in {"localhost", "127.0.0.1", "::1"}:
                errors.append(f"{name}: use a local run instruction for {target}")
            if url.scheme or url.netloc:
                continue
            dest = (path.parent / unquote(url.path)).resolve() if url.path else path.resolve()
            if not dest.exists() or (url.fragment and dest.suffix == ".md" and unquote(url.fragment) not in anchors(dest)):
                errors.append(f"{name}: missing {target}")
    if errors:
        raise SystemExit("\n".join(errors))
    print("PASS documentation links and local demo addresses")


if __name__ == "__main__":
    main()
