package net.sodium.client.render.chunk.compile.pipeline;

import net.fabricmc.fabric.api.client.render.fluid.v1.FluidRenderHandlerRegistry;
import net.minecraft.SharedConstants;
import net.minecraft.server.Bootstrap;
import net.minecraft.world.level.block.Blocks;
import org.junit.jupiter.api.BeforeAll;
import org.junit.jupiter.api.Test;
import static org.junit.jupiter.api.Assertions.*;

class NativeFluidOverlayPropertyTest {
    @BeforeAll static void bootstrap() {
        SharedConstants.tryDetectVersion();
        Bootstrap.bootStrap();
    }
    @Test void copiedFlagMatchesThePlatformRegistryForEveryVanillaState() {
        var registry = FluidRenderHandlerRegistry.INSTANCE;
        for (var block : net.minecraft.core.registries.BuiltInRegistries.BLOCK) {
            for (var state : block.getStateDefinition().getPossibleStates()) {
                assertEquals(registry.isBlockTransparent(block)
                        ? NativeStaticBlockModelRegistry.STATE_FLAG_FLUID_OVERLAY_TRANSPARENT : 0,
                    NativeStaticBlockModelRegistry.fluidOverlayFlags(state), state.toString());
            }
        }
        assertEquals(0, NativeStaticBlockModelRegistry.fluidOverlayFlags(Blocks.SHORT_GRASS.defaultBlockState()));
        assertFalse(Blocks.OAK_DOOR.defaultBlockState().canOcclude());
        assertFalse(Blocks.OAK_DOOR.defaultBlockState().isAir());
        assertEquals(0, NativeStaticBlockModelRegistry.fluidOverlayFlags(Blocks.OAK_DOOR.defaultBlockState()));
        assertNotEquals(0, NativeStaticBlockModelRegistry.fluidOverlayFlags(Blocks.ICE.defaultBlockState()));
        assertNotEquals(0, NativeStaticBlockModelRegistry.fluidOverlayFlags(Blocks.OAK_LEAVES.defaultBlockState()));
    }
    @Test void explicitTransparencyOverridesAreCopiedWithoutInferringBlockShape() {
        var registry = FluidRenderHandlerRegistry.INSTANCE;
        for (var block : new net.minecraft.world.level.block.Block[] {Blocks.STONE, Blocks.GLASS}) {
            boolean original = registry.isBlockTransparent(block);
            try {
                registry.setBlockTransparency(block, true);
                assertEquals(1 << 10, NativeStaticBlockModelRegistry.fluidOverlayFlags(block.defaultBlockState()));
                registry.setBlockTransparency(block, false);
                assertEquals(0, NativeStaticBlockModelRegistry.fluidOverlayFlags(block.defaultBlockState()));
            } finally {
                registry.setBlockTransparency(block, original);
            }
        }
    }
}
