package io.github.juanluci.starling;

import android.app.Activity;
import android.app.Presentation;
import android.content.Context;
import android.graphics.Color;
import android.graphics.PixelFormat;
import android.graphics.Rect;
import android.hardware.display.DisplayManager;
import android.os.Build;
import android.os.Bundle;
import android.os.Handler;
import android.os.Looper;
import android.util.Log;
import android.view.Display;
import android.view.KeyEvent;
import android.view.MotionEvent;
import android.view.Surface;
import android.view.SurfaceHolder;
import android.view.SurfaceView;
import android.view.View;
import android.view.ViewGroup;
import android.view.Window;
import android.view.WindowInsets;
import android.view.WindowInsetsController;
import android.view.WindowManager;
import android.widget.FrameLayout;

/**
 * Drives the secondary touch display on dual-screen Android handhelds (such as the Retroid Pocket Duo
 * 4:3 lower touch panel and AYN Thor) via an immersive, non-focus-stealing {@link Presentation}
 * hosting a full-bleed {@link SurfaceView} bound to Rust's {@code dual_screen_android} bridge.
 */
public final class SecondaryDisplay {
    private static final String TAG = "starling";

    static native void onSurfaceReady(Surface surface, int width, int height);
    static native void onSurfaceDestroyed();
    static native void onImeActive(boolean active);
    static native void onTouch(
            int primaryPointerId,
            int actionMasked,
            int actionPointerId,
            float x,
            float y,
            float width,
            float height);

    private final Activity activity;
    private final DisplayManager displayManager;
    private final Handler mainHandler = new Handler(Looper.getMainLooper());
    private boolean resumed;
    private BottomPresentation presentation;
    private int currentDisplayId = Display.INVALID_DISPLAY;

    private final DisplayManager.DisplayListener displayListener =
            new DisplayManager.DisplayListener() {
                @Override
                public void onDisplayAdded(int displayId) {
                    reconcile();
                }

                @Override
                public void onDisplayRemoved(int displayId) {
                    if (displayId == currentDisplayId) {
                        dismissCurrent();
                    }
                    reconcile();
                }

                @Override
                public void onDisplayChanged(int displayId) {
                    reconcile();
                }
            };

    public SecondaryDisplay(Activity activity) {
        this.activity = activity;
        this.displayManager =
                (DisplayManager) activity.getSystemService(Context.DISPLAY_SERVICE);
    }

    public void resume() {
        if (displayManager == null) {
            return;
        }
        if (!resumed) {
            resumed = true;
            displayManager.registerDisplayListener(displayListener, mainHandler);
        }
        reconcile();
    }

    public void pause() {
        if (resumed && displayManager != null) {
            resumed = false;
            displayManager.unregisterDisplayListener(displayListener);
        }
        dismissCurrent();
    }

    public void destroy() {
        pause();
    }

    public boolean showTextMirror(String text, int cursor, int anchor, int maxLength, int flags, int generation) {
        if (presentation == null || !presentation.isShowing()) {
            return false;
        }
        presentation.showTextMirror(text, cursor, anchor, maxLength, flags, generation);
        return true;
    }

    public boolean updateTextMirror(String text, int cursor, int anchor, int maxLength, int flags, int generation) {
        if (presentation == null || !presentation.isShowing()) {
            return false;
        }
        presentation.updateTextMirror(text, cursor, anchor, maxLength, flags, generation);
        return true;
    }

    public boolean hideTextMirror() {
        if (presentation == null || !presentation.isShowing()) {
            return false;
        }
        presentation.hideTextMirror();
        return true;
    }

    @SuppressWarnings("deprecation")
    private int activityDisplayId() {
        Display d = Build.VERSION.SDK_INT >= 30
                ? activity.getDisplay()
                : activity.getWindowManager().getDefaultDisplay();
        return d != null ? d.getDisplayId() : Display.DEFAULT_DISPLAY;
    }

    private Display findSecondaryDisplay() {
        if (displayManager == null) {
            return null;
        }
        int primaryId = activityDisplayId();
        Display[] presentationDisplays =
                displayManager.getDisplays(DisplayManager.DISPLAY_CATEGORY_PRESENTATION);
        if (presentationDisplays != null) {
            for (Display d : presentationDisplays) {
                if (isUsableSecondary(d, primaryId)) {
                    return d;
                }
            }
        }
        // Fallback for handheld firmware exposing the built-in lower panel without FLAG_PRESENTATION.
        Display[] allDisplays = displayManager.getDisplays();
        if (allDisplays != null) {
            for (Display d : allDisplays) {
                if (isUsableSecondary(d, primaryId)) {
                    return d;
                }
            }
        }
        return null;
    }

    private static boolean isUsableSecondary(Display d, int primaryId) {
        if (d == null || !d.isValid()) {
            return false;
        }
        int id = d.getDisplayId();
        if (id == Display.DEFAULT_DISPLAY || id == primaryId) {
            return false;
        }
        return d.getState() != Display.STATE_OFF;
    }

    private void reconcile() {
        if (!resumed || activity.isFinishing() || activity.isDestroyed()) {
            dismissCurrent();
            return;
        }
        Display target = findSecondaryDisplay();
        if (target == null) {
            dismissCurrent();
            return;
        }
        if (presentation != null
                && presentation.isShowing()
                && currentDisplayId == target.getDisplayId()) {
            presentation.hideSystemBars();
            return;
        }
        dismissCurrent();
        try {
            BottomPresentation next = new BottomPresentation(activity, activity, target);
            next.show();
            presentation = next;
            currentDisplayId = target.getDisplayId();
            Log.i(TAG, "ShulkDS: showing secondary Presentation on display "
                    + currentDisplayId + " (" + target.getName() + ")");
        } catch (RuntimeException e) {
            Log.w(TAG, "ShulkDS: could not show secondary Presentation on display "
                    + target.getDisplayId(), e);
            dismissCurrent();
        }
    }

    private void dismissCurrent() {
        BottomPresentation old = presentation;
        presentation = null;
        currentDisplayId = Display.INVALID_DISPLAY;
        if (old != null) {
            try {
                if (old.imeActive) {
                    old.imeActive = false;
                    old.imeHadFocus = false;
                    onImeActive(false);
                }
                if (old.surfaceActive) {
                    old.surfaceActive = false;
                    onSurfaceDestroyed();
                }
                old.dismiss();
            } catch (RuntimeException e) {
                Log.w(TAG, "ShulkDS: error dismissing secondary Presentation", e);
            }
        }
    }

    private static final class BottomPresentation extends Presentation
            implements SurfaceHolder.Callback, View.OnTouchListener {
        private final Activity ownerActivity;
        private SurfaceView surfaceView;
        private TextMirror mirror;
        boolean surfaceActive;
        boolean imeActive;
        boolean imeHadFocus;
        private int trackedPointerId = MotionEvent.INVALID_POINTER_ID;
        private float bestRefreshRate = 0f;

        BottomPresentation(Activity ownerActivity, Context outerContext, Display display) {
            super(outerContext, display, android.R.style.Theme_Black_NoTitleBar_Fullscreen);
            this.ownerActivity = ownerActivity;
        }

        @Override
        protected void onCreate(Bundle savedInstanceState) {
            super.onCreate(savedInstanceState);
            requestWindowFeature(Window.FEATURE_NO_TITLE);
            setCancelable(false);
            setCanceledOnTouchOutside(false);

            Window win = getWindow();
            if (win != null) {
                // FLAG_NOT_FOCUSABLE keeps gamepad / key focus on Display 0's NativeActivity
                // while still delivering touch MotionEvents directly to this SurfaceView.
                win.addFlags(WindowManager.LayoutParams.FLAG_NOT_FOCUSABLE
                        | WindowManager.LayoutParams.FLAG_HARDWARE_ACCELERATED
                        | WindowManager.LayoutParams.FLAG_FULLSCREEN
                        | WindowManager.LayoutParams.FLAG_LAYOUT_IN_SCREEN
                        | WindowManager.LayoutParams.FLAG_LAYOUT_NO_LIMITS
                        | WindowManager.LayoutParams.FLAG_KEEP_SCREEN_ON);
                win.setBackgroundDrawableResource(android.R.color.black);
                if (Build.VERSION.SDK_INT >= 28) {
                    WindowManager.LayoutParams attrs = win.getAttributes();
                    attrs.layoutInDisplayCutoutMode =
                            WindowManager.LayoutParams.LAYOUT_IN_DISPLAY_CUTOUT_MODE_SHORT_EDGES;
                    win.setAttributes(attrs);
                }
                applyMaxRefreshRate(win);
            }

            surfaceView = new SurfaceView(getContext());
            surfaceView.setBackgroundColor(Color.TRANSPARENT);
            surfaceView.setFocusable(false);
            surfaceView.setFocusableInTouchMode(false);
            surfaceView.getHolder().setFormat(PixelFormat.RGBA_8888);
            surfaceView.getHolder().addCallback(this);
            surfaceView.setOnTouchListener(this);

            FrameLayout root = new FrameLayout(getContext());
            root.addView(
                    surfaceView,
                    new FrameLayout.LayoutParams(
                            ViewGroup.LayoutParams.MATCH_PARENT,
                            ViewGroup.LayoutParams.MATCH_PARENT));
            mirror = new TextMirror(getContext());
            root.addView(mirror, new FrameLayout.LayoutParams(1, 1));
            setContentView(
                    root,
                    new ViewGroup.LayoutParams(
                            ViewGroup.LayoutParams.MATCH_PARENT,
                            ViewGroup.LayoutParams.MATCH_PARENT));
            hideSystemBars();
        }

        void showTextMirror(String text, int cursor, int anchor, int maxLength, int flags, int generation) {
            imeActive = true;
            imeHadFocus = false;
            onImeActive(true);
            Window win = getWindow();
            if (win != null) {
                win.clearFlags(WindowManager.LayoutParams.FLAG_NOT_FOCUSABLE
                        | WindowManager.LayoutParams.FLAG_ALT_FOCUSABLE_IM);
                win.setSoftInputMode(WindowManager.LayoutParams.SOFT_INPUT_ADJUST_NOTHING
                        | WindowManager.LayoutParams.SOFT_INPUT_STATE_ALWAYS_VISIBLE);
            }
            if (mirror != null) {
                mirror.show(text, cursor, anchor, maxLength, flags, generation);
            }
        }

        void updateTextMirror(String text, int cursor, int anchor, int maxLength, int flags, int generation) {
            if (mirror != null) {
                mirror.update(text, cursor, anchor, maxLength, flags, generation);
            }
        }

        void hideTextMirror() {
            imeActive = false;
            imeHadFocus = false;
            if (mirror != null) {
                mirror.hide();
            }
            Window win = getWindow();
            if (win != null) {
                win.addFlags(WindowManager.LayoutParams.FLAG_NOT_FOCUSABLE);
            }
            hideSystemBars();
            onImeActive(false);
            if (ownerActivity != null && ownerActivity.getWindow() != null) {
                final View decor = ownerActivity.getWindow().getDecorView();
                decor.post(new Runnable() {
                    @Override
                    public void run() {
                        decor.setFocusableInTouchMode(true);
                        decor.requestFocus();
                    }
                });
            }
        }

        @Override
        public void onWindowFocusChanged(boolean hasFocus) {
            super.onWindowFocusChanged(hasFocus);
            if (!imeActive) {
                return;
            }
            if (hasFocus) {
                imeHadFocus = true;
            } else if (imeHadFocus) {
                hideTextMirror();
            }
        }

        @Override
        public boolean dispatchKeyEvent(KeyEvent event) {
            if (super.dispatchKeyEvent(event)) {
                return true;
            }
            return ownerActivity != null && ownerActivity.dispatchKeyEvent(event);
        }

        @SuppressWarnings("deprecation")
        void hideSystemBars() {
            Window win = getWindow();
            if (win == null) {
                return;
            }
            if (Build.VERSION.SDK_INT >= 30) {
                WindowInsetsController controller = win.getInsetsController();
                if (controller != null) {
                    controller.hide(WindowInsets.Type.systemBars());
                    controller.setSystemBarsBehavior(
                            WindowInsetsController.BEHAVIOR_SHOW_TRANSIENT_BARS_BY_SWIPE);
                }
            } else {
                win.getDecorView().setSystemUiVisibility(
                        View.SYSTEM_UI_FLAG_IMMERSIVE_STICKY
                                | View.SYSTEM_UI_FLAG_FULLSCREEN
                                | View.SYSTEM_UI_FLAG_HIDE_NAVIGATION
                                | View.SYSTEM_UI_FLAG_LAYOUT_FULLSCREEN
                                | View.SYSTEM_UI_FLAG_LAYOUT_HIDE_NAVIGATION
                                | View.SYSTEM_UI_FLAG_LAYOUT_STABLE);
            }
        }

        @SuppressWarnings("deprecation")
        private void applyMaxRefreshRate(Window win) {
            if (win == null || Build.VERSION.SDK_INT < 23) {
                return;
            }
            Display display = getDisplay();
            if (display == null) {
                return;
            }
            Display.Mode current = display.getMode();
            Display.Mode[] modes = display.getSupportedModes();
            if (modes == null || modes.length == 0) {
                return;
            }
            Display.Mode bestSameRes = null;
            Display.Mode bestOverall = null;
            for (Display.Mode mode : modes) {
                if (bestOverall == null || mode.getRefreshRate() > bestOverall.getRefreshRate()) {
                    bestOverall = mode;
                }
                if (current != null
                        && mode.getPhysicalWidth() == current.getPhysicalWidth()
                        && mode.getPhysicalHeight() == current.getPhysicalHeight()
                        && (bestSameRes == null
                                || mode.getRefreshRate() > bestSameRes.getRefreshRate())) {
                    bestSameRes = mode;
                }
            }
            Display.Mode bestMode = bestSameRes != null ? bestSameRes : bestOverall;
            if (bestMode == null) {
                return;
            }
            bestRefreshRate = bestMode.getRefreshRate();
            WindowManager.LayoutParams attrs = win.getAttributes();
            attrs.preferredDisplayModeId = bestMode.getModeId();
            attrs.preferredRefreshRate = bestRefreshRate;
            win.setAttributes(attrs);
        }

        private void applySurfaceFrameRate(Surface surface) {
            if (Build.VERSION.SDK_INT >= 30 && bestRefreshRate > 0f && surface != null && surface.isValid()) {
                try {
                    surface.setFrameRate(bestRefreshRate, Surface.FRAME_RATE_COMPATIBILITY_FIXED_SOURCE);
                } catch (Throwable ignored) {
                }
            }
        }

        @Override
        public void surfaceCreated(SurfaceHolder holder) {
            Rect frame = holder.getSurfaceFrame();
            int w = frame != null ? frame.width() : 0;
            int h = frame != null ? frame.height() : 0;
            Surface surface = holder.getSurface();
            if (surface != null && surface.isValid()) {
                applySurfaceFrameRate(surface);
                surfaceActive = true;
                onSurfaceReady(surface, w, h);
            }
        }

        @Override
        public void surfaceChanged(SurfaceHolder holder, int format, int width, int height) {
            Surface surface = holder.getSurface();
            if (surface != null && surface.isValid()) {
                applySurfaceFrameRate(surface);
                surfaceActive = true;
                onSurfaceReady(surface, width, height);
            }
        }

        @Override
        public void surfaceDestroyed(SurfaceHolder holder) {
            trackedPointerId = MotionEvent.INVALID_POINTER_ID;
            if (surfaceActive) {
                surfaceActive = false;
                onSurfaceDestroyed();
            }
        }

        @Override
        public boolean onTouch(View view, MotionEvent event) {
            int actionMasked = event.getActionMasked();
            int actionIndex = event.getActionIndex();
            int actionPointerId = event.getPointerId(actionIndex);

            if (actionMasked == MotionEvent.ACTION_DOWN) {
                trackedPointerId = actionPointerId;
            } else if (actionMasked == MotionEvent.ACTION_POINTER_DOWN
                    && trackedPointerId == MotionEvent.INVALID_POINTER_ID) {
                trackedPointerId = actionPointerId;
            }

            int queryId = trackedPointerId != MotionEvent.INVALID_POINTER_ID
                    ? trackedPointerId
                    : actionPointerId;
            int pointerIndex = event.findPointerIndex(queryId);
            if (pointerIndex < 0) {
                pointerIndex = actionIndex;
                queryId = event.getPointerId(pointerIndex);
            }

            float x = event.getX(pointerIndex);
            float y = event.getY(pointerIndex);
            float w = view.getWidth();
            float h = view.getHeight();
            SecondaryDisplay.onTouch(queryId, actionMasked, actionPointerId, x, y, w, h);

            if (actionMasked == MotionEvent.ACTION_UP
                    || actionMasked == MotionEvent.ACTION_CANCEL
                    || (actionMasked == MotionEvent.ACTION_POINTER_UP
                            && actionPointerId == trackedPointerId)) {
                trackedPointerId = MotionEvent.INVALID_POINTER_ID;
            }
            return true;
        }
    }
}
