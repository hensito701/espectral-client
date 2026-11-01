package es.spectral.menu.ui;

import java.util.List;

import net.minecraft.client.Minecraft;
import net.minecraft.client.gui.components.AbstractButton;
import net.minecraft.client.gui.components.AbstractSliderButton;
import net.minecraft.client.gui.components.AbstractWidget;
import net.minecraft.client.gui.components.events.GuiEventListener;
import net.minecraft.client.gui.screens.Screen;
import net.minecraft.resources.Identifier;
import net.minecraft.util.Util;
import com.mojang.blaze3d.platform.InputConstants;

import es.spectral.menu.ClientConfig;
import es.spectral.menu.Compat;
import es.spectral.menu.agent.AgentScreenExtras;
import es.spectral.menu.agent.AgentWidget;

/**
 * The title-menu logo as a toy: left- or right-click it and it drops out of
 * its spot, then it can be dragged and thrown around every out-of-world menu.
 * It bounces off the four window edges and off the current screen's real
 * buttons (vanilla buttons and sliders at the top level of the screen, plus
 * the hand-painted title buttons), and rests on top of them. List rows and
 * scroll bars are not solid.
 *
 * <p>State is session-only and survives screen changes (it lives here, not on
 * a screen). Loading a world puts the logo back in place; so does a restart.
 * Under reduced motion there is no gravity, bounce or throw: the ball only
 * moves while it is dragged.
 *
 * <p>Version lanes only forward events: a per-version paint mixin calls
 * {@link #paint} at the tail of the current screen's render pass, and the
 * shared {@code MenuBallInputMixin} feeds raw mouse buttons to
 * {@link #onMouseButton} before vanilla dispatches them.
 */
public final class MenuBall {

    private static final Identifier LOGO = Identifier.fromNamespaceAndPath("espectral-menu", "logo");

    /** Physics substep (s); frames advance in whole substeps. */
    private static final float STEP = 1f / 240f;
    /** Longest frame gap simulated (s): a stalled frame never teleports the ball. */
    private static final float MAX_FRAME = 0.1f;
    /** Gravity in window heights per s². */
    private static final float GRAVITY = 3.2f;
    private static final float BOUNCE_WALL = 0.6f;
    private static final float BOUNCE_BUTTON = 0.5f;
    /** Normal speeds below this (GUI px/s) stop instead of bouncing (no resting jitter). */
    private static final float REST_SPEED = 40f;
    /** Rolling resistance per second while touching something. */
    private static final float ROLL_DRAG = 1.4f;
    private static final float AIR_SPIN_DRAG = 0.3f;
    /** Throws faster than this many window widths per second are capped. */
    private static final float MAX_THROW = 4f;
    /** Pointer travel (GUI px) under which a press-release on the docked logo is a click. */
    private static final float CLICK_SLOP = 3f;
    /** Velocity window for throws (ms). */
    private static final long THROW_WINDOW_MS = 60L;
    private static final int SAMPLES = 8;

    private static final MenuBall INSTANCE = new MenuBall();

    // Dock: where the title menu draws the logo this frame.
    private Screen dockHost;
    private float dockX;
    private float dockY;
    private float dockR;

    private boolean free;
    private float x;
    private float y;
    private float r;
    private float vx;
    private float vy;
    private float angle;
    private float spin;
    private int width;
    private int height;
    private long lastFrameMs = -1;
    private float accumulator;

    // Drag state.
    private boolean held;
    private int heldButton;
    private boolean fromDock;
    private float grabDx;
    private float grabDy;
    private float pressX;
    private float pressY;
    private float pointerX;
    private float pointerY;
    private final long[] sampleMs = new long[SAMPLES];
    private final float[] sampleX = new float[SAMPLES];
    private final float[] sampleY = new float[SAMPLES];
    private int samples;
    private int sampleHead;

    // Solid rects of the current screen (x0, y0, x1, y1 per rect), refilled per frame.
    private float[] solids = new float[64];
    private int solidCount;
    // Hand-painted buttons (AgentScreenExtras) only move on re-layout: cached per screen + size.
    private Screen paintedFor;
    private int paintedW;
    private int paintedH;
    private float[] painted = new float[0];

    private MenuBall() {}

    public static MenuBall get() {
        return INSTANCE;
    }

    /** True while the logo sits in its title-menu spot (the title draws it). */
    public boolean docked() {
        return !free;
    }

    /**
     * Title menu, every frame it draws the logo: where the logo is, so a
     * click there can knock it loose.
     */
    public void publishDock(Screen host, float cx, float cy, float radius) {
        dockHost = host;
        dockX = cx;
        dockY = cy;
        dockR = radius;
    }

    /** Client tick: a loaded world puts the logo back in its spot. */
    public void onClientTick(Minecraft minecraft) {
        if (minecraft != null && minecraft.level != null && (free || held)) {
            free = false;
            held = false;
            dockHost = null;
        }
    }

    // ── Input ───────────────────────────────────────────────────────────────

    /**
     * Raw mouse button from {@code MouseHandler.onButton}, before vanilla
     * dispatches it. Returns true when the ball took the event (the press that
     * grabbed it and the matching release), so the screen never sees it.
     */
    public boolean onMouseButton(Minecraft minecraft, int button, int action) {
        if (held) {
            if (action == InputConstants.RELEASE && button == heldButton) {
                release(minecraft);
                return true;
            }
            return action == InputConstants.PRESS;
        }
        if (action != InputConstants.PRESS
                || (button != InputConstants.MOUSE_BUTTON_LEFT && button != InputConstants.MOUSE_BUTTON_RIGHT)
                || minecraft == null || minecraft.level != null) {
            return false;
        }
        Screen screen = Compat.getScreen(minecraft);
        if (screen == null) {
            return false;
        }
        float mx = (float) minecraft.mouseHandler.getScaledXPos(minecraft.getWindow());
        float my = (float) minecraft.mouseHandler.getScaledYPos(minecraft.getWindow());
        if (free) {
            if (!inside(mx, my, x, y, r)) {
                return false;
            }
            fromDock = false;
        } else {
            if (screen != dockHost || dockR <= 0f || !inside(mx, my, dockX, dockY, dockR)) {
                return false;
            }
            free = true;
            fromDock = true;
            x = dockX;
            y = dockY;
            r = dockR;
            angle = 0f;
            width = screen.width;
            height = screen.height;
            accumulator = 0f;
        }
        held = true;
        heldButton = button;
        grabDx = x - mx;
        grabDy = y - my;
        pressX = mx;
        pressY = my;
        pointerX = mx;
        pointerY = my;
        vx = 0f;
        vy = 0f;
        samples = 0;
        sample(Util.getMillis(), mx, my);
        return true;
    }

    private void release(Minecraft minecraft) {
        held = false;
        if (ClientConfig.getInstance().reducedMotion()) {
            vx = 0f;
            vy = 0f;
            spin = 0f;
            return;
        }
        float mx = (float) minecraft.mouseHandler.getScaledXPos(minecraft.getWindow());
        float my = (float) minecraft.mouseHandler.getScaledYPos(minecraft.getWindow());
        long now = Util.getMillis();
        sample(now, mx, my);
        boolean click = Math.abs(mx - pressX) <= CLICK_SLOP && Math.abs(my - pressY) <= CLICK_SLOP;
        if (fromDock && click) {
            // Knocked out of its spot: a small hop away from the click side.
            float side = heldButton == InputConstants.MOUSE_BUTTON_RIGHT ? 1f : -1f;
            vx = side * height * 0.35f;
            vy = -height * 0.55f;
            spin = side * 6f;
            return;
        }
        // Throw: pointer velocity over the last few frames.
        int newest = (sampleHead + SAMPLES - 1) % SAMPLES;
        int oldest = newest;
        for (int i = 1; i < samples; i++) {
            int idx = (newest - i + SAMPLES) % SAMPLES;
            oldest = idx;
            if (now - sampleMs[idx] >= THROW_WINDOW_MS) {
                break;
            }
        }
        float dt = (sampleMs[newest] - sampleMs[oldest]) / 1000f;
        if (dt > 0.004f && now - sampleMs[oldest] <= THROW_WINDOW_MS * 3) {
            vx = (sampleX[newest] - sampleX[oldest]) / dt;
            vy = (sampleY[newest] - sampleY[oldest]) / dt;
            float max = MAX_THROW * Math.max(1, width);
            float speed = (float) Math.sqrt(vx * vx + vy * vy);
            if (speed > max) {
                vx *= max / speed;
                vy *= max / speed;
            }
        } else {
            vx = 0f;
            vy = 0f;
        }
    }

    private void sample(long now, float mx, float my) {
        sampleMs[sampleHead] = now;
        sampleX[sampleHead] = mx;
        sampleY[sampleHead] = my;
        sampleHead = (sampleHead + 1) % SAMPLES;
        if (samples < SAMPLES) {
            samples++;
        }
    }

    private static boolean inside(float px, float py, float cx, float cy, float radius) {
        float dx = px - cx;
        float dy = py - cy;
        return dx * dx + dy * dy <= radius * radius;
    }

    // ── Frame ───────────────────────────────────────────────────────────────

    /**
     * Steps and draws the ball on top of {@code screen}. Called at the tail of
     * the screen's render pass; only the current screen of an out-of-world
     * menu paints (screens that render their parent behind them do not
     * double-step it).
     */
    public void paint(Object gfx, Screen screen) {
        Minecraft minecraft = Minecraft.getInstance();
        if (!free || minecraft == null || minecraft.level != null
                || Compat.getScreen(minecraft) != screen) {
            lastFrameMs = -1;
            return;
        }
        long now = Util.getMillis();
        float dt = lastFrameMs < 0 ? 0f : Math.min(MAX_FRAME, (now - lastFrameMs) / 1000f);
        lastFrameMs = now;
        fit(screen.width, screen.height);
        collectSolids(screen);
        if (held) {
            pointerX = (float) minecraft.mouseHandler.getScaledXPos(minecraft.getWindow());
            pointerY = (float) minecraft.mouseHandler.getScaledYPos(minecraft.getWindow());
            sample(now, pointerX, pointerY);
            x = clamp(pointerX + grabDx, r, width - r);
            y = clamp(pointerY + grabDy, r, height - r);
            spin *= (float) Math.exp(-dt * 4f);
            angle += spin * dt;
            accumulator = 0f;
        } else if (ClientConfig.getInstance().reducedMotion()) {
            vx = 0f;
            vy = 0f;
            spin = 0f;
            resolve(false);
        } else {
            accumulator += dt;
            while (accumulator >= STEP) {
                step(STEP);
                accumulator -= STEP;
            }
        }
        Compat.uiNextLayer(gfx);
        Compat.uiPush(gfx, x, y);
        Compat.uiRotate(gfx, angle);
        int size = Math.round(r * 2f);
        Compat.uiSprite(gfx, LOGO, -size / 2, -size / 2, size, size, 0xFFFFFFFF);
        Compat.uiPop(gfx);
    }

    /** Window resized (or first frame on a screen): keep the ball proportionally in place. */
    private void fit(int w, int h) {
        if (w <= 0 || h <= 0 || (w == width && h == height)) {
            return;
        }
        if (width > 0 && height > 0) {
            x *= w / (float) width;
            y *= h / (float) height;
        }
        width = w;
        height = h;
        // A ball wider than a tiny window would wedge between the walls.
        r = Math.min(r, Math.min(w, h) * 0.25f);
        x = clamp(x, r, w - r);
        y = clamp(y, r, h - r);
    }

    private void collectSolids(Screen screen) {
        solidCount = 0;
        List<? extends GuiEventListener> children = screen.children();
        for (int i = 0, n = children.size(); i < n; i++) {
            GuiEventListener child = children.get(i);
            if ((child instanceof AbstractButton || child instanceof AbstractSliderButton)
                    && child instanceof AbstractWidget widget && widget.visible) {
                addSolid(widget.getX(), widget.getY(), widget.getWidth(), widget.getHeight());
            }
        }
        if (screen instanceof AgentScreenExtras extras) {
            if (screen != paintedFor || screen.width != paintedW || screen.height != paintedH) {
                List<AgentWidget> regions = extras.agentWidgets();
                painted = new float[regions.size() * 4];
                for (int i = 0; i < regions.size(); i++) {
                    AgentWidget w = regions.get(i);
                    painted[i * 4] = w.x();
                    painted[i * 4 + 1] = w.y();
                    painted[i * 4 + 2] = w.w();
                    painted[i * 4 + 3] = w.h();
                }
                paintedFor = screen;
                paintedW = screen.width;
                paintedH = screen.height;
            }
            for (int i = 0; i < painted.length; i += 4) {
                addSolid((int) painted[i], (int) painted[i + 1], (int) painted[i + 2], (int) painted[i + 3]);
            }
        }
    }

    private void addSolid(int sx, int sy, int sw, int sh) {
        if (sw <= 0 || sh <= 0) {
            return;
        }
        if ((solidCount + 1) * 4 > solids.length) {
            solids = java.util.Arrays.copyOf(solids, solids.length * 2);
        }
        int i = solidCount * 4;
        solids[i] = sx;
        solids[i + 1] = sy;
        solids[i + 2] = sx + sw;
        solids[i + 3] = sy + sh;
        solidCount++;
    }

    private void step(float dt) {
        vy += GRAVITY * height * dt;
        x += vx * dt;
        y += vy * dt;
        if (resolve(true)) {
            float k = (float) Math.exp(-dt * ROLL_DRAG);
            vx *= k;
            vy *= k;
        } else {
            spin *= (float) Math.exp(-dt * AIR_SPIN_DRAG);
        }
        angle += spin * dt;
        if (angle > Math.PI * 2 || angle < -Math.PI * 2) {
            angle %= (float) (Math.PI * 2);
        }
    }

    /**
     * Pushes the ball out of the walls and solid buttons. With {@code bounce}
     * the velocity reflects off each contact and the spin follows the
     * surface (rolling); without it the ball is only moved. Returns true when
     * anything was touched.
     */
    private boolean resolve(boolean bounce) {
        boolean touched = false;
        // Buttons first, walls last: the walls always win, so the ball never
        // ends a step outside the window.
        for (int i = 0; i < solidCount; i++) {
            int o = i * 4;
            touched |= hitRect(solids[o], solids[o + 1], solids[o + 2], solids[o + 3], bounce);
        }
        if (x < r) touched |= contact(r - x, 1f, 0f, BOUNCE_WALL, bounce);
        if (x > width - r) touched |= contact(x - (width - r), -1f, 0f, BOUNCE_WALL, bounce);
        if (y < r) touched |= contact(r - y, 0f, 1f, BOUNCE_WALL, bounce);
        if (y > height - r) touched |= contact(y - (height - r), 0f, -1f, BOUNCE_WALL, bounce);
        return touched;
    }

    private boolean hitRect(float x0, float y0, float x1, float y1, boolean bounce) {
        float nx;
        float ny;
        float pen;
        if (x >= x0 && x <= x1 && y >= y0 && y <= y1) {
            // Centre inside the rect: leave through the nearest edge.
            float left = x - x0;
            float right = x1 - x;
            float top = y - y0;
            float bottom = y1 - y;
            float min = Math.min(Math.min(left, right), Math.min(top, bottom));
            if (min == top) { nx = 0f; ny = -1f; }
            else if (min == bottom) { nx = 0f; ny = 1f; }
            else if (min == left) { nx = -1f; ny = 0f; }
            else { nx = 1f; ny = 0f; }
            pen = min + r;
        } else {
            float dx = x - clamp(x, x0, x1);
            float dy = y - clamp(y, y0, y1);
            float d2 = dx * dx + dy * dy;
            if (d2 >= r * r) {
                return false;
            }
            float d = (float) Math.sqrt(d2);
            nx = dx / d;
            ny = dy / d;
            pen = r - d;
        }
        return contact(pen, nx, ny, BOUNCE_BUTTON, bounce);
    }

    /** One contact: normal (nx, ny) points from the surface to the ball centre. */
    private boolean contact(float pen, float nx, float ny, float restitution, boolean bounce) {
        x += nx * pen;
        y += ny * pen;
        if (!bounce) {
            return true;
        }
        float vn = vx * nx + vy * ny;
        if (vn < 0f) {
            float out = -vn * restitution;
            if (out < REST_SPEED) {
                out = 0f;
            }
            vx += (out - vn) * nx;
            vy += (out - vn) * ny;
        }
        // Rolling: pull the spin toward the speed along the surface (no-slip
        // contact point; y points down, so positive angles turn clockwise).
        float tx = -ny;
        float ty = nx;
        float vt = vx * tx + vy * ty;
        float slip = vt - spin * r;
        spin += slip * 0.25f / r;
        vx -= slip * 0.05f * tx;
        vy -= slip * 0.05f * ty;
        return true;
    }

    private static float clamp(float v, float lo, float hi) {
        return v < lo ? lo : (v > hi ? hi : v);
    }
}
