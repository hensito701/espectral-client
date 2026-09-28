package es.spectral.menu.cosmetics.render;

import java.util.ArrayList;
import java.util.HashMap;
import java.util.Iterator;
import java.util.LinkedHashMap;
import java.util.List;
import java.util.Locale;
import java.util.Map;
import java.util.Set;
import java.util.UUID;

import net.fabricmc.loader.api.FabricLoader;
import net.minecraft.client.Minecraft;
import net.minecraft.client.player.AbstractClientPlayer;
import net.minecraft.util.Mth;
import net.minecraft.world.entity.player.Player;

import es.spectral.menu.cosmetics.Catalog;
import es.spectral.menu.cosmetics.Cosmetics;
import es.spectral.menu.cosmetics.Loadout;

/**
 * Per-wearer cloth/pose simulation state for the cosmetic renderers.
 *
 * <p>One {@link Entry} per wearer that may render something. The simulation
 * runs on the client tick in the wearer's model frame (the frame a render
 * layer writes into: +X wearer's left, +Y down, +Z behind, origin at the model
 * root). Only the nearest {@link #MAX_SIMULATED} wearers with a loadout within
 * {@link #SIM_RANGE} blocks of the viewer simulate; everyone else keeps a
 * static pose. {@code personalization.reduced_motion} pins everything to the
 * static rest pose.
 *
 * <p>Also owns the dev-only preview override:
 * {@code -Despectral.cosmetics.preview=<itemId,...>} forces the own player's
 * loadout without a grant — but only inside a development environment, never
 * in a production jar.
 */
public final class WearerSims {

    /** Dev-only loadout preview override (never honoured in production). */
    public static final String PREVIEW_PROPERTY = "espectral.cosmetics.preview";

    /** Wearers that simulate physics per tick. */
    public static final int MAX_SIMULATED = 8;
    /** Simulation range (blocks) around the viewer. */
    public static final float SIM_RANGE = 24.0f;

    /** Cloth footprint cap: nothing lower than the knees (model units). */
    public static final float CAP_Y = ClothSim.CAP_KNEES;
    /** Cloth footprint cap: nothing above the head top (model units). */
    public static final float HEAD_TOP = ClothSim.CAP_HEAD_TOP;
    /** Cloth footprint cap: shoulder width + 1 block (model units). */
    public static final float CAP_X = ClothSim.CAP_X;

    /** Wind response: cloth push per (blocks/tick) of wearer speed. */
    private static final float WIND_K = 0.16f;
    /** Vanilla crouch body tilt (HumanoidModel#setupAnim sets 0.5 rad). */
    private static final float CROUCH_TILT = 0.5f;

    private static final Map<UUID, Entry> ENTRIES = new HashMap<>();
    private static final List<Entry> SORTED = new ArrayList<>();

    private static Loadout previewLoadout;
    private static boolean previewResolved;

    // Model-frame rest anchors (blocks, model root = neck line).
    private static final float[][] MANTLE_TOP = {
            {-0.32f, -0.34f, -0.12f},
            {-0.26f, -0.24f,  0.12f},
            {-0.02f, -0.10f,  0.17f},
            { 0.22f,  0.02f,  0.12f},
            { 0.30f,  0.10f, -0.10f},
    };
    private static final float[][] CLOAK_TOP = {
            {-0.30f, -0.10f, -0.14f},
            {-0.24f, -0.06f,  0.14f},
            { 0.00f, -0.02f,  0.18f},
            { 0.24f, -0.06f,  0.14f},
            { 0.30f, -0.10f, -0.14f},
    };
    private static final float[][] HEM_TOP = {
            {-0.13f, 0.80f, 0.15f},
            { 0.13f, 0.80f, 0.15f},
    };
    private static final float[][] TATTER_TOP = {
            {-0.17f, 0.12f, 0.16f},
            { 0.00f, 0.18f, 0.19f},
            { 0.17f, 0.12f, 0.16f},
    };
    private static final float[][] SCARF_TOP = {
            {-0.11f, -0.18f, 0.12f},
            { 0.11f, -0.18f, 0.12f},
    };

    private WearerSims() {}

    // ---------------------------------------------------------------
    // Loadout resolution (preview-aware)
    // ---------------------------------------------------------------

    /**
     * Verified loadout for the wearer, or {@code null} when nothing may render.
     * In a development environment {@code -Despectral.cosmetics.preview}
     * forces the own player's loadout without a grant.
     */
    public static Loadout loadoutFor(UUID uuid) {
        // The preview forces the loadout without a grant, but never past the
        // Suite master / per-feature toggle: master-off must stay cosmetic-free.
        if (!es.spectral.menu.ClientConfig.getInstance()
                .isFeatureEnabled(Cosmetics.FEATURE_COSMETICS)) {
            return null;
        }
        if (previewActive() && isSelf(uuid)) {
            Loadout preview = previewLoadout();
            return preview != null ? preview : Cosmetics.loadoutFor(uuid);
        }
        return Cosmetics.loadoutFor(uuid);
    }

    /** True when the dev preview override may apply at all (dev env only). */
    public static boolean previewActive() {
        return FabricLoader.getInstance().isDevelopmentEnvironment()
                && System.getProperty(PREVIEW_PROPERTY) != null
                && !System.getProperty(PREVIEW_PROPERTY).isBlank();
    }

    private static boolean isSelf(UUID uuid) {
        Minecraft mc = Minecraft.getInstance();
        return mc != null && mc.player != null && uuid.equals(mc.player.getUUID());
    }

    /**
     * Parses the preview property into a one-item-per-slot loadout through the
     * MOD-CORE {@link Catalog} (item existence, slot placement and the
     * catalog's exclusivity rules are the catalog's business — this only picks
     * the ids and pretends the wearer carries the MVP flags a real grant
     * would). Resolved lazily on the game thread; no I/O on the render thread.
     */
    public static Loadout previewLoadout() {
        if (previewResolved) return previewLoadout;
        previewResolved = true;
        String prop = System.getProperty(PREVIEW_PROPERTY);
        if (prop == null || prop.isBlank()) return null;
        Map<String, String> bySlot = new LinkedHashMap<>(5);
        for (String raw : prop.split(",")) {
            String id = raw.trim().toLowerCase(Locale.ROOT);
            if (id.isEmpty()) continue;
            Catalog.Item item = Catalog.get().item(id);
            if (item == null) continue;
            bySlot.putIfAbsent(item.slot(), id); // one item per slot
        }
        previewLoadout = Catalog.get().filter(
                Catalog.get().fromMap(bySlot), Set.of("supporter", "mvp"));
        return previewLoadout;
    }

    // ---------------------------------------------------------------
    // Entry lookup
    // ---------------------------------------------------------------

    /** Per-wearer sim state; created lazily, pruned when the wearer leaves. */
    public static Entry entryFor(UUID uuid, Loadout loadout) {
        Entry e = ENTRIES.get(uuid);
        if (e == null) {
            e = new Entry(uuid);
            ENTRIES.put(uuid, e);
        }
        e.loadout = loadout;
        return e;
    }

    /** Drops every wearer entry (world change / disconnect). */
    public static void clear() {
        ENTRIES.clear();
    }

    /** Read-only lookup for the render path (draw-time posing). */
    public static Entry lookup(UUID uuid) {
        return uuid == null ? null : ENTRIES.get(uuid);
    }

    /** Wearer uuid behind a render state, or null (not a player / no level). */
    public static UUID uuidOf(net.minecraft.client.renderer.entity.state.AvatarRenderState state) {
        Minecraft mc = Minecraft.getInstance();
        if (mc == null || mc.level == null || state == null) return null;
        net.minecraft.world.entity.Entity e = mc.level.getEntity(state.id);
        return e instanceof Player p ? p.getUUID() : null;
    }

    // ---------------------------------------------------------------
    // Tick driver
    // ---------------------------------------------------------------

    /**
     * Client tick: simulates the nearest visible wearers with cosmetics.
     * Cheap when nothing is equipped (one loadout lookup per player).
     */
    public static void tick(Minecraft mc) {
        if (previewActive() && !previewResolved) previewLoadout();
        if (mc == null || mc.level == null || mc.player == null) {
            if (!ENTRIES.isEmpty()) ENTRIES.clear();
            return;
        }
        List<AbstractClientPlayer> players = mc.level.players();
        double px = mc.player.getX(), py = mc.player.getY(), pz = mc.player.getZ();

        // Refresh entries + collect simulation candidates.
        SORTED.clear();
        for (AbstractClientPlayer p : players) {
            Loadout loadout = loadoutFor(p.getUUID());
            if (loadout == null) {
                ENTRIES.remove(p.getUUID());
                continue;
            }
            Entry e = entryFor(p.getUUID(), loadout);
            e.updateMotion(p);
            double dx = p.getX() - px, dy = p.getY() - py, dz = p.getZ() - pz;
            e.distSq = dx * dx + dy * dy + dz * dz;
            e.simulated = false;
            SORTED.add(e);
        }
        // Nearest-first selection; everyone else keeps a static pose.
        SORTED.sort((a, b) -> Double.compare(a.distSq, b.distSq));
        int simulated = 0;
        for (Entry e : SORTED) {
            if (e.distSq > (double) SIM_RANGE * SIM_RANGE) break;
            if (simulated >= MAX_SIMULATED) break;
            e.simulated = true;
            simulated++;
        }

        // Step the simulated wearers; everyone else freezes at a static pose.
        for (Entry e : SORTED) {
            if (!e.simulated) {
                e.freezeCloth();
                continue;
            }
            Player wearer = e.find(mc);
            if (wearer == null) continue;
            e.step(wearer);
        }

        // Prune entries for wearers no longer present.
        if (ENTRIES.size() > SORTED.size()) {
            Iterator<Map.Entry<UUID, Entry>> it = ENTRIES.entrySet().iterator();
            while (it.hasNext()) {
                if (!containsEntry(SORTED, it.next().getValue())) it.remove();
            }
        }
    }

    private static boolean containsEntry(List<Entry> list, Entry e) {
        for (int i = 0; i < list.size(); i++) if (list.get(i) == e) return true;
        return false;
    }

    // ---------------------------------------------------------------
    // Per-wearer state
    // ---------------------------------------------------------------

    /** Cloth renderers + sim chains for one wearer. */
    public static final class Entry {

        final UUID uuid;

        /** Last resolved loadout (slot-keyed, one item per slot). */
        public Loadout loadout;

        // Cloth primitives (created per equipped item).
        public ClothSim.Sheet mantle, cloak;
        public ClothSim.Chain[] hoodTails = new ClothSim.Chain[2];
        public ClothSim.Chain[] tatters = new ClothSim.Chain[3];
        public ClothSim.Chain[] scarfTails = new ClothSim.Chain[2];
        public ClothSim.Chain[] wrapEnds = new ClothSim.Chain[2];

        // Reusable deferred renderers, one per submitted piece.
        public final CosmeticGeo.Sheet mantleRenderer = new CosmeticGeo.Sheet();
        public final CosmeticGeo.Sheet cloakRenderer = new CosmeticGeo.Sheet();
        public final CosmeticGeo.Ribbon[] hoodTailRenderers = ribbons(2);
        public final CosmeticGeo.Ribbon[] tatterRenderers = ribbons(3);
        public final CosmeticGeo.Ribbon[] scarfTailRenderers = ribbons(2);
        public final CosmeticGeo.Ribbon[] wrapEndRenderers = ribbons(2);

        // Motion bookkeeping (model frame).
        float prevBodyRot, prevHeadRot;
        float tilt;               // smoothed body tilt (rad) for anchors
        float flyBlend, swimBlend, flyTicks;
        float walkPos, walkSpeed, swingR, swingL;
        float vySmooth;
        /** Ear twitch impulse, decaying. */
        public float earTwitch;
        /** Ear landing flop impulse, decaying. */
        public float earFlop;
        /** Wing open amount 0..1 (smoothed per tick). */
        public float wingOpen = 0.12f;

        boolean hasPrev;
        boolean simulated;
        float lastSteppedBodyRot;
        double distSq;

        // Static per-item anchor state (rest pose).
        float[] anchorX, anchorY, anchorZ;      // sheet top rows (rotated)
        float[] wrapX = new float[2], wrapY = new float[2], wrapZ = new float[2];

        private final float[] vtmp = new float[3];

        Entry(UUID uuid) {
            this.uuid = uuid;
        }

        Player find(Minecraft mc) {
            if (mc.level == null) return null;
            for (AbstractClientPlayer p : mc.level.players()) {
                if (uuid.equals(p.getUUID())) return p;
            }
            return null;
        }

        // ---------------- per-tick motion ----------------

        void updateMotion(Player p) {
            float bodyRot = p.yBodyRot;
            float headRot = p.yHeadRot;
            if (hasPrev) {
                float dy = Mth.wrapDegrees(bodyRot - prevBodyRot);
                float dh = Mth.wrapDegrees(headRot - prevHeadRot);
                // Ear twitch follows head-yaw deltas.
                earTwitch = Mth.clamp(earTwitch * 0.78f + dh * 0.035f, -0.6f, 0.6f);
            } else {
                prevBodyRot = bodyRot;
                prevHeadRot = headRot;
                hasPrev = true;
            }
            prevBodyRot = bodyRot;
            prevHeadRot = headRot;

            float vy = (float) p.getDeltaMovement().y;
            // Landing flop: vertical speed collapses while falling.
            if (vySmooth < -0.22f && vy > -0.05f) earFlop = 0.7f;
            vySmooth += (vy - vySmooth) * 0.5f;

            boolean gliding = p.isFallFlying();
            flyTicks = gliding ? flyTicks + 1f : 0f;
            flyBlend += ((gliding ? 1f : 0f) - flyBlend) * (gliding ? 0.12f : 0.08f);
            swimBlend += ((p.getPose() == net.minecraft.world.entity.Pose.SWIMMING ? 1f : 0f) - swimBlend) * 0.18f;
            tilt += ((p.isCrouching() ? CROUCH_TILT : 0f) - tilt) * 0.35f;
            earTwitch *= 0.9f;
            earFlop *= 0.82f;

            walkPos = p.walkAnimation.position();
            walkSpeed = p.walkAnimation.speed();
            swingR = Mth.cos(walkPos * 0.6662f + (float) Math.PI) * walkSpeed * 1.15f;
            swingL = Mth.cos(walkPos * 0.6662f) * walkSpeed * 1.15f;

            // Wing target: open on fall/jump/glide, beat-ish while sprinting.
            float target;
            if (p.isCrouching()) target = 0.06f;
            else if (gliding) target = 1.0f;
            else if (vySmooth < -0.12f || vySmooth > 0.22f) target = 0.85f;
            else if (p.isSprinting()) target = 0.55f;
            else target = 0.12f;
            wingOpen += (target - wingOpen) * 0.22f;
        }

        // ---------------- simulation ----------------

        void step(Player p) {
            boolean still = Cosmetics.reducedMotion();
            float bodyRot = p.yBodyRot;
            // Carry the stored cloth through the body's yaw change since the
            // previous tick (points live in the body-rotating model frame).
            float delta = (float) Math.toRadians(Mth.wrapDegrees(bodyRot - lastSteppedBodyRot));
            if (delta != 0) yawCarry(-delta);
            lastSteppedBodyRot = bodyRot;

            // Frame tilt (glide / swim) — gravity expressed in the model frame.
            float theta = flyBlend * (float) Math.toRadians(-90f - p.getXRot())
                    * Mth.clamp(flyTicks * flyTicks / 100f, 0f, 1f)
                    + swimBlend * (float) Math.toRadians(-90f);
            float downY = Mth.cos(theta);
            float downZ = Mth.sin(theta);

            // Wearer velocity expressed in the model frame drives the wind.
            worldToModel(p, vtmp);
            float wx = -vtmp[0] * WIND_K, wy = -vtmp[1] * WIND_K, wz = -vtmp[2] * WIND_K;

            float damp = still ? 1f : ClothSim.DAMP;

            // Anchor rows/points, tilted with the body (crouch).
            float ct = Mth.cos(tilt), st = Mth.sin(tilt);

            if (mantle != null) {
                anchors(MANTLE_TOP, ct, st);
                if (still) {
                    mantle.reset(anchorX, anchorY, anchorZ, 0, 1, 0);
                } else {
                    mantle.step(anchorX, anchorY, anchorZ,
                            0, downY, downZ, wx, wy, wz,
                            damp, 0.004f, 0.0f, p.tickCount, -0.30f, CAP_Y);
                }

            }
            if (cloak != null) {
                anchors(CLOAK_TOP, ct, st);
                if (still) {
                    cloak.reset(anchorX, anchorY, anchorZ, 0, 1, 0);
                } else {
                    cloak.step(anchorX, anchorY, anchorZ,
                            0, downY, downZ, wx, wy, wz,
                            damp, 0.004f, 1.7f, p.tickCount, -0.18f, CAP_Y);
                }

                stepChains(hoodTails, HEM_TOP, ct, st, downY, downZ, wx, wy, wz, damp, still, p, 0.06f, 0.006f);
            }
            stepChains(tatters, TATTER_TOP, ct, st, downY, downZ, wx, wy, wz, damp, still, p, 0.05f, 0.008f);
            stepChains(scarfTails, SCARF_TOP, ct, st, downY, downZ, wx, wy, wz, damp, still, p, 0.04f, 0.007f);
            stepWrapEnds(p, downY, downZ, wx, wy, wz, damp, still);
        }

        private void stepChains(ClothSim.Chain[] chains, float[][] top, float ct, float st,
                float downY, float downZ, float wx, float wy, float wz,
                float damp, boolean still, Player p, float minZ, float flutter) {
            for (int i = 0; i < chains.length; i++) {
                ClothSim.Chain c = chains[i];
                if (c == null) continue;
                float ax = top[i][0], ay = top[i][1], az = top[i][2];
                float ry = ay * ct - az * st;
                float rz = ay * st + az * ct;
                if (still) {
                    c.reset(ax, ry, rz, 0, 1, 0);
                } else {
                    c.step(ax, ry, rz, 0, downY, downZ, wx, wy, wz,
                            damp, flutter, i * 2.1f, p.tickCount, minZ, CAP_Y);
                }
            }
        }

        private void stepWrapEnds(Player p, float downY, float downZ,
                float wx, float wy, float wz, float damp, boolean still) {
            for (int i = 0; i < 2; i++) {
                ClothSim.Chain c = wrapEnds[i];
                if (c == null) continue;
                updateWrapAnchor(p, i);
                float ax = wrapX[i], ay = wrapY[i], az = wrapZ[i];
                if (still) {
                    c.reset(ax, ay, az, 0, 1, 0);
                } else {
                    c.step(ax, ay, az, 0, downY, downZ, wx * 0.6f, wy * 0.6f, wz * 0.6f,
                            damp, 0.004f, i * 1.9f, p.tickCount, -1000f, CAP_Y);
                }
            }
        }

        /** Wrist anchor in the model frame; follows the vanilla arm swing. */
        private void updateWrapAnchor(Player p, int side) {
            boolean right = side == 0;
            float swing = right ? swingR : swingL;
            float shoulderX = right ? -0.3125f : 0.3125f;
            float armY = 0.125f + 0.5f * Mth.cos(swing);
            float armZ = 0.5f * Mth.sin(swing);
            wrapX[side] = shoulderX;
            wrapY[side] = armY;
            wrapZ[side] = armZ;
        }

        private void anchors(float[][] rest, float ct, float st) {
            int n = rest.length;
            if (anchorX == null || anchorX.length != n) {
                anchorX = new float[n];
                anchorY = new float[n];
                anchorZ = new float[n];
            }
            for (int i = 0; i < n; i++) {
                float ax = rest[i][0], ay = rest[i][1], az = rest[i][2];
                anchorX[i] = ax;
                anchorY[i] = ay * ct - az * st;
                anchorZ[i] = ay * st + az * ct;
            }
        }

        /** Static pose: freezes every cloth piece's render lerp. */
        void freezeCloth() {
            if (mantle != null) mantle.freeze();
            if (cloak != null) cloak.freeze();
            freezeChains(hoodTails);
            freezeChains(tatters);
            freezeChains(scarfTails);
            freezeChains(wrapEnds);
        }

        private static void freezeChains(ClothSim.Chain[] chains) {
            for (ClothSim.Chain c : chains) if (c != null) c.freeze();
        }

        /** Rotates every stored chain through a body-yaw delta (model frame). */
        private void yawCarry(float rad) {
            if (mantle != null) mantle.yawAll(rad);
            if (cloak != null) cloak.yawAll(rad);
            yawChains(rad, hoodTails);
            yawChains(rad, tatters);
            yawChains(rad, scarfTails);
            yawChains(rad, wrapEnds);
        }

        private static void yawChains(float rad, ClothSim.Chain[] chains) {
            for (ClothSim.Chain c : chains) if (c != null) c.yawAll(rad);
        }

        /**
         * Wearer velocity (blocks/tick) rotated into the wearer's model frame:
         * inverse body yaw, inverse frame tilt, then the renderer's axis flip.
         */
        private void worldToModel(Player p, float[] out) {
            float vx = (float) p.getDeltaMovement().x;
            float vy = (float) p.getDeltaMovement().y;
            float vz = (float) p.getDeltaMovement().z;
            float yaw = (float) Math.toRadians(p.yBodyRot - 180f);
            float cy = Mth.cos(yaw), sy = Mth.sin(yaw);
            float x1 = vx * cy + vz * sy;
            float z1 = -vx * sy + vz * cy;
            float theta = flyBlend * (float) Math.toRadians(-90f - p.getXRot())
                    * Mth.clamp(flyTicks * flyTicks / 100f, 0f, 1f)
                    + swimBlend * (float) Math.toRadians(-90f);
            float ct = Mth.cos(-theta), st = Mth.sin(-theta);
            float y2 = vy * ct - z1 * st;
            float z2 = vy * st + z1 * ct;
            out[0] = -x1;
            out[1] = -y2;
            out[2] = z2;
        }

        // ---------------- chain lifecycle ----------------

        /** Ensures the cloth primitives for the current loadout exist. */
        public void ensureCloth() {
            Loadout l = loadout;
            if (l == null) return;
            String espalda = l.espalda();
            if ("manto_espectral".equals(espalda)) {
                if (mantle == null) {
                    mantle = new ClothSim.Sheet(5, 7, 0.27f, 0.15f);
                    mantle.reset(MANTLE_TOP, 0, 1, 0);
                }
            } else {
                mantle = null;
            }
            if ("capucha_errante".equals(espalda)) {
                if (cloak == null) {
                    cloak = new ClothSim.Sheet(5, 6, 0.30f, 0.14f);
                    cloak.reset(CLOAK_TOP, 0, 1, 0);
                    for (int i = 0; i < 2; i++) {
                        hoodTails[i] = new ClothSim.Chain(3, 0.12f);
                        hoodTails[i].reset(HEM_TOP[i][0], HEM_TOP[i][1], HEM_TOP[i][2], 0, 1, 0);
                    }
                }
            } else {
                if (cloak != null) {
                    cloak = null;
                    hoodTails[0] = hoodTails[1] = null;
                }
            }
            if ("jirones_ceniza".equals(espalda)) {
                if (tatters[0] == null) {
                    for (int i = 0; i < 3; i++) {
                        tatters[i] = new ClothSim.Chain(6, 0.16f);
                        tatters[i].reset(TATTER_TOP[i][0], TATTER_TOP[i][1], TATTER_TOP[i][2], 0, 1, 0);
                    }
                }
            } else if (tatters[0] != null) {
                tatters[0] = tatters[1] = tatters[2] = null;
            }
            if ("bufanda_llama_alma".equals(l.cuello())) {
                if (scarfTails[0] == null) {
                    for (int i = 0; i < 2; i++) {
                        scarfTails[i] = new ClothSim.Chain(5, 0.14f);
                        scarfTails[i].reset(SCARF_TOP[i][0], SCARF_TOP[i][1], SCARF_TOP[i][2], 0, 1, 0);
                    }
                }
            } else if (scarfTails[0] != null) {
                scarfTails[0] = scarfTails[1] = null;
            }
            if ("vendas_runicas".equals(l.brazos())) {
                if (wrapEnds[0] == null) {
                    for (int i = 0; i < 2; i++) {
                        wrapEnds[i] = new ClothSim.Chain(3, 0.08f);
                        wrapEnds[i].reset(0, 0.6f, 0, 0, 1, 0);
                    }
                }
            } else if (wrapEnds[0] != null) {
                wrapEnds[0] = wrapEnds[1] = null;
            }
        }
    }

    private static CosmeticGeo.Ribbon[] ribbons(int n) {
        CosmeticGeo.Ribbon[] out = new CosmeticGeo.Ribbon[n];
        for (int i = 0; i < n; i++) out[i] = new CosmeticGeo.Ribbon();
        return out;
    }
}
