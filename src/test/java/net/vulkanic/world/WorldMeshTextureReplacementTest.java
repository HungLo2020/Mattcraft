package net.vulkanic.world;

import java.util.LinkedHashMap;
import java.util.LinkedHashSet;
import java.util.List;
import net.vulkanic.bridge.VulkanicGalBridge.WorldMeshTextureAssetRecord;
import net.vulkanic.bridge.VulkanicGalBridge.WorldMeshAnimationFrameRecord;
import org.junit.jupiter.api.Test;
import static org.junit.jupiter.api.Assertions.*;

class WorldMeshTextureReplacementTest {
    @org.junit.jupiter.api.BeforeAll
    static void bootstrap() {
        net.minecraft.SharedConstants.tryDetectVersion();
        net.minecraft.server.Bootstrap.bootStrap();
    }
    private static WorldMeshTextureAssetRecord texture(int[] fields, byte[] base,
        List<byte[]> mips, List<WorldMeshAnimationFrameRecord> frames) {
        return new WorldMeshTextureAssetRecord(fields[0], base, fields[1], fields[2], fields[3],
            fields[4], fields[5], fields[6], fields[7], frames, fields[8], mips);
    }

    @Test
    @SuppressWarnings("unchecked")
    void realReloadRetainsPublicationWorkForUnchangedTexturePayloads() throws Exception {
        var texturesField = RustGalWorldPrimitiveRenderer.class.getDeclaredField("WORLD_MESH_TEXTURES");
        var dirtyField = RustGalWorldPrimitiveRenderer.class.getDeclaredField("DIRTY_WORLD_MESH_TEXTURES");
        var uploadedField = RustGalWorldPrimitiveRenderer.class.getDeclaredField("UPLOADED_WORLD_MESH_TEXTURES");
        texturesField.setAccessible(true);
        dirtyField.setAccessible(true);
        uploadedField.setAccessible(true);
        var textures = (java.util.Map<Integer, WorldMeshTextureAssetRecord>) texturesField.get(null);
        var dirty = (java.util.Set<Integer>) dirtyField.get(null);
        var uploaded = (java.util.Map<Integer, Long>) uploadedField.get(null);
        int identity = 0x74657374;
        var payload = new WorldMeshTextureAssetRecord(identity, new byte[]{1, 2}, List.of());
        textures.put(identity, payload);
        uploaded.put(identity, 7L);
        dirty.remove(identity);
        try {
            RustGalWorldPrimitiveRenderer.reloadWorldAssets(null);
            assertSame(payload, textures.get(identity), "reload must retain copied semantic payloads");
            assertFalse(uploaded.containsKey(identity), "old acceptance cannot prove the new generation");
            assertThrows(IllegalStateException.class,
                () -> RustGalWorldPrimitiveRenderer.requireAcceptedWorldMeshTextureGeneration(identity));
            assertTrue(dirty.contains(identity), "retained payload must be queued for native re-publication");
            assertFalse(RustGalWorldPrimitiveRenderer.registerChangedTexture(textures, dirty,
                new WorldMeshTextureAssetRecord(identity, new byte[]{1, 2}, List.of())));
            assertTrue(dirty.contains(identity), "equal producer payload must not erase pending publication");
        } finally {
            textures.remove(identity);
            dirty.remove(identity);
            uploaded.remove(identity);
        }
    }

    @Test
    void samplingMetadataOnlyReplacementIsPublishedWithIdenticalPngBytes() {
        var textures = new LinkedHashMap<Integer, WorldMeshTextureAssetRecord>();
        var dirty = new LinkedHashSet<Integer>();
        var nearest = new WorldMeshTextureAssetRecord(17, new byte[]{1, 2}).withTextureMetadata(false, false);
        assertTrue(RustGalWorldPrimitiveRenderer.registerChangedTexture(textures, dirty, nearest));
        dirty.clear();
        var linear = nearest.withTextureMetadata(true, false);
        assertArrayEquals(nearest.pngBytes(), linear.pngBytes());
        assertFalse(nearest.sameContent(linear));
        assertTrue(RustGalWorldPrimitiveRenderer.registerChangedTexture(textures, dirty, linear));
        assertEquals(java.util.Set.of(17), dirty);
        assertEquals(2, linear.samplingFilter());
        assertEquals(1, linear.samplingAddress());
        dirty.clear();
        assertTrue(RustGalWorldPrimitiveRenderer.registerChangedTexture(textures, dirty,
            linear.withTextureMetadata(true, true)));
        assertEquals(java.util.Set.of(17), dirty);
    }

    @Test void equivalentSectionPublicationPreservesCollectedAtlasObject() {
        var textures = new LinkedHashMap<Integer, WorldMeshTextureAssetRecord>();
        var dirty = new LinkedHashSet<Integer>();
        var eager = new WorldMeshTextureAssetRecord(17, new byte[]{1, 2}, List.of(new byte[]{3, 4}));
        assertTrue(RustGalWorldPrimitiveRenderer.registerChangedTexture(textures, dirty, eager));
        dirty.clear();
        var section = new WorldMeshTextureAssetRecord(17, new byte[]{1, 2}, List.of(new byte[]{3, 4}));
        assertNotSame(eager, section);
        assertFalse(RustGalWorldPrimitiveRenderer.registerChangedTexture(textures, dirty, section));
        assertSame(eager, textures.get(17), "equivalent section copies cannot invalidate collected GUI source identity");
        assertTrue(dirty.isEmpty());
        var replacement = section.withTextureMetadata(true, false);
        assertTrue(RustGalWorldPrimitiveRenderer.registerChangedTexture(textures, dirty, replacement));
        assertSame(replacement, textures.get(17));
        assertTrue(dirty.contains(17), "real semantic changes must still require native acceptance");
    }

    @Test
    void allSemanticFieldsAndMipBytesParticipateInIdentity() {
        var base = new WorldMeshTextureAssetRecord(17, new byte[]{1, 2});
        assertFalse(base.sameContent(base.withMipLevels(1)));
        assertFalse(base.withMipLevels(1).sameContent(base.withMipLevels(5)));
        assertEquals(5, base.withMipLevels(5).withTextureMetadata(false, true).requestedMipLevels());
        assertThrows(IllegalArgumentException.class, () -> base.withMipLevels(0));
        assertThrows(IllegalArgumentException.class, () -> base.withMipLevels(33));
        int[] fields = {17, 2, 2, 2, 3, 0, 2, 0, 0};
        var frames = List.of(new WorldMeshAnimationFrameRecord(0, 3), new WorldMeshAnimationFrameRecord(1, 7));
        var original = texture(fields, new byte[]{1, 2}, List.of(new byte[]{3, 4}), frames);
        original.mipPngBytes().getFirst()[0] = 99;
        assertArrayEquals(new byte[]{3, 4}, original.mipPngBytes().getFirst(), "caller must not mutate retained identity");
        assertTrue(original.sameContent(texture(fields.clone(), new byte[]{1, 2}, List.of(new byte[]{3, 4}), frames)));
        assertFalse(original.sameContent(null));
        for (int field = 0; field < fields.length; field++) {
            int[] changed = fields.clone();
            changed[field]++;
            assertFalse(original.sameContent(texture(changed, new byte[]{1, 2}, List.of(new byte[]{3, 4}), frames)),
                "field " + field + " must invalidate texture identity");
        }
        assertFalse(original.sameContent(texture(fields, new byte[]{2, 1}, List.of(new byte[]{3, 4}), frames)));
        assertFalse(original.sameContent(texture(fields, new byte[]{1, 2}, List.of(new byte[]{4, 3}), frames)));
        assertFalse(original.sameContent(texture(fields, new byte[]{1, 2}, List.of(), frames)));
        assertFalse(original.sameContent(texture(fields, new byte[]{1, 2}, List.of(new byte[]{3, 4}),
            List.of(new WorldMeshAnimationFrameRecord(1, 7), new WorldMeshAnimationFrameRecord(0, 3)))));
    }

    @Test
    void publisherRedirtiesMipOnlyReplacementButNotEqualIndependentCopies() {
        var textures = new LinkedHashMap<Integer, WorldMeshTextureAssetRecord>();
        var dirty = new LinkedHashSet<Integer>();
        var first = new WorldMeshTextureAssetRecord(17, new byte[]{1, 2}, List.of(new byte[]{3, 4}));
        assertTrue(RustGalWorldPrimitiveRenderer.registerChangedTexture(textures, dirty, first));
        assertEquals(java.util.Set.of(17), dirty);
        dirty.clear(); // Previous upload was accepted.
        var same = new WorldMeshTextureAssetRecord(17, new byte[]{1, 2}, List.of(new byte[]{3, 4}));
        assertFalse(RustGalWorldPrimitiveRenderer.registerChangedTexture(textures, dirty, same));
        assertTrue(dirty.isEmpty());
        var changed = new WorldMeshTextureAssetRecord(17, new byte[]{1, 2}, List.of(new byte[]{4, 3}));
        assertTrue(RustGalWorldPrimitiveRenderer.registerChangedTexture(textures, dirty, changed));
        assertEquals(java.util.Set.of(17), dirty);
        assertSame(changed, textures.get(17));
    }
}
