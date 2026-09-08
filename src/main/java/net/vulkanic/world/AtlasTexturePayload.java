package net.vulkanic.world;

import java.awt.image.BufferedImage;
import java.io.ByteArrayOutputStream;
import java.io.IOException;
import java.util.ArrayList;
import java.util.Arrays;
import javax.imageio.ImageIO;
import net.minecraft.client.renderer.texture.TextureAtlas;
import net.vulkanic.bridge.VulkanicGalBridge;

/** Copies a complete immutable CPU atlas incarnation, never a selected GPU frame. */
final class AtlasTexturePayload {
    static VulkanicGalBridge.WorldMeshTextureAssetRecord copy(AtlasAnimationResource resource,
        TextureAtlas.SemanticRawSnapshot snapshot) throws IOException {
        resource.requireOpen();
        var source = resource.source();
        if (snapshot == null || snapshot.width() != source.width() || snapshot.height() != source.height()) {
            throw new IllegalArgumentException("Incoherent atlas texture extent");
        }
        return copy(resource.semanticTextureId(), resource.atlas(), source.generation(), source.mipCount(), snapshot);
    }

    /** Static and animated atlases retain the exact same declared storage and copied levels. */
    static VulkanicGalBridge.WorldMeshTextureAssetRecord copy(int textureId,
        net.minecraft.resources.ResourceLocation atlas, long generation, int mipCount,
        TextureAtlas.SemanticRawSnapshot snapshot) throws IOException {
        if (snapshot == null || !snapshot.atlasLocation().equals(atlas)
            || generation <= 0 || snapshot.generation() != generation
            || snapshot.width() <= 0 || snapshot.height() <= 0
            || (long)snapshot.width() * snapshot.height() > 16L * 1024 * 1024
            || mipCount <= 0 || mipCount > 32) {
            throw new IllegalArgumentException("Incoherent atlas texture incarnation");
        }
        var levels = snapshot.mipPixels();
        if (levels.size() != mipCount || !Arrays.equals(snapshot.pixels(), levels.getFirst())) {
            throw new IllegalArgumentException("Incomplete atlas mip chain");
        }
        var encoded = new ArrayList<byte[]>(levels.size());
        long bytes = 0;
        for (int level = 0; level < levels.size(); level++) {
            int width = Math.max(1, snapshot.width() >> level);
            int height = Math.max(1, snapshot.height() >> level);
            byte[] rgba = levels.get(level);
            if (rgba.length != (long)width * height * 4) {
                throw new IllegalArgumentException("Invalid atlas mip byte count");
            }
            var image = new BufferedImage(width, height, BufferedImage.TYPE_INT_ARGB);
            for (int y = 0; y < height; y++) for (int x = 0; x < width; x++) {
                int offset = (y * width + x) * 4;
                image.setRGB(x, y, (rgba[offset + 3] & 255) << 24 | (rgba[offset] & 255) << 16
                    | (rgba[offset + 1] & 255) << 8 | (rgba[offset + 2] & 255));
            }
            var output = new ByteArrayOutputStream();
            if (!ImageIO.write(image, "PNG", output)) throw new IOException("PNG encoder unavailable");
            bytes += output.size();
            if (bytes > 4L * 1024 * 1024) throw new IllegalArgumentException("Atlas PNG payload bound exceeded");
            encoded.add(output.toByteArray());
        }
        return new VulkanicGalBridge.WorldMeshTextureAssetRecord(textureId,
            encoded.getFirst(), encoded.subList(1, encoded.size()));
    }
}
