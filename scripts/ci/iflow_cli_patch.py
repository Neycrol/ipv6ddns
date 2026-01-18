import os
import re
import shutil
import subprocess
from pathlib import Path

def run(cmd):
    return subprocess.check_output(cmd, text=True).strip()


def candidate_paths():
    candidates = []
    env_bundle = os.environ.get("IFLOW_BUNDLE_PATH", "").strip()
    if env_bundle:
        candidates.append(Path(env_bundle))

    which_iflow = shutil.which("iflow")
    if which_iflow:
        bin_path = Path(which_iflow).resolve()
        prefix = bin_path.parent.parent
        candidates.append(prefix / "lib/node_modules/@iflow-ai/iflow-cli/bundle/iflow.js")

    try:
        root = run(["npm", "root", "-g"])
        if root:
            candidates.append(Path(root) / "@iflow-ai/iflow-cli/bundle/iflow.js")
    except Exception:
        pass

    try:
        prefix = run(["npm", "config", "get", "prefix"])
        if prefix:
            candidates.append(Path(prefix) / "lib/node_modules/@iflow-ai/iflow-cli/bundle/iflow.js")
    except Exception:
        pass

    # Common fallbacks
    candidates.extend(
        [
            Path("/usr/local/lib/node_modules/@iflow-ai/iflow-cli/bundle/iflow.js"),
            Path("/usr/lib/node_modules/@iflow-ai/iflow-cli/bundle/iflow.js"),
            Path("/opt/hostedtoolcache/node/current/x64/lib/node_modules/@iflow-ai/iflow-cli/bundle/iflow.js"),
        ]
    )

    # Deduplicate while preserving order
    seen = set()
    unique = []
    for c in candidates:
        if c not in seen:
            unique.append(c)
            seen.add(c)
    return unique

path = None
attempted = []
for candidate in candidate_paths():
    attempted.append(str(candidate))
    if candidate.exists():
        path = candidate
        break

if not path:
    raise SystemExit("iflow bundle not found. Tried:\n- " + "\n- ".join(attempted))

s = path.read_text(encoding='utf-8')

# Set SubAgent retry parameters and fixed delay.
header_pat = r"Wst=class\{maxRetries=\d+;backoffMultiplier=\d+;baseDelayMs=\d+e3;"
new_header = "Wst=class{maxRetries=100;backoffMultiplier=1;baseDelayMs=1e3;"

s2, n1 = re.subn(header_pat, new_header, s, count=1)
if n1 == 0:
    # Already patched header or unexpected layout
    if new_header not in s:
        raise SystemExit("Wst header not found or replaced 0 times")

calc_pat = r"calculateDelay\(\w\)\{return this\.baseDelayMs\*Math\.pow\(this\.backoffMultiplier,\w-1\)\}"
# Force fixed 1000ms delay regardless of retry count
s3, n2 = re.subn(calc_pat, "calculateDelay(e){return 1e3}", s2, count=1)
if n2 == 0:
    # Already patched delay? Replace any simple return variant.
    s3, n2 = re.subn(r"calculateDelay\(\w\)\{return \d+e3\}", "calculateDelay(e){return 1e3}", s2, count=1)
if n2 != 1:
    raise SystemExit(f"calculateDelay not found or replaced {n2} times")

path.write_text(s3, encoding='utf-8')
print("patched iflow CLI retry settings")
