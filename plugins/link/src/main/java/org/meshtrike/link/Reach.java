package org.meshtrike.link;

import android.content.Context;
import android.content.Intent;
import android.content.IntentFilter;
import android.content.SharedPreferences;
import android.os.BatteryManager;

/**
 * The add-on's own switches, set on its screen. They can only hold service
 * work back, never widen it: while paused (or a route is switched off here)
 * Wi-Fi Direct refuses new links and the internet route stays down, and
 * Meshtrike keeps working over its other routes.
 */
public final class Reach {
    private static final String FILE = "reach";
    public static final String PAUSED = "paused";
    public static final String WIFI = "wifi";
    public static final String INET = "inet";
    public static final String CHARGING_ONLY = "chargingOnly";
    public static final String SETUP_DONE = "setupDone";

    private Reach() {}

    private static SharedPreferences prefs(Context c) {
        return c.getApplicationContext().getSharedPreferences(FILE, Context.MODE_PRIVATE);
    }

    public static boolean get(Context c, String key) {
        boolean def = WIFI.equals(key) || INET.equals(key);
        return prefs(c).getBoolean(key, def);
    }

    public static void set(Context c, String key, boolean v) {
        prefs(c).edit().putBoolean(key, v).commit();
    }

    /** Wi-Fi Direct may open new links. */
    public static boolean wifiAllowed(Context c) {
        return !get(c, PAUSED) && get(c, WIFI);
    }

    /** Why the internet route is held back here, or null when it may run. */
    public static String routeHold(Context c) {
        if (get(c, PAUSED)) return "paused";
        if (!get(c, INET)) return "off in add-on";
        if (get(c, CHARGING_ONLY) && !charging(c)) return "waiting for charger";
        return null;
    }

    /** The sticky battery broadcast: no permission and no receiver needed. */
    public static boolean charging(Context c) {
        try {
            Intent b = c.getApplicationContext().registerReceiver(null, new IntentFilter(Intent.ACTION_BATTERY_CHANGED));
            return b != null && b.getIntExtra(BatteryManager.EXTRA_PLUGGED, 0) != 0;
        } catch (RuntimeException e) {
            return false;
        }
    }
}
