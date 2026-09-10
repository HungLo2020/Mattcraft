package net.minecraft.client.renderer.item;

import java.lang.reflect.Proxy;
import net.blaze3d.vertex.PoseStack;
import net.minecraft.client.model.TridentModel;
import net.minecraft.client.renderer.SubmitNodeCollector;
import net.minecraft.client.renderer.special.TridentSpecialRenderer;
import net.minecraft.world.item.ItemDisplayContext;
import org.junit.jupiter.api.Test;
import static org.junit.jupiter.api.Assertions.*;

class TridentFoilSemanticReplayTest {
    @org.junit.jupiter.api.BeforeAll static void bootstrap() {
        net.minecraft.SharedConstants.tryDetectVersion();
        net.minecraft.server.Bootstrap.bootStrap();
    }

    @Test void coverageVisitsRealTridentSemanticsWithoutGpuOrNativeEnqueue() {
        String property = "mattmc.dev.rustGalVulkanWholeFrame";
        String previous = System.getProperty(property);
        System.setProperty(property, "true");
        try {
            var model = new TridentModel(TridentModel.createLayer().bakeRoot());
            var renderer = new TridentSpecialRenderer(model);
            for (boolean foil : new boolean[]{false, true}) {
                int[] visits = {0};
                var collector = (SubmitNodeCollector) Proxy.newProxyInstance(
                    SubmitNodeCollector.class.getClassLoader(), new Class<?>[]{SubmitNodeCollector.class},
                    (proxy, method, args) -> {
                        if (method.getName().equals("isSemanticCoverageOnly")) return true;
                        if (!method.getName().equals("submitModelPartSemantic")) {
                            throw new AssertionError("unexpected submission: " + method.getName());
                        }
                        visits[0]++;
                        assertSame(model.root(), args[0]);
                        assertEquals(15728640, args[3]);
                        assertEquals(0, args[4]);
                        assertNull(args[5]);
                        assertEquals(false, args[6]);
                        assertEquals(foil, args[7]);
                        var pose = ((PoseStack)args[1]).last().pose();
                        assertEquals(-1.0F, pose.m11());
                        assertEquals(-1.0F, pose.m22());
                        return null;
                    });
                var pose = new PoseStack();
                renderer.submit(ItemDisplayContext.FIRST_PERSON_RIGHT_HAND, pose, collector, 15728640, 0, foil, 0);
                assertEquals(1, visits[0]);
                assertTrue(pose.isEmpty());
                assertTrue(pose.last().pose().equals(new org.joml.Matrix4f()));
            }
        } finally {
            if (previous == null) System.clearProperty(property); else System.setProperty(property, previous);
        }
    }
}
