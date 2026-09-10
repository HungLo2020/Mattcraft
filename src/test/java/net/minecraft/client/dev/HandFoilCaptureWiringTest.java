package net.minecraft.client.dev;

import java.nio.file.Files;
import java.nio.file.Path;
import org.junit.jupiter.api.Test;
import static org.junit.jupiter.api.Assertions.*;

class HandFoilCaptureWiringTest {
    @Test void nativeShellResetsObserversWithoutEnteringTheLegacyRenderer() throws Exception {
        String source = Files.readString(Path.of("src/main/java/net/minecraft/client/renderer/GameRenderer.java"));
        int shell = source.indexOf("public boolean renderRustVulkanWholeFrameShell(");
        int begin = source.indexOf("GraphicsAuditHandFoilTiming.beginFrame()",shell);
        int hand = source.indexOf("this.itemInHandRenderer.renderRustVulkanHands(",shell);
        assertTrue(shell >= 0 && begin > shell && hand > begin);
        String legacy = source.substring(source.indexOf("public void render(DeltaTracker"),shell);
        assertTrue(legacy.contains("Java GameRenderer.render is unavailable while Rust Vulkan owns the whole frame"));
    }
}
