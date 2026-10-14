package org.meshtrike.internet;

/** The Rust side (native/): a Nym mixnet client that carries sealed envelopes. */
final class Native {
    static final int READY = 1;
    static final int DOWN = 2;
    static final int ENVELOPE = 3;
    static final int GATEWAY = 4;

    static {
        System.loadLibrary("meshtrike_internet");
    }

    interface Listener {
        void onEvent(int kind, byte[] data);
    }

    static volatile Listener listener;

    private Native() {}

    /**
     * Start (or restart) the client with keys in {@code dir}. A non-null
     * {@code proxyHost} forces every connection of this process through that
     * SOCKS5 endpoint; it must not change for the life of the process.
     */
    static native boolean start(String dir, String proxyHost, int proxyPort, String user, String pass);

    static native void stop();

    static native boolean send(String address, byte[] envelope);

    /** Called from Rust threads. */
    static void onEvent(int kind, byte[] data) {
        Listener l = listener;
        if (l != null) l.onEvent(kind, data);
    }
}
