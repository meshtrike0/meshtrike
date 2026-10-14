package org.meshtrike.wifilink;

import android.Manifest;
import android.animation.ValueAnimator;
import android.app.Activity;
import android.app.AlertDialog;
import android.content.Intent;
import android.content.pm.PackageInfo;
import android.content.pm.PackageManager;
import android.graphics.Insets;
import android.graphics.drawable.GradientDrawable;
import android.net.Uri;
import android.os.Build;
import android.os.Bundle;
import android.os.Handler;
import android.os.Looper;
import android.os.PowerManager;
import android.provider.Settings;
import android.text.TextUtils;
import android.view.Gravity;
import android.view.View;
import android.view.WindowInsets;
import android.view.WindowInsetsController;
import android.view.WindowManager;
import android.view.animation.AccelerateDecelerateInterpolator;
import android.view.animation.OvershootInterpolator;
import android.widget.FrameLayout;
import android.widget.ImageView;
import android.widget.LinearLayout;
import android.widget.ScrollView;
import android.widget.TextView;
import android.window.OnBackInvokedCallback;
import android.window.OnBackInvokedDispatcher;

import org.meshtrike.internet.RouteStatus;
import org.meshtrike.link.R;
import org.meshtrike.link.Reach;

import java.util.Locale;

import static org.meshtrike.wifilink.Ui.*;

/**
 * MeshGRID's only screen: first-run setup, the home screen with the
 * state of Wi-Fi Direct and the internet route, and settings. Meshtrike opens
 * it when Wi-Fi Direct is not yet allowed. Everything shown is read from the
 * services in this process; nothing is invented. Its switches can only hold
 * the services back (see Reach). Uninstalling the add-on turns both off.
 */
public final class InfoActivity extends Activity {
    private static final int REQ_WIFI = 1;
    private static final long REFRESH_MS = 1_000;

    private Ui ui;
    private final Handler h = new Handler(Looper.getMainLooper());
    private FrameLayout screenHost;
    private TextView toast;
    private String screen; // "setup", "home", "settings"
    private String phase = "intro"; // setup: "intro", "linking", "done"
    private boolean setupDenied;
    private Object backCallback;

    // Home views, updated in place by refresh().
    private LinearLayout statusCard;
    private LinearLayout eyebrowRow;
    private int statusDotKey = -1;
    private int wifiCardKey = -1;
    private TextView statusEyebrow, statusTitle, statusBody, pauseLabel, openLabel;
    private ImageView pauseIcon;
    private LinearLayout pauseBtn, openBtn;
    private LinearLayout wifiCard, inetCard;
    private TextView wifiLabel, wifiDetail, inetLabel, inetDesc, bridgesHeader;
    private LinearLayout wifiBusy;
    private View inetSpinner;
    private Toggle wifiToggle, inetToggle;
    private LinearLayout bridgeList;
    private String bridgeKey = "";

    // Settings views.
    private Toggle chargingToggle;
    private TextView permLabel, batteryLabel;

    private final Runnable tick = new Runnable() {
        @Override
        public void run() {
            refresh();
            h.postDelayed(this, REFRESH_MS);
        }
    };

    @Override
    protected void onCreate(Bundle b) {
        super.onCreate(b);
        getWindow().setFlags(WindowManager.LayoutParams.FLAG_SECURE, WindowManager.LayoutParams.FLAG_SECURE);
        ui = new Ui(this);

        FrameLayout root = new FrameLayout(this);
        root.setBackgroundColor(PAPER);
        screenHost = new FrameLayout(this);
        root.addView(screenHost, new FrameLayout.LayoutParams(MATCH, MATCH));
        toast = ui.text("", 13.5f, 500, 0xFFFFFFFF);
        toast.setBackground(ui.round(INK, 18));
        toast.setPadding(ui.dp(16), ui.dp(11), ui.dp(16), ui.dp(11));
        toast.setGravity(Gravity.CENTER);
        toast.setLineSpacing(0, 1.2f);
        toast.setMaxWidth(ui.dp(320));
        toast.setAlpha(0);
        FrameLayout.LayoutParams tl = new FrameLayout.LayoutParams(WRAP, WRAP, Gravity.BOTTOM | Gravity.CENTER_HORIZONTAL);
        tl.bottomMargin = ui.dp(44);
        root.addView(toast, tl);
        setContentView(root);
        edgeToEdge(root);

        screen = b != null ? b.getString("screen") : null;
        if (screen == null) screen = Reach.get(this, Reach.SETUP_DONE) ? "home" : "setup";
        if ("setup".equals(screen) && b != null) phase = b.getString("phase", "intro");
        show(screen);

        // As before: opened by Meshtrike because Wi-Fi Direct is not allowed yet, ask straight away.
        if (b == null && !"setup".equals(screen) && needsWifiPermission()) requestWifiPermission();
    }

    @Override
    protected void onSaveInstanceState(Bundle out) {
        super.onSaveInstanceState(out);
        out.putString("screen", screen);
        out.putString("phase", phase);
    }

    @Override
    protected void onResume() {
        super.onResume();
        h.removeCallbacks(tick);
        h.post(tick);
    }

    @Override
    protected void onPause() {
        h.removeCallbacks(tick);
        super.onPause();
    }

    @Override
    public void onRequestPermissionsResult(int code, String[] perms, int[] results) {
        if (code != REQ_WIFI) return;
        boolean granted = results.length > 0 && results[0] == PackageManager.PERMISSION_GRANTED;
        if ("setup".equals(screen) && "linking".equals(phase)) {
            setupDenied = !granted;
            phase = "done";
            show("setup");
            return;
        }
        if (!granted && perms.length > 0 && !shouldShowRequestPermissionRationale(perms[0])) {
            // Android no longer shows the dialog: the switch is in App info.
            showToast(Build.VERSION.SDK_INT >= 33
                ? "Allow “Nearby devices” in App info, Permissions"
                : "Allow “Location” in App info, Permissions");
            openAppInfo();
        }
        refresh();
    }

    @SuppressWarnings("deprecation")
    @Override
    public void onBackPressed() {
        if ("settings".equals(screen)) show("home");
        else super.onBackPressed();
    }

    // ===== Window =====

    @SuppressWarnings("deprecation")
    private void edgeToEdge(View root) {
        getWindow().setStatusBarColor(PAPER);
        getWindow().setNavigationBarColor(PAPER);
        if (Build.VERSION.SDK_INT >= 30) {
            getWindow().setDecorFitsSystemWindows(false);
            WindowInsetsController c = getWindow().getInsetsController();
            if (c != null) {
                int light = WindowInsetsController.APPEARANCE_LIGHT_STATUS_BARS | WindowInsetsController.APPEARANCE_LIGHT_NAVIGATION_BARS;
                c.setSystemBarsAppearance(light, light);
            }
            root.setOnApplyWindowInsetsListener((v, ins) -> {
                Insets i = ins.getInsets(WindowInsets.Type.systemBars() | WindowInsets.Type.displayCutout());
                v.setPadding(i.left, i.top, i.right, i.bottom);
                return WindowInsets.CONSUMED;
            });
        } else {
            getWindow().getDecorView().setSystemUiVisibility(
                View.SYSTEM_UI_FLAG_LIGHT_STATUS_BAR | View.SYSTEM_UI_FLAG_LIGHT_NAVIGATION_BAR);
        }
    }

    private void show(String s) {
        screen = s;
        screenHost.removeAllViews();
        clearHomeRefs();
        View v = "setup".equals(s) ? buildSetup() : "settings".equals(s) ? buildSettings() : buildHome();
        screenHost.addView(v, new FrameLayout.LayoutParams(MATCH, MATCH));
        v.setAlpha(0);
        v.setTranslationX("settings".equals(s) ? ui.dp(18) : 0);
        v.setTranslationY("settings".equals(s) ? 0 : ui.dp(10));
        v.animate().alpha(1).translationX(0).translationY(0).setDuration(300)
            .setInterpolator(new AccelerateDecelerateInterpolator()).start();
        updateBack();
        refresh();
    }

    /** Back on Settings returns home (Android 13+ predictive back needs a callback). */
    private void updateBack() {
        if (Build.VERSION.SDK_INT < 33) return;
        OnBackInvokedDispatcher d = getOnBackInvokedDispatcher();
        if (backCallback != null) {
            d.unregisterOnBackInvokedCallback((OnBackInvokedCallback) backCallback);
            backCallback = null;
        }
        if ("settings".equals(screen)) {
            OnBackInvokedCallback cb = () -> show("home");
            d.registerOnBackInvokedCallback(OnBackInvokedDispatcher.PRIORITY_DEFAULT, cb);
            backCallback = cb;
        }
    }

    private void clearHomeRefs() {
        statusCard = null;
        wifiCard = null;
        inetCard = null;
        bridgeList = null;
        bridgeKey = "";
        statusDotKey = -1;
        wifiCardKey = -1;
        chargingToggle = null;
        permLabel = null;
        batteryLabel = null;
    }

    // ===== Facts =====

    private boolean meshtrikeLinked() {
        return getPackageManager().checkSignatures(LinkService.MAIN_APP, getPackageName()) == PackageManager.SIGNATURE_MATCH;
    }

    private String meshtrikeVersion() {
        try {
            PackageInfo p = getPackageManager().getPackageInfo(LinkService.MAIN_APP, 0);
            return p.versionName;
        } catch (PackageManager.NameNotFoundException e) {
            return null;
        }
    }

    private long ownVersionCode() {
        try {
            return getPackageManager().getPackageInfo(getPackageName(), 0).getLongVersionCode();
        } catch (PackageManager.NameNotFoundException e) {
            return 0;
        }
    }

    private String ownVersionName() {
        try {
            return getPackageManager().getPackageInfo(getPackageName(), 0).versionName;
        } catch (PackageManager.NameNotFoundException e) {
            return "";
        }
    }

    private static String wifiPermission() {
        return Build.VERSION.SDK_INT >= 33 ? Manifest.permission.NEARBY_WIFI_DEVICES : Manifest.permission.ACCESS_FINE_LOCATION;
    }

    private boolean needsWifiPermission() {
        int st = LinkService.statusOf(this);
        return (st & LinkService.HAS_HARDWARE) != 0
            && checkSelfPermission(wifiPermission()) != PackageManager.PERMISSION_GRANTED;
    }

    private void requestWifiPermission() {
        requestPermissions(new String[] {wifiPermission()}, REQ_WIFI);
    }

    private boolean paused() {
        return Reach.get(this, Reach.PAUSED);
    }

    // ===== Actions =====

    private void openMeshtrike() {
        Intent i = getPackageManager().getLaunchIntentForPackage(LinkService.MAIN_APP);
        if (i == null) {
            showToast("Meshtrike isn’t installed on this phone");
            return;
        }
        startActivity(i);
    }

    private void openAppInfo() {
        try {
            startActivity(new Intent(Settings.ACTION_APPLICATION_DETAILS_SETTINGS, Uri.fromParts("package", getPackageName(), null)));
        } catch (RuntimeException e) {
            showToast("Open Android Settings, Apps, MeshGRID");
        }
    }

    private void openBatterySettings() {
        try {
            startActivity(new Intent(Settings.ACTION_IGNORE_BATTERY_OPTIMIZATION_SETTINGS));
        } catch (RuntimeException e) {
            showToast("Open Android Settings, Battery");
        }
    }

    /** Wi-Fi Direct needs fixing before it can work: ask, or say where to go. */
    private void fixWifi() {
        int st = LinkService.statusOf(this);
        if ((st & LinkService.HAS_HARDWARE) == 0) {
            showToast("This phone has no Wi-Fi Direct");
        } else if (checkSelfPermission(wifiPermission()) != PackageManager.PERMISSION_GRANTED) {
            requestWifiPermission();
        } else if ((st & LinkService.PERMITTED) == 0) {
            try {
                startActivity(new Intent(Settings.ACTION_LOCATION_SOURCE_SETTINGS));
            } catch (RuntimeException e) {
                showToast("Turn on location in Android Settings");
            }
        } else if ((st & LinkService.WIFI_ON) == 0) {
            try {
                startActivity(new Intent(Settings.Panel.ACTION_WIFI));
            } catch (RuntimeException e) {
                showToast("Turn on Wi-Fi. No Wi-Fi network is needed.");
            }
        }
    }

    private void togglePause() {
        boolean p = !paused();
        Reach.set(this, Reach.PAUSED, p);
        if (p) LinkService.stopNow();
        RouteStatus.reapply();
        showToast(p ? "MeshGRID paused · Meshtrike falls back to Bluetooth, LoRa and the mesh" : "MeshGRID resumed");
        refresh();
    }

    private void showToast(String s) {
        toast.setText(s);
        toast.animate().cancel();
        toast.setTranslationY(ui.dp(24));
        toast.animate().alpha(1).translationY(0).setDuration(300).setInterpolator(new OvershootInterpolator(1.2f)).start();
        h.removeCallbacks(hideToast);
        h.postDelayed(hideToast, 2_400);
    }

    private final Runnable hideToast = () -> toast.animate().alpha(0).translationY(ui.dp(24)).setDuration(300).start();

    // ===== Setup =====

    private View buildSetup() {
        LinearLayout page = ui.col();
        page.setPadding(ui.dp(24), ui.dp(16), ui.dp(24), ui.dp(34));

        LinearLayout brand = ui.row();
        brand.addView(ui.tile(R.drawable.ic_link, VIOLET_SOFT, VIOLET, 32, 11, 17));
        TextView name = ui.text("MeshGRID", 17, 700, INK);
        LinearLayout.LayoutParams nl = ui.lp(WRAP, WRAP);
        nl.leftMargin = ui.dp(10);
        brand.addView(name, nl);
        page.addView(brand);

        LinearLayout middle = ui.col();
        middle.setGravity(Gravity.CENTER_VERTICAL);
        ui.gap(middle, 26);
        middle.addView(setupArt(), ui.lp(MATCH, ui.dp(96)));

        boolean linked = meshtrikeLinked();
        String title, body;
        if ("intro".equals(phase)) {
            title = "Give Meshtrike more reach";
            body = "Meshtrike itself never connects to the internet. MeshGRID is a separate, optional app that adds two things when you allow them.";
        } else if ("linking".equals(phase)) {
            title = "Linking…";
            body = "Android asks you to allow Wi-Fi Direct for MeshGRID. Nothing on this phone but Meshtrike, signed with the same key, can use MeshGRID.";
        } else if (!linked) {
            title = "Almost there";
            body = "Install Meshtrike from the same place you got MeshGRID. Only Meshtrike signed with the same key can use it.";
        } else {
            title = "You’re linked";
            body = "MeshGRID stays out of the way. Meshtrike uses it only when a route needs it."
                + (setupDenied ? " Wi-Fi Direct stays off until you allow it; tap Wi-Fi Direct on the next screen." : "");
        }
        LinearLayout words = ui.col();
        ui.gap(words, 10);
        TextView t = ui.display(title, 32);
        t.setLetterSpacing(-0.03f);
        t.setLineSpacing(0, 1.05f);
        words.addView(t);
        TextView bd = ui.text(body, 15.5f, 400, BODY);
        bd.setLineSpacing(0, 1.4f);
        words.addView(bd);
        middle.addView(words);

        if ("intro".equals(phase)) {
            LinearLayout card = ui.col();
            card.setBackground(ui.round(CARD, 22));
            card.setPadding(ui.dp(16), ui.dp(4), ui.dp(16), ui.dp(4));
            card.addView(featureRow(R.drawable.ic_wifi, TEAL_SOFT, TEAL, "Wi-Fi Direct",
                "Photos and files move much faster between nearby phones than over Bluetooth.", true));
            card.addView(featureRow(R.drawable.ic_globe, VIOLET_SOFT, VIOLET, "Private internet route",
                "Reach far-away contacts through the Nym mixnet, which hides who talks to whom.", false));
            middle.addView(card);
        }
        page.addView(middle, new LinearLayout.LayoutParams(MATCH, 0, 1f));

        if ("linking".equals(phase)) {
            LinearLayout wait = ui.row();
            wait.setGravity(Gravity.CENTER);
            wait.addView(ui.spinner(INK, 18, 2.5f));
            TextView w = ui.text("Confirm on screen…", 14.5f, 500, MUTED);
            LinearLayout.LayoutParams wl = ui.lp(WRAP, WRAP);
            wl.leftMargin = ui.dp(10);
            wait.addView(w, wl);
            page.addView(wait, ui.lp(MATCH, ui.dp(58)));
        } else {
            TextView btn = bigButton("intro".equals(phase) ? "Link with Meshtrike" : "Done");
            btn.setOnClickListener(v -> {
                if ("intro".equals(phase)) {
                    if (needsWifiPermission()) {
                        phase = "linking";
                        show("setup");
                        requestWifiPermission();
                    } else {
                        phase = "done";
                        show("setup");
                    }
                } else {
                    Reach.set(this, Reach.SETUP_DONE, true);
                    phase = "intro";
                    show("home");
                }
            });
            page.addView(btn, ui.lp(MATCH, ui.dp(58)));
        }
        return page;
    }

    private TextView bigButton(String label) {
        TextView b = ui.text(label, 16, 600, 0xFFFFFFFF);
        b.setGravity(Gravity.CENTER);
        b.setBackground(Ui.pressable(ui.round(INK, 99), 0x33FFFFFF));
        b.setClickable(true);
        return b;
    }

    private View featureRow(int icon, int bg, int fg, String title, String body, boolean divider) {
        LinearLayout r = ui.row();
        r.setGravity(Gravity.TOP);
        r.setPadding(0, ui.dp(13), 0, ui.dp(13));
        r.addView(ui.tile(icon, bg, fg, 36, 12, 18));
        LinearLayout c = ui.col();
        ui.gap(c, 3);
        c.addView(ui.text(title, 14.5f, 600, INK));
        TextView d = ui.text(body, 12.5f, 400, MUTED);
        d.setLineSpacing(0, 1.3f);
        c.addView(d);
        LinearLayout.LayoutParams cl = ui.weight();
        cl.leftMargin = ui.dp(12);
        r.addView(c, cl);
        if (!divider) return r;
        LinearLayout wrap = ui.col();
        wrap.addView(r);
        View line = new View(this);
        line.setBackgroundColor(DIVIDER);
        wrap.addView(line, ui.lp(MATCH, ui.dp(1)));
        return wrap;
    }

    /** Meshtrike's tile, a dashed line, Reach's tile; a dot travels while linking, a check when done. */
    private View setupArt() {
        FrameLayout f = new FrameLayout(this);
        f.setPadding(ui.dp(18), 0, ui.dp(18), 0);
        FrameLayout left = ui.tile(R.drawable.ic_mesh, INK, LIME, 72, 24, 30);
        FrameLayout right = ui.tile(R.drawable.ic_link, VIOLET_SOFT, VIOLET, 72, 24, 30);
        Dashes dashes = new Dashes(this);
        FrameLayout.LayoutParams dl = new FrameLayout.LayoutParams(MATCH, ui.dp(12), Gravity.CENTER_VERTICAL);
        dl.leftMargin = ui.dp(78);
        dl.rightMargin = ui.dp(78);
        f.addView(dashes, dl);
        f.addView(left, new FrameLayout.LayoutParams(ui.dp(72), ui.dp(72), Gravity.CENTER_VERTICAL | Gravity.START));
        f.addView(right, new FrameLayout.LayoutParams(ui.dp(72), ui.dp(72), Gravity.CENTER_VERTICAL | Gravity.END));
        if ("linking".equals(phase)) {
            FrameLayout lane = new FrameLayout(this);
            View dot = new View(this);
            GradientDrawable g = new GradientDrawable();
            g.setShape(GradientDrawable.OVAL);
            g.setColor(LIME);
            g.setStroke(ui.dp(4), 0x59CDF25C);
            dot.setBackground(g);
            lane.addView(dot, new FrameLayout.LayoutParams(ui.dp(20), ui.dp(20), Gravity.CENTER_VERTICAL));
            FrameLayout.LayoutParams ll = new FrameLayout.LayoutParams(MATCH, ui.dp(20), Gravity.CENTER_VERTICAL);
            ll.leftMargin = ui.dp(74);
            ll.rightMargin = ui.dp(74);
            f.addView(lane, ll);
            ValueAnimator a = ValueAnimator.ofFloat(0, 1);
            a.setDuration(1000);
            a.setRepeatCount(ValueAnimator.INFINITE);
            a.setRepeatMode(ValueAnimator.REVERSE);
            a.setInterpolator(new AccelerateDecelerateInterpolator());
            a.addUpdateListener(x -> dot.setTranslationX((lane.getWidth() - dot.getWidth()) * (float) x.getAnimatedValue()));
            lane.addOnAttachStateChangeListener(new View.OnAttachStateChangeListener() {
                @Override
                public void onViewAttachedToWindow(View v) {
                    a.start();
                }

                @Override
                public void onViewDetachedFromWindow(View v) {
                    a.cancel();
                }
            });
        } else if ("done".equals(phase)) {
            FrameLayout check = new FrameLayout(this);
            GradientDrawable g = new GradientDrawable();
            g.setShape(GradientDrawable.OVAL);
            g.setColor(meshtrikeLinked() ? LIME : PAUSED_BG);
            check.setBackground(g);
            check.addView(ui.icon(meshtrikeLinked() ? R.drawable.ic_check : R.drawable.ic_unlink, INK, 18),
                new FrameLayout.LayoutParams(ui.dp(18), ui.dp(18), Gravity.CENTER));
            f.addView(check, new FrameLayout.LayoutParams(ui.dp(36), ui.dp(36), Gravity.CENTER));
            check.setScaleX(0.6f);
            check.setScaleY(0.6f);
            check.setAlpha(0);
            check.animate().scaleX(1).scaleY(1).alpha(1).setDuration(400).setInterpolator(new OvershootInterpolator(2f)).start();
        }
        return f;
    }

    // ===== Home =====

    private View buildHome() {
        LinearLayout page = ui.col();

        LinearLayout head = ui.row();
        head.setPadding(ui.dp(20), ui.dp(6), ui.dp(16), ui.dp(10));
        head.addView(ui.tile(R.drawable.ic_link, VIOLET_SOFT, VIOLET, 36, 12, 19));
        LinearLayout names = ui.col();
        ui.gap(names, 3);
        names.addView(ui.display("MeshGRID", 20));
        TextView ver = ui.text("v" + ownVersionCode() + " · add-on", 11.5f, 500, FAINT);
        ver.setTypeface(Ui.mono(500));
        names.addView(ver);
        LinearLayout.LayoutParams nl = ui.weight();
        nl.leftMargin = ui.dp(10);
        head.addView(names, nl);
        FrameLayout gear = ui.tile(R.drawable.ic_sliders, CARD, INK, 42, 14, 19);
        gear.setBackground(ui.pressable(ui.round(CARD, 14)));
        gear.setContentDescription("Settings");
        gear.setOnClickListener(v -> show("settings"));
        head.addView(gear);
        page.addView(head);

        LinearLayout list = ui.col();
        list.setPadding(ui.dp(16), ui.dp(4), ui.dp(16), ui.dp(30));
        ui.gap(list, 12);

        // Status card.
        statusCard = ui.col();
        statusCard.setPadding(ui.dp(20), ui.dp(20), ui.dp(20), ui.dp(20));
        ui.gap(statusCard, 14);
        eyebrowRow = ui.row();
        statusEyebrow = ui.text("", 12, 600, INK);
        statusEyebrow.setTypeface(Ui.mono(600));
        statusEyebrow.setLetterSpacing(0.06f);
        statusEyebrow.setAlpha(0.8f);
        eyebrowRow.addView(statusEyebrow);
        statusCard.addView(eyebrowRow);
        statusTitle = ui.display("", 26);
        statusTitle.setLineSpacing(0, 1.1f);
        statusCard.addView(statusTitle);
        statusBody = ui.text("", 13.5f, 400, INK);
        statusBody.setLineSpacing(0, 1.35f);
        statusBody.setAlpha(0.78f);
        statusCard.addView(statusBody);
        LinearLayout buttons = ui.row();
        pauseBtn = ui.row();
        pauseBtn.setPadding(ui.dp(16), 0, ui.dp(16), 0);
        pauseIcon = ui.icon(R.drawable.ic_pause, INK, 16);
        pauseBtn.addView(pauseIcon);
        pauseLabel = ui.text("", 14, 600, INK);
        LinearLayout.LayoutParams pl = ui.lp(WRAP, WRAP);
        pl.leftMargin = ui.dp(7);
        pauseBtn.addView(pauseLabel, pl);
        pauseBtn.setClickable(true);
        pauseBtn.setOnClickListener(v -> togglePause());
        buttons.addView(pauseBtn, ui.lp(WRAP, ui.dp(42)));
        openBtn = ui.row();
        openBtn.setPadding(ui.dp(16), 0, ui.dp(16), 0);
        openLabel = ui.text("Open Meshtrike", 14, 600, INK);
        openBtn.addView(openLabel);
        openBtn.setClickable(true);
        openBtn.setOnClickListener(v -> openMeshtrike());
        LinearLayout.LayoutParams ol = ui.lp(WRAP, ui.dp(42));
        ol.leftMargin = ui.dp(8);
        buttons.addView(openBtn, ol);
        statusCard.addView(buttons);
        list.addView(statusCard);

        // Wi-Fi Direct.
        wifiCard = serviceCard();
        LinearLayout wifiHead = ui.row();
        wifiHead.addView(ui.tile(R.drawable.ic_wifi, TEAL_SOFT, TEAL, 42, 14, 20));
        LinearLayout wt = ui.col();
        ui.gap(wt, 3);
        wt.addView(ui.text("Wi-Fi Direct", 15, 600, INK));
        wifiLabel = ui.text("", 12.5f, 500, MUTED);
        wt.addView(wifiLabel);
        LinearLayout.LayoutParams wtl = ui.weight();
        wtl.leftMargin = ui.dp(12);
        wtl.rightMargin = ui.dp(8);
        wifiHead.addView(wt, wtl);
        wifiToggle = new Toggle(this, "Wi-Fi Direct");
        wifiToggle.set(Reach.get(this, Reach.WIFI), false);
        wifiToggle.setOnClickListener(v -> {
            if (paused()) {
                showToast("Resume MeshGRID first");
                return;
            }
            boolean on = !Reach.get(this, Reach.WIFI);
            Reach.set(this, Reach.WIFI, on);
            if (!on) LinkService.stopNow();
            wifiToggle.set(on, true);
            refresh();
        });
        wifiHead.addView(wifiToggle);
        wifiCard.addView(wifiHead);
        wifiCard.setOnClickListener(v -> fixWifi());
        wifiBusy = ui.col();
        ui.gap(wifiBusy, 6);
        wifiBusy.addView(new ActivityBar(this, TEAL), ui.lp(MATCH, WRAP));
        wifiDetail = ui.text("", 12, 500, MUTED);
        wifiDetail.setFontFeatureSettings("tnum");
        wifiBusy.addView(wifiDetail);
        wifiCard.addView(wifiBusy);
        list.addView(wifiCard);

        // Internet route.
        inetCard = serviceCard();
        LinearLayout inetHead = ui.row();
        inetHead.addView(ui.tile(R.drawable.ic_globe, VIOLET_SOFT, VIOLET, 42, 14, 20));
        LinearLayout it = ui.col();
        ui.gap(it, 3);
        it.addView(ui.text("Internet route", 15, 600, INK));
        LinearLayout labelRow = ui.row();
        inetSpinner = ui.spinner(0xFF4E525B, 11, 2);
        LinearLayout.LayoutParams sl = ui.lp(ui.dp(11), ui.dp(11));
        sl.rightMargin = ui.dp(6);
        labelRow.addView(inetSpinner, sl);
        inetLabel = ui.text("", 12.5f, 500, MUTED);
        labelRow.addView(inetLabel);
        it.addView(labelRow);
        LinearLayout.LayoutParams itl = ui.weight();
        itl.leftMargin = ui.dp(12);
        itl.rightMargin = ui.dp(8);
        inetHead.addView(it, itl);
        inetToggle = new Toggle(this, "Internet route");
        inetToggle.set(Reach.get(this, Reach.INET), false);
        inetToggle.setOnClickListener(v -> {
            if (paused()) {
                showToast("Resume MeshGRID first");
                return;
            }
            boolean on = !Reach.get(this, Reach.INET);
            Reach.set(this, Reach.INET, on);
            RouteStatus.reapply();
            inetToggle.set(on, true);
            refresh();
        });
        inetHead.addView(inetToggle);
        inetCard.addView(inetHead);
        inetDesc = ui.text("", 12.5f, 400, MUTED);
        inetDesc.setLineSpacing(0, 1.35f);
        inetCard.addView(inetDesc);
        list.addView(inetCard);

        // Bridges.
        LinearLayout bh = ui.row();
        bh.setPadding(ui.dp(6), ui.dp(8), ui.dp(6), 0);
        bridgesHeader = ui.eyebrow("BRIDGES · 0", FAINT);
        bh.addView(bridgesHeader, ui.weight());
        TextView add = ui.text("Add · coming soon", 12.5f, 600, FAINT);
        add.setPadding(ui.dp(4), ui.dp(4), ui.dp(4), ui.dp(4));
        add.setOnClickListener(v -> showToast("Adding bridges here is coming soon. Add them in Meshtrike: Settings, Internet route."));
        bh.addView(add);
        list.addView(bh);
        bridgeList = ui.col();
        ui.gap(bridgeList, 8);
        list.addView(bridgeList);

        // Footnote.
        LinearLayout foot = ui.row();
        foot.setGravity(Gravity.TOP);
        foot.setPadding(ui.dp(6), ui.dp(10), ui.dp(6), 0);
        ImageView lock = ui.icon(R.drawable.ic_lock, MUTED, 15);
        LinearLayout.LayoutParams lockl = ui.lp(ui.dp(15), ui.dp(15));
        lockl.topMargin = ui.dp(2);
        foot.addView(lock, lockl);
        TextView ft = ui.text("MeshGRID never sees what you write. Meshtrike seals every message before handing it over.", 12.5f, 400, MUTED);
        ft.setLineSpacing(0, 1.4f);
        LinearLayout.LayoutParams ftl = ui.weight();
        ftl.leftMargin = ui.dp(10);
        foot.addView(ft, ftl);
        list.addView(foot);

        ScrollView sv = new ScrollView(this);
        sv.setClipToPadding(false);
        sv.addView(list);
        page.addView(sv, new LinearLayout.LayoutParams(MATCH, 0, 1f));
        return page;
    }

    private LinearLayout serviceCard() {
        LinearLayout c = ui.col();
        c.setBackground(ui.round(CARD, 24));
        c.setPadding(ui.dp(16), ui.dp(16), ui.dp(16), ui.dp(16));
        ui.gap(c, 12);
        return c;
    }

    private void refresh() {
        if (statusCard != null) refreshHome();
        if (chargingToggle != null) refreshSettings();
    }

    private void refreshHome() {
        boolean p = paused();
        boolean linked = meshtrikeLinked();
        int st = LinkService.statusOf(this);
        int wifiPhase = LinkService.phase;
        RouteStatus rs = RouteStatus.now();
        boolean active = wifiPhase != LinkService.IDLE || rs.state == RouteStatus.CONNECTED || rs.state == RouteStatus.CONNECTING;

        // Status card.
        int bg, fg, dot, key;
        boolean live = false;
        if (p) {
            bg = PAUSED_BG;
            fg = INK;
            dot = PAUSED_DOT;
            statusEyebrow.setText("PAUSED");
            statusTitle.setText("MeshGRID is paused");
            statusBody.setText("Meshtrike keeps working over Bluetooth, LoRa and the mesh. Wi-Fi Direct and the internet route are off until you resume.");
            key = 0;
        } else if (!linked) {
            bg = PAUSED_BG;
            fg = INK;
            dot = WARN_LINE;
            statusEyebrow.setText("NOT LINKED");
            statusTitle.setText("Meshtrike isn’t installed");
            statusBody.setText("MeshGRID only works for Meshtrike signed with the same key. Install Meshtrike from the same place you got MeshGRID.");
            key = 1;
        } else {
            bg = INK;
            fg = 0xFFFFFFFF;
            dot = LIVE;
            live = active;
            statusEyebrow.setText(active ? "RUNNING" : "READY");
            statusTitle.setText("Linked to Meshtrike");
            statusBody.setText(active
                ? "Meshtrike is using MeshGRID right now. It asks for Wi-Fi Direct and the internet route only when it needs them."
                : "Waiting in the background. Meshtrike asks MeshGRID for Wi-Fi Direct and the internet route only when it needs them.");
            key = active ? 3 : 2;
        }
        if (key != statusDotKey) {
            statusDotKey = key;
            statusCard.setBackground(ui.round(bg, 28));
            statusEyebrow.setTextColor(fg);
            statusTitle.setTextColor(fg);
            statusBody.setTextColor(fg);
            if (eyebrowRow.getChildCount() > 1) eyebrowRow.removeViewAt(0);
            eyebrowRow.addView(ui.liveDot(dot, live), 0);
            pauseBtn.setBackground(Ui.pressable(ui.round(p ? INK : (linked ? LIME : INK), 99), 0x33FFFFFF));
            int pfg = p || !linked ? 0xFFFFFFFF : INK;
            pauseLabel.setText(p ? "Resume" : "Pause");
            pauseLabel.setTextColor(pfg);
            pauseIcon.setImageDrawable(tinted(p ? R.drawable.ic_play : R.drawable.ic_pause, pfg));
            openBtn.setBackground(ui.pressable(ui.round(bg == INK ? 0x1FFFFFFF : 0x1415171C, 99)));
            openLabel.setTextColor(fg);
        }
        boolean canOpen = getPackageManager().getLaunchIntentForPackage(LinkService.MAIN_APP) != null;
        openBtn.setVisibility(canOpen ? View.VISIBLE : View.GONE);

        // Wi-Fi Direct.
        boolean wifiOn = Reach.get(this, Reach.WIFI);
        if (wifiToggle.isOn() != wifiOn) wifiToggle.set(wifiOn, true);
        wifiCard.setAlpha(p ? 0.5f : 1f);
        String wl;
        int wc = MUTED;
        boolean busy = false;
        boolean fixable = false;
        if (!wifiOn) wl = "Off in MeshGRID";
        else if (p) wl = "Paused";
        else if ((st & LinkService.HAS_HARDWARE) == 0) wl = "This phone has no Wi-Fi Direct";
        else if (checkSelfPermission(wifiPermission()) != PackageManager.PERMISSION_GRANTED) {
            wl = "Not allowed yet · tap to allow";
            wc = WARN;
            fixable = true;
        } else if ((st & LinkService.PERMITTED) == 0) {
            wl = "Needs location on (Android 12 and older) · tap";
            wc = WARN;
            fixable = true;
        } else if ((st & LinkService.WIFI_ON) == 0) {
            wl = "Ready once Wi-Fi is on · no network needed";
            fixable = true;
        } else if (wifiPhase == LinkService.SETTING_UP) {
            wl = "Setting up a link to a nearby phone…";
            wc = TEAL;
            busy = true;
            wifiDetail.setText("Joining the group your phones agreed on over Bluetooth");
        } else if (wifiPhase == LinkService.LINKED) {
            wl = "Linked to a nearby phone";
            wc = TEAL;
            busy = true;
            wifiDetail.setText(size(LinkService.sent.get()) + " sent · " + size(LinkService.received.get()) + " received");
        } else {
            wl = "Ready for large photos and files";
            wc = TEAL;
        }
        wifiLabel.setText(wl);
        wifiLabel.setTextColor(wc);
        wifiBusy.setVisibility(busy ? View.VISIBLE : View.GONE);
        int wk = fixable && !p ? 1 : 0;
        if (wk != wifiCardKey) {
            wifiCardKey = wk;
            wifiCard.setClickable(wk == 1);
            wifiCard.setBackground(wk == 1 ? ui.pressable(ui.round(CARD, 24)) : ui.round(CARD, 24));
        }

        // Internet route.
        boolean inetOn = Reach.get(this, Reach.INET);
        if (inetToggle.isOn() != inetOn) inetToggle.set(inetOn, true);
        inetCard.setAlpha(p ? 0.5f : 1f);
        String il, id;
        int ic = MUTED;
        boolean spin = false;
        String about = "Switch it on in Meshtrike: Settings, Internet route. It reaches far-away contacts through the Nym mixnet, "
            + "behind a bridge that looks like an ordinary website.";
        if (!inetOn) {
            il = "Off in MeshGRID";
            id = "Meshtrike cannot use the internet route while this is off. Messages travel only phone to phone.";
        } else if (p) {
            il = "Paused";
            id = "Resume MeshGRID to use the internet route again.";
        } else if (rs.state == RouteStatus.IDLE) {
            il = "Not in use";
            id = about;
        } else if (rs.state == RouteStatus.OFF) {
            il = "Off in Meshtrike";
            id = about;
        } else if (rs.state == RouteStatus.HELD) {
            il = "Waiting for a charger";
            id = "“Only when charging” is on in MeshGRID’s settings, so the route pauses on battery.";
        } else if (rs.state == RouteStatus.NEEDS_BRIDGE) {
            il = "Needs a bridge";
            id = "Add a bridge in Meshtrike, or allow direct connections there.";
            ic = WARN;
        } else if (rs.state == RouteStatus.CONNECTING) {
            il = "Connecting…";
            spin = true;
            id = "Waiting for the internet and the mixnet"
                + ("bridge".equals(rs.mode) ? ", through your bridge." : ".");
        } else if ("bridge".equals(rs.mode)) {
            il = "Connected · through a bridge";
            ic = GOOD;
            id = "Through the Nym mixnet, behind a bridge that looks like an ordinary website.";
        } else {
            il = "Connected · directly";
            ic = GOOD;
            id = rs.bridges.isEmpty()
                ? "Through the Nym mixnet, directly. Your network can see that you use Nym, not who you talk to."
                : "Through the mixnet, directly: your bridge cannot be reached from this network. It is used again once it can.";
        }
        inetLabel.setText(il);
        inetLabel.setTextColor(ic);
        inetSpinner.setVisibility(spin ? View.VISIBLE : View.GONE);
        inetDesc.setText(id);

        // Bridges: only what Meshtrike gave the running route; nothing is stored here.
        StringBuilder k = new StringBuilder().append(rs.state);
        for (RouteStatus.Bridge b : rs.bridges) k.append('|').append(b.endpoint).append(b.looksLike).append(b.inUse);
        if (!k.toString().equals(bridgeKey)) {
            bridgeKey = k.toString();
            bridgesHeader.setText("BRIDGES · " + rs.bridges.size());
            bridgeList.removeAllViews();
            if (rs.bridges.isEmpty()) {
                TextView none = ui.text(rs.state == RouteStatus.IDLE
                    ? "Meshtrike shares its bridges with MeshGRID while the internet route is on."
                    : "No bridges. Add one in Meshtrike: Settings, Internet route.", 12.5f, 400, MUTED);
                none.setLineSpacing(0, 1.35f);
                none.setBackground(ui.round(CARD, 22));
                none.setPadding(ui.dp(16), ui.dp(14), ui.dp(16), ui.dp(14));
                bridgeList.addView(none);
            }
            for (RouteStatus.Bridge b : rs.bridges) bridgeList.addView(bridgeRow(b));
        }
    }

    private View bridgeRow(RouteStatus.Bridge b) {
        LinearLayout r = ui.row();
        r.setBackground(ui.round(CARD, 22));
        r.setPadding(ui.dp(14), ui.dp(12), ui.dp(14), ui.dp(12));
        r.addView(ui.tile(R.drawable.ic_shield, TRACK, INK, 40, 13, 19));
        LinearLayout c = ui.col();
        ui.gap(c, 4);
        TextView ep = ui.text(b.endpoint, 14, 600, INK);
        ep.setTypeface(Ui.mono(600));
        ep.setSingleLine(true);
        ep.setEllipsize(TextUtils.TruncateAt.MIDDLE);
        c.addView(ep);
        String sub = (b.looksLike != null ? "Looks like " + b.looksLike : "REALITY bridge") + (b.inUse ? " · in use" : "");
        TextView s = ui.text(sub, 12, 400, b.inUse ? GOOD : MUTED);
        s.setSingleLine(true);
        s.setEllipsize(TextUtils.TruncateAt.END);
        c.addView(s);
        LinearLayout.LayoutParams cl = ui.weight();
        cl.leftMargin = ui.dp(12);
        r.addView(c, cl);
        return r;
    }

    private android.graphics.drawable.Drawable tinted(int res, int color) {
        android.graphics.drawable.Drawable d = getDrawable(res).mutate();
        d.setTint(color);
        return d;
    }

    private static String size(long bytes) {
        if (bytes < 1024) return bytes + " B";
        if (bytes < 1024 * 1024) return String.format(Locale.ROOT, "%.0f KB", bytes / 1024.0);
        if (bytes < 1024L * 1024 * 1024) return String.format(Locale.ROOT, "%.1f MB", bytes / (1024.0 * 1024));
        return String.format(Locale.ROOT, "%.2f GB", bytes / (1024.0 * 1024 * 1024));
    }

    // ===== Settings =====

    private View buildSettings() {
        LinearLayout page = ui.col();
        LinearLayout head = ui.row();
        head.setPadding(ui.dp(6), ui.dp(4), ui.dp(14), ui.dp(8));
        FrameLayout back = new FrameLayout(this);
        back.addView(ui.icon(R.drawable.ic_left, INK, 24), new FrameLayout.LayoutParams(ui.dp(24), ui.dp(24), Gravity.CENTER));
        back.setBackground(ui.pressable(ui.round(0x00000000, 14)));
        back.setContentDescription("Back");
        back.setOnClickListener(v -> show("home"));
        head.addView(back, ui.lp(ui.dp(44), ui.dp(44)));
        TextView title = ui.display("Settings", 24);
        LinearLayout.LayoutParams tl = ui.lp(WRAP, WRAP);
        tl.leftMargin = ui.dp(6);
        head.addView(title, tl);
        page.addView(head);

        LinearLayout list = ui.col();
        list.setPadding(ui.dp(16), ui.dp(4), ui.dp(16), ui.dp(30));
        ui.gap(list, 12);

        LinearLayout prefs = ui.col();
        prefs.setBackground(ui.round(CARD, 24));
        prefs.setPadding(ui.dp(16), ui.dp(4), ui.dp(16), ui.dp(4));

        chargingToggle = new Toggle(this, "Only when charging");
        chargingToggle.set(Reach.get(this, Reach.CHARGING_ONLY), false);
        chargingToggle.setOnClickListener(v -> {
            boolean on = !Reach.get(this, Reach.CHARGING_ONLY);
            Reach.set(this, Reach.CHARGING_ONLY, on);
            RouteStatus.reapply();
            chargingToggle.set(on, true);
        });
        prefs.addView(settingRow(R.drawable.ic_plug, "Only when charging",
            ui.text("Saves battery: the internet route pauses on battery", 12, 400, MUTED), chargingToggle, true, null));

        permLabel = ui.text("", 12, 400, MUTED);
        prefs.addView(settingRow(R.drawable.ic_wifi, Build.VERSION.SDK_INT >= 33 ? "Nearby Wi-Fi devices" : "Location for Wi-Fi Direct",
            permLabel, chevron(), true, v -> {
                if (needsWifiPermission()) requestWifiPermission();
                else openAppInfo();
            }));

        batteryLabel = ui.text("", 12, 400, MUTED);
        prefs.addView(settingRow(R.drawable.ic_battery, "Battery optimisation", batteryLabel, chevron(), false,
            v -> openBatterySettings()));
        list.addView(prefs);

        LinearLayout about = ui.col();
        about.setBackground(ui.round(CARD, 24));
        about.setPadding(ui.dp(16), ui.dp(16), ui.dp(16), ui.dp(16));
        ui.gap(about, 6);
        about.addView(ui.text("About", 15, 600, INK));
        String mv = meshtrikeVersion();
        TextView at = ui.text("MeshGRID v" + ownVersionCode() + " (" + ownVersionName() + ")"
            + (mv != null ? " · Meshtrike " + mv + " is installed" : " · Meshtrike is not installed") + ". "
            + "Meshtrike itself never touches the internet; MeshGRID is the only part that does, and only when you allow it. "
            + "Android starts MeshGRID when Meshtrike asks for it, so it needs no start-up setting of its own.", 13, 400, MUTED);
        at.setLineSpacing(0, 1.4f);
        about.addView(at);
        list.addView(about);

        LinearLayout unlink = ui.row();
        unlink.setGravity(Gravity.CENTER);
        GradientDrawable ub = ui.round(CARD, 99);
        ub.setStroke(Math.max(1, Math.round(ui.dp(1.5f))), WARN_LINE);
        unlink.setBackground(ui.pressable(ub, 0x1FE0703B));
        unlink.addView(ui.icon(R.drawable.ic_unlink, WARN, 17));
        TextView ul = ui.text("Unlink from Meshtrike", 15, 600, WARN);
        LinearLayout.LayoutParams ull = ui.lp(WRAP, WRAP);
        ull.leftMargin = ui.dp(8);
        unlink.addView(ul, ull);
        unlink.setClickable(true);
        unlink.setOnClickListener(v -> new AlertDialog.Builder(this)
            .setTitle("Unlink from Meshtrike?")
            .setMessage("MeshGRID is unlinked by uninstalling it. Meshtrike then falls back to Bluetooth, LoRa and the mesh. "
                + "To only stop it for a while, use Pause instead.\n\nOpen App info to uninstall MeshGRID?")
            .setPositiveButton("App info", (d, w) -> openAppInfo())
            .setNegativeButton("Cancel", null)
            .show());
        list.addView(unlink, ui.lp(MATCH, ui.dp(52)));

        ScrollView sv = new ScrollView(this);
        sv.addView(list);
        page.addView(sv, new LinearLayout.LayoutParams(MATCH, 0, 1f));
        return page;
    }

    private ImageView chevron() {
        return ui.icon(R.drawable.ic_right, FAINT, 18);
    }

    private View settingRow(int icon, String title, TextView sub, View end, boolean divider, View.OnClickListener click) {
        LinearLayout wrap = ui.col();
        LinearLayout r = ui.row();
        r.setPadding(0, ui.dp(13), 0, ui.dp(13));
        r.addView(ui.icon(icon, INK, 19));
        LinearLayout c = ui.col();
        ui.gap(c, 3);
        c.addView(ui.text(title, 14.5f, 500, INK));
        sub.setLineSpacing(0, 1.3f);
        c.addView(sub);
        LinearLayout.LayoutParams cl = ui.weight();
        cl.leftMargin = ui.dp(12);
        cl.rightMargin = ui.dp(8);
        r.addView(c, cl);
        r.addView(end);
        wrap.addView(r);
        if (click != null) {
            wrap.setOnClickListener(click);
            wrap.setBackground(ui.pressable(ui.round(0x00000000, 12)));
        }
        if (divider) {
            View line = new View(this);
            line.setBackgroundColor(DIVIDER);
            wrap.addView(line, ui.lp(MATCH, ui.dp(1)));
        }
        return wrap;
    }

    private void refreshSettings() {
        boolean charging = Reach.get(this, Reach.CHARGING_ONLY);
        if (chargingToggle.isOn() != charging) chargingToggle.set(charging, true);
        int st = LinkService.statusOf(this);
        if ((st & LinkService.HAS_HARDWARE) == 0) permLabel.setText("Not needed: this phone has no Wi-Fi Direct");
        else if (checkSelfPermission(wifiPermission()) == PackageManager.PERMISSION_GRANTED) {
            permLabel.setText(Build.VERSION.SDK_INT >= 33
                ? "Allowed · used only to set up Wi-Fi Direct, never for location"
                : "Allowed · Android 12 and older tie Wi-Fi Direct to location; it is never read");
        } else permLabel.setText("Not allowed · Wi-Fi Direct stays off until you allow it");
        PowerManager pm = getSystemService(PowerManager.class);
        boolean exempt = pm != null && pm.isIgnoringBatteryOptimizations(getPackageName());
        batteryLabel.setText(exempt
            ? "Not optimised · MeshGRID stays awake"
            : "Optimised · Android may stop MeshGRID in the background");
    }
}
