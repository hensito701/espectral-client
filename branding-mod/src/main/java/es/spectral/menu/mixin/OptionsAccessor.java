package es.spectral.menu.mixin;

import org.spongepowered.asm.mixin.Mixin;
import org.spongepowered.asm.mixin.Mutable;
import org.spongepowered.asm.mixin.gen.Accessor;

import net.minecraft.client.KeyMapping;
import net.minecraft.client.OptionInstance;
import net.minecraft.client.Options;

/**
 * Write access to the vanilla key-mapping array so modded bindings show up in
 * Controls and persist to options.txt (KeyBindingHelper equivalent without a
 * Fabric API dependency).
 */
@Mixin(Options.class)
public interface OptionsAccessor {

    @Mutable
    @Accessor("keyMappings")
    void espectral$setKeyMappings(KeyMapping[] value);


    /** Raw option-value access lives here so key and gamma access share one mixin boundary. */
    @Mixin(OptionInstance.class)
    interface OptionValueAccessor<T> {
        @Mutable
        @Accessor("value")
        void espectral$setValue(T value);
    }
}
