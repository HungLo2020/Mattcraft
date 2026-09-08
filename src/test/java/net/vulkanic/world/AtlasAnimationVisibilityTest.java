package net.vulkanic.world;

import java.util.List;
import net.minecraft.client.renderer.texture.SemanticAtlasAnimationSource;
import net.minecraft.client.renderer.texture.SpriteContents;
import net.minecraft.resources.ResourceLocation;
import org.junit.jupiter.api.Test;
import static org.junit.jupiter.api.Assertions.*;

class AtlasAnimationVisibilityTest {
    private static final ResourceLocation ATLAS = ResourceLocation.withDefaultNamespace("textures/atlas/blocks.png");
    private static ResourceLocation name(String name) { return ResourceLocation.withDefaultNamespace(name); }
    private static SemanticAtlasAnimationSource.Sprite sprite(int id, String name) {
        return new SemanticAtlasAnimationSource.Sprite(id, name(name), 0, 0,
            new SpriteContents.SemanticAnimationSource(1, 1, 1, false,
                List.of(new SpriteContents.SemanticAnimationFrame(0, 1)),
                List.of(new SpriteContents.SemanticAnimationMip(1, 1, new byte[4]))));
    }
    private static SemanticAtlasAnimationSource source(SemanticAtlasAnimationSource.Sprite... sprites) {
        return new SemanticAtlasAnimationSource(7, 2, 1, 1, List.of(sprites));
    }

    @Test
    void resourceUsesAccumulateAcrossFramesAndDetachAtTickBoundary() {
        var visibility = new AtlasAnimationVisibility(ATLAS, source(sprite(5, "block/water"), sprite(2, "block/lava")));
        assertFalse(visibility.recordUse(name("textures/atlas/particles.png"), name("block/water")), "foreign atlas must not activate an identically named sprite");
        assertTrue(visibility.recordUse(ATLAS, name("block/water")));
        assertFalse(visibility.recordUse(ATLAS, name("block/water")), "duplicate layers/frames must not duplicate an ID");
        assertFalse(visibility.recordUse(ATLAS, name("block/static_stone")));
        assertTrue(visibility.recordUse(ATLAS, name("block/lava")));
        int[] tick = visibility.takeUses();
        assertArrayEquals(new int[]{2, 5}, tick);
        assertArrayEquals(new int[0], visibility.takeUses());
        assertTrue(visibility.recordUse(ATLAS, name("block/water")));
        tick[0] = 99;
        assertArrayEquals(new int[]{5}, visibility.takeUses(), "new tick must not alias an earlier pending event");
    }

    @Test
    void replacementStartsEmptyAndAmbiguousResourceIdentitiesReject() {
        var old = new AtlasAnimationVisibility(ATLAS, source(sprite(1, "block/water")));
        old.recordUse(ATLAS, name("block/water"));
        var replacement = new AtlasAnimationVisibility(ATLAS, source(sprite(9, "block/water")));
        assertArrayEquals(new int[0], replacement.takeUses());
        replacement.recordUse(ATLAS, name("block/water"));
        assertArrayEquals(new int[]{9}, replacement.takeUses());
        assertThrows(IllegalArgumentException.class, () -> new AtlasAnimationVisibility(ATLAS,
            source(sprite(1, "block/water"), sprite(1, "block/lava"))));
        assertThrows(IllegalArgumentException.class, () -> new AtlasAnimationVisibility(ATLAS,
            source(sprite(1, "block/water"), sprite(2, "block/water"))));
    }
}
