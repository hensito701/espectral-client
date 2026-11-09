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
 * Per-wearer cosmetic motion state.
 *
 * <p>One {@link Entry} per wearer that may render something. Every client tick
 * the entry derives the driver inputs (sneak, sprint, airborne, gliding,
 * swimming, speed, yaw changes, landing, age) and the nearest
 * {@link #MAX_SIMULATED} wearers within {@link #SIM_RANGE} blocks step their
 * items' drivers. Everyone else — and everyone while cosmetics physics is off
 * or reduced motion is on — keeps the neutral rest pose, exactly as the
 * contract requires.
 *
 * <p>Driver state arrays are allocated once per loadout change; the per-tick
 * path writes into those arrays and never allocates.
 *
 * <p>Also owns the dev-only preview override:
 * {@code -Despectral.cosmetics.preview=<itemId,...>} forces the own player's
 * loadout without a grant — but only inside a development environment, never
 * in a production jar.
 */
public final class WearerSims {

    /** Dev-only loadout preview override (never honoured in production). */
    public static final String PREVIEW_PROPERTY = "espectral.cosmetics.preview";

    /** Wearers that simulate motion per tick. */
    public static final int MAX_SIMULATED = 8;
    /** Simulation range (blocks) around the viewer. */
    public static final float SIM_RANGE = 24.0f;

    private static final String[] NO_IDS = new String[0];
    private static final MotionDriver.State[][] NO_STATES = new MotionDriver.State[0][];

    private static final Map<UUID, Entry> ENTRIES = new HashMap<>();
    private static final List<Entry> SORTED = new ArrayList<>();

    private static Loadout previewLoadout;
    private static boolean previewResolved;

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
        // Suite master / per-feature toggle: master-off stays cosmetic-free.
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

    /** Per-wearer motion state; created lazily, pruned when the wearer leaves. */
    public static Entry entryFor(UUID uuid, Loadout loadout) {
        Entry e = ENTRIES.get(uuid);
        if (e == null) {
            e = new Entry(uuid);
            ENTRIES.put(uuid, e);
        }
        e.setLoadout(loadout);
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
     * Client tick: steps the nearest visible wearers with cosmetics. Cheap when
     * nothing is equipped (one loadout lookup per player).
     */
    public static void tick(Minecraft mc) {
        ItemModels.tick(mc);
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
        // Nearest-first selection; everyone else keeps the neutral rest pose.
        SORTED.sort((a, b) -> Double.compare(a.distSq, b.distSq));
        int simulated = 0;
        for (Entry e : SORTED) {
            if (e.distSq > (double) SIM_RANGE * SIM_RANGE) break;
            if (simulated >= MAX_SIMULATED) break;
            e.simulated = true;
            simulated++;
        }

        boolean physics = Cosmetics.physicsEnabled();
        for (Entry e : SORTED) {
            if (!e.simulated || !physics) {
                e.restMotion();
                continue;
            }
            e.stepMotion();
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

    /** Motion state for one wearer: derived inputs plus per-item driver arrays. */
    public static final class Entry {

        final UUID uuid;

        /** Last resolved loadout (slot-keyed, one item per slot). */
        public Loadout loadout;

        /** Driver inputs, written once per tick and read by every driver. */
        final MotionDriver.Input input = new MotionDriver.Input();

        /** Equipped model items and their driver state, both replaced on loadout change. */
        private volatile StateSet motion = new StateSet(NO_IDS, NO_STATES);

        float prevBodyRot, prevHeadRot;
        float vySmooth;
        boolean hasPrev;
        /** True while this wearer is inside the simulation set (set every tick). */
        boolean simulated;
        double distSq;

        Entry(UUID uuid) {
            this.uuid = uuid;
        }

        /** Installs a new loadout, reallocating the driver arrays only if it changed. */
        void setLoadout(Loadout loadout) {
            if (loadout == this.loadout || loadout.equals(this.loadout)) return;
            this.loadout = loadout;
            this.motion = buildState(loadout);
        }

        /** Driver state arrays of {@code itemId}, or {@code null} when it does not animate. */
        public MotionDriver.State[] statesFor(String itemId) {
            if (itemId == null) return null;
            StateSet set = motion;
            for (int i = 0; i < set.ids.length; i++) {
                if (itemId.equals(set.ids[i])) return set.states[i];
            }
            return null;
        }

        // ---------------- per-tick motion ----------------

        /**
         * Derives the driver inputs from the publicly visible pose state. Runs
         * for every wearer with a loadout, whether or not it simulates: the
         * yaw deltas must not jump when a wearer enters the simulation set.
         */
        void updateMotion(Player p) {
            float bodyRot = p.yBodyRot;
            float headRot = p.yHeadRot;
            float yawDelta = 0f, headYawDelta = 0f;
            if (hasPrev) {
                yawDelta = Mth.wrapDegrees(bodyRot - prevBodyRot);
                headYawDelta = Mth.wrapDegrees(headRot - prevHeadRot);
            } else {
                hasPrev = true;
            }
            prevBodyRot = bodyRot;
            prevHeadRot = headRot;

            float vy = (float) p.getDeltaMovement().y;
            boolean landing = vySmooth < -0.22f && vy > -0.05f;
            vySmooth += (vy - vySmooth) * 0.5f;

            boolean gliding = p.isFallFlying();
            boolean swimming = p.getPose() == net.minecraft.world.entity.Pose.SWIMMING;

            float yaw = (float) Math.toRadians(bodyRot);
            float forward = (float) (-p.getDeltaMovement().x * Math.sin(yaw)
                    + p.getDeltaMovement().z * Math.cos(yaw));

            input.sneak = p.isCrouching();
            input.sprint = p.isSprinting();
            input.glide = gliding;
            input.swim = swimming;
            input.airborne = !p.onGround();
            input.speed = forward;
            input.fallSpeed = Math.max(0f, -vy);
            input.yawDelta = yawDelta;
            input.headYawDelta = headYawDelta;
            input.landing = landing;
            input.ageInTicks = p.tickCount;
        }

        /** Steps every equipped item's drivers one client tick. */
        void stepMotion() {
            StateSet set = motion;
            for (int i = 0; i < set.ids.length; i++) {
                String id = set.ids[i];
                MotionDriver.State[] states = set.states[i];
                // A hot reload can change how many drivers an item declares;
                // resize that one array instead of dropping its extra drivers.
                if (ItemModels.specCount(id) != states.length) {
                    states = newArray(id);
                    set.states[i] = states;
                }
                ItemModels.step(id, states, input);
            }
        }

        /** Pins every equipped item to its neutral rest pose. */
        void restMotion() {
            StateSet set = motion;
            for (int i = 0; i < set.ids.length; i++) {
                ItemModels.rest(set.ids[i], set.states[i]);
            }
        }

        private static StateSet buildState(Loadout loadout) {
            if (loadout == null || loadout.isEmpty()) return new StateSet(NO_IDS, NO_STATES);
            String[] slots = {"espalda", "cuello", "cabeza"};
            List<String> ids = new ArrayList<>(slots.length);
            List<MotionDriver.State[]> states = new ArrayList<>(slots.length);
            for (String slot : slots) {
                String id = loadout.forSlot(slot);
                if (id == null || ItemModels.specCount(id) <= 0) continue;
                ids.add(id);
                states.add(newArray(id));
            }
            return new StateSet(ids.toArray(new String[0]), states.toArray(new MotionDriver.State[0][]));
        }

        /** Fresh driver state, one per declared driver, at the item's rest pose. */
        private static MotionDriver.State[] newArray(String id) {
            MotionDriver.State[] array = new MotionDriver.State[ItemModels.specCount(id)];
            for (int i = 0; i < array.length; i++) array[i] = new MotionDriver.State();
            ItemModels.rest(id, array);
            return array;
        }
    }

    /** Immutable pair of item ids and their driver arrays (one reference write swaps it). */
    private record StateSet(String[] ids, MotionDriver.State[][] states) { }
}
