package net.vulkanic.world;

import net.minecraft.client.model.ShieldModel;
import org.junit.jupiter.api.Test;
import static org.junit.jupiter.api.Assertions.*;

class ShieldModelSemanticAdmissionTest {
    @org.junit.jupiter.api.BeforeAll static void bootstrap() {
        net.minecraft.SharedConstants.tryDetectVersion();
        net.minecraft.server.Bootstrap.bootStrap();
    }
    @Test void atlasFoilRequiresExactFamilyStateMaterialAndSourceSprite() {
        var model=new ShieldModel(ShieldModel.createLayer().bakeRoot());
        var type=net.minecraft.client.renderer.RenderType.entityGlint();
        var sprite=net.minecraft.client.resources.model.ModelBakery.SHIELD_BASE.texture();
        var unit=net.minecraft.util.Unit.INSTANCE;
        String key="mattmc.dev.rustGalShieldPatterns", before=System.getProperty(key);
        try {
            System.clearProperty(key);
            assertNull(RustGalWorldPrimitiveRenderer.atlasModelFoilIneligibility(model,unit,type,sprite));
            assertNotNull(RustGalWorldPrimitiveRenderer.atlasModelFoilIneligibility(null,unit,type,sprite));
            assertNotNull(RustGalWorldPrimitiveRenderer.atlasModelFoilIneligibility(model,null,type,sprite));
            assertNotNull(RustGalWorldPrimitiveRenderer.atlasModelFoilIneligibility(model,unit,net.minecraft.client.renderer.RenderType.armorEntityGlint(),sprite));
            assertNotNull(RustGalWorldPrimitiveRenderer.atlasModelFoilIneligibility(model,unit,type,null));
            assertNotNull(RustGalWorldPrimitiveRenderer.atlasModelFoilIneligibility(model,unit,type,
                net.minecraft.client.resources.model.ModelBakery.NO_PATTERN_SHIELD.texture()));
        } finally { if(before==null)System.clearProperty(key);else System.setProperty(key,before); }
    }
    @Test void onlyStaticPlainShieldPartFoilIsNormallyAdmitted() {
        var plain=net.minecraft.client.resources.model.ModelBakery.NO_PATTERN_SHIELD.texture();
        assertTrue(RustGalWorldPrimitiveRenderer.isAdmittedShieldPartFoil(plain,false));
        assertFalse(RustGalWorldPrimitiveRenderer.isAdmittedShieldPartFoil(plain,true));
        assertFalse(RustGalWorldPrimitiveRenderer.isAdmittedShieldPartFoil(null,false));
        assertFalse(RustGalWorldPrimitiveRenderer.isAdmittedShieldPartFoil(
            net.minecraft.client.resources.model.ModelBakery.SHIELD_BASE.texture(),false));
        assertFalse(RustGalWorldPrimitiveRenderer.isAdmittedShieldPartFoil(
            net.minecraft.resources.ResourceLocation.withDefaultNamespace("entity/unknown"),false));
    }
    @Test void wholeShieldModelDoesNotRequirePrivateOptIn() throws Exception {
        var method=RustGalWorldPrimitiveRenderer.class.getDeclaredMethod("isSupportedModelMeshModel",net.minecraft.client.model.Model.class);
        method.setAccessible(true);
        var model=new ShieldModel(ShieldModel.createLayer().bakeRoot());
        String key="mattmc.dev.rustGalShieldPatterns";
        String before=System.getProperty(key);
        try {
            System.clearProperty(key);
            assertEquals(true,method.invoke(null,model));
            System.setProperty(key,"false");
            assertEquals(true,method.invoke(null,model));
            assertEquals(false,method.invoke(null,(Object)null));
        } finally {
            if(before==null) System.clearProperty(key); else System.setProperty(key,before);
        }
    }
}
