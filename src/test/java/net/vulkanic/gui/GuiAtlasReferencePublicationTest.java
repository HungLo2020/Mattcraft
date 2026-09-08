package net.vulkanic.gui;

import java.util.List;
import java.util.concurrent.atomic.AtomicInteger;
import java.util.concurrent.atomic.AtomicReference;
import net.vulkanic.bridge.VulkanicGalBridge.GuiAtlasReferenceRecord;
import net.vulkanic.bridge.VulkanicGalBridge.WorldMeshTextureAssetRecord;
import org.junit.jupiter.api.Test;
import static org.junit.jupiter.api.Assertions.*;

class GuiAtlasReferencePublicationTest {
    @Test void nestedOnlyItemSourcesStayLiveUntilTheirLastSemanticUse() {
        var table = new GuiAtlasReferencePublication();
        table.stage(reference(101,7));
        table.stage(reference(102,7));
        table.stage(reference(103,7)); // genuinely unused; must still retire
        var layer = new net.vulkanic.bridge.VulkanicGalBridge.GuiItemRasterLayerRecord(
            102,-1,1,net.vulkanic.bridge.VulkanicGalBridge.GuiItemRasterGeometryRecord.FULL,0,0,1,1);
        var parent = new net.vulkanic.bridge.VulkanicGalBridge.GuiAffineQuadRecord(
            1,101,0,0,16,0,0,16,0,0,0,1,1,-1,320,180).withMaterialMode(1).withItemRasterScale(2);
        var layered = parent.withItemRasterLayers(List.of(layer));
        table.retainUsedCommands(List.of(layered),List.of(),List.of());
        table.flush(GuiAtlasReferencePublicationTest::accepted,(revision,refs) ->
            assertEquals(List.of(101L,102L),refs.stream().map(GuiAtlasReferenceRecord::assetId).toList()));
        table.retainUsedCommands(List.of(layered),List.of(),List.of());
        table.flush(GuiAtlasReferencePublicationTest::accepted,(revision,refs) -> fail("unchanged uses must not republish"));
        table.stage(reference(102,8));
        table.retainUsedCommands(List.of(layered),List.of(),List.of());
        table.flush(GuiAtlasReferencePublicationTest::accepted,(revision,refs) -> {
            assertEquals(2,refs.size());
            assertEquals(8L,refs.get(1).atlasGeneration());
        });
        table.retainUsedCommands(List.of(parent),List.of(),List.of());
        table.flush(GuiAtlasReferencePublicationTest::accepted,(revision,refs) ->
            assertEquals(List.of(101L),refs.stream().map(GuiAtlasReferenceRecord::assetId).toList()));
        table.retainUsedCommands(List.of(),List.of(),List.of());
        table.flush(GuiAtlasReferencePublicationTest::accepted,(revision,refs) -> assertTrue(refs.isEmpty()));
    }

    private static final WorldMeshTextureAssetRecord SOURCE7 = new WorldMeshTextureAssetRecord(17, new byte[]{7});
    private static final WorldMeshTextureAssetRecord SOURCE8 = new WorldMeshTextureAssetRecord(17, new byte[]{8});
    private static long accepted(WorldMeshTextureAssetRecord payload) {
        assertTrue(payload == SOURCE7 || payload == SOURCE8);
        return payload == SOURCE7 ? 7 : 8;
    }
    private static GuiAtlasRegion reference(long id, long generation) {
        return new GuiAtlasRegion(id, generation == 7 ? SOURCE7 : SOURCE8, 8, 8, 2, 3, 4, 2);
    }

    @Test void rejectionRetriesTheSameImmutableSnapshotAndAcceptanceIsContextScoped() {
        var table = new GuiAtlasReferencePublication();
        var calls = new AtomicInteger();
        table.flush(GuiAtlasReferencePublicationTest::accepted, (revision, refs) -> fail("empty initial table needs no publication"));
        var reference = reference(101, 7);
        table.stage(reference);
        table.stage(reference);
        var first = new AtomicReference<List<GuiAtlasReferenceRecord>>();
        assertThrows(IllegalStateException.class, () -> table.flush(GuiAtlasReferencePublicationTest::accepted, (revision, refs) -> {
            assertEquals(1L, revision);
            first.set(refs);
            throw new IllegalStateException("native rejection");
        }));
        assertThrows(UnsupportedOperationException.class, () -> first.get().clear());
        table.flush(GuiAtlasReferencePublicationTest::accepted, (revision, refs) -> {
            assertEquals(1L, revision);
            assertEquals(first.get(), refs);
            calls.incrementAndGet();
        });
        table.flush(GuiAtlasReferencePublicationTest::accepted, (revision, refs) -> fail("accepted table should not be republished"));
        table.resetAcceptance();
        table.flush(GuiAtlasReferencePublicationTest::accepted, (revision, refs) -> {
            assertEquals(1L, revision);
            assertEquals(List.of(reference.resolve(GuiAtlasReferencePublicationTest::accepted)), refs);
            calls.incrementAndGet();
        });
        table.stage(reference(101, 8));
        table.flush(GuiAtlasReferencePublicationTest::accepted, (revision, refs) -> {
            assertEquals(2L, revision);
            assertEquals(8L, refs.getFirst().atlasGeneration());
        });
        table.invalidate();
        table.flush(GuiAtlasReferencePublicationTest::accepted, (revision, refs) -> {
            assertEquals(3L, revision);
            assertTrue(refs.isEmpty());
        });
        assertEquals(2, calls.get());
    }

    @Test void capacityRejectsWithoutChangingThePendingRevisionOrDeclarations() {
        var table = new GuiAtlasReferencePublication();
        for (int i = 1; i <= 4096; i++) table.stage(reference(i, 7));
        assertThrows(IllegalStateException.class, () -> table.stage(reference(4097, 7)));
        table.flush(GuiAtlasReferencePublicationTest::accepted, (revision, refs) -> {
            assertEquals(4096L, revision);
            assertEquals(4096, refs.size());
        });
        table.stage(reference(1, 8));
        table.flush(GuiAtlasReferencePublicationTest::accepted, (revision, refs) -> {
            assertEquals(4097L, revision);
            assertEquals(4096, refs.size());
            assertEquals(8L, refs.getFirst().atlasGeneration());
        });
    }

    @Test void typedSourcesKeepAtlasReferencesSeparateFromImmutablePixels() {
        var pixels = new byte[] {1, 2, 3, 4};
        var raw = new GuiItemTextureSource.Raw(new RustGalGuiRawImageAssets.Asset(11, "raw", 1, 1, pixels));
        pixels[0] = 99;
        assertEquals(11L, raw.assetId());
        assertArrayEquals(new byte[] {1, 2, 3, 4}, raw.asset().pixels());
        var declaration = reference(101, 7);
        var atlas = new GuiItemTextureSource.Atlas(declaration);
        assertEquals(101L, atlas.assetId());
        assertSame(declaration, atlas.region());
        assertThrows(NullPointerException.class, () -> new GuiItemTextureSource.Atlas(null));
        assertThrows(NullPointerException.class, () -> new GuiItemTextureSource.Raw(null));
    }

    @Test void pendingSourcesResolveOnlyAtFlushAndRevalidateAfterAcceptance() {
        var table = new GuiAtlasReferencePublication();
        table.stage(reference(101, 7));
        assertThrows(IllegalStateException.class, () -> table.flush(payload -> {
            throw new IllegalStateException("upload pending");
        }, (revision, refs) -> fail("must not publish before exact source acceptance")));
        table.flush(payload -> 11, (revision, refs) -> {
            assertEquals(1L, revision);
            assertEquals(11L, refs.getFirst().atlasGeneration());
        });
        assertThrows(IllegalStateException.class, () -> table.flush(payload -> {
            throw new IllegalStateException("source replaced");
        }, (revision, refs) -> fail("cached declaration cannot bypass source validation")));
        table.flush(payload -> 12, (revision, refs) -> {
            assertEquals(2L, revision);
            assertEquals(12L, refs.getFirst().atlasGeneration());
        });
        table.flush(payload -> 12, (revision, refs) -> fail("unchanged accepted incarnation"));
    }

    @Test void unusedReplacedSourcesRetireButUsedStaleSourcesStillReject() {
        var table = new GuiAtlasReferencePublication();
        table.stage(reference(101, 7));
        table.stage(reference(102, 8));
        table.flush(GuiAtlasReferencePublicationTest::accepted, (revision, refs) -> assertEquals(2, refs.size()));
        table.retainUsed(java.util.stream.LongStream.of(102, 102, 999));
        assertThrows(IllegalStateException.class, () -> table.flush(payload -> {
            assertSame(SOURCE8, payload, "unused old source must not be resolved");
            throw new IllegalStateException("used source is stale");
        }, (revision, refs) -> fail("used stale source must still reject")));
        table.flush(payload -> {
            assertSame(SOURCE8, payload);
            return 9;
        }, (revision, refs) -> {
            assertEquals(3L, revision);
            assertEquals(List.of(102L), refs.stream().map(GuiAtlasReferenceRecord::assetId).toList());
        });
        table.retainUsed(java.util.stream.LongStream.empty());
        assertThrows(IllegalStateException.class, () -> table.flush(payload -> {
            throw new AssertionError("empty replacement must not resolve old sources");
        }, (revision, refs) -> {
            assertTrue(refs.isEmpty());
            throw new IllegalStateException("retirement rejected");
        }));
        table.flush(GuiAtlasReferencePublicationTest::accepted, (revision, refs) -> {
            assertEquals(4L, revision);
            assertTrue(refs.isEmpty());
        });
        for (int id = 1; id <= 4096; id++) table.stage(reference(id, 7));
        table.retainUsed(java.util.stream.LongStream.of(1));
        table.stage(reference(4097, 7));
        table.flush(GuiAtlasReferencePublicationTest::accepted, (revision, refs) -> assertEquals(2, refs.size()));
    }
}
