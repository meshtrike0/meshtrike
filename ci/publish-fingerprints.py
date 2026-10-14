#!/usr/bin/env python3
"""Copy the public release fingerprints everywhere users compare them.

Reads PUBLIC-FINGERPRINTS.txt (written by ci/make-release-keys.sh; public parts
only) and fills in: verify.sh, website/js/verify.js, every website page with
fingerprint placeholders (data-fp) and README.md. Run it once after creating the keys,
then regenerate website/meshtrike-whitepaper.pdf and commit.

    python3 ci/publish-fingerprints.py [path/to/PUBLIC-FINGERPRINTS.txt]
"""
import os
import pathlib
import re
import sys

ROOT = pathlib.Path(os.environ.get("MESHTRIKE_ROOT", pathlib.Path(__file__).resolve().parents[1]))
src = pathlib.Path(sys.argv[1] if len(sys.argv) > 1 else pathlib.Path.home() / "MeshtrikeKeys/PUBLIC-FINGERPRINTS.txt")


def value_after(label, text):
    m = re.search(re.escape(label) + r"[^\n]*\n\s*(\S+)", text)
    if not m:
        sys.exit(f"'{label}' not found in {src}")
    return m.group(1)


text = src.read_text()
cert = value_after("Android APK signing certificate", text).upper()
pgp = value_after("OpenPGP key fingerprint", text).upper()
mini = value_after("minisign public key", text)

if not re.fullmatch(r"([0-9A-F]{2}:){31}[0-9A-F]{2}", cert):
    sys.exit(f"Unexpected certificate fingerprint format: {cert}")
if not re.fullmatch(r"[0-9A-F]{40}", pgp):
    sys.exit(f"Unexpected OpenPGP fingerprint format: {pgp}")
if not re.fullmatch(r"RW[A-Za-z0-9+/=]{54}", mini):
    sys.exit(f"Unexpected minisign public key format: {mini}")
pgp_spaced = " ".join(pgp[i:i + 4] for i in range(0, 40, 4))
values = {"android": cert, "openpgp": pgp_spaced, "minisign": mini}


def edit(rel, fn):
    p = ROOT / rel
    before = p.read_text(encoding="utf-8")
    after = fn(before)
    if after == before:
        if cert in before or cert.replace(":", "").lower() in before.lower():
            print(f"already published in {rel}")
            return
        sys.exit(f"{rel}: no placeholder found and no fingerprint present; check it by hand")
    p.write_text(after, encoding="utf-8")
    print(f"updated {rel}")


edit("verify.sh", lambda s: s.replace('APK_CERT_SHA256="NOT_YET_PUBLISHED"', f'APK_CERT_SHA256="{cert}"')
     .replace('MINISIGN_PUBKEY="NOT_YET_PUBLISHED"', f'MINISIGN_PUBKEY="{mini}"'))
edit("website/js/verify.js", lambda s: s.replace("var OFFICIAL_CERT_SHA256 = 'NOT_YET_PUBLISHED';",
                                                 f"var OFFICIAL_CERT_SHA256 = '{cert}';"))


def fill_fp(s):
    return re.sub(r'(data-fp="(android|openpgp|minisign)">)[^<]*(<)',
                  lambda m: m.group(1) + values[m.group(2)] + m.group(3), s)


for page in sorted((ROOT / "website").glob("*.html")):
    if 'data-fp="' in page.read_text(encoding="utf-8"):
        edit(page.relative_to(ROOT), fill_fp)
edit("README.md", lambda s: s
     .replace("| APK signing certificate (SHA-256) | _published with the first signed release_ |", f"| APK signing certificate (SHA-256) | `{cert}` |")
     .replace("| OpenPGP release key | _published with the first signed release_ |", f"| OpenPGP release key | `{pgp_spaced}` |")
     .replace("| minisign public key | _published with the first signed release_ |", f"| minisign public key | `{mini}` |"))
print("Now regenerate website/meshtrike-whitepaper.pdf, review the diff, and commit.")
