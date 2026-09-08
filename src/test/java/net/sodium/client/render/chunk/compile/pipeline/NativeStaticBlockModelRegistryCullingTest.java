package net.sodium.client.render.chunk.compile.pipeline;

import net.minecraft.SharedConstants;
import net.minecraft.core.Direction;
import net.minecraft.server.Bootstrap;
import net.minecraft.world.level.block.IronBarsBlock;
import net.minecraft.world.level.block.Blocks;
import net.minecraft.world.level.block.state.BlockState;
import org.junit.jupiter.api.BeforeAll;
import org.junit.jupiter.api.Test;

import static org.junit.jupiter.api.Assertions.assertFalse;
import static org.junit.jupiter.api.Assertions.assertTrue;

class NativeStaticBlockModelRegistryCullingTest {
    @BeforeAll
    static void bootstrap() {
        SharedConstants.tryDetectVersion();
        Bootstrap.bootStrap();
    }

    @Test
    void glassBlocksAdvertiseSameBlockSkipRendering() {
        assertTrue(NativeStaticBlockModelRegistry.skipsRenderingAgainstSameBlock(Blocks.GLASS.defaultBlockState()));
        assertTrue(NativeStaticBlockModelRegistry.skipsRenderingAgainstSameBlock(Blocks.WHITE_STAINED_GLASS.defaultBlockState()));
        assertTrue(NativeStaticBlockModelRegistry.skipsRenderingAgainstSameBlock(Blocks.TINTED_GLASS.defaultBlockState()));
        assertMask(NativeStaticBlockModelRegistry.sameBlockSkipMask(Blocks.GLASS.defaultBlockState()),
                Direction.DOWN, Direction.UP, Direction.NORTH, Direction.SOUTH, Direction.WEST, Direction.EAST);
    }

    @Test
    void ordinaryBlocksDoNotReceiveTransparentSkipMasks() {
        assertFalse(NativeStaticBlockModelRegistry.skipsRenderingAgainstSameBlock(Blocks.STONE.defaultBlockState()));
        assertFalse(NativeStaticBlockModelRegistry.skipsRenderingAgainstSameBlock(Blocks.OAK_STAIRS.defaultBlockState()));
        assertMask(NativeStaticBlockModelRegistry.sameBlockSkipMask(Blocks.STONE.defaultBlockState()));
        assertMask(NativeStaticBlockModelRegistry.sameBlockSkipMask(Blocks.OAK_STAIRS.defaultBlockState()));
    }

    @Test
    void paneSkipMasksFollowConnectedFaces() {
        BlockState isolatedPane = Blocks.GLASS_PANE.defaultBlockState();
        assertFalse(NativeStaticBlockModelRegistry.skipsRenderingAgainstSameBlock(isolatedPane));
        assertMask(NativeStaticBlockModelRegistry.sameBlockSkipMask(isolatedPane), Direction.DOWN, Direction.UP);

        BlockState eastWestPane = isolatedPane.setValue(IronBarsBlock.EAST, true).setValue(IronBarsBlock.WEST, true);
        assertMask(NativeStaticBlockModelRegistry.sameBlockSkipMask(eastWestPane),
                Direction.DOWN, Direction.UP, Direction.WEST, Direction.EAST);

        BlockState northSouthPane = isolatedPane.setValue(IronBarsBlock.NORTH, true).setValue(IronBarsBlock.SOUTH, true);
        assertMask(NativeStaticBlockModelRegistry.sameBlockSkipMask(northSouthPane),
                Direction.DOWN, Direction.UP, Direction.NORTH, Direction.SOUTH);

        BlockState stainedPane = Blocks.WHITE_STAINED_GLASS_PANE.defaultBlockState()
                .setValue(IronBarsBlock.NORTH, true)
                .setValue(IronBarsBlock.SOUTH, true);
        assertMask(NativeStaticBlockModelRegistry.sameBlockSkipMask(stainedPane),
                Direction.DOWN, Direction.UP, Direction.NORTH, Direction.SOUTH);
    }

    @Test
    void paneEndpointSkipMasksUseOwnConnectionFacesNotSelfNeighborProperties() {
        BlockState eastEndpoint = Blocks.GLASS_PANE.defaultBlockState()
                .setValue(IronBarsBlock.EAST, true);
        BlockState westNeighbor = Blocks.GLASS_PANE.defaultBlockState()
                .setValue(IronBarsBlock.WEST, true);

        assertTrue(eastEndpoint.skipRendering(westNeighbor, Direction.EAST),
                "Vanilla culls the joined face between endpoint panes with opposite connection state");
        assertMask(NativeStaticBlockModelRegistry.sameBlockSkipMask(eastEndpoint),
                Direction.DOWN, Direction.UP, Direction.EAST);
        assertMask(NativeStaticBlockModelRegistry.sameBlockSkipMask(westNeighbor),
                Direction.DOWN, Direction.UP, Direction.WEST);
    }

    @Test
    void paneCornerSkipMasksIncludeEveryConnectedHorizontalArm() {
        BlockState corner = Blocks.GLASS_PANE.defaultBlockState()
                .setValue(IronBarsBlock.NORTH, true)
                .setValue(IronBarsBlock.EAST, true);
        BlockState southNeighbor = Blocks.GLASS_PANE.defaultBlockState()
                .setValue(IronBarsBlock.SOUTH, true);
        BlockState westNeighbor = Blocks.GLASS_PANE.defaultBlockState()
                .setValue(IronBarsBlock.WEST, true);

        assertTrue(corner.skipRendering(southNeighbor, Direction.NORTH));
        assertTrue(corner.skipRendering(westNeighbor, Direction.EAST));
        assertMask(NativeStaticBlockModelRegistry.sameBlockSkipMask(corner),
                Direction.DOWN, Direction.UP, Direction.NORTH, Direction.EAST);
    }

    private static void assertMask(int mask, Direction... directions) {
        int expected = 0;
        for (Direction direction : directions) {
            expected |= 1 << direction.get3DDataValue();
        }
        org.junit.jupiter.api.Assertions.assertEquals(expected, mask);
    }
}
