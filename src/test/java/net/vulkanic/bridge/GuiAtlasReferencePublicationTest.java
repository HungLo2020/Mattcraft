package net.vulkanic.bridge;

import java.awt.image.BufferedImage;
import java.io.ByteArrayOutputStream;
import java.util.List;
import javax.imageio.ImageIO;
import org.junit.jupiter.api.Test;
import static org.junit.jupiter.api.Assertions.*;

class GuiAtlasReferencePublicationTest {
    private static VulkanicGalBridge.GuiAtlasReferenceRecord reference(long asset, long generation) {
        return new VulkanicGalBridge.GuiAtlasReferenceRecord(asset, 17, generation, 8, 8, 2, 3, 4, 2);
    }

    @Test void nativeOwnerAcceptanceAndAtomicReplacementAreRequired() throws Exception {
        var output = new ByteArrayOutputStream();
        assertTrue(ImageIO.write(new BufferedImage(8, 8, BufferedImage.TYPE_INT_ARGB), "PNG", output));
        var texture = new VulkanicGalBridge.WorldMeshTextureAssetRecord(17, output.toByteArray()).withMipLevels(1);
        var first = reference(101, 7);
        var copied = new VulkanicGalBridge.GuiRawImageAssetRecord(101, 2, 1, 1, new byte[4]);
        try (var bridge = VulkanicGalBridge.create("rust-vulkan")) {
            assertThrows(IllegalStateException.class, () -> bridge.updateGuiAtlasReferences(1, List.of(first)));
            bridge.updateWorldMeshAssets(7, List.of(), List.of(texture), List.of());
            bridge.updateGuiAtlasReferences(1, List.of(first));
            bridge.updateGuiAtlasReferences(1, List.of(first));
            assertThrows(IllegalStateException.class, () -> bridge.updateGuiAtlasReferences(2, List.of(first, first)));
            assertThrows(IllegalStateException.class, () -> bridge.updateGuiRawImages(1, List.of(copied)),
                "failed declaration update must preserve the original identity collision");
            bridge.updateGuiAtlasReferences(2, List.of(reference(102, 7)));
            bridge.updateGuiRawImages(1, List.of(copied));
            assertThrows(IllegalStateException.class, () -> bridge.updateGuiAtlasReferences(3, List.of(first)));
            bridge.updateWorldMeshAssets(8, List.of(), List.of(texture), List.of());
            assertThrows(IllegalStateException.class, () -> bridge.updateGuiAtlasReferences(3, List.of(reference(102, 7))));
            bridge.updateGuiAtlasReferences(3, List.of(reference(102, 8)));
            bridge.updateGuiAtlasReferences(4, List.of());
            bridge.updateGuiRawImages(2, List.of(new VulkanicGalBridge.GuiRawImageAssetRecord(102, 2, 1, 1, new byte[4])));
        }
    }

    @Test void semanticRecordsRejectOverflowAndInvalidRegionsWithoutNativeState() {
        assertThrows(IllegalArgumentException.class, () -> new VulkanicGalBridge.GuiAtlasReferenceRecord(1, 17, 1,
            8, 8, Integer.MAX_VALUE, 0, 2, 2));
        assertThrows(IllegalArgumentException.class, () -> new VulkanicGalBridge.GuiAtlasReferenceRecord(1, 17, 1,
            8, 8, 0, 0, 0, 2));
        assertThrows(IllegalArgumentException.class, () -> new VulkanicGalBridge.GuiAtlasReferenceRecord(1, 17, 0,
            8, 8, 0, 0, 2, 2));
    }
}
