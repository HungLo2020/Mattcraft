package net.vulkanic.world;

import java.io.ByteArrayInputStream;
import java.io.IOException;
import java.lang.reflect.Proxy;
import java.nio.charset.StandardCharsets;
import java.util.Base64;
import java.util.concurrent.atomic.AtomicBoolean;
import net.minecraft.server.packs.PackResources;
import net.minecraft.server.packs.resources.Resource;
import net.minecraft.server.packs.resources.ResourceMetadata;
import org.junit.jupiter.api.Test;
import static org.junit.jupiter.api.Assertions.*;

class StandardItemFoilResourceTest {
    @org.junit.jupiter.api.BeforeAll static void bootstrap() {
        net.minecraft.SharedConstants.tryDetectVersion();
        net.minecraft.server.Bootstrap.bootStrap();
    }

    @Test void groundStandardFoilUsesOwnedNativeInstancePayloadWithoutPrivateGate() throws Exception {
        var source = java.nio.file.Files.readString(java.nio.file.Path.of(
            "src/main/java/net/vulkanic/world/RustGalWorldPrimitiveRenderer.java"));
        int start=source.indexOf("BlockMeshExtraction glintExtraction = null;");
        int end=source.indexOf("PENDING_MESH_PRODUCERS.add",start);
        var ground=source.substring(start,end);
        assertTrue(ground.contains("foilType == ItemStackRenderState.FoilType.STANDARD"));
        assertFalse(source.contains("mattmc.dev.rustGalWorldItemFoil"));
        assertTrue(ground.contains("standardFoil = new VulkanicGalBridge.StandardItemFoilRecord("));
        assertTrue(ground.contains("\"minecraft:item_entity/ground-glint\", true)"));
        assertTrue(ground.contains("glintInstance.withItemFoil(standardFoil)"));
        assertFalse(ground.contains("standardItemFoilUv"));
        int first = source.indexOf("private static BlockMeshExtraction extractItemQuadMesh(");
        int body = source.indexOf("private static BlockMeshExtraction extractItemQuadMesh(", first + 1);
        var extraction = source.substring(body, source.indexOf("\n\tprivate static", body + 1));
        assertFalse(extraction.contains("ticks % 110000L"));
        assertFalse(extraction.contains("specialFoilInversePose"));
        assertTrue(extraction.contains("float sourceU = quad.getTexU(vertexIndex)"));
    }

    @Test void unsupportedProjectedFoilRejectsBeforeTextureOrGpuAccess() {
        assertEquals("special-foil-native-projection-unavailable",
            RustGalWorldPrimitiveRenderer.itemEntityMeshIneligibility(
                net.minecraft.world.item.ItemDisplayContext.GROUND, 15728640, 0, 0,
                new int[0], null, null,
                net.minecraft.client.renderer.item.ItemStackRenderState.FoilType.SPECIAL));
    }

    @Test void modelFoilCopiesResourcesAndOriginalUvsWithoutJavaAnimation() throws Exception {
        var source = java.nio.file.Files.readString(java.nio.file.Path.of(
            "src/main/java/net/vulkanic/world/RustGalWorldPrimitiveRenderer.java"));
        assertFalse(source.contains("ticks % 110000L"));
        int first = source.indexOf("private static BlockMeshExtraction extractModelPartMesh(");
        int start = source.indexOf("private static BlockMeshExtraction extractModelPartMesh(", first + 1);
        var extraction = source.substring(start, source.indexOf("\n\tprivate static", start + 1));
        assertTrue(extraction.contains("copyStandardItemFoilTexture(stableTextureId(effectiveTexture), resource)"));
        assertTrue(extraction.contains("glint ? semanticFoilTexture : localModelTextureAsset"));
        assertFalse(extraction.contains("glintStrength()"));
        assertFalse(extraction.contains("Math.sin"));
        assertTrue(source.contains("VulkanicGalBridge.StandardFoilKind.ENTITY"));
        assertTrue(source.contains("withItemFoil(modelFoil)"));
        assertTrue(source.contains("model-foil-parity-unadmitted"));
    }

    @Test void firstPersonProjectedFoilAlsoRejectsBeforeResourceAccess() throws Exception {
        var state = new net.minecraft.client.renderer.item.ItemStackRenderState();
        var context = state.getClass().getDeclaredField("displayContext");
        context.setAccessible(true);
        context.set(state, net.minecraft.world.item.ItemDisplayContext.FIRST_PERSON_RIGHT_HAND);
        state.newLayer().setFoilType(net.minecraft.client.renderer.item.ItemStackRenderState.FoilType.SPECIAL);
        var check = RustGalWorldPrimitiveRenderer.class.getDeclaredMethod("firstPersonItemMeshIneligibility",
            state.getClass(), int.class, String.class);
        check.setAccessible(true);
        assertEquals("special-foil-native-projection-unavailable", check.invoke(null, state, 15728640, "minecraft:compass"));
    }

    @Test void defaultStandaloneFoilOnlyCopiesTheVerifiedTridentContract() throws Exception {
        var model = new net.minecraft.client.model.TridentModel(
            net.minecraft.client.model.TridentModel.createLayer().bakeRoot());
        var texture = net.minecraft.client.model.TridentModel.TEXTURE;
        var state = net.minecraft.util.Unit.INSTANCE;
        var material = net.minecraft.client.renderer.RenderType.entitySolid(texture);
        assertNull(RustGalWorldPrimitiveRenderer.standaloneModelFoilIneligibility(model,state,material,texture));
        assertEquals("armor-foil-native-contract-unavailable",
            RustGalWorldPrimitiveRenderer.standaloneModelFoilIneligibility(null,null,
                net.minecraft.client.renderer.RenderType.armorEntityGlint(),null));
        assertEquals("standalone-model-foil-family-unadmitted",
            RustGalWorldPrimitiveRenderer.standaloneModelFoilIneligibility(null,state,material,texture));
        assertEquals("standalone-model-foil-family-unadmitted",
            RustGalWorldPrimitiveRenderer.standaloneModelFoilIneligibility(model,null,material,texture));
        assertEquals("standalone-model-foil-family-unadmitted",
            RustGalWorldPrimitiveRenderer.standaloneModelFoilIneligibility(model,state,material,null));
        assertEquals("standalone-model-foil-material-unadmitted",
            RustGalWorldPrimitiveRenderer.standaloneModelFoilIneligibility(model,state,null,texture));
        var source = java.nio.file.Files.readString(java.nio.file.Path.of(
            "src/main/java/net/vulkanic/world/RustGalWorldPrimitiveRenderer.java"));
        int start=source.indexOf("boolean enqueueStandaloneGlintModelMesh(");
        var body=source.substring(start,source.indexOf("static String standaloneModelFoilIneligibility(",start));
        assertFalse(body.contains("mattmc.dev.rustGalModelItemFoil"));
        assertTrue(body.indexOf("standaloneModelFoilIneligibility(")<body.indexOf("readTexturePayloadForResource("));
        assertTrue(body.contains("withItemFoil(foil)"));
    }

    @Test void sharedResourceContractDoesNotDependOnNativeGeometryAdmission() throws Exception {
        var source = java.nio.file.Files.readString(java.nio.file.Path.of(
            "src/main/java/net/vulkanic/world/RustGalWorldPrimitiveRenderer.java"));
        int start = source.indexOf("VulkanicGalBridge.WorldMeshTextureAssetRecord semanticFoilTexture = null;");
        int end = source.indexOf("int tintColor = itemQuadTintColor", start);
        assertTrue(start >= 0 && end > start);
        var extraction = source.substring(start, end);
        assertTrue(extraction.contains("if (glint) {"));
        assertTrue(extraction.contains("semanticFoilTexture = copyStandardItemFoilTexture("));
        assertTrue(extraction.contains("textures.add(semanticFoilTexture)"));
        assertFalse(extraction.contains("if (nativeStandardFoil)"),
            "shared resource identity must not alternate explicit and unspecified sampling between producers");
        var resource = new Resource(pack(), () -> new ByteArrayInputStream(PNG));
        var held = RustGalWorldPrimitiveRenderer.copyStandardItemFoilTexture(123, resource);
        var ground = RustGalWorldPrimitiveRenderer.copyStandardItemFoilTexture(123, resource);
        assertTrue(held.sameContent(ground));
        assertFalse(held.sameContent(new net.vulkanic.bridge.VulkanicGalBridge.WorldMeshTextureAssetRecord(123, PNG)),
            "the old legacy descriptor loses sampling/mip semantics even with identical pixels");
    }

    private static final byte[] PNG = Base64.getDecoder().decode("iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVQIHWP4z8DwHwAFgAI/ScLttAAAAABJRU5ErkJggg==");
    private static PackResources pack() {
        return (PackResources) Proxy.newProxyInstance(PackResources.class.getClassLoader(),
            new Class<?>[]{PackResources.class}, (proxy, method, args) -> {
                if (method.getName().equals("packId")) return "selected-pack";
                throw new UnsupportedOperationException(method.getName());
            });
    }

    @Test void copiesSelectedBytesAndEverySamplingCombinationWithoutGpuAccess() throws Exception {
        for (boolean blur : new boolean[]{false,true}) for (boolean clamp : new boolean[]{false,true}) {
            var closed = new AtomicBoolean();
            var resource = new Resource(pack(), () -> new ByteArrayInputStream(PNG) {
                @Override public void close() { closed.set(true); }
            }, () -> ResourceMetadata.fromJsonStream(new ByteArrayInputStream(
                ("{\"texture\":{\"blur\":"+blur+",\"clamp\":"+clamp+"}}").getBytes(StandardCharsets.UTF_8))));
            var asset = RustGalWorldPrimitiveRenderer.copyStandardItemFoilTexture(123,resource);
            assertTrue(closed.get());
            assertArrayEquals(PNG,asset.pngBytes());
            assertEquals(123,asset.textureId());
            assertEquals(blur ? 2 : 1,asset.samplingFilter());
            assertEquals(clamp ? 2 : 1,asset.samplingAddress());
            assertEquals(1,asset.requestedMipLevels());
            assertEquals(0,asset.coordinateOrigin());
        }
    }

    @Test void absentMetadataUsesExplicitResourceDefaults() throws Exception {
        var asset = RustGalWorldPrimitiveRenderer.copyStandardItemFoilTexture(123,
            new Resource(pack(), () -> new ByteArrayInputStream(PNG)));
        assertEquals(1,asset.samplingFilter());
        assertEquals(1,asset.samplingAddress());
        assertEquals(1,asset.requestedMipLevels());
    }

    @Test void brokenSelectedMetadataFailsClosedAndClosesPixels() {
        var closed = new AtomicBoolean();
        var resource = new Resource(pack(), () -> new ByteArrayInputStream(PNG) {
            @Override public void close() { closed.set(true); }
        }, () -> { throw new IOException("selected metadata unreadable"); });
        assertThrows(IOException.class, () -> RustGalWorldPrimitiveRenderer.copyStandardItemFoilTexture(123,resource));
        assertTrue(closed.get());
    }
}
