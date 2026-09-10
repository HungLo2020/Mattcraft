package net.vulkanic.bridge;

import java.lang.foreign.Arena;
import java.lang.foreign.MemorySegment;
import java.lang.foreign.ValueLayout;
import java.util.ArrayList;
import java.util.Collections;
import java.util.List;
import org.junit.jupiter.api.Test;

import static net.vulkanic.bridge.VulkanicGalBridge.*;
import static org.junit.jupiter.api.Assertions.*;

class AtlasAnimationEncodingTest {
    @org.junit.jupiter.params.ParameterizedTest
    @org.junit.jupiter.params.provider.ValueSource(ints = {0x54a17a1a, 0xfdf71712, 0x80000000, 0xffffffff})
    void semanticSnapshotStagesAgainstAcceptedTextureGenerationNotResourceCounter(int atlas) throws Exception {
        var name = net.minecraft.resources.ResourceLocation.withDefaultNamespace("audit/animated");
        var pixels = new net.blaze3d.platform.NativeImage(2, 1, false);
        pixels.setPixel(0, 0, 0xff123456);
        pixels.setPixel(1, 0, 0xffabcdef);
        var metadata = new net.minecraft.client.resources.metadata.animation.AnimationMetadataSection(
            java.util.Optional.of(List.of(
                new net.minecraft.client.resources.metadata.animation.AnimationFrame(1, java.util.Optional.of(3)),
                new net.minecraft.client.resources.metadata.animation.AnimationFrame(0, java.util.Optional.of(7)))),
            java.util.Optional.of(1), java.util.Optional.of(1), 1, true);
        try (var contents = new net.minecraft.client.renderer.texture.SpriteContents(name,
            new net.minecraft.client.resources.metadata.animation.FrameSize(1, 1), pixels,
            java.util.Optional.of(metadata), List.of());
            var bridge = VulkanicGalBridge.create("rust-vulkan")) {
            var snapshot = new net.minecraft.client.renderer.texture.SemanticAtlasAnimationSource(
                77, 2, 1, 1, List.of(new net.minecraft.client.renderer.texture.SemanticAtlasAnimationSource.Sprite(
                    9, name, 1, 0, contents.semanticAnimationSource().orElseThrow())));
            var records = VulkanicGalBridge.atlasAnimationRecords(snapshot);
            var duplicate = new net.minecraft.client.renderer.texture.SemanticAtlasAnimationSource(
                77, 2, 1, 1, List.of(snapshot.sprites().getFirst(), snapshot.sprites().getFirst()));
            assertThrows(IllegalArgumentException.class, () -> VulkanicGalBridge.atlasAnimationRecords(duplicate));
            assertThrows(IllegalArgumentException.class, () -> bridge.stageAtlasAnimationAssets(1, 0, 40, snapshot));
            assertEquals(9, records.getFirst().spriteId());
            assertEquals(1, records.getFirst().atlasX());
            assertEquals(List.of(new WorldMeshAnimationFrameRecord(1, 3), new WorldMeshAnimationFrameRecord(0, 7)),
                records.getFirst().frames());
            assertTrue(records.getFirst().interpolate());
            assertArrayEquals(new byte[]{0x12, 0x34, 0x56, (byte)255, (byte)0xab, (byte)0xcd, (byte)0xef, (byte)255},
                records.getFirst().mips().getFirst().rgba());
            pixels.setPixel(0, 0, 0);
            assertEquals(0x12, records.getFirst().mips().getFirst().rgba()[0]);
            var image = new java.awt.image.BufferedImage(2, 1, java.awt.image.BufferedImage.TYPE_INT_ARGB);
            var png = new java.io.ByteArrayOutputStream();
            assertTrue(javax.imageio.ImageIO.write(image, "PNG", png));
            Status before = bridge.updateWorldMeshAssets(1, List.of(),
                List.of(new WorldMeshTextureAssetRecord(atlas, png.toByteArray(), List.of())), List.of());
            assertThrows(IllegalStateException.class, () -> bridge.stageAtlasAnimationAssets(atlas, 77, 40, snapshot));
            Status staged = bridge.stageAtlasAnimationAssets(atlas, 1, 40, snapshot);
            assertEquals(before.submissionId(), staged.submissionId());
            assertThrows(IllegalStateException.class, () -> bridge.stageAtlasAnimationAssets(atlas, 1, 40, snapshot));
            assertThrows(IllegalStateException.class, () -> bridge.tickAtlasAnimation(atlas, 77, 41, new int[]{9}, true));
            assertThrows(IllegalStateException.class, () -> bridge.tickAtlasAnimation(atlas, 1, 41, new int[]{9, 9}, true));
            assertThrows(IllegalStateException.class, () -> bridge.tickAtlasAnimation(atlas, 1, 41, new int[]{10}, true));
            var firstTick = bridge.tickAtlasAnimation(atlas, 1, 41, new int[]{9}, true);
            assertTrue(firstTick.accepted());
            assertTrue(firstTick.status().submissionId() > staged.submissionId());
            var duplicateTick = bridge.tickAtlasAnimation(atlas, 1, 41, new int[]{9}, true);
            assertTrue(duplicateTick.accepted());
            assertEquals(firstTick.status().submissionId(), duplicateTick.status().submissionId());
            assertThrows(IllegalStateException.class, () -> bridge.tickAtlasAnimation(atlas, 1, 43, new int[]{9}, true));
            var invisibleTick = bridge.tickAtlasAnimation(atlas, 1, 42, new int[0], true);
            assertTrue(invisibleTick.accepted());
            assertEquals(firstTick.status().submissionId(), invisibleTick.status().submissionId());
        }
    }

    @Test
    void stagesAnExplicitNonTerrainAtlasThroughRealRustVulkanContextWithoutSubmittingGpuWork() throws Exception {
        int terrainAtlas = 202; // Semantic test asset, deliberately not the terrain-atlas identity.
        var image = new java.awt.image.BufferedImage(2, 1, java.awt.image.BufferedImage.TYPE_INT_ARGB);
        image.setRGB(0, 0, 0xff123456);
        image.setRGB(1, 0, 0xffabcdef);
        var png = new java.io.ByteArrayOutputStream();
        assertTrue(javax.imageio.ImageIO.write(image, "PNG", png));
        var texture = new WorldMeshTextureAssetRecord(terrainAtlas, png.toByteArray(), List.of());
        var source = new AtlasAnimationSourceRecord(1, 0, 0, 1, 1, true, List.of(
            new WorldMeshAnimationFrameRecord(0, 2), new WorldMeshAnimationFrameRecord(1, 2)),
            List.of(new SpriteAnimationMipRecord(2, 1, new byte[]{10, 20, 30, 7, 110, 120, (byte)130, 99})));
        try (var bridge = VulkanicGalBridge.create("rust-vulkan")) {
            assertThrows(IllegalStateException.class,
                () -> bridge.stageAtlasAnimationAssets(terrainAtlas, 1, 40, List.of(source)));
            Status before = bridge.updateWorldMeshAssets(1, List.of(), List.of(texture), List.of());
            Status staged = bridge.stageAtlasAnimationAssets(terrainAtlas, 1, 40, List.of(source));
            assertEquals(before.submissionId(), staged.submissionId());
            assertEquals(before.ffiCalls() + 1, staged.ffiCalls());
            assertEquals(48 + 64 + 32 + 32 + 8, staged.ffiInputBytes() - before.ffiInputBytes());
            assertThrows(IllegalStateException.class,
                () -> bridge.stageAtlasAnimationAssets(terrainAtlas, 1, 40, List.of(source)));
            bridge.updateWorldMeshAssets(2, List.of(), List.of(texture), List.of());
            assertThrows(IllegalStateException.class,
                () -> bridge.stageAtlasAnimationAssets(terrainAtlas, 1, 40, List.of(source)));
            bridge.stageAtlasAnimationAssets(terrainAtlas, 2, 40, List.of(source));
        }
    }

    @Test
    void encodesRealNativeLayoutsWithOwnedPixelCopies() {
        byte[] pixels = {10, 20, 30, 7, 110, 120, (byte)130, 99};
        var mip = new SpriteAnimationMipRecord(2, 1, pixels);
        pixels[0] = 0;
        var source = new AtlasAnimationSourceRecord(5, 4, 8, 1, 1, true, List.of(
            new WorldMeshAnimationFrameRecord(1, 3), new WorldMeshAnimationFrameRecord(0, 7)), List.of(mip));
        try (Arena arena = Arena.ofConfined()) {
            MemorySegment request = VulkanicGalBridge.encodeAtlasAnimationUpdate(arena, 17, 23, 41, List.of(source));
            assertEquals(48, request.byteSize());
            assertEquals(54, request.get(ValueLayout.JAVA_INT, 0));
            assertEquals(48, request.get(ValueLayout.JAVA_INT, 4));
            assertEquals(17, request.get(ValueLayout.JAVA_INT, 8));
            assertEquals(0, request.get(ValueLayout.JAVA_INT, 12));
            assertEquals(23, request.get(ValueLayout.JAVA_LONG, 16));
            assertEquals(41, request.get(ValueLayout.JAVA_LONG, 24));
            assertEquals(1, request.get(ValueLayout.JAVA_LONG, 40));
            MemorySegment encoded = request.get(ValueLayout.ADDRESS, 32).reinterpret(64);
            assertArrayEquals(new int[]{64, 5, 4, 8, 1, 1, 1, 0}, encoded.asSlice(0, 32).toArray(ValueLayout.JAVA_INT));
            assertEquals(2, encoded.get(ValueLayout.JAVA_LONG, 40));
            MemorySegment frames = encoded.get(ValueLayout.ADDRESS, 32).reinterpret(32);
            assertArrayEquals(new int[]{16, 1, 3, 0, 16, 0, 7, 0}, frames.toArray(ValueLayout.JAVA_INT));
            assertEquals(1, encoded.get(ValueLayout.JAVA_LONG, 56));
            MemorySegment image = encoded.get(ValueLayout.ADDRESS, 48).reinterpret(32);
            assertArrayEquals(new int[]{32, 2, 1, 0}, image.asSlice(0, 16).toArray(ValueLayout.JAVA_INT));
            assertEquals(8, image.get(ValueLayout.JAVA_LONG, 24));
            MemorySegment copied = image.get(ValueLayout.ADDRESS, 16).reinterpret(8);
            assertArrayEquals(new byte[]{10, 20, 30, 7, 110, 120, (byte)130, 99}, copied.toArray(ValueLayout.JAVA_BYTE));
            copied.set(ValueLayout.JAVA_BYTE, 0, (byte)55);
            assertEquals(10, mip.rgba()[0]);
        }
    }

    @Test
    void aggregateAndDuplicateRejectionPrecedesAnyArenaAllocation() {
        var mip = new SpriteAnimationMipRecord(1, 1, new byte[4]);
        var frames = Collections.nCopies(16384, new WorldMeshAnimationFrameRecord(0, 1));
        var sources = new ArrayList<AtlasAnimationSourceRecord>();
        for (int i = 1; i <= 5; i++) sources.add(new AtlasAnimationSourceRecord(i, 0, 0, 1, 1, false, frames, List.of(mip)));
        Arena closed = Arena.ofConfined();
        closed.close();
        assertThrows(IllegalArgumentException.class,
            () -> VulkanicGalBridge.encodeAtlasAnimationUpdate(closed, 1, 1, 0, sources));
        assertThrows(IllegalArgumentException.class,
            () -> VulkanicGalBridge.encodeAtlasAnimationUpdate(closed, 1, 1, 0, List.of(sources.getFirst(), sources.getFirst())));
        assertThrows(IllegalArgumentException.class,
            () -> new SpriteAnimationMipRecord(1 << 30, 1 << 30, new byte[0]));
        assertThrows(UnsupportedOperationException.class, () -> sources.getFirst().frames().clear());
        byte[] external = mip.rgba();
        external[0] = 42;
        assertEquals(0, mip.rgba()[0]);
        List<WorldMeshAnimationFrameRecord> oversizedFrames = new java.util.AbstractList<>() {
            @Override public int size() { return 16385; }
            @Override public WorldMeshAnimationFrameRecord get(int index) {
                throw new AssertionError("Oversized input must be rejected before copying");
            }
        };
        assertThrows(IllegalArgumentException.class, () -> new AtlasAnimationSourceRecord(
            1, 0, 0, 1, 1, false, oversizedFrames, List.of(mip)));
        List<AtlasAnimationSourceRecord> oversizedSources = new java.util.AbstractList<>() {
            @Override public int size() { return 16385; }
            @Override public AtlasAnimationSourceRecord get(int index) {
                throw new AssertionError("Oversized input must be rejected before copying");
            }
        };
        assertThrows(IllegalArgumentException.class,
            () -> VulkanicGalBridge.encodeAtlasAnimationUpdate(closed, 1, 1, 0, oversizedSources));
    }
}
