package net.sodium.client.render.chunk.compile.tasks;

import net.minecraft.client.color.block.BlockColors;
import org.junit.jupiter.api.Test;

import static org.junit.jupiter.api.Assertions.assertEquals;

class NativeSectionSnapshotTintTest {
    @Test
    void leafLitterUsesDryFoliageRatherThanGreenFoliageAcrossCopiedLattice() throws Exception {
        net.minecraft.SharedConstants.tryDetectVersion();
        net.minecraft.server.Bootstrap.bootStrap();
        var colors = BlockColors.createDefault();
        var leafLitter = net.minecraft.world.level.block.Blocks.LEAF_LITTER.defaultBlockState();
        var oak = net.minecraft.world.level.block.Blocks.OAK_LEAVES.defaultBlockState();
        for (var entry : new Object[][] {
                {net.sodium.client.render.chunk.compile.pipeline.NativeStaticBlockModelRegistry.class, "tintType"},
                {net.sodium.client.perf.real.RealChunkMeshingReplayRunner.class, "nativeTintType"}}) {
            var method = ((Class<?>)entry[0]).getDeclaredMethod((String)entry[1], net.minecraft.world.level.block.state.BlockState.class);
            method.setAccessible(true);
            assertEquals(5, method.invoke(null, leafLitter), "leaf litter must retain per-block, not per-vertex, tint semantics");
            assertEquals(2, method.invoke(null, oak), "oak retains blended foliage semantics");
        }
        var getter = (net.minecraft.world.level.BlockAndTintGetter) java.lang.reflect.Proxy.newProxyInstance(
                getClass().getClassLoader(), new Class<?>[] {net.minecraft.world.level.BlockAndTintGetter.class},
                (proxy, method, arguments) -> {
                    if (!method.getName().equals("getBlockTint")) throw new AssertionError("unexpected world query: " + method);
                    var position = (net.minecraft.core.BlockPos)arguments[0];
                    int variation = (position.getX() & 3) << 16 | (position.getY() & 3) << 8 | (position.getZ() & 3);
                    if (arguments[1] == net.minecraft.client.renderer.BiomeColors.DRY_FOLIAGE_COLOR_RESOLVER) return 0x986034 | variation;
                    if (arguments[1] == net.minecraft.client.renderer.BiomeColors.FOLIAGE_COLOR_RESOLVER) return 0x389824 | variation;
                    throw new AssertionError("unexpected color resolver");
                });
        // Exercise every coordinate used by the immutable 4x4x4 tint lattice,
        // against the same authored BlockColors provider used by Frozen OpenGL.
        for (int y = -1; y <= 2; y++) for (int z = -1; z <= 2; z++) for (int x = -1; x <= 2; x++) {
            var pos = new net.minecraft.core.BlockPos(150 + x, 80 + y, 530 + z);
            assertEquals(colors.getColor(leafLitter, getter, pos, 0) | 0xFF000000,
                    NativeSectionSnapshot.blockTint(getter, leafLitter, pos), "leaf litter at " + pos);
            assertEquals(colors.getColor(oak, getter, pos, 0) | 0xFF000000,
                    NativeSectionSnapshot.blockTint(getter, oak, pos), "oak at " + pos);
        }
    }

    @Test
    void normalizeBlockTintPreservesOpaqueNegativeLilyPadColors() {
        assertEquals(0xFF208030, NativeSectionSnapshot.normalizeBlockTintColor(BlockColors.LILY_PAD_IN_WORLD));
        assertEquals(0xFF71C35C, NativeSectionSnapshot.normalizeBlockTintColor(BlockColors.LILY_PAD_DEFAULT));
    }

    @Test
    void normalizeBlockTintOnlyTreatsMinusOneAsMissingTint() {
        assertEquals(-1, NativeSectionSnapshot.normalizeBlockTintColor(-1));
        assertEquals(0xFF00FF00, NativeSectionSnapshot.normalizeBlockTintColor(0x00FF00));
    }
}
