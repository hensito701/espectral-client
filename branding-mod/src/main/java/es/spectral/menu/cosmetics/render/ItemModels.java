package es.spectral.menu.cosmetics.render;

import java.io.InputStream;
import java.io.InputStreamReader;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.ArrayList;
import java.util.HashMap;
import java.util.LinkedHashMap;
import java.util.List;
import java.util.Map;
import java.util.Set;
import java.util.concurrent.ConcurrentHashMap;

import org.slf4j.Logger;
import org.slf4j.LoggerFactory;

import com.google.gson.JsonArray;
import com.google.gson.JsonElement;
import com.google.gson.JsonObject;
import com.google.gson.JsonParser;
import com.mojang.blaze3d.platform.NativeImage;

import net.minecraft.client.Minecraft;
import net.minecraft.client.model.Model;
import net.minecraft.client.model.geom.ModelPart;
import net.minecraft.client.model.geom.PartPose;
import net.minecraft.client.model.geom.builders.CubeDeformation;
import net.minecraft.client.model.geom.builders.CubeListBuilder;
import net.minecraft.client.model.geom.builders.LayerDefinition;
import net.minecraft.client.model.geom.builders.MeshDefinition;
import net.minecraft.client.model.geom.builders.PartDefinition;
import net.minecraft.client.renderer.entity.state.AvatarRenderState;
import net.minecraft.client.renderer.texture.DynamicTexture;
import net.minecraft.resources.Identifier;
import net.minecraft.util.Util;

import es.spectral.menu.cosmetics.Catalog;

/**
 * The generic cosmetic renderer's asset layer: it loads
 * {@code assets/espectral-menu/cosmetics/models/<id>.json} for every catalog
 * item that ships one, bakes one {@link ModelPart} tree per attachment part and
 * caches the result (contract: {@code docs/cosmetics-pipeline.md}, "Compiled
 * model file").
 *
 * <p><b>Nothing item-specific lives in Java.</b> A compiled file already holds
 * vanilla model-space positions and radian rotations, so the loader only feeds
 * them to {@link CubeListBuilder} / {@link PartPose}. Motion comes from the
 * file's {@code motion} blocks — the driver table {@link MotionDriver}
 * implements — so an item's animation is authored in its {@code item.json}.
 *
 * <p><b>Hot reload.</b> With {@code -Despectral.cosmetics.dir=<folder>} the mod
 * rechecks file times once per second and rebuilds the items that changed. The
 * folder holds {@code models/<id>.json} and {@code textures/<id>.png}; a whole
 * resource tree ({@code <folder>/assets/espectral-menu/cosmetics/models/…}) is
 * accepted too. Only ids the catalog already lists are replaced, and the
 * property grants nothing.
 *
 * <p>A missing or broken file logs once and renders nothing for that item, so
 * a bad model never crashes a client.
 */
public final class ItemModels {

    /** Hot-reload folder property. */
    public static final String DIR_PROPERTY = "espectral.cosmetics.dir";

    private static final Logger LOGGER = LoggerFactory.getLogger("espectral-client");

    private static final String MODEL_PATH = "assets/espectral-menu/cosmetics/models/";
    /** Dev textures use their own path so a reload never shadows the bundled file. */
    private static final String DEV_TEXTURE_PATH = "textures/cosmetics_dev/";

    /** Attachment part names, in {@code PlayerModel} field order. */
    static final String[] PARTS = {
            "head", "body", "right_arm", "left_arm", "right_leg", "left_leg"};

    /** Slots that draw geometry (elytra skins do not). */
    private static final Set<String> EQUIPPABLE_SLOTS = Set.of("espalda", "cuello", "cabeza");

    private static final Map<String, Item> ITEMS = new ConcurrentHashMap<>();
    private static final Map<String, Identifier> DEV_TEXTURES = new ConcurrentHashMap<>();
    /** File times of the hot-reload folder, touched only on the client tick. */
    private static final Map<String, Long> MTIMES = new HashMap<>();

    private static Path devDir;
    private static long lastScanMs;
    private static boolean warmed;

    private ItemModels() {}

    // ---------------------------------------------------------------
    // Render-path lookups (allocation-free in steady state)
    // ---------------------------------------------------------------

    /** The baked item, or {@code null} when it has no usable model. */
    public static Item item(String id) {
        if (id == null) return null;
        Item cached = ITEMS.get(id);
        if (cached != null) return cached.missing ? null : cached;
        Item loaded = loadBundled(id);
        if (loaded == null) {
            Catalog.Item catalogItem = Catalog.get().item(id);
            // Elytra skins are textures, not geometry: only a drawn slot that
            // ships no model is a real gap worth one line in the log.
            if (catalogItem != null && EQUIPPABLE_SLOTS.contains(catalogItem.slot())) {
                LOGGER.warn("[espectral] cosmetic {} has no model file; it renders nothing", id);
            }
        }
        ITEMS.put(id, loaded == null ? Item.MISSING : loaded);
        return loaded;
    }

    /** Number of motion-driver states one wearer needs for {@code id}. */
    public static int specCount(String id) {
        Item item = item(id);
        return item == null ? 0 : item.specs.length;
    }

    /** Texture to draw {@code id} with: hot-reload override, else the bundled file. */
    public static Identifier texture(String id) {
        Identifier dev = DEV_TEXTURES.get(id);
        return dev != null ? dev : CosmeticTextures.texture(id);
    }

    /** Restores every driver of {@code id} to its neutral pose. */
    public static void rest(String id, MotionDriver.State[] states) {
        Item item = item(id);
        if (item == null || states == null) return;
        int n = Math.min(item.specs.length, states.length);
        for (int i = 0; i < n; i++) MotionDriver.neutral(item.specs[i], states[i]);
    }

    /** Steps every driver of {@code id} one client tick. */
    public static void step(String id, MotionDriver.State[] states, MotionDriver.Input in) {
        Item item = item(id);
        if (item == null || states == null) return;
        int n = Math.min(item.specs.length, states.length);
        for (int i = 0; i < n; i++) MotionDriver.step(item.specs[i], states[i], in);
    }

    // ---------------------------------------------------------------
    // Hot reload (-Despectral.cosmetics.dir)
    // ---------------------------------------------------------------

    /** Client tick: stat-checks the hot-reload folder at most once per second. */
    public static void tick(Minecraft minecraft) {
        warm();
        String prop = System.getProperty(DIR_PROPERTY);
        if (prop == null || prop.isBlank()) {
            if (devDir != null) closeDev();
            return;
        }
        long now = Util.getMillis();
        if (now - lastScanMs < 1000L) return;
        lastScanMs = now;
        Path dir = resolveDir(prop);
        if (dir == null) {
            if (devDir != null) closeDev();
            return;
        }
        if (!dir.equals(devDir)) {
            closeDev();
            devDir = dir;
            LOGGER.info("[espectral] cosmetics hot reload watching {}", dir);
        }
        try {
            scan(dir);
        } catch (RuntimeException e) {
            LOGGER.warn("[espectral] cosmetics hot reload failed: {}", e.toString());
        }
    }

    /** Parses every bundled model once, on the first tick (game thread, no I/O on draw). */
    private static void warm() {
        if (warmed) return;
        warmed = true;
        for (Catalog.Item item : Catalog.get().items()) {
            if (!EQUIPPABLE_SLOTS.contains(item.slot())) continue;
            item(item.id());
        }
    }

    private static Path resolveDir(String prop) {
        try {
            Path path = Path.of(prop.trim());
            return Files.isDirectory(path) ? path : null;
        } catch (RuntimeException e) {
            return null;
        }
    }

    /** Reloads every catalog item whose model or texture file changed. */
    private static void scan(Path dir) {
        Path models = dir.resolve("models");
        Path textures = dir.resolve("textures");
        if (!Files.isDirectory(models)) {
            // Convenience: the property may point at a resource tree root.
            Path assets = dir.resolve("assets").resolve("espectral-menu");
            if (!Files.isDirectory(assets.resolve("cosmetics").resolve("models"))) return;
            models = assets.resolve("cosmetics").resolve("models");
            textures = assets.resolve("textures").resolve("cosmetics");
        }
        for (Catalog.Item catalogItem : Catalog.get().items()) {
            String id = catalogItem.id();
            // Elytra skins are textures chosen by CosmeticElytra from the jar;
            // only the slots this layer draws take dev files.
            if (!EQUIPPABLE_SLOTS.contains(catalogItem.slot())) continue;
            scanModel(id, models.resolve(id + ".json"));
            scanTexture(id, textures.resolve(id + ".png"));
        }
    }

    private static void scanModel(String id, Path file) {
        String key = "m:" + id;
        long mtime = mtime(file);
        Long prev = MTIMES.get(key);
        if (mtime <= 0L) {
            if (prev != null) {
                MTIMES.remove(key);
                ITEMS.remove(id); // dev model removed: fall back to the bundled one
                LOGGER.info("[espectral] cosmetics hot reload dropped model {}", id);
            }
            return;
        }
        if (prev != null && prev == mtime) return;
        MTIMES.put(key, mtime);
        Item loaded = loadFile(id, file);
        ITEMS.put(id, loaded == null ? Item.MISSING : loaded);
        LOGGER.info("[espectral] cosmetics hot reload {} model {}", loaded == null ? "rejected" : "reloaded", id);
    }

    private static void scanTexture(String id, Path file) {
        Minecraft minecraft = Minecraft.getInstance();
        if (minecraft == null) return;
        String key = "t:" + id;
        long mtime = mtime(file);
        Long prev = MTIMES.get(key);
        if (mtime <= 0L) {
            if (prev != null) {
                MTIMES.remove(key);
                Identifier old = DEV_TEXTURES.remove(id);
                if (old != null) minecraft.getTextureManager().release(old);
                LOGGER.info("[espectral] cosmetics hot reload dropped texture {}", id);
            }
            return;
        }
        if (prev != null && prev == mtime) return;
        NativeImage image = null;
        try {
            try (InputStream in = Files.newInputStream(file)) {
                image = NativeImage.read(in);
            }
            Identifier dev = Identifier.fromNamespaceAndPath(CosmeticTextures.NAMESPACE,
                    DEV_TEXTURE_PATH + id + ".png");
            DynamicTexture texture = new DynamicTexture(() -> "espectral-menu cosmetic " + id, image);
            image = null; // the texture owns it now
            minecraft.getTextureManager().register(dev, texture);
            DEV_TEXTURES.put(id, dev);
            MTIMES.put(key, mtime);
            LOGGER.info("[espectral] cosmetics hot reload texture {}", id);
        } catch (Exception e) {
            if (image != null) image.close();
            LOGGER.warn("[espectral] cosmetics hot reload texture {} failed: {}", id, e.toString());
        }
    }

    private static long mtime(Path file) {
        try {
            return Files.isRegularFile(file) ? Files.getLastModifiedTime(file).toMillis() : 0L;
        } catch (Exception e) {
            return 0L;
        }
    }

    /** Drops every hot-reload model and texture. */
    private static void closeDev() {
        Minecraft minecraft = Minecraft.getInstance();
        if (minecraft != null) {
            for (Identifier id : DEV_TEXTURES.values()) minecraft.getTextureManager().release(id);
        }
        DEV_TEXTURES.clear();
        for (String key : MTIMES.keySet()) {
            if (key.startsWith("m:")) ITEMS.remove(key.substring(2));
        }
        MTIMES.clear();
        devDir = null;
        lastScanMs = 0L;
    }

    // ---------------------------------------------------------------
    // Loading / baking
    // ---------------------------------------------------------------

    private static Item loadBundled(String id) {
        String path = MODEL_PATH + id + ".json";
        try (InputStream in = ItemModels.class.getClassLoader().getResourceAsStream(path)) {
            if (in == null) return null;
            return parse(id, JsonParser.parseReader(new InputStreamReader(in, StandardCharsets.UTF_8))
                    .getAsJsonObject(), path);
        } catch (Exception e) {
            LOGGER.warn("[espectral] cosmetic model {} could not be read: {}", path, e.toString());
            return null;
        }
    }

    private static Item loadFile(String id, Path file) {
        try (InputStream in = Files.newInputStream(file)) {
            return parse(id, JsonParser.parseReader(new InputStreamReader(in, StandardCharsets.UTF_8))
                    .getAsJsonObject(), file.toString());
        } catch (Exception e) {
            LOGGER.warn("[espectral] cosmetic model {} could not be read: {}", file, e.toString());
            return null;
        }
    }

    private static Item parse(String id, JsonObject root, String source) {
        int texW = 64, texH = 32;
        if (root.has("texture") && root.get("texture").isJsonObject()) {
            JsonObject texture = root.getAsJsonObject("texture");
            texW = intOr(texture, "width", 64);
            texH = intOr(texture, "height", 32);
        }
        String render = stringOr(root, "render", "cutout");
        String hideWith = stringOr(root, "hide_with", defaultHide(id));

        JsonArray partsJson = root.getAsJsonArray("parts");
        if (partsJson == null || partsJson.isEmpty()) {
            LOGGER.warn("[espectral] cosmetic model {} has no parts", source);
            return null;
        }

        List<Part> parts = new ArrayList<>(partsJson.size());
        List<MotionDriver.Spec> specs = new ArrayList<>();
        for (JsonElement element : partsJson) {
            Part part = bakePart(id, element.getAsJsonObject(), texW, texH, render, specs);
            if (part != null) parts.add(part);
        }
        if (parts.isEmpty()) {
            LOGGER.warn("[espectral] cosmetic model {} has no drawable parts", source);
            return null;
        }
        return new Item(id, hideWith, List.copyOf(parts), specs.toArray(new MotionDriver.Spec[0]));
    }

    /** Contract default: head items hide with the hat layer, back items with the cape. */
    private static String defaultHide(String id) {
        Catalog.Item item = Catalog.get().item(id);
        if (item == null) return "none";
        return switch (item.slot()) {
            case "cabeza" -> "hat";
            case "espalda" -> "cape";
            default -> "none";
        };
    }

    private static Part bakePart(String id, JsonObject partJson, int texW, int texH,
            String render, List<MotionDriver.Spec> specs) {
        String partName = stringOr(partJson, "part", null);
        if (partName == null || !isPlayerPart(partName)) {
            LOGGER.warn("[espectral] cosmetic {}: unknown attachment part {}", id, partName);
            return null;
        }
        JsonArray bonesJson = partJson.getAsJsonArray("bones");
        if (bonesJson == null || bonesJson.isEmpty()) return null;

        Map<String, BoneDef> byName = new LinkedHashMap<>();
        for (JsonElement element : bonesJson) {
            BoneDef def = BoneDef.parse(element.getAsJsonObject());
            if (def == null) {
                LOGGER.warn("[espectral] cosmetic {}: bone without a name", id);
                return null;
            }
            if (byName.put(def.name, def) != null) {
                LOGGER.warn("[espectral] cosmetic {}: duplicate bone {}", id, def.name);
                return null;
            }
        }
        for (BoneDef def : byName.values()) {
            for (String childName : def.childNames) {
                BoneDef child = byName.get(childName);
                if (child == null) {
                    LOGGER.warn("[espectral] cosmetic {}: bone {} names unknown child {}",
                            id, def.name, childName);
                    return null;
                }
                if (child.parent != null) {
                    LOGGER.warn("[espectral] cosmetic {}: bone {} has two parents", id, child.name);
                    return null;
                }
                child.parent = def;
                def.children.add(child);
            }
        }

        MeshDefinition mesh = new MeshDefinition();
        PartDefinition meshRoot = mesh.getRoot();
        for (BoneDef def : byName.values()) {
            if (def.parent == null) buildBone(meshRoot, def);
        }
        ModelPart baked = LayerDefinition.create(mesh, texW, texH).bakeRoot();

        List<Bone> bones = new ArrayList<>(byName.size());
        for (BoneDef def : byName.values()) {
            ModelPart bakedPart = partFor(baked, def);
            if (bakedPart == null) {
                LOGGER.warn("[espectral] cosmetic {}: bone {} did not bake", id, def.name);
                return null;
            }
            bones.add(new Bone(def, bakedPart));
        }
        for (Bone bone : bones) {
            for (JsonElement element : bone.def.motion) {
                MotionDriver.Spec spec = parseDriver(element.getAsJsonObject());
                // An unknown driver type (written by a newer build) leaves the
                // bone at rest instead of failing the whole item.
                if (spec == null) continue;
                bone.attach(spec, specs.size());
                specs.add(spec);
            }
        }
        return new Part(id, partName, render, baked, bones.toArray(new Bone[0]));
    }

    private static void buildBone(PartDefinition parent, BoneDef def) {
        CubeListBuilder cubes = CubeListBuilder.create();
        for (Cube cube : def.cubes) {
            cubes.texOffs(cube.u, cube.v);
            // Both flags are sticky on the builder, so set them on every cube.
            cubes.mirror(cube.mirror);
            cubes.addBox(cube.x, cube.y, cube.z, cube.w, cube.h, cube.d,
                    new CubeDeformation(cube.inflate));
        }
        PartDefinition bakedPart = parent.addOrReplaceChild(def.name, cubes,
                PartPose.offsetAndRotation(def.px, def.py, def.pz, def.rx, def.ry, def.rz));
        for (BoneDef child : def.children) buildBone(bakedPart, child);
    }

    /** Resolves a bone's baked {@link ModelPart} by walking down from the attachment root. */
    private static ModelPart partFor(ModelPart attachment, BoneDef def) {
        if (def.parent == null) {
            return attachment.hasChild(def.name) ? attachment.getChild(def.name) : null;
        }
        ModelPart parent = partFor(attachment, def.parent);
        if (parent == null || !parent.hasChild(def.name)) return null;
        return parent.getChild(def.name);
    }

    /**
     * Parses one driver. Returns {@code null} for an unknown type so a newer
     * build's driver never breaks an older client.
     */
    private static MotionDriver.Spec parseDriver(JsonObject json) {
        MotionDriver.Spec spec = new MotionDriver.Spec();
        int type = MotionDriver.typeOf(stringOr(json, "type", null));
        if (type < 0) {
            LOGGER.warn("[espectral] unknown cosmetic motion driver {}", stringOr(json, "type", "?"));
            return null;
        }
        spec.type = (byte) type;
        spec.axis = MotionDriver.axisOf(stringOr(json, "axis", "x"));
        switch (type) {
            case MotionDriver.TYPE_SWING -> {
                spec.rest = floatOr(json, "rest", 0f);
                spec.min = floatOr(json, "min", spec.rest);
                spec.max = floatOr(json, "max", spec.rest);
                spec.stiffness = clamp(floatOr(json, "stiffness", 0.18f), 0.02f, 0.6f);
                spec.damping = clamp(floatOr(json, "damping", 0.82f), 0.5f, 0.98f);
                spec.drag = floatOr(json, "drag", 0f);
                spec.fall = floatOr(json, "fall", 0f);
                spec.turn = floatOr(json, "turn", 0f);
                spec.sway = floatOr(json, "sway", 0f);
            }
            case MotionDriver.TYPE_REACT -> {
                readOffsets(json, "sneak", spec.sneak);
                readOffsets(json, "sprint", spec.sprint);
                readOffsets(json, "airborne", spec.airborne);
                readOffsets(json, "glide", spec.glide);
                readOffsets(json, "swim", spec.swim);
                spec.rate = clamp(floatOr(json, "rate", 0.2f), 0.05f, 1f);
            }
            case MotionDriver.TYPE_HEAD_TURN -> {
                spec.gain = floatOr(json, "gain", 1f);
                spec.maxAngle = floatOr(json, "max", 30f);
                spec.decay = clamp(floatOr(json, "decay", 0.8f), 0.5f, 0.95f);
            }
            case MotionDriver.TYPE_LAND -> {
                spec.impulse = floatOr(json, "impulse", 0f);
                spec.decay = clamp(floatOr(json, "decay", 0.8f), 0.5f, 0.95f);
            }
            case MotionDriver.TYPE_FLAP -> {
                spec.closed = floatOr(json, "closed", 0f);
                spec.open = floatOr(json, "open", 0f);
                spec.beat = floatOr(json, "beat", 0f);
                spec.beatSpeed = floatOr(json, "beat_speed", 0.3f);
            }
            default -> { }
        }
        return spec;
    }

    private static void readOffsets(JsonObject json, String key, float[] out) {
        if (!json.has(key) || !json.get(key).isJsonObject()) return;
        JsonObject offsets = json.getAsJsonObject(key);
        out[0] = floatOr(offsets, "x", 0f);
        out[1] = floatOr(offsets, "y", 0f);
        out[2] = floatOr(offsets, "z", 0f);
    }

    private static boolean isPlayerPart(String name) {
        for (String part : PARTS) if (part.equals(name)) return true;
        return false;
    }

    // ---------------------------------------------------------------
    // Ready-to-draw data
    // ---------------------------------------------------------------

    /** One ready-to-draw item. */
    public static final class Item {

        static final Item MISSING = new Item("", "none", List.of(), new MotionDriver.Spec[0], true);

        public final String id;
        /** Vanilla skin layer that hides it: {@code hat}, {@code cape} or {@code none}. */
        public final String hideWith;
        public final List<Part> parts;
        /** Every driver of this item, in bone order; driver {@code i} reads state {@code i}. */
        final MotionDriver.Spec[] specs;
        final boolean missing;

        Item(String id, String hideWith, List<Part> parts, MotionDriver.Spec[] specs) {
            this(id, hideWith, parts, specs, false);
        }

        private Item(String id, String hideWith, List<Part> parts,
                MotionDriver.Spec[] specs, boolean missing) {
            this.id = id;
            this.hideWith = hideWith;
            this.parts = parts;
            this.specs = specs;
            this.missing = missing;
        }
    }

    /**
     * One attachment part's baked tree. Extends {@link Model} so the deferred
     * renderer re-runs {@link #setupAnim} at draw time — the same mechanism the
     * vanilla player model uses, and the reason one shared instance per item is
     * safe for every wearer on screen.
     */
    public static final class Part extends Model<AvatarRenderState> {

        public final String part;
        private final String itemId;
        private final Bone[] bones;

        Part(String itemId, String part, String render, ModelPart root, Bone[] bones) {
            super(root, "translucent".equals(render) ? CosmeticTextures::cloth : CosmeticTextures::cutout);
            this.itemId = itemId;
            this.part = part;
            this.bones = bones;
        }

        @Override
        public void setupAnim(AvatarRenderState state) {
            WearerSims.Entry entry = WearerSims.lookup(WearerSims.uuidOf(state));
            MotionDriver.State[] states = entry == null ? null : entry.statesFor(itemId);
            if (states == null) {
                // No driver state: the authored rest rotation is the whole pose.
                // Rest first, because the instance is shared and re-posed at
                // draw time and a previous wearer's angles must never leak.
                for (Bone bone : bones) bone.rest();
                return;
            }
            // The arrays already hold the neutral values when the wearer is not
            // simulating (physics off, reduced motion, out of range), so the
            // static pose is the authored rest plus each driver's neutral.
            for (Bone bone : bones) bone.pose(states);
        }
    }

    /** One baked bone: rest rotation plus the drivers that bend it. */
    static final class Bone {

        final BoneDef def;
        final ModelPart part;
        final float restX, restY, restZ;

        private final List<MotionDriver.Spec> drivers = new ArrayList<>(2);
        private int[] indices = new int[0];

        Bone(BoneDef def, ModelPart part) {
            this.def = def;
            this.part = part;
            this.restX = def.rx;
            this.restY = def.ry;
            this.restZ = def.rz;
        }

        void attach(MotionDriver.Spec spec, int index) {
            drivers.add(spec);
            int[] grown = new int[indices.length + 1];
            System.arraycopy(indices, 0, grown, 0, indices.length);
            grown[indices.length] = index;
            indices = grown;
        }

        void rest() {
            part.setRotation(restX, restY, restZ);
        }

        void pose(MotionDriver.State[] states) {
            float dx = 0f, dy = 0f, dz = 0f;
            float[] add = SCRATCH.get();
            for (int i = 0; i < indices.length; i++) {
                int index = indices[i];
                if (index >= states.length) break;
                MotionDriver.accumulate(drivers.get(i), states[index], add);
                dx += add[0];
                dy += add[1];
                dz += add[2];
            }
            part.setRotation(restX + dx, restY + dy, restZ + dz);
        }
    }

    private static final ThreadLocal<float[]> SCRATCH = ThreadLocal.withInitial(() -> new float[3]);

    /** Parsed bone metadata from the compiled file. */
    static final class BoneDef {

        String name;
        float px, py, pz;
        float rx, ry, rz;
        final List<String> childNames = new ArrayList<>(2);
        final List<BoneDef> children = new ArrayList<>(2);
        final List<Cube> cubes = new ArrayList<>(2);
        final List<JsonElement> motion = new ArrayList<>(1);
        BoneDef parent;

        static BoneDef parse(JsonObject json) {
            BoneDef def = new BoneDef();
            def.name = stringOr(json, "name", null);
            if (def.name == null) return null;
            float[] pivot = floats(json.getAsJsonArray("pivot"), 3);
            def.px = pivot[0];
            def.py = pivot[1];
            def.pz = pivot[2];
            float[] rotation = floats(json.getAsJsonArray("rotation"), 3);
            def.rx = rotation[0];
            def.ry = rotation[1];
            def.rz = rotation[2];
            JsonArray children = json.getAsJsonArray("children");
            if (children != null) {
                for (JsonElement child : children) def.childNames.add(child.getAsString());
            }
            JsonArray cubes = json.getAsJsonArray("cubes");
            if (cubes != null) {
                for (JsonElement cube : cubes) def.cubes.add(Cube.parse(cube.getAsJsonObject()));
            }
            JsonArray motion = json.getAsJsonArray("motion");
            if (motion != null) {
                for (JsonElement driver : motion) def.motion.add(driver);
            }
            return def;
        }
    }

    /** One cube, in vanilla model space, relative to its bone's pivot. */
    static final class Cube {

        float x, y, z, w, h, d, inflate;
        int u, v;
        boolean mirror;

        static Cube parse(JsonObject json) {
            Cube cube = new Cube();
            float[] from = floats(json.getAsJsonArray("from"), 3);
            float[] size = floats(json.getAsJsonArray("size"), 3);
            cube.x = from[0];
            cube.y = from[1];
            cube.z = from[2];
            cube.w = size[0];
            cube.h = size[1];
            cube.d = size[2];
            float[] uv = floats(json.getAsJsonArray("uv"), 2);
            cube.u = Math.round(uv[0]);
            cube.v = Math.round(uv[1]);
            cube.inflate = floatOr(json, "inflate", 0f);
            cube.mirror = boolOr(json, "mirror", false);
            return cube;
        }
    }

    // ---------------------------------------------------------------
    // Small JSON helpers
    // ---------------------------------------------------------------

    private static String stringOr(JsonObject json, String key, String fallback) {
        if (json == null || !json.has(key) || json.get(key).isJsonNull()) return fallback;
        return json.get(key).getAsString();
    }

    private static float floatOr(JsonObject json, String key, float fallback) {
        if (json == null || !json.has(key) || !json.get(key).isJsonPrimitive()) return fallback;
        return json.get(key).getAsFloat();
    }

    private static int intOr(JsonObject json, String key, int fallback) {
        if (json == null || !json.has(key) || !json.get(key).isJsonPrimitive()) return fallback;
        return json.get(key).getAsInt();
    }

    private static boolean boolOr(JsonObject json, String key, boolean fallback) {
        if (json == null || !json.has(key) || !json.get(key).isJsonPrimitive()) return fallback;
        return json.get(key).getAsBoolean();
    }

    private static float[] floats(JsonArray array, int count) {
        float[] out = new float[count];
        if (array != null) {
            for (int i = 0; i < count && i < array.size(); i++) out[i] = array.get(i).getAsFloat();
        }
        return out;
    }

    private static float clamp(float value, float min, float max) {
        return value < min ? min : Math.min(value, max);
    }
}
