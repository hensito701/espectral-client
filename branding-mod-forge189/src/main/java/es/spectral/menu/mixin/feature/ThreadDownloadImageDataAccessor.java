package es.spectral.menu.mixin.feature;

import java.awt.image.BufferedImage;

import org.spongepowered.asm.mixin.Mixin;
import org.spongepowered.asm.mixin.gen.Accessor;

import net.minecraft.client.renderer.ThreadDownloadImageData;

/** The downloaded (processed) skin image; null until the download finished. */
@Mixin(ThreadDownloadImageData.class)
public interface ThreadDownloadImageDataAccessor {

    @Accessor("bufferedImage")
    BufferedImage espectral$image();
}
