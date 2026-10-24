package es.spectral.menu.cosmetics.render;

import java.util.UUID;

import net.minecraft.client.renderer.entity.state.AvatarRenderState;
import net.minecraft.client.renderer.entity.state.HumanoidRenderState;
import net.minecraft.resources.Identifier;

import es.spectral.menu.cosmetics.Cosmetics;
import es.spectral.menu.cosmetics.Loadout;

/**
 * Per-player elytra skins (Élitros Vidriera / Nebulosa / Ceniza). Resolved from
 * the {@code elitros} loadout slot and returned by the
 * {@code WingsLayer#getPlayerElytraTexture} hook, so vanilla keeps drawing the
 * wings (equipment checks, glow, outline) and only the texture changes.
 * Nebulosa and Ceniza are frame-animated (bundled per-frame files, chosen here
 * because entity textures cannot animate through {@code .mcmeta}).
 */
public final class CosmeticElytra {

    private CosmeticElytra() {}

    /**
     * The cosmetic elytra texture for this wearer, or {@code null} to keep the
     * vanilla result (no cosmetic, or the wearer must not show cosmetics).
     */
    public static Identifier skinFor(HumanoidRenderState state) {
        if (!(state instanceof AvatarRenderState avatar)) return null;
        // Same guard set as CosmeticLayer: the design only forbids cosmetics on
        // invisible and spectating wearers. `isDiscrete` is `isShiftKeyDown()`
        // in 26.2, so testing it here silently dropped the elytra skin to the
        // vanilla one whenever the wearer crouched.
        if (avatar.isInvisible || avatar.isInvisibleToPlayer || avatar.isSpectator) {
            return null;
        }
        UUID uuid = WearerSims.uuidOf(avatar);
        if (uuid == null) return null;
        Loadout loadout = WearerSims.loadoutFor(uuid);
        if (loadout == null) return null;
        String id = loadout.elitros();
        if (id == null) return null;
        boolean still = Cosmetics.reducedMotion();
        int frame = CosmeticTextures.frameAt((int) avatar.ageInTicks, still);
        return switch (id) {
            case "elitros_vidriera" -> CosmeticTextures.ELYTRA_VIDRIERA;
            case "elitros_nebulosa" -> CosmeticTextures.elytraFrame("elitros_nebulosa", frame);
            case "elitros_ceniza" -> CosmeticTextures.elytraFrame("elitros_ceniza", frame);
            default -> null;
        };
    }
}
