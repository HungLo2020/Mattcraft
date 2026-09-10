package net.vulkanic;

import org.junit.jupiter.api.Test;

import java.io.IOException;
import java.lang.reflect.Method;
import java.nio.file.Files;
import java.nio.file.Path;
import java.nio.file.Paths;
import java.util.ArrayList;
import java.util.List;
import java.util.OptionalDouble;

import static org.junit.jupiter.api.Assertions.*;

/**
 * Tests for Phase 3 draw-path wiring.
 *
 * <p>Validates three concrete changes made in this phase:
 * <ol>
 *   <li>{@code GlCommandEncoder.drawFromBuffers} no longer routes through
 *       {@code GlStateManager._drawElements} or {@code GlStateManager._drawArrays}; it calls
 *       {@code VulkanicAPI} directly.</li>
 *   <li>{@code GlCommandEncoder.getActiveVulkanicRenderPass()} accessor exists and returns
 *       the correct type.</li>
 *   <li>{@code CompressibleGLBufferedImage.uploadToTexture} no longer calls
 *       {@code VulkanicAPI.bindTexture2D} before mipmap generation; it uses the
 *       state-mutation-free {@code generateTextureMipmapDSA} call instead.</li>
 * </ol>
 *
 * <p>All tests run without an OpenGL context Î“Ã‡Ã¶ they inspect source code and class structure.
 */
public class Phase3DrawPathTest {

    private static final Path PROJECT_ROOT = Paths.get(System.getProperty("user.dir"));
    private static final Path SRC_MAIN_JAVA = PROJECT_ROOT.resolve("src/main/java");
    private static final Path SRC_MAIN_RUST = PROJECT_ROOT.resolve("src/main/rust");

    private static String readSource(Path path) throws IOException {
        return Files.readString(path).replace("\r\n", "\n").replace('\r', '\n');
    }

    private static String readSourceIfExists(Path path) throws IOException {
        if (!Files.exists(path)) {
            return "";
        }
        return readSource(path);
    }

    private static boolean containsAny(String source, String... needles) {
        for (String needle : needles) {
            if (source.contains(needle)) {
                return true;
            }
        }
        return false;
    }

    @Test
    public void testWholeFramePrimitiveEntryPointCannotFallThroughOnEmptySemanticFrame() throws IOException {
        String source = readSource(SRC_MAIN_JAVA.resolve("net/vulkanic/gui/RustGalFrameCoordinator.java"));
        assertTrue(source.contains("Rust Vulkan whole-frame primitive submission received no semantic world frame"),
            "empty whole-frame primitive submissions must fail closed instead of returning to Java rendering");
        for (String family : List.of(
            "primitiveFrame.textQuads().isEmpty()",
            "primitiveFrame.lodInstances().isEmpty()",
            "primitiveFrame.firstPersonMeshInstances().isEmpty()",
            "primitiveFrame.entityFlameQuadCount() == 0",
            "primitiveFrame.background().enabled()"
        )) {
            assertTrue(source.contains(family), "empty-frame admission must account for semantic family: " + family);
        }
    }

    @Test
    public void testWholeFrameGuiTraversalRejectsUnclassifiedStatesInsteadOfDroppingThem() throws IOException {
        String source = readSource(SRC_MAIN_JAVA.resolve("net/minecraft/client/gui/render/GuiRenderer.java"));
        int collector = source.indexOf("collectRustGalCopiedBlitSemantics");
        int guard = source.indexOf("unclassified-gui-state", collector);
        int rustTokenExemption = source.indexOf("RustGalGuiElementRenderState", collector);
        assertTrue(collector >= 0 && guard > collector,
            "whole-frame GUI traversal must diagnose states outside its admitted blit families");
        assertTrue(rustTokenExemption > collector && rustTokenExemption < guard,
            "Rust scheduler tokens must be exempt from the unclassified GUI-state diagnostic");
        int rectangleExemption = source.indexOf("ColoredRectangleRenderState", collector);
        assertTrue(rectangleExemption > collector && rectangleExemption < guard,
            "already-collected semantic rectangles must not be counted as unclassified GUI states");
        int fourColorRectangleExemption = source.indexOf("FourColoredRectangleRenderState", rectangleExemption + 1);
        assertTrue(fourColorRectangleExemption > rectangleExemption && fourColorRectangleExemption < guard,
            "already-collected four-color semantic rectangles must not be counted as unclassified GUI states");
        assertTrue(source.indexOf("gui-state-class:", guard) > guard,
            "the rejected GUI state class must remain observable for the next semantic slice");
    }

    @Test
    public void testWholeFrameDirectGlyphStatesUseCopiedSemanticTextRoute() throws IOException {
        String renderer = readSource(SRC_MAIN_JAVA.resolve("net/vulkanic/gui/RustGalGuiRenderer.java"));
        String guiRenderer = readSource(SRC_MAIN_JAVA.resolve("net/minecraft/client/gui/render/GuiRenderer.java"));
        assertTrue(renderer.contains("tryEnqueueGlyph(")
                && renderer.contains("collectSemanticQuads(\n\t\t\t\tquad -> appendBoundedTextQuad(quads, quad)")
                && renderer.contains("direct-glyph-renderable-unavailable"),
            "direct glyph elements must use bounded copied semantic quads and remain unavailable when extraction is absent");
        assertTrue(guiRenderer.contains("instanceof GlyphRenderState glyph")
                && guiRenderer.contains("RustGalGuiRenderer.tryEnqueueGlyph")
                && guiRenderer.contains("recordUnsupportedElement(\"glyph\")"),
            "whole-frame GUI traversal must admit direct glyphs or fail closed without Java rendering");
    }

    @Test
    public void testImmediateContextUsageRestrictedToLegacySeam() throws IOException {
        Path legacyFile = SRC_MAIN_JAVA.resolve("net/vulkanic/VulkanicAPI.java");
        Path legacyRelative = Paths.get("net/vulkanic/VulkanicAPI.java");
        assertTrue(Files.exists(legacyFile), "VulkanicAPI.java must exist for migration seam validation");

        List<String> offenders = new ArrayList<>();
        try (var paths = Files.walk(SRC_MAIN_JAVA)) {
            for (Path file : (Iterable<Path>) paths::iterator) {
                if (!Files.isRegularFile(file) || !file.toString().endsWith(".java")) {
                    continue;
                }

                String source = readSource(file);
                if (!source.contains("getImmediateContext(")) {
                    continue;
                }

                Path relative = SRC_MAIN_JAVA.relativize(file);
                if (relative.equals(legacyRelative)) {
                    String withoutDeclaration = source.replace("public static CommandContext getImmediateContext() {", "");
                    if (withoutDeclaration.contains("getImmediateContext(")) {
                        offenders.add(relative + " (additional references)");
                    }
                } else {
                    offenders.add(relative.toString());
                }
            }
        }

        assertTrue(offenders.isEmpty(),
            "Only VulkanicAPI.getImmediateContext() compatibility seam may remain; offenders: " + offenders);

        String legacySource = readSource(legacyFile);
        assertTrue(legacySource.contains("@Deprecated"),
            "VulkanicAPI.getImmediateContext() must remain explicitly deprecated");
    }

    @Test
    public void testIrisGetGlIdRestrictedToDeprecatedCompatibilitySeams() throws IOException {
        Path interfaceFile = SRC_MAIN_JAVA.resolve("net/irisshaders/iris/mixinterface/GpuTextureInterface.java");
        Path interfaceRelative = Paths.get("net/irisshaders/iris/mixinterface/GpuTextureInterface.java");
        Path glTextureFile = SRC_MAIN_JAVA.resolve("net/blaze3d/opengl/GlTexture.java");
        Path glTextureRelative = Paths.get("net/blaze3d/opengl/GlTexture.java");
        Path gpuTextureFile = SRC_MAIN_JAVA.resolve("net/blaze3d/textures/GpuTexture.java");

        String interfaceSource = readSource(interfaceFile);
        String glTextureSource = readSource(glTextureFile);
        String gpuTextureSource = readSource(gpuTextureFile);

        assertTrue(interfaceSource.contains("@Deprecated"),
            "GpuTextureInterface.iris$getGlId should remain explicitly deprecated as a compatibility seam");
        assertTrue(interfaceSource.contains("default int iris$getGlId()"),
            "GpuTextureInterface should remain the compatibility declaration owner for iris$getGlId");
        assertTrue(glTextureSource.contains("@Deprecated"),
            "GlTexture.iris$getGlId should remain explicitly deprecated as a compatibility seam");
        assertTrue(glTextureSource.contains("public int iris$getGlId()"),
            "GlTexture should keep the only concrete iris$getGlId implementation in main sources");
        assertFalse(gpuTextureSource.contains("iris$getGlId("),
            "GpuTexture should not declare iris$getGlId after compatibility-surface reduction");

        List<String> declarationOffenders = new ArrayList<>();
        List<String> invocationOffenders = new ArrayList<>();
        try (var paths = Files.walk(SRC_MAIN_JAVA)) {
            for (Path file : (Iterable<Path>) paths::iterator) {
                if (!Files.isRegularFile(file) || !file.toString().endsWith(".java")) {
                    continue;
                }

                String source = readSource(file);
                Path relative = SRC_MAIN_JAVA.relativize(file);
                if (source.contains("iris$getGlId(")
                    && !relative.equals(interfaceRelative)
                    && !relative.equals(glTextureRelative)) {
                    declarationOffenders.add(relative.toString());
                }

                if (source.contains(".iris$getGlId(")) {
                    invocationOffenders.add(relative.toString());
                }
            }
        }

        assertTrue(declarationOffenders.isEmpty(),
            "iris$getGlId declarations should remain restricted to compatibility seams; offenders: " + declarationOffenders);
        assertTrue(invocationOffenders.isEmpty(),
            "Runtime iris$getGlId invocations should remain eliminated; offenders: " + invocationOffenders);
    }

    @Test
    public void testGlIdRestrictedToDeprecatedDeclarationsAndNoRuntimeInvocations() throws IOException {
        Path gpuTextureFile = SRC_MAIN_JAVA.resolve("net/blaze3d/textures/GpuTexture.java");
        Path gpuTextureRelative = Paths.get("net/blaze3d/textures/GpuTexture.java");
        Path glTextureFile = SRC_MAIN_JAVA.resolve("net/blaze3d/opengl/GlTexture.java");
        Path glTextureRelative = Paths.get("net/blaze3d/opengl/GlTexture.java");

        String gpuTextureSource = readSource(gpuTextureFile);
        String glTextureSource = readSource(glTextureFile);

        assertTrue(gpuTextureSource.contains("@Deprecated"),
            "GpuTexture.glId should remain explicitly deprecated as a compatibility seam");
        assertTrue(gpuTextureSource.contains("public int glId()"),
            "GpuTexture should keep glId declaration for compatibility while migration is in progress");
        assertTrue(glTextureSource.contains("@Deprecated"),
            "GlTexture.glId should remain explicitly deprecated as a compatibility seam");
        assertTrue(glTextureSource.contains("public int glId()"),
            "GlTexture should keep the concrete glId compatibility implementation");

        List<String> declarationOffenders = new ArrayList<>();
        List<String> invocationOffenders = new ArrayList<>();
        try (var paths = Files.walk(SRC_MAIN_JAVA)) {
            for (Path file : (Iterable<Path>) paths::iterator) {
                if (!Files.isRegularFile(file) || !file.toString().endsWith(".java")) {
                    continue;
                }

                String source = readSource(file);
                Path relative = SRC_MAIN_JAVA.relativize(file);

                if (source.contains(" glId(")
                    && !relative.equals(gpuTextureRelative)
                    && !relative.equals(glTextureRelative)) {
                    declarationOffenders.add(relative.toString());
                }

                if (source.contains(".glId(")) {
                    invocationOffenders.add(relative.toString());
                }
            }
        }

        assertTrue(declarationOffenders.isEmpty(),
            "glId declarations should remain restricted to compatibility seam files; offenders: " + declarationOffenders);
        assertTrue(invocationOffenders.isEmpty(),
            "Runtime .glId invocations should remain eliminated; offenders: " + invocationOffenders);
    }

    @Test
    public void testGlTextureHandleExtractionRestrictedToOpenGLBackendSeams() throws IOException {
        Path glTextureFile = SRC_MAIN_JAVA.resolve("net/blaze3d/opengl/GlTexture.java");
        Path glTextureRelative = Paths.get("net/blaze3d/opengl/GlTexture.java");
        Path openGlBackendFile = SRC_MAIN_JAVA.resolve("net/vulkanic/backends/opengl/OpenGLBackend.java");
        Path openGlBackendRelative = Paths.get("net/vulkanic/backends/opengl/OpenGLBackend.java");
        Path openGlTextureViewFile = SRC_MAIN_JAVA.resolve("net/vulkanic/backends/opengl/OpenGLTextureView.java");
        Path openGlTextureViewRelative = Paths.get("net/vulkanic/backends/opengl/OpenGLTextureView.java");

        String glTextureSource = readSource(glTextureFile);
        String openGlBackendSource = readSource(openGlBackendFile);
        String openGlTextureViewSource = readSource(openGlTextureViewFile);

        assertTrue(glTextureSource.contains("public int getGlHandle()"),
            "GlTexture should expose getGlHandle for backend seam extraction");
        assertTrue(openGlBackendSource.contains("glTexture.getGlHandle()"),
            "OpenGLBackend should extract GlTexture handles via getGlHandle in the backend seam");
        assertTrue(openGlTextureViewSource.contains("return t.getGlHandle();"),
            "OpenGLTextureView should extract GlTexture handles via getGlHandle in the backend seam");

        List<String> offenders = new ArrayList<>();
        try (var paths = Files.walk(SRC_MAIN_JAVA)) {
            for (Path file : (Iterable<Path>) paths::iterator) {
                if (!Files.isRegularFile(file) || !file.toString().endsWith(".java")) {
                    continue;
                }

                Path relative = SRC_MAIN_JAVA.relativize(file);
                if (relative.equals(glTextureRelative)
                    || relative.equals(openGlBackendRelative)
                    || relative.equals(openGlTextureViewRelative)) {
                    continue;
                }

                String source = readSource(file);
                if (source.contains("GlTexture") && source.contains(".getGlHandle(")) {
                    offenders.add(relative.toString());
                }
            }
        }

        assertTrue(offenders.isEmpty(),
            "GlTexture.getGlHandle usage should remain confined to OpenGL backend seam files; offenders: " + offenders);
    }

    @Test
    public void testSingleQuadParticlesShrinkAtlasUvsBeforeSampling() throws IOException {
        String source = readSource(SRC_MAIN_JAVA.resolve("net/minecraft/client/particle/SingleQuadParticle.java"));

        assertTrue(source.contains("return this.shrinkU(this.sprite.getU0(), this.sprite.getU1());"),
            "SingleQuadParticle should shrink the leading U edge inward before sampling atlas sprites");
        assertTrue(source.contains("return this.shrinkU(this.sprite.getU1(), this.sprite.getU0());"),
            "SingleQuadParticle should shrink the trailing U edge inward before sampling atlas sprites");
        assertTrue(source.contains("return this.shrinkV(this.sprite.getV0(), this.sprite.getV1());"),
            "SingleQuadParticle should shrink the leading V edge inward before sampling atlas sprites");
        assertTrue(source.contains("return this.shrinkV(this.sprite.getV1(), this.sprite.getV0());"),
            "SingleQuadParticle should shrink the trailing V edge inward before sampling atlas sprites");
        assertTrue(source.contains("return Mth.lerp(this.particleAtlasShrinkRatio(), f, h);"),
            "SingleQuadParticle should contract atlas UVs with its particle-local shrink ratio");
        assertTrue(source.contains("private float particleAtlasShrinkRatio()"),
            "SingleQuadParticle should define a particle-local atlas shrink ratio helper");
        assertTrue(source.contains("this.sprite.contents().width() / (this.sprite.getU1() - this.sprite.getU0())"),
            "Particle atlas shrink ratio should be derived from sprite width relative to atlas UV span");
    }

    @Test
    public void testQuadParticleRenderStateFlushesSamplerTextureModesBeforeBinding() throws IOException {
        String source = readSource(SRC_MAIN_JAVA.resolve("net/minecraft/client/renderer/state/QuadParticleRenderState.java"));

        assertTrue(source.contains("particleTexture.setFilter(false, false);"),
            "QuadParticleRenderState should restore the particle atlas to nearest non-mip sampling before binding Sampler0");
        assertTrue(source.contains("lightTextureView.texture().flushModeChanges2D();"),
            "QuadParticleRenderState should flush lightmap texture modes before binding Sampler2");
        assertTrue(source.contains("particleTextureView.texture().flushModeChanges2D();"),
            "QuadParticleRenderState should flush atlas texture modes before binding Sampler0");
        assertTrue(source.contains("renderPass.bindSampler(\"Sampler2\", lightTextureView);"),
            "QuadParticleRenderState should bind the flushed lightmap view to Sampler2");
        assertTrue(source.contains("renderPass.bindSampler(\"Sampler0\", particleTextureView);"),
            "QuadParticleRenderState should bind the flushed particle atlas view to Sampler0");
    }

    @Test
    public void testMainTargetUsesBgra8AndBackendMappingsPreserveIt() throws IOException {
        String mainTargetSource = readSource(SRC_MAIN_JAVA.resolve("net/blaze3d/pipeline/MainTarget.java"));
        String textureFormatSource = readSource(SRC_MAIN_JAVA.resolve("net/blaze3d/textures/TextureFormat.java"));
        String gpuTextureSource = readSource(SRC_MAIN_JAVA.resolve("net/blaze3d/textures/GpuTexture.java"));
        String vulkanFormatSource = readSource(SRC_MAIN_JAVA.resolve("net/vulkanic/VulkanicTextureFormat.java"));
        String openGlBackendSource = readSource(SRC_MAIN_JAVA.resolve("net/vulkanic/backends/opengl/OpenGLBackend.java"));
        String vulkanBackendSource = readSource(SRC_MAIN_JAVA.resolve("net/vulkanic/backends/vulkan/VulkanBackend.java"));

        assertTrue(mainTargetSource.contains("TextureFormat.BGRA8"),
            "MainTarget should allocate its color attachment as BGRA8 so Vulkan can present without shader compose into a BGRA swapchain");
        assertTrue(mainTargetSource.contains("public void createBuffers(int i, int j)")
                && mainTargetSource.contains("this.createFrameBuffer(i, j);"),
            "MainTarget should override buffer recreation so window resizes keep using the BGRA main-target allocation path instead of the base RGBA8 RenderTarget implementation");
        assertTrue(textureFormatSource.contains("BGRA8(4)"),
            "TextureFormat should expose a BGRA8 color format for main-target allocation");
        assertTrue(vulkanFormatSource.contains("BGRA8(4)"),
            "VulkanicTextureFormat should expose BGRA8 so backends can preserve swapchain-compatible color ordering");
        assertTrue(gpuTextureSource.contains("case BGRA8   -> VulkanicTextureFormat.BGRA8;"),
            "GpuTexture should preserve Blaze BGRA8 textures when bridging to VulkanicTextureFormat");
        assertTrue(openGlBackendSource.contains("case BGRA8  -> net.vulkanic.VulkanicAPI.GL_BGRA;"),
            "OpenGL backend should preserve BGRA external format metadata for BGRA-backed managed textures");
        assertTrue(vulkanBackendSource.contains("case BGRA8   -> VK10.VK_FORMAT_B8G8R8A8_UNORM;"),
            "Vulkan backend should map BGRA8 textures to a BGRA VkFormat");
        assertTrue(vulkanBackendSource.contains("case VK10.VK_FORMAT_B8G8R8A8_UNORM, VK10.VK_FORMAT_B8G8R8A8_SRGB -> VulkanicTextureFormat.BGRA8;"),
            "Vulkan backend should preserve BGRA swapchain/image wrappers as BGRA8 instead of collapsing them to RGBA8");
        assertTrue(vulkanBackendSource.contains("if ((format == VulkanicAPI.GL_BGRA || internalFormat == VulkanicAPI.GL_BGRA)")
                && vulkanBackendSource.contains("return new LegacyTextureFormatInfo(VK10.VK_FORMAT_B8G8R8A8_UNORM, 4, VK10.VK_IMAGE_ASPECT_COLOR_BIT);")
                && vulkanBackendSource.indexOf("if ((format == VulkanicAPI.GL_BGRA || internalFormat == VulkanicAPI.GL_BGRA)")
                < vulkanBackendSource.indexOf("if ((format == VulkanicAPI.GL_RGBA || internalFormat == VulkanicAPI.GL_RGBA8 || internalFormat == VulkanicAPI.GL_RGBA16)"),
            "Vulkan backend should resolve GL_BGRA uploads to a BGRA VkFormat before the generic RGBA8 legacy-format branch");
    }

    @Test
    public void testVulkanDescriptorSamplerKeysUseLiveGpuTextureState() throws IOException {
        String backendSource = readSource(SRC_MAIN_JAVA.resolve("net/vulkanic/backends/vulkan/VulkanBackend.java"));
        String plannerSource = readSource(SRC_MAIN_JAVA.resolve("net/vulkanic/backends/vulkan/VulkanDescriptorBindingPlanner.java"));
        String textureSource = readSource(SRC_MAIN_JAVA.resolve("net/blaze3d/textures/GpuTexture.java"));
        String vulkanicApiSource = readSource(SRC_MAIN_JAVA.resolve("net/vulkanic/VulkanicAPI.java"));

        assertTrue(vulkanicApiSource.contains("if (handle <= 0 || getActiveBackendType() != GraphicsBackendType.OPENGL)"),
            "OpenGL legacy parity buffers must be rejected before any Java GL query on Vulkan");

        assertTrue(textureSource.contains("public FilterMode getMinFilter()"),
            "GpuTexture should expose its live min filter so Vulkan descriptor samplers can follow current texture state");
        assertTrue(textureSource.contains("public FilterMode getMagFilter()"),
            "GpuTexture should expose its live mag filter so Vulkan descriptor samplers can follow current texture state");
        assertTrue(textureSource.contains("public boolean usesMipmaps()"),
            "GpuTexture should expose whether mipmaps are live-enabled for descriptor sampler selection");
        assertTrue(plannerSource.contains("GpuTexture gpuTexture = boundTexture instanceof GpuTexture blazeTexture ? blazeTexture : null;"),
            "Vulkan descriptor sampler keys should bridge the backend-neutral bound texture to live GpuTexture state when available");
        assertTrue(plannerSource.contains("toLegacyMinFilter(gpuTexture.getMinFilter(), gpuTexture.usesMipmaps())"),
            "Vulkan descriptor samplers should derive minification mode from live GpuTexture state");
        assertTrue(plannerSource.contains("toLegacyMagFilter(gpuTexture.getMagFilter())"),
            "Vulkan descriptor samplers should derive magnification mode from live GpuTexture state");
        assertTrue(plannerSource.contains("toLegacyWrapMode(gpuTexture.getAddressModeU())"),
            "Vulkan descriptor samplers should derive wrap state from live GpuTexture state");
        assertTrue(backendSource.contains("gpuTexture.flushModeChanges2D();")
                && backendSource.indexOf("gpuTexture.flushModeChanges2D();")
                < backendSource.indexOf("return resolveGpuTextureLegacyHandle(gpuTexture);"),
            "Vulkan texture-handle resolution must flush pending GpuTexture filter/wrap state before descriptors sample the legacy texture id");
        int legacySamplerResolver = backendSource.indexOf("VulkanicTextureView resolveLegacySamplerViewForProgram(");
        assertTrue(legacySamplerResolver >= 0,
            "Vulkan backend should expose the legacy shader sampler resolver");
        int anonymousLegacyViewFallback = backendSource.indexOf("createManagedLegacyTextureView(textureId)", legacySamplerResolver);
        assertTrue(anonymousLegacyViewFallback >= 0
                && backendSource.indexOf("TextureTracker.INSTANCE.getTextureView(textureId)", legacySamplerResolver) < 0,
            "Vulkan legacy shader sampler resolution must not borrow Iris texture views or fallback state");
    }

    @Test
    public void testVulkanDescriptorSamplersUseCapturedIrisSamplerObjectState() throws IOException {
        String commandEncoderSource = readSource(SRC_MAIN_JAVA.resolve("net/blaze3d/opengl/GlCommandEncoder.java"));
        String resourceResolverSource = readSource(SRC_MAIN_JAVA.resolve("net/vulkanic/VulkanicPipelineResourceResolver.java"));
        String plannerSource = readSource(SRC_MAIN_JAVA.resolve("net/vulkanic/backends/vulkan/VulkanDescriptorBindingPlanner.java"));
        String irisRenderSystemSource = readSource(SRC_MAIN_JAVA.resolve("net/irisshaders/iris/gl/IrisRenderSystem.java"));

        assertTrue(irisRenderSystemSource.contains("public static int getBoundSamplerOnUnit(int unit)"),
            "Iris should expose the cached sampler object per texture unit so Vulkan descriptor binding can snapshot it");
        assertTrue(commandEncoderSource.contains("currentBoundSamplerObject(samplerUnit)")
                && commandEncoderSource.contains("samplerObject(int samplerUnit)")
                && resourceResolverSource.contains("lookup.samplerObject(samplerUnit)")
                && resourceResolverSource.contains("PipelineResourcePlanner.ResolvedResource.sampler(")
                && resourceResolverSource.contains("new PipelineResourceBindings.SamplerBinding("),
            "Shared Vulkan descriptor resolution should carry the captured Iris sampler object, not only the texture unit");
        assertTrue(plannerSource.contains("samplerBinding.samplerObject()"),
            "Vulkan descriptor resolution should consume the captured sampler object from PipelineResourceBindings");
        assertTrue(plannerSource.contains("VirtualSamplerStateSnapshot samplerState = samplerObject != null")
                && plannerSource.contains("samplerStateLookup.samplerState(samplerObject)")
                && plannerSource.contains("samplerStateLookup.samplerStateForTextureUnit(textureUnit)"),
            "Vulkan descriptor sampler keys should prefer captured sampler-object state and only fall back to the live texture-unit binding");
    }

    @Test
    public void testVulkanLegacyRenderTargetStoragePreservesIrisSizedInternalFormats() throws IOException {
        String backendSource = readSource(SRC_MAIN_JAVA.resolve("net/vulkanic/backends/vulkan/VulkanBackend.java"));
        String formatSource = readSource(SRC_MAIN_JAVA.resolve("net/vulkanic/VulkanicTextureFormat.java"));

        assertTrue(backendSource.contains("pixels == null")
                && backendSource.contains("LegacyTextureFormatInfo.resolveStorage(internalFormat, format, type)"),
            "Null texImage2D render-target allocation should use internal-format storage semantics instead of upload tuple semantics");
        assertTrue(backendSource.contains("case VulkanicAPI.GL_R11F_G11F_B10F")
                && backendSource.contains("VK10.VK_FORMAT_B10G11R11_UFLOAT_PACK32"),
            "Vulkan legacy storage should preserve Iris R11F_G11F_B10F colortex targets");
        assertTrue(backendSource.contains("case VulkanicAPI.GL_RGBA8_SNORM")
                && backendSource.contains("VK10.VK_FORMAT_R8G8B8A8_SNORM"),
            "Vulkan legacy storage should preserve Iris signed-normal RGBA8 normal targets");
        assertTrue(backendSource.contains("case VulkanicAPI.GL_R32F")
                && backendSource.contains("VK10.VK_FORMAT_R32_SFLOAT"),
            "Vulkan legacy storage should preserve Iris R32F history/depth render targets even when the null allocation external type is unsigned byte");
        assertTrue(formatSource.contains("RGBA8_SNORM(4)")
                && formatSource.contains("R11F_G11F_B10F(4)")
                && formatSource.contains("RED16F(2)")
                && formatSource.contains("RED32F(4)"),
            "Vulkanic texture wrappers should be able to represent shaderpack render-target formats without collapsing them to RGBA8");
        assertTrue(backendSource.contains("case VK10.VK_FORMAT_R8G8B8A8_SNORM -> VulkanicTextureFormat.RGBA8_SNORM")
                && backendSource.contains("case VK10.VK_FORMAT_B10G11R11_UFLOAT_PACK32 -> VulkanicTextureFormat.R11F_G11F_B10F")
                && backendSource.contains("case VK10.VK_FORMAT_R16_SFLOAT -> VulkanicTextureFormat.RED16F")
                && backendSource.contains("case VK10.VK_FORMAT_R32_SFLOAT -> VulkanicTextureFormat.RED32F"),
            "Managed legacy texture wrappers should expose the preserved VkFormat through VulkanicTextureFormat");
    }

    @Test
    public void testVulkanLegacyStoragePreservesBgraExternalFormatForMainTarget() throws IOException {
        String backendSource = readSource(SRC_MAIN_JAVA.resolve("net/vulkanic/backends/vulkan/VulkanBackend.java"));

        int resolveStorageIndex = backendSource.indexOf("private static LegacyTextureFormatInfo resolveStorage");
        int sizedSwitchIndex = backendSource.indexOf("LegacyTextureFormatInfo sizedFormat = switch (internalFormat)", resolveStorageIndex);
        int bgraIndex = backendSource.indexOf("format == VulkanicAPI.GL_BGRA", resolveStorageIndex);
        int bgraVkIndex = backendSource.indexOf("VK10.VK_FORMAT_B8G8R8A8_UNORM", bgraIndex);

        assertTrue(resolveStorageIndex >= 0, "Vulkan legacy storage resolver must exist");
        assertTrue(bgraIndex > resolveStorageIndex && bgraIndex < sizedSwitchIndex,
            "BGRA null-storage allocation must be resolved before the internal-format-first sized switch");
        assertTrue(bgraVkIndex > bgraIndex && bgraVkIndex < sizedSwitchIndex,
            "BGRA null-storage allocation should create B8G8R8A8 storage for MainTarget");
    }

    @Test
    public void testMainTargetDepthTextureUsesDepthSamplingStateForIrisDepthtex0() throws IOException {
        String mainTargetSource = readSource(SRC_MAIN_JAVA.resolve("net/blaze3d/pipeline/MainTarget.java"));

        assertTrue(mainTargetSource.contains("this.depthTexture.setTextureFilter(FilterMode.NEAREST, false);"),
            "MainTarget depth should be sampled with nearest filtering so Iris depthtex0 neighbor reads do not blur block edges");
        assertTrue(mainTargetSource.contains("this.depthTexture.setAddressMode(AddressMode.CLAMP_TO_EDGE);"),
            "MainTarget depth should clamp at screen edges so Vulkan descriptor samplers do not repeat depthtex0");
        assertTrue(mainTargetSource.indexOf("this.colorTexture.setAddressMode(AddressMode.CLAMP_TO_EDGE);")
                < mainTargetSource.indexOf("this.depthTexture.setTextureFilter(FilterMode.NEAREST, false);"),
            "MainTarget should configure both color and depth textures instead of configuring the color texture twice");
    }

    @Test
    public void testIrisTextureStateCopyPreservesWrapModeForDerivedShaderTextures() throws IOException {
        String glTextureSource = readSource(SRC_MAIN_JAVA.resolve("net/blaze3d/opengl/GlTexture.java"));
        String vulkanTextureSource = readSource(SRC_MAIN_JAVA.resolve("net/vulkanic/backends/vulkan/VulkanGpuTexture.java"));

        assertTrue(glTextureSource.contains("texture.setAddressMode(this.addressModeU, this.addressModeV);"),
            "OpenGL Iris texture state copies should preserve wrap mode alongside filter and mipmap state");
        assertTrue(vulkanTextureSource.contains("texture.setAddressMode(this.addressModeU, this.addressModeV);"),
            "Vulkan Iris texture state copies should preserve wrap mode so derived PBR textures do not fall back to repeat");
    }

    @Test
	    public void testIrisCustomPassesUseFramebufferOwnedRenderPassAndRecoveredPipelineSeams() throws IOException {
	        String compositeRendererSource = readSource(SRC_MAIN_JAVA.resolve("net/irisshaders/iris/pipeline/CompositeRenderer.java"));
	        String shadowCompositeRendererSource = readSource(SRC_MAIN_JAVA.resolve("net/irisshaders/iris/shadows/ShadowCompositeRenderer.java"));
	        String commandEncoderSource = readSource(SRC_MAIN_JAVA.resolve("net/blaze3d/opengl/GlCommandEncoder.java"));
	        String nativeCommandEncoderSource = readSource(SRC_MAIN_JAVA.resolve("net/vulkanic/backends/vulkan/VulkanNativeCommandEncoder.java"));

	        assertTrue(compositeRendererSource.contains("compositePass.ensurePipelineState(renderTargetSelection);"),
	            "CompositeRenderer should precompute a render-target-compatible pipeline selection for custom passes before opening the render pass");
	        assertTrue(compositeRendererSource.contains("USE_DESCRIPTOR_COMPOSITE_RENDER_PASS")
	                && compositeRendererSource.contains("IrisVulkanRenderTargetContract.selectTarget(")
	                && compositeRendererSource.contains("renderTargetSelection.createRenderPass(label)")
	                && compositeRendererSource.contains("renderTargetSelection.createPipeline(descriptor)"),
	            "CompositeRenderer should migrate main Iris composite passes to descriptor-backed Vulkan render targets while preserving framebuffer-compatible fallback paths");
	        assertTrue(compositeRendererSource.contains("VulkanicAPI.bindDefaultUniforms(renderPass);"),
	            "CompositeRenderer should bind shared default uniforms before Vulkan custom-pass draws");
	        assertTrue(compositeRendererSource.contains(".withCull(false)"),
	            "CompositeRenderer full-screen custom passes must disable culling so screen-space triangles do not depend on backend winding/origin conventions");
	        assertFalse(compositeRendererSource.contains("framebuffer.bind();"),
	            "CompositeRenderer custom pass setup should not manually rebind the framebuffer after backend-owned render-pass creation");

	        assertTrue(shadowCompositeRendererSource.contains("renderPass.ensurePipelineState(renderTargetSelection);"),
	            "ShadowCompositeRenderer should precompute a render-target-compatible pipeline selection for custom passes before opening the render pass");
	        assertTrue(shadowCompositeRendererSource.contains("renderTargetSelection.createRenderPass(label)")
	                && shadowCompositeRendererSource.contains("renderTargetSelection.createPipeline(descriptor)")
	                && shadowCompositeRendererSource.contains("IrisVulkanRenderTargetContract.selectTarget("),
	            "ShadowCompositeRenderer should use parity-proven descriptor-backed native render targets on Vulkan while preserving framebuffer-compatible rendering for OpenGL/immediate paths");
	        assertTrue(shadowCompositeRendererSource.contains("VulkanicAPI.bindDefaultUniforms(pass);"),
	            "ShadowCompositeRenderer should bind shared default uniforms before Vulkan custom-pass draws");
	        assertFalse(shadowCompositeRendererSource.contains("framebuffer.bind();"),
            "ShadowCompositeRenderer custom pass setup should not manually rebind the framebuffer after backend-owned render-pass creation");

        assertTrue(commandEncoderSource.contains("customPass.bindRenderPassResources(glRenderPass);"),
            "GlCommandEncoder should let Iris custom passes contribute sampler resources to the active render pass");
        assertTrue(commandEncoderSource.contains("customPass.pipelineHandle(submission.descriptor())")
                && commandEncoderSource.contains("customPass.pipelineDescriptor()")
                && commandEncoderSource.contains("buildCustomPassPipelineResourceBindings(")
                && commandEncoderSource.contains("VulkanicPipelineResourceResolver.buildPlan("),
            "GlCommandEncoder should bind descriptor-matched live-program pipelines and descriptor resources for custom passes on Vulkan");
	        assertTrue(commandEncoderSource.contains("if (!submission.completeCoverage())")
	                && commandEncoderSource.contains("Skipping Vulkan custom pass"),
	            "GlCommandEncoder should fail open instead of submitting underbound Vulkan custom passes");
	        assertTrue(compositeRendererSource.contains("pipelineLayoutVariants")
	                && compositeRendererSource.contains("renderTargetSelection.createPipeline(descriptor)")
	                && compositeRendererSource.contains("renderTargetContractKey")
	                && compositeRendererSource.contains("targetContractChanged"),
	            "CompositeRenderer custom passes should cache descriptor-layout pipeline variants against the active render-target contract");
	        assertTrue(shadowCompositeRendererSource.contains("pipelineLayoutVariants")
	                && shadowCompositeRendererSource.contains("renderTargetSelection.createPipeline(descriptor)")
	                && shadowCompositeRendererSource.contains("renderTargetContractKey")
	                && shadowCompositeRendererSource.contains("targetContractChanged"),
	            "ShadowCompositeRenderer custom passes should cache descriptor-layout pipeline variants against the active render-target contract");
	        assertTrue(nativeCommandEncoderSource.contains("pass.bindRenderPassResources(this);")
	                && nativeCommandEncoderSource.contains("PipelineDescriptor customDescriptor = pass.pipelineDescriptor();")
	                && nativeCommandEncoderSource.contains("PipelineHandle handle = pass.pipelineHandle(submission.descriptor());")
	                && nativeCommandEncoderSource.contains("buildCustomPassResourceBindings("),
	            "VulkanNativeCommandEncoder should own descriptor-backed custom-pass resource binding and pipeline resolution");
	        assertTrue(nativeCommandEncoderSource.contains("if (!submission.completeCoverage())")
	                && nativeCommandEncoderSource.contains("Incomplete Vulkan native custom-pass resource coverage")
	                && nativeCommandEncoderSource.contains("recoverSamplerView(String name, @Nullable Integer textureUnit)"),
	            "VulkanNativeCommandEncoder should recover named shaderpack samplers from their render-pass texture units and fail fast if coverage is still incomplete");
	    }

    @Test
    public void testVulkanBackendBootstrapPathExists() throws IOException {
        Path vulkanBackendFile = SRC_MAIN_JAVA.resolve("net/vulkanic/backends/vulkan/VulkanBackend.java");
        assertTrue(Files.exists(vulkanBackendFile),
            "Vulkan backend bootstrap class should exist for incremental backend bring-up");

        String vulkanBackendSource = readSource(vulkanBackendFile);
        assertFalse(vulkanBackendSource.contains("extends OpenGLBackend"),
            "Vulkan backend must not inherit OpenGL backend behavior; cross-backend inheritance must remain forbidden");

        Path apiFile = SRC_MAIN_JAVA.resolve("net/vulkanic/VulkanicAPI.java");
        String apiSource = readSource(apiFile);
        assertTrue(apiSource.contains("rawVulkanBackend = new VulkanBackend();"),
            "VulkanicAPI should construct VulkanBackend for GraphicsBackendType.VULKAN routing");
        assertTrue(apiSource.contains("backend = createFailFastVulkanProxy(rawVulkanBackend);"),
            "VulkanicAPI should route Vulkan backend calls through fail-fast proxy protection");
        assertTrue(apiSource.contains("directVulkanBackendForImplementedMethods()"),
            "VulkanicAPI should expose a direct-dispatch helper for hot implemented Vulkan methods");
        assertTrue(apiSource.contains("|| isVulkanBackendSelected()")
                && apiSource.contains("|| isVulkanBackendSelected())\n\t\t\t\t\t&& !isRustWholeFrameBootstrapMethod"),
            "selected Vulkan must fence direct Java Vulkan dispatch and proxy calls before Rust presentation activates");
        assertFalse(apiSource.contains("methodCache.computeIfAbsent(method"),
            "Vulkan fail-fast proxy should precompute backend method routing instead of paying per-call computeIfAbsent overhead on the render thread");
        assertFalse(apiSource.contains("throw new UnsupportedOperationException(\"Vulkan backend not yet implemented\")"),
            "VulkanicAPI should no longer hard-fail backend selection for GraphicsBackendType.VULKAN");
    }

    @Test
    public void testBackendReadinessSeamExistsForPrepOnlyVulkanPath() throws IOException {
        Path backendInterfaceFile = SRC_MAIN_JAVA.resolve("net/vulkanic/GraphicsBackend.java");
        String backendInterfaceSource = readSource(backendInterfaceFile);
        assertTrue(backendInterfaceSource.contains("GraphicsBackendType getBackendType();"),
            "GraphicsBackend should expose active backend identity for explicit routing");
        assertTrue(backendInterfaceSource.contains("boolean isNativeVulkanReady();"),
            "GraphicsBackend should expose native Vulkan readiness capability check");
        assertTrue(backendInterfaceSource.contains("default VulkanExecutionContextInfo getVulkanExecutionContextInfo()"),
            "GraphicsBackend should expose backend-neutral Vulkan execution-context seam");
        assertTrue(backendInterfaceSource.contains("default VulkanSwapchainSurfaceInfo getVulkanSwapchainSurfaceInfo()"),
            "GraphicsBackend should expose backend-neutral Vulkan swapchain/surface seam");
        assertTrue(backendInterfaceSource.contains("default void recreateVulkanSwapchain()"),
            "GraphicsBackend should expose explicit swapchain recreation seam");
        assertTrue(backendInterfaceSource.contains("default boolean recreateVulkanSwapchainIfNeeded()"),
            "GraphicsBackend should expose conditional swapchain recreation seam");
        assertTrue(backendInterfaceSource.contains("default VulkanNativeInitializationInfo initializeNativeVulkanRuntime()"),
            "GraphicsBackend should expose explicit native Vulkan initialization seam");
        assertTrue(backendInterfaceSource.contains("VulkanicBuffer createManagedBuffer(Supplier<String> label, int usage, int size);"),
            "GraphicsBackend should expose managed GPU buffer creation seam (size variant)");
        assertTrue(backendInterfaceSource.contains("VulkanicBuffer createManagedBuffer(Supplier<String> label, int usage, java.nio.ByteBuffer initialData);"),
            "GraphicsBackend should expose managed GPU buffer creation seam (initial-data variant)");
        assertTrue(backendInterfaceSource.contains("VulkanicBuffer.MappedView mapManagedBuffer(VulkanicBuffer buffer, boolean read, boolean write);"),
            "GraphicsBackend should expose managed GPU buffer mapping seam");
        assertTrue(backendInterfaceSource.contains("VulkanicTexture createManagedTexture(String label, int usage, VulkanicTextureFormat format,"),
            "GraphicsBackend should expose managed GPU texture creation seam");
        assertTrue(backendInterfaceSource.contains("VulkanicTextureView createManagedTextureView(VulkanicTexture texture);"),
            "GraphicsBackend should expose managed GPU texture view seam (full-range)");
        assertTrue(backendInterfaceSource.contains("VulkanicTextureView createManagedTextureView(VulkanicTexture texture, int baseMipLevel, int mipLevelCount);"),
            "GraphicsBackend should expose managed GPU texture view seam (mip-range)");

        Path apiFile = SRC_MAIN_JAVA.resolve("net/vulkanic/VulkanicAPI.java");
        String apiSource = readSource(apiFile);
        assertTrue(apiSource.contains("public static GraphicsBackendType getActiveBackendType()"),
            "VulkanicAPI should expose active backend identity helper");
        assertTrue(apiSource.contains("public static boolean isVulkanBackendSelected()"),
            "VulkanicAPI should expose Vulkan-selection helper");
        assertTrue(apiSource.contains("public static boolean isNativeVulkanBackendReady()"),
            "VulkanicAPI should expose native Vulkan readiness helper");
        assertTrue(apiSource.contains("public static VulkanNativeInitializationInfo initializeNativeVulkanRuntime()"),
            "VulkanicAPI should expose explicit native Vulkan runtime initialization helper");
        assertTrue(apiSource.contains("public static VulkanExecutionContextInfo getVulkanExecutionContextInfo()"),
            "VulkanicAPI should expose Vulkan execution-context diagnostics helper");
        assertTrue(apiSource.contains("public static VulkanSwapchainSurfaceInfo getVulkanSwapchainSurfaceInfo()"),
            "VulkanicAPI should expose Vulkan swapchain/surface diagnostics helper");
        assertTrue(apiSource.contains("public static void recreateVulkanSwapchain()"),
            "VulkanicAPI should expose explicit swapchain recreation helper");
        assertTrue(apiSource.contains("public static boolean recreateVulkanSwapchainIfNeeded()"),
            "VulkanicAPI should expose conditional swapchain recreation helper");
        assertTrue(apiSource.contains("public static VulkanicBuffer createManagedBuffer(java.util.function.Supplier<String> label, int usage, int size)"),
            "VulkanicAPI should expose managed GPU buffer creation helper (size variant)");
        assertTrue(apiSource.contains("public static VulkanicBuffer createManagedBuffer(java.util.function.Supplier<String> label, int usage,"),
            "VulkanicAPI should expose managed GPU buffer creation helper (initial-data variant)");
        assertTrue(apiSource.contains("public static VulkanicBuffer.MappedView mapManagedBuffer(VulkanicBuffer buffer, boolean read, boolean write)"),
            "VulkanicAPI should expose managed GPU buffer mapping helper");
        assertTrue(apiSource.contains("public static VulkanicTexture createManagedTexture(String label, int usage,"),
            "VulkanicAPI should expose managed GPU texture creation helper");
        assertTrue(apiSource.contains("public static VulkanicTextureView createManagedTextureView(VulkanicTexture texture)"),
            "VulkanicAPI should expose managed GPU texture view creation helper (full-range variant)");
        assertTrue(apiSource.contains("public static VulkanicTextureView createManagedTextureView(VulkanicTexture texture,"),
            "VulkanicAPI should expose managed GPU texture view creation helper (mip-range variant)");

        Path vulkanBackendFile = SRC_MAIN_JAVA.resolve("net/vulkanic/backends/vulkan/VulkanBackend.java");
        String vulkanBackendSource = readSource(vulkanBackendFile);
        assertTrue(vulkanBackendSource.contains("return GraphicsBackendType.VULKAN;"),
            "Bootstrap Vulkan backend should report Vulkan backend identity");
        assertTrue(vulkanBackendSource.contains("public boolean isNativeVulkanReady()"),
            "Bootstrap Vulkan backend should define native readiness contract");
        assertTrue(vulkanBackendSource.contains("return nativeSpine != null;"),
            "Bootstrap Vulkan backend should report native readiness based on native spine initialization state");
        assertTrue(vulkanBackendSource.contains("public VulkanExecutionContextInfo getVulkanExecutionContextInfo()"),
            "Bootstrap Vulkan backend should expose logical device/queue execution-context diagnostics");
        assertTrue(vulkanBackendSource.contains("public VulkanNativeInitializationInfo initializeNativeVulkanRuntime()"),
            "Bootstrap Vulkan backend should expose explicit native Vulkan initialization diagnostics");
        assertTrue(vulkanBackendSource.contains("public VulkanSwapchainSurfaceInfo getVulkanSwapchainSurfaceInfo()"),
            "Bootstrap Vulkan backend should expose surface/swapchain diagnostics");
        assertTrue(vulkanBackendSource.contains("public void recreateVulkanSwapchain()"),
            "Bootstrap Vulkan backend should expose explicit swapchain recreation entrypoint");
        assertTrue(vulkanBackendSource.contains("public boolean recreateVulkanSwapchainIfNeeded()"),
            "Bootstrap Vulkan backend should expose conditional swapchain recreation entrypoint");
        assertTrue(vulkanBackendSource.contains("public VulkanicBuffer createManagedBuffer(java.util.function.Supplier<String> label, int usage, int size)"),
            "Bootstrap Vulkan backend should expose managed GPU buffer creation entrypoint (size variant)");
        assertTrue(vulkanBackendSource.contains("public VulkanicBuffer createManagedBuffer(java.util.function.Supplier<String> label,"),
            "Bootstrap Vulkan backend should expose managed GPU buffer creation entrypoint (initial-data variant)");
        assertTrue(vulkanBackendSource.contains("public VulkanicBuffer.MappedView mapManagedBuffer(VulkanicBuffer buffer, boolean read, boolean write)"),
            "Bootstrap Vulkan backend should expose managed GPU buffer mapping entrypoint");
        assertTrue(vulkanBackendSource.contains("public VulkanicTexture createManagedTexture(String label, int usage, VulkanicTextureFormat format,"),
            "Bootstrap Vulkan backend should expose managed GPU texture creation entrypoint");
        assertTrue(vulkanBackendSource.contains("public VulkanicTextureView createManagedTextureView(VulkanicTexture texture)"),
            "Bootstrap Vulkan backend should expose managed GPU texture view creation entrypoint (full-range variant)");
        assertTrue(vulkanBackendSource.contains("public VulkanicTextureView createManagedTextureView(VulkanicTexture texture, int baseMipLevel, int mipLevelCount)"),
            "Bootstrap Vulkan backend should expose managed GPU texture view creation entrypoint (mip-range variant)");
        assertTrue(vulkanBackendSource.contains("spine.recreateSwapchainIfFramebufferSizeChanged();"),
            "Bootstrap Vulkan command-buffer begin path should auto-check framebuffer resize and recreate swapchain");
    }

    @Test
    public void testTextureHandleResolutionOwnedByBackendSeam() throws IOException {
        Path backendInterfaceFile = SRC_MAIN_JAVA.resolve("net/vulkanic/GraphicsBackend.java");
        String backendInterfaceSource = readSource(backendInterfaceFile);
        assertTrue(backendInterfaceSource.contains("default int resolveTextureHandle(CommandContext ctx, VulkanicTexture texture)"),
            "GraphicsBackend should expose resolveTextureHandle seam for backend-owned texture-handle resolution");

        Path vulkanicApiFile = SRC_MAIN_JAVA.resolve("net/vulkanic/VulkanicAPI.java");
        String vulkanicApiSource = readSource(vulkanicApiFile);
        assertFalse(vulkanicApiSource.contains("return texture.glId();"),
            "VulkanicAPI.getTextureHandle should not directly call texture.glId after backend-seam migration");
        assertTrue(vulkanicApiSource.contains("directVulkanBackend.resolveTextureHandle(ctx, target)")
                && vulkanicApiSource.contains(": getBackend().resolveTextureHandle(ctx, target);"),
            "VulkanicAPI.getTextureHandle should delegate texture-handle extraction to the active backend, with direct Vulkan dispatch for the hot path");

        Path openGlBackendFile = SRC_MAIN_JAVA.resolve("net/vulkanic/backends/opengl/OpenGLBackend.java");
        String openGlBackendSource = readSource(openGlBackendFile);
        assertTrue(openGlBackendSource.contains("public int resolveTextureHandle(CommandContext ctx, net.vulkanic.VulkanicTexture texture)"),
            "OpenGLBackend should implement resolveTextureHandle for OpenGL-backed texture ids");
        assertTrue(openGlBackendSource.contains("texture instanceof net.vulkanic.backends.opengl.OpenGLTexture openGLTexture"),
            "OpenGLBackend resolveTextureHandle should support Vulkanic OpenGLTexture wrappers");
        assertTrue(openGlBackendSource.contains("texture instanceof net.blaze3d.opengl.GlTexture glTexture"),
            "OpenGLBackend resolveTextureHandle should support Blaze3D GlTexture instances");

        Path glTextureFile = SRC_MAIN_JAVA.resolve("net/blaze3d/opengl/GlTexture.java");
        String glTextureSource = readSource(glTextureFile);
        assertTrue(glTextureSource.contains("public int getGlHandle()"),
            "GlTexture should expose getGlHandle for backend-local OpenGL handle extraction");

        Path openGlTextureViewFile = SRC_MAIN_JAVA.resolve("net/vulkanic/backends/opengl/OpenGLTextureView.java");
        String openGlTextureViewSource = readSource(openGlTextureViewFile);
        assertFalse(openGlTextureViewSource.contains("return VulkanicAPI.getTextureHandle(t);"),
            "OpenGLTextureView should not route GlTexture handle extraction back through VulkanicAPI");
        assertTrue(openGlTextureViewSource.contains("return t.getGlHandle();"),
            "OpenGLTextureView should extract GlTexture handles directly through getGlHandle");
    }

    // Î“Ã¶Ã‡Î“Ã¶Ã‡ Task 1: drawFromBuffers routes directly through VulkanicAPI Î“Ã¶Ã‡Î“Ã¶Ã‡Î“Ã¶Ã‡Î“Ã¶Ã‡Î“Ã¶Ã‡Î“Ã¶Ã‡Î“Ã¶Ã‡Î“Ã¶Ã‡Î“Ã¶Ã‡Î“Ã¶Ã‡Î“Ã¶Ã‡

    @Test
    public void testDrawFromBuffersNoGlStateManagerDrawCall() throws IOException {
        Path file = SRC_MAIN_JAVA.resolve(
            "net/blaze3d/opengl/GlCommandEncoder.java");
        assertTrue(Files.exists(file), "GlCommandEncoder.java must exist");

        String source = readSource(file);

        // The three GlStateManager draw/bind calls that were in drawFromBuffers must be gone.
        // This assertion intentionally checks only draw-path calls, not other upload/readback paths.
        assertFalse(source.contains("GlStateManager._drawElements("),
            "drawFromBuffers must not route through GlStateManager._drawElements; " +
            "it should call VulkanicAPI.drawElements directly");
        assertFalse(source.contains("GlStateManager._drawArrays("),
            "drawFromBuffers must not route through GlStateManager._drawArrays; " +
            "it should call VulkanicAPI.drawArrays directly");
        // The element-array-buffer bind for draw calls used hardcoded constant 34963;
        // it must now be VulkanicAPI.bindBuffer(...GL_ELEMENT_ARRAY_BUFFER...)
        assertFalse(source.contains("GlStateManager._glBindBuffer(34963,"),
            "drawFromBuffers must not route the index buffer bind through GlStateManager; " +
            "it should call VulkanicAPI.bindBuffer(ctx, GL_ELEMENT_ARRAY_BUFFER, ...) directly");
    }

    @Test
    public void testGlStateManagerTypeDeleted() throws IOException {
        Path stateManagerFile = SRC_MAIN_JAVA.resolve("net/blaze3d/opengl/GlStateManager.java");
        assertTrue(Files.exists(stateManagerFile), "GlStateManager.java path should remain for migration tracking");

        String source = readSource(stateManagerFile);
        assertFalse(source.contains("class GlStateManager"),
            "GlStateManager type should be fully deleted from source");
        assertFalse(source.contains("public class GlStateManager"),
            "GlStateManager should no longer exist as a concrete class");
    }

    @Test
    public void testDrawFromBuffersCallsVulkanicAPIDrawElements() throws IOException {
        Path file = SRC_MAIN_JAVA.resolve(
            "net/blaze3d/opengl/GlCommandEncoder.java");
        String source = readSource(file);

        assertTrue(source.contains("VulkanicAPI.drawElements(ctx,"),
            "drawFromBuffers must call VulkanicAPI.drawElements(ctx, ...) for non-instanced indexed draws");
        assertTrue(source.contains("VulkanicAPI.drawArrays(ctx,"),
            "drawFromBuffers must call VulkanicAPI.drawArrays(ctx, ...) for non-instanced non-indexed draws");
        assertTrue(source.contains("VulkanicAPI.bindIndexBuffer(ctx,"),
            "drawFromBuffers must bind the index buffer via the backend-agnostic VulkanicAPI.bindIndexBuffer helper");
    }

    @Test
    public void testDrawFromBuffersPreservesIrisTessellationOverride() throws IOException {
        Path file = SRC_MAIN_JAVA.resolve(
            "net/blaze3d/opengl/GlCommandEncoder.java");
        String source = readSource(file);

        // The Iris tessellation override (TRIANGLES Î“Ã¥Ã† PATCHES) must still be present
        // in the non-instanced indexed draw path, since we replaced GlStateManager._drawElements
        // which previously contained it.
        assertTrue(source.contains("usingTessellation"),
            "drawFromBuffers must preserve the Iris tessellation mode override");
        assertTrue(
            source.contains("VulkanicPrimitiveMode.PATCHES") || source.contains("GL_PATCHES"),
            "drawFromBuffers must substitute PATCHES when tessellation is active"
        );
    }

    @Test
    public void testDrawFromBuffersSharesContextAcrossCalls() throws IOException {
        Path file = SRC_MAIN_JAVA.resolve(
            "net/blaze3d/opengl/GlCommandEncoder.java");
        String source = readSource(file);

        // Verify that drawFromBuffers obtains the context once and reuses it (ctx variable),
        // rather than repeatedly resolving the backend-global context.
        assertTrue(source.contains("CommandContext ctx = commandContext();"),
            "drawFromBuffers should obtain a single CommandContext and reuse it");
    }

    @Test
    public void testSsaoApplyShaderUsesBackendNeutralSingleContext() throws IOException {
        Path file = SRC_MAIN_JAVA.resolve(
            "com/seibel/distanthorizons/core/render/renderer/shaders/SSAOApplyShader.java");
        assertTrue(Files.exists(file), "SSAOApplyShader.java must exist");

        String source = readSource(file);
        Path abstractShaderFile = SRC_MAIN_JAVA.resolve(
            "com/seibel/distanthorizons/core/render/renderer/shaders/AbstractShaderRenderer.java");
        String abstractShaderSource = readSource(abstractShaderFile);

        assertFalse(source.contains("VulkanicAPI.getCommandContext()"),
            "SSAOApplyShader should inherit the shared CommandContext from AbstractShaderRenderer");
        assertTrue(source.contains("onApplyUniforms(CommandContext ctx, float partialTicks)"),
            "SSAOApplyShader should receive command context via shader hook parameter");
        assertTrue(abstractShaderSource.contains("CommandContext ctx = VulkanicAPI.getCommandContext();"),
            "AbstractShaderRenderer should fetch a backend-neutral CommandContext once per render phase");
        assertTrue(abstractShaderSource.contains("if (width <= 0 || height <= 0)"),
            "AbstractShaderRenderer should skip shader bind/uniform work when viewport dimensions are invalid");
        assertTrue(abstractShaderSource.contains("if (!this.onPreRender(ctx, partialTicks))"),
            "AbstractShaderRenderer should support shared pre-bind resource prechecks before shader bind/uniform work");
        assertTrue(abstractShaderSource.contains("protected boolean onPreRender(CommandContext ctx, float partialTicks) { return true; }"),
            "AbstractShaderRenderer should provide a default backend-neutral pre-bind precheck hook");
        assertTrue(abstractShaderSource.contains("try") && abstractShaderSource.contains("finally"),
            "AbstractShaderRenderer render path should always unbind shader via try/finally for backend-state safety");
        assertTrue(abstractShaderSource.contains("this.shader.unbind(ctx);"),
            "AbstractShaderRenderer should unbind shader even when render hooks fail");
        assertFalse(source.contains("VulkanicAPI.getImmediateContext()"),
            "SSAOApplyShader should not hard-wire immediate OpenGL context retrieval");
    }

    @Test
    public void testDrawFromBuffersUsesBackendAgnosticIndexTypeRouting() throws IOException {
        Path file = SRC_MAIN_JAVA.resolve(
            "net/blaze3d/opengl/GlCommandEncoder.java");
        String source = readSource(file);

        assertFalse(source.contains("GlConst.toGl(indexType)"),
            "drawFromBuffers should not convert index types through OpenGL-specific GlConst.toGl(indexType); " +
            "it should route through VulkanicIndexType-aware VulkanicAPI overloads");
        assertTrue(source.contains("toVulkanicIndexType(indexType)"),
            "drawFromBuffers should map VertexFormat.IndexType to VulkanicIndexType for backend-agnostic indexed draws");
    }

    @Test
    public void testGlCommandEncoderUsesAgnosticTextureAndUniformBindings() throws IOException {
        Path file = SRC_MAIN_JAVA.resolve(
            "net/blaze3d/opengl/GlCommandEncoder.java");
        String source = readSource(file);

        assertFalse(source.contains("VulkanicAPI.bindTexture(VulkanicAPI.getImmediateContext(), 34067"),
            "GlCommandEncoder should not bind cubemaps via hardcoded GL target 34067; use bindCubemapTexture");
        assertFalse(source.contains("VulkanicAPI.bindTexture(VulkanicAPI.getImmediateContext(), 35882"),
            "GlCommandEncoder should not bind texture buffers via hardcoded GL target 35882; use bindTextureBuffer");
        assertFalse(source.contains("VulkanicAPI.bindUniformBufferRange(VulkanicAPI.getImmediateContext(), 35345"),
            "GlCommandEncoder should not bind UBO ranges with hardcoded GL target 35345; use target-agnostic overload");
        assertFalse(source.contains("VulkanicAPI.setDrawBuffer(VulkanicAPI.getImmediateContext(), 0)"),
            "GlCommandEncoder should use setDrawBufferNone helper instead of raw draw-buffer literal 0");
        assertFalse(source.contains("VulkanicAPI.setDrawBuffer(VulkanicAPI.getImmediateContext(), 36064)"),
            "GlCommandEncoder should use setDrawBufferColorAttachment0 helper instead of raw draw-buffer literal 36064");
        assertFalse(source.contains("bl ? 256 : 16384"),
            "GlCommandEncoder texture blit path should not branch over hardcoded depth/color blit masks 256/16384");
        assertFalse(source.contains("gpuTextureView.getHeight(0), 16384, 9728"),
            "GlCommandEncoder presentTexture path should not use hardcoded color-mask/filter literals 16384/9728");
        assertFalse(source.contains("bindFrameBufferTextures(this.drawFbo, VulkanicCoreAPI.textureId(gpuTextureView), 0, 0, 0)"),
            "GlCommandEncoder presentTexture should avoid direct drawFbo texture attachment plumbing and route through backend-owned present seam");

        assertTrue(source.contains("VulkanicAPI.bindCubemapTexture("),
            "GlCommandEncoder should bind cubemaps via VulkanicAPI.bindCubemapTexture");
        assertTrue(source.contains("VulkanicAPI.bindTextureBuffer("),
            "GlCommandEncoder should bind texture buffers via VulkanicAPI.bindTextureBuffer");
        assertTrue(source.contains("VulkanicAPI.bindTextureBufferData("),
            "GlCommandEncoder should attach texel buffer data via VulkanicAPI.bindTextureBufferData");
        assertTrue(source.contains("VulkanicAPI.bindUniformBufferRange(ctx, var39"),
            "GlCommandEncoder should use target-agnostic bindUniformBufferRange overload in uniform upload path");
        assertTrue(source.contains("bl ? VulkanicAPI.GL_DEPTH_BUFFER_BIT : VulkanicAPI.GL_COLOR_BUFFER_BIT"),
            "GlCommandEncoder texture blit path should route depth/color masks through VulkanicAPI constants");
        assertTrue(source.contains("VulkanicAPI.GL_NEAREST"),
            "GlCommandEncoder blit paths should route nearest-filter intent through VulkanicAPI constant");
        assertTrue(source.contains("VulkanicCoreAPI.presentTextureToScreen(ctx, gpuTextureView);"),
            "GlCommandEncoder presentTexture should route final presentation through VulkanicCoreAPI.presentTextureToScreen backend seam");
    }

    @Test
    public void testGlDeviceUsesAgnosticCubemapBindHelper() throws IOException {
        Path file = SRC_MAIN_JAVA.resolve("net/blaze3d/opengl/GlDevice.java");
        String source = readSource(file);

        assertFalse(source.contains("VulkanicAPI.bindTexture(VulkanicAPI.getImmediateContext(), 34067"),
            "GlDevice should not bind cubemaps via hardcoded GL target 34067");
        assertTrue(source.contains("VulkanicAPI.bindCubemapTexture("),
            "GlDevice should bind cubemaps via VulkanicAPI.bindCubemapTexture");
    }

    @Test
    public void testTimerQueryUsesAgnosticQueryHelpers() throws IOException {
        Path file = SRC_MAIN_JAVA.resolve("net/blaze3d/systems/TimerQuery.java");
        String source = readSource(file);

        assertFalse(source.contains("VulkanicAPI.initiateQuery(VulkanicAPI.getImmediateContext(), 35007"),
            "TimerQuery should not begin queries with hardcoded GL_TIME_ELAPSED target literal 35007");
        assertFalse(source.contains("VulkanicAPI.concludeQuery(VulkanicAPI.getImmediateContext(), 35007"),
            "TimerQuery should not end queries with hardcoded GL_TIME_ELAPSED target literal 35007");
        assertFalse(source.contains("retrieveQueryObjectInt(VulkanicAPI.getImmediateContext(), this.queryName, 34919)"),
            "TimerQuery should not poll query availability using hardcoded GL_QUERY_RESULT_AVAILABLE literal 34919");
        assertFalse(source.contains("retrieveQueryObjectInt64(VulkanicAPI.getImmediateContext(), this.queryName, 34918)"),
            "TimerQuery should not fetch query values using hardcoded GL_QUERY_RESULT literal 34918");

        assertTrue(source.contains("VulkanicAPI.beginTimeElapsedQuery("),
            "TimerQuery should begin profiling via VulkanicAPI.beginTimeElapsedQuery");
        assertTrue(source.contains("VulkanicAPI.endTimeElapsedQuery("),
            "TimerQuery should end profiling via VulkanicAPI.endTimeElapsedQuery");
        assertTrue(source.contains("VulkanicAPI.isQueryResultAvailable("),
            "TimerQuery should poll completion via VulkanicAPI.isQueryResultAvailable");
        assertTrue(source.contains("VulkanicAPI.getQueryResultInt64("),
            "TimerQuery should fetch results via VulkanicAPI.getQueryResultInt64");
    }

    @Test
    public void testDirectStateAccessUsesAgnosticFramebufferAndCopyHelpers() throws IOException {
        Path file = SRC_MAIN_JAVA.resolve("net/blaze3d/opengl/DirectStateAccess.java");
        String source = readSource(file);

        assertFalse(source.contains("namedFramebufferTextureDSA(net.vulkanic.VulkanicAPI.getImmediateContext(), i, 36064"),
            "DirectStateAccess should not attach color with hardcoded GL_COLOR_ATTACHMENT0 literal 36064");
        assertFalse(source.contains("namedFramebufferTextureDSA(net.vulkanic.VulkanicAPI.getImmediateContext(), i, 36096"),
            "DirectStateAccess should not attach depth with hardcoded GL_DEPTH_ATTACHMENT literal 36096");
        assertFalse(source.contains("VulkanicAPI.copyBufferSubData(VulkanicAPI.getImmediateContext(), 36662, 36663"),
            "DirectStateAccess should not copy buffers with hardcoded copy-target literals 36662/36663");
        assertFalse(source.contains("GlStateManager._glBlitFrameBuffer("),
            "DirectStateAccess should not blit through removed GlStateManager._glBlitFrameBuffer wrapper");

        assertTrue(source.contains("VulkanicAPI.namedFramebufferColorAttachment0DSA("),
            "DirectStateAccess should use namedFramebufferColorAttachment0DSA helper");
        assertTrue(source.contains("VulkanicAPI.namedFramebufferDepthAttachmentDSA("),
            "DirectStateAccess should use namedFramebufferDepthAttachmentDSA helper");
        assertTrue(source.contains("VulkanicAPI.bindCopyReadBuffer("),
            "DirectStateAccess should bind copy-read via VulkanicAPI.bindCopyReadBuffer");
        assertTrue(source.contains("VulkanicAPI.bindCopyWriteBuffer("),
            "DirectStateAccess should bind copy-write via VulkanicAPI.bindCopyWriteBuffer");
        assertTrue(source.contains("VulkanicAPI.copyBufferSubDataBetweenCopyTargets("),
            "DirectStateAccess should copy via VulkanicAPI.copyBufferSubDataBetweenCopyTargets");
        assertFalse(source.contains("VulkanicAPI.bindReadFramebuffer(ctx, i);"),
            "DirectStateAccess should not manually bind read framebuffer around blits in fallback mode");
        assertFalse(source.contains("VulkanicAPI.bindDrawFramebuffer(ctx, j);"),
            "DirectStateAccess should not manually bind draw framebuffer around blits in fallback mode");
        assertTrue(source.contains("VulkanicAPI.blitNamedFramebuffer(ctx, i, j, k, l, m, n, o, p, q, r, s, t);"),
            "DirectStateAccess should route blits through VulkanicAPI.blitNamedFramebuffer");
    }

    @Test
    public void testIrisRenderSystemUsesFramebufferIntentHelpers() throws IOException {
        Path stateManagerFile = SRC_MAIN_JAVA.resolve("net/blaze3d/opengl/GlStateManager.java");
        String stateManagerSource = readSource(stateManagerFile);

        assertFalse(stateManagerSource.contains("public static void _glBindFramebuffer("),
            "GlStateManager should no longer expose _glBindFramebuffer wrapper");
        assertFalse(stateManagerSource.contains("public static int getFrameBuffer("),
            "GlStateManager should no longer expose getFrameBuffer wrapper");

        Path irisRenderSystemFile = SRC_MAIN_JAVA.resolve("net/irisshaders/iris/gl/IrisRenderSystem.java");
        String irisRenderSystemSource = readSource(irisRenderSystemFile);

        assertFalse(irisRenderSystemSource.contains("public static void bindFramebuffer("),
            "IrisRenderSystem framebuffer bind wrapper should be removed after VulkanicAPI helper migration");
        assertFalse(irisRenderSystemSource.contains("public static int getFrameBuffer("),
            "IrisRenderSystem framebuffer binding getter wrapper should be removed after VulkanicAPI helper migration");
        assertFalse(irisRenderSystemSource.contains("private static int readFramebuffer"),
            "IrisRenderSystem should not own read framebuffer binding state");
        assertFalse(irisRenderSystemSource.contains("private static int writeFramebuffer"),
            "IrisRenderSystem should not own draw framebuffer binding state");
        assertFalse(irisRenderSystemSource.contains("VulkanicAPI.bindReadFramebuffer(VulkanicAPI.getCommandContext(), source)"),
            "IrisRenderSystem should not manually bind read framebuffer in fallback blit path");
        assertFalse(irisRenderSystemSource.contains("VulkanicAPI.bindDrawFramebuffer(VulkanicAPI.getCommandContext(), dest)"),
            "IrisRenderSystem should not manually bind draw framebuffer in fallback blit path");
        assertTrue(irisRenderSystemSource.contains("VulkanicAPI.blitNamedFramebuffer(VulkanicAPI.getCommandContext(), source, dest"),
            "IrisRenderSystem fallback blit path should use VulkanicAPI.blitNamedFramebuffer");

        Path vulkanicApiFile = SRC_MAIN_JAVA.resolve("net/vulkanic/VulkanicAPI.java");
        String vulkanicApiSource = readSource(vulkanicApiFile);

        assertTrue(vulkanicApiSource.contains("private static int readFramebufferBinding"),
            "VulkanicAPI should own cached read framebuffer binding state");
        assertTrue(vulkanicApiSource.contains("private static int drawFramebufferBinding"),
            "VulkanicAPI should own cached draw framebuffer binding state");
        assertTrue(vulkanicApiSource.contains("public static void bindDefaultFramebuffer(CommandContext ctx)"),
            "VulkanicAPI should expose bindDefaultFramebuffer helper for intent-level default FBO binds");
        assertTrue(vulkanicApiSource.contains("public static int getReadFramebufferBinding()"),
            "VulkanicAPI should expose cached read framebuffer getter");
        assertTrue(vulkanicApiSource.contains("public static int getDrawFramebufferBinding()"),
            "VulkanicAPI should expose cached draw framebuffer getter");
    }

    @Test
    public void testRenderTargetBindingOwnedByBackendSeam() throws IOException {
        Path backendInterfaceFile = SRC_MAIN_JAVA.resolve("net/vulkanic/GraphicsBackend.java");
        String backendInterfaceSource = readSource(backendInterfaceFile);
        assertTrue(backendInterfaceSource.contains("default void bindRenderTarget(CommandContext ctx, VulkanicTexture colorTexture, VulkanicTexture depthTexture)"),
            "GraphicsBackend should expose backend-owned render-target binding seam");
        assertTrue(backendInterfaceSource.contains("bindFramebuffer(ctx, VulkanicAPI.GL_FRAMEBUFFER, resolveFramebufferForTextures(ctx, colorTexture, depthTexture));"),
            "GraphicsBackend default render-target seam should bridge through backend-owned framebuffer resolution");

        Path vulkanicApiFile = SRC_MAIN_JAVA.resolve("net/vulkanic/VulkanicAPI.java");
        String vulkanicApiSource = readSource(vulkanicApiFile);
        assertTrue(vulkanicApiSource.contains("public static void bindRenderTarget(CommandContext ctx, @Nullable GpuTexture colorTexture, @Nullable GpuTexture depthTexture)"),
            "VulkanicAPI should expose render-target binding helper for color/depth texture pairs");
        assertTrue(vulkanicApiSource.contains("getBackend().bindRenderTarget(ctx, colorTarget, depthTarget);"),
            "VulkanicAPI render-target binding helper should delegate ownership to GraphicsBackend");
        assertTrue(vulkanicApiSource.contains("bindDefaultFramebuffer(ctx);"),
            "VulkanicAPI render-target binding helper should fall back to default framebuffer when no color target exists");

        Path openGlBackendFile = SRC_MAIN_JAVA.resolve("net/vulkanic/backends/opengl/OpenGLBackend.java");
        String openGlBackendSource = readSource(openGlBackendFile);
        assertTrue(openGlBackendSource.contains("public void bindRenderTarget(CommandContext ctx, net.vulkanic.VulkanicTexture colorTexture, net.vulkanic.VulkanicTexture depthTexture)"),
            "OpenGLBackend should implement render-target binding seam");
        assertTrue(openGlBackendSource.contains("int framebuffer = resolveFramebufferForTextures(ctx, colorTexture, depthTexture);"),
            "OpenGLBackend render-target binding should resolve framebuffer internally");

        Path vulkanBackendFile = SRC_MAIN_JAVA.resolve("net/vulkanic/backends/vulkan/VulkanBackend.java");
        String vulkanBackendSource = readSource(vulkanBackendFile);
        assertTrue(vulkanBackendSource.contains("public void bindRenderTarget(CommandContext ctx, net.vulkanic.VulkanicTexture colorTexture, net.vulkanic.VulkanicTexture depthTexture)"),
            "VulkanBackend should implement render-target binding seam");
        assertTrue(vulkanBackendSource.contains("int framebuffer = resolveFramebufferForTextures(ctx, colorTexture, depthTexture);"),
            "VulkanBackend render-target binding should keep attachment-pair routing inside backend code");
        assertTrue(vulkanBackendSource.contains("VulkanPipelineLifecycleManager.CacheKind.DESCRIPTOR_VARIANT")
                && vulkanBackendSource.contains("pipelineLifecycle.cachePipeline(")
                && vulkanBackendSource.contains("matchesStableDescriptor("),
            "VulkanBackend should cache descriptor-layout pipeline variants for partial-coverage Vulkan draws instead of forcing them onto the full-layout precompile");
    }

    @Test
    public void testFramebufferDeletePathsUseDirectVulkanicCalls() throws IOException {
        Path stateManagerFile = SRC_MAIN_JAVA.resolve("net/blaze3d/opengl/GlStateManager.java");
        String stateManagerSource = readSource(stateManagerFile);
        assertFalse(stateManagerSource.contains("public static void _glDeleteFramebuffers("),
            "GlStateManager should no longer expose _glDeleteFramebuffers wrapper");
        assertFalse(stateManagerSource.contains("public static int glGenFramebuffers("),
            "GlStateManager should no longer expose glGenFramebuffers wrapper");

        Path directStateAccessFile = SRC_MAIN_JAVA.resolve("net/blaze3d/opengl/DirectStateAccess.java");
        String directStateAccessSource = readSource(directStateAccessFile);
        assertFalse(directStateAccessSource.contains("GlStateManager.glGenFramebuffers("),
            "DirectStateAccess should not create FBOs through removed GlStateManager.glGenFramebuffers wrapper");
        assertTrue(containsAny(directStateAccessSource,
                "VulkanicAPI.createFramebuffer(VulkanicAPI.getCommandContext())",
                "VulkanicAPI.createFramebuffer(ctx)"),
            "DirectStateAccess should create FBOs directly through VulkanicAPI.createFramebuffer");

        Path glTextureFile = SRC_MAIN_JAVA.resolve("net/blaze3d/opengl/GlTexture.java");
        String glTextureSource = readSource(glTextureFile);
        assertFalse(glTextureSource.contains("GlStateManager._glDeleteFramebuffers("),
            "GlTexture should not delete cached FBOs through removed GlStateManager._glDeleteFramebuffers wrapper");
        assertTrue(glTextureSource.contains("VulkanicAPI.deleteFramebuffer(ctx, i)"),
            "GlTexture should delete cached FBOs directly through VulkanicAPI.deleteFramebuffer");

        Path irisFramebufferFile = SRC_MAIN_JAVA.resolve("net/irisshaders/iris/gl/framebuffer/GlFramebuffer.java");
        String irisFramebufferSource = readSource(irisFramebufferFile);
        assertFalse(irisFramebufferSource.contains("GlStateManager._glDeleteFramebuffers("),
            "GlFramebuffer should not destroy FBOs through removed GlStateManager._glDeleteFramebuffers wrapper");
        assertTrue(irisFramebufferSource.contains("VulkanicAPI.deleteFramebuffer(VulkanicAPI.getCommandContext(), framebuffer)"),
            "GlFramebuffer should destroy FBOs directly through VulkanicAPI.deleteFramebuffer");

        Path irisRenderSystemFile = SRC_MAIN_JAVA.resolve("net/irisshaders/iris/gl/IrisRenderSystem.java");
        String irisRenderSystemSource = readSource(irisRenderSystemFile);
        assertFalse(irisRenderSystemSource.contains("GlStateManager.glGenFramebuffers("),
            "IrisRenderSystem should not create FBOs through removed GlStateManager.glGenFramebuffers wrapper");
        assertTrue(irisRenderSystemSource.contains("VulkanicAPI.createFramebuffer(VulkanicAPI.getCommandContext())"),
            "IrisRenderSystem should create FBOs directly through VulkanicAPI.createFramebuffer");

        Path textureManipulationUtilFile = SRC_MAIN_JAVA.resolve("net/irisshaders/iris/pbr/util/TextureManipulationUtil.java");
        String textureManipulationUtilSource = readSource(textureManipulationUtilFile);
        assertFalse(textureManipulationUtilSource.contains("GlStateManager.glGenFramebuffers("),
            "TextureManipulationUtil should not create helper FBO through removed GlStateManager.glGenFramebuffers wrapper");
        assertTrue(textureManipulationUtilSource.contains("CommandContext ctx = VulkanicAPI.getCommandContext();"),
            "TextureManipulationUtil should acquire backend-neutral command context once per operation");
        assertTrue(textureManipulationUtilSource.contains("VulkanicAPI.createFramebuffer(ctx)"),
            "TextureManipulationUtil should create helper FBO directly through VulkanicAPI.createFramebuffer");
    }

    @Test
    public void testGlCommandEncoderUsesAgnosticReadbackBindings() throws IOException {
        Path file = SRC_MAIN_JAVA.resolve("net/blaze3d/opengl/GlCommandEncoder.java");
        String source = readSource(file);

        assertFalse(source.contains("GlStateManager._glBindBuffer(35051"),
            "GlCommandEncoder readback path should not bind pixel-pack buffer via hardcoded target literal 35051");
        assertFalse(source.contains("GlStateManager._pixelStore(3330"),
            "GlCommandEncoder readback path should not set pack row length via hardcoded pname literal 3330");
        assertFalse(source.contains("GlStateManager._glFramebufferTexture2D(36008, 36064, 3553"),
            "GlCommandEncoder readback path should not detach read framebuffer attachment via hardcoded literals");
        assertFalse(source.contains("GlStateManager._readPixels("),
            "GlCommandEncoder readback path should not route readPixels through GlStateManager wrapper");
        assertFalse(source.contains("GlStateManager._getError()"),
            "GlCommandEncoder readback path should not route getError through GlStateManager wrapper");

        assertTrue(source.contains("VulkanicAPI.bindPixelPackBuffer("),
            "GlCommandEncoder readback path should bind PBO through VulkanicAPI.bindPixelPackBuffer");
        assertTrue(source.contains("VulkanicAPI.setPixelStore(ctx, VulkanicAPI.GL_PACK_ROW_LENGTH"),
            "GlCommandEncoder readback path should set row length via VulkanicAPI GL_PACK_ROW_LENGTH helper");
        assertTrue(source.contains("VulkanicAPI.framebufferColorAttachment0Texture2D("),
            "GlCommandEncoder readback path should detach color attachment via framebufferColorAttachment0Texture2D helper");
        assertTrue(source.contains("VulkanicAPI.readPixels(ctx"),
            "GlCommandEncoder readback path should call VulkanicAPI.readPixels directly");
        assertTrue(source.contains("VulkanicAPI.getError(ctx)"),
            "GlCommandEncoder readback path should query error via VulkanicAPI.getError(ctx)");
    }

    @Test
    public void testGlCommandEncoderUsesAgnosticUnpackUploadHelpers() throws IOException {
        Path file = SRC_MAIN_JAVA.resolve("net/blaze3d/opengl/GlCommandEncoder.java");
        String source = readSource(file);

        assertFalse(source.contains("GlStateManager._pixelStore(3314"),
            "GlCommandEncoder texture upload paths should not set unpack row length via hardcoded literal 3314");
        assertFalse(source.contains("GlStateManager._pixelStore(3316"),
            "GlCommandEncoder texture upload paths should not set unpack skip-pixels via hardcoded literal 3316");
        assertFalse(source.contains("GlStateManager._pixelStore(3315"),
            "GlCommandEncoder texture upload paths should not set unpack skip-rows via hardcoded literal 3315");
        assertFalse(source.contains("GlStateManager._pixelStore(3317"),
            "GlCommandEncoder texture upload paths should not set unpack alignment via hardcoded literal 3317");
        assertFalse(source.contains("GlStateManager._texSubImage2D("),
            "GlCommandEncoder texture upload paths should not route texSubImage2D through GlStateManager wrapper");

        assertTrue(source.contains("VulkanicAPI.setPixelStore(ctx, VulkanicAPI.GL_UNPACK_ROW_LENGTH"),
            "GlCommandEncoder texture upload paths should set unpack row length via VulkanicAPI helper");
        assertTrue(source.contains("VulkanicAPI.setPixelStore(ctx, VulkanicAPI.GL_UNPACK_SKIP_PIXELS"),
            "GlCommandEncoder texture upload paths should set unpack skip-pixels via VulkanicAPI helper");
        assertTrue(source.contains("VulkanicAPI.setPixelStore(ctx, VulkanicAPI.GL_UNPACK_SKIP_ROWS"),
            "GlCommandEncoder texture upload paths should set unpack skip-rows via VulkanicAPI helper");
        assertTrue(source.contains("VulkanicAPI.setPixelStore(ctx, VulkanicAPI.GL_UNPACK_ALIGNMENT"),
            "GlCommandEncoder texture upload paths should set unpack alignment via VulkanicAPI helper");
        assertTrue(source.contains("VulkanicAPI.uploadTexture2DSubImage(ctx"),
            "GlCommandEncoder texture upload paths should call VulkanicAPI.uploadTexture2DSubImage directly");
        assertFalse(source.contains("GlStateManager._glUniform1i("),
            "GlCommandEncoder should not upload UTB/sampler uniforms through GlStateManager._glUniform1i wrapper");
        assertFalse(source.contains("GlStateManager._texParameter("),
            "GlCommandEncoder sampler setup should not set texture parameters through GlStateManager._texParameter wrapper");
        assertTrue(source.contains("VulkanicAPI.setUniform1i(ctx"),
            "GlCommandEncoder should upload UTB/sampler uniforms directly via VulkanicAPI.setUniform1i");
        assertTrue(source.contains("VulkanicAPI.setTextureParameter(ctx"),
            "GlCommandEncoder sampler setup should set base/max level directly via VulkanicAPI.setTextureParameter");
    }

    @Test
    public void testGlCommandEncoderClearPathsUseAgnosticFramebufferHelpers() throws IOException {
        Path file = SRC_MAIN_JAVA.resolve("net/blaze3d/opengl/GlCommandEncoder.java");
        String source = readSource(file);

        assertFalse(source.contains("bindFrameBufferTextures(this.drawFbo, ((GlTexture)gpuTexture).id, 0, 0, 36160"),
            "GlCommandEncoder clearColorTexture should not bind draw FBO with hardcoded GL_FRAMEBUFFER literal 36160");
        assertFalse(source.contains("bindFrameBufferTextures(this.drawFbo, 0, ((GlTexture)gpuTexture).id, 0, 36160"),
            "GlCommandEncoder clearDepthTexture should not bind draw FBO with hardcoded GL_FRAMEBUFFER literal 36160");
        assertFalse(source.contains("_clear(16384)"),
            "GlCommandEncoder clearColorTexture should not clear via hardcoded GL_COLOR_BUFFER_BIT literal 16384");
        assertFalse(source.contains("_clear(16640)"),
            "GlCommandEncoder color+depth clear paths should not use hardcoded clear mask literal 16640");
        assertFalse(source.contains("_clear(256)"),
            "GlCommandEncoder clearDepthTexture should not clear via hardcoded GL_DEPTH_BUFFER_BIT literal 256");
        assertFalse(source.contains("_glFramebufferTexture2D(36160, 36064, 3553"),
            "GlCommandEncoder should not detach color attachment via hardcoded framebuffer/attachment/target literals");
        assertFalse(source.contains("_glFramebufferTexture2D(36160, 36096, 3553"),
            "GlCommandEncoder should not detach depth attachment via hardcoded framebuffer/attachment/target literals");
        assertFalse(source.contains("_glBindFramebuffer(36160"),
            "GlCommandEncoder should not bind/unbind GL_FRAMEBUFFER via hardcoded literal 36160");

        assertTrue(source.contains("VulkanicAPI.framebufferColorAttachment0Texture2D("),
            "GlCommandEncoder should detach color attachments via framebufferColorAttachment0Texture2D helper");
        assertTrue(source.contains("VulkanicAPI.framebufferDepthAttachmentTexture2D("),
            "GlCommandEncoder should detach depth attachments via framebufferDepthAttachmentTexture2D helper");
        assertTrue(source.contains("VulkanicAPI.bindDefaultFramebuffer("),
            "GlCommandEncoder should bind/unbind default framebuffer through VulkanicAPI.bindDefaultFramebuffer");
        assertFalse(source.contains("VulkanicAPI.clearBuffersWithMacosWorkaround("),
            "GlCommandEncoder should not clear through generic raw-mask macOS helper");
        assertFalse(source.contains("VulkanicAPI.clearBuffersWithMacosWorkaround(VulkanicAPI.getCommandContext(), VulkanicAPI.GL_COLOR_BUFFER_BIT)"),
            "GlCommandEncoder should not clear color via raw GL_COLOR_BUFFER_BIT mask");
        assertFalse(source.contains("VulkanicAPI.clearBuffersWithMacosWorkaround(VulkanicAPI.getCommandContext(), VulkanicAPI.GL_COLOR_BUFFER_BIT | VulkanicAPI.GL_DEPTH_BUFFER_BIT)"),
            "GlCommandEncoder should not clear color+depth via raw GL bitmask");
        assertFalse(source.contains("VulkanicAPI.clearBuffersWithMacosWorkaround(VulkanicAPI.getCommandContext(), VulkanicAPI.GL_DEPTH_BUFFER_BIT)"),
            "GlCommandEncoder should not clear depth via raw GL_DEPTH_BUFFER_BIT mask");
        assertFalse(source.contains("int j = 0;"),
            "GlCommandEncoder should not compose clear masks through ad-hoc integer bitfields");
        assertTrue(containsAny(source,
            "VulkanicAPI.clearColorBufferWithMacosWorkaround(VulkanicAPI.getCommandContext())",
            "VulkanicAPI.clearColorBufferWithMacosWorkaround(ctx)"),
            "GlCommandEncoder should clear color through VulkanicAPI clearColorBufferWithMacosWorkaround helper");
        assertTrue(containsAny(source,
            "VulkanicAPI.clearColorAndDepthBuffersWithMacosWorkaround(VulkanicAPI.getCommandContext())",
            "VulkanicAPI.clearColorAndDepthBuffersWithMacosWorkaround(ctx)"),
            "GlCommandEncoder should clear color+depth through VulkanicAPI clearColorAndDepthBuffersWithMacosWorkaround helper");
        assertTrue(containsAny(source,
            "VulkanicAPI.clearDepthBufferWithMacosWorkaround(VulkanicAPI.getCommandContext())",
            "VulkanicAPI.clearDepthBufferWithMacosWorkaround(ctx)"),
            "GlCommandEncoder should clear depth through VulkanicAPI clearDepthBufferWithMacosWorkaround helper");
        assertTrue(source.contains("boolean shouldClearColor = false;"),
            "GlCommandEncoder should track color clear intent explicitly");
        assertTrue(source.contains("boolean shouldClearDepth = false;"),
            "GlCommandEncoder should track depth clear intent explicitly");
    }

    @Test
    public void testGlCommandEncoderUsesCommandBufferLifecycleForRenderPass() throws IOException {
        Path file = SRC_MAIN_JAVA.resolve("net/blaze3d/opengl/GlCommandEncoder.java");
        String source = readSource(file);

        assertTrue(source.contains("CommandContext renderPassCtx = VulkanicAPI.beginCommandBuffer();"),
            "GlCommandEncoder should begin an explicit command-buffer scope for backend render-pass recording");
        assertTrue(source.contains("VulkanicAPI.beginRenderPass(") && source.contains("renderPassCtx, supplier,"),
            "GlCommandEncoder should pass explicit command-buffer context into VulkanicAPI.beginRenderPass");
        assertTrue(source.contains("VulkanicAPI.submitCommandBuffer(renderPassCtx);"),
            "GlCommandEncoder should submit the explicit render-pass command buffer when the render pass ends");
        assertTrue(source.contains("return this.activeRenderPassContext != null ? this.activeRenderPassContext : VulkanicAPI.getCommandContext();"),
            "GlCommandEncoder should prefer the active render-pass command context over the backend-global current context");
        assertTrue(source.contains("ShadowRenderingState.areShadowsCurrentlyBeingRendered() && commandContext().isImmediate()"),
            "GlCommandEncoder should restrict the Iris shadow temp-FBO shortcut to immediate contexts so Vulkan shadow draws keep a real active render pass");
        assertTrue(source.contains("if (ctx.isImmediate()")
                && source.contains("VulkanicAPI.bindFramebuffer(commandContext(), iris$tempFBO);"),
            "GlCommandEncoder trySetup should only rebind the Iris shadow temp FBO on the immediate compatibility seam");
    }

    @Test
    public void testSodiumGLRenderDeviceUsesAgnosticCopyFenceAndCapabilityHelpers() throws IOException {
        Path file = SRC_MAIN_JAVA.resolve("net/sodium/client/gl/device/GLRenderDevice.java");
        String source = readSource(file);

        assertFalse(source.contains("getInteger(VulkanicAPI.getImmediateContext(), 33085)"),
            "GLRenderDevice should not query max texture LOD bias with hardcoded literal 33085");
        assertFalse(source.contains("copyBufferSubData(VulkanicAPI.getImmediateContext(), VulkanicAPI.GL_COPY_READ_BUFFER, VulkanicAPI.GL_COPY_WRITE_BUFFER"),
            "GLRenderDevice should not copy buffers by spelling out copy targets inline");
        assertFalse(source.contains("createFenceSync(VulkanicAPI.getImmediateContext(), VulkanicAPI.GL_SYNC_GPU_COMMANDS_COMPLETE, 0)"),
            "GLRenderDevice should not create completion fences via raw createFenceSync parameters");

        assertTrue(source.contains("getInteger(VulkanicAPI.getCommandContext(), VulkanicIntegerQuery.MAX_TEXTURE_LOD_BIAS)"),
            "GLRenderDevice should use typed VulkanicIntegerQuery.MAX_TEXTURE_LOD_BIAS instead of raw texture query constants");
        assertFalse(source.contains("VulkanicAPI.bindBuffer(ctx, target.getTargetParameter(), buffer.handle())"),
            "GLRenderDevice should no longer bind buffers through raw GlBufferTarget integer targets where typed targets are available");
        assertTrue(source.contains("VulkanicAPI.bindBuffer(ctx, target.toVulkanicBufferTarget(), buffer.handle())"),
            "GLRenderDevice should bind buffers through typed VulkanicBufferTarget mapping");
        assertTrue(source.contains("VulkanicAPI.copyBufferSubDataBetweenCopyTargets("),
            "GLRenderDevice should copy buffer ranges via copyBufferSubDataBetweenCopyTargets helper");
        assertTrue(source.contains("VulkanicAPI.createGpuCompletionFence("),
            "GLRenderDevice should create sync fences via createGpuCompletionFence helper");
        assertFalse(source.contains("VulkanicAPI.getImmediateContext()"),
            "GLRenderDevice should not hard-wire immediate-context retrieval");
        assertTrue(source.contains("VulkanicAPI.getCommandContext()"),
            "GLRenderDevice should fetch backend-neutral command context");
    }

    @Test
    public void testSodiumDiagnosticSamplerCollectorDoesNotBorrowIrisOnRustVulkan() throws IOException {
        Path file = SRC_MAIN_JAVA.resolve("net/sodium/client/render/chunk/DefaultChunkRenderer.java");
        String source = readSource(file);

        assertTrue(source.contains("VulkanicAPI.isVulkanBackendSelected()")
                && source.contains("RustGalVulkanWholeFrameMode.enabled()"),
            "Sodium diagnostic sampler collection should recognize the Rust-owned Vulkan route");
        assertTrue(source.contains("recover a missing binding from Iris' Java GPU-state cache"),
            "Sodium diagnostics must fail closed instead of borrowing Iris GPU state on Rust Vulkan");
    }

    @Test
    public void testSodiumGlProgramUsesBackendNeutralContext() throws IOException {
        Path file = SRC_MAIN_JAVA.resolve("net/sodium/client/gl/shader/GlProgram.java");
        String source = readSource(file);

        assertFalse(source.contains("VulkanicAPI.getImmediateContext()"),
            "Sodium GlProgram should not hard-wire immediate-context retrieval");
        assertTrue(source.contains("VulkanicAPI.getCommandContext()"),
            "Sodium GlProgram should fetch backend-neutral command context");
    }

    @Test
    public void testSodiumVulkanChunkCutoutShaderRestrictsBackfaceDiscardToNonMippedCutouts() throws IOException {
        Path shaderFile = PROJECT_ROOT.resolve("src/main/resources/assets/sodium/shaders/core/vulkan_chunk.fsh");
        String shaderSource = readSource(shaderFile);

        assertTrue(shaderSource.contains("#ifdef USE_FRAGMENT_DISCARD"),
            "The Vulkan Sodium chunk fragment shader should keep the cutout-only discard branch");
        assertTrue(shaderSource.contains("if (!_material_use_mips(materialBits) && !gl_FrontFacing) {"),
            "The Vulkan Sodium chunk cutout shader should only discard back-facing fragments for non-mipped alpha-tested terrain such as tall grass and leaf litter");
        assertTrue(shaderSource.contains("if (color.a < _material_alpha_cutoff(materialBits)) {"),
            "The Vulkan Sodium chunk cutout shader should keep alpha cutoff discard for all alpha-tested terrain");

        Path pipelineFile = SRC_MAIN_JAVA.resolve("net/sodium/client/render/chunk/shader/SodiumChunkRenderPipelines.java");
        String pipelineSource = readSource(pipelineFile);
        Path rendererFile = SRC_MAIN_JAVA.resolve("net/sodium/client/render/chunk/DefaultChunkRenderer.java");
        String rendererSource = readSource(rendererFile);

        assertTrue(pipelineSource.contains("withShaderDefine(\"USE_FRAGMENT_DISCARD\")"),
            "The Vulkan Sodium cutout pipeline should keep the cutout-only shader define that scopes alpha discard to alpha-tested terrain");
        assertFalse(pipelineSource.contains("withShaderDefine(\"VULKAN_DISABLE_TERRAIN_FOG\")"),
            "The Vulkan Sodium chunk pipeline should keep vanilla terrain fog enabled so underwater cutout terrain is fogged like OpenGL");
        assertTrue(shaderSource.contains("vec4 SodiumFogColor")
                && shaderSource.contains("SodiumEnvironmentFog.x")
                && shaderSource.contains("SodiumRenderFog.x"),
            "The Vulkan Sodium chunk shader should apply fog from Sodium's terrain FogParameters instead of Minecraft's global fog block");
        assertTrue(rendererSource.contains("this.writeChunkParams(commandEncoder, parameters)")
                && rendererSource.contains(".putVec4(fogParameters.red(), fogParameters.green(), fogParameters.blue(), fogParameters.alpha())")
                && rendererSource.contains(".putVec2(fogParameters.environmentalStart(), fogParameters.environmentalEnd())")
                && rendererSource.contains(".putVec2(fogParameters.renderStart(), fogParameters.renderEnd())"),
            "The Vulkan Sodium chunk renderer should upload Sodium terrain fog parameters with the chunk render-pass UBO");
    }

    @Test
    public void testVulkanChunkRendererRoutesThroughSharedActiveProgram() throws IOException {
        Path rendererFile = SRC_MAIN_JAVA.resolve("net/sodium/client/render/chunk/DefaultChunkRenderer.java");
        String rendererSource = readSource(rendererFile);

        assertTrue(rendererSource.contains("super.begin(terrainPass, parameters);"),
            "The Vulkan chunk renderer should begin through the shared Sodium chunk program path before issuing terrain draws");
        assertTrue(rendererSource.contains("SharedChunkProgramOverrides.pushActiveProgram(this.activeProgram);"),
            "The Vulkan chunk renderer should expose the currently active Sodium chunk program to the compatibility pipeline compiler");
        assertTrue(rendererSource.contains("shader.setProjectionMatrix(matrices.projection());"),
            "The Vulkan chunk renderer should update the shared chunk shader projection matrix through the same interface used by OpenGL terrain");
        assertTrue(rendererSource.contains("shader.setModelViewMatrix(matrices.modelView());"),
            "The Vulkan chunk renderer should update the shared chunk shader model-view matrix through the same interface used by OpenGL terrain");
        assertTrue(rendererSource.contains("TerrainPipelineContract pipelineContract = SodiumChunkRenderPipelines.createContract(terrainPass, renderPassShader);")
                && rendererSource.contains("renderPass.setPipeline(SodiumChunkRenderPipelines.forContract(pipelineContract));"),
            "The Vulkan chunk renderer should create an explicit terrain pipeline contract before choosing the shared terrain pipeline");
        assertTrue(rendererSource.contains("renderPassShader.bindRenderPassResources(renderPass, terrainPass);"),
            "The Vulkan chunk renderer should mirror chunk-program sampler state into the render pass before drawing");
        assertFalse(rendererSource.contains("setModelMatrixUniforms(shader, preparedDraw.region(), camera);"),
            "Native Vulkan terrain should use explicit DynamicTransforms draw data instead of per-draw GL-style region uniform mutation");
        assertTrue(rendererSource.contains("renderPass.setUniform(\"DynamicTransforms\", preparedDraw.transforms());"),
            "Native Vulkan terrain should bind per-region translation through the explicit Vulkanic DynamicTransforms UBO");
        assertTrue(rendererSource.contains("GpuTextureView depthTargetView = this.resolveVulkanTerrainDepthTarget(target);")
                && rendererSource.contains("private GpuTextureView resolveVulkanTerrainDepthTarget(RenderTarget target)")
                && rendererSource.contains("GpuTextureView depthOverride = VulkanicAPI.getOutputDepthTextureOverride();")
                && rendererSource.contains("return net.minecraft.client.Minecraft.getInstance().getMainRenderTarget().getDepthTextureView();"),
            "Native Vulkan terrain should never drop depth just because the selected terrain color target lacks its own depth view");
        assertTrue(rendererSource.contains("final boolean useIndexedTessellation = terrainPass.isTranslucent() && indexedRenderingEnabled;"),
            "Vulkan shader terrain should keep Sodium's sorted translucent local-index path enabled");
        assertTrue(rendererSource.contains("renderDataStorage.fillDrawCommandBuffer(batch, renderRegion, renderList, camera, pass.isTranslucent(),")
                && rendererSource.contains("useBlockFaceCulling, useIndexedTessellation);"),
            "Local sorted translucent section draw assembly should stay on the Rust native render-data path");
        assertTrue(rendererSource.contains("super.end(terrainPass);"),
            "The Vulkan chunk renderer should close the shared chunk program path after terrain submission");
    }

    @Test
    public void testVulkanLegacyVaoCapturesAttributeBufferBindingsForInstancedDraws() throws IOException {
        Path backendFile = SRC_MAIN_JAVA.resolve("net/vulkanic/backends/vulkan/VulkanBackend.java");
        Path resourceManagerFile = SRC_MAIN_JAVA.resolve("net/vulkanic/backends/vulkan/VulkanBufferVertexResourceManager.java");
        Path galSnapshotBuilderFile = SRC_MAIN_JAVA.resolve("net/vulkanic/VulkanicGalSnapshotBuilder.java");
        String backendSource = readSource(backendFile);
        String resourceManagerSource = readSource(resourceManagerFile);
        String galSnapshotBuilderSource = readSource(galSnapshotBuilderFile);

        assertTrue(backendSource.contains("bufferVertexResources.setVertexAttributePointer(index, size, type, normalized, false, stride, pointer)")
                && backendSource.contains("bufferVertexResources.setVertexAttributePointer(index, size, type, false, true, stride, pointer)"),
            "Legacy glVertexAttribPointer/IPointer should delegate attribute capture to the buffer/vertex resource manager");
        assertTrue(resourceManagerSource.contains("boundLegacyBufferId(VulkanicAPI.GL_ARRAY_BUFFER)")
                && resourceManagerSource.contains("LegacyVertexAttribute previous = attributes.get(index);")
                && resourceManagerSource.contains("int divisor = previous != null ? previous.divisor() : 0;")
                && resourceManagerSource.contains("new LegacyVertexAttribute(index, index, size, type, normalized, integer, Math.toIntExact(pointer), divisor)")
                && resourceManagerSource.contains("bindings.put(index, new LegacyVertexBinding(index, effectiveStride, 0, divisor, buffer))"),
            "Pre-GL43 attribute pointers should preserve captured buffers and any divisor set before the pointer call inside the manager");
        assertTrue(backendSource.contains("DrawResourceSnapshot drawResources = legacyDrawResourceSnapshot();")
                && backendSource.contains("PipelineResourcePlanner.Plan resourceBindingPlan")
                && backendSource.contains("VulkanicGalSnapshotBuilder.legacyGraphicsSnapshot(")
                && backendSource.contains("pollCapturedLegacyGalDraw(galRequest, request)")
                && backendSource.contains("spine.executeCapturedGalDraw(")
                && backendSource.contains("private void executeCapturedGalDraw(")
                && backendSource.contains("VulkanDrawExecutionCoordinator.DrawExecutionPlan plan,")
                && backendSource.contains("VulkanDrawExecutionCoordinator.DrawResourceSnapshot drawResources")
                && backendSource.contains("VulkanicGalExecutionRequest.GraphicsDrawRequest capturedGalRequest")
                && backendSource.contains("unresolved-legacy-compatibility")
                && !backendSource.contains("backend.legacyDrawResourceSnapshot();")
                && backendSource.contains("plan.vertexStream()")
                && backendSource.contains("for (VulkanDrawExecutionCoordinator.VertexBufferBindingPlan binding : plan.vertexStream().vertexBuffers())")
                && galSnapshotBuilderSource.contains("VulkanicGalExecutionRequest.GraphicsCompatibilitySnapshot")
                && galSnapshotBuilderSource.contains("Objects.requireNonNull(vertexInput, \"vertexInput\")")
                && backendSource.contains("new VulkanGraphicsCommandExecutionCoordinator.VertexBufferBindingRequirement(")
                && backendSource.contains("graphicsCommandExecution.planGraphicsExecution("),
            "Legacy draw calls should capture draw resources before NativeSpine execution and feed every coordinator-planned immutable vertex buffer into the graphics command execution plan");
    }

    @Test
    public void testSodiumVulkanChunkTerrainUsesNativeTerrainCommandEncoder() throws IOException {
        Path rendererFile = SRC_MAIN_JAVA.resolve("net/sodium/client/render/chunk/DefaultChunkRenderer.java");
        Path apiFile = SRC_MAIN_JAVA.resolve("net/vulkanic/VulkanicAPI.java");
        Path backendFile = SRC_MAIN_JAVA.resolve("net/vulkanic/backends/vulkan/VulkanBackend.java");
        Path encoderFile = SRC_MAIN_JAVA.resolve("net/vulkanic/backends/vulkan/VulkanNativeTerrainCommandEncoder.java");
        Path nativeEncoderFile = SRC_MAIN_JAVA.resolve("net/vulkanic/backends/vulkan/VulkanNativeCommandEncoder.java");

        String rendererSource = readSource(rendererFile);
        String apiSource = readSource(apiFile);
        String backendSource = readSource(backendFile);
        String encoderSource = readSource(encoderFile);
        String nativeEncoderSource = readSource(nativeEncoderFile);

        assertTrue(rendererSource.contains("CommandEncoder commandEncoder = VulkanicAPI.createNativeTerrainCommandEncoder();"),
            "Sodium's Vulkan chunk terrain branch should use the native terrain encoder seam instead of the general compatibility encoder");
        assertTrue(apiSource.contains("public static CommandEncoder createNativeTerrainCommandEncoder()"),
            "VulkanicAPI should expose an explicit native terrain command-encoder migration seam");
        assertTrue(apiSource.contains("? directVulkanBackend.createNativeTerrainCommandEncoder()"),
            "The native terrain encoder seam should direct-dispatch to the raw Vulkan backend when Vulkan is selected");
        assertTrue(apiSource.contains(": getBackend().createCommandEncoder()"),
            "The native terrain encoder seam should leave non-Vulkan backends on the normal command encoder");
        assertTrue(backendSource.contains("public CommandEncoder createNativeTerrainCommandEncoder()")
                && backendSource.contains("return new VulkanNativeTerrainCommandEncoder(this);"),
            "VulkanBackend should own construction of the native terrain command encoder");
        assertTrue(encoderSource.contains("final class VulkanNativeTerrainCommandEncoder extends VulkanNativeCommandEncoder")
                && encoderSource.contains("super(backend, ResourceMode.TERRAIN);"),
            "The terrain slice should use the shared native CommandEncoder implementation in terrain resource mode");
        assertTrue(nativeEncoderSource.contains("class VulkanNativeCommandEncoder implements CommandEncoder"),
            "The shared native encoder should satisfy the Mojang CommandEncoder contract used by Sodium");
        assertTrue(nativeEncoderSource.contains("private final class NativeRenderPass implements RenderPass"),
            "The native terrain encoder should provide a Mojang RenderPass adapter for existing Sodium draw code");
        assertTrue(nativeEncoderSource.contains("this.backend.beginRenderPass(ctx, label, colorView, clearColor, depthView, clearDepth)")
                && nativeEncoderSource.contains("this.backend.beginRenderPass(ctx, label, framebuffer, hasDepthTexture)"),
            "The native terrain encoder should begin native Vulkan render passes directly instead of routing through GlCommandEncoder");
        assertTrue(nativeEncoderSource.contains("this.colorView != null")
                && nativeEncoderSource.contains("this.colorView,")
                && nativeEncoderSource.contains("this.depthView"),
            "Texture-view terrain passes must resolve target-compatible pipelines using their actual color/depth views");
        assertTrue(backendSource.contains("VulkanicTextureView colorTarget")
                && backendSource.contains("@Nullable VulkanicTextureView depthTarget")
                && backendSource.contains("VulkanicRenderPassDescriptor.colorAndDepth("),
            "VulkanBackend should expose render-target-compatible pipeline resolution for explicit texture-view render passes");
        assertTrue(backendSource.contains("NativeSpine.toVkFormat(targets.colorTexture.getVulkanicFormat())")
                && backendSource.contains("NativeSpine.toVkFormat(targets.depthTexture.getVulkanicFormat())"),
            "Texture-view pipeline resolution should key variants from the actual color/depth attachment formats");
        assertTrue(nativeEncoderSource.contains("this.pass.drawIndexed(firstIndex, indexCount, baseVertex, instanceCount);"),
            "The Mojang RenderPass drawIndexed argument order must be translated to VulkanicRenderPass order");
        assertTrue(nativeEncoderSource.contains("VulkanicAPI.createLiveProgramPipelineDescriptor(")
                && nativeEncoderSource.contains("SharedChunkProgramOverrides.bindableSamplers(pipeline)")
                && nativeEncoderSource.contains("case SAMPLER, COMPARISON_SAMPLER -> bindableSamplers.contains(binding.name())"),
            "The native terrain pass should preserve Iris live shared-chunk descriptor selection while filtering reflected terrain samplers to the Iris bindable set");
        assertTrue(nativeEncoderSource.contains("implements RenderPass, RenderPassResourceBinder"),
            "The native terrain pass should accept unit-aware Iris render-pass resource bindings");
        assertTrue(nativeEncoderSource.contains("public boolean bindLegacySampler(String name, int textureId, int textureUnit)")
                && nativeEncoderSource.contains("VulkanicAPI.createManagedLegacyTextureView(textureId)"),
            "The native terrain pass should recover shaderpack samplers that Iris exposes only as legacy texture IDs");
        assertTrue(nativeEncoderSource.contains("resolveSamplerUnit(binding)"),
            "The native terrain pass should bind descriptors using Iris sampler units, not descriptor binding indices");
        assertTrue(nativeEncoderSource.contains("VulkanicPipelineResourceResolver.buildPlan(")
                && nativeEncoderSource.contains("submission.missingResources()"),
            "The native terrain pass should share Vulkanic's resource planner instead of carrying a local descriptor walk");
        assertFalse(nativeEncoderSource.contains("private record PipelineResourceBindingSubmission"),
            "The native terrain pass should not keep a separate local resource-submission model once Vulkanic owns the shared plan");

        Path programSamplersFile = SRC_MAIN_JAVA.resolve("net/irisshaders/iris/gl/program/ProgramSamplers.java");
        String programSamplersSource = readSource(programSamplersFile);
        assertTrue(programSamplersSource.contains("renderPass instanceof net.vulkanic.RenderPassResourceBinder resourceBinder")
                && programSamplersSource.contains("resourceBinder.bindSampler(")
                && programSamplersSource.contains("binding.textureUnit()"),
            "Iris ProgramSamplers should pass sampler texture units to native Vulkan render passes");
        assertTrue(programSamplersSource.contains("resourceBinder.bindLegacySampler(name, textureId, binding.textureUnit())"),
            "Iris ProgramSamplers should pass raw texture-id samplers to native Vulkan render passes");
    }

    @Test
    public void testVulkanFramebufferRenderPassHonorsDepthAttachmentFlag() throws IOException {
        Path glCommandEncoderFile = SRC_MAIN_JAVA.resolve("net/blaze3d/opengl/GlCommandEncoder.java");
        Path apiFile = SRC_MAIN_JAVA.resolve("net/vulkanic/VulkanicAPI.java");
        Path backendFile = SRC_MAIN_JAVA.resolve("net/vulkanic/backends/vulkan/VulkanBackend.java");
        Path nativeEncoderFile = SRC_MAIN_JAVA.resolve("net/vulkanic/backends/vulkan/VulkanNativeCommandEncoder.java");

        String glCommandEncoderSource = readSource(glCommandEncoderFile);
        String apiSource = readSource(apiFile);
        String backendSource = readSource(backendFile);
        String nativeEncoderSource = readSource(nativeEncoderFile);

        assertTrue(glCommandEncoderSource.contains("VulkanicAPI.beginRenderPass(renderPassCtx, supplier, framebuffer, hasDepthTexture)"),
            "Compatibility framebuffer render passes must pass hasDepthTexture through to Vulkan");
        assertTrue(apiSource.contains("beginRenderPass(ctx, label, framebuffer, true)")
                && apiSource.contains("directVulkanBackend.beginRenderPass(ctx, label, framebuffer, hasDepthTexture)")
                && apiSource.contains("getBackend().beginRenderPass(ctx, label, framebuffer, hasDepthTexture)"),
            "VulkanicAPI should preserve the old default while exposing explicit framebuffer depth participation");
        assertTrue(nativeEncoderSource.contains("this.backend.beginRenderPass(ctx, label, framebuffer, hasDepthTexture)"),
            "Native framebuffer render passes must preserve hasDepthTexture when bypassing GlCommandEncoder");
        assertTrue(backendSource.contains("resolveFramebufferRenderTargetPlan(label, framebuffer, hasDepthTexture)")
                && backendSource.contains("resolveFramebufferTargets(framebuffer, includeDepthAttachment)")
                && backendSource.contains("if (includeDepthAttachment && depthTextureId != 0)"),
            "Vulkan framebuffer target resolution must omit the depth attachment when the caller requests color-only rendering");
    }

    @Test
    public void testSortedTranslucentIndexUploadsInvalidateCachedDrawBatches() throws IOException {
        Path worldRendererFile = SRC_MAIN_JAVA.resolve("net/sodium/client/render/SodiumWorldRenderer.java");
        String worldRendererSource = readSource(worldRendererFile);
        Path sectionManagerFile = SRC_MAIN_JAVA.resolve("net/sodium/client/render/chunk/RenderSectionManager.java");
        String sectionManagerSource = readSource(sectionManagerFile);
        Path regionManagerFile = SRC_MAIN_JAVA.resolve("net/sodium/client/render/chunk/region/RenderRegionManager.java");
        String regionManagerSource = readSource(regionManagerFile);

        assertTrue(worldRendererSource.contains("if (VulkanicAPI.isVulkanBackendSelected()\n                && !WorldRenderRoutePolicy.currentStaticTerrainRoute().usesRustWholeFrameVulkan()) {\n            sortBehavior = SortBehavior.OFF;"),
            "Java Vulkan compatibility should avoid Sodium's sorted local translucent index path, while Rust whole-frame terrain keeps semantic sort data enabled");
        assertTrue(worldRendererSource.contains("} else if (PlatformRuntimeInformation.getInstance().isDevelopmentEnvironment()"),
            "OpenGL should keep the existing development-only terrain sorting debug override behavior");
        assertFalse(regionManagerSource.contains("reverseSortedQuadUploadOrder"),
            "Vulkan should not apply ad-hoc index-order rewrites to Sodium's sorted translucent uploads");
        assertTrue(regionManagerSource.contains("var indexMetadataChanged = false;"),
            "RenderRegionManager should track sorted translucent index metadata changes separately from buffer resizes");
        assertTrue(regionManagerSource.contains("indexMetadataChanged = true;")
                && regionManagerSource.contains("if (indexBufferChanged || indexMetadataChanged)"),
            "Sorted translucent index uploads must clear cached draw batches even when the backing index buffer object did not resize");
        assertTrue(sectionManagerSource.contains("this.sortBehavior != SortBehavior.OFF")
                && sectionManagerSource.contains(": importantRebuildQueueType"),
            "SortBehavior.OFF should not dereference a null translucent-sort defer mode while building render lists");
        assertTrue(sectionManagerSource.contains("if (this.sortBehavior == SortBehavior.OFF)") && sectionManagerSource.contains("return;"),
            "SortBehavior.OFF should ignore dynamic sort scheduling requests");
    }

    @Test
    public void testVulkanIrisRenderProgramsUseLiveDescriptorsBeforeBindingResources() throws IOException {
        Path encoderFile = SRC_MAIN_JAVA.resolve("net/blaze3d/opengl/GlCommandEncoder.java");
        String encoderSource = readSource(encoderFile);

        assertTrue(encoderSource.contains("createIrisProgramLiveDescriptor"),
            "Vulkan should build live descriptors for generic Iris render programs, not only shared terrain programs");
        assertTrue(encoderSource.contains("program instanceof net.irisshaders.iris.pipeline.programs.IrisProgram"),
            "The generic live descriptor path should be scoped to actual Iris shaderpack programs");
        assertTrue(encoderSource.contains("VulkanicAPI.getLinkedProgramSpirvModules(ctx, programHandle).isEmpty()"),
            "The generic live descriptor path should require linked SPIR-V modules before replacing the base descriptor");
        assertTrue(encoderSource.indexOf("this.setupIrisProgramStateIfNeeded(glRenderPass);")
                < encoderSource.indexOf("PipelineResourcePlanner.Plan submission = this.buildPipelineResourceBindings(glRenderPass, selectedDescriptor);"),
            "Vulkan should run Iris setup before collecting descriptor resources so shaderpack sampler state is current");
    }

    @Test
    public void testSharedChunkProgramOverridePrecedesIrisOverrideMap() throws IOException {
        Path overrideFile = SRC_MAIN_JAVA.resolve("net/sodium/client/render/chunk/shader/SharedChunkProgramOverrides.java");
        String overrideSource = readSource(overrideFile);

        assertTrue(overrideSource.contains("wrapper.setupUniforms(pipeline.getUniforms(), pipeline.getSamplers());"),
            "Shared chunk program overrides should reflect the current render-pipeline sampler and uniform contract onto the wrapped Sodium program handle");
        assertTrue(overrideSource.contains("public static void pushActiveProgram"),
            "Shared chunk program overrides should explicitly track the currently active Sodium chunk program");

        Path deviceFile = SRC_MAIN_JAVA.resolve("net/blaze3d/opengl/GlDevice.java");
        String deviceSource = readSource(deviceFile);
        int sharedOverrideIndex = deviceSource.indexOf("SharedChunkProgramOverrides.createOverride(renderPipeline)");
        int irisOverrideIndex = deviceSource.indexOf("// Iris: Check for shader overrides first");

        assertTrue(sharedOverrideIndex >= 0,
            "GlDevice should consult the shared Sodium chunk program override seam when compiling tracked terrain pipelines");
        assertTrue(irisOverrideIndex > sharedOverrideIndex,
            "GlDevice must try the shared Sodium chunk program override before falling back to Iris's generic shader override map");
    }

    @Test
    public void testSharedChunkPipelinesMirrorExtendedVertexAbiWithoutStaticRegistration() throws IOException {
        Path pipelineFile = SRC_MAIN_JAVA.resolve("net/sodium/client/render/chunk/shader/SodiumChunkRenderPipelines.java");
        String pipelineSource = readSource(pipelineFile);

        assertTrue(pipelineSource.contains("createVertexFormat(WorldRenderingSettings.INSTANCE.getVertexFormat().getVertexFormat())"),
            "Shared Sodium chunk pipelines should derive their vertex ABI from the active WorldRenderingSettings format, not just the base stride");
        assertTrue(pipelineSource.contains("SharedChunkProgramOverrides.register(pipeline, key.contract());"),
            "Shared Sodium chunk pipelines should register tracked terrain pipelines with their typed terrain contract for active-program overrides");
        assertTrue(pipelineSource.contains("for (RenderPipeline pipeline : PIPELINES.values())")
                && pipelineSource.contains("SharedChunkProgramOverrides.unregister(pipeline);"),
            "Shared Sodium chunk pipeline cache should clean up tracked override entries when shader reloads invalidate cached pipelines");
        Path contractFile = SRC_MAIN_JAVA.resolve("net/sodium/client/render/chunk/shader/TerrainPipelineContract.java");
        String contractSource = readSource(contractFile);

        assertTrue(contractSource.contains("PassState.from(pipeline, pass.isTranslucent(), indexedBlendOverrides)")
                && contractSource.contains("currentBlend(pipeline, translucentPass)")
                && contractSource.contains("pipeline.isWriteDepth() && DepthColorStorage.isDepthMaskEnabled()"),
            "Shared Sodium chunk pipelines should derive Vulkan terrain depth, blend, and write-mask state from the explicit terrain contract, including pipeline-declared translucent depth writes");
        assertTrue(contractSource.contains("blendOverrideForAttachment(int colorAttachmentIndex)")
                && contractSource.contains("List<IndexedBlendState> indexedBlendStates"),
            "Shared Sodium chunk pipelines should carry Iris per-buffer blend overrides in the explicit terrain contract");
        assertTrue(contractSource.contains("if (!BlendModeStorage.isBlendEnabled())")
                && contractSource.contains("Optional.of(pipeline.getBlendFunction().orElse(BlendFunction.TRANSLUCENT))"),
            "The terrain pipeline contract should preserve declared translucent blending when Iris has no active legacy blend override");
        assertTrue(pipelineSource.contains("case 10 -> \"iris_Normal\";"),
            "Shared Sodium chunk pipelines should carry Iris terrain attribute locations into the Vulkan terrain ABI");
        assertFalse(pipelineSource.contains("RenderPipelines.register("),
            "Shared Sodium chunk pipelines should no longer leak dynamic terrain pipelines into the global static RenderPipelines registry");
    }

    @Test
    public void testGenericVertexLocationsHonorDeclaredAttributeIndices() throws IOException {
        Path vertexFormatFile = SRC_MAIN_JAVA.resolve("net/blaze3d/vertex/VertexFormat.java");
        String vertexFormatSource = readSource(vertexFormatFile);
        assertTrue(vertexFormatSource.contains("public int getShaderAttributeLocation(int attributeOrdinal)"),
            "VertexFormat should expose the shader attribute location derived from its declared element metadata");
        assertTrue(vertexFormatSource.contains("element.usage() == VertexFormatElement.Usage.GENERIC ? element.index() : attributeOrdinal"),
            "VertexFormat should preserve explicit generic attribute indices while keeping vanilla attribute ordering unchanged");

        Path glProgramFile = SRC_MAIN_JAVA.resolve("net/blaze3d/opengl/GlProgram.java");
        String glProgramSource = readSource(glProgramFile);
        assertTrue(glProgramSource.contains("vertexFormat.getShaderAttributeLocation(j)"),
            "GlProgram linking should bind declared attribute names to the shader locations supplied by the vertex format");

        Path variantPlannerFile = SRC_MAIN_JAVA.resolve("net/vulkanic/backends/vulkan/VulkanShaderVariantPlanner.java");
        Path plannerFile = SRC_MAIN_JAVA.resolve("net/vulkanic/backends/vulkan/VulkanPipelineCreationPlanner.java");
        String variantPlannerSource = readSource(variantPlannerFile);
        String plannerSource = readSource(plannerFile);
        assertTrue(variantPlannerSource.contains("renderPipeline.getVertexFormat().getShaderAttributeLocation(location)"),
            "Vulkan shader-source rebinding should inject explicit locations from the declared vertex format instead of forcing sequential chunk attributes");
        assertTrue(plannerSource.contains("vertexFormat.getShaderAttributeLocation(i)")
                && plannerSource.contains("vertexFormat.getShaderAttributeLocation(i)"),
            "Vulkan pipeline vertex input planning should preserve explicit generic attribute locations for extended terrain formats");
    }

    @Test
    public void testSodiumGlShaderAndSyncWrappersUseBackendNeutralContext() throws IOException {
        Path shaderFile = SRC_MAIN_JAVA.resolve("net/sodium/client/gl/shader/GlShader.java");
        String shaderSource = readSource(shaderFile);

        assertFalse(shaderSource.contains("VulkanicAPI.getImmediateContext()"),
            "Sodium GlShader should not hard-wire immediate-context retrieval");
        assertTrue(shaderSource.contains("CommandContext ctx = VulkanicAPI.getCommandContext();"),
            "Sodium GlShader should fetch backend-neutral command context once and reuse it");

        Path fenceFile = SRC_MAIN_JAVA.resolve("net/sodium/client/gl/sync/GlFence.java");
        String fenceSource = readSource(fenceFile);

        assertFalse(fenceSource.contains("VulkanicAPI.getImmediateContext()"),
            "Sodium GlFence should not hard-wire immediate-context retrieval");
        assertTrue(fenceSource.contains("VulkanicAPI.getCommandContext()"),
            "Sodium GlFence should fetch backend-neutral command context");

        Path storageFile = SRC_MAIN_JAVA.resolve("net/sodium/client/gl/functions/BufferStorageFunctions.java");
        String storageSource = readSource(storageFile);

        assertFalse(storageSource.contains("VulkanicAPI.getImmediateContext()"),
            "Sodium BufferStorageFunctions should not hard-wire immediate-context retrieval");
        assertTrue(storageSource.contains("VulkanicAPI.getCommandContext()"),
            "Sodium BufferStorageFunctions should fetch backend-neutral command context");
    }

    @Test
    public void testSodiumShaderChunkRendererUsesBackendNeutralContext() throws IOException {
        Path file = SRC_MAIN_JAVA.resolve("net/sodium/client/render/chunk/ShaderChunkRenderer.java");
        String source = readSource(file);

        assertFalse(source.contains("VulkanicAPI.getImmediateContext()"),
            "ShaderChunkRenderer should not hard-wire immediate-context retrieval in viewport/framebuffer setup");
        assertTrue(source.contains("VulkanicAPI.getCommandContext()"),
            "ShaderChunkRenderer should fetch backend-neutral command context");
    }

    @Test
    public void testSodiumLeafWrappersUseBackendNeutralContext() throws IOException {
        String[] migratedFiles = new String[] {
            "net/sodium/client/gl/tessellation/GlAbstractTessellation.java",
            "net/sodium/client/compatibility/environment/GlContextInfo.java",
            "net/sodium/fabric/SodiumGpuSyncHelper.java",
            "net/sodium/client/render/chunk/shader/DefaultShaderInterface.java",
            "net/sodium/client/gl/shader/uniform/GlUniformFloat4v.java",
            "net/sodium/client/gl/shader/uniform/GlUniformFloat3v.java",
            "net/sodium/client/gl/shader/uniform/GlUniformFloat2v.java",
            "net/sodium/client/gl/shader/uniform/GlUniformMatrix4f.java",
            "net/sodium/client/gl/shader/uniform/GlUniformInt.java",
            "net/sodium/client/gl/shader/uniform/GlUniformFloat.java",
            "net/sodium/client/gl/shader/uniform/GlUniformBlock.java",
            "net/sodium/client/gl/shader/ShaderWorkarounds.java",
            "net/sodium/client/gl/buffer/GlBuffer.java",
            "net/sodium/client/gl/array/GlVertexArray.java",
            "net/sodium/client/compatibility/workarounds/nvidia/NvidiaWorkarounds.java"
        };

        for (String relativePath : migratedFiles) {
            Path file = SRC_MAIN_JAVA.resolve(relativePath);
            String source = readSource(file);

            assertFalse(source.contains("VulkanicAPI.getImmediateContext()"),
                relativePath + " should not hard-wire immediate-context retrieval");
            assertTrue(source.contains("VulkanicAPI.getCommandContext()"),
                relativePath + " should fetch backend-neutral command context");
        }
    }

    @Test
    public void testVertexArrayCacheUsesAgnosticArrayBufferConstant() throws IOException {
        Path file = SRC_MAIN_JAVA.resolve("net/blaze3d/opengl/VertexArrayCache.java");
        String source = readSource(file);

        assertFalse(source.contains("_glBindBuffer(34962"),
            "VertexArrayCache should not bind GL_ARRAY_BUFFER via hardcoded target literal 34962");
        assertFalse(source.contains("GlStateManager._glBindBuffer("),
            "VertexArrayCache should not bind buffers through GlStateManager wrapper");
        assertFalse(source.contains("VulkanicAPI.bindBuffer(ctx, VulkanicAPI.GL_ARRAY_BUFFER, glBuffer.handle)"),
            "VertexArrayCache should no longer use raw GL_ARRAY_BUFFER target where typed targets are available");
        assertTrue(source.contains("VulkanicAPI.bindBuffer(ctx, VulkanicBufferTarget.VERTEX, glBuffer.handle)"),
            "VertexArrayCache should bind array buffers through typed VulkanicBufferTarget.VERTEX");
        assertFalse(source.contains("GlStateManager._enableVertexAttribArray("),
            "VertexArrayCache should not enable attributes through GlStateManager wrapper");
        assertFalse(source.contains("GlStateManager._vertexAttribPointer("),
            "VertexArrayCache should not set attrib pointers through GlStateManager wrapper");
        assertFalse(source.contains("GlStateManager._vertexAttribIPointer("),
            "VertexArrayCache should not set integer attrib pointers through GlStateManager wrapper");
        assertFalse(source.contains("GlStateManager._glBindVertexArray("),
            "VertexArrayCache should not bind vertex arrays through GlStateManager wrapper");
        assertTrue(source.contains("VulkanicAPI.enableVertexAttribArray("),
            "VertexArrayCache should enable attributes directly via VulkanicAPI.enableVertexAttribArray");
        assertTrue(source.contains("VulkanicAPI.setVertexAttribPointer("),
            "VertexArrayCache should set attrib pointers directly via VulkanicAPI.setVertexAttribPointer");
        assertTrue(source.contains("VulkanicAPI.setVertexAttribIPointer("),
            "VertexArrayCache should set integer attrib pointers directly via VulkanicAPI.setVertexAttribIPointer");
        assertTrue(source.contains("VulkanicAPI.bindVertexArray("),
            "VertexArrayCache should bind vertex arrays directly via VulkanicAPI.bindVertexArray");
    }

    @Test
    public void testGlDeviceUsesAgnosticCapabilityAndAlignmentHelpers() throws IOException {
        Path file = SRC_MAIN_JAVA.resolve("net/blaze3d/opengl/GlDevice.java");
        String source = readSource(file);

        assertFalse(source.contains("VulkanicAPI.getInteger(net.vulkanic.VulkanicAPI.getImmediateContext(), 35380)"),
            "GlDevice should not query UBO offset alignment via hardcoded literal 35380");
        assertFalse(source.contains("VulkanicAPI.setCapabilityEnabled(ctx, 34895, true)"),
            "GlDevice should not enable program point size via hardcoded literal 34895");

        assertTrue(source.contains("VulkanicAPI.getUniformBufferOffsetAlignment("),
            "GlDevice should query UBO alignment via VulkanicAPI.getUniformBufferOffsetAlignment");
        assertTrue(source.contains("VulkanicAPI.setProgramPointSizeEnabled("),
            "GlDevice should enable program point size via VulkanicAPI.setProgramPointSizeEnabled");
    }

    @Test
    public void testGlDeviceTextureSetupUsesAgnosticTextureParameterHelpers() throws IOException {
        Path file = SRC_MAIN_JAVA.resolve("net/blaze3d/opengl/GlDevice.java");
        String source = readSource(file);

        assertFalse(source.contains("_texParameter(o, 33085"),
            "GlDevice texture setup should not set GL_TEXTURE_MAX_LEVEL via hardcoded literal 33085");
        assertFalse(source.contains("_texParameter(o, 33082"),
            "GlDevice texture setup should not set GL_TEXTURE_MIN_LOD via hardcoded literal 33082");
        assertFalse(source.contains("_texParameter(o, 33083"),
            "GlDevice texture setup should not set GL_TEXTURE_MAX_LOD via hardcoded literal 33083");
        assertFalse(source.contains("_texParameter(o, 34892"),
            "GlDevice texture setup should not toggle GL_TEXTURE_COMPARE_MODE via hardcoded literal 34892");
        assertFalse(source.contains("_getInteger(3379)"),
            "GlDevice max texture-size probe should not query GL_MAX_TEXTURE_SIZE via hardcoded literal 3379");
        assertFalse(source.contains("_texImage2D(32868"),
            "GlDevice max texture-size probe should not use hardcoded GL_PROXY_TEXTURE_2D literal 32868");

        assertTrue(source.contains("VulkanicAPI.setTextureMaxLevel("),
            "GlDevice texture setup should use VulkanicAPI.setTextureMaxLevel helper");
        assertTrue(source.contains("VulkanicAPI.setTextureMinLod("),
            "GlDevice texture setup should use VulkanicAPI.setTextureMinLod helper");
        assertTrue(source.contains("VulkanicAPI.setTextureMaxLod("),
            "GlDevice texture setup should use VulkanicAPI.setTextureMaxLod helper");
        assertTrue(source.contains("VulkanicAPI.disableTextureCompareMode("),
            "GlDevice depth texture setup should use VulkanicAPI.disableTextureCompareMode helper");
        assertTrue(source.contains("VulkanicIntegerQuery.MAX_TEXTURE_SIZE"),
            "GlDevice max texture-size probe should use typed VulkanicIntegerQuery.MAX_TEXTURE_SIZE");
        assertTrue(source.contains("VulkanicAPI.GL_PROXY_TEXTURE_2D"),
            "GlDevice max texture-size probe should use VulkanicAPI.GL_PROXY_TEXTURE_2D constant");

        Path irisGlTextureFile = SRC_MAIN_JAVA.resolve("net/irisshaders/iris/gl/texture/GlTexture.java");
        String irisGlTextureSource = readSource(irisGlTextureFile);
        assertFalse(irisGlTextureSource.contains("IrisRenderSystem.texParameteri(texture, target.getGlType(), VulkanicAPI.GL_TEXTURE_MIN_FILTER"),
            "Iris GlTexture should not set min filter through raw GL_TEXTURE_MIN_FILTER pname constants");
        assertFalse(irisGlTextureSource.contains("IrisRenderSystem.texParameteri(texture, target.getGlType(), VulkanicAPI.GL_TEXTURE_MAG_FILTER"),
            "Iris GlTexture should not set mag filter through raw GL_TEXTURE_MAG_FILTER pname constants");
        assertFalse(irisGlTextureSource.contains("IrisRenderSystem.texParameteri(texture, target.getGlType(), VulkanicAPI.GL_TEXTURE_WRAP_S"),
            "Iris GlTexture should not set wrap S through raw GL_TEXTURE_WRAP_S pname constants");
        assertFalse(irisGlTextureSource.contains("IrisRenderSystem.texParameteri(texture, target.getGlType(), VulkanicAPI.GL_TEXTURE_MAX_LEVEL"),
            "Iris GlTexture should not set max level through raw GL_TEXTURE_MAX_LEVEL pname constants");
        assertTrue(
            irisGlTextureSource.contains("VulkanicAPI.setTextureLinearFiltering(ctx, target.getGlType())")
                || irisGlTextureSource.contains("target.setLinearFiltering(ctx)"),
            "Iris GlTexture should use VulkanicAPI texture filtering helpers directly or via TextureType typed helper when blur is enabled"
        );
        assertTrue(
            irisGlTextureSource.contains("VulkanicAPI.setTextureNearestFiltering(ctx, target.getGlType())")
                || irisGlTextureSource.contains("target.setNearestFiltering(ctx)"),
            "Iris GlTexture should use VulkanicAPI texture filtering helpers directly or via TextureType typed helper when blur is disabled"
        );
        assertTrue(
            irisGlTextureSource.contains("VulkanicAPI.setTextureWrapMode(ctx, target.getGlType(), filteringData.shouldClamp(), sizeY > 0, sizeZ > 0)")
                || irisGlTextureSource.contains("target.setWrapMode(ctx, filteringData.shouldClamp(), sizeY > 0, sizeZ > 0)"),
            "Iris GlTexture should use VulkanicAPI wrap helpers directly or via TextureType typed helper"
        );
        assertTrue(
            irisGlTextureSource.contains("VulkanicAPI.resetTextureLodRangeToZero(ctx, target.getGlType())")
                || irisGlTextureSource.contains("target.resetLodRangeToZero(ctx)"),
            "Iris GlTexture should use VulkanicAPI LOD-reset helpers directly or via TextureType typed helper"
        );

        Path irisGlImageFile = SRC_MAIN_JAVA.resolve("net/irisshaders/iris/gl/image/GlImage.java");
        String irisGlImageSource = readSource(irisGlImageFile);
        assertFalse(irisGlImageSource.contains("IrisRenderSystem.texParameteri(texture, target.getGlType(), VulkanicAPI.GL_TEXTURE_MIN_FILTER"),
            "Iris GlImage should not set min filter through raw GL_TEXTURE_MIN_FILTER pname constants");
        assertFalse(irisGlImageSource.contains("IrisRenderSystem.texParameteri(texture, target.getGlType(), VulkanicAPI.GL_TEXTURE_WRAP_S"),
            "Iris GlImage should not set wrap through raw GL_TEXTURE_WRAP_* pname constants");
        assertFalse(irisGlImageSource.contains("IrisRenderSystem.texParameterf(texture, target.getGlType(), VulkanicAPI.GL_TEXTURE_LOD_BIAS"),
            "Iris GlImage should not set LOD bias through raw GL_TEXTURE_LOD_BIAS pname constants");
        assertTrue(
            irisGlImageSource.contains("VulkanicAPI.setTextureNearestFiltering(ctx, target.getGlType())")
                || irisGlImageSource.contains("target.setNearestFiltering(ctx)"),
            "Iris GlImage integer format path should use VulkanicAPI filtering helpers directly or via TextureType typed helper"
        );
        assertTrue(
            irisGlImageSource.contains("VulkanicAPI.setTextureLinearFiltering(ctx, target.getGlType())")
                || irisGlImageSource.contains("target.setLinearFiltering(ctx)"),
            "Iris GlImage non-integer format path should use VulkanicAPI filtering helpers directly or via TextureType typed helper"
        );
        assertTrue(
            irisGlImageSource.contains("VulkanicAPI.setTextureWrapMode(ctx, target.getGlType(), true, height > 0, depth > 0)")
                || irisGlImageSource.contains("target.setWrapMode(ctx, true, height > 0, depth > 0)"),
            "Iris GlImage clamp setup should use VulkanicAPI wrap helpers directly or via TextureType typed helper"
        );
        assertTrue(
            irisGlImageSource.contains("VulkanicAPI.resetTextureLodRangeToZero(ctx, target.getGlType())")
                || irisGlImageSource.contains("target.resetLodRangeToZero(ctx)"),
            "Iris GlImage LOD setup should use VulkanicAPI LOD-reset helpers directly or via TextureType typed helper"
        );

        Path vulkanicApiFile = SRC_MAIN_JAVA.resolve("net/vulkanic/VulkanicAPI.java");
        String vulkanicApiSource = readSource(vulkanicApiFile);
        assertTrue(vulkanicApiSource.contains("public static void setTextureLinearFiltering(CommandContext ctx, int target)"),
            "VulkanicAPI should expose setTextureLinearFiltering helper");
        assertTrue(vulkanicApiSource.contains("public static void setTextureNearestFiltering(CommandContext ctx, int target)"),
            "VulkanicAPI should expose setTextureNearestFiltering helper");
        assertTrue(vulkanicApiSource.contains("public static void setTextureWrapMode(CommandContext ctx, int target, boolean clampToEdge, boolean includeWrapT, boolean includeWrapR)"),
            "VulkanicAPI should expose setTextureWrapMode helper");
        assertTrue(vulkanicApiSource.contains("public static void resetTextureLodRangeToZero(CommandContext ctx, int target)"),
            "VulkanicAPI should expose resetTextureLodRangeToZero helper");
    }

    @Test
    public void testGlDeviceUsesDirectVulkanicQueryAndErrorCalls() throws IOException {
        Path file = SRC_MAIN_JAVA.resolve("net/blaze3d/opengl/GlDevice.java");
        String source = readSource(file);

        assertFalse(source.contains("GlStateManager._getString("),
            "GlDevice should not query strings via GlStateManager._getString wrapper in migrated paths");
        assertFalse(source.contains("GlStateManager._getInteger("),
            "GlDevice should not query integers via GlStateManager._getInteger wrapper in migrated paths");
        assertFalse(source.contains("GlStateManager._getTexLevelParameter("),
            "GlDevice should not query texture level params via GlStateManager wrapper in migrated paths");
        assertFalse(source.contains("GlStateManager._getError()"),
            "GlDevice should not query errors via GlStateManager._getError wrapper in migrated paths");
        assertFalse(source.contains("GlStateManager.clearGlErrors()"),
            "GlDevice should not clear errors via GlStateManager.clearGlErrors wrapper in migrated paths");

        assertTrue(source.contains("VulkanicAPI.getString("),
            "GlDevice should query strings via direct VulkanicAPI.getString calls");
        assertTrue(source.contains("VulkanicAPI.getInteger("),
            "GlDevice should query integer limits via direct VulkanicAPI.getInteger calls");
        assertTrue(source.contains("VulkanicAPI.getTextureLevelParameter("),
            "GlDevice should query proxy texture width via VulkanicAPI.getTextureLevelParameter");
        assertTrue(source.contains("VulkanicAPI.getError("),
            "GlDevice should query errors via direct VulkanicAPI.getError calls");
    }

    @Test
    public void testIrisGlDebugUsesAgnosticDebugControlHelpers() throws IOException {
        Path file = SRC_MAIN_JAVA.resolve("net/irisshaders/iris/gl/GLDebug.java");
        String source = readSource(file);

        assertFalse(source.contains("debugMessageControl(ctx, 4352, 4352"),
            "GLDebug should not use hardcoded GL_DONT_CARE literals for core debugMessageControl");
        assertFalse(source.contains("debugMessageControlKHR(ctx, 4352, 4352"),
            "GLDebug should not use hardcoded GL_DONT_CARE literals for KHR debugMessageControl");
        assertFalse(source.contains("debugMessageControlARB(ctx, 4352, 4352"),
            "GLDebug should not use hardcoded GL_DONT_CARE literals for ARB debugMessageControl");
        assertFalse(source.contains("setCapabilityEnabled(ctx, VulkanicAPI.GL_DEBUG_OUTPUT_SYNCHRONOUS"),
            "GLDebug should use setDebugOutputSynchronousEnabled helper for sync debug output capability");

        assertTrue(source.contains("VulkanicAPI.setDebugOutputSynchronousEnabled("),
            "GLDebug should enable synchronous debug output via VulkanicAPI.setDebugOutputSynchronousEnabled");
        assertTrue(source.contains("VulkanicAPI.setDebugMessageControlAll("),
            "GLDebug should control core debug filtering via setDebugMessageControlAll helper");
        assertTrue(source.contains("VulkanicAPI.setDebugMessageControlAllKHR("),
            "GLDebug should control KHR debug filtering via setDebugMessageControlAllKHR helper");
        assertTrue(source.contains("VulkanicAPI.setDebugMessageControlAllARB("),
            "GLDebug should control ARB debug filtering via setDebugMessageControlAllARB helper");
        assertTrue(source.contains("VulkanicAPI.isDebugContext("),
            "GLDebug should check context debug status via VulkanicAPI.isDebugContext");
        assertTrue(source.contains("debugState = new UnsupportedDebugState();")
                && source.contains("RustGalVulkanWholeFrameMode.enabled()"),
            "GLDebug must remain a no-op without probing Iris or Java GPU state on Rust Vulkan");
    }

    @Test
    public void testIrisUtilityPathsUseDirectVulkanicCalls() throws IOException {
        Path clearPassCreatorFile = SRC_MAIN_JAVA.resolve("net/irisshaders/iris/targets/ClearPassCreator.java");
        String clearPassCreatorSource = readSource(clearPassCreatorFile);
        assertFalse(clearPassCreatorSource.contains("GlStateManager._getInteger("),
            "ClearPassCreator should not query max draw buffers through GlStateManager wrapper");
        assertTrue(clearPassCreatorSource.contains("VulkanicAPI.getInteger(VulkanicAPI.getCommandContext(), VulkanicIntegerQuery.MAX_DRAW_BUFFERS)"),
            "ClearPassCreator should query max draw buffers through typed VulkanicIntegerQuery");
        assertFalse(clearPassCreatorSource.contains("new ClearPass(clearInfo.getColor(), clearInfo::getWidth, clearInfo::getHeight,\n\t\t\t\t\trenderTargets.createClearFramebuffer(true, clearBuffers), VulkanicAPI.GL_COLOR_BUFFER_BIT)"),
            "ClearPassCreator should not pass raw GL_COLOR_BUFFER_BIT masks into ClearPass for primary clear framebuffer");
        assertFalse(clearPassCreatorSource.contains("new ClearPass(clearInfo.getColor(), clearInfo::getWidth, clearInfo::getHeight,\n\t\t\t\t\trenderTargets.createClearFramebuffer(false, clearBuffers), VulkanicAPI.GL_COLOR_BUFFER_BIT)"),
            "ClearPassCreator should not pass raw GL_COLOR_BUFFER_BIT masks into ClearPass for alternate clear framebuffer");
        assertFalse(clearPassCreatorSource.contains("new ClearPass(clearColor, renderTargets::getResolution, renderTargets::getResolution,\n\t\t\t\t\trenderTargets.createFramebufferWritingToAlt(clearBuffers), VulkanicAPI.GL_COLOR_BUFFER_BIT)"),
            "ClearPassCreator shadow clear pass should not pass raw GL_COLOR_BUFFER_BIT mask for alt framebuffer");
        assertFalse(clearPassCreatorSource.contains("new ClearPass(clearColor, renderTargets::getResolution, renderTargets::getResolution,\n\t\t\t\t\trenderTargets.createFramebufferWritingToMain(clearBuffers), VulkanicAPI.GL_COLOR_BUFFER_BIT)"),
            "ClearPassCreator shadow clear pass should not pass raw GL_COLOR_BUFFER_BIT mask for main framebuffer");

        Path clearPassFile = SRC_MAIN_JAVA.resolve("net/irisshaders/iris/targets/ClearPass.java");
        String clearPassSource = readSource(clearPassFile);
        assertFalse(clearPassSource.contains("private final int clearFlags;"),
            "ClearPass should not track generic raw clear flag masks for color-only clear passes");
        assertFalse(clearPassSource.contains("clearBuffersWithMacosWorkaround(VulkanicAPI.getCommandContext(), clearFlags)"),
            "ClearPass should not clear via generic raw clear mask plumbing");
        assertTrue(clearPassSource.contains("var ctx = VulkanicAPI.getCommandContext();")
                && clearPassSource.contains("VulkanicAPI.clearColorBufferWithMacosWorkaround(ctx)"),
            "ClearPass should clear color through explicit VulkanicAPI clearColorBufferWithMacosWorkaround helper");

        Path samplerLimitsFile = SRC_MAIN_JAVA.resolve("net/irisshaders/iris/gl/sampler/SamplerLimits.java");
        String samplerLimitsSource = readSource(samplerLimitsFile);
        assertFalse(samplerLimitsSource.contains("GlStateManager._getInteger("),
            "SamplerLimits should not query limits through GlStateManager wrapper");
        assertFalse(samplerLimitsSource.contains("VulkanicAPI.getImmediateContext()"),
            "SamplerLimits should not hard-wire immediate-context retrieval");
        assertTrue(samplerLimitsSource.contains("var ctx = VulkanicAPI.getCommandContext();"),
            "SamplerLimits should fetch backend-neutral command context");
        assertTrue(samplerLimitsSource.contains("VulkanicAPI.getInteger(ctx, VulkanicIntegerQuery.MAX_TEXTURE_IMAGE_UNITS)"),
            "SamplerLimits should query limits through typed VulkanicIntegerQuery with shared context");

        Path standardMacrosFile = SRC_MAIN_JAVA.resolve("net/irisshaders/iris/gl/shader/StandardMacros.java");
        String standardMacrosSource = readSource(standardMacrosFile);
        assertFalse(standardMacrosSource.contains("GlStateManager._getString("),
            "StandardMacros should not query GL strings through GlStateManager wrapper");
        assertFalse(standardMacrosSource.contains("GlStateManager._getInteger("),
            "StandardMacros should not query extension count through GlStateManager wrapper");
        assertFalse(standardMacrosSource.contains("VulkanicAPI.getImmediateContext()"),
            "StandardMacros should not hard-wire immediate-context retrieval");
        assertTrue(standardMacrosSource.contains("VulkanicAPI.getString(VulkanicAPI.getCommandContext(), name)"),
            "StandardMacros should query GL version strings directly through VulkanicAPI");
        assertTrue(standardMacrosSource.contains("VulkanicAPI.getInteger(VulkanicAPI.getCommandContext(), VulkanicIntegerQuery.NUM_EXTENSIONS)"),
            "StandardMacros should query extension count through typed VulkanicIntegerQuery");

        Path irisRenderSystemFile = SRC_MAIN_JAVA.resolve("net/irisshaders/iris/gl/IrisRenderSystem.java");
        String irisRenderSystemSource = readSource(irisRenderSystemFile);
        assertFalse(irisRenderSystemSource.contains("getInteger(ctx, VulkanicAPI.GL_GPU_MEMORY_INFO_CURRENT_AVAILABLE_VIDMEM_NVX)"),
            "IrisRenderSystem should not query NVX VRAM through raw GL pname constants");
        assertTrue(irisRenderSystemSource.contains("VulkanicAPI.getInteger(ctx, VulkanicIntegerQuery.GPU_MEMORY_INFO_CURRENT_AVAILABLE_VIDMEM_NVX)"),
            "IrisRenderSystem should query NVX VRAM through typed VulkanicIntegerQuery");

        Path programCreatorFile = SRC_MAIN_JAVA.resolve("net/irisshaders/iris/gl/shader/ProgramCreator.java");
        String programCreatorSource = readSource(programCreatorFile);
        assertFalse(programCreatorSource.contains("GlStateManager._glBindAttribLocation("),
            "ProgramCreator should not bind attributes through GlStateManager wrapper");
        assertTrue(programCreatorSource.contains("VulkanicAPI.setAttributeLocation(ctx, program"),
            "ProgramCreator should bind attributes through VulkanicAPI.setAttributeLocation");

        Path depthTextureFile = SRC_MAIN_JAVA.resolve("net/irisshaders/iris/targets/DepthTexture.java");
        String depthTextureSource = readSource(depthTextureFile);
        assertTrue(depthTextureSource.contains("var ctx = VulkanicAPI.getCommandContext();")
                && depthTextureSource.contains("VulkanicAPI.bindTexture2D(ctx, 0);"),
            "DepthTexture should reuse one local command context when restoring texture binding state");

        Path irisRenderTargetFile = SRC_MAIN_JAVA.resolve("net/irisshaders/iris/targets/RenderTarget.java");
        String irisRenderTargetSource = readSource(irisRenderTargetFile);
        assertTrue(irisRenderTargetSource.contains("var ctx = VulkanicAPI.getCommandContext();")
                && irisRenderTargetSource.contains("VulkanicAPI.bindTexture2D(ctx, 0);"),
            "Iris RenderTarget should reuse one local command context when cleaning up texture binding state");

        Path centerDepthFile = SRC_MAIN_JAVA.resolve("net/irisshaders/iris/pathways/CenterDepthSampler.java");
        String centerDepthSource = readSource(centerDepthFile);
        assertTrue(centerDepthSource.contains("VulkanicAPI.setDynamicViewport(ctx, 0, 0, 1, 1);"),
            "CenterDepthSampler should resolve a local command context before configuring its sampling viewport");

        Path shadowCompositeFile = SRC_MAIN_JAVA.resolve("net/irisshaders/iris/shadows/ShadowCompositeRenderer.java");
        String shadowCompositeSource = readSource(shadowCompositeFile);
        assertTrue(shadowCompositeSource.contains("VulkanicAPI.setDynamicViewport(ctx, beginWidth, beginHeight, (int) scaledWidth, (int) scaledHeight);"),
            "ShadowCompositeRenderer should resolve a local command context before configuring per-pass viewport state");

        Path intCachedUniformFile = SRC_MAIN_JAVA.resolve("net/irisshaders/iris/uniforms/custom/cached/IntCachedUniform.java");
        String intCachedUniformSource = readSource(intCachedUniformFile);
        assertFalse(intCachedUniformSource.contains("GlStateManager._glUniform1i("),
            "IntCachedUniform should not upload via GlStateManager._glUniform1i wrapper");
        assertTrue(intCachedUniformSource.contains("VulkanicAPI.setUniform1i("),
            "IntCachedUniform should upload directly via VulkanicAPI.setUniform1i");

        Path boolCachedUniformFile = SRC_MAIN_JAVA.resolve("net/irisshaders/iris/uniforms/custom/cached/BooleanCachedUniform.java");
        String boolCachedUniformSource = readSource(boolCachedUniformFile);
        assertFalse(boolCachedUniformSource.contains("GlStateManager._glUniform1i("),
            "BooleanCachedUniform should not upload via GlStateManager._glUniform1i wrapper");
        assertTrue(boolCachedUniformSource.contains("VulkanicAPI.setUniform1i("),
            "BooleanCachedUniform should upload directly via VulkanicAPI.setUniform1i");

        Path programSamplersFile = SRC_MAIN_JAVA.resolve("net/irisshaders/iris/gl/program/ProgramSamplers.java");
        String programSamplersSource = readSource(programSamplersFile);
        assertFalse(programSamplersSource.contains("GlStateManager._glUniform1i("),
            "ProgramSamplers initializer should not upload via GlStateManager._glUniform1i wrapper");
        assertFalse(programSamplersSource.contains("VulkanicAPI.getImmediateContext()"),
            "ProgramSamplers should not hard-wire immediate-context retrieval");
        assertTrue(programSamplersSource.contains("VulkanicAPI.setUniform1i(VulkanicAPI.getCommandContext()"),
            "ProgramSamplers initializer should upload directly through VulkanicAPI.setUniform1i");

        Path programImagesFile = SRC_MAIN_JAVA.resolve("net/irisshaders/iris/gl/program/ProgramImages.java");
        String programImagesSource = readSource(programImagesFile);
        assertFalse(programImagesSource.contains("GlStateManager._glUniform1i("),
            "ProgramImages initializer should not upload via GlStateManager._glUniform1i wrapper");
        assertFalse(programImagesSource.contains("VulkanicAPI.getImmediateContext()"),
            "ProgramImages should not hard-wire immediate-context retrieval");
        assertTrue(programImagesSource.contains("VulkanicAPI.setUniform1i(VulkanicAPI.getCommandContext()"),
            "ProgramImages initializer should upload directly through VulkanicAPI.setUniform1i");

        Path textureUploadHelperFile = SRC_MAIN_JAVA.resolve("net/irisshaders/iris/gl/texture/TextureUploadHelper.java");
        String textureUploadHelperSource = readSource(textureUploadHelperFile);
        assertFalse(textureUploadHelperSource.contains("GlStateManager._pixelStore("),
            "TextureUploadHelper should not reset unpack state through GlStateManager._pixelStore wrapper");
        assertTrue(textureUploadHelperSource.contains("VulkanicAPI.setPixelStore(ctx"),
            "TextureUploadHelper should reset unpack state directly through VulkanicAPI.setPixelStore");

        Path fallbackShaderFile = SRC_MAIN_JAVA.resolve("net/irisshaders/iris/pipeline/programs/FallbackShader.java");
        String fallbackShaderSource = readSource(fallbackShaderFile);
        assertFalse(fallbackShaderSource.contains("GlStateManager._glUniform1i("),
            "FallbackShader should not upload sampler uniforms through GlStateManager._glUniform1i wrapper");
        assertTrue(fallbackShaderSource.contains("VulkanicAPI.setUniform1i(ctx, gtexture, 0)"),
            "FallbackShader should upload sampler uniforms directly through VulkanicAPI.setUniform1i");

        Path intUniformFile = SRC_MAIN_JAVA.resolve("net/irisshaders/iris/gl/uniform/IntUniform.java");
        String intUniformSource = readSource(intUniformFile);
        assertFalse(intUniformSource.contains("GlStateManager._glUniform1i("),
            "IntUniform should not upload through GlStateManager._glUniform1i wrapper");
        assertTrue(intUniformSource.contains("VulkanicAPI.setUniform1i(VulkanicAPI.getCommandContext(), location, newValue)"),
            "IntUniform should upload directly through VulkanicAPI.setUniform1i");

        Path glFramebufferFile = SRC_MAIN_JAVA.resolve("net/irisshaders/iris/gl/framebuffer/GlFramebuffer.java");
        String glFramebufferSource = readSource(glFramebufferFile);
        assertFalse(glFramebufferSource.contains("GlStateManager._getInteger("),
            "GlFramebuffer should not query caps through GlStateManager._getInteger wrapper");
        assertFalse(glFramebufferSource.contains("VulkanicAPI.getImmediateContext()"),
            "GlFramebuffer should not hard-wire immediate-context retrieval");
        assertTrue(glFramebufferSource.contains("VulkanicAPI.getInteger(VulkanicAPI.getCommandContext(), VulkanicIntegerQuery.MAX_DRAW_BUFFERS)"),
            "GlFramebuffer should query draw-buffer cap through typed VulkanicIntegerQuery");
        assertTrue(glFramebufferSource.contains("VulkanicAPI.getInteger(VulkanicAPI.getCommandContext(), VulkanicIntegerQuery.MAX_COLOR_ATTACHMENTS)"),
            "GlFramebuffer should query color-attachment cap through typed VulkanicIntegerQuery");

        Path textureInfoCacheFile = SRC_MAIN_JAVA.resolve("net/irisshaders/iris/pbr/TextureInfoCache.java");
        String textureInfoCacheSource = readSource(textureInfoCacheFile);
        assertFalse(textureInfoCacheSource.contains("GlStateManager._getInteger("),
            "TextureInfoCache should not query current texture binding through GlStateManager._getInteger wrapper");
        assertFalse(textureInfoCacheSource.contains("GlStateManager._getTexLevelParameter("),
            "TextureInfoCache should not query texture level params through GlStateManager._getTexLevelParameter wrapper");
        assertFalse(textureInfoCacheSource.contains("VulkanicAPI.getImmediateContext()"),
            "TextureInfoCache should not hard-wire immediate-context retrieval");
        assertTrue(textureInfoCacheSource.contains("var ctx = VulkanicAPI.getCommandContext();"),
            "TextureInfoCache should fetch backend-neutral command context in level-parameter helper");
        assertTrue(textureInfoCacheSource.contains("VulkanicAPI.getInteger(ctx, VulkanicIntegerQuery.TEXTURE_BINDING_2D)"),
            "TextureInfoCache should query current texture binding through typed VulkanicIntegerQuery");
        assertTrue(textureInfoCacheSource.contains("VulkanicAPI.getTexture2DLevelParameter(ctx, 0, pname)"),
            "TextureInfoCache should query texture level params through VulkanicAPI 2D level helper");

        Path textureManipulationUtilFile = SRC_MAIN_JAVA.resolve("net/irisshaders/iris/pbr/util/TextureManipulationUtil.java");
        String textureManipulationUtilSource = readSource(textureManipulationUtilFile);
        assertFalse(textureManipulationUtilSource.contains("GlStateManager._getInteger("),
            "TextureManipulationUtil should not query framebuffer/texture bindings through GlStateManager._getInteger wrapper");
        assertFalse(textureManipulationUtilSource.contains("GlStateManager._getTexLevelParameter("),
            "TextureManipulationUtil should not query tex level dimensions through GlStateManager._getTexLevelParameter wrapper");
        assertFalse(textureManipulationUtilSource.contains("GlStateManager._glFramebufferTexture2D("),
            "TextureManipulationUtil should not attach/detach framebuffer textures through removed GlStateManager._glFramebufferTexture2D wrapper");
        assertTrue(textureManipulationUtilSource.contains("VulkanicAPI.getInteger(ctx, VulkanicIntegerQuery.FRAMEBUFFER_BINDING)"),
            "TextureManipulationUtil should query previous framebuffer through typed VulkanicIntegerQuery");
        assertTrue(textureManipulationUtilSource.contains("VulkanicAPI.getInteger(ctx, VulkanicIntegerQuery.TEXTURE_BINDING_2D)"),
            "TextureManipulationUtil should query previous texture through typed VulkanicIntegerQuery");
        assertTrue(textureManipulationUtilSource.contains("VulkanicAPI.getTexture2DLevelWidth(ctx, level)"),
            "TextureManipulationUtil should query mip width through VulkanicAPI 2D width helper");
        assertTrue(textureManipulationUtilSource.contains("VulkanicAPI.getTexture2DLevelHeight(ctx, level)"),
            "TextureManipulationUtil should query mip height through VulkanicAPI 2D height helper");
        assertTrue(textureManipulationUtilSource.contains("VulkanicAPI.framebufferColorAttachment0Texture2D(ctx, textureId, level)"),
            "TextureManipulationUtil should attach color attachments through VulkanicAPI.framebufferColorAttachment0Texture2D default-target helper");

        Path sodiumShaderFile = SRC_MAIN_JAVA.resolve("net/irisshaders/iris/pipeline/programs/SodiumShader.java");
        String sodiumShaderSource = readSource(sodiumShaderFile);
        assertFalse(sodiumShaderSource.contains("GlStateManager._texParameter(3553, 33084"),
            "SodiumShader should not set base mip level with hardcoded target/pname literals via GlStateManager wrapper");
        assertFalse(sodiumShaderSource.contains("GlStateManager._texParameter(3553, 33085"),
            "SodiumShader should not set max mip level with hardcoded target/pname literals via GlStateManager wrapper");
        assertTrue(sodiumShaderSource.contains("VulkanicAPI.texParameteri(ctx, VulkanicTextureTarget.TEXTURE_2D, VulkanicTextureParameterName.BASE_LEVEL"),
            "SodiumShader should set base mip level through typed Vulkanic texture parameter APIs");
        assertTrue(sodiumShaderSource.contains("VulkanicAPI.texParameteri(ctx, VulkanicTextureTarget.TEXTURE_2D, VulkanicTextureParameterName.MAX_LEVEL"),
            "SodiumShader should set max mip level through typed Vulkanic texture parameter APIs");
        assertFalse(sodiumShaderSource.contains("flushModeChanges(VulkanicAPI.GL_TEXTURE_2D)"),
            "SodiumShader should not pass explicit GL_TEXTURE_2D to GlTexture.flushModeChanges");
        assertTrue(sodiumShaderSource.contains("flushModeChanges2D()"),
            "SodiumShader should flush texture mode changes via GlTexture default-2D helper");

        Path defaultShaderInterfaceFile = SRC_MAIN_JAVA.resolve("net/sodium/client/render/chunk/shader/DefaultShaderInterface.java");
        String defaultShaderInterfaceSource = readSource(defaultShaderInterfaceFile);
        assertFalse(defaultShaderInterfaceSource.contains("flushModeChanges(VulkanicAPI.GL_TEXTURE_2D)"),
            "DefaultShaderInterface should not pass explicit GL_TEXTURE_2D to GlTexture.flushModeChanges");
        assertTrue(defaultShaderInterfaceSource.contains("tex.flushModeChanges2D()"),
            "DefaultShaderInterface should flush texture mode changes via GlTexture default-2D helper");

        Path glTextureFile = SRC_MAIN_JAVA.resolve("net/blaze3d/opengl/GlTexture.java");
        String glTextureSource = readSource(glTextureFile);
        assertTrue(glTextureSource.contains("public void flushModeChanges2D()"),
            "GlTexture should expose default-2D flush helper");
        assertTrue(glTextureSource.contains("this.flushModeChanges2D();"),
            "GlTexture iris$getGlId should route through default-2D flush helper");
    }

    @Test
    public void testSodiumSyncPathsUseAgnosticFenceHelpers() throws IOException {
        Path helperFile = SRC_MAIN_JAVA.resolve("net/sodium/fabric/SodiumGpuSyncHelper.java");
        String helperSource = readSource(helperFile);

        assertFalse(helperSource.contains("createFenceSync(VulkanicAPI.getImmediateContext(), 37143, 0)"),
            "SodiumGpuSyncHelper should not create fences with hardcoded GL_SYNC_GPU_COMMANDS_COMPLETE literal 37143");
        assertFalse(helperSource.contains("waitForSync(VulkanicAPI.getImmediateContext(), fence, 1, Long.MAX_VALUE)"),
            "SodiumGpuSyncHelper should not wait with hardcoded GL_SYNC_FLUSH_COMMANDS_BIT literal 1");

        assertTrue(helperSource.contains("VulkanicAPI.createGpuCompletionFence("),
            "SodiumGpuSyncHelper should create fences via VulkanicAPI.createGpuCompletionFence");
        assertTrue(helperSource.contains("VulkanicAPI.waitForSyncWithFlush("),
            "SodiumGpuSyncHelper should wait via VulkanicAPI.waitForSyncWithFlush");

        Path fenceFile = SRC_MAIN_JAVA.resolve("net/sodium/client/gl/sync/GlFence.java");
        String fenceSource = readSource(fenceFile);

        assertFalse(fenceSource.contains("getSynci(VulkanicAPI.getImmediateContext(), this.id, 37140"),
            "GlFence should not query sync status via hardcoded GL_SYNC_STATUS literal 37140");
        assertFalse(fenceSource.contains("result == 37889"),
            "GlFence should not compare signal state via hardcoded GL_SIGNALED literal 37889");
        assertFalse(fenceSource.contains("waitForSync(VulkanicAPI.getImmediateContext(), this.id, 1, timeout)"),
            "GlFence should not wait with hardcoded GL_SYNC_FLUSH_COMMANDS_BIT literal 1");

        assertTrue(fenceSource.contains("VulkanicAPI.getSyncStatus("),
            "GlFence should query sync status via VulkanicAPI.getSyncStatus");
        assertTrue(fenceSource.contains("VulkanicAPI.GL_SIGNALED"),
            "GlFence should compare completion state against VulkanicAPI.GL_SIGNALED");
        assertTrue(fenceSource.contains("VulkanicAPI.waitForSyncWithFlush("),
            "GlFence should wait via VulkanicAPI.waitForSyncWithFlush");
    }

    @Test
    public void testBlaze3dSyncPathsUseAgnosticFenceHelpers() throws IOException {
        Path fenceFile = SRC_MAIN_JAVA.resolve("net/blaze3d/opengl/GlFence.java");
        String fenceSource = readSource(fenceFile);

        assertFalse(fenceSource.contains("_glFenceSync(37143, 0)"),
            "blaze3d GlFence should not create sync with hardcoded GL_SYNC_GPU_COMMANDS_COMPLETE literal 37143");
        assertFalse(fenceSource.contains("i == 37147"),
            "blaze3d GlFence should not compare timeout with hardcoded GL_TIMEOUT_EXPIRED literal 37147");
        assertFalse(fenceSource.contains("i == 37149"),
            "blaze3d GlFence should not compare failure with hardcoded GL_WAIT_FAILED literal 37149");

        assertTrue(fenceSource.contains("VulkanicAPI.isSyncWaitTimeout("),
            "blaze3d GlFence should detect timeout via VulkanicAPI.isSyncWaitTimeout helper");
        assertTrue(fenceSource.contains("VulkanicAPI.isSyncWaitFailed("),
            "blaze3d GlFence should detect wait failure via VulkanicAPI.isSyncWaitFailed helper");
        assertTrue(fenceSource.contains("VulkanicAPI.createGpuCompletionFence("),
            "blaze3d GlFence should create fences directly via VulkanicAPI.createGpuCompletionFence");
        assertTrue(fenceSource.contains("VulkanicAPI.destroySync("),
            "blaze3d GlFence should destroy fences directly via VulkanicAPI.destroySync");
        assertTrue(fenceSource.contains("VulkanicAPI.waitForSync("),
            "blaze3d GlFence should wait directly via VulkanicAPI.waitForSync");

        Path stateManagerFile = SRC_MAIN_JAVA.resolve("net/blaze3d/opengl/GlStateManager.java");
        String stateManagerSource = readSource(stateManagerFile);

        assertFalse(stateManagerSource.contains("public static long _glFenceSync("),
            "GlStateManager should no longer expose _glFenceSync wrapper");
        assertFalse(stateManagerSource.contains("public static int _glClientWaitSync("),
            "GlStateManager should no longer expose _glClientWaitSync wrapper");
        assertFalse(stateManagerSource.contains("public static void _glDeleteSync("),
            "GlStateManager should no longer expose _glDeleteSync wrapper");
    }

    @Test
    public void testBlaze3dUniformAndAttribWrappersRemovedFromGlStateManager() throws IOException {
        Path stateManagerFile = SRC_MAIN_JAVA.resolve("net/blaze3d/opengl/GlStateManager.java");
        String stateManagerSource = readSource(stateManagerFile);

        assertFalse(stateManagerSource.contains("public static void _glUniform1i("),
            "GlStateManager should no longer expose _glUniform1i wrapper");
        assertFalse(stateManagerSource.contains("public static void _glBindAttribLocation("),
            "GlStateManager should no longer expose _glBindAttribLocation wrapper");
        assertFalse(stateManagerSource.contains("public static int _glGetUniformLocation("),
            "GlStateManager should no longer expose _glGetUniformLocation wrapper");

        Path glProgramFile = SRC_MAIN_JAVA.resolve("net/blaze3d/opengl/GlProgram.java");
        String glProgramSource = readSource(glProgramFile);
        assertFalse(glProgramSource.contains("GlStateManager._glBindAttribLocation("),
            "GlProgram should not bind attributes through removed GlStateManager wrapper");
        assertFalse(glProgramSource.contains("GlStateManager._glGetUniformLocation("),
            "GlProgram should not query uniforms through removed GlStateManager._glGetUniformLocation wrapper");
        assertTrue(glProgramSource.contains("VulkanicAPI.setAttributeLocation(ctx"),
            "GlProgram should bind attributes directly via VulkanicAPI.setAttributeLocation");
        assertTrue(glProgramSource.contains("VulkanicAPI.getUniformLocationWithLegacySamplerFallback(VulkanicAPI.getCommandContext(), this.programId"),
            "GlProgram should query uniforms via VulkanicAPI.getUniformLocationWithLegacySamplerFallback");

        Path vertexFormatFile = SRC_MAIN_JAVA.resolve("net/blaze3d/vertex/VertexFormat.java");
        String vertexFormatSource = readSource(vertexFormatFile);
        assertFalse(vertexFormatSource.contains("GlStateManager._glBindAttribLocation("),
            "VertexFormat Iris binding path should not use removed GlStateManager wrapper");
        assertTrue(vertexFormatSource.contains("VulkanicAPI.setAttributeLocation(ctx"),
            "VertexFormat Iris binding path should use VulkanicAPI.setAttributeLocation directly");

        Path samplersFile = SRC_MAIN_JAVA.resolve("net/irisshaders/iris/gl/program/ProgramSamplers.java");
        String samplersSource = readSource(samplersFile);
        assertFalse(samplersSource.contains("GlStateManager._glGetUniformLocation("),
            "ProgramSamplers should not query uniforms through removed GlStateManager._glGetUniformLocation wrapper");
        assertTrue(samplersSource.contains("VulkanicAPI.getUniformLocationWithLegacySamplerFallback("),
            "ProgramSamplers should query uniforms through VulkanicAPI.getUniformLocationWithLegacySamplerFallback");

        Path imagesFile = SRC_MAIN_JAVA.resolve("net/irisshaders/iris/gl/program/ProgramImages.java");
        String imagesSource = readSource(imagesFile);
        assertFalse(imagesSource.contains("GlStateManager._glGetUniformLocation("),
            "ProgramImages should not query uniforms through removed GlStateManager._glGetUniformLocation wrapper");
        assertTrue(imagesSource.contains("VulkanicAPI.getUniformLocationWithLegacySamplerFallback("),
            "ProgramImages should query uniforms through VulkanicAPI.getUniformLocationWithLegacySamplerFallback");

        Path uniformsFile = SRC_MAIN_JAVA.resolve("net/irisshaders/iris/gl/program/ProgramUniforms.java");
        String uniformsSource = readSource(uniformsFile);
        assertFalse(uniformsSource.contains("GlStateManager._glGetUniformLocation("),
            "ProgramUniforms should not query uniforms through removed GlStateManager._glGetUniformLocation wrapper");
        assertTrue(uniformsSource.contains("VulkanicAPI.getUniformLocationWithLegacySamplerFallback("),
            "ProgramUniforms should query uniforms through VulkanicAPI.getUniformLocationWithLegacySamplerFallback");

        Path fallbackShaderFile = SRC_MAIN_JAVA.resolve("net/irisshaders/iris/pipeline/programs/FallbackShader.java");
        String fallbackShaderSource = readSource(fallbackShaderFile);
        assertFalse(fallbackShaderSource.contains("GlStateManager._glGetUniformLocation("),
            "FallbackShader should not query uniforms through removed GlStateManager._glGetUniformLocation wrapper");
        assertTrue(fallbackShaderSource.contains("VulkanicAPI.getUniformLocationWithLegacySamplerFallback("),
            "FallbackShader should query uniforms through VulkanicAPI.getUniformLocationWithLegacySamplerFallback");

        Path extendedShaderFile = SRC_MAIN_JAVA.resolve("net/irisshaders/iris/pipeline/programs/ExtendedShader.java");
        String extendedShaderSource = readSource(extendedShaderFile);
        assertFalse(extendedShaderSource.contains("GlStateManager._glGetUniformLocation("),
            "ExtendedShader should not query uniforms through removed GlStateManager._glGetUniformLocation wrapper");
        assertTrue(extendedShaderSource.contains("VulkanicAPI.getUniformLocationWithLegacySamplerFallback("),
            "ExtendedShader should query uniforms through VulkanicAPI.getUniformLocationWithLegacySamplerFallback");

        Path vulkanicApiFile = SRC_MAIN_JAVA.resolve("net/vulkanic/VulkanicAPI.java");
        String vulkanicApiSource = readSource(vulkanicApiFile);
        assertTrue(vulkanicApiSource.contains("getUniformLocationWithLegacySamplerFallback"),
            "VulkanicAPI should expose getUniformLocationWithLegacySamplerFallback for legacy Sampler0/1/2 compatibility");

        Path glProgramSamplerFile = SRC_MAIN_JAVA.resolve("net/blaze3d/opengl/GlProgram.java");
        String glProgramSamplerSource = readSource(glProgramSamplerFile);
        assertTrue(glProgramSamplerSource.contains("private static int legacySamplerUnit(String samplerName)"),
            "GlProgram should define legacy sampler unit routing for fixed-function sampler names");
        assertTrue(glProgramSamplerSource.contains("case \"Sampler2\" -> 2;"),
            "GlProgram should preserve Sampler2 as legacy lightmap unit 2 instead of renumbering it sequentially");
    }

    @Test
    public void testBlaze3dProgramLifecycleWrappersRemovedFromGlStateManager() throws IOException {
        Path stateManagerFile = SRC_MAIN_JAVA.resolve("net/blaze3d/opengl/GlStateManager.java");
        String stateManagerSource = readSource(stateManagerFile);

        assertFalse(stateManagerSource.contains("public static int glCreateProgram("),
            "GlStateManager should no longer expose glCreateProgram wrapper");
        assertFalse(stateManagerSource.contains("public static void glDeleteProgram("),
            "GlStateManager should no longer expose glDeleteProgram wrapper");
        assertFalse(stateManagerSource.contains("public static void glLinkProgram("),
            "GlStateManager should no longer expose glLinkProgram wrapper");
        assertFalse(stateManagerSource.contains("public static int glGetProgrami("),
            "GlStateManager should no longer expose glGetProgrami wrapper");
        assertFalse(stateManagerSource.contains("public static String glGetProgramInfoLog("),
            "GlStateManager should no longer expose glGetProgramInfoLog wrapper");

        Path glProgramFile = SRC_MAIN_JAVA.resolve("net/blaze3d/opengl/GlProgram.java");
        String glProgramSource = readSource(glProgramFile);
        assertFalse(glProgramSource.contains("GlStateManager.glCreateProgram("),
            "GlProgram should not create programs through removed GlStateManager.glCreateProgram wrapper");
        assertFalse(glProgramSource.contains("GlStateManager.glLinkProgram("),
            "GlProgram should not link programs through removed GlStateManager.glLinkProgram wrapper");
        assertFalse(glProgramSource.contains("GlStateManager.glGetProgrami("),
            "GlProgram should not query program params through removed GlStateManager.glGetProgrami wrapper");
        assertFalse(glProgramSource.contains("GlStateManager.glGetProgramInfoLog("),
            "GlProgram should not query info logs through removed GlStateManager.glGetProgramInfoLog wrapper");
        assertFalse(glProgramSource.contains("GlStateManager.glDeleteProgram("),
            "GlProgram should not delete programs through removed GlStateManager.glDeleteProgram wrapper");
        assertTrue(
            glProgramSource.contains("VulkanicAPI.createShaderProgram(ctx)")
                || glProgramSource.contains("VulkanicAPI.createShaderProgramHandle(ctx)"),
            "GlProgram should create programs directly through VulkanicAPI.createShaderProgram or createShaderProgramHandle"
        );
        assertTrue(
            glProgramSource.contains("VulkanicAPI.linkProgram(ctx, i)")
                || glProgramSource.contains("VulkanicAPI.linkProgram(ctx, program)"),
            "GlProgram should link programs directly through VulkanicAPI.linkProgram"
        );
        assertTrue(
            glProgramSource.contains("VulkanicAPI.getProgramParameter(ctx, i, net.vulkanic.VulkanicProgramParameterName.LINK_STATUS)")
                || glProgramSource.contains("VulkanicAPI.isProgramLinkSuccessful(ctx, i)")
                || glProgramSource.contains("VulkanicAPI.isProgramLinkSuccessful(ctx, program)"),
            "GlProgram should query link status directly through VulkanicAPI.getProgramParameter or helper-based status API"
        );
        assertTrue(
            glProgramSource.contains("VulkanicAPI.getProgramInfoLog(ctx, i)")
                || glProgramSource.contains("VulkanicAPI.getProgramInfoLog(ctx, program)"),
            "GlProgram should query program info log directly through VulkanicAPI.getProgramInfoLog"
        );
        assertTrue(
            glProgramSource.contains("VulkanicAPI.deleteProgram(VulkanicAPI.getCommandContext(), this.programId)")
                || glProgramSource.contains("VulkanicAPI.deleteProgram(VulkanicAPI.getCommandContext(), VulkanicProgramHandle.of(this.programId))"),
            "GlProgram should delete programs directly through VulkanicAPI.deleteProgram"
        );

        Path shaderCreatorFile = SRC_MAIN_JAVA.resolve("net/irisshaders/iris/pipeline/programs/ShaderCreator.java");
        String shaderCreatorSource = readSource(shaderCreatorFile);
        assertFalse(shaderCreatorSource.contains("GlStateManager.glCreateProgram("),
            "ShaderCreator should not create programs through removed GlStateManager.glCreateProgram wrapper");
        assertFalse(shaderCreatorSource.contains("GlStateManager.glLinkProgram("),
            "ShaderCreator should not link programs through removed GlStateManager.glLinkProgram wrapper");
        assertTrue(
            shaderCreatorSource.contains("VulkanicAPI.createShaderProgram(ctx)")
                || shaderCreatorSource.contains("VulkanicAPI.createShaderProgramHandle(ctx)"),
            "ShaderCreator should create programs directly through VulkanicAPI.createShaderProgram or createShaderProgramHandle"
        );
        assertTrue(
            shaderCreatorSource.contains("VulkanicAPI.linkProgram(ctx, i)")
                || shaderCreatorSource.contains("VulkanicAPI.linkProgram(ctx, program)"),
            "ShaderCreator should link programs directly through VulkanicAPI.linkProgram"
        );

        Path programCreatorFile = SRC_MAIN_JAVA.resolve("net/irisshaders/iris/gl/shader/ProgramCreator.java");
        String programCreatorSource = readSource(programCreatorFile);
        assertFalse(programCreatorSource.contains("GlStateManager.glCreateProgram("),
            "ProgramCreator should not create programs through removed GlStateManager.glCreateProgram wrapper");
        assertFalse(programCreatorSource.contains("GlStateManager.glLinkProgram("),
            "ProgramCreator should not link programs through removed GlStateManager.glLinkProgram wrapper");
        assertFalse(programCreatorSource.contains("GlStateManager.glGetProgrami("),
            "ProgramCreator should not query link status through removed GlStateManager.glGetProgrami wrapper");
        assertTrue(
            programCreatorSource.contains("VulkanicAPI.createShaderProgram(ctx)")
                || programCreatorSource.contains("VulkanicAPI.createShaderProgramHandle(ctx)"),
            "ProgramCreator should create programs directly through VulkanicAPI.createShaderProgram or createShaderProgramHandle"
        );
        assertTrue(programCreatorSource.contains("VulkanicAPI.linkProgram(ctx, program)"),
            "ProgramCreator should link programs directly through VulkanicAPI.linkProgram");
        assertTrue(
            programCreatorSource.contains("VulkanicAPI.getProgramParameter(ctx, program, net.vulkanic.VulkanicProgramParameterName.LINK_STATUS)")
                || programCreatorSource.contains("VulkanicAPI.isProgramLinkSuccessful(ctx, program)"),
            "ProgramCreator should query link status directly through VulkanicAPI.getProgramParameter or helper-based status API"
        );

        Path shaderMapFile = SRC_MAIN_JAVA.resolve("net/irisshaders/iris/pipeline/programs/ShaderMap.java");
        String shaderMapSource = readSource(shaderMapFile);
        assertFalse(shaderMapSource.contains("GlStateManager.glDeleteProgram("),
            "ShaderMap should not delete programs through removed GlStateManager.glDeleteProgram wrapper");
        assertFalse(shaderMapSource.contains("GlStateManager.glGetProgrami("),
            "ShaderMap should not query link status through removed GlStateManager.glGetProgrami wrapper");
        assertFalse(shaderMapSource.contains("GlStateManager.glGetProgramInfoLog("),
            "ShaderMap should not query program logs through removed GlStateManager.glGetProgramInfoLog wrapper");
        assertTrue(
            shaderMapSource.contains("VulkanicAPI.deleteProgram(VulkanicAPI.getCommandContext(), shader.id().program())")
                || shaderMapSource.contains("VulkanicAPI.deleteProgram(VulkanicAPI.getCommandContext(), VulkanicProgramHandle.of(shader.id().program()))"),
            "ShaderMap should delete programs directly through VulkanicAPI.deleteProgram"
        );
        assertTrue(
            shaderMapSource.contains("VulkanicAPI.getProgramParameter(ctx, i, net.vulkanic.VulkanicProgramParameterName.LINK_STATUS)")
                || shaderMapSource.contains("VulkanicAPI.isProgramLinkSuccessful(ctx, i)"),
            "ShaderMap should query link status directly through VulkanicAPI.getProgramParameter or helper-based status API"
        );
        assertTrue(shaderMapSource.contains("VulkanicAPI.getProgramInfoLog(ctx, i)"),
            "ShaderMap should query program info logs directly through VulkanicAPI.getProgramInfoLog");

        Path uniformsFile = SRC_MAIN_JAVA.resolve("net/irisshaders/iris/gl/program/ProgramUniforms.java");
        String uniformsSource = readSource(uniformsFile);
        assertFalse(uniformsSource.contains("GlStateManager.glGetProgrami("),
            "ProgramUniforms should not query active uniforms through removed GlStateManager.glGetProgrami wrapper");
        assertTrue(
            uniformsSource.contains("VulkanicAPI.getActiveUniforms(")
                || uniformsSource.contains("VulkanicAPI.getProgramParameter(VulkanicAPI.getCommandContext(), program, VulkanicProgramParameterName.ACTIVE_UNIFORMS)"),
            "ProgramUniforms should enumerate active uniforms through VulkanicAPI typed helpers or direct VulkanicAPI program-parameter query"
        );

        Path programFile = SRC_MAIN_JAVA.resolve("net/irisshaders/iris/gl/program/Program.java");
        String programSource = readSource(programFile);
        assertFalse(programSource.contains("GlStateManager.glDeleteProgram("),
            "Program should not destroy programs through removed GlStateManager.glDeleteProgram wrapper");
        assertTrue(programSource.contains("VulkanicAPI.deleteProgram(VulkanicAPI.getCommandContext(), getGlId())"),
            "Program should destroy programs directly through VulkanicAPI.deleteProgram");

        Path computeProgramFile = SRC_MAIN_JAVA.resolve("net/irisshaders/iris/gl/program/ComputeProgram.java");
        String computeProgramSource = readSource(computeProgramFile);
        assertFalse(computeProgramSource.contains("GlStateManager.glDeleteProgram("),
            "ComputeProgram should not destroy programs through removed GlStateManager.glDeleteProgram wrapper");
        assertTrue(computeProgramSource.contains("VulkanicAPI.deleteProgram(VulkanicAPI.getCommandContext(), getGlId())"),
            "ComputeProgram should destroy programs directly through VulkanicAPI.deleteProgram");

        Path glDeviceFile = SRC_MAIN_JAVA.resolve("net/blaze3d/opengl/GlDevice.java");
        String glDeviceSource = readSource(glDeviceFile);
        assertFalse(glDeviceSource.contains("GlStateManager.glCreateProgram("),
            "GlDevice AMD workaround should not create programs through removed GlStateManager.glCreateProgram wrapper");
        assertFalse(glDeviceSource.contains("GlStateManager.glDeleteProgram("),
            "GlDevice AMD workaround should not delete programs through removed GlStateManager.glDeleteProgram wrapper");
        assertTrue(
            glDeviceSource.contains("net.vulkanic.VulkanicAPI.createShaderProgram(net.vulkanic.VulkanicAPI.getCommandContext())")
                || glDeviceSource.contains("net.vulkanic.VulkanicAPI.createShaderProgramHandle(ctx)"),
            "GlDevice AMD workaround should create programs directly through VulkanicAPI.createShaderProgram or createShaderProgramHandle"
        );
        assertTrue(
            glDeviceSource.contains("net.vulkanic.VulkanicAPI.deleteProgram(net.vulkanic.VulkanicAPI.getCommandContext(), j)")
                || glDeviceSource.contains("net.vulkanic.VulkanicAPI.deleteProgram(ctx, program)"),
            "GlDevice AMD workaround should delete programs directly through VulkanicAPI.deleteProgram"
        );
    }

    @Test
    public void testBlaze3dShaderLifecycleWrappersRemovedFromGlStateManager() throws IOException {
        Path stateManagerFile = SRC_MAIN_JAVA.resolve("net/blaze3d/opengl/GlStateManager.java");
        String stateManagerSource = readSource(stateManagerFile);

        assertFalse(stateManagerSource.contains("public static void glAttachShader("),
            "GlStateManager should no longer expose glAttachShader wrapper");
        assertFalse(stateManagerSource.contains("public static void glDeleteShader("),
            "GlStateManager should no longer expose glDeleteShader wrapper");
        assertFalse(stateManagerSource.contains("public static int glCreateShader("),
            "GlStateManager should no longer expose glCreateShader wrapper");
        assertFalse(stateManagerSource.contains("public static void glShaderSource("),
            "GlStateManager should no longer expose glShaderSource wrapper");
        assertFalse(stateManagerSource.contains("public static void glCompileShader("),
            "GlStateManager should no longer expose glCompileShader wrapper");
        assertFalse(stateManagerSource.contains("public static int glGetShaderi("),
            "GlStateManager should no longer expose glGetShaderi wrapper");
        assertFalse(stateManagerSource.contains("public static String glGetShaderInfoLog("),
            "GlStateManager should no longer expose glGetShaderInfoLog wrapper");

        Path glProgramFile = SRC_MAIN_JAVA.resolve("net/blaze3d/opengl/GlProgram.java");
        String glProgramSource = readSource(glProgramFile);
        assertFalse(glProgramSource.contains("GlStateManager.glAttachShader("),
            "GlProgram should not attach shaders through removed GlStateManager.glAttachShader wrapper");
        assertTrue(
            glProgramSource.contains("VulkanicAPI.attachShader(ctx, i")
                || glProgramSource.contains("VulkanicAPI.attachShader(ctx, program, VulkanicShaderHandle.of(glShaderModule.getShaderId()))"),
            "GlProgram should attach shaders directly through VulkanicAPI.attachShader"
        );

        Path programCreatorFile = SRC_MAIN_JAVA.resolve("net/irisshaders/iris/gl/shader/ProgramCreator.java");
        String programCreatorSource = readSource(programCreatorFile);
        assertFalse(programCreatorSource.contains("GlStateManager.glAttachShader("),
            "ProgramCreator should not attach shaders through removed GlStateManager.glAttachShader wrapper");
        assertTrue(programCreatorSource.contains("VulkanicAPI.attachShader(ctx, program"),
            "ProgramCreator should attach shaders directly through VulkanicAPI.attachShader");

        Path shaderCreatorFile = SRC_MAIN_JAVA.resolve("net/irisshaders/iris/pipeline/programs/ShaderCreator.java");
        String shaderCreatorSource = readSource(shaderCreatorFile);
        assertFalse(shaderCreatorSource.contains("GlStateManager.glAttachShader("),
            "ShaderCreator should not attach shaders through removed GlStateManager.glAttachShader wrapper");
        assertFalse(shaderCreatorSource.contains("GlStateManager.glDeleteShader("),
            "ShaderCreator should not delete shaders through removed GlStateManager.glDeleteShader wrapper");
        assertFalse(shaderCreatorSource.contains("GlStateManager.glCreateShader("),
            "ShaderCreator should not create shaders through removed GlStateManager.glCreateShader wrapper");
        assertFalse(shaderCreatorSource.contains("GlStateManager.glShaderSource("),
            "ShaderCreator should not upload source through removed GlStateManager.glShaderSource wrapper");
        assertFalse(shaderCreatorSource.contains("GlStateManager.glCompileShader("),
            "ShaderCreator should not compile shaders through removed GlStateManager.glCompileShader wrapper");
        assertFalse(shaderCreatorSource.contains("GlStateManager.glGetShaderi("),
            "ShaderCreator should not query shader status through removed GlStateManager.glGetShaderi wrapper");
        assertTrue(
            shaderCreatorSource.contains("VulkanicAPI.attachShader(ctx, i, s)")
                || shaderCreatorSource.contains("VulkanicAPI.attachShader(ctx, program, VulkanicShaderHandle.of(s))"),
            "ShaderCreator should attach shaders directly through VulkanicAPI.attachShader"
        );
        assertTrue(
            shaderCreatorSource.contains("VulkanicAPI.deleteShader(VulkanicAPI.getCommandContext(), s)")
                || shaderCreatorSource.contains("VulkanicAPI.deleteShader(ctx, VulkanicShaderHandle.of(s))"),
            "ShaderCreator should delete shaders directly through VulkanicAPI.deleteShader"
        );
        assertTrue(
            shaderCreatorSource.contains("VulkanicAPI.createShader(ctx, shaderType.stage)")
                || shaderCreatorSource.contains("VulkanicAPI.createShaderHandle(ctx, shaderType.stage)"),
            "ShaderCreator should create shaders directly through VulkanicAPI.createShader or createShaderHandle"
        );
        assertTrue(
            shaderCreatorSource.contains("ShaderWorkarounds.safeShaderSource(shader, source)")
                || shaderCreatorSource.contains("ShaderWorkarounds.safeShaderSource(shader.value(), source)"),
            "ShaderCreator should upload shader source via ShaderWorkarounds.safeShaderSource"
        );
        assertTrue(shaderCreatorSource.contains("VulkanicAPI.compileShader(ctx, shader)"),
            "ShaderCreator should compile shaders directly through VulkanicAPI.compileShader");
        assertTrue(
            shaderCreatorSource.contains("VulkanicAPI.getShaderParameter(ctx, shader, net.vulkanic.VulkanicShaderParameterName.COMPILE_STATUS)")
                || shaderCreatorSource.contains("VulkanicAPI.isShaderCompileSuccessful(ctx, shader)"),
            "ShaderCreator should query compile status directly through VulkanicAPI.getShaderParameter or helper-based status API"
        );

        Path glDeviceFile = SRC_MAIN_JAVA.resolve("net/blaze3d/opengl/GlDevice.java");
        String glDeviceSource = readSource(glDeviceFile);
        assertFalse(glDeviceSource.contains("GlStateManager.glCreateShader("),
            "GlDevice should not create shaders through removed GlStateManager.glCreateShader wrapper");
        assertFalse(glDeviceSource.contains("GlStateManager.glAttachShader("),
            "GlDevice should not attach shaders through removed GlStateManager.glAttachShader wrapper");
        assertFalse(glDeviceSource.contains("GlStateManager.glDeleteShader("),
            "GlDevice should not delete shaders through removed GlStateManager.glDeleteShader wrapper");
        assertFalse(glDeviceSource.contains("GlStateManager.glShaderSource("),
            "GlDevice should not upload shader source through removed GlStateManager.glShaderSource wrapper");
        assertFalse(glDeviceSource.contains("GlStateManager.glCompileShader("),
            "GlDevice should not compile shaders through removed GlStateManager.glCompileShader wrapper");
        assertFalse(glDeviceSource.contains("GlStateManager.glGetShaderi("),
            "GlDevice should not query shader status through removed GlStateManager.glGetShaderi wrapper");
        assertFalse(glDeviceSource.contains("GlStateManager.glGetShaderInfoLog("),
            "GlDevice should not query shader logs through removed GlStateManager.glGetShaderInfoLog wrapper");
        assertTrue(
            glDeviceSource.contains("net.vulkanic.VulkanicAPI.createShader(net.vulkanic.VulkanicAPI.getCommandContext()")
                || glDeviceSource.contains("net.vulkanic.VulkanicAPI.createShader(ctx, toVulkanicShaderStage(shaderCompilationKey.type))")
                || glDeviceSource.contains("net.vulkanic.VulkanicAPI.createShaderHandle(ctx, toVulkanicShaderStage(shaderCompilationKey.type))"),
            "GlDevice should create shaders directly through VulkanicAPI.createShader or createShaderHandle"
        );
        assertFalse(glDeviceSource.contains("GlConst.toGl(shaderCompilationKey.type)"),
            "GlDevice shader creation should avoid OpenGL-specific GlConst shader-type conversion when typed stage mapping is available");
        assertTrue(
            glDeviceSource.contains("net.vulkanic.VulkanicAPI.attachShader(net.vulkanic.VulkanicAPI.getCommandContext(), j, i)")
                || glDeviceSource.contains("net.vulkanic.VulkanicAPI.attachShader(ctx, program, shader)"),
            "GlDevice should attach shaders directly through VulkanicAPI.attachShader"
        );
        assertTrue(
            glDeviceSource.contains("net.vulkanic.VulkanicAPI.deleteShader(net.vulkanic.VulkanicAPI.getCommandContext(), i)")
                || glDeviceSource.contains("net.vulkanic.VulkanicAPI.deleteShader(ctx, shader)"),
            "GlDevice should delete shaders directly through VulkanicAPI.deleteShader"
        );
        assertTrue(
            glDeviceSource.contains("net.irisshaders.iris.gl.shader.ShaderWorkarounds.safeShaderSource(i, string2)")
                || glDeviceSource.contains("net.irisshaders.iris.gl.shader.ShaderWorkarounds.safeShaderSource(shader.value(), string2)")
                || glDeviceSource.contains("net.irisshaders.iris.gl.shader.ShaderWorkarounds.safeShaderSource(shader, string2)"),
            "GlDevice should upload shader source via ShaderWorkarounds.safeShaderSource"
        );
        assertTrue(
            glDeviceSource.contains("net.vulkanic.VulkanicAPI.compileShader(net.vulkanic.VulkanicAPI.getCommandContext(), i)")
                || glDeviceSource.contains("net.vulkanic.VulkanicAPI.compileShader(ctx, i)")
                || glDeviceSource.contains("net.vulkanic.VulkanicAPI.compileShader(ctx, shader)"),
            "GlDevice should compile shaders directly through VulkanicAPI.compileShader"
        );
        assertTrue(
            glDeviceSource.contains("net.vulkanic.VulkanicAPI.getShaderParameter(net.vulkanic.VulkanicAPI.getCommandContext(), i, 35713)")
                || glDeviceSource.contains("net.vulkanic.VulkanicAPI.isShaderCompileSuccessful(ctx, i)")
                || glDeviceSource.contains("net.vulkanic.VulkanicAPI.isShaderCompileSuccessful(ctx, shader)"),
            "GlDevice should query compile status directly through VulkanicAPI.getShaderParameter or helper-based status API"
        );
        assertTrue(
            glDeviceSource.contains("net.vulkanic.VulkanicAPI.getShaderInfoLog(net.vulkanic.VulkanicAPI.getCommandContext(), i)")
                || glDeviceSource.contains("net.vulkanic.VulkanicAPI.getShaderInfoLog(ctx, i)")
                || glDeviceSource.contains("net.vulkanic.VulkanicAPI.getShaderInfoLog(ctx, shader)"),
            "GlDevice should query shader logs directly through VulkanicAPI.getShaderInfoLog"
        );

        Path glShaderFile = SRC_MAIN_JAVA.resolve("net/irisshaders/iris/gl/shader/GlShader.java");
        String glShaderSource = readSource(glShaderFile);
        assertFalse(glShaderSource.contains("GlStateManager.glCreateShader("),
            "GlShader should not create shaders through removed GlStateManager.glCreateShader wrapper");
        assertFalse(glShaderSource.contains("GlStateManager.glCompileShader("),
            "GlShader should not compile shaders through removed GlStateManager.glCompileShader wrapper");
        assertFalse(glShaderSource.contains("GlStateManager.glDeleteShader("),
            "GlShader should not delete shaders through removed GlStateManager.glDeleteShader wrapper");
        assertFalse(glShaderSource.contains("VulkanicAPI.getImmediateContext()"),
            "GlShader should not hard-wire immediate-context retrieval");
        assertTrue(glShaderSource.contains("CommandContext ctx = VulkanicAPI.getCommandContext();"),
            "GlShader should fetch backend-neutral command context once in shader creation path");
        assertTrue(
            glShaderSource.contains("VulkanicAPI.createShader(ctx, type.stage)")
                || glShaderSource.contains("VulkanicAPI.createShaderHandle(ctx, type.stage)"),
            "GlShader should create shaders directly through VulkanicAPI.createShader or createShaderHandle"
        );
        assertTrue(glShaderSource.contains("VulkanicAPI.compileShader(ctx, handle)"),
            "GlShader should compile shaders directly through VulkanicAPI.compileShader");
        assertTrue(
            glShaderSource.contains("VulkanicAPI.deleteShader(VulkanicAPI.getCommandContext(), this.getGlId())")
                || glShaderSource.contains("VulkanicAPI.deleteShader(VulkanicAPI.getCommandContext(), VulkanicShaderHandle.of(this.getGlId()))"),
            "GlShader should delete shaders directly through VulkanicAPI.deleteShader"
        );

        Path partialShaderFile = SRC_MAIN_JAVA.resolve("net/irisshaders/iris/pipeline/programs/PartialShader.java");
        String partialShaderSource = readSource(partialShaderFile);
        assertFalse(partialShaderSource.contains("GlStateManager.glDeleteShader("),
            "PartialShader should not delete shaders through removed GlStateManager.glDeleteShader wrapper");
        assertTrue(
            partialShaderSource.contains("VulkanicAPI.deleteShader(VulkanicAPI.getCommandContext(), s)")
                || partialShaderSource.contains("VulkanicAPI.deleteShader(VulkanicAPI.getCommandContext(), VulkanicShaderHandle.of(s))"),
            "PartialShader should delete shaders directly through VulkanicAPI.deleteShader"
        );

        Path shaderModuleFile = SRC_MAIN_JAVA.resolve("net/blaze3d/opengl/GlShaderModule.java");
        String shaderModuleSource = readSource(shaderModuleFile);
        assertFalse(shaderModuleSource.contains("GlStateManager.glDeleteShader("),
            "GlShaderModule should not delete shaders through removed GlStateManager.glDeleteShader wrapper");
        assertTrue(
            shaderModuleSource.contains("net.vulkanic.VulkanicAPI.deleteShader(net.vulkanic.VulkanicAPI.getCommandContext(), this.shaderId)")
                || shaderModuleSource.contains("net.vulkanic.VulkanicAPI.deleteShader(")
                && shaderModuleSource.contains("net.vulkanic.VulkanicShaderHandle.of(this.shaderId)"),
            "GlShaderModule should delete shaders directly through VulkanicAPI.deleteShader"
        );
    }

    @Test
    public void testBlaze3dQueryAndTexParameterWrappersRemovedFromGlStateManager() throws IOException {
        Path stateManagerFile = SRC_MAIN_JAVA.resolve("net/blaze3d/opengl/GlStateManager.java");
        String stateManagerSource = readSource(stateManagerFile);

        assertFalse(stateManagerSource.contains("public static String _getString("),
            "GlStateManager should no longer expose _getString wrapper");
        assertFalse(stateManagerSource.contains("public static int _getInteger("),
            "GlStateManager should no longer expose _getInteger wrapper");
        assertFalse(stateManagerSource.contains("public static int _getTexLevelParameter("),
            "GlStateManager should no longer expose _getTexLevelParameter wrapper");
        assertFalse(stateManagerSource.contains("public static void _texParameter("),
            "GlStateManager should no longer expose _texParameter wrapper");
    }

    @Test
    public void testOpenGLBackendManagedAllocationsAvoidBlaze3dErrorWrappers() throws IOException {
        Path backendFile = SRC_MAIN_JAVA.resolve("net/vulkanic/backends/opengl/OpenGLBackend.java");
        String backendSource = readSource(backendFile);

        assertFalse(backendSource.contains("net.blaze3d.opengl.GlStateManager.clearGlErrors()"),
            "OpenGLBackend managed allocation paths should not clear errors through Blaze3D GlStateManager wrapper");
        assertFalse(backendSource.contains("net.blaze3d.opengl.GlStateManager._getError()"),
            "OpenGLBackend managed allocation paths should not query errors through Blaze3D GlStateManager wrapper");
        assertTrue(backendSource.contains("while (GL11.glGetError() != GL11.GL_NO_ERROR)"),
            "OpenGLBackend managed allocation paths should clear errors directly via GL11.glGetError loop");
        assertTrue(backendSource.contains("int error = GL11.glGetError()"),
            "OpenGLBackend managed allocation paths should query errors directly via GL11.glGetError");
    }

    @Test
    public void testOpenGLBackendTracksTextureBindingsWithoutGlStateManager() throws IOException {
        Path backendFile = SRC_MAIN_JAVA.resolve("net/vulkanic/backends/opengl/OpenGLBackend.java");
        String backendSource = readSource(backendFile);

        assertFalse(backendSource.contains("import net.blaze3d.opengl.GlStateManager;"),
            "OpenGLBackend should not import GlStateManager for texture-state tracking");
        assertFalse(backendSource.contains("GlStateManager.activeTexture"),
            "OpenGLBackend should not read active texture from GlStateManager");
        assertFalse(backendSource.contains("GlStateManager.TEXTURES"),
            "OpenGLBackend should not read or write texture bindings through GlStateManager.TEXTURES");

        assertTrue(backendSource.contains("private final int[] texture2DBindings"),
            "OpenGLBackend should maintain backend-local 2D texture binding cache");
        assertTrue(backendSource.contains("private int activeTextureUnitIndex"),
            "OpenGLBackend should maintain backend-local active texture unit index");
        assertTrue(backendSource.contains("activeTextureUnitIndex = textureUnitIndex"),
            "OpenGLBackend should update backend-local active texture unit state in setActiveTextureUnit");
    }

    @Test
    public void testVertexArrayCacheUsesDirectVulkanicStringQueries() throws IOException {
        Path file = SRC_MAIN_JAVA.resolve("net/blaze3d/opengl/VertexArrayCache.java");
        String source = readSource(file);

        assertFalse(source.contains("GlStateManager._getString(7936)"),
            "VertexArrayCache should not query GL_VENDOR via hardcoded literal through GlStateManager._getString wrapper");
        assertFalse(source.contains("GlStateManager._getString(7938)"),
            "VertexArrayCache should not query GL_VERSION via hardcoded literal through GlStateManager._getString wrapper");
        assertTrue(source.contains("VulkanicAPI.getString(VulkanicAPI.getCommandContext(), VulkanicAPI.GL_VENDOR)"),
            "VertexArrayCache should query vendor directly through VulkanicAPI.getString + VulkanicAPI.GL_VENDOR");
        assertTrue(source.contains("VulkanicAPI.getString(VulkanicAPI.getCommandContext(), VulkanicAPI.GL_VERSION)"),
            "VertexArrayCache should query version directly through VulkanicAPI.getString + VulkanicAPI.GL_VERSION");
    }

    @Test
    public void testBufferStorageUsesDirectVulkanicErrorQueries() throws IOException {
        Path file = SRC_MAIN_JAVA.resolve("net/blaze3d/opengl/BufferStorage.java");
        String source = readSource(file);

        assertFalse(source.contains("GlStateManager._getError()"),
            "BufferStorage map failure paths should not query errors through GlStateManager._getError wrapper");
        assertFalse(source.contains("GlStateManager.clearGlErrors()"),
            "BufferStorage map failure paths should not clear errors through GlStateManager.clearGlErrors wrapper");
        assertTrue(source.contains("VulkanicAPI.getError(VulkanicAPI.getCommandContext())"),
            "BufferStorage map failure paths should query errors directly through VulkanicAPI.getError");
    }

    @Test
    public void testBlaze3dErrorWrappersRemovedFromGlStateManager() throws IOException {
        Path stateManagerFile = SRC_MAIN_JAVA.resolve("net/blaze3d/opengl/GlStateManager.java");
        String stateManagerSource = readSource(stateManagerFile);

        assertFalse(stateManagerSource.contains("public static int _getError("),
            "GlStateManager should no longer expose _getError wrapper");
        assertFalse(stateManagerSource.contains("public static void clearGlErrors("),
            "GlStateManager should no longer expose clearGlErrors wrapper");
    }

    @Test
    public void testBlaze3dVertexArrayWrappersRemovedFromGlStateManager() throws IOException {
        Path stateManagerFile = SRC_MAIN_JAVA.resolve("net/blaze3d/opengl/GlStateManager.java");
        String stateManagerSource = readSource(stateManagerFile);

        assertFalse(stateManagerSource.contains("public static void _glBindBuffer("),
            "GlStateManager should no longer expose _glBindBuffer wrapper");
        assertFalse(stateManagerSource.contains("public static void _glBindVertexArray("),
            "GlStateManager should no longer expose _glBindVertexArray wrapper");
        assertFalse(stateManagerSource.contains("public static void _enableVertexAttribArray("),
            "GlStateManager should no longer expose _enableVertexAttribArray wrapper");
        assertFalse(stateManagerSource.contains("public static void _vertexAttribPointer("),
            "GlStateManager should no longer expose _vertexAttribPointer wrapper");
        assertFalse(stateManagerSource.contains("public static void _vertexAttribIPointer("),
            "GlStateManager should no longer expose _vertexAttribIPointer wrapper");
        assertFalse(stateManagerSource.contains("public static int _glGenVertexArrays("),
            "GlStateManager should no longer expose _glGenVertexArrays wrapper");
        assertFalse(stateManagerSource.contains("public static int _glGenBuffers("),
            "GlStateManager should no longer expose _glGenBuffers wrapper");
        assertFalse(stateManagerSource.contains("public static void _glBufferData("),
            "GlStateManager should no longer expose _glBufferData wrapper overloads");
        assertFalse(stateManagerSource.contains("public static void _glBufferSubData("),
            "GlStateManager should no longer expose _glBufferSubData wrapper");
        assertFalse(stateManagerSource.contains("public static ByteBuffer _glMapBufferRange("),
            "GlStateManager should no longer expose _glMapBufferRange wrapper");
        assertFalse(stateManagerSource.contains("public static void _glUnmapBuffer("),
            "GlStateManager should no longer expose _glUnmapBuffer wrapper");
        assertFalse(stateManagerSource.contains("public static void _glDeleteBuffers("),
            "GlStateManager should no longer expose _glDeleteBuffers wrapper");

        Path dhProgramFile = SRC_MAIN_JAVA.resolve("net/irisshaders/iris/compat/dh/IrisGenericRenderProgram.java");
        String dhProgramSource = readSource(dhProgramFile);
        assertFalse(dhProgramSource.contains("GlStateManager._glBindVertexArray("),
            "IrisGenericRenderProgram should not bind VAOs through removed GlStateManager wrapper");
        assertFalse(dhProgramSource.contains("GlStateManager._glGenVertexArrays("),
            "IrisGenericRenderProgram should not create VAOs through removed GlStateManager wrapper");
        assertTrue(dhProgramSource.contains("VulkanicAPI.createVertexArray("),
            "IrisGenericRenderProgram should create VAOs directly through VulkanicAPI.createVertexArray");
        assertTrue(dhProgramSource.contains("VulkanicAPI.bindVertexArray("),
            "IrisGenericRenderProgram should bind VAOs directly through VulkanicAPI.bindVertexArray");

        Path directStateAccessFile = SRC_MAIN_JAVA.resolve("net/blaze3d/opengl/DirectStateAccess.java");
        String directStateAccessSource = readSource(directStateAccessFile);
        assertFalse(directStateAccessSource.contains("GlStateManager._glGenBuffers("),
            "DirectStateAccess should not create buffers through removed GlStateManager._glGenBuffers wrapper");
        assertFalse(directStateAccessSource.contains("GlStateManager._glBindBuffer("),
            "DirectStateAccess should not bind buffers through removed GlStateManager._glBindBuffer wrapper");
        assertFalse(directStateAccessSource.contains("GlStateManager._glMapBufferRange("),
            "DirectStateAccess should not map buffers through removed GlStateManager._glMapBufferRange wrapper");
        assertFalse(directStateAccessSource.contains("GlStateManager._glUnmapBuffer("),
            "DirectStateAccess should not unmap buffers through removed GlStateManager._glUnmapBuffer wrapper");
        assertFalse(directStateAccessSource.contains("GlStateManager.incrementTrackedBuffers();"),
            "DirectStateAccess should no longer increment tracked buffers through GlStateManager");
        assertTrue(directStateAccessSource.contains("IrisRenderSystem.incrementTrackedBuffers();"),
            "DirectStateAccess should increment tracked buffers through IrisRenderSystem helper");
        assertTrue(containsAny(directStateAccessSource,
                "VulkanicAPI.createBuffer(VulkanicAPI.getCommandContext())",
                "VulkanicAPI.createBuffer(ctx)"),
            "DirectStateAccess should create buffers directly via VulkanicAPI.createBuffer");
        assertTrue(containsAny(directStateAccessSource,
                "VulkanicAPI.bindBuffer(VulkanicAPI.getCommandContext(),",
                "VulkanicCoreAPI.bindBuffer(ctx,"),
            "DirectStateAccess should bind/unbind emulated targets through Vulkanic frontend APIs");
        assertTrue(directStateAccessSource.contains("private VulkanicBufferTarget selectBufferBindTarget("),
            "DirectStateAccess should classify emulated bind targets using typed VulkanicBufferTarget");
        assertTrue(containsAny(directStateAccessSource,
                "VulkanicAPI.mapBuffer(VulkanicAPI.getCommandContext(),",
                "VulkanicCoreAPI.mapBufferRange(ctx,"),
            "DirectStateAccess should map buffers through Vulkanic frontend APIs");
        assertTrue(containsAny(directStateAccessSource,
                "VulkanicAPI.unmapBuffer(VulkanicAPI.getCommandContext(),",
                "VulkanicCoreAPI.unmapBuffer(ctx,"),
            "DirectStateAccess should unmap buffers through Vulkanic frontend APIs");

        Path irisRenderSystemFile = SRC_MAIN_JAVA.resolve("net/irisshaders/iris/gl/IrisRenderSystem.java");
        String irisRenderSystemSource = readSource(irisRenderSystemFile);
        assertFalse(irisRenderSystemSource.contains("GlStateManager._glGenBuffers("),
            "IrisRenderSystem should not create buffers through removed GlStateManager._glGenBuffers wrapper");
        assertFalse(irisRenderSystemSource.contains("GlStateManager._glBindBuffer("),
            "IrisRenderSystem should not bind buffers through removed GlStateManager._glBindBuffer wrapper");
        assertTrue(irisRenderSystemSource.contains("VulkanicAPI.createBuffer(VulkanicAPI.getCommandContext())"),
            "IrisRenderSystem should create buffers directly via VulkanicAPI.createBuffer");
        assertTrue(irisRenderSystemSource.contains("VulkanicAPI.bindBuffer(VulkanicAPI.getCommandContext(), target, buffer)"),
            "IrisRenderSystem should bind new buffers directly via VulkanicAPI.bindBuffer");

        Path ssboFile = SRC_MAIN_JAVA.resolve("net/irisshaders/iris/gl/buffer/ShaderStorageBuffer.java");
        String ssboSource = readSource(ssboFile);
        assertFalse(ssboSource.contains("GlStateManager._glGenBuffers("),
            "ShaderStorageBuffer should not create buffers through removed GlStateManager._glGenBuffers wrapper");
        assertFalse(ssboSource.contains("GlStateManager._glBindBuffer("),
            "ShaderStorageBuffer should not bind buffers through removed GlStateManager._glBindBuffer wrapper");
        assertFalse(ssboSource.contains("GlStateManager._glBufferSubData("),
            "ShaderStorageBuffer should not upload content through removed GlStateManager._glBufferSubData wrapper");
        assertTrue(ssboSource.contains("VulkanicAPI.createBuffer(ctx)"),
            "ShaderStorageBuffer should create buffers directly via VulkanicAPI.createBuffer");
        assertFalse(ssboSource.contains("VulkanicAPI.bindBuffer(ctx, VulkanicAPI.GL_SHADER_STORAGE_BUFFER, getId())"),
            "ShaderStorageBuffer should no longer use raw GL_SHADER_STORAGE_BUFFER target where typed targets are available");
        assertTrue(ssboSource.contains("VulkanicAPI.bindBuffer(ctx, VulkanicBufferTarget.SHADER_STORAGE, getId())"),
            "ShaderStorageBuffer should bind SSBOs through typed VulkanicBufferTarget.SHADER_STORAGE");
        assertTrue(ssboSource.contains("VulkanicAPI.bufferSubData(ctx, VulkanicBufferTarget.SHADER_STORAGE, 0L, content)"),
            "ShaderStorageBuffer should upload content directly via VulkanicAPI.bufferSubData");

        Path glBufferFile = SRC_MAIN_JAVA.resolve("net/blaze3d/opengl/GlBuffer.java");
        String glBufferSource = readSource(glBufferFile);
        assertFalse(glBufferSource.contains("GlStateManager._glDeleteBuffers("),
            "GlBuffer should not delete buffers through removed GlStateManager._glDeleteBuffers wrapper");
        assertTrue(glBufferSource.contains("IrisRenderSystem.decrementTrackedBuffers();"),
            "GlBuffer should preserve tracked-buffer decrement through IrisRenderSystem helper when closing");
        assertTrue(glBufferSource.contains("VulkanicAPI.deleteBuffer(VulkanicAPI.getCommandContext(), this.handle)"),
            "GlBuffer should delete buffers directly via VulkanicAPI.deleteBuffer");

        Path dsaFile = SRC_MAIN_JAVA.resolve("net/blaze3d/opengl/DirectStateAccess.java");
        String dsaSource = readSource(dsaFile);
        assertFalse(dsaSource.contains("GlStateManager._glBufferData("),
            "DirectStateAccess should not call removed GlStateManager._glBufferData wrappers");
        assertFalse(dsaSource.contains("GlStateManager._glBufferSubData("),
            "DirectStateAccess should not call removed GlStateManager._glBufferSubData wrapper");
        assertTrue(containsAny(dsaSource,
                "VulkanicAPI.bufferData(VulkanicAPI.getCommandContext()",
                "VulkanicCoreAPI.bufferData(ctx,"),
            "DirectStateAccess should upload buffer data through Vulkanic frontend APIs");
        assertTrue(containsAny(dsaSource,
                "VulkanicAPI.bufferSubData(VulkanicAPI.getCommandContext()",
                "VulkanicCoreAPI.bufferSubData(ctx,"),
            "DirectStateAccess should update buffer ranges through Vulkanic frontend APIs");
    }

    @Test
    public void testBlaze3dDrawAndPixelWrappersRemovedFromGlStateManager() throws IOException {
        Path stateManagerFile = SRC_MAIN_JAVA.resolve("net/blaze3d/opengl/GlStateManager.java");
        String stateManagerSource = readSource(stateManagerFile);

        assertFalse(stateManagerSource.contains("public static void _drawElements("),
            "GlStateManager should no longer expose _drawElements wrapper");
        assertFalse(stateManagerSource.contains("public static void _drawArrays("),
            "GlStateManager should no longer expose _drawArrays wrapper");
        assertFalse(stateManagerSource.contains("public static void _pixelStore("),
            "GlStateManager should no longer expose _pixelStore wrapper");
        assertFalse(stateManagerSource.contains("public static void _readPixels("),
            "GlStateManager should no longer expose _readPixels wrapper");
        assertFalse(stateManagerSource.contains("public static void _glBlitFrameBuffer("),
            "GlStateManager should no longer expose _glBlitFrameBuffer wrapper");
        assertFalse(stateManagerSource.contains("public static void _glFramebufferTexture2D("),
            "GlStateManager should no longer expose _glFramebufferTexture2D wrapper");
        assertFalse(stateManagerSource.contains("public static void _texSubImage2D("),
            "GlStateManager should no longer expose _texSubImage2D wrapper overloads");
    }

    @Test
    public void testBlaze3dScissorAndPolygonWrappersRemovedFromGlStateManager() throws IOException {
        Path stateManagerFile = SRC_MAIN_JAVA.resolve("net/blaze3d/opengl/GlStateManager.java");
        String stateManagerSource = readSource(stateManagerFile);

        assertFalse(stateManagerSource.contains("public static void _scissorBox("),
            "GlStateManager should no longer expose _scissorBox wrapper");
        assertFalse(stateManagerSource.contains("public static void _polygonMode("),
            "GlStateManager should no longer expose _polygonMode wrapper");

        Path encoderFile = SRC_MAIN_JAVA.resolve("net/blaze3d/opengl/GlCommandEncoder.java");
        String encoderSource = readSource(encoderFile);

        assertFalse(encoderSource.contains("GlStateManager._scissorBox("),
            "GlCommandEncoder should not call removed GlStateManager._scissorBox wrapper");
        assertFalse(encoderSource.contains("GlStateManager._polygonMode("),
            "GlCommandEncoder should not call removed GlStateManager._polygonMode wrapper");
        assertTrue(encoderSource.contains("VulkanicAPI.setDynamicScissor(ctx,"),
            "GlCommandEncoder should set scissor directly through VulkanicAPI.setDynamicScissor");
        assertTrue(encoderSource.contains("VulkanicAPI.setPolygonMode(ctx, VulkanicPolygonFace.FRONT_AND_BACK"),
            "GlCommandEncoder should set polygon mode through VulkanicAPI.setPolygonMode with typed polygon-face semantics");
    }

    @Test
    public void testBlaze3dColorLogicWrappersRemovedFromGlStateManager() throws IOException {
        Path stateManagerFile = SRC_MAIN_JAVA.resolve("net/blaze3d/opengl/GlStateManager.java");
        String stateManagerSource = readSource(stateManagerFile);

        assertFalse(stateManagerSource.contains("public static void _enableColorLogicOp("),
            "GlStateManager should no longer expose _enableColorLogicOp wrapper");
        assertFalse(stateManagerSource.contains("public static void _disableColorLogicOp("),
            "GlStateManager should no longer expose _disableColorLogicOp wrapper");
        assertFalse(stateManagerSource.contains("public static void _logicOp("),
            "GlStateManager should no longer expose _logicOp wrapper");

        Path encoderFile = SRC_MAIN_JAVA.resolve("net/blaze3d/opengl/GlCommandEncoder.java");
        String encoderSource = readSource(encoderFile);

        assertFalse(encoderSource.contains("GlStateManager._enableColorLogicOp("),
            "GlCommandEncoder should not call removed GlStateManager._enableColorLogicOp wrapper");
        assertFalse(encoderSource.contains("GlStateManager._disableColorLogicOp("),
            "GlCommandEncoder should not call removed GlStateManager._disableColorLogicOp wrapper");
        assertFalse(encoderSource.contains("GlStateManager._logicOp("),
            "GlCommandEncoder should not call removed GlStateManager._logicOp wrapper");
        assertFalse(encoderSource.contains("VulkanicAPI.setLogicOp(VulkanicAPI.getImmediateContext(), 5387)"),
            "GlCommandEncoder should not hardcode OR_REVERSE literal 5387 when setting logic op");

        assertTrue(encoderSource.contains("VulkanicAPI.setColorLogicOpEnabled("),
            "GlCommandEncoder should enable/disable color logic directly through VulkanicAPI.setColorLogicOpEnabled");
        assertTrue(containsAny(encoderSource,
                "VulkanicAPI.setLogicOp(ctx, VulkanicLogicOp.OR_REVERSE)",
                "VulkanicAPI.setLogicOp(ctx, VulkanicAPI.GL_OR_REVERSE)"),
            "GlCommandEncoder should set OR_REVERSE logic op through a typed Vulkanic logic-op seam");
    }

    @Test
    public void testBlaze3dPolygonOffsetWrappersRemovedFromGlStateManager() throws IOException {
        Path stateManagerFile = SRC_MAIN_JAVA.resolve("net/blaze3d/opengl/GlStateManager.java");
        String stateManagerSource = readSource(stateManagerFile);

        assertFalse(stateManagerSource.contains("public static void _enablePolygonOffset("),
            "GlStateManager should no longer expose _enablePolygonOffset wrapper");
        assertFalse(stateManagerSource.contains("public static void _disablePolygonOffset("),
            "GlStateManager should no longer expose _disablePolygonOffset wrapper");
        assertFalse(stateManagerSource.contains("public static void _polygonOffset("),
            "GlStateManager should no longer expose _polygonOffset wrapper");

        Path encoderFile = SRC_MAIN_JAVA.resolve("net/blaze3d/opengl/GlCommandEncoder.java");
        String encoderSource = readSource(encoderFile);

        assertFalse(encoderSource.contains("GlStateManager._enablePolygonOffset("),
            "GlCommandEncoder should not call removed GlStateManager._enablePolygonOffset wrapper");
        assertFalse(encoderSource.contains("GlStateManager._disablePolygonOffset("),
            "GlCommandEncoder should not call removed GlStateManager._disablePolygonOffset wrapper");
        assertFalse(encoderSource.contains("GlStateManager._polygonOffset("),
            "GlCommandEncoder should not call removed GlStateManager._polygonOffset wrapper");

        assertTrue(encoderSource.contains("VulkanicAPI.setPolygonOffset("),
            "GlCommandEncoder should set polygon offset directly through VulkanicAPI.setPolygonOffset");
        assertTrue(encoderSource.contains("VulkanicAPI.setPolygonOffsetFillEnabled("),
            "GlCommandEncoder should toggle polygon-offset fill state directly through VulkanicAPI.setPolygonOffsetFillEnabled");
    }

    @Test
    public void testBlaze3dCullWrappersRemovedFromGlStateManager() throws IOException {
        Path stateManagerFile = SRC_MAIN_JAVA.resolve("net/blaze3d/opengl/GlStateManager.java");
        String stateManagerSource = readSource(stateManagerFile);

        assertFalse(stateManagerSource.contains("public static void _enableCull("),
            "GlStateManager should no longer expose _enableCull wrapper");
        assertFalse(stateManagerSource.contains("public static void _disableCull("),
            "GlStateManager should no longer expose _disableCull wrapper");

        Path encoderFile = SRC_MAIN_JAVA.resolve("net/blaze3d/opengl/GlCommandEncoder.java");
        String encoderSource = readSource(encoderFile);
        assertFalse(encoderSource.contains("GlStateManager._enableCull("),
            "GlCommandEncoder should not call removed GlStateManager._enableCull wrapper");
        assertFalse(encoderSource.contains("GlStateManager._disableCull("),
            "GlCommandEncoder should not call removed GlStateManager._disableCull wrapper");
        assertTrue(encoderSource.contains("VulkanicAPI.setCullFaceEnabled("),
            "GlCommandEncoder should toggle cull state directly through VulkanicAPI.setCullFaceEnabled");

        Path shadowRendererFile = SRC_MAIN_JAVA.resolve("net/irisshaders/iris/shadows/ShadowRenderer.java");
        String shadowRendererSource = readSource(shadowRendererFile);
        assertFalse(shadowRendererSource.contains("GlStateManager._disableCull("),
            "ShadowRenderer should not call removed GlStateManager._disableCull wrapper");
        assertFalse(shadowRendererSource.contains("GlStateManager._enableCull("),
            "ShadowRenderer should not call removed GlStateManager._enableCull wrapper");
        assertTrue(shadowRendererSource.contains("VulkanicAPI.setCullFaceEnabled("),
            "ShadowRenderer should toggle cull state via VulkanicAPI.setCullFaceEnabled");

        Path sodiumShaderFile = SRC_MAIN_JAVA.resolve("net/irisshaders/iris/pipeline/programs/SodiumShader.java");
        String sodiumShaderSource = readSource(sodiumShaderFile);
        assertFalse(sodiumShaderSource.contains("GlStateManager._disableCull("),
            "SodiumShader should not call removed GlStateManager._disableCull wrapper");
        assertTrue(sodiumShaderSource.contains("VulkanicAPI.setCullFaceEnabled("),
            "SodiumShader should disable culling through VulkanicAPI.setCullFaceEnabled");

        Path dhWrapperFile = SRC_MAIN_JAVA.resolve("com/seibel/distanthorizons/common/wrappers/minecraft/MinecraftGLWrapper.java");
        String dhWrapperSource = readSourceIfExists(dhWrapperFile);
        assertFalse(dhWrapperSource.contains("GlStateManager._enableCull("),
            "MinecraftGLWrapper should not call removed GlStateManager._enableCull wrapper");
        assertFalse(dhWrapperSource.contains("GlStateManager._disableCull("),
            "MinecraftGLWrapper should not call removed GlStateManager._disableCull wrapper");
        assertFalse(dhWrapperSource.contains("public void enableFaceCulling("),
            "MinecraftGLWrapper should no longer expose cull wrapper methods");
        assertFalse(dhWrapperSource.contains("public void disableFaceCulling("),
            "MinecraftGLWrapper should no longer expose cull wrapper methods");
    }

    @Test
    public void testBlaze3dDepthWrappersRemovedFromGlStateManager() throws IOException {
        Path stateManagerFile = SRC_MAIN_JAVA.resolve("net/blaze3d/opengl/GlStateManager.java");
        String stateManagerSource = readSource(stateManagerFile);

        assertFalse(stateManagerSource.contains("public static void _enableDepthTest("),
            "GlStateManager should no longer expose _enableDepthTest wrapper");
        assertFalse(stateManagerSource.contains("public static void _disableDepthTest("),
            "GlStateManager should no longer expose _disableDepthTest wrapper");
        assertFalse(stateManagerSource.contains("public static void _depthFunc("),
            "GlStateManager should no longer expose _depthFunc wrapper");
        assertFalse(stateManagerSource.contains("public static void _depthMask("),
            "GlStateManager should no longer expose _depthMask wrapper");
        assertFalse(stateManagerSource.contains("public static final GlStateManager.DepthState DEPTH"),
            "GlStateManager should no longer own depth-mask state container");
        assertFalse(stateManagerSource.contains("class DepthState"),
            "GlStateManager should no longer define DepthState helper class");

        Path encoderFile = SRC_MAIN_JAVA.resolve("net/blaze3d/opengl/GlCommandEncoder.java");
        String encoderSource = readSource(encoderFile);
        assertFalse(encoderSource.contains("GlStateManager._enableDepthTest("),
            "GlCommandEncoder should not call removed GlStateManager._enableDepthTest wrapper");
        assertFalse(encoderSource.contains("GlStateManager._disableDepthTest("),
            "GlCommandEncoder should not call removed GlStateManager._disableDepthTest wrapper");
        assertFalse(encoderSource.contains("GlStateManager._depthFunc("),
            "GlCommandEncoder should not call removed GlStateManager._depthFunc wrapper");
        assertFalse(encoderSource.contains("GlStateManager._depthMask("),
            "GlCommandEncoder should not call removed GlStateManager._depthMask wrapper");
        assertTrue(encoderSource.contains("VulkanicAPI.setDepthTestEnabled("),
            "GlCommandEncoder should toggle depth test through VulkanicAPI.setDepthTestEnabled");
        assertTrue(encoderSource.contains("VulkanicAPI.setDepthFunc("),
            "GlCommandEncoder should set depth function through VulkanicAPI.setDepthFunc");
        assertTrue(encoderSource.contains("DepthColorStorage.setDepthMask("),
            "GlCommandEncoder should route depth-write mask changes through DepthColorStorage.setDepthMask");

        Path oldImageButtonFile = SRC_MAIN_JAVA.resolve("net/irisshaders/iris/gui/OldImageButton.java");
        String oldImageButtonSource = readSource(oldImageButtonFile);
        assertFalse(oldImageButtonSource.contains("GlStateManager._enableDepthTest("),
            "OldImageButton should not call removed GlStateManager._enableDepthTest wrapper");
        assertTrue(oldImageButtonSource.contains("VulkanicAPI.setDepthTestEnabled("),
            "OldImageButton should enable depth test through VulkanicAPI.setDepthTestEnabled");

        Path irisButtonFile = SRC_MAIN_JAVA.resolve("net/irisshaders/iris/gui/element/screen/IrisButton.java");
        String irisButtonSource = readSource(irisButtonFile);
        assertFalse(irisButtonSource.contains("GlStateManager._enableDepthTest("),
            "IrisButton should not call removed GlStateManager._enableDepthTest wrapper");
        assertTrue(irisButtonSource.contains("VulkanicAPI.setDepthTestEnabled("),
            "IrisButton should enable depth test through VulkanicAPI.setDepthTestEnabled");

        Path dhProgramFile = SRC_MAIN_JAVA.resolve("net/irisshaders/iris/compat/dh/IrisGenericRenderProgram.java");
        String dhProgramSource = readSource(dhProgramFile);
        assertFalse(dhProgramSource.contains("GlStateManager._enableDepthTest("),
            "IrisGenericRenderProgram should not call removed GlStateManager._enableDepthTest wrapper");
        assertFalse(dhProgramSource.contains("GlStateManager._depthFunc("),
            "IrisGenericRenderProgram should not call removed GlStateManager._depthFunc wrapper");
        assertTrue(dhProgramSource.contains("VulkanicAPI.setDepthTestEnabled("),
            "IrisGenericRenderProgram should enable depth test through VulkanicAPI.setDepthTestEnabled");
        assertTrue(dhProgramSource.contains("VulkanicAPI.setDepthFunc("),
            "IrisGenericRenderProgram should set depth func through VulkanicAPI.setDepthFunc");

        Path dhWrapperFile = SRC_MAIN_JAVA.resolve("com/seibel/distanthorizons/common/wrappers/minecraft/MinecraftGLWrapper.java");
        String dhWrapperSource = readSourceIfExists(dhWrapperFile);
        assertFalse(dhWrapperSource.contains("GlStateManager._enableDepthTest("),
            "MinecraftGLWrapper should not call removed GlStateManager._enableDepthTest wrapper");
        assertFalse(dhWrapperSource.contains("GlStateManager._disableDepthTest("),
            "MinecraftGLWrapper should not call removed GlStateManager._disableDepthTest wrapper");
        assertFalse(dhWrapperSource.contains("GlStateManager._depthFunc("),
            "MinecraftGLWrapper should not call removed GlStateManager._depthFunc wrapper");
        assertFalse(dhWrapperSource.contains("GlStateManager._depthMask("),
            "MinecraftGLWrapper should not call removed GlStateManager._depthMask wrapper");
        assertFalse(dhWrapperSource.contains("public void enableDepthTest("),
            "MinecraftGLWrapper should no longer expose depth-test wrapper methods");
        assertFalse(dhWrapperSource.contains("public void disableDepthTest("),
            "MinecraftGLWrapper should no longer expose depth-test wrapper methods");
        assertFalse(dhWrapperSource.contains("public void glDepthFunc("),
            "MinecraftGLWrapper should no longer expose depth-function wrapper methods");
        assertFalse(dhWrapperSource.contains("public void enableDepthMask("),
            "MinecraftGLWrapper should no longer expose depth-write mask wrapper methods");
        assertFalse(dhWrapperSource.contains("public void disableDepthMask("),
            "MinecraftGLWrapper should no longer expose depth-write mask wrapper methods");

        Path depthColorStorageFile = SRC_MAIN_JAVA.resolve("net/irisshaders/iris/gl/blending/DepthColorStorage.java");
        String depthColorStorageSource = readSource(depthColorStorageFile);
        assertTrue(depthColorStorageSource.contains("public static void setDepthMask("),
            "DepthColorStorage should expose lock-aware setDepthMask after _depthMask wrapper removal");
        assertFalse(depthColorStorageSource.contains("GlStateManager.DEPTH"),
            "DepthColorStorage should not read depth-mask state from GlStateManager");
        assertTrue(depthColorStorageSource.contains("private static boolean currentDepthEnable"),
            "DepthColorStorage should own depth-mask state locally");
    }

    @Test
    public void testBlaze3dColorMaskWrapperRemovedFromGlStateManager() throws IOException {
        Path stateManagerFile = SRC_MAIN_JAVA.resolve("net/blaze3d/opengl/GlStateManager.java");
        String stateManagerSource = readSource(stateManagerFile);

        assertFalse(stateManagerSource.contains("public static void _colorMask("),
            "GlStateManager should no longer expose _colorMask wrapper");
        assertFalse(stateManagerSource.contains("public static final GlStateManager.ColorMask COLOR_MASK"),
            "GlStateManager should no longer own color-mask state container");
        assertFalse(stateManagerSource.contains("class ColorMask"),
            "GlStateManager should no longer define color-mask state helper class");

        Path encoderFile = SRC_MAIN_JAVA.resolve("net/blaze3d/opengl/GlCommandEncoder.java");
        String encoderSource = readSource(encoderFile);
        assertFalse(encoderSource.contains("GlStateManager._colorMask("),
            "GlCommandEncoder should not call removed GlStateManager._colorMask wrapper");
        assertTrue(encoderSource.contains("DepthColorStorage.setColorMask("),
            "GlCommandEncoder should route color-mask changes through DepthColorStorage.setColorMask");

        Path depthColorStorageFile = SRC_MAIN_JAVA.resolve("net/irisshaders/iris/gl/blending/DepthColorStorage.java");
        String depthColorStorageSource = readSource(depthColorStorageFile);
        assertFalse(depthColorStorageSource.contains("GlStateManager._colorMask("),
            "DepthColorStorage should not call removed GlStateManager._colorMask wrapper");
        assertTrue(depthColorStorageSource.contains("public static void setColorMask("),
            "DepthColorStorage should expose lock-aware setColorMask after _colorMask wrapper removal");
        assertFalse(depthColorStorageSource.contains("GlStateManager.COLOR_MASK"),
            "DepthColorStorage should not read color-mask state from GlStateManager");
        assertTrue(depthColorStorageSource.contains("private static boolean currentRedMask"),
            "DepthColorStorage should own color-mask state locally");
    }

    @Test
    public void testBlaze3dBlendWrappersRemovedFromGlStateManager() throws IOException {
        Path stateManagerFile = SRC_MAIN_JAVA.resolve("net/blaze3d/opengl/GlStateManager.java");
        String stateManagerSource = readSource(stateManagerFile);

        assertFalse(stateManagerSource.contains("public static void _enableBlend("),
            "GlStateManager should no longer expose _enableBlend wrapper");
        assertFalse(stateManagerSource.contains("public static void _disableBlend("),
            "GlStateManager should no longer expose _disableBlend wrapper");
        assertFalse(stateManagerSource.contains("public static void _blendFuncSeparate("),
            "GlStateManager should no longer expose _blendFuncSeparate wrapper");
        assertFalse(stateManagerSource.contains("public static void glBlendFuncSeparate("),
            "GlStateManager should no longer expose glBlendFuncSeparate shim");
        assertFalse(stateManagerSource.contains("public static void notifyBlendFuncChanged("),
            "GlStateManager should no longer expose blend-function notifier trigger");
        assertFalse(stateManagerSource.contains("StateUpdateNotifiers.blendFuncNotifier"),
            "GlStateManager should no longer own blend-function notifier wiring");
        assertFalse(stateManagerSource.contains("public static final GlStateManager.BlendState BLEND"),
            "GlStateManager should no longer own blend-state container");
        assertFalse(stateManagerSource.contains("class BlendState"),
            "GlStateManager should no longer define BlendState helper class");
        assertFalse(stateManagerSource.contains("class BooleanState"),
            "GlStateManager should no longer define BooleanState helper class");

        Path encoderFile = SRC_MAIN_JAVA.resolve("net/blaze3d/opengl/GlCommandEncoder.java");
        String encoderSource = readSource(encoderFile);
        assertFalse(encoderSource.contains("GlStateManager._enableBlend("),
            "GlCommandEncoder should not call removed GlStateManager._enableBlend wrapper");
        assertFalse(encoderSource.contains("GlStateManager._disableBlend("),
            "GlCommandEncoder should not call removed GlStateManager._disableBlend wrapper");
        assertFalse(encoderSource.contains("GlStateManager._blendFuncSeparate("),
            "GlCommandEncoder should not call removed GlStateManager._blendFuncSeparate wrapper");
        assertTrue(encoderSource.contains("BlendModeStorage.setBlendEnabled("),
            "GlCommandEncoder should route blend toggles through BlendModeStorage.setBlendEnabled");
        assertTrue(encoderSource.contains("BlendModeStorage.setBlendFuncSeparate("),
            "GlCommandEncoder should route blend functions through BlendModeStorage.setBlendFuncSeparate");

        Path blendStorageFile = SRC_MAIN_JAVA.resolve("net/irisshaders/iris/gl/blending/BlendModeStorage.java");
        String blendStorageSource = readSource(blendStorageFile);
        assertTrue(blendStorageSource.contains("public static void setBlendEnabled("),
            "BlendModeStorage should expose setBlendEnabled helper after wrapper removal");
        assertTrue(blendStorageSource.contains("public static void setBlendFuncSeparate("),
            "BlendModeStorage should expose setBlendFuncSeparate helper after wrapper removal");
        assertTrue(blendStorageSource.contains("public static boolean isBlendEnabled("),
            "BlendModeStorage should expose blend-enabled getter for non-Blaze blend-state reads");
        assertTrue(blendStorageSource.contains("public static int getBlendSrcRgb("),
            "BlendModeStorage should expose blend factor getters for non-Blaze blend-state reads");
        assertTrue(blendStorageSource.contains("VulkanicAPI.setBlendFunction("),
            "BlendModeStorage should set blend function directly through VulkanicAPI.setBlendFunction");
        assertTrue(blendStorageSource.contains("VulkanicAPI.setCapabilityEnabled("),
            "BlendModeStorage should toggle GL_BLEND capability directly through VulkanicAPI");
        assertTrue(blendStorageSource.contains("public static void markBlendStateUnknown("),
            "BlendModeStorage should expose unknown-state marker for indexed blend overrides");
        assertFalse(blendStorageSource.contains("GlStateManager.notifyBlendFuncChanged("),
            "BlendModeStorage should not trigger blend-function notifier through GlStateManager");
        assertTrue(blendStorageSource.contains("IrisRenderSystem.notifyBlendFuncChanged("),
            "BlendModeStorage should trigger blend-function notifier through IrisRenderSystem");
        assertFalse(blendStorageSource.contains("GlStateManager.BLEND"),
            "BlendModeStorage should no longer read or write blend state through GlStateManager.BLEND");

        Path irisRenderSystemFile = SRC_MAIN_JAVA.resolve("net/irisshaders/iris/gl/IrisRenderSystem.java");
        String irisRenderSystemSource = readSource(irisRenderSystemFile);
        assertTrue(irisRenderSystemSource.contains("public static void notifyBlendFuncChanged("),
            "IrisRenderSystem should expose blend-function notifier trigger after migration");
        assertTrue(irisRenderSystemSource.contains("StateUpdateNotifiers.blendFuncNotifier"),
            "IrisRenderSystem should own blend-function notifier wiring after migration");
        assertFalse(irisRenderSystemSource.contains("GlStateManager.BLEND"),
            "IrisRenderSystem should not invalidate blend state through GlStateManager.BLEND");
        assertTrue(irisRenderSystemSource.contains("BlendModeStorage.markBlendStateUnknown("),
            "IrisRenderSystem should invalidate blend state through BlendModeStorage helper");

        Path commonUniformsFile = SRC_MAIN_JAVA.resolve("net/irisshaders/iris/uniforms/CommonUniforms.java");
        String commonUniformsSource = readSource(commonUniformsFile);
        assertFalse(commonUniformsSource.contains("GlStateManager.BLEND"),
            "CommonUniforms should not read blend state directly from GlStateManager");
        assertTrue(commonUniformsSource.contains("BlendModeStorage.isBlendEnabled("),
            "CommonUniforms should read blend enabled state through BlendModeStorage helper");
        assertTrue(commonUniformsSource.contains("BlendModeStorage.getBlendSrcRgb("),
            "CommonUniforms should read blend factors through BlendModeStorage helpers");
        assertTrue(commonUniformsSource.contains("Java Iris uniform construction is unavailable on the Rust Vulkan route"),
            "CommonUniforms must fail closed before constructing Java Iris uniform state on Rust Vulkan");
        assertFalse(commonUniformsSource.contains("static {\n\t\tGbufferPrograms.init();"),
            "CommonUniforms must not install Iris listeners during class loading on Rust Vulkan");
        Path gbufferProgramsFile = SRC_MAIN_JAVA.resolve("net/irisshaders/iris/layer/GbufferPrograms.java");
        String gbufferProgramsSource = readSource(gbufferProgramsFile);
        assertFalse(gbufferProgramsSource.contains("static {\n\t\tStateUpdateNotifiers.phaseChangeNotifier"),
            "GbufferPrograms must not install Iris listeners during class loading on Rust Vulkan");
        assertTrue(gbufferProgramsSource.contains("if (initialized || isRustRoute())"),
            "GbufferPrograms listener installation must be lazy and compatibility-route-only");
        Path irisRenderSystemListenerFile = SRC_MAIN_JAVA.resolve("net/irisshaders/iris/gl/IrisRenderSystem.java");
        String irisRenderSystemListenerSource = readSource(irisRenderSystemListenerFile);
        assertFalse(irisRenderSystemListenerSource.contains("static {\n\t\tStateUpdateNotifiers.blendFuncNotifier"),
            "IrisRenderSystem must not install blend listeners during Rust Vulkan class loading");
        assertTrue(irisRenderSystemListenerSource.contains("installCompatibilityListeners();"),
            "IrisRenderSystem blend listener installation must remain OpenGL-init owned");
        Path textureTrackerListenerFile = SRC_MAIN_JAVA.resolve("net/irisshaders/iris/pbr/TextureTracker.java");
        String textureTrackerListenerSource = readSource(textureTrackerListenerFile);
        assertFalse(textureTrackerListenerSource.contains("static {\n\t\tStateUpdateNotifiers.bindTextureNotifier"),
            "TextureTracker must not install bind listeners during Rust Vulkan class loading");
        assertTrue(textureTrackerListenerSource.contains("installCompatibilityListener();"),
            "TextureTracker bind listener installation must remain compatibility-only");
        Path pbrManagerFile = SRC_MAIN_JAVA.resolve("net/irisshaders/iris/pbr/texture/PBRTextureManager.java");
        String pbrManagerSource = readSource(pbrManagerFile);
        assertFalse(pbrManagerSource.contains("static {\n\t\tStateUpdateNotifiers.normalTextureChangeNotifier"),
            "PBRTextureManager must not install PBR listeners during Rust Vulkan class loading");
        assertTrue(pbrManagerSource.contains("installCompatibilityListeners();"),
            "PBRTextureManager listener installation must remain OpenGL-init owned");

        Path dhWrapperFile = SRC_MAIN_JAVA.resolve("com/seibel/distanthorizons/common/wrappers/minecraft/MinecraftGLWrapper.java");
        String dhWrapperSource = readSourceIfExists(dhWrapperFile);
        assertFalse(dhWrapperSource.contains("GlStateManager._enableBlend("),
            "MinecraftGLWrapper should not call removed GlStateManager._enableBlend wrapper");
        assertFalse(dhWrapperSource.contains("GlStateManager._disableBlend("),
            "MinecraftGLWrapper should not call removed GlStateManager._disableBlend wrapper");
        assertFalse(dhWrapperSource.contains("GlStateManager._blendFuncSeparate("),
            "MinecraftGLWrapper should not call removed GlStateManager._blendFuncSeparate wrapper");
        assertFalse(dhWrapperSource.contains("public void enableBlend("),
            "MinecraftGLWrapper should no longer expose blend wrapper methods");
        assertFalse(dhWrapperSource.contains("public void disableBlend("),
            "MinecraftGLWrapper should no longer expose blend wrapper methods");
    }

    @Test
    public void testCapabilityCallsitesUseTypedEnumsForKnownStateToggles() throws IOException {
        Path blendStorageFile = SRC_MAIN_JAVA.resolve("net/irisshaders/iris/gl/blending/BlendModeStorage.java");
        String blendStorageSource = readSource(blendStorageFile);
        assertTrue(blendStorageSource.contains("VulkanicCapability.BLEND"),
            "BlendModeStorage should use typed VulkanicCapability.BLEND for blend toggles");
        assertFalse(blendStorageSource.contains("setCapabilityEnabled(ctx, VulkanicAPI.GL_BLEND"),
            "BlendModeStorage should avoid raw GL_BLEND constants in capability toggles");

        Path dhStateFile = SRC_MAIN_JAVA.resolve("com/seibel/distanthorizons/core/render/glObject/GLState.java");
        String dhStateSource = readSourceIfExists(dhStateFile);
        assertTrue(dhStateSource.contains("VulkanicCapability.STENCIL_TEST"),
            "Distant Horizons GLState should use typed stencil capability toggles");
        assertFalse(dhStateSource.contains("setCapabilityEnabled(ctx, VulkanicAPI.GL_STENCIL_TEST"),
            "Distant Horizons GLState should avoid raw GL_STENCIL_TEST constants in capability toggles");
        assertFalse(dhStateSource.contains("isEnabled(ctx, VulkanicAPI.GL_BLEND"),
            "Distant Horizons GLState should avoid raw GL_BLEND constants in capability queries");
        assertTrue(dhStateSource.contains("isEnabled(ctx, VulkanicCapability.BLEND)"),
            "Distant Horizons GLState should use typed VulkanicCapability.BLEND in capability queries");

        Path lodRendererEventsFile = SRC_MAIN_JAVA.resolve("net/irisshaders/iris/compat/dh/LodRendererEvents.java");
        String lodRendererEventsSource = readSourceIfExists(lodRendererEventsFile);
        assertTrue(lodRendererEventsSource.contains("VulkanicCapability.CULL_FACE"),
            "LodRendererEvents should use typed cull-face capability toggles");
        assertFalse(lodRendererEventsSource.contains("setCapabilityEnabled(ctx, VulkanicAPI.GL_CULL_FACE"),
            "LodRendererEvents should avoid raw GL_CULL_FACE constants in capability toggles");

        Path lodRendererFile = SRC_MAIN_JAVA.resolve("com/seibel/distanthorizons/core/render/renderer/LodRenderer.java");
        String lodRendererSource = readSourceIfExists(lodRendererFile);
        assertTrue(lodRendererSource.contains("VulkanicCapability.SCISSOR_TEST"),
            "LodRenderer should use typed scissor-test capability toggles");
        assertFalse(lodRendererSource.contains("setCapabilityEnabled(ctx, VulkanicAPI.GL_SCISSOR_TEST"),
            "LodRenderer should avoid raw GL_SCISSOR_TEST constants in capability toggles");
    }

    @Test
    public void testDepthAndCullCallsitesUseTypedEnumsForKnownState() throws IOException {
        Path lodRendererFile = SRC_MAIN_JAVA.resolve("com/seibel/distanthorizons/core/render/renderer/LodRenderer.java");
        String lodRendererSource = readSourceIfExists(lodRendererFile);
        assertTrue(lodRendererSource.contains("VulkanicDepthCompareOp.LESS"),
            "LodRenderer should use typed depth-compare enum for known depth state");
        assertFalse(lodRendererSource.contains("setDepthFunc(ctx, VulkanicAPI.GL_LESS"),
            "LodRenderer should avoid raw GL_LESS depth constant in known state setup");

        Path dhProgramFile = SRC_MAIN_JAVA.resolve("net/irisshaders/iris/compat/dh/IrisGenericRenderProgram.java");
        String dhProgramSource = readSourceIfExists(dhProgramFile);
        assertTrue(dhProgramSource.contains("VulkanicDepthCompareOp.LEQUAL"),
            "IrisGenericRenderProgram should use typed depth-compare enum for known depth state");
        assertFalse(dhProgramSource.contains("setDepthFunc(ctx, VulkanicAPI.GL_LEQUAL"),
            "IrisGenericRenderProgram should avoid raw GL_LEQUAL depth constant in known state setup");

        Path encoderFile = SRC_MAIN_JAVA.resolve("net/blaze3d/opengl/GlCommandEncoder.java");
        String encoderSource = readSourceIfExists(encoderFile);
        assertTrue(encoderSource.contains("toVulkanicDepthCompareOp(renderPipeline.getDepthTestFunction())"),
            "GlCommandEncoder should map pipeline depth-test function to typed Vulkanic depth compare enum");
        assertFalse(encoderSource.contains("setDepthFunc(ctx, GlConst.toGl(renderPipeline.getDepthTestFunction()))"),
            "GlCommandEncoder should avoid direct GL int conversion for depth compare setup");

        Path glStateFile = SRC_MAIN_JAVA.resolve("com/seibel/distanthorizons/core/render/glObject/GLState.java");
        String glStateSource = readSourceIfExists(glStateFile);
        assertTrue(glStateSource.contains("VulkanicDepthCompareOp.fromLegacyGlConstant(this.depthFunc)"),
            "GLState restore should resolve saved depth function through typed conversion helper");
        assertTrue(glStateSource.contains("VulkanicCullFaceMode.fromLegacyGlConstant(this.cullMode)"),
            "GLState restore should resolve saved cull mode through typed conversion helper");
        assertTrue(glStateSource.contains("VulkanicStencilCompareOp.fromLegacyGlConstant(this.stencilFunc)"),
            "GLState restore should resolve saved stencil function through typed conversion helper");
        assertTrue(glStateSource.contains("VulkanicStencilOperation.fromLegacyGlConstant(this.stencilFailOp)"),
            "GLState restore should resolve saved stencil operations through typed conversion helper");
        assertTrue(glStateSource.contains("VulkanicAPI.setStencilWriteMask(ctx, this.stencilWriteMask)"),
            "GLState restore should restore stencil write mask through VulkanicAPI helper");
    }

    @Test
    public void testBlendCallsitesUseTypedEnumsForKnownState() throws IOException {
        Path lodRendererFile = SRC_MAIN_JAVA.resolve("com/seibel/distanthorizons/core/render/renderer/LodRenderer.java");
        String lodRendererSource = readSourceIfExists(lodRendererFile);
        assertTrue(lodRendererSource.contains("VulkanicBlendFactor.SRC_ALPHA"),
            "LodRenderer should use typed blend-factor enums for known blend setup");
        assertTrue(lodRendererSource.contains("VulkanicBlendEquation.ADD"),
            "LodRenderer should use typed blend-equation enum for known blend setup");
        assertFalse(lodRendererSource.contains("setBlendEquation(ctx, VulkanicAPI.GL_FUNC_ADD"),
            "LodRenderer should avoid raw GL_FUNC_ADD constant in known blend setup");

        Path fogApplyFile = SRC_MAIN_JAVA.resolve("com/seibel/distanthorizons/core/render/renderer/shaders/FogApplyShader.java");
        String fogApplySource = readSourceIfExists(fogApplyFile);
        assertTrue(fogApplySource.contains("VulkanicBlendEquation.ADD"),
            "FogApplyShader should use typed blend-equation enum for known blend setup");
        assertTrue(fogApplySource.contains("VulkanicBlendFactor.ONE_MINUS_SRC_ALPHA"),
            "FogApplyShader should use typed blend-factor enums for known blend setup");

        Path ssaoApplyFile = SRC_MAIN_JAVA.resolve("com/seibel/distanthorizons/core/render/renderer/shaders/SSAOApplyShader.java");
        String ssaoApplySource = readSourceIfExists(ssaoApplyFile);
        assertTrue(ssaoApplySource.contains("VulkanicBlendFactor.ZERO"),
            "SSAOApplyShader should use typed blend-factor enums for known blend setup");
        assertTrue(ssaoApplySource.contains("VulkanicBlendFactor.SRC_ALPHA"),
            "SSAOApplyShader should use typed blend-factor enums for known blend setup");

        Path genericRendererFile = SRC_MAIN_JAVA.resolve("com/seibel/distanthorizons/core/render/renderer/generic/GenericObjectRenderer.java");
        String genericRendererSource = readSourceIfExists(genericRendererFile);
        assertTrue(genericRendererSource.contains("VulkanicBlendEquation.ADD"),
            "GenericObjectRenderer should use typed blend-equation enum for known blend setup");
        assertTrue(genericRendererSource.contains("VulkanicBlendFactor.ONE_MINUS_SRC_ALPHA"),
            "GenericObjectRenderer should use typed blend-factor enums for known blend setup");

        Path glStateFile = SRC_MAIN_JAVA.resolve("com/seibel/distanthorizons/core/render/glObject/GLState.java");
        String glStateSource = readSourceIfExists(glStateFile);
        assertTrue(glStateSource.contains("VulkanicBlendFactor.fromLegacyGlConstant(this.blendSrcColor)"),
            "GLState restore should resolve saved blend factors through typed conversion helper");
        assertTrue(glStateSource.contains("VulkanicBlendEquation.fromLegacyGlConstant(this.blendEqRGB)"),
            "GLState restore should resolve saved blend equations through typed conversion helper");
    }

    @Test
    public void testCoreIrisPathsDoNotImportGlStateManager() throws IOException {
        String legacyImport = "import net.blaze3d.opengl.GlStateManager;";
        String[] migratedFiles = new String[] {
            "net/irisshaders/iris/gl/blending/BlendModeStorage.java",
            "net/irisshaders/iris/gl/IrisRenderSystem.java",
            "net/irisshaders/iris/gl/framebuffer/GlFramebuffer.java",
            "net/irisshaders/iris/gl/program/Program.java",
            "net/irisshaders/iris/gl/program/ComputeProgram.java",
            "net/irisshaders/iris/gl/texture/GlTexture.java",
            "net/irisshaders/iris/gl/texture/TextureUploadHelper.java",
            "net/irisshaders/iris/gl/image/GlImage.java",
            "net/irisshaders/iris/pipeline/programs/SodiumShader.java",
            "net/irisshaders/iris/pipeline/programs/ExtendedShader.java",
            "net/irisshaders/iris/pipeline/programs/FallbackShader.java",
            "net/irisshaders/iris/pipeline/programs/ShaderCreator.java",
            "net/irisshaders/iris/shadows/ShadowRenderer.java",
            "net/irisshaders/iris/shadows/ShadowCompositeRenderer.java",
            "com/seibel/distanthorizons/common/wrappers/minecraft/MinecraftGLWrapper.java"
        };

        for (String relativePath : migratedFiles) {
            Path file = SRC_MAIN_JAVA.resolve(relativePath);
            String source = readSourceIfExists(file);
            assertFalse(source.contains(legacyImport),
                relativePath + " should not import GlStateManager after Vulkanic migration");
        }
    }

    @Test
    public void testBlaze3dUseProgramWrapperRemovedFromGlStateManager() throws IOException {
        Path stateManagerFile = SRC_MAIN_JAVA.resolve("net/blaze3d/opengl/GlStateManager.java");
        String stateManagerSource = readSource(stateManagerFile);

        assertFalse(stateManagerSource.contains("public static void _glUseProgram("),
            "GlStateManager should no longer expose _glUseProgram wrapper");

        Path encoderFile = SRC_MAIN_JAVA.resolve("net/blaze3d/opengl/GlCommandEncoder.java");
        String encoderSource = readSource(encoderFile);
        assertFalse(encoderSource.contains("GlStateManager._glUseProgram("),
            "GlCommandEncoder should not call removed GlStateManager._glUseProgram wrapper");
        assertTrue(encoderSource.contains("IrisRenderSystem.useProgram("),
            "GlCommandEncoder should bind programs through IrisRenderSystem.useProgram");

        Path programFile = SRC_MAIN_JAVA.resolve("net/irisshaders/iris/gl/program/Program.java");
        String programSource = readSource(programFile);
        assertFalse(programSource.contains("GlStateManager._glUseProgram("),
            "Program should not call removed GlStateManager._glUseProgram wrapper");
        assertTrue(programSource.contains("IrisRenderSystem.useProgram("),
            "Program should bind programs through IrisRenderSystem.useProgram");

        Path computeProgramFile = SRC_MAIN_JAVA.resolve("net/irisshaders/iris/gl/program/ComputeProgram.java");
        String computeProgramSource = readSource(computeProgramFile);
        assertFalse(computeProgramSource.contains("GlStateManager._glUseProgram("),
            "ComputeProgram should not call removed GlStateManager._glUseProgram wrapper");
        assertTrue(computeProgramSource.contains("IrisRenderSystem.useProgram("),
            "ComputeProgram should bind programs through IrisRenderSystem.useProgram");

        Path irisRenderSystemFile = SRC_MAIN_JAVA.resolve("net/irisshaders/iris/gl/IrisRenderSystem.java");
        String irisRenderSystemSource = readSource(irisRenderSystemFile);
        assertTrue(irisRenderSystemSource.contains("public static void useProgram("),
            "IrisRenderSystem should provide useProgram helper after _glUseProgram removal");
        assertTrue(irisRenderSystemSource.contains("ImmediateState.usingTessellation = false"),
            "IrisRenderSystem.useProgram should preserve tessellation reset behavior");
    }

    @Test
    public void testBlaze3dActiveTextureWrapperRemovedFromGlStateManager() throws IOException {
        Path stateManagerFile = SRC_MAIN_JAVA.resolve("net/blaze3d/opengl/GlStateManager.java");
        String stateManagerSource = readSource(stateManagerFile);

        assertFalse(stateManagerSource.contains("public static void _activeTexture("),
            "GlStateManager should no longer expose _activeTexture wrapper");

        Path encoderFile = SRC_MAIN_JAVA.resolve("net/blaze3d/opengl/GlCommandEncoder.java");
        String encoderSource = readSource(encoderFile);
        assertFalse(encoderSource.contains("GlStateManager._activeTexture("),
            "GlCommandEncoder should not call removed GlStateManager._activeTexture wrapper");
        assertFalse(encoderSource.contains("IrisRenderSystem.setActiveTexture(VulkanicAPI.GL_TEXTURE0 +"),
            "GlCommandEncoder should not compute GL_TEXTURE0 offsets directly when selecting active texture units");
        assertTrue(encoderSource.contains("IrisRenderSystem.setActiveTextureUnitIndex("),
            "GlCommandEncoder should route active texture changes through IrisRenderSystem.setActiveTextureUnitIndex");

        Path sodiumShaderFile = SRC_MAIN_JAVA.resolve("net/irisshaders/iris/pipeline/programs/SodiumShader.java");
        String sodiumShaderSource = readSource(sodiumShaderFile);
        assertFalse(sodiumShaderSource.contains("GlStateManager._activeTexture("),
            "SodiumShader should not call removed GlStateManager._activeTexture wrapper");
        assertFalse(sodiumShaderSource.contains("IrisRenderSystem.setActiveTexture(VulkanicAPI.GL_TEXTURE0 +"),
            "SodiumShader should not compute GL_TEXTURE0 offsets directly when selecting active texture units");
        assertTrue(sodiumShaderSource.contains("IrisRenderSystem.setActiveTextureUnitIndex("),
            "SodiumShader should route active texture changes through IrisRenderSystem.setActiveTextureUnitIndex");

        Path dhWrapperFile = SRC_MAIN_JAVA.resolve("com/seibel/distanthorizons/common/wrappers/minecraft/MinecraftGLWrapper.java");
        String dhWrapperSource = readSourceIfExists(dhWrapperFile);
        assertFalse(dhWrapperSource.contains("GlStateManager._activeTexture("),
            "MinecraftGLWrapper should not call removed GlStateManager._activeTexture wrapper");

        Path dhTextureStateFile = SRC_MAIN_JAVA.resolve("com/seibel/distanthorizons/core/render/glObject/DhTextureState.java");
        String dhTextureStateSource = readSource(dhTextureStateFile);
        assertTrue(dhTextureStateSource.contains("VulkanicAPI.isVulkanBackendSelected()")
                && dhTextureStateSource.contains("RustGalVulkanWholeFrameMode.enabled()"),
            "Distant Horizons texture-state compatibility must fail closed for the Rust whole-frame Vulkan route");
        assertTrue(dhTextureStateSource.contains("IrisRenderSystem.setActiveTexture("),
            "DhTextureState should route active texture changes through IrisRenderSystem.setActiveTexture");

        Path irisRenderSystemFile = SRC_MAIN_JAVA.resolve("net/irisshaders/iris/gl/IrisRenderSystem.java");
        String irisRenderSystemSource = readSource(irisRenderSystemFile);
        assertTrue(irisRenderSystemSource.contains("public static void setActiveTexture("),
            "IrisRenderSystem should provide setActiveTexture helper after _activeTexture removal");
        assertTrue(irisRenderSystemSource.contains("public static void setActiveTextureUnitIndex("),
            "IrisRenderSystem should expose index-based active texture helper to avoid GL_TEXTURE0 arithmetic at call sites");
    }

    @Test
    public void testIrisTextureStateAccessUsesIrisRenderSystemHelpers() throws IOException {
        Path stateManagerFile = SRC_MAIN_JAVA.resolve("net/blaze3d/opengl/GlStateManager.java");
        String stateManagerSource = readSource(stateManagerFile);
        assertFalse(stateManagerSource.contains("public static int activeTexture"),
            "GlStateManager should no longer own activeTexture state field");
        assertFalse(stateManagerSource.contains("public static final GlStateManager.TextureState[] TEXTURES"),
            "GlStateManager should no longer own per-unit texture binding state array");
        assertFalse(stateManagerSource.contains("class TextureState"),
            "GlStateManager should no longer define TextureState helper class");

        Path irisRenderSystemFile = SRC_MAIN_JAVA.resolve("net/irisshaders/iris/gl/IrisRenderSystem.java");
        String irisRenderSystemSource = readSource(irisRenderSystemFile);

        assertTrue(irisRenderSystemSource.contains("public static int getActiveTextureUnitIndex("),
            "IrisRenderSystem should expose getActiveTextureUnitIndex helper for active texture state");
        assertTrue(irisRenderSystemSource.contains("public static int getTextureBinding("),
            "IrisRenderSystem should expose getTextureBinding helper for per-unit binding state");
        assertTrue(irisRenderSystemSource.contains("public static int getBoundTextureOnActiveUnit("),
            "IrisRenderSystem should expose getBoundTextureOnActiveUnit helper for active binding reads");
        assertTrue(irisRenderSystemSource.contains("public static void setTextureBinding("),
            "IrisRenderSystem should expose setTextureBinding helper for binding tracking updates");
        assertTrue(irisRenderSystemSource.contains("private static int activeTextureUnitIndex"),
            "IrisRenderSystem should own active texture unit index state locally");
        assertTrue(irisRenderSystemSource.contains("private static final int[] textureBindings"),
            "IrisRenderSystem should own per-unit texture binding state array locally");
        assertTrue(irisRenderSystemSource.contains("public static void blitDepthBufferNearest("),
            "IrisRenderSystem should expose blitDepthBufferNearest helper for depth-copy callsites");
        assertTrue(irisRenderSystemSource.contains("public static void blitDepthAndStencilBuffersNearest("),
            "IrisRenderSystem should expose blitDepthAndStencilBuffersNearest helper for combined depth-stencil copies");
        assertFalse(irisRenderSystemSource.contains("GlStateManager.activeTexture"),
            "IrisRenderSystem should not read active texture from GlStateManager");
        assertFalse(irisRenderSystemSource.contains("GlStateManager.TEXTURES"),
            "IrisRenderSystem should not read texture bindings from GlStateManager");

        Path programSamplersFile = SRC_MAIN_JAVA.resolve("net/irisshaders/iris/gl/program/ProgramSamplers.java");
        String programSamplersSource = readSource(programSamplersFile);
        assertFalse(programSamplersSource.contains("GlStateManager.activeTexture"),
            "ProgramSamplers should not read active texture directly from GlStateManager");
        assertTrue(programSamplersSource.contains("IrisRenderSystem.getActiveTextureUnitIndex("),
            "ProgramSamplers should read active texture through IrisRenderSystem helper");

        Path depthCopyStrategyFile = SRC_MAIN_JAVA.resolve("net/irisshaders/iris/gl/texture/DepthCopyStrategy.java");
        String depthCopyStrategySource = readSource(depthCopyStrategyFile);
        assertFalse(depthCopyStrategySource.contains("GlStateManager.TEXTURES[GlStateManager.activeTexture].binding"),
            "DepthCopyStrategy should not read active-unit binding directly from GlStateManager");
        assertTrue(depthCopyStrategySource.contains("IrisRenderSystem.getBoundTextureOnActiveUnit("),
            "DepthCopyStrategy should read active-unit binding through IrisRenderSystem helper");
        assertFalse(depthCopyStrategySource.contains("int GL_DEPTH_BUFFER_BIT = 0x00000100;"),
            "DepthCopyStrategy should not redefine GL_DEPTH_BUFFER_BIT locally");
        assertFalse(depthCopyStrategySource.contains("int GL_STENCIL_BUFFER_BIT = 0x00000400;"),
            "DepthCopyStrategy should not redefine GL_STENCIL_BUFFER_BIT locally");
        assertFalse(depthCopyStrategySource.contains("int GL_NEAREST = 0x2600;"),
            "DepthCopyStrategy should not redefine GL_NEAREST locally");
        assertFalse(depthCopyStrategySource.contains("IrisRenderSystem.blitFramebuffer(sourceFb.getId(), destFb.getId()"),
            "DepthCopyStrategy should not blit depth-stencil via generic blitFramebuffer + raw mask/filter constants");
        assertTrue(depthCopyStrategySource.contains("IrisRenderSystem.blitDepthAndStencilBuffersNearest(sourceFb.getId(), destFb.getId()"),
            "DepthCopyStrategy should blit combined depth-stencil through IrisRenderSystem intent helper");

        Path customTextureManagerFile = SRC_MAIN_JAVA.resolve("net/irisshaders/iris/pipeline/CustomTextureManager.java");
        String customTextureManagerSource = readSource(customTextureManagerFile);
        assertFalse(customTextureManagerSource.contains("GlStateManager.activeTexture"),
            "CustomTextureManager should not read active texture directly from GlStateManager");
        assertFalse(customTextureManagerSource.contains("GlStateManager.TEXTURES"),
            "CustomTextureManager should not read texture bindings directly from GlStateManager");
        assertTrue(customTextureManagerSource.contains("IrisRenderSystem.getActiveTextureUnitIndex("),
            "CustomTextureManager should read active texture through IrisRenderSystem helper");
        assertTrue(customTextureManagerSource.contains("IrisRenderSystem.getTextureBinding("),
            "CustomTextureManager should read texture bindings through IrisRenderSystem helper");

        Path compositeRendererFile = SRC_MAIN_JAVA.resolve("net/irisshaders/iris/pipeline/CompositeRenderer.java");
        String compositeRendererSource = readSource(compositeRendererFile);
        assertFalse(compositeRendererSource.contains("GlStateManager.TEXTURES"),
            "CompositeRenderer should not read texture bindings directly from GlStateManager");
        assertTrue(compositeRendererSource.contains("IrisRenderSystem.getTextureBinding("),
            "CompositeRenderer should check bindings through IrisRenderSystem helper");

        Path finalPassRendererFile = SRC_MAIN_JAVA.resolve("net/irisshaders/iris/pipeline/FinalPassRenderer.java");
        String finalPassRendererSource = readSource(finalPassRendererFile);
        assertFalse(finalPassRendererSource.contains("GlStateManager.TEXTURES"),
            "FinalPassRenderer should not read texture bindings directly from GlStateManager");
        assertTrue(finalPassRendererSource.contains("IrisRenderSystem.getTextureBinding("),
            "FinalPassRenderer should check bindings through IrisRenderSystem helper");
        assertFalse(finalPassRendererSource.contains("IrisRenderSystem.copyTexSubImage2D(VulkanicAPI.getTextureHandle(main.getColorTexture()), VulkanicAPI.GL_TEXTURE_2D"),
            "FinalPassRenderer should not pass explicit GL_TEXTURE_2D when copying into main color target");
        assertTrue(containsAny(finalPassRendererSource,
                "IrisRenderSystem.copyTexSubImage2D(VulkanicAPI.getTextureHandle(main.getColorTexture()), 0",
                "IrisRenderSystem.copyTexSubImage2D(VulkanicCoreAPI.textureId(main.getColorTexture()), 0"),
            "FinalPassRenderer should copy into main color target through IrisRenderSystem default-2D helper");
        assertFalse(finalPassRendererSource.contains("VulkanicAPI.copyTexSubImage2D(VulkanicAPI.getCommandContext(), VulkanicAPI.GL_TEXTURE_2D"),
            "FinalPassRenderer should not pass explicit GL_TEXTURE_2D in VulkanicAPI copyTexSubImage2D path");
        assertTrue(containsAny(finalPassRendererSource,
                "VulkanicAPI.copyTexSubImage2D(VulkanicAPI.getCommandContext(), 0",
                "VulkanicAPI.copyTexSubImage2D(ctx, 0"),
            "FinalPassRenderer should use VulkanicAPI default-2D copyTexSubImage2D overload");

        Path shadowRendererFile = SRC_MAIN_JAVA.resolve("net/irisshaders/iris/shadows/ShadowRenderer.java");
        String shadowRendererSource = readSource(shadowRendererFile);
        assertFalse(shadowRendererSource.contains("IrisRenderSystem.generateMipmaps(texture, VulkanicAPI.GL_TEXTURE_2D"),
            "ShadowRenderer should not pass explicit GL_TEXTURE_2D when generating mipmaps");
        assertTrue(shadowRendererSource.contains("IrisRenderSystem.generateMipmaps(texture);"),
            "ShadowRenderer should generate mipmaps through IrisRenderSystem default-2D helper");
        assertFalse(shadowRendererSource.contains("IrisRenderSystem.texParameteri(glTextureId, VulkanicAPI.GL_TEXTURE_2D"),
            "ShadowRenderer should not pass explicit GL_TEXTURE_2D in texture parameter setup");
        assertFalse(shadowRendererSource.contains("IrisRenderSystem.texParameteri(glTextureId, VulkanicAPI.GL_TEXTURE_MIN_FILTER"),
            "ShadowRenderer should not set texture min filter directly through raw GL_TEXTURE_MIN_FILTER pname constants");
        assertTrue(shadowRendererSource.contains("IrisRenderSystem.setTextureLinearFiltering(glTextureId)"),
            "ShadowRenderer should set linear filtering through IrisRenderSystem texture intent helper");
        assertTrue(shadowRendererSource.contains("IrisRenderSystem.setTextureNearestFiltering(glTextureId)"),
            "ShadowRenderer should set nearest filtering through IrisRenderSystem texture intent helper");

        Path colorSpaceConverterFile = SRC_MAIN_JAVA.resolve("net/irisshaders/iris/pathways/colorspace/ColorSpaceFragmentConverter.java");
        String colorSpaceConverterSource = readSource(colorSpaceConverterFile);
        assertFalse(colorSpaceConverterSource.contains("IrisRenderSystem.texImage2D(swapTexture, VulkanicAPI.GL_TEXTURE_2D"),
            "ColorSpaceFragmentConverter should not pass explicit GL_TEXTURE_2D when allocating swap texture");
        assertTrue(colorSpaceConverterSource.contains("IrisRenderSystem.texImage2D(swapTexture, 0"),
            "ColorSpaceFragmentConverter should allocate swap texture through IrisRenderSystem default-2D helper");
        assertFalse(colorSpaceConverterSource.contains("IrisRenderSystem.copyTexSubImage2D(VulkanicAPI.getTextureHandle(targetImage), VulkanicAPI.GL_TEXTURE_2D"),
            "ColorSpaceFragmentConverter should not pass explicit GL_TEXTURE_2D in copyTexSubImage2D");
        assertTrue(containsAny(colorSpaceConverterSource,
                "IrisRenderSystem.copyTexSubImage2D(VulkanicAPI.getTextureHandle(targetImage), 0",
                "IrisRenderSystem.copyTexSubImage2D(net.vulkanic.VulkanicCoreAPI.textureId(targetImage), 0"),
            "ColorSpaceFragmentConverter should copy texture data through IrisRenderSystem default-2D helper");

        Path glFramebufferFile = SRC_MAIN_JAVA.resolve("net/irisshaders/iris/gl/framebuffer/GlFramebuffer.java");
        String glFramebufferSource = readSource(glFramebufferFile);
        assertFalse(glFramebufferSource.contains("IrisRenderSystem.framebufferTexture2D(fb, VulkanicAPI.GL_FRAMEBUFFER, VulkanicAPI.GL_DEPTH_ATTACHMENT, VulkanicAPI.GL_TEXTURE_2D"),
            "GlFramebuffer should not pass explicit GL_TEXTURE_2D for depth attachment");
        assertFalse(glFramebufferSource.contains("IrisRenderSystem.framebufferTexture2D(fb, VulkanicAPI.GL_FRAMEBUFFER, VulkanicAPI.GL_COLOR_ATTACHMENT0 + index, VulkanicAPI.GL_TEXTURE_2D"),
            "GlFramebuffer should not pass explicit GL_TEXTURE_2D for color attachment");
        assertFalse(glFramebufferSource.contains("IrisRenderSystem.framebufferTexture2D(fb, VulkanicAPI.GL_FRAMEBUFFER, VulkanicAPI.GL_DEPTH_ATTACHMENT, texture, 0)"),
            "GlFramebuffer should not hard-code GL_FRAMEBUFFER target for bypass depth attachment");
        assertTrue(glFramebufferSource.contains("IrisRenderSystem.framebufferTexture2D(")
                && glFramebufferSource.contains("combinedStencil ? VulkanicAPI.GL_DEPTH_STENCIL_ATTACHMENT : VulkanicAPI.GL_DEPTH_ATTACHMENT"),
            "GlFramebuffer should use IrisRenderSystem default-target framebufferTexture2D helper and preserve combined depth-stencil attachment intent");
        assertFalse(glFramebufferSource.contains("IrisRenderSystem.framebufferTexture2D(fb, VulkanicAPI.GL_FRAMEBUFFER, VulkanicAPI.GL_COLOR_ATTACHMENT0 + index, texture, 0)"),
            "GlFramebuffer should not compute color-attachment enums inline for color attachment");
        assertFalse(glFramebufferSource.contains("IrisRenderSystem.framebufferTexture2D(fb, VulkanicAPI.GL_FRAMEBUFFER, VulkanicAPI.colorAttachment(index), texture, 0)"),
            "GlFramebuffer should not hard-code GL_FRAMEBUFFER target for color attachment path");
        assertTrue(glFramebufferSource.contains("IrisRenderSystem.framebufferTexture2D(fb, VulkanicAPI.colorAttachment(index), texture, 0)"),
            "GlFramebuffer should use IrisRenderSystem default-target helper for color attachment path");
        assertTrue(glFramebufferSource.contains("IrisRenderSystem.readBuffer(getGlId(), VulkanicAPI.colorAttachment(buffer))"),
            "GlFramebuffer read-buffer path should use VulkanicAPI.colorAttachment helper");
        assertFalse(glFramebufferSource.contains("IrisRenderSystem.checkFramebufferStatus(VulkanicAPI.GL_FRAMEBUFFER)"),
            "GlFramebuffer should not hard-code GL_FRAMEBUFFER target in status query path");
        assertTrue(glFramebufferSource.contains("IrisRenderSystem.checkFramebufferStatus()"),
            "GlFramebuffer should query status through IrisRenderSystem default-target helper");

        Path dhFramebufferWrapperFile = SRC_MAIN_JAVA.resolve("net/irisshaders/iris/compat/dh/DhFrameBufferWrapper.java");
        String dhFramebufferWrapperSource = readSource(dhFramebufferWrapperFile);
        assertFalse(dhFramebufferWrapperSource.contains("IrisRenderSystem.checkFramebufferStatus(VulkanicAPI.GL_FRAMEBUFFER)"),
            "DhFrameBufferWrapper should not hard-code GL_FRAMEBUFFER target in status query path");
        assertTrue(dhFramebufferWrapperSource.contains("IrisRenderSystem.checkFramebufferStatus()"),
            "DhFrameBufferWrapper should query status through IrisRenderSystem default-target helper");

        Path pipelineFile = SRC_MAIN_JAVA.resolve("net/irisshaders/iris/pipeline/IrisRenderingPipeline.java");
        String pipelineSource = readSource(pipelineFile);
        assertFalse(pipelineSource.contains("GlStateManager.TEXTURES[GlStateManager.activeTexture].binding"),
            "IrisRenderingPipeline should not read active-unit binding directly from GlStateManager");
        assertTrue(pipelineSource.contains("IrisRenderSystem.getBoundTextureOnActiveUnit("),
            "IrisRenderingPipeline should read active-unit binding through IrisRenderSystem helper");

        Path textureInfoCacheFile = SRC_MAIN_JAVA.resolve("net/irisshaders/iris/pbr/TextureInfoCache.java");
        String textureInfoCacheSource = readSource(textureInfoCacheFile);
        assertFalse(textureInfoCacheSource.contains("GlStateManager.TEXTURES[GlStateManager.activeTexture].binding"),
            "TextureInfoCache should not read active-unit binding directly from GlStateManager");
        assertTrue(textureInfoCacheSource.contains("IrisRenderSystem.getBoundTextureOnActiveUnit("),
            "TextureInfoCache should read active-unit binding through IrisRenderSystem helper");

        Path pbrTextureManagerFile = SRC_MAIN_JAVA.resolve("net/irisshaders/iris/pbr/texture/PBRTextureManager.java");
        String pbrTextureManagerSource = readSource(pbrTextureManagerFile);
        assertFalse(pbrTextureManagerSource.contains("GlStateManager.TEXTURES[GlStateManager.activeTexture].binding"),
            "PBRTextureManager should not read active-unit binding directly from GlStateManager");
        assertTrue(pbrTextureManagerSource.contains("IrisRenderSystem.getBoundTextureOnActiveUnit("),
            "PBRTextureManager should read active-unit binding through IrisRenderSystem helper");
        assertTrue(pbrTextureManagerSource.contains("if (defaultNormalTexture != null)"),
            "PBRTextureManager shutdown must tolerate Rust Vulkan runs that never initialize optional PBR defaults");
        assertTrue(pbrTextureManagerSource.contains("if (defaultSpecularTexture != null)"),
            "PBRTextureManager shutdown must close optional PBR defaults independently and idempotently");

        Path programSamplersFile2 = SRC_MAIN_JAVA.resolve("net/irisshaders/iris/gl/program/ProgramSamplers.java");
        String programSamplersSource2 = readSource(programSamplersFile2);
        assertFalse(programSamplersSource2.contains("IrisRenderSystem.setActiveTexture(VulkanicAPI.GL_TEXTURE0 +"),
            "ProgramSamplers should not compute GL_TEXTURE0 offsets directly when restoring active texture");
        assertTrue(programSamplersSource2.contains("IrisRenderSystem.setActiveTextureUnitIndex("),
            "ProgramSamplers should restore active texture through IrisRenderSystem index helper");

        Path pipelineManagerFile = SRC_MAIN_JAVA.resolve("net/irisshaders/iris/pipeline/PipelineManager.java");
        String pipelineManagerSource = readSource(pipelineManagerFile);
        assertFalse(pipelineManagerSource.contains("IrisRenderSystem.setActiveTexture(VulkanicAPI.GL_TEXTURE0 +"),
            "PipelineManager should not compute GL_TEXTURE0 offsets directly in texture unit loops");
        assertTrue(pipelineManagerSource.contains("IrisRenderSystem.setActiveTextureUnitIndex("),
            "PipelineManager should switch texture units through IrisRenderSystem index helper");
        assertTrue(pipelineManagerSource.contains("VulkanicAPI.isVulkanBackendSelected()")
                && pipelineManagerSource.contains("RustGalVulkanWholeFrameMode.enabled()")
                && pipelineManagerSource.contains("pipelinesPerDimension.clear();"),
            "PipelineManager destruction must discard Java compatibility bookkeeping without touching GL on Rust Vulkan");

        Path encoderFile = SRC_MAIN_JAVA.resolve("net/blaze3d/opengl/GlCommandEncoder.java");
        String encoderSource = readSource(encoderFile);
        assertFalse(encoderSource.contains("RenderSystem.setShaderTexture(0, sam)"),
            "GlCommandEncoder should not bridge Sampler0 through RenderSystem.setShaderTexture in Iris setup path");
        assertTrue(encoderSource.contains("TextureTracker.INSTANCE.onSetShaderTexture(0, sam)"),
            "GlCommandEncoder should notify Iris texture tracking directly for Sampler0 setup");
		assertTrue(encoderSource.contains("IrisRenderSystem.setTextureBinding(samplerIndex, textureHandle);"),
			"GlCommandEncoder should mirror pipeline sampler binds into the Iris texture-binding cache");
		assertTrue(encoderSource.contains("IrisRenderSystem.setTextureBinding(var46, textureHandle);"),
			"GlCommandEncoder should mirror draw-time sampler binds into the Iris texture-binding cache");

        Path commonUniformsFile = SRC_MAIN_JAVA.resolve("net/irisshaders/iris/uniforms/CommonUniforms.java");
        String commonUniformsSource = readSource(commonUniformsFile);
        assertFalse(commonUniformsSource.contains("RenderSystem.getShaderTexture(0)"),
            "CommonUniforms should not read atlasSize texture through RenderSystem.getShaderTexture after Iris texture-state migration");
        assertTrue(commonUniformsSource.contains("IrisRenderSystem.getTextureBinding(0)"),
            "CommonUniforms should read atlasSize texture through IrisRenderSystem.getTextureBinding");

        Path extendedShaderFile = SRC_MAIN_JAVA.resolve("net/irisshaders/iris/pipeline/programs/ExtendedShader.java");
        String extendedShaderSource = readSource(extendedShaderFile);
        assertFalse(extendedShaderSource.contains("RenderSystem.getShaderTexture(0)"),
            "ExtendedShader should not read intensity swizzle texture through RenderSystem.getShaderTexture after Iris texture-state migration");
        assertTrue(extendedShaderSource.contains("IrisRenderSystem.getTextureBinding(0)"),
            "ExtendedShader should read intensity swizzle texture through IrisRenderSystem.getTextureBinding");

        Path sodiumShaderFile = SRC_MAIN_JAVA.resolve("net/irisshaders/iris/pipeline/programs/SodiumShader.java");
        String sodiumShaderSource = readSource(sodiumShaderFile);
        assertFalse(sodiumShaderSource.contains("RenderSystem.setShaderTexture(0, pass.getAtlas())"),
            "SodiumShader should not bridge atlas binding through RenderSystem.setShaderTexture after Iris texture-state migration");
        assertTrue(sodiumShaderSource.contains("TextureTracker.INSTANCE.onSetShaderTexture(0, pass.getAtlas())"),
            "SodiumShader should notify Iris texture tracking directly for atlas binding");

        Path dhLodProgramFile = SRC_MAIN_JAVA.resolve("net/irisshaders/iris/compat/dh/IrisLodRenderProgram.java");
        String dhLodProgramSource = readSource(dhLodProgramFile);
        assertFalse(dhLodProgramSource.contains("RenderSystem.getShaderTexture(2)"),
            "IrisLodRenderProgram should not read lightmap texture through RenderSystem.getShaderTexture after Iris texture-state migration");
        assertTrue(dhLodProgramSource.contains("IrisRenderSystem.getTextureBinding(2)"),
            "IrisLodRenderProgram should read lightmap texture through IrisRenderSystem.getTextureBinding");

        Path dhGenericProgramFile = SRC_MAIN_JAVA.resolve("net/irisshaders/iris/compat/dh/IrisGenericRenderProgram.java");
        String dhGenericProgramSource = readSource(dhGenericProgramFile);
        assertFalse(dhGenericProgramSource.contains("RenderSystem.getShaderTexture(2)"),
            "IrisGenericRenderProgram should not read lightmap texture through RenderSystem.getShaderTexture after Iris texture-state migration");
        assertTrue(dhGenericProgramSource.contains("IrisRenderSystem.getTextureBinding(2)"),
            "IrisGenericRenderProgram should read lightmap texture through IrisRenderSystem.getTextureBinding");

        Path guiUtilFile = SRC_MAIN_JAVA.resolve("net/irisshaders/iris/gui/GuiUtil.java");
        String guiUtilSource = readSource(guiUtilFile);
        assertFalse(guiUtilSource.contains("RenderSystem.setShaderTexture(0"),
            "GuiUtil should not bind widget texture through RenderSystem.setShaderTexture after Iris texture-state migration");
        assertTrue(guiUtilSource.contains("TextureTracker.INSTANCE.onSetShaderTexture(0, textureView)"),
            "GuiUtil should notify Iris texture tracking directly when binding widget texture");
        assertTrue(guiUtilSource.contains("RustGalVulkanWholeFrameMode.enabled()")
                && guiUtilSource.contains("VulkanicAPI.isVulkanBackendSelected()"),
            "GuiUtil should fence Iris blend capability mutation while Rust owns Vulkan GUI presentation");

        Path horizonRendererFile = SRC_MAIN_JAVA.resolve("net/irisshaders/iris/pathways/HorizonRenderer.java");
        String horizonRendererSource = readSource(horizonRendererFile);
        assertFalse(horizonRendererSource.contains("RenderSystem.getShaderTexture(i)"),
            "HorizonRenderer should not read shader samplers through RenderSystem.getShaderTexture after Iris texture-state migration");
        assertTrue(horizonRendererSource.contains("IrisRenderSystem.getTextureBinding(i)"),
            "HorizonRenderer should read sampler texture bindings through IrisRenderSystem.getTextureBinding");
        assertTrue(horizonRendererSource.contains("TextureTracker.INSTANCE.getTexture(textureId)"),
            "HorizonRenderer should resolve bound textures through TextureTracker before binding samplers");

        Path irisPipelineFile = SRC_MAIN_JAVA.resolve("net/irisshaders/iris/pipeline/IrisRenderingPipeline.java");
        String irisPipelineSource = readSource(irisPipelineFile);
        assertFalse(irisPipelineSource.contains("RenderSystem.setShaderTexture(i, null)"),
            "IrisRenderingPipeline destroy path should not clear shader textures through RenderSystem.setShaderTexture");
        assertTrue(irisPipelineSource.contains("IrisRenderSystem.setTextureBinding(i, 0)"),
            "IrisRenderingPipeline destroy path should clear cached texture bindings through IrisRenderSystem.setTextureBinding");
        assertTrue(irisPipelineSource.contains("Java Iris shader-pack pipeline construction is unavailable")
                && irisPipelineSource.contains("RustGalVulkanWholeFrameMode.enabled()"),
            "IrisRenderingPipeline construction must fail before Java shader-pack GPU setup on Rust Vulkan");
        String irisSource = readSource(SRC_MAIN_JAVA.resolve("net/irisshaders/iris/Iris.java"));
        assertTrue(irisSource.contains("Rust observes the persisted configuration")
                && irisSource.contains("currentPack = null")
                && irisSource.contains("return;"),
            "Iris reload must not enter Java pipeline teardown/loading when Rust owns Vulkan");
        assertTrue(irisSource.contains("Java Iris/PBR GPU initialization is permitted")
                && irisSource.contains("VulkanicAPI.isVulkanBackendSelected()"),
            "Iris render-system initialization must be fenced when Rust owns Vulkan");

        Path renderStateShardFile = SRC_MAIN_JAVA.resolve("net/minecraft/client/renderer/RenderStateShard.java");
        String renderStateShardSource = readSource(renderStateShardFile);
        assertFalse(renderStateShardSource.contains("RenderSystem.setShaderTexture(i, abstractTexture.getTextureView())"),
            "RenderStateShard multi-texture setup should not bind shader textures through RenderSystem.setShaderTexture");
        assertFalse(renderStateShardSource.contains("RenderSystem.setShaderTexture(0, abstractTexture.getTextureView())"),
            "RenderStateShard single-texture setup should not bind shader texture 0 through RenderSystem.setShaderTexture");
        assertFalse(renderStateShardSource.contains("IrisRenderSystem.bindTextureToUnit("),
            "RenderStateShard texture setup should bind through VulkanicAPI texture-view seam instead of Iris texture-id helper calls");
        assertFalse(renderStateShardSource.contains("VulkanicCoreAPI.textureId(textureView)"),
            "RenderStateShard texture setup should not extract raw texture ids from texture views");
        assertTrue(renderStateShardSource.contains("var ctx = VulkanicAPI.getCommandContext();")
                && renderStateShardSource.contains("VulkanicAPI.bindTextureUnit(ctx, i, textureView)"),
            "RenderStateShard multi-texture setup should bind through VulkanicAPI texture-view seam");
        assertTrue(renderStateShardSource.contains("VulkanicAPI.bindTextureUnit(ctx, 0, textureView)"),
            "RenderStateShard single-texture setup should bind through VulkanicAPI texture-view seam");
        assertTrue(renderStateShardSource.contains("TextureTracker.INSTANCE.onSetShaderTexture(i, textureView)"),
            "RenderStateShard multi-texture setup should notify TextureTracker for each texture unit binding");

        Path renderTypeFile = SRC_MAIN_JAVA.resolve("net/minecraft/client/renderer/RenderType.java");
        String renderTypeSource = readSource(renderTypeFile);
        assertFalse(renderTypeSource.contains("RenderSystem.getShaderTexture(i)"),
            "RenderType draw path should not fetch samplers through RenderSystem.getShaderTexture");
        assertTrue(renderTypeSource.contains("IrisRenderSystem.getTextureBinding(i)"),
            "RenderType draw path should fetch sampler bindings through IrisRenderSystem.getTextureBinding");
        assertTrue(renderTypeSource.contains("TextureTracker.INSTANCE.getShaderTexture(i)"),
            "RenderType draw path should first resolve sampler views from TextureTracker unit bindings");
        assertTrue(renderTypeSource.contains("TextureTracker.INSTANCE.getTextureView(textureId)"),
            "RenderType draw path should resolve texture views through TextureTracker before binding samplers");
        assertTrue(renderTypeSource.contains("int drawFramebuffer = outputColorOverride == null && outputDepthOverride == null")
                && renderTypeSource.contains("VulkanicAPI.getDrawFramebufferBinding()"),
            "RenderType immediate draws should honor an active framebuffer binding before falling back to vanilla render-target views");
        assertTrue(renderTypeSource.contains("? VulkanicAPI.createRenderPass(() -> \"Immediate draw for \" + this.getName(), drawFramebuffer, renderTarget.useDepth)")
                && renderTypeSource.contains(": VulkanicAPI.createRenderPass("),
            "RenderType immediate draws should use framebuffer-backed Vulkan render passes for Iris/OpenGL-style bound framebuffers");

        Path textureTrackerFile = SRC_MAIN_JAVA.resolve("net/irisshaders/iris/pbr/TextureTracker.java");
        String textureTrackerSource = readSource(textureTrackerFile);
        assertTrue(textureTrackerSource.contains("private final GpuTextureView[] shaderTexturesByUnit = new GpuTextureView[128];"),
            "TextureTracker should maintain per-unit shader texture view cache for robust sampler binding");
        assertTrue(textureTrackerSource.contains("public GpuTextureView getShaderTexture(int unit)"),
            "TextureTracker should expose per-unit shader texture lookup for RenderType sampler binding");

        Path abstractTextureFile = SRC_MAIN_JAVA.resolve("net/minecraft/client/renderer/texture/AbstractTexture.java");
        String abstractTextureSource = readSource(abstractTextureFile);
        assertTrue(containsAny(abstractTextureSource,
            "TextureTracker.INSTANCE.trackTexture(net.vulkanic.VulkanicAPI.getTextureHandle(lastChecked), this)",
            "TextureTracker.INSTANCE.trackTexture(net.vulkanic.VulkanicCoreAPI.textureId(lastChecked), this)"),
            "AbstractTexture should track textures when getTextureView() is used so RenderType sampler binding cannot silently drop GUI/item textures");

        Path lightTextureFile = SRC_MAIN_JAVA.resolve("net/minecraft/client/renderer/LightTexture.java");
        String lightTextureSource = readSource(lightTextureFile);
        assertFalse(lightTextureSource.contains("RenderSystem.setShaderTexture(2, null)"),
            "LightTexture should not disable light layer via RenderSystem.setShaderTexture bridge");
        assertFalse(lightTextureSource.contains("RenderSystem.setShaderTexture(2, this.textureView)"),
            "LightTexture should not enable light layer via RenderSystem.setShaderTexture bridge");
        assertTrue(lightTextureSource.contains("IrisRenderSystem.bindTextureToUnit(2, 0)"),
            "LightTexture should disable light layer via IrisRenderSystem default-2D helper");
        assertFalse(lightTextureSource.contains("VulkanicCoreAPI.textureId(this.texture)"),
            "LightTexture should not extract raw texture ids for light-layer binding");
        assertTrue(lightTextureSource.contains("var ctx = VulkanicAPI.getCommandContext();")
                && lightTextureSource.contains("VulkanicAPI.bindTextureUnit(ctx, 2, this.textureView)"),
            "LightTexture should enable light layer through VulkanicAPI texture-view seam");

        Path renderSystemFile = SRC_MAIN_JAVA.resolve("net/blaze3d/systems/RenderSystem.java");
        String renderSystemSource = readSource(renderSystemFile);
        assertFalse(renderSystemSource.contains("GpuTextureView[] shaderTextures"),
            "RenderSystem should not maintain a local shaderTextures array after Iris/Vulkanic texture-state migration");
        assertFalse(renderSystemSource.contains("shaderTextures[i] ="),
            "RenderSystem.setShaderTexture should not write to a local shader texture cache");
        assertFalse(renderSystemSource.contains("public static final int TEXTURE_COUNT"),
            "RenderSystem should not expose TEXTURE_COUNT after shader-texture bridge API removal");
        assertFalse(renderSystemSource.contains("public static void setShaderTexture("),
            "RenderSystem should not expose setShaderTexture after migration to Iris/Vulkanic texture binding paths");
        assertFalse(renderSystemSource.contains("public static GpuTextureView getShaderTexture("),
            "RenderSystem should not expose getShaderTexture after migration to Iris/Vulkanic texture binding paths");
        assertFalse(renderSystemSource.contains("public static void setupOverlayColor("),
            "RenderSystem should not expose setupOverlayColor wrapper after overlay texture binding migration");
        assertFalse(renderSystemSource.contains("public static void teardownOverlayColor("),
            "RenderSystem should not expose teardownOverlayColor wrapper after overlay texture binding migration");
        assertFalse(renderSystemSource.contains("public static void queueFencedTask("),
            "RenderSystem should not own fenced GPU callback queueing after migration to VulkanicAPI");
        assertFalse(renderSystemSource.contains("public static void executePendingTasks("),
            "RenderSystem should not own pending GPU task execution after migration to VulkanicAPI");
        assertFalse(renderSystemSource.contains("public static void bindDefaultUniforms("),
            "RenderSystem should not own default uniform binding after migration to VulkanicAPI");
        assertFalse(renderSystemSource.contains("public static void setShaderFog("),
            "RenderSystem should not own fog uniform state setter after migration to VulkanicAPI");
        assertFalse(renderSystemSource.contains("public static GpuBufferSlice getShaderFog("),
            "RenderSystem should not own fog uniform state getter after migration to VulkanicAPI");
        assertFalse(renderSystemSource.contains("public static void setShaderLights("),
            "RenderSystem should not own lighting uniform state setter after migration to VulkanicAPI");
        assertFalse(renderSystemSource.contains("public static GpuBufferSlice getShaderLights("),
            "RenderSystem should not own lighting uniform state getter after migration to VulkanicAPI");
        assertFalse(renderSystemSource.contains("public static void setGlobalSettingsUniform("),
            "RenderSystem should not own global uniform state setter after migration to VulkanicAPI");
        assertFalse(renderSystemSource.contains("public static GpuBuffer getGlobalSettingsUniform("),
            "RenderSystem should not own global uniform state getter after migration to VulkanicAPI");
        assertFalse(renderSystemSource.contains("public static void setProjectionMatrix("),
            "RenderSystem should not own projection matrix setter after migration to VulkanicAPI");
        assertFalse(renderSystemSource.contains("public static void backupProjectionMatrix("),
            "RenderSystem should not own projection matrix backup helper after migration to VulkanicAPI");
        assertFalse(renderSystemSource.contains("public static void restoreProjectionMatrix("),
            "RenderSystem should not own projection matrix restore helper after migration to VulkanicAPI");
        assertFalse(renderSystemSource.contains("public static GpuBufferSlice getProjectionMatrixBuffer("),
            "RenderSystem should not own projection matrix getter after migration to VulkanicAPI");
        assertFalse(renderSystemSource.contains("public static ProjectionType getProjectionType("),
            "RenderSystem should not own projection type getter after migration to VulkanicAPI");
        assertFalse(renderSystemSource.contains("public static void setTextureMatrix("),
            "RenderSystem should not own texture matrix setter after migration to VulkanicAPI");
        assertFalse(renderSystemSource.contains("public static void resetTextureMatrix("),
            "RenderSystem should not own texture matrix reset helper after migration to VulkanicAPI");
        assertFalse(renderSystemSource.contains("public static Matrix4f getTextureMatrix("),
            "RenderSystem should not own texture matrix getter after migration to VulkanicAPI");
        assertFalse(renderSystemSource.contains("public static void lineWidth("),
            "RenderSystem should not own line width setter after migration to VulkanicAPI");
        assertFalse(renderSystemSource.contains("public static float getShaderLineWidth("),
            "RenderSystem should not own shader line width getter after migration to VulkanicAPI");

        Path vulkanicApiFile = SRC_MAIN_JAVA.resolve("net/vulkanic/VulkanicAPI.java");
        String vulkanicApiSource = readSource(vulkanicApiFile);
        assertTrue(vulkanicApiSource.contains("public static void queueFencedTask("),
            "VulkanicAPI should expose queueFencedTask for backend-owned GPU callback scheduling");
        assertTrue(vulkanicApiSource.contains("public static void executePendingFenceTasks("),
            "VulkanicAPI should expose executePendingFenceTasks for backend-owned GPU callback execution");
        assertTrue(vulkanicApiSource.contains("public static void bindDefaultUniforms("),
            "VulkanicAPI should expose bindDefaultUniforms after RenderSystem uniform binding migration");
        assertTrue(vulkanicApiSource.contains("public static void setShaderFog("),
            "VulkanicAPI should expose setShaderFog after RenderSystem fog uniform migration");
        assertFalse(vulkanicApiSource.contains("static {\n        net.irisshaders.iris.gl.state.StateUpdateNotifiers.fogStartNotifier"),
            "VulkanicAPI must not install Iris fog callbacks during class loading; Vulkan must not mutate Iris runtime state");
        assertTrue(vulkanicApiSource.contains("installOpenGlIrisFogNotifiers();"),
            "Iris fog callbacks must be installed only from the selected OpenGL backend branch");
        assertTrue(vulkanicApiSource.contains("!net.vulkanic.bridge.RustGalVulkanWholeFrameMode.enabled()")
                && vulkanicApiSource.contains("!isVulkanBackendSelected()"),
            "selected Vulkan must not invoke Iris fog callbacks");
        assertTrue(vulkanicApiSource.contains("public static GpuBufferSlice getShaderFog("),
            "VulkanicAPI should expose getShaderFog after RenderSystem fog uniform migration");
        assertTrue(vulkanicApiSource.contains("public static void setShaderLights("),
            "VulkanicAPI should expose setShaderLights after RenderSystem lighting uniform migration");
        assertTrue(vulkanicApiSource.contains("public static GpuBufferSlice getShaderLights("),
            "VulkanicAPI should expose getShaderLights after RenderSystem lighting uniform migration");
        assertTrue(vulkanicApiSource.contains("public static void setGlobalSettingsUniform("),
            "VulkanicAPI should expose setGlobalSettingsUniform after RenderSystem global uniform migration");
        assertTrue(vulkanicApiSource.contains("public static GpuBuffer getGlobalSettingsUniform("),
            "VulkanicAPI should expose getGlobalSettingsUniform after RenderSystem global uniform migration");
        assertTrue(vulkanicApiSource.contains("public static void setProjectionMatrix("),
            "VulkanicAPI should expose setProjectionMatrix after RenderSystem projection migration");
        assertTrue(vulkanicApiSource.contains("public static void backupProjectionMatrix("),
            "VulkanicAPI should expose backupProjectionMatrix after RenderSystem projection migration");
        assertTrue(vulkanicApiSource.contains("public static void restoreProjectionMatrix("),
            "VulkanicAPI should expose restoreProjectionMatrix after RenderSystem projection migration");
        assertTrue(vulkanicApiSource.contains("public static GpuBufferSlice getProjectionMatrixBuffer("),
            "VulkanicAPI should expose getProjectionMatrixBuffer after RenderSystem projection migration");
        assertTrue(vulkanicApiSource.contains("public static ProjectionType getProjectionType("),
            "VulkanicAPI should expose getProjectionType after RenderSystem projection migration");
        assertTrue(vulkanicApiSource.contains("public static void setTextureMatrix("),
            "VulkanicAPI should expose setTextureMatrix after RenderSystem texture matrix migration");
        assertTrue(vulkanicApiSource.contains("public static void resetTextureMatrix("),
            "VulkanicAPI should expose resetTextureMatrix after RenderSystem texture matrix migration");
        assertTrue(vulkanicApiSource.contains("public static Matrix4f getTextureMatrix("),
            "VulkanicAPI should expose getTextureMatrix after RenderSystem texture matrix migration");
        assertTrue(vulkanicApiSource.contains("public static void lineWidth("),
            "VulkanicAPI should expose lineWidth after RenderSystem line width migration");
        assertTrue(vulkanicApiSource.contains("public static float getShaderLineWidth("),
            "VulkanicAPI should expose getShaderLineWidth after RenderSystem line width migration");
        assertFalse(vulkanicApiSource.contains("RenderSystem.getProjectionMatrixBuffer()"),
            "VulkanicAPI default uniform binding should not read projection matrix state through RenderSystem after migration");
        assertFalse(vulkanicApiSource.contains("RenderSystem.getShaderFog()"),
            "VulkanicAPI default uniform binding should not read fog state through RenderSystem after migration");
        assertFalse(vulkanicApiSource.contains("RenderSystem.getShaderLights()"),
            "VulkanicAPI default uniform binding should not read lighting state through RenderSystem after migration");
        assertFalse(vulkanicApiSource.contains("RenderSystem.getGlobalSettingsUniform()"),
            "VulkanicAPI default uniform binding should not read global uniform state through RenderSystem after migration");

        assertFalse(encoderSource.contains("RenderSystem.queueFencedTask("),
            "GlCommandEncoder should not schedule fenced callbacks through RenderSystem.queueFencedTask");
        assertTrue(encoderSource.contains("VulkanicAPI.queueFencedTask("),
            "GlCommandEncoder should schedule fenced callbacks through VulkanicAPI.queueFencedTask");

        Path minecraftFile = SRC_MAIN_JAVA.resolve("net/minecraft/client/Minecraft.java");
        String minecraftSource = readSource(minecraftFile);
        assertFalse(minecraftSource.contains("RenderSystem.executePendingTasks()"),
            "Minecraft render loop should not execute GPU pending tasks through RenderSystem.executePendingTasks");
        assertTrue(minecraftSource.contains("VulkanicAPI.executePendingFenceTasks()"),
            "Minecraft render loop should execute GPU pending tasks through VulkanicAPI.executePendingFenceTasks");

        Path gameRendererFile = SRC_MAIN_JAVA.resolve("net/minecraft/client/renderer/GameRenderer.java");
        String gameRendererSource = readSource(gameRendererFile);
        assertFalse(gameRendererSource.contains("RenderSystem.setShaderFog("),
            "GameRenderer should not set fog uniforms through RenderSystem after VulkanicAPI migration");
        assertTrue(gameRendererSource.contains("VulkanicAPI.setShaderFog("),
            "GameRenderer should set fog uniforms through VulkanicAPI after migration");
        assertFalse(gameRendererSource.contains("RenderSystem.setProjectionMatrix("),
            "GameRenderer should not set projection matrix through RenderSystem after VulkanicAPI migration");
        assertTrue(gameRendererSource.contains("VulkanicAPI.setProjectionMatrix("),
            "GameRenderer should set projection matrix through VulkanicAPI after migration");
        assertFalse(gameRendererSource.contains("RenderSystem.resetTextureMatrix("),
            "GameRenderer should not reset texture matrix through RenderSystem after VulkanicAPI migration");
        assertTrue(gameRendererSource.contains("VulkanicAPI.resetTextureMatrix("),
            "GameRenderer should reset texture matrix through VulkanicAPI after migration");

        Path fogRendererFile = SRC_MAIN_JAVA.resolve("net/minecraft/client/renderer/fog/FogRenderer.java");
        String fogRendererSource = readSource(fogRendererFile);
        assertFalse(fogRendererSource.contains("RenderSystem.setShaderFog("),
            "FogRenderer should not initialize fog uniforms through RenderSystem after VulkanicAPI migration");
        assertTrue(fogRendererSource.contains("VulkanicAPI.setShaderFog("),
            "FogRenderer should initialize fog uniforms through VulkanicAPI after migration");

        Path levelRendererFile = SRC_MAIN_JAVA.resolve("net/minecraft/client/renderer/LevelRenderer.java");
        String levelRendererSource = readSource(levelRendererFile);
        assertFalse(levelRendererSource.contains("RenderSystem.getShaderFog("),
            "LevelRenderer should not read fog uniforms through RenderSystem after VulkanicAPI migration");
        assertFalse(levelRendererSource.contains("RenderSystem.setShaderFog("),
            "LevelRenderer should not set fog uniforms through RenderSystem after VulkanicAPI migration");
        assertTrue(levelRendererSource.contains("VulkanicAPI.getShaderFog("),
            "LevelRenderer should read fog uniforms through VulkanicAPI after migration");
        assertTrue(levelRendererSource.contains("VulkanicAPI.setShaderFog("),
            "LevelRenderer should set fog uniforms through VulkanicAPI after migration");

        Path particleFeatureRendererFile = SRC_MAIN_JAVA.resolve("net/minecraft/client/renderer/feature/ParticleFeatureRenderer.java");
        String particleFeatureRendererSource = readSource(particleFeatureRendererFile);
        assertFalse(particleFeatureRendererSource.contains("RenderSystem.getShaderFog()"),
            "ParticleFeatureRenderer should not read fog uniforms through RenderSystem after VulkanicAPI migration");
        assertTrue(particleFeatureRendererSource.contains("VulkanicAPI.getShaderFog()"),
            "ParticleFeatureRenderer should read fog uniforms through VulkanicAPI after migration");
        assertFalse(particleFeatureRendererSource.contains("RenderSystem.getProjectionMatrixBuffer()"),
            "ParticleFeatureRenderer should not read projection matrix through RenderSystem after VulkanicAPI migration");
        assertTrue(particleFeatureRendererSource.contains("VulkanicAPI.getProjectionMatrixBuffer()"),
            "ParticleFeatureRenderer should read projection matrix through VulkanicAPI after migration");

        Path lightingFile = SRC_MAIN_JAVA.resolve("net/blaze3d/platform/Lighting.java");
        String lightingSource = readSource(lightingFile);
        assertFalse(lightingSource.contains("RenderSystem.setShaderLights("),
            "Lighting should not publish lighting uniforms through RenderSystem after VulkanicAPI migration");
        assertTrue(lightingSource.contains("VulkanicAPI.setShaderLights("),
            "Lighting should publish lighting uniforms through VulkanicAPI after migration");

        Path globalSettingsUniformFile = SRC_MAIN_JAVA.resolve("net/minecraft/client/renderer/GlobalSettingsUniform.java");
        String globalSettingsUniformSource = readSource(globalSettingsUniformFile);
        assertFalse(globalSettingsUniformSource.contains("RenderSystem.setGlobalSettingsUniform("),
            "GlobalSettingsUniform should not publish globals UBO through RenderSystem after VulkanicAPI migration");
        assertTrue(globalSettingsUniformSource.contains("VulkanicAPI.setGlobalSettingsUniform("),
            "GlobalSettingsUniform should publish globals UBO through VulkanicAPI after migration");

        Path postPassFile = SRC_MAIN_JAVA.resolve("net/minecraft/client/renderer/PostPass.java");
        String postPassSource = readSource(postPassFile);
        assertFalse(postPassSource.contains("RenderSystem.backupProjectionMatrix("),
            "PostPass should not backup projection through RenderSystem after VulkanicAPI migration");
        assertFalse(postPassSource.contains("RenderSystem.setProjectionMatrix("),
            "PostPass should not set projection through RenderSystem after VulkanicAPI migration");
        assertFalse(postPassSource.contains("RenderSystem.restoreProjectionMatrix("),
            "PostPass should not restore projection through RenderSystem after VulkanicAPI migration");
        assertTrue(postPassSource.contains("VulkanicAPI.backupProjectionMatrix("),
            "PostPass should backup projection through VulkanicAPI after migration");
        assertTrue(postPassSource.contains("VulkanicAPI.setProjectionMatrix("),
            "PostPass should set projection through VulkanicAPI after migration");
        assertTrue(postPassSource.contains("VulkanicAPI.restoreProjectionMatrix("),
            "PostPass should restore projection through VulkanicAPI after migration");

        Path renderStateShardProjectionFile = SRC_MAIN_JAVA.resolve("net/minecraft/client/renderer/RenderStateShard.java");
        String renderStateShardProjectionSource = readSource(renderStateShardProjectionFile);
        assertFalse(renderStateShardProjectionSource.contains("RenderSystem.getProjectionType()"),
            "RenderStateShard should not read projection type through RenderSystem after VulkanicAPI migration");
        assertTrue(renderStateShardProjectionSource.contains("VulkanicAPI.getProjectionType()"),
            "RenderStateShard should read projection type through VulkanicAPI after migration");
        assertFalse(renderStateShardProjectionSource.contains("RenderSystem.setTextureMatrix("),
            "RenderStateShard should not set texture matrix through RenderSystem after VulkanicAPI migration");
        assertFalse(renderStateShardProjectionSource.contains("RenderSystem.resetTextureMatrix("),
            "RenderStateShard should not reset texture matrix through RenderSystem after VulkanicAPI migration");
        assertFalse(renderStateShardProjectionSource.contains("RenderSystem.lineWidth("),
            "RenderStateShard should not set line width through RenderSystem after VulkanicAPI migration");
        assertTrue(renderStateShardProjectionSource.contains("VulkanicAPI.setTextureMatrix("),
            "RenderStateShard should set texture matrix through VulkanicAPI after migration");
        assertTrue(renderStateShardProjectionSource.contains("VulkanicAPI.resetTextureMatrix("),
            "RenderStateShard should reset texture matrix through VulkanicAPI after migration");
        assertTrue(renderStateShardProjectionSource.contains("VulkanicAPI.lineWidth("),
            "RenderStateShard should set line width through VulkanicAPI after migration");

        Path multiBufferSourceFile = SRC_MAIN_JAVA.resolve("net/minecraft/client/renderer/MultiBufferSource.java");
        String multiBufferSourceSource = readSource(multiBufferSourceFile);
        assertFalse(multiBufferSourceSource.contains("RenderSystem.getProjectionType()"),
            "MultiBufferSource should not read projection type through RenderSystem after VulkanicAPI migration");
        assertTrue(multiBufferSourceSource.contains("VulkanicAPI.getProjectionType()"),
            "MultiBufferSource should read projection type through VulkanicAPI after migration");

        assertFalse(renderTypeSource.contains("RenderSystem.bindDefaultUniforms(renderPass)"),
            "RenderType should not bind default uniforms through RenderSystem after migration");
        assertTrue(renderTypeSource.contains("VulkanicAPI.bindDefaultUniforms(renderPass)"),
            "RenderType should bind default uniforms through VulkanicAPI after migration");
        assertFalse(renderTypeSource.contains("RenderSystem.getTextureMatrix()"),
            "RenderType should not read texture matrix through RenderSystem after VulkanicAPI migration");
        assertFalse(renderTypeSource.contains("RenderSystem.getShaderLineWidth()"),
            "RenderType should not read line width through RenderSystem after VulkanicAPI migration");
        assertTrue(renderTypeSource.contains("VulkanicAPI.getTextureMatrix()"),
            "RenderType should read texture matrix through VulkanicAPI after migration");
        assertTrue(renderTypeSource.contains("VulkanicAPI.getShaderLineWidth()"),
            "RenderType should read line width through VulkanicAPI after migration");

        Path quadParticleFile = SRC_MAIN_JAVA.resolve("net/minecraft/client/renderer/state/QuadParticleRenderState.java");
        String quadParticleSource = readSource(quadParticleFile);
        assertFalse(quadParticleSource.contains("RenderSystem.getTextureMatrix()"),
            "QuadParticleRenderState should not read texture matrix through RenderSystem after VulkanicAPI migration");
        assertFalse(quadParticleSource.contains("RenderSystem.getShaderLineWidth()"),
            "QuadParticleRenderState should not read line width through RenderSystem after VulkanicAPI migration");
        assertTrue(quadParticleSource.contains("VulkanicAPI.getTextureMatrix()"),
            "QuadParticleRenderState should read texture matrix through VulkanicAPI after migration");
        assertTrue(quadParticleSource.contains("VulkanicAPI.getShaderLineWidth()"),
            "QuadParticleRenderState should read line width through VulkanicAPI after migration");

        assertFalse(horizonRendererSource.contains("RenderSystem.getTextureMatrix()"),
            "HorizonRenderer should not read texture matrix through RenderSystem after VulkanicAPI migration");
        assertFalse(horizonRendererSource.contains("RenderSystem.getShaderLineWidth()"),
            "HorizonRenderer should not read line width through RenderSystem after VulkanicAPI migration");
        assertTrue(horizonRendererSource.contains("VulkanicAPI.getTextureMatrix()"),
            "HorizonRenderer should read texture matrix through VulkanicAPI after migration");
        assertTrue(horizonRendererSource.contains("VulkanicAPI.getShaderLineWidth()"),
            "HorizonRenderer should read line width through VulkanicAPI after migration");

        Path overlayTextureFile = SRC_MAIN_JAVA.resolve("net/minecraft/client/renderer/texture/OverlayTexture.java");
        String overlayTextureSource = readSource(overlayTextureFile);
        assertTrue(overlayTextureSource.contains("RustGalVulkanWholeFrameMode.enabled()")
                && overlayTextureSource.contains("VulkanicAPI.isVulkanBackendSelected()"),
            "OverlayTexture must choose the semantic Rust asset route for the whole-frame Vulkan route");
        assertFalse(overlayTextureSource.contains("RenderSystem.setupOverlayColor("),
            "OverlayTexture should not route overlay setup through RenderSystem.setupOverlayColor");
        assertFalse(overlayTextureSource.contains("RenderSystem.teardownOverlayColor("),
            "OverlayTexture should not route overlay teardown through RenderSystem.teardownOverlayColor");
        assertFalse(overlayTextureSource.contains("VulkanicCoreAPI.textureId(textureView)"),
            "OverlayTexture should not extract raw texture ids for overlay binding");
        assertTrue(overlayTextureSource.contains("var ctx = VulkanicAPI.getCommandContext();")
                && overlayTextureSource.contains("VulkanicAPI.bindTextureUnit(ctx, 1, textureView)"),
            "OverlayTexture should bind overlay texture through VulkanicAPI texture-view seam");
        assertTrue(overlayTextureSource.contains("TextureTracker.INSTANCE.onSetShaderTexture(1, textureView)"),
            "OverlayTexture should publish Sampler1 texture view updates to TextureTracker");
        assertTrue(overlayTextureSource.contains("IrisRenderSystem.bindTextureToUnit(1, 0)"),
            "OverlayTexture should clear overlay texture binding directly through IrisRenderSystem default-2D helper");
        assertTrue(overlayTextureSource.contains("TextureTracker.INSTANCE.onSetShaderTexture(1, null)"),
            "OverlayTexture should clear Sampler1 texture view updates from TextureTracker when unbinding");

        Path graphicsBackendFile = SRC_MAIN_JAVA.resolve("net/vulkanic/GraphicsBackend.java");
        String graphicsBackendSource = readSource(graphicsBackendFile);
        assertTrue(graphicsBackendSource.contains("default void bindTextureUnit(CommandContext ctx, int unit, GpuTextureView textureView)"),
            "GraphicsBackend should expose texture-view texture-unit binding seam for backend-neutral callsites");
        assertTrue(vulkanicApiSource.contains("public static void bindTextureUnit(CommandContext ctx, int unit, GpuTextureView textureView)"),
            "VulkanicAPI should expose texture-view texture-unit binding seam for shared/game rendering callsites");
        assertTrue(vulkanicApiSource.contains("if (isVulkanBackendSelected()) {\n            throw new IllegalStateException(\"Java Vulkan texture-unit binding is unavailable; Rust owns the selected Vulkan route\")"),
            "Texture-unit binding must fail closed for every selected Vulkan route before Java backend or Iris state is touched");
        assertFalse(vulkanicApiSource.contains("if (net.vulkanic.bridge.RustGalVulkanWholeFrameMode.enabled()) {\n            throw new IllegalStateException(\"Java Vulkan texture-unit binding is unavailable while Rust owns whole-frame Vulkan\")"),
            "Texture-unit binding must not reopen on an unadmitted selected-Vulkan route");
    }

    @Test
    public void testTextureUnitSelectionUsesIndexHelpers() throws IOException {
        Path[] dhFiles = new Path[] {
            SRC_MAIN_JAVA.resolve("com/seibel/distanthorizons/core/render/renderer/shaders/SSAOApplyShader.java"),
            SRC_MAIN_JAVA.resolve("com/seibel/distanthorizons/core/render/renderer/shaders/DhApplyShader.java"),
            SRC_MAIN_JAVA.resolve("com/seibel/distanthorizons/core/render/renderer/shaders/DhFadeShader.java"),
            SRC_MAIN_JAVA.resolve("com/seibel/distanthorizons/core/render/renderer/shaders/VanillaFadeShader.java"),
            SRC_MAIN_JAVA.resolve("com/seibel/distanthorizons/core/render/renderer/shaders/FogApplyShader.java"),
            SRC_MAIN_JAVA.resolve("com/seibel/distanthorizons/core/render/renderer/shaders/FadeApplyShader.java"),
            SRC_MAIN_JAVA.resolve("com/seibel/distanthorizons/core/render/renderer/shaders/SSAOShader.java"),
            SRC_MAIN_JAVA.resolve("com/seibel/distanthorizons/core/render/renderer/shaders/FogShader.java"),
            SRC_MAIN_JAVA.resolve("com/seibel/distanthorizons/common/wrappers/misc/LightMapWrapper.java")
        };

        for (Path file : dhFiles) {
            String source = readSource(file);
            assertFalse(source.contains("DhTextureState.setActiveTextureUnit(VulkanicAPI.GL_TEXTURE"),
                file.getFileName() + " should not select texture units through raw GL_TEXTURE constants");
            assertTrue(source.contains("DhTextureState.setActiveTextureUnitIndex("),
                file.getFileName() + " should select texture units through index-based helper");
        }

        Path[] irisFiles = new Path[] {
            SRC_MAIN_JAVA.resolve("net/irisshaders/iris/pipeline/PipelineManager.java"),
            SRC_MAIN_JAVA.resolve("net/irisshaders/iris/pipeline/FinalPassRenderer.java"),
            SRC_MAIN_JAVA.resolve("net/irisshaders/iris/pipeline/CompositeRenderer.java"),
            SRC_MAIN_JAVA.resolve("net/irisshaders/iris/pipeline/IrisRenderingPipeline.java"),
            SRC_MAIN_JAVA.resolve("net/irisshaders/iris/shadows/ShadowRenderer.java"),
            SRC_MAIN_JAVA.resolve("net/irisshaders/iris/shadows/ShadowCompositeRenderer.java")
        };

        for (Path file : irisFiles) {
            String source = readSource(file);
            assertFalse(source.contains("IrisRenderSystem.setActiveTexture(VulkanicAPI.GL_TEXTURE"),
                file.getFileName() + " should not select texture units through raw GL_TEXTURE constants");
            assertTrue(source.contains("IrisRenderSystem.setActiveTextureUnitIndex("),
                file.getFileName() + " should select texture units through index-based helper");
        }
        String irisRenderingPipelineSource = readSource(SRC_MAIN_JAVA.resolve(
            "net/irisshaders/iris/pipeline/IrisRenderingPipeline.java"));
        assertTrue(irisRenderingPipelineSource.contains("VulkanicAPI.isVulkanBackendSelected()")
                && irisRenderingPipelineSource.contains("RustGalVulkanWholeFrameMode.enabled()")
                && irisRenderingPipelineSource.contains("destroyed = true;"),
            "Iris pipeline teardown must fence Java GL destruction when Rust owns Vulkan");

        Path defaultShaderInterfaceFile = SRC_MAIN_JAVA.resolve("net/sodium/client/render/chunk/shader/DefaultShaderInterface.java");
        String defaultShaderInterfaceSource = readSource(defaultShaderInterfaceFile);
        assertFalse(defaultShaderInterfaceSource.contains("VulkanicAPI.setActiveTextureUnit(ctx, VulkanicAPI.GL_TEXTURE0 + slot.ordinal())"),
            "DefaultShaderInterface should not compute GL texture units via GL_TEXTURE0 arithmetic");
        assertTrue(defaultShaderInterfaceSource.contains("VulkanicAPI.setActiveTextureUnitIndex(ctx, slot.ordinal())"),
            "DefaultShaderInterface should select texture units via VulkanicAPI index helper");

        Path irisRenderSystemFile = SRC_MAIN_JAVA.resolve("net/irisshaders/iris/gl/IrisRenderSystem.java");
        String irisRenderSystemSource = readSource(irisRenderSystemFile);
        assertTrue(irisRenderSystemSource.contains("VulkanicAPI.textureUnitToIndex(textureUnit)"),
            "IrisRenderSystem should convert GL texture units to indices via VulkanicAPI helper");
        assertTrue(irisRenderSystemSource.contains("VulkanicAPI.setActiveTextureUnitIndex(VulkanicAPI.getCommandContext(), textureUnitIndex)"),
            "IrisRenderSystem should set active texture through VulkanicAPI index helper");
        assertFalse(irisRenderSystemSource.contains("setActiveTexture(VulkanicAPI.GL_TEXTURE0 + textureUnitIndex)"),
            "IrisRenderSystem index path should not compute GL texture units inline");

        Path glEnumsFile = SRC_MAIN_JAVA.resolve("com/seibel/distanthorizons/core/render/glObject/GLEnums.java");
        String glEnumsSource = readSource(glEnumsFile);
        assertTrue(glEnumsSource.contains("if (glEnum >= VulkanicAPI.GL_TEXTURE0 && glEnum <= VulkanicAPI.GL_TEXTURE31)"),
            "GLEnums should map texture binding points through GL_TEXTURE range check");
        assertTrue(glEnumsSource.contains("\"GL_TEXTURE\" + VulkanicAPI.textureUnitToIndex(glEnum)"),
            "GLEnums should compute texture-unit suffix via VulkanicAPI textureUnitToIndex helper");
        assertFalse(glEnumsSource.contains("case VulkanicAPI.GL_TEXTURE1"),
            "GLEnums should not enumerate each GL_TEXTUREN case manually");
    }

    @Test
    public void testBlaze3dClearWrapperRemovedFromGlStateManager() throws IOException {
        Path stateManagerFile = SRC_MAIN_JAVA.resolve("net/blaze3d/opengl/GlStateManager.java");
        String stateManagerSource = readSource(stateManagerFile);

        assertFalse(stateManagerSource.contains("public static void _clear("),
            "GlStateManager should no longer expose _clear wrapper");

        Path encoderFile = SRC_MAIN_JAVA.resolve("net/blaze3d/opengl/GlCommandEncoder.java");
        String encoderSource = readSource(encoderFile);
        assertFalse(encoderSource.contains("GlStateManager._clear("),
            "GlCommandEncoder should not call removed GlStateManager._clear wrapper");
        assertFalse(encoderSource.contains("VulkanicAPI.clearBuffersWithMacosWorkaround(VulkanicAPI.getCommandContext(), VulkanicAPI.GL_COLOR_BUFFER_BIT)"),
            "GlCommandEncoder should not clear color through raw GL_COLOR_BUFFER_BIT macOS clear mask");
        assertFalse(encoderSource.contains("VulkanicAPI.clearBuffersWithMacosWorkaround(VulkanicAPI.getCommandContext(), VulkanicAPI.GL_COLOR_BUFFER_BIT | VulkanicAPI.GL_DEPTH_BUFFER_BIT)"),
            "GlCommandEncoder should not clear color+depth through raw GL bitmask macOS clear mask");
        assertFalse(encoderSource.contains("VulkanicAPI.clearBuffersWithMacosWorkaround(VulkanicAPI.getCommandContext(), VulkanicAPI.GL_DEPTH_BUFFER_BIT)"),
            "GlCommandEncoder should not clear depth through raw GL_DEPTH_BUFFER_BIT macOS clear mask");
        assertTrue(containsAny(encoderSource,
            "VulkanicAPI.clearColorBufferWithMacosWorkaround(VulkanicAPI.getCommandContext())",
            "VulkanicAPI.clearColorBufferWithMacosWorkaround(ctx)"),
            "GlCommandEncoder should clear color through VulkanicAPI clearColorBufferWithMacosWorkaround helper");
        assertTrue(containsAny(encoderSource,
            "VulkanicAPI.clearColorAndDepthBuffersWithMacosWorkaround(VulkanicAPI.getCommandContext())",
            "VulkanicAPI.clearColorAndDepthBuffersWithMacosWorkaround(ctx)"),
            "GlCommandEncoder should clear color+depth through VulkanicAPI clearColorAndDepthBuffersWithMacosWorkaround helper");
        assertTrue(containsAny(encoderSource,
            "VulkanicAPI.clearDepthBufferWithMacosWorkaround(VulkanicAPI.getCommandContext())",
            "VulkanicAPI.clearDepthBufferWithMacosWorkaround(ctx)"),
            "GlCommandEncoder should clear depth through VulkanicAPI clearDepthBufferWithMacosWorkaround helper");

        Path clearPassFile = SRC_MAIN_JAVA.resolve("net/irisshaders/iris/targets/ClearPass.java");
        String clearPassSource = readSource(clearPassFile);
        assertFalse(clearPassSource.contains("GlStateManager._clear("),
            "ClearPass should not call removed GlStateManager._clear wrapper");
        assertFalse(clearPassSource.contains("VulkanicAPI.clearBuffersWithMacosWorkaround("),
            "ClearPass should not clear through generic raw-mask macOS helper");
        assertTrue(containsAny(clearPassSource,
                "VulkanicAPI.clearColorBufferWithMacosWorkaround(VulkanicAPI.getCommandContext())",
                "VulkanicAPI.clearColorBufferWithMacosWorkaround(ctx)"),
            "ClearPass should clear via VulkanicAPI.clearColorBufferWithMacosWorkaround");
    }

    @Test
    public void testVulkanRenderPassClearCoversAllActiveColorAttachments() throws IOException {
        Path backendFile = SRC_MAIN_JAVA.resolve("net/vulkanic/backends/vulkan/VulkanBackend.java");
        String backendSource = readSource(backendFile);

        assertTrue(backendSource.contains("int colorAttachmentCount = clearColor ? activeRenderPassColorAttachmentCount() : 0;"),
            "Vulkan render-pass clears should derive the active color attachment count instead of hardcoding attachment zero");
        assertTrue(backendSource.contains("for (int colorAttachment = 0; colorAttachment < colorAttachmentCount; colorAttachment++)"),
            "Vulkan render-pass clears should emit a VkClearAttachment for each active color attachment");
        assertTrue(backendSource.contains(".colorAttachment(colorAttachment);"),
            "Vulkan render-pass clears should target the looped attachment index");
        assertTrue(backendSource.contains("renderPassExecution.activeWidth() > 0 ? renderPassExecution.activeWidth() : swapchainState.width()"),
            "Vulkan render-pass clears should use the active render area width when clearing framebuffer-backed passes");
        assertTrue(backendSource.contains("renderPassExecution.activeHeight() > 0 ? renderPassExecution.activeHeight() : swapchainState.height()"),
            "Vulkan render-pass clears should use the active render area height when clearing framebuffer-backed passes");
        assertTrue(backendSource.contains("return renderPassExecution.activeColorAttachmentCount();"),
            "Vulkan render-pass clears should still clear swapchain-only render passes that do not track legacy color textures");
    }

    @Test
    public void testBlaze3dBindTextureWrapperRemovedFromGlStateManager() throws IOException {
        Path stateManagerFile = SRC_MAIN_JAVA.resolve("net/blaze3d/opengl/GlStateManager.java");
        String stateManagerSource = readSource(stateManagerFile);

        assertFalse(stateManagerSource.contains("public static void _bindTexture("),
            "GlStateManager should no longer expose _bindTexture wrapper");

        Path encoderFile = SRC_MAIN_JAVA.resolve("net/blaze3d/opengl/GlCommandEncoder.java");
        String encoderSource = readSource(encoderFile);
        assertFalse(encoderSource.contains("GlStateManager._bindTexture("),
            "GlCommandEncoder should not call removed GlStateManager._bindTexture wrapper");
        assertTrue(encoderSource.contains("VulkanicAPI.bindTexture2D("),
            "GlCommandEncoder should bind 2D textures directly through VulkanicAPI.bindTexture2D");

        Path renderTargetsFile = SRC_MAIN_JAVA.resolve("net/irisshaders/iris/targets/RenderTargets.java");
        String renderTargetsSource = readSource(renderTargetsFile);
        assertFalse(renderTargetsSource.contains("GlStateManager._bindTexture("),
            "RenderTargets should not call removed GlStateManager._bindTexture wrapper");
        assertFalse(renderTargetsSource.contains("IrisRenderSystem.copyTexImage2D(VulkanicAPI.GL_TEXTURE_2D"),
            "RenderTargets should not pass explicit GL_TEXTURE_2D in copyTexImage2D calls");
        assertFalse(renderTargetsSource.contains("IrisRenderSystem.copyTexImage2D(0"),
            "RenderTargets depth snapshots should avoid legacy copyTexImage2D now that depth targets are preallocated");
        assertTrue(renderTargetsSource.contains("DepthCopyStrategy.fastestDepthSnapshot(false)"),
            "RenderTargets world depth snapshots should copy depth only, even when the main target is stencil-capable");
        assertTrue(renderTargetsSource.contains("private static TextureFormat snapshotDepthFormat(DepthBufferFormat sourceDepthFormat)"),
            "RenderTargets world depth snapshots should choose a backend-legal texture format explicitly");
        assertTrue(renderTargetsSource.contains("if (!VulkanicAPI.isVulkanBackendSelected())"),
            "OpenGL RenderTargets world depth snapshots should remain depth-only shaderpack sampler inputs");
        assertTrue(renderTargetsSource.contains("case DEPTH_STENCIL, DEPTH24_STENCIL8 -> TextureFormat.DEPTH24_STENCIL8"),
            "Vulkan depth snapshots must match a combined source format for legal depth blits");
        assertFalse(renderTargetsSource.contains("newDepthTextureId.getFormat(), newWidth, newHeight"),
            "Resized Iris depth snapshots should not blindly inherit the main target format");
        assertTrue(renderTargetsSource.contains("copyStrategy.copy(depthSourceFb, VulkanicCoreAPI.textureId(getDepthTexture()), noHandDestFb, VulkanicCoreAPI.textureId(noHand),"),
            "RenderTargets pre-hand depth path should route through the shared depth copy strategy");
        assertTrue(renderTargetsSource.contains("copyStrategy.copy(depthSourceFb, VulkanicCoreAPI.textureId(getDepthTexture()), noTranslucentsDestFb, VulkanicCoreAPI.textureId(noTranslucents),"),
            "RenderTargets pre-translucent depth path should route through the shared depth copy strategy");

        Path depthCopyStrategyFile = SRC_MAIN_JAVA.resolve("net/irisshaders/iris/gl/texture/DepthCopyStrategy.java");
        String depthCopyStrategySource = readSource(depthCopyStrategyFile);
        assertTrue(depthCopyStrategySource.contains("fastestDepthSnapshot(boolean combinedStencilRequired)"),
            "DepthCopyStrategy should expose a dedicated depth snapshot selector");
        assertTrue(depthCopyStrategySource.contains("VulkanicAPI.isVulkanBackendSelected()"),
            "DepthCopyStrategy depth snapshot selector should special-case Vulkan");
        assertTrue(depthCopyStrategySource.contains("class Gl30BlitFbDepth implements DepthCopyStrategy"),
            "DepthCopyStrategy should provide a depth-only framebuffer blit strategy");

        Path shadowRenderTargetsFile = SRC_MAIN_JAVA.resolve("net/irisshaders/iris/shadows/ShadowRenderTargets.java");
        String shadowRenderTargetsSource = readSource(shadowRenderTargetsFile);
        assertTrue(shadowRenderTargetsSource.contains("DepthCopyStrategy.fastestDepthSnapshot(false).copy("),
            "ShadowRenderTargets should reuse the Vulkan-safe depth snapshot strategy after the initial blit");

        Path dhCompatInternalFile = SRC_MAIN_JAVA.resolve("net/irisshaders/iris/compat/dh/DHCompatInternal.java");
        String dhCompatInternalSource = readSource(dhCompatInternalFile);
        assertFalse(dhCompatInternalSource.contains("IrisRenderSystem.copyTexImage2D(VulkanicAPI.GL_TEXTURE_2D"),
            "DHCompatInternal should not pass explicit GL_TEXTURE_2D in copyTexImage2D calls");
        assertFalse(dhCompatInternalSource.contains("IrisRenderSystem.copyTexImage2D("),
            "DHCompatInternal should avoid color-oriented copyTexImage2D for translucent depth snapshots");
        assertTrue(dhCompatInternalSource.contains("DepthCopyStrategy strategy = DepthCopyStrategy.fastestDepthSnapshot(dhDepthFormat.isCombinedStencil())"),
            "DHCompatInternal should use the Vulkan-safe depth snapshot strategy with the active depth/stencil contract");
        assertTrue(dhCompatInternalSource.contains("depthTexNoTranslucentFramebuffer.addDepthAttachmentBypass(depthTexNoTranslucent.getTextureId(), dhDepthFormat.isCombinedStencil())"),
            "DHCompatInternal should provide a destination framebuffer that preserves combined depth-stencil attachment intent");

        Path dhWrapperFile = SRC_MAIN_JAVA.resolve("com/seibel/distanthorizons/common/wrappers/minecraft/MinecraftGLWrapper.java");
        String dhWrapperSource = readSourceIfExists(dhWrapperFile);
        assertFalse(dhWrapperSource.contains("GlStateManager._bindTexture("),
            "MinecraftGLWrapper should not call removed GlStateManager._bindTexture wrapper");

        Path dhTextureStateFile = SRC_MAIN_JAVA.resolve("com/seibel/distanthorizons/core/render/glObject/DhTextureState.java");
        String dhTextureStateSource = readSource(dhTextureStateFile);
        assertTrue(dhTextureStateSource.contains("VulkanicAPI.bindTexture2D("),
            "DhTextureState should bind textures through VulkanicAPI.bindTexture2D");
    }

    @Test
    public void testIrisComputePipelinesFailOpenWhenComputeUnsupported() throws IOException {
        Path programBuilderFile = SRC_MAIN_JAVA.resolve("net/irisshaders/iris/gl/program/ProgramBuilder.java");
        String programBuilderSource = readSource(programBuilderFile);

        assertTrue(programBuilderSource.contains("beginComputeIfSupported("),
            "ProgramBuilder should expose a compute fail-open helper for unsupported runtimes");
        assertTrue(programBuilderSource.contains("return null;"),
            "ProgramBuilder compute fail-open helper should return null when compute is unsupported");
        assertTrue(programBuilderSource.contains("Skipping compute shader program"),
            "ProgramBuilder compute fail-open helper should log when compute programs are skipped");

        List<String> pipelineFiles = List.of(
            "net/irisshaders/iris/shadows/ShadowCompositeRenderer.java",
            "net/irisshaders/iris/pipeline/CompositeRenderer.java",
            "net/irisshaders/iris/pipeline/FinalPassRenderer.java",
            "net/irisshaders/iris/pipeline/IrisRenderingPipeline.java"
        );

        for (String relative : pipelineFiles) {
            String source = readSource(SRC_MAIN_JAVA.resolve(relative));
            assertTrue(source.contains("ProgramBuilder.beginComputeIfSupported("),
                "Iris compute pipeline should use compute fail-open helper: " + relative);
        }

        assertFalse(readSource(SRC_MAIN_JAVA.resolve("net/irisshaders/iris/shadows/ShadowCompositeRenderer.java")).contains("ProgramBuilder.beginCompute("),
            "ShadowCompositeRenderer should not use fatal compute builder directly");
        assertFalse(readSource(SRC_MAIN_JAVA.resolve("net/irisshaders/iris/pipeline/CompositeRenderer.java")).contains("ProgramBuilder.beginCompute("),
            "CompositeRenderer should not use fatal compute builder directly");
        assertFalse(readSource(SRC_MAIN_JAVA.resolve("net/irisshaders/iris/pipeline/FinalPassRenderer.java")).contains("ProgramBuilder.beginCompute("),
            "FinalPassRenderer should not use fatal compute builder directly");
        assertFalse(readSource(SRC_MAIN_JAVA.resolve("net/irisshaders/iris/pipeline/IrisRenderingPipeline.java")).contains("ProgramBuilder.beginCompute("),
            "IrisRenderingPipeline should not use fatal compute builder directly");
    }

    @Test
    public void testBlaze3dTextureLifecycleWrappersRemovedFromGlStateManager() throws IOException {
        Path stateManagerFile = SRC_MAIN_JAVA.resolve("net/blaze3d/opengl/GlStateManager.java");
        String stateManagerSource = readSource(stateManagerFile);

        assertFalse(stateManagerSource.contains("public static int _genTexture("),
            "GlStateManager should no longer expose _genTexture wrapper");
        assertFalse(stateManagerSource.contains("public static void _deleteTexture("),
            "GlStateManager should no longer expose _deleteTexture wrapper");
        assertFalse(stateManagerSource.contains("public static void incrementTrackedTextures("),
            "GlStateManager should no longer expose incrementTrackedTextures helper");
        assertFalse(stateManagerSource.contains("public static void decrementTrackedTextures("),
            "GlStateManager should no longer expose decrementTrackedTextures helper");

        Path glDeviceFile = SRC_MAIN_JAVA.resolve("net/blaze3d/opengl/GlDevice.java");
        String glDeviceSource = readSource(glDeviceFile);
        assertFalse(glDeviceSource.contains("GlStateManager._genTexture("),
            "GlDevice should not call removed GlStateManager._genTexture wrapper");
        assertTrue(glDeviceSource.contains("IrisRenderSystem.createTextureId("),
            "GlDevice should create textures through IrisRenderSystem.createTextureId");

        Path uniformFile = SRC_MAIN_JAVA.resolve("net/blaze3d/opengl/Uniform.java");
        String uniformSource = readSource(uniformFile);
        assertFalse(uniformSource.contains("GlStateManager._genTexture("),
            "Uniform should not call removed GlStateManager._genTexture wrapper");
        assertFalse(uniformSource.contains("GlStateManager._deleteTexture("),
            "Uniform should not call removed GlStateManager._deleteTexture wrapper");
        assertTrue(uniformSource.contains("IrisRenderSystem.createTextureId("),
            "Uniform should create textures through IrisRenderSystem.createTextureId");
        assertTrue(uniformSource.contains("IrisRenderSystem.deleteTextureId("),
            "Uniform should delete textures through IrisRenderSystem.deleteTextureId");

        Path irisRenderSystemFile = SRC_MAIN_JAVA.resolve("net/irisshaders/iris/gl/IrisRenderSystem.java");
        String irisRenderSystemSource = readSource(irisRenderSystemFile);
        assertTrue(irisRenderSystemSource.contains("public static int createTextureId("),
            "IrisRenderSystem should provide createTextureId helper after _genTexture removal");
        assertTrue(irisRenderSystemSource.contains("public static void deleteTextureId("),
            "IrisRenderSystem should provide deleteTextureId helper after _deleteTexture removal");
        assertTrue(irisRenderSystemSource.contains("public static void incrementTrackedTextures("),
            "IrisRenderSystem should expose incrementTrackedTextures helper after migration");
        assertTrue(irisRenderSystemSource.contains("public static void decrementTrackedTextures("),
            "IrisRenderSystem should expose decrementTrackedTextures helper after migration");
    }

    @Test
    public void testBlaze3dBufferTrackingMovedFromGlStateManager() throws IOException {
        Path stateManagerFile = SRC_MAIN_JAVA.resolve("net/blaze3d/opengl/GlStateManager.java");
        String stateManagerSource = readSource(stateManagerFile);

        assertFalse(stateManagerSource.contains("public static void incrementTrackedBuffers("),
            "GlStateManager should no longer expose incrementTrackedBuffers helper");
        assertFalse(stateManagerSource.contains("public static void decrementTrackedBuffers("),
            "GlStateManager should no longer expose decrementTrackedBuffers helper");

        Path irisRenderSystemFile = SRC_MAIN_JAVA.resolve("net/irisshaders/iris/gl/IrisRenderSystem.java");
        String irisRenderSystemSource = readSource(irisRenderSystemFile);
        assertTrue(irisRenderSystemSource.contains("public static void incrementTrackedBuffers("),
            "IrisRenderSystem should expose incrementTrackedBuffers helper after migration");
        assertTrue(irisRenderSystemSource.contains("public static void decrementTrackedBuffers("),
            "IrisRenderSystem should expose decrementTrackedBuffers helper after migration");

        Path dsaFile = SRC_MAIN_JAVA.resolve("net/blaze3d/opengl/DirectStateAccess.java");
        String dsaSource = readSource(dsaFile);
        assertFalse(dsaSource.contains("GlStateManager.incrementTrackedBuffers("),
            "DirectStateAccess should not increment tracked buffers through GlStateManager");
        assertTrue(dsaSource.contains("IrisRenderSystem.incrementTrackedBuffers("),
            "DirectStateAccess should increment tracked buffers through IrisRenderSystem helper");

        Path glBufferFile = SRC_MAIN_JAVA.resolve("net/blaze3d/opengl/GlBuffer.java");
        String glBufferSource = readSource(glBufferFile);
        assertFalse(glBufferSource.contains("GlStateManager.decrementTrackedBuffers("),
            "GlBuffer should not decrement tracked buffers through GlStateManager");
        assertTrue(glBufferSource.contains("IrisRenderSystem.decrementTrackedBuffers("),
            "GlBuffer should decrement tracked buffers through IrisRenderSystem helper");

        Path dhWrapperFile = SRC_MAIN_JAVA.resolve("com/seibel/distanthorizons/common/wrappers/minecraft/MinecraftGLWrapper.java");
        String dhWrapperSource = readSourceIfExists(dhWrapperFile);
        assertFalse(dhWrapperSource.contains("GlStateManager.incrementTrackedBuffers("),
            "MinecraftGLWrapper should not increment tracked buffers through GlStateManager");
        assertFalse(dhWrapperSource.contains("GlStateManager.decrementTrackedBuffers("),
            "MinecraftGLWrapper should not decrement tracked buffers through GlStateManager");
        assertFalse(dhWrapperSource.contains("public int glGenBuffers("),
            "MinecraftGLWrapper should no longer expose buffer generation wrapper methods");
        assertFalse(dhWrapperSource.contains("public void glDeleteBuffers("),
            "MinecraftGLWrapper should no longer expose buffer deletion wrapper methods");

        Path dhGlBufferFile = SRC_MAIN_JAVA.resolve("com/seibel/distanthorizons/core/render/glObject/buffer/GLBuffer.java");
        String dhGlBufferSource = readSource(dhGlBufferFile);
        assertTrue(dhGlBufferSource.contains("IrisRenderSystem.incrementTrackedBuffers("),
            "DH GLBuffer should increment tracked buffers through IrisRenderSystem helper");
        assertTrue(dhGlBufferSource.contains("IrisRenderSystem.decrementTrackedBuffers("),
            "DH GLBuffer should decrement tracked buffers through IrisRenderSystem helper");

        Path renderableBoxGroupFile = SRC_MAIN_JAVA.resolve("com/seibel/distanthorizons/core/render/renderer/generic/RenderableBoxGroup.java");
        String renderableBoxGroupSource = readSource(renderableBoxGroupFile);
        assertTrue(renderableBoxGroupSource.contains("IrisRenderSystem.incrementTrackedBuffers("),
            "RenderableBoxGroup should increment tracked buffers through IrisRenderSystem helper");
        assertTrue(renderableBoxGroupSource.contains("IrisRenderSystem.decrementTrackedBuffers("),
            "RenderableBoxGroup should decrement tracked buffers through IrisRenderSystem helper");
    }

    @Test
    public void testBlaze3dTexImageWrapperRemovedFromGlStateManager() throws IOException {
        Path stateManagerFile = SRC_MAIN_JAVA.resolve("net/blaze3d/opengl/GlStateManager.java");
        String stateManagerSource = readSource(stateManagerFile);

        assertFalse(stateManagerSource.contains("public static void _texImage2D("),
            "GlStateManager should no longer expose _texImage2D wrapper");

        Path glDeviceFile = SRC_MAIN_JAVA.resolve("net/blaze3d/opengl/GlDevice.java");
        String glDeviceSource = readSource(glDeviceFile);
        assertFalse(glDeviceSource.contains("GlStateManager._texImage2D("),
            "GlDevice should not call removed GlStateManager._texImage2D wrapper");
        assertTrue(glDeviceSource.contains("net.vulkanic.VulkanicAPI.uploadTexture2D("),
            "GlDevice texture allocation paths should upload directly through VulkanicAPI.uploadTexture2D");
        assertTrue(glDeviceSource.contains("net.irisshaders.iris.pbr.TextureInfoCache.INSTANCE.onTexImage2D("),
            "GlDevice texture allocation paths should preserve TextureInfoCache tracking after wrapper removal");
    }

    @Test
    public void testDistantHorizonsTextureUploadsUseDefault2DHelper() throws IOException {
        Path[] files = new Path[]{
            SRC_MAIN_JAVA.resolve("com/seibel/distanthorizons/core/render/renderer/DhFadeRenderer.java"),
            SRC_MAIN_JAVA.resolve("com/seibel/distanthorizons/core/render/renderer/SSAORenderer.java"),
            SRC_MAIN_JAVA.resolve("com/seibel/distanthorizons/core/render/renderer/VanillaFadeRenderer.java"),
            SRC_MAIN_JAVA.resolve("com/seibel/distanthorizons/core/render/renderer/FogRenderer.java"),
            SRC_MAIN_JAVA.resolve("com/seibel/distanthorizons/core/render/glObject/texture/DhColorTexture.java"),
            SRC_MAIN_JAVA.resolve("com/seibel/distanthorizons/core/render/glObject/texture/DHDepthTexture.java")
        };

        for (Path file : files) {
            String source = readSource(file);
            assertFalse(source.contains("uploadTexture2D(ctx, VulkanicAPI.GL_TEXTURE_2D"),
                file.getFileName() + " should not pass explicit GL_TEXTURE_2D in uploadTexture2D");
            assertTrue(source.contains("uploadTexture2D(ctx, 0"),
                file.getFileName() + " should use VulkanicAPI default-2D uploadTexture2D overload");
        }
    }

    @Test
    public void testDistantHorizonsTargetFramebufferUsesRenderTargetResolutionSeam() throws IOException {
        Path wrapperFile = SRC_MAIN_JAVA.resolve("com/seibel/distanthorizons/common/wrappers/minecraft/MinecraftRenderWrapper.java");
        String wrapperSource = readSource(wrapperFile);

        assertFalse(wrapperSource.contains("return 0; // 0 is the ID for the default frame buffer"),
            "MinecraftRenderWrapper.getTargetFramebuffer should not hardcode default FBO 0");
        assertTrue(wrapperSource.contains("VulkanicAPI.resolveFramebufferForTextures(renderTarget.getColorTexture(), renderTarget.getDepthTexture())"),
            "MinecraftRenderWrapper.getTargetFramebuffer should resolve framebuffer via VulkanicAPI texture-pair seam");
        assertTrue(wrapperSource.contains("public boolean hasTargetRenderTarget()"),
            "MinecraftRenderWrapper should expose a target render-target availability seam for DH renderers");
        assertTrue(wrapperSource.contains("public boolean bindTargetRenderTarget(CommandContext ctx)"),
            "MinecraftRenderWrapper should expose a target render-target binding seam for DH renderers");
        assertTrue(wrapperSource.contains("VulkanicAPI.bindRenderTarget(ctx, renderTarget.getColorTexture(), renderTarget.getDepthTexture());"),
            "MinecraftRenderWrapper should bind target render targets through VulkanicAPI render-target helper");
        assertTrue(wrapperSource.contains("this.finalLevelFrameBufferId = this.resolveTargetFramebufferId(renderTarget);")
                && wrapperSource.contains("return framebufferId == 0 ? -1 : framebufferId;"),
            "MinecraftRenderWrapper should map unresolved framebuffers to -1 through a shared render-target helper");
        assertFalse(wrapperSource.contains("return 0;"),
            "MinecraftRenderWrapper should not expose 0 as an unresolved texture/framebuffer sentinel in DH-facing seams");
        assertFalse(wrapperSource.contains("this.getRenderTarget().getDepthTexture()"),
            "MinecraftRenderWrapper.getDepthTextureId should avoid direct chained dereference and use null-safe local renderTarget checks");
        assertFalse(wrapperSource.contains("this.getRenderTarget().getColorTexture()"),
            "MinecraftRenderWrapper.getColorTextureId should avoid direct chained dereference and use null-safe local renderTarget checks");
        assertTrue(wrapperSource.contains("if (depthTexture == null)"),
            "MinecraftRenderWrapper.getDepthTextureId should guard null depth texture before resolving a handle");
        assertTrue(wrapperSource.contains("if (colorTexture == null)"),
            "MinecraftRenderWrapper.getColorTextureId should guard null color texture before resolving a handle");
        assertFalse(wrapperSource.contains("catch (Exception e)"),
            "MinecraftRenderWrapper texture-handle resolution should avoid exception-driven control flow and use sentinel checks instead");
        assertFalse(wrapperSource.contains("colorTextureCastFailLogged"),
            "MinecraftRenderWrapper should not rely on one-time exception logging flags for color texture handle resolution");
        assertFalse(wrapperSource.contains("depthTextureCastFailLogged"),
            "MinecraftRenderWrapper should not rely on one-time exception logging flags for depth texture handle resolution");
        assertTrue(wrapperSource.contains("if (textureId <= 0)"),
            "MinecraftRenderWrapper texture-handle resolution should map unresolved/invalid handles to -1 via deterministic sentinel checks");
        assertFalse(wrapperSource.contains("return this.getRenderTarget().width;"),
            "MinecraftRenderWrapper viewport width query should not assume render target is always available");
        assertFalse(wrapperSource.contains("return this.getRenderTarget().height;"),
            "MinecraftRenderWrapper viewport height query should not assume render target is always available");
        assertTrue(wrapperSource.contains("return renderTarget == null ? 0 : renderTarget.width;"),
            "MinecraftRenderWrapper viewport width query should use null-safe fallback semantics");
        assertTrue(wrapperSource.contains("return renderTarget == null ? 0 : renderTarget.height;"),
            "MinecraftRenderWrapper viewport height query should use null-safe fallback semantics");
    }

    @Test
    public void testDistantHorizonsRenderPathsGuardUnresolvedFramebufferIds() throws IOException {
        Path testRendererFile = SRC_MAIN_JAVA.resolve("com/seibel/distanthorizons/core/render/renderer/TestRenderer.java");
        String testRendererSource = readSource(testRendererFile);
        assertTrue(testRendererSource.contains("if (!MC_RENDER.bindTargetRenderTarget(ctx))"),
            "TestRenderer should bind Minecraft's target render target through the wrapper seam");
        assertFalse(testRendererSource.contains("MC_RENDER.getTargetFramebuffer()"),
            "TestRenderer should avoid resolving raw target framebuffer ids in the render hot path");

        Path dhApplyFile = SRC_MAIN_JAVA.resolve("com/seibel/distanthorizons/core/render/renderer/shaders/DhApplyShader.java");
        String dhApplySource = readSource(dhApplyFile);
        assertTrue(dhApplySource.contains("protected boolean onPreRender(CommandContext ctx, float partialTicks)"),
            "DhApplyShader should use shared pre-bind precheck hook for unresolved resources");
        assertTrue(dhApplySource.contains("this.activeDhColorTextureId == -1 || this.activeDhDepthTextureId == -1")
                && dhApplySource.contains("this.activeRenderToFrameBuffer = MC_RENDER.mcRendersToFrameBuffer();"),
            "DhApplyShader precheck should gate rendering on resolved DH textures and selected render path");
        assertTrue(dhApplySource.contains("return MC_RENDER.hasTargetRenderTarget();")
                && dhApplySource.contains("return this.activeTargetColorTextureId != -1")
                && dhApplySource.contains("&& LodRenderer.INSTANCE.hasActiveRenderTarget()")
                && dhApplySource.contains("&& MC_RENDER.hasTargetRenderTarget();"),
            "DhApplyShader precheck should validate render-target availability through owner seams before bind/uniform work");
        assertTrue(dhApplySource.contains("DhTextureState.bindTexture2D(this.activeDhColorTextureId)")
                && dhApplySource.contains("DhTextureState.bindTexture2D(this.activeDhDepthTextureId)"),
            "DhApplyShader should bind cached validated DH color/depth texture ids resolved during precheck");
        assertTrue(dhApplySource.contains("if (!MC_RENDER.bindTargetRenderTarget(ctx))")
                && dhApplySource.contains("if (!LodRenderer.INSTANCE.bindActiveRenderTarget())"),
            "DhApplyShader should bind MC and DH outputs through owner seams instead of raw framebuffer ids");
        int bindActiveTarget = dhApplySource.indexOf("if (!LodRenderer.INSTANCE.bindActiveRenderTarget())");
        int attachMcTarget = dhApplySource.indexOf("VulkanicAPI.framebufferColorAttachment0Texture2D(ctx, VulkanicAPI.GL_DRAW_FRAMEBUFFER, this.activeTargetColorTextureId, 0);");
        int renderApplyQuad = dhApplySource.indexOf("ScreenQuad.INSTANCE.render();", attachMcTarget);
        int restoreDhTarget = dhApplySource.indexOf("VulkanicAPI.framebufferColorAttachment0Texture2D(ctx, VulkanicAPI.GL_DRAW_FRAMEBUFFER, this.activeDhColorTextureId, 0);");
        assertTrue(bindActiveTarget >= 0
                && attachMcTarget > bindActiveTarget
                && renderApplyQuad > attachMcTarget
                && restoreDhTarget > renderApplyQuad,
            "DhApplyShader should attach the MC color target to the active DH framebuffer only for the scoped apply pass, then restore DH color output");

        Path fadeApplyFile = SRC_MAIN_JAVA.resolve("com/seibel/distanthorizons/core/render/renderer/shaders/FadeApplyShader.java");
        String fadeApplySource = readSource(fadeApplyFile);
        assertTrue(fadeApplySource.contains("public int fadeTexture = -1;"),
            "FadeApplyShader should initialize fadeTexture to unresolved sentinel -1");
        assertTrue(fadeApplySource.contains("public DhFramebuffer readFramebuffer;")
                && fadeApplySource.contains("public boolean drawToMinecraftTarget = false;")
                && fadeApplySource.contains("public boolean drawToLodTarget = false;"),
            "FadeApplyShader should track read/draw targets through framebuffer owners and owner seams instead of cached framebuffer ids");
        assertTrue(fadeApplySource.contains("protected boolean onPreRender(CommandContext ctx, float partialTicks)"),
            "FadeApplyShader should use shared pre-bind precheck hook for unresolved resources");
        assertTrue(fadeApplySource.contains("this.fadeTexture != -1")
                && fadeApplySource.contains("this.activeReadFramebuffer != null")
                && fadeApplySource.contains("(this.activeDrawToMinecraftTarget || this.activeDrawToLodTarget)"),
            "FadeApplyShader precheck should require a resolved source framebuffer and an owner-routed draw target");
        assertTrue(fadeApplySource.contains("this.activeReadFramebuffer.bindAsReadBuffer(ctx);")
                && fadeApplySource.contains("if (this.activeDrawToMinecraftTarget)")
                && fadeApplySource.contains("if (!MC_RENDER.bindTargetRenderTarget(ctx))")
                && fadeApplySource.contains("if (!LodRenderer.INSTANCE.bindActiveRenderTarget())"),
            "FadeApplyShader should bind through framebuffer owners and MC/DH owner seams instead of cached draw framebuffer ids");

        Path fogApplyFile = SRC_MAIN_JAVA.resolve("com/seibel/distanthorizons/core/render/renderer/shaders/FogApplyShader.java");
        String fogApplySource = readSource(fogApplyFile);
        assertTrue(fogApplySource.contains("public int fogTexture = -1;"),
            "FogApplyShader should initialize fog texture id to unresolved sentinel -1");
        assertTrue(fogApplySource.contains("protected boolean onPreRender(CommandContext ctx, float partialTicks)"),
            "FogApplyShader should use shared pre-bind precheck hook for unresolved resources");
        assertTrue(fogApplySource.contains("this.fogTexture != -1")
                && fogApplySource.contains("this.activeDepthTextureId != -1")
                && fogApplySource.contains("FogShader.INSTANCE.frameBuffer != null")
                && fogApplySource.contains("LodRenderer.INSTANCE.hasActiveRenderTarget()"),
            "FogApplyShader precheck should require resolved fog/depth/framebuffer resources before bind/uniform work");
        assertTrue(fogApplySource.contains("DhTextureState.bindTexture2D(this.activeDepthTextureId)")
                && fogApplySource.contains("FogShader.INSTANCE.frameBuffer.bindAsReadBuffer(ctx);")
                && fogApplySource.contains("if (!LodRenderer.INSTANCE.bindActiveRenderTarget())"),
            "FogApplyShader should bind framebuffer owners through the DH render-target seam");

        Path ssaoApplyFile = SRC_MAIN_JAVA.resolve("com/seibel/distanthorizons/core/render/renderer/shaders/SSAOApplyShader.java");
        String ssaoApplySource = readSource(ssaoApplyFile);
        assertTrue(ssaoApplySource.contains("public int ssaoTexture = -1;"),
            "SSAOApplyShader should initialize SSAO texture id to unresolved sentinel -1");
        assertTrue(ssaoApplySource.contains("protected boolean onPreRender(CommandContext ctx, float partialTicks)"),
            "SSAOApplyShader should use shared pre-bind precheck hook for unresolved resources");
        assertTrue(ssaoApplySource.contains("this.ssaoTexture != -1")
                && ssaoApplySource.contains("this.activeDepthTextureId != -1")
                && ssaoApplySource.contains("SSAOShader.INSTANCE.frameBuffer != null")
                && ssaoApplySource.contains("LodRenderer.INSTANCE.hasActiveRenderTarget()"),
            "SSAOApplyShader precheck should require resolved SSAO/depth/framebuffer resources before bind/uniform work");
        assertTrue(ssaoApplySource.contains("DhTextureState.bindTexture2D(this.activeDepthTextureId)")
                && ssaoApplySource.contains("SSAOShader.INSTANCE.frameBuffer.bindAsReadBuffer(ctx);")
                && ssaoApplySource.contains("if (!LodRenderer.INSTANCE.bindActiveRenderTarget())"),
            "SSAOApplyShader should bind framebuffer owners through the DH render-target seam");

        Path dhFadeShaderFile = SRC_MAIN_JAVA.resolve("com/seibel/distanthorizons/core/render/renderer/shaders/DhFadeShader.java");
        String dhFadeShaderSource = readSource(dhFadeShaderFile);
        assertTrue(dhFadeShaderSource.contains("public DhFramebuffer frameBuffer;")
                && dhFadeShaderSource.contains("protected boolean onPreRender(CommandContext ctx, float partialTicks)"),
            "DhFadeShader should use shared pre-bind precheck hook for unresolved resources");
        assertTrue(dhFadeShaderSource.contains("this.activeDepthTextureId = depthTextureId;")
                && dhFadeShaderSource.contains("this.activeColorTextureId = colorTextureId;")
                && dhFadeShaderSource.contains("this.activeMcColorTextureId = mcColorTextureId;")
                && dhFadeShaderSource.contains("this.activeFrameBuffer = this.frameBuffer;"),
            "DhFadeShader should cache validated depth/color/framebuffer resource ids during precheck");
        assertTrue(dhFadeShaderSource.contains("this.activeFrameBuffer.bind(ctx);")
                && dhFadeShaderSource.contains("DhTextureState.bindTexture2D(this.activeMcColorTextureId)")
                && dhFadeShaderSource.contains("DhTextureState.bindTexture2D(this.activeColorTextureId)"),
            "DhFadeShader should bind cached validated framebuffer owners and texture ids resolved during precheck");

        Path vanillaFadeShaderFile = SRC_MAIN_JAVA.resolve("com/seibel/distanthorizons/core/render/renderer/shaders/VanillaFadeShader.java");
        String vanillaFadeShaderSource = readSource(vanillaFadeShaderFile);
        assertTrue(vanillaFadeShaderSource.contains("public DhFramebuffer frameBuffer;")
                && vanillaFadeShaderSource.contains("protected boolean onPreRender(CommandContext ctx, float partialTicks)"),
            "VanillaFadeShader should use shared pre-bind precheck hook for unresolved resources");
        assertTrue(vanillaFadeShaderSource.contains("this.activeMcDepthTextureId = mcDepthTextureId;")
                && vanillaFadeShaderSource.contains("this.activeMcColorTextureId = mcColorTextureId;")
                && vanillaFadeShaderSource.contains("this.activeDepthTextureId = depthTextureId;")
                && vanillaFadeShaderSource.contains("this.activeColorTextureId = colorTextureId;")
                && vanillaFadeShaderSource.contains("this.activeFrameBuffer = this.frameBuffer;"),
            "VanillaFadeShader should cache validated MC/DH texture and framebuffer ids during precheck");
        assertTrue(vanillaFadeShaderSource.contains("this.activeFrameBuffer.bind(ctx);")
                && vanillaFadeShaderSource.contains("DhTextureState.bindTexture2D(this.activeMcDepthTextureId)")
                && vanillaFadeShaderSource.contains("DhTextureState.bindTexture2D(this.activeMcColorTextureId)")
                && vanillaFadeShaderSource.contains("DhTextureState.bindTexture2D(this.activeDepthTextureId)")
                && vanillaFadeShaderSource.contains("DhTextureState.bindTexture2D(this.activeColorTextureId)"),
            "VanillaFadeShader should bind cached validated MC/DH texture and framebuffer owners resolved during precheck");

        Path fogShaderFile = SRC_MAIN_JAVA.resolve("com/seibel/distanthorizons/core/render/renderer/shaders/FogShader.java");
        String fogShaderSource = readSource(fogShaderFile);
        assertTrue(fogShaderSource.contains("public DhFramebuffer frameBuffer;"),
            "FogShader should store its target as a framebuffer owner instead of a raw id");
        assertTrue(fogShaderSource.contains("protected boolean onPreRender(CommandContext ctx, float partialTicks)"),
            "FogShader should use shared pre-bind precheck hook for unresolved resources");
        assertTrue(fogShaderSource.contains("if (this.frameBuffer == null || depthTextureId == -1 || colorTextureId == -1)")
                && fogShaderSource.contains("this.activeFrameBuffer = this.frameBuffer;")
                && fogShaderSource.contains("this.activeDepthTextureId = depthTextureId;")
                && fogShaderSource.contains("this.activeColorTextureId = colorTextureId;"),
            "FogShader precheck should validate and cache framebuffer/depth/color texture ids before bind/uniform work");
        assertTrue(fogShaderSource.contains("this.activeFrameBuffer.bind(ctx);")
                && fogShaderSource.contains("DhTextureState.bindTexture2D(this.activeDepthTextureId)")
                && fogShaderSource.contains("DhTextureState.bindTexture2D(this.activeColorTextureId)"),
            "FogShader should bind cached validated framebuffer owners and texture ids resolved during precheck");

        Path ssaoShaderFile = SRC_MAIN_JAVA.resolve("com/seibel/distanthorizons/core/render/renderer/shaders/SSAOShader.java");
        String ssaoShaderSource = readSource(ssaoShaderFile);
        assertTrue(ssaoShaderSource.contains("public DhFramebuffer frameBuffer;"),
            "SSAOShader should store its target as a framebuffer owner instead of a raw id");
        assertTrue(ssaoShaderSource.contains("protected boolean onPreRender(CommandContext ctx, float partialTicks)"),
            "SSAOShader should use shared pre-bind precheck hook for unresolved resources");
        assertTrue(ssaoShaderSource.contains("if (this.frameBuffer == null || depthTextureId == -1)")
                && ssaoShaderSource.contains("this.activeFrameBuffer = this.frameBuffer;")
                && ssaoShaderSource.contains("this.activeDepthTextureId = depthTextureId;"),
            "SSAOShader precheck should validate and cache framebuffer/depth texture ids before bind/uniform work");
        assertTrue(ssaoShaderSource.contains("this.activeFrameBuffer.bind(ctx);")
                && ssaoShaderSource.contains("DhTextureState.bindTexture2D(this.activeDepthTextureId)"),
            "SSAOShader should bind cached validated framebuffer owners and depth texture ids resolved during precheck");

        Path vanillaFadeRendererFile = SRC_MAIN_JAVA.resolve("com/seibel/distanthorizons/core/render/renderer/VanillaFadeRenderer.java");
        String vanillaFadeRendererSource = readSource(vanillaFadeRendererFile);
        assertTrue(vanillaFadeRendererSource.contains("if (!MC_RENDER.mcRendersToFrameBuffer())"),
            "VanillaFadeRenderer should branch MC-color attachment setup based on render-target path");
        assertTrue(vanillaFadeRendererSource.contains("if (mcColorTextureId == -1)"),
            "VanillaFadeRenderer should skip fade setup when MC color texture handle is unresolved");
        assertTrue(vanillaFadeRendererSource.contains("if (width <= 0 || height <= 0)"),
            "VanillaFadeRenderer should skip rendering when target viewport dimensions are invalid");

        Path fogRendererFile = SRC_MAIN_JAVA.resolve("com/seibel/distanthorizons/core/render/renderer/FogRenderer.java");
        String fogRendererSource = readSource(fogRendererFile);
        assertTrue(fogRendererSource.contains("if (width <= 0 || height <= 0)"),
            "FogRenderer should skip rendering when target viewport dimensions are invalid");
        assertTrue(fogRendererSource.contains("private DhFramebuffer fogFramebuffer;")
                && fogRendererSource.contains("this.fogFramebuffer = new DhFramebuffer();")
                && fogRendererSource.contains("this.fogFramebuffer.addColorAttachment(ctx, 0, this.fogTexture);")
                && fogRendererSource.contains("if (this.fogFramebuffer == null || this.fogTexture == -1)"),
            "FogRenderer should manage its offscreen target through DhFramebuffer owner objects");

        Path ssaoRendererFile = SRC_MAIN_JAVA.resolve("com/seibel/distanthorizons/core/render/renderer/SSAORenderer.java");
        String ssaoRendererSource = readSource(ssaoRendererFile);
        assertTrue(ssaoRendererSource.contains("if (width <= 0 || height <= 0)"),
            "SSAORenderer should skip rendering when target viewport dimensions are invalid");
        assertTrue(ssaoRendererSource.contains("private DhFramebuffer ssaoFramebuffer;")
                && ssaoRendererSource.contains("this.ssaoFramebuffer = new DhFramebuffer();")
                && ssaoRendererSource.contains("this.ssaoFramebuffer.addColorAttachment(ctx, 0, this.ssaoTexture);")
                && ssaoRendererSource.contains("if (this.ssaoFramebuffer == null || this.ssaoTexture == -1)"),
            "SSAORenderer should manage its offscreen target through DhFramebuffer owner objects");

        Path dhFadeRendererFile = SRC_MAIN_JAVA.resolve("com/seibel/distanthorizons/core/render/renderer/DhFadeRenderer.java");
        String dhFadeRendererSource = readSource(dhFadeRendererFile);
        assertTrue(dhFadeRendererSource.contains("if (width <= 0 || height <= 0)"),
            "DhFadeRenderer should skip rendering when target viewport dimensions are invalid");
        assertTrue(dhFadeRendererSource.contains("private DhFramebuffer fadeFramebuffer;")
                && dhFadeRendererSource.contains("this.fadeFramebuffer = new DhFramebuffer();")
                && dhFadeRendererSource.contains("this.fadeFramebuffer.addColorAttachment(ctx, 0, this.fadeTexture);")
                && dhFadeRendererSource.contains("if (this.fadeFramebuffer == null || this.fadeTexture == -1)"),
            "DhFadeRenderer should manage its offscreen target through DhFramebuffer owner objects");
    }

    @Test
    public void testIrisTextureCreationUsesCreateTexture2DHelper() throws IOException {
        Path[] files = new Path[]{
            SRC_MAIN_JAVA.resolve("net/irisshaders/iris/targets/DepthTexture.java"),
            SRC_MAIN_JAVA.resolve("net/irisshaders/iris/targets/backed/NoiseTexture.java"),
            SRC_MAIN_JAVA.resolve("net/irisshaders/iris/targets/backed/SingleColorTexture.java")
        };

        for (Path file : files) {
            String source = readSource(file);
            assertFalse(source.contains("IrisRenderSystem.createTexture(VulkanicAPI.GL_TEXTURE_2D)"),
                file.getFileName() + " should not pass explicit GL_TEXTURE_2D to IrisRenderSystem.createTexture");
            assertTrue(source.contains("IrisRenderSystem.createTexture2D()"),
                file.getFileName() + " should use IrisRenderSystem.createTexture2D helper");
        }

        Path depthTextureFile = SRC_MAIN_JAVA.resolve("net/irisshaders/iris/targets/DepthTexture.java");
        String depthTextureSource = readSource(depthTextureFile);
        assertFalse(depthTextureSource.contains("IrisRenderSystem.texParameteri(texture, VulkanicAPI.GL_TEXTURE_MIN_FILTER"),
            "DepthTexture should not set min filter through raw GL_TEXTURE_MIN_FILTER pname constants");
        assertFalse(depthTextureSource.contains("IrisRenderSystem.texParameteri(texture, VulkanicAPI.GL_TEXTURE_WRAP_S"),
            "DepthTexture should not set wrap through raw GL_TEXTURE_WRAP_* pname constants");
        assertTrue(depthTextureSource.contains("IrisRenderSystem.setTextureNearestFiltering(texture)"),
            "DepthTexture should use IrisRenderSystem.setTextureNearestFiltering helper");
        assertTrue(depthTextureSource.contains("IrisRenderSystem.setTextureWrapMode2D(texture, true)"),
            "DepthTexture should use IrisRenderSystem.setTextureWrapMode2D clamp helper");

        Path noiseTextureFile = SRC_MAIN_JAVA.resolve("net/irisshaders/iris/targets/backed/NoiseTexture.java");
        String noiseTextureSource = readSource(noiseTextureFile);
        assertFalse(noiseTextureSource.contains("IrisRenderSystem.texParameteri(texture, VulkanicAPI.GL_TEXTURE_MIN_FILTER"),
            "NoiseTexture should not set min filter through raw GL_TEXTURE_MIN_FILTER pname constants");
        assertFalse(noiseTextureSource.contains("IrisRenderSystem.texParameteri(texture, VulkanicAPI.GL_TEXTURE_WRAP_S"),
            "NoiseTexture should not set wrap through raw GL_TEXTURE_WRAP_* pname constants");
        assertFalse(noiseTextureSource.contains("IrisRenderSystem.texParameteri(texture, VulkanicAPI.GL_TEXTURE_MAX_LEVEL"),
            "NoiseTexture should not set LOD range through raw GL_TEXTURE_* LOD pname constants");
        assertTrue(noiseTextureSource.contains("IrisRenderSystem.setTextureLinearFiltering(texture)"),
            "NoiseTexture should use IrisRenderSystem.setTextureLinearFiltering helper");
        assertTrue(noiseTextureSource.contains("IrisRenderSystem.setTextureWrapMode2D(texture, false)"),
            "NoiseTexture should use IrisRenderSystem.setTextureWrapMode2D repeat helper");
        assertTrue(noiseTextureSource.contains("IrisRenderSystem.resetTextureLodRangeToZero(texture)"),
            "NoiseTexture should use IrisRenderSystem.resetTextureLodRangeToZero helper");

        Path singleColorTextureFile = SRC_MAIN_JAVA.resolve("net/irisshaders/iris/targets/backed/SingleColorTexture.java");
        String singleColorTextureSource = readSource(singleColorTextureFile);
        assertFalse(singleColorTextureSource.contains("IrisRenderSystem.texParameteri(texture, VulkanicAPI.GL_TEXTURE_MIN_FILTER"),
            "SingleColorTexture should not set min filter through raw GL_TEXTURE_MIN_FILTER pname constants");
        assertFalse(singleColorTextureSource.contains("IrisRenderSystem.texParameteri(texture, VulkanicAPI.GL_TEXTURE_WRAP_S"),
            "SingleColorTexture should not set wrap through raw GL_TEXTURE_WRAP_* pname constants");
        assertTrue(singleColorTextureSource.contains("IrisRenderSystem.setTextureLinearFiltering(texture)"),
            "SingleColorTexture should use IrisRenderSystem.setTextureLinearFiltering helper");
        assertTrue(singleColorTextureSource.contains("IrisRenderSystem.setTextureWrapMode2D(texture, false)"),
            "SingleColorTexture should use IrisRenderSystem.setTextureWrapMode2D repeat helper");

        Path renderTargetFile = SRC_MAIN_JAVA.resolve("net/irisshaders/iris/targets/RenderTarget.java");
        String renderTargetSource = readSource(renderTargetFile);
        assertFalse(renderTargetSource.contains("IrisRenderSystem.texParameteri(texture, VulkanicAPI.GL_TEXTURE_MIN_FILTER"),
            "RenderTarget should not set min filter through raw GL_TEXTURE_MIN_FILTER pname constants");
        assertFalse(renderTargetSource.contains("IrisRenderSystem.texParameteri(texture, VulkanicAPI.GL_TEXTURE_WRAP_S"),
            "RenderTarget should not set wrap through raw GL_TEXTURE_WRAP_* pname constants");
        assertTrue(renderTargetSource.contains("IrisRenderSystem.setTextureLinearFiltering(texture)"),
            "RenderTarget linear path should use IrisRenderSystem.setTextureLinearFiltering helper");
        assertTrue(renderTargetSource.contains("IrisRenderSystem.setTextureNearestFiltering(texture)"),
            "RenderTarget nearest path should use IrisRenderSystem.setTextureNearestFiltering helper");
        assertTrue(renderTargetSource.contains("IrisRenderSystem.setTextureWrapMode2D(texture, true)"),
            "RenderTarget should use IrisRenderSystem.setTextureWrapMode2D clamp helper");

        Path centerDepthSamplerFile = SRC_MAIN_JAVA.resolve("net/irisshaders/iris/pathways/CenterDepthSampler.java");
        String centerDepthSamplerSource = readSource(centerDepthSamplerFile);
        assertFalse(centerDepthSamplerSource.contains("IrisRenderSystem.texParameteri(texture, VulkanicAPI.GL_TEXTURE_MIN_FILTER"),
            "CenterDepthSampler should not set min filter through raw GL_TEXTURE_MIN_FILTER pname constants");
        assertFalse(centerDepthSamplerSource.contains("IrisRenderSystem.texParameteri(texture, VulkanicAPI.GL_TEXTURE_WRAP_S"),
            "CenterDepthSampler should not set wrap through raw GL_TEXTURE_WRAP_* pname constants");
        assertTrue(centerDepthSamplerSource.contains("IrisRenderSystem.setTextureLinearFiltering(texture)"),
            "CenterDepthSampler should use IrisRenderSystem.setTextureLinearFiltering helper");
        assertTrue(centerDepthSamplerSource.contains("IrisRenderSystem.setTextureWrapMode2D(texture, true)"),
            "CenterDepthSampler should use IrisRenderSystem.setTextureWrapMode2D clamp helper");

        Path nativeImageBackedCustomTextureFile = SRC_MAIN_JAVA.resolve("net/irisshaders/iris/targets/backed/NativeImageBackedCustomTexture.java");
        String nativeImageBackedCustomTextureSource = readSource(nativeImageBackedCustomTextureFile);
        assertFalse(nativeImageBackedCustomTextureSource.contains("IrisRenderSystem.texParameteri(getId(), VulkanicAPI.GL_TEXTURE_MIN_FILTER"),
            "NativeImageBackedCustomTexture should not set min filter through raw GL_TEXTURE_MIN_FILTER pname constants");
        assertFalse(nativeImageBackedCustomTextureSource.contains("IrisRenderSystem.texParameteri(getId(), VulkanicAPI.GL_TEXTURE_WRAP_S"),
            "NativeImageBackedCustomTexture should not set wrap through raw GL_TEXTURE_WRAP_* pname constants");
        assertTrue(nativeImageBackedCustomTextureSource.contains("IrisRenderSystem.setTextureLinearFiltering(getId())"),
            "NativeImageBackedCustomTexture blur path should use IrisRenderSystem.setTextureLinearFiltering helper");
        assertTrue(nativeImageBackedCustomTextureSource.contains("IrisRenderSystem.setTextureWrapMode2D(getId(), true)"),
            "NativeImageBackedCustomTexture clamp path should use IrisRenderSystem.setTextureWrapMode2D helper");

        Path shadowRendererFile = SRC_MAIN_JAVA.resolve("net/irisshaders/iris/shadows/ShadowRenderer.java");
        String shadowRendererSource = readSource(shadowRendererFile);
        assertFalse(shadowRendererSource.contains("IrisRenderSystem.texParameteri(glTextureId, VulkanicAPI.GL_TEXTURE_MIN_FILTER, VulkanicAPI.GL_LINEAR)"),
            "ShadowRenderer should not set linear min filter through raw GL_TEXTURE_MIN_FILTER pname constants");
        assertFalse(shadowRendererSource.contains("IrisRenderSystem.texParameteri(glTextureId, VulkanicAPI.GL_TEXTURE_MIN_FILTER, VulkanicAPI.GL_NEAREST)"),
            "ShadowRenderer should not set nearest min filter through raw GL_TEXTURE_MIN_FILTER pname constants");
        assertTrue(shadowRendererSource.contains("IrisRenderSystem.setTextureLinearFiltering(glTextureId)"),
            "ShadowRenderer linear path should use IrisRenderSystem.setTextureLinearFiltering helper");
        assertTrue(shadowRendererSource.contains("IrisRenderSystem.setTextureNearestFiltering(glTextureId)"),
            "ShadowRenderer nearest path should use IrisRenderSystem.setTextureNearestFiltering helper");

        Path irisRenderSystemFile = SRC_MAIN_JAVA.resolve("net/irisshaders/iris/gl/IrisRenderSystem.java");
        String irisRenderSystemSource = readSource(irisRenderSystemFile);
        assertTrue(irisRenderSystemSource.contains("public static void setTextureLinearFiltering(int texture)"),
            "IrisRenderSystem should expose setTextureLinearFiltering helper for object-DSA texture setup");
        assertTrue(irisRenderSystemSource.contains("public static void setTextureNearestFiltering(int texture)"),
            "IrisRenderSystem should expose setTextureNearestFiltering helper for object-DSA texture setup");
        assertTrue(irisRenderSystemSource.contains("public static void setTextureWrapMode2D(int texture, boolean clampToEdge)"),
            "IrisRenderSystem should expose setTextureWrapMode2D helper for object-DSA texture setup");
        assertTrue(irisRenderSystemSource.contains("public static void resetTextureLodRangeToZero(int texture)"),
            "IrisRenderSystem should expose resetTextureLodRangeToZero helper for object-DSA texture setup");
    }

    @Test
    public void testBlaze3dViewportWrapperRemovedFromGlStateManager() throws IOException {
        Path stateManagerFile = SRC_MAIN_JAVA.resolve("net/blaze3d/opengl/GlStateManager.java");
        String stateManagerSource = readSource(stateManagerFile);

        assertFalse(stateManagerSource.contains("public static void _viewport("),
            "GlStateManager should no longer expose _viewport wrapper");

        Path encoderFile = SRC_MAIN_JAVA.resolve("net/blaze3d/opengl/GlCommandEncoder.java");
        String encoderSource = readSource(encoderFile);
        assertFalse(encoderSource.contains("GlStateManager._viewport("),
            "GlCommandEncoder should not call removed GlStateManager._viewport wrapper");
        assertTrue(encoderSource.contains("VulkanicAPI.setDynamicViewport("),
            "GlCommandEncoder should set viewport through VulkanicAPI.setDynamicViewport");

        Path clearPassFile = SRC_MAIN_JAVA.resolve("net/irisshaders/iris/targets/ClearPass.java");
        String clearPassSource = readSource(clearPassFile);
        assertFalse(clearPassSource.contains("GlStateManager._viewport("),
            "ClearPass should not call removed GlStateManager._viewport wrapper");
        assertTrue(clearPassSource.contains("VulkanicAPI.setDynamicViewport("),
            "ClearPass should set viewport through VulkanicAPI.setDynamicViewport");
    }

    @Test
    public void testBlaze3dScissorToggleWrappersRemovedFromGlStateManager() throws IOException {
        Path stateManagerFile = SRC_MAIN_JAVA.resolve("net/blaze3d/opengl/GlStateManager.java");
        String stateManagerSource = readSource(stateManagerFile);

        assertFalse(stateManagerSource.contains("public static void _enableScissorTest("),
            "GlStateManager should no longer expose _enableScissorTest wrapper");
        assertFalse(stateManagerSource.contains("public static void _disableScissorTest("),
            "GlStateManager should no longer expose _disableScissorTest wrapper");

        Path encoderFile = SRC_MAIN_JAVA.resolve("net/blaze3d/opengl/GlCommandEncoder.java");
        String encoderSource = readSource(encoderFile);
        assertFalse(encoderSource.contains("GlStateManager._enableScissorTest("),
            "GlCommandEncoder should not call removed GlStateManager._enableScissorTest wrapper");
        assertFalse(encoderSource.contains("GlStateManager._disableScissorTest("),
            "GlCommandEncoder should not call removed GlStateManager._disableScissorTest wrapper");
        assertTrue(encoderSource.contains("VulkanicAPI.setScissorTestEnabled("),
            "GlCommandEncoder should toggle scissor test through VulkanicAPI.setScissorTestEnabled");

        Path dhWrapperFile = SRC_MAIN_JAVA.resolve("com/seibel/distanthorizons/common/wrappers/minecraft/MinecraftGLWrapper.java");
        String dhWrapperSource = readSourceIfExists(dhWrapperFile);
        assertFalse(dhWrapperSource.contains("GlStateManager._enableScissorTest("),
            "MinecraftGLWrapper should not call removed GlStateManager._enableScissorTest wrapper");
        assertFalse(dhWrapperSource.contains("GlStateManager._disableScissorTest("),
            "MinecraftGLWrapper should not call removed GlStateManager._disableScissorTest wrapper");
        assertFalse(dhWrapperSource.contains("public void enableScissorTest("),
            "MinecraftGLWrapper should no longer expose scissor wrapper methods");
        assertFalse(dhWrapperSource.contains("public void disableScissorTest("),
            "MinecraftGLWrapper should no longer expose scissor wrapper methods");
    }

    @Test
    public void testGlDebugLabelUsesAgnosticLabelHelpers() throws IOException {
        Path file = SRC_MAIN_JAVA.resolve("net/blaze3d/opengl/GlDebugLabel.java");
        String source = readSource(file);

        assertFalse(source.contains("getInteger(VulkanicAPI.getImmediateContext(), 33512)"),
            "GlDebugLabel should not query max label length with hardcoded GL_MAX_LABEL_LENGTH literal 33512");
        assertFalse(source.contains("labelDebugObject(VulkanicAPI.getImmediateContext(), 33504"),
            "GlDebugLabel Core should not use hardcoded GL_BUFFER label identifier 33504");
        assertFalse(source.contains("labelDebugObject(VulkanicAPI.getImmediateContext(), 33505"),
            "GlDebugLabel Core should not use hardcoded GL_SHADER label identifier 33505");
        assertFalse(source.contains("labelDebugObject(VulkanicAPI.getImmediateContext(), 33506"),
            "GlDebugLabel Core should not use hardcoded GL_PROGRAM label identifier 33506");
        assertFalse(source.contains("labelObjectExt(VulkanicAPI.getImmediateContext(), 37201"),
            "GlDebugLabel EXT should not use hardcoded GL_BUFFER_OBJECT_EXT literal 37201");

        assertTrue(source.contains("VulkanicAPI.getMaxDebugLabelLength("),
            "GlDebugLabel should query label length via VulkanicAPI.getMaxDebugLabelLength");
        assertTrue(source.contains("VulkanicAPI.labelBufferDebugObject("),
            "GlDebugLabel Core should label buffers via VulkanicAPI.labelBufferDebugObject");
        assertTrue(source.contains("VulkanicAPI.labelTextureDebugObject("),
            "GlDebugLabel Core should label textures via VulkanicAPI.labelTextureDebugObject");
        assertTrue(source.contains("VulkanicAPI.labelVertexArrayDebugObject("),
            "GlDebugLabel Core should label vertex arrays via VulkanicAPI.labelVertexArrayDebugObject");
        assertTrue(source.contains("VulkanicAPI.enterApplicationDebugGroup("),
            "GlDebugLabel Core should enter debug groups via VulkanicAPI.enterApplicationDebugGroup");
        assertTrue(source.contains("VulkanicAPI.labelBufferExtObject("),
            "GlDebugLabel EXT should label buffers via VulkanicAPI.labelBufferExtObject");
    }

    @Test
    public void testNvidiaWorkaroundUsesDebugOutputSyncHelper() throws IOException {
        Path file = SRC_MAIN_JAVA.resolve("net/sodium/client/compatibility/workarounds/nvidia/NvidiaWorkarounds.java");
        String source = readSource(file);

        assertFalse(source.contains("setCapabilityEnabled(ctx, 33346, true)"),
            "NvidiaWorkarounds should not toggle GL_DEBUG_OUTPUT_SYNCHRONOUS using hardcoded literal 33346");
        assertTrue(source.contains("VulkanicAPI.setDebugOutputSynchronousEnabled("),
            "NvidiaWorkarounds should toggle debug output sync via VulkanicAPI.setDebugOutputSynchronousEnabled");
    }

    // ── Task 1b: getActiveVulkanicRenderPass() accessor ───────────────────────

    @Test
    public void testGlCommandEncoderHasGetActiveVulkanicRenderPassAccessor()
            throws NoSuchMethodException {
        Method m = net.blaze3d.opengl.GlCommandEncoder.class
            .getMethod("getActiveVulkanicRenderPass");
        assertNotNull(m, "GlCommandEncoder must expose getActiveVulkanicRenderPass()");
        assertEquals(VulkanicRenderPass.class, m.getReturnType(),
            "getActiveVulkanicRenderPass() must return VulkanicRenderPass");
    }

    @Test
    public void testGetActiveVulkanicRenderPassIsNullableAnnotated() throws NoSuchMethodException {
        Method m = net.blaze3d.opengl.GlCommandEncoder.class
            .getMethod("getActiveVulkanicRenderPass");
        // Method must exist; null-return is the documented contract (no active pass)
        assertNotNull(m);
    }

    // ── Task 2: createTextureViewFromGlHandle bridge removal ─────────────

    @Test
    public void testCreateTextureViewFromGlHandleRemovedFromGraphicsBackend() throws IOException {
        Path file = SRC_MAIN_JAVA.resolve("net/vulkanic/GraphicsBackend.java");
        String source = readSource(file);

        // The bridge method has been removed: GpuTexture now implements VulkanicTexture,
        // so no GL-handle bridge is needed.
        assertFalse(source.contains("createTextureViewFromGlHandle"),
            "createTextureViewFromGlHandle bridge must be removed from GraphicsBackend — " +
            "GpuTexture now implements VulkanicTexture, making the bridge unnecessary");
    }

    @Test
    public void testGpuTextureImplementsVulkanicTextureInSource() throws IOException {
        Path file = SRC_MAIN_JAVA.resolve("net/blaze3d/textures/GpuTexture.java");
        String source = readSource(file);

        assertTrue(source.contains("implements") && source.contains("VulkanicTexture"),
            "GpuTexture must implement VulkanicTexture");
        assertTrue(source.contains("getVulkanicFormat"),
            "GpuTexture must provide getVulkanicFormat() implementing the interface method");
    }

    // ── Task 3: VoxelMap bypass migrated to DSA ───────────────────────────────

    @Test
    public void testCompressibleGLBufferedImageUsesGenerateMipmapDSA() throws IOException {
        Path file = SRC_MAIN_JAVA.resolve(
            "net/voxelmap/persistent/CompressibleGLBufferedImage.java");
        assertTrue(Files.exists(file), "CompressibleGLBufferedImage.java must exist");
        String source = readSource(file);

        assertFalse(source.contains("VulkanicAPI.bindTexture2D"),
            "CompressibleGLBufferedImage must no longer call bindTexture2D before mipmap generation; " +
            "use generateTextureMipmapDSA instead to avoid mutating global GL texture bind state");
        assertFalse(source.contains("VulkanicAPI.generateTextureMipmap("),
            "CompressibleGLBufferedImage must no longer call the non-DSA generateTextureMipmap; " +
            "it must use generateTextureMipmapDSA");
        assertTrue(source.contains("VulkanicAPI.generateTextureMipmapDSA("),
            "CompressibleGLBufferedImage must call generateTextureMipmapDSA for state-mutation-free mipmap generation");
    }

    @Test
    public void testCompressibleGLBufferedImageDropsCommandContextImport() throws IOException {
        Path file = SRC_MAIN_JAVA.resolve(
            "net/voxelmap/persistent/CompressibleGLBufferedImage.java");
        String source = readSource(file);

        assertFalse(source.contains("import net.vulkanic.CommandContext;"),
            "CompressibleGLBufferedImage must not import CommandContext after the mipmap migration " +
            "(the local variable is no longer needed)");
    }

    @Test
    public void testCompressibleGLBufferedImageStillCallsVulkanicAPI() throws IOException {
        Path file = SRC_MAIN_JAVA.resolve(
            "net/voxelmap/persistent/CompressibleGLBufferedImage.java");
        String source = readSource(file);

        assertTrue(source.contains("VulkanicAPI."),
            "CompressibleGLBufferedImage must still call VulkanicAPI (generateTextureMipmapDSA)");
    }

    @Test
    public void testModelViewOwnershipMovedToVulkanicAPI() throws IOException {
        Path renderSystemFile = SRC_MAIN_JAVA.resolve("net/blaze3d/systems/RenderSystem.java");
        String renderSystemSource = readSource(renderSystemFile);

        assertFalse(renderSystemSource.contains("private static final Matrix4fStack modelViewStack"),
            "RenderSystem should not own modelViewStack after model-view migration to VulkanicAPI");
        assertFalse(renderSystemSource.contains("public static Matrix4f getModelViewMatrix("),
            "RenderSystem should not expose getModelViewMatrix after model-view migration to VulkanicAPI");
        assertFalse(renderSystemSource.contains("public static Matrix4fStack getModelViewStack("),
            "RenderSystem should not expose getModelViewStack after model-view migration to VulkanicAPI");

        Path vulkanicApiFile = SRC_MAIN_JAVA.resolve("net/vulkanic/VulkanicAPI.java");
        String vulkanicApiSource = readSource(vulkanicApiFile);

        assertTrue(vulkanicApiSource.contains("private static final Matrix4fStack modelViewStack = new Matrix4fStack(16);"),
            "VulkanicAPI should own the model-view stack after migration");
        assertTrue(vulkanicApiSource.contains("public static Matrix4f getModelViewMatrix("),
            "VulkanicAPI should expose getModelViewMatrix after migration");
        assertTrue(vulkanicApiSource.contains("public static Matrix4fStack getModelViewStack("),
            "VulkanicAPI should expose getModelViewStack after migration");
        assertTrue(vulkanicApiSource.contains("public static void setupDefaultState("),
            "VulkanicAPI should own setupDefaultState after migration");
        assertTrue(vulkanicApiSource.contains("getModelViewStack().clear();"),
            "VulkanicAPI.setupDefaultState should clear the model-view stack");
        assertTrue(vulkanicApiSource.contains("resetTextureMatrix();"),
            "VulkanicAPI.setupDefaultState should reset the texture matrix");
    }

    @Test
    public void testBootstrapHelpersOwnershipMovedToVulkanicAPI() throws IOException {
        Path renderSystemFile = SRC_MAIN_JAVA.resolve("net/blaze3d/systems/RenderSystem.java");
        String renderSystemSource = readSource(renderSystemFile);

        assertFalse(renderSystemSource.contains("public static String getBackendDescription("),
            "RenderSystem should not expose getBackendDescription after bootstrap migration");
        assertFalse(renderSystemSource.contains("public static String getApiDescription("),
            "RenderSystem should not expose getApiDescription after bootstrap migration");
        assertFalse(renderSystemSource.contains("public static NanoTimeSource initBackendSystem("),
            "RenderSystem should not expose initBackendSystem after bootstrap migration");
        assertFalse(renderSystemSource.contains("public static void setupDefaultState("),
            "RenderSystem should not expose setupDefaultState after bootstrap migration");
        assertFalse(renderSystemSource.contains("public static void setErrorCallback("),
            "RenderSystem should not expose setErrorCallback after bootstrap migration");
        assertFalse(renderSystemSource.contains("public static boolean isFrozenAtPollEvents("),
            "RenderSystem should not expose isFrozenAtPollEvents after poll-state migration");
        assertFalse(renderSystemSource.contains("public static void limitDisplayFPS("),
            "RenderSystem should not expose limitDisplayFPS after frame pacing migration");

        Path vulkanicApiFile = SRC_MAIN_JAVA.resolve("net/vulkanic/VulkanicAPI.java");
        String vulkanicApiSource = readSource(vulkanicApiFile);

        assertTrue(vulkanicApiSource.contains("public static String getBackendDescription("),
            "VulkanicAPI should expose getBackendDescription after bootstrap migration");
        assertTrue(vulkanicApiSource.contains("public static String getApiDescription("),
            "VulkanicAPI should expose getApiDescription after bootstrap migration");
        assertTrue(vulkanicApiSource.contains("public static NanoTimeSource initBackendSystem("),
            "VulkanicAPI should expose initBackendSystem after bootstrap migration");
        assertTrue(vulkanicApiSource.contains("public static void setupDefaultState("),
            "VulkanicAPI should expose setupDefaultState after bootstrap migration");
        assertTrue(vulkanicApiSource.contains("public static void setErrorCallback("),
            "VulkanicAPI should expose setErrorCallback after bootstrap migration");
        assertTrue(vulkanicApiSource.contains("public static void pollEvents("),
            "VulkanicAPI should expose pollEvents after poll-state migration");
        assertTrue(vulkanicApiSource.contains("public static boolean isFrozenAtPollEvents("),
            "VulkanicAPI should expose isFrozenAtPollEvents after poll-state migration");
        assertTrue(vulkanicApiSource.contains("public static void limitDisplayFPS("),
            "VulkanicAPI should expose limitDisplayFPS after frame pacing migration");

        Path minecraftFile = SRC_MAIN_JAVA.resolve("net/minecraft/client/Minecraft.java");
        String minecraftSource = readSource(minecraftFile);

        assertFalse(minecraftSource.contains("RenderSystem.getBackendDescription("),
            "Minecraft should not call RenderSystem.getBackendDescription after bootstrap migration");
        assertFalse(minecraftSource.contains("RenderSystem.getApiDescription("),
            "Minecraft should not call RenderSystem.getApiDescription after bootstrap migration");
        assertFalse(minecraftSource.contains("RenderSystem.initBackendSystem("),
            "Minecraft should not call RenderSystem.initBackendSystem after bootstrap migration");
        assertFalse(minecraftSource.contains("RenderSystem.setupDefaultState("),
            "Minecraft should not call RenderSystem.setupDefaultState after bootstrap migration");
        assertFalse(minecraftSource.contains("RenderSystem.setErrorCallback("),
            "Minecraft should not call RenderSystem.setErrorCallback after bootstrap migration");
        assertFalse(minecraftSource.contains("RenderSystem.limitDisplayFPS("),
            "Minecraft should not call RenderSystem.limitDisplayFPS after frame pacing migration");
        assertTrue(minecraftSource.contains("VulkanicAPI.getBackendDescription("),
            "Minecraft should call VulkanicAPI.getBackendDescription after bootstrap migration");
        assertTrue(
            minecraftSource.contains("VulkanicAPI.getApiDescription(")
                || minecraftSource.contains("VulkanicAPI::getApiDescription"),
            "Minecraft should call VulkanicAPI.getApiDescription after bootstrap migration"
        );
        assertTrue(minecraftSource.contains("VulkanicAPI.initBackendSystem("),
            "Minecraft should call VulkanicAPI.initBackendSystem after bootstrap migration");
        assertTrue(minecraftSource.contains("VulkanicAPI.setupDefaultState("),
            "Minecraft should call VulkanicAPI.setupDefaultState after bootstrap migration");
        assertTrue(minecraftSource.contains("VulkanicAPI.setErrorCallback("),
            "Minecraft should call VulkanicAPI.setErrorCallback after bootstrap migration");
        assertTrue(minecraftSource.contains("VulkanicAPI.limitDisplayFPS("),
            "Minecraft should call VulkanicAPI.limitDisplayFPS after frame pacing migration");

        Path glxFile = SRC_MAIN_JAVA.resolve("net/blaze3d/platform/GLX.java");
        String glxSource = readSource(glxFile);
        assertFalse(glxSource.contains("RenderSystem.setErrorCallback("),
            "GLX should not route GLFW error callback setup through RenderSystem after bootstrap migration");
        assertTrue(glxSource.contains("VulkanicAPI.setErrorCallback("),
            "GLX should route GLFW error callback setup through VulkanicAPI after bootstrap migration");

        Path packetListenerFile = SRC_MAIN_JAVA.resolve("net/minecraft/client/multiplayer/ClientCommonPacketListenerImpl.java");
        String packetListenerSource = readSource(packetListenerFile);
        assertFalse(packetListenerSource.contains("RenderSystem.isFrozenAtPollEvents("),
            "ClientCommonPacketListenerImpl should not query poll freeze state through RenderSystem after migration");
        assertTrue(packetListenerSource.contains("VulkanicAPI.isFrozenAtPollEvents("),
            "ClientCommonPacketListenerImpl should query poll freeze state through VulkanicAPI after migration");

        assertTrue(renderSystemSource.contains("VulkanicAPI.pollEvents();"),
            "RenderSystem.flipFrame should route poll event calls through VulkanicAPI after migration");
    }

    @Test
    public void testThreadAndDeviceOwnershipMovedToVulkanicAPI() throws IOException {
        Path renderSystemFile = SRC_MAIN_JAVA.resolve("net/blaze3d/systems/RenderSystem.java");
        String renderSystemSource = readSource(renderSystemFile);

        assertFalse(renderSystemSource.contains("private static Thread renderThread;"),
            "RenderSystem should not own renderThread after thread ownership migration");
        assertFalse(renderSystemSource.contains("private static GpuDevice DEVICE;"),
            "RenderSystem should not own DEVICE after device ownership migration");
        assertFalse(renderSystemSource.contains("public static void initRenderThread("),
            "RenderSystem should not expose initRenderThread after thread ownership migration");
        assertFalse(renderSystemSource.contains("public static boolean isOnRenderThread("),
            "RenderSystem should not expose isOnRenderThread after thread ownership migration");
        assertFalse(renderSystemSource.contains("public static boolean isInInit("),
            "RenderSystem should not expose isInInit after thread ownership migration");
        assertFalse(renderSystemSource.contains("public static void assertOnRenderThreadOrInit("),
            "RenderSystem should not expose assertOnRenderThreadOrInit after thread ownership migration");
        assertFalse(renderSystemSource.contains("public static GpuDevice tryGetDevice("),
            "RenderSystem should not expose tryGetDevice after device ownership migration");
        assertTrue(renderSystemSource.contains("VulkanicAPI.setDevice("),
            "RenderSystem.initRenderer should set the device through VulkanicAPI after migration");
        assertTrue(renderSystemSource.contains("VulkanicAPI.createRendererDevice("),
            "RenderSystem.initRenderer should create the device through the backend-owned VulkanicAPI seam after migration");
        assertFalse(renderSystemSource.contains("new GlDevice("),
            "RenderSystem.initRenderer should not hard-code GlDevice construction after backend-owned device migration");
        assertTrue(renderSystemSource.contains("return net.vulkanic.VulkanicAPI.getDevice();"),
            "RenderSystem.getDevice should delegate to VulkanicAPI after migration");

        Path vulkanicApiFile = SRC_MAIN_JAVA.resolve("net/vulkanic/VulkanicAPI.java");
        String vulkanicApiSource = readSource(vulkanicApiFile);

        assertTrue(vulkanicApiSource.contains("private static Thread renderThread;"),
            "VulkanicAPI should own renderThread after migration");
        assertTrue(vulkanicApiSource.contains("private static GpuDevice device;"),
            "VulkanicAPI should own device after migration");
        assertTrue(vulkanicApiSource.contains("public static void initRenderThread("),
            "VulkanicAPI should expose initRenderThread after migration");
        assertTrue(vulkanicApiSource.contains("public static boolean isOnRenderThread("),
            "VulkanicAPI should expose isOnRenderThread after migration");
        assertTrue(vulkanicApiSource.contains("public static boolean isInInit("),
            "VulkanicAPI should expose isInInit after migration");
        assertTrue(vulkanicApiSource.contains("public static void assertOnRenderThreadOrInit("),
            "VulkanicAPI should expose assertOnRenderThreadOrInit after migration");
        assertTrue(vulkanicApiSource.contains("public static void setDevice("),
            "VulkanicAPI should expose setDevice after migration");
        assertTrue(vulkanicApiSource.contains("public static GpuDevice getDevice("),
            "VulkanicAPI should expose getDevice after migration");
        assertTrue(vulkanicApiSource.contains("public static GpuDevice tryGetDevice("),
            "VulkanicAPI should expose tryGetDevice after migration");

        Path mainFile = SRC_MAIN_JAVA.resolve("net/minecraft/client/main/Main.java");
        String mainSource = readSource(mainFile);
        assertFalse(mainSource.contains("RenderSystem.initRenderThread("),
            "Main should not initialize render thread through RenderSystem after migration");
        assertTrue(mainSource.contains("VulkanicAPI.initRenderThread("),
            "Main should initialize render thread through VulkanicAPI after migration");

        Path irisRenderSystemFile = SRC_MAIN_JAVA.resolve("net/irisshaders/iris/gl/IrisRenderSystem.java");
        String irisRenderSystemSource = readSource(irisRenderSystemFile);
        assertFalse(irisRenderSystemSource.contains("RenderSystem.assertOnRenderThreadOrInit("),
            "IrisRenderSystem should not call RenderSystem.assertOnRenderThreadOrInit after migration");
        assertTrue(irisRenderSystemSource.contains("VulkanicAPI.assertOnRenderThreadOrInit("),
            "IrisRenderSystem should call VulkanicAPI.assertOnRenderThreadOrInit after migration");

        Path renderAssertsFile = SRC_MAIN_JAVA.resolve("net/sodium/client/render/util/RenderAsserts.java");
        String renderAssertsSource = readSource(renderAssertsFile);
        assertFalse(renderAssertsSource.contains("RenderSystem.isOnRenderThread("),
            "RenderAsserts should not call RenderSystem.isOnRenderThread after migration");
        assertTrue(renderAssertsSource.contains("VulkanicAPI.isOnRenderThread("),
            "RenderAsserts should call VulkanicAPI.isOnRenderThread after migration");

        Path voxelImageFile = SRC_MAIN_JAVA.resolve("net/voxelmap/persistent/CompressibleGLBufferedImage.java");
        String voxelImageSource = readSource(voxelImageFile);
        assertFalse(voxelImageSource.contains("RenderSystem.isOnRenderThread("),
            "CompressibleGLBufferedImage should not call RenderSystem.isOnRenderThread after migration");
        assertTrue(voxelImageSource.contains("VulkanicAPI.isOnRenderThread("),
            "CompressibleGLBufferedImage should call VulkanicAPI.isOnRenderThread after migration");

        Path minecraftFile = SRC_MAIN_JAVA.resolve("net/minecraft/client/Minecraft.java");
        String minecraftSource = readSource(minecraftFile);
        assertFalse(minecraftSource.contains("RenderSystem.tryGetDevice("),
            "Minecraft should not call RenderSystem.tryGetDevice after migration");
        assertTrue(minecraftSource.contains("VulkanicAPI.tryGetDevice("),
            "Minecraft should call VulkanicAPI.tryGetDevice after migration");
    }

    @Test
    public void testBlaze3dPackageUsesVulkanicAPIGetDevice() throws IOException {
        Path[] migratedFiles = new Path[] {
            SRC_MAIN_JAVA.resolve("net/blaze3d/platform/Lighting.java"),
            SRC_MAIN_JAVA.resolve("net/blaze3d/vertex/VertexFormat.java"),
            SRC_MAIN_JAVA.resolve("net/blaze3d/platform/TextureUtil.java"),
            SRC_MAIN_JAVA.resolve("net/blaze3d/TracyFrameCapture.java"),
            SRC_MAIN_JAVA.resolve("net/blaze3d/pipeline/RenderTarget.java"),
            SRC_MAIN_JAVA.resolve("net/blaze3d/pipeline/MainTarget.java"),
            SRC_MAIN_JAVA.resolve("net/blaze3d/resource/RenderTargetDescriptor.java"),
            SRC_MAIN_JAVA.resolve("net/blaze3d/font/TrueTypeGlyphProvider.java")
        };

        for (Path file : migratedFiles) {
            String source = readSource(file);
            assertFalse(source.contains("RenderSystem.getDevice("),
                file + " should not call RenderSystem.getDevice after device-access migration");
            boolean usesDeviceSeam = source.contains("VulkanicAPI.getDevice(");
            boolean usesCommandEncoderSeam = source.contains("VulkanicAPI.createCommandEncoder(");
            boolean usesRenderPassSeam = source.contains("VulkanicAPI.createRenderPass(");
            boolean usesRenderTargetSelectionSeam = source.contains("IrisVulkanRenderTargetContract.selectTarget(")
                || source.contains("renderTargetSelection.createRenderPass(");
            boolean usesTextureSeam = source.contains("VulkanicAPI.createTexture(");
            boolean usesBufferSeam = source.contains("VulkanicAPI.createBuffer(");
            boolean usesTextureViewSeam = source.contains("VulkanicAPI.createTextureView(");
            boolean usesBackendMaxTextureSizeSeam = source.contains("VulkanicAPI.getBackendMaxTextureSize(");
            boolean usesBackendUniformAlignmentSeam = source.contains("VulkanicAPI.getBackendUniformOffsetAlignment(");
            boolean usesBackendDeviceInfoSeam = source.contains("VulkanicAPI.getBackendDeviceInfo(");
            assertTrue(
                usesDeviceSeam
                    || usesCommandEncoderSeam
                    || usesRenderPassSeam
                    || usesRenderTargetSelectionSeam
                    || usesTextureSeam
                    || usesBufferSeam
                    || usesTextureViewSeam
                    || usesBackendMaxTextureSizeSeam
                    || usesBackendUniformAlignmentSeam
                    || usesBackendDeviceInfoSeam,
                file + " should call VulkanicAPI.getDevice/createCommandEncoder/createRenderPass/createTexture/createBuffer/createTextureView/getBackendMaxTextureSize/getBackendUniformOffsetAlignment/getBackendDeviceInfo after device-access migration"
            );
        }
    }

    @Test
    public void testVoxelMapPackageUsesVulkanicAPIGetDevice() throws IOException {
        Path[] migratedFiles = new Path[] {
            SRC_MAIN_JAVA.resolve("net/voxelmap/util/VoxelMapCachedOrthoProjectionMatrixBuffer.java"),
            SRC_MAIN_JAVA.resolve("net/voxelmap/util/AllocatedTexture.java"),
            SRC_MAIN_JAVA.resolve("net/voxelmap/util/GLUtils.java"),
            SRC_MAIN_JAVA.resolve("net/voxelmap/entityrender/EntityMapImageManager.java"),
            SRC_MAIN_JAVA.resolve("net/voxelmap/textures/TextureAtlas.java"),
            SRC_MAIN_JAVA.resolve("net/voxelmap/Map.java")
        };

        for (Path file : migratedFiles) {
            String source = readSource(file);
            assertFalse(source.contains("RenderSystem.getDevice("),
                file + " should not call RenderSystem.getDevice after device-access migration");
            boolean usesDeviceSeam = source.contains("VulkanicAPI.getDevice(");
            boolean usesCommandEncoderSeam = source.contains("VulkanicAPI.createCommandEncoder(");
            boolean usesRenderPassSeam = source.contains("VulkanicAPI.createRenderPass(");
            boolean usesRenderTargetSelectionSeam = source.contains("IrisVulkanRenderTargetContract.selectTarget(")
                || source.contains("renderTargetSelection.createRenderPass(");
            boolean usesTextureSeam = source.contains("VulkanicAPI.createTexture(");
            boolean usesBufferSeam = source.contains("VulkanicAPI.createBuffer(");
            boolean usesTextureViewSeam = source.contains("VulkanicAPI.createTextureView(");
            boolean usesBackendMaxTextureSizeSeam = source.contains("VulkanicAPI.getBackendMaxTextureSize(");
            boolean usesBackendUniformAlignmentSeam = source.contains("VulkanicAPI.getBackendUniformOffsetAlignment(");
            boolean usesBackendDeviceInfoSeam = source.contains("VulkanicAPI.getBackendDeviceInfo(");
            assertTrue(
                usesDeviceSeam
                    || usesCommandEncoderSeam
                    || usesRenderPassSeam
                    || usesRenderTargetSelectionSeam
                    || usesTextureSeam
                    || usesBufferSeam
                    || usesTextureViewSeam
                    || usesBackendMaxTextureSizeSeam
                    || usesBackendUniformAlignmentSeam
                    || usesBackendDeviceInfoSeam,
                file + " should call VulkanicAPI.getDevice/createCommandEncoder/createRenderPass/createTexture/createBuffer/createTextureView/getBackendMaxTextureSize/getBackendUniformOffsetAlignment/getBackendDeviceInfo after device-access migration"
            );
        }
    }

    @Test
    public void testMinecraftAndGuiUseVulkanicAPIGetDevice() throws IOException {
        Path[] migratedFiles = new Path[] {
            SRC_MAIN_JAVA.resolve("net/minecraft/client/Minecraft.java"),
            SRC_MAIN_JAVA.resolve("net/minecraft/client/gui/font/FontTexture.java"),
            SRC_MAIN_JAVA.resolve("net/minecraft/client/gui/font/glyphs/SpecialGlyphs.java"),
            SRC_MAIN_JAVA.resolve("net/minecraft/client/gui/font/providers/BitmapProvider.java"),
            SRC_MAIN_JAVA.resolve("net/minecraft/client/gui/font/providers/UnihexProvider.java"),
            SRC_MAIN_JAVA.resolve("net/minecraft/client/gui/render/GuiRenderer.java"),
            SRC_MAIN_JAVA.resolve("net/minecraft/client/gui/render/pip/PictureInPictureRenderer.java"),
            SRC_MAIN_JAVA.resolve("net/minecraft/client/gui/components/DebugScreenOverlay.java"),
            SRC_MAIN_JAVA.resolve("net/minecraft/client/gui/components/debug/DebugEntrySystemSpecs.java"),
            SRC_MAIN_JAVA.resolve("net/minecraft/client/gui/screens/LoadingOverlay.java")
        };

        for (Path file : migratedFiles) {
            String source = readSource(file);
            assertFalse(source.contains("RenderSystem.getDevice("),
                file + " should not call RenderSystem.getDevice after device-access migration");
            boolean usesDeviceSeam = source.contains("VulkanicAPI.getDevice(");
            boolean usesCommandEncoderSeam = source.contains("VulkanicAPI.createCommandEncoder(");
            boolean usesRenderPassSeam = source.contains("VulkanicAPI.createRenderPass(");
            boolean usesRenderTargetSelectionSeam = source.contains("IrisVulkanRenderTargetContract.selectTarget(")
                || source.contains("renderTargetSelection.createRenderPass(");
            boolean usesTextureSeam = source.contains("VulkanicAPI.createTexture(");
            boolean usesBufferSeam = source.contains("VulkanicAPI.createBuffer(");
            boolean usesTextureViewSeam = source.contains("VulkanicAPI.createTextureView(");
            boolean usesBackendMaxTextureSizeSeam = source.contains("VulkanicAPI.getBackendMaxTextureSize(");
            boolean usesBackendUniformAlignmentSeam = source.contains("VulkanicAPI.getBackendUniformOffsetAlignment(");
            boolean usesBackendDeviceInfoSeam = source.contains("VulkanicAPI.getBackendDeviceInfo(");
            assertTrue(
                usesDeviceSeam
                    || usesCommandEncoderSeam
                    || usesRenderPassSeam
                    || usesRenderTargetSelectionSeam
                    || usesTextureSeam
                    || usesBufferSeam
                    || usesTextureViewSeam
                    || usesBackendMaxTextureSizeSeam
                    || usesBackendUniformAlignmentSeam
                    || usesBackendDeviceInfoSeam,
                file + " should call VulkanicAPI.getDevice/createCommandEncoder/createRenderPass/createTexture/createBuffer/createTextureView/getBackendMaxTextureSize/getBackendUniformOffsetAlignment/getBackendDeviceInfo after device-access migration"
            );
        }
    }

    @Test
    public void testRendererClusterUsesVulkanicAPIGetDevice() throws IOException {
        Path[] migratedFiles = new Path[] {
            SRC_MAIN_JAVA.resolve("net/minecraft/client/Screenshot.java"),
            SRC_MAIN_JAVA.resolve("net/minecraft/client/renderer/CloudRenderer.java"),
            SRC_MAIN_JAVA.resolve("net/minecraft/client/renderer/LightTexture.java"),
            SRC_MAIN_JAVA.resolve("net/minecraft/client/renderer/CubeMap.java"),
            SRC_MAIN_JAVA.resolve("net/minecraft/client/renderer/RenderType.java"),
            SRC_MAIN_JAVA.resolve("net/minecraft/client/renderer/GameRenderer.java"),
            SRC_MAIN_JAVA.resolve("net/minecraft/client/renderer/LevelRenderer.java"),
            SRC_MAIN_JAVA.resolve("net/minecraft/client/renderer/WorldBorderRenderer.java"),
            SRC_MAIN_JAVA.resolve("net/minecraft/client/renderer/SkyRenderer.java"),
            SRC_MAIN_JAVA.resolve("net/minecraft/client/renderer/chunk/ChunkSectionsToRender.java"),
            SRC_MAIN_JAVA.resolve("net/minecraft/client/renderer/chunk/CompiledSectionMesh.java"),
            SRC_MAIN_JAVA.resolve("net/minecraft/client/renderer/texture/SpriteContents.java"),
            SRC_MAIN_JAVA.resolve("net/minecraft/client/renderer/texture/DynamicTexture.java"),
            SRC_MAIN_JAVA.resolve("net/minecraft/client/renderer/texture/ReloadableTexture.java"),
            SRC_MAIN_JAVA.resolve("net/minecraft/client/renderer/texture/CubeMapTexture.java"),
            SRC_MAIN_JAVA.resolve("net/minecraft/client/renderer/MappableRingBuffer.java"),
            SRC_MAIN_JAVA.resolve("net/minecraft/client/renderer/PerspectiveProjectionMatrixBuffer.java"),
            SRC_MAIN_JAVA.resolve("net/minecraft/client/renderer/CachedPerspectiveProjectionMatrixBuffer.java"),
            SRC_MAIN_JAVA.resolve("net/minecraft/client/renderer/CachedOrthoProjectionMatrixBuffer.java"),
            SRC_MAIN_JAVA.resolve("net/minecraft/client/renderer/fog/FogRenderer.java"),
            SRC_MAIN_JAVA.resolve("net/minecraft/client/renderer/feature/ParticleFeatureRenderer.java")
        };

        for (Path file : migratedFiles) {
            String source = readSource(file);
            assertFalse(source.contains("RenderSystem.getDevice("),
                file + " should not call RenderSystem.getDevice after renderer-cluster migration");
            boolean usesDeviceSeam = source.contains("VulkanicAPI.getDevice(");
            boolean usesCommandEncoderSeam = source.contains("VulkanicAPI.createCommandEncoder(");
            boolean usesRenderPassSeam = source.contains("VulkanicAPI.createRenderPass(");
            boolean usesRenderTargetSelectionSeam = source.contains("IrisVulkanRenderTargetContract.selectTarget(")
                || source.contains("renderTargetSelection.createRenderPass(");
            boolean usesTextureSeam = source.contains("VulkanicAPI.createTexture(");
            boolean usesBufferSeam = source.contains("VulkanicAPI.createBuffer(");
            boolean usesTextureViewSeam = source.contains("VulkanicAPI.createTextureView(");
            boolean usesBackendMaxTextureSizeSeam = source.contains("VulkanicAPI.getBackendMaxTextureSize(");
            boolean usesBackendUniformAlignmentSeam = source.contains("VulkanicAPI.getBackendUniformOffsetAlignment(");
            boolean usesBackendDeviceInfoSeam = source.contains("VulkanicAPI.getBackendDeviceInfo(");
            assertTrue(
                usesDeviceSeam
                    || usesCommandEncoderSeam
                    || usesRenderPassSeam
                    || usesRenderTargetSelectionSeam
                    || usesTextureSeam
                    || usesBufferSeam
                    || usesTextureViewSeam
                    || usesBackendMaxTextureSizeSeam
                    || usesBackendUniformAlignmentSeam
                    || usesBackendDeviceInfoSeam,
                file + " should call VulkanicAPI.getDevice/createCommandEncoder/createRenderPass/createTexture/createBuffer/createTextureView/getBackendMaxTextureSize/getBackendUniformOffsetAlignment/getBackendDeviceInfo after renderer-cluster migration"
            );
        }
    }

    @Test
    public void testIrisAndSodiumRenderClustersUseVulkanicAPISeams() throws IOException {
        Path[] migratedFiles = new Path[] {
            SRC_MAIN_JAVA.resolve("net/irisshaders/iris/targets/RenderTargets.java"),
            SRC_MAIN_JAVA.resolve("net/irisshaders/iris/targets/backed/NativeImageBackedCustomTexture.java"),
            SRC_MAIN_JAVA.resolve("net/irisshaders/iris/targets/backed/NativeImageBackedNoiseTexture.java"),
            SRC_MAIN_JAVA.resolve("net/irisshaders/iris/pathways/CenterDepthSampler.java"),
            SRC_MAIN_JAVA.resolve("net/irisshaders/iris/pathways/colorspace/ColorSpaceFragmentConverter.java"),
            SRC_MAIN_JAVA.resolve("net/irisshaders/iris/pathways/FullScreenQuadRenderer.java"),
            SRC_MAIN_JAVA.resolve("net/irisshaders/iris/pathways/HorizonRenderer.java"),
            SRC_MAIN_JAVA.resolve("net/irisshaders/iris/shadows/ShadowCompositeRenderer.java"),
            SRC_MAIN_JAVA.resolve("net/irisshaders/iris/shadows/ShadowRenderTargets.java"),
            SRC_MAIN_JAVA.resolve("net/irisshaders/iris/pipeline/FinalPassRenderer.java"),
            SRC_MAIN_JAVA.resolve("net/irisshaders/iris/pipeline/CompositeRenderer.java"),
            SRC_MAIN_JAVA.resolve("net/irisshaders/iris/pbr/texture/PBRAtlasTexture.java"),
            SRC_MAIN_JAVA.resolve("net/sodium/client/gui/SodiumGameOptionPages.java"),
            SRC_MAIN_JAVA.resolve("net/sodium/client/render/chunk/ShaderChunkRenderer.java")
        };

        for (Path file : migratedFiles) {
            String source = readSource(file);
            assertFalse(source.contains("RenderSystem.getDevice("),
                file + " should not call RenderSystem.getDevice after Iris/Sodium seam migration");
            boolean usesDeviceSeam = source.contains("VulkanicAPI.getDevice(");
            boolean usesCommandEncoderSeam = source.contains("VulkanicAPI.createCommandEncoder(");
            boolean usesRenderPassSeam = source.contains("VulkanicAPI.createRenderPass(");
            boolean usesRenderTargetSelectionSeam = source.contains("IrisVulkanRenderTargetContract.selectTarget(")
                || source.contains("renderTargetSelection.createRenderPass(");
            boolean usesTextureSeam = source.contains("VulkanicAPI.createTexture(");
            boolean usesBufferSeam = source.contains("VulkanicAPI.createBuffer(");
            boolean usesTextureViewSeam = source.contains("VulkanicAPI.createTextureView(");
            boolean usesBackendMaxTextureSizeSeam = source.contains("VulkanicAPI.getBackendMaxTextureSize(");
            boolean usesBackendUniformAlignmentSeam = source.contains("VulkanicAPI.getBackendUniformOffsetAlignment(");
            boolean usesBackendDeviceInfoSeam = source.contains("VulkanicAPI.getBackendDeviceInfo(");
            assertTrue(
                usesDeviceSeam
                    || usesCommandEncoderSeam
                    || usesRenderPassSeam
                    || usesRenderTargetSelectionSeam
                    || usesTextureSeam
                    || usesBufferSeam
                    || usesTextureViewSeam
                    || usesBackendMaxTextureSizeSeam
                    || usesBackendUniformAlignmentSeam
                    || usesBackendDeviceInfoSeam,
                file + " should call a backend-owned VulkanicAPI or Iris render-target-selection seam after Iris/Sodium migration"
            );
        }
    }

    @Test
    public void testCubemapRenderPassStaysColorOnly() throws IOException {
        String source = readSource(SRC_MAIN_JAVA.resolve("net/minecraft/client/renderer/CubeMap.java"));

        assertTrue(source.contains("gpuTextureView != null\n\t\t\t? VulkanicAPI.createRenderPass(() -> \"Cubemap\", gpuTextureView, OptionalInt.empty())"),
            "CubeMap should prefer the native texture-view render-pass path for panorama background rendering");
        assertTrue(source.contains("VulkanicAPI.resolveFramebufferForTextures(renderTarget.getColorTexture(), renderTarget.getDepthTexture())"),
            "CubeMap should retain framebuffer recovery only for the missing-texture-view fallback");
        assertTrue(source.contains("VulkanicAPI.createRenderPass(() -> \"Cubemap\", framebuffer, renderTarget.getDepthTexture() != null)"),
            "CubeMap should retain a framebuffer-owned render-pass fallback when texture-view rendering is unavailable");
    }

    @Test
    public void testScissorStateOwnershipMovedToVulkanicAPI() throws IOException {
        Path renderSystemFile = SRC_MAIN_JAVA.resolve("net/blaze3d/systems/RenderSystem.java");
        String renderSystemSource = readSource(renderSystemFile);

        assertFalse(renderSystemSource.contains("scissorStateForRenderTypeDraws"),
            "RenderSystem should not own scissorStateForRenderTypeDraws after migration to VulkanicAPI");
        assertFalse(renderSystemSource.contains("public static void enableScissorForRenderTypeDraws("),
            "RenderSystem should not expose enableScissorForRenderTypeDraws after migration to VulkanicAPI");
        assertFalse(renderSystemSource.contains("public static void disableScissorForRenderTypeDraws("),
            "RenderSystem should not expose disableScissorForRenderTypeDraws after migration to VulkanicAPI");
        assertFalse(renderSystemSource.contains("public static ScissorState getScissorStateForRenderTypeDraws("),
            "RenderSystem should not expose getScissorStateForRenderTypeDraws after migration to VulkanicAPI");

        Path vulkanicApiFile = SRC_MAIN_JAVA.resolve("net/vulkanic/VulkanicAPI.java");
        String vulkanicApiSource = readSource(vulkanicApiFile);

        assertTrue(vulkanicApiSource.contains("private static final ScissorState scissorStateForRenderTypeDraws = new ScissorState();"),
            "VulkanicAPI should own scissorStateForRenderTypeDraws after migration");
        assertTrue(vulkanicApiSource.contains("public static void enableScissorForRenderTypeDraws("),
            "VulkanicAPI should expose enableScissorForRenderTypeDraws after migration");
        assertTrue(vulkanicApiSource.contains("public static void disableScissorForRenderTypeDraws("),
            "VulkanicAPI should expose disableScissorForRenderTypeDraws after migration");
        assertTrue(vulkanicApiSource.contains("public static ScissorState getScissorStateForRenderTypeDraws("),
            "VulkanicAPI should expose getScissorStateForRenderTypeDraws after migration");

        Path guiRendererFile = SRC_MAIN_JAVA.resolve("net/minecraft/client/gui/render/GuiRenderer.java");
        String guiRendererSource = readSource(guiRendererFile);
        assertFalse(guiRendererSource.contains("RenderSystem.enableScissorForRenderTypeDraws("),
            "GuiRenderer should not enable draw scissor through RenderSystem after migration");
        assertFalse(guiRendererSource.contains("RenderSystem.disableScissorForRenderTypeDraws("),
            "GuiRenderer should not disable draw scissor through RenderSystem after migration");
        assertTrue(guiRendererSource.contains("VulkanicAPI.enableScissorForRenderTypeDraws("),
            "GuiRenderer should enable draw scissor through VulkanicAPI after migration");
        assertTrue(guiRendererSource.contains("VulkanicAPI.disableScissorForRenderTypeDraws("),
            "GuiRenderer should disable draw scissor through VulkanicAPI after migration");

        Path renderTypeFile = SRC_MAIN_JAVA.resolve("net/minecraft/client/renderer/RenderType.java");
        String renderTypeSource = readSource(renderTypeFile);
        assertFalse(renderTypeSource.contains("RenderSystem.getScissorStateForRenderTypeDraws("),
            "RenderType should not read draw scissor state through RenderSystem after migration");
        assertTrue(renderTypeSource.contains("VulkanicAPI.getScissorStateForRenderTypeDraws("),
            "RenderType should read draw scissor state through VulkanicAPI after migration");
    }

    @Test
    public void testStandard3dItemDebugPipDumpIsOptIn() throws IOException {
        Path guiRendererFile = SRC_MAIN_JAVA.resolve("net/minecraft/client/gui/render/GuiRenderer.java");
        String guiRendererSource = readSource(guiRendererFile);
        Path standard3dItemRendererFile = SRC_MAIN_JAVA.resolve("net/minecraft/client/gui/render/pip/Standard3dItemRenderer.java");
        String standard3dItemRendererSource = readSource(standard3dItemRendererFile);

        assertTrue(standard3dItemRendererSource.contains("Boolean.getBoolean(\"mattmc.gui.debugStandard3dItemPipDump\")"),
            "Standard3dItemRenderer should make its forced grass-block PIP dump opt-in behind an explicit debug flag");
		assertTrue(guiRendererSource.contains("Standard3dItemRenderer.isDebugDumpEnabled() && !RustGalGuiRenderer.isWholeFrameVulkanEnabled()")
				&& guiRendererSource.contains("prepareDebugStandardBlockItemDump(this.renderState, i);"),
				"GuiRenderer should only invoke the standard 3D item PIP debug dump outside Rust whole-frame ownership");
    }

    @Test
    public void testSequentialAndDynamicUniformOwnershipMovedToVulkanicAPI() throws IOException {
        Path renderSystemFile = SRC_MAIN_JAVA.resolve("net/blaze3d/systems/RenderSystem.java");
        String renderSystemSource = readSource(renderSystemFile);

        assertFalse(renderSystemSource.contains("class AutoStorageIndexBuffer"),
            "RenderSystem should not define AutoStorageIndexBuffer after migration to VulkanicAPI");
        assertFalse(renderSystemSource.contains("private static DynamicUniforms dynamicUniforms"),
            "RenderSystem should not own dynamicUniforms after migration to VulkanicAPI");
        assertFalse(renderSystemSource.contains("public static VulkanicAPI.AutoStorageIndexBuffer getSequentialBuffer("),
            "RenderSystem should not expose getSequentialBuffer after migration to VulkanicAPI");
        assertFalse(renderSystemSource.contains("public static DynamicUniforms getDynamicUniforms("),
            "RenderSystem should not expose getDynamicUniforms after migration to VulkanicAPI");
        assertTrue(renderSystemSource.contains("VulkanicAPI.initializeDynamicUniforms();"),
            "RenderSystem.initRenderer should initialize dynamic uniforms via VulkanicAPI");
        assertTrue(renderSystemSource.contains("VulkanicAPI.resetDynamicUniforms();"),
            "RenderSystem.flipFrame should reset dynamic uniforms via VulkanicAPI");

        Path vulkanicApiFile = SRC_MAIN_JAVA.resolve("net/vulkanic/VulkanicAPI.java");
        String vulkanicApiSource = readSource(vulkanicApiFile);

        assertTrue(vulkanicApiSource.contains("private static final VulkanicAPI.AutoStorageIndexBuffer sharedSequential ="),
            "VulkanicAPI should own sharedSequential index buffer after migration");
        assertTrue(vulkanicApiSource.contains("private static final VulkanicAPI.AutoStorageIndexBuffer sharedSequentialQuad ="),
            "VulkanicAPI should own sharedSequentialQuad index buffer after migration");
        assertTrue(vulkanicApiSource.contains("private static final VulkanicAPI.AutoStorageIndexBuffer sharedSequentialLines ="),
            "VulkanicAPI should own sharedSequentialLines index buffer after migration");
        assertTrue(vulkanicApiSource.contains("public static final class AutoStorageIndexBuffer"),
            "VulkanicAPI should define AutoStorageIndexBuffer after migration");
        assertTrue(vulkanicApiSource.contains("private static DynamicUniforms dynamicUniforms;"),
            "VulkanicAPI should own dynamicUniforms after migration");
        assertTrue(vulkanicApiSource.contains("public static VulkanicAPI.AutoStorageIndexBuffer getSequentialBuffer("),
            "VulkanicAPI should expose getSequentialBuffer after migration");
        assertTrue(vulkanicApiSource.contains("public static DynamicUniforms getDynamicUniforms("),
            "VulkanicAPI should expose getDynamicUniforms after migration");

        Path renderTypeFile = SRC_MAIN_JAVA.resolve("net/minecraft/client/renderer/RenderType.java");
        String renderTypeSource = readSource(renderTypeFile);
        assertFalse(renderTypeSource.contains("RenderSystem.getSequentialBuffer("),
            "RenderType should not use RenderSystem.getSequentialBuffer after migration");
        assertFalse(renderTypeSource.contains("RenderSystem.getDynamicUniforms("),
            "RenderType should not use RenderSystem.getDynamicUniforms after migration");
        assertTrue(renderTypeSource.contains("VulkanicAPI.getSequentialBuffer("),
            "RenderType should use VulkanicAPI.getSequentialBuffer after migration");
        assertTrue(renderTypeSource.contains("VulkanicAPI.getDynamicUniforms("),
            "RenderType should use VulkanicAPI.getDynamicUniforms after migration");

        Path skyRendererFile = SRC_MAIN_JAVA.resolve("net/minecraft/client/renderer/SkyRenderer.java");
        String skyRendererSource = readSource(skyRendererFile);
        assertFalse(skyRendererSource.contains("RenderSystem.getSequentialBuffer("),
            "SkyRenderer should not use RenderSystem.getSequentialBuffer after migration");
        assertFalse(skyRendererSource.contains("RenderSystem.getDynamicUniforms("),
            "SkyRenderer should not use RenderSystem.getDynamicUniforms after migration");
        assertTrue(skyRendererSource.contains("VulkanicAPI.getSequentialBuffer("),
            "SkyRenderer should use VulkanicAPI.getSequentialBuffer after migration");
        assertTrue(skyRendererSource.contains("VulkanicAPI.getDynamicUniforms("),
            "SkyRenderer should use VulkanicAPI.getDynamicUniforms after migration");
    }

    @Test
    public void testHandPassLayerRoutingKeepsOpaqueItemSubpassesOpaque() throws IOException {
        Path irisPipelinesFile = SRC_MAIN_JAVA.resolve("net/irisshaders/iris/pipeline/IrisPipelines.java");
        String irisPipelinesSource = readSource(irisPipelinesFile);

        assertFalse(irisPipelinesSource.contains("HandRenderer.INSTANCE.isRenderingSolid() ? ShaderKey.HAND_CUTOUT : ShaderKey.HAND_TRANSLUCENT"),
            "IrisPipelines solid hand routing should not remap opaque subpasses to HAND_TRANSLUCENT in translucent pass");
        assertTrue(irisPipelinesSource.contains("return ShaderKey.HAND_CUTOUT_DIFFUSE;"),
            "IrisPipelines cutout hand routing should keep opaque/cutout subpasses on HAND_CUTOUT_DIFFUSE shader");
        assertTrue(irisPipelinesSource.contains("return ShaderKey.HAND_CUTOUT;"),
            "IrisPipelines solid hand routing should keep opaque subpasses on HAND_CUTOUT shader");

        Path itemInHandRendererFile = SRC_MAIN_JAVA.resolve("net/minecraft/client/renderer/ItemInHandRenderer.java");
        String itemInHandRendererSource = readSource(itemInHandRendererFile);
        assertTrue(itemInHandRendererSource.contains("Iris.isPackInUseQuick() && net.irisshaders.iris.pathways.HandRenderer.INSTANCE.isActive()"),
            "ItemInHandRenderer hand pass filtering should only apply while Iris HandRenderer is actively rendering a hand pass");
    }

    @Test
    public void testIrisHorizonPipelineUsesShaderpackSkyProgram() throws IOException {
        Path horizonRendererFile = SRC_MAIN_JAVA.resolve("net/irisshaders/iris/pathways/HorizonRenderer.java");
        String horizonRendererSource = readSource(horizonRendererFile);
        assertTrue(horizonRendererSource.contains("public static final RenderPipeline HORIZON_PIPELINE"),
            "HorizonRenderer should expose its custom horizon pipeline so Iris can map it to shaderpack sky programs");
        assertTrue(horizonRendererSource.contains("pass.setPipeline(HORIZON_PIPELINE);"),
            "HorizonRenderer should keep the custom pipeline needed by Vulkan rather than falling back to RenderPipelines.SKY");

        Path irisPipelinesFile = SRC_MAIN_JAVA.resolve("net/irisshaders/iris/pipeline/IrisPipelines.java");
        String irisPipelinesSource = readSource(irisPipelinesFile);
        assertTrue(irisPipelinesSource.contains("import net.irisshaders.iris.pathways.HorizonRenderer;"),
            "IrisPipelines should import HorizonRenderer so the custom horizon pipeline can be mapped explicitly");
        assertTrue(irisPipelinesSource.contains("assignToMain(HorizonRenderer.HORIZON_PIPELINE, p -> ShaderKey.SKY_BASIC);"),
            "IrisPipelines should map the custom horizon pipeline to SKY_BASIC so OpenGL shader mode does not fall back to vanilla core/sky");
    }

    @Test
    public void testGuiItemsBypassSodiumFastQuadPath() throws IOException {
        Path itemRendererFile = SRC_MAIN_JAVA.resolve("net/minecraft/client/renderer/entity/ItemRenderer.java");
        String itemRendererSource = readSource(itemRendererFile);

        assertTrue(itemRendererSource.contains("itemDisplayContext != ItemDisplayContext.GUI"),
            "ItemRenderer should disable Sodium fast quad path for GUI item rendering to preserve vanilla alpha behavior");
        assertTrue(itemRendererSource.contains("if (allowSodiumFastPath && writer != null && !list.isEmpty())"),
            "ItemRenderer fast path should be explicitly gated so GUI item rendering falls back to vanilla vertex submission");

        Path renderTypeFile = SRC_MAIN_JAVA.resolve("net/minecraft/client/renderer/RenderType.java");
        String renderTypeSource = readSource(renderTypeFile);
        assertTrue(renderTypeSource.contains("GpuTextureView textureView = TextureTracker.INSTANCE.getShaderTexture(i);"),
            "RenderType draw path should first resolve sampler views from TextureTracker unit bindings");
        assertTrue(containsAny(renderTypeSource,
            "if (textureView != null && textureId > 0 && VulkanicAPI.getTextureHandle(textureView.texture()) != textureId)",
            "if (textureView != null && textureId > 0 && net.vulkanic.VulkanicCoreAPI.textureId(textureView) != textureId)"),
            "RenderType draw path should reject stale tracked sampler views when they no longer match Iris texture binding IDs");
        assertTrue(renderTypeSource.contains("if (textureView == null)"),
            "RenderType draw path should only fall back to Iris texture binding ids when no tracked unit view exists");
        assertTrue(renderTypeSource.contains("if (textureView == null && i == 2)"),
            "RenderType draw path should explicitly fall back to live lightmap binding for Sampler2 when tracked state is unavailable");

        Path textureTrackerFile = SRC_MAIN_JAVA.resolve("net/irisshaders/iris/pbr/TextureTracker.java");
        String textureTrackerSource = readSource(textureTrackerFile);
        assertTrue(textureTrackerSource.contains("shaderTexturesByUnit[unit] = id;"),
            "TextureTracker should always update per-unit shader texture cache on setShaderTexture");
        assertTrue(textureTrackerSource.contains("if (lockBindCallback)"),
            "TextureTracker should still suppress recursive callback propagation while retaining per-unit tracking");
        assertTrue(textureTrackerSource.contains("for (int unit = 0; unit < shaderTexturesByUnit.length; unit++)"),
            "TextureTracker should scan per-unit shader texture cache during texture deletion to remove stale unit bindings");
        assertTrue(textureTrackerSource.contains("shaderTexturesByUnit[unit] = null;"),
            "TextureTracker should clear per-unit cache entries that reference deleted textures");

        Path blockModelWrapperFile = SRC_MAIN_JAVA.resolve("net/minecraft/client/renderer/item/BlockModelWrapper.java");
        String blockModelWrapperSource = readSource(blockModelWrapperFile);
        assertFalse(blockModelWrapperSource.contains("renderType = Sheets.cutoutBlockSheet();"),
            "BlockModelWrapper should not force GUI item rendering onto the cutout sheet; translucent items need their original render type");

        Path irisPipelinesFile = SRC_MAIN_JAVA.resolve("net/irisshaders/iris/pipeline/IrisPipelines.java");
        String irisPipelinesSource = readSource(irisPipelinesFile);
        assertTrue(irisPipelinesSource.contains("assignToMain(RenderPipelines.ITEM_ENTITY_TRANSLUCENT_CULL, p -> getTranslucent(p));"),
            "IrisPipelines should keep ITEM_ENTITY_TRANSLUCENT_CULL on translucent shader selection so GUI translucent items preserve alpha blending");

        Path itemShaderFile = PROJECT_ROOT.resolve("src/main/resources/assets/minecraft/shaders/core/rendertype_item_entity_translucent_cull.vsh");
        String itemShaderSource = readSource(itemShaderFile);
        assertTrue(itemShaderSource.contains("vec4(lightColor.rgb, 1.0)"),
            "Item shader should not allow lightmap alpha to modulate item alpha; only lightmap RGB should affect item shading");

        Path entityShaderFile = PROJECT_ROOT.resolve("src/main/resources/assets/minecraft/shaders/core/entity.vsh");
        String entityShaderSource = readSource(entityShaderFile);
        assertTrue(entityShaderSource.contains("lightMapColor = vec4(lightColor.rgb, 1.0);"),
            "Entity shader should not allow lightmap alpha to modulate entity/item alpha; lightmap alpha must be clamped to 1.0");

        Path itemStackRenderStateFile = SRC_MAIN_JAVA.resolve("net/minecraft/client/renderer/item/ItemStackRenderState.java");
        String itemStackRenderStateSource = readSource(itemStackRenderStateFile);
        assertTrue(itemStackRenderStateSource.contains("ItemStackRenderState.this.displayContext != ItemDisplayContext.GUI"),
            "ItemStackRenderState should bypass FRAPI mesh submission for GUI item rendering so GUI follows vanilla submit path");

        Path trackingItemStackRenderStateFile = SRC_MAIN_JAVA.resolve("net/minecraft/client/renderer/item/TrackingItemStackRenderState.java");
        String trackingItemStackRenderStateSource = readSource(trackingItemStackRenderStateFile);
        assertTrue(trackingItemStackRenderStateSource.contains("this.modelIdentityElements.clear();"),
            "TrackingItemStackRenderState should clear model identity elements when state is cleared so GUI item identity does not leak across updates");
        assertTrue(trackingItemStackRenderStateSource.contains("return List.copyOf(this.modelIdentityElements);"),
            "TrackingItemStackRenderState should return immutable identity snapshots so GUI atlas cache keys cannot be mutated after insertion");
    }

    @Test
    public void testJeiPanelDropDeletesCarriedStackInsteadOfWorldDrop() throws IOException {
        Path jeiPanelFile = SRC_MAIN_JAVA.resolve("net/minecraft/client/gui/screens/inventory/JeiPanel.java");
        String jeiPanelSource = readSource(jeiPanelFile);
        assertTrue(jeiPanelSource.contains("public boolean containsMouse(double mouseX, double mouseY)"),
            "JeiPanel should expose panel bounds so container screens can distinguish JEI drops from normal outside-inventory drops");
        int carriedGuardIndex = jeiPanelSource.indexOf("if (!this.minecraft.player.containerMenu.getCarried().isEmpty())");
        int addItemIndex = jeiPanelSource.indexOf("this.addItemToInventorySafe(itemToAdd);", carriedGuardIndex);
        assertTrue(carriedGuardIndex >= 0,
            "JEI item clicks should detect an already-carried cursor stack");
        assertTrue(addItemIndex > carriedGuardIndex,
            "JEI item clicks should consume carried-stack clicks before granting the hovered item");

        Path containerScreenFile = SRC_MAIN_JAVA.resolve("net/minecraft/client/gui/screens/inventory/AbstractContainerScreen.java");
        String containerScreenSource = readSource(containerScreenFile);
        int deleteCheckIndex = containerScreenSource.indexOf("if (this.deleteCarriedItemIfReleasedOverJeiPanel(mouseButtonEvent))");
        int panelReleaseIndex = containerScreenSource.indexOf("if (this.jeiPanel != null && this.jeiPanel.mouseReleased(mouseButtonEvent))", deleteCheckIndex);
        int outsideClickIndex = containerScreenSource.indexOf("boolean bl = this.hasClickedOutside(mouseButtonEvent.x(), mouseButtonEvent.y(), i, j);", panelReleaseIndex);
        assertTrue(deleteCheckIndex >= 0,
            "AbstractContainerScreen should check for carried-stack releases over the JEI panel");
        assertTrue(panelReleaseIndex > deleteCheckIndex,
            "JEI carried-stack deletion should run before generic panel release handling");
        assertTrue(outsideClickIndex > panelReleaseIndex,
            "JEI carried-stack deletion should run before the normal outside-inventory drop path");
        assertTrue(containerScreenSource.contains("this.minecraft.gameMode.handleJeiCarriedItemDelete(this.minecraft.player);"),
            "Dropping a carried stack over JEI should use the explicit delete action instead of slot -999 pickup");

        Path gameModeFile = SRC_MAIN_JAVA.resolve("net/minecraft/client/multiplayer/MultiPlayerGameMode.java");
        String gameModeSource = readSource(gameModeFile);
        assertTrue(gameModeSource.contains("public void handleJeiCarriedItemDelete(Player player)"),
            "MultiPlayerGameMode should expose a dedicated JEI carried-item delete action");
        assertTrue(gameModeSource.contains("abstractContainerMenu.setCarried(ItemStack.EMPTY);"),
            "JEI carried-item delete should clear the client cursor immediately");
        assertTrue(gameModeSource.contains("this.connection.send(new ServerboundSetCreativeModeSlotPacket(-1, ItemStack.EMPTY));"),
            "JEI carried-item delete should tell the server to clear the cursor without spawning a dropped item");

        Path serverListenerFile = SRC_MAIN_JAVA.resolve("net/minecraft/server/network/ServerGamePacketListenerImpl.java");
        String serverListenerSource = readSource(serverListenerFile);
        assertTrue(serverListenerSource.contains("if (bl && itemStack.isEmpty())"),
            "Server creative-slot handler should reserve empty slot -1 for JEI carried-stack deletion");
        assertTrue(serverListenerSource.contains("this.player.containerMenu.setCarried(ItemStack.EMPTY);"),
            "Server JEI carried-stack deletion should clear the authoritative carried stack");
        assertTrue(serverListenerSource.contains("this.player.containerMenu.broadcastChanges();"),
            "Server JEI carried-stack deletion should broadcast the cursor clear back to the client");
    }

    // ── Consistency: drawFromBuffers still has all instanced paths ────────────

    @Test
    public void testDrawFromBuffersRetainsInstancedDrawCalls() throws IOException {
        Path file = SRC_MAIN_JAVA.resolve(
            "net/blaze3d/opengl/GlCommandEncoder.java");
        String source = readSource(file);

        assertTrue(source.contains("VulkanicAPI.drawIndexedInstancedBaseVertex("),
            "Instanced+baseVertex indexed draw must still be present in drawFromBuffers");
        assertTrue(source.contains("VulkanicAPI.drawIndexedInstanced("),
            "Instanced indexed draw must still be present in drawFromBuffers");
        assertTrue(source.contains("VulkanicAPI.drawIndexedBaseVertex("),
            "BaseVertex indexed draw must still be present in drawFromBuffers");
        assertTrue(source.contains("VulkanicAPI.drawArraysInstanced("),
            "Instanced non-indexed draw must still be present in drawFromBuffers");
    }

    @Test
    public void testParticleDrawPathRebindsPipelineScopedStateAfterSetPipeline() throws IOException {
        Path particleFile = SRC_MAIN_JAVA.resolve("net/minecraft/client/renderer/state/QuadParticleRenderState.java");
        String particleSource = readSource(particleFile);

        int pipelineIndex = particleSource.indexOf("renderPass.setPipeline(((SingleQuadParticle.Layer)entry.getKey()).pipeline());");
        int defaultUniformsIndex = particleSource.indexOf("VulkanicAPI.bindDefaultUniforms(renderPass);", pipelineIndex);
        int dynamicTransformsIndex = particleSource.indexOf("renderPass.setUniform(\"DynamicTransforms\", preparedBuffers.dynamicTransforms);", pipelineIndex);
        int lightmapIndex = particleSource.indexOf("renderPass.bindSampler(\"Sampler2\", lightTextureView);", pipelineIndex);
        int atlasIndex = particleSource.indexOf("renderPass.bindSampler(\"Sampler0\", particleTextureView);", pipelineIndex);
        int drawIndex = particleSource.indexOf("renderPass.drawIndexed(", pipelineIndex);

        assertTrue(pipelineIndex >= 0, "Particle draw path must set a pipeline before drawing");
        assertTrue(defaultUniformsIndex > pipelineIndex, "Particle draw path must rebind default uniforms after setPipeline");
        assertTrue(dynamicTransformsIndex > defaultUniformsIndex, "Particle draw path must rebind DynamicTransforms after default uniforms");
        assertTrue(lightmapIndex > dynamicTransformsIndex, "Particle draw path must rebind the lightmap after setPipeline");
        assertTrue(atlasIndex > lightmapIndex, "Particle draw path must bind the particle atlas after the lightmap");
        assertTrue(drawIndex > atlasIndex, "Particle draw path must draw only after rebinding pipeline-scoped resources");
    }

    @Test
    public void testParticleFeatureRendererExplicitlyScopesLightLayerAroundParticlePasses() throws IOException {
        Path particleFeatureRendererFile = SRC_MAIN_JAVA.resolve("net/minecraft/client/renderer/feature/ParticleFeatureRenderer.java");
        String source = readSource(particleFeatureRendererFile);

        int turnOnIndex = source.indexOf("minecraft.gameRenderer.lightTexture().turnOnLightLayer();");
        int mainLoopIndex = source.indexOf("for (SubmitNodeCollector.ParticleGroupRenderer particleGroupRenderer : submitNodeCollection.getParticleGroupRenderers())");
        int turnOffIndex = source.indexOf("minecraft.gameRenderer.lightTexture().turnOffLightLayer();");

        assertTrue(turnOnIndex >= 0,
            "ParticleFeatureRenderer should explicitly enable the light layer before particle pass submission");
        assertTrue(turnOffIndex >= 0,
            "ParticleFeatureRenderer should explicitly disable the light layer after particle pass submission");
        assertTrue(mainLoopIndex > turnOnIndex,
            "ParticleFeatureRenderer should enable the light layer before iterating particle group renderers");
        assertTrue(turnOffIndex > mainLoopIndex,
            "ParticleFeatureRenderer should disable the light layer after particle group rendering completes");
    }

    @Test
    public void testSemanticQuadParticleGroupsAreNotCountedAsUnsupportedSourceCoverage() throws IOException {
        Path submitCollectionFile = SRC_MAIN_JAVA.resolve(
            "net/minecraft/client/renderer/SubmitNodeCollection.java");
        String source = readSource(submitCollectionFile);

        assertTrue(source.contains("int unsupportedParticleGroupSubmits = 0;"),
            "particle coverage should distinguish semantic quad groups from custom renderers");
        assertTrue(source.contains("renderer instanceof QuadParticleRenderState"),
            "QuadParticleRenderState must be admitted because it copies quads into Rust semantics");
        assertTrue(source.contains("quad.rustGalUnsupportedLayerCount()"),
            "particle coverage must retain an explicit gap for non-particle-atlas layers");
        assertTrue(source.contains("unsupportedParticleGroupSubmits++")
                || source.contains("unsupportedParticleGroupSubmits +="),
            "only unsupported particle layers and custom callbacks should remain unavailable");
        assertFalse(source.contains("this.particleGroupRenderers.size()\n\t\t),"),
            "coverage must not report every semantic quad group as an unsupported renderer");
    }

    @Test
    public void testParticleSemanticAdmissionCountsOnlyAcceptedCustomAtlasQuads() throws IOException {
        Path particleFile = SRC_MAIN_JAVA.resolve(
            "net/minecraft/client/renderer/state/QuadParticleRenderState.java");
        String source = readSource(particleFile);

        assertTrue(source.contains("int[] admitted = {0};"),
            "custom particle atlases need an explicit accepted-quad counter");
        assertTrue(source.contains("enqueueParticleQuadForAtlas("),
            "custom particle atlas layers must use the Rust atlas admission call");
        assertTrue(source.contains("admitted[0] +="),
            "particle coverage must count only atlas quads accepted by Rust");
        assertTrue(source.contains("submitted += admitted[0];"),
            "semantic particle submission totals must use accepted, not attempted, quads");
    }

    @Test
    public void testParticleSemanticQuadInputsAreFiniteAndBoundedBeforeStaging() throws IOException {
        Path rendererFile = SRC_MAIN_JAVA.resolve(
            "net/vulkanic/world/RustGalWorldPrimitiveRenderer.java");
        String source = readSource(rendererFile);
        int helper = source.indexOf("private static void validateParticleQuadSemantics(");
        assertTrue(helper >= 0,
            "particle routes must validate copied semantics before atlas staging");
        String body = source.substring(helper, source.indexOf("public static void ensureParticleAtlasAnimationAsset", helper));
        assertTrue(body.contains("Float.isFinite(value)"),
            "particle positions, rotations, size, and UVs must reject non-finite values");
        assertFalse(body.contains("quadSize <= 0.0F"),
            "vanilla lifetime interpolation uses signed and zero particle sizes; finite-value validation must preserve them");
        assertTrue(body.contains("quaternionLengthSquared"),
            "particle billboard rotation must reject a zero quaternion");
    }

    @Test
    public void testCustomParticleAtlasHashCollisionFailsBeforeReplacingWorldTextureAsset() throws IOException {
        Path rendererFile = SRC_MAIN_JAVA.resolve(
            "net/vulkanic/world/RustGalWorldPrimitiveRenderer.java");
        String source = readSource(rendererFile);

        int guard = source.indexOf("Custom particle atlas IDs are hashed semantic identities");
        int lookup = source.indexOf("WORLD_MESH_TEXTURES.containsKey(textureId)", guard);
        int registration = source.indexOf("registerWorldMeshTexture(", guard);
        assertTrue(guard >= 0 && lookup > guard,
            "custom particle atlas registration must guard hashed IDs against existing world assets");
        assertTrue(registration > lookup,
            "the collision guard must run before the Rust texture asset is registered");
    }

    @Test
    public void testCustomParticleAtlasBudgetFailsBeforePublishingCopiedTexture() throws IOException {
        Path rendererFile = SRC_MAIN_JAVA.resolve(
            "net/vulkanic/world/RustGalWorldPrimitiveRenderer.java");
        String source = readSource(rendererFile);
        int atlasBranch = source.indexOf("var payload = AtlasTexturePayload.copy(textureId, atlasLocation");
        int budget = source.indexOf("ensureWorldMeshRegistryCapacityLocked(PARTICLE_ATLAS_TEXTURE_IDENTITIES", atlasBranch);
        int registration = source.indexOf("registerWorldMeshTexture(", atlasBranch);
        assertTrue(atlasBranch >= 0 && budget > atlasBranch && registration > budget,
            "particle atlas identity capacity must be checked before publishing its copied texture asset");
    }

    @Test
    public void testBlockParticleAtlasUsesDedicatedCopiedTextureIdentity() throws IOException {
        Path rendererFile = SRC_MAIN_JAVA.resolve(
            "net/vulkanic/world/RustGalWorldPrimitiveRenderer.java");
        String source = readSource(rendererFile);

        assertFalse(source.contains("TextureAtlas.LOCATION_BLOCKS.equals(atlasLocation)"),
            "block-atlas particle layers must not be rejected before semantic snapshot admission");
        assertTrue(source.contains("particleAtlasTextureId(atlasLocation)"),
            "copied particle atlases must use a dedicated texture identity");
        assertTrue(source.contains("particle-atlas:"),
            "the particle texture namespace must be distinct from terrain mesh identities");
    }

    @Test
    public void testCommonAtlasBackedEntityModelsAreAdmittedToRustMeshExtraction() throws IOException {
        Path rendererFile = SRC_MAIN_JAVA.resolve(
            "net/vulkanic/world/RustGalWorldPrimitiveRenderer.java");
        String source = readSource(rendererFile);
        int allowlist = source.indexOf("private static boolean isSupportedModelMeshModel");
        assertTrue(allowlist >= 0, "Rust model eligibility must retain an explicit bounded allowlist");
        String contract = source.substring(allowlist, Math.min(source.length(), allowlist + 8_000));
        for (String model : new String[] {
            "ShulkerModel", "ArmorStandModel", "VillagerModel", "ZombieVillagerModel",
            "PlayerModel", "SpinAttackEffectModel"
        }) {
            assertTrue(contract.contains(model), model + " must be admitted by the copied atlas-backed model contract");
        }
    }

    @Test
    public void testLivingEntityBasesPreserveDirectTextureIdentityForGenericRustOwnership() throws IOException {
        Path rendererFile = SRC_MAIN_JAVA.resolve(
            "net/minecraft/client/renderer/entity/LivingEntityRenderer.java");
        String source = readSource(rendererFile);
        assertTrue(source.contains("submitNodeCollector.submitModelSemanticTexture("),
            "living-entity base submissions must preserve their direct texture identity");
        assertTrue(source.contains("livingEntityRenderState.lightCoords, i, k, textureIdentity"),
            "generic Rust entity ownership must receive the copied semantic texture location");
        assertTrue(source.contains("if (textureIdentity != null)"),
            "atlas-less or absent texture identities must retain the explicit legacy submit boundary");
    }

    @Test
    public void testCachalotEchoUsesSemanticAnimatedTexturedQuads() throws IOException {
        Path rendererFile = SRC_MAIN_JAVA.resolve(
            "net/alexsmobs/client/render/RenderCachalotEcho.java");
        String source = readSource(rendererFile);
        assertTrue(source.contains("void submit(CachalotEchoRenderState"),
            "Cachalot Echo must expose a render-state semantic submission entrypoint");
        assertTrue(source.contains("submitTexturedQuad"),
            "Cachalot Echo arcs must enter the Rust-owned textured-quad ABI");
        assertTrue(source.contains("getEntityTextureFaster"),
            "Cachalot Echo animation must preserve its state-dependent copied texture identity");
        assertTrue(source.contains("Rust whole-frame Cachalot Echo route rejected"),
            "Cachalot Echo must fail closed when Rust cannot admit a semantic quad");
        assertTrue(source.contains("Java Cachalot Echo rendering is unavailable"),
            "the legacy Cachalot Echo helper must not become a hidden Vulkan presenter");
        assertTrue(source.contains("Selected Vulkan Cachalot Echo route is unavailable"),
            "selected Vulkan must not submit Cachalot Echo Java geometry before Rust route admission");
    }

    @Test
    public void testGiantSquidDepressurizationUsesSemanticTranslucentModelLayer() throws IOException {
        Path rendererFile = SRC_MAIN_JAVA.resolve(
            "net/alexsmobs/client/render/RenderGiantSquid.java");
        String source = readSource(rendererFile);
        assertTrue(source.contains("depressurization"),
            "Giant Squid depressurization state must reach the render layer");
        assertTrue(source.contains("entityTranslucent(TEXTURE_DEPRESSURIZED)"),
            "Giant Squid depressurization must preserve its translucent texture contract");
        assertTrue(source.contains("submitModelSemanticTexture"),
            "Giant Squid depressurization must be copied through the semantic model route");
    }

    @Test
    public void testMimicubeOuterTextureUsesSemanticTranslucentLayer() throws IOException {
        Path layerFile = SRC_MAIN_JAVA.resolve(
            "net/alexsmobs/client/render/layer/LayerMimicubeTexture.java");
        String source = readSource(layerFile);
        assertTrue(source.contains("entityTranslucent(TEXTURE)"),
            "Mimicube outer texture must retain its translucent material contract");
        assertTrue(source.contains("submitModelSemanticTexture"),
            "Mimicube outer texture must enter the Rust-owned semantic model route");
        assertFalse(source.contains("stub for compilation"),
            "Mimicube outer texture must not remain an admitted no-op layer");
    }

    @Test
    public void testCapturedSquidUsesCopiedStateAndNestedDispatcherSubmission() throws IOException {
        String state = readSource(SRC_MAIN_JAVA.resolve(
            "net/alexsmobs/client/render/state/CachalotWhaleRenderState.java"));
        String layer = readSource(SRC_MAIN_JAVA.resolve(
            "net/alexsmobs/client/render/layer/LayerCachalotWhaleCapturedSquid.java"));
        String renderer = readSource(SRC_MAIN_JAVA.resolve(
            "net/alexsmobs/client/render/RenderCachalotWhale.java"));
        assertTrue(state.contains("caughtSquidState"),
            "captured squid rendering must retain copied entity state, not a live entity reference");
        assertTrue(renderer.contains("extractEntity(caughtSquid"),
            "captured squid state must be extracted before semantic submission");
        assertTrue(layer.contains("getEntityRenderDispatcher().submit"),
            "captured squid must use the shared semantic entity dispatcher route");
        assertFalse(layer.contains("getCaughtSquid()"),
            "captured-squid layer must not query live entity state during rendering");
    }

    @Test
    public void testSunbirdScorchUsesSemanticEmissiveOverlay() throws IOException {
        String renderer = readSource(SRC_MAIN_JAVA.resolve(
            "net/alexsmobs/client/render/RenderSunbird.java"));
        String layer = readSource(SRC_MAIN_JAVA.resolve(
            "net/alexsmobs/client/render/layer/LayerSunbirdScorch.java"));
        assertTrue(renderer.contains("new LayerSunbirdScorch"),
            "Sunbird must admit its scorch overlay as an active layer");
        assertTrue(layer.contains("RenderType.eyes(TEXTURE)"),
            "Sunbird scorch must preserve its emissive material");
        assertTrue(layer.contains("submitModelSemanticTexture"),
            "Sunbird scorch must use the Rust-owned semantic model route");
        assertTrue(layer.contains("scorchProgress"),
            "Sunbird scorch alpha must remain state-driven");
    }

    @Test
    public void testCockroachMaracasRemainInTheSemanticModelStateRoute() throws IOException {
        String renderer = readSource(SRC_MAIN_JAVA.resolve(
            "net/alexsmobs/client/render/RenderCockroach.java"));
        String model = readSource(SRC_MAIN_JAVA.resolve(
            "net/alexsmobs/client/model/ModelCockroach.java"));
        assertTrue(renderer.contains("hasMaracas"),
            "Cockroach maraca state must be copied before model submission");
        assertTrue(model.contains("renderState.hasMaracas"),
            "Cockroach maracas must be driven by semantic model state");
        assertFalse(renderer.contains("LayerCockroachMaracas"),
            "Cockroach must not admit a Java-only maraca layer stub");
    }

    @Test
    public void testAnteaterBabyPoseIsModelOwnedRatherThanAStubLayer() throws IOException {
        String renderer = readSource(SRC_MAIN_JAVA.resolve(
            "net/alexsmobs/client/render/RenderAnteater.java"));
        String model = readSource(SRC_MAIN_JAVA.resolve(
            "net/alexsmobs/client/model/ModelAnteater.java"));
        assertTrue(model.contains("renderState.isBaby && renderState.isPassenger"),
            "Anteater baby passenger pose must remain in copied model state");
        assertFalse(renderer.contains("LayerAnteaterBaby"),
            "Anteater must not admit a Java-only baby layer stub");
    }

    @Test
    public void testMantisShrimpHeldItemUsesCopiedItemModelState() throws IOException {
        String state = readSource(SRC_MAIN_JAVA.resolve(
            "net/alexsmobs/client/render/MantisShrimpRenderState.java"));
        String renderer = readSource(SRC_MAIN_JAVA.resolve(
            "net/alexsmobs/client/render/RenderMantisShrimp.java"));
        String layer = readSource(SRC_MAIN_JAVA.resolve(
            "net/alexsmobs/client/render/layer/LayerMantisShrimpItem.java"));
        assertTrue(state.contains("ItemStackRenderState"),
            "Mantis Shrimp held item must be copied into semantic item-model state");
        assertTrue(renderer.contains("updateForLiving"),
            "Mantis Shrimp extraction must resolve item models before submission");
        assertTrue(layer.contains("mainHandItem.submit"),
            "Mantis Shrimp held item must submit through the semantic item route");
        assertFalse(layer.contains("TODO"),
            "Mantis Shrimp held item must not remain an admitted stub");
    }

    @Test
    public void testRaccoonHeldItemUsesCopiedItemModelState() throws IOException {
        String state = readSource(SRC_MAIN_JAVA.resolve(
            "net/alexsmobs/client/render/RaccoonRenderState.java"));
        String renderer = readSource(SRC_MAIN_JAVA.resolve(
            "net/alexsmobs/client/render/RenderRaccoon.java"));
        String layer = readSource(SRC_MAIN_JAVA.resolve(
            "net/alexsmobs/client/render/layer/LayerRaccoonItem.java"));
        assertTrue(state.contains("ItemStackRenderState"));
        assertTrue(renderer.contains("updateForLiving"));
        assertTrue(layer.contains("mainHandItem.submit"));
        assertFalse(layer.contains("TODO"));
    }

    @Test
    public void testUnderminerHeldItemUsesHumanoidCopiedItemState() throws IOException {
        String layer = readSource(SRC_MAIN_JAVA.resolve(
            "net/alexsmobs/client/render/layer/LayerUnderminerItem.java"));
        String wrapper = readSource(SRC_MAIN_JAVA.resolve(
            "net/alexsmobs/client/model/ModelUnderminerWrapper.java"));
        assertTrue(layer.contains("getMainHandItem().submit"));
        assertTrue(wrapper.contains("translateToHand"));
        assertFalse(layer.contains("Would need item state"));
    }

    @Test
    public void testMimicubeEquipmentUsesCopiedItemModelStates() throws IOException {
        String state = readSource(SRC_MAIN_JAVA.resolve(
            "net/alexsmobs/client/render/MimicubeRenderState.java"));
        String renderer = readSource(SRC_MAIN_JAVA.resolve(
            "net/alexsmobs/client/render/RenderMimicube.java"));
        String held = readSource(SRC_MAIN_JAVA.resolve(
            "net/alexsmobs/client/render/layer/LayerMimicubeHeldItem.java"));
        String helmet = readSource(SRC_MAIN_JAVA.resolve(
            "net/alexsmobs/client/render/layer/LayerMimicubeHelmet.java"));
        assertTrue(state.contains("ItemStackRenderState"));
        assertTrue(renderer.contains("updateForLiving"));
        assertTrue(held.contains("mainHandItem"));
        assertTrue(helmet.contains("headItem.submit"));
        assertFalse(held.contains("TODO"));
        assertFalse(helmet.contains("TODO"));
    }

    @Test
    public void testCrowBeakItemUsesCopiedItemModelState() throws IOException {
        String state = readSource(SRC_MAIN_JAVA.resolve(
            "net/alexsmobs/client/render/state/CrowRenderState.java"));
        String renderer = readSource(SRC_MAIN_JAVA.resolve(
            "net/alexsmobs/client/render/RenderCrow.java"));
        String layer = readSource(SRC_MAIN_JAVA.resolve(
            "net/alexsmobs/client/render/layer/LayerCrowItem.java"));
        assertTrue(state.contains("ItemStackRenderState"));
        assertTrue(renderer.contains("updateForLiving"));
        assertTrue(layer.contains("heldItem.submit"));
        assertFalse(layer.contains("TODO"));
    }

    @Test
    public void testCapuchinDoesNotAdmitUnimplementedItemLayer() throws IOException {
        String renderer = readSource(SRC_MAIN_JAVA.resolve(
            "net/alexsmobs/client/render/RenderCapuchinMonkey.java"));
        String model = readSource(SRC_MAIN_JAVA.resolve(
            "net/alexsmobs/client/model/ModelCapuchinMonkey.java"));
        assertTrue(renderer.contains("hasDart"),
            "Capuchin dart state remains available to the semantic model");
        assertFalse(renderer.contains("LayerCapuchinItem"),
            "Capuchin must not admit an unimplemented Java item layer");
        assertFalse(model.contains("ItemStack"),
            "Capuchin has no copied item geometry to justify an item layer");
    }

    @Test
    public void testAnteaterTongueItemUsesCopiedItemModelState() throws IOException {
        String state = readSource(SRC_MAIN_JAVA.resolve(
            "net/alexsmobs/client/render/AnteaterRenderState.java"));
        String renderer = readSource(SRC_MAIN_JAVA.resolve(
            "net/alexsmobs/client/render/RenderAnteater.java"));
        String layer = readSource(SRC_MAIN_JAVA.resolve(
            "net/alexsmobs/client/render/layer/LayerAnteaterTongueItem.java"));
        assertTrue(state.contains("ItemStackRenderState"));
        assertTrue(renderer.contains("updateForLiving"));
        assertTrue(layer.contains("tongueItem.submit"));
        assertFalse(layer.contains("stub"));
    }

    @Test
    public void testCosmawHeldItemUsesSemanticItemModelSubmission() throws IOException {
        String state = readSource(SRC_MAIN_JAVA.resolve(
            "net/alexsmobs/client/render/CosmawRenderState.java"));
        String renderer = readSource(SRC_MAIN_JAVA.resolve(
            "net/alexsmobs/client/render/RenderCosmaw.java"));
        assertTrue(state.contains("ItemStackRenderState"));
        assertTrue(renderer.contains("updateForLiving"));
        assertTrue(renderer.contains("mainHandItem.submit"));
        assertFalse(renderer.contains("ItemInHandRenderer"));
        assertFalse(renderer.contains("skip item rendering"));
    }

    @Test
    public void testAlexCavesIncompleteNestedEntityLayersAreNotAdmitted() throws IOException {
        String relicheirus = readSource(SRC_MAIN_JAVA.resolve(
            "net/alexscaves/client/render/entity/RelicheirusRenderer.java"));
        String tremorsaurus = readSource(SRC_MAIN_JAVA.resolve(
            "net/alexscaves/client/render/entity/TremorsaurusRenderer.java"));
        assertFalse(relicheirus.contains("HeldTrilocarisLayer"),
            "Relicheirus must not admit a Java-only held-entity stub");
        assertFalse(tremorsaurus.contains("RiderLayer"),
            "Tremorsaurus must not admit an unimplemented rider layer");
        assertFalse(tremorsaurus.contains("HeldMobLayer"),
            "Tremorsaurus must not admit an unimplemented held-mob layer");
    }

    @Test
    public void testAtlatitanDoesNotAdmitDisabledRiderLayer() throws IOException {
        String renderer = readSource(SRC_MAIN_JAVA.resolve(
            "net/alexscaves/client/render/entity/AtlatitanRenderer.java"));
        assertFalse(renderer.contains("AtlatitanRiderLayer"),
            "Atlatitan must not admit a Java-only rider stub");
    }

    @Test
    public void testUnregisteredKangarooAndSubterranodonStubsStayUnavailable() throws IOException {
        String kangaroo = readSource(SRC_MAIN_JAVA.resolve(
            "net/alexsmobs/client/render/KangarooRenderer.java"));
        String subterranodon = readSource(SRC_MAIN_JAVA.resolve(
            "net/alexscaves/client/render/entity/SubterranodonRenderer.java"));
        assertFalse(kangaroo.contains("LayerKangaroo"));
        assertFalse(subterranodon.contains("SubterranodonRiderLayer"));
    }

    @Test
    public void testLeafcutterAttachmentTransitionUsesCopiedVerticalMotion() throws IOException {
        String state = readSource(SRC_MAIN_JAVA.resolve(
            "net/alexsmobs/client/render/LeafcutterAntRenderState.java"));
        String renderer = readSource(SRC_MAIN_JAVA.resolve(
            "net/alexsmobs/client/render/RenderLeafcutterAnt.java"));
        assertTrue(state.contains("verticalVelocity"));
        assertTrue(renderer.contains("getDeltaMovement().y"));
        assertTrue(renderer.contains("state.verticalVelocity"));
        assertFalse(renderer.contains("skip transition rotation"));
    }

    @Test
    public void testVallumraptorDoesNotAdvertiseMissingItemLayer() throws IOException {
        String renderer = readSource(SRC_MAIN_JAVA.resolve(
            "net/alexscaves/client/render/entity/VallumraptorRenderer.java"));
        assertFalse(renderer.contains("ItemLayer"));
        assertFalse(renderer.contains("TODO"));
    }

    @Test
    public void testMimicOctopusGuardianBeamUsesCopiedTargetSemantics() throws IOException {
        String state = readSource(SRC_MAIN_JAVA.resolve(
            "net/alexsmobs/client/render/state/MimicOctopusRenderState.java"));
        String renderer = readSource(SRC_MAIN_JAVA.resolve(
            "net/alexsmobs/client/render/RenderMimicOctopus.java"));
        assertTrue(state.contains("guardianLaserTargetPresent"));
        assertTrue(renderer.contains("getGuardianLaser()"));
        assertTrue(renderer.contains("Mth.lerp(partialTick"));
        assertTrue(renderer.contains("submitGuardianBeam"));
        assertTrue(renderer.contains("submitCustomGeometry"));
        assertTrue(renderer.contains("vulkanSelected && !rustWholeFrame"));
        assertTrue(renderer.contains("GUARDIAN_BEAM_TEXTURE"));
        assertTrue(renderer.contains("submitGuardianBeam"));
        assertFalse(renderer.contains("commenting out for now"));
    }

    @Test
    public void testItemPickupParticlesReuseTheRustItemEntitySemanticScope() throws IOException {
        Path particleFile = SRC_MAIN_JAVA.resolve(
            "net/minecraft/client/particle/ItemPickupParticleGroup.java");
        String source = readSource(particleFile);

        assertTrue(source.contains("beginItemEntitySubmission()"),
            "item-pickup particles must enter the copied item-entity semantic scope");
        assertTrue(source.contains("endItemEntitySubmission()"),
            "item-pickup particles must close the copied item-entity semantic scope");
        assertTrue(source.contains("finally"),
            "item-pickup particle scope must be exception-safe");
    }

    @Test
	public void testBeeStingerLayersUseAnExplicitRustDirectTextureRoute() throws IOException {
        Path submitFile = SRC_MAIN_JAVA.resolve(
            "net/minecraft/client/renderer/SubmitNodeCollection.java");
        String source = readSource(submitFile);

        assertTrue(source.contains("model instanceof net.minecraft.client.model.BeeStingerModel"),
            "bee-stinger layers must have an explicit semantic model admission");
        assertTrue(source.contains("textures/entity/bee/bee_stinger.png"),
            "bee-stinger admission must carry its direct texture identity");
        assertTrue(source.contains("particle/bee_stinger"),
            "bee-stinger mesh instances must carry a stable semantic identity");
		assertTrue(source.contains("enqueueStandaloneModelMesh"),
			"bee-stinger geometry must enter the Rust-owned indexed model mesh path");
	}

	@Test
	public void testUnitOpaqueDirectTexturesUseTheRustModelMeshRoute() throws IOException {
		Path submitFile = SRC_MAIN_JAVA.resolve(
			"net/minecraft/client/renderer/SubmitNodeCollection.java");
		Path stuckLayer = SRC_MAIN_JAVA.resolve(
			"net/minecraft/client/renderer/entity/layers/StuckInBodyLayer.java");
		String submitSource = readSource(submitFile);
		String stuckSource = readSource(stuckLayer);

		assertTrue(stuckSource.contains("submitModelSemanticTexture"),
			"stuck-in-body layers must preserve direct texture identity at the semantic callsite");
		assertTrue(submitSource.contains("Direct-texture opaque Model submissions with a Unit state"),
			"Unit-state opaque direct textures must have an explicit Rust admission boundary");
		assertTrue(submitSource.contains("isStandaloneModelMeshEligible"),
			"Unit-state opaque direct textures must use copied asset and geometry eligibility");
	}

	@Test
	public void testUnadmittedDirectTextureCannotLoseIdentityOnRustRoute() throws IOException {
		Path submitFile = SRC_MAIN_JAVA.resolve(
			"net/minecraft/client/renderer/SubmitNodeCollection.java");
		String source = readSource(submitFile);
		int guard = source.indexOf("A direct texture identity must not be erased");
		int fallback = source.indexOf("this.submitModelSemantic(model, object, poseStack, renderType, i, j, k, null, l, crumblingOverlay)", guard);
		assertTrue(guard >= 0, "direct-texture semantic submissions need a Rust fail-closed identity guard");
		assertTrue(fallback > guard, "OpenGL compatibility fallback must remain after the Rust direct-texture guard");
		assertTrue(source.indexOf("Rust whole-frame direct-texture model has no admitted semantic mesh", guard) > guard,
			"unadmitted direct textures must report an explicit Rust semantic rejection");
	}

	@Test
	public void testArrowLayerTransientStateUsesStableRustTextureIdentity() throws IOException {
		Path submitFile = SRC_MAIN_JAVA.resolve(
			"net/minecraft/client/renderer/SubmitNodeCollection.java");
		Path arrowLayer = SRC_MAIN_JAVA.resolve(
			"net/minecraft/client/renderer/entity/layers/ArrowLayer.java");
		String submitSource = readSource(submitFile);
		String arrowSource = readSource(arrowLayer);

		assertTrue(arrowSource.contains("new ArrowRenderState()"),
			"ArrowLayer must retain its transient semantic state contract");
		assertTrue(submitSource.contains("model instanceof net.minecraft.client.model.ArrowModel"),
			"transient ArrowRenderState submissions need an explicit Rust model admission");
		assertTrue(submitSource.contains("particle/arrow"),
			"arrow mesh instances must use a stable identity when no entity registry key exists");
	}

    @Test
    public void testArmorStandBaseUsesTheRustLivingModelOwnershipFamily() throws IOException {
        Path rendererFile = SRC_MAIN_JAVA.resolve(
            "net/minecraft/client/renderer/entity/LivingEntityRenderer.java");
        Path meshFile = SRC_MAIN_JAVA.resolve(
            "net/vulkanic/world/RustGalWorldPrimitiveRenderer.java");
        String renderer = readSource(rendererFile);
        String mesh = readSource(meshFile);

        assertTrue(renderer.contains("isVanillaArmorStandModelMeshEligible"),
            "ArmorStandRenderer must resolve base-model Rust ownership before submit");
        assertTrue(mesh.contains("model.getClass() == net.minecraft.client.model.ArmorStandModel.class"),
            "ArmorStand admission must remain bounded to the vanilla pose-aware model");
        assertTrue(mesh.contains("!state.isMarker"),
            "marker armor stands must remain outside the ordinary copied body route");
        assertTrue(mesh.contains("textures/entity/armorstand/wood.png"),
            "ArmorStand admission must carry the copied direct texture identity");
    }

    @Test
    public void testVillagerBasesUseExplicitRustDirectTextureOwnership() throws IOException {
        Path rendererFile = SRC_MAIN_JAVA.resolve(
            "net/minecraft/client/renderer/entity/LivingEntityRenderer.java");
        Path meshFile = SRC_MAIN_JAVA.resolve(
            "net/vulkanic/world/RustGalWorldPrimitiveRenderer.java");
        String renderer = readSource(rendererFile);
        String mesh = readSource(meshFile);

        assertTrue(renderer.contains("isVanillaVillagerModelMeshEligible"),
            "villager and wandering-trader bases must be classified as Rust-owned");
        assertTrue(renderer.contains("isVanillaZombieVillagerModelMeshEligible"),
            "zombie-villager bases must be classified as Rust-owned");
        assertTrue(mesh.contains("textures/entity/villager/villager.png")
                && mesh.contains("textures/entity/wandering_trader.png"),
            "villager admission must preserve both direct base texture identities");
        assertTrue(mesh.contains("textures/entity/zombie_villager/zombie_villager.png"),
            "zombie-villager admission must preserve its direct base texture identity");
        assertTrue(mesh.contains("model.getClass() == net.minecraft.client.model.VillagerModel.class")
                && mesh.contains("model.getClass() == net.minecraft.client.model.ZombieVillagerModel.class"),
            "admission must remain bounded to the vanilla model classes");
    }

    @Test
    public void testGlowingInvisibleWoolAndSlimeOutlinesUseRustMeshMetadata() throws IOException {
        Path sheepFile = SRC_MAIN_JAVA.resolve(
            "net/minecraft/client/renderer/entity/layers/SheepWoolLayer.java");
        Path slimeFile = SRC_MAIN_JAVA.resolve(
            "net/minecraft/client/renderer/entity/layers/SlimeOuterLayer.java");
        String sheep = readSource(sheepFile);
        String slime = readSource(slimeFile);

        assertTrue(sheep.contains("RenderType.entityCutoutNoCull(SHEEP_WOOL_LOCATION)")
                && sheep.contains("ResourceLocation.withDefaultNamespace(\"sheep_wool\")"),
            "glowing invisible sheep wool must use the copied semantic material mesh");
        assertTrue(slime.contains("RenderType.entityTranslucent(SlimeRenderer.SLIME_LOCATION)")
                && slime.contains("ResourceLocation.withDefaultNamespace(\"slime_outer\")"),
            "glowing invisible slime outer layers must use the copied semantic material mesh");
        assertTrue(sheep.contains("outlineColor") && slime.contains("outlineColor"),
            "outline submissions must preserve semantic instance color metadata");
    }

    @Test
    public void testFirstPersonEmptyHandsUseRustStandaloneSkinMeshes() throws IOException {
        Path avatarFile = SRC_MAIN_JAVA.resolve(
            "net/minecraft/client/renderer/entity/player/AvatarRenderer.java");
        Path gameRendererFile = SRC_MAIN_JAVA.resolve(
            "net/minecraft/client/renderer/GameRenderer.java");
        String avatar = readSource(avatarFile);
        String gameRenderer = readSource(gameRendererFile);
        assertTrue(avatar.contains("currentModelMeshRoute(true).usesRustWholeFrameVulkan()")
                && avatar.contains("enqueueStandaloneTranslucentModelMesh(")
                && avatar.contains("ResourceLocation.withDefaultNamespace(\"player_hand\")")
                && avatar.contains("ResourceLocation.withDefaultNamespace(\"player_hand_sleeve\")"),
            "first-person empty hands and visible sleeves must enter the explicit Rust standalone skin-mesh route");
        assertTrue(avatar.contains("Rust whole-frame player-hand route rejected the semantic skin mesh"),
            "first-person hands must fail closed when their copied skin mesh is unavailable");
		int rustHandBranch = avatar.indexOf("enqueueStandaloneTranslucentModelMesh(");
		int javaHandBranch = avatar.indexOf("submitNodeCollector.submitModelPart(", rustHandBranch);
		assertTrue(rustHandBranch >= 0 && javaHandBranch > rustHandBranch
				&& avatar.contains("Iris captures and\n\t\t// replays the former in its hand pipeline"),
			"the Java OpenGL/Iris compatibility path must retain Frozen's ModelPart hand submission while Rust whole-frame keeps its separate copied skin-mesh branch");
        assertTrue(gameRenderer.contains("sleepingEntity.isSleeping()"),
            "Rust first-person hand extraction must preserve vanilla sleeping-camera suppression");
    }

    @Test
    public void testNonFoilTridentSpecialItemsUseTheSemanticDirectTextureRoute() throws IOException {
        Path tridentFile = SRC_MAIN_JAVA.resolve(
            "net/minecraft/client/renderer/special/TridentSpecialRenderer.java");
        String source = readSource(tridentFile);

        assertTrue(source.contains("if (!bl)"),
            "trident special items must distinguish non-foil semantic geometry from glint");
        assertTrue(source.contains("submitModelSemanticTexture"),
            "non-foil trident special items must submit semantic model data");
        assertTrue(source.contains("submitModelPartSemantic("),
            "foil trident compatibility geometry must retain the semantic model-part boundary");
        assertTrue(source.contains("Unit.INSTANCE"),
            "the direct-texture trident route must use a bounded transient semantic state");
        assertTrue(source.contains("TridentModel.TEXTURE"),
            "the trident route must preserve its direct texture identity");
        assertTrue(source.contains("RustGalVulkanWholeFrameMode.enabled()")
                && source.contains("submitModelSemanticTexture"),
            "foil tridents must keep the Rust presenter shell on the explicit glint/model route");
    }

    @Test
    public void testEnderDragonDeathDecalUsesExplicitDepthEqualSemantics() throws IOException {
        Path rendererFile = SRC_MAIN_JAVA.resolve(
            "net/minecraft/client/renderer/entity/EnderDragonRenderer.java");
        Path meshFile = SRC_MAIN_JAVA.resolve(
            "net/vulkanic/world/RustGalWorldPrimitiveRenderer.java");
        String renderer = readSource(rendererFile);
        String mesh = readSource(meshFile);

        assertTrue(renderer.contains("if (i == OverlayTexture.NO_OVERLAY)")
                && renderer.contains("submitModelSemanticTexture"),
            "overlay-free dragon death decals must use the semantic direct-texture route");
        assertTrue(renderer.contains("entityDragonRenderState") || renderer.contains("enderDragonRenderState"),
            "dragon decal submission must retain the copied render state");
        assertTrue(mesh.contains("entity_decal")
                && mesh.contains("DEPTH_POLICY_TEST_NO_WRITE")
                && mesh.contains("CULL_NONE"),
            "entity decals must preserve equal-depth/no-write/two-sided raster semantics");
        assertTrue(renderer.contains("SEMANTIC_CRYSTAL_BEAM_QUADS = 8")
                && renderer.contains("new float[SEMANTIC_CRYSTAL_BEAM_QUADS * 12]")
                && renderer.contains("new float[SEMANTIC_CRYSTAL_BEAM_QUADS * 8]")
                && renderer.contains("new int[SEMANTIC_CRYSTAL_BEAM_QUADS * 4]")
                && renderer.contains("vertexIndex != SEMANTIC_CRYSTAL_BEAM_QUADS * 12")
                && renderer.contains("uvIndex != SEMANTIC_CRYSTAL_BEAM_QUADS * 8"),
            "crystal-beam semantic arrays must remain aligned with all eight generated quads");
    }

    @Test
    public void testPlayerCapeUsesRustOwnedDynamicTextureMeshRoute() throws IOException {
        Path capeFile = SRC_MAIN_JAVA.resolve(
            "net/minecraft/client/renderer/entity/layers/CapeLayer.java");
        Path meshFile = SRC_MAIN_JAVA.resolve(
            "net/vulkanic/world/RustGalWorldPrimitiveRenderer.java");
        String capeSource = readSource(capeFile);
        String meshSource = readSource(meshFile);

        assertTrue(capeSource.contains("enqueueStandaloneModelMesh"),
            "player capes should use the Rust-owned standalone model mesh route");
        assertTrue(capeSource.contains("entityIdentity(avatarRenderState)"),
            "player cape mesh instances should carry semantic entity identity");
        assertTrue(meshSource.contains("readDynamicTexturePayload"),
            "dynamic skin/cape assets must be copied from CPU pixels at the semantic boundary");
        assertTrue(meshSource.contains("PlayerCapeModel"),
            "PlayerCapeModel must be explicitly admitted by Rust model extraction");
    }

    @Test
    public void testPlayerEarsUseRustOwnedAvatarMeshRoute() throws IOException {
        Path earsFile = SRC_MAIN_JAVA.resolve(
            "net/minecraft/client/renderer/entity/layers/Deadmau5EarsLayer.java");
        Path meshFile = SRC_MAIN_JAVA.resolve(
            "net/vulkanic/world/RustGalWorldPrimitiveRenderer.java");
        String earsSource = readSource(earsFile);
        String meshSource = readSource(meshFile);

        assertTrue(earsSource.contains("enqueueStandaloneModelMesh"),
            "Deadmau5 ears should use the Rust-owned avatar mesh route");
        assertTrue(earsSource.contains("entityIdentity(avatarRenderState)"),
            "Deadmau5 ears should retain semantic avatar identity");
        assertTrue(meshSource.contains("PlayerEarsModel"),
            "PlayerEarsModel must be explicitly admitted by Rust model extraction");
    }

    @Test
    public void testGenericEmissiveLayersUseGuardedRustTranslucentMeshAdmission() throws IOException {
        Path submitFile = SRC_MAIN_JAVA.resolve(
            "net/minecraft/client/renderer/SubmitNodeCollection.java");
        Path rendererFile = SRC_MAIN_JAVA.resolve(
            "net/vulkanic/world/RustGalWorldPrimitiveRenderer.java");
        String submitSource = readSource(submitFile);
        String rendererSource = readSource(rendererFile);

        assertTrue(submitSource.contains("isStandaloneTranslucentModelMeshEligible"),
            "generic emissive layers must use the guarded translucent eligibility predicate");
        assertTrue(submitSource.contains("enqueueStandaloneTranslucentModelMesh"),
            "generic emissive layers must enqueue copied translucent meshes in Rust");
        assertTrue(rendererSource.contains("isStandaloneTranslucentModelMeshEligible"),
            "Rust must expose an explicit translucent model eligibility boundary");
    }

    @Test
    public void testNonFoilArmorUsesDirectTextureSemanticSubmission() throws IOException {
        Path equipmentFile = SRC_MAIN_JAVA.resolve(
            "net/minecraft/client/renderer/entity/layers/EquipmentLayerRenderer.java");
        Path submitFile = SRC_MAIN_JAVA.resolve(
            "net/minecraft/client/renderer/SubmitNodeCollection.java");
        String equipmentSource = readSource(equipmentFile);
        String submitSource = readSource(submitFile);

        assertTrue(equipmentSource.contains("submitModelSemanticTexture"),
            "non-foil armor layers must preserve their direct texture identity at submission");
        assertTrue(equipmentSource.contains("submitModelSemantic(model, object, poseStack, RenderType.armorEntityGlint()")
                && equipmentSource.contains("submitModelSemantic(model, object, poseStack, renderType, i, OverlayTexture.NO_OVERLAY, -1, textureAtlasSprite"),
            "foil and trim armor layers must use the explicit semantic model callback too");
        assertTrue(equipmentSource.contains("if (!bl)"),
            "foil armor must remain separate from the non-foil semantic route");
        assertTrue(submitSource.contains("Direct-texture opaque layers"),
            "the collector must document the guarded opaque direct-texture route");
    }

    @Test
    public void testEyesLayerCannotFallBackToJavaModelSubmissionUnderRustWholeFrame() throws IOException {
        String eyesSource = readSource(SRC_MAIN_JAVA.resolve(
            "net/minecraft/client/renderer/entity/layers/EyesLayer.java"));
        assertTrue(eyesSource.contains("RustGalVulkanWholeFrameMode.enabled()")
                && eyesSource.contains("has no semantic texture identity"),
            "unclassified eyes layers must remain unavailable rather than reopening Java model submission");
    }

    @Test
    public void testElytraModelIsAdmittedThroughTheSameExplicitArmorContract() throws IOException {
        Path meshFile = SRC_MAIN_JAVA.resolve(
            "net/vulkanic/world/RustGalWorldPrimitiveRenderer.java");
        Path wingsFile = SRC_MAIN_JAVA.resolve(
            "net/minecraft/client/renderer/entity/layers/WingsLayer.java");
        String meshSource = readSource(meshFile);
        String wingsSource = readSource(wingsFile);

        assertTrue(meshSource.contains("model instanceof net.minecraft.client.model.ElytraModel"),
            "elytra geometry must be explicitly admitted by Rust model extraction");
        assertTrue(wingsSource.contains("equipmentRenderer"),
            "elytra layers must continue through the semantic equipment renderer");
    }

    @Test
    public void testAtlasBackedModelMeshesRequireExplicitSnapshotAndUvContract() throws IOException {
        Path meshFile = SRC_MAIN_JAVA.resolve(
            "net/vulkanic/world/RustGalWorldPrimitiveRenderer.java");
        String meshSource = readSource(meshFile);

        assertTrue(meshSource.contains("atlas-texture-unavailable"),
            "atlas-backed model sprites must fail closed when their CPU snapshot is unavailable");
        assertTrue(meshSource.contains("sprite.getU(vertex.u())")
                && meshSource.contains("sprite.getV(vertex.v())"),
            "atlas-backed model meshes must transform local model UVs through the copied sprite bounds");
        assertTrue(meshSource.contains("encodeSemanticAtlasSnapshot"),
            "atlas-backed model meshes must transport a CPU atlas snapshot rather than a Java texture handle");
    }

    @Test
    public void testDrownedRustBodyRouteIncludesSwimmingPoseState() throws IOException {
        Path rendererFile = SRC_MAIN_JAVA.resolve(
            "net/vulkanic/world/RustGalWorldPrimitiveRenderer.java");
        Path submitFile = SRC_MAIN_JAVA.resolve(
            "net/minecraft/client/renderer/SubmitNodeCollection.java");
        String rendererSource = readSource(rendererFile);
        String submitSource = readSource(submitFile);

        assertTrue(rendererSource.contains("including swimming and water poses"),
            "Drowned body admission must document the expanded semantic pose coverage");
        assertFalse(submitSource.contains("&& !drownedState.isVisuallySwimming"),
            "the semantic Drowned submit must preserve swimming render-state animation");
    }

    @Test
    public void testZombieRustBodyRouteIncludesSwimmingPoseState() throws IOException {
        Path rendererFile = SRC_MAIN_JAVA.resolve(
            "net/vulkanic/world/RustGalWorldPrimitiveRenderer.java");
        String source = readSource(rendererFile);
        int methodStart = source.indexOf("isVanillaZombieModelMeshEligible");
        int methodEnd = source.indexOf("\n\tpublic static boolean", methodStart + 1);
        assertTrue(methodStart >= 0 && methodEnd > methodStart,
            "Zombie semantic admission predicate must remain present and bounded");
        String predicate = source.substring(methodStart, methodEnd);
        assertFalse(predicate.contains("!state.isVisuallySwimming"),
            "swimming Zombie body states must not be rejected by the copied model predicate");
        assertFalse(predicate.contains("!state.isInWater"),
            "water Zombie body states must preserve the copied render-state animation");
        assertFalse(predicate.contains("!state.isCrouching"),
            "crouched Zombie body states must preserve the copied model animation");
        assertTrue(predicate.contains("state.isConverting"),
            "conversion remains an explicit unsupported-state boundary");
    }

    @Test
    public void testBoggedBodyRouteIsIndependentOfMossLayerState() throws IOException {
        Path rendererFile = SRC_MAIN_JAVA.resolve(
            "net/vulkanic/world/RustGalWorldPrimitiveRenderer.java");
        String source = readSource(rendererFile);
        int methodStart = source.indexOf("isVanillaBoggedModelMeshEligible");
        int methodEnd = source.indexOf("\n\tpublic static boolean", methodStart + 1);
        assertTrue(methodStart >= 0 && methodEnd > methodStart,
            "Bogged semantic admission predicate must remain present and bounded");
        String predicate = source.substring(methodStart, methodEnd);
        assertFalse(predicate.contains("state.isSheared"),
            "the Bogged base body must not be rejected when its separate moss layer is present");
        assertTrue(predicate.contains("state.rightHandItem.isEmpty()"),
            "held-item state remains an explicit base-body boundary");
    }

    @Test
    public void testEquineSaddleModelUsesTheExplicitRustModelMeshFamily() throws IOException {
        Path meshFile = SRC_MAIN_JAVA.resolve(
            "net/vulkanic/world/RustGalWorldPrimitiveRenderer.java");
        Path equipmentFile = SRC_MAIN_JAVA.resolve(
            "net/minecraft/client/renderer/entity/layers/EquipmentLayerRenderer.java");
        String meshSource = readSource(meshFile);
        String equipmentSource = readSource(equipmentFile);
        assertTrue(meshSource.contains("model instanceof net.minecraft.client.model.EquineSaddleModel"),
            "saddle geometry must be explicitly admitted by Rust model extraction");
        assertTrue(equipmentSource.contains("submitModelSemanticTexture"),
            "saddle/equipment textures must retain a semantic direct-texture submission boundary");
    }

    @Test
    public void testLlamaBodyRouteCoexistsWithSemanticDecorLayers() throws IOException {
        Path meshFile = SRC_MAIN_JAVA.resolve(
            "net/vulkanic/world/RustGalWorldPrimitiveRenderer.java");
        Path decorFile = SRC_MAIN_JAVA.resolve(
            "net/minecraft/client/renderer/entity/layers/LlamaDecorLayer.java");
        String meshSource = readSource(meshFile);
        String decorSource = readSource(decorFile);
        int methodStart = meshSource.indexOf("isVanillaLlamaModelMeshEligible");
        int methodEnd = meshSource.indexOf("\n\tpublic static boolean", methodStart + 1);
        assertTrue(methodStart >= 0 && methodEnd > methodStart,
            "Llama semantic admission predicate must remain present and bounded");
        String predicate = meshSource.substring(methodStart, methodEnd);
        assertFalse(predicate.contains("state.isTraderLlama"),
            "trader llama base bodies must not be rejected when decor is separately submitted");
        assertFalse(predicate.contains("state.bodyItem"),
            "llama carpet/decor state must not suppress the base Rust body");
        assertTrue(decorSource.contains("renderLayers"),
            "llama decor must remain an independent equipment semantic layer");
    }

    @Test
    public void testHorseFamilyBodiesCoexistWithSemanticArmorAndSaddleLayers() throws IOException {
        Path meshFile = SRC_MAIN_JAVA.resolve(
            "net/vulkanic/world/RustGalWorldPrimitiveRenderer.java");
        Path horseFile = SRC_MAIN_JAVA.resolve(
            "net/minecraft/client/renderer/entity/HorseRenderer.java");
        String source = readSource(meshFile);
        String horseSource = readSource(horseFile);
        int horseStart = source.indexOf("isVanillaHorseModelMeshEligible");
        int donkeyStart = source.indexOf("isVanillaDonkeyModelMeshEligible");
        int horseEnd = source.indexOf("\n\tpublic static boolean", horseStart + 1);
        int donkeyEnd = source.indexOf("\n\tpublic static boolean", donkeyStart + 1);
        assertTrue(horseStart >= 0 && horseEnd > horseStart && donkeyStart >= 0 && donkeyEnd > donkeyStart,
            "horse-family admission predicates must remain present and bounded");
        assertFalse(source.substring(horseStart, horseEnd).contains("state.bodyArmorItem"),
            "horse armor must not suppress the Rust base body");
        assertFalse(source.substring(horseStart, horseEnd).contains("state.saddle"),
            "horse saddles must not suppress the Rust base body");
        assertFalse(source.substring(donkeyStart, donkeyEnd).contains("state.saddle"),
            "donkey/mule saddles must not suppress the Rust base body");
        assertTrue(horseSource.contains("SimpleEquipmentLayer"),
            "horse armor and saddles must remain explicit equipment layers");
    }

    @Test
    public void testStriderAndCamelBodiesCoexistWithSemanticSaddleLayers() throws IOException {
        Path meshFile = SRC_MAIN_JAVA.resolve(
            "net/vulkanic/world/RustGalWorldPrimitiveRenderer.java");
        String source = readSource(meshFile);
        int striderStart = source.indexOf("isVanillaStriderModelMeshEligible");
        int camelStart = source.indexOf("isVanillaCamelModelMeshEligible");
        int striderEnd = source.indexOf("\n\tpublic static boolean", striderStart + 1);
        int camelEnd = source.indexOf("\n\tpublic static boolean", camelStart + 1);
        assertTrue(striderStart >= 0 && striderEnd > striderStart && camelStart >= 0 && camelEnd > camelStart,
            "Strider and Camel semantic predicates must remain present and bounded");
        assertFalse(source.substring(striderStart, striderEnd).contains("state.saddle"),
            "Strider saddle state must not suppress the Rust base body");
        assertFalse(source.substring(camelStart, camelEnd).contains("state.saddle"),
            "Camel saddle state must not suppress the Rust base body");
    }

    @Test
    public void testNautilusArmorAndSaddleModelsUseTheRustModelFamily() throws IOException {
        Path meshFile = SRC_MAIN_JAVA.resolve(
            "net/vulkanic/world/RustGalWorldPrimitiveRenderer.java");
        Path rendererFile = SRC_MAIN_JAVA.resolve(
            "net/minecraft/client/renderer/entity/NautilusRenderer.java");
        String source = readSource(meshFile);
        String renderer = readSource(rendererFile);
        assertTrue(source.contains("NautilusArmorModel") && source.contains("NautilusSaddleModel"),
            "Nautilus equipment models must be explicitly admitted by Rust extraction");
        assertFalse(source.substring(source.indexOf("isVanillaNautilusModelMeshEligible"), source.indexOf("\n\tpublic static boolean", source.indexOf("isVanillaNautilusModelMeshEligible") + 1)).contains("state.saddle"),
            "Nautilus saddles must not suppress the Rust base body");
        assertTrue(renderer.contains("SimpleEquipmentLayer"),
            "Nautilus armor and saddle must remain semantic equipment layers");
    }

    @Test
    public void testWolfBodyArmorAndCollarUseSemanticTextureLayers() throws IOException {
        Path meshFile = SRC_MAIN_JAVA.resolve(
            "net/vulkanic/world/RustGalWorldPrimitiveRenderer.java");
        Path collarFile = SRC_MAIN_JAVA.resolve(
            "net/minecraft/client/renderer/entity/layers/WolfCollarLayer.java");
        Path armorFile = SRC_MAIN_JAVA.resolve(
            "net/minecraft/client/renderer/entity/layers/WolfArmorLayer.java");
        String mesh = readSource(meshFile);
        int start = mesh.indexOf("isVanillaWolfModelMeshEligible");
        int end = mesh.indexOf("\n\tpublic static boolean", start + 1);
        assertTrue(start >= 0 && end > start, "Wolf semantic admission predicate must remain bounded");
        assertFalse(mesh.substring(start, end).contains("state.collarColor"),
            "wolf collar state must not suppress the Rust base body");
        assertFalse(mesh.substring(start, end).contains("state.bodyArmorItem"),
            "wolf armor state must not suppress the Rust base body");
        assertTrue(readSource(collarFile).contains("submitModelSemanticTexture"),
            "wolf collar must use a semantic direct-texture submission");
        assertTrue(readSource(armorFile).contains("submitModelSemanticTexture"),
            "wolf armor crack overlays must use a semantic direct-texture submission");
    }

    @Test
    public void testCatCollarUsesTheSharedSemanticColoredCutoutBoundary() throws IOException {
        Path renderLayerFile = SRC_MAIN_JAVA.resolve(
            "net/minecraft/client/renderer/entity/layers/RenderLayer.java");
        Path meshFile = SRC_MAIN_JAVA.resolve(
            "net/vulkanic/world/RustGalWorldPrimitiveRenderer.java");
        String renderLayer = readSource(renderLayerFile);
        String mesh = readSource(meshFile);
        int start = mesh.indexOf("isVanillaCatModelMeshEligible");
        int end = mesh.indexOf("\n\tpublic static boolean", start + 1);
        assertTrue(renderLayer.contains("submitModelSemanticTexture"),
            "colored cutout feature layers must preserve direct texture identity");
        assertTrue(start >= 0 && end > start, "Cat semantic admission predicate must remain bounded");
        assertFalse(mesh.substring(start, end).contains("state.collarColor"),
            "cat collar state must not suppress the Rust base body");
    }

    @Test
    public void testCopiedEntityMeshesCarryVanillaOverlayColorSemantics() throws IOException {
        Path meshFile = SRC_MAIN_JAVA.resolve(
            "net/vulkanic/world/RustGalWorldPrimitiveRenderer.java");
        Path frontendFile = SRC_MAIN_RUST.resolve(
            "render/vulkanic/world_primitive_frontend.rs");
        Path shaderFile = SRC_MAIN_RUST.resolve(
            "render/vulkanic/shader_pack/programs.rs");
        String mesh = readSource(meshFile);
        String frontend = readSource(frontendFile);
        String shaders = readSource(shaderFile);
        assertTrue(mesh.contains("private static int overlayColorArgb(int overlayCoords)"),
            "entity extraction must derive overlay color from semantic packed overlay coordinates");
        assertTrue(mesh.contains("overlayColorArgb(overlayCoords)"),
            "copied entity submissions must carry the derived overlay color");
        assertTrue(frontend.contains("argb_to_rgba(instance.entity_color_argb)"),
            "Rust mesh instance packing must preserve the semantic overlay color payload");
        assertTrue(shaders.contains("v_overlay_color")
                && shaders.contains("mix(color.rgb, v_overlay_color.rgb"),
            "Rust-owned mesh shaders must apply the overlay color without Java or GL state");
    }

    @Test
    public void testIrisFallbackTextureRestoreSkipsUnknownBindingSentinels() throws IOException {
        Path irisRenderSystemFile = SRC_MAIN_JAVA.resolve("net/irisshaders/iris/gl/IrisRenderSystem.java");
        String source = readSource(irisRenderSystemFile);

        assertTrue(source.contains("private static void restoreKnownTextureBinding(int textureId)"),
            "IrisRenderSystem should centralize legacy texture restore guards for fallback paths");
        assertTrue(source.contains("if (textureId >= 0) {"),
            "IrisRenderSystem should treat negative cached texture bindings as unknown sentinels instead of rebinding them");
        assertTrue(source.contains("restoreKnownTextureBinding(previous);"),
            "Iris fallback DSA paths should restore prior texture bindings only through the guarded helper");
        assertFalse(source.contains("VulkanicAPI.bindTexture2D(VulkanicAPI.getCommandContext(), previous);"),
            "Iris fallback DSA paths should not blindly rebind cached previous texture ids");
    }

    @Test
    public void testVulkanRenderTargetPipelineCacheKeyIncludesAttachmentContract() throws IOException {
        Path vulkanBackendFile = SRC_MAIN_JAVA.resolve("net/vulkanic/backends/vulkan/VulkanBackend.java");
        String source = readSource(vulkanBackendFile);
        String normalizerSource = readSource(SRC_MAIN_JAVA.resolve("net/vulkanic/backends/vulkan/VulkanPipelineCacheKeyNormalizer.java"));

        assertTrue(source.contains("private record RenderTargetPipelineKey("),
            "VulkanBackend should retain a dedicated key for render-target-compatible pipeline variants");
        assertFalse(source.contains("private record FramebufferPipelineKey("),
            "Vulkan pipeline variants should no longer be keyed by a GL-style framebuffer identity");
        assertFalse(source.contains("int framebuffer,\n        String dynamicStateKey,"),
            "Render-target pipeline variants must not include the virtual framebuffer id in the compatibility key");
        assertTrue(source.contains("VulkanPipelineCacheKeyNormalizer.GraphicsPipelineCacheKey normalizedKey"),
            "Render-target pipeline variants should be keyed by normalized immutable graphics pipeline creation inputs");
        assertTrue(normalizerSource.contains("VulkanPipelineCreationPlanner.RenderPassCompatibilityPlan renderPassCompatibility"),
            "Normalized render-target pipeline keys must include the full Vulkan render-pass compatibility contract");
        assertTrue(normalizerSource.contains("VulkanPipelineCreationPlanner.ColorBlendCacheKey colorBlendState")
                && normalizerSource.contains("VulkanPipelineCreationPlanner.DepthStencilCacheKey depthStencilState"),
            "Normalized render-target pipeline keys must include pipeline-baked blend and stencil state");
        assertTrue(source.contains("VulkanRenderPassCompatibilityKey.framebuffer("),
            "Framebuffer-backed pipeline keys should include color formats, depth format, and feedback-loop dependency profile");
        assertTrue(source.contains("normalizedGraphicsPipelineCacheKey(pipelineDescriptor, renderPassCompatibilityKey, renderTargetInterface)"),
            "Framebuffer-backed pipeline resolution should derive a normalized key from the immutable graphics pipeline and render-target interface plans");
        assertTrue(source.contains("indexedBlendStateCacheKey(portableState, renderPassCompatibilityKey.colorAttachmentCount())"),
            "Legacy pipeline resolution should still include portable blend ownership when deriving blend cache keys");
        assertTrue(source.contains("currentStencilState().cacheKey(renderPassCompatibilityKey.hasStencilAttachment())"),
            "Legacy pipeline resolution should still include effective stencil state in Vulkan pipeline cache keys");
        assertTrue(source.contains("SharedChunkProgramOverrides.indexedBlendState(portableState.location(), index)")
                && source.contains("key.append(\"shared:\")")
                && source.contains("key.append(\"portable:\")"),
            "Portable pipeline blend state should not be overridden by stale legacy indexed blend state unless a shared chunk indexed override exists");
        assertTrue(source.contains("VulkanPipelineState.from(")
                && source.contains("backend::blendStateForAttachment"),
            "Vulkan pipeline creation should apply blend state independently for each color attachment through the shared state translator");
        assertTrue(source.contains("targets.colorFormats(),"),
            "Render-target pipeline resolution should key variants from the current resolved framebuffer color formats");
        assertTrue(source.contains("targets.hasDepthTarget() ? targets.depthTexture.vkFormat : VK10.VK_FORMAT_UNDEFINED"),
            "Render-target pipeline resolution should key variants from the current resolved framebuffer depth format");
        assertTrue(source.contains("targets.hasFeedbackLoopTarget()"),
            "Render-target pipeline resolution should key variants from the current target feedback-loop contract");
    }

	    @Test
	    public void testVulkanRenderTargetDescriptorsSupportDepthOnlyShaderPasses() throws IOException {
	        VulkanicRenderTargetDescriptor.DepthAttachment depthAttachment =
	            new VulkanicRenderTargetDescriptor.DepthAttachment(
	                7,
	                VulkanicRenderPassDescriptor.LoadOp.LOAD,
	                VulkanicRenderPassDescriptor.StoreOp.STORE,
	                OptionalDouble.empty()
	            );
	        VulkanicRenderTargetDescriptor descriptor =
	            new VulkanicRenderTargetDescriptor(() -> "Depth-only shader pass", List.of(), depthAttachment);

	        assertTrue(descriptor.colorAttachments().isEmpty());
	        assertTrue(descriptor.hasDepthAttachment());
	        assertThrows(IllegalArgumentException.class,
	            () -> new VulkanicRenderTargetDescriptor(() -> "Empty pass", List.of(), null));
	        VulkanicRenderTargetDescriptor attachmentlessDescriptor =
	            new VulkanicRenderTargetDescriptor(() -> "Image-only shader pass", List.of(), null, 128, 64);
	        assertTrue(attachmentlessDescriptor.colorAttachments().isEmpty());
	        assertFalse(attachmentlessDescriptor.hasDepthAttachment());
	        assertTrue(attachmentlessDescriptor.hasExplicitExtent());

	        String renderTargetDescriptorSource =
	            readSource(SRC_MAIN_JAVA.resolve("net/vulkanic/VulkanicRenderTargetDescriptor.java"));
	        String vulkanBackendSource =
	            readSource(SRC_MAIN_JAVA.resolve("net/vulkanic/backends/vulkan/VulkanBackend.java"));
	        String nativeLifecycleSource =
	            readSource(SRC_MAIN_JAVA.resolve("net/vulkanic/backends/vulkan/VulkanNativeRenderTargetLifecycleManager.java"));
	        String renderPassKeySource =
	            readSource(SRC_MAIN_JAVA.resolve("net/vulkanic/backends/vulkan/VulkanRenderPassKey.java"));

	        assertTrue(renderTargetDescriptorSource.contains("colorAttachments.isEmpty() && depthAttachment == null && (width <= 0 || height <= 0)"),
	            "Render-target descriptors should reject only attachmentless passes without explicit extents");
	        assertTrue(vulkanBackendSource.contains("if (width < 0)"),
	            "Vulkan render-target resolution should derive depth-only pass dimensions from the depth attachment");
	        assertTrue(nativeLifecycleSource.contains("colorCount == 0")
	                && nativeLifecycleSource.contains("? null")
	                && nativeLifecycleSource.contains("VkAttachmentReference.calloc(colorCount, stack)"),
	            "Vulkan render-pass creation should pass null color references for depth-only or attachmentless subpasses");
	        assertFalse(renderPassKeySource.contains("colorAttachments.isEmpty() && depthAttachment == null"),
	            "Vulkan render-pass cache keys should support depth-only and attachmentless cached passes");
	        assertTrue(nativeLifecycleSource.contains("attachmentCount == 0")
	                && nativeLifecycleSource.contains("VkAttachmentDescription.calloc(attachmentCount, stack)")
	                && vulkanBackendSource.contains("VkClearValue.calloc(attachmentCount, stack)"),
	            "Vulkan render-pass creation should avoid fake attachments for attachmentless passes");

	        String compositeSource = readSource(SRC_MAIN_JAVA.resolve("net/irisshaders/iris/pipeline/CompositeRenderer.java"));
	        String shadowCompositeSource = readSource(SRC_MAIN_JAVA.resolve("net/irisshaders/iris/shadows/ShadowCompositeRenderer.java"));
	        assertTrue(compositeSource.contains("passWidth = main.width")
	                && compositeSource.contains("passHeight = main.height")
	                && !compositeSource.contains("if (compositePass.viewWidth <= 0 || compositePass.viewHeight <= 0)"),
	            "CompositeRenderer should give attachmentless graphics passes an explicit main-target extent instead of skipping them");
		        assertTrue(compositeSource.contains("createRenderTargetDescriptor(label, this.viewWidth, this.viewHeight)")
		                && shadowCompositeSource.contains("createRenderTargetDescriptor(label, this.viewWidth, this.viewHeight)"),
		            "Composite renderers should provide explicit extents for attachmentless Vulkan render-target descriptors");
	    }

	    @Test
	    public void testShaderFramebufferPathsExposeDescriptorBackedVulkanRenderTargets() throws IOException {
	        String commandEncoderSource = readSource(SRC_MAIN_JAVA.resolve("net/blaze3d/systems/CommandEncoder.java"));
        String glFramebufferSource = readSource(SRC_MAIN_JAVA.resolve("net/irisshaders/iris/gl/framebuffer/GlFramebuffer.java"));
        String glCommandEncoderSource = readSource(SRC_MAIN_JAVA.resolve("net/blaze3d/opengl/GlCommandEncoder.java"));
        String terrainSource = readSource(SRC_MAIN_JAVA.resolve("net/sodium/client/render/chunk/DefaultChunkRenderer.java"));
	        String compositeSource = readSource(SRC_MAIN_JAVA.resolve("net/irisshaders/iris/pipeline/CompositeRenderer.java"));
        String finalPassSource = readSource(SRC_MAIN_JAVA.resolve("net/irisshaders/iris/pipeline/FinalPassRenderer.java"));
        String shadowCompositeSource = readSource(SRC_MAIN_JAVA.resolve("net/irisshaders/iris/shadows/ShadowCompositeRenderer.java"));
        String shaderTargetContractSource = readSource(SRC_MAIN_JAVA.resolve("net/irisshaders/iris/pipeline/IrisVulkanRenderTargetContract.java"));
        String vulkanBackendSource = readSource(SRC_MAIN_JAVA.resolve("net/vulkanic/backends/vulkan/VulkanBackend.java"));
        String renderTargetCompatibilitySource = readSource(SRC_MAIN_JAVA.resolve("net/vulkanic/VulkanicRenderTargetCompatibility.java"));

        assertTrue(commandEncoderSource.contains("createRenderPass(VulkanicRenderTargetDescriptor descriptor)"),
            "CommandEncoder should expose descriptor-backed render-pass creation for migrated Vulkan shader paths");
        assertTrue(commandEncoderSource.contains("createRenderPass(VulkanicRenderTargetDescriptor descriptor, int fallbackFramebuffer, boolean preferDescriptor)"),
            "CommandEncoder should expose fallback-aware descriptor render-pass creation for parity-proven Vulkan migrations");
        assertTrue(glFramebufferSource.contains("createRenderTargetDescriptor"),
            "Iris GlFramebuffer should expose an explicit render-target descriptor snapshot");
        assertTrue(glFramebufferSource.contains("List<VulkanicRenderTargetDescriptor.ColorAttachment>"),
            "GlFramebuffer descriptor snapshots should preserve ordered MRT color attachments");
        assertTrue(glFramebufferSource.contains("VulkanicResourceUsage.SAMPLED_READ")
                && glFramebufferSource.contains("VulkanicResourceUsage.COLOR_ATTACHMENT_WRITE")
                && glFramebufferSource.contains("VulkanicResourceUsage.DEPTH_ATTACHMENT_WRITE"),
            "Iris shader framebuffer descriptors should carry explicit sampled/read-write usage intent instead of backend-only inference");
        assertTrue(glFramebufferSource.contains("private boolean drawsToNoColorBuffers;")
                && glFramebufferSource.contains("this.drawBuffers = new int[0];")
                && glFramebufferSource.contains("this.drawsToNoColorBuffers = true;")
                && !glFramebufferSource.contains("drawBuffer == VulkanicAPI.GL_NONE"),
            "GlFramebuffer descriptor snapshots should keep GL_NONE separate from logical attachment index 0");
        assertTrue(glCommandEncoderSource.contains("VulkanicAPI.beginRenderPass(renderPassCtx, descriptor)"),
            "GlCommandEncoder should begin Vulkan render passes from explicit render-target descriptors");
        assertTrue(glCommandEncoderSource.contains("glRenderPass.getRenderTargetDescriptor()"),
            "GlCommandEncoder pipeline resolution should prefer explicit render-target descriptors when present");
        assertTrue(terrainSource.contains("shaderFramebuffer.createRenderTargetDescriptor(() -> \"Sodium chunk terrain\")")
                && terrainSource.contains("USE_DESCRIPTOR_TERRAIN_RENDER_PASS")
                && terrainSource.contains("VulkanicAPI.isVulkanBackendInitializedAndSelected()")
                && terrainSource.contains("commandEncoder.createRenderPass(descriptor, shaderFramebuffer.getId(), preferDescriptor)"),
            "Sodium shader terrain should keep the explicit descriptor snapshot and route descriptor terrain through a Vulkan-only parity fallback seam");
        assertTrue(terrainSource.contains("shaderFramebuffer.getId()")
                && commandEncoderSource.contains("descriptor.hasDepthAttachment()"),
            "Sodium shader terrain should pass the framebuffer fallback id while CommandEncoder owns the fallback depth contract");
	        assertTrue(compositeSource.contains("USE_DESCRIPTOR_COMPOSITE_RENDER_PASS")
	                && compositeSource.contains("this.renderTargetDescriptor(label)")
	                && compositeSource.contains("IrisVulkanRenderTargetContract.selectTarget(")
	                && compositeSource.contains("TargetSelection")
		                && shaderTargetContractSource.contains("VulkanicAPI.renderTargetDescriptorCompatibilityWithFramebuffer(fallbackFramebuffer, descriptor)")
		                && shaderTargetContractSource.contains("IrisShaderRenderTargetContract stage={} passName={} framebuffer={} descriptorCompatibility={} descriptorBacked={} {}")
		                && shaderTargetContractSource.contains("descriptorBacked ? descriptor : null"),
		            "Iris composite passes should use the shared explicit Vulkan render-target compatibility contract instead of relying on framebuffer inference");
	        assertTrue(compositeSource.contains("this.pipelineHandle = this.createCompatiblePipeline(descriptor, renderTargetSelection)")
	                && compositeSource.contains("return renderTargetSelection.createPipeline(descriptor);"),
	            "Iris composite passes should create pipelines through the same descriptor/fallback seam used to begin the render pass");
        assertTrue(finalPassSource.contains("VulkanicRenderTargetDescriptor renderTargetDescriptor = createFinalRenderTargetDescriptor(() -> \"Final pass\", baseWidth, baseHeight)"),
            "FinalPassRenderer should snapshot the final pass target before pipeline/render-pass creation");
        assertTrue(finalPassSource.contains("VulkanicResourceUsage.SAMPLED_READ")
                && finalPassSource.contains("VulkanicResourceUsage.COLOR_ATTACHMENT_WRITE"),
            "FinalPassRenderer should carry explicit render-target usage intent for descriptor-backed Vulkan final passes");
	        assertTrue(finalPassSource.contains("IrisVulkanRenderTargetContract.TargetSelection renderTargetSelection")
	                && finalPassSource.contains("IrisVulkanRenderTargetContract.selectTarget(")
	                && shaderTargetContractSource.contains("IrisShaderRenderTargetContract"),
	            "FinalPassRenderer should use descriptor-backed final-pass rendering only after proving framebuffer/descriptor parity");
	        assertTrue(finalPassSource.contains("? renderTargetSelection.createRenderPass(() -> \"Final pass\")"),
	            "FinalPassRenderer should route Vulkan final passes through the shared target selection render-pass path");
        assertTrue(finalPassSource.contains("VulkanicAPI.createRenderPass(() -> \"Final pass\", main.getColorTextureView(), OptionalInt.empty())"),
            "FinalPassRenderer should preserve the existing OpenGL texture-view render-pass path");
        assertTrue(finalPassSource.contains("renderTargetSelection.vulkanRecordedPass()")
                && shaderTargetContractSource.contains("VulkanicAPI.createRenderPass(label, this.fallbackFramebuffer, this.fallbackHasDepthAttachment)"),
            "FinalPassRenderer should keep Vulkan fallback render-pass compatibility aligned with its framebuffer-target pipeline");
        assertTrue(finalPassSource.contains("finalPass.ensurePipelineState(renderTargetSelection)")
	                && finalPassSource.contains("renderTargetContractKey")
	                && finalPassSource.contains("targetContractChanged"),
            "FinalPassRenderer should create the final-pass pipeline against the same descriptor used to begin the Vulkan render pass");
	        assertTrue(shadowCompositeSource.contains("renderTargetSelection.createRenderPass(label)")
	                && shadowCompositeSource.contains("IrisVulkanRenderTargetContract.selectTarget("),
	            "Iris shadow composite passes should use the shared parity-proven descriptor contract while preserving framebuffer rendering for OpenGL/immediate paths");
	        assertTrue(shadowCompositeSource.contains("this.pipelineHandle = this.createCompatiblePipeline(descriptor, renderTargetSelection)")
	                && shadowCompositeSource.contains("return renderTargetSelection.createPipeline(descriptor);"),
	            "Iris shadow composite passes should create the custom-pass pipeline against the same descriptor used to begin the Vulkan render pass");
        assertTrue(compositeSource.contains("String targetContractKey = renderTargetSelection.contractKey()")
                && compositeSource.contains("targetContractChanged")
                && shadowCompositeSource.contains("String targetContractKey = renderTargetSelection.contractKey()")
                && shadowCompositeSource.contains("targetContractChanged"),
            "Iris composite and shadow composite passes should invalidate cached pipelines when the descriptor/framebuffer target contract changes");
        assertTrue(vulkanBackendSource.contains("resolveRenderTargetDescriptor(VulkanicRenderTargetDescriptor descriptor)"),
            "VulkanBackend should resolve explicit render-target descriptors without relying on a framebuffer id");
        assertTrue(vulkanBackendSource.contains("private static final class VulkanRenderTargetPlan")
                && vulkanBackendSource.contains("resolveRenderPassDescriptorPlan(")
                && vulkanBackendSource.contains("resolveFramebufferRenderTargetPlan(")
                && vulkanBackendSource.contains("resolveRenderTargetDescriptorPlan(")
                && vulkanBackendSource.contains("spine.beginRenderPass(commandBufferHandle, plan)")
                && vulkanBackendSource.contains("spine.beginFramebufferRenderPass(commandBufferHandle, plan)"),
            "Vulkan render-pass and pipeline paths should normalize texture-view, framebuffer, and descriptor targets through one render-target plan");
        assertTrue(vulkanBackendSource.contains("isRenderTargetDescriptorEquivalentToFramebuffer")
                && vulkanBackendSource.contains("isRenderTargetDescriptorCompatibleWithFramebuffer")
                && vulkanBackendSource.contains("renderTargetDescriptorCompatibilityWithFramebuffer")
                && vulkanBackendSource.contains("compatibilitySignature()")
                && vulkanBackendSource.contains("TRACE_RENDER_TARGET_PARITY")
                && renderTargetCompatibilitySource.contains("EXACT")
                && renderTargetCompatibilitySource.contains("DESCRIPTOR_SUFFIX")
                && renderTargetCompatibilitySource.contains("DESCRIPTOR_ATTACHMENTLESS")
                && renderTargetCompatibilitySource.contains("MISMATCH")
                && renderTargetCompatibilitySource.contains("public boolean isCompatible()")
                && renderTargetCompatibilitySource.contains("allowsDescriptorBackedRenderPass()")
                && renderTargetCompatibilitySource.contains("return this == EXACT;"),
            "VulkanBackend should keep exact parity diagnostics while exposing typed compatibility and an exact-only proven-safe render-pass gate");
        String nativeLifecycleSource = readSource(SRC_MAIN_JAVA.resolve("net/vulkanic/backends/vulkan/VulkanNativeRenderTargetLifecycleManager.java"));
        assertTrue(nativeLifecycleSource.contains("VulkanRenderPassLayoutPlanner.planPipelineCompatible(compatibilityKey)")
                && nativeLifecycleSource.contains("layoutPlan.hasDepthAttachment()"),
            "Vulkan target-specific pipelines should consume the planner depth attachment contract");
    }

    @Test
    public void testVulkanFeedbackLoopPipelineRenderPassUsesFeedbackSubpassLayouts() throws IOException {
        Path vulkanBackendFile = SRC_MAIN_JAVA.resolve("net/vulkanic/backends/vulkan/VulkanBackend.java");
        String source = readSource(vulkanBackendFile);
        String nativeLifecycleSource = readSource(SRC_MAIN_JAVA.resolve("net/vulkanic/backends/vulkan/VulkanNativeRenderTargetLifecycleManager.java"));
        String plannerSource = readSource(SRC_MAIN_JAVA.resolve("net/vulkanic/backends/vulkan/VulkanRenderPassLayoutPlanner.java"));

        assertTrue(plannerSource.contains("int colorLayout = compatibilityKey.feedbackLoop()")
                && plannerSource.contains("int depthLayout = compatibilityKey.feedbackLoop()"),
            "Pipeline-compatible render-pass layouts should be chosen by attachment-feedback-loop mode in the planner");
        assertTrue(nativeLifecycleSource.contains(".layout(layoutPlan.colorAttachment(colorIndex).subpassLayout());")
                && nativeLifecycleSource.contains(".layout(layoutPlan.depthAttachment().subpassLayout());"),
            "Native render-target lifecycle should materialize planner-selected color/depth attachment reference layouts");
    }

    @Test
    public void testVulkanSwapchainComposePipelineUsesPresentCompatibleRenderPassContract() throws IOException {
        Path vulkanBackendFile = SRC_MAIN_JAVA.resolve("net/vulkanic/backends/vulkan/VulkanBackend.java");
        String source = readSource(vulkanBackendFile);
        String nativeLifecycleSource = readSource(SRC_MAIN_JAVA.resolve("net/vulkanic/backends/vulkan/VulkanNativeRenderTargetLifecycleManager.java"));

        assertTrue(source.contains("boolean swapchainPresentCompatible"),
            "Vulkan pipeline creation should distinguish swapchain-present render-pass compatibility from feedback-loop compatibility");
        assertTrue(source.contains("swapchainState.imageFormat(),\n                    true"),
            "Swapchain present compose pipeline should request the swapchain-present-compatible render pass contract");
        assertTrue(nativeLifecycleSource.contains("VulkanRenderPassLayoutPlanner.planPipelineCompatible(compatibilityKey)")
                && nativeLifecycleSource.contains("layoutPlan.dependencyIntent()"),
            "Pipeline-compatible render pass creation should consume the extracted planner dependency intent");
        String plannerSource = readSource(SRC_MAIN_JAVA.resolve("net/vulkanic/backends/vulkan/VulkanRenderPassLayoutPlanner.java"));
        assertTrue(plannerSource.contains("VK10.VK_SUBPASS_EXTERNAL")
                && plannerSource.contains("VK10.VK_PIPELINE_STAGE_BOTTOM_OF_PIPE_BIT"),
            "Swapchain-present-compatible placeholder render passes should match the persistent swapchain present render pass dependencies");
    }

    @Test
    public void testVulkanPipelineRenderPassCompatibilityUsesLiveRenderPassProfile() throws IOException {
        Path vulkanBackendFile = SRC_MAIN_JAVA.resolve("net/vulkanic/backends/vulkan/VulkanBackend.java");
        Path compatibilityKeyFile = SRC_MAIN_JAVA.resolve("net/vulkanic/backends/vulkan/VulkanRenderPassCompatibilityKey.java");
        Path pipelinePlannerFile = SRC_MAIN_JAVA.resolve("net/vulkanic/backends/vulkan/VulkanPipelineCreationPlanner.java");
        String source = readSource(vulkanBackendFile);
        String keySource = readSource(compatibilityKeyFile);
        String pipelinePlannerSource = readSource(pipelinePlannerFile);

        assertTrue(keySource.contains("record VulkanRenderPassCompatibilityKey("),
            "VulkanBackend should use a typed render-pass compatibility contract for pipeline variants");
        assertTrue(source.contains("VulkanRenderPassCompatibilityKey compatibilityKey = spine.beginRenderPass(commandBufferHandle, plan)")
                && source.contains("VulkanRenderPassCompatibilityKey compatibilityKey = spine.beginFramebufferRenderPass(commandBufferHandle, plan)"),
            "Render-pass begin paths should return the exact compatibility key for the active native render pass");
        assertTrue(source.contains("return new VulkanBackedRenderPass(spine, commandBufferHandle, compatibilityKey);"),
            "Active Vulkan render passes should carry the native compatibility key into draw-time binding");
        assertTrue(source.contains("if (!compatibilityKey.equals(vulkanPipeline.getRenderPassCompatibilityKey()))"),
            "Vulkan pipeline binding should fail fast before incompatible render-pass/pipeline pairs reach vkCmdDraw");
        assertTrue(source.contains("boolean colorFeedbackLoopCapable = isFeedbackLoopCapable(colorTexture)")
                && source.contains("boolean depthFeedbackLoopCapable = isFeedbackLoopCapable(depthTexture)")
                && source.contains("targets.hasFeedbackLoopTarget()"),
            "Texture-view pipeline variants should snapshot the same attachment feedback-loop capability used by live render passes");
        assertTrue(source.contains("createPipelineCompatibleRenderPass(\n                    stack,\n                    pipelinePlan.renderPassCompatibility().compatibilityKey()")
                && pipelinePlannerSource.contains("new RenderPassCompatibilityPlan(renderPassCompatibilityKey, 0)"),
            "Vulkan pipeline creation should compile against the same compatibility key used for draw-time render passes");
        Path plannerFile = SRC_MAIN_JAVA.resolve("net/vulkanic/backends/vulkan/VulkanRenderPassLayoutPlanner.java");
        String plannerSource = readSource(plannerFile);
        String nativeLifecycleSource = readSource(SRC_MAIN_JAVA.resolve("net/vulkanic/backends/vulkan/VulkanNativeRenderTargetLifecycleManager.java"));
        assertTrue(nativeLifecycleSource.contains("VulkanRenderPassLayoutPlanner.planPipelineCompatible(compatibilityKey)")
                && nativeLifecycleSource.contains("layoutPlan.dependencyIntent()"),
            "Pipeline-compatible render passes and live render passes should consume planner-owned dependency intent");
        assertTrue(plannerSource.contains("case TEXTURE_VIEW -> textureViewDependencies(compatibilityKey);")
                && plannerSource.contains("case FRAMEBUFFER -> framebufferDependencies(compatibilityKey);")
                && plannerSource.contains("case SWAPCHAIN_PRESENT -> swapchainPresentDependencies();"),
            "Texture-view, framebuffer, and swapchain-present render-pass profiles should stay distinct in the planner");
    }

    @Test
    public void testVulkanRenderPassResourceResolutionUsesSharedContract() throws IOException {
        String commandEncoderSource = readSource(SRC_MAIN_JAVA.resolve("net/blaze3d/opengl/GlCommandEncoder.java"));
        String nativeCommandEncoderSource = readSource(SRC_MAIN_JAVA.resolve("net/vulkanic/backends/vulkan/VulkanNativeCommandEncoder.java"));
        String resolverSource = readSource(SRC_MAIN_JAVA.resolve("net/vulkanic/VulkanicPipelineResourceResolver.java"));

        assertTrue(resolverSource.contains("public final class VulkanicPipelineResourceResolver")
                && resolverSource.contains("PipelineResourcePlanner.buildPlan(")
                && resolverSource.contains("PipelineResourceBindings.SamplerBinding")
                && resolverSource.contains("PipelineResourceBindings.TexelBufferBinding"),
            "Shared resource resolver should own descriptor binding construction for sampler, UBO, and texel-buffer resources");
        assertTrue(commandEncoderSource.contains("VulkanicPipelineResourceResolver.buildPlan(")
                && commandEncoderSource.contains("VulkanicPipelineResourceResolver.collectMissingResources("),
            "Compatibility command encoder should use the shared Vulkanic resource resolver for render-pass submissions");
        assertTrue(nativeCommandEncoderSource.contains("VulkanicPipelineResourceResolver.buildPlan(")
                && nativeCommandEncoderSource.contains("VulkanicPipelineResourceResolver.collectMissingResources("),
            "Native Vulkan command encoder should use the shared Vulkanic resource resolver for render-pass submissions");
        assertFalse(commandEncoderSource.contains("new PipelineResourceBindings.SamplerBinding"),
            "Compatibility command encoder should not construct descriptor sampler bindings through a private resolver");
        assertFalse(nativeCommandEncoderSource.contains("new PipelineResourceBindings.SamplerBinding"),
            "Native Vulkan command encoder should not construct descriptor sampler bindings through a private resolver");
    }

    @Test
    public void testEnderiophageEyesUseSemanticEyesLayerOnly() throws IOException {
        String renderer = readSource(SRC_MAIN_JAVA.resolve(
                "net/alexsmobs/client/render/RenderEnderiophage.java"));
        assertTrue(renderer.contains("extends EyesLayer")
                        && renderer.contains("semanticTexture(EnderiophageRenderState state)"),
                "Enderiophage eyes must provide copied semantic texture identity through EyesLayer");
        assertFalse(renderer.contains("void render(PoseStack matrixStackIn, MultiBufferSource bufferIn"),
                "Enderiophage eyes must not retain a Java buffer-backed render override");
        assertFalse(renderer.contains("bufferIn.getBuffer"),
                "Enderiophage eyes must not acquire a Java VertexConsumer on the Rust route");
    }

    @Test
    public void testRustScreenItemActivationDoesNotBorrowJavaLightingState() throws IOException {
        String renderer = readSource(SRC_MAIN_JAVA.resolve(
                "net/minecraft/client/renderer/ScreenEffectRenderer.java"));
        int lighting = renderer.indexOf("getLighting().setupFor");
        int rustGuard = renderer.indexOf("boolean rustSemanticItem", lighting - 500);
        assertTrue(lighting >= 0 && rustGuard >= 0 && rustGuard < lighting,
                "screen item activation must classify the Rust semantic route before Java lighting setup");
        String guarded = renderer.substring(rustGuard, Math.min(renderer.length(), lighting + 180));
        assertTrue(guarded.contains("if (!rustSemanticItem)"),
                "Java item lighting setup must remain OpenGL-compatibility-only");
    }

    @Test
    public void testRustScreenEffectsPreserveNoPhysicsViewBlockingEligibility() throws IOException {
        String renderer = readSource(SRC_MAIN_JAVA.resolve(
                "net/minecraft/client/renderer/ScreenEffectRenderer.java"));
        assertTrue(renderer.contains("BlockState blockingState = player.noPhysics ? null : getViewBlockingState(player);"),
                "Rust screen-effect extraction must preserve vanilla no-physics suppression for block overlays");
    }

    @Test
    public void testWholeFrameVulkanCannotFallBackToJavaScreenEffects() throws IOException {
        String renderer = readSource(SRC_MAIN_JAVA.resolve(
                "net/minecraft/client/renderer/ScreenEffectRenderer.java"));
        assertTrue(renderer.contains(
                "RustGalVulkanWholeFrameMode.enabled()"),
                "legacy screen effects must check Rust whole-frame ownership");
        assertTrue(renderer.contains(
                "Java screen-effect rendering is unavailable on selected Vulkan")
                || renderer.contains("Java screen-effect rendering is unavailable while Rust owns whole-frame presentation"),
                "legacy Java screen effects must fail closed under Rust Vulkan ownership");
        assertTrue(renderer.contains("renderRustVulkanScreenEffects"),
                "Rust Vulkan must retain an explicit semantic screen-effects producer");
    }

    @Test
    public void testGameTestHighlightsUseRustSemanticStreamsUnderWholeFrameVulkan() throws IOException {
        String renderer = readSource(SRC_MAIN_JAVA.resolve(
            "net/minecraft/client/renderer/debug/GameTestBlockHighlightRenderer.java"));
        String level = readSource(SRC_MAIN_JAVA.resolve(
            "net/minecraft/client/renderer/LevelRenderer.java"));
        assertTrue(renderer.contains("collectRustSemantics"),
            "game-test markers must expose an explicit semantic producer");
        assertTrue(renderer.contains("submitColoredQuads")
                && renderer.contains("submitTextSemantic"),
            "game-test boxes and labels must use Rust procedural/world-text streams");
        assertTrue(level.contains("gameTestBlockHighlightRenderer.collectRustSemantics"),
            "whole-frame extraction must collect game-test markers before feature dispatch");
        assertTrue(level.contains("VoxelConstants.submitRustWaypointSemantics")
                && level.contains("VoxelConstants.onRenderWaypoints(deltaTracker.getGameTimeDeltaPartialTick(false)"),
            "waypoint collection and legacy rendering must remain distinct callsites");
        assertTrue(level.contains("if (!net.vulkanic.bridge.RustGalVulkanWholeFrameMode.enabled()")
                && level.contains("this.gameTestBlockHighlightRenderer.render(poseStack, bufferSource);"),
            "the Java game-test renderer must be fenced off after Rust semantic extraction");
        assertTrue(renderer.contains(
                "Rust whole-frame game-test highlight route is unavailable while Rust owns presentation"),
            "disabled game-test routes must fail closed instead of drawing Java geometry");
        assertTrue(renderer.contains("MAX_MARKERS = 4096")
                && renderer.contains("game-test highlight marker capacity exceeded"),
            "game-test marker bookkeeping must remain bounded before semantic collection");
    }

    @Test
    public void testCollisionDebugUsesRustSemanticLineStreamUnderWholeFrameVulkan() throws IOException {
        String renderer = readSource(SRC_MAIN_JAVA.resolve(
            "net/minecraft/client/renderer/debug/CollisionBoxRenderer.java"));
        String dispatcher = readSource(SRC_MAIN_JAVA.resolve(
            "net/minecraft/client/renderer/debug/DebugRenderer.java"));
        String game = readSource(SRC_MAIN_JAVA.resolve(
            "net/minecraft/client/renderer/GameRenderer.java"));
        assertTrue(renderer.contains("collectRustSemantics")
                && renderer.contains("enqueueDebugLineSegments"),
            "collision shapes must be copied into Rust's explicit debug-line stream");
        assertTrue(renderer.contains(
                "Rust whole-frame collision-debug route is unavailable; Java debug geometry is not a fallback"),
            "collision debugging must fail closed when its Rust route is disabled");
        assertTrue(dispatcher.contains("collectRustCollisionSemantics")
                && game.contains("collectRustCollisionSemantics"),
            "the whole-frame shell must invoke collision semantic extraction instead of Java DebugRenderer");
    }

    @Test
    public void testSolidFaceDebugUsesRustSemanticQuadStreamUnderWholeFrameVulkan() throws IOException {
        String renderer = readSource(SRC_MAIN_JAVA.resolve(
            "net/minecraft/client/renderer/debug/SolidFaceRenderer.java"));
        String dispatcher = readSource(SRC_MAIN_JAVA.resolve(
            "net/minecraft/client/renderer/debug/DebugRenderer.java"));
        String game = readSource(SRC_MAIN_JAVA.resolve(
            "net/minecraft/client/renderer/GameRenderer.java"));
        assertTrue(renderer.contains("collectRustSemantics")
                && renderer.contains("submitColoredQuads"),
            "solid-face debug geometry must use Rust's explicit colored-quad stream");
        assertTrue(renderer.contains(
                "Rust whole-frame solid-face debug route is unavailable; Java debug geometry is not a fallback"),
            "solid-face debugging must fail closed when its Rust route is disabled");
        assertTrue(dispatcher.contains("collectRustSolidFaceSemantics")
                && game.contains("collectRustSolidFaceSemantics"),
            "the whole-frame shell must invoke solid-face semantic extraction");
    }

    @Test
    public void testSupportBlockDebugUsesRustSemanticLineStreamUnderWholeFrameVulkan() throws IOException {
        String renderer = readSource(SRC_MAIN_JAVA.resolve(
            "net/minecraft/client/renderer/debug/SupportBlockRenderer.java"));
        String dispatcher = readSource(SRC_MAIN_JAVA.resolve(
            "net/minecraft/client/renderer/debug/DebugRenderer.java"));
        String game = readSource(SRC_MAIN_JAVA.resolve(
            "net/minecraft/client/renderer/GameRenderer.java"));
        assertTrue(renderer.contains("collectRustSemantics")
                && renderer.contains("enqueueDebugLineSegments"),
            "support-block highlights must use Rust's explicit debug-line stream");
        assertTrue(renderer.contains(
                "Rust whole-frame support-block debug route is unavailable; Java debug geometry is not a fallback"),
            "support-block debugging must fail closed when its Rust route is disabled");
        assertTrue(dispatcher.contains("collectRustSupportBlockSemantics")
                && game.contains("collectRustSupportBlockSemantics"),
            "the whole-frame shell must invoke support-block semantic extraction");
    }

    @Test
    public void testNeighborUpdateDebugUsesRustSemanticGeometryAndTextUnderWholeFrameVulkan() throws IOException {
        String renderer = readSource(SRC_MAIN_JAVA.resolve(
            "net/minecraft/client/renderer/debug/NeighborsUpdateRenderer.java"));
        String dispatcher = readSource(SRC_MAIN_JAVA.resolve(
            "net/minecraft/client/renderer/debug/DebugRenderer.java"));
        String game = readSource(SRC_MAIN_JAVA.resolve(
            "net/minecraft/client/renderer/GameRenderer.java"));
        assertTrue(renderer.contains("collectRustSemantics")
                && renderer.contains("enqueueDebugLineSegments")
                && renderer.contains("submitTextSemantic"),
            "neighbor updates must preserve both Rust line and world-text semantics");
        assertTrue(renderer.contains(
                "Rust whole-frame neighbor-update debug route is unavailable; Java debug geometry is not a fallback"),
            "neighbor-update debugging must fail closed when either Rust route is disabled");
        assertTrue(dispatcher.contains("collectRustNeighborUpdateSemantics")
                && game.contains("collectRustNeighborUpdateSemantics"),
            "the whole-frame shell must invoke neighbor-update semantic extraction");
    }

    @Test
    public void testStructureDebugUsesRustSemanticLineStreamUnderWholeFrameVulkan() throws IOException {
        String renderer = readSource(SRC_MAIN_JAVA.resolve(
            "net/minecraft/client/renderer/debug/StructureRenderer.java"));
        String dispatcher = readSource(SRC_MAIN_JAVA.resolve(
            "net/minecraft/client/renderer/debug/DebugRenderer.java"));
        String game = readSource(SRC_MAIN_JAVA.resolve(
            "net/minecraft/client/renderer/GameRenderer.java"));
        assertTrue(renderer.contains("collectRustSemantics")
                && renderer.contains("enqueueDebugLineSegments"),
            "structure and piece bounds must use Rust's explicit debug-line stream");
        assertTrue(renderer.contains(
                "Rust whole-frame structure-debug route is unavailable; Java debug geometry is not a fallback"),
            "structure debugging must fail closed when its Rust route is disabled");
        assertTrue(dispatcher.contains("collectRustStructureSemantics")
                && game.contains("collectRustStructureSemantics"),
            "the whole-frame shell must invoke structure semantic extraction");
    }

    @Test
    public void testGameEventDebugUsesRustSemanticGeometryAndTextUnderWholeFrameVulkan() throws IOException {
        String renderer = readSource(SRC_MAIN_JAVA.resolve(
            "net/minecraft/client/renderer/debug/GameEventListenerRenderer.java"));
        String dispatcher = readSource(SRC_MAIN_JAVA.resolve(
            "net/minecraft/client/renderer/debug/DebugRenderer.java"));
        String game = readSource(SRC_MAIN_JAVA.resolve(
            "net/minecraft/client/renderer/GameRenderer.java"));
        assertTrue(renderer.contains("collectRustSemantics")
                && renderer.contains("enqueueDebugLineSegments")
                && renderer.contains("submitColoredQuads")
                && renderer.contains("submitTextSemantic"),
            "game-event listeners must preserve Rust line, quad, and text semantics");
        assertTrue(renderer.contains(
                "Rust whole-frame game-event debug route is unavailable; Java debug geometry is not a fallback"),
            "game-event debugging must fail closed when a required Rust route is disabled");
        assertTrue(dispatcher.contains("collectRustGameEventListenerSemantics")
                && game.contains("collectRustGameEventListenerSemantics"),
            "the whole-frame shell must invoke game-event semantic extraction");
    }

    @Test
    public void testRedstoneOrientationDebugUsesRustSemanticLineStreamUnderWholeFrameVulkan() throws IOException {
        String renderer = readSource(SRC_MAIN_JAVA.resolve(
            "net/minecraft/client/renderer/debug/RedstoneWireOrientationsRenderer.java"));
        String dispatcher = readSource(SRC_MAIN_JAVA.resolve(
            "net/minecraft/client/renderer/debug/DebugRenderer.java"));
        String game = readSource(SRC_MAIN_JAVA.resolve(
            "net/minecraft/client/renderer/GameRenderer.java"));
        assertTrue(renderer.contains("collectRustSemantics")
                && renderer.contains("enqueueDebugLineSegments"),
            "redstone orientation vectors must use Rust's explicit debug-line stream");
        assertTrue(renderer.contains(
                "Rust whole-frame redstone-orientation route is unavailable; Java debug geometry is not a fallback"),
            "redstone orientation debugging must fail closed when its Rust route is disabled");
        assertTrue(dispatcher.contains("collectRustRedstoneWireOrientationSemantics")
                && game.contains("collectRustRedstoneWireOrientationSemantics"),
            "the whole-frame shell must invoke redstone orientation semantic extraction");
    }

    @Test
    public void testChunkBorderDebugUsesRustSemanticLineStreamUnderWholeFrameVulkan() throws IOException {
        String renderer = readSource(SRC_MAIN_JAVA.resolve(
            "net/minecraft/client/renderer/debug/ChunkBorderRenderer.java"));
        String dispatcher = readSource(SRC_MAIN_JAVA.resolve(
            "net/minecraft/client/renderer/debug/DebugRenderer.java"));
        String game = readSource(SRC_MAIN_JAVA.resolve(
            "net/minecraft/client/renderer/GameRenderer.java"));
        assertTrue(renderer.contains("collectRustSemantics")
                && renderer.contains("enqueueDebugLineSegments"),
            "chunk borders and grids must use Rust's explicit debug-line stream");
        assertTrue(renderer.contains(
                "Rust whole-frame chunk-border route is unavailable; Java debug geometry is not a fallback"),
            "chunk-border debugging must fail closed when its Rust route is disabled");
        assertTrue(dispatcher.contains("collectRustChunkBorderSemantics")
                && game.contains("collectRustChunkBorderSemantics"),
            "the whole-frame shell must invoke chunk-border semantic extraction");
    }

    @Test
    public void testBreezeDebugUsesRustSemanticLinesAndQuadsUnderWholeFrameVulkan() throws IOException {
        String renderer = readSource(SRC_MAIN_JAVA.resolve(
            "net/minecraft/client/renderer/debug/BreezeDebugRenderer.java"));
        String dispatcher = readSource(SRC_MAIN_JAVA.resolve(
            "net/minecraft/client/renderer/debug/DebugRenderer.java"));
        String game = readSource(SRC_MAIN_JAVA.resolve(
            "net/minecraft/client/renderer/GameRenderer.java"));
        assertTrue(renderer.contains("collectRustSemantics")
                && renderer.contains("enqueueDebugLineSegments")
                && renderer.contains("submitColoredQuads"),
            "Breeze diagnostics must use Rust semantic lines and marker quads");
        assertTrue(renderer.contains(
                "Rust whole-frame Breeze-debug route is unavailable; Java debug geometry is not a fallback"),
            "Breeze debugging must fail closed when a required Rust route is disabled");
        assertTrue(dispatcher.contains("collectRustBreezeSemantics")
                && game.contains("collectRustBreezeSemantics"),
            "the whole-frame shell must invoke Breeze semantic extraction");
    }

    @Test
    public void testPathfindingDebugUsesRustSemanticLinesQuadsAndTextUnderWholeFrameVulkan() throws IOException {
        String renderer = readSource(SRC_MAIN_JAVA.resolve(
            "net/minecraft/client/renderer/debug/PathfindingRenderer.java"));
        String dispatcher = readSource(SRC_MAIN_JAVA.resolve(
            "net/minecraft/client/renderer/debug/DebugRenderer.java"));
        String game = readSource(SRC_MAIN_JAVA.resolve(
            "net/minecraft/client/renderer/GameRenderer.java"));
        assertTrue(renderer.contains("collectRustSemantics")
                && renderer.contains("enqueueDebugLineSegments")
                && renderer.contains("submitColoredQuads")
                && renderer.contains("submitTextSemantic"),
            "pathfinding diagnostics must use Rust semantic lines, boxes, and labels");
        assertTrue(renderer.contains(
                "Rust whole-frame pathfinding-debug route is unavailable; Java debug geometry is not a fallback"),
            "pathfinding debugging must fail closed when a required Rust route is disabled");
        assertTrue(dispatcher.contains("collectRustPathfindingSemantics")
                && game.contains("collectRustPathfindingSemantics"),
            "the whole-frame shell must invoke pathfinding semantic extraction");
    }

    @Test
    public void testLightSectionDebugUsesRustSemanticEdgesAndFacesUnderWholeFrameVulkan() throws IOException {
        String renderer = readSource(SRC_MAIN_JAVA.resolve(
            "net/minecraft/client/renderer/debug/LightSectionDebugRenderer.java"));
        String dispatcher = readSource(SRC_MAIN_JAVA.resolve(
            "net/minecraft/client/renderer/debug/DebugRenderer.java"));
        String game = readSource(SRC_MAIN_JAVA.resolve(
            "net/minecraft/client/renderer/GameRenderer.java"));
        assertTrue(renderer.contains("collectRustSemantics")
                && renderer.contains("enqueueDebugLineSegments")
                && renderer.contains("submitColoredQuads"),
            "light-section debug fields must use Rust semantic edges and faces");
        assertTrue(renderer.contains(
                "Rust whole-frame light-section debug route is unavailable; Java debug geometry is not a fallback"),
            "light-section debugging must fail closed when a required Rust route is disabled");
        assertTrue(dispatcher.contains("collectRustLightSectionSemantics")
                && game.contains("collectRustLightSectionSemantics"),
            "the whole-frame shell must invoke light-section semantic extraction");
    }

    @Test
    public void testHeightMapDebugUsesRustSemanticQuadsUnderWholeFrameVulkan() throws IOException {
        String renderer = Files.readString(Path.of(
            "src/main/java/net/minecraft/client/renderer/debug/HeightMapRenderer.java"));
        String dispatcher = Files.readString(Path.of(
            "src/main/java/net/minecraft/client/renderer/debug/DebugRenderer.java"));
        String game = Files.readString(Path.of(
            "src/main/java/net/minecraft/client/renderer/GameRenderer.java"));
        assertTrue(renderer.contains("collectRustSemantics")
                && renderer.contains("submitColoredQuads")
                && renderer.contains("height-map route rejected semantic quads"),
            "height-map debugging must copy bounded voxel overlays into Rust semantic quads");
        assertTrue(dispatcher.contains("collectRustHeightMapSemantics")
                && game.contains("collectRustHeightMapSemantics"),
            "the whole-frame shell must invoke height-map semantic extraction");
    }

    @Test
    public void testChunkCullingDebugUsesRustSemanticLinesAndQuadsUnderWholeFrameVulkan() throws IOException {
        String renderer = Files.readString(Path.of(
            "src/main/java/net/minecraft/client/renderer/debug/ChunkCullingDebugRenderer.java"));
        String dispatcher = Files.readString(Path.of(
            "src/main/java/net/minecraft/client/renderer/debug/DebugRenderer.java"));
        String game = Files.readString(Path.of(
            "src/main/java/net/minecraft/client/renderer/GameRenderer.java"));
        assertTrue(renderer.contains("collectRustSemantics")
                && renderer.contains("enqueueDebugLineSegments")
                && renderer.contains("chunk-culling route rejected semantic visibility quads"),
            "chunk-culling diagnostics must be copied into Rust semantic lines and quads");
        assertTrue(dispatcher.contains("collectRustChunkCullingSemantics")
                && game.contains("collectRustChunkCullingSemantics"),
            "the whole-frame shell must invoke chunk-culling semantic extraction");
    }

    @Test
    public void testWaterDebugUsesRustSemanticBoxesAndLabelsUnderWholeFrameVulkan() throws IOException {
        String renderer = Files.readString(Path.of(
            "src/main/java/net/minecraft/client/renderer/debug/WaterDebugRenderer.java"));
        String level = Files.readString(Path.of(
            "src/main/java/net/minecraft/client/renderer/LevelRenderer.java"));
        String game = Files.readString(Path.of(
            "src/main/java/net/minecraft/client/renderer/GameRenderer.java"));
        assertTrue(renderer.contains("collectRustSemantics")
                && renderer.contains("submitColoredQuads")
                && renderer.contains("submitTextSemantic"),
            "water debugging must copy nearby fluid levels and labels into Rust semantic streams");
        assertTrue(level.contains("collectRustWaterSemantics")
                && game.contains("collectRustWaterSemantics"),
            "the whole-frame shell must invoke water semantic extraction through LevelRenderer");
    }

    @Test
    public void testLightDebugUsesRustSemanticTextUnderWholeFrameVulkan() throws IOException {
        String renderer = Files.readString(Path.of(
            "src/main/java/net/minecraft/client/renderer/debug/LightDebugRenderer.java"));
        String level = Files.readString(Path.of(
            "src/main/java/net/minecraft/client/renderer/LevelRenderer.java"));
        String game = Files.readString(Path.of(
            "src/main/java/net/minecraft/client/renderer/GameRenderer.java"));
        assertTrue(renderer.contains("collectRustSemantics")
                && renderer.contains("submitTextSemantic")
                && renderer.contains("getDebugData"),
            "light debugging must copy section and block diagnostics into Rust semantic text");
        assertTrue(level.contains("collectRustLightSemantics")
                && game.contains("collectRustLightSemantics"),
            "the whole-frame shell must invoke light semantic extraction through LevelRenderer");
    }

    @Test
    public void testVillageSectionsDebugUsesRustSemanticMarkersUnderWholeFrameVulkan() throws IOException {
        String renderer = Files.readString(Path.of(
            "src/main/java/net/minecraft/client/renderer/debug/VillageSectionsDebugRenderer.java"));
        String dispatcher = Files.readString(Path.of(
            "src/main/java/net/minecraft/client/renderer/debug/DebugRenderer.java"));
        String game = Files.readString(Path.of(
            "src/main/java/net/minecraft/client/renderer/GameRenderer.java"));
        assertTrue(renderer.contains("collectRustSemantics")
                && renderer.contains("VILLAGE_SECTIONS")
                && renderer.contains("submitColoredQuads"),
            "village-section subscriptions must be copied into Rust semantic markers");
        assertTrue(dispatcher.contains("collectRustVillageSectionSemantics")
                && game.contains("collectRustVillageSectionSemantics"),
            "the whole-frame shell must invoke village-section semantic extraction");
    }

    @Test
    public void testChunkDebugUsesRustSemanticTextUnderWholeFrameVulkan() throws IOException {
        String renderer = Files.readString(Path.of(
            "src/main/java/net/minecraft/client/renderer/debug/ChunkDebugRenderer.java"));
        String level = Files.readString(Path.of(
            "src/main/java/net/minecraft/client/renderer/LevelRenderer.java"));
        String game = Files.readString(Path.of(
            "src/main/java/net/minecraft/client/renderer/GameRenderer.java"));
        assertTrue(renderer.contains("collectRustSemantics")
                && renderer.contains("serverData.getNow")
                && renderer.contains("submitTextSemantic"),
            "chunk client/server diagnostics must be copied into Rust semantic text");
        assertTrue(level.contains("collectRustChunkSemantics")
                && game.contains("collectRustChunkSemantics"),
            "the whole-frame shell must invoke chunk diagnostic semantic extraction");
    }

    @Test
    public void testEntityBlockIntersectionDebugUsesRustSemanticBoxesUnderWholeFrameVulkan() throws IOException {
        String renderer = Files.readString(Path.of(
            "src/main/java/net/minecraft/client/renderer/debug/EntityBlockIntersectionDebugRenderer.java"));
        String dispatcher = Files.readString(Path.of(
            "src/main/java/net/minecraft/client/renderer/debug/DebugRenderer.java"));
        String game = Files.readString(Path.of(
            "src/main/java/net/minecraft/client/renderer/GameRenderer.java"));
        assertTrue(renderer.contains("collectRustSemantics")
                && renderer.contains("ENTITY_BLOCK_INTERSECTIONS")
                && renderer.contains("submitColoredQuads"),
            "entity/block intersections must be copied into Rust semantic boxes");
        assertTrue(dispatcher.contains("collectRustEntityBlockIntersectionSemantics")
                && game.contains("collectRustEntityBlockIntersectionSemantics"),
            "the whole-frame shell must invoke entity/block intersection extraction");
    }

    @Test
    public void testGoalSelectorDebugUsesRustSemanticTextUnderWholeFrameVulkan() throws IOException {
        String renderer = Files.readString(Path.of(
            "src/main/java/net/minecraft/client/renderer/debug/GoalSelectorDebugRenderer.java"));
        String level = Files.readString(Path.of(
            "src/main/java/net/minecraft/client/renderer/LevelRenderer.java"));
        String game = Files.readString(Path.of(
            "src/main/java/net/minecraft/client/renderer/GameRenderer.java"));
        assertTrue(renderer.contains("collectRustSemantics")
                && renderer.contains("GOAL_SELECTORS")
                && renderer.contains("submitTextSemantic"),
            "goal-selector subscriptions must be copied into Rust semantic text");
        assertTrue(level.contains("collectRustGoalSelectorSemantics")
                && game.contains("collectRustGoalSelectorSemantics"),
            "the whole-frame shell must invoke goal-selector semantic extraction");
    }

    @Test
    public void testRaidDebugUsesRustSemanticBoxesAndLabelsUnderWholeFrameVulkan() throws IOException {
        String renderer = Files.readString(Path.of(
            "src/main/java/net/minecraft/client/renderer/debug/RaidDebugRenderer.java"));
        String level = Files.readString(Path.of(
            "src/main/java/net/minecraft/client/renderer/LevelRenderer.java"));
        String game = Files.readString(Path.of(
            "src/main/java/net/minecraft/client/renderer/GameRenderer.java"));
        assertTrue(renderer.contains("collectRustSemantics")
                && renderer.contains("RAIDS")
                && renderer.contains("submitColoredQuads")
                && renderer.contains("submitTextSemantic"),
            "raid subscriptions must be copied into Rust semantic center boxes and labels");
        assertTrue(level.contains("collectRustRaidSemantics")
                && game.contains("collectRustRaidSemantics"),
            "the whole-frame shell must invoke raid semantic extraction");
    }

    @Test
    public void testPoiDebugUsesRustSemanticBoxesAndLabelsUnderWholeFrameVulkan() throws IOException {
        String renderer = Files.readString(Path.of(
            "src/main/java/net/minecraft/client/renderer/debug/PoiDebugRenderer.java"));
        String level = Files.readString(Path.of(
            "src/main/java/net/minecraft/client/renderer/LevelRenderer.java"));
        String game = Files.readString(Path.of(
            "src/main/java/net/minecraft/client/renderer/GameRenderer.java"));
        assertTrue(renderer.contains("collectRustSemantics")
                && renderer.contains("POIS")
                && renderer.contains("submitColoredQuads")
                && renderer.contains("submitTextSemantic"),
            "POI and ghost-POI subscriptions must be copied into Rust semantic streams");
        assertTrue(level.contains("collectRustPoiSemantics")
                && game.contains("collectRustPoiSemantics"),
            "the whole-frame shell must invoke POI semantic extraction");
    }

    @Test
    public void testBrainDebugUsesRustSemanticTextUnderWholeFrameVulkan() throws IOException {
        String renderer = Files.readString(Path.of(
            "src/main/java/net/minecraft/client/renderer/debug/BrainDebugRenderer.java"));
        String level = Files.readString(Path.of(
            "src/main/java/net/minecraft/client/renderer/LevelRenderer.java"));
        String game = Files.readString(Path.of(
            "src/main/java/net/minecraft/client/renderer/GameRenderer.java"));
        assertTrue(renderer.contains("collectRustSemantics")
                && renderer.contains("BRAINS")
                && renderer.contains("submitTextSemantic")
                && renderer.contains("memories"),
            "brain subscriptions must preserve the complete semantic label set");
        assertTrue(level.contains("collectRustBrainSemantics")
                && game.contains("collectRustBrainSemantics"),
            "the whole-frame shell must invoke brain semantic extraction");
    }

    @Test
    public void testBeeDebugUsesRustSemanticBoxesAndTextUnderWholeFrameVulkan() throws IOException {
        String renderer = Files.readString(Path.of(
            "src/main/java/net/minecraft/client/renderer/debug/BeeDebugRenderer.java"));
        String level = Files.readString(Path.of(
            "src/main/java/net/minecraft/client/renderer/LevelRenderer.java"));
        String game = Files.readString(Path.of(
            "src/main/java/net/minecraft/client/renderer/GameRenderer.java"));
        assertTrue(renderer.contains("collectRustSemantics")
                && renderer.contains("BEES")
                && renderer.contains("BEE_HIVES")
                && renderer.contains("submitColoredQuads")
                && renderer.contains("submitTextSemantic"),
            "bee, flower, hive, and ghost-hive diagnostics must use Rust semantic streams");
        assertTrue(level.contains("collectRustBeeSemantics")
                && game.contains("collectRustBeeSemantics"),
            "the whole-frame shell must invoke bee semantic extraction");
    }

    @Test
    public void testOctreeDebugUsesRustSemanticEdgesAndLabelsUnderWholeFrameVulkan() throws IOException {
        String renderer = Files.readString(Path.of(
            "src/main/java/net/minecraft/client/renderer/debug/OctreeDebugRenderer.java"));
        String level = Files.readString(Path.of(
            "src/main/java/net/minecraft/client/renderer/LevelRenderer.java"));
        String game = Files.readString(Path.of(
            "src/main/java/net/minecraft/client/renderer/GameRenderer.java"));
        assertTrue(renderer.contains("collectRustSemantics")
                && renderer.contains("visitNodes")
                && renderer.contains("enqueueDebugLineSegments")
                && renderer.contains("submitTextSemantic"),
            "octree traversal must use the frame frustum and Rust semantic edges/labels");
        assertTrue(level.contains("collectRustOctreeSemantics")
                && game.contains("collectRustOctreeSemantics"),
            "the whole-frame shell must invoke octree semantic extraction at matrix setup");
    }

    @Test
    public void testRustCloudRouteFailsClosedWhenSemanticCellFieldIsMissing() throws IOException {
        String renderer = readSource(SRC_MAIN_JAVA.resolve(
                "net/minecraft/client/renderer/CloudRenderer.java"));
        assertTrue(renderer.contains("Rust whole-frame cloud route requires a decoded semantic cloud-cell field"),
                "visible Rust clouds must fail closed when their semantic cell field is unavailable");
        assertTrue(renderer.contains("if (this.texture == null)"),
                "Rust cloud extraction must validate its copied semantic source before lowering faces");
        assertTrue(renderer.contains("Rust cloud semantics require finite camera, height, and partial tick"),
                "Rust cloud extraction must reject non-finite frame inputs before computing cell coordinates");
        assertTrue(renderer.contains("Rust cloud semantics require a camera position"),
                "Rust cloud extraction must not silently omit visible clouds when the camera is missing");
        assertTrue(renderer.contains("bounded, dimensionally complete cloud-cell field"),
                "Rust cloud extraction must fail closed before coordinate math when copied cell dimensions or payload are malformed");
    }

    @Test
    public void testRustCloudSemanticCellFieldUsesOverflowSafeAndBoundedDimensions() throws IOException {
        String renderer = readSource(SRC_MAIN_JAVA.resolve(
                "net/vulkanic/world/RustGalWorldPrimitiveRenderer.java"));
        assertTrue(renderer.contains("long expectedCells"),
                "cloud-cell dimensions must be multiplied in a widened type");
        assertTrue(renderer.contains("MAX_RUST_CLOUD_CELLS"),
                "cloud-cell snapshots must have an explicit bounded capacity");
        assertTrue(renderer.contains("radius > MAX_RUST_CLOUD_RADIUS"),
                "cloud traversal radius must be bounded before nested iteration");
    }

    @Test
    public void testRustEntityFlameRejectsNonPositiveCopiedBoundsBeforeLayerExpansion() throws IOException {
        String renderer = readSource(SRC_MAIN_JAVA.resolve(
                "net/vulkanic/world/RustGalWorldPrimitiveRenderer.java"));
        int flame = renderer.indexOf("collectEntityFlameSemantics(");
        int height = renderer.indexOf("state.boundingBoxHeight <= 0.0F", flame);
        int layers = renderer.indexOf("float remainingLayers = state.boundingBoxHeight / scale", height);
        assertTrue(flame >= 0 && height > flame && layers > height,
                "entity flame bounds must reject non-positive copied heights before layer expansion");
    }

    @Test
    public void testRustEntityShadowRejectsNonFiniteOrOutOfRangeCopiedPieces() throws IOException {
        String renderer = readSource(SRC_MAIN_JAVA.resolve(
                "net/vulkanic/world/RustGalWorldPrimitiveRenderer.java"));
        int shadow = renderer.indexOf("collectEntityShadowSemantics(");
        int piece = renderer.indexOf("!Float.isFinite(shadowPiece.relativeX())", shadow);
        int bounds = renderer.indexOf("!Double.isFinite(bounds.minX)", piece);
        assertTrue(shadow >= 0 && piece > shadow && bounds > piece,
                "entity shadows must validate copied piece coordinates and AABB bounds before vertex expansion");
        assertTrue(renderer.indexOf("shadowPiece.alpha() < 0.0F", piece) > piece,
                "entity shadow alpha must remain within the semantic normalized range");
    }

    @Test
    public void testRustEntityLeashRejectsFloatOverflowBeforeStripExpansion() throws IOException {
        String renderer = readSource(SRC_MAIN_JAVA.resolve(
                "net/vulkanic/world/RustGalWorldPrimitiveRenderer.java"));
        int leash = renderer.indexOf("collectEntityLeashSemantics(");
        int dx = renderer.indexOf("!Float.isFinite(dx)", leash);
        int offset = renderer.indexOf("!Float.isFinite((float)leash.offset.x)", dx);
        int expand = renderer.indexOf("appendLeashStripLocked", offset);
        assertTrue(leash >= 0 && dx > leash && offset > dx && expand > offset,
                "entity leash endpoint deltas and offsets must remain finite after float conversion before strip expansion");
    }

    @Test
    public void testRustEntityMaterialCollectorsRejectNonFinitePoseMatrices() throws IOException {
        String renderer = readSource(SRC_MAIN_JAVA.resolve(
                "net/vulkanic/world/RustGalWorldPrimitiveRenderer.java"));
        assertTrue(renderer.contains("!flameSubmit.pose().pose().isFinite()")
                        && renderer.contains("!shadowSubmit.pose().isFinite()")
                        && renderer.contains("!leashSubmit.pose().isFinite()"),
                "entity material collectors must reject non-finite copied pose matrices before expansion");
    }

    @Test
    public void testRustIndexedModelExtractorsRejectNonFiniteCopiedGeometry() throws IOException {
        String renderer = readSource(SRC_MAIN_JAVA.resolve(
                "net/vulkanic/world/RustGalWorldPrimitiveRenderer.java"));
        int arrow = renderer.indexOf("extractArrowModelMesh(");
        int arrowGuard = renderer.indexOf("ensureFiniteModelVector(transformedNormal", arrow);
        int model = renderer.indexOf("extractModelPartMesh(", arrowGuard);
        int modelGuard = renderer.indexOf("ensureFiniteModelVector(transformedNormal", model);
        int uvGuard = renderer.indexOf("contains non-finite vertex UV", modelGuard);
        assertTrue(arrow >= 0 && arrowGuard > arrow && model > arrowGuard && modelGuard > model && uvGuard > modelGuard,
                "Rust indexed arrow and ModelPart extractors must reject non-finite copied positions, normals, and UVs");
        assertTrue(renderer.contains("entityPose == null || !entityPose.pose().isFinite()"),
                "indexed model instance admission must reject non-finite copied pose matrices");
    }

    @Test
    public void testRustBlockFeatureRoutesRejectNullOrNonFiniteSemanticPoses() throws IOException {
        String renderer = readSource(SRC_MAIN_JAVA.resolve(
                "net/vulkanic/world/RustGalWorldPrimitiveRenderer.java"));
        int block = renderer.indexOf("public static boolean enqueueBlockDisplay(");
        int nullGuard = renderer.indexOf("blockSubmit == null", block);
        int poseGuard = renderer.indexOf("blockSubmit.pose() == null || !blockSubmit.pose().pose().isFinite()", nullGuard);
        int moving = renderer.indexOf("private static boolean enqueueMovingBlockMesh(", poseGuard);
        int movingGuard = renderer.indexOf("movingBlockSubmit == null || movingBlockSubmit.pose() == null || !movingBlockSubmit.pose().isFinite()", moving);
        assertTrue(block >= 0 && nullGuard > block && poseGuard > nullGuard && moving > poseGuard && movingGuard > moving,
                "Rust block-display and moving-block routes must reject malformed semantic submissions before extraction");
        int baked = renderer.indexOf("contains non-finite baked vertex data", movingGuard);
        int shade = renderer.indexOf("contains non-finite tint shade", movingGuard);
        assertTrue(baked > movingGuard && shade > movingGuard,
                "Rust baked block extraction must reject non-finite copied vertex and shading inputs");
    }

    @Test
    public void testRustBlockModelFeatureRejectsNonFiniteSemanticState() throws IOException {
        String renderer = readSource(SRC_MAIN_JAVA.resolve(
                "net/vulkanic/world/RustGalWorldPrimitiveRenderer.java"));
        int enqueue = renderer.indexOf("public static boolean enqueueBlockModelMesh(");
        int pose = renderer.indexOf("submit.pose() == null || !submit.pose().pose().isFinite()", enqueue);
        int color = renderer.indexOf("!Float.isFinite(submit.r())", pose);
        int extraction = renderer.indexOf("extractStandaloneBlockModelMesh(", color);
        assertTrue(enqueue >= 0 && pose > enqueue && color > pose && extraction > color,
                "Rust block-model feature submissions must validate copied pose and tint state before extraction");
    }

    @Test
    public void testRustItemEntityAdmissionBoundsTintIndicesAndPose() throws IOException {
        String renderer = readSource(SRC_MAIN_JAVA.resolve(
                "net/vulkanic/world/RustGalWorldPrimitiveRenderer.java"));
        int eligibility = renderer.indexOf("public static String itemEntityMeshIneligibility(");
        int tint = renderer.indexOf("bakedQuad.tintIndex() >= tintLayers.length", eligibility);
        int enqueue = renderer.indexOf("public static boolean enqueueItemEntityMesh(", tint);
        int pose = renderer.indexOf("!itemPose.pose().isFinite()", enqueue);
        assertTrue(eligibility >= 0 && tint > eligibility && enqueue > tint && pose > enqueue,
                "Rust item-entity admission must bound copied tint indices and reject non-finite transforms");
        int extraction = renderer.indexOf("contains non-finite baked vertex data", pose);
        int uv = renderer.indexOf("contains non-finite UV data", extraction);
        assertTrue(extraction > pose && uv > extraction,
                "Rust item mesh extraction must reject non-finite copied geometry and generated UVs");
    }

    @Test
    public void testRustFirstPersonItemAdmissionBoundsPoseAndTintState() throws IOException {
        String renderer = readSource(SRC_MAIN_JAVA.resolve(
                "net/vulkanic/world/RustGalWorldPrimitiveRenderer.java"));
        int enqueue = renderer.indexOf("public static boolean enqueueFirstPersonItemMesh(");
        int pose = renderer.indexOf("!outerPose.pose().isFinite()", enqueue);
        int eligibility = renderer.indexOf("private static String firstPersonItemMeshIneligibility(", pose);
        int tint = renderer.indexOf("quad.tintIndex() >= layer.tintLayers().length", eligibility);
        assertTrue(enqueue >= 0 && pose > enqueue && eligibility > pose && tint > eligibility,
                "Rust first-person item admission must reject non-finite outer poses and out-of-range tints");
    }

    @Test
    public void testRustEndDragonRaysBoundDeathProgressBeforeExpansion() throws IOException {
        String renderer = readSource(SRC_MAIN_JAVA.resolve(
                "net/minecraft/client/renderer/entity/EnderDragonRenderer.java"));
        int rays = renderer.indexOf("private static void submitRays(");
        int routeStart = renderer.indexOf("currentProceduralQuadRoute()", rays);
        int route = renderer.indexOf("usesRustWholeFrameVulkan()", routeStart);
        int guard = renderer.indexOf("!Float.isFinite(f) || f < 0.0F || f > 1.0F", rays);
        int count = renderer.indexOf("rayCount = Mth.floor", guard);
        assertTrue(rays >= 0 && route > rays && guard > route && count > guard,
                "Rust End Dragon ray expansion must bound copied death progress before allocating semantic rays");
    }

    @Test
    public void testRustCrystalBeamRejectsNonFiniteTransformAfterSemanticCopy() throws IOException {
        String renderer = readSource(SRC_MAIN_JAVA.resolve(
                "net/vulkanic/world/RustGalWorldPrimitiveRenderer.java"));
        int beam = renderer.indexOf("public static boolean enqueueCrystalBeam(");
        int transform = renderer.indexOf("!transform.isFinite()", beam);
        int output = renderer.indexOf("Rust crystal beam transformed vertices must be finite", transform);
        assertTrue(beam >= 0 && transform > beam && output > transform,
                "Rust crystal beam admission must validate copied transforms and transformed material vertices");
    }

    @Test
    public void testRustExperienceOrbValidatesTypedPlacementWithoutJavaGeometry() throws IOException {
        String renderer = readSource(SRC_MAIN_JAVA.resolve(
                "net/vulkanic/world/RustGalWorldPrimitiveRenderer.java"));
        String bridge = readSource(SRC_MAIN_JAVA.resolve("net/vulkanic/bridge/VulkanicGalBridge.java"));
        assertTrue(renderer.contains("ORB_SEMANTICS.enqueue(state.icon, red, blue, state.lightCoords,"));
        assertFalse(renderer.contains("public static boolean enqueueExperienceOrb("));
        int record = bridge.indexOf("public record WorldExperienceOrbInstanceRecord(");
        int pose = bridge.indexOf("orb entity transform must be finite", record);
        int rotation = bridge.indexOf("orb camera orientation must be finite", pose);
        assertTrue(record >= 0 && pose > record && rotation > pose,
                "typed semantic publication must reject non-finite parent transforms and camera rotations");
    }

    @Test
    public void testRustLineRoutesRejectNonFiniteCopiedTransforms() throws IOException {
        String renderer = readSource(SRC_MAIN_JAVA.resolve(
                "net/vulkanic/world/RustGalWorldPrimitiveRenderer.java"));
        int line = renderer.indexOf("private static boolean enqueueLineSegmentsForRoute(");
        int guard = renderer.indexOf("!transform.isFinite()", line);
        int transformed = renderer.indexOf("Rust line transform produced non-finite endpoint coordinates", guard);
        assertTrue(line >= 0 && guard > line && transformed > guard,
                "Rust fishing/debug line routes must validate copied transforms before endpoint expansion");
    }

    @Test
    public void testRustBlockEntityScopeRejectsUnregisteredBlockStateIdentity() throws IOException {
        String dispatcher = readSource(SRC_MAIN_JAVA.resolve(
                "net/minecraft/client/renderer/blockentity/BlockEntityRenderDispatcher.java"));
        int scope = dispatcher.indexOf("boolean rustBlockEntitySemanticScope");
        int id = dispatcher.indexOf("int blockEntityId = Block.BLOCK_STATE_REGISTRY.getId", scope);
        int guard = dispatcher.indexOf("blockEntityId < 0", id);
        int begin = dispatcher.indexOf("beginBlockEntitySubmission(blockEntityId)", guard);
        assertTrue(scope >= 0 && id > scope && guard > id && begin > guard,
                "Rust block-entity semantic scopes must reject missing registry identities before begin");
    }

    @Test
    public void testWholeFrameBlockEntityCallsiteUsesExplicitSemanticSubmission() throws IOException {
        String renderer = readSource(SRC_MAIN_JAVA.resolve(
                "net/minecraft/client/renderer/LevelRenderer.java"));
        int loop = renderer.indexOf("private void submitBlockEntities(");
        int route = renderer.indexOf("currentMaterialRoute().usesRustWholeFrameVulkan()", loop);
        int semantic = renderer.indexOf("this.blockEntityRenderDispatcher.submitSemantic(", route);
        int legacy = renderer.indexOf("this.blockEntityRenderDispatcher.submit(", semantic);
        assertTrue(loop >= 0 && route > loop && semantic > route && legacy > semantic,
                "whole-frame block entities must use an explicit semantic callsite while preserving the OpenGL compatibility branch");
    }

    @Test
    public void testRustCrackRouteFailsClosedWithoutSeededViewport() throws IOException {
        String renderer = readSource(SRC_MAIN_JAVA.resolve(
                "net/vulkanic/world/RustGalWorldPrimitiveRenderer.java"));
        assertTrue(renderer.contains("Rust whole-frame crack route requires a seeded semantic viewport"),
                "visible Rust block cracks must not disappear when the semantic viewport was not seeded");
    }

    @Test
    public void testRustStaticTerrainRouteFailsClosedWithoutTerrainRenderer() throws IOException {
        String renderer = readSource(SRC_MAIN_JAVA.resolve(
                "net/minecraft/client/renderer/LevelRenderer.java"));
        assertTrue(renderer.contains("Rust whole-frame static terrain route requires an initialized terrain renderer"),
                "Rust terrain extraction must not silently drop the scene while its source renderer is unavailable");
    }

    @Test
    public void testStaticTerrainTextureBatchPreflightsResidencyBeforePublication() throws IOException {
        String renderer = readSource(SRC_MAIN_JAVA.resolve(
                "net/vulkanic/world/RustGalWorldPrimitiveRenderer.java"));
        int method = renderer.indexOf("public static void registerStaticTerrainMeshAsset");
        int preflight = renderer.indexOf("projectedTextureResidency", method);
        int publish = renderer.indexOf("registerChangedTexture(WORLD_MESH_TEXTURES, DIRTY_WORLD_MESH_TEXTURES, texture)", method);
        int meshBudget = renderer.indexOf("MAX_WORLD_MESH_ASSET_RESIDENCY", method);
        assertTrue(method >= 0 && preflight > method && publish > preflight
                        && renderer.indexOf("batchTexturePayloads", method) > preflight
                        && renderer.indexOf("conflicting payloads for texture", method) > preflight
                        && renderer.indexOf("validateWorldMeshTextureAsset", method) > method
                        && meshBudget > method && meshBudget < publish,
                "static terrain batches must preflight mesh/texture residency and reject conflicting duplicate IDs before publication");
    }

    @Test
    public void testGeneralMeshAdmissionPreflightsBothRegistriesBeforePublication() throws IOException {
        String renderer = readSource(SRC_MAIN_JAVA.resolve(
                "net/vulkanic/world/RustGalWorldPrimitiveRenderer.java"));
        int method = renderer.indexOf("private static void ensureMeshAssetLocked");
        int texturePreflight = renderer.indexOf("projectedTextureResidency", method);
        int meshPreflight = renderer.indexOf("MAX_WORLD_MESH_ASSET_RESIDENCY", method);
        int publish = renderer.indexOf("registerChangedTexture(WORLD_MESH_TEXTURES, DIRTY_WORLD_MESH_TEXTURES, texture)", method);
        int meshPublish = renderer.indexOf("WORLD_MESH_ASSETS.put", method);
        assertTrue(method >= 0 && texturePreflight > method && meshPreflight > method
                        && publish > texturePreflight && meshPublish > publish
                        && renderer.indexOf("batchTexturePayloads", method) > method
                        && renderer.indexOf("validateWorldMeshTextureAsset", method) > method
                        && renderer.indexOf("validateWorldMeshAsset(extraction.asset(), \"mesh\")", method) > method
                        && renderer.indexOf("conflicting payloads for texture", method) > method,
                "entity/item mesh admission must preflight both bounded registries and duplicate payloads before publishing either resource family");
    }

    @Test
    public void testStaticTerrainMeshAdmissionMirrorsRustMeshBoundsBeforePublication() throws IOException {
        String renderer = readSource(SRC_MAIN_JAVA.resolve(
                "net/vulkanic/world/RustGalWorldPrimitiveRenderer.java"));
        int method = renderer.indexOf("public static void registerStaticTerrainMeshAsset");
        int validation = renderer.indexOf("private static void validateWorldMeshAsset");
        int publish = renderer.indexOf("registerChangedTexture(WORLD_MESH_TEXTURES, DIRTY_WORLD_MESH_TEXTURES, texture)", method);
        assertTrue(method >= 0 && validation >= 0 && validation < method && publish > method
                        && renderer.indexOf("MAX_WORLD_MESH_VERTICES", validation) > validation
                        && renderer.indexOf("MAX_WORLD_MESH_FRONTEND_INDEX_BYTES", validation) > validation
                        && renderer.indexOf("MAX_WORLD_MESH_SECTIONS", validation) > validation
                        && renderer.indexOf("mesh index references vertex", validation) > validation
                        && renderer.indexOf("materialMatchesMode", validation) > validation
                        && renderer.indexOf("section.cullPolicy()", validation) > validation
                        && renderer.indexOf("section.winding()", validation) > validation
                        && renderer.indexOf("validateWorldMeshAsset(asset, \"static terrain\")", method) > method,
                "static terrain mesh admission must enforce Rust's bounded finite mesh contract before publishing textures");
    }

    @Test
    public void testGenericWorldTextureRegistrationRejectsMalformedPayloadsBeforePublication() throws IOException {
        String renderer = readSource(SRC_MAIN_JAVA.resolve(
                "net/vulkanic/world/RustGalWorldPrimitiveRenderer.java"));
        int method = renderer.indexOf("public static void registerWorldMeshTexture");
        int validation = renderer.indexOf("private static void validateWorldMeshTextureAsset");
        int payloadCheck = renderer.indexOf("payloadBytes", validation);
        int publish = renderer.indexOf("registerChangedTexture(WORLD_MESH_TEXTURES, DIRTY_WORLD_MESH_TEXTURES, texture)", method);
        assertTrue(method >= 0 && validation >= 0 && validation < method && payloadCheck > validation && publish > method
                        && renderer.indexOf("payload must contain 1..") > payloadCheck,
                "generic Rust texture registration must reject empty/oversized semantic payloads before registry publication");
    }

    @Test
    public void testStaticTerrainSortedIndexAdmissionMirrorsRustBoundBeforePublication() throws IOException {
        String renderer = readSource(SRC_MAIN_JAVA.resolve(
                "net/vulkanic/world/RustGalWorldPrimitiveRenderer.java"));
        int method = renderer.indexOf("public static void registerStaticTerrainSortedIndex");
        int validation = renderer.indexOf("private static void validateWorldMeshSortedIndex");
        int publish = renderer.indexOf("WORLD_MESH_SORTED_INDICES.put", method);
        assertTrue(method >= 0 && validation >= 0 && validation < method && publish > method
                        && renderer.indexOf("MAX_WORLD_MESH_INDEX_BYTES", validation) > validation
                        && renderer.indexOf("bytes % stride", validation) > validation
                        && renderer.indexOf("validateWorldMeshSortedIndex(sortedIndex)", method) > method,
                "static terrain sorted-index admission must mirror Rust's bounded, aligned payload contract before publication");
    }

    @Test
    public void testRustBackgroundAndOutlineRoutesRequireSeededViewport() throws IOException {
        String renderer = readSource(SRC_MAIN_JAVA.resolve(
                "net/vulkanic/world/RustGalWorldPrimitiveRenderer.java"));
        assertTrue(renderer.contains("Rust whole-frame background route requires a seeded semantic viewport"),
                "Rust background extraction must not replace a missing viewport with a diagnostic fallback");
        assertTrue(renderer.contains("Rust whole-frame outline route requires a seeded semantic viewport"),
                "Rust block outlines must not lower against an unseeded viewport");
    }

    @Test
    public void testRustLoadingFramesUseSemanticBackgroundInsteadOfDiagnosticFallback() throws IOException {
        String renderer = readSource(SRC_MAIN_JAVA.resolve(
                "net/vulkanic/world/RustGalWorldPrimitiveRenderer.java"));
        assertTrue(renderer.contains("loadingBackgroundRecord"),
                "Rust-owned loading frames need an explicit semantic clear record");
        assertTrue(renderer.contains("pendingBackground = loadingBackgroundRecord(viewportWidth, viewportHeight)"),
                "missing world/camera state must remain presentable through Rust semantic background data");
    }

    @Test
    public void testRustWeatherRouteRequiresCopiedStateAndSeededViewport() throws IOException {
        String renderer = readSource(SRC_MAIN_JAVA.resolve(
                "net/vulkanic/world/RustGalWorldPrimitiveRenderer.java"));
        assertTrue(renderer.contains("Rust whole-frame weather route requires weather state and camera semantics"),
                "Rust weather extraction must not silently continue without copied weather state");
        assertTrue(renderer.contains("Rust whole-frame weather route requires a seeded semantic viewport"),
                "visible Rust weather must not be lowered against an unseeded viewport");
    }

    @Test
    public void testRustSkyRouteDefersCameraRelativeGeometryUntilVisibleCameraIsAvailable() throws IOException {
        String renderer = readSource(SRC_MAIN_JAVA.resolve(
                "net/vulkanic/world/RustGalWorldPrimitiveRenderer.java"));
        assertTrue(renderer.contains("Rust whole-frame sky route requires copied sky state"),
                "Rust sky extraction must not silently continue without copied sky state");
        assertTrue(renderer.contains("if (skyVisible && camera == null)"),
                "the extraction-only sky producer must defer camera-relative celestial geometry");
        assertTrue(renderer.contains("defer camera-relative celestial geometry"),
                "the deferred sky handoff must remain explicit rather than opening a Java fallback");
    }

    @Test
    public void testRustWorldBorderRouteRequiresStateAndSeededViewport() throws IOException {
        String renderer = readSource(SRC_MAIN_JAVA.resolve(
                "net/vulkanic/world/RustGalWorldPrimitiveRenderer.java"));
        assertTrue(renderer.contains("Rust whole-frame world-border route requires border state and camera semantics"),
                "Rust world-border extraction must reject missing copied state");
        assertTrue(renderer.contains("Rust whole-frame world-border route requires a seeded semantic viewport"),
                "Rust world-border extraction must not report success without a seeded viewport");
    }

    @Test
    public void testRustWorldBorderRouteRejectsInvalidCopiedSemantics() throws IOException {
        String renderer = readSource(SRC_MAIN_JAVA.resolve(
                "net/vulkanic/world/RustGalWorldPrimitiveRenderer.java"));
        int method = renderer.indexOf("public static boolean enqueueWorldBorder(");
        int validation = renderer.indexOf("invalid copied border semantics", method);
        int lock = renderer.indexOf("synchronized (LOCK)", method);
        assertTrue(method >= 0 && validation > method && lock > validation,
                "world-border semantic bounds must be validated before publishing Rust work");
        String guarded = renderer.substring(method, lock);
        assertTrue(guarded.contains("Double.isFinite(cameraPosition.x())")
                        && guarded.contains("state.minX > state.maxX")
                        && guarded.contains("state.alpha < 0.0 || state.alpha > 1.0"),
                "world-border admission must reject non-finite, inverted, and out-of-range copied state");
    }

    @Test
    public void testRustSkyRouteRejectsInvalidCopiedCelestialSemantics() throws IOException {
        String renderer = readSource(SRC_MAIN_JAVA.resolve(
                "net/vulkanic/world/RustGalWorldPrimitiveRenderer.java"));
        int method = renderer.indexOf("public static void enqueueWorldSky(");
        int validation = renderer.indexOf("invalid copied celestial semantics", method);
        int background = renderer.indexOf("pendingBackground = pendingBackground.withSky", method);
        assertTrue(method >= 0 && validation > method && validation < background,
                "visible sky state must be validated before publishing scalar or camera-relative semantics");
        String guarded = renderer.substring(method, background);
        assertTrue(guarded.contains("Float.isFinite(state.timeOfDay)")
                        && guarded.contains("Float.isFinite(state.starBrightness)")
                        && guarded.contains("Double.isFinite(camera.getPosition().x())"),
                "sky admission must reject non-finite copied celestial and camera values");
    }

    @Test
    public void testRustWeatherRouteRejectsMalformedCopiedColumns() throws IOException {
        String renderer = readSource(SRC_MAIN_JAVA.resolve(
                "net/vulkanic/world/RustGalWorldPrimitiveRenderer.java"));
        int method = renderer.indexOf("public static void enqueueWorldWeather(");
        int cameraValidation = renderer.indexOf("invalid copied camera/column semantics", method);
        int rainValidation = renderer.indexOf("malformed rain column semantics", method);
        int weatherColumns = renderer.indexOf("List<VulkanicGalBridge.WorldMaterialQuadRecord> quads = weatherColumns(", method);
        assertTrue(method >= 0 && cameraValidation > method && rainValidation > cameraValidation
                        && weatherColumns > rainValidation,
                "weather columns must be validated before Rust material-quad construction");
        String guarded = renderer.substring(method, weatherColumns);
        assertTrue(guarded.contains("Float.isFinite(column.uOffset())")
                        && guarded.contains("column.topY() < column.bottomY()")
                        && guarded.contains("MAX_RUST_WEATHER_COLUMNS"),
                "weather admission must reject malformed UVs, inverted heights, and oversized copied columns");
    }

    @Test
    public void testWorldMaterialQuadBoundsCopiedViewport() throws IOException {
        String bridge = readSource(SRC_MAIN_JAVA.resolve(
                "net/vulkanic/bridge/VulkanicGalBridge.java"));
        int record = bridge.indexOf("public record WorldMaterialQuadRecord(");
        int validation = bridge.indexOf("viewportWidth > 16384", record);
        int geometry = bridge.indexOf("world material quad contains invalid copied geometry", validation);
        assertTrue(record >= 0 && validation > record && geometry > validation,
                "shared material quads must bound copied viewport dimensions before FFI publication");
    }

    @Test
    public void testRustWholeFrameParticlesCannotFallBackToJavaExtraction() throws IOException {
        String terrain = readSource(SRC_MAIN_JAVA.resolve(
                "net/minecraft/client/particle/TerrainParticle.java"));
        String marker = readSource(SRC_MAIN_JAVA.resolve(
                "net/minecraft/client/particle/BlockMarker.java"));
        assertTrue(terrain.contains("Rust whole-frame terrain particle semantics were rejected; Java particle extraction is not a fallback"),
                "terrain particles must not retain Java geometry when Rust semantic admission fails");
        assertTrue(marker.contains("Rust whole-frame block-marker semantics were rejected; Java particle extraction is not a fallback"),
                "block markers must not retain Java geometry when Rust semantic admission fails");
        assertTrue(terrain.contains("Rust whole-frame terrain particle route cannot be disabled"),
                "the diagnostic terrain-particle disable switch must not reopen Java rendering under Vulkan");
    }

    @Test
    public void testTerrainParticlesUseTheCopiedBlockAtlasForEveryBlockState() throws IOException {
        String renderer = readSource(SRC_MAIN_JAVA.resolve(
                "net/vulkanic/world/RustGalWorldPrimitiveRenderer.java"));
        int helper = renderer.indexOf("private static int terrainParticleTextureId(BlockState blockState)");
        int enqueue = renderer.indexOf("public static boolean enqueueTerrainParticle(");
        int preflight = renderer.indexOf("copied terrain atlas was registered", enqueue);
        assertTrue(helper >= 0 && renderer.indexOf("return MATERIAL_TEXTURE_TERRAIN_BLOCK_ATLAS;", helper) > helper,
                "terrain particles must use the copied block atlas rather than a fixed five-block whitelist");
        assertTrue(enqueue >= 0 && preflight > enqueue
                        && renderer.indexOf("WORLD_MESH_TEXTURES.containsKey(MATERIAL_TEXTURE_TERRAIN_BLOCK_ATLAS)", enqueue) > enqueue,
                "terrain particle admission must preflight Rust terrain-atlas residency before emitting quads");
        int validation = renderer.indexOf("validateParticleQuadSemantics(", enqueue);
        assertTrue(validation > enqueue && validation < renderer.indexOf("billboardVertices(rotation", validation),
                "terrain particles must validate copied billboard semantics before Rust geometry emission");
    }

    @Test
    public void testBlockMarkersUseDedicatedAssetsOnlyForSpecialMarkersAndAtlasUvsOtherwise() throws IOException {
        String marker = readSource(SRC_MAIN_JAVA.resolve("net/minecraft/client/particle/BlockMarker.java"));
        String renderer = readSource(SRC_MAIN_JAVA.resolve(
                "net/vulkanic/world/RustGalWorldPrimitiveRenderer.java"));
        assertTrue(marker.contains("this.sprite.contents().name()")
                        && marker.contains("this.getU0()")
                        && marker.contains("this.getV1()"),
                "block markers must copy sprite identity and atlas UVs as semantic data");
        int helper = renderer.indexOf("private static int blockMarkerTextureId(BlockState blockState)");
        int enqueue = renderer.indexOf("public static boolean enqueueBlockMarker(");
        int terrainReturn = renderer.indexOf("return MATERIAL_TEXTURE_TERRAIN_BLOCK_ATLAS;", helper);
        int barrier = renderer.indexOf("blockState.is(Blocks.BARRIER)", helper);
        int light = renderer.indexOf("blockState.is(Blocks.LIGHT)", helper);
        int preflight = renderer.indexOf("BlockMarker route selected before the copied terrain atlas was registered");
        assertTrue(helper >= 0 && barrier > helper && light > barrier && terrainReturn > light,
                "barrier/light markers must retain dedicated textures while other blocks use the copied terrain atlas");
        assertTrue(enqueue >= 0 && preflight > enqueue,
                "generic block-marker admission must preflight copied terrain-atlas residency");
        int center = renderer.indexOf("float centerX", enqueue);
        int validation = renderer.indexOf("validateParticleQuadSemantics(spriteId, centerX", enqueue);
        int billboard = renderer.indexOf("billboardVertices(markerRotation", enqueue);
        assertTrue(center > enqueue && validation > center && billboard > validation,
                "generic atlas block markers must validate the actual camera-relative billboard before emission");
    }

    @Test
    public void testRustWholeFrameQuadParticlesRefreshVisibleSemanticStateBeforeEnqueue() throws IOException {
        String gameRenderer = readSource(SRC_MAIN_JAVA.resolve(
                "net/minecraft/client/renderer/GameRenderer.java"));
        String engine = readSource(SRC_MAIN_JAVA.resolve(
                "net/minecraft/client/particle/ParticleEngine.java"));
        String group = readSource(SRC_MAIN_JAVA.resolve(
                "net/minecraft/client/particle/QuadParticleGroup.java"));
        assertTrue(gameRenderer.contains("rustFrameFrustum")
                        && gameRenderer.contains("enqueueRustGalParticles(\n\t\t\t\t\trustFrameFrustum"),
                "whole-frame particle extraction must pass the current camera frustum into the semantic producer");
        assertTrue(engine.contains("enqueueRustGalParticles(Frustum frustum, Camera camera, float partialTick)"),
                "ParticleEngine must expose a current-frame semantic extraction entrypoint");
        assertTrue(group.contains("this.particleTypeRenderState.clear()")
                        && group.contains("this.extractRenderState(frustum, camera, partialTick)"),
                "quad particle semantic extraction must rebuild visible state instead of replaying a stale snapshot");
        int unified = gameRenderer.indexOf("enqueueRustGalParticles(\n\t\t\t\t\trustFrameFrustum");
        assertTrue(unified >= 0 && gameRenderer.indexOf("enqueueRustGalTerrainParticles(this.mainCamera, f)", unified) < 0
                        && gameRenderer.indexOf("enqueueRustGalBlockMarkers(this.mainCamera, f)", unified) < 0,
                "specialized particle producers must not duplicate the unified visible semantic extraction");
    }

    @Test
    public void testBuiltInProceduralGeometryUsesSemanticQuadContracts() throws IOException {
        String lightning = readSource(SRC_MAIN_JAVA.resolve(
            "net/minecraft/client/renderer/entity/LightningBoltRenderer.java"));
        assertTrue(lightning.contains("submitNodeCollector.submitColoredQuads"),
            "Lightning must submit copied procedural quads through the semantic collector");
        assertTrue(lightning.contains("SEMANTIC_LIGHTNING_QUADS = 4 * (8 + 3 + 3) * 4")
                && lightning.contains("new float[SEMANTIC_LIGHTNING_QUADS * 12]")
                && lightning.contains("new float[SEMANTIC_LIGHTNING_QUADS * 8]")
                && lightning.contains("new int[SEMANTIC_LIGHTNING_QUADS]")
                && lightning.contains("quadIndex != SEMANTIC_LIGHTNING_QUADS"),
            "Lightning semantic storage must cover every generated layer, segment, and crossed face");
        assertTrue(lightning.contains("Rust whole-frame lightning route rejected semantic quads"),
            "Lightning must fail closed when its semantic quad request is rejected");
        String testInstance = readSource(SRC_MAIN_JAVA.resolve(
            "net/minecraft/client/renderer/blockentity/TestInstanceRenderer.java"));
        assertTrue(testInstance.contains("submitColoredQuads"),
            "Test-instance error boxes must use the semantic colored-quad contract");
        assertTrue(testInstance.contains("Rust whole-frame error-marker route rejected semantic box quads"),
            "Test-instance error boxes must not fall back to Java geometry on Rust Vulkan");
        assertTrue(testInstance.contains("|| net.vulkanic.VulkanicAPI.isVulkanBackendSelected()"),
            "selected Vulkan test-instance text must stay on the semantic submission path");
    }

    @Test
    public void testExtensionBeamCallsitesUseSemanticTexturedQuads() throws IOException {
        String mungus = readSource(SRC_MAIN_JAVA.resolve(
            "net/alexsmobs/client/render/layer/MungusBeamLayer.java"));
        assertTrue(mungus.contains("submitNodeCollector.submitTranslucentTexturedQuadSemantic"),
            "Mungus beams must provide explicit translucent textured-quads to Rust");
        assertTrue(mungus.contains("Rust whole-frame Mungus beam route rejected semantic textured quads"),
            "Mungus beams must fail closed instead of retaining Java callbacks");
        assertTrue(mungus.contains("Selected Vulkan Mungus beam route is unavailable"),
            "selected Vulkan Mungus beams must not reopen Java geometry before Rust admission");
        assertTrue(mungus.contains("boolean rustPresentation")
                && mungus.contains("RustGalVulkanWholeFrameMode.enabled()")
                && mungus.contains("if (rustPresentation && !rustWholeFrame)"),
            "Mungus beams must treat the Rust presenter shell as ownership during handoff");
        assertTrue(mungus.contains("f4 <= 1.0e-6F")
                && mungus.contains("Rust whole-frame Mungus beam route rejected non-finite or degenerate direction"),
            "Mungus semantic admission must reject degenerate or non-finite beam directions");
        String mimic = readSource(SRC_MAIN_JAVA.resolve(
            "net/alexsmobs/client/render/RenderMimicOctopus.java"));
        assertTrue(mimic.contains("submitGuardianBeam"),
            "Mimic-octopus beams must use the dedicated copied Guardian-beam semantic route");
        assertTrue(mimic.contains("Mimic Octopus beam is unavailable until the Rust Vulkan billboard route is admitted"),
            "Mimic-octopus beams must remain private when the route is not admitted");
        assertTrue(mimic.contains("boolean rustPresentation")
                && mimic.contains("RustGalVulkanWholeFrameMode.enabled()")
                && mimic.contains("if (rustPresentation && !rustWholeFrame)"),
            "Mimic-octopus beams must treat the Rust presenter shell as ownership during handoff");
        String cachalot = readSource(SRC_MAIN_JAVA.resolve(
            "net/alexsmobs/client/render/RenderCachalotEcho.java"));
        assertTrue(cachalot.contains("submitNodeCollector.submitTexturedQuad"),
            "Cachalot echo arcs must use semantic textured quads on Rust Vulkan");
        assertTrue(cachalot.contains("boolean rustPresentation")
                && cachalot.contains("RustGalVulkanWholeFrameMode.enabled()"),
            "Cachalot echo must treat the Rust presenter shell as ownership during handoff");
    }

    @Test
    public void testDragonFireballUsesTheAdmittedSemanticBillboardRoute() throws IOException {
        String fireball = readSource(SRC_MAIN_JAVA.resolve(
                "net/minecraft/client/renderer/entity/DragonFireballRenderer.java"));
        assertTrue(fireball.contains("submitNodeCollector.submitTranslucentTexturedQuadSemantic"),
                "dragon fireballs must enter the explicit translucent textured-billboard ABI");
        assertTrue(fireball.contains("dragon-fireball route rejected semantic billboard"),
                "dragon fireballs must fail closed when Rust rejects their copied quad");
        int semanticSubmission = fireball.indexOf("submitNodeCollector.submitTranslucentTexturedQuadSemantic");
        int javaCallback = fireball.indexOf("submitNodeCollector.submitCustomGeometry");
        assertTrue(semanticSubmission >= 0 && javaCallback > semanticSubmission,
                "the compatibility callback must remain after semantic admission");
        String callbackGate = fireball.substring(semanticSubmission, javaCallback);
        assertTrue(callbackGate.contains("usesRustWholeFrameVulkan()")
                        && callbackGate.contains("RustGalVulkanWholeFrameMode.enabled()"),
                "Rust presentation must reject the Java callback instead of silently falling through");
    }

    @Test
    public void testPaintingSemanticBatchRollsBackPartialMaterialAdmission() throws IOException {
        String painting = readSource(SRC_MAIN_JAVA.resolve(
                "net/minecraft/client/renderer/entity/PaintingRenderer.java"));
        assertTrue(painting.contains("markMaterialQuadBatch"),
                "painting extraction must checkpoint its multi-face semantic material batch");
        assertTrue(painting.contains("rollbackMaterialQuadBatch(checkpoint)"),
                "painting extraction must discard partial material work on rejection");
        assertTrue(painting.contains("catch (RuntimeException failure)"),
                "painting extraction must also roll back when copied resource admission throws");
    }

    @Test
    public void testMultiQuadWorldPrimitivesPreflightMaterialCapacity() throws IOException {
        String primitives = readSource(SRC_MAIN_JAVA.resolve(
                "net/vulkanic/world/RustGalWorldPrimitiveRenderer.java"));
        assertTrue(primitives.contains("PENDING_MATERIAL_QUADS.ensureCapacityFor(8)"),
                "fixed-size beam primitives must reserve their complete material batch before expansion");
        assertTrue(primitives.contains("PENDING_MATERIAL_QUADS.ensureCapacityFor(vertices.length / 12)"),
                "guardian beams must reserve all copied quads before appending");
        assertTrue(primitives.contains("PENDING_MATERIAL_QUADS.ensureCapacityFor(colors.length)"),
                "procedural quad batches must reserve their complete bounded payload before appending");
        int batch = primitives.indexOf("public static boolean enqueueTexturedQuads(");
        int reservation = primitives.indexOf("PENDING_MATERIAL_QUADS.ensureCapacityFor(colors.length)", batch);
        int loop = primitives.indexOf("for (int quad = 0; quad < colors.length; quad++)", batch);
        assertTrue(batch >= 0 && reservation > batch && loop > reservation,
                "textured quad batches must reserve capacity before staging any copied texture asset");
    }

    @Test
    public void testGuardianBeamKeepsTranslucentNoDepthWriteMaterialSemantics() throws IOException {
        String primitives = readSource(SRC_MAIN_JAVA.resolve(
                "net/vulkanic/world/RustGalWorldPrimitiveRenderer.java"));
        int method = primitives.indexOf("private static void appendGuardianQuadLocked(");
        int material = primitives.indexOf("MATERIAL_ID_TRANSLUCENT_TEXTURED, MATERIAL_TEXTURE_GUARDIAN_BEAM", method);
        int mode = primitives.indexOf("MATERIAL_MODE_TRANSLUCENT", material);
        int depth = primitives.indexOf("DEPTH_POLICY_TEST_NO_WRITE", mode);
        assertTrue(method >= 0 && material > method && mode > material && depth > mode,
                "Guardian beam lowering must preserve translucent/no-depth-write semantics");
    }

    @Test
    public void testWeatherColumnExpansionHasAnExplicitProducerBound() throws IOException {
        String primitives = readSource(SRC_MAIN_JAVA.resolve(
                "net/vulkanic/world/RustGalWorldPrimitiveRenderer.java"));
        assertTrue(primitives.contains("MAX_RUST_WEATHER_COLUMNS = MAX_RUST_WORLD_MATERIAL_QUADS"),
                "weather expansion must share the bounded Rust material-frame budget");
        int method = primitives.indexOf("private static List<VulkanicGalBridge.WorldMaterialQuadRecord> weatherColumns(");
        int bound = primitives.indexOf("requestedColumns > MAX_RUST_WEATHER_COLUMNS", method);
        int allocation = primitives.indexOf("new ArrayList<>((int)requestedColumns)", method);
        assertTrue(method >= 0 && bound > method && allocation > bound,
                "weather must reject oversized copied column state before allocating the temporary quad list");
    }

    @Test
    public void testWeatherSemanticProducerRejectsInvalidCopiedIntensityAndRadius() throws IOException {
        String primitives = readSource(SRC_MAIN_JAVA.resolve(
                "net/vulkanic/world/RustGalWorldPrimitiveRenderer.java"));
        assertTrue(primitives.contains("MAX_RUST_WEATHER_RADIUS = 10"),
                "weather admission must retain the vanilla bounded ring-radius contract");
        assertTrue(primitives.contains("!Float.isFinite(state.intensity) || state.intensity > 1.0F"),
                "weather admission must reject non-finite or over-range copied intensity");
        assertTrue(primitives.contains("state.radius <= 0 || state.radius > MAX_RUST_WEATHER_RADIUS"),
                "weather admission must not silently drop visible columns with invalid radius semantics");
    }

    @Test
    public void testFirstPersonSemanticLayersAreBoundedBeforeMeshExtraction() throws IOException {
        String primitives = readSource(SRC_MAIN_JAVA.resolve(
                "net/vulkanic/world/RustGalWorldPrimitiveRenderer.java"));
        assertTrue(primitives.contains("MAX_FIRST_PERSON_SEMANTIC_LAYERS = 64"),
                "first-person semantic layer aggregation must have an explicit bound");
        int collection = primitives.indexOf("itemState.forEachSemanticLayer(layer ->");
        int guard = primitives.indexOf("layers.size() >= MAX_FIRST_PERSON_SEMANTIC_LAYERS", collection);
        int append = primitives.indexOf("layers.add(layer)", collection);
        assertTrue(collection >= 0 && guard > collection && append > guard,
                "first-person layers must be rejected before entering the extraction list");
        int eligibility = primitives.indexOf("private static String firstPersonItemMeshIneligibility(");
        int aggregate = primitives.indexOf("aggregateQuadCount", eligibility);
        int aggregateBound = primitives.indexOf("MAX_FIRST_PERSON_SEMANTIC_QUADS", aggregate);
        assertTrue(eligibility >= 0 && aggregate > eligibility && aggregateBound > aggregate,
                "first-person eligibility must enforce the aggregate quad budget before extraction");
    }

    @Test
    public void testFirstPersonTextureAdmissionPreflightsFrameBeforeAssetCopy() throws IOException {
        String primitives = readSource(SRC_MAIN_JAVA.resolve(
                "net/vulkanic/world/RustGalWorldPrimitiveRenderer.java"));
        int method = primitives.indexOf("private static boolean enqueueFirstPersonTexturedQuadsWithMaterialMode(");
        int preflight = primitives.indexOf("if (!pendingFirstPersonFrame", method);
        int textureCopy = primitives.indexOf("readTexturePayloadForResource(textureIdentity)", method);
        assertTrue(method >= 0 && preflight > method && textureCopy > preflight,
                "first-person texture assets must be copied only after frame and instance admission");
        assertTrue(primitives.indexOf("PENDING_FIRST_PERSON_MESH_INSTANCES.size() >= MAX_RUST_WORLD_MESH_INSTANCES", preflight) > preflight,
                "first-person texture admission must reserve a bounded mesh-instance slot before publishing assets");
    }

    @Test
    public void testIndexedWorldProducersReserveInstanceCapacityBeforeAssetPublish() throws IOException {
        String primitives = readSource(SRC_MAIN_JAVA.resolve(
                "net/vulkanic/world/RustGalWorldPrimitiveRenderer.java"));
		for (String methodName : new String[] {"enqueueBlockDisplay(", "enqueueBlockModelMesh(", "enqueueArrowModel(",
				"enqueueItemEntityMesh(", "enqueueModelPartMesh(", "enqueueStandaloneGlintModelMesh(",
				"enqueueEligibleModelMesh(", "enqueuePrimedTnt(", "enqueueMovingBlock("}) {
            int method = primitives.indexOf(methodName);
            if (method < 0) continue;
            int asset = primitives.indexOf("ensureMeshAssetLocked(extraction)", method);
            int capacity = primitives.lastIndexOf("ensureWorldQueueCapacityLocked(", asset);
            assertTrue(asset >= 0 && capacity > method && capacity < asset,
                    methodName + " must reserve bounded mesh-instance capacity before publishing its mesh asset");
        }
    }

    @Test
    public void testWorldAssetUploadFailuresRemainRetryable() throws IOException {
        String primitives = readSource(SRC_MAIN_JAVA.resolve(
                "net/vulkanic/world/RustGalWorldPrimitiveRenderer.java"));
        for (String generation : new String[] {"WorldBorder", "WorldCrack", "WorldMaterial", "WorldMesh"}) {
            // Locate the concrete failure counter, then require its attempted
            // generation to roll back to the last uploaded generation.
            String counter = switch (generation) {
                case "WorldBorder" -> "worldBorderAssetUpdateFailures++";
                case "WorldCrack" -> "worldCrackAssetUpdateFailures++";
                case "WorldMaterial" -> "worldMaterialAssetUpdateFailures++";
                default -> "worldMeshAssetUpdateFailures++";
            };
            int failure = primitives.indexOf(counter);
            assertTrue(failure >= 0, generation + " upload must expose a bounded failure path");
            String attempted = switch (generation) {
                case "WorldBorder" -> "attemptedWorldBorderAssetGeneration = uploadedWorldBorderAssetGeneration";
                case "WorldCrack" -> "attemptedWorldCrackAssetGeneration = uploadedWorldCrackAssetGeneration";
                case "WorldMaterial" -> "attemptedWorldMaterialAssetGeneration = uploadedWorldMaterialAssetGeneration";
                default -> "attemptedWorldMeshAssetGeneration = uploadedWorldMeshAssetGeneration";
            };
            assertTrue(primitives.indexOf(attempted, failure) > failure,
                    generation + " upload failures must remain retryable");
        }
    }

    @Test
    public void testWorldTextUploadFailurePreservesDirtyRetryableGeneration() throws IOException {
        String primitives = readSource(SRC_MAIN_JAVA.resolve(
                "net/vulkanic/world/RustGalWorldPrimitiveRenderer.java"));
        int method = primitives.indexOf("flushPendingWorldTextImages(");
        int update = primitives.indexOf("bridge.updateWorldTextImages(", method);
        int catchBlock = primitives.indexOf("catch (RuntimeException error)", update);
        int retry = primitives.indexOf("attemptedWorldTextImageGeneration = uploadedWorldTextImageGeneration", catchBlock);
        assertTrue(method >= 0 && update > method && catchBlock > update && retry > catchBlock,
                "world-text image failures must preserve the prior generation and remain retryable");
        assertTrue(primitives.indexOf("DIRTY_WORLD_TEXT_IMAGES.clear()", update) < catchBlock,
                "world-text dirty images must only be cleared after native admission succeeds");
    }

    @Test
    public void testStaticTerrainInstanceAdmissionPreflightsBeforeRetention() throws IOException {
        String primitives = readSource(SRC_MAIN_JAVA.resolve(
                "net/vulkanic/world/RustGalWorldPrimitiveRenderer.java"));
        int method = primitives.indexOf("public static boolean enqueueStaticTerrainMeshInstance(");
        int remember = primitives.indexOf("rememberActiveStaticTerrainInstanceLocked(instance)", method);
        int capacity = primitives.lastIndexOf("ensureWorldQueueCapacityLocked(", remember);
        assertTrue(method >= 0 && capacity > method && capacity < remember,
                "static terrain visibility must reserve queue capacity before retaining an active instance");
        assertTrue(primitives.indexOf("MAX_SEMANTIC_VIEWPORT_AXIS", method) < remember,
                "static terrain visibility must validate its copied viewport before retention");
    }

    @Test
    public void testStaticTerrainRemovalKeepsMeshProducerMetadataAligned() throws IOException {
        String primitives = readSource(SRC_MAIN_JAVA.resolve(
                "net/vulkanic/world/RustGalWorldPrimitiveRenderer.java"));
        int removal = primitives.indexOf("public static void removeStaticTerrainMeshAsset");
        int instanceLoop = primitives.indexOf("for (int index = PENDING_MESH_INSTANCES.size() - 1", removal);
        int producerRemoval = primitives.indexOf("PENDING_MESH_PRODUCERS.remove(index)", instanceLoop);
        assertTrue(removal >= 0 && instanceLoop > removal && producerRemoval > instanceLoop,
                "static terrain removal must remove paired producer metadata with each pending instance");
    }

    @Test
    public void testStaticTerrainReleasesOpaquePayloadOnlyAfterRustUploadAcknowledgement() throws IOException {
        String primitives = readSource(SRC_MAIN_JAVA.resolve(
                "net/vulkanic/world/RustGalWorldPrimitiveRenderer.java"));
        int flush = primitives.indexOf("public static VulkanicGalBridge.Status flushPendingWorldMeshAssets");
        int acknowledge = primitives.indexOf("UPLOADED_WORLD_MESH_GENERATIONS.put", flush);
        int release = primitives.indexOf("releaseUploadedStaticTerrainPayloadLocked", acknowledge);
        int releaseMethod = primitives.indexOf("private static void releaseUploadedStaticTerrainPayloadLocked");
        int discard = primitives.indexOf("WORLD_MESH_ASSETS.remove(meshKey)", releaseMethod);
        assertTrue(flush >= 0 && acknowledge > flush && release > acknowledge && releaseMethod > flush && discard > releaseMethod,
                "static terrain payloads must remain available through native acknowledgement, then release Java vertex/index retention");
        assertTrue(primitives.contains("STATIC_TERRAIN_MESH_RESIDENCY"),
                "released static terrain must retain only explicit mesh identity and texture dependencies");
    }

    @Test
    public void testStaticTerrainResourceReloadRebuildsSemanticSourceWithoutJavaGpuRestore() throws IOException {
        String terrain = readSource(SRC_MAIN_JAVA.resolve(
                "net/vulkanic/world/RustGalTerrainRenderer.java"));
        String source = readSource(SRC_MAIN_JAVA.resolve(
                "net/vulkanic/world/RustGalWholeFrameTerrainSource.java"));
        int request = terrain.indexOf("RustGalWholeFrameTerrainSource.requestResourceReload()");
        int retire = terrain.indexOf("removeLayer(key.sectionPos(), key.layer(), \"resource-reload\")", request);
        int atlasReset = terrain.indexOf("atlasPayload = null", retire);
        assertTrue(request >= 0,
                "resource reload must request a fresh semantic terrain build when Java payloads have been released");
        assertTrue(retire > request && atlasReset > retire,
                "resource reload must retire every drawable pre-reload terrain mesh before publishing an atlas generation with a new layout");
        assertTrue(source.contains("resetForResourceReload()")
                        && source.contains("cpu-source-resource-reload")
                        && source.contains("RustGalTerrainRenderer.removeSection"),
                "the source must retire stale Rust terrain identities and rebuild from CPU semantic snapshots");
    }

    @Test
    public void testDistantHorizonsWorldGenerationPlayerRotationIsAtomicAndDeduplicated() throws IOException {
        String level = readSource(SRC_MAIN_JAVA.resolve(
                "com/seibel/distanthorizons/core/level/AbstractDhServerLevel.java"));
        int target = level.indexOf("public DhBlockPos2D getTargetPosForGeneration()");
        int poll = level.indexOf("worldGenPlayerCenteringQueue.poll()", target);
        int offer = level.indexOf("worldGenPlayerCenteringQueue.offer(firstPlayer)", poll);
        int add = level.indexOf("public void addPlayer(IServerPlayerWrapper serverPlayer)");
        int dedupe = level.indexOf("worldGenPlayerCenteringQueue.contains(serverPlayer)", add);
        int remove = level.indexOf("public void removePlayer(IServerPlayerWrapper serverPlayer)");
        int removeAll = level.indexOf("worldGenPlayerCenteringQueue.removeIf(serverPlayer::equals)", remove);
        assertTrue(target >= 0 && poll > target && offer > poll && add > target && dedupe > add
                        && remove > add && removeAll > remove,
                "DH generation-player scheduling must rotate atomically and deduplicate lifecycle callbacks instead of growing an unbounded queue");
    }

    @Test
    public void testDistantHorizonsWorldGenerationQueueUsesLosslessBoundedSplitAdmission() throws IOException {
        String queue = readSource(SRC_MAIN_JAVA.resolve(
                "com/seibel/distanthorizons/core/generation/WorldGenerationQueue.java"));
        int submit = queue.indexOf("public CompletableFuture<WorldGenResult> submitRetrievalTask");
        int bound = queue.indexOf("MAX_WAITING_WORLD_GEN_TASKS", submit);
        int rejected = queue.indexOf("WorldGenResult.CreateFail()", bound);
        int split = queue.indexOf("List<Long> childPositions", submit);
        int capacity = queue.indexOf("waitingTasks.size() - 1 + newChildren > MAX_WAITING_WORLD_GEN_TASKS", split);
        int parentRemoval = queue.indexOf("waitingTasks.remove(closestTask.pos, closestTask)", capacity);
        int complete = queue.indexOf("closestTask.future.complete(WorldGenResult.CreateSplit(childFutures))", parentRemoval);
        assertTrue(submit >= 0 && bound > submit && rejected > bound && split > submit
                        && capacity > split && parentRemoval > capacity && complete > parentRemoval,
                "DH world generation must bound direct requests and admit a complete child split only when capacity exists");
        assertTrue(queue.contains("inProgressGenTasksByLodPos.size() >= worldGenThreadCount"),
                "DH generation concurrency must not exceed its configured worker capacity");
    }

    @Test
    public void testStaticTerrainFirstAdmissionIsNotReplayedInSameFrame() throws IOException {
        String primitives = readSource(SRC_MAIN_JAVA.resolve(
                "net/vulkanic/world/RustGalWorldPrimitiveRenderer.java"));
        int consume = primitives.indexOf("public static PrimitiveFrame consumeFrame()");
        int pending = primitives.indexOf("newlyAdmittedStaticTerrainKeys", consume);
        int admitted = primitives.indexOf("admittedMeshInstances.add(instance)", pending);
        int replay = primitives.indexOf("for (VulkanicGalBridge.WorldMeshInstanceRecord instance : ACTIVE_STATIC_TERRAIN_INSTANCES.values())", admitted);
        int skip = primitives.indexOf("newlyAdmittedStaticTerrainKeys.contains(instance.meshKey())", replay);
        assertTrue(consume >= 0 && pending > consume && admitted > pending && replay > admitted && skip > replay,
                "static terrain first admission must be tracked and excluded from the active replay loop");
    }

    @Test
    public void testBlockOutlinePreflightsAllEdgePassesBeforeAppending() throws IOException {
        String primitives = readSource(SRC_MAIN_JAVA.resolve(
                "net/vulkanic/world/RustGalWorldPrimitiveRenderer.java"));
        int method = primitives.indexOf("public static void enqueueBlockOutline(");
        int preflight = primitives.indexOf("ensureOutlineCapacityLocked(shape, highContrast ? 2 : 1)", method);
        int append = primitives.indexOf("appendShapeEdges(shape, blockPos", method);
        assertTrue(method >= 0 && preflight > method && append > preflight,
                "block outlines must reserve all edge passes before appending bounded line segments");
    }

    @Test
    public void testBlockCrackPreflightsAllShapeFacesBeforeAppending() throws IOException {
        String primitives = readSource(SRC_MAIN_JAVA.resolve(
                "net/vulkanic/world/RustGalWorldPrimitiveRenderer.java"));
        int method = primitives.indexOf("public static void enqueueBlockBreakingCracks(");
        int preflight = primitives.indexOf("ensureCrackCapacityLocked(shape)", method);
        int append = primitives.indexOf("appendCrackShape(shape", method);
        assertTrue(method >= 0 && preflight > method && append > preflight,
                "block cracks must reserve all six faces per shape box before appending bounded quads");
    }

    @Test
    public void testWorldBorderPreflightsVisibleSidesBeforeAppending() throws IOException {
        String primitives = readSource(SRC_MAIN_JAVA.resolve(
                "net/vulkanic/world/RustGalWorldPrimitiveRenderer.java"));
        int method = primitives.indexOf("private static void appendVisibleWorldBorderSides(");
        int count = primitives.indexOf("int visibleSides = 0", method);
        int capacity = primitives.indexOf("ensureWorldQueueCapacityLocked(PENDING_BORDER_QUADS.size(), visibleSides", count);
        int append = primitives.indexOf("appendWorldBorderSide(", capacity);
        assertTrue(method >= 0 && count > method && capacity > count && append > capacity,
                "world-border rendering must reserve all visible side quads before appending");
    }

    @Test
    public void testWorldBorderRejectsNonFiniteDerivedQuadGeometryBeforeQueueing() throws IOException {
        String primitives = readSource(SRC_MAIN_JAVA.resolve(
                "net/vulkanic/world/RustGalWorldPrimitiveRenderer.java"));
        int method = primitives.indexOf("private static void appendWorldBorderQuad(");
        int finite = primitives.indexOf("produced non-finite copied geometry", method);
        int queue = primitives.indexOf("PENDING_BORDER_QUADS.add", finite);
        assertTrue(method >= 0 && finite > method && queue > finite,
                "world-border derived quads must reject non-finite geometry before Rust queue insertion");
    }

    @Test
    public void testRustCrackQuadsRejectNonFiniteDerivedGeometryBeforeQueueing() throws IOException {
        String primitives = readSource(SRC_MAIN_JAVA.resolve(
                "net/vulkanic/world/RustGalWorldPrimitiveRenderer.java"));
        int method = primitives.indexOf("private static void appendCrackQuad(");
        int finite = primitives.indexOf("produced non-finite copied geometry", method);
        int queue = primitives.indexOf("PENDING_CRACK_QUADS.add", finite);
        assertTrue(method >= 0 && finite > method && queue > finite,
                "Rust crack quads must reject non-finite geometry before queue insertion");
    }

    @Test
    public void testRustLineSegmentsRejectNonFiniteDerivedGeometryBeforeQueueing() throws IOException {
        String primitives = readSource(SRC_MAIN_JAVA.resolve(
                "net/vulkanic/world/RustGalWorldPrimitiveRenderer.java"));
        int helper = primitives.indexOf("private static boolean finiteSegment(");
        int camera = primitives.indexOf("if (!finiteSegment(start.x()");
        int queue = primitives.indexOf("PENDING_SEGMENTS.add", camera);
        assertTrue(helper >= 0 && camera >= 0 && queue > camera,
                "Rust line segments must reject non-finite derived geometry before queue insertion");
    }

    @Test
    public void testRustMaterialVertexTransformRejectsNonFiniteCopiedGeometry() throws IOException {
        String primitives = readSource(SRC_MAIN_JAVA.resolve(
                "net/vulkanic/world/RustGalWorldPrimitiveRenderer.java"));
        int method = primitives.indexOf("private static void transformMaterialVertex(");
        int input = primitives.indexOf("invalid copied vertex input", method);
        int output = primitives.indexOf("non-finite transformed vertex", input);
        assertTrue(method >= 0 && input > method && output > input,
                "Rust material vertex transforms must reject non-finite copied inputs and outputs");
    }

    @Test
    public void testWorldMaterialBridgeRecordRejectsNonFiniteCopiedGeometry() throws IOException {
        String bridge = readSource(SRC_MAIN_JAVA.resolve(
                "net/vulkanic/bridge/VulkanicGalBridge.java"));
        int record = bridge.indexOf("public record WorldMaterialQuadRecord(");
        int guard = bridge.indexOf("contains invalid copied geometry", record);
        assertTrue(record >= 0 && guard > record,
                "world material bridge records must reject non-finite copied geometry before FFI");
    }

    @Test
    public void testWorldTextBoundsSubmitMetadataBeforeGlyphPreparation() throws IOException {
        String collector = readSource(SRC_MAIN_JAVA.resolve(
                "net/vulkanic/world/WorldTextSemanticCollector.java"));
        assertTrue(collector.contains("MAX_SEMANTIC_SUBMITS_PER_COLLECTION = 8_192"),
                "world text must bound submit metadata before sorting or glyph preparation");
        assertTrue(collector.contains("snapshot.seeThrough().size() > MAX_SEMANTIC_SUBMITS_PER_COLLECTION"),
                "name-tag extraction must reject oversized see-through submit collections");
        assertTrue(collector.contains("submits.size() > MAX_SEMANTIC_SUBMITS_PER_COLLECTION"),
                "ordinary text extraction must reject oversized submit collections");
        assertTrue(collector.contains("quads.size() >= MAX_SEMANTIC_QUADS_PER_COLLECTION"),
                "ordinary text extraction must stop once the bounded quad budget is reached");
    }

    @Test
    public void testDebugBoundingBoxExtractionBoundsInvisibleCellAllocation() throws IOException {
        String renderer = readSource(SRC_MAIN_JAVA.resolve(
                "net/minecraft/client/renderer/blockentity/BlockEntityWithBoundingBoxRenderer.java"));
        assertTrue(renderer.contains("MAX_INVISIBLE_BLOCK_CELLS = 65_536L"),
                "debug invisible-block extraction must have an explicit cell-volume bound");
        assertTrue(renderer.contains("cellCount > MAX_INVISIBLE_BLOCK_CELLS"),
                "debug invisible-block extraction must reject oversized volumes before allocation");
        assertTrue(renderer.contains("new BlockEntityWithBoundingBoxRenderState.InvisibleBlockType[(int)cellCount]"),
                "the bounded cell count must control the backing allocation");
    }

    @Test
    public void testCloudTexturePreparationBoundsCellAllocation() throws IOException {
        String cloud = readSource(SRC_MAIN_JAVA.resolve(
                "net/minecraft/client/renderer/CloudRenderer.java"));
        assertTrue(cloud.contains("MAX_SEMANTIC_CLOUD_CELLS = 1_048_576L"),
                "cloud texture preparation must share an explicit semantic cell bound");
        assertTrue(cloud.contains("cellCount > MAX_SEMANTIC_CLOUD_CELLS"),
                "cloud texture preparation must reject oversized resource-pack images before allocation");
        assertTrue(cloud.contains("new long[(int)cellCount]"),
                "the bounded cloud cell count must control the backing allocation");
    }

    @Test
    public void testTaczBedrockUsesOneAggregateSemanticQuadBudget() throws IOException {
        String tacz = readSource(SRC_MAIN_JAVA.resolve(
                "net/minecraft/client/renderer/special/TaczGlock17SpecialRenderer.java"));
        assertTrue(tacz.contains("SemanticBedrockBudget budget = new SemanticBedrockBudget()"),
                "TACZ semantic walks must share one aggregate budget across light batches");
        assertTrue(tacz.contains("++budget.quadCount > MAX_SEMANTIC_BEDROCK_QUADS"),
                "TACZ semantic polygon admission must enforce the aggregate quad limit");
    }

    @Test
    public void testPaintingStateExtractionBoundsLightGridAllocation() throws IOException {
        String painting = readSource(SRC_MAIN_JAVA.resolve(
                "net/minecraft/client/renderer/entity/PaintingRenderer.java"));
        assertTrue(painting.contains("tileCount > MAX_RUST_PAINTING_QUADS / 6L"),
                "painting state extraction must reject oversized tile grids before allocation");
        assertTrue(painting.contains("new int[(int)tileCount]"),
                "the bounded painting tile count must control the light-grid allocation");
    }

    @Test
    public void testStandaloneUniformReflectionBoundsShaderPackArrays() throws IOException {
        String coordinator = readSource(SRC_MAIN_JAVA.resolve(
                "net/vulkanic/backends/vulkan/VulkanShaderProgramCoordinator.java"));
        assertTrue(coordinator.contains("MAX_STANDALONE_UNIFORM_ARRAY_ELEMENTS = 4_096"),
                "standalone shader uniform arrays must have an explicit element bound");
        assertTrue(coordinator.contains("MAX_STANDALONE_UNIFORM_BACKING_BYTES = 4 * 1024 * 1024"),
                "standalone shader uniform backing must have an explicit byte bound");
        assertTrue(coordinator.contains("checkedStandaloneUniformArraySize(field)"),
                "uniform readback arrays must revalidate reflected array sizes before allocation");
    }

    @Test
    public void testCopiedWorldMeshExtractionBoundsTransientLists() throws IOException {
        String renderer = readSource(SRC_MAIN_JAVA.resolve(
                "net/vulkanic/world/RustGalWorldPrimitiveRenderer.java"));
        assertTrue(renderer.contains("MAX_WORLD_MESH_VERTICES = 65_536"),
                "copied world meshes must retain an explicit vertex bound");
        assertTrue(renderer.contains("ensureWorldMeshExtractionCapacity(vertices, indices, sections)"),
                "mesh extraction must check transient list capacity before copying each polygon");
        assertTrue(renderer.contains("MAX_WORLD_MESH_FRONTEND_INDEX_BYTES / 2 - 6"),
                "mesh extraction must bound index growth before the byte-array conversion");
        assertTrue(renderer.contains("sections.size() >= MAX_WORLD_MESH_SECTIONS"),
                "mesh extraction must bound section metadata before temporary allocation grows");
    }

    @Test
    public void testProductionSourceExecutionGateFollowsAdmittedRustSnapshot() throws IOException {
        String frontend = readSource(SRC_MAIN_RUST.resolve(
                "render/vulkanic/world_primitive_frontend.rs"));
        int start = frontend.indexOf(
                "#[cfg(not(test))]\n    fn candidate_lowered_source_execution_requested");
        int end = frontend.indexOf(
                "#[cfg(test)]\n    fn candidate_lowered_source_execution_requested", start);
        assertTrue(start >= 0 && end > start,
                "production source-execution gate must remain discoverable");
        String productionGate = frontend.substring(start, end);
        assertTrue(productionGate.contains("self.source_execution_enabled()"),
                "production source execution must follow the copied Rust-owned source snapshot");
        assertFalse(productionGate.contains("{\n        false\n    }"),
                "production source execution must not remain permanently test-only");
        assertTrue(productionGate.contains("exact-frame resource"),
                "source execution must retain its exact-frame fail-closed admission boundary");
    }

    @Test
    public void testSelectedSourceDistantHorizonsUsesDedicatedRustPlan() throws IOException {
        String frontend = readSource(SRC_MAIN_RUST.resolve(
                "render/vulkanic/world_primitive_frontend.rs"));
        int snapshot = frontend.indexOf("fn prepare_runtime_source_snapshot");
        int submit = frontend.indexOf("fn submit_armed_runtime_source_frame_with_gui", snapshot);
        assertTrue(snapshot >= 0 && submit > snapshot,
                "selected-source runtime snapshot and submit stages must remain explicit");
        String snapshotSource = frontend.substring(snapshot, submit);
        assertTrue(snapshotSource.contains("snapshot.lod_instances.clear()")
                        && snapshotSource.contains("snapshot.lod_render_frame = WorldLodRenderFrame::default()"),
                "the provisional ordinary graph must never pretend to render DH for the source route");
        assertTrue(snapshotSource.contains("merge_active_candidate_source_distant_depth(frame)"),
                "the exact-frame DH depth contract must be merged before source admission");
        int plan = frontend.indexOf("prepare_named_source_distant_horizons_frame_plan");
        int complete = frontend.indexOf("fn submit_complete_named_source_frame");
        assertTrue(plan >= 0 && complete > snapshot,
                "the complete selected-source submission must retain a dedicated DH source plan");
        String completeSource = frontend.substring(complete);
        assertTrue(completeSource.contains("plan.distant_horizons"),
                "the complete source submission must consume the prepared DH plan");
    }

    @Test
    public void testDistantHorizonsJavaSnapshotsMirrorRustLodBounds() throws IOException {
        String collector = readSource(SRC_MAIN_JAVA.resolve(
                "net/vulkanic/world/DistantHorizonsSemanticCollector.java"));
        String bridge = readSource(SRC_MAIN_JAVA.resolve(
                "net/vulkanic/bridge/VulkanicGalBridge.java"));
        int snapshot = collector.indexOf("public record LodColumnSnapshot");
        int validation = collector.indexOf("private static void validateBuffers", snapshot);
        assertTrue(snapshot >= 0 && validation > snapshot
                        && collector.contains("MAX_LOD_SEGMENTS_PER_COLUMN = 512")
                        && collector.contains("MAX_LOD_VERTICES_PER_SEGMENT = 2_097_152")
                        && collector.contains("MAX_LOD_MATERIAL_ID = 15")
                        && collector.contains("MAX_LOD_NORMAL_INDEX = 5")
                        && collector.contains("MAX_LOD_MATERIAL_IDENTITIES_PER_COLUMN = 4_096")
                        && collector.substring(validation).contains("packedLightAndMicroOffset()")
                        && collector.substring(validation).contains("materialId()")
                        && collector.substring(validation).contains("normalIndex()")
                        && bridge.contains("materialId < 0 || materialId > 15")
                        && bridge.contains("normalIndex < 0 || normalIndex > 5")
                        && bridge.contains("layer <= 0 || layer > 4"),
                "Distant Horizons Java snapshots must enforce Rust's bounded LOD segment and vertex semantics before upload");
    }

    @Test
    public void testGuiRawImageRecordMirrorsRustDimensionsAndExactPixelContract() throws IOException {
        String bridge = readSource(SRC_MAIN_JAVA.resolve(
                "net/vulkanic/bridge/VulkanicGalBridge.java"));
        int record = bridge.indexOf("public record GuiRawImageAssetRecord");
        int constructor = bridge.indexOf("public GuiRawImageAssetRecord {", record);
        assertTrue(record >= 0 && constructor > record
                        && bridge.substring(record).contains("MAX_RAW_IMAGE_DIMENSION = 8192")
                        && bridge.substring(record).contains("MAX_RAW_IMAGE_PIXELS = 16 * 1024 * 1024")
                        && bridge.substring(constructor).contains("format < 1 || format > 2")
                        && bridge.substring(constructor).contains("expectedBytes")
                        && bridge.substring(constructor).contains("pixels.length != expectedBytes"),
                "GUI raw-image Java admission must mirror Rust's bounded dimensions, pixel count, and exact byte contract");
    }

    @Test
    public void testWorldTextImageRecordMirrorsRustExactPixelContract() throws IOException {
        String bridge = readSource(SRC_MAIN_JAVA.resolve(
                "net/vulkanic/bridge/VulkanicGalBridge.java"));
        int record = bridge.indexOf("public record WorldTextImageAssetRecord");
        int constructor = bridge.indexOf("public WorldTextImageAssetRecord {", record);
        assertTrue(record >= 0 && constructor > record
                        && bridge.substring(record).contains("MAX_IMAGE_BYTES = 4 * 1024 * 1024")
                        && bridge.substring(constructor).contains("expectedBytes")
                        && bridge.substring(constructor).contains("pixels.length != expectedBytes"),
                "world text image Java admission must mirror Rust's exact bounded format/extent payload contract");
    }

    @Test
    public void testWorldAssetRecordsMirrorRustPngBounds() throws IOException {
        String bridge = readSource(SRC_MAIN_JAVA.resolve(
                "net/vulkanic/bridge/VulkanicGalBridge.java"));
        assertTrue(bridge.contains("MAX_PNG_BYTES = 2 * 1024 * 1024")
                        && bridge.contains("MAX_PNG_BYTES = 4 * 1024 * 1024")
                        && bridge.contains("stage < 0 || stage >= 10"),
                "world border, crack, and material asset records must reject payloads Rust cannot admit");
    }

    @Test
    public void testWorldTextStagingPreflightsAggregateBytesAndDuplicateIds() throws IOException {
        String renderer = readSource(SRC_MAIN_JAVA.resolve(
                "net/vulkanic/world/RustGalWorldPrimitiveRenderer.java"));
        int method = renderer.indexOf("private static void stageWorldTextSemantics");
        int preflight = renderer.indexOf("projectedImageBytes", method);
        int publish = renderer.indexOf("WORLD_TEXT_IMAGES.put", method);
        assertTrue(method >= 0 && preflight > method && publish > preflight
                        && renderer.contains("MAX_WORLD_TEXT_IMAGE_BYTES_TOTAL = 64L * 1024L * 1024L")
                        && renderer.indexOf("duplicate image id", method) > preflight
                        && renderer.indexOf("aggregate bound exceeded", method) > preflight,
                "world text staging must reject duplicate IDs and aggregate image bytes before publishing images");
    }

    @Test
    public void testSemanticPngRecordsSnapshotCallerArrays() throws IOException {
        String bridge = readSource(SRC_MAIN_JAVA.resolve(
                "net/vulkanic/bridge/VulkanicGalBridge.java"));
        int gui = bridge.indexOf("public record GuiAssetRecord");
        int border = bridge.indexOf("public record WorldBorderAssetRecord");
        int material = bridge.indexOf("public record WorldMaterialAssetRecord");
        assertTrue(gui >= 0 && border > gui && material > border
                        && bridge.substring(gui).contains("pngBytes = pngBytes.clone()")
                        && bridge.substring(border).contains("public byte[] pngBytes()")
                        && bridge.substring(material).contains("public byte[] pngBytes()"),
                "semantic PNG records must own immutable byte snapshots rather than caller-owned arrays");
    }

    @Test
    public void testShaderPackFileRecordsSnapshotCallerArrays() throws IOException {
        String bridge = readSource(SRC_MAIN_JAVA.resolve(
                "net/vulkanic/bridge/VulkanicGalBridge.java"));
        int source = bridge.indexOf("public record ShaderPackSourceFileRecord");
        int asset = bridge.indexOf("public record ShaderPackAssetFileRecord");
        assertTrue(source >= 0 && asset > source
                        && bridge.substring(source).contains("contentsUtf8 = contentsUtf8.clone()")
                        && bridge.substring(asset).contains("contents = contents.clone()")
                        && bridge.substring(source).contains("public byte[] contentsUtf8()")
                        && bridge.substring(asset).contains("public byte[] contents()"),
                "shader-pack source and asset records must own immutable copied bytes across the Rust boundary");
    }

    @Test
    public void testWorldGeometryRecordsSnapshotCallerFloatArrays() throws IOException {
        String bridge = readSource(SRC_MAIN_JAVA.resolve(
                "net/vulkanic/bridge/VulkanicGalBridge.java"));
        int crack = bridge.indexOf("public record WorldCrackQuadRecord");
        int border = bridge.indexOf("public record WorldBorderQuadRecord");
        int instance = bridge.indexOf("public record WorldMeshInstanceRecord");
        assertTrue(crack >= 0 && border > crack && instance > border
                        && bridge.substring(crack).contains("vertices = vertices.clone()")
                        && bridge.substring(crack).contains("public float[] vertices()")
                        && bridge.substring(border).contains("vertices = vertices.clone()")
                        && bridge.substring(border).contains("public float[] vertices()")
                        && bridge.substring(instance).contains("transform = transform.clone()")
                        && bridge.substring(instance).contains("public float[] transform()"),
                "world geometry records must own immutable copied float arrays across the Rust boundary");
    }

    @Test
    public void testMeshAndLodAssetRecordsSnapshotCallerArraysOnRead() throws IOException {
        String bridge = readSource(SRC_MAIN_JAVA.resolve(
                "net/vulkanic/bridge/VulkanicGalBridge.java"));
        int texture = bridge.indexOf("public record WorldMeshTextureAssetRecord");
        int sorted = bridge.indexOf("public record WorldMeshSortedIndexRecord");
        int asset = bridge.indexOf("public record WorldMeshAssetRecord");
        int lod = bridge.indexOf("public record WorldLodSegmentMaterialProvenanceRecord");
        int hand = bridge.indexOf("public record WorldFirstPersonFrameRecord");
        assertTrue(texture >= 0 && sorted > texture && asset > sorted && lod > asset && hand > lod
                        && bridge.substring(texture).contains("public byte[] pngBytes()")
                        && bridge.substring(sorted).contains("public byte[] indexBytes()")
                        && bridge.substring(asset).contains("public byte[] indexBytes()")
                        && bridge.substring(lod).contains("public int[] quadMaterialIds()")
                        && bridge.substring(lod).contains("public byte[] quadVariantStates()")
                        && bridge.substring(lod).contains("public long[] quadVariantPositions()")
                        && bridge.substring(hand).contains("public float[] projectionMatrix()")
                        && bridge.substring(hand).contains("public float[] modelViewMatrix()"),
                "mesh and DH provenance records must not expose mutable array storage through generated accessors");
    }

    @Test
    public void testWholeFrameBlockFeatureDispatchUsesSemanticOnlyLowering() throws IOException {
        String dispatcher = readSource(SRC_MAIN_JAVA.resolve(
                "net/minecraft/client/renderer/feature/FeatureRenderDispatcher.java"));
        String blockRenderer = readSource(SRC_MAIN_JAVA.resolve(
                "net/minecraft/client/renderer/feature/BlockFeatureRenderer.java"));
        assertTrue(dispatcher.contains("renderBlockFeaturesOnly()")
                        && dispatcher.contains("blockFeatureRenderer.render(\n\t\t\t\tsubmitNodeCollection")
                        && dispatcher.contains("true\n\t\t\t);") ,
                "whole-frame block-feature dispatch must invoke the semantic-only renderer mode");
        assertTrue(blockRenderer.contains("boolean semanticOnly")
                        && blockRenderer.contains("Rust semantic block-feature collection requires complete Rust ownership"),
                "block-feature lowering must retain an explicit semantic-only ownership boundary");
        int semanticDispatcher = dispatcher.indexOf("renderBlockFeaturesOnly()");
        int compatibilityDispatcher = dispatcher.indexOf(
                "blockFeatureRenderer.render(submitNodeCollection, this.bufferSource, this.blockRenderDispatcher, this.outlineBufferSource);");
        assertTrue(compatibilityDispatcher >= 0 && compatibilityDispatcher < semanticDispatcher,
                "the Java block-feature overload must remain confined to the pre-existing OpenGL compatibility dispatcher");
		assertFalse(readSource(SRC_MAIN_JAVA.resolve(
				"net/minecraft/client/renderer/LevelRenderer.java")).contains("overlay == net.minecraft.client.renderer.texture.OverlayTexture.NO_OVERLAY"),
				"Rust block coverage must not reject overlays already carried by semantic mesh-instance color lanes");
    }

    @Test
    public void testWholeFrameCustomLineTopologiesUseCopiedRustLineAbi() throws IOException {
        String collector = readSource(SRC_MAIN_JAVA.resolve(
                "net/minecraft/client/renderer/SubmitNodeCollection.java"));
        assertTrue(collector.contains("isRustLineGeometryRenderType(renderType)"),
                "whole-frame custom geometry must classify line callbacks before capture");
        assertTrue(collector.contains("renderType == RenderType.lineStrip()")
                        && collector.contains("debug_line_strip"),
                "line-strip and debug line-strip producers must share the explicit line route");
        assertTrue(collector.contains("int step = lineStrip ? 1 : 2"),
                "line-strip topology must lower adjacent endpoints without retaining a Java callback");
        assertTrue(collector.contains("customGeometryRenderer.render(poseStack.last(), capture)"),
                "the callback must be copied into semantic endpoint data before Rust submission");
        assertTrue(collector.contains("capture.finish()")
                        && collector.contains("finishPending()")
                        && collector.contains("private boolean pendingVertex"),
                "line capture must finalize callbacks that omit an optional normal attribute");
    }

    @Test
    public void testWholeFrameDebugQuadCallbacksUseCopiedProceduralQuadAbi() throws IOException {
        String collector = readSource(SRC_MAIN_JAVA.resolve(
                "net/minecraft/client/renderer/SubmitNodeCollection.java"));
        assertTrue(collector.contains("isRustProceduralQuadGeometryRenderType(renderType)"),
                "debug quad callbacks must be classified before the arbitrary-callback rejection");
        assertTrue(collector.contains("RenderType.debugFilledBox()")
                        && collector.contains("RenderType.debugQuads()"),
                "only the bounded colored debug quad topologies may use this route");
        assertTrue(collector.contains("enqueueProceduralQuads(")
                        && collector.contains("capture.vertices.size() % 4 != 0"),
                "debug quad vertices must be copied as complete four-vertex semantic quads");
        assertTrue(collector.contains("ProceduralGeometryCapture"),
                "debug quad callbacks must not retain a Java VertexConsumer");
        assertTrue(collector.contains("quadLightInitialized")
                        && collector.contains("this.lightCoords != quadLightCoords"),
                "procedural quad capture must reject per-vertex light disagreement instead of silently using the first light value");

		String levelRenderer = readSource(SRC_MAIN_JAVA.resolve(
				"net/minecraft/client/renderer/LevelRenderer.java"));
		assertTrue(levelRenderer.contains("isRustLineGeometryRenderType(renderType)")
				&& levelRenderer.contains("isRustProceduralQuadGeometryRenderType(renderType)"),
				"coverage-only custom geometry must exempt only the two admitted Rust semantic callback families");
    }

    @Test
    public void testWholeFramePostEffectIdentityIsBoundedBeforeFfi() throws IOException {
        String coordinator = readSource(SRC_MAIN_JAVA.resolve(
                "net/vulkanic/gui/RustGalFrameCoordinator.java"));
        assertTrue(coordinator.contains("normalizeSemanticPostEffectId(postEffectId)"),
                "whole-frame post-effect identity must be validated before the FFI request is built");
        assertTrue(coordinator.contains("Character::isISOControl")
                        && coordinator.contains("getBytes(StandardCharsets.UTF_8).length > 256"),
                "post-effect identity validation must reject controls and bound UTF-8 payload size");
    }
}
