package net.minecraft.client.particle;

import java.util.ArrayList;
import java.util.Map;
import net.minecraft.client.Camera;
import net.minecraft.client.renderer.state.QuadParticleRenderState;
import net.minecraft.client.renderer.texture.MissingTextureAtlasSprite;
import net.minecraft.client.renderer.texture.TextureAtlas;
import net.minecraft.client.renderer.texture.TextureAtlasSprite;
import net.minecraft.world.phys.Vec3;
import org.junit.jupiter.api.Test;
import static org.junit.jupiter.api.Assertions.*;

class GraphicsAuditVibrationParticleFixtureTest {
    @Test void elevationTranslatesOrdinarySourceAndTargetWithoutChangingOrientation() throws Exception {
        var contents = MissingTextureAtlasSprite.create();
        try {
            var sprite = new TextureAtlasSprite(TextureAtlas.LOCATION_PARTICLES, contents, 16,16,0,0) {};
            var origin = new Vec3(147.64617596880336,101.0991813627766,529.7355156371002);
            var ground = GraphicsAuditVibrationParticleFixture.create(null,origin,sprite,false,0,0);
            var elevated = GraphicsAuditVibrationParticleFixture.create(null,origin,sprite,false,0,1);
            assertEquals(ground.x,elevated.x);
            assertEquals(ground.y+1,elevated.y);
            assertEquals(ground.z,elevated.z);
            for (String name : new String[]{"rot","rotO","pitch","pitchO"}) {
                var field=VibrationSignalParticle.class.getDeclaredField(name);
                field.setAccessible(true);
                assertEquals(field.getFloat(ground),field.getFloat(elevated),name);
            }
            assertEquals(ground.getQuadSize(1),elevated.getQuadSize(1));
            assertEquals(ground.age,elevated.age);
            assertEquals(ground.lifetime,elevated.lifetime);
            for (int i=0;i<100;i++) elevated.tick();
            assertEquals(origin.y+1,elevated.y);
            assertThrows(IllegalArgumentException.class,()->GraphicsAuditVibrationParticleFixture.create(null,origin,sprite,false,0,2));
            assertThrows(IllegalArgumentException.class,()->GraphicsAuditVibrationParticleFixture.create(null,origin,sprite,false,6,1));
        } finally { contents.close(); }
    }

    @Test void sixStepSnapshotUsesOrdinarySimulationAndThenHolds() {
        var contents = MissingTextureAtlasSprite.create();
        try {
            var sprite = new TextureAtlasSprite(TextureAtlas.LOCATION_PARTICLES, contents, 16,16,0,0) {};
            var origin = new Vec3(147.64617596880336,101.0991813627766,529.7355156371002);
            var target = new net.minecraft.world.level.gameevent.BlockPositionSource(
                net.minecraft.core.BlockPos.containing(origin.add(2,1,-3)));
            var ordinary = new VibrationSignalParticle(null,origin.x,origin.y,origin.z,target,40,sprite);
            ordinary.age = 12;
            for (int i=0; i<6; i++) ordinary.tick();
            var snapshot = GraphicsAuditVibrationParticleFixture.create(null,origin,sprite,false,6);
            for (int i=0; i<100; i++) snapshot.tick();
            assertEquals(18,snapshot.age);
            assertEquals(ordinary.x,snapshot.x);
            assertEquals(ordinary.y,snapshot.y);
            assertEquals(ordinary.z,snapshot.z);
            assertEquals(ordinary.xo,snapshot.xo);
            var destination = target.getPosition(null).orElseThrow();
            assertEquals(origin.x+(destination.x-origin.x)*6/27,snapshot.x,1e-12);
            assertEquals(origin.y+(destination.y-origin.y)*6/27,snapshot.y,1e-12);
            assertEquals(origin.z+(destination.z-origin.z)*6/27,snapshot.z,1e-12);
            assertThrows(IllegalArgumentException.class,
                () -> GraphicsAuditVibrationParticleFixture.create(null,origin,sprite,false,7));
        } finally { contents.close(); }
    }
    @SuppressWarnings("unchecked")
    @Test void ordinaryExtractionKeepsBothTargetOrientedQuadsAndSimulationIsFixed() throws Exception {
        var contents = MissingTextureAtlasSprite.create();
        try {
            var sprite = new TextureAtlasSprite(TextureAtlas.LOCATION_PARTICLES, contents, 16,16,0,0) {};
            var origin = new Vec3(1,2,-3);
            for (boolean hidden : new boolean[]{false,true}) {
                var particle = GraphicsAuditVibrationParticleFixture.create(null, origin, sprite, hidden);
                for (int i=0; i<100; i++) particle.tick();
                assertEquals(12, particle.age);
                assertEquals(40, particle.lifetime);
                assertEquals(origin.x, particle.x);
                assertEquals(origin.y, particle.y);
                assertEquals(origin.z, particle.z);
                var state = new QuadParticleRenderState();
                particle.extract(state, new Camera(), 0.25F);
                var field = QuadParticleRenderState.class.getDeclaredField("particles");
                field.setAccessible(true);
                var layers = (Map<SingleQuadParticle.Layer,Object>)field.get(state);
                assertEquals(1, layers.size());
                var storage = layers.get(SingleQuadParticle.Layer.TRANSLUCENT);
                var visit = storage.getClass().getDeclaredMethod("forEachParticle", QuadParticleRenderState.ParticleConsumer.class);
                visit.setAccessible(true);
                var quads = new ArrayList<float[]>();
                visit.invoke(storage, (QuadParticleRenderState.ParticleConsumer)
                    (x,y,z,qx,qy,qz,qw,size,u0,u1,v0,v1,color,light) -> {
                        assertEquals(hidden ? 0 : 255, color >>> 24);
                        assertEquals(240, light);
                        assertEquals(0.3F, size);
                        assertEquals(origin.x, x);
                        assertEquals(origin.y, y);
                        assertEquals(origin.z, z);
                        assertEquals(1F, qx*qx+qy*qy+qz*qz+qw*qw, 0.00001F);
                        quads.add(new float[]{qx,qy,qz,qw});
                    });
                assertEquals(2, quads.size());
                assertFalse(java.util.Arrays.equals(quads.get(0),quads.get(1)));
                assertNotEquals(0F, quads.get(0)[0]);
                assertNotEquals(0F, quads.get(0)[1]);
                var first = quads.get(0).clone();
                state.clear();
                particle.extract(state, new Camera(), 0.9F);
                quads.clear();
                visit.invoke(storage, (QuadParticleRenderState.ParticleConsumer)
                    (x,y,z,qx,qy,qz,qw,size,u0,u1,v0,v1,color,light) ->
                        quads.add(new float[]{qx,qy,qz,qw}));
                assertArrayEquals(first,quads.get(0), "fixture sub-tick input must not depend on launcher timing");
            }
        } finally { contents.close(); }
    }
}
