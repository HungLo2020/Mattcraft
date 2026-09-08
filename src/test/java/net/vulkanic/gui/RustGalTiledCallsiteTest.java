package net.vulkanic.gui;

import net.minecraft.client.Minecraft;
import net.minecraft.client.gui.GuiGraphics;
import net.minecraft.client.gui.render.state.GuiElementRenderState;
import net.minecraft.client.gui.render.state.GuiRenderState;
import net.minecraft.client.gui.render.state.TiledBlitRenderState;
import net.minecraft.client.renderer.RenderPipelines;
import net.minecraft.client.resources.model.AtlasManager;
import net.minecraft.resources.ResourceLocation;
import net.vulkanic.bridge.RustGalVulkanWholeFrameMode;
import org.junit.jupiter.api.Test;
import org.mockito.ArgumentCaptor;

import static org.junit.jupiter.api.Assertions.*;
import static org.mockito.Mockito.*;

class RustGalTiledCallsiteTest {
    @Test
    void vulkanNoErrorContextControlDoesNotQueryAJavaBackend() throws Exception {
        String property = RustGalVulkanWholeFrameMode.propertyName();
        String previous = System.getProperty(property);
        String previousByteBuddy = System.getProperty("net.bytebuddy.experimental");
        System.setProperty(property, "true");
        System.setProperty("net.bytebuddy.experimental", "true");
        try (var api = mockStatic(net.vulkanic.VulkanicAPI.class);
             var sodium = mockStatic(net.sodium.client.SodiumClientMod.class);
             var game = mockStatic(Minecraft.class)) {
            game.when(Minecraft::getInstance).thenReturn(mock(Minecraft.class));
            var method = net.sodium.client.gui.SodiumGameOptionPages.class.getDeclaredMethod("supportsNoErrorContext");
            method.setAccessible(true);
            assertEquals(false, method.invoke(null));
            api.verifyNoInteractions();
        } finally {
            if (previous == null) System.clearProperty(property); else System.setProperty(property, previous);
            if (previousByteBuddy == null) System.clearProperty("net.bytebuddy.experimental");
            else System.setProperty("net.bytebuddy.experimental", previousByteBuddy);
        }
    }

    @Test
    void panoramaIsBeforeTheFirstScreenBlurBoundary() {
        var state = new GuiRenderState();
        state.nextStratum(); // Screen.renderWithTooltipAndSubtitles
        state.blurBeforeThisStratum(); // Screen.renderBlurredBackground
        assertEquals(1, state.blurBeforeStratumIndex());
        assertTrue(GuiRenderStratum.GUI_PANORAMA.order() > 0,
            "the explicit GUI mesh ABI reserves zero as an invalid stratum");
        assertTrue(GuiRenderStratum.GUI_PANORAMA.order() < state.blurBeforeStratumIndex() * 3,
            "the panorama must already be drawn when Rust snapshots the menu background for blur");
    }

    @Test
    void unavailableMenuCapabilitiesDoNotConstructRuntimeBoundControls() {
        var builder = net.sodium.client.gui.options.OptionGroup.createBuilder();
        assertSame(builder, builder.addIf(false, () -> {
            fail("Unavailable controls must not read an uninitialized renderer runtime");
            return null;
        }));
    }

    @Test
    void menuBackgroundHasOneTextureRepeatPerTileAtEveryWindowSize() {
        withGui((gui, state) -> {
            for (int[] size : new int[][] {{640, 338}, {1920, 1012}, {3840, 2160}}) {
                clearInvocations(state);
                gui.blitTiled(RenderPipelines.GUI_TEXTURED,
                    ResourceLocation.withDefaultNamespace("textures/gui/menu_background.png"),
                    7, 11, 8, 16, size[0], size[1], 32, 32, -1);
                TiledBlitRenderState tile = captured(state);
                assertEquals(32, tile.tileWidth());
                assertEquals(32, tile.tileHeight());
                assertEquals(7 + size[0], tile.x1());
                assertEquals(11 + size[1], tile.y1());
                assertEquals(0.25F, tile.u0());
                assertEquals(1.25F, tile.u1());
                assertEquals(0.5F, tile.v0());
                assertEquals(1.5F, tile.v1());
                assertNull(tile.textureSetup().texure0(), "semantic callsite must not acquire a Java GPU texture");
            }
        });
    }

    @Test
    void fullscreenTiledSemanticsPreserveFieldOrder() {
        withGui((gui, state) -> {
            gui.submitRustSemanticTiledBlit(ResourceLocation.withDefaultNamespace("textures/gui/menu_background.png"),
                7, 11, 640, 338, 32, 16, 0.25F, 0.5F, 0.75F, 1.0F, -1);
            TiledBlitRenderState tile = captured(state);
            assertEquals(32, tile.tileWidth());
            assertEquals(16, tile.tileHeight());
            assertEquals(7, tile.x0());
            assertEquals(11, tile.y0());
            assertEquals(647, tile.x1());
            assertEquals(349, tile.y1());
            assertEquals(0.25F, tile.u0());
            assertEquals(0.75F, tile.u1());
            assertEquals(0.5F, tile.v0());
            assertEquals(1.0F, tile.v1());
        });
    }

    private static TiledBlitRenderState captured(GuiRenderState state) {
        var argument = ArgumentCaptor.forClass(GuiElementRenderState.class);
        verify(state).submitGuiElement(argument.capture());
        return assertInstanceOf(TiledBlitRenderState.class, argument.getValue());
    }

    private static void withGui(java.util.function.BiConsumer<GuiGraphics, GuiRenderState> test) {
        String property = RustGalVulkanWholeFrameMode.propertyName();
        String previous = System.getProperty(property);
        // The bundled Mockito/Byte Buddy predates the project's Java 25
        // toolchain. This opt-in affects test instrumentation only.
        String previousByteBuddy = System.getProperty("net.bytebuddy.experimental");
        System.setProperty("net.bytebuddy.experimental", "true");
        System.setProperty(property, "true");
        try {
            net.minecraft.SharedConstants.tryDetectVersion();
            net.minecraft.server.Bootstrap.bootStrap();
            Minecraft minecraft = mock(Minecraft.class);
            when(minecraft.getAtlasManager()).thenReturn(mock(AtlasManager.class));
            GuiRenderState state = mock(GuiRenderState.class);
            test.accept(new GuiGraphics(minecraft, state), state);
        } finally {
            if (previous == null) System.clearProperty(property);
            else System.setProperty(property, previous);
            if (previousByteBuddy == null) System.clearProperty("net.bytebuddy.experimental");
            else System.setProperty("net.bytebuddy.experimental", previousByteBuddy);
        }
    }
}
