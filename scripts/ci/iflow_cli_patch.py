import os
import re
import shutil
import subprocess
import glob
from pathlib import Path

def run(cmd):
    return subprocess.check_output(cmd, text=True).strip()


def add_bin_candidate(candidates, bin_path):
    try:
        resolved = Path(bin_path).resolve()
    except Exception:
        resolved = Path(bin_path)

    # If the resolved path is inside the package, use it directly.
    for parent in resolved.parents:
        if parent.name == "iflow-cli":
            candidates.append(parent / "bundle/iflow.js")
            break

    # Fallback: derive from prefix (../..)
    prefix = resolved.parent.parent
    candidates.append(prefix / "lib/node_modules/@iflow-ai/iflow-cli/bundle/iflow.js")


def candidate_paths():
    candidates = []
    env_bundle = os.environ.get("IFLOW_BUNDLE_PATH", "").strip()
    if env_bundle:
        candidates.append(Path(env_bundle))

    which_iflow = shutil.which("iflow")
    if which_iflow:
        add_bin_candidate(candidates, which_iflow)

    for bin_path in [
        "/usr/local/bin/iflow",
        "/usr/bin/iflow",
        "/opt/hostedtoolcache/node/current/x64/bin/iflow",
    ]:
        if Path(bin_path).exists():
            add_bin_candidate(candidates, bin_path)

    npm_bins = [shutil.which("npm"), "/usr/local/bin/npm", "/usr/bin/npm", "/opt/hostedtoolcache/node/current/x64/bin/npm"]
    for npm_bin in [p for p in npm_bins if p]:
        try:
            root = run([npm_bin, "root", "-g"])
            if root:
                candidates.append(Path(root) / "@iflow-ai/iflow-cli/bundle/iflow.js")
        except Exception:
            pass

    for npm_bin in [p for p in npm_bins if p]:
        try:
            prefix = run([npm_bin, "config", "get", "prefix"])
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

    # Targeted glob for GitHub Actions toolcache versions.
    for match in glob.glob("/opt/hostedtoolcache/node/*/x64/lib/node_modules/@iflow-ai/iflow-cli/bundle/iflow.js"):
        candidates.append(Path(match))

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
text = s3

# Bump sub-agent timeout from 300000ms to 900000ms (15 minutes).
needle_variants = [
    "constructor(e,r=3e5,n,o){this.agentId=e,this.timeoutMs=r",
    "constructor(e,r=300000,n,o){this.agentId=e,this.timeoutMs=r",
]
replacement = "constructor(e,r=9e5,n,o){this.agentId=e,this.timeoutMs=r"
n3 = 0
text2 = text
for needle in needle_variants:
    if needle in text2:
        text2 = text2.replace(needle, replacement, 1)
        n3 = 1
        break
if n3 == 0:
    # Try regex fallback for minor minifier variations.
    timeout_pat = r"constructor\\(e,r=3e5,n,o\\)\\{this\\.agentId=e,this\\.timeoutMs=r"
    text2, n3 = re.subn(timeout_pat, replacement, text2, count=1)
if n3 == 0:
    timeout_pat2 = r"constructor\\(e,r=300000,n,o\\)\\{this\\.agentId=e,this\\.timeoutMs=r"
    text2, n3 = re.subn(timeout_pat2, replacement, text2, count=1)
if n3 == 0:
    timeout_pat3 = r"constructor\\(e,r=\\d+(?:e\\d+)?,n,o\\)\\{this\\.agentId=e,this\\.timeoutMs=r"
    text2, n3 = re.subn(timeout_pat3, replacement, text2, count=1)
if n3 == 0:
    raise SystemExit("Agent timeout constructor not found; aborting to avoid partial patch")

path.write_text(text2, encoding="utf-8")
if "constructor(e,r=9e5,n,o){this.agentId=e,this.timeoutMs=r" not in text2:
    raise SystemExit("Agent timeout patch did not persist; aborting.")
print(f"patched iflow CLI retry settings + agent timeout at {path}")
