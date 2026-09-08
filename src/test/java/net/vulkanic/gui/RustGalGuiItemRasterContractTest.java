package net.vulkanic.gui;

import org.junit.jupiter.api.Test;
import static org.junit.jupiter.api.Assertions.*;

class RustGalGuiItemRasterContractTest {
    @Test void redstoneFixtureUsesUnblendedCutoutLayersNotTranslucentItemLayers() {
        net.minecraft.SharedConstants.tryDetectVersion();
        net.minecraft.server.Bootstrap.bootStrap();
        String key=net.vulkanic.bridge.RustGalVulkanWholeFrameMode.propertyName();
        String previous=System.getProperty(key);
        try {
            System.setProperty(key,"true");
            var type=net.minecraft.client.renderer.ItemBlockRenderTypes.getRenderType(
                new net.minecraft.world.item.ItemStack(net.minecraft.world.item.Items.REDSTONE));
            assertEquals(2,RustGalGuiItemRenderer.flatItemMaterial(type.pipeline()));
            assertTrue(type.pipeline().getBlendFunction().isEmpty());
        } finally {
            if(previous == null) System.clearProperty(key); else System.setProperty(key,previous);
        }
    }
    @Test void atlasVisibilityIsReportedOnlyAfterBoundedItemAdmission() throws Exception {
        String source = java.nio.file.Files.readString(java.nio.file.Path.of(
            "src/main/java/net/vulkanic/gui/RustGalGuiItemRenderer.java"));
        int admission = source.indexOf("private-item-raster-mixed-source-layers");
        int use = source.indexOf("RustGalWorldPrimitiveRenderer.recordAtlasSpriteUse(",admission);
        int enqueue = source.indexOf("List<RustGalGuiElementRenderState> elements",admission);
        assertTrue(admission >= 0 && use > admission && enqueue > use);
        String extraction = source.substring(use,enqueue);
        assertTrue(extraction.contains("sprite.semanticAnimationResource()"));
        assertFalse(extraction.contains("getTexture"));
        assertFalse(extraction.contains("tickAndUpload"));
    }
    @Test void authoredItemUvConversionNeverClampsInvalidRegionsIntoAdmission() {
        assertEquals(0.25F,RustGalGuiItemRenderer.itemLocalUv(0.5625F,0.5F,0.75F));
        assertEquals(-0.5F,RustGalGuiItemRenderer.itemLocalUv(0.375F,0.5F,0.75F));
        assertEquals(2.0F,RustGalGuiItemRenderer.itemLocalUv(1.0F,0.5F,0.75F));
        assertFalse(Float.isFinite(RustGalGuiItemRenderer.itemLocalUv(0.5F,0.5F,0.5F)));
    }
    @Test void renderTypeNamesCannotAdmitAnUnsupportedMaterial() {
        var texture = net.minecraft.resources.ResourceLocation.withDefaultNamespace("item/solid_cutout.png");
        assertTrue(RustGalGuiItemRenderer.supportedGuiRenderType(
            net.minecraft.client.renderer.RenderType.itemEntityTranslucentCull(texture)));
        assertTrue(RustGalGuiItemRenderer.supportedGuiRenderType(
            net.minecraft.client.renderer.RenderType.entityCutout(texture)));
        assertFalse(RustGalGuiItemRenderer.supportedGuiRenderType(
            net.minecraft.client.renderer.RenderType.entitySolid(texture)));
        assertFalse(RustGalGuiItemRenderer.supportedGuiRenderType(
            net.minecraft.client.renderer.RenderType.entityCutoutNoCull(texture)));
        assertFalse(RustGalGuiItemRenderer.supportedGuiRenderType(
            net.minecraft.client.renderer.RenderType.entityTranslucent(texture)));
        assertFalse(RustGalGuiItemRenderer.supportedGuiRenderType(null));
    }
    @Test void onlyKnownAuthoredItemMaterialsAreExtracted() {
        assertEquals(1, RustGalGuiItemRenderer.flatItemMaterial(
            net.minecraft.client.renderer.RenderPipelines.ITEM_ENTITY_TRANSLUCENT_CULL));
        assertEquals(2, RustGalGuiItemRenderer.flatItemMaterial(
            net.minecraft.client.renderer.RenderPipelines.ENTITY_CUTOUT));
        assertEquals(0, RustGalGuiItemRenderer.flatItemMaterial(
            net.minecraft.client.renderer.RenderPipelines.ENTITY_SOLID));
        assertEquals(0, RustGalGuiItemRenderer.flatItemMaterial(null));
    }
    @Test void everyOtherAuthoredPipelineRemainsUnadmitted() throws Exception {
        int inspected = 0;
        for (var field : net.minecraft.client.renderer.RenderPipelines.class.getFields()) {
            if (field.getType() != net.blaze3d.pipeline.RenderPipeline.class) continue;
            var pipeline = (net.blaze3d.pipeline.RenderPipeline) field.get(null);
            if (pipeline == net.minecraft.client.renderer.RenderPipelines.ITEM_ENTITY_TRANSLUCENT_CULL
                || pipeline == net.minecraft.client.renderer.RenderPipelines.ENTITY_CUTOUT) continue;
            assertEquals(0, RustGalGuiItemRenderer.flatItemMaterial(pipeline), field.getName());
            inspected++;
        }
        assertTrue(inspected > 20, "exercise the real authored pipeline catalog");
    }
    @Test void affineFacesRequireTheActualFourthCorner() {
        assertTrue(RustGalGuiItemRenderer.affineFourthCorner(0.5F,0,1,0.5F,0.5F,1,0,0.5F));
        assertTrue(RustGalGuiItemRenderer.affineFourthCorner(0.5F,0,1,0.5F,Math.nextUp(0.5F),1,0,0.5F));
        assertFalse(RustGalGuiItemRenderer.affineFourthCorner(0.5F,0,1,0.5F,0.6F,1,0,0.5F));
        assertFalse(RustGalGuiItemRenderer.affineFourthCorner(0,0,1,0,Float.NaN,1,0,1));
        assertFalse(RustGalGuiItemRenderer.affineFourthCorner(0,0,1,0,1,1,Float.POSITIVE_INFINITY,1));
        assertArrayEquals(new float[]{8,0,16,8,0,8},
            RustGalGuiItemRenderer.boundedItemRasterGeometry(0.5F,1,1,0.5F,0,0.5F).corners());
    }

    @Test void authoredGeometrySurvivesTheCanonicalSixteenUnitRasterBoundary() {
        assertEquals(net.vulkanic.bridge.VulkanicGalBridge.GuiItemRasterGeometryRecord.FULL,
            RustGalGuiItemRenderer.boundedItemRasterGeometry(0,1,1,1,0,0));
        assertArrayEquals(new float[] {4,2,12,2,4,14},
            RustGalGuiItemRenderer.boundedItemRasterGeometry(0.25F,0.875F,0.75F,0.875F,0.25F,0.125F).corners());
        assertNull(RustGalGuiItemRenderer.boundedItemRasterGeometry(0,1,2,1,0,0));
        assertArrayEquals(new float[] {16,0,0,0,16,16},
            RustGalGuiItemRenderer.boundedItemRasterGeometry(1,1,0,1,1,0).corners());
        assertNull(RustGalGuiItemRenderer.boundedItemRasterGeometry(0,1,0,1,0,0));
        assertNull(RustGalGuiItemRenderer.boundedItemRasterGeometry(Float.NaN,1,1,1,0,0));
    }
}
