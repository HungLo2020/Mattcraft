package net.minecraft.client.dev;

import com.google.gson.JsonParser;
import java.nio.file.Files;
import java.nio.file.Path;

/** Capture-only CPU receipt reader. Never participates in render decisions. */
final class NativeTerrainOrderCapture {
    static final int MAX_RECEIPT_BYTES = 512 * 1024;

    static String read(Path path, long presentedFrame) {
        if (presentedFrame <= 0) return "null";
        try {
            if (!Files.isRegularFile(path) || Files.size(path) > MAX_RECEIPT_BYTES) return "null";
            // Bound the read as well as checking size: a concurrent diagnostic
            // writer cannot turn the earlier size observation into permission
            // for an unbounded allocation.
            byte[] bytes;
            try (var stream = Files.newInputStream(path)) {
                bytes = stream.readNBytes(MAX_RECEIPT_BYTES + 1);
            }
            if (bytes.length > MAX_RECEIPT_BYTES) return "null";
            var json = JsonParser.parseString(new String(bytes, java.nio.charset.StandardCharsets.UTF_8)).getAsJsonObject();
            if (!"mattmc-static-terrain-batch-trace-v1".equals(json.get("schema").getAsString())
                    || Long.parseLong(json.get("frameId").getAsString()) != presentedFrame) return "null";
            return json.toString();
        } catch (Exception ignored) {
            // Missing, stale, truncated or malformed evidence fails the parity
            // gate; it must not alter gameplay or crash the rendering thread.
            return "null";
        }
    }

    private NativeTerrainOrderCapture() {}
}
