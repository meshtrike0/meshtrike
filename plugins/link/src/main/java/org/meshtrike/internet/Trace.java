package org.meshtrike.internet;

import android.os.SystemClock;
import android.util.Log;

import java.security.MessageDigest;

/**
 * Opt-in message trace for testing, separate from (and more revealing than)
 * the normal diagnostics: it logs mixnet addresses and gateway IPs. Off
 * unless switched on over adb, never in normal use:
 *
 *   adb shell setprop debug.meshtrike.trace 1
 *
 * Envelopes are named by a short fingerprint (#1a2b3c4d, the first bytes of
 * SHA-256 over the sealed envelope), the same on both phones, so one message
 * can be followed from sender to receiver. Content is sealed and never logged.
 */
final class Trace {
    private static volatile boolean on;
    private static volatile long checkedAt;

    private Trace() {}

    static boolean on() {
        long now = SystemClock.elapsedRealtime();
        if (now - checkedAt > 5_000) {
            checkedAt = now;
            try {
                Object v = Class.forName("android.os.SystemProperties")
                    .getMethod("get", String.class, String.class)
                    .invoke(null, "debug.meshtrike.trace", "0");
                on = "1".equals(v);
            } catch (Exception e) {
                on = false;
            }
        }
        return on;
    }

    static void log(String msg) {
        if (on()) Log.i("MeshtrikeTrace", "add-on: " + msg);
    }

    static String fp(byte[] b) {
        try {
            byte[] d = MessageDigest.getInstance("SHA-256").digest(b);
            return String.format("#%02x%02x%02x%02x", d[0], d[1], d[2], d[3]);
        } catch (Exception e) {
            return "#????????";
        }
    }
}
