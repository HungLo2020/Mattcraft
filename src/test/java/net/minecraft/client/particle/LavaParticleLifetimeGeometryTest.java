package net.minecraft.client.particle;

import java.lang.reflect.Proxy;
import java.util.ArrayList;
import net.blaze3d.vertex.VertexConsumer;
import net.minecraft.client.renderer.state.QuadParticleRenderState;
import org.junit.jupiter.api.Test;
import static org.junit.jupiter.api.Assertions.*;

/** Executes the ordinary lifetime formula and CPU vertex emitter without a GPU. */
class LavaParticleLifetimeGeometryTest {
    @Test void finalLiveTickHasZeroAndSignedSizesWithUnchangedVertexOrientation() {
        var particle = new LavaParticle(null, 0, 0, 0, null);
        particle.age = 42;
        particle.lifetime = 43;
        particle.quadSize = 0.25F;
        particle.hasPhysics = false;
        particle.tick();
        assertEquals(43, particle.age);
        assertTrue(particle.isAlive());
        assertEquals(0F, particle.getQuadSize(0F));
        assertTrue(particle.getQuadSize(0.5F) < 0);
        assertTrue(particle.getQuadSize(1F) < 0);
        for (float partial : new float[] {0F, 0.5F, 1F}) {
            float size = particle.getQuadSize(partial);
            float[] actual = new Emitter().vertices(size);
            assertArrayEquals(new float[] {
                1 + size, 2 - size, 3, 1 + size, 2 + size, 3,
                1 - size, 2 + size, 3, 1 - size, 2 - size, 3}, actual, 1e-7F);
        }
        particle.tick();
        assertFalse(particle.isAlive());
    }
    private static class Emitter extends QuadParticleRenderState {
        float[] vertices(float size) {
            var values = new ArrayList<Float>();
            var consumer = (VertexConsumer) Proxy.newProxyInstance(VertexConsumer.class.getClassLoader(),
                new Class<?>[] {VertexConsumer.class}, (proxy, method, args) -> {
                    if (method.getName().equals("addVertex") && args.length == 3)
                        for (Object value : args) values.add((Float)value);
                    return proxy;
                });
            renderRotatedQuad(consumer, 1, 2, 3, 0, 0, 0, 1, size, 0, 1, 0, 1, -1, 240);
            float[] result = new float[values.size()];
            for (int i = 0; i < result.length; i++) result[i] = values.get(i);
            return result;
        }
    }
}
