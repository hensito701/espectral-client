package es.spectral.menu.mixin;

import org.spongepowered.asm.mixin.Mixin;
import org.spongepowered.asm.mixin.injection.At;
import org.spongepowered.asm.mixin.injection.Inject;
import org.spongepowered.asm.mixin.injection.callback.CallbackInfoReturnable;

import net.minecraft.client.renderer.entity.EntityRenderer;
import net.minecraft.client.renderer.entity.state.EntityRenderState;
import net.minecraft.world.entity.Entity;

import es.spectral.menu.cosmetics.Cosmetics;

/**
 * Decorates entity nametags with the ⚛/★ badge. Hooks the final
 * {@code createRenderState(entity, tickDelta)} on {@link EntityRenderer} — it
 * runs after every subclass' {@code extractRenderState}, so the badge lands
 * regardless of which renderer produced the state. The hook prepends a styled
 * sibling component; the server-sent name is never recoloured or rebuilt, and
 * entities whose nametag vanilla would hide ({@code state.nameTag == null})
 * are skipped entirely.
 */
@Mixin(EntityRenderer.class)
public abstract class EntityRendererBadgeMixin {

    @Inject(
            method = "createRenderState(Lnet/minecraft/world/entity/Entity;F)"
                    + "Lnet/minecraft/client/renderer/entity/state/EntityRenderState;",
            at = @At("RETURN"))
    private void espectral$badgeNameTag(Entity entity, float tickDelta,
            CallbackInfoReturnable<EntityRenderState> cir) {
        EntityRenderState state = cir.getReturnValue();
        if (state == null || state.nameTag == null) return;
        state.nameTag = Cosmetics.decorateEntityNameTag(state.nameTag, entity);
    }
}
