package net.minecraft.client.dev;

import java.nio.file.Files;
import java.nio.file.Path;
import org.junit.jupiter.api.Test;
import org.junit.jupiter.api.io.TempDir;
import static org.junit.jupiter.api.Assertions.*;

class NativeTerrainOrderCaptureTest {
    @TempDir Path directory;

    @Test void onlyTheExactPresentedFrameCanSupplyNativeOrderingEvidence() throws Exception {
        Path path = directory.resolve("order.json");
        assertEquals("null", NativeTerrainOrderCapture.read(path, 3));
        String receipt = "{\"schema\":\"mattmc-static-terrain-batch-trace-v1\",\"frameId\":3,\"cameraSortedQuads\":true}";
        Files.writeString(path, receipt);
        assertEquals(receipt, NativeTerrainOrderCapture.read(path, 3));
        assertEquals("null", NativeTerrainOrderCapture.read(path, 2));
        for (String bad : new String[]{"{", "[]", "null", receipt.replace(":3,", ":3.5,"),
                receipt.replace("-v1", "-v2"), " ".repeat(NativeTerrainOrderCapture.MAX_RECEIPT_BYTES + 1)}) {
            Files.writeString(path, bad);
            assertEquals("null", NativeTerrainOrderCapture.read(path, 3));
        }
    }
}
