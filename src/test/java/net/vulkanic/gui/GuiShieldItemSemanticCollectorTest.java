package net.vulkanic.gui;

import net.minecraft.client.model.ShieldModel;
import org.junit.jupiter.api.Test;
import static org.junit.jupiter.api.Assertions.*;

class GuiShieldItemSemanticCollectorTest {
    @org.junit.jupiter.api.BeforeAll static void bootstrap() {
        net.minecraft.SharedConstants.tryDetectVersion();
        net.minecraft.server.Bootstrap.bootStrap();
    }

    @Test void patternedLayersPreserveWholeModelTintOrderAndVanillaLimit() {
        var model = new ShieldModel(ShieldModel.createLayer().bakeRoot());
        var pattern = net.minecraft.core.Holder.direct(new net.minecraft.world.level.block.entity.BannerPattern(
            net.minecraft.resources.ResourceLocation.withDefaultNamespace("cross"), "block.minecraft.banner.cross"));
        var authored = new java.util.ArrayList<net.minecraft.world.level.block.entity.BannerPatternLayers.Layer>();
        for (int i=0;i<20;i++) authored.add(new net.minecraft.world.level.block.entity.BannerPatternLayers.Layer(
            pattern, i%2==0 ? net.minecraft.world.item.DyeColor.RED : net.minecraft.world.item.DyeColor.BLUE));
        var components = net.minecraft.core.component.DataComponentMap.builder()
            .set(net.minecraft.core.component.DataComponents.BASE_COLOR,net.minecraft.world.item.DyeColor.YELLOW)
            .set(net.minecraft.core.component.DataComponents.BANNER_PATTERNS,new net.minecraft.world.level.block.entity.BannerPatternLayers(authored)).build();
        var layers=GuiShieldItemSemanticCollector.layers(model,components,true);
        assertEquals(20,layers.size()); // handle + whole base + foil + dyed base + first16
        assertSame(model.handle(),layers.get(0).part());
        assertSame(model.root(),layers.get(1).part());
        assertTrue(layers.get(2).foil());
        assertFalse(layers.get(2).overlay());
        assertEquals(net.minecraft.world.item.DyeColor.YELLOW.getTextureDiffuseColor(),layers.get(3).tint());
        for(int i=4;i<20;i++) {
            assertSame(model.root(),layers.get(i).part());
            assertTrue(layers.get(i).overlay());
            assertFalse(layers.get(i).foil());
            assertEquals(authored.get(i-4).color().getTextureDiffuseColor(),layers.get(i).tint());
        }
        assertThrows(UnsupportedOperationException.class,layers::clear);
        assertEquals(48,GuiShieldItemSemanticCollector.copyPart(model.root(),0,1,0,1,0xff123456).size());
        assertTrue(GuiShieldItemSemanticCollector.copyPart(model.root(),0,1,0,1,0xff123456)
            .stream().allMatch(v->v.colorArgb()==0xff123456));
        assertEquals(2,GuiShieldItemSemanticCollector.layers(model,net.minecraft.core.component.DataComponentMap.EMPTY,false).size());
    }
    @Test void copyKeepsModelUnitsAndSeparatesLocalFromAtlasFoilCoordinates() {
        var model = new ShieldModel(ShieldModel.createLayer().bakeRoot());
        var base = GuiShieldItemSemanticCollector.copyPart(model.plate(), 0, 1, 0, 1);
        var foil = GuiShieldItemSemanticCollector.copyPart(model.plate(), .25F, .75F, .5F, 1);
        assertEquals(24, base.size());
        for (int i = 0; i < base.size(); i++) {
            var a = base.get(i);
            var b = foil.get(i);
            assertArrayEquals(a.position(), b.position());
            assertEquals(a.normalPacked(), b.normalPacked());
            assertEquals(0xffffffff, a.colorArgb());
            assertArrayEquals(a.localUv(), b.localUv());
            assertArrayEquals(new float[]{.25F + .5F*a.localUv()[0], .5F + .5F*a.localUv()[1]}, b.atlasUv());
            assertTrue(Math.abs(a.position()[0]) <= 6.0F/16.0F);
            assertTrue(Math.abs(a.position()[1]) <= 11.0F/16.0F);
        }
        assertEquals(24, GuiShieldItemSemanticCollector.copyPart(model.handle(), 0, 1, 0, 1).size());
        assertThrows(UnsupportedOperationException.class, base::clear);
        assertThrows(IllegalArgumentException.class, () -> GuiShieldItemSemanticCollector.copyPart(model.plate(), 1, 0, 0, 1));
        assertThrows(IllegalArgumentException.class, () -> GuiShieldItemSemanticCollector.copyPart(model.plate(), Float.NaN, 1, 0, 1));
    }
}
