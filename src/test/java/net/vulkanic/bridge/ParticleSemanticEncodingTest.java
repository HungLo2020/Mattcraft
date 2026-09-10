package net.vulkanic.bridge;

import java.lang.foreign.Arena;
import java.lang.foreign.ValueLayout;
import java.util.List;
import org.junit.jupiter.api.Test;
import static org.junit.jupiter.api.Assertions.*;
import static net.vulkanic.bridge.VulkanicGalBridge.*;

class ParticleSemanticEncodingTest {
    @org.junit.jupiter.api.Test
    void terrainSurfaceEncodingSurvivesOrderingWithoutOrdinaryCoercion() {
        for (var surface : new ParticleSurface[]{ParticleSurface.TERRAIN_OPAQUE,ParticleSurface.TERRAIN_CUTOUT,
                ParticleSurface.TERRAIN_TRANSLUCENT}) {
            var p=new WorldParticleQuadRecord(17,surface,0,new float[]{1,2,3},
                new float[]{0,0,0,1},.35F,new float[]{.5F,.25F,.25F,.5F},-1,240);
            var mapped=mapParticleMaterialOrder(List.of(),List.of(p));
            assertEquals(surface,mapped.getFirst().surface());
            assertEquals(surface==ParticleSurface.TERRAIN_TRANSLUCENT,mapped.getFirst().translucent());
            try (var arena=java.lang.foreign.Arena.ofConfined()) {
                var bytes=encodeParticleQuads(arena,mapped);
                assertEquals(surface==ParticleSurface.TERRAIN_OPAQUE ? 2 : surface==ParticleSurface.TERRAIN_CUTOUT ? 3 : 4,
                    bytes.get(java.lang.foreign.ValueLayout.JAVA_INT,8));
                assertEquals(.35F,bytes.get(java.lang.foreign.ValueLayout.JAVA_FLOAT,
                    Struct.WORLD_PARTICLE_QUAD_REQUEST.offset(6)));
            }
        }
    }
    @Test void mapsParticleOrderAcrossTheExistingMaterialPartition() throws Exception {
        var materials=List.of(material(false),material(true),material(false));
        var particles=new java.util.ArrayList<WorldParticleQuadRecord>();
        for(int index:new int[]{0,1,2,2,3}) particles.add(new WorldParticleQuadRecord(1,true,index,
            new float[3],new float[]{0,0,0,1},1,new float[]{0,1,0,1},-1,240));
        var mapped=mapParticleMaterialOrder(materials,particles);
        assertEquals(List.of(1,2,2,2,3),mapped.stream().map(WorldParticleQuadRecord::materialIndex).toList());
        assertEquals(List.of(0,1,2,2,3),particles.stream().map(WorldParticleQuadRecord::materialIndex).toList());
        assertThrows(IllegalArgumentException.class,()->mapParticleMaterialOrder(materials,List.of(particles.get(4),particles.get(0))));
        assertThrows(IllegalArgumentException.class,()->mapParticleMaterialOrder(List.of(),List.of(particles.get(4))));
    }

    private static WorldMaterialQuadRecord material(boolean modulated) throws Exception {
        var components=WorldMaterialQuadRecord.class.getRecordComponents();
        var types=new Class<?>[components.length];var values=new Object[components.length];
        for(int i=0;i<components.length;i++) {
            var c=components[i];types[i]=c.getType();
            if(c.getType()==float.class) values[i]=Float.valueOf(0);
            else values[i]=Integer.valueOf(c.getName().startsWith("viewport") ? 1280 :
                c.getName().equals("vertex0ColorArgb") && modulated ? 1 : 0);
        }
        return WorldMaterialQuadRecord.class.getDeclaredConstructor(types).newInstance(values);
    }

    @Test void copiesSemanticInputsWithoutExpandingOrNormalizingGeometry() {
        float[] center={1,2,3}, rotation={.2F,-.3F,.4F,.5F}, uv={.8F,.2F,.1F,.9F};
        var p=new WorldParticleQuadRecord(0x50415254,true,2,center,rotation,-.3F,uv,0x80abcdef,240);
        center[0]=99;rotation[0]=99;uv[0]=99;
        p.center()[0]=88;p.rotation()[0]=88;p.uvBounds()[0]=88;
        try(var arena=Arena.ofConfined()) {
            var bytes=encodeParticleQuads(arena,List.of(p));
            var layout=Struct.WORLD_PARTICLE_QUAD_REQUEST;
            assertEquals(72,layout.byteSize());
            assertEquals(0x50415254,bytes.get(ValueLayout.JAVA_INT,layout.offset(1)));
            assertEquals(1,bytes.get(ValueLayout.JAVA_INT,layout.offset(2)));
            assertEquals(2,bytes.get(ValueLayout.JAVA_INT,layout.offset(3)));
            assertArrayEquals(new float[]{1,2,3},bytes.asSlice(layout.offset(4),12).toArray(ValueLayout.JAVA_FLOAT));
            assertArrayEquals(new float[]{.2F,-.3F,.4F,.5F},bytes.asSlice(layout.offset(5),16).toArray(ValueLayout.JAVA_FLOAT));
            assertEquals(-.3F,bytes.get(ValueLayout.JAVA_FLOAT,layout.offset(6)));
            assertArrayEquals(new float[]{.8F,.2F,.1F,.9F},bytes.asSlice(layout.offset(7),16).toArray(ValueLayout.JAVA_FLOAT));
            assertEquals(0x80abcdef,bytes.get(ValueLayout.JAVA_INT,layout.offset(8)));
            assertEquals(240,bytes.get(ValueLayout.JAVA_INT,layout.offset(9)));
            var earlier=new WorldParticleQuadRecord(p.textureId(),true,1,p.center(),p.rotation(),p.size(),p.uvBounds(),p.colorArgb(),p.packedLight());
            assertThrows(IllegalArgumentException.class,()->encodeParticleQuads(arena,List.of(p,earlier)));
        }
    }
}
