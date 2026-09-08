package net.minecraft.client.gui.render;

import com.google.common.collect.ImmutableMap;
import com.google.common.collect.ImmutableMap.Builder;
import net.blaze3d.ProjectionType;
import net.blaze3d.buffers.GpuBuffer;
import net.blaze3d.buffers.GpuBufferSlice;
import net.blaze3d.pipeline.RenderPipeline;
import net.blaze3d.pipeline.RenderTarget;
import net.blaze3d.platform.Lighting;
import net.blaze3d.platform.Window;
import net.blaze3d.systems.CommandEncoder;
import net.blaze3d.systems.RenderPass;
import net.blaze3d.systems.RenderSystem;
import net.blaze3d.textures.FilterMode;
import net.blaze3d.textures.GpuTexture;
import net.blaze3d.textures.GpuTextureView;
import net.blaze3d.textures.TextureFormat;
import net.blaze3d.vertex.BufferBuilder;
import net.blaze3d.vertex.ByteBufferBuilder;
import net.blaze3d.vertex.MeshData;
import net.blaze3d.vertex.PoseStack;
import net.blaze3d.vertex.VertexFormat;
import net.logging.LogUtils;
import it.unimi.dsi.fastutil.objects.Object2IntMap;
import it.unimi.dsi.fastutil.objects.Object2IntOpenHashMap;
import it.unimi.dsi.fastutil.objects.Object2ObjectOpenHashMap;
import java.nio.ByteBuffer;
import java.util.ArrayList;
import java.util.Collections;
import java.util.Comparator;
import java.util.IdentityHashMap;
import java.util.Iterator;
import java.util.List;
import java.util.Map;
import java.util.OptionalDouble;
import java.util.OptionalInt;
import java.util.Set;
import java.util.Map.Entry;
import java.util.function.Supplier;
import net.minecraft.api.EnvType;
import net.minecraft.api.Environment;
import net.minecraft.SharedConstants;
import net.minecraft.client.Minecraft;
import net.minecraft.client.gui.Font;
import net.minecraft.client.gui.font.TextRenderable;
import net.minecraft.client.gui.navigation.ScreenRectangle;
import net.minecraft.client.gui.render.pip.OversizedItemRenderer;
import net.minecraft.client.gui.render.pip.PictureInPictureRenderer;
import net.minecraft.client.gui.render.pip.Standard3dItemRenderer;
import net.minecraft.client.gui.render.state.BlitRenderState;
import net.minecraft.client.gui.render.state.ColoredRectangleRenderState;
import net.voxelmap.util.FourColoredRectangleRenderState;
import net.minecraft.client.gui.render.state.GlyphRenderState;
import net.minecraft.client.gui.render.state.GuiElementRenderState;
import net.minecraft.client.gui.render.state.GuiItemRenderState;
import net.minecraft.client.gui.render.state.GuiRenderState;
import net.minecraft.client.gui.render.state.TiledBlitRenderState;
import net.minecraft.client.gui.render.state.pip.OversizedItemRenderState;
import net.minecraft.client.gui.render.state.pip.PictureInPictureRenderState;
import net.minecraft.client.gui.render.state.pip.GuiSkinRenderState;
import net.minecraft.client.gui.render.state.pip.GuiBookModelRenderState;
import net.minecraft.client.gui.render.state.pip.GuiSignRenderState;
import net.minecraft.client.gui.render.state.pip.GuiBannerResultRenderState;
import net.minecraft.client.gui.render.state.pip.GuiEntityRenderState;
import net.minecraft.client.renderer.CachedOrthoProjectionMatrixBuffer;
import net.minecraft.client.renderer.MappableRingBuffer;
import net.minecraft.client.renderer.MultiBufferSource;
import net.minecraft.client.renderer.RenderPipelines;
import net.minecraft.client.renderer.SubmitNodeCollector;
import net.minecraft.client.renderer.feature.FeatureRenderDispatcher;
import net.minecraft.client.renderer.item.TrackingItemStackRenderState;
import net.minecraft.client.renderer.texture.OverlayTexture;
import net.minecraft.util.Mth;
import net.math.Axis;
import net.vulkanic.VulkanicAPI;
import net.vulkanic.VulkanicResourceBarriers;
import net.vulkanic.gui.RustGalFrameCoordinator;
import net.vulkanic.gui.RustGalGuiElementRenderState;
import net.vulkanic.gui.RustGalGuiItemRenderer;
import net.vulkanic.gui.RustGalGuiRenderer;
import org.apache.commons.lang3.mutable.MutableBoolean;
import org.jetbrains.annotations.Nullable;
import org.joml.Matrix3x2f;
import org.joml.Matrix4f;
import org.joml.Vector3f;
import org.joml.Vector4f;
import org.lwjgl.system.MemoryUtil;
import org.slf4j.Logger;

@Environment(EnvType.CLIENT)
public class GuiRenderer implements AutoCloseable {
	private static int rustGalLoadingGridConsumerDiagnostics;
	private static final Logger LOGGER = LogUtils.getLogger();
	private static final VulkanicResourceBarriers OFFSCREEN_COLOR_WRITES_VISIBLE_TO_TEXTURE_FETCH = VulkanicResourceBarriers.of(
		VulkanicResourceBarriers.Barrier.TEXTURE_FETCH
	);
	private static final float MAX_GUI_Z = 10000.0F;
	/** Bounds copied PIP inputs before model/atlas expansion on Rust Vulkan. */
	private static final int MAX_RUST_PICTURE_IN_PICTURE_STATES = 1_024;
	public static final float MIN_GUI_Z = 0.0F;
	private static final float GUI_Z_NEAR = 1000.0F;
	public static final int GUI_3D_Z_FAR = 1000;
	public static final int GUI_3D_Z_NEAR = -1000;
	public static final int DEFAULT_ITEM_SIZE = 16;
	private static final int MINIMUM_ITEM_ATLAS_SIZE = 512;
	private static final int MAXIMUM_ITEM_ATLAS_SIZE = VulkanicAPI.getBackendMaxTextureSize();
	public static final int CLEAR_COLOR = 0;
	private static final Comparator<ScreenRectangle> SCISSOR_COMPARATOR = Comparator.nullsFirst(
		Comparator.comparing(ScreenRectangle::top).thenComparing(ScreenRectangle::bottom).thenComparing(ScreenRectangle::left).thenComparing(ScreenRectangle::right)
	);
	private static final Comparator<TextureSetup> TEXTURE_COMPARATOR = Comparator.nullsFirst(Comparator.comparing(TextureSetup::getSortKey));
	private static final Comparator<GuiElementRenderState> ELEMENT_SORT_COMPARATOR = Comparator.comparing(GuiElementRenderState::scissorArea, SCISSOR_COMPARATOR)
		.thenComparing(GuiElementRenderState::pipeline, Comparator.comparing(RenderPipeline::getSortKey))
		.thenComparing(GuiElementRenderState::textureSetup, TEXTURE_COMPARATOR);
	private final Map<Object, GuiRenderer.AtlasPosition> atlasPositions = new Object2ObjectOpenHashMap<>();
	private final Map<Object, OversizedItemRenderer> oversizedItemRenderers = new Object2ObjectOpenHashMap<>();
	private final Map<Object, Standard3dItemRenderer> standard3dItemRenderers = new Object2ObjectOpenHashMap<>();
	/** States selected before GUI prepare; selected states never enter Java PIP. */
	private final Set<GuiItemRenderState> rustOwnedStandard3dItems = Collections.newSetFromMap(new IdentityHashMap<>());
	/** Picture-in-picture states already copied into Rust GUI meshes this frame. */
	private final Set<PictureInPictureRenderState> rustOwnedPictureInPictureStates = Collections.newSetFromMap(new IdentityHashMap<>());
	final GuiRenderState renderState;
	private final List<GuiRenderer.DrawStep> draws = new ArrayList();
	private final List<GuiRenderer.PreparedStep> meshesToDraw = new ArrayList();
	private final ByteBufferBuilder byteBufferBuilder = new ByteBufferBuilder(786432);
	private final Map<VertexFormat, MappableRingBuffer> vertexBuffers = new Object2ObjectOpenHashMap<>();
	private int firstDrawIndexAfterBlur = Integer.MAX_VALUE;
	@Nullable
	private final CachedOrthoProjectionMatrixBuffer guiProjectionMatrixBuffer;
	@Nullable
	private final CachedOrthoProjectionMatrixBuffer itemsProjectionMatrixBuffer;
	private final MultiBufferSource.BufferSource bufferSource;
	private final SubmitNodeCollector submitNodeCollector;
	private final FeatureRenderDispatcher featureRenderDispatcher;
	private final Map<Class<? extends PictureInPictureRenderState>, PictureInPictureRenderer<?>> pictureInPictureRenderers;
	@Nullable
	private GpuTexture itemsAtlas;
	@Nullable
	private GpuTextureView itemsAtlasView;
	@Nullable
	private GpuTexture itemsAtlasDepth;
	@Nullable
	private GpuTextureView itemsAtlasDepthView;
	private int itemAtlasX;
	private int itemAtlasY;
	private int cachedGuiScale;
	private int frameNumber;
	@Nullable
	private ScreenRectangle previousScissorArea = null;
	@Nullable
	private RenderPipeline previousPipeline = null;
	@Nullable
	private TextureSetup previousTextureSetup = null;
	@Nullable
	private String previousShaderInputParityGeometryContext = null;
	@Nullable
	private BufferBuilder bufferBuilder = null;

	public GuiRenderer(
		GuiRenderState guiRenderState,
		MultiBufferSource.BufferSource bufferSource,
		SubmitNodeCollector submitNodeCollector,
		FeatureRenderDispatcher featureRenderDispatcher,
		List<PictureInPictureRenderer<?>> list
	) {
		this.renderState = guiRenderState;
		this.bufferSource = bufferSource;
		this.submitNodeCollector = submitNodeCollector;
		this.featureRenderDispatcher = featureRenderDispatcher;
		if (RustGalGuiRenderer.isWholeFrameVulkanEnabled()
			|| VulkanicAPI.isVulkanBackendSelected()) {
			// Rust owns GUI projection and mesh lowering for whole-frame Vulkan;
			// avoid constructing Java compatibility UBOs that the semantic route
			// never consumes.
			this.guiProjectionMatrixBuffer = null;
			this.itemsProjectionMatrixBuffer = null;
		} else {
			this.guiProjectionMatrixBuffer = new CachedOrthoProjectionMatrixBuffer("gui", 1000.0F, 11000.0F, true);
			this.itemsProjectionMatrixBuffer = new CachedOrthoProjectionMatrixBuffer("items", -1000.0F, 1000.0F, true);
		}
		Builder<Class<? extends PictureInPictureRenderState>, PictureInPictureRenderer<?>> builder = ImmutableMap.builder();

		for (PictureInPictureRenderer<?> pictureInPictureRenderer : list) {
			builder.put((Class<? extends PictureInPictureRenderState>)pictureInPictureRenderer.getRenderStateClass(), pictureInPictureRenderer);
		}

		this.pictureInPictureRenderers = builder.buildOrThrow();
	}

	public void incrementFrameNumber() {
		this.frameNumber++;
	}

	/**
	 * Extracts text semantics for the Rust whole-frame route without preparing
	 * Java meshes or issuing a Java draw.
	 */
	public void collectRustGalTextSemantics() {
		this.renderState.forEachText(guiTextRenderState -> {
			int dynamicLayerOrder = this.renderState.currentSemanticLayerOrder(GuiRenderState.SemanticPhase.TEXT);
			List<RustGalGuiElementRenderState> elements = RustGalGuiRenderer.tryEnqueueText(
				guiTextRenderState,
				Minecraft.getInstance().getWindow().getGuiScaledWidth(),
				Minecraft.getInstance().getWindow().getGuiScaledHeight(),
				dynamicLayerOrder
			);
			if (elements != null) {
				for (RustGalGuiElementRenderState element : elements) {
					this.renderState.submitGlyphToCurrentLayer(element);
				}
			} else {
				RustGalGuiRenderer.recordUnsupportedElement("text");
			}
		});
	}

	/**
	 * Converts admitted flat and standard-3D vanilla item semantics for a
	 * Rust-owned whole frame. Once a standard-3D item is selected, its Java PIP
	 * renderer is excluded from that frame rather than drawn a second time.
	 */
	public void collectRustGalItemSemantics() {
		net.minecraft.client.dev.GraphicsAuditGuiFoilTiming.beginFrame();
		int guiWidth = Minecraft.getInstance().getWindow().getGuiScaledWidth();
		int guiHeight = Minecraft.getInstance().getWindow().getGuiScaledHeight();
		this.renderState.forEachItem(guiItemRenderState -> {
			// An item whose copied screen bounds do not intersect the active GUI
			// viewport contributes no visible work.  Omit it before semantic
			// admission; treating an entirely clipped HUD element as an
			// unsupported item would incorrectly reject the whole Rust frame.
			ScreenRectangle bounds = guiItemRenderState.bounds();
			if (bounds != null && (bounds.left() >= guiWidth || bounds.top() >= guiHeight
				|| bounds.right() <= 0 || bounds.bottom() <= 0)) {
				return;
			}
			int dynamicLayerOrder = this.renderState.currentSemanticLayerOrder(GuiRenderState.SemanticPhase.ITEMS);
			boolean hasSpecialRenderer = guiItemRenderState.itemStackRenderState().hasSpecialRenderer();
			List<RustGalGuiElementRenderState> specialElements = hasSpecialRenderer
				? RustGalGuiItemRenderer.tryEnqueueSpecialItem(guiItemRenderState, guiWidth, guiHeight, dynamicLayerOrder)
				: List.of();
			if (hasSpecialRenderer) {
				// A special item has no faithful generic-flat representation. If its
				// explicit Rust collector rejects it, keep the whole family unavailable
				// rather than silently drawing a partial fallback from ordinary layers.
				if (specialElements.isEmpty()) {
					RustGalGuiRenderer.recordUnsupportedElement("item:special-renderer");
				}
				for (RustGalGuiElementRenderState element : specialElements) this.renderState.submitGlyphToCurrentLayer(element);
				return;
			}
			boolean standard3dCandidate = guiItemRenderState.itemStackRenderState().usesBlockLight()
				&& RustGalGuiItemRenderer.standard3dRouteEnabled()
				&& RustGalGuiRenderer.currentExecutionRoute() == RustGalGuiRenderer.GuiExecutionRoute.RUST_VULKAN_WHOLE_FRAME;
			List<RustGalGuiElementRenderState> elements = standard3dCandidate
				? RustGalGuiItemRenderer.tryEnqueueStandard3dItem(guiItemRenderState, guiWidth, guiHeight, dynamicLayerOrder)
				: RustGalGuiItemRenderer.tryEnqueueFlatItem(guiItemRenderState, guiWidth, guiHeight, dynamicLayerOrder);
			if (standard3dCandidate && !elements.isEmpty()) {
				this.rustOwnedStandard3dItems.add(guiItemRenderState);
			}
			if (elements.isEmpty()) {
				RustGalGuiRenderer.recordUnsupportedElement("item");
			}
			for (RustGalGuiElementRenderState element : elements) {
				this.renderState.submitGlyphToCurrentLayer(element);
			}
		});
	}

	/**
	 * Copies oversized-item picture-in-picture states into the same explicit
	 * Rust GUI item routes used by ordinary item nodes. Whole-frame Vulkan never
	 * prepares the Java off-screen PIP renderer, so admitted states must become
	 * owned semantic elements here or remain absent for the frame.
	 */
	public void collectRustGalPictureInPictureSemantics() {
		int guiWidth = Minecraft.getInstance().getWindow().getGuiScaledWidth();
		int guiHeight = Minecraft.getInstance().getWindow().getGuiScaledHeight();
		int[] pictureInPictureCount = {0};
		this.renderState.forEachPictureInPicture(pictureInPictureRenderState -> {
			if (++pictureInPictureCount[0] > MAX_RUST_PICTURE_IN_PICTURE_STATES) {
				throw new IllegalStateException(
					"Rust whole-frame GUI picture-in-picture bound exceeded " + MAX_RUST_PICTURE_IN_PICTURE_STATES
				);
			}
			if (pictureInPictureRenderState instanceof net.minecraft.client.gui.render.state.pip.GuiProfilerChartRenderState chart) {
				int dynamicLayerOrder = this.renderState.currentSemanticLayerOrder(GuiRenderState.SemanticPhase.ELEMENTS);
				List<RustGalGuiElementRenderState> elements = RustGalGuiRenderer.tryEnqueueProfilerChart(
					chart, guiWidth, guiHeight, dynamicLayerOrder
				);
				if (elements != null && !elements.isEmpty()) {
					this.rustOwnedPictureInPictureStates.add(pictureInPictureRenderState);
					for (RustGalGuiElementRenderState element : elements) this.renderState.submitGlyphToCurrentLayer(element);
				} else {
					RustGalGuiRenderer.recordUnsupportedElement("picture-in-picture:profiler-chart");
				}
				return;
			}
			int modelLayerOrder = this.renderState.currentSemanticLayerOrder(GuiRenderState.SemanticPhase.ITEMS);
			if (pictureInPictureRenderState instanceof GuiEntityRenderState entityPip) {
				List<RustGalGuiElementRenderState> elements = RustGalGuiRenderer.tryEnqueueEntityPip(entityPip, modelLayerOrder);
				if (elements != null && !elements.isEmpty()) {
					this.rustOwnedPictureInPictureStates.add(pictureInPictureRenderState);
					for (RustGalGuiElementRenderState element : elements) this.renderState.submitGlyphToCurrentLayer(element);
				} else {
					RustGalGuiRenderer.recordUnsupportedElement("picture-in-picture:entity");
				}
				return;
			}
			if (pictureInPictureRenderState instanceof GuiSkinRenderState skin) {
				List<RustGalGuiElementRenderState> elements = RustGalGuiRenderer.tryEnqueueModelPip(
					skin.playerModel(), skin.texture(), skin.x0(), skin.y0(), skin.x1(), skin.y1(), skin.scale(),
					new Matrix3x2f(), skin.scissorArea(), modelLayerOrder, pose -> {
						pose.translate(0.0F, -skin.pivotY(), 0.0F);
						pose.mulPose(Axis.XP.rotationDegrees(skin.rotationX()));
						pose.translate(0.0F, skin.pivotY(), 0.0F);
						pose.mulPose(Axis.YP.rotationDegrees(-skin.rotationY()));
						pose.translate(0.0F, -1.6010001F, 0.0F);
					});
				if (elements != null && !elements.isEmpty()) {
					this.rustOwnedPictureInPictureStates.add(pictureInPictureRenderState);
					for (RustGalGuiElementRenderState element : elements) this.renderState.submitGlyphToCurrentLayer(element);
				} else {
					RustGalGuiRenderer.recordUnsupportedElement("picture-in-picture:skin");
				}
				return;
			}
			if (pictureInPictureRenderState instanceof GuiBookModelRenderState book) {
				float h = Mth.clamp(Mth.frac(book.flip() + 0.25F) * 1.6F - 0.3F, 0.0F, 1.0F);
				float i = Mth.clamp(Mth.frac(book.flip() + 0.75F) * 1.6F - 0.3F, 0.0F, 1.0F);
				book.bookModel().setupAnim(new net.minecraft.client.model.BookModel.State(0.0F, h, i, book.open()));
				List<RustGalGuiElementRenderState> elements = RustGalGuiRenderer.tryEnqueueModelPip(
					book.bookModel(), book.texture(), book.x0(), book.y0(), book.x1(), book.y1(), book.scale(),
					new Matrix3x2f(), book.scissorArea(), modelLayerOrder, pose -> {
						pose.mulPose(Axis.YP.rotationDegrees(180.0F));
						pose.mulPose(Axis.XP.rotationDegrees(25.0F));
						float open = book.open();
						pose.translate((1.0F - open) * 0.2F, (1.0F - open) * 0.1F, (1.0F - open) * 0.25F);
						pose.mulPose(Axis.YP.rotationDegrees(-(1.0F - open) * 90.0F - 90.0F));
						pose.mulPose(Axis.XP.rotationDegrees(180.0F));
					});
				if (elements != null && !elements.isEmpty()) {
					this.rustOwnedPictureInPictureStates.add(pictureInPictureRenderState);
					for (RustGalGuiElementRenderState element : elements) this.renderState.submitGlyphToCurrentLayer(element);
				} else {
					RustGalGuiRenderer.recordUnsupportedElement("picture-in-picture:book");
				}
				return;
			}
			if (pictureInPictureRenderState instanceof GuiSignRenderState sign) {
				net.minecraft.client.resources.model.Material signMaterial = net.minecraft.client.renderer.Sheets.getSignMaterial(sign.woodType());
				List<RustGalGuiElementRenderState> elements = RustGalGuiRenderer.tryEnqueueModelPip(
					sign.signModel(), signMaterial.texture(), sign.x0(), sign.y0(), sign.x1(), sign.y1(), sign.scale(),
					new Matrix3x2f(), sign.scissorArea(), modelLayerOrder, pose -> pose.translate(0.0F, -0.75F, 0.0F));
				if (elements != null && !elements.isEmpty()) {
					this.rustOwnedPictureInPictureStates.add(pictureInPictureRenderState);
					for (RustGalGuiElementRenderState element : elements) this.renderState.submitGlyphToCurrentLayer(element);
				} else {
					RustGalGuiRenderer.recordUnsupportedElement("picture-in-picture:sign");
				}
				return;
			}
			if (pictureInPictureRenderState instanceof GuiBannerResultRenderState banner) {
				List<RustGalGuiElementRenderState> elements = RustGalGuiRenderer.tryEnqueueBannerPip(banner, modelLayerOrder);
				if (elements != null && !elements.isEmpty()) {
					this.rustOwnedPictureInPictureStates.add(pictureInPictureRenderState);
					for (RustGalGuiElementRenderState element : elements) this.renderState.submitGlyphToCurrentLayer(element);
				} else {
					RustGalGuiRenderer.recordUnsupportedElement("picture-in-picture:banner");
				}
				return;
			}
			if (!(pictureInPictureRenderState instanceof OversizedItemRenderState oversized)) {
				RustGalGuiRenderer.recordUnsupportedElement("picture-in-picture:" + pictureInPictureRenderState.getClass().getName());
				return;
			}
			GuiItemRenderState item = oversized.guiItemRenderState();
			int dynamicLayerOrder = this.renderState.currentSemanticLayerOrder(GuiRenderState.SemanticPhase.ITEMS);
			if (item.itemStackRenderState().hasSpecialRenderer()) {
				List<RustGalGuiElementRenderState> specialElements = RustGalGuiItemRenderer.tryEnqueueSpecialItem(
					item, guiWidth, guiHeight, dynamicLayerOrder
				);
				if (specialElements != null && !specialElements.isEmpty()) {
					this.rustOwnedPictureInPictureStates.add(pictureInPictureRenderState);
					for (RustGalGuiElementRenderState element : specialElements) {
						this.renderState.submitGlyphToCurrentLayer(element);
					}
				} else {
					RustGalGuiRenderer.recordUnsupportedElement("picture-in-picture:oversized-special-item");
				}
				return;
			}
			boolean standard3dCandidate = item.itemStackRenderState().usesBlockLight()
				&& RustGalGuiItemRenderer.standard3dRouteEnabled()
				&& RustGalGuiRenderer.currentExecutionRoute() == RustGalGuiRenderer.GuiExecutionRoute.RUST_VULKAN_WHOLE_FRAME;
			List<RustGalGuiElementRenderState> elements = standard3dCandidate
				? RustGalGuiItemRenderer.tryEnqueueStandard3dItem(item, guiWidth, guiHeight, dynamicLayerOrder)
				: RustGalGuiItemRenderer.tryEnqueueFlatItem(item, guiWidth, guiHeight, dynamicLayerOrder);
			if (!elements.isEmpty()) {
				this.rustOwnedPictureInPictureStates.add(pictureInPictureRenderState);
				if (standard3dCandidate) {
					this.rustOwnedStandard3dItems.add(item);
				}
			}
			for (RustGalGuiElementRenderState element : elements) {
				this.renderState.submitGlyphToCurrentLayer(element);
			}
			if (elements.isEmpty()) {
				RustGalGuiRenderer.recordUnsupportedElement("picture-in-picture:oversized-item");
			}
		});
	}

	/**
	 * Extracts only exact uniform-color GUI rectangles. Unsupported GUI element
	 * families remain unavailable to the whole-frame route instead of being
	 * reconstructed through Java rendering.
	 */
	public void collectRustGalRectangleSemantics() {
		int guiWidth = Minecraft.getInstance().getWindow().getGuiScaledWidth();
		int guiHeight = Minecraft.getInstance().getWindow().getGuiScaledHeight();
		Map<Integer, List<ColoredRectangleRenderState>> largeGroups = new java.util.HashMap<>();
		this.renderState.forEachElement(guiElementRenderState -> {
			if (guiElementRenderState instanceof ColoredRectangleRenderState rectangle) {
				int layer = this.renderState.currentSemanticLayerOrder(GuiRenderState.SemanticPhase.ELEMENTS);
				largeGroups.computeIfAbsent(layer, ignored -> new ArrayList<>()).add(rectangle);
			} else if (guiElementRenderState instanceof FourColoredRectangleRenderState rectangle
				&& rectangle.color00() == rectangle.color10() && rectangle.color00() == rectangle.color01() && rectangle.color00() == rectangle.color11()
				&& rectangle.x0() == Math.rint(rectangle.x0()) && rectangle.y0() == Math.rint(rectangle.y0())
				&& rectangle.x1() == Math.rint(rectangle.x1()) && rectangle.y1() == Math.rint(rectangle.y1())) {
				int layer = this.renderState.currentSemanticLayerOrder(GuiRenderState.SemanticPhase.ELEMENTS);
				largeGroups.computeIfAbsent(layer, ignored -> new ArrayList<>()).add(new ColoredRectangleRenderState(
					rectangle.pipeline(), rectangle.textureSetup(), rectangle.pose(), (int) rectangle.x0(), (int) rectangle.y0(),
					(int) rectangle.x1(), (int) rectangle.y1(), rectangle.color00(), rectangle.color00(), rectangle.scissorArea()));
			}
		}, GuiRenderState.TraverseRange.ALL);
		Set<Integer> groupedLayers = new java.util.HashSet<>();
		this.renderState.forEachElement(guiElementRenderState -> {
			if (guiElementRenderState instanceof net.vulkanic.gui.RustGalLoadingGridRenderState grid) {
				int layer = this.renderState.currentSemanticLayerOrder(GuiRenderState.SemanticPhase.ELEMENTS);
				List<RustGalGuiElementRenderState> elements = RustGalGuiRenderer.tryEnqueueLoadingGrid(grid.colors(), grid.gridSize(), grid.originX(), grid.originY(), grid.cellSize(), grid.stride(), guiWidth, guiHeight);
				if (Boolean.getBoolean("mattmc.dev.graphicsAuditSliceMetrics") && rustGalLoadingGridConsumerDiagnostics++ < 4) {
					System.out.println("[MattMC graphics audit] loading-grid semantic consumer elements=" + (elements == null ? "null" : elements.size()) + " layer=" + layer);
				}
				if (elements != null) for (RustGalGuiElementRenderState element : elements) this.renderState.submitGlyphToCurrentLayer(element);
				else RustGalGuiRenderer.recordUnsupportedElement("loading-grid");
			} else if (guiElementRenderState instanceof FourColoredRectangleRenderState rectangle) {
				int dynamicLayerOrder = this.renderState.currentSemanticLayerOrder(GuiRenderState.SemanticPhase.ELEMENTS);
				List<ColoredRectangleRenderState> group = largeGroups.get(dynamicLayerOrder);
				if (group != null && group.size() >= 1024 && rectangle.color00() == rectangle.color10() && rectangle.color00() == rectangle.color01() && rectangle.color00() == rectangle.color11()
					&& rectangle.x0() == Math.rint(rectangle.x0()) && rectangle.y0() == Math.rint(rectangle.y0()) && rectangle.x1() == Math.rint(rectangle.x1()) && rectangle.y1() == Math.rint(rectangle.y1())) {
					if (groupedLayers.add(dynamicLayerOrder)) {
						List<RustGalGuiElementRenderState> elements = RustGalGuiRenderer.tryEnqueueRectangleGroup(group, guiWidth, guiHeight, dynamicLayerOrder);
						if (elements != null) for (RustGalGuiElementRenderState element : elements) this.renderState.submitGlyphToCurrentLayer(element);
						else RustGalGuiRenderer.recordUnsupportedElement("rectangle-group");
					}
					return;
				}
				List<RustGalGuiElementRenderState> elements = RustGalGuiRenderer.tryEnqueueFourColoredRectangle(
					rectangle, guiWidth, guiHeight, dynamicLayerOrder
				);
				if (elements != null) {
					for (RustGalGuiElementRenderState element : elements) {
						this.renderState.submitGlyphToCurrentLayer(element);
					}
				} else {
					RustGalGuiRenderer.recordUnsupportedElement("four-colored-rectangle");
				}
			} else if (guiElementRenderState instanceof ColoredRectangleRenderState rectangle) {
				int dynamicLayerOrder = this.renderState.currentSemanticLayerOrder(GuiRenderState.SemanticPhase.ELEMENTS);
				List<ColoredRectangleRenderState> group = largeGroups.get(dynamicLayerOrder);
				if (group != null && group.size() >= 1024) {
					if (groupedLayers.add(dynamicLayerOrder)) {
						List<RustGalGuiElementRenderState> elements = RustGalGuiRenderer.tryEnqueueRectangleGroup(group, guiWidth, guiHeight, dynamicLayerOrder);
						if (elements != null) for (RustGalGuiElementRenderState element : elements) this.renderState.submitGlyphToCurrentLayer(element);
						else RustGalGuiRenderer.recordUnsupportedElement("rectangle-group");
					}
					return;
				}
				List<RustGalGuiElementRenderState> elements = RustGalGuiRenderer.tryEnqueueUniformRectangle(
					rectangle, guiWidth, guiHeight, dynamicLayerOrder
				);
				if (elements != null) {
					for (RustGalGuiElementRenderState element : elements) {
						this.renderState.submitGlyphToCurrentLayer(element);
					}
				} else {
					RustGalGuiRenderer.recordUnsupportedElement("rectangle");
				}
			}
		}, GuiRenderState.TraverseRange.ALL);
	}

	/**
	 * Extracts only one-sampler GUI_TEXTURED blits backed by copied resource PNGs
	 * or copied stitched-atlas pixels. All other GUI materials remain absent
	 * rather than crossing the whole-frame boundary through a Java texture view.
	 */
	public void collectRustGalCopiedBlitSemantics() {
		int guiWidth = Minecraft.getInstance().getWindow().getGuiScaledWidth();
		int guiHeight = Minecraft.getInstance().getWindow().getGuiScaledHeight();
		this.renderState.forEachElement(guiElementRenderState -> {
			if (guiElementRenderState instanceof BlitRenderState blit) {
				boolean vignette = blit.pipeline() == RenderPipelines.VIGNETTE;
				int dynamicLayerOrder = this.renderState.currentSemanticLayerOrder(GuiRenderState.SemanticPhase.ELEMENTS);
				List<RustGalGuiElementRenderState> elements = RustGalGuiRenderer.tryEnqueueCopiedBlit(
					blit, guiWidth, guiHeight, dynamicLayerOrder
				);
				if (elements != null) {
					for (RustGalGuiElementRenderState element : elements) {
						this.renderState.submitGlyphToCurrentLayer(element);
					}
				} else {
					RustGalGuiRenderer.recordUnsupportedElement("blit");
					RustGalGuiRenderer.recordUnsupportedElementDetail(
						"blit-source:" + (blit.semanticTexture() == null ? "missing-texture" : blit.semanticTexture())
					);
					RustGalGuiRenderer.recordUnsupportedElementDetail("blit-reason:" + RustGalGuiRenderer.copiedBlitFailureDetail(blit));
				}
			} else if (guiElementRenderState instanceof TiledBlitRenderState tiledBlit) {
				int dynamicLayerOrder = this.renderState.currentSemanticLayerOrder(GuiRenderState.SemanticPhase.ELEMENTS);
				List<RustGalGuiElementRenderState> elements = RustGalGuiRenderer.tryEnqueueTiledCopiedBlit(
					tiledBlit, guiWidth, guiHeight, dynamicLayerOrder
				);
				if (elements != null) {
					for (RustGalGuiElementRenderState element : elements) {
						this.renderState.submitGlyphToCurrentLayer(element);
					}
				} else {
					RustGalGuiRenderer.recordUnsupportedElement("tiled-blit");
					RustGalGuiRenderer.recordUnsupportedElementDetail(
						"tiled-blit-source:" + (tiledBlit.semanticTexture() == null ? "missing-texture" : tiledBlit.semanticTexture())
					);
				}
			} else if (guiElementRenderState instanceof GlyphRenderState glyph) {
				int dynamicLayerOrder = this.renderState.currentSemanticLayerOrder(GuiRenderState.SemanticPhase.TEXT);
				List<RustGalGuiElementRenderState> elements = RustGalGuiRenderer.tryEnqueueGlyph(
					glyph, guiWidth, guiHeight, dynamicLayerOrder
				);
				if (elements != null) {
					for (RustGalGuiElementRenderState element : elements) {
						this.renderState.submitGlyphToCurrentLayer(element);
					}
				} else {
					RustGalGuiRenderer.recordUnsupportedElement("glyph");
				}
			} else if (!(guiElementRenderState instanceof net.vulkanic.gui.RustGalGuiElementRenderState)
				&& !(guiElementRenderState instanceof net.vulkanic.gui.RustGalLoadingGridRenderState)
				&& !(guiElementRenderState instanceof ColoredRectangleRenderState)
				&& !(guiElementRenderState instanceof FourColoredRectangleRenderState)) {
				// The Rust collectors add their own scheduler tokens to the same
				// glyph traversal.  Those are already explicit semantic work and
				// must not be counted again; every other state is a callsite that
				// this whole-frame extraction pass does not understand yet.
				// Record it rather than silently dropping a future GUI family.
				RustGalGuiRenderer.recordUnsupportedElement("unclassified-gui-state");
				RustGalGuiRenderer.recordUnsupportedElementDetail(
					"gui-state-class:" + guiElementRenderState.getClass().getName()
				);
			}
		}, GuiRenderState.TraverseRange.ALL);
	}

	public void render(GpuBufferSlice gpuBufferSlice) {
		if (net.vulkanic.bridge.RustGalVulkanWholeFrameMode.enabled()) {
			throw new IllegalStateException("Java GUI rendering is unavailable while Rust owns whole-frame presentation");
		}
		if (VulkanicAPI.isVulkanBackendSelected()) {
			throw new IllegalStateException("Java Vulkan GUI rendering is unavailable until the Rust whole-frame GUI route is admitted");
		}
		this.prepare();
		this.draw(gpuBufferSlice);

		for (MappableRingBuffer mappableRingBuffer : this.vertexBuffers.values()) {
			mappableRingBuffer.rotate();
		}

		this.draws.clear();
		this.meshesToDraw.clear();
		this.rustOwnedStandard3dItems.clear();
		this.rustOwnedPictureInPictureStates.clear();
		this.renderState.reset();
		this.firstDrawIndexAfterBlur = Integer.MAX_VALUE;
		this.clearUnusedOversizedItemRenderers();
		this.clearUnusedStandard3dItemRenderers();
		if (SharedConstants.DEBUG_SHUFFLE_UI_RENDERING_ORDER) {
			RenderPipeline.updateSortKeySeed();
			TextureSetup.updateSortKeySeed();
		}
	}

	private void clearUnusedOversizedItemRenderers() {
		Iterator<Entry<Object, OversizedItemRenderer>> iterator = this.oversizedItemRenderers.entrySet().iterator();

		while (iterator.hasNext()) {
			Entry<Object, OversizedItemRenderer> entry = (Entry<Object, OversizedItemRenderer>)iterator.next();
			OversizedItemRenderer oversizedItemRenderer = (OversizedItemRenderer)entry.getValue();
			if (!oversizedItemRenderer.usedOnThisFrame()) {
				oversizedItemRenderer.close();
				iterator.remove();
			} else {
				oversizedItemRenderer.resetUsedOnThisFrame();
			}
		}
	}

	private void clearUnusedStandard3dItemRenderers() {
		Iterator<Entry<Object, Standard3dItemRenderer>> iterator = this.standard3dItemRenderers.entrySet().iterator();

		while (iterator.hasNext()) {
			Entry<Object, Standard3dItemRenderer> entry = (Entry<Object, Standard3dItemRenderer>)iterator.next();
			Standard3dItemRenderer standard3dItemRenderer = (Standard3dItemRenderer)entry.getValue();
			if (!standard3dItemRenderer.usedOnThisFrame()) {
				standard3dItemRenderer.close();
				iterator.remove();
			} else {
				standard3dItemRenderer.resetUsedOnThisFrame();
			}
		}
	}

	private void prepare() {
		this.bufferSource.endBatch();
		this.preparePictureInPicture();
		this.prepareItemElements();
		this.prepareText();
		this.renderState.sortElements(ELEMENT_SORT_COMPARATOR);
		this.addElementsToMeshes(GuiRenderState.TraverseRange.BEFORE_BLUR);
		this.firstDrawIndexAfterBlur = this.meshesToDraw.size();
		this.addElementsToMeshes(GuiRenderState.TraverseRange.AFTER_BLUR);
		this.recordDraws();
	}

	private void addElementsToMeshes(GuiRenderState.TraverseRange traverseRange) {
		this.previousScissorArea = null;
		this.previousPipeline = null;
		this.previousTextureSetup = null;
		this.previousScissorArea = null;
		this.previousShaderInputParityGeometryContext = null;
		this.bufferBuilder = null;
		this.renderState.forEachElement(this::addElementToMesh, traverseRange);
		if (this.bufferBuilder != null) {
			this.recordMesh(this.bufferBuilder, this.previousPipeline, this.previousTextureSetup, this.previousScissorArea, this.previousShaderInputParityGeometryContext);
		}
	}

	private void draw(GpuBufferSlice gpuBufferSlice) {
		if (net.vulkanic.bridge.RustGalVulkanWholeFrameMode.enabled()
			|| net.vulkanic.VulkanicAPI.isVulkanBackendSelected()) {
			throw new IllegalStateException(
				"Java GUI render-pass execution is unavailable while Rust owns whole-frame Vulkan presentation"
			);
		}
		if (!this.draws.isEmpty()) {
			Minecraft minecraft = Minecraft.getInstance();
			Window window = minecraft.getWindow();
			net.vulkanic.VulkanicAPI.setProjectionMatrix(
				this.guiProjectionMatrixBuffer.getBuffer((float)window.getWidth() / window.getGuiScale(), (float)window.getHeight() / window.getGuiScale()),
				ProjectionType.ORTHOGRAPHIC
			);
			RenderTarget renderTarget = minecraft.getMainRenderTarget();
			int i = 0;

			for (GuiRenderer.DrawStep step : this.draws) {
				if (step instanceof GuiRenderer.Draw draw && draw.indexCount > i) {
					i = draw.indexCount;
				}
			}

			VulkanicAPI.AutoStorageIndexBuffer autoStorageIndexBuffer = VulkanicAPI.getSequentialBuffer(VertexFormat.Mode.QUADS);
			GpuBuffer gpuBuffer = autoStorageIndexBuffer.getBuffer(i);
			VertexFormat.IndexType indexType = autoStorageIndexBuffer.type();
			GpuBufferSlice gpuBufferSlice2 = VulkanicAPI.getDynamicUniforms()
				.writeTransform(new Matrix4f().setTranslation(0.0F, 0.0F, -11000.0F), new Vector4f(1.0F, 1.0F, 1.0F, 1.0F), new Vector3f(), new Matrix4f(), 0.0F);
			if (this.firstDrawIndexAfterBlur > 0) {
				this.executeDrawRange(
					() -> "GUI before blur",
					renderTarget,
					gpuBufferSlice,
					gpuBufferSlice2,
					gpuBuffer,
					indexType,
					0,
					Math.min(this.firstDrawIndexAfterBlur, this.draws.size())
				);

				if (this.draws.size() > this.firstDrawIndexAfterBlur) {
					VulkanicAPI.createCommandEncoder().clearDepthTexture(renderTarget.getDepthTexture(), 1.0);
					minecraft.gameRenderer.processBlurEffect();
					this.executeDrawRange(
						() -> "GUI after blur",
						renderTarget,
						gpuBufferSlice,
						gpuBufferSlice2,
						gpuBuffer,
						indexType,
						this.firstDrawIndexAfterBlur,
						this.draws.size()
					);
				}
			} else if (this.draws.size() > this.firstDrawIndexAfterBlur) {
				VulkanicAPI.createCommandEncoder().clearDepthTexture(renderTarget.getDepthTexture(), 1.0);
				minecraft.gameRenderer.processBlurEffect();
				this.executeDrawRange(
					() -> "GUI after blur",
					renderTarget,
					gpuBufferSlice,
					gpuBufferSlice2,
					gpuBuffer,
					indexType,
					this.firstDrawIndexAfterBlur,
					this.draws.size()
				);
			}
		}
	}

	static List<RustGalGuiElementRenderState> contiguousRustGalDrawGroup(List<GuiRenderer.DrawStep> draws, int start, int end) {
		if (start < 0 || end < start || end > draws.size()) {
			throw new IndexOutOfBoundsException("invalid Rust GUI draw group range " + start + ".." + end + " for " + draws.size() + " draws");
		}
		List<RustGalGuiElementRenderState> group = new ArrayList<>();
		for (int index = start; index < end; index++) {
			if (!(draws.get(index) instanceof GuiRenderer.RustGalDraw rustGalDraw)) {
				break;
			}
			group.add(rustGalDraw.element());
		}
		return List.copyOf(group);
	}

	private void executeDrawRange(
		Supplier<String> supplier,
		RenderTarget renderTarget,
		GpuBufferSlice gpuBufferSlice,
		GpuBufferSlice gpuBufferSlice2,
		GpuBuffer gpuBuffer,
		VertexFormat.IndexType indexType,
		int i,
		int j
	) {
		Minecraft minecraft = Minecraft.getInstance();
		MutableBoolean rustGalFrameExecuted = new MutableBoolean(false);
		int k = i;
		while (k < j) {
			GuiRenderer.DrawStep step = this.draws.get(k);
			if (step instanceof GuiRenderer.RustGalDraw) {
				if (!rustGalFrameExecuted.booleanValue()) {
					List<RustGalGuiElementRenderState> rustGalDrawGroup = contiguousRustGalDrawGroup(this.draws, k, j);
					if (rustGalDrawGroup.isEmpty()) {
						throw new IllegalStateException("Rust GUI draw marker produced an empty contiguous group");
					}
					// The borrowed OpenGL backend discovers its target from the current
					// OpenGL state; it is never handed a Java framebuffer handle.  Keep
					// the normal GUI color/depth attachment scope active while Rust
					// acquires its explicit GAL frame.  Without that scope acquisition
					// observes the window framebuffer, and the later main-target blit
					// correctly overwrites the Rust GUI work.
					//
					// This scope binds an attachment only.  It issues no Java GUI draw,
					// submits no Java presentation, and is restricted to the borrowed
					// OpenGL route.  Vulkan whole-frame rendering never reaches this
					// method and therefore cannot acquire a second presenter/pass.
					try (RenderPass targetScope = VulkanicAPI.createRenderPass(
							() -> "Rust GAL GUI borrowed OpenGL target scope",
							renderTarget.getColorTextureView(),
							OptionalInt.empty(),
							renderTarget.useDepth ? renderTarget.getDepthTextureView() : null,
							OptionalDouble.empty()
						)) {
						RustGalFrameCoordinator.executeGuiFrame(minecraft, rustGalDrawGroup);
					}
					rustGalFrameExecuted.setTrue();
				}
				k++;
				continue;
			}

			rustGalFrameExecuted.setFalse();
			int start = k;
			while (k < j && this.draws.get(k) instanceof GuiRenderer.Draw) {
				k++;
			}
			try (RenderPass renderPass = VulkanicAPI.createRenderPass(
						supplier,
						renderTarget.getColorTextureView(),
						OptionalInt.empty(),
						renderTarget.useDepth ? renderTarget.getDepthTextureView() : null,
						OptionalDouble.empty()
					)) {
				net.vulkanic.VulkanicAPI.bindDefaultUniforms(renderPass);
				renderPass.setUniform("Fog", gpuBufferSlice);
				renderPass.setUniform("DynamicTransforms", gpuBufferSlice2);

				for (int drawIndex = start; drawIndex < k; drawIndex++) {
					GuiRenderer.Draw draw = (GuiRenderer.Draw)this.draws.get(drawIndex);
					this.executeDraw(draw, renderPass, gpuBuffer, indexType);
				}
			}
		}
	}

	private void addElementToMesh(GuiElementRenderState guiElementRenderState) {
		if (guiElementRenderState instanceof RustGalGuiElementRenderState rustGalElement) {
			if (this.bufferBuilder != null) {
				this.recordMesh(this.bufferBuilder, this.previousPipeline, this.previousTextureSetup, this.previousScissorArea, this.previousShaderInputParityGeometryContext);
				this.bufferBuilder = null;
			}

			this.previousPipeline = null;
			this.previousTextureSetup = null;
			this.previousScissorArea = null;
			this.previousShaderInputParityGeometryContext = null;
			this.meshesToDraw.add(new GuiRenderer.RustGalDraw(rustGalElement));
			return;
		}
		RenderPipeline renderPipeline = guiElementRenderState.pipeline();
		TextureSetup textureSetup = guiElementRenderState.textureSetup();
		ScreenRectangle screenRectangle = guiElementRenderState.scissorArea();
		String shaderInputParityGeometryContext = VulkanicAPI.isShaderInputParityTracingEnabled()
			? guiElementRenderState.shaderInputParityGeometryContext()
			: "";
		if (renderPipeline != this.previousPipeline
			|| this.scissorChanged(screenRectangle, this.previousScissorArea)
			|| !textureSetup.equals(this.previousTextureSetup)
			|| !java.util.Objects.equals(shaderInputParityGeometryContext, this.previousShaderInputParityGeometryContext)) {
			if (this.bufferBuilder != null) {
				this.recordMesh(this.bufferBuilder, this.previousPipeline, this.previousTextureSetup, this.previousScissorArea, this.previousShaderInputParityGeometryContext);
			}

			this.bufferBuilder = this.getBufferBuilder(renderPipeline);
			this.previousPipeline = renderPipeline;
			this.previousTextureSetup = textureSetup;
			this.previousScissorArea = screenRectangle;
			this.previousShaderInputParityGeometryContext = shaderInputParityGeometryContext;
		}

		guiElementRenderState.buildVertices(this.bufferBuilder);
	}

	private void prepareText() {
		this.renderState.forEachText(guiTextRenderState -> {
			List<RustGalGuiElementRenderState> rustGalText = RustGalGuiRenderer.tryEnqueueText(
				guiTextRenderState,
				Minecraft.getInstance().getWindow().getGuiScaledWidth(),
				Minecraft.getInstance().getWindow().getGuiScaledHeight()
			);
			if (rustGalText != null) {
				for (RustGalGuiElementRenderState element : rustGalText) {
					this.renderState.submitGlyphToCurrentLayer(element);
				}
				return;
			}
			if (RustGalGuiRenderer.isWholeFrameVulkanEnabled() || VulkanicAPI.isVulkanBackendSelected()) {
				// A semantic extraction miss is an admission failure for the
				// exclusive Rust presenter. Do not put Java glyph state back into
				// the render state, where it could become a hidden same-frame
				// fallback or be silently dropped by the Rust scheduler.
				RustGalGuiRenderer.recordUnsupportedElement("text");
				return;
			}
			final Matrix3x2f matrix3x2f = guiTextRenderState.pose;
			final ScreenRectangle screenRectangle = guiTextRenderState.scissor;
			guiTextRenderState.ensurePrepared().visit(new Font.GlyphVisitor() {
				@Override
				public void acceptGlyph(TextRenderable textRenderable) {
					this.accept(textRenderable);
				}

				@Override
				public void acceptEffect(TextRenderable textRenderable) {
					this.accept(textRenderable);
				}

				private void accept(TextRenderable textRenderable) {
					GuiRenderer.this.renderState.submitGlyphToCurrentLayer(new GlyphRenderState(matrix3x2f, textRenderable, screenRectangle));
				}
			});
		});
	}

	private void prepareItemElements() {
		int i = this.getGuiScaleInvalidatingItemAtlasIfChanged();
		if (Standard3dItemRenderer.isDebugDumpEnabled() && !RustGalGuiRenderer.isWholeFrameVulkanEnabled()
			&& !RustGalGuiRenderer.isWholeFrameVulkanActive()
			&& !VulkanicAPI.isVulkanBackendSelected()) {
			Standard3dItemRenderer debugStandard3dItemRenderer = (Standard3dItemRenderer)this.standard3dItemRenderers
				.computeIfAbsent("debug_standard_3d_grass_block", object -> new Standard3dItemRenderer(this.bufferSource));
			debugStandard3dItemRenderer.prepareDebugStandardBlockItemDump(this.renderState, i);
		}

		if (VulkanicAPI.isVulkanBackendSelected()
			|| net.vulkanic.bridge.RustGalVulkanWholeFrameMode.enabled()) {
			if (!RustGalGuiRenderer.isWholeFrameVulkanEnabled()
				&& !this.renderState.getItemModelIdentities().isEmpty()) {
				throw new IllegalStateException("Java Vulkan GUI picture-in-picture items are unavailable until the Rust whole-frame GUI route is admitted");
			}

			return;
		}

		if (!this.renderState.getItemModelIdentities().isEmpty()) {
			int j = 16 * i;
			int k = this.calculateAtlasSizeInPixels(j);
			if (this.itemsAtlas == null) {
				this.createAtlasTextures(k);
			}

			GpuBufferSlice previousProjectionMatrix = VulkanicAPI.getProjectionMatrixBuffer();
			ProjectionType previousProjectionType = VulkanicAPI.getProjectionType();
			GpuTextureView previousColorTextureOverride = VulkanicAPI.getOutputColorTextureOverride();
			GpuTextureView previousDepthTextureOverride = VulkanicAPI.getOutputDepthTextureOverride();
			MutableBoolean mutableBoolean = new MutableBoolean(false);
			MutableBoolean mutableBoolean2 = new MutableBoolean(false);
			boolean atlasWritten = false;
			try {
				net.vulkanic.VulkanicAPI.setOutputColorTextureOverride(this.itemsAtlasView);
				net.vulkanic.VulkanicAPI.setOutputDepthTextureOverride(this.itemsAtlasDepthView);
				net.vulkanic.VulkanicAPI.setProjectionMatrix(this.itemsProjectionMatrixBuffer.getBuffer(k, k), ProjectionType.ORTHOGRAPHIC);
				Minecraft.getInstance().gameRenderer.getLighting().setupFor(Lighting.Entry.ITEMS_3D);
				PoseStack poseStack = new PoseStack();
				this.renderState
					.forEachItem(
						guiItemRenderState -> {
							if (guiItemRenderState.oversizedItemBounds() != null) {
								mutableBoolean2.setTrue();
							} else {
								TrackingItemStackRenderState trackingItemStackRenderState = guiItemRenderState.itemStackRenderState();
								GuiRenderer.AtlasPosition atlasPosition = (GuiRenderer.AtlasPosition)this.atlasPositions.get(trackingItemStackRenderState.getModelIdentity());
								if (atlasPosition == null || trackingItemStackRenderState.isAnimated() && atlasPosition.lastAnimatedOnFrame != this.frameNumber) {
									if (this.itemAtlasX + j > k) {
										this.itemAtlasX = 0;
										this.itemAtlasY += j;
									}

									boolean bl = trackingItemStackRenderState.isAnimated() && atlasPosition != null;
									if (!bl && this.itemAtlasY + j > k) {
										if (mutableBoolean.isFalse()) {
											LOGGER.warn("Trying to render too many items in GUI at the same time. Skipping some of them.");
											mutableBoolean.setTrue();
										}
									} else {
										int kx = bl ? atlasPosition.x : this.itemAtlasX;
										int l = bl ? atlasPosition.y : this.itemAtlasY;
										if (bl) {
											VulkanicAPI.createCommandEncoder().clearColorAndDepthTextures(this.itemsAtlas, 0, this.itemsAtlasDepth, 1.0, kx, k - l - j, j, j);
										}

										this.renderItemToAtlas(trackingItemStackRenderState, poseStack, kx, l, j);
										float f = (float)kx / k;
										float g = (float)(k - l) / k;
										this.submitBlitFromItemAtlas(guiItemRenderState, f, g, j, k);
										if (bl) {
											atlasPosition.lastAnimatedOnFrame = this.frameNumber;
										} else {
											this.atlasPositions
												.put(
													guiItemRenderState.itemStackRenderState().getModelIdentity(),
													new GuiRenderer.AtlasPosition(this.itemAtlasX, this.itemAtlasY, f, g, this.frameNumber)
												);
											this.itemAtlasX += j;
										}
									}
								} else {
									this.submitBlitFromItemAtlas(guiItemRenderState, atlasPosition.u, atlasPosition.v, j, k);
								}
							}
						}
					);
				atlasWritten = true;
			} finally {
				net.vulkanic.VulkanicAPI.setOutputColorTextureOverride(previousColorTextureOverride);
				net.vulkanic.VulkanicAPI.setOutputDepthTextureOverride(previousDepthTextureOverride);
				net.vulkanic.VulkanicAPI.setProjectionMatrix(previousProjectionMatrix, previousProjectionType);
				if (atlasWritten) {
					VulkanicAPI.applyResourceBarriers(VulkanicAPI.getCommandContext(), OFFSCREEN_COLOR_WRITES_VISIBLE_TO_TEXTURE_FETCH);
				}
			}

			if (mutableBoolean2.getValue()) {
				this.renderState
					.forEachItem(
						guiItemRenderState -> {
							if (guiItemRenderState.oversizedItemBounds() != null) {
								TrackingItemStackRenderState trackingItemStackRenderState = guiItemRenderState.itemStackRenderState();
								OversizedItemRenderer oversizedItemRenderer = (OversizedItemRenderer)this.oversizedItemRenderers
									.computeIfAbsent(trackingItemStackRenderState.getModelIdentity(), object -> new OversizedItemRenderer(this.bufferSource));
								ScreenRectangle screenRectangle = guiItemRenderState.oversizedItemBounds();
								OversizedItemRenderState oversizedItemRenderState = new OversizedItemRenderState(
									guiItemRenderState, screenRectangle.left(), screenRectangle.top(), screenRectangle.right(), screenRectangle.bottom()
								);
								oversizedItemRenderer.prepare(oversizedItemRenderState, this.renderState, i);
							}
						}
					);
			}
		}
	}

	private void prepareItemsViaPictureInPicture(int i) {
		this.renderState.forEachItem(guiItemRenderState -> {
			if (this.rustOwnedStandard3dItems.contains(guiItemRenderState)) {
				return;
			}
			TrackingItemStackRenderState trackingItemStackRenderState = guiItemRenderState.itemStackRenderState();
			ScreenRectangle screenRectangle = guiItemRenderState.oversizedItemBounds();
			int j = screenRectangle != null ? screenRectangle.left() : guiItemRenderState.x();
			int k = screenRectangle != null ? screenRectangle.top() : guiItemRenderState.y();
			int l = screenRectangle != null ? screenRectangle.right() : guiItemRenderState.x() + 16;
			int m = screenRectangle != null ? screenRectangle.bottom() : guiItemRenderState.y() + 16;
			OversizedItemRenderState oversizedItemRenderState = new OversizedItemRenderState(guiItemRenderState, j, k, l, m);
			if (screenRectangle == null && trackingItemStackRenderState.usesBlockLight()) {
				Standard3dItemRenderer standard3dItemRenderer = (Standard3dItemRenderer)this.standard3dItemRenderers
					.computeIfAbsent(trackingItemStackRenderState.getModelIdentity(), object -> new Standard3dItemRenderer(this.bufferSource));
				standard3dItemRenderer.prepare(oversizedItemRenderState, this.renderState, i);
			} else {
				OversizedItemRenderer oversizedItemRenderer = (OversizedItemRenderer)this.oversizedItemRenderers
					.computeIfAbsent(trackingItemStackRenderState.getModelIdentity(), object -> new OversizedItemRenderer(this.bufferSource));
				oversizedItemRenderer.prepare(oversizedItemRenderState, this.renderState, i);
			}
		});
	}

	private void preparePictureInPicture() {
		int i = Minecraft.getInstance().getWindow().getGuiScale();
		this.renderState.forEachPictureInPicture(pictureInPictureRenderState -> {
			if (!this.rustOwnedPictureInPictureStates.contains(pictureInPictureRenderState)
				&& !RustGalGuiRenderer.isWholeFrameVulkanEnabled()
				&& !VulkanicAPI.isVulkanBackendSelected()) {
				this.preparePictureInPictureState(pictureInPictureRenderState, i);
			}
		});
	}

	private <T extends PictureInPictureRenderState> void preparePictureInPictureState(T pictureInPictureRenderState, int i) {
		PictureInPictureRenderer<T> pictureInPictureRenderer = (PictureInPictureRenderer<T>)this.pictureInPictureRenderers
			.get(pictureInPictureRenderState.getClass());
		if (pictureInPictureRenderer != null) {
			pictureInPictureRenderer.prepare(pictureInPictureRenderState, this.renderState, i);
		}
	}

	private void renderItemToAtlas(TrackingItemStackRenderState trackingItemStackRenderState, PoseStack poseStack, int i, int j, int k) {
		if (net.vulkanic.bridge.RustGalVulkanWholeFrameMode.enabled()
			|| VulkanicAPI.isVulkanBackendSelected()) {
			throw new IllegalStateException(
				"Java GUI item-atlas rendering is unavailable while Rust owns whole-frame Vulkan presentation"
			);
		}
		poseStack.pushPose();
		poseStack.translate(i + k / 2.0F, j + k / 2.0F, 0.0F);
		poseStack.scale(k, -k, k);
		boolean bl = !trackingItemStackRenderState.usesBlockLight();
		if (bl) {
			Minecraft.getInstance().gameRenderer.getLighting().setupFor(Lighting.Entry.ITEMS_FLAT);
		} else {
			Minecraft.getInstance().gameRenderer.getLighting().setupFor(Lighting.Entry.ITEMS_3D);
		}

		boolean bl2 = !VulkanicAPI.isVulkanBackendSelected();
		if (bl2) {
			net.vulkanic.VulkanicAPI.enableScissorForRenderTypeDraws(i, this.itemsAtlas.getHeight(0) - j - k, k, k);
		}
		trackingItemStackRenderState.submit(poseStack, this.submitNodeCollector, 15728880, OverlayTexture.NO_OVERLAY, 0);
		this.featureRenderDispatcher.renderAllFeatures();
		this.bufferSource.endBatch();
		if (bl2) {
			net.vulkanic.VulkanicAPI.disableScissorForRenderTypeDraws();
		}
		poseStack.popPose();
	}

	private void submitBlitFromItemAtlas(GuiItemRenderState guiItemRenderState, float f, float g, int i, int j) {
		float h = f + (float)i / j;
		float k = g + (float)(-i) / j;
		this.renderState
			.submitBlitToCurrentLayer(
				new BlitRenderState(
					RenderPipelines.GUI_TEXTURED,
					TextureSetup.singleTexture(this.itemsAtlasView),
					guiItemRenderState.pose(),
					guiItemRenderState.x(),
					guiItemRenderState.y(),
					guiItemRenderState.x() + 16,
					guiItemRenderState.y() + 16,
					f,
					h,
					g,
					k,
					-1,
					guiItemRenderState.scissorArea(),
					null,
					shaderInputParityGuiItemContext("gui-item", guiItemRenderState)
				)
			);
	}

	public static String shaderInputParityGuiItemContext(String source, GuiItemRenderState guiItemRenderState) {
		String name = VulkanicAPI.shaderInputParityDiagnosticLabel(guiItemRenderState.name());
		return source
			+ ":name=" + name
			+ ":pos=" + guiItemRenderState.x() + "x" + guiItemRenderState.y()
			+ ":oversized=" + (guiItemRenderState.oversizedItemBounds() != null);
	}

	private void createAtlasTextures(int i) {
		if (net.vulkanic.bridge.RustGalVulkanWholeFrameMode.enabled()
			|| VulkanicAPI.isVulkanBackendSelected()) {
			throw new IllegalStateException(
				"Java GUI item-atlas allocation is unavailable while Rust owns whole-frame Vulkan"
			);
		}
		this.itemsAtlas = VulkanicAPI.createTexture("UI items atlas", 12, TextureFormat.RGBA8, i, i, 1, 1);
		this.itemsAtlas.setTextureFilter(FilterMode.NEAREST, false);
		this.itemsAtlasView = VulkanicAPI.createTextureView(this.itemsAtlas);
		this.itemsAtlasDepth = VulkanicAPI.createTexture("UI items atlas depth", 8, TextureFormat.DEPTH32, i, i, 1, 1);
		this.itemsAtlasDepthView = VulkanicAPI.createTextureView(this.itemsAtlasDepth);
		VulkanicAPI.createCommandEncoder().clearColorAndDepthTextures(this.itemsAtlas, 0, this.itemsAtlasDepth, 1.0);
	}

	private int calculateAtlasSizeInPixels(int i) {
		Set<Object> set = this.renderState.getItemModelIdentities();
		int j;
		if (this.atlasPositions.isEmpty()) {
			j = set.size();
		} else {
			j = this.atlasPositions.size();

			for (Object object : set) {
				if (!this.atlasPositions.containsKey(object)) {
					j++;
				}
			}
		}

		if (this.itemsAtlas != null) {
			int k = this.itemsAtlas.getWidth(0) / i;
			int l = k * k;
			if (j < l) {
				return this.itemsAtlas.getWidth(0);
			}

			this.invalidateItemAtlas();
		}

		int k = set.size();
		int l = Mth.smallestSquareSide(k + k / 2);
		return Math.clamp(Mth.smallestEncompassingPowerOfTwo(l * i), 512, MAXIMUM_ITEM_ATLAS_SIZE);
	}

	private int getGuiScaleInvalidatingItemAtlasIfChanged() {
		int i = Minecraft.getInstance().getWindow().getGuiScale();
		if (i != this.cachedGuiScale) {
			this.invalidateItemAtlas();

			for (OversizedItemRenderer oversizedItemRenderer : this.oversizedItemRenderers.values()) {
				oversizedItemRenderer.invalidateTexture();
			}

			for (Standard3dItemRenderer standard3dItemRenderer : this.standard3dItemRenderers.values()) {
				standard3dItemRenderer.invalidateTexture();
			}

			this.cachedGuiScale = i;
		}

		return i;
	}

	private void invalidateItemAtlas() {
		this.itemAtlasX = 0;
		this.itemAtlasY = 0;
		this.atlasPositions.clear();
		if (this.itemsAtlas != null) {
			this.itemsAtlas.close();
			this.itemsAtlas = null;
		}

		if (this.itemsAtlasView != null) {
			this.itemsAtlasView.close();
			this.itemsAtlasView = null;
		}

		if (this.itemsAtlasDepth != null) {
			this.itemsAtlasDepth.close();
			this.itemsAtlasDepth = null;
		}

		if (this.itemsAtlasDepthView != null) {
			this.itemsAtlasDepthView.close();
			this.itemsAtlasDepthView = null;
		}
	}

	private void recordMesh(
		BufferBuilder bufferBuilder,
		RenderPipeline renderPipeline,
		TextureSetup textureSetup,
		@Nullable ScreenRectangle screenRectangle,
		@Nullable String shaderInputParityGeometryContext
	) {
		MeshData meshData = bufferBuilder.build();
		if (meshData != null) {
			this.meshesToDraw.add(new GuiRenderer.MeshToDraw(meshData, renderPipeline, textureSetup, screenRectangle, shaderInputParityGeometryContext));
		}
	}

	private void recordDraws() {
		this.ensureVertexBufferSizes();
		CommandEncoder commandEncoder = VulkanicAPI.createCommandEncoder();
		Object2IntMap<VertexFormat> object2IntMap = new Object2IntOpenHashMap<>();

		for (GuiRenderer.PreparedStep step : this.meshesToDraw) {
			if (step instanceof GuiRenderer.RustGalDraw rustGalDraw) {
				this.draws.add(rustGalDraw);
				continue;
			}
			GuiRenderer.MeshToDraw meshToDraw = (GuiRenderer.MeshToDraw)step;
			MeshData meshData = meshToDraw.mesh;
			MeshData.DrawState drawState = meshData.drawState();
			VertexFormat vertexFormat = drawState.format();
			MappableRingBuffer mappableRingBuffer = (MappableRingBuffer)this.vertexBuffers.get(vertexFormat);
			if (!object2IntMap.containsKey(vertexFormat)) {
				object2IntMap.put(vertexFormat, 0);
			}

			ByteBuffer byteBuffer = meshData.vertexBuffer();
			int i = byteBuffer.remaining();
			int j = object2IntMap.getInt(vertexFormat);

			try (GpuBuffer.MappedView mappedView = commandEncoder.mapBuffer(mappableRingBuffer.currentBuffer().slice(j, i), false, true)) {
				MemoryUtil.memCopy(byteBuffer, mappedView.data());
			}

			object2IntMap.put(vertexFormat, j + i);
			this.draws
				.add(
					new GuiRenderer.Draw(
						mappableRingBuffer.currentBuffer(),
						j / vertexFormat.getVertexSize(),
						drawState.mode(),
						drawState.indexCount(),
						meshToDraw.pipeline,
						meshToDraw.textureSetup,
						meshToDraw.scissorArea,
						meshToDraw.shaderInputParityGeometryContext
					)
				);
			meshToDraw.close();
		}
	}

	private void ensureVertexBufferSizes() {
		Object2IntMap<VertexFormat> object2IntMap = this.calculatedRequiredVertexBufferSizes();

		for (it.unimi.dsi.fastutil.objects.Object2IntMap.Entry<VertexFormat> entry : object2IntMap.object2IntEntrySet()) {
			VertexFormat vertexFormat = (VertexFormat)entry.getKey();
			int i = entry.getIntValue();
			MappableRingBuffer mappableRingBuffer = (MappableRingBuffer)this.vertexBuffers.get(vertexFormat);
			if (mappableRingBuffer == null || mappableRingBuffer.size() < i) {
				if (mappableRingBuffer != null) {
					mappableRingBuffer.close();
				}

				this.vertexBuffers.put(vertexFormat, new MappableRingBuffer(() -> "GUI vertex buffer for " + vertexFormat, 34, i));
			}
		}
	}

	private Object2IntMap<VertexFormat> calculatedRequiredVertexBufferSizes() {
		Object2IntMap<VertexFormat> object2IntMap = new Object2IntOpenHashMap<>();

		for (GuiRenderer.PreparedStep step : this.meshesToDraw) {
			if (!(step instanceof GuiRenderer.MeshToDraw meshToDraw)) {
				continue;
			}
			MeshData.DrawState drawState = meshToDraw.mesh.drawState();
			VertexFormat vertexFormat = drawState.format();
			if (!object2IntMap.containsKey(vertexFormat)) {
				object2IntMap.put(vertexFormat, 0);
			}

			object2IntMap.put(vertexFormat, object2IntMap.getInt(vertexFormat) + drawState.vertexCount() * vertexFormat.getVertexSize());
		}

		return object2IntMap;
	}

	private void executeDraw(GuiRenderer.Draw draw, RenderPass renderPass, GpuBuffer gpuBuffer, VertexFormat.IndexType indexType) {
		RenderPipeline renderPipeline = draw.pipeline();
		renderPass.setPipeline(renderPipeline);
		renderPass.setVertexBuffer(0, draw.vertexBuffer);
		ScreenRectangle screenRectangle = draw.scissorArea();
		if (screenRectangle != null) {
			this.enableScissor(screenRectangle, renderPass);
		} else {
			renderPass.disableScissor();
		}

		if (draw.textureSetup.texure0() != null) {
			renderPass.bindSampler("Sampler0", draw.textureSetup.texure0());
		}

		if (draw.textureSetup.texure1() != null) {
			renderPass.bindSampler("Sampler1", draw.textureSetup.texure1());
		}

		if (draw.textureSetup.texure2() != null) {
			renderPass.bindSampler("Sampler2", draw.textureSetup.texure2());
		}

		renderPass.setIndexBuffer(gpuBuffer, indexType);
		try (VulkanicAPI.ShaderInputParityScope ignored = VulkanicAPI.pushShaderInputParitySemanticContext("gui:" + draw.shaderInputParityGeometryContext())) {
			renderPass.drawIndexed(draw.baseVertex, 0, draw.indexCount, 1);
		}
	}

	private BufferBuilder getBufferBuilder(RenderPipeline renderPipeline) {
		return new BufferBuilder(this.byteBufferBuilder, renderPipeline.getVertexFormatMode(), renderPipeline.getVertexFormat());
	}

	private boolean scissorChanged(ScreenRectangle screenRectangle, @Nullable ScreenRectangle screenRectangle2) {
		if (screenRectangle == screenRectangle2) {
			return false;
		} else {
			return screenRectangle != null ? !screenRectangle.equals(screenRectangle2) : true;
		}
	}

	private void enableScissor(ScreenRectangle screenRectangle, RenderPass renderPass) {
		Window window = Minecraft.getInstance().getWindow();
		int i = window.getHeight();
		int j = window.getGuiScale();
		double d = screenRectangle.left() * j;
		double e = i - screenRectangle.bottom() * j;
		double f = screenRectangle.width() * j;
		double g = screenRectangle.height() * j;
		renderPass.enableScissor((int)d, (int)e, Math.max(0, (int)f), Math.max(0, (int)g));
	}

	public void close() {
		this.byteBufferBuilder.close();
		if (this.itemsAtlas != null) {
			this.itemsAtlas.close();
		}

		if (this.itemsAtlasView != null) {
			this.itemsAtlasView.close();
		}

		if (this.itemsAtlasDepth != null) {
			this.itemsAtlasDepth.close();
		}

		if (this.itemsAtlasDepthView != null) {
			this.itemsAtlasDepthView.close();
		}

		this.pictureInPictureRenderers.values().forEach(PictureInPictureRenderer::close);
		if (this.guiProjectionMatrixBuffer != null) {
			this.guiProjectionMatrixBuffer.close();
		}
		if (this.itemsProjectionMatrixBuffer != null) {
			this.itemsProjectionMatrixBuffer.close();
		}

		for (MappableRingBuffer mappableRingBuffer : this.vertexBuffers.values()) {
			mappableRingBuffer.close();
		}

		this.oversizedItemRenderers.values().forEach(PictureInPictureRenderer::close);
		this.standard3dItemRenderers.values().forEach(PictureInPictureRenderer::close);
	}

	/** Releases Java GUI GPU state while retaining semantic render-state inputs. */
	public void ensureRustSemanticRoute() {
		if (!(RustGalGuiRenderer.isWholeFrameVulkanEnabled() || VulkanicAPI.isVulkanBackendSelected())) return;
		if (this.itemsAtlas != null) {
			this.itemsAtlas.close();
			this.itemsAtlas = null;
		}
		if (this.itemsAtlasView != null) {
			this.itemsAtlasView.close();
			this.itemsAtlasView = null;
		}
		if (this.itemsAtlasDepth != null) {
			this.itemsAtlasDepth.close();
			this.itemsAtlasDepth = null;
		}
		if (this.itemsAtlasDepthView != null) {
			this.itemsAtlasDepthView.close();
			this.itemsAtlasDepthView = null;
		}
		if (this.guiProjectionMatrixBuffer != null) this.guiProjectionMatrixBuffer.ensureRustSemanticRoute();
		if (this.itemsProjectionMatrixBuffer != null) this.itemsProjectionMatrixBuffer.ensureRustSemanticRoute();
		this.pictureInPictureRenderers.values().forEach(PictureInPictureRenderer::close);
		this.oversizedItemRenderers.values().forEach(PictureInPictureRenderer::close);
		this.standard3dItemRenderers.values().forEach(PictureInPictureRenderer::close);
		for (MappableRingBuffer buffer : this.vertexBuffers.values()) buffer.close();
		this.vertexBuffers.clear();
	}

	@Environment(EnvType.CLIENT)
	static final class AtlasPosition {
		final int x;
		final int y;
		final float u;
		final float v;
		int lastAnimatedOnFrame;

		AtlasPosition(int i, int j, float f, float g, int k) {
			this.x = i;
			this.y = j;
			this.u = f;
			this.v = g;
			this.lastAnimatedOnFrame = k;
		}
	}

	@Environment(EnvType.CLIENT)
	record Draw(
		GpuBuffer vertexBuffer,
		int baseVertex,
		VertexFormat.Mode mode,
		int indexCount,
		RenderPipeline pipeline,
		TextureSetup textureSetup,
		@Nullable ScreenRectangle scissorArea,
		String shaderInputParityGeometryContext
	) implements DrawStep {
	}

	@Environment(EnvType.CLIENT)
	interface DrawStep {
	}

	@Environment(EnvType.CLIENT)
	interface PreparedStep extends AutoCloseable {
		@Override
		default void close() {
		}
	}

	@Environment(EnvType.CLIENT)
	record RustGalDraw(RustGalGuiElementRenderState element) implements DrawStep, PreparedStep {
	}

	@Environment(EnvType.CLIENT)
	record MeshToDraw(
		MeshData mesh,
		RenderPipeline pipeline,
		TextureSetup textureSetup,
		@Nullable ScreenRectangle scissorArea,
		@Nullable String shaderInputParityGeometryContext
	) implements PreparedStep {

		@Override
		public void close() {
			this.mesh.close();
		}
	}
}
