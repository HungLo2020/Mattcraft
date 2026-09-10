package net.minecraft.client.renderer.item;

import java.util.ArrayList;
import java.util.List;
import net.blaze3d.vertex.PoseStack;
import net.minecraft.client.renderer.SubmitNodeCollector;
import net.minecraft.client.renderer.entity.ItemEntityRenderer;
import net.minecraft.client.renderer.entity.state.ItemClusterRenderState;
import net.minecraft.client.renderer.special.SpecialModelRenderer;
import net.minecraft.util.RandomSource;
import net.minecraft.world.item.ItemDisplayContext;
import net.minecraft.world.phys.AABB;
import org.joml.Matrix4f;
import org.junit.jupiter.api.Test;
import static org.junit.jupiter.api.Assertions.*;

/** Shared producer contract, supplemental to real paired GPU captures. */
class DroppedItemStackSubmissionParityTest {
    @org.junit.jupiter.api.BeforeAll
    static void bootstrap() {
        net.minecraft.SharedConstants.tryDetectVersion();
        net.minecraft.server.Bootstrap.bootStrap();
    }
    @Test
    void vanillaStackSizeBoundariesSelectTheExpectedNumberOfCopies() {
        int[] amounts = {1, 2, 16, 17, 32, 33, 48, 49, 64};
        int[] copies = {1, 2, 2, 3, 3, 4, 4, 5, 5};
        for (int i = 0; i < amounts.length; i++) {
            assertEquals(copies[i], ItemClusterRenderState.getRenderedAmount(amounts[i]));
        }
    }

    @Test
    void flatAndSolidStacksPreserveEveryCopyAndSeededTransformDuringCoverageReplay() {
        String property = "mattmc.dev.rustGalVulkanWholeFrame";
        String previous = System.getProperty(property);
        System.setProperty(property, "true");
        try {
            for (float depth : new float[] {0.0625F, 1.0F}) {
                for (int count = 0; count <= 5; count++) verifyCopies(count, depth);
            }
        } finally {
            if (previous == null) System.clearProperty(property);
            else System.setProperty(property, previous);
        }
    }

    private void verifyCopies(int count, float depth) {
        var state = new ItemClusterRenderState();
        state.count = count;
        state.seed = 187;
        state.item.displayContext = ItemDisplayContext.GROUND;
        List<Matrix4f> observed = new ArrayList<>();
        // Frozen has no coverage-query API. The identical collector answers it
        // only when Current asks; all rendering callbacks remain mocked.
        SubmitNodeCollector collector = (SubmitNodeCollector) java.lang.reflect.Proxy.newProxyInstance(
            SubmitNodeCollector.class.getClassLoader(), new Class<?>[] {SubmitNodeCollector.class},
            (proxy, method, args) -> {
                if (method.getName().equals("isSemanticCoverageOnly")) return true;
                throw new AssertionError("unexpected collector callback: " + method.getName());
            });
        SpecialModelRenderer<Object> renderer = new SpecialModelRenderer<>() {
            public void submit(Object argument, ItemDisplayContext context, PoseStack submitted,
                    SubmitNodeCollector target, int light, int overlay, boolean foil, int outline) {
                observed.add(new Matrix4f(submitted.last().pose()));
                assertSame(collector, target);
                assertEquals(ItemDisplayContext.GROUND, context);
                assertEquals(15728640, light);
                assertTrue(foil);
            }
            public void getExtents(java.util.Set<org.joml.Vector3f> set) {}
            public Object extractArgument(net.minecraft.world.item.ItemStack stack) { return null; }
        };
        var layer = state.item.newLayer();
        layer.setupSpecialModel(renderer, "stack-semantics");
        layer.setFoilType(ItemStackRenderState.FoilType.STANDARD);
        PoseStack pose = new PoseStack();
        ItemEntityRenderer.submitMultipleFromCount(pose, collector, 15728640,
            state, RandomSource.create(999), new AABB(0, 0, 0, 1, 1, depth));
        assertEquals(count, observed.size(), "each intended stack copy must survive");
        var random = new java.util.Random(187);
        float spacing = depth * 1.5F;
        for (int i = 0; i < count; i++) {
            float x = 0, y = 0, z = depth <= .0625F ? -spacing * (count - 1) / 2 + spacing * i : 0;
            if (i > 0) {
                x = (random.nextFloat() * 2 - 1) * .15F;
                y = (random.nextFloat() * 2 - 1) * .15F;
                if (depth > .0625F) z = (random.nextFloat() * 2 - 1) * .15F;
                else { x *= .5F; y *= .5F; }
            }
            // Vanilla's NO_TRANSFORM still centers the model around its origin.
            assertArrayEquals(new Matrix4f().translation(x - .5F, y - .5F, z - .5F).get(new float[16]),
                observed.get(i).get(new float[16]), 0.000001F);
        }
        assertTrue(pose.isEmpty(), "stack copies must not leak pushed poses");
    }
}
