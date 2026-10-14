#!/usr/bin/env python3
"""Two-device test over real Android Bluetooth (two emulators share the
emulator's virtual Bluetooth controller): channel post across devices, then
remote pairing and a private message.

Usage: python3 e2e/android_mesh.py SERIAL_A SERIAL_B
Device A must already be onboarded by android_drive.py (PIN 246810).
"""
import sys
import time

import android_drive as d

A, B = sys.argv[1], sys.argv[2]


def on(serial):
    d.SERIAL = serial


def ensure_home(serial, pin):
    on(serial)
    d.adb("shell", "am", "start", "-n", "org.meshtrike.app/.MainActivity")
    t0 = time.time()
    while time.time() - t0 < 60:
        ns = d.nodes()
        labels = {n["text"] or n["desc"] for n in ns}
        allow = [n for n in ns if n["text"] in ("Allow", "While using the app", "Wait")]
        if allow:
            d.adb("shell", "input", "tap", str(allow[0]["cx"]), str(allow[0]["cy"]))
        elif "Enter PIN" in labels:
            for digit in pin:
                d.tap(f"Digit {digit}")
            d.find("Chats", timeout=60)
            return
        elif "Chats" in labels:
            return
        elif "Back" in labels:
            d.tap("Back")
        elif "Continue" in labels:
            raise AssertionError(f"{serial} is not onboarded")
        time.sleep(1)
    raise AssertionError(f"{serial}: home not reached")


def open_channel(serial, name):
    on(serial)
    d.tap("New chat")
    d.tap("Public channels", exact=False)
    d.type_into("Channel name", name)
    d.tap("Join")
    d.find("Message text", timeout=20)


if __name__ == "__main__":
    try:
        on(B)
        d.step("device B: first launch and onboarding", lambda: (d.launch_and_grant(), d.onboard()))
        d.step("device A: unlocked at home", lambda: ensure_home(A, "246810"))
        d.step("both join #square", lambda: (open_channel(A, "square"), open_channel(B, "square")))

        def post_across():
            on(B)
            d.type_into("Message text", "over_real_bluetooth")
            d.tap("Send")
            on(A)
            d.find("over_real_bluetooth", timeout=180)

        d.step("post from B arrives on A over BLE GATT + Noise link", post_across)

        def remote_pair():
            on(A)
            d.tap("Start private chat")
            on(B)
            d.find("wants to chat", timeout=180, exact=False)
            d.tap("Accept")
            on(A)
            d.tap("Back")
            d.tap("Back")
            d.tap("Back")
            d.tap("New chat")
            d.find("Not verified", timeout=180, exact=False)

        d.step("remote pairing (Noise XX over the mesh) creates an unverified contact", remote_pair)
        print("ANDROID MESH PASSED")
    except AssertionError as e:
        print("FAILED:", e)
        print([n["text"] or n["desc"] for n in d.nodes() if n["text"] or n["desc"]][:30])
        sys.exit(1)
