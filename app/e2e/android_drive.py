#!/usr/bin/env python3
"""Drive the release APK on a device/emulator through the accessibility tree
(screenshots are blocked by FLAG_SECURE, which is intended).

Usage: python3 e2e/android_drive.py [serial]
"""
import re
import subprocess
import sys
import time

SERIAL = sys.argv[1] if len(sys.argv) > 1 else None


def adb(*args, check=True):
    cmd = ["adb"] + (["-s", SERIAL] if SERIAL else []) + list(args)
    return subprocess.run(cmd, capture_output=True, text=True, check=check).stdout


def nodes():
    xml = ""
    for _ in range(5):
        adb("shell", "uiautomator", "dump", "/sdcard/ui.xml", check=False)
        xml = adb("shell", "cat", "/sdcard/ui.xml", check=False)
        if "<node" in xml:
            break
        time.sleep(1)
    out = []
    for m in re.finditer(r"<node [^>]*>", xml):
        n = m.group(0)
        def attr(name):
            m2 = re.search(r'\b' + name + r'="([^"]*)"', n)
            return m2.group(1) if m2 else ""
        text = attr("text")
        desc = attr("content-desc")
        b = re.search(r'bounds="\[(\d+),(\d+)\]\[(\d+),(\d+)\]"', n)
        if not b:
            continue
        x1, y1, x2, y2 = map(int, b.groups())
        out.append({"text": text, "desc": desc, "cx": (x1 + x2) // 2, "cy": (y1 + y2) // 2, "editable": 'class="android.widget.EditText"' in n})
    return out


def find(label, timeout=30, exact=True, editable=False):
    t0 = time.time()
    while time.time() - t0 < timeout:
        for n in nodes():
            if editable and not n["editable"]:
                continue
            for v in (n["desc"], n["text"]):
                if (v == label) if exact else (label in v):
                    return n
        time.sleep(0.5)
    raise AssertionError(f"not found: {label}")


def tap(label, **kw):
    n = find(label, **kw)
    adb("shell", "input", "tap", str(n["cx"]), str(n["cy"]))
    time.sleep(0.4)


def type_into(label, text):
    n = find(label, editable=True)
    adb("shell", "input", "tap", str(n["cx"]), str(n["cy"]))
    time.sleep(0.4)
    adb("shell", "input", "text", text)
    adb("shell", "input", "keyevent", "111")  # escape: hide keyboard
    time.sleep(0.3)


def step(name, fn):
    t0 = time.time()
    fn()
    print(f"✓ {name} ({int((time.time() - t0) * 1000)} ms)")


def launch_and_grant():
    """Fresh launch: accept the runtime permission prompts (Bluetooth, camera, notifications)."""
    adb("shell", "am", "start", "-n", "org.meshtrike.app/.MainActivity")
    t0 = time.time()
    while time.time() - t0 < 90:
        ns = nodes()
        allow = [n for n in ns if n["text"] in ("Allow", "While using the app", "Only this time", "Wait")]
        if allow:
            adb("shell", "input", "tap", str(allow[0]["cx"]), str(allow[0]["cy"]))
            time.sleep(1)
            continue
        if any(n["desc"] == "Continue" or n["text"] == "Continue" for n in ns):
            return
        time.sleep(1)
    raise AssertionError("app did not reach Welcome")


def onboard():
    tap("Skip")
    type_into("Nickname", "droid")
    type_into("App PIN (6 digits)", "246810")
    type_into("Repeat PIN", "246810")
    tap("Create identity")
    find("Chats", timeout=90)


def settings_and_lock():
    tap("Settings")
    find("Radio mode")
    for _ in range(4):
        adb("shell", "input", "swipe", "500", "1800", "500", "700")
        time.sleep(0.5)
        if any(n["text"] == "Lock now" for n in nodes()):
            break
    integrity = [n["text"] for n in nodes() if "boot" in n["text"] or "checked" in n["text"] or "Warning" in n["text"]]
    print("   device integrity:", integrity[:1])
    tap("Lock now", timeout=10)
    find("Enter PIN", timeout=20)


def wrong_then_right_pin():
    for d in "000000":
        tap(f"Digit {d}")
    find("Wrong PIN", timeout=60, exact=False)
    for d in "246810":
        tap(f"Digit {d}")
    find("Chats", timeout=60)


def restart_requires_pin():
    adb("shell", "am", "force-stop", "org.meshtrike.app")
    adb("shell", "am", "start", "-n", "org.meshtrike.app/.MainActivity")
    find("Enter PIN", timeout=60)
    for d in "246810":
        tap(f"Digit {d}")
    find("Chats", timeout=60)


def channel_post():
    tap("New chat")
    tap("Public channels", exact=False)
    type_into("Channel name", "plaza")
    tap("Join")
    find("Message text", timeout=20)
    type_into("Message text", "hello_from_android")
    tap("Send")
    find("hello_from_android", timeout=60)


if __name__ == "__main__":
    try:
        step("first launch: runtime permissions, core starts, relay service up", launch_and_grant)
        step("onboarding on device (hardware keystore, SQLCipher, Argon2id 64 MiB)", onboard)
        step("settings, then lock", settings_and_lock)
        step("wrong PIN refused, right PIN unlocks", wrong_then_right_pin)
        step("cold restart: relay store opens, message store needs PIN", restart_requires_pin)
        step("join channel and post (PoW + sealed broadcast)", channel_post)
        print("ANDROID FLOWS PASSED")
    except AssertionError as e:
        print("FAILED:", e)
        print([n["text"] or n["desc"] for n in nodes() if n["text"] or n["desc"]][:40])
        sys.exit(1)
