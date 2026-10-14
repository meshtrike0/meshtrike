package org.meshtrike.wifilink;

import android.animation.ArgbEvaluator;
import android.animation.ValueAnimator;
import android.content.Context;
import android.content.res.ColorStateList;
import android.graphics.Canvas;
import android.graphics.Paint;
import android.graphics.RectF;
import android.graphics.Typeface;
import android.graphics.drawable.Drawable;
import android.graphics.drawable.GradientDrawable;
import android.graphics.drawable.RippleDrawable;
import android.view.Gravity;
import android.view.View;
import android.view.accessibility.AccessibilityNodeInfo;
import android.view.animation.AccelerateDecelerateInterpolator;
import android.view.animation.LinearInterpolator;
import android.view.animation.OvershootInterpolator;
import android.widget.FrameLayout;
import android.widget.ImageView;
import android.widget.LinearLayout;
import android.widget.Switch;
import android.widget.TextView;

/** Small view helpers and the few custom views the MeshGRID screen needs (no libraries). */
final class Ui {
    // Palette from the MeshGRID design.
    static final int INK = 0xFF15171C;
    static final int PAPER = 0xFFF6F5F1;
    static final int CARD = 0xFFFFFFFF;
    static final int LIME = 0xFFCDF25C;
    static final int VIOLET = 0xFF6236D0;
    static final int VIOLET_SOFT = 0xFFECE5FF;
    static final int TEAL = 0xFF08736E;
    static final int TEAL_SOFT = 0xFFD9F3EF;
    static final int BODY = 0xFF41454D;
    static final int MUTED = 0xFF5D616B;
    static final int FAINT = 0xFF7A766D;
    static final int DIVIDER = 0xFFF0EDE7;
    static final int TRACK = 0xFFEEEBE4;
    static final int SWITCH_OFF = 0xFFD6D2C9;
    static final int SPINNER_TRACK = 0xFFE6E3DC;
    static final int LIVE = 0xFF22A36B;
    static final int GOOD = 0xFF0B6E4A;
    static final int WARN = 0xFFA3360A;
    static final int WARN_LINE = 0xFFE0703B;
    static final int PAUSED_BG = 0xFFEDEBE6;
    static final int PAUSED_DOT = 0xFF9A968C;

    static final int MATCH = LinearLayout.LayoutParams.MATCH_PARENT;
    static final int WRAP = LinearLayout.LayoutParams.WRAP_CONTENT;

    final Context c;
    private final float density;

    Ui(Context c) {
        this.c = c;
        this.density = c.getResources().getDisplayMetrics().density;
    }

    int dp(float v) {
        return Math.round(v * density);
    }

    static Typeface sans(int weight) {
        return Typeface.create(Typeface.SANS_SERIF, weight, false);
    }

    static Typeface mono(int weight) {
        return Typeface.create(Typeface.MONOSPACE, weight, false);
    }

    TextView text(CharSequence s, float sp, int weight, int color) {
        TextView t = new TextView(c);
        t.setText(s);
        t.setTextSize(sp);
        t.setTypeface(sans(weight));
        t.setTextColor(color);
        t.setIncludeFontPadding(false);
        return t;
    }

    /** Headline text: tight, heavy, slightly condensed tracking. */
    TextView display(CharSequence s, float sp) {
        TextView t = text(s, sp, 700, INK);
        t.setLetterSpacing(-0.02f);
        return t;
    }

    TextView eyebrow(CharSequence s, int color) {
        TextView t = text(s, 11, 600, color);
        t.setTypeface(mono(600));
        t.setLetterSpacing(0.08f);
        return t;
    }

    GradientDrawable round(int color, float radiusDp) {
        GradientDrawable g = new GradientDrawable();
        g.setColor(color);
        g.setCornerRadius(dp(radiusDp));
        return g;
    }

    /** A background that ripples on touch, clipped to its own shape. */
    static Drawable pressable(Drawable bg, int rippleColor) {
        return new RippleDrawable(ColorStateList.valueOf(rippleColor), bg, null);
    }

    Drawable pressable(Drawable bg) {
        return pressable(bg, 0x1F15171C);
    }

    ImageView icon(int res, int color, float sizeDp) {
        ImageView v = new ImageView(c);
        Drawable d = c.getDrawable(res).mutate();
        d.setTint(color);
        v.setImageDrawable(d);
        v.setLayoutParams(new LinearLayout.LayoutParams(dp(sizeDp), dp(sizeDp)));
        v.setImportantForAccessibility(View.IMPORTANT_FOR_ACCESSIBILITY_NO);
        return v;
    }

    /** A rounded square with a centred icon (the design's icon tiles). */
    FrameLayout tile(int res, int bg, int fg, float sizeDp, float radiusDp, float iconDp) {
        FrameLayout f = new FrameLayout(c);
        f.setBackground(round(bg, radiusDp));
        ImageView i = icon(res, fg, iconDp);
        f.addView(i, new FrameLayout.LayoutParams(dp(iconDp), dp(iconDp), Gravity.CENTER));
        f.setLayoutParams(new LinearLayout.LayoutParams(dp(sizeDp), dp(sizeDp)));
        return f;
    }

    LinearLayout row() {
        LinearLayout l = new LinearLayout(c);
        l.setOrientation(LinearLayout.HORIZONTAL);
        l.setGravity(Gravity.CENTER_VERTICAL);
        return l;
    }

    LinearLayout col() {
        LinearLayout l = new LinearLayout(c);
        l.setOrientation(LinearLayout.VERTICAL);
        return l;
    }

    /** Vertical gap between children, as CSS `gap`. */
    void gap(LinearLayout l, float dp) {
        GradientDrawable g = new GradientDrawable();
        g.setSize(dp(dp), dp(dp));
        g.setColor(0);
        l.setDividerDrawable(g);
        l.setShowDividers(LinearLayout.SHOW_DIVIDER_MIDDLE);
    }

    LinearLayout.LayoutParams lp(int w, int h) {
        return new LinearLayout.LayoutParams(w, h);
    }

    LinearLayout.LayoutParams weight() {
        return new LinearLayout.LayoutParams(0, WRAP, 1f);
    }

    View spinner(int color, float sizeDp, float strokeDp) {
        Spinner s = new Spinner(c, color, dp(strokeDp));
        s.setLayoutParams(new LinearLayout.LayoutParams(dp(sizeDp), dp(sizeDp)));
        return s;
    }

    /** The design's ring spinner: a light track with one coloured quarter turning. */
    static final class Spinner extends View {
        private final Paint paint = new Paint(Paint.ANTI_ALIAS_FLAG);
        private final int color;
        private final float stroke;
        private ValueAnimator anim;
        private float angle;

        Spinner(Context c, int color, float stroke) {
            super(c);
            this.color = color;
            this.stroke = stroke;
            paint.setStyle(Paint.Style.STROKE);
            paint.setStrokeWidth(stroke);
            setImportantForAccessibility(IMPORTANT_FOR_ACCESSIBILITY_NO);
        }

        @Override
        protected void onAttachedToWindow() {
            super.onAttachedToWindow();
            anim = ValueAnimator.ofFloat(0, 360);
            anim.setDuration(800);
            anim.setRepeatCount(ValueAnimator.INFINITE);
            anim.setInterpolator(new LinearInterpolator());
            anim.addUpdateListener(a -> {
                angle = (float) a.getAnimatedValue();
                invalidate();
            });
            anim.start();
        }

        @Override
        protected void onDetachedFromWindow() {
            if (anim != null) anim.cancel();
            anim = null;
            super.onDetachedFromWindow();
        }

        @Override
        protected void onDraw(Canvas cv) {
            float h = stroke / 2;
            RectF r = new RectF(h, h, getWidth() - h, getHeight() - h);
            paint.setColor(SPINNER_TRACK);
            cv.drawOval(r, paint);
            paint.setColor(color);
            cv.drawArc(r, angle - 90, 90, false, paint);
        }
    }

    /** The design's 52x32 switch: ink track and lime knob when on. */
    static final class Toggle extends View {
        private final Paint paint = new Paint(Paint.ANTI_ALIAS_FLAG);
        private final ArgbEvaluator argb = new ArgbEvaluator();
        private final float d;
        private boolean on;
        private float pos;
        private ValueAnimator anim;

        Toggle(Context c, String label) {
            super(c);
            d = c.getResources().getDisplayMetrics().density;
            setContentDescription(label);
            setClickable(true);
            setFocusable(true);
        }

        boolean isOn() {
            return on;
        }

        void set(boolean v, boolean animate) {
            if (v == on && anim == null) {
                pos = v ? 1 : 0;
                invalidate();
                return;
            }
            on = v;
            if (anim != null) anim.cancel();
            if (!animate || !isAttachedToWindow()) {
                anim = null;
                pos = v ? 1 : 0;
                invalidate();
                return;
            }
            anim = ValueAnimator.ofFloat(pos, v ? 1 : 0);
            anim.setDuration(280);
            anim.setInterpolator(new OvershootInterpolator(1.6f));
            anim.addUpdateListener(a -> {
                pos = (float) a.getAnimatedValue();
                invalidate();
            });
            ValueAnimator mine = anim;
            anim.addListener(new android.animation.AnimatorListenerAdapter() {
                @Override
                public void onAnimationEnd(android.animation.Animator a) {
                    if (anim == mine) anim = null;
                }
            });
            anim.start();
        }

        @Override
        protected void onMeasure(int w, int h) {
            setMeasuredDimension(Math.round(52 * d), Math.round(32 * d));
        }

        @Override
        protected void onDraw(Canvas cv) {
            float p = Math.max(0, Math.min(1, pos));
            paint.setColor((int) argb.evaluate(p, SWITCH_OFF, INK));
            float h = getHeight();
            cv.drawRoundRect(new RectF(0, 0, getWidth(), h), h / 2, h / 2, paint);
            paint.setColor((int) argb.evaluate(p, 0xFFFFFFFF, LIME));
            float r = 13 * d;
            cv.drawCircle(3 * d + r + pos * 20 * d, h / 2, r, paint);
        }

        @Override
        public void onInitializeAccessibilityNodeInfo(AccessibilityNodeInfo info) {
            super.onInitializeAccessibilityNodeInfo(info);
            info.setClassName(Switch.class.getName());
            info.setCheckable(true);
            info.setChecked(on);
        }
    }

    /**
     * A moving bar for work of unknown size: MeshGRID relays bytes it cannot
     * read, so it never knows a file's total and shows no percentage.
     */
    static final class ActivityBar extends View {
        private final Paint paint = new Paint(Paint.ANTI_ALIAS_FLAG);
        private final float d;
        private final int color;
        private float t;
        private ValueAnimator anim;

        ActivityBar(Context c, int color) {
            super(c);
            this.color = color;
            d = c.getResources().getDisplayMetrics().density;
        }

        @Override
        protected void onMeasure(int w, int h) {
            setMeasuredDimension(MeasureSpec.getSize(w), Math.round(5 * d));
        }

        @Override
        protected void onAttachedToWindow() {
            super.onAttachedToWindow();
            anim = ValueAnimator.ofFloat(0, 1);
            anim.setDuration(1400);
            anim.setRepeatCount(ValueAnimator.INFINITE);
            anim.setInterpolator(new AccelerateDecelerateInterpolator());
            anim.addUpdateListener(a -> {
                t = (float) a.getAnimatedValue();
                invalidate();
            });
            anim.start();
        }

        @Override
        protected void onDetachedFromWindow() {
            if (anim != null) anim.cancel();
            anim = null;
            super.onDetachedFromWindow();
        }

        @Override
        protected void onDraw(Canvas cv) {
            float w = getWidth();
            float h = getHeight();
            paint.setColor(TRACK);
            cv.drawRoundRect(new RectF(0, 0, w, h), h, h, paint);
            float seg = w * 0.32f;
            float x = -seg + t * (w + seg);
            cv.save();
            cv.clipRect(0, 0, w, h);
            paint.setColor(color);
            cv.drawRoundRect(new RectF(x, 0, x + seg, h), h, h, paint);
            cv.restore();
        }
    }

    /** The dashed line between the two tiles on the setup screen. */
    static final class Dashes extends View {
        private final Paint paint = new Paint(Paint.ANTI_ALIAS_FLAG);
        private final float d;

        Dashes(Context c) {
            super(c);
            d = c.getResources().getDisplayMetrics().density;
            paint.setColor(INK);
            paint.setAlpha(90);
        }

        @Override
        protected void onDraw(Canvas cv) {
            float y0 = (getHeight() - 2 * d) / 2;
            for (float x = 0; x < getWidth(); x += 9 * d) {
                cv.drawRect(x, y0, Math.min(x + 4 * d, getWidth()), y0 + 2 * d, paint);
            }
        }
    }

    /** A dot that pings outwards (the design's "breathe"). */
    FrameLayout liveDot(int color, boolean live) {
        FrameLayout f = new FrameLayout(c);
        View halo = new View(c);
        GradientDrawable hg = new GradientDrawable();
        hg.setShape(GradientDrawable.OVAL);
        hg.setColor(color);
        halo.setBackground(hg);
        View dot = new View(c);
        GradientDrawable dg = new GradientDrawable();
        dg.setShape(GradientDrawable.OVAL);
        dg.setColor(color);
        dot.setBackground(dg);
        // 20dp box (the halo grows to twice the dot), pulled back to sit like a 10dp dot.
        f.addView(halo, new FrameLayout.LayoutParams(dp(10), dp(10), Gravity.CENTER));
        f.addView(dot, new FrameLayout.LayoutParams(dp(10), dp(10), Gravity.CENTER));
        LinearLayout.LayoutParams lp = new LinearLayout.LayoutParams(dp(20), dp(20));
        lp.leftMargin = -dp(5);
        lp.rightMargin = dp(5);
        f.setLayoutParams(lp);
        if (live) {
            ValueAnimator a = ValueAnimator.ofFloat(0, 1);
            a.setDuration(1000);
            a.setRepeatCount(ValueAnimator.INFINITE);
            a.setRepeatMode(ValueAnimator.REVERSE);
            a.setInterpolator(new LinearInterpolator());
            a.addUpdateListener(x -> {
                float v = (float) x.getAnimatedValue();
                halo.setScaleX(1 + v);
                halo.setScaleY(1 + v);
                halo.setAlpha(1 - v);
            });
            f.addOnAttachStateChangeListener(new View.OnAttachStateChangeListener() {
                @Override
                public void onViewAttachedToWindow(View v) {
                    a.start();
                }

                @Override
                public void onViewDetachedFromWindow(View v) {
                    a.cancel();
                }
            });
        } else {
            halo.setVisibility(View.GONE);
        }
        return f;
    }
}
