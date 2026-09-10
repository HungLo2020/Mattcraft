package net.minecraft.client.particle;

import java.util.ArrayList;
import java.util.Map;
import net.minecraft.client.Camera;
import net.minecraft.client.renderer.state.QuadParticleRenderState;
import net.minecraft.client.renderer.texture.MissingTextureAtlasSprite;
import net.minecraft.client.renderer.texture.TextureAtlas;
import net.minecraft.client.renderer.texture.TextureAtlasSprite;
import org.junit.jupiter.api.Test;
import static org.junit.jupiter.api.Assertions.*;

class GraphicsAuditShriekParticleFixtureTest {
    @SuppressWarnings("unchecked")
    @Test void ordinaryExtractionEmitsTwoDistinctTranslucentQuadsAndDelayEmitsNone() throws Exception {
        var contents = MissingTextureAtlasSprite.create();
        try {
            var sprite = new TextureAtlasSprite(TextureAtlas.LOCATION_PARTICLES, contents, 16, 16, 0, 0) {};
            var particle = new ShriekParticle(null, 0, 0, -3, 0, sprite);
            GraphicsAuditShriekParticleFixture.configure(particle);
            var state = new QuadParticleRenderState();
            particle.extract(state, new Camera(), 1);
            var field = QuadParticleRenderState.class.getDeclaredField("particles");
            field.setAccessible(true);
            var layers = (Map<SingleQuadParticle.Layer, Object>)field.get(state);
            assertEquals(1, layers.size());
            var quads = new ArrayList<float[]>();
            var storage = layers.get(SingleQuadParticle.Layer.TRANSLUCENT);
            var visit = storage.getClass().getDeclaredMethod("forEachParticle", QuadParticleRenderState.ParticleConsumer.class);
            visit.setAccessible(true);
            visit.invoke(storage, (QuadParticleRenderState.ParticleConsumer)
                (x,y,z,qx,qy,qz,qw,size,u0,u1,v0,v1,color,light) -> {
                    quads.add(new float[]{qx,qy,qz,qw});
                    assertEquals(0.31875F, size, 0.000001F);
                    assertEquals(127, color >>> 24);
                    assertEquals(240, light);
                    assertEquals(-3, z);
                });
            assertEquals(2, quads.size(), "one shriek must not collapse to one billboard");
            assertNotEquals(quads.get(0)[1], quads.get(1)[1]);
            assertEquals(-0.5F, quads.get(0)[0], 0.00001F);
            assertEquals(0.5F, particle.alpha);
            float size = particle.getQuadSize(1);
            particle.age += 10;
            assertEquals(size, particle.getQuadSize(1), "bounded capture duration must preserve fixture size");

            var delayed = new ShriekParticle(null, 0, 0, -3, Integer.MAX_VALUE, sprite);
            GraphicsAuditShriekParticleFixture.configure(delayed);
            state.clear();
            delayed.extract(state, new Camera(), 1);
            visit.invoke(storage, (QuadParticleRenderState.ParticleConsumer)
                (x,y,z,qx,qy,qz,qw,size2,u0,u1,v0,v1,color,light) ->
                    fail("ordinary delay must not emit a quad into retained layer storage"));
        } finally {
            contents.close();
        }
    }
}
