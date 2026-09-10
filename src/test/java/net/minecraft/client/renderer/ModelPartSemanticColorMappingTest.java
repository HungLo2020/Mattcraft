package net.minecraft.client.renderer;

import net.blaze3d.vertex.PoseStack;
import net.minecraft.client.model.geom.ModelPart;
import net.minecraft.client.renderer.texture.TextureAtlasSprite;
import net.vulkanic.world.RustGalWorldPrimitiveRenderer;
import net.vulkanic.world.WorldRenderRoutePolicy;
import org.junit.jupiter.api.Test;
import static org.junit.jupiter.api.Assertions.*;
import static org.mockito.Mockito.*;

class ModelPartSemanticColorMappingTest {
    @Test void atlasModelFoilSubmitsOnlyTheFoilCommandWithoutJavaQueueing() {
        var model = new net.minecraft.client.model.ShieldModel(net.minecraft.client.model.ShieldModel.createLayer().bakeRoot());
        var sprite = mock(TextureAtlasSprite.class);
        var pose = new PoseStack();
        var type = RenderType.entityGlint();
        var state = net.minecraft.util.Unit.INSTANCE;
        int overlay = net.minecraft.client.renderer.texture.OverlayTexture.NO_OVERLAY;
        try (var nativeCalls=mockStatic(RustGalWorldPrimitiveRenderer.class);
             var policy=mockStatic(WorldRenderRoutePolicy.class);
             var api=mockStatic(net.vulkanic.VulkanicAPI.class)) {
            api.when(net.vulkanic.VulkanicAPI::isVulkanBackendSelected).thenReturn(true);
            policy.when(()->WorldRenderRoutePolicy.currentModelMeshRoute(true)).thenReturn(WorldRenderRoutePolicy.Route.RUST_VULKAN_WHOLE_FRAME);
            nativeCalls.when(()->RustGalWorldPrimitiveRenderer.enqueueAtlasGlintModelMesh(
                model,state,pose.last(),type,sprite,15728880,overlay,-1,0,null)).thenReturn(true);
            var collector=new SubmitNodeCollection(null);
            collector.submitModelSemantic(model,state,pose,type,15728880,overlay,-1,sprite,0,null);
            nativeCalls.verify(()->RustGalWorldPrimitiveRenderer.enqueueAtlasGlintModelMesh(
                model,state,pose.last(),type,sprite,15728880,overlay,-1,0,null));
            nativeCalls.verifyNoMoreInteractions();
            assertEquals(0,collector.getModelSubmits().totalSubmitCount());
        }
    }
    private String previousByteBuddy;
    @org.junit.jupiter.api.BeforeEach void allowCurrentJdkInstrumentation() {
        previousByteBuddy = System.getProperty("net.bytebuddy.experimental");
        System.setProperty("net.bytebuddy.experimental", "true");
    }
    @org.junit.jupiter.api.AfterEach void restoreInstrumentationProperty() {
        if (previousByteBuddy == null) System.clearProperty("net.bytebuddy.experimental");
        else System.setProperty("net.bytebuddy.experimental", previousByteBuddy);
    }
    @org.junit.jupiter.api.BeforeAll static void bootstrap() {
        net.minecraft.SharedConstants.tryDetectVersion();
        net.minecraft.server.Bootstrap.bootStrap();
    }
    @Test void nativeAdapterKeepsTintAndOutlineInTheirNamedFields() {
        var part = mock(ModelPart.class);
        var sprite = mock(TextureAtlasSprite.class);
        var contents = mock(net.minecraft.client.renderer.texture.SpriteContents.class);
        when(sprite.contents()).thenReturn(contents);
        when(contents.name()).thenReturn(net.minecraft.resources.ResourceLocation.parse("minecraft:entity/shield_base_nopattern"));
        var pose = new PoseStack();
        try (var nativeCalls = mockStatic(RustGalWorldPrimitiveRenderer.class);
             var policy = mockStatic(WorldRenderRoutePolicy.class)) {
            policy.when(() -> WorldRenderRoutePolicy.currentModelPartMeshRoute(true))
                .thenReturn(WorldRenderRoutePolicy.Route.RUST_VULKAN_WHOLE_FRAME);
            for (int[] colors : new int[][]{{-1, 0}, {0xff226688, 0xffddaa44}}) {
                int tint = colors[0], outline = colors[1];
                nativeCalls.when(() -> RustGalWorldPrimitiveRenderer.modelPartMeshEligibilityReason(
                    part, null, sprite, 0, false, false, outline, null)).thenReturn("eligible");
                nativeCalls.when(() -> RustGalWorldPrimitiveRenderer.enqueueModelPartMesh(
                    part, pose.last(), null, sprite, 15728880, 0, false, false, tint, null, outline)).thenReturn(true);
                var collector = new SubmitNodeCollection(null);
                assertDoesNotThrow(() -> collector.submitModelPartSemantic(part, pose, null,
                    15728880, 0, sprite, false, false, tint, null, outline));
                nativeCalls.verify(() -> RustGalWorldPrimitiveRenderer.enqueueModelPartMesh(
                    part, pose.last(), null, sprite, 15728880, 0, false, false, tint, null, outline));
                assertEquals(0, collector.getModelPartSubmits().totalSubmitCount());
            }
        }
    }
}
