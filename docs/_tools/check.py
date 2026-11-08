"""Check the docs: every source file has a page, every function a heading, every test a row,
and every link (file, #L line, #heading anchor) resolves.

  python docs/_tools/check.py

Exits non-zero when anything is wrong.
"""
import os
import subprocess
import sys

TOOLS = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.dirname(os.path.dirname(TOOLS))

ok = True
for script in ("coverage.py", "linkcheck.py"):
    print(f"== {script}")
    r = subprocess.run([sys.executable, os.path.join(TOOLS, script)], cwd=ROOT)
    ok = ok and r.returncode == 0
sys.exit(0 if ok else 1)
