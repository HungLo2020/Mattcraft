package net.minecraft.client.renderer.special;

import net.minecraft.client.model.ShieldModel;
import net.minecraft.client.renderer.item.ItemStackRenderState;
import net.minecraft.client.renderer.item.ModelRenderProperties;
import net.minecraft.client.renderer.item.SpecialModelWrapper;
import net.minecraft.client.renderer.texture.TextureAtlasSprite;
import net.minecraft.client.resources.model.MaterialSet;
import net.minecraft.client.resources.model.ModelBakery;
import net.minecraft.core.component.DataComponentMap;
import net.minecraft.core.component.DataComponents;
import net.minecraft.world.item.DyeColor;
import net.minecraft.world.item.ItemDisplayContext;
import net.minecraft.world.item.ItemStack;
import net.minecraft.world.level.block.entity.BannerPatternLayers;
import org.junit.jupiter.api.Test;
import static org.junit.jupiter.api.Assertions.*;
import static org.mockito.Mockito.*;

class ShieldGuiAnimationTest {
    @org.junit.jupiter.api.BeforeAll static void bootstrap() {
        net.minecraft.SharedConstants.tryDetectVersion();
        net.minecraft.server.Bootstrap.bootStrap();
    }
    private String previousByteBuddy;
    @org.junit.jupiter.api.BeforeEach void enableJava25Mocks() {
        previousByteBuddy = System.getProperty("net.bytebuddy.experimental");
        System.setProperty("net.bytebuddy.experimental", "true");
    }
    @org.junit.jupiter.api.AfterEach void restoreMockSetting() {
        if (previousByteBuddy == null) System.clearProperty("net.bytebuddy.experimental");
        else System.setProperty("net.bytebuddy.experimental", previousByteBuddy);
    }
    @Test void plainShieldTracksSelectedMaterialAndReloadedContents() {
        var materials = mock(MaterialSet.class);
        var sprite = mock(TextureAtlasSprite.class, RETURNS_DEEP_STUBS);
        when(materials.get(ModelBakery.NO_PATTERN_SHIELD)).thenReturn(sprite);
        var renderer = new ShieldSpecialRenderer(materials, mock(ShieldModel.class));
        assertFalse(renderer.isAnimated(null));
        when(sprite.contents().isAnimated()).thenReturn(true);
        assertTrue(renderer.isAnimated(null));
        var replacement = mock(TextureAtlasSprite.class, RETURNS_DEEP_STUBS);
        when(materials.get(ModelBakery.NO_PATTERN_SHIELD)).thenReturn(replacement);
        assertFalse(renderer.isAnimated(null));
        verify(materials, never()).get(ModelBakery.SHIELD_BASE);
    }

    @Test void coloredShieldDoesNotConsultUnusedPlainTexture() {
        var materials = mock(MaterialSet.class, RETURNS_DEEP_STUBS);
        var components = mock(DataComponentMap.class);
        when(components.getOrDefault(DataComponents.BANNER_PATTERNS, BannerPatternLayers.EMPTY))
            .thenReturn(BannerPatternLayers.EMPTY);
        when(components.get(DataComponents.BASE_COLOR)).thenReturn(DyeColor.RED);
        var renderer = new ShieldSpecialRenderer(materials, mock(ShieldModel.class));
        assertFalse(renderer.isAnimated(components));
        when(materials.get(ModelBakery.SHIELD_BASE).contents().isAnimated()).thenReturn(true);
        assertTrue(renderer.isAnimated(components));
        verify(materials, never()).get(ModelBakery.NO_PATTERN_SHIELD);
    }

    @Test void wrapperPropagatesMaterialAnimationWithoutRequiringFoil() {
        var renderer = mock(SpecialModelRenderer.class);
        var properties = mock(ModelRenderProperties.class);
        var wrapper = new SpecialModelWrapper<>(renderer, properties);
        var stack = mock(ItemStack.class);
        for (boolean animated : new boolean[]{false, true}) {
            when(renderer.isAnimated(null)).thenReturn(animated);
            var state = new ItemStackRenderState();
            wrapper.update(state, stack, null, ItemDisplayContext.GUI, null, null, 0);
            assertEquals(animated, state.isAnimated());
        }
        when(renderer.isAnimated(null)).thenReturn(false);
        when(stack.hasFoil()).thenReturn(true);
        var foilState = new ItemStackRenderState();
        wrapper.update(foilState, stack, null, ItemDisplayContext.GUI, null, null, 0);
        assertTrue(foilState.isAnimated());
    }

    @Test void onlyRenderedPatternLayersInvalidateTheCache() {
        var materials = mock(MaterialSet.class, RETURNS_DEEP_STUBS);
        var renderer = new ShieldSpecialRenderer(materials, mock(ShieldModel.class));
        var components = mock(DataComponentMap.class);
        var layers = new java.util.ArrayList<BannerPatternLayers.Layer>();
        for (int i = 0; i < 17; i++) {
            var pattern = net.minecraft.core.Holder.direct(new net.minecraft.world.level.block.entity.BannerPattern(
                net.minecraft.resources.ResourceLocation.withDefaultNamespace("test_" + i), "test"));
            layers.add(new BannerPatternLayers.Layer(pattern, DyeColor.WHITE));
        }
        when(components.getOrDefault(DataComponents.BANNER_PATTERNS, BannerPatternLayers.EMPTY))
            .thenReturn(new BannerPatternLayers(layers));
        when(materials.get(net.minecraft.client.renderer.Sheets.getShieldMaterial(layers.get(16).pattern()))
            .contents().isAnimated()).thenReturn(true);
        assertFalse(renderer.isAnimated(components), "the seventeenth pattern is never rendered");
        when(materials.get(net.minecraft.client.renderer.Sheets.getShieldMaterial(layers.get(15).pattern()))
            .contents().isAnimated()).thenReturn(true);
        assertTrue(renderer.isAnimated(components), "the last visible pattern must invalidate the GUI cache");
    }
}
