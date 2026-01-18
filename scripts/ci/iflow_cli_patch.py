import re
from pathlib import Path

path = Path('/usr/lib/node_modules/@iflow-ai/iflow-cli/bundle/iflow.js')
if not path.exists():
    raise SystemExit(f"iflow bundle not found at {path}")

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
