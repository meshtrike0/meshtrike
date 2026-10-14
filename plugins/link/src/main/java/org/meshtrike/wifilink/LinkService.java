package org.meshtrike.wifilink;

import android.Manifest;
import android.annotation.SuppressLint;
import android.app.Service;
import android.content.Context;
import android.content.Intent;
import android.content.pm.PackageManager;
import android.location.LocationManager;
import android.net.wifi.WifiManager;
import android.net.wifi.p2p.WifiP2pConfig;
import android.net.wifi.p2p.WifiP2pManager;
import android.os.Binder;
import android.os.Build;
import android.os.Handler;
import android.os.HandlerThread;
import android.os.IBinder;
import android.os.Parcel;
import android.os.ParcelFileDescriptor;
import android.os.Parcelable;
import android.os.SystemClock;

import java.io.FileInputStream;
import java.io.FileOutputStream;
import java.io.IOException;
import java.io.InputStream;
import java.io.OutputStream;
import java.net.Inet4Address;
import java.net.InetAddress;
import java.net.InetSocketAddress;
import java.net.InterfaceAddress;
import java.net.NetworkInterface;
import java.net.ServerSocket;
import java.net.Socket;
import java.util.Collections;
import java.util.concurrent.atomic.AtomicLong;
import java.util.regex.Pattern;

import org.meshtrike.link.Reach;

/**
 * Meshtrike's Wi-Fi Direct link. Meshtrike itself holds no INTERNET and no
 * Wi-Fi permission; it asks this service to create or join a Wi-Fi Direct
 * group and receives one end of a private Unix socket pair, whose bytes this
 * service relays over a single TCP connection inside that group.
 *
 * What it can and cannot do:
 * - Only callers signed with the Meshtrike key reach it (signature permission
 *   on the service, checked again per call).
 * - It never discovers peers or shows device names: both phones already know
 *   the group name and passphrase from their Bluetooth link, so the joining
 *   phone connects to that name directly.
 * - Its sockets only bind or connect on a p2p* interface. No DNS, no other
 *   interface, no other address.
 * - It never sees keys, names or plaintext. Meshtrike's frames are already
 *   encrypted, and the two Meshtrike apps then authenticate each other with
 *   a token this service is never given.
 * - While the add-on is paused, or Wi-Fi Direct is switched off on its
 *   screen, new links are refused (Meshtrike stays on Bluetooth).
 */
@SuppressLint("MissingPermission")
public final class LinkService extends Service {
    static final String DESCRIPTOR = "org.meshtrike.wifilink.ILink/2";
    /** open(role, networkName, passphrase, timeoutMs) -> ParcelFileDescriptor or nothing. */
    static final int OPEN = IBinder.FIRST_CALL_TRANSACTION;
    /** status() -> bit flags below. */
    static final int STATUS = IBinder.FIRST_CALL_TRANSACTION + 1;
    static final int ROLE_CREATE = 0;
    static final int ROLE_JOIN = 1;
    static final int HAS_HARDWARE = 1;
    static final int WIFI_ON = 2;
    static final int PERMITTED = 4;
    static final int READY = HAS_HARDWARE | WIFI_ON | PERMITTED;
    static final String MAIN_APP = "org.meshtrike.app";

    private static final int PORT = 47_821;
    private static final int BUF = 64 * 1024;
    /** Names Meshtrike's core produces. Only such groups are ever created, joined or removed. */
    private static final Pattern OUR_GROUP = Pattern.compile("^DIRECT-[A-Za-z0-9]{2}-[0-9a-f]{8}$");
    private static final Pattern PASSPHRASE = Pattern.compile("^[0-9a-f]{24}$");

    private HandlerThread thread;
    private Handler h;
    private WifiP2pManager manager;
    private WifiP2pManager.Channel channel;
    /** One link at a time. Guarded by the handler thread. */
    private Session session;

    // For the add-on's own screen (same process): link state and byte counts only.
    static final int IDLE = 0;
    static final int SETTING_UP = 1;
    static final int LINKED = 2;
    static volatile int phase = IDLE;
    static final AtomicLong sent = new AtomicLong();
    static final AtomicLong received = new AtomicLong();
    private static volatile LinkService live;

    private static final class Session {
        final int role;
        final String name;
        final String pass;
        final long deadline;
        final ParcelFileDescriptor local;
        volatile boolean closed;
        volatile boolean linked;
        volatile Socket tcp;
        volatile ServerSocket server;

        Session(int role, String name, String pass, long deadline, ParcelFileDescriptor local) {
            this.role = role;
            this.name = name;
            this.pass = pass;
            this.deadline = deadline;
            this.local = local;
        }
    }

    @Override
    public void onCreate() {
        super.onCreate();
        thread = new HandlerThread("wifi-link");
        thread.start();
        h = new Handler(thread.getLooper());
        manager = (WifiP2pManager) getSystemService(Context.WIFI_P2P_SERVICE);
        live = this;
    }

    /** Ends the current link at once (the add-on was paused or Wi-Fi Direct switched off). */
    static void stopNow() {
        LinkService l = live;
        if (l != null) l.h.post(() -> {
            if (l.session != null) l.close(l.session);
        });
    }

    @Override
    public void onDestroy() {
        if (live == this) live = null;
        phase = IDLE;
        h.post(() -> {
            if (session != null) close(session);
        });
        thread.quitSafely();
        super.onDestroy();
    }

    @Override
    public IBinder onBind(Intent intent) {
        return binder;
    }

    private final Binder binder = new Binder() {
        @Override
        protected boolean onTransact(int code, Parcel data, Parcel reply, int flags) throws android.os.RemoteException {
            if (code == INTERFACE_TRANSACTION) {
                reply.writeString(DESCRIPTOR);
                return true;
            }
            if (code != OPEN && code != STATUS) return super.onTransact(code, data, reply, flags);
            data.enforceInterface(DESCRIPTOR);
            if (!callerIsMeshtrike(Binder.getCallingUid())) throw new SecurityException("not Meshtrike");
            if (code == STATUS) {
                reply.writeNoException();
                reply.writeInt(status());
                return true;
            }
            int role = data.readInt();
            String name = data.readString();
            String pass = data.readString();
            int timeoutMs = data.readInt();
            ParcelFileDescriptor pfd = open(role, name, pass, timeoutMs);
            reply.writeNoException();
            if (pfd == null) {
                reply.writeInt(0);
            } else {
                reply.writeInt(1);
                // Writes the descriptor and closes this process's copy.
                pfd.writeToParcel(reply, Parcelable.PARCELABLE_WRITE_RETURN_VALUE);
            }
            return true;
        }
    };

    private boolean callerIsMeshtrike(int uid) {
        PackageManager pm = getPackageManager();
        String[] pkgs = pm.getPackagesForUid(uid);
        if (pkgs == null) return false;
        for (String p : pkgs) {
            if (MAIN_APP.equals(p) && pm.checkSignatures(uid, android.os.Process.myUid()) == PackageManager.SIGNATURE_MATCH) {
                return true;
            }
        }
        return false;
    }

    // ===== Status =====

    int status() {
        return statusOf(this);
    }

    static int statusOf(Context c) {
        int s = 0;
        try {
            if (c.getPackageManager().hasSystemFeature(PackageManager.FEATURE_WIFI_DIRECT)
                && c.getSystemService(Context.WIFI_P2P_SERVICE) != null) {
                s |= HAS_HARDWARE;
            }
            WifiManager wm = (WifiManager) c.getApplicationContext().getSystemService(Context.WIFI_SERVICE);
            if (wm != null && wm.isWifiEnabled()) s |= WIFI_ON;
            if (permitted(c)) s |= PERMITTED;
        } catch (RuntimeException ignored) {
        }
        return s;
    }

    /** Android 13+: Nearby Wi-Fi devices (never used for location). 10-12: location, and location on. */
    static boolean permitted(Context c) {
        if (Build.VERSION.SDK_INT >= 33) {
            return c.checkSelfPermission(Manifest.permission.NEARBY_WIFI_DEVICES) == PackageManager.PERMISSION_GRANTED;
        }
        LocationManager lm = (LocationManager) c.getSystemService(Context.LOCATION_SERVICE);
        return c.checkSelfPermission(Manifest.permission.ACCESS_FINE_LOCATION) == PackageManager.PERMISSION_GRANTED
            && lm != null && lm.isLocationEnabled();
    }

    // ===== Open =====

    /** Validates the request and starts the group; the socket pair end is returned at once. */
    private ParcelFileDescriptor open(int role, String name, String pass, int timeoutMs) {
        if (role != ROLE_CREATE && role != ROLE_JOIN) return null;
        if (name == null || !OUR_GROUP.matcher(name).matches()) return null;
        if (pass == null || !PASSPHRASE.matcher(pass).matches()) return null;
        if (!Reach.wifiAllowed(this)) {
            Diag.log("wifi: open refused, held on the add-on's screen");
            return null;
        }
        if (manager == null || status() != READY) {
            Diag.log("wifi: open refused, status " + status());
            return null;
        }
        ParcelFileDescriptor[] pair;
        try {
            pair = ParcelFileDescriptor.createSocketPair();
        } catch (IOException e) {
            return null;
        }
        long deadline = SystemClock.elapsedRealtime() + Math.max(5_000, Math.min(timeoutMs, 60_000));
        Session s = new Session(role, name, pass, deadline, pair[0]);
        h.post(() -> start(s));
        return pair[1];
    }

    private void start(Session s) {
        if (session != null && !session.closed) {
            // One group at a time: refuse, and Meshtrike stays on Bluetooth.
            closePair(s.local);
            return;
        }
        session = s;
        phase = SETTING_UP;
        sent.set(0);
        received.set(0);
        Diag.log("wifi: open as " + (s.role == ROLE_CREATE ? "owner" : "joiner"));
        h.postDelayed(() -> {
            if (!s.linked) {
                Diag.log("wifi: setup deadline passed");
                close(s);
            }
        }, s.deadline - SystemClock.elapsedRealtime());
        if (channel == null) channel = manager.initialize(this, thread.getLooper(), () -> h.post(() -> {
            channel = null;
            if (session != null) close(session);
        }));
        // Clear a stale group of ours (left by a crash); never touch anyone else's.
        manager.requestGroupInfo(channel, g -> h.post(() -> {
            if (s.closed) return;
            String n = g == null ? null : g.getNetworkName();
            if (g == null) {
                form(s);
            } else if (n != null && OUR_GROUP.matcher(n).matches()) {
                manager.removeGroup(channel, listener("clear stale group", () -> form(s), () -> form(s)));
            } else {
                Diag.log("wifi: another app's group is active; staying on Bluetooth");
                close(s);
            }
        }));
    }

    private WifiP2pConfig config(Session s) {
        return new WifiP2pConfig.Builder()
            .setNetworkName(s.name)
            .setPassphrase(s.pass)
            .enablePersistentMode(false)
            .setGroupOperatingBand(WifiP2pConfig.GROUP_OWNER_BAND_AUTO)
            .build();
    }

    private void form(Session s) {
        if (s.closed) return;
        if (s.role == ROLE_CREATE) {
            manager.createGroup(channel, config(s), listener("create group", () -> awaitGroup(s, 0), () -> close(s)));
        } else {
            join(s);
        }
    }

    /** The joining phone may ask before the owner's group exists: retry until the deadline. */
    private void join(Session s) {
        if (s.closed) return;
        manager.connect(channel, config(s), listener("join group", () -> awaitGroup(s, 0), () -> retryJoin(s)));
    }

    private void retryJoin(Session s) {
        if (s.closed || SystemClock.elapsedRealtime() > s.deadline) {
            close(s);
            return;
        }
        manager.cancelConnect(channel, null);
        h.postDelayed(() -> join(s), 2_500);
    }

    /** Poll until our group is formed, then open the socket on the right side. */
    private void awaitGroup(Session s, int polls) {
        if (s.closed) return;
        if (SystemClock.elapsedRealtime() > s.deadline) {
            close(s);
            return;
        }
        manager.requestGroupInfo(channel, g -> h.post(() -> {
            if (s.closed) return;
            if (g == null || !s.name.equals(g.getNetworkName())) {
                // A join that went nowhere for 10 s is asked again.
                if (s.role == ROLE_JOIN && polls > 0 && polls % 20 == 0) retryJoin(s);
                else h.postDelayed(() -> awaitGroup(s, polls + 1), 500);
                return;
            }
            manager.requestConnectionInfo(channel, info -> h.post(() -> {
                if (s.closed) return;
                InetAddress owner = info == null ? null : info.groupOwnerAddress;
                boolean ownerHere = s.role == ROLE_CREATE;
                if (info == null || !info.groupFormed || owner == null || info.isGroupOwner != ownerHere) {
                    h.postDelayed(() -> awaitGroup(s, polls + 1), 500);
                    return;
                }
                Diag.log("wifi: group formed after " + polls + " polls");
                new Thread(() -> link(s, owner), "wifi-link-session").start();
            }));
        }));
    }

    // ===== Socket (Wi-Fi Direct interface only) =====

    private void link(Session s, InetAddress owner) {
        InterfaceAddress p2p = p2pInterfaceFor(owner, s.role == ROLE_CREATE);
        Socket tcp = p2p == null ? null : s.role == ROLE_CREATE ? accept(s, owner) : dial(s, p2p.getAddress(), owner);
        if (tcp == null || s.closed) {
            Diag.log("wifi: socket failed (p2p interface " + (p2p == null ? "missing" : "found") + ")");
            closeQuietly(tcp);
            h.post(() -> close(s));
            return;
        }
        s.tcp = tcp;
        s.linked = true;
        h.post(() -> {
            if (session == s && !s.closed) phase = LINKED;
        });
        Diag.log("wifi: link up");
        relay(s.local, tcp);
        Diag.log("wifi: link closed");
        h.post(() -> close(s));
    }

    /**
     * The p2p* interface address that makes `addr` acceptable: for the owner,
     * `addr` must be that address itself; for the joiner, on the same subnet.
     */
    private static InterfaceAddress p2pInterfaceFor(InetAddress addr, boolean local) {
        if (!(addr instanceof Inet4Address) || !addr.isSiteLocalAddress()) return null;
        try {
            for (NetworkInterface ni : Collections.list(NetworkInterface.getNetworkInterfaces())) {
                if (!ni.isUp() || !ni.getName().startsWith("p2p")) continue;
                for (InterfaceAddress ia : ni.getInterfaceAddresses()) {
                    if (!(ia.getAddress() instanceof Inet4Address)) continue;
                    if (local ? ia.getAddress().equals(addr) : sameSubnet(ia.getAddress(), addr, ia.getNetworkPrefixLength())) {
                        return ia;
                    }
                }
            }
        } catch (IOException e) {
            return null;
        }
        return null;
    }

    private static boolean sameSubnet(InetAddress a, InetAddress b, int prefix) {
        if (prefix < 8 || prefix > 30) return false;
        int x = toInt(a.getAddress());
        int y = toInt(b.getAddress());
        int mask = -1 << (32 - prefix);
        return (x & mask) == (y & mask) && x != y;
    }

    private static int toInt(byte[] b) {
        return ((b[0] & 0xff) << 24) | ((b[1] & 0xff) << 16) | ((b[2] & 0xff) << 8) | (b[3] & 0xff);
    }

    private static Socket accept(Session s, InetAddress own) {
        try (ServerSocket ss = new ServerSocket()) {
            s.server = ss;
            ss.setReuseAddress(true);
            ss.bind(new InetSocketAddress(own, PORT), 1);
            long left = s.deadline - SystemClock.elapsedRealtime();
            if (left <= 0) return null;
            ss.setSoTimeout((int) left);
            return tune(ss.accept());
        } catch (IOException e) {
            return null;
        } finally {
            s.server = null;
        }
    }

    /** Bound to our own p2p address, so the route can only be the Wi-Fi Direct link. */
    private static Socket dial(Session s, InetAddress own, InetAddress owner) {
        while (!s.closed && SystemClock.elapsedRealtime() < s.deadline) {
            Socket sock = new Socket();
            try {
                sock.bind(new InetSocketAddress(own, 0));
                sock.connect(new InetSocketAddress(owner, PORT), 3_000);
                return tune(sock);
            } catch (IOException e) {
                closeQuietly(sock);
                SystemClock.sleep(500);
            }
        }
        return null;
    }

    private static Socket tune(Socket s) throws IOException {
        s.setTcpNoDelay(true);
        s.setKeepAlive(true);
        return s;
    }

    /** Copies bytes both ways until either side closes. */
    private static void relay(ParcelFileDescriptor local, Socket tcp) {
        try {
            InputStream fromApp = new FileInputStream(local.getFileDescriptor());
            OutputStream toApp = new FileOutputStream(local.getFileDescriptor());
            InputStream fromPeer = tcp.getInputStream();
            OutputStream toPeer = tcp.getOutputStream();
            Thread up = new Thread(() -> {
                pump(fromApp, toPeer, sent);
                closeQuietly(tcp);
                closePair(local);
            }, "wifi-link-up");
            up.start();
            pump(fromPeer, toApp, received);
            closeQuietly(tcp);
            closePair(local);
            up.join();
        } catch (IOException | InterruptedException e) {
            closeQuietly(tcp);
        }
    }

    private static void pump(InputStream in, OutputStream out, AtomicLong count) {
        byte[] buf = new byte[BUF];
        try {
            int n;
            while ((n = in.read(buf)) > 0) {
                out.write(buf, 0, n);
                out.flush();
                count.addAndGet(n);
            }
        } catch (IOException ignored) {
        }
    }

    // ===== Teardown =====

    private void close(Session s) {
        if (s.closed) return;
        s.closed = true;
        closeQuietly(s.tcp);
        closeQuietly(s.server);
        closePair(s.local);
        if (channel != null) {
            if (s.role == ROLE_JOIN) manager.cancelConnect(channel, null);
            manager.removeGroup(channel, null);
        }
        if (session == s) {
            session = null;
            phase = IDLE;
        }
    }

    private WifiP2pManager.ActionListener listener(String step, Runnable ok, Runnable fail) {
        return new WifiP2pManager.ActionListener() {
            @Override
            public void onSuccess() {
                Diag.log("wifi: " + step + " ok");
                h.post(ok);
            }

            @Override
            public void onFailure(int reason) {
                // 0 error, 1 unsupported, 2 busy, 3 no service requests.
                Diag.log("wifi: " + step + " failed, reason " + reason);
                h.post(fail);
            }
        };
    }

    /** shutdown() first: it wakes a thread blocked reading the pair; close() alone may not. */
    private static void closePair(ParcelFileDescriptor p) {
        try {
            android.system.Os.shutdown(p.getFileDescriptor(), android.system.OsConstants.SHUT_RDWR);
        } catch (Exception ignored) {
        }
        try {
            p.close();
        } catch (IOException ignored) {
        }
    }

    private static void closeQuietly(java.io.Closeable c) {
        try {
            if (c != null) c.close();
        } catch (IOException ignored) {
        }
    }
}
