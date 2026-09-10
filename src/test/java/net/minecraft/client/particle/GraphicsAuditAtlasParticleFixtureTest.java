package net.minecraft.client.particle;
import net.minecraft.world.phys.Vec3;
import org.junit.jupiter.api.Test;
import static org.junit.jupiter.api.Assertions.*;

class GraphicsAuditAtlasParticleFixtureTest {
    @Test void signedSizeRemainsOrdinaryOpaqueFlameInput() {
        String enabled = "mattmc.dev.graphicsAuditAtlasParticle";
        String sign = "mattmc.dev.graphicsAuditAtlasParticleSizeSign";
        String oldEnabled = System.getProperty(enabled), oldSign = System.getProperty(sign);
        try {
            System.setProperty(enabled, "true");
            for (int value : new int[]{1,-1}) {
                System.setProperty(sign, Integer.toString(value));
                var particle = new FlameParticle(null,1,2,3,0,0,0,null);
                GraphicsAuditAtlasParticleFixture.configure(particle,new Vec3(1,2,3));
                assertEquals(FlameParticle.class,particle.getClass(), "no extraction or renderer override");
                assertEquals(SingleQuadParticle.Layer.OPAQUE,particle.getLayer());
                for (int tick=0; tick<20000; tick++) {
                    particle.tick();
                    assertEquals(.35F*value,particle.getQuadSize(.5F));
                    assertEquals(1,particle.x); assertEquals(2,particle.y); assertEquals(3,particle.z);
                }
            }
            for (String invalid : new String[]{"0","2","-2","NaN"}) {
                System.setProperty(sign,invalid);
                assertThrows(IllegalArgumentException.class,GraphicsAuditAtlasParticleFixture::requestedSizeSign);
            }
        } finally {
            if (oldEnabled==null) System.clearProperty(enabled); else System.setProperty(enabled,oldEnabled);
            if (oldSign==null) System.clearProperty(sign); else System.setProperty(sign,oldSign);
        }
    }
    @Test void ordinaryFlameTicksKeepExactFixtureGeometryForMoreThanTheCaptureWindow() {
        var particle = new FlameParticle(null, 1, 2, 3, 0, 0, 0, null);
        String key = "mattmc.dev.graphicsAuditAtlasParticle";
        String previous = System.getProperty(key);
        try {
            System.clearProperty(key);
            assertThrows(IllegalStateException.class, () -> GraphicsAuditAtlasParticleFixture.configure(particle, Vec3.ZERO));
            System.setProperty(key, "true");
            GraphicsAuditAtlasParticleFixture.configure(particle, new Vec3(1,2,3));
            for (int tick = 0; tick < 20000; tick++) {
                particle.tick();
                assertTrue(particle.isAlive());
                assertEquals(1, particle.x); assertEquals(2, particle.y); assertEquals(3, particle.z);
                for (float partial : new float[]{0, 0.5F, 1})
                    assertEquals(Float.floatToRawIntBits(0.35F), Float.floatToRawIntBits(particle.getQuadSize(partial)));
            }
            assertEquals(20000, particle.age, "the fixture does not suppress normal simulation ticks");
        } finally {
            if (previous == null) System.clearProperty(key); else System.setProperty(key, previous);
        }
    }
}
