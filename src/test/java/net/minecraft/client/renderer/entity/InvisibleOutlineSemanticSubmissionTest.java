package net.minecraft.client.renderer.entity;

import java.lang.reflect.Proxy;
import net.blaze3d.vertex.PoseStack;
import net.minecraft.client.model.CowModel;
import net.minecraft.client.renderer.SubmitNodeCollector;
import net.minecraft.client.renderer.entity.state.CowRenderState;
import net.minecraft.client.renderer.state.CameraRenderState;
import net.minecraft.resources.ResourceLocation;
import net.minecraft.world.entity.EntityType;
import net.minecraft.world.entity.animal.Cow;
import org.junit.jupiter.api.Test;
import static org.junit.jupiter.api.Assertions.*;
import static org.mockito.Mockito.mock;

class InvisibleOutlineSemanticSubmissionTest {
    @org.junit.jupiter.api.BeforeAll static void bootstrap() {
        net.minecraft.SharedConstants.tryDetectVersion();
        net.minecraft.server.Bootstrap.bootStrap();
    }

    static final class Renderer extends LivingEntityRenderer<Cow, CowRenderState, CowModel> {
        Renderer() { super(mock(EntityRendererProvider.Context.class), new CowModel(CowModel.createBodyLayer().bakeRoot()), 0.0F); }
        @Override public CowRenderState createRenderState() { return new CowRenderState(); }
        @Override public ResourceLocation getTextureLocation(CowRenderState state) {
            return ResourceLocation.withDefaultNamespace("textures/entity/cow/temperate_cow.png");
        }
    }

    @Test
    @SuppressWarnings("unchecked")
    void eachSemanticTraversalEmitsExactlyOneOutlineAndNeverDirectlyQueuesTheBody() throws Exception {
        var field = EntityRenderDispatcher.class.getDeclaredField("SEMANTIC_SUBMISSION");
        field.setAccessible(true);
        var semantic = (ThreadLocal<Boolean>) field.get(null);
        boolean previous = semantic.get();
        String key = "mattmc.dev.rustGalVulkanWholeFrame";
        String property = System.getProperty(key);
        try {
            semantic.set(true);
            System.setProperty(key, "true");
            Renderer renderer = new Renderer();
            CowRenderState state = renderer.createRenderState();
            state.entityType = EntityType.COW;
            state.isInvisible = true;
            state.isInvisibleToPlayer = true;
            state.outlineColor = 0xffffffff;
            for (boolean coverage : new boolean[] {false, true}) {
                int[] calls = {0};
                var collector = (SubmitNodeCollector) Proxy.newProxyInstance(SubmitNodeCollector.class.getClassLoader(),
                    new Class<?>[] {SubmitNodeCollector.class}, (proxy, method, args) -> {
                        if (method.getName().equals("isSemanticCoverageOnly")) return coverage;
                        if (!method.getName().equals("submitModelOutlineSemanticTexture"))
                            throw new AssertionError("unexpected body/other submission: " + method.getName());
                        calls[0]++;
                        assertSame(state, args[1]);
                        assertEquals(renderer.getTextureLocation(state), args[5]);
                        assertEquals(0xffffffff, args[6]);
                        return null;
                    });
                renderer.submit(state, new PoseStack(), collector, new CameraRenderState());
                assertEquals(1, calls[0]);
            }
        } finally {
            semantic.set(previous);
            if (property == null) System.clearProperty(key); else System.setProperty(key, property);
        }
    }
}
