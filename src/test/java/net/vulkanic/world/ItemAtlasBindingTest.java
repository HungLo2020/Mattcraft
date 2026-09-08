package net.vulkanic.world;

import java.lang.reflect.Method;
import java.util.List;
import java.util.Optional;
import net.blaze3d.platform.NativeImage;
import net.minecraft.client.renderer.block.model.BakedQuad;
import net.minecraft.client.renderer.texture.SpriteContents;
import net.minecraft.client.renderer.texture.TextureAtlas;
import net.minecraft.client.renderer.texture.TextureAtlasSprite;
import net.minecraft.client.resources.metadata.animation.AnimationMetadataSection;
import net.minecraft.client.resources.metadata.animation.FrameSize;
import net.minecraft.core.Direction;
import net.minecraft.resources.ResourceLocation;
import net.vulkanic.bridge.VulkanicGalBridge;
import org.joml.Matrix4f;
import org.junit.jupiter.api.Test;
import static org.junit.jupiter.api.Assertions.*;

class ItemAtlasBindingTest {
    @org.junit.jupiter.api.BeforeAll
    static void bootstrap() {
        net.minecraft.SharedConstants.tryDetectVersion();
        net.minecraft.server.Bootstrap.bootStrap();
    }
    private static final class Sprite extends TextureAtlasSprite {
        Sprite(SpriteContents contents) { super(TextureAtlas.LOCATION_BLOCKS, contents, 16, 16, 4, 8); }
    }

    @Test
    void animatedBakedItemReferencesOwnedAtlasWithoutJavaFrameExtractionOrTexturePublication() throws Exception {
        String property = "mattmc.dev.rustGalAtlasAnimation";
        String previous = System.getProperty(property);
        var name = ResourceLocation.withDefaultNamespace("audit/animated-item");
        try (var contents = new SpriteContents(name, new FrameSize(4, 4), new NativeImage(4, 8, true),
            Optional.of(new AnimationMetadataSection(Optional.empty(), Optional.of(4), Optional.of(4), 2, true)), List.of());
             var resource = new AtlasAnimationResource(TextureAtlas.LOCATION_BLOCKS,
                 RustGalWorldPrimitiveRenderer.MATERIAL_TEXTURE_TERRAIN_BLOCK_ATLAS,
                 new net.minecraft.client.renderer.texture.SemanticAtlasAnimationSource(
                 77, 16, 16, 1, List.of(new net.minecraft.client.renderer.texture.SemanticAtlasAnimationSource.Sprite(
                     11, name, 4, 8, contents.semanticAnimationSource().orElseThrow()))))) {
            var sprite = new Sprite(contents);
            var bind = TextureAtlasSprite.class.getDeclaredMethod("bindSemanticAnimationResource", AtlasAnimationResource.class);
            bind.setAccessible(true);
            bind.invoke(sprite, resource);
            int[] packed = new int[32];
            for (int vertex = 0; vertex < 4; vertex++) {
                int base = vertex * 8;
                packed[base] = Float.floatToRawIntBits(vertex == 1 || vertex == 2 ? 1 : 0);
                packed[base + 1] = Float.floatToRawIntBits(vertex >= 2 ? 1 : 0);
                packed[base + 3] = 0xffffffff;
                packed[base + 4] = Float.floatToRawIntBits(sprite.getU(vertex == 1 || vertex == 2 ? 0.9F : 0.1F));
                packed[base + 5] = Float.floatToRawIntBits(sprite.getV(vertex >= 2 ? 0.9F : 0.1F));
                packed[base + 7] = vertex % 2 == 0 ? 0 : 0x00007f00;
            }
            var quad = new BakedQuad(packed, -1, Direction.SOUTH, sprite, true, 0);
            Class<?> semanticsType = Class.forName(RustGalWorldPrimitiveRenderer.class.getName() + "$ModelMeshRenderSemantics");
            var constructor = semanticsType.getDeclaredConstructor(int.class, int.class, int.class, int.class);
            constructor.setAccessible(true);
            var semantics = constructor.newInstance(RustGalWorldPrimitiveRenderer.MATERIAL_ID_CUTOUT_TEXTURED,
                RustGalWorldPrimitiveRenderer.MATERIAL_MODE_CUTOUT,
                RustGalWorldPrimitiveRenderer.DEPTH_POLICY_TEST_WRITE, RustGalWorldPrimitiveRenderer.CULL_BACK);
            Method extract = RustGalWorldPrimitiveRenderer.class.getDeclaredMethod("extractItemQuadMesh",
                List.class, int[].class, int.class, semanticsType, String.class, boolean.class, Matrix4f.class);
            extract.setAccessible(true);
            System.setProperty(property, "true");
            // No Minecraft singleton, Java GPU device, resource manager, or selected
            // animation frame is available. Extraction must only emit atlas semantics.
            Object result = extract.invoke(null, List.of(quad), new int[0], 0x00f000f0, semantics,
                "minecraft:audit/item", false, null);
            var assetAccessor = result.getClass().getDeclaredMethod("asset");
            assetAccessor.setAccessible(true);
            var asset = (VulkanicGalBridge.WorldMeshAssetRecord) assetAccessor.invoke(result);
            var textureAccessor = result.getClass().getDeclaredMethod("textures");
            textureAccessor.setAccessible(true);
            assertTrue(((List<?>) textureAccessor.invoke(result)).isEmpty(), "do not replace or duplicate the owned atlas");
            assertEquals(1, asset.sections().size());
            var section = asset.sections().getFirst();
            assertEquals(RustGalWorldPrimitiveRenderer.MATERIAL_TEXTURE_TERRAIN_BLOCK_ATLAS, section.textureId());
            assertEquals(RustGalWorldPrimitiveRenderer.CULL_BACK, section.cullPolicy());
            assertEquals(RustGalWorldPrimitiveRenderer.MATERIAL_MODE_CUTOUT, section.materialMode());
            assertArrayEquals(new byte[] {0,0,1,0,2,0,2,0,3,0,0,0}, asset.indexBytes());
            for (int index = 0; index < 4; index++) {
                var vertex = asset.vertices().get(index);
                assertEquals(quad.getTexU(index), vertex.u());
                assertEquals(quad.getTexV(index), vertex.v());
                assertEquals(vertex.u(), vertex.atlasU());
                assertEquals(vertex.v(), vertex.atlasV());
                assertEquals(quad.getX(index), vertex.x());
                assertEquals(quad.getY(index), vertex.y());
                assertEquals(quad.getAccurateNormal(index), vertex.normalPacked(),
                    "vanilla baked quads may omit vertex normals; Frozen resolves their geometric face normal");
                assertEquals(0x00f000f0, vertex.light());
                assertEquals(0xffffffff, vertex.colorArgb());
            }
            resource.enqueueTick(1, true);
            assertEquals(1, resource.producedTickForDiagnostics());
            assertEquals(1, resource.producedTickForDiagnostics(), "observation must not advance the clock");
            assertTrue(resource.drain(17, (texture, generation, tick, visible, onlyVisible) -> {
                assertEquals(RustGalWorldPrimitiveRenderer.MATERIAL_TEXTURE_TERRAIN_BLOCK_ATLAS, texture);
                assertEquals(17, generation);
                assertArrayEquals(new int[] {11}, visible, "the item consumer reports its own semantic sprite use");
                assertTrue(onlyVisible);
                return true;
            }));
            System.setProperty(property, "false");
            Object separate = extract.invoke(null, List.of(quad), new int[0], 0x00f000f0, semantics,
                "minecraft:audit/item", false, null);
            var separateAsset = (VulkanicGalBridge.WorldMeshAssetRecord) assetAccessor.invoke(separate);
            assertEquals(1, ((List<?>) textureAccessor.invoke(separate)).size());
            assertNotEquals(section.textureId(), separateAsset.sections().getFirst().textureId());
            assertEquals(0.1F, separateAsset.vertices().getFirst().u(), 0.000001F);
            System.setProperty(property, "true");
            Object foil = extract.invoke(null, List.of(quad), new int[0], 0x00f000f0, semantics,
                "minecraft:audit/item", false, new Matrix4f());
            assertEquals(1, ((List<?>) textureAccessor.invoke(foil)).size(), "special foil stays a separate texture projection");
            assertNotEquals(section.textureId(), ((VulkanicGalBridge.WorldMeshAssetRecord)
                assetAccessor.invoke(foil)).sections().getFirst().textureId());
            resource.enqueueTick(2, true);
            assertTrue(resource.drain(17, (texture, generation, tick, visible, onlyVisible) -> {
                assertEquals(0, visible.length, "separate textures and foil must not activate atlas animation");
                return true;
            }));
            var device = net.vulkanic.VulkanicAPI.class.getDeclaredField("device");
            device.setAccessible(true);
            Object previousDevice = device.get(null);
            String mode = "mattmc.dev.rustGalVulkanWholeFrame";
            String previousMode = System.getProperty(mode);
            try {
                System.setProperty(mode, "true");
                device.set(null, new net.vulkanic.backends.vulkan.VulkanWholeFrameSemanticGpuDevice());
                // The resource already owns immutable source pixels. Neither
                // eligibility nor extraction may read this retired Java image.
                contents.originalImage.close();
                var firstPersonState = new net.minecraft.client.renderer.item.ItemStackRenderState();
                var context = firstPersonState.getClass().getDeclaredField("displayContext");
                context.setAccessible(true);
                context.set(firstPersonState, net.minecraft.world.item.ItemDisplayContext.FIRST_PERSON_RIGHT_HAND);
                var layer = firstPersonState.newLayer();
                layer.prepareQuadList().add(quad);
                layer.setRenderType(net.minecraft.client.renderer.RenderType.itemEntityTranslucentCull(TextureAtlas.LOCATION_BLOCKS));
                var firstPersonEligibility = RustGalWorldPrimitiveRenderer.class.getDeclaredMethod(
                    "firstPersonItemMeshIneligibility", firstPersonState.getClass(), int.class, String.class);
                firstPersonEligibility.setAccessible(true);
                assertNull(firstPersonEligibility.invoke(null, firstPersonState, 0x00f000f0, "minecraft:magma_block"),
                    "first-person eligibility must use owned source data, not the Java animation frame");
                var blockVertices = new java.util.ArrayList<VulkanicGalBridge.WorldMeshVertexRecord>();
                var blockIndices = new java.util.ArrayList<Integer>();
                var blockSections = new java.util.ArrayList<VulkanicGalBridge.WorldMeshSectionRecord>();
                var blockTextures = new java.util.ArrayList<VulkanicGalBridge.WorldMeshTextureAssetRecord>();
                var appendBlock = RustGalWorldPrimitiveRenderer.class.getDeclaredMethod("appendBlockModelQuads",
                    List.class, net.minecraft.world.level.block.state.BlockState.class,
                    net.minecraft.world.level.BlockAndTintGetter.class, net.minecraft.core.BlockPos.class,
                    int.class, int.class, int.class, int.class, int.class, boolean.class,
                    List.class, List.class, List.class, List.class);
                appendBlock.setAccessible(true);
                var worldShade = (net.minecraft.world.level.BlockAndTintGetter) java.lang.reflect.Proxy.newProxyInstance(
                    getClass().getClassLoader(), new Class<?>[] {net.minecraft.world.level.BlockAndTintGetter.class},
                    (proxy, method, args) -> {
                        if (method.getName().equals("getShade")) return 0.5F;
                        throw new AssertionError("unexpected world lookup " + method.getName());
                    });
                appendBlock.invoke(null, List.of(quad), null, worldShade, net.minecraft.core.BlockPos.ZERO,
                    RustGalWorldPrimitiveRenderer.MATERIAL_ID_CUTOUT_TEXTURED,
                    RustGalWorldPrimitiveRenderer.MATERIAL_MODE_CUTOUT, 17, 1, 0x00f000f0, false,
                    blockVertices, blockIndices, blockSections, blockTextures);
                assertTrue(blockTextures.isEmpty(), "baked blocks must not publish duplicate sprite images");
                assertEquals(section.textureId(), blockSections.getFirst().textureId());
                assertEquals(List.of(0, 1, 2, 2, 3, 0), blockIndices);
                for (int i = 0; i < 4; i++) {
                    var v = blockVertices.get(i);
                    assertEquals(quad.getTexU(i), v.u());
                    assertEquals(quad.getTexV(i), v.v());
                    assertEquals(quad.getX(i), v.x());
                    assertEquals(quad.getY(i), v.y());
                    assertEquals(quad.getVertexNormal(i), v.normalPacked(), "block lighting semantics remain unchanged");
                    assertEquals(0x00f000f0, v.light());
                    assertEquals(0xffffffff, v.colorArgb(), "single-block models do not bake world face shading");
                }
                blockVertices.clear(); blockIndices.clear(); blockSections.clear();
                appendBlock.invoke(null, List.of(quad), null, worldShade, net.minecraft.core.BlockPos.ZERO,
                    RustGalWorldPrimitiveRenderer.MATERIAL_ID_CUTOUT_TEXTURED,
                    RustGalWorldPrimitiveRenderer.MATERIAL_MODE_CUTOUT, 17, 1, 0x00f000f0, true,
                    blockVertices, blockIndices, blockSections, blockTextures);
                assertEquals(0xff808080, blockVertices.getFirst().colorArgb(), "moving-world face shading is preserved");
                resource.enqueueTick(3, true);
                assertTrue(resource.drain(17, (texture, generation, tick, visible, onlyVisible) -> {
                    assertArrayEquals(new int[] {11}, visible, "baked block consumers report semantic sprite use");
                    return true;
                }));
                assertTrue(RustGalWorldPrimitiveRenderer.isItemEntityMeshEligible(
                    net.minecraft.world.item.ItemDisplayContext.GROUND, 0x00f000f0, 0, 0, new int[0], List.of(quad),
                    net.minecraft.client.renderer.RenderType.itemEntityTranslucentCull(TextureAtlas.LOCATION_BLOCKS),
                    net.minecraft.client.renderer.item.ItemStackRenderState.FoilType.NONE));
                var cube = new net.minecraft.client.model.geom.ModelPart.Cube(0, 0,
                    0, 0, 0, 2, 2, 2, 0, 0, 0, false, 16, 16, java.util.EnumSet.allOf(Direction.class));
                var model = new net.minecraft.client.model.geom.ModelPart(List.of(cube), java.util.Map.of());
                model.x = 16;
                var modelExtract = RustGalWorldPrimitiveRenderer.class.getDeclaredMethod("extractModelPartMesh",
                    model.getClass(), ResourceLocation.class, TextureAtlasSprite.class, String.class,
                    int.class, int.class, int.class, int.class);
                modelExtract.setAccessible(true);
                Object modelResult = modelExtract.invoke(null, model, TextureAtlas.LOCATION_BLOCKS, sprite,
                    "minecraft:audit/modelpart", 0x00f000f0,
                    RustGalWorldPrimitiveRenderer.MATERIAL_ID_CUTOUT_TEXTURED,
                    RustGalWorldPrimitiveRenderer.MATERIAL_MODE_CUTOUT, RustGalWorldPrimitiveRenderer.CULL_BACK);
                var modelAsset = (VulkanicGalBridge.WorldMeshAssetRecord) assetAccessor.invoke(modelResult);
                assertTrue(((List<?>)textureAccessor.invoke(modelResult)).isEmpty(),
                    "ModelPart must not copy or republish an owned atlas");
                assertEquals(24, modelAsset.vertices().size());
                assertEquals(6, modelAsset.sections().size());
                for (var modelSection : modelAsset.sections()) assertEquals(section.textureId(), modelSection.textureId());
                int modelVertex = 0;
                for (var polygon : cube.polygons) for (var sourceVertex : polygon.vertices()) {
                    var emitted = modelAsset.vertices().get(modelVertex++);
                    assertEquals(sourceVertex.worldX() + 1, emitted.x(), 0.000001F);
                    assertEquals(sourceVertex.worldY(), emitted.y(), 0.000001F);
                    assertEquals(sourceVertex.worldZ(), emitted.z(), 0.000001F);
                    assertEquals(sprite.getU(sourceVertex.u()), emitted.u(), 0.000001F);
                    assertEquals(sprite.getV(sourceVertex.v()), emitted.v(), 0.000001F);
                }
                resource.enqueueTick(4, true);
                assertTrue(resource.drain(17, (texture, generation, tick, visible, onlyVisible) -> {
                    assertArrayEquals(new int[]{11}, visible, "ModelPart reports its semantic sprite use");
                    return true;
                }));
                verifyTerrainParticleUse(resource, sprite);
                resource.close();
                var retiredModel = assertThrows(java.lang.reflect.InvocationTargetException.class,
                    () -> modelExtract.invoke(null, model, TextureAtlas.LOCATION_BLOCKS, sprite,
                        "minecraft:audit/modelpart", 0x00f000f0,
                        RustGalWorldPrimitiveRenderer.MATERIAL_ID_CUTOUT_TEXTURED,
                        RustGalWorldPrimitiveRenderer.MATERIAL_MODE_CUTOUT, RustGalWorldPrimitiveRenderer.CULL_BACK));
                assertInstanceOf(IllegalStateException.class, retiredModel.getCause());
                assertEquals("retired-owned-atlas", firstPersonEligibility.invoke(
                    null, firstPersonState, 0x00f000f0, "minecraft:magma_block"));
                assertFalse(RustGalWorldPrimitiveRenderer.isItemEntityMeshEligible(
                    net.minecraft.world.item.ItemDisplayContext.GROUND, 0x00f000f0, 0, 0, new int[0], List.of(quad),
                    net.minecraft.client.renderer.RenderType.itemEntityTranslucentCull(TextureAtlas.LOCATION_BLOCKS),
                    net.minecraft.client.renderer.item.ItemStackRenderState.FoilType.NONE),
                    "a retired resource cannot admit an old animated sprite");
            } finally {
                device.set(null, previousDevice);
                if (previousMode == null) System.clearProperty(mode); else System.setProperty(mode, previousMode);
            }
        } finally {
            if (previous == null) System.clearProperty(property); else System.setProperty(property, previous);
        }
    }

    @SuppressWarnings("unchecked")
    private static void verifyTerrainParticleUse(AtlasAnimationResource resource, TextureAtlasSprite sprite) throws Exception {
        var renderer = RustGalWorldPrimitiveRenderer.class;
        var width = renderer.getDeclaredField("pendingViewportWidth");
        var height = renderer.getDeclaredField("pendingViewportHeight");
        var textures = renderer.getDeclaredField("WORLD_MESH_TEXTURES");
        var quads = renderer.getDeclaredField("PENDING_MATERIAL_QUADS");
        var diagnostics = renderer.getDeclaredField("TERRAIN_PARTICLE_DIAGNOSTICS");
        var diagnosticCount = renderer.getDeclaredField("terrainParticleEnqueueDiagnosticLogs");
        for (var field : List.of(width, height, textures, quads, diagnostics, diagnosticCount)) field.setAccessible(true);
        var diagnosticList = (List<Object>)diagnostics.get(null);
        var oldDiagnostics = List.copyOf(diagnosticList);
        int oldDiagnosticCount = diagnosticCount.getInt(null);
        int oldWidth = width.getInt(null), oldHeight = height.getInt(null);
        var assets = (java.util.Map<Integer, VulkanicGalBridge.WorldMeshTextureAssetRecord>)textures.get(null);
        int atlas = RustGalWorldPrimitiveRenderer.MATERIAL_TEXTURE_TERRAIN_BLOCK_ATLAS;
        boolean hadAtlas = assets.containsKey(atlas);
        var oldAtlas = assets.get(atlas);
        int checkpoint = RustGalWorldPrimitiveRenderer.markMaterialQuadBatch();
        var camera = new net.minecraft.client.Camera();
        try {
            width.setInt(null, 640);
            height.setInt(null, 480);
            // Atlas registration is a precondition; this CPU-only test must not
            // create a Java GPU object or decode the already-closed source image.
            assets.put(atlas, null);
            assertThrows(IllegalArgumentException.class, () -> enqueueParticle(resource, sprite, camera, Float.NaN));
            assertEquals(checkpoint, RustGalWorldPrimitiveRenderer.markMaterialQuadBatch());
            resource.enqueueTick(5, true);
            assertTrue(resource.drain(17, (texture, generation, tick, visible, onlyVisible) -> {
                assertArrayEquals(new int[0], visible, "rejected particles must not report sprite use");
                return true;
            }));
            assertTrue(enqueueParticle(resource, sprite, camera, 0.25F));
            assertEquals(checkpoint + 1, RustGalWorldPrimitiveRenderer.markMaterialQuadBatch());
            var emitted = ((List<VulkanicGalBridge.WorldMaterialQuadRecord>)quads.get(null)).getLast();
            assertEquals(atlas, emitted.textureId());
            assertEquals(sprite.getU(0.25F), emitted.uv0U());
            assertEquals(sprite.getV(0.9F), emitted.uv0V());
            assertNotEquals(0.75F, emitted.uv0U(), "atlas coordinates must not collapse to sprite-local UVs");
            assertEquals(sprite.getU(0.75F), emitted.uv2U(), "reversed U must retain vanilla's corner orientation");
            assertEquals(0x00400020, emitted.packedLight());
            assertEquals(0xff804020, emitted.colorArgb(), "Java must preserve raw color for Rust lightmap evaluation");
            assertEquals(0xff804020, emitted.sourceColorArgb());
            assertEquals(0xff804020, emitted.vertex0ColorArgb());
            assertEquals(0xff804020, emitted.vertex1ColorArgb());
            assertEquals(0xff804020, emitted.vertex2ColorArgb());
            assertEquals(0xff804020, emitted.vertex3ColorArgb());
            resource.enqueueTick(6, true);
            assertTrue(resource.drain(17, (texture, generation, tick, visible, onlyVisible) -> {
                assertArrayEquals(new int[]{11}, visible, "accepted particle alone activates its owned sprite");
                return true;
            }));
            resource.close();
            assertThrows(IllegalStateException.class, () -> enqueueParticle(resource, sprite, camera, 0.25F));
            assertEquals(checkpoint + 1, RustGalWorldPrimitiveRenderer.markMaterialQuadBatch(),
                "retired-resource rejection must precede publishing a quad");
        } finally {
            RustGalWorldPrimitiveRenderer.rollbackMaterialQuadBatch(checkpoint);
            if (hadAtlas) assets.put(atlas, oldAtlas); else assets.remove(atlas);
            width.setInt(null, oldWidth);
            height.setInt(null, oldHeight);
            diagnosticList.clear();
            diagnosticList.addAll(oldDiagnostics);
            diagnosticCount.setInt(null, oldDiagnosticCount);
        }
    }

    private static boolean enqueueParticle(AtlasAnimationResource resource, TextureAtlasSprite sprite,
            net.minecraft.client.Camera camera, float size) {
        return RustGalWorldPrimitiveRenderer.enqueueTerrainParticle(
            net.minecraft.world.level.block.Blocks.MAGMA_BLOCK.defaultBlockState(), sprite.contents().name(), resource, camera,
            0, 0, 0, 0, -2, -2, new org.joml.Quaternionf(), 1, size,
            sprite.getU(0.75F), sprite.getU(0.25F), sprite.getV(0.1F), sprite.getV(0.9F), 0xff804020, 0x00400020, true);
    }
}
