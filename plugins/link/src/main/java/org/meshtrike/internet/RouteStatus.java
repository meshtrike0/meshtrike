package org.meshtrike.internet;

import java.net.URI;
import java.util.ArrayList;
import java.util.Collections;
import java.util.List;

/**
 * A read-only snapshot of the internet route for the add-on's own screen,
 * which runs in the same process as the service. It carries no addresses,
 * keys or messages: only the state, the mode and where each bridge is.
 */
public final class RouteStatus {
    /** The service is not running: Meshtrike has not asked for the route. */
    public static final int IDLE = 0;
    /** Meshtrike's Internet route switch is off. */
    public static final int OFF = 1;
    /** Held down by the add-on's own switches; see {@link #held}. */
    public static final int HELD = 2;
    /** On in Meshtrike, but there is no bridge and direct is not allowed. */
    public static final int NEEDS_BRIDGE = 3;
    public static final int CONNECTING = 4;
    public static final int CONNECTED = 5;

    public static final class Bridge {
        /** host:port of the bridge. */
        public final String endpoint;
        /** The website the bridge looks like (REALITY server name), or null. */
        public final String looksLike;
        /** The route connected through this bridge right now. */
        public final boolean inUse;

        Bridge(String endpoint, String looksLike, boolean inUse) {
            this.endpoint = endpoint;
            this.looksLike = looksLike;
            this.inUse = inUse;
        }
    }

    public final int state;
    /** "bridge" or "direct" when connected or connecting, else null. */
    public final String mode;
    public final boolean allowDirect;
    public final String held;
    public final List<Bridge> bridges;

    private RouteStatus(int state, String mode, boolean allowDirect, String held, List<Bridge> bridges) {
        this.state = state;
        this.mode = mode;
        this.allowDirect = allowDirect;
        this.held = held;
        this.bridges = bridges;
    }

    public static RouteStatus now() {
        Route r = Route.live;
        Route.Config c = r == null ? null : r.last();
        if (c == null) return new RouteStatus(IDLE, null, false, null, Collections.emptyList());
        String mode = r.mode();
        boolean connected = r.address() != null;
        int state;
        if (!c.enabled) state = OFF;
        else if (r.held() != null) state = HELD;
        else if (mode == null) state = NEEDS_BRIDGE;
        else state = connected ? CONNECTED : CONNECTING;

        List<Bridge> out = new ArrayList<>();
        List<String> links = c.bridges;
        int active = -1;
        List<String> running = r.bridges();
        if (connected && "bridge".equals(mode) && !running.isEmpty()) {
            active = links.indexOf(running.get(r.bridgeIndex() % running.size()));
        }
        for (int i = 0; i < links.size(); i++) {
            Bridge b = describe(links.get(i), i == active);
            if (b != null) out.add(b);
        }
        return new RouteStatus(state, state >= CONNECTING ? mode : null, c.allowDirect, r.held(), out);
    }

    /** Re-applies Meshtrike's last config after the add-on's switches changed. */
    public static void reapply() {
        Route r = Route.live;
        if (r != null) r.reapply();
    }

    private static Bridge describe(String link, boolean inUse) {
        try {
            URI u = new URI(link);
            if (u.getHost() == null) return null;
            String sni = null;
            String q = u.getRawQuery();
            if (q != null) {
                for (String kv : q.split("&")) {
                    if (kv.startsWith("sni=")) sni = java.net.URLDecoder.decode(kv.substring(4), "UTF-8");
                }
            }
            String host = u.getHost().contains(":") && !u.getHost().startsWith("[") ? "[" + u.getHost() + "]" : u.getHost();
            return new Bridge(host + (u.getPort() > 0 ? ":" + u.getPort() : ""), sni == null || sni.isEmpty() ? null : sni, inUse);
        } catch (Exception e) {
            return null;
        }
    }
}
