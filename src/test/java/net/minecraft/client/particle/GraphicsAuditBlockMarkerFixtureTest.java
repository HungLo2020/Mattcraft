package net.minecraft.client.particle;

import net.minecraft.world.level.block.Blocks;
import net.minecraft.world.level.block.LightBlock;
import net.minecraft.world.phys.Vec3;
import org.junit.jupiter.api.Test;
import static org.junit.jupiter.api.Assertions.*;

class GraphicsAuditBlockMarkerFixtureTest {
    @org.junit.jupiter.api.BeforeAll static void bootstrap() {
        net.minecraft.SharedConstants.tryDetectVersion();
        net.minecraft.server.Bootstrap.bootStrap();
    }
    @Test void allActualLightLevelsAndBarrierAreSelectedWithoutSubstitutes() {
        assertTrue(GraphicsAuditBlockMarkerFixture.state("barrier").is(Blocks.BARRIER));
        for (int level=0;level<16;level++) {
            var state=GraphicsAuditBlockMarkerFixture.state("light-"+level);
            assertTrue(state.is(Blocks.LIGHT));
            assertEquals(level,state.getValue(LightBlock.LEVEL));
        }
        for(String value:new String[]{"", "light-16", "light--1", "light-01", "stone", "light-x"})
            assertThrows(IllegalArgumentException.class,()->GraphicsAuditBlockMarkerFixture.state(value));
    }
    @Test void positioningKeepsFullMarkerAboveTheTerrain() {
        assertEquals(new Vec3(1,3,6), GraphicsAuditBlockMarkerFixture.position(
            new Vec3(1,2,3),new Vec3(0,0,8)));
        assertThrows(IllegalArgumentException.class,()->GraphicsAuditBlockMarkerFixture.position(Vec3.ZERO,Vec3.ZERO));
    }
    @Test void installationNeedsExplicitRequestAndRealGroupMembership() {
        String key=GraphicsAuditBlockMarkerFixture.PROPERTY, previous=System.getProperty(key);
        var engine=new ParticleEngine(null,null);
        var value=new Particle(null,1,2,3) {
            @Override public ParticleRenderType getGroup() {return ParticleRenderType.NO_RENDER;}
            @Override public void tick() {throw new AssertionError("capture installation must not tick");}
        };
        try {
            System.clearProperty(key);
            assertFalse(GraphicsAuditBlockMarkerFixture.requested());
            assertThrows(IllegalStateException.class,()->engine.installGraphicsAuditParticle(value));
            System.setProperty(key,"barrier");
            engine.installGraphicsAuditParticle(value);
            assertTrue(engine.containsGraphicsAuditParticle(value));
            assertTrue(GraphicsAuditBlockMarkerFixture.retained(engine,value));
            assertFalse(GraphicsAuditBlockMarkerFixture.retained(engine,null));
            assertEquals("1",engine.countParticles());
            engine.clearParticles();
            assertFalse(engine.containsGraphicsAuditParticle(value));
            assertTrue(value.isAlive(), "reload detaches a still-alive particle");
            assertFalse(GraphicsAuditBlockMarkerFixture.retained(engine,value));
        } finally {
            if(previous==null)System.clearProperty(key);else System.setProperty(key,previous);
        }
    }
}
