package org.meshtrike.internet;

import android.app.Service;
import android.content.Intent;
import android.content.pm.PackageManager;
import android.os.Binder;
import android.os.IBinder;
import android.os.Parcel;
import android.os.ParcelFileDescriptor;
import android.os.Parcelable;

import org.json.JSONArray;
import org.json.JSONObject;

import java.io.DataInputStream;
import java.io.DataOutputStream;
import java.io.FileInputStream;
import java.io.FileOutputStream;
import java.io.IOException;
import java.nio.charset.StandardCharsets;
import java.util.ArrayList;
import java.util.List;

/**
 * Meshtrike's internet route. Meshtrike holds no INTERNET permission; it
 * binds this service, receives one end of a private Unix socket pair, and
 * speaks a small framed protocol over it:
 *
 * <pre>
 *   frame = u32 length (type + payload) | u8 type | payload
 *   app -> add-on   0x01 CONFIG   JSON {"enabled", "bridges": [links], "allow_direct"}
 *                   0x02 SEND     u16 address length | address | sealed envelope
 *   add-on -> app   0x81 READY    our mixnet address
 *                   0x82 DOWN     reason (no identifiers)
 *                   0x83 ENVELOPE sealed envelope
 *                   0x84 MODE     "bridge" or "direct", after READY
 * </pre>
 *
 * Everything this service carries is already sealed end to end by
 * Meshtrike; it never sees names, keys or plaintext.
 */
public final class NetService extends Service {
    static final String DESCRIPTOR = "org.meshtrike.internet.INet/1";
    static final int OPEN = IBinder.FIRST_CALL_TRANSACTION;
    static final String MAIN_APP = "org.meshtrike.app";

    static final byte CONFIG = 0x01;
    static final byte SEND = 0x02;
    static final byte READY = (byte) 0x81;
    static final byte DOWN = (byte) 0x82;
    static final byte ENVELOPE = (byte) 0x83;
    static final byte MODE = (byte) 0x84;

    private static final int MAX_FRAME = 70_000;

    private Route route;
    private volatile Session session;

    @Override
    public void onCreate() {
        super.onCreate();
        route = new Route(this, new Route.Listener() {
            @Override
            public void ready(String address) {
                Session s = session;
                if (s != null) s.write(READY, address.getBytes(StandardCharsets.UTF_8));
            }

            @Override
            public void down(String reason) {
                Session s = session;
                if (s != null) s.write(DOWN, reason.getBytes(StandardCharsets.UTF_8));
            }

            @Override
            public void envelope(byte[] env) {
                Session s = session;
                if (s != null) s.write(ENVELOPE, env);
            }

            @Override
            public void mode(String mode) {
                Session s = session;
                if (s != null) s.write(MODE, mode.getBytes(StandardCharsets.UTF_8));
            }
        });
    }

    @Override
    public void onDestroy() {
        Session s = session;
        if (s != null) s.close();
        route.shutdown();
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
            if (code != OPEN) return super.onTransact(code, data, reply, flags);
            data.enforceInterface(DESCRIPTOR);
            if (!callerIsMeshtrike(Binder.getCallingUid())) throw new SecurityException("not Meshtrike");
            ParcelFileDescriptor pfd = open();
            reply.writeNoException();
            if (pfd == null) {
                reply.writeInt(0);
            } else {
                reply.writeInt(1);
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

    /** A new pipe replaces the old one (Meshtrike restarted or rebound). */
    private synchronized ParcelFileDescriptor open() {
        ParcelFileDescriptor[] pair;
        try {
            pair = ParcelFileDescriptor.createSocketPair();
        } catch (IOException e) {
            return null;
        }
        Session old = session;
        if (old != null) old.close();
        Session s = new Session(pair[0]);
        session = s;
        new Thread(s::readLoop, "net-session").start();
        // A late joiner learns the current state straight away.
        String addr = route.address();
        if (addr != null) {
            s.write(READY, addr.getBytes(StandardCharsets.UTF_8));
            String m = route.mode();
            if (m != null) s.write(MODE, m.getBytes(StandardCharsets.UTF_8));
        }
        return pair[1];
    }

    private final class Session {
        final ParcelFileDescriptor pfd;
        final DataOutputStream out;
        volatile boolean closed;

        Session(ParcelFileDescriptor pfd) {
            this.pfd = pfd;
            this.out = new DataOutputStream(new FileOutputStream(pfd.getFileDescriptor()));
        }

        synchronized void write(byte type, byte[] payload) {
            if (closed) return;
            try {
                out.writeInt(payload.length + 1);
                out.writeByte(type);
                out.write(payload);
                out.flush();
            } catch (IOException e) {
                close();
            }
        }

        void readLoop() {
            try (DataInputStream in = new DataInputStream(new FileInputStream(pfd.getFileDescriptor()))) {
                while (!closed) {
                    int n = in.readInt();
                    if (n < 1 || n > MAX_FRAME) break;
                    byte type = in.readByte();
                    byte[] p = new byte[n - 1];
                    in.readFully(p);
                    if (type == CONFIG) {
                        route.apply(parseConfig(p));
                    } else if (type == SEND && p.length > 2) {
                        int al = ((p[0] & 0xff) << 8) | (p[1] & 0xff);
                        if (al <= 0 || 2 + al > p.length) continue;
                        String addr = new String(p, 2, al, StandardCharsets.UTF_8);
                        byte[] env = new byte[p.length - 2 - al];
                        System.arraycopy(p, 2 + al, env, 0, env.length);
                        route.send(addr, env);
                    }
                }
            } catch (IOException ignored) {
            } finally {
                close();
            }
        }

        void close() {
            if (closed) return;
            closed = true;
            try {
                android.system.Os.shutdown(pfd.getFileDescriptor(), android.system.OsConstants.SHUT_RDWR);
            } catch (Exception ignored) {
            }
            try {
                pfd.close();
            } catch (IOException ignored) {
            }
            if (session == this) session = null;
        }
    }

    static Route.Config parseConfig(byte[] json) {
        Route.Config c = new Route.Config();
        try {
            JSONObject o = new JSONObject(new String(json, StandardCharsets.UTF_8));
            c.enabled = o.optBoolean("enabled", false);
            c.allowDirect = o.optBoolean("allow_direct", false);
            JSONArray b = o.optJSONArray("bridges");
            List<String> links = new ArrayList<>();
            if (b != null) {
                for (int i = 0; i < b.length() && i < 16; i++) {
                    String l = b.optString(i, "");
                    if (l.startsWith("vless://") && l.length() <= 768) links.add(l);
                }
            }
            c.bridges = links;
        } catch (Exception e) {
            c.enabled = false;
        }
        return c;
    }
}
