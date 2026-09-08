package net.minecraft.client.dev;

import org.junit.jupiter.api.Test;
import static org.junit.jupiter.api.Assertions.*;

class GraphicsAuditPostEffectFixtureTest {
    @Test void fixtureIsOptInAndOnlyAcceptsNamedVanillaEffects() {
        for (String effect : new String[]{"", "invert", "creeper", "spider"})
            assertEquals(effect, GraphicsAuditPostEffectFixture.validate(effect));
        for (String effect : new String[]{"custom", "minecraft:invert", "INVERT", "../invert", " "})
            assertThrows(IllegalArgumentException.class, () -> GraphicsAuditPostEffectFixture.validate(effect));
    }
}

