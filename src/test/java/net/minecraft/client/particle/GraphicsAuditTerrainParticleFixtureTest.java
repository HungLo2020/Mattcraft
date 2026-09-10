package net.minecraft.client.particle;
import org.junit.jupiter.api.Test;
import net.minecraft.world.phys.Vec3;
import static org.junit.jupiter.api.Assertions.*;

class GraphicsAuditTerrainParticleFixtureTest {
    @Test void fractionalSourceAlphaIsDistinguishedFromCutoutAndOpaque() {
        try(var pixels=new net.blaze3d.platform.NativeImage(16,16,true)) {
            for(int y=0;y<16;y++) for(int x=0;x<16;x++) pixels.setPixel(x,y,0x80405090);
            var evidence=GraphicsAuditTerrainParticleFixture.sourcePixels(pixels);
            assertEquals(16,evidence.get("quarterTranslucentPixels").getAsInt());
            assertEquals(0,evidence.get("quarterTransparentPixels").getAsInt());
            assertEquals(0,evidence.get("quarterOpaquePixels").getAsInt());
        }
    }
    @Test void cutoutSourceEvidenceCountsTheSampledQuarterAndHashesAllPixels() {
        try(var pixels=new net.blaze3d.platform.NativeImage(16,16,true)) {
            for(int y=0;y<16;y++) for(int x=0;x<16;x++) pixels.setPixel(x,y,0xff307050);
            for(int y=4;y<8;y++) for(int x=4;x<6;x++) pixels.setPixel(x,y,0);
            var evidence=GraphicsAuditTerrainParticleFixture.sourcePixels(pixels);
            assertEquals(8,evidence.get("quarterTransparentPixels").getAsInt());
            assertEquals(8,evidence.get("quarterOpaquePixels").getAsInt());
            String hash=evidence.get("rgbaFnv64").getAsString();
            pixels.setPixel(0,0,0xff102030);
            var changed=GraphicsAuditTerrainParticleFixture.sourcePixels(pixels);
            assertNotEquals(hash,changed.get("rgbaFnv64").getAsString());
            assertEquals(8,changed.get("quarterTransparentPixels").getAsInt());
        }
    }
    @Test void cutoutFixtureIsExplicitlyRequestedAndHiddenControlIsScoped() {
        String enabled="mattmc.dev.graphicsAuditCutoutTerrainParticle";
        String hidden="mattmc.dev.graphicsAuditCutoutTerrainHidden";
        String oldEnabled=System.getProperty(enabled),oldHidden=System.getProperty(hidden);
        try {
            System.clearProperty(enabled); System.setProperty(hidden,"true");
            assertFalse(GraphicsAuditTerrainParticleFixture.cutoutRequested());
            assertFalse(GraphicsAuditTerrainParticleFixture.hidden());
            System.setProperty(enabled,"true");
            assertTrue(GraphicsAuditTerrainParticleFixture.requested());
            assertTrue(GraphicsAuditTerrainParticleFixture.hidden());
        } finally {
            if(oldEnabled==null)System.clearProperty(enabled);else System.setProperty(enabled,oldEnabled);
            if(oldHidden==null)System.clearProperty(hidden);else System.setProperty(hidden,oldHidden);
        }
    }
    @Test void frozenTickFixtureRequiresRealEngineMembershipWithoutAdvancingSimulation() {
        var engine = new ParticleEngine(null, null);
        var value = new Particle(null, 1, 2, 3) {
            @Override public ParticleRenderType getGroup() { return ParticleRenderType.NO_RENDER; }
            @Override public void tick() { throw new AssertionError("fixture must not tick simulation"); }
        };
        String key = "mattmc.dev.graphicsAuditMagmaParticle";
        String previous = System.getProperty(key);
        try {
            System.clearProperty(key);
            assertThrows(IllegalStateException.class, () -> engine.installGraphicsAuditParticle(value));
            engine.add(value);
            assertTrue(value.isAlive());
            assertFalse(engine.containsGraphicsAuditParticle(value), "alive in pending queue is not rendered membership");
            engine.clearParticles();
            System.setProperty(key, "true");
            engine.installGraphicsAuditParticle(value);
            assertTrue(engine.containsGraphicsAuditParticle(value));
            assertEquals("1", engine.countParticles());
            engine.clearParticles();
            assertFalse(engine.containsGraphicsAuditParticle(value));
        } finally {
            if (previous == null) System.clearProperty(key); else System.setProperty(key, previous);
        }
    }
    @Test void ordinaryConstructionRetainsItsInputs() {
        assertEquals(0.37F, GraphicsAuditTerrainParticleFixture.offset(0.37F));
        GraphicsAuditTerrainParticleFixture.configure(null); // ordinary path does not touch particles
    }
    @Test void fixtureScopeRestoresOrdinaryInputsAfterSuccessAndFailure() {
        assertEquals(1.0F, GraphicsAuditTerrainParticleFixture.construct(
            () -> GraphicsAuditTerrainParticleFixture.offset(0.37F)));
        assertEquals(0.37F, GraphicsAuditTerrainParticleFixture.offset(0.37F));
        assertThrows(IllegalArgumentException.class, () -> GraphicsAuditTerrainParticleFixture.construct(
            () -> { throw new IllegalArgumentException("fixture construction failed"); }));
        assertEquals(0.37F, GraphicsAuditTerrainParticleFixture.offset(0.37F));
        assertThrows(IllegalStateException.class, () -> GraphicsAuditTerrainParticleFixture.construct(
            () -> GraphicsAuditTerrainParticleFixture.construct(() -> 1)));
        assertEquals(0.37F, GraphicsAuditTerrainParticleFixture.offset(0.37F));
    }
    @Test void positionIsThreeBlocksAlongNormalizedCameraDirection() {
        assertEquals(new Vec3(1,2,6), GraphicsAuditTerrainParticleFixture.position(new Vec3(1,2,3), new Vec3(0,0,8)));
        assertThrows(IllegalArgumentException.class, () -> GraphicsAuditTerrainParticleFixture.position(Vec3.ZERO, Vec3.ZERO));
    }
}
