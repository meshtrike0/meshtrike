package org.meshtrike.internet;

import android.content.BroadcastReceiver;
import android.content.Context;
import android.content.Intent;
import android.content.IntentFilter;
import android.os.Build;
import android.os.Handler;
import android.os.HandlerThread;

import org.json.JSONArray;
import org.json.JSONObject;

import android.content.SharedPreferences;

import org.meshtrike.link.Reach;

import java.io.File;
import java.net.InetAddress;
import java.net.InetSocketAddress;
import java.net.ServerSocket;
import java.net.Socket;
import java.net.URI;
import java.nio.charset.StandardCharsets;
import java.security.SecureRandom;
import java.util.ArrayList;
import java.util.List;

/**
 * Runs the internet route: Xray (VLESS + REALITY to a bridge) behind one
 * fixed local SOCKS5 endpoint, and the Nym client (Native) forced through it.
 *
 * - The SOCKS endpoint listens on 127.0.0.1 only, on a port and with a
 *   password chosen at random for this process, so no other app can use it.
 * - The endpoint never changes while the process lives (the Nym HTTP client
 *   keeps the proxy it was built with); bridges rotate behind it.
 * - Switching between "bridge" and "direct" restarts the process, so a
 *   process never mixes the two.
 * - If a bridge does not get the client connected within BRIDGE_TRY_MS, the
 *   next one is tried.
 * - With "direct" allowed, bridges are preferred but not required: when no
 *   bridge can be reached from this network (or none gets the client
 *   connected), the route connects directly, and goes back to the bridge as
 *   soon as one is reachable again. The app is told which mode is in use.
 * - The add-on's own switches (Reach) can hold the route down: paused,
 *   switched off on the add-on's screen, or "only when charging" on battery.
 */
final class Route {
    interface Listener {
        void ready(String address);

        void down(String reason);

        void envelope(byte[] env);

        /** "bridge" or "direct", sent each time the route connects. */
        void mode(String mode);
    }

    static final class Config {
        boolean enabled;
        boolean allowDirect;
        List<String> bridges = new ArrayList<>();
    }

    private static final long BRIDGE_TRY_MS = 60_000;
    /** How often a direct route checks whether a bridge is reachable again. */
    private static final long RECHECK_DIRECT_MS = 5 * 60_000;
    /** How often a bridge route that is not connected re-evaluates. */
    private static final long RECHECK_BRIDGE_MS = 30_000;
    /** A reachable bridge that never got the client connected is skipped this long. */
    private static final long BRIDGE_FAIL_HOLD_MS = 30 * 60_000;
    private static final int PROBE_TIMEOUT_MS = 3_000;
    private static final int PROBE_MAX_BRIDGES = 3;

    private final Context ctx;
    private final Listener listener;
    private final HandlerThread thread = new HandlerThread("net-route");
    private final Handler h;
    // One local SOCKS endpoint per process, not per Route: Nym builds its
    // HTTP client once per process with the first proxy it sees, and the
    // service (with its Route) is recreated whenever Meshtrike restarts.
    private static final SecureRandom RNG = new SecureRandom();
    private static final int socksPort = freePort();
    private static final String socksUser = "u" + Long.toHexString(RNG.nextLong());
    private static final String socksPass = Long.toHexString(RNG.nextLong()) + Long.toHexString(RNG.nextLong());
    /** "bridge" or "direct": how Nym first ran in this process (its HTTP client keeps that). */
    private static String processMode;

    /** The route of the running service, read by the add-on's screen (same process). */
    static volatile Route live;

    // Guarded by the handler thread (volatile ones are also read by RouteStatus).
    private volatile String mode; // null (off), "bridge" or "direct"
    private boolean nativeRunning;
    private boolean xrayRunning;
    private volatile List<String> bridges = new ArrayList<>();
    private volatile int bridgeIndex;
    private Runnable rotation;
    private Runnable recheck;
    private volatile Config last;
    /** Why the add-on holds the route down (Reach.routeHold), or null. */
    private volatile String held;
    private long bridgeSince; // bridge mode without a connection since (0: connected)
    private volatile String address;
    private volatile String gateway; // far end of the gateway socket (trace only)

    Route(Context ctx, Listener listener) {
        this.ctx = ctx.getApplicationContext();
        this.listener = listener;
        thread.start();
        h = new Handler(thread.getLooper());
        Native.listener = (kind, data) -> h.post(() -> onNative(kind, data));
        // "Only when charging": re-evaluate when the charger comes or goes.
        IntentFilter f = new IntentFilter(Intent.ACTION_POWER_CONNECTED);
        f.addAction(Intent.ACTION_POWER_DISCONNECTED);
        if (Build.VERSION.SDK_INT >= 33) this.ctx.registerReceiver(power, f, Context.RECEIVER_NOT_EXPORTED);
        else this.ctx.registerReceiver(power, f);
        live = this;
    }

    private final BroadcastReceiver power = new BroadcastReceiver() {
        @Override
        public void onReceive(Context c, Intent i) {
            if (Reach.get(c, Reach.CHARGING_ONLY)) reapply();
        }
    };

    /** Apply the last config again, after the add-on's own switches changed. */
    void reapply() {
        h.post(() -> {
            Config c = last;
            if (c != null) applyNow(c);
        });
    }

    Config last() {
        return last;
    }

    List<String> bridges() {
        return bridges;
    }

    int bridgeIndex() {
        return bridgeIndex;
    }

    String held() {
        return held;
    }

    String address() {
        return address;
    }

    String mode() {
        return mode;
    }

    void apply(Config c) {
        h.post(() -> applyNow(c));
    }

    boolean send(String addr, byte[] env) {
        boolean ok = address != null && Native.send(addr, env);
        if (Trace.on()) {
            Trace.log(ok
                ? "→ envelope " + Trace.fp(env) + " (" + env.length + " B, sealed) from our address " + address
                    + " to " + addr + " · into the mixnet " + via()
                : "envelope " + Trace.fp(env) + " not sent: route not connected");
        }
        return ok;
    }

    /** How this phone reaches its gateway, for the trace. */
    private String via() {
        String gw = gateway == null ? "?" : gateway;
        if ("bridge".equals(mode) && !bridges.isEmpty()) {
            String b = bridges.get(bridgeIndex % bridges.size());
            String host = "?";
            try {
                URI u = new URI(b);
                host = u.getHost() + ":" + u.getPort();
            } catch (Exception ignored) {
            }
            return "through bridge " + host + " (disguised as a web visit) to Nym entry gateway " + gw;
        }
        return "directly to Nym entry gateway " + gw;
    }

    void shutdown() {
        if (live == this) live = null;
        try {
            ctx.unregisterReceiver(power);
        } catch (RuntimeException ignored) {
        }
        h.post(this::stopAll);
        thread.quitSafely();
    }

    // ===== Control =====

    private void applyNow(Config c) {
        last = c;
        if (!c.enabled) {
            held = null;
            stopAll();
            listener.down("off");
            return;
        }
        String hold = Reach.routeHold(ctx);
        held = hold;
        if (hold != null) {
            stopAll();
            listener.down(hold);
            return;
        }
        boolean useBridge = !c.bridges.isEmpty() && (!c.allowDirect || bridgeUsable(c.bridges));
        if (useBridge) {
            if ("direct".equals(mode) || "direct".equals(processMode)) {
                restartProcess();
                return;
            }
            boolean changed = !c.bridges.equals(bridges);
            bridges = new ArrayList<>(c.bridges);
            if (changed) bridgeIndex = 0;
            mode = "bridge";
            if (!xrayRunning || changed) startXray();
            if (!nativeRunning) {
                nativeRunning = Native.start(dataDir(), "127.0.0.1", socksPort, socksUser, socksPass);
                if (nativeRunning) processMode = "bridge";
            }
            if (address == null && bridgeSince == 0) bridgeSince = System.currentTimeMillis();
            scheduleRotation();
            scheduleRecheck();
        } else if (c.allowDirect) {
            if ("bridge".equals(mode) || "bridge".equals(processMode)) {
                restartProcess();
                return;
            }
            mode = "direct";
            if (!nativeRunning) {
                nativeRunning = Native.start(dataDir(), null, 0, null, null);
                if (nativeRunning) processMode = "direct";
            }
            scheduleRecheck();
        } else {
            stopAll();
            listener.down("needs a bridge");
        }
    }

    private void onNative(int kind, byte[] data) {
        String s = new String(data, StandardCharsets.UTF_8);
        if (kind == Native.READY) {
            address = s;
            bridgeSince = 0;
            cancelRotation();
            listener.ready(s);
            if (mode != null) listener.mode(mode);
            Trace.log("connected to the Nym mixnet · mode " + mode + " · our address " + s
                + " (identity.encryption@gateway)");
        } else if (kind == Native.GATEWAY) {
            gateway = s;
            Trace.log("gateway connection " + via());
        } else if (kind == Native.DOWN) {
            address = null;
            listener.down(s);
            if ("bridge".equals(mode)) {
                if (bridgeSince == 0) bridgeSince = System.currentTimeMillis();
                scheduleRotation();
            }
        } else if (kind == Native.ENVELOPE) {
            if (Trace.on()) {
                Trace.log("← envelope " + Trace.fp(data) + " (" + data.length + " B, sealed) arrived at our address "
                    + address + " from our gateway " + (gateway == null ? "?" : gateway)
                    + " · Meshtrike marker checked, passed to Meshtrike");
            }
            listener.envelope(data);
        }
    }

    /** No connection through this bridge within the window: try the next. */
    private void scheduleRotation() {
        if (rotation != null || bridges.size() < 2) return;
        rotation = () -> {
            rotation = null;
            if (address != null || !"bridge".equals(mode)) return;
            bridgeIndex = (bridgeIndex + 1) % bridges.size();
            startXray();
            scheduleRotation();
        };
        h.postDelayed(rotation, BRIDGE_TRY_MS);
    }

    private void cancelRotation() {
        if (rotation != null) h.removeCallbacks(rotation);
        rotation = null;
    }

    // ===== Bridge or direct =====

    /** Bridges can be used from here: one answers, and none failed recently. */
    private boolean bridgeUsable(List<String> links) {
        if (System.currentTimeMillis() < prefs().getLong("bridgeFailUntil", 0)) return false;
        int n = 0;
        for (String l : links) {
            if (n++ >= PROBE_MAX_BRIDGES) break;
            if (reachable(l)) return true;
        }
        return false;
    }

    /** A plain TCP connect to the bridge (what Xray would open anyway). */
    private static boolean reachable(String link) {
        try {
            URI u = new URI(link);
            if (u.getHost() == null || u.getPort() <= 0) return false;
            try (Socket s = new Socket()) {
                s.connect(new InetSocketAddress(u.getHost(), u.getPort()), PROBE_TIMEOUT_MS);
                return true;
            }
        } catch (Exception e) {
            return false;
        }
    }

    /**
     * With direct allowed and bridges saved, keep checking the choice: back
     * to the bridge once one answers; to direct when the bridge route has not
     * connected for a while. Either switch restarts the process (one proxy
     * policy per process).
     */
    private void scheduleRecheck() {
        Config c = last;
        if (recheck != null || c == null || !c.enabled || !c.allowDirect || c.bridges.isEmpty()) return;
        recheck = () -> {
            recheck = null;
            Config k = last;
            if (k == null || !k.enabled || !k.allowDirect || k.bridges.isEmpty()) return;
            if ("direct".equals(mode)) {
                if (bridgeUsable(k.bridges)) {
                    restartProcess();
                    return;
                }
            } else if ("bridge".equals(mode) && address == null && bridgeSince != 0) {
                long waited = System.currentTimeMillis() - bridgeSince;
                if (waited >= BRIDGE_TRY_MS * Math.max(1, Math.min(k.bridges.size(), PROBE_MAX_BRIDGES))) {
                    // Reachable but never connected: skip bridges for a while.
                    if (bridgeUsable(k.bridges)) {
                        prefs().edit().putLong("bridgeFailUntil", System.currentTimeMillis() + BRIDGE_FAIL_HOLD_MS).apply();
                    }
                    restartProcess();
                    return;
                }
            }
            scheduleRecheck();
        };
        h.postDelayed(recheck, "direct".equals(mode) ? RECHECK_DIRECT_MS : RECHECK_BRIDGE_MS);
    }

    private void cancelRecheck() {
        if (recheck != null) h.removeCallbacks(recheck);
        recheck = null;
    }

    private SharedPreferences prefs() {
        return ctx.getSharedPreferences("route", Context.MODE_PRIVATE);
    }

    private void stopAll() {
        cancelRecheck();
        cancelRotation();
        if (nativeRunning) Native.stop();
        nativeRunning = false;
        stopXray();
        address = null;
        mode = null;
        bridgeSince = 0;
    }

    /** The process's proxy policy is fixed once Nym starts; a new mode needs a new process. */
    private void restartProcess() {
        listener.down("restarting");
        stopAll();
        android.os.Process.killProcess(android.os.Process.myPid());
    }

    private String dataDir() {
        File d = new File(ctx.getNoBackupFilesDir(), "nym");
        //noinspection ResultOfMethodCallIgnored
        d.mkdirs();
        return d.getAbsolutePath();
    }

    // ===== Xray =====

    private void startXray() {
        stopXray();
        String link = bridges.get(bridgeIndex % bridges.size());
        try {
            JSONObject outbound = outboundFor(link);
            outbound.put("tag", "bridge");
            JSONObject socks = new JSONObject()
                .put("listen", "127.0.0.1")
                .put("port", socksPort)
                .put("protocol", "socks")
                .put("settings", new JSONObject()
                    .put("auth", "password")
                    .put("udp", false)
                    .put("accounts", new JSONArray().put(new JSONObject().put("user", socksUser).put("pass", socksPass))));
            JSONObject cfg = new JSONObject()
                .put("log", new JSONObject().put("loglevel", "none"))
                .put("inbounds", new JSONArray().put(socks))
                .put("outbounds", new JSONArray().put(outbound));
            JSONObject r = invoke("runXray", new JSONObject().put("xrayJson", cfg.toString()));
            xrayRunning = r.optBoolean("success", false);
            if (!xrayRunning) listener.down("bridge rejected");
        } catch (Exception e) {
            xrayRunning = false;
            listener.down("bridge rejected");
        }
    }

    private void stopXray() {
        if (!xrayRunning) return;
        try {
            invoke("stopXray", new JSONObject());
        } catch (Exception ignored) {
        }
        xrayRunning = false;
    }

    /** The share link as an Xray outbound, parsed and validated by libXray. */
    private static JSONObject outboundFor(String link) throws Exception {
        JSONObject r = invoke("convertShareLinksToXrayJson", new JSONObject().put("text", link));
        JSONArray outs = r.getJSONObject("data").getJSONArray("outbounds");
        JSONObject o = outs.getJSONObject(0);
        // Only REALITY bridges: anything else would show an unusual connection.
        JSONObject ss = o.optJSONObject("streamSettings");
        if (ss == null || !"reality".equals(ss.optString("security"))) throw new IllegalArgumentException("not reality");
        return o;
    }

    private static JSONObject invoke(String method, JSONObject payload) throws Exception {
        JSONObject req = new JSONObject().put("apiVersion", 3).put("method", method).put("payload", payload);
        return new JSONObject(libXray.LibXray.invoke(req.toString()));
    }

    private static int freePort() {
        try (ServerSocket s = new ServerSocket(0, 1, InetAddress.getByName("127.0.0.1"))) {
            return s.getLocalPort();
        } catch (Exception e) {
            return 47_000 + new SecureRandom().nextInt(10_000);
        }
    }
}
