"""Report video extension numbers the renderer does not actually clear.

Bound to three independently-verified facts so none can be inferred from the
others: the number appears in an array literal, a loop iterates THAT array over
its full ARRAY_SIZE, and the loop body masks the indexed element off the capset.
Comments are stripped first -- a gate that reads its own annotations as evidence
has been the single most repeated defect in this lab.
"""
import re
import sys

argv = sys.argv[1:]
sep = argv.index("--")
# xargs appends the file list AFTER our separator, so numbers come first.
wanted = [int(x) for x in argv[:sep] if x.strip()]
files = argv[sep + 1:]

src = ""
for f in files:
    try:
        src += open(f, encoding="utf-8", errors="replace").read() + "\n"
    except OSError:
        pass

# Strip comments BEFORE any analysis.
src = re.sub(r"/\*.*?\*/", " ", src, flags=re.S)
src = re.sub(r"//[^\n]*", " ", src)

# The array that actually BECOMES the capset, discovered rather than assumed:
# whatever is memcpy'd into vk_extension_mask*. Without this, the mask target
# was unbound -- the check accepted any indexed `&= ~` in the loop body, so
# masking an unrelated array passed while the capset kept the video bits. That
# was the security reviewer's finding against the previous version of this
# file: the fourth mutation tested the SOURCE array and never the TARGET.
capset_arrays = set(re.findall(
    r"memcpy\s*\([^,]*\bvk_extension_mask\w*\s*,\s*(\w+)\s*,", src))
if not capset_arrays:
    print("NO-CAPSET-TARGET", file=sys.stderr)
    sys.exit(2)

cleared = set()
for arr in re.finditer(
        r"\b(\w+)\s*\[\s*\]\s*=\s*\{([^}]*)\}", src):
    name, body = arr.group(1), arr.group(2)
    nums = {int(n) for n in re.findall(r"\b(\d+)\b", body)}
    if not nums:
        continue
    # A loop over this array's FULL extent...
    for loop in re.finditer(
            r"for\s*\([^;]*;\s*\w+\s*<\s*ARRAY_SIZE\s*\(\s*%s\s*\)\s*;[^)]*\)\s*\{(.*?)\n\s*\}"
            % re.escape(name), src, re.S):
        body_txt = loop.group(1)
        # ...whose body masks a value taken from this array off the capset.
        idx = re.search(r"=\s*%s\s*\[\s*\w+\s*\]" % re.escape(name), body_txt)
        # ...into the array that reaches the capset, not merely into something.
        masks = any(
            re.search(r"\b%s\s*\[[^\]]+\]\s*&=\s*~" % re.escape(cap), body_txt)
            for cap in capset_arrays)
        # An early exit makes the ARRAY_SIZE bound a lie: the loop is written to
        # cover the array but stops short. Raised by the test reviewer as a
        # theoretical vector; closed rather than argued about, because "not
        # present today" is what was said about several defects that later were.
        early_exit = re.search(r"\b(break|continue|return|goto)\b", body_txt)
        if idx and masks and not early_exit:
            cleared |= nums

for n in sorted(set(wanted) - cleared):
    print(n)
