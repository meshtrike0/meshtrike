package org.meshtrike.wifilink;

import android.os.SystemClock;
import android.util.Log;

/**
 * Opt-in diagnostics, off unless switched on over adb:
 *
 *   adb shell setprop debug.meshtrike.log 1
 *
 * Lines carry steps and Android's reason codes only: no group names,
 * passphrases, addresses or content.
 */
public final class Diag {
    private static volatile boolean on;
    private static volatile long checkedAt;

    private Diag() {}

    static boolean enabled() {
        long now = SystemClock.elapsedRealtime();
        if (now - checkedAt > 5_000) {
            checkedAt = now;
            try {
                Object v = Class.forName("android.os.SystemProperties")
                    .getMethod("get", String.class, String.class)
                    .invoke(null, "debug.meshtrike.log", "0");
                on = "1".equals(v);
            } catch (Exception e) {
                on = false;
            }
        }
        return on;
    }

    public static void log(String msg) {
        if (enabled()) Log.i("MeshtrikeLink", msg);
    }
}
