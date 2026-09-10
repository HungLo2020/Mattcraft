package net.vulkanic.gui;

import net.blaze3d.platform.Window;
import net.logging.LogUtils;
import net.minecraft.client.Minecraft;
import net.minecraft.client.dev.GraphicsFrameBenchmark;
import net.minecraft.client.dev.DeterministicCameraCapture;
import net.minecraft.client.dev.RenderDocCaptureHook;
import net.minecraft.client.gui.render.state.GuiRenderState;
import net.minecraft.client.gui.screens.TitleScreen;
import net.minecraft.resources.ResourceLocation;
import net.minecraft.server.packs.resources.ResourceManager;
import net.minecraft.util.profiling.TracyCompat;
import net.vulkanic.VulkanicAPI;
import net.vulkanic.bridge.RustGalFrameScheduler;
import net.vulkanic.bridge.RustGalVulkanWholeFrameMode;
import net.vulkanic.bridge.VulkanicGalBridge;
import net.vulkanic.shaderpack.RustShaderPackSourceCollector;
import net.vulkanic.world.DistantHorizonsSemanticCollector;
import net.vulkanic.world.RustGalTerrainRenderer;
import net.vulkanic.world.RustGalWorldPrimitiveRenderer;
import org.slf4j.Logger;

import java.io.IOException;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.nio.file.Path;
import java.nio.file.StandardCopyOption;
import java.util.ArrayList;
import java.util.Arrays;
import java.util.LinkedHashMap;
import java.util.List;
import java.util.Map;

public final class RustGalFrameCoordinator {
	private static final int MAX_RUST_GUI_AFFINE_QUADS = 65_536;
	private static final int MAX_RUST_GUI_MESH_BATCHES = 1_024;
	private static final int MAX_RUST_GUI_MESH_VERTICES = 65_536;
	private static final int MAX_RUST_GUI_MESH_INDICES = 196_608;
	private static final Logger LOGGER = LogUtils.getLogger();
	private static final Object LOCK = new Object();
	private static final GuiAtlasReferencePublication GUI_ATLAS_REFERENCES = new GuiAtlasReferencePublication();
	private static VulkanicGalBridge bridge;
	/**
	 * The bridge owns a native graphics context for its entire lifetime.  These
	 * modes must never be interchanged: doing so would let a whole-frame Vulkan
	 * submission accidentally reach the borrowed OpenGL bridge created by the
	 * partial-frame route.
	 */
	private enum BridgeMode {
		NONE,
		BORROWED_OPENGL,
		WINDOWED_VULKAN
	}

	private static BridgeMode bridgeMode = BridgeMode.NONE;
	private static Thread renderThread;
	private static int configuredWidth;
	private static int configuredHeight;
	private static long nextCorrelationId = 1L;
	private static long generation = 1L;
	private static long assetGeneration = 1L;
	private static long uploadedAssetGeneration;
	private static long attemptedAssetGeneration;
	private static long lastAssetPayloadCount;
	private static long lastAssetPayloadBytes;
	private static long assetUpdateFailures;
	private static long rawImageGeneration = 1L;
	private static long uploadedRawImageGeneration;
	private static long attemptedRawImageGeneration;
	private static final int MAX_PENDING_RAW_IMAGES = 4_096;
	/** Matches the Rust-owned aggregate raw GUI image payload bound. */
	private static final long MAX_PENDING_RAW_IMAGE_BYTES = 256L * 1024L * 1024L;
	private static final Map<Long, VulkanicGalBridge.GuiRawImageAssetRecord> pendingRawImages = new LinkedHashMap<>();
	private static long nextShaderPackSourceGeneration = 1L;
	private static long uploadedShaderPackSourceGeneration;
	private static long attemptedShaderPackSourceGeneration;
	private static long shaderPackSourceUpdateFailures;
	private static long uploadedShaderPackAssetGeneration;
	private static long attemptedShaderPackAssetGeneration;
	private static long shaderPackAssetUpdateFailures;
	private static RustShaderPackSourceCollector.SourceGeneration pendingShaderPackSources;
	private static String pendingShaderPackSourceName = "";
	private static long lastSubmitted;
	private static long lastRetiredSubmission;
	/** Bounded diagnostic receipts emitted only after Rust has presented a title frame. */
	private static int graphicsAuditTitlePresentationReceipts;
	private static long lastDhParityPhaseFrame = Long.MIN_VALUE;
	private static volatile boolean observedRenderableWholeFrameWorld;
	private static volatile long lastRenderableWholeFrameWorldFrame;
	/**
	 * Latest frame acquired by the Rust whole-frame presenter. This intentionally
	 * shares the GAL frame-id clock with {@link #lastRenderableWholeFrameWorldFrame}
	 * so capture freshness never compares it to a Java render-hook counter.
	 */
	private static volatile long lastAcquiredWholeFrameFrame;
	private static List<VulkanicGalBridge.GuiAssetRecord> pendingAssets = List.of();
	private static final RustGalFrameScheduler<QueuedGuiRequest> SCHEDULER =
		new RustGalFrameScheduler<>("Rust VulkanicGAL deferred GUI");
	private static final Metrics METRICS = new Metrics();

	private RustGalFrameCoordinator() {
	}

	/** One ordered, generation-bound semantic GUI work item. */
	private record QueuedGuiRequest(
		VulkanicGalBridge.GuiSpriteRecord sprite,
		List<VulkanicGalBridge.GuiAffineQuadRecord> affineQuads,
		List<VulkanicGalBridge.GuiMeshBatchRecord> meshBatches,
		VulkanicGalBridge.GuiTiledQuadRecord tiledQuad
	) {
		private static final int MAX_AFFINE_QUADS_PER_ITEM = (int) RustGalFrameScheduler.SEQUENCE_STRIDE - 1;
		static QueuedGuiRequest sprite(VulkanicGalBridge.GuiSpriteRecord sprite) {
			return new QueuedGuiRequest(java.util.Objects.requireNonNull(sprite, "sprite"), List.of(), List.of(), null);
		}

		static QueuedGuiRequest affineQuad(VulkanicGalBridge.GuiAffineQuadRecord affineQuad) {
			return affineQuads(List.of(java.util.Objects.requireNonNull(affineQuad, "affineQuad")));
		}

		static QueuedGuiRequest affineQuads(List<VulkanicGalBridge.GuiAffineQuadRecord> affineQuads) {
			if (affineQuads == null || affineQuads.isEmpty() || affineQuads.size() > MAX_AFFINE_QUADS_PER_ITEM
				|| affineQuads.stream().anyMatch(java.util.Objects::isNull)) {
				throw new IllegalArgumentException("GUI affine-quad item requires quads");
			}
			return new QueuedGuiRequest(null, List.copyOf(affineQuads), List.of(), null);
		}

		static QueuedGuiRequest meshBatches(List<VulkanicGalBridge.GuiMeshBatchRecord> meshBatches) {
			if (meshBatches == null || meshBatches.isEmpty()) throw new IllegalArgumentException("GUI mesh item requires layers");
			return new QueuedGuiRequest(null, List.of(), List.copyOf(meshBatches), null);
		}

		static QueuedGuiRequest tiledQuad(VulkanicGalBridge.GuiTiledQuadRecord tile) {
			return new QueuedGuiRequest(null, List.of(), List.of(), java.util.Objects.requireNonNull(tile));
		}

		int guiWidth() {
			if (this.tiledQuad != null) return this.tiledQuad.guiWidth();
			return this.sprite != null ? this.sprite.guiWidth() : !this.affineQuads.isEmpty() ? this.affineQuads.getFirst().guiWidth() : this.meshBatches.getFirst().guiWidth();
		}

		int guiHeight() {
			if (this.tiledQuad != null) return this.tiledQuad.guiHeight();
			return this.sprite != null ? this.sprite.guiHeight() : !this.affineQuads.isEmpty() ? this.affineQuads.getFirst().guiHeight() : this.meshBatches.getFirst().guiHeight();
		}

		void appendTo(
			List<VulkanicGalBridge.GuiSpriteRecord> sprites,
			List<VulkanicGalBridge.GuiAffineQuadRecord> affineQuads,
			List<VulkanicGalBridge.GuiMeshBatchRecord> meshBatches,
			List<VulkanicGalBridge.GuiTiledQuadRecord> tiledQuads,
			long sequence
		) {
			if (this.tiledQuad != null) {
				tiledQuads.add(this.tiledQuad.withSequence(sequence));
			} else if (this.sprite != null) {
				sprites.add(this.sprite.withSequence(sequence));
			} else if (!this.affineQuads.isEmpty()) {
				for (int index = 0; index < this.affineQuads.size(); index++) {
					affineQuads.add(this.affineQuads.get(index).withSequence(sequence + index));
				}
			} else {
				for (VulkanicGalBridge.GuiMeshBatchRecord batch : this.meshBatches) meshBatches.add(batch.withSequence(sequence));
			}
		}
	}

	static RustGalFrameScheduler.Token enqueueGuiRequest(
		VulkanicGalBridge.GuiSpriteRecord request,
		GuiRenderStratum stratum,
		long startedNanos
	) {
		requireRustGuiRoute();
		long lockStartedNanos = System.nanoTime();
		synchronized (LOCK) {
			RustGalFrameScheduler.Token token = SCHEDULER.enqueue(
				generation, stratum.id(), stratum.order(), QueuedGuiRequest.sprite(request));
			// Measure only this enqueue's coordinator critical section. Callers may
			// reuse a producer timestamp across a batch (notably text glyphs), which
			// would otherwise count overlapping intervals once per glyph.
			METRICS.enqueueNanos += elapsedSince(lockStartedNanos);
			return token;
		}
	}

	static RustGalFrameScheduler.Token enqueueGuiAffineQuadRequest(
		VulkanicGalBridge.GuiAffineQuadRecord request,
		GuiRenderStratum stratum,
		long startedNanos
	) {
		return enqueueGuiAffineQuadRequest(request, stratum.id(), stratum.order(), startedNanos);
	}

	/** Queues an explicitly ordered whole-frame GUI primitive without a fixed HUD stratum. */
	static RustGalFrameScheduler.Token enqueueGuiAffineQuadRequest(
		VulkanicGalBridge.GuiAffineQuadRecord request,
		String semanticLayerId,
		int semanticLayerOrder,
		long startedNanos
	) {
		requireRustGuiRoute();
		if (semanticLayerId == null || semanticLayerId.isBlank() || semanticLayerOrder < 0) {
			throw new IllegalArgumentException("invalid semantic GUI layer");
		}
		long lockStartedNanos = System.nanoTime();
		synchronized (LOCK) {
			RustGalFrameScheduler.Token token = SCHEDULER.enqueue(
				generation, semanticLayerId, semanticLayerOrder, QueuedGuiRequest.affineQuad(request));
			METRICS.enqueueNanos += elapsedSince(lockStartedNanos);
			return token;
		}
	}

	/** Queues one immutable tiled command; only Rust expands it into draw instances. */
	static RustGalFrameScheduler.Token enqueueGuiTiledQuadRequest(
		VulkanicGalBridge.GuiTiledQuadRecord request, String semanticLayerId, int semanticLayerOrder
	) {
		requireRustGuiRoute();
		if (semanticLayerId == null || semanticLayerId.isBlank() || semanticLayerOrder < 0) {
			throw new IllegalArgumentException("invalid semantic GUI layer");
		}
		long started = System.nanoTime();
		synchronized (LOCK) {
			RustGalFrameScheduler.Token token = SCHEDULER.enqueue(
				generation, semanticLayerId, semanticLayerOrder, QueuedGuiRequest.tiledQuad(request));
			METRICS.enqueueNanos += elapsedSince(started);
			return token;
		}
	}

	/** Queues an ordered batch of semantic affine quads as one scheduler item. */
	static RustGalFrameScheduler.Token enqueueGuiAffineQuadRequests(
		List<VulkanicGalBridge.GuiAffineQuadRecord> requests,
		String semanticLayerId,
		int semanticLayerOrder,
		long startedNanos
	) {
		requireRustGuiRoute();
		if (semanticLayerId == null || semanticLayerId.isBlank() || semanticLayerOrder < 0) {
			throw new IllegalArgumentException("invalid semantic GUI layer");
		}
		long lockStartedNanos = System.nanoTime();
		synchronized (LOCK) {
			RustGalFrameScheduler.Token token = SCHEDULER.enqueue(
				generation, semanticLayerId, semanticLayerOrder, QueuedGuiRequest.affineQuads(requests));
			METRICS.enqueueNanos += elapsedSince(lockStartedNanos);
			return token;
		}
	}

	public static RustGalFrameScheduler.Token enqueueGuiMeshItemRequest(
		List<VulkanicGalBridge.GuiMeshBatchRecord> batches,
		GuiRenderStratum stratum,
		long startedNanos
	) {
		return enqueueGuiMeshItemRequest(batches, stratum.id(), stratum.order(), startedNanos);
	}

	/** Queues a whole-frame GUI mesh item at its explicit source-layer position. */
	static RustGalFrameScheduler.Token enqueueGuiMeshItemRequest(
		List<VulkanicGalBridge.GuiMeshBatchRecord> batches,
		String semanticLayerId,
		int semanticLayerOrder,
		long startedNanos
	) {
		requireRustGuiRoute();
		if (semanticLayerId == null || semanticLayerId.isBlank() || semanticLayerOrder < 0) {
			throw new IllegalArgumentException("invalid semantic GUI layer");
		}
		long lockStartedNanos = System.nanoTime();
		synchronized (LOCK) {
			RustGalFrameScheduler.Token token = SCHEDULER.enqueue(
				generation, semanticLayerId, semanticLayerOrder, QueuedGuiRequest.meshBatches(batches));
			METRICS.enqueueNanos += elapsedSince(lockStartedNanos);
			return token;
		}
	}

	/**
	 * Semantic GUI requests are only meaningful on an admitted Rust GUI route.
	 * Reject before taking the scheduler lock so a disabled/diagnostic route
	 * cannot accumulate work that might later be mistaken for a frame.
	 */
	private static void requireRustGuiRoute() {
		if (!RustGalGuiRenderer.currentExecutionRoute().usesRustGui()) {
			throw new IllegalStateException(
				"Rust GUI semantic enqueue requires an admitted Rust GUI route; current route is "
					+ RustGalGuiRenderer.currentExecutionRoute()
			);
		}
	}

	static void stageGuiRawImage(VulkanicGalBridge.GuiRawImageAssetRecord asset) {
		requireRustGuiRoute();
		synchronized (LOCK) {
			VulkanicGalBridge.GuiRawImageAssetRecord previous = pendingRawImages.get(asset.assetId());
			if (previous != null
				&& previous.format() == asset.format()
				&& previous.width() == asset.width()
				&& previous.height() == asset.height()
				&& previous.samplingFilter() == asset.samplingFilter()
				&& previous.samplingAddress() == asset.samplingAddress()
				&& Arrays.equals(previous.pixels(), asset.pixels())) {
				return;
			}
			if (previous == null && pendingRawImages.size() >= MAX_PENDING_RAW_IMAGES) {
				throw new IllegalStateException("semantic GUI raw-image staging bound exceeded " + MAX_PENDING_RAW_IMAGES);
			}
			long projectedBytes = 0L;
			for (VulkanicGalBridge.GuiRawImageAssetRecord candidate : pendingRawImages.values()) {
				if (candidate != previous) projectedBytes = Math.addExact(projectedBytes, candidate.pixels().length);
			}
			projectedBytes = Math.addExact(projectedBytes, asset.pixels().length);
			if (projectedBytes > MAX_PENDING_RAW_IMAGE_BYTES) {
				throw new IllegalStateException(
					"semantic GUI raw-image byte bound exceeded " + MAX_PENDING_RAW_IMAGE_BYTES
						+ " projected=" + projectedBytes
				);
			}
			pendingRawImages.put(asset.assetId(), asset);
			rawImageGeneration++;
			attemptedRawImageGeneration = Math.min(attemptedRawImageGeneration, uploadedRawImageGeneration);
		}
	}

	/**
	 * Drops the Java-side raw-image generation together with its source caches.
	 * The next flush publishes an explicit empty replacement generation, so Rust
	 * cannot retain images from a resource pack that has already been invalidated.
	 */
	static void invalidateGuiRawImages() {
		synchronized (LOCK) {
			GUI_ATLAS_REFERENCES.invalidate();
			pendingRawImages.clear();
			rawImageGeneration++;
			attemptedRawImageGeneration = Math.min(attemptedRawImageGeneration, uploadedRawImageGeneration);
		}
	}

	static void stageGuiAtlasReference(GuiAtlasRegion reference) {
		synchronized (LOCK) {
			GUI_ATLAS_REFERENCES.stage(reference);
		}
	}

	public static void executeGuiFrame(Minecraft minecraft, List<RustGalGuiElementRenderState> elements) {
		if (elements.isEmpty()) {
			// A producer can enqueue semantic work before a screen decides that its
			// current frame has no drawable elements.  Those tokens cannot be
			// consumed by a later frame without violating ordering; discard them at
			// the frame boundary so large loading-screen payloads do not accumulate
			// indefinitely in the scheduler.
			synchronized (LOCK) {
				int cancelled = SCHEDULER.cancelAll("empty-frame");
				METRICS.cancellations += cancelled == 0 ? 0 : 1;
				METRICS.batchesCancelled += cancelled;
			}
			return;
		}
		RustGalGuiRenderer.GuiExecutionRoute route = RustGalGuiRenderer.currentExecutionRoute();
		if (route != RustGalGuiRenderer.GuiExecutionRoute.RUST_OPENGL_BORROWED_CONTEXT) {
			throw new IllegalStateException("Rust OpenGL borrowed-context GUI execution requires route "
				+ RustGalGuiRenderer.GuiExecutionRoute.RUST_OPENGL_BORROWED_CONTEXT + "; current route is " + route);
		}
		for (RustGalGuiElementRenderState element : elements) {
			if (!element.stratum().supportedForPartialFrame()) {
				throw new IllegalArgumentException("unsupported Rust GAL GUI stratum: " + element.stratum().id());
			}
		}
		ensureRenderThreadAndContext(minecraft);
		Window window = minecraft.getWindow();
		ensureConfigured(window);
		synchronized (LOCK) {
			flushPendingGuiAssetsLocked();
			List<RustGalFrameScheduler.Token> tokens = elements.stream().map(RustGalGuiElementRenderState::token).toList();
			List<RustGalFrameScheduler.Item<QueuedGuiRequest>> requests = SCHEDULER.takeAllItems(tokens, generation);
			executeFrameBatches(window, requests, false, window.getGuiScaledWidth(), window.getGuiScaledHeight(), -1, -1, null, null);
		}
	}

	public static boolean executeWorldPrimitiveFrame(
		Minecraft minecraft,
		RustGalWorldPrimitiveRenderer.PrimitiveFrame primitiveFrame,
		String producerLabel
	) {
		if (primitiveFrame == null
			|| ((primitiveFrame.background() == null || !primitiveFrame.background().enabled())
			&& primitiveFrame.segments().isEmpty()
				&& primitiveFrame.crackQuads().isEmpty()
				&& primitiveFrame.borderQuads().isEmpty()
				&& primitiveFrame.materialQuads().isEmpty() && primitiveFrame.particleQuads().isEmpty()
				&& primitiveFrame.textQuads().isEmpty()
				&& primitiveFrame.meshInstances().isEmpty()
				&& primitiveFrame.lodInstances().isEmpty()
				&& primitiveFrame.firstPersonMeshInstances().isEmpty()
				&& primitiveFrame.entityFlameQuadCount() == 0)) {
			if (RustGalGuiRenderer.isWholeFrameVulkanActive()) {
				throw new IllegalStateException(
					"Rust Vulkan whole-frame primitive submission received no semantic world frame; "
						+ "an empty frame cannot fall through to Java rendering"
				);
			}
			return false;
		}
		RustGalGuiRenderer.GuiExecutionRoute route = RustGalGuiRenderer.currentExecutionRoute();
		if (route != RustGalGuiRenderer.GuiExecutionRoute.RUST_OPENGL_BORROWED_CONTEXT) {
			throw new IllegalStateException("Rust OpenGL borrowed-context world primitive execution requires route "
				+ RustGalGuiRenderer.GuiExecutionRoute.RUST_OPENGL_BORROWED_CONTEXT + "; current route is " + route);
		}
		ensureRenderThreadAndContext(minecraft);
		Window window = minecraft.getWindow();
		ensureConfigured(window);
		synchronized (LOCK) {
			flushPendingWorldAssetsLocked();
			long executeStarted = System.nanoTime();
			GraphicsFrameBenchmark.beginPhase("rust-gal.world-primitives.execute");
			long correlationId = nextCorrelationId++;
			long frameId = 0L;
			long submissionId = 0L;
			boolean executeCounted = false;
			try {
				GraphicsFrameBenchmark.beginPhase("rust-gal.world-primitives.ffi.acquire");
				long acquireStarted = System.nanoTime();
				recordFixedOperation(Operation.FRAME_ACQUIRE, VulkanicGalBridge.Struct.FRAME_ACQUIRE.byteSize());
				VulkanicGalBridge.AcquiredFrame frame = bridge.acquireFrame(correlationId, window.getWidth(), window.getHeight());
				METRICS.frameAcquireNanos += elapsedSince(acquireStarted);
				GraphicsFrameBenchmark.endPhase("rust-gal.world-primitives.ffi.acquire");
				frameId = frame.frameId();
				if (frame.status() == 4 || frame.frameTarget() == 0L) {
					METRICS.cancellations++;
					return false;
				}
				GraphicsFrameBenchmark.beginPhase("rust-gal.world-primitives.submit-call");
				long packingStarted = System.nanoTime();
				VulkanicGalBridge.WholeFrameSubmitResult result = bridge.submitWorldPrimitives(
					generation,
					frameId,
					correlationId,
					frame.frameTarget(),
					primitiveFrame.viewportWidth() <= 0 ? window.getWidth() : primitiveFrame.viewportWidth(),
					primitiveFrame.viewportHeight() <= 0 ? window.getHeight() : primitiveFrame.viewportHeight(),
					primitiveFrame.viewMatrix(),
					primitiveFrame.projectionMatrix(),
					primitiveFrame.segments(),
					primitiveFrame.crackQuads(),
					primitiveFrame.borderQuads(),
					primitiveFrame.materialQuads(),
					primitiveFrame.meshInstances()
				);
				METRICS.abiPackingNanos += elapsedSince(packingStarted);
				GraphicsFrameBenchmark.endPhase("rust-gal.world-primitives.submit-call");
				recordStatus(Operation.SUBMIT, result.asStatus());
				submissionId = result.submissionId();
				lastSubmitted = Math.max(lastSubmitted, submissionId);
				GraphicsFrameBenchmark.beginPhase("rust-gal.world-primitives.ffi.present");
				long presentStarted = System.nanoTime();
				recordFixedOperation(Operation.FRAME_PRESENT, VulkanicGalBridge.Struct.FRAME_PRESENT.byteSize());
				bridge.presentFrame(frameId, correlationId, submissionId);
				METRICS.framePresentNanos += elapsedSince(presentStarted);
				GraphicsFrameBenchmark.endPhase("rust-gal.world-primitives.ffi.present");
				METRICS.frames++;
				METRICS.submissions++;
				recordWorldMetrics(result);
				TracyCompat.message("gal.frame.deferred producer=" + producerLabel
					+ " stratum=world.primitives frame=" + frameId + " submission=" + submissionId
					+ " segments=" + result.worldSegmentCount()
					+ " crackQuads=" + result.worldCrackQuadCount()
					+ " borderQuads=" + result.worldBorderQuadCount()
					+ " materialQuads=" + result.worldMaterialQuadCount()
					+ " meshInstances=" + result.worldMeshInstanceCount());
				retireOutstanding(forceDeterministicCaptureRetirement());
				auditMessage(metricsAuditLine(0, frameId, submissionId, false));
				METRICS.executeNanos += elapsedSince(executeStarted);
				executeCounted = true;
				return true;
			} finally {
				if (!executeCounted) {
					METRICS.executeNanos += elapsedSince(executeStarted);
				}
				GraphicsFrameBenchmark.endPhase("rust-gal.world-primitives.execute");
			}
		}
	}

	public static void executeWholeFrameVulkan(Minecraft minecraft, GuiRenderState renderState) {
		executeWholeFrameVulkan(minecraft, renderState, null);
	}

	/** Semantic post-effect identity copied from the active GameRenderer state. */
	public static void executeWholeFrameVulkan(Minecraft minecraft, GuiRenderState renderState, String postEffectId) {
		executeWholeFrameVulkan(minecraft, renderState, postEffectId, null);
	}

	public static void executeWholeFrameVulkan(Minecraft minecraft, GuiRenderState renderState,
		String postEffectId, VulkanicGalBridge.EngineGlobalsRecord engineGlobals) {
		if (!RustGalGuiRenderer.isWholeFrameVulkanActive()) {
			throw new IllegalStateException("Rust Vulkan whole-frame shell requires "
				+ "an admitted Vulkan backend selection (or "
				+ RustGalVulkanWholeFrameMode.propertyName() + " for bootstrap diagnostics)");
		}
		if (!VulkanicAPI.isVulkanBackendSelected()) {
			throw new IllegalStateException("Rust Vulkan whole-frame shell requires Vulkan backend selection at startup");
		}
		if (renderState == null) {
			throw new IllegalStateException("Rust Vulkan whole-frame shell requires copied GUI render state");
		}
		int unsupportedGuiElements = RustGalGuiRenderer.wholeFrameUnsupportedElementCount();
		if (unsupportedGuiElements != 0) {
			throw new IllegalStateException(
				"Rust Vulkan whole-frame GUI contains " + unsupportedGuiElements
					+ " unsupported semantic element(s) ["
					+ RustGalGuiRenderer.wholeFrameUnsupportedElementSummary()
					+ "]; Java GUI rendering is not a same-frame fallback"
			);
		}
		int unsupportedWorldText = RustGalWorldPrimitiveRenderer.pendingUnsupportedWorldTextSubmits();
		if (unsupportedWorldText != 0) {
			throw new IllegalStateException(
				"Rust Vulkan whole-frame world text contains " + unsupportedWorldText
					+ " unsupported semantic submission(s); Java text rendering is not a same-frame fallback"
			);
		}
		int unsupportedFirstPersonItems = RustGalWorldPrimitiveRenderer.pendingUnsupportedFirstPersonItems();
		if (unsupportedFirstPersonItems != 0) {
			throw new IllegalStateException(
				"Rust Vulkan whole-frame first-person rendering contains " + unsupportedFirstPersonItems
					+ " unsupported item(s); Java hand rendering is not a same-frame fallback"
			);
		}
		int unsupportedCustomGeometry = RustGalWorldPrimitiveRenderer.pendingUnsupportedCustomGeometry();
		if (unsupportedCustomGeometry != 0) {
			throw new IllegalStateException(
				"Rust Vulkan whole-frame contains " + unsupportedCustomGeometry
					+ " unsupported custom-geometry callback(s); Java geometry is not a same-frame fallback"
			);
		}
		int unsupportedParticleGroups = RustGalWorldPrimitiveRenderer.pendingUnsupportedParticleGroups();
		if (unsupportedParticleGroups != 0) {
			throw new IllegalStateException(
				"Rust Vulkan whole-frame contains " + unsupportedParticleGroups
					+ " unsupported particle group(s); Java particle rendering is not a same-frame fallback"
			);
		}
		ensureRenderThreadAndWindowedVulkanContext(minecraft);
		Window window = minecraft.getWindow();
		ensureConfigured(window);
		List<RustGalFrameScheduler.Item<QueuedGuiRequest>> requests;
		int blurRadius = Math.clamp(minecraft.options.getMenuBackgroundBlurriness(), 0, 64);
		synchronized (LOCK) {
			flushPendingGuiAssetsLocked();
			List<RustGalFrameScheduler.Token> tokens = new ArrayList<>();
			int[] rustElementCount = {0};
			renderState.forEachElement(element -> {
				if (element instanceof RustGalGuiElementRenderState rustGalElement) {
					rustElementCount[0]++;
					tokens.add(rustGalElement.token());
				}
			}, GuiRenderState.TraverseRange.ALL);
			requests = SCHEDULER.takeAllItems(tokens, generation);
			if (Boolean.getBoolean("mattmc.dev.graphicsAuditSliceMetrics")) {
				auditMessage("Rust GUI render-state Rust elements=" + rustElementCount[0]
					+ " scheduler_tokens=" + tokens.size() + " admitted_requests=" + requests.size());
			}
		}
		if (Boolean.getBoolean("mattmc.dev.graphicsAuditSliceMetrics")) {
			int meshItems = 0;
			int meshBatches = 0;
			java.util.Set<Long> meshAssetIds = new java.util.LinkedHashSet<>();
			java.util.Set<String> affineAssets = new java.util.LinkedHashSet<>();
			for (RustGalFrameScheduler.Item<QueuedGuiRequest> request : requests) {
				for (VulkanicGalBridge.GuiAffineQuadRecord affine : request.payload().affineQuads()) {
					affineAssets.add(affine.stratum() + ":" + affine.assetId());
				}
				if (request.payload().meshBatches().isEmpty()) continue;
				meshItems++;
				meshBatches += request.payload().meshBatches().size();
				for (VulkanicGalBridge.GuiMeshBatchRecord batch : request.payload().meshBatches()) {
					meshAssetIds.add(batch.assetId());
				}
			}
				auditMessage("Rust GUI extracted mesh items=" + meshItems
					+ " batches=" + meshBatches + " asset_ids=" + meshAssetIds);
			auditMessage("Rust GUI extracted affine strata/assets=" + affineAssets);
		}
		// Native Vulkan execution must not monopolize the semantic producer lock.
		executeFrameBatches(window, requests, true, window.getGuiScaledWidth(), window.getGuiScaledHeight(), renderState.blurBeforeStratumIndex(), blurRadius,
			normalizeSemanticPostEffectId(postEffectId), engineGlobals);
		// The Java GUI renderer normally resets this state after its draw pass.  The
		// whole-frame route replaces that pass, so it must retire the copied semantic
		// state here or every screen frame would accumulate prior elements forever.
		renderState.reset();
		// The Rust presenter is the deterministic capture boundary. Advance the
		// capture only after executeFrameBatches has acquired, submitted, and
		// presented this whole-frame Vulkan submission, and after the coordinator
		// lock is released so capture/readiness diagnostics cannot participate in
		// a backend asset-lock cycle.
		net.minecraft.client.dev.DeterministicCameraCapture.afterRender(minecraft);
	}

	/**
	 * Bounds the only post-effect value crossing the Java/Rust semantic
	 * boundary. The Rust decoder repeats these checks, but rejecting malformed
	 * identities here keeps the FFI request itself a bounded semantic value and
	 * never transports a Java post-chain object or renderer state.
	 */
	private static String normalizeSemanticPostEffectId(String postEffectId) {
		if (postEffectId == null || postEffectId.isEmpty()) return postEffectId;
		if (postEffectId.trim().isEmpty() || postEffectId.chars().anyMatch(Character::isISOControl)) {
			throw new IllegalArgumentException("Rust Vulkan post-effect identity must be non-empty and control-free");
		}
		if (postEffectId.getBytes(StandardCharsets.UTF_8).length > 256) {
			throw new IllegalArgumentException("Rust Vulkan post-effect identity exceeds 256 UTF-8 bytes");
		}
		try {
			ResourceLocation.parse(postEffectId);
		} catch (RuntimeException error) {
			throw new IllegalArgumentException("Rust Vulkan post-effect identity is not a valid resource location", error);
		}
		return postEffectId;
	}

	public static void resize(int width, int height) {
		synchronized (LOCK) {
			configuredWidth = 0;
			configuredHeight = 0;
			int cancelled = SCHEDULER.cancelAll("resize");
			retireOutstanding(true);
			METRICS.cancellations++;
			METRICS.batchesCancelled += cancelled;
		}
	}

	/** True only when a non-disabled shader-pack source snapshot is staged for Rust. */
	public static boolean isRustShaderPackSourceReady() {
		synchronized (LOCK) {
			return pendingShaderPackSources != null
				&& !"disabled".equals(pendingShaderPackSources.packName())
				&& !pendingShaderPackSources.packName().startsWith("minecraft-resource-pack:")
				&& !pendingShaderPackSources.files().isEmpty();
		}
	}

	public static void reload(ResourceManager resourceManager) {
		RustGalGuiRenderer.invalidateLoadingGridAsset();
		if (RustGalGuiRenderer.assetUpdatesDisabled()) {
			auditMessage("Rust VulkanicGAL GUI asset update skipped reason=diagnostic-disabled");
			return;
		}
		List<VulkanicGalBridge.GuiAssetRecord> assets = RustGalGuiRenderer.collectResolvedAssets(resourceManager);
		RustGalGuiItemRenderer.invalidateAssets();
		RustGalWorldPrimitiveRenderer.reloadWorldAssets(resourceManager);
		long shaderPackSourceGeneration;
		synchronized (LOCK) {
			shaderPackSourceGeneration = nextShaderPackSourceGeneration++;
		}
		RustShaderPackSourceCollector.SourceGeneration shaderPackSources = null;
		try {
			shaderPackSources = RustShaderPackSourceCollector.collectConfiguredPack(shaderPackSourceGeneration);
		} catch (IOException | RuntimeException error) {
			LOGGER.error("Rust VulkanicGAL shader-pack source collection failed; preserving the prior complete source generation", error);
			auditMessage("Rust VulkanicGAL shader-pack source collection failed generation=" + shaderPackSourceGeneration
				+ " preserve_last_valid=true");
		}
		synchronized (LOCK) {
			generation++;
			assetGeneration++;
			pendingAssets = assets;
			if (shaderPackSources != null) {
				stageShaderPackSourcesLocked(shaderPackSources);
			}
			attemptedAssetGeneration = Math.min(attemptedAssetGeneration, uploadedAssetGeneration);
			int cancelled = SCHEDULER.cancelAll("resource-reload");
			METRICS.reloadInvalidations++;
			METRICS.batchesCancelled += cancelled;
			retireOutstanding(true);
			flushPendingGuiAssetsLocked();
			flushPendingWorldAssetsLocked();
		}
	}

	public static void cancelPending(String reason) {
		synchronized (LOCK) {
			int cancelled = SCHEDULER.cancelAll(reason);
			METRICS.cancellations++;
			METRICS.batchesCancelled += cancelled;
		}
	}

	public static void shutdown() {
		RustGalGuiRenderer.invalidateLoadingGridAsset();
		VulkanicGalBridge existing;
		synchronized (LOCK) {
			int cancelled = SCHEDULER.cancelAll("shutdown");
			existing = bridge;
			retireOutstanding(true);
			auditMessage(metricsAuditLine(0L, METRICS.frames, lastSubmitted, RustGalGuiRenderer.isWholeFrameVulkanEnabled()));
			bridge = null;
			GUI_ATLAS_REFERENCES.resetAcceptance();
			bridgeMode = BridgeMode.NONE;
			RustGalVulkanWholeFrameMode.deactivateRustPresentation();
			renderThread = null;
			lastSubmitted = 0L;
			lastRetiredSubmission = 0L;
			uploadedRawImageGeneration = 0L;
			attemptedRawImageGeneration = 0L;
			uploadedShaderPackSourceGeneration = 0L;
			attemptedShaderPackSourceGeneration = 0L;
			uploadedShaderPackAssetGeneration = 0L;
			attemptedShaderPackAssetGeneration = 0L;
			pendingShaderPackSourceName = "";
			configuredWidth = 0;
			configuredHeight = 0;
			METRICS.cancellations++;
			METRICS.batchesCancelled += cancelled;
		}
		if (existing != null) {
			try {
				existing.shutdownFrame();
			} finally {
				existing.close();
			}
		}
	}

	public static MetricsSnapshot metricsSnapshot() {
		synchronized (LOCK) {
			return new MetricsSnapshot(
				METRICS.frames,
				METRICS.submissions,
				METRICS.cacheHits,
				METRICS.cacheMisses,
				METRICS.resourceCreates,
				METRICS.resourceDestroys,
				METRICS.ffiCalls,
				METRICS.ffiBytes,
				METRICS.cancellations,
				METRICS.reloadInvalidations,
				METRICS.completionPolls,
				METRICS.completionTimeouts,
				SCHEDULER.pendingCount(),
				METRICS.batchesExecuted,
				METRICS.spriteBatchesExecuted,
				METRICS.packedSpritesExecuted,
				METRICS.batchesCancelled,
				METRICS.contextCreateCalls,
				METRICS.capabilityCalls,
				METRICS.frameConfigureCalls,
				METRICS.frameAcquireCalls,
				METRICS.frameResizeCalls,
				METRICS.framePresentCalls,
				METRICS.resourceBatchCalls,
				METRICS.submitCalls,
				METRICS.completionQueryCalls,
				METRICS.retireCalls,
				METRICS.contextCreateBytes,
				METRICS.capabilityBytes,
				METRICS.frameConfigureBytes,
				METRICS.frameAcquireBytes,
				METRICS.frameResizeBytes,
				METRICS.framePresentBytes,
				METRICS.resourceBatchBytes,
				METRICS.submitBytes,
				METRICS.completionQueryBytes,
				METRICS.retireBytes,
				METRICS.enqueueNanos,
				METRICS.resourceLookupNanos,
				METRICS.resourceCreateNanos,
				METRICS.abiPackingNanos,
				METRICS.frameAcquireNanos,
				METRICS.submitNanos,
				METRICS.framePresentNanos,
				METRICS.retireNanos,
				METRICS.completionQueryNanos,
				METRICS.executeNanos,
				METRICS.commandLists,
				METRICS.commandOps,
				METRICS.backendSubmissions,
				METRICS.backendWaits,
				METRICS.glCalls,
				METRICS.glFlushes,
				METRICS.glFinishes,
				METRICS.glFencesInserted,
				METRICS.glFencesPolled,
				METRICS.glFencesWaited,
				METRICS.glFencesDeleted
			);
		}
	}

	public static String currentAuditMetricsLine() {
		synchronized (LOCK) {
			return metricsAuditLine(0L, METRICS.frames, lastSubmitted, RustGalGuiRenderer.isWholeFrameVulkanEnabled());
		}
	}

	static void auditMessage(String message) {
		if (Boolean.getBoolean("mattmc.dev.graphicsAuditSliceMetrics")) {
			System.out.println("[MattMC graphics audit] " + message);
		}
	}

	private static void executeFrameBatches(
		Window window,
		List<RustGalFrameScheduler.Item<QueuedGuiRequest>> requests,
		boolean allowEmpty,
		int guiWidth,
		int guiHeight,
		int guiBlurBeforeStratum,
		int guiBlurRadius,
		String postEffectId,
		VulkanicGalBridge.EngineGlobalsRecord engineGlobals
	) {
		if (requests.isEmpty() && !allowEmpty) {
			return;
		}
		if (requests.size() > 4096 && Boolean.getBoolean("mattmc.dev.graphicsAuditSliceMetrics")) {
			Map<String, Integer> stratumCounts = new LinkedHashMap<>();
			for (RustGalFrameScheduler.Item<QueuedGuiRequest> request : requests) {
				stratumCounts.merge(request.token().stratumId(), 1, Integer::sum);
			}
			auditMessage("Rust GUI semantic batch pressure count=" + requests.size()
				+ " strata=" + stratumCounts);
		}
		long executeStarted = System.nanoTime();
		GraphicsFrameBenchmark.beginPhase("rust-gal.gui-frame.execute");
		long correlationId = nextCorrelationId++;
		long frameId = 0L;
		long submissionId = 0L;
		boolean executeCounted = false;
		boolean frameCancelled = false;
		RustGalWorldPrimitiveRenderer.PrimitiveFrame primitiveFrame = null;
		boolean wholeFrameVulkan = allowEmpty && RustGalGuiRenderer.isWholeFrameVulkanActive();
		boolean renderdocFrameCaptureStarted = false;
		long acquireStarted = 0L;
		long acquireEnded = 0L;
		long submitStarted = 0L;
		long submitEnded = 0L;
		long presentStarted = 0L;
		long presentEnded = 0L;
		try {
			if (wholeFrameVulkan) {
				GraphicsFrameBenchmark.beginPhase("rust-gal.frame.consume-and-flush-world");
				synchronized (LOCK) {
					refreshConfiguredShaderPackSourcesLocked();
					flushPendingWorldAssetsLocked();
					// Publish immutable world assets before freezing the semantic frame.
					// A frame must never contain a visible reference to the generation that
					// an asset update replaces immediately before native submission.
					primitiveFrame = RustGalWorldPrimitiveRenderer.consumeFrame();
					assertWholeFrameFeatureCoverage(primitiveFrame.featureCoverage());
					// This is a per-frame observation, not a lifetime capability.  A
					// later empty semantic frame must not satisfy deterministic capture
					// readiness and cause a screenshot of the diagnostic shell.
					observedRenderableWholeFrameWorld = false;
					// Consuming a selected semantic frame may publish an immutable shared
					// resource for the first time. DH exact-atlas columns do this for the
					// copied block atlas. Flush it before this same frame reaches native
					// execution; otherwise the first selected draw observes no atlas and
					// can only fail or render with an unrelated later-frame resource.
					flushPendingWorldAssetsAfterFrameConsumeLocked();
				}
				GraphicsFrameBenchmark.endPhase("rust-gal.frame.consume-and-flush-world");
				if (!primitiveFrame.segments().isEmpty()
					|| !primitiveFrame.crackQuads().isEmpty()
					|| !primitiveFrame.borderQuads().isEmpty()
					|| !primitiveFrame.materialQuads().isEmpty() || !primitiveFrame.particleQuads().isEmpty()
					|| !primitiveFrame.meshInstances().isEmpty()) {
					RenderDocCaptureHook.triggerNextFrameOnce(
						"rust-vulkan-whole-frame-world#" + correlationId
							+ "-segments=" + primitiveFrame.segments().size()
							+ "-crackQuads=" + primitiveFrame.crackQuads().size()
							+ "-borderQuads=" + primitiveFrame.borderQuads().size()
							+ "-materialQuads=" + primitiveFrame.materialQuads().size()
							+ "-meshInstances=" + primitiveFrame.meshInstances().size()
					);
					renderdocFrameCaptureStarted = RenderDocCaptureHook.beginFrameCaptureOnce(
						window,
						"rust-vulkan-whole-frame-world#" + correlationId
							+ "-segments=" + primitiveFrame.segments().size()
							+ "-crackQuads=" + primitiveFrame.crackQuads().size()
							+ "-borderQuads=" + primitiveFrame.borderQuads().size()
							+ "-materialQuads=" + primitiveFrame.materialQuads().size()
							+ "-meshInstances=" + primitiveFrame.meshInstances().size()
					);
				}
			}
			GraphicsFrameBenchmark.beginPhase("rust-gal.gui-frame.ffi.acquire");
			acquireStarted = System.nanoTime();
			recordFixedOperation(Operation.FRAME_ACQUIRE, VulkanicGalBridge.Struct.FRAME_ACQUIRE.byteSize());
			VulkanicGalBridge.AcquiredFrame frame = bridge.acquireFrame(correlationId, window.getWidth(), window.getHeight());
			acquireEnded = System.nanoTime();
			METRICS.frameAcquireNanos += Math.max(0L, acquireEnded - acquireStarted);
			GraphicsFrameBenchmark.endPhase("rust-gal.gui-frame.ffi.acquire");
			frameId = frame.frameId();
			if (wholeFrameVulkan) {
				lastAcquiredWholeFrameFrame = frameId;
				recordWholeFrameAcquire(frame, correlationId);
			}
			if (frame.status() == 4 || frame.frameTarget() == 0L) {
				int cancelled;
				synchronized (LOCK) {
					cancelled = SCHEDULER.cancelFrame(frameId, "acquire-skipped");
				}
				frameCancelled = true;
				METRICS.cancellations++;
				METRICS.batchesCancelled += cancelled;
				return;
			}
			if (wholeFrameVulkan && primitiveFrame != null) {
				GraphicsFrameBenchmark.beginPhase("rust-gal.frame.viewport-seed");
				primitiveFrame = RustGalWorldPrimitiveRenderer.withViewport(
					primitiveFrame,
					Math.max(1, frame.width()),
					Math.max(1, frame.height())
				);
				GraphicsFrameBenchmark.endPhase("rust-gal.frame.viewport-seed");
			}
			if (wholeFrameVulkan) {
				writeWholeFrameAttachmentCaptureRequest(
					frame,
					correlationId,
					net.minecraft.client.dev.DeterministicCameraCapture
						.claimWholeFrameAttachmentCaptureRenderedFrameIndex()
				);
			}

			GraphicsFrameBenchmark.beginPhase("rust-gal.frame.submit-call");
			submitStarted = System.nanoTime();
			long packingStarted = submitStarted;
			int frameGuiWidth = requests.isEmpty() ? guiWidth : requests.get(0).payload().guiWidth();
			int frameGuiHeight = requests.isEmpty() ? guiHeight : requests.get(0).payload().guiHeight();
			// Copy the same exact semantic orthographic extent as vanilla. Layout
			// uses ceil(width/scale), but projection must retain the fractional part.
			var guiProjection = new VulkanicGalBridge.GuiProjectionRecord(
				(float)window.getWidth() / window.getGuiScale(),
				(float)window.getHeight() / window.getGuiScale());
			// Loading screens can legitimately contain tens of thousands of ordered
			// semantic items.  Reserve the bounded request cardinality up front so
			// ArrayList growth does not retain several transient backing arrays while
			// the FFI arena is packed; this changes no ordering or admission policy.
			int requestCapacity = Math.min(requests.size(), RustGalFrameScheduler.SEQUENCE_STRIDE > Integer.MAX_VALUE
				? Integer.MAX_VALUE : (int) RustGalFrameScheduler.SEQUENCE_STRIDE);
			List<VulkanicGalBridge.GuiSpriteRecord> spriteRequests = new ArrayList<>(requestCapacity);
			List<VulkanicGalBridge.GuiAffineQuadRecord> affineQuadRequests = new ArrayList<>(requestCapacity);
			List<VulkanicGalBridge.GuiMeshBatchRecord> meshBatchRequests = new ArrayList<>(requestCapacity);
			List<VulkanicGalBridge.GuiTiledQuadRecord> tiledQuadRequests = new ArrayList<>(requests.size());
			int guiTextAffineQuadCount = 0;
			int guiItemAffineQuadCount = 0;
			int meshVertexCount = 0;
			int meshIndexCount = 0;
			for (RustGalFrameScheduler.Item<QueuedGuiRequest> request : requests) {
				int incomingAffineQuadCount = request.payload().affineQuads().size();
				if ((long)affineQuadRequests.size() + incomingAffineQuadCount > MAX_RUST_GUI_AFFINE_QUADS) {
					throw new IllegalStateException(
						"Rust whole-frame GUI affine-quad capacity exceeded " + MAX_RUST_GUI_AFFINE_QUADS
					);
				}
				if (!request.payload().meshBatches().isEmpty()) {
					long incomingVertices = 0L;
					long incomingIndices = 0L;
					for (VulkanicGalBridge.GuiMeshBatchRecord batch : request.payload().meshBatches()) {
						incomingVertices += batch.vertices().size();
						incomingIndices += batch.indices().size();
					}
					if (meshBatchRequests.size() + request.payload().meshBatches().size() > MAX_RUST_GUI_MESH_BATCHES
						|| (long)meshVertexCount + incomingVertices > MAX_RUST_GUI_MESH_VERTICES
						|| (long)meshIndexCount + incomingIndices > MAX_RUST_GUI_MESH_INDICES) {
						throw new IllegalStateException(
							"Rust whole-frame GUI mesh capacity exceeded batches=" + MAX_RUST_GUI_MESH_BATCHES
								+ " vertices=" + MAX_RUST_GUI_MESH_VERTICES + " indices=" + MAX_RUST_GUI_MESH_INDICES
							);
					}
					meshVertexCount = Math.addExact(meshVertexCount, Math.toIntExact(incomingVertices));
					meshIndexCount = Math.addExact(meshIndexCount, Math.toIntExact(incomingIndices));
				}
				request.payload().appendTo(spriteRequests, affineQuadRequests, meshBatchRequests, tiledQuadRequests, request.token().sequence());
				if (!request.payload().affineQuads().isEmpty()) {
					if (request.token().stratumId().equals(GuiRenderStratum.GUI_TEXT.id())) {
						guiTextAffineQuadCount += request.payload().affineQuads().size();
					} else if (request.token().stratumId().equals(GuiRenderStratum.GUI_ITEM.id())) {
						guiItemAffineQuadCount += request.payload().affineQuads().size();
					}
				}
			}
			VulkanicGalBridge.WholeFrameSubmitResult wholeFrameResult = null;
			VulkanicGalBridge.GuiFrameSubmitResult guiResult = null;
			// All world uploads have completed before immutable GUI declarations
			// are accepted. Rejection propagates; never render copied fallback pixels.
			GUI_ATLAS_REFERENCES.retainUsedCommands(affineQuadRequests, tiledQuadRequests, meshBatchRequests);
			GUI_ATLAS_REFERENCES.flush(RustGalWorldPrimitiveRenderer::requireAcceptedWorldMeshTextureGeneration,
				bridge::updateGuiAtlasReferences);
			if (wholeFrameVulkan) {
				if (primitiveFrame == null) {
					GraphicsFrameBenchmark.beginPhase("rust-gal.frame.consume-world-frame");
					primitiveFrame = RustGalWorldPrimitiveRenderer.consumeFrame();
					GraphicsFrameBenchmark.endPhase("rust-gal.frame.consume-world-frame");
				}
				GraphicsFrameBenchmark.beginPhase("rust-gal.frame.viewport-seed");
				primitiveFrame = RustGalWorldPrimitiveRenderer.withViewport(
					primitiveFrame,
					Math.max(1, frame.width()),
					Math.max(1, frame.height())
				);
				GraphicsFrameBenchmark.endPhase("rust-gal.frame.viewport-seed");
				// Cross-backend parity names describe semantic workload families, not
				// ownership of the implementation.  Mark the families only when this
				// Rust frame contains the corresponding owned work; this keeps the
				// comparator honest without pretending that Sodium or DH Java renderers
				// ran on the Rust route.
				RustGalWorldPrimitiveRenderer.requireAcceptedParticleTextures(primitiveFrame.materialQuads());
				RustGalWorldPrimitiveRenderer.requireAcceptedSemanticParticleTextures(primitiveFrame.particleQuads());
				wholeFrameResult = bridge.submitWholeFrameWithAffineGuiAndWorldTextAndFirstPerson(
					generation,
					frameId,
					correlationId,
					frame.frameTarget(),
					frameGuiWidth,
					frameGuiHeight,
					primitiveFrame.viewportWidth() <= 0 ? window.getWidth() : primitiveFrame.viewportWidth(),
					primitiveFrame.viewportHeight() <= 0 ? window.getHeight() : primitiveFrame.viewportHeight(),
					primitiveFrame.viewMatrix(),
					primitiveFrame.projectionMatrix(),
					primitiveFrame.background(),
					primitiveFrame.segments(),
					primitiveFrame.crackQuads(),
					primitiveFrame.borderQuads(),
					primitiveFrame.materialQuads(),
					primitiveFrame.meshInstances(),
					primitiveFrame.voxelVolumeFrame(),
					primitiveFrame.shaderEnvironmentFrame(),
					primitiveFrame.lodInstances(),
					primitiveFrame.lodRenderFrame(),
					primitiveFrame.featureCoverage(),
					spriteRequests,
					affineQuadRequests,
					meshBatchRequests,
					RustGalWorldPrimitiveRenderer.encodeWorldTextQuads(primitiveFrame.textQuads()),
					primitiveFrame.firstPersonFrame(),
					primitiveFrame.firstPersonMeshInstances(),
					guiBlurBeforeStratum,
					guiBlurRadius,
					postEffectId,
					guiProjection,
					tiledQuadRequests,
					engineGlobals,
					primitiveFrame.particleQuads(),
					primitiveFrame.orbInstances()
				);
				if (Boolean.getBoolean("mattmc.dev.graphicsAuditSliceMetrics")) {
					auditMessage("Rust GUI whole-frame result mesh items=" + wholeFrameResult.guiMeshItemCount()
						+ " batches=" + wholeFrameResult.guiMeshBatchCount()
						+ " draws=" + wholeFrameResult.guiMeshDrawCount());
				}
				if (primitiveFrame.background().enabled()
					&& wholeFrameResult.worldBackgroundDiagnosticFallbackCount() != 0) {
					throw new IllegalStateException(
						"Rust Vulkan whole-frame admitted a semantic world background but used "
							+ wholeFrameResult.worldBackgroundDiagnosticFallbackCount()
							+ " diagnostic background fallback(s)"
					);
				}
				// Readiness is based on work that the Rust GAL actually admitted and
				// submitted, rather than merely on a queued semantic snapshot.
				observedRenderableWholeFrameWorld = wholeFrameResult.worldMeshInstanceCount() > 0
					|| wholeFrameResult.worldDrawCount() > 0
					|| wholeFrameResult.worldMaterialDrawCount() > 0;
				if (observedRenderableWholeFrameWorld) {
					lastRenderableWholeFrameWorldFrame = frameId;
				}
			} else {
				guiResult = bridge.submitGuiFrame(
					generation,
					frameId,
					frame.frameTarget(),
					frameGuiWidth,
					frameGuiHeight,
					spriteRequests,
					affineQuadRequests,
					meshBatchRequests,
					guiProjection,
					tiledQuadRequests
				);
			}
				submitEnded = System.nanoTime();
			METRICS.abiPackingNanos += Math.max(0L, submitEnded - packingStarted);
			GraphicsFrameBenchmark.endPhase("rust-gal.frame.submit-call");
			if (wholeFrameVulkan && primitiveFrame != null && wholeFrameResult != null
				&& primitiveFrame.shaderEnvironmentFrame().enabled()
				&& net.vulkanic.shaderpack.RustShaderPackSourceCollector.activeConfiguredPackName().isPresent()
				&& wholeFrameResult.profile().passCount() > 0) {
				// These are semantic shader-pack family samples, backed by the
				// explicit Rust frame's admitted pass graph. They are not Java Iris
				// callbacks and are intentionally emitted only when the Rust Vulkan
				// shader environment and at least one native pass were submitted.
				GraphicsFrameBenchmark.recordPhaseSample("iris.shadows", 1L);
				GraphicsFrameBenchmark.recordPhaseSample("iris.deferred-translucents", 1L);
				GraphicsFrameBenchmark.recordPhaseSample("iris.composite-final", 1L);
				// The Rust shader graph also performs a distinct shadow terrain
				// traversal (opaque and cutout), in addition to the three ordinary
				// semantic terrain layers emitted by the producer.
				GraphicsFrameBenchmark.recordPhaseSample("sodium.terrain.setup", 1L);
				GraphicsFrameBenchmark.recordPhaseSample("sodium.terrain.draw", 1L);
				GraphicsFrameBenchmark.recordPhaseSample("sodium.terrain.draw", 1L);
			}
			recordStatus(Operation.SUBMIT, wholeFrameResult != null ? wholeFrameResult.asStatus() : guiResult.asStatus());
			submissionId = wholeFrameResult != null ? wholeFrameResult.submissionId() : guiResult.submissionId();
			if (wholeFrameVulkan && primitiveFrame != null
				&& (primitiveFrame.lodRenderFrame().flags()
					& DistantHorizonsSemanticCollector.RENDER_FLAG_RUST_NON_WATER_ROUTE_SELECTED) != 0) {
				// The route-selection receipt is the authoritative semantic DH
				// workload boundary. Record parity-family phases here (rather than
				// relying on the next frame's consumed instance list), so a selected
				// Rust LOD frame remains visible to the comparator even when its
				// immutable instances are retired immediately after submission.
				if (lastDhParityPhaseFrame != frameId) {
					LOGGER.info("Rust Vulkan semantic DistantHorizons route selected frame={} instances={}",
						frameId, primitiveFrame.lodInstances().size());
					GraphicsFrameBenchmark.recordPhaseSample("distant-horizons.lod-render", 1L);
					GraphicsFrameBenchmark.recordPhaseSample("distant-horizons.translucent-fade", 1L);
					GraphicsFrameBenchmark.recordPhaseSample("distant-horizons.opaque-fade", 1L);
					if (primitiveFrame.shaderEnvironmentFrame().enabled()) {
						// Shader packs consume DH's transparent fade and its deferred
						// translucent LOD stage as separate semantic operations.
						GraphicsFrameBenchmark.recordPhaseSample("distant-horizons.translucent-fade", 1L);
					}
					lastDhParityPhaseFrame = frameId;
				}
				METRICS.worldLodSelectedFrames++;
				METRICS.worldLodInstancesSubmitted += primitiveFrame.lodInstances().size();
				METRICS.worldLodFramesExecuted++;
				if (!primitiveFrame.lodInstances().isEmpty()) {
					int opaqueInstances = (int)primitiveFrame.lodInstances().stream()
						.filter(instance -> instance.layer() == 1)
						.count();
					int transparentInstances = (int)primitiveFrame.lodInstances().stream()
						.filter(instance -> instance.layer() == 2 || instance.layer() == 3)
						.count();
					int waterInstances = (int)primitiveFrame.lodInstances().stream()
						.filter(instance -> instance.layer() == 4)
						.count();
					DistantHorizonsSemanticCollector.recordRustMaterialRouteExecution(
						frameId,
						submissionId,
						// Use the coordinator's armed attachment correlation when a
						// selected-source capture is active. This is the exact frame
						// identity shared by the Rust execution and screenshot receipt;
						// do not invent a route-specific frame offset.
						net.minecraft.client.dev.DeterministicCameraCapture.currentCaptureCorrelationRenderedFrameIndex(),
						primitiveFrame.lodInstances().size(),
						opaqueInstances,
						transparentInstances,
						waterInstances,
						primitiveFrame.lodRenderFrame().enabled(),
						primitiveFrame.lodInstances()
					);
					net.minecraft.client.dev.DeterministicCameraCapture.recordSubmittedWorkIdentityForCompletedFrame(
						"distant-horizons", "rust-vulkan-whole-frame:material-lod"
					);
				}
			}
			if (wholeFrameVulkan) {
				recordWholeFrameTerrainReadiness(primitiveFrame);
				if (wholeFrameResult.guiMeshItemCount() > 0L) {
					net.minecraft.client.dev.DeterministicCameraCapture.recordSubmittedWorkIdentity(
						"gui-standard-3d",
						"rust-vulkan-whole-frame:items=" + wholeFrameResult.guiMeshItemCount()
							+ ":batches=" + wholeFrameResult.guiMeshBatchCount()
							+ ":draws=" + wholeFrameResult.guiMeshDrawCount()
					);
				}
				RustGalWorldPrimitiveRenderer.recordWholeFrameMovingMeshExecution(
					frameId,
					submissionId,
					primitiveFrame
				);
				RustGalWorldPrimitiveRenderer.recordWholeFrameWeatherExecution(
					frameId,
					submissionId,
					primitiveFrame.materialQuads()
				);
				RustGalWorldPrimitiveRenderer.recordWholeFrameExperienceOrbExecution(
					frameId,
					submissionId,
					primitiveFrame.materialQuads(), primitiveFrame.orbInstances()
				);
				RustGalWorldPrimitiveRenderer.recordWholeFrameBeaconBeamExecution(
					frameId,
					submissionId,
					primitiveFrame.materialQuads()
				);
				RustGalWorldPrimitiveRenderer.recordWholeFrameCrystalBeamExecution(
					frameId,
					submissionId,
					primitiveFrame.materialQuads()
				);
				RustGalWorldPrimitiveRenderer.recordWholeFrameEntityFlameExecution(
					frameId,
					submissionId,
					primitiveFrame.entityFlameQuadCount()
				);
				RustGalWorldPrimitiveRenderer.recordWholeFrameEntityShadowExecution(
					frameId,
					submissionId,
					primitiveFrame.materialQuads()
				);
				RustGalWorldPrimitiveRenderer.recordWholeFrameEntityLeashExecution(
					frameId,
					submissionId,
					primitiveFrame.materialQuads()
				);
				RustGalWorldPrimitiveRenderer.recordWholeFrameCloudExecution(
					frameId,
					submissionId,
					primitiveFrame.materialQuads()
				);
				RustGalWorldPrimitiveRenderer.recordWholeFrameEntityModelExecution(
					frameId,
					submissionId,
					primitiveFrame.materialQuads()
				);
				RustGalWorldPrimitiveRenderer.recordWholeFrameProceduralQuadExecution(
					frameId,
					submissionId,
					primitiveFrame.materialQuads()
				);
				auditWholeFrameTarget(frame, primitiveFrame);
			}
			lastSubmitted = Math.max(lastSubmitted, submissionId);
			TracyCompat.message("gal.frame.deferred producer=gui.frame stratum=gui.frame"
				+ " frame=" + frameId + " submission=" + submissionId + " batches=" + requests.size());

			GraphicsFrameBenchmark.beginPhase("rust-gal.gui-frame.ffi.present");
			presentStarted = System.nanoTime();
			recordFixedOperation(Operation.FRAME_PRESENT, VulkanicGalBridge.Struct.FRAME_PRESENT.byteSize());
			VulkanicGalBridge.PresentedFrame presented = bridge.presentFrame(frameId, correlationId, submissionId);
			// Publish this successful submission's semantic execution evidence
			// before a final-output capture can acknowledge its presented image.
			if (wholeFrameResult != null) {
				RustGalTerrainRenderer.recordExecutedStaticTerrainInstances(
					primitiveFrame.meshInstances(), frameId, submissionId
				);
			}
				if (wholeFrameVulkan) {
					auditMessage("gal.frame.present backend=vulkan correlation=" + correlationId
						+ " frame=" + presented.frameId()
					+ " image=" + presented.frameTargetIdentity()
					+ " submission=" + submissionId
					+ " status=" + presented.status());
					net.minecraft.client.dev.DeterministicCameraCapture.recordWholeFramePresentation(
						net.minecraft.client.dev.DeterministicCameraCapture.currentCaptureCorrelationRenderedFrameIndex(),
						frame.frameId(),
						correlationId,
						submissionId,
						frame.frameTargetIdentity(),
						presented.frameTargetIdentity()
					);
						writeWholeFrameAttachmentCorrelation(
						frame,
						presented,
						wholeFrameResult,
					primitiveFrame,
					affineQuadRequests.size(),
					guiTextAffineQuadCount,
					guiItemAffineQuadCount
				);
				}
			presentEnded = System.nanoTime();
			METRICS.framePresentNanos += Math.max(0L, presentEnded - presentStarted);
			GraphicsFrameBenchmark.endPhase("rust-gal.gui-frame.ffi.present");
			if (renderdocFrameCaptureStarted) {
				RenderDocCaptureHook.endFrameCaptureOnce(window, "rust-vulkan-whole-frame-world#" + frameId + "-submission=" + submissionId);
				renderdocFrameCaptureStarted = false;
			}
			if (wholeFrameVulkan) {
				if ((Minecraft.getInstance().screen instanceof net.minecraft.client.gui.screens.TitleScreen
					|| net.minecraft.client.dev.GraphicsAuditMenuFixture.isRequestedScreen())
					&& Minecraft.getInstance().getOverlay() == null
					&& net.minecraft.client.gui.screens.TitleScreen.graphicsAuditTitleScreenSemanticsObserved()
					&& net.minecraft.client.gui.screens.TitleScreen.graphicsAuditTitleScreenFadeComplete()) {
					net.minecraft.client.gui.screens.TitleScreen.requestGraphicsAuditPresentedTitleFrameCapture();
				}
				// DH extraction can build a replacement while this frame still refers
				// to the last acknowledged column generation. Publish the replacement
				// only after presentation so one frame never mixes those generations.
				synchronized (LOCK) {
					flushPendingWorldLodAssetsLocked();
				}
			}

			METRICS.frames++;
			METRICS.submissions++;
			METRICS.batchesExecuted += requests.size();
			if (wholeFrameResult != null) {
				GraphicsFrameBenchmark.beginPhase("rust-gal.frame.post-submit-metrics");
				recordWholeFrameMetrics(wholeFrameResult);
				GraphicsFrameBenchmark.recordRustWholeFrameTimeline(
					correlationId,
					frameId,
					submissionId,
					frame.frameTargetIdentity(),
					presented.frameTargetIdentity(),
					executeStarted,
					acquireStarted,
					acquireEnded,
					submitStarted,
					submitEnded,
					presentStarted,
					presentEnded,
					wholeFrameResult.spriteCount(),
					wholeFrameResult.worldMeshInstanceCount(),
					wholeFrameResult.worldMeshDrawCount(),
					wholeFrameResult.profile().gpuFrameTotalNanos(),
					wholeFrameResult.profile().vulkanPresentMode(),
					wholeFrameResult.profile().vulkanImagesInFlight(),
						wholeFrameResult.profile().vulkanAvailableFrameSlots()
					);
					if ((Boolean.getBoolean("mattmc.dev.graphicsAuditSliceMetrics")
						|| "true".equalsIgnoreCase(System.getenv("MATTMC_TITLE_SCREEN_CAPTURE")))
						&& Minecraft.getInstance().screen instanceof TitleScreen
						&& TitleScreen.graphicsAuditTitleScreenSemanticsObserved()
						&& graphicsAuditTitlePresentationReceipts++ < 4) {
						LOGGER.info(
							"[MattMC graphics audit] rust-title-frame-presented frame={} submission={} image={}",
							presented.frameId(), submissionId, presented.frameTargetIdentity()
						);
					}
					GraphicsFrameBenchmark.endPhase("rust-gal.frame.post-submit-metrics");
			} else {
				recordGuiMetrics(guiResult);
			}
			GraphicsFrameBenchmark.beginPhase("rust-gal.frame.retire-outstanding");
			retireOutstanding(forceDeterministicCaptureRetirement());
			GraphicsFrameBenchmark.endPhase("rust-gal.frame.retire-outstanding");
			auditMessage(metricsAuditLine(requests.size(), frameId, submissionId, wholeFrameResult != null));
			METRICS.executeNanos += elapsedSince(executeStarted);
			executeCounted = true;
			if (Boolean.getBoolean("mattmc.dev.graphicsAuditSliceMetrics")) {
				TracyCompat.message("Rust VulkanicGAL GUI frame executed"
					+ " batches=" + requests.size()
					+ " spriteBatches=" + (wholeFrameResult != null ? wholeFrameResult.spriteBatchCount() : guiResult.spriteBatchCount())
					+ " frame=" + frameId
					+ " submission=" + submissionId);
			}
		} finally {
			if (!executeCounted && frameId != 0L && !frameCancelled) {
				int cancelled;
				synchronized (LOCK) {
					cancelled = SCHEDULER.cancelFrame(frameId, "frame-aborted");
				}
				METRICS.cancellations++;
				METRICS.batchesCancelled += cancelled;
				try {
					recordFixedOperation(Operation.FRAME_CANCEL, VulkanicGalBridge.Struct.FRAME_CANCEL.byteSize());
					bridge.cancelFrame(frameId, correlationId);
				} catch (RuntimeException cancelError) {
					LOGGER.error("Rust VulkanicGAL could not cancel the acquired frame after an aborted transaction", cancelError);
				}
			}
			if (renderdocFrameCaptureStarted) {
				RenderDocCaptureHook.endFrameCaptureOnce(window, "rust-vulkan-whole-frame-world#aborted");
			}
			if (!executeCounted) {
				METRICS.executeNanos += elapsedSince(executeStarted);
			}
			GraphicsFrameBenchmark.endPhase("rust-gal.gui-frame.execute");
		}
	}

	/**
	 * Rejects any copied feature-family inventory that did not enter a Rust-owned
	 * semantic stream before the whole-frame submission. The ordinary Rust
	 * material route must be just as strict as selected-source execution: a
	 * nonzero count here means visible work would otherwise disappear between
	 * Java extraction and the single Rust presenter.
	 */
	private static void assertWholeFrameFeatureCoverage(VulkanicGalBridge.WorldFeatureCoverageRecord coverage) {
		if (coverage == null) {
			throw new IllegalStateException("Rust whole-frame feature coverage record is missing");
		}
		StringBuilder unsupported = new StringBuilder();
		appendCoverage(unsupported, "model", coverage.modelSubmits());
		appendCoverage(unsupported, "model-part", coverage.modelPartSubmits());
		appendCoverage(unsupported, "block-model", coverage.blockModelSubmits());
		appendCoverage(unsupported, "ordinary-block", coverage.ordinaryBlockSubmits());
		appendCoverage(unsupported, "item", coverage.itemSubmits());
		appendCoverage(unsupported, "custom-geometry", coverage.customGeometrySubmits());
		appendCoverage(unsupported, "shadow", coverage.shadowSubmits());
		appendCoverage(unsupported, "flame", coverage.flameSubmits());
		appendCoverage(unsupported, "name-tag", coverage.nameTagSubmits());
		appendCoverage(unsupported, "text", coverage.textSubmits());
		appendCoverage(unsupported, "hitbox", coverage.hitboxSubmits());
		appendCoverage(unsupported, "leash", coverage.leashSubmits());
		// Particle groups are admitted by the Rust semantic material coverage gate.
		if (unsupported.length() != 0) {
			throw new IllegalStateException(
				"Rust whole-frame feature coverage contains unadmitted semantic families: " + unsupported
			);
		}
	}

	private static void appendCoverage(StringBuilder unsupported, String family, int count) {
		if (count <= 0) {
			return;
		}
		if (unsupported.length() != 0) {
			unsupported.append(',');
		}
		unsupported.append(family).append('=').append(count);
	}

	/** True only after a Rust whole-frame submission has consumed visible world semantics. */
	public static boolean hasObservedRenderableWholeFrameWorld() {
		return observedRenderableWholeFrameWorld;
	}

	/** Render-thread frame id of the most recent Rust GAL world draw admission. */
	public static long lastRenderableWholeFrameWorldFrame() {
		return lastRenderableWholeFrameWorldFrame;
	}

	/** Latest acquired Rust whole-frame id, on the same clock as world admission. */
	public static long lastAcquiredWholeFrameFrame() {
		return lastAcquiredWholeFrameFrame;
	}

	private static void recordWholeFrameAcquire(VulkanicGalBridge.AcquiredFrame frame, long correlationId) {
		if (frame.frameTarget() != 0L) {
			METRICS.frameTargetGenerations++;
			if (METRICS.lastFrameTargetIdentity != 0L && METRICS.lastFrameTargetIdentity != frame.frameTargetIdentity()) {
				METRICS.frameTargetIdentityChanges++;
			}
			METRICS.lastFrameTargetGeneration = frame.frameId();
			METRICS.lastFrameTargetIdentity = frame.frameTargetIdentity();
		}
		auditMessage("gal.frame.acquire backend=vulkan correlation=" + correlationId
			+ " frame=" + frame.frameId()
			+ " image=" + frame.frameTargetIdentity()
			+ " target=0x" + Long.toUnsignedString(frame.frameTarget(), 16)
			+ " extent=" + frame.width() + "x" + frame.height());
	}

	private static void auditWholeFrameTarget(VulkanicGalBridge.AcquiredFrame frame, RustGalWorldPrimitiveRenderer.PrimitiveFrame primitiveFrame) {
		if (primitiveFrame == null && RustGalGuiRenderer.isWholeFrameVulkanActive()) {
			throw new IllegalStateException(
				"Rust Vulkan whole-frame target audit requires a consumed semantic primitive frame"
			);
		}
		VulkanicGalBridge.WorldBackgroundRecord background = primitiveFrame == null
			? VulkanicGalBridge.WorldBackgroundRecord.diagnosticFallback()
			: primitiveFrame.background();
		String clearExpectation = background.enabled()
			? "clear=" + clearColorString(background.colorArgb())
				+ " expected=semantic-world-background sky_type=" + background.skyType()
				+ " color_argb=0x" + Integer.toUnsignedString(background.colorArgb(), 16)
			: "clear=0.063,0.157,0.855,1.000 expected=blue-diagnostic-shell";
		auditMessage("gal.frame.target.begin backend=vulkan frame=" + frame.frameId()
			+ " image=" + frame.frameTargetIdentity()
			+ " extent=" + frame.width() + "x" + frame.height()
			+ " " + (primitiveFrame == null
				? "material_marker_barrier_quads=0 material_marker_light_quads=0 material_marker_light_level_mask=0 material_marker_last_light_level=-1 material_marker_last_texture_id=0"
				: RustGalWorldPrimitiveRenderer.materialMarkerSummary(primitiveFrame.materialQuads(), primitiveFrame.particleQuads()))
			+ " " + clearExpectation);
		auditMessage("gal.frame.target.present-ready backend=vulkan frame=" + frame.frameId()
			+ " image=" + frame.frameTargetIdentity());
	}

	private static void writeWholeFrameAttachmentCorrelation(
		VulkanicGalBridge.AcquiredFrame acquired,
		VulkanicGalBridge.PresentedFrame presented,
		VulkanicGalBridge.WholeFrameSubmitResult result,
		RustGalWorldPrimitiveRenderer.PrimitiveFrame primitiveFrame,
		int guiAffineQuadCount,
		int guiTextAffineQuadCount,
		int guiItemAffineQuadCount
	) {
		String dir = System.getenv("MATTMC_RUST_WHOLE_FRAME_ATTACHMENT_DIR");
		if (dir == null || dir.isBlank() || result == null || primitiveFrame == null) {
			return;
		}
		Path root = Path.of(dir);
		WholeFrameAttachmentRequest request = readWholeFrameAttachmentRequest(root);
		if (request == null
			|| request.gameplayFrameId() != acquired.frameId()
			|| request.correlationId() != acquired.correlationId()
			|| !attachmentManifestMatches(root, request, result.submissionId())) {
			return;
		}
		String json = "{\n"
			+ "  \"artifact_class\":\"rust_vulkan_whole_frame_gameplay_correlation\",\n"
			+ "  \"source\":\"java-frame-coordinator-after-present\",\n"
			+ "  \"gameplay_frame_id\":" + acquired.frameId() + ",\n"
			+ "  \"correlation_id\":" + acquired.correlationId() + ",\n"
			+ "  \"deterministic_rendered_frame_index\":" + request.deterministicRenderedFrameIndex() + ",\n"
			+ "  \"gal_submission_id\":" + result.submissionId() + ",\n"
			+ "  \"vulkan_submission_timeline_value\":" + result.submissionId() + ",\n"
			+ "  \"acquired_swapchain_image\":" + acquired.frameTargetIdentity() + ",\n"
			+ "  \"presented_swapchain_image\":" + presented.frameTargetIdentity() + ",\n"
			+ "  \"present_completed_submission_id\":" + presented.completedSubmissionId() + ",\n"
			+ "  \"extent\":{\"width\":" + acquired.width() + ",\"height\":" + acquired.height() + "},\n"
			+ "  \"same_acquired_presented_image\":" + (acquired.frameTargetIdentity() == presented.frameTargetIdentity()) + ",\n"
				+ "  \"producer_workload_fingerprint\":\"" + escapeJson(primitiveFrameFingerprint(primitiveFrame)) + "\",\n"
				+ "  \"gui_sprites\":" + result.spriteCount() + ",\n"
				+ "  \"gui_affine_quads\":" + guiAffineQuadCount + ",\n"
				+ "  \"gui_text_affine_quads\":" + guiTextAffineQuadCount + ",\n"
			+ "  \"gui_item_affine_quads\":" + guiItemAffineQuadCount + ",\n"
			+ "  \"gui_mesh_items\":" + result.guiMeshItemCount() + ",\n"
			+ "  \"gui_mesh_batches\":" + result.guiMeshBatchCount() + ",\n"
			+ "  \"gui_mesh_draws\":" + result.guiMeshDrawCount() + ",\n"
			+ "  \"world_mesh_instances\":" + result.worldMeshInstanceCount() + ",\n"
			+ "  \"world_mesh_batches\":" + result.worldMeshBatchCount() + ",\n"
			+ "  \"world_mesh_draws\":" + result.worldMeshDrawCount() + ",\n"
			+ "  \"world_lod_instances\":" + primitiveFrame.lodInstances().size() + ",\n"
			+ "  \"world_lod_route_selected\":" + lodRouteSelected(primitiveFrame.lodRenderFrame()) + ",\n"
			+ "  \"world_material_quads\":" + result.worldMaterialQuadCount() + ",\n"
			+ "  \"world_crack_quads\":" + result.worldCrackQuadCount() + ",\n"
			+ "  \"world_border_quads\":" + result.worldBorderQuadCount() + ",\n"
			+ "  \"java_vulkan_frame_execution\":false,\n"
			+ "  \"rust_whole_frame_presenter\":true\n"
			+ "}\n";
		try {
			Files.createDirectories(root);
			Files.writeString(root.resolve("gameplay-correlation-frame-" + acquired.frameId() + ".json"), json, StandardCharsets.UTF_8);
			if (request.sourceSelectedCapture()
				|| net.minecraft.client.dev.DeterministicCameraCapture.normalRouteFinalOutputCaptureRequested()) {
				net.minecraft.client.dev.DeterministicCameraCapture.confirmWholeFrameSourceCapture(
					acquired.frameId(),
					acquired.correlationId(),
					request.deterministicRenderedFrameIndex(),
					result.submissionId(),
					acquired.frameTargetIdentity(),
					presented.frameTargetIdentity()
				);
			}
		} catch (IOException exception) {
			LOGGER.warn("Unable to write Rust whole-frame gameplay attachment correlation sidecar", exception);
		}
	}

	private static void writeWholeFrameAttachmentCaptureRequest(
		VulkanicGalBridge.AcquiredFrame acquired,
		long correlationId,
		long deterministicRenderedFrameIndex
	) {
		if (deterministicRenderedFrameIndex <= 0L) {
			return;
		}
		String configuredPath = System.getenv("MATTMC_RUST_WHOLE_FRAME_ATTACHMENT_REQUEST");
		if (configuredPath == null || configuredPath.isBlank()) {
			return;
		}
		Path requestPath = Path.of(configuredPath);
		Path parent = requestPath.getParent();
		boolean sourceSelectedCapture = selectedSourceExecutionRequested();
		boolean requireEntityMesh = sourceSelectedCapture
			&& net.minecraft.client.dev.DeterministicCameraCapture.requiresSourceEntityMeshCapture();
		String request = "gameplay_frame_id=" + acquired.frameId() + "\n"
			+ "correlation_id=" + correlationId + "\n"
			+ "deterministic_rendered_frame_index=" + deterministicRenderedFrameIndex + "\n"
			+ "source_selected_capture=" + (sourceSelectedCapture ? 1 : 0) + "\n"
			+ "source_selected_pending=" + (sourceSelectedCapture ? 1 : 0) + "\n"
			+ "required_entity_mesh=" + (requireEntityMesh ? 1 : 0) + "\n";
		try {
			if (parent != null) {
				Files.createDirectories(parent);
			}
			Path temporary = requestPath.resolveSibling(requestPath.getFileName() + ".tmp");
			Files.writeString(temporary, request, StandardCharsets.UTF_8);
			Files.move(temporary, requestPath, StandardCopyOption.REPLACE_EXISTING, StandardCopyOption.ATOMIC_MOVE);
			auditMessage("gal.frame.attachment-capture.request frame=" + acquired.frameId()
				+ " correlation=" + correlationId
				+ " deterministicRender=" + deterministicRenderedFrameIndex);
		} catch (IOException exception) {
			LOGGER.warn("Unable to write Rust whole-frame attachment capture request", exception);
		}
	}

	private static WholeFrameAttachmentRequest readWholeFrameAttachmentRequest(Path root) {
		String configuredPath = System.getenv("MATTMC_RUST_WHOLE_FRAME_ATTACHMENT_REQUEST");
		if (configuredPath == null || configuredPath.isBlank()) {
			return null;
		}
		try {
			long frameId = -1L;
			long correlationId = -1L;
			long deterministicRenderedFrameIndex = -1L;
			boolean sourceSelectedCapture = false;
			for (String line : Files.readAllLines(Path.of(configuredPath), StandardCharsets.UTF_8)) {
				int separator = line.indexOf('=');
				if (separator <= 0) {
					continue;
				}
				long value = Long.parseLong(line.substring(separator + 1).trim());
				switch (line.substring(0, separator).trim()) {
					case "gameplay_frame_id" -> frameId = value;
					case "correlation_id" -> correlationId = value;
					case "deterministic_rendered_frame_index" -> deterministicRenderedFrameIndex = value;
					case "source_selected_capture" -> sourceSelectedCapture = value != 0L;
					default -> { }
				}
			}
			return frameId > 0L && correlationId > 0L && deterministicRenderedFrameIndex > 0L
				? new WholeFrameAttachmentRequest(frameId, correlationId, deterministicRenderedFrameIndex, sourceSelectedCapture)
				: null;
		} catch (IOException | NumberFormatException exception) {
			return null;
		}
	}

	private static boolean attachmentManifestMatches(
		Path root,
		WholeFrameAttachmentRequest request,
		long submissionId
	) {
		Path manifest = root.resolve("gameplay-attachments-frame-" + request.gameplayFrameId() + ".json");
		try {
			String contents = Files.readString(manifest, StandardCharsets.UTF_8);
			return contents.contains("\"gameplay_frame_id\":" + request.gameplayFrameId())
				&& contents.contains("\"correlation_id\":" + request.correlationId())
				&& contents.contains("\"deterministic_rendered_frame_index\":" + request.deterministicRenderedFrameIndex())
				&& contents.contains("\"gal_submission_id\":" + submissionId);
		} catch (IOException exception) {
			return false;
		}
	}

	private record WholeFrameAttachmentRequest(
		long gameplayFrameId,
		long correlationId,
		long deterministicRenderedFrameIndex,
		boolean sourceSelectedCapture
	) {
	}

	private static long parseLongEnv(String name, long fallback) {
		String value = System.getenv(name);
		if (value == null || value.isBlank()) {
			return fallback;
		}
		try {
			return Long.parseLong(value.trim());
		} catch (NumberFormatException ignored) {
			return fallback;
		}
	}

	private static boolean selectedSourceExecutionRequested() {
		// A deterministic capture can require a native source receipt without
		// selecting the source route. The Rust frontend alone admits that route;
		// this coordinator only marks an attachment request for correlation.
		if (!System.getProperty(
			"mattmc.dev.deterministicCameraCapture.requiredRustSourceExecutionDir", ""
		).trim().isEmpty()) {
			return true;
		}
		String value = System.getenv("MATTMC_RUST_SELECTED_SOURCE_EXECUTION");
		return value != null && (value.equals("1") || value.equalsIgnoreCase("true") || value.equalsIgnoreCase("yes"));
	}

	private static int wholeFramePresentMode(Minecraft minecraft) {
		String override = System.getProperty("mattmc.dev.rustVulkan.presentMode", "").trim().toLowerCase();
		return switch (override) {
			case "immediate" -> VulkanicGalBridge.PRESENT_IMMEDIATE;
			case "mailbox" -> VulkanicGalBridge.PRESENT_MAILBOX;
			case "fifo" -> VulkanicGalBridge.PRESENT_FIFO;
			case "fifo-relaxed", "fifo_relaxed" -> VulkanicGalBridge.PRESENT_FIFO_RELAXED;
			case "auto-vsync", "auto_vsync" -> VulkanicGalBridge.PRESENT_AUTO_VSYNC;
			case "auto-no-vsync", "auto_no_vsync" -> VulkanicGalBridge.PRESENT_AUTO_NO_VSYNC;
			default -> minecraft.options.enableVsync().get()
				? VulkanicGalBridge.PRESENT_AUTO_VSYNC
				: VulkanicGalBridge.PRESENT_AUTO_NO_VSYNC;
		};
	}

	private static String primitiveFrameFingerprint(RustGalWorldPrimitiveRenderer.PrimitiveFrame frame) {
		VulkanicGalBridge.WorldFeatureCoverageRecord coverage = frame.featureCoverage();
		long particleQuads = frame.materialQuads().stream()
			.filter(quad -> quad.sourceProgram() == RustGalWorldPrimitiveRenderer.MATERIAL_SOURCE_PARTICLES)
			.count() + frame.particleQuads().size();
		return "segments=" + frame.segments().size()
			+ " crack_quads=" + frame.crackQuads().size()
			+ " border_quads=" + frame.borderQuads().size()
			+ " material_quads=" + frame.materialQuads().size()
			+ " particle_quads=" + particleQuads
			+ " particle_semantic_quads=" + frame.particleQuads().size()
			+ " mesh_instances=" + frame.meshInstances().size()
			+ " lod_instances=" + frame.lodInstances().size()
			+ " lod_route_selected=" + lodRouteSelected(frame.lodRenderFrame())
			+ " feature_models=" + coverage.modelSubmits()
			+ " feature_model_parts=" + coverage.modelPartSubmits()
			+ " feature_block_models=" + coverage.blockModelSubmits()
			+ " feature_blocks=" + coverage.ordinaryBlockSubmits()
			+ " feature_items=" + coverage.itemSubmits()
			+ " feature_custom=" + coverage.customGeometrySubmits()
			+ " feature_particles=" + coverage.particleGroupSubmits()
			+ " background_enabled=" + frame.background().enabled();
	}

	/**
	 * Records the semantic terrain set that was accepted by the Rust whole-frame
	 * submission.  The shared settled-work gate uses this identity instead of
	 * the legacy Sodium producer, which is intentionally absent on this route.
	 */
	private static void recordWholeFrameTerrainReadiness(RustGalWorldPrimitiveRenderer.PrimitiveFrame frame) {
		if (frame == null || frame.meshInstances().isEmpty()) {
			return;
		}
		List<Long> terrainKeys = new ArrayList<>();
		for (VulkanicGalBridge.WorldMeshInstanceRecord instance : frame.meshInstances()) {
			if (instance.stratum() == RustGalWorldPrimitiveRenderer.STRATUM_WORLD_TERRAIN) {
				terrainKeys.add(instance.meshKey() ^ Long.rotateLeft(instance.meshGeneration(), 17));
			}
		}
		if (terrainKeys.isEmpty()) {
			return;
		}
		terrainKeys.sort(Long::compare);
		long fingerprint = 0xcbf29ce484222325L;
		for (long key : terrainKeys) {
			fingerprint ^= key;
			fingerprint *= 0x100000001b3L;
		}
		String identity = "rust-vulkan-whole-frame:terrain:count=" + terrainKeys.size()
			+ ":fingerprint=" + Long.toUnsignedString(fingerprint);
		// Whole-frame terrain can complete before the deterministic capture hook
		// has initialized for the first playable frame.  Associate the receipt
		// with the frame that just completed so early valid submissions are not
		// silently dropped by the ordinary initialized-only recorder path.
		DeterministicCameraCapture.recordSubmittedWorkIdentityForCompletedFrame("sodium-terrain", identity);
		DeterministicCameraCapture.recordSubmittedWorkIdentityForCompletedFrame("static-terrain", identity);
	}

	private static boolean lodRouteSelected(VulkanicGalBridge.WorldLodRenderFrameRecord frame) {
		return frame.enabled()
			&& (frame.flags() & DistantHorizonsSemanticCollector.RENDER_FLAG_RUST_NON_WATER_ROUTE_SELECTED) != 0;
	}

	private static String escapeJson(String value) {
		return value
			.replace("\\", "\\\\")
			.replace("\"", "\\\"")
			.replace("\n", "\\n")
			.replace("\r", "\\r");
	}

	private static void recordGuiMetrics(VulkanicGalBridge.GuiFrameSubmitResult result) {
		METRICS.spriteBatchesExecuted += result.spriteBatchCount();
		METRICS.packedSpritesExecuted += result.spriteCount();
		METRICS.cacheHits += result.cacheHits();
		METRICS.cacheMisses += result.cacheMisses();
		METRICS.resourceCreates += result.resourceCreates();
	}

	private static void recordWholeFrameMetrics(VulkanicGalBridge.WholeFrameSubmitResult result) {
		METRICS.spriteBatchesExecuted += result.spriteBatchCount();
		METRICS.packedSpritesExecuted += result.spriteCount();
		recordWorldMetrics(result);
		recordWholeFrameProfile(result.profile());
		METRICS.worldBackgroundClearsExecuted += result.worldBackgroundClearCount();
		METRICS.worldBackgroundDiagnosticFallbacks += result.worldBackgroundDiagnosticFallbackCount();
		if (result.worldBackgroundSkyType() != 0L) {
			METRICS.lastWorldBackgroundSkyType = result.worldBackgroundSkyType();
			METRICS.lastWorldBackgroundColorArgb = result.worldBackgroundColorArgb();
		}
	}

	private static void recordWholeFrameProfile(VulkanicGalBridge.WholeFrameProfile profile) {
		recordWholeFrameProfilePhaseSamples(profile);
		METRICS.profileFfiDecodeNanos += profile.ffiDecodeNanos();
		METRICS.profileGuiFrontendNanos += profile.guiFrontendNanos();
		METRICS.profileWorldFrontendNanos += profile.worldFrontendTotalNanos();
		METRICS.profileWorldValidateFrameNanos += profile.worldValidateFrameNanos();
		METRICS.profileWorldBatchingNanos += profile.worldBatchingNanos();
		METRICS.profileWorldResourcePrepareNanos += profile.worldResourcePrepareNanos();
		METRICS.profileWorldMeshSectionExpandGroupNanos += profile.worldMeshSectionExpandGroupNanos();
		METRICS.profileShaderPlanLookupNanos += profile.shaderPlanLookupNanos();
		METRICS.profileGalCommandGenerationNanos += profile.galCommandGenerationNanos();
		METRICS.profileGalSubmitTotalNanos += profile.galSubmitTotalNanos();
		METRICS.profileGalValidateOpsNanos += profile.galValidateOpsNanos();
		METRICS.profileGalValidateHandlesNanos += profile.galValidateHandlesNanos();
		METRICS.profileGalHazardAnalysisNanos += profile.galHazardAnalysisNanos();
		METRICS.profileGalHazardReadEvents += profile.galHazardReadEvents();
		METRICS.profileGalHazardWriteEvents += profile.galHazardWriteEvents();
		METRICS.profileGalHazardCandidatesExamined += profile.galHazardCandidatesExamined();
		METRICS.profileGalHazardConflicts += profile.galHazardConflicts();
		METRICS.profileGalHazardBarriersApplied += profile.galHazardBarriersApplied();
		METRICS.profileGalHazardActiveReadEntries += profile.galHazardActiveReadEntries();
		METRICS.profileGalHazardActiveWriteEntries += profile.galHazardActiveWriteEntries();
		METRICS.profileGalCommandOpsBeforeNormalize += profile.galCommandOpsBeforeNormalize();
		METRICS.profileGalCommandOpsAfterNormalize += profile.galCommandOpsAfterNormalize();
		METRICS.profileGalRedundantPipelineBindsRemoved += profile.galRedundantPipelineBindsRemoved();
		METRICS.profileGalRedundantResourceSetBindsRemoved += profile.galRedundantResourceSetBindsRemoved();
		METRICS.profileGalRedundantVertexBufferBindsRemoved += profile.galRedundantVertexBufferBindsRemoved();
		METRICS.profileGalRedundantIndexBufferBindsRemoved += profile.galRedundantIndexBufferBindsRemoved();
		METRICS.profileBackendEncodeNanos += profile.backendEncodeNanos();
		METRICS.profileBackendSubmitNanos += profile.backendSubmitNanos();
		METRICS.profileBackendRetireNanos += profile.backendRetireNanos();
		METRICS.profileVulkanCommandBufferAllocNanos += profile.vulkanCommandBufferAllocNanos();
		METRICS.profileVulkanCommandBufferBeginNanos += profile.vulkanCommandBufferBeginNanos();
		METRICS.profileVulkanCommandRecordingNanos += profile.vulkanCommandRecordingNanos();
		METRICS.profileVulkanCommandBufferEndNanos += profile.vulkanCommandBufferEndNanos();
		METRICS.profileVulkanQueueSubmitNanos += profile.vulkanQueueSubmitNanos();
		METRICS.profileVulkanTimelinePollNanos += profile.vulkanTimelinePollNanos();
		METRICS.profileVulkanTimelineWaitNanos += profile.vulkanTimelineWaitNanos();
		METRICS.profileVulkanDeviceWaitIdleNanos += profile.vulkanDeviceWaitIdleNanos();
		METRICS.profileVulkanAcquireNanos += profile.vulkanAcquireNanos();
		METRICS.profileVulkanPresentNanos += profile.vulkanPresentNanos();
		METRICS.profileVulkanPresentWaitNanos += profile.vulkanPresentWaitNanos();
		METRICS.profileVulkanCommandBuffersAllocated += profile.vulkanCommandBuffersAllocated();
		METRICS.profileVulkanCommandBuffersFreed += profile.vulkanCommandBuffersFreed();
		METRICS.profileVulkanWaitCount += profile.vulkanWaitCount();
		METRICS.profileVulkanDeviceWaitIdleCount += profile.vulkanDeviceWaitIdleCount();
		METRICS.profileVulkanPresentMode = profile.vulkanPresentMode();
		METRICS.profileVulkanLastAcquiredImageIndex = profile.vulkanAcquiredImageIndex();
		METRICS.profileVulkanLastSwapchainGeneration = profile.vulkanSwapchainGeneration();
		METRICS.profileVulkanLastImagesInFlight = profile.vulkanImagesInFlight();
		METRICS.profileVulkanLastAvailableFrameSlots = profile.vulkanAvailableFrameSlots();
		METRICS.profileResourceCreatesDelta += profile.resourceCreatesDelta();
		METRICS.profileResourceDestroysDelta += profile.resourceDestroysDelta();
		METRICS.profileHostWriteOps += profile.hostWriteOps();
		METRICS.profileHostWriteBytes += profile.hostWriteBytes();
		METRICS.profileBarrierOps += profile.barrierOps();
		METRICS.profilePassCount += profile.passCount();
		METRICS.profileDrawOps += profile.drawOps();
		METRICS.profileDrawIndexedOps += profile.drawIndexedOps();
		METRICS.profilePipelineBinds += profile.pipelineBinds();
		METRICS.profileResourceSetBinds += profile.resourceSetBinds();
		METRICS.profileGpuTimestampUnavailableFrames += profile.gpuTimestampStatus() == 0L ? 1L : 0L;
		METRICS.profileGBufferPersistentCacheHits += profile.gBufferPersistentCacheHits();
		METRICS.profileGBufferPersistentCacheMisses += profile.gBufferPersistentCacheMisses();
		METRICS.profileGBufferFinalBindingCacheHits += profile.gBufferFinalBindingCacheHits();
		METRICS.profileGBufferFinalBindingCacheMisses += profile.gBufferFinalBindingCacheMisses();
		METRICS.profileGBufferAttachmentCreates += profile.gBufferAttachmentCreates();
		METRICS.profileGBufferPipelineCreates += profile.gBufferPipelineCreates();
		METRICS.profileGBufferShaderModuleCreates += profile.gBufferShaderModuleCreates();
		METRICS.profileGBufferDescriptorCreates += profile.gBufferDescriptorCreates();
		METRICS.profileGBufferRenderTargetCreates += profile.gBufferRenderTargetCreates();
		METRICS.profileGBufferResourcesRetired += profile.gBufferResourcesRetired();
		METRICS.profileGBufferFinalPassCreates += profile.gBufferFinalPassCreates();
	}

	private static void recordWholeFrameProfilePhaseSamples(VulkanicGalBridge.WholeFrameProfile profile) {
		GraphicsFrameBenchmark.recordPhaseSample("rust-gal.native-profile.ffi-decode", profile.ffiDecodeNanos());
		GraphicsFrameBenchmark.recordPhaseSample("rust-gal.native-profile.gui-frontend", profile.guiFrontendNanos());
		GraphicsFrameBenchmark.recordPhaseSample("rust-gal.native-profile.world-frontend", profile.worldFrontendTotalNanos());
		GraphicsFrameBenchmark.recordPhaseSample("rust-gal.native-profile.world-validate-frame", profile.worldValidateFrameNanos());
		GraphicsFrameBenchmark.recordPhaseSample("rust-gal.native-profile.world-batching", profile.worldBatchingNanos());
		GraphicsFrameBenchmark.recordPhaseSample("rust-gal.native-profile.world-resource-prepare", profile.worldResourcePrepareNanos());
		GraphicsFrameBenchmark.recordPhaseSample("rust-gal.native-profile.world-prepare-target-query", profile.worldPrepareTargetQueryNanos());
		GraphicsFrameBenchmark.recordPhaseSample("rust-gal.native-profile.world-prepare-render-resources", profile.worldPrepareRenderResourcesNanos());
		GraphicsFrameBenchmark.recordPhaseSample("rust-gal.native-profile.world-prepare-depth-attachment", profile.worldPrepareDepthAttachmentNanos());
		GraphicsFrameBenchmark.recordPhaseSample("rust-gal.native-profile.world-prepare-g-buffer-resources", profile.worldPrepareGBufferResourcesNanos());
		GraphicsFrameBenchmark.recordPhaseSample("rust-gal.native-profile.world-prepare-g-buffer-cache-check", profile.worldPrepareGBufferCacheCheckNanos());
		GraphicsFrameBenchmark.recordPhaseSample("rust-gal.native-profile.world-prepare-g-buffer-destroy", profile.worldPrepareGBufferDestroyNanos());
		GraphicsFrameBenchmark.recordPhaseSample("rust-gal.native-profile.world-prepare-g-buffer-plan", profile.worldPrepareGBufferPlanNanos());
		GraphicsFrameBenchmark.recordPhaseSample("rust-gal.native-profile.world-prepare-g-buffer-create", profile.worldPrepareGBufferCreateNanos());
		GraphicsFrameBenchmark.recordPhaseSample("rust-gal.native-profile.world-prepare-g-buffer-persistent-key", profile.worldPrepareGBufferPersistentKeyNanos());
		GraphicsFrameBenchmark.recordPhaseSample("rust-gal.native-profile.world-prepare-g-buffer-persistent-lookup", profile.worldPrepareGBufferPersistentLookupNanos());
		GraphicsFrameBenchmark.recordPhaseSample("rust-gal.native-profile.world-prepare-g-buffer-final-key", profile.worldPrepareGBufferFinalKeyNanos());
		GraphicsFrameBenchmark.recordPhaseSample("rust-gal.native-profile.world-prepare-g-buffer-final-lookup", profile.worldPrepareGBufferFinalLookupNanos());
		GraphicsFrameBenchmark.recordPhaseSample("rust-gal.native-profile.world-prepare-g-buffer-final-create", profile.worldPrepareGBufferFinalCreateNanos());
		GraphicsFrameBenchmark.recordPhaseSample("rust-gal.native-profile.world-prepare-frame-target-attachment-query", profile.worldPrepareFrameTargetAttachmentQueryNanos());
		GraphicsFrameBenchmark.recordPhaseSample("rust-gal.native-profile.world-prepare-mesh-material-asset", profile.worldPrepareMeshMaterialAssetNanos());
		GraphicsFrameBenchmark.recordPhaseSample("rust-gal.native-profile.world-prepare-metrics-accounting", profile.worldPrepareMetricsAccountingNanos());
		GraphicsFrameBenchmark.recordPhaseSample("rust-gal.native-profile.g-buffer-persistent-cache-hits", profile.gBufferPersistentCacheHits());
		GraphicsFrameBenchmark.recordPhaseSample("rust-gal.native-profile.g-buffer-persistent-cache-misses", profile.gBufferPersistentCacheMisses());
		GraphicsFrameBenchmark.recordPhaseSample("rust-gal.native-profile.g-buffer-final-binding-cache-hits", profile.gBufferFinalBindingCacheHits());
		GraphicsFrameBenchmark.recordPhaseSample("rust-gal.native-profile.g-buffer-final-binding-cache-misses", profile.gBufferFinalBindingCacheMisses());
		GraphicsFrameBenchmark.recordPhaseSample("rust-gal.native-profile.g-buffer-attachment-creates", profile.gBufferAttachmentCreates());
		GraphicsFrameBenchmark.recordPhaseSample("rust-gal.native-profile.g-buffer-pipeline-creates", profile.gBufferPipelineCreates());
		GraphicsFrameBenchmark.recordPhaseSample("rust-gal.native-profile.g-buffer-shader-module-creates", profile.gBufferShaderModuleCreates());
		GraphicsFrameBenchmark.recordPhaseSample("rust-gal.native-profile.g-buffer-descriptor-creates", profile.gBufferDescriptorCreates());
		GraphicsFrameBenchmark.recordPhaseSample("rust-gal.native-profile.g-buffer-render-target-creates", profile.gBufferRenderTargetCreates());
		GraphicsFrameBenchmark.recordPhaseSample("rust-gal.native-profile.g-buffer-resources-retired", profile.gBufferResourcesRetired());
		GraphicsFrameBenchmark.recordPhaseSample("rust-gal.native-profile.g-buffer-final-pass-creates", profile.gBufferFinalPassCreates());
		GraphicsFrameBenchmark.recordPhaseSample("rust-gal.native-profile.world-prepare-frame-pass", profile.worldPrepareFramePassNanos());
		GraphicsFrameBenchmark.recordPhaseSample("rust-gal.native-profile.world-mesh-expand-group", profile.worldMeshSectionExpandGroupNanos());
		GraphicsFrameBenchmark.recordPhaseSample("rust-gal.native-profile.world-prepare-mesh-cache-scan", profile.worldPrepareMeshCacheScanNanos());
		GraphicsFrameBenchmark.recordPhaseSample("rust-gal.native-profile.world-prepare-material-resource", profile.worldPrepareMaterialResourceNanos());
		GraphicsFrameBenchmark.recordPhaseSample("rust-gal.native-profile.world-prepare-mesh-stream-capacity", profile.worldPrepareMeshStreamCapacityNanos());
		GraphicsFrameBenchmark.recordPhaseSample("rust-gal.native-profile.world-prepare-mesh-stream-lookup", profile.worldPrepareMeshStreamLookupNanos());
		GraphicsFrameBenchmark.recordPhaseSample("rust-gal.native-profile.world-prepare-mesh-stream-grow", profile.worldPrepareMeshStreamGrowNanos());
		GraphicsFrameBenchmark.recordPhaseSample("rust-gal.native-profile.world-prepare-mesh-resource", profile.worldPrepareMeshResourceNanos());
		GraphicsFrameBenchmark.recordPhaseSample("rust-gal.native-profile.world-prepare-material-slot-check", profile.worldPrepareMaterialSlotCheckNanos());
		GraphicsFrameBenchmark.recordPhaseSample("rust-gal.native-profile.world-prepare-mesh-slot-check", profile.worldPrepareMeshSlotCheckNanos());
		GraphicsFrameBenchmark.recordPhaseSample("rust-gal.native-profile.world-prepare-mesh-batch-count", profile.worldPrepareMeshBatchCount());
		GraphicsFrameBenchmark.recordPhaseSample("rust-gal.native-profile.world-prepare-mesh-stream-required-bytes", profile.worldPrepareMeshStreamRequiredBytes());
		GraphicsFrameBenchmark.recordPhaseSample("rust-gal.native-profile.world-prepare-mesh-stream-capacity-bytes", profile.worldPrepareMeshStreamCapacityBytes());
		GraphicsFrameBenchmark.recordPhaseSample("rust-gal.native-profile.world-prepare-mesh-stream-grows", profile.worldPrepareMeshStreamGrows());
		GraphicsFrameBenchmark.recordPhaseSample("rust-gal.native-profile.world-mesh-stream-payload-pack", profile.worldMeshStreamPayloadPackNanos());
		GraphicsFrameBenchmark.recordPhaseSample("rust-gal.native-profile.world-mesh-draw-record", profile.worldMeshDrawRecordNanos());
		GraphicsFrameBenchmark.recordPhaseSample("rust-gal.native-profile.world-mesh-stream-payload-bytes", profile.worldMeshStreamPayloadBytes());
		GraphicsFrameBenchmark.recordPhaseSample("rust-gal.native-profile.world-mesh-dynamic-offset-count", profile.worldMeshDynamicOffsetCount());
		GraphicsFrameBenchmark.recordPhaseSample("rust-gal.native-profile.shader-plan-lookup", profile.shaderPlanLookupNanos());
		GraphicsFrameBenchmark.recordPhaseSample("rust-gal.native-profile.gal-command-generation", profile.galCommandGenerationNanos());
		GraphicsFrameBenchmark.recordPhaseSample("rust-gal.native-profile.gal-submit-total", profile.galSubmitTotalNanos());
		GraphicsFrameBenchmark.recordPhaseSample("rust-gal.native-profile.gal-validate-ops", profile.galValidateOpsNanos());
		GraphicsFrameBenchmark.recordPhaseSample("rust-gal.native-profile.gal-validate-handles", profile.galValidateHandlesNanos());
		GraphicsFrameBenchmark.recordPhaseSample("rust-gal.native-profile.gal-hazard-analysis", profile.galHazardAnalysisNanos());
		GraphicsFrameBenchmark.recordPhaseSample("rust-gal.native-profile.gal-hazard-read-events", profile.galHazardReadEvents());
		GraphicsFrameBenchmark.recordPhaseSample("rust-gal.native-profile.gal-hazard-write-events", profile.galHazardWriteEvents());
		GraphicsFrameBenchmark.recordPhaseSample("rust-gal.native-profile.gal-hazard-candidates-examined", profile.galHazardCandidatesExamined());
		GraphicsFrameBenchmark.recordPhaseSample("rust-gal.native-profile.gal-hazard-conflicts", profile.galHazardConflicts());
		GraphicsFrameBenchmark.recordPhaseSample("rust-gal.native-profile.gal-hazard-barriers-applied", profile.galHazardBarriersApplied());
		GraphicsFrameBenchmark.recordPhaseSample("rust-gal.native-profile.gal-hazard-active-read-entries", profile.galHazardActiveReadEntries());
		GraphicsFrameBenchmark.recordPhaseSample("rust-gal.native-profile.gal-hazard-active-write-entries", profile.galHazardActiveWriteEntries());
		GraphicsFrameBenchmark.recordPhaseSample("rust-gal.native-profile.gal-command-ops-before-normalize", profile.galCommandOpsBeforeNormalize());
		GraphicsFrameBenchmark.recordPhaseSample("rust-gal.native-profile.gal-command-ops-after-normalize", profile.galCommandOpsAfterNormalize());
		GraphicsFrameBenchmark.recordPhaseSample("rust-gal.native-profile.gal-redundant-pipeline-binds-removed", profile.galRedundantPipelineBindsRemoved());
		GraphicsFrameBenchmark.recordPhaseSample("rust-gal.native-profile.gal-redundant-resource-set-binds-removed", profile.galRedundantResourceSetBindsRemoved());
		GraphicsFrameBenchmark.recordPhaseSample("rust-gal.native-profile.gal-redundant-vertex-buffer-binds-removed", profile.galRedundantVertexBufferBindsRemoved());
		GraphicsFrameBenchmark.recordPhaseSample("rust-gal.native-profile.gal-redundant-index-buffer-binds-removed", profile.galRedundantIndexBufferBindsRemoved());
		GraphicsFrameBenchmark.recordPhaseSample("rust-gal.native-profile.backend-encode", profile.backendEncodeNanos());
		GraphicsFrameBenchmark.recordPhaseSample("rust-gal.native-profile.backend-submit", profile.backendSubmitNanos());
		GraphicsFrameBenchmark.recordPhaseSample("rust-gal.native-profile.backend-retire", profile.backendRetireNanos());
		GraphicsFrameBenchmark.recordPhaseSample("rust-gal.native-profile.vulkan-command-buffer-alloc", profile.vulkanCommandBufferAllocNanos());
		GraphicsFrameBenchmark.recordPhaseSample("rust-gal.native-profile.vulkan-command-buffer-begin", profile.vulkanCommandBufferBeginNanos());
		GraphicsFrameBenchmark.recordPhaseSample("rust-gal.native-profile.vulkan-command-recording", profile.vulkanCommandRecordingNanos());
		GraphicsFrameBenchmark.recordPhaseSample("rust-gal.native-profile.vulkan-command-buffer-end", profile.vulkanCommandBufferEndNanos());
		GraphicsFrameBenchmark.recordPhaseSample("rust-gal.native-profile.vulkan-queue-submit", profile.vulkanQueueSubmitNanos());
		GraphicsFrameBenchmark.recordPhaseSample("rust-gal.native-profile.vulkan-timeline-poll", profile.vulkanTimelinePollNanos());
		GraphicsFrameBenchmark.recordPhaseSample("rust-gal.native-profile.vulkan-timeline-wait", profile.vulkanTimelineWaitNanos());
		GraphicsFrameBenchmark.recordPhaseSample("rust-gal.native-profile.vulkan-device-wait-idle", profile.vulkanDeviceWaitIdleNanos());
		GraphicsFrameBenchmark.recordPhaseSample("rust-gal.native-profile.vulkan-acquire", profile.vulkanAcquireNanos());
		GraphicsFrameBenchmark.recordPhaseSample("rust-gal.native-profile.vulkan-present", profile.vulkanPresentNanos());
		GraphicsFrameBenchmark.recordPhaseSample("rust-gal.native-profile.vulkan-present-wait", profile.vulkanPresentWaitNanos());
		GraphicsFrameBenchmark.recordPhaseSample("rust-gal.native-profile.vulkan-present-mode", profile.vulkanPresentMode());
		GraphicsFrameBenchmark.recordPhaseSample("rust-gal.native-profile.vulkan-requested-present-mode", profile.vulkanRequestedPresentMode());
		GraphicsFrameBenchmark.recordPhaseSample("rust-gal.native-profile.vulkan-supported-present-modes", profile.vulkanSupportedPresentModes());
		GraphicsFrameBenchmark.recordPhaseSample("rust-gal.native-profile.vulkan-present-mode-fallback-reason", profile.vulkanPresentModeFallbackReason());
		GraphicsFrameBenchmark.recordPhaseSample("rust-gal.native-profile.vulkan-acquired-image-index", profile.vulkanAcquiredImageIndex());
		GraphicsFrameBenchmark.recordPhaseSample("rust-gal.native-profile.vulkan-swapchain-generation", profile.vulkanSwapchainGeneration());
		GraphicsFrameBenchmark.recordPhaseSample("rust-gal.native-profile.vulkan-swapchain-image-count", profile.vulkanSwapchainImageCount());
		GraphicsFrameBenchmark.recordPhaseSample("rust-gal.native-profile.vulkan-surface-min-image-count", profile.vulkanSurfaceMinImageCount());
		GraphicsFrameBenchmark.recordPhaseSample("rust-gal.native-profile.vulkan-surface-max-image-count", profile.vulkanSurfaceMaxImageCount());
		GraphicsFrameBenchmark.recordPhaseSample("rust-gal.native-profile.vulkan-configured-frames-in-flight", profile.vulkanConfiguredFramesInFlight());
		GraphicsFrameBenchmark.recordPhaseSample("rust-gal.native-profile.vulkan-images-in-flight", profile.vulkanImagesInFlight());
		GraphicsFrameBenchmark.recordPhaseSample("rust-gal.native-profile.vulkan-available-frame-slots", profile.vulkanAvailableFrameSlots());
		GraphicsFrameBenchmark.recordPhaseSample("rust-gal.native-profile.gpu-shadow-depth", profile.gpuShadowDepthNanos());
		GraphicsFrameBenchmark.recordPhaseSample("rust-gal.native-profile.gpu-terrain-opaque", profile.gpuTerrainOpaqueNanos());
		GraphicsFrameBenchmark.recordPhaseSample("rust-gal.native-profile.gpu-terrain-cutout", profile.gpuTerrainCutoutNanos());
		GraphicsFrameBenchmark.recordPhaseSample("rust-gal.native-profile.gpu-deferred-lighting", profile.gpuDeferredLightingNanos());
		GraphicsFrameBenchmark.recordPhaseSample("rust-gal.native-profile.gpu-composite-0", profile.gpuComposite0Nanos());
		GraphicsFrameBenchmark.recordPhaseSample("rust-gal.native-profile.gpu-composite-1", profile.gpuComposite1Nanos());
		GraphicsFrameBenchmark.recordPhaseSample("rust-gal.native-profile.gpu-final-output", profile.gpuFinalOutputNanos());
		GraphicsFrameBenchmark.recordPhaseSample("rust-gal.native-profile.gpu-frame-total", profile.gpuFrameTotalNanos());
	}

	private static void recordWorldMetrics(VulkanicGalBridge.WholeFrameSubmitResult result) {
		METRICS.worldPrimitiveBatchesExecuted += result.worldBatchCount();
		METRICS.worldLineSegmentsExecuted += result.worldSegmentCount();
		METRICS.worldLineVerticesExecuted += result.worldVertexCount();
		METRICS.worldPrimitiveDrawsExecuted += result.worldDrawCount();
		METRICS.worldCrackQuadsExecuted += result.worldCrackQuadCount();
		METRICS.worldCrackBatchesExecuted += result.worldCrackBatchCount();
		METRICS.worldCrackDrawsExecuted += result.worldCrackDrawCount();
		METRICS.worldBorderQuadsExecuted += result.worldBorderQuadCount();
		METRICS.worldBorderBatchesExecuted += result.worldBorderBatchCount();
		METRICS.worldBorderDrawsExecuted += result.worldBorderDrawCount();
		METRICS.worldMaterialQuadsExecuted += result.worldMaterialQuadCount();
		METRICS.worldMaterialBatchesExecuted += result.worldMaterialBatchCount();
		METRICS.worldMaterialDrawsExecuted += result.worldMaterialDrawCount();
		METRICS.worldMeshInstancesExecuted += result.worldMeshInstanceCount();
		METRICS.worldMeshBatchesExecuted += result.worldMeshBatchCount();
		METRICS.worldMeshDrawsExecuted += result.worldMeshDrawCount();
		METRICS.worldDepthAttachmentCreates += result.depthAttachmentCreates();
		METRICS.worldDepthAttachmentReuses += result.depthAttachmentReuses();
		METRICS.worldDepthAttachmentRetires += result.depthAttachmentRetires();
		METRICS.worldOutlineCacheHits += result.outlineCacheHits();
		METRICS.worldOutlineCacheMisses += result.outlineCacheMisses();
		METRICS.worldCrackCacheHits += result.crackCacheHits();
		METRICS.worldCrackCacheMisses += result.crackCacheMisses();
		METRICS.worldBorderCacheHits += result.borderCacheHits();
		METRICS.worldBorderCacheMisses += result.borderCacheMisses();
		METRICS.worldMaterialCacheHits += result.materialCacheHits();
		METRICS.worldMaterialCacheMisses += result.materialCacheMisses();
		METRICS.worldMeshCacheHits += result.meshCacheHits();
		METRICS.worldMeshCacheMisses += result.meshCacheMisses();
		METRICS.cacheHits += result.cacheHits();
		METRICS.cacheMisses += result.cacheMisses();
		METRICS.resourceCreates += result.resourceCreates();
	}

	private static void retireOutstanding(boolean force) {
		if (bridge == null || lastSubmitted == 0L || lastSubmitted <= lastRetiredSubmission) {
			return;
		}
		long retireThrough = lastSubmitted;
		if (!force) {
			// Poll the Rust timeline before retiring.  The old non-forced path was
			// intentionally a no-op, which left every submitted Vulkan command
			// buffer in flight during a normal client session (and made RSS grow
			// until the capture guard killed the process).  Completion polling is
			// non-blocking; only work already completed by the GPU is retired here.
			long completionStarted = System.nanoTime();
			VulkanicGalBridge.Completion completion = bridge.completion(lastSubmitted);
			METRICS.completionQueryNanos += elapsedSince(completionStarted);
			recordFixedOperation(Operation.COMPLETION_QUERY, VulkanicGalBridge.Struct.COMPLETION_QUERY.byteSize());
			METRICS.completionPolls++;
			retireThrough = completion.completedSubmissionId();
			if (!completion.complete() && retireThrough <= lastRetiredSubmission) {
				return;
			}
		}
		if (retireThrough <= lastRetiredSubmission) {
			return;
		}
		long started = System.nanoTime();
		recordStatus(Operation.RETIRE, bridge.retire(retireThrough));
		METRICS.retireNanos += elapsedSince(started);
		lastRetiredSubmission = retireThrough;
	}

	/**
	 * Heavy deterministic fixtures must not accumulate native Vulkan ownership
	 * while the driver reports no completed timeline value.  Retiring through the
	 * submitted token uses the backend's existing explicit wait path and is
	 * enabled only for capture diagnostics or the terrain-particle fixture.
	 */
	private static boolean forceDeterministicCaptureRetirement() {
		return net.minecraft.client.dev.DeterministicCameraCapture.isEnabledForDiagnostics()
			|| !System.getProperty("mattmc.dev.rustGalWorldMaterial.terrainParticleScenario", "").isBlank();
	}

	private static void ensureRenderThreadAndContext(Minecraft minecraft) {
		Thread current = Thread.currentThread();
		if (renderThread == null) {
			renderThread = current;
		} else if (renderThread != current) {
			throw new IllegalStateException("Rust VulkanicGAL deferred frame queue used from the wrong render thread");
		}
		Window window = minecraft.getWindow();
		if (!VulkanicGalBridge.isBorrowedOpenGlContextCurrent(window)) {
			throw new IllegalStateException("Rust VulkanicGAL deferred OpenGL execution requires Minecraft's current GL context");
		}
		if (bridge != null && bridgeMode != BridgeMode.BORROWED_OPENGL) {
			throw new IllegalStateException("Rust VulkanicGAL borrowed OpenGL execution cannot reuse a "
				+ bridgeMode + " bridge");
		}
		if (bridge == null) {
			bridge = VulkanicGalBridge.createBorrowedOpenGl(window);
			bridgeMode = BridgeMode.BORROWED_OPENGL;
			recordFixedOperation(Operation.CONTEXT_CREATE, VulkanicGalBridge.Struct.BORROWED_OPENGL_CONTEXT_CREATE.byteSize());
			recordFixedOperation(Operation.CAPABILITY_QUERY, VulkanicGalBridge.Struct.CAPABILITY_QUERY.byteSize());
			flushPendingGuiAssetsLocked();
			flushPendingWorldAssetsLocked();
			configuredWidth = 0;
			configuredHeight = 0;
		}
	}

	private static void ensureRenderThreadAndWindowedVulkanContext(Minecraft minecraft) {
		Thread current = Thread.currentThread();
		if (renderThread == null) {
			renderThread = current;
		} else if (renderThread != current) {
			throw new IllegalStateException("Rust VulkanicGAL whole-frame queue used from the wrong render thread");
		}
		Window window = minecraft.getWindow();
		if (bridge != null && bridgeMode != BridgeMode.WINDOWED_VULKAN) {
			throw new IllegalStateException("Rust Vulkan whole-frame execution cannot reuse a "
				+ bridgeMode + " bridge; create a Rust-owned windowed Vulkan context instead");
		}
		if (bridge == null) {
			bridge = VulkanicGalBridge.createWindowedVulkan(
				window,
				Math.max(1, window.getWidth()),
				Math.max(1, window.getHeight()),
				wholeFramePresentMode(minecraft)
			);
			bridgeMode = BridgeMode.WINDOWED_VULKAN;
			// From this point the Rust bridge owns the only active presenter.
			// Do not let normal Java backend lookups fall back to the temporary
			// OpenGL bootstrap backend while the frame route is live.
			RustGalVulkanWholeFrameMode.activateRustPresentation();
			recordFixedOperation(Operation.CONTEXT_CREATE, VulkanicGalBridge.Struct.WINDOWED_VULKAN_CONTEXT_CREATE.byteSize());
			recordFixedOperation(Operation.CAPABILITY_QUERY, VulkanicGalBridge.Struct.CAPABILITY_QUERY.byteSize());
			flushPendingGuiAssetsLocked();
			flushPendingWorldAssetsLocked();
			configuredWidth = 0;
			configuredHeight = 0;
		}
	}

	private static void ensureConfigured(Window window) {
		int width = Math.max(1, window.getWidth());
		int height = Math.max(1, window.getHeight());
		if (configuredWidth == width && configuredHeight == height) {
			return;
		}
		if (configuredWidth == 0 || configuredHeight == 0) {
			String label = bridgeMode == BridgeMode.WINDOWED_VULKAN
				? "minecraft.rust-vulkan.swapchain"
				: "minecraft.borrowed.opengl.default";
			int presentMode = bridgeMode == BridgeMode.WINDOWED_VULKAN
				? wholeFramePresentMode(Minecraft.getInstance())
				: VulkanicGalBridge.PRESENT_FIFO;
			recordStatus(Operation.FRAME_CONFIGURE, bridge.configureFrame(
				label,
				width,
				height,
				VulkanicGalBridge.FORMAT_RGBA8,
				presentMode
			));
		} else {
			recordFixedOperation(Operation.FRAME_RESIZE, VulkanicGalBridge.Struct.FRAME_RESIZE.byteSize());
			bridge.resizeFrame(nextCorrelationId++, width, height);
		}
		configuredWidth = width;
		configuredHeight = height;
	}

	private static void flushPendingGuiAssetsLocked() {
		if (bridge != null && uploadedAssetGeneration < assetGeneration && attemptedAssetGeneration < assetGeneration) {
			attemptedAssetGeneration = assetGeneration;
			try {
				recordStatus(Operation.GUI_ASSET_UPDATE, bridge.updateGuiAssets(assetGeneration, pendingAssets));
				lastAssetPayloadCount = pendingAssets.size();
				lastAssetPayloadBytes = pendingAssets.stream().mapToLong(asset -> asset.pngBytes().length).sum();
				uploadedAssetGeneration = assetGeneration;
				auditMessage(
					"Rust VulkanicGAL GUI asset update accepted"
						+ " generation=" + assetGeneration
						+ " payloads=" + lastAssetPayloadCount
						+ " payload_bytes=" + lastAssetPayloadBytes
						+ " uploaded_generation=" + uploadedAssetGeneration
				);
			} catch (RuntimeException error) {
				assetUpdateFailures++;
				// The bridge did not admit this generation. Keep it retryable so a
				// transient native/resource failure cannot strand the last valid atlas.
				attemptedAssetGeneration = uploadedAssetGeneration;
				LOGGER.error(
					"Rust VulkanicGAL GUI asset update failed for generation {}; preserving last valid atlas",
					assetGeneration,
					error
				);
				auditMessage(
					"Rust VulkanicGAL GUI asset update failed"
						+ " generation=" + assetGeneration
						+ " uploaded_generation=" + uploadedAssetGeneration
						+ " failures=" + assetUpdateFailures
						+ " preserve_last_valid=true"
				);
			}
		}
		if (bridge == null || uploadedRawImageGeneration >= rawImageGeneration || attemptedRawImageGeneration >= rawImageGeneration) {
			return;
		}
		attemptedRawImageGeneration = rawImageGeneration;
		try {
			List<VulkanicGalBridge.GuiRawImageAssetRecord> assets = List.copyOf(pendingRawImages.values());
			recordStatus(Operation.GUI_ASSET_UPDATE, bridge.updateGuiRawImages(rawImageGeneration, assets));
			uploadedRawImageGeneration = rawImageGeneration;
			auditMessage("Rust VulkanicGAL GUI raw image update accepted generation=" + rawImageGeneration
				+ " payloads=" + assets.size());
		} catch (RuntimeException error) {
			assetUpdateFailures++;
			// Preserve the last valid image set, but leave this generation retryable
			// on the next frame after a transient native/resource failure.
			attemptedRawImageGeneration = uploadedRawImageGeneration;
			LOGGER.error("Rust VulkanicGAL GUI raw image update failed for generation {}; preserving last valid images", rawImageGeneration, error);
			auditMessage("Rust VulkanicGAL GUI raw image update failed generation=" + rawImageGeneration
				+ " uploaded_generation=" + uploadedRawImageGeneration + " preserve_last_valid=true");
		}
	}

	/** Resource traffic must progress even when no frame is drawn or presented. */
	public static void pumpAtlasAnimationResources() {
		if (!RustGalVulkanWholeFrameMode.enabledForBackend(VulkanicAPI.isVulkanBackendSelected())) return;
		synchronized (LOCK) {
			// The existing coordinator owns the sole native context. Never create
			// a second presenter or consult a Java graphics context for this pump.
			if (bridge == null) return;
			if (bridgeMode != BridgeMode.WINDOWED_VULKAN || renderThread != Thread.currentThread()) {
				throw new IllegalStateException("Atlas event pump requires the owning Rust Vulkan render thread");
			}
			// Startup/resource reload may tick before the atlas upload publishes its
			// immutable incarnation. There is no animation resource to pump yet.
			// Do not interpret the uninitialized atlas's zero extent as oversized,
			// or publish an old cached payload while the new incarnation is absent.
			var atlas = Minecraft.getInstance().getAtlasManager().getAtlasOrThrow(
				net.minecraft.data.AtlasIds.BLOCKS);
			if (atlas.semanticAnimationResource() != null) {
				net.vulkanic.world.RustGalTerrainRenderer.ensureTerrainAtlasAssetForWorldMesh();
			}
			RustGalWorldPrimitiveRenderer.ensureParticleAtlasAnimationAsset();
			RustGalWorldPrimitiveRenderer.ensureShieldAtlasAnimationAsset();
			var status = RustGalWorldPrimitiveRenderer.flushPendingWorldMeshAssets(bridge);
			if (status != null) recordStatus(Operation.WORLD_MESH_ASSET_UPDATE, status);
			RustGalWorldPrimitiveRenderer.flushPendingAtlasAnimationTicks(bridge);
		}
	}

	private static void flushPendingWorldAssetsLocked() {
		flushPendingWorldAssetsLocked(true);
	}

	/**
	 * Freezing a combined frame can register an immutable mesh texture or asset
	 * while decoding the semantic work. Publish those resources before native
	 * submission, but do not mutate the already-frozen instance list: newly
	 * created geometry is admitted on the following frame. This keeps a reused
	 * terrain atlas generation available to already-admitted terrain work and
	 * avoids an unknown-texture failure during the next whole-frame submit.
	 */
	private static void flushPendingWorldAssetsAfterFrameConsumeLocked() {
		flushPendingWorldAssetsLocked(true);
	}

	private static void flushPendingWorldAssetsLocked(boolean includeWorldMeshAssets) {
		flushPendingShaderPackSourcesLocked();
		VulkanicGalBridge.Status status = RustGalWorldPrimitiveRenderer.flushPendingWorldBorderAssets(bridge);
		if (status != null) {
			recordStatus(Operation.WORLD_BORDER_ASSET_UPDATE, status);
		}
		status = RustGalWorldPrimitiveRenderer.flushPendingWorldCrackAssets(bridge);
		if (status != null) {
			recordStatus(Operation.WORLD_CRACK_ASSET_UPDATE, status);
		}
		status = RustGalWorldPrimitiveRenderer.flushPendingWorldMaterialAssets(bridge);
		if (status != null) {
			recordStatus(Operation.WORLD_MATERIAL_ASSET_UPDATE, status);
		}
		status = RustGalWorldPrimitiveRenderer.flushPendingWorldTextImages(bridge);
		if (status != null) {
			recordStatus(Operation.WORLD_TEXT_ASSET_UPDATE, status);
		}
		if (includeWorldMeshAssets) {
			status = RustGalWorldPrimitiveRenderer.flushPendingWorldMeshAssets(bridge);
			if (status != null) {
				recordStatus(Operation.WORLD_MESH_ASSET_UPDATE, status);
			}
			RustGalWorldPrimitiveRenderer.flushPendingAtlasAnimationTicks(bridge);
		}
	}

	private static void flushPendingWorldLodAssetsLocked() {
		if (!RustGalGuiRenderer.isWholeFrameVulkanEnabled()) {
			return;
		}
		VulkanicGalBridge.Status status = DistantHorizonsSemanticCollector.flushPendingAssets(bridge);
		if (status != null) {
			recordStatus(Operation.WORLD_LOD_ASSET_UPDATE, status);
		}
	}

	/**
	 * Publishes pending DH semantic assets before the next render-list preflight.
	 * This is deliberately an asset-only Vulkan operation: it neither consumes
	 * visible segments nor issues a draw or presentation. Without this phase, a
	 * first DH frame can reject itself because route admission runs before the
	 * normal whole-frame GUI batch has an opportunity to acknowledge its assets.
	 */
	public static void flushPendingWorldLodAssetsForSemanticPreflight() {
		synchronized (LOCK) {
			flushPendingWorldLodAssetsLocked();
		}
	}

	/**
	 * Resource reload can run before Iris has built its configured pipeline,
	 * leaving the pending semantic source generation as the deliberately empty
	 * disabled snapshot. Once Iris reports an active pack, collect it once and
	 * publish the immutable source/asset generation through the existing FFI
	 * path. This does not observe a GL object or select a renderer route.
	 */
	private static void refreshConfiguredShaderPackSourcesLocked() {
		var activePack = RustShaderPackSourceCollector.activeConfiguredPackName();
		if (activePack.isEmpty() || activePack.get().equals(pendingShaderPackSourceName)) {
			return;
		}
		long sourceGeneration = nextShaderPackSourceGeneration++;
		try {
			stageShaderPackSourcesLocked(
				RustShaderPackSourceCollector.collectConfiguredPack(sourceGeneration),
				activePack.get()
			);
			auditMessage("Rust VulkanicGAL shader-pack source became active after reload"
				+ " generation=" + sourceGeneration
				+ " pack=" + activePack.get());
		} catch (IOException | RuntimeException error) {
			// Iris can briefly report the configured name while its resolved pack
			// is still changing. Leave the prior valid generation active and retry
			// on a later frame without treating this as renderer execution.
			LOGGER.warn("Rust VulkanicGAL shader-pack source is not ready for semantic collection yet", error);
		}
	}

	/**
	 * Refreshes filesystem-backed shader-pack semantics before camera extraction
	 * for a Rust-owned frame. Keeping this ahead of matrix construction makes
	 * the first frame after pack activation use the same source-ready state as
	 * the Rust shader executor, without consulting Iris renderer state.
	 */
	public static void refreshConfiguredShaderPackSourcesForSemanticFrame() {
		synchronized (LOCK) {
			refreshConfiguredShaderPackSourcesLocked();
		}
	}

	private static void stageShaderPackSourcesLocked(RustShaderPackSourceCollector.SourceGeneration source) {
		stageShaderPackSourcesLocked(source, source.packName());
	}

	private static void stageShaderPackSourcesLocked(
		RustShaderPackSourceCollector.SourceGeneration source,
		String selectionKey
	) {
		pendingShaderPackSources = source;
		pendingShaderPackSourceName = selectionKey;
		attemptedShaderPackSourceGeneration = Math.min(
			attemptedShaderPackSourceGeneration,
			uploadedShaderPackSourceGeneration
		);
		attemptedShaderPackAssetGeneration = Math.min(
			attemptedShaderPackAssetGeneration,
			uploadedShaderPackAssetGeneration
		);
	}

	private static void flushPendingShaderPackSourcesLocked() {
		if (!RustGalGuiRenderer.isWholeFrameVulkanActive()
			|| bridge == null
			|| pendingShaderPackSources == null) {
			return;
		}
		long generation = pendingShaderPackSources.generation();
		if (uploadedShaderPackSourceGeneration < generation
			&& attemptedShaderPackSourceGeneration < generation) {
			attemptedShaderPackSourceGeneration = generation;
			try {
				recordStatus(
					Operation.SHADER_PACK_SOURCE_UPDATE,
					bridge.updateShaderPackSources(
						generation,
						pendingShaderPackSources.packName(),
						pendingShaderPackSources.files()
					)
				);
				uploadedShaderPackSourceGeneration = generation;
				auditMessage("Rust VulkanicGAL shader-pack source update accepted generation="
					+ uploadedShaderPackSourceGeneration
					+ " pack=" + pendingShaderPackSources.packName()
					+ " files=" + pendingShaderPackSources.files().size()
					+ " source_bytes=" + pendingShaderPackSources.totalBytes()
					+ " source_execution_requested=" + selectedSourceExecutionRequested()
					+ " source_execution_selected=deferred-until-frame-admission");
			} catch (RuntimeException error) {
				shaderPackSourceUpdateFailures++;
				// This generation was not accepted.  Keep it retryable on the
				// next frame; otherwise the attempted-generation guard would
				// permanently strand the source while the prior valid generation
				// remains active.
				attemptedShaderPackSourceGeneration = uploadedShaderPackSourceGeneration;
				LOGGER.error("Rust VulkanicGAL shader-pack source update failed for generation {}; preserving the last valid source", generation, error);
				auditMessage("Rust VulkanicGAL shader-pack source update failed generation="
					+ generation
					+ " failures=" + shaderPackSourceUpdateFailures
					+ " preserve_last_valid=true");
			}
		}
		if (uploadedShaderPackSourceGeneration < generation
			|| uploadedShaderPackAssetGeneration >= generation
			|| attemptedShaderPackAssetGeneration >= generation) {
			return;
		}
		attemptedShaderPackAssetGeneration = generation;
		try {
			recordStatus(
				Operation.SHADER_PACK_ASSET_UPDATE,
				bridge.updateShaderPackAssets(
					generation,
					pendingShaderPackSources.packName(),
					pendingShaderPackSources.assets()
				)
			);
			uploadedShaderPackAssetGeneration = generation;
			auditMessage("Rust VulkanicGAL shader-pack asset update accepted generation="
				+ uploadedShaderPackAssetGeneration
				+ " pack=" + pendingShaderPackSources.packName()
				+ " files=" + pendingShaderPackSources.assets().size()
				+ " asset_bytes=" + pendingShaderPackSources.assetTotalBytes()
				+ " source_execution_requested=" + selectedSourceExecutionRequested()
				+ " source_execution_selected=deferred-until-frame-admission");
		} catch (RuntimeException error) {
			shaderPackAssetUpdateFailures++;
			// Asset admission is generation-coherent with source admission.  A
			// failed copy must remain retryable; otherwise the source generation
			// can never acquire its required asset snapshot and can never arm.
			attemptedShaderPackAssetGeneration = uploadedShaderPackAssetGeneration;
			LOGGER.error("Rust VulkanicGAL shader-pack asset update failed for generation {}; source remains unadmitted", generation, error);
			auditMessage("Rust VulkanicGAL shader-pack asset update failed generation="
				+ generation
				+ " failures=" + shaderPackAssetUpdateFailures
				+ " source_execution_requested=" + selectedSourceExecutionRequested()
				+ " source_execution_selected=deferred-until-frame-admission");
		}
	}

	private static void recordStatus(Operation operation, VulkanicGalBridge.Status status) {
		long ffiCalls = status.ffiCalls();
		long ffiBytes = status.ffiInputBytes();
		recordBackendMetrics(status.backendMetrics());
		long deltaCalls = 0L;
		long deltaBytes = 0L;
		if (ffiCalls >= METRICS.lastContextFfiCalls) {
			deltaCalls = ffiCalls - METRICS.lastContextFfiCalls;
			METRICS.ffiCalls += deltaCalls;
		}
		if (ffiBytes >= METRICS.lastContextFfiBytes) {
			deltaBytes = ffiBytes - METRICS.lastContextFfiBytes;
			METRICS.ffiBytes += deltaBytes;
		}
		addOperation(operation, deltaCalls, deltaBytes);
		METRICS.lastContextFfiCalls = ffiCalls;
		METRICS.lastContextFfiBytes = ffiBytes;
	}

	private static void recordBackendMetrics(VulkanicGalBridge.BackendMetrics metrics) {
		if (metrics == null) {
			return;
		}
		METRICS.commandLists = Math.max(METRICS.commandLists, metrics.commandLists());
		METRICS.commandOps = Math.max(METRICS.commandOps, metrics.commandOps());
		METRICS.backendSubmissions = Math.max(METRICS.backendSubmissions, metrics.backendSubmissions());
		METRICS.backendWaits = Math.max(METRICS.backendWaits, metrics.backendWaits());
		METRICS.glCalls = Math.max(METRICS.glCalls, metrics.glCalls());
		METRICS.glFlushes = Math.max(METRICS.glFlushes, metrics.glFlushes());
		METRICS.glFinishes = Math.max(METRICS.glFinishes, metrics.glFinishes());
		METRICS.glFencesInserted = Math.max(METRICS.glFencesInserted, metrics.glFencesInserted());
		METRICS.glFencesPolled = Math.max(METRICS.glFencesPolled, metrics.glFencesPolled());
		METRICS.glFencesWaited = Math.max(METRICS.glFencesWaited, metrics.glFencesWaited());
		METRICS.glFencesDeleted = Math.max(METRICS.glFencesDeleted, metrics.glFencesDeleted());
	}

	private static void recordFixedOperation(Operation operation, long inputBytes) {
		METRICS.ffiCalls++;
		METRICS.ffiBytes += inputBytes;
		METRICS.lastContextFfiCalls++;
		METRICS.lastContextFfiBytes += inputBytes;
		addOperation(operation, 1L, inputBytes);
	}

	private static void addOperation(Operation operation, long calls, long bytes) {
		switch (operation) {
			case CONTEXT_CREATE -> {
				METRICS.contextCreateCalls += calls;
				METRICS.contextCreateBytes += bytes;
			}
			case CAPABILITY_QUERY -> {
				METRICS.capabilityCalls += calls;
				METRICS.capabilityBytes += bytes;
			}
			case FRAME_CONFIGURE -> {
				METRICS.frameConfigureCalls += calls;
				METRICS.frameConfigureBytes += bytes;
			}
			case FRAME_ACQUIRE -> {
				METRICS.frameAcquireCalls += calls;
				METRICS.frameAcquireBytes += bytes;
			}
			case FRAME_RESIZE -> {
				METRICS.frameResizeCalls += calls;
				METRICS.frameResizeBytes += bytes;
			}
			case FRAME_PRESENT -> {
				METRICS.framePresentCalls += calls;
				METRICS.framePresentBytes += bytes;
			}
			case FRAME_CANCEL -> { }
			case RESOURCE_BATCH -> {
				METRICS.resourceBatchCalls += calls;
				METRICS.resourceBatchBytes += bytes;
			}
			case SUBMIT -> {
				METRICS.submitCalls += calls;
				METRICS.submitBytes += bytes;
			}
			case COMPLETION_QUERY -> {
				METRICS.completionQueryCalls += calls;
				METRICS.completionQueryBytes += bytes;
			}
			case RETIRE -> {
				METRICS.retireCalls += calls;
				METRICS.retireBytes += bytes;
			}
			case GUI_ASSET_UPDATE -> {
				METRICS.guiAssetUpdateCalls += calls;
				METRICS.guiAssetUpdateBytes += bytes;
			}
			case WORLD_BORDER_ASSET_UPDATE -> {
				METRICS.worldBorderAssetUpdateCalls += calls;
				METRICS.worldBorderAssetUpdateBytes += bytes;
			}
			case WORLD_CRACK_ASSET_UPDATE -> {
				METRICS.worldCrackAssetUpdateCalls += calls;
				METRICS.worldCrackAssetUpdateBytes += bytes;
			}
			case WORLD_MATERIAL_ASSET_UPDATE -> {
				METRICS.worldMaterialAssetUpdateCalls += calls;
				METRICS.worldMaterialAssetUpdateBytes += bytes;
			}
			case WORLD_TEXT_ASSET_UPDATE -> {
				METRICS.worldTextAssetUpdateCalls += calls;
				METRICS.worldTextAssetUpdateBytes += bytes;
			}
			case WORLD_MESH_ASSET_UPDATE -> {
				METRICS.worldMeshAssetUpdateCalls += calls;
				METRICS.worldMeshAssetUpdateBytes += bytes;
			}
			case WORLD_LOD_ASSET_UPDATE -> {
				METRICS.worldLodAssetUpdateCalls += calls;
				METRICS.worldLodAssetUpdateBytes += bytes;
			}
			case SHADER_PACK_SOURCE_UPDATE -> {
			}
			case SHADER_PACK_ASSET_UPDATE -> {
			}
		}
	}

	private static String metricsAuditLine(long frameBatchCount, long frameId, long submissionId, boolean wholeFrameVulkan) {
		RustGalWorldPrimitiveRenderer.WorldBorderAssetMetrics worldBorderAssetMetrics =
			RustGalWorldPrimitiveRenderer.worldBorderAssetMetrics();
		RustGalWorldPrimitiveRenderer.WorldCrackAssetMetrics worldCrackAssetMetrics =
			RustGalWorldPrimitiveRenderer.worldCrackAssetMetrics();
		RustGalWorldPrimitiveRenderer.WorldMaterialAssetMetrics worldMaterialAssetMetrics =
			RustGalWorldPrimitiveRenderer.worldMaterialAssetMetrics();
		return auditBackendPrefix(wholeFrameVulkan) + " GUI frame executed producer=gui.frame"
			+ " stratum=gui.frame"
			+ " frame_batch_count=" + frameBatchCount
			+ " frame=" + frameId
			+ " submission=" + submissionId
			+ " rust_gal_cache_hits=" + METRICS.cacheHits
			+ " rust_gal_cache_misses=" + METRICS.cacheMisses
			+ " rust_gal_queue_depth=" + SCHEDULER.pendingCount()
			+ " rust_gal_frames_executed=" + METRICS.frames
			+ " rust_gal_batches_executed=" + METRICS.batchesExecuted
			+ " rust_gal_sprite_batches_executed=" + METRICS.spriteBatchesExecuted
			+ " rust_gal_packed_sprites_executed=" + METRICS.packedSpritesExecuted
			+ " rust_gal_world_primitive_batches_executed=" + METRICS.worldPrimitiveBatchesExecuted
			+ " rust_gal_world_line_segments_executed=" + METRICS.worldLineSegmentsExecuted
			+ " rust_gal_world_line_vertices_executed=" + METRICS.worldLineVerticesExecuted
			+ " rust_gal_world_primitive_draws_executed=" + METRICS.worldPrimitiveDrawsExecuted
			+ " rust_gal_world_crack_quads_executed=" + METRICS.worldCrackQuadsExecuted
			+ " rust_gal_world_crack_batches_executed=" + METRICS.worldCrackBatchesExecuted
			+ " rust_gal_world_crack_draws_executed=" + METRICS.worldCrackDrawsExecuted
			+ " rust_gal_world_border_quads_executed=" + METRICS.worldBorderQuadsExecuted
			+ " rust_gal_world_border_batches_executed=" + METRICS.worldBorderBatchesExecuted
			+ " rust_gal_world_border_draws_executed=" + METRICS.worldBorderDrawsExecuted
			+ " rust_gal_world_material_quads_executed=" + METRICS.worldMaterialQuadsExecuted
			+ " rust_gal_world_material_batches_executed=" + METRICS.worldMaterialBatchesExecuted
			+ " rust_gal_world_material_draws_executed=" + METRICS.worldMaterialDrawsExecuted
			+ " rust_gal_world_mesh_instances_executed=" + METRICS.worldMeshInstancesExecuted
			+ " rust_gal_world_mesh_batches_executed=" + METRICS.worldMeshBatchesExecuted
			+ " rust_gal_world_mesh_draws_executed=" + METRICS.worldMeshDrawsExecuted
			+ " rust_gal_last_renderable_world_frame=" + lastRenderableWholeFrameWorldFrame
			+ " rust_gal_world_lod_selected_frames=" + METRICS.worldLodSelectedFrames
			+ " rust_gal_world_lod_instances_submitted=" + METRICS.worldLodInstancesSubmitted
			+ " rust_gal_world_lod_frames_executed=" + METRICS.worldLodFramesExecuted
			+ " rust_gal_world_background_clears_executed=" + METRICS.worldBackgroundClearsExecuted
			+ " rust_gal_world_background_diagnostic_fallbacks=" + METRICS.worldBackgroundDiagnosticFallbacks
			+ " rust_gal_world_background_sky_type=" + METRICS.lastWorldBackgroundSkyType
			+ " rust_gal_world_background_color_argb=" + Long.toUnsignedString(METRICS.lastWorldBackgroundColorArgb, 16)
			+ " rust_gal_world_depth_attachment_creates=" + METRICS.worldDepthAttachmentCreates
			+ " rust_gal_world_depth_attachment_reuses=" + METRICS.worldDepthAttachmentReuses
			+ " rust_gal_world_depth_attachment_retires=" + METRICS.worldDepthAttachmentRetires
			+ " rust_gal_world_outline_cache_hits=" + METRICS.worldOutlineCacheHits
			+ " rust_gal_world_outline_cache_misses=" + METRICS.worldOutlineCacheMisses
			+ " rust_gal_world_crack_cache_hits=" + METRICS.worldCrackCacheHits
			+ " rust_gal_world_crack_cache_misses=" + METRICS.worldCrackCacheMisses
			+ " rust_gal_world_crack_asset_generation=" + worldCrackAssetMetrics.generation()
			+ " rust_gal_world_crack_uploaded_asset_generation=" + worldCrackAssetMetrics.uploadedGeneration()
			+ " rust_gal_world_crack_asset_payload_count=" + worldCrackAssetMetrics.payloadCount()
			+ " rust_gal_world_crack_asset_payload_bytes=" + worldCrackAssetMetrics.payloadBytes()
			+ " rust_gal_world_crack_asset_update_failures=" + worldCrackAssetMetrics.failures()
			+ " rust_gal_world_crack_asset_source_pack=" + metricValue(worldCrackAssetMetrics.sourcePack())
			+ " rust_gal_world_crack_asset_sha256=" + metricValue(worldCrackAssetMetrics.sha256())
			+ " rust_gal_world_crack_asset_fallback=" + worldCrackAssetMetrics.fallback()
			+ " rust_gal_world_border_cache_hits=" + METRICS.worldBorderCacheHits
			+ " rust_gal_world_border_cache_misses=" + METRICS.worldBorderCacheMisses
			+ " rust_gal_world_material_cache_hits=" + METRICS.worldMaterialCacheHits
			+ " rust_gal_world_material_cache_misses=" + METRICS.worldMaterialCacheMisses
			+ " rust_gal_world_mesh_cache_hits=" + METRICS.worldMeshCacheHits
			+ " rust_gal_world_mesh_cache_misses=" + METRICS.worldMeshCacheMisses
			+ " rust_gal_world_border_asset_generation=" + worldBorderAssetMetrics.generation()
			+ " rust_gal_world_border_uploaded_asset_generation=" + worldBorderAssetMetrics.uploadedGeneration()
			+ " rust_gal_world_border_asset_payload_count=" + worldBorderAssetMetrics.payloadCount()
			+ " rust_gal_world_border_asset_payload_bytes=" + worldBorderAssetMetrics.payloadBytes()
			+ " rust_gal_world_border_asset_update_failures=" + worldBorderAssetMetrics.failures()
			+ " rust_gal_world_border_asset_source_pack=" + metricValue(worldBorderAssetMetrics.sourcePack())
			+ " rust_gal_world_border_asset_sha256=" + metricValue(worldBorderAssetMetrics.sha256())
			+ " rust_gal_world_border_asset_fallback=" + worldBorderAssetMetrics.fallback()
			+ " rust_gal_world_material_asset_generation=" + worldMaterialAssetMetrics.generation()
			+ " rust_gal_world_material_uploaded_asset_generation=" + worldMaterialAssetMetrics.uploadedGeneration()
			+ " rust_gal_world_material_asset_payload_count=" + worldMaterialAssetMetrics.payloadCount()
			+ " rust_gal_world_material_asset_payload_bytes=" + worldMaterialAssetMetrics.payloadBytes()
			+ " rust_gal_world_material_asset_update_failures=" + worldMaterialAssetMetrics.failures()
			+ " rust_gal_world_material_asset_source_pack=" + metricValue(worldMaterialAssetMetrics.sourcePack())
			+ " rust_gal_world_material_asset_sha256=" + metricValue(worldMaterialAssetMetrics.sha256())
			+ " rust_gal_world_material_asset_fallback=" + worldMaterialAssetMetrics.fallback()
			+ " rust_gal_frame_target_generations=" + METRICS.frameTargetGenerations
			+ " rust_gal_frame_target_identity_changes=" + METRICS.frameTargetIdentityChanges
			+ " rust_gal_last_frame_target_generation=" + METRICS.lastFrameTargetGeneration
			+ " rust_gal_last_frame_target_identity=" + METRICS.lastFrameTargetIdentity
			+ " rust_gal_batches_cancelled=" + METRICS.batchesCancelled
			+ " rust_gal_completion_polls=" + METRICS.completionPolls
			+ " rust_gal_completion_timeouts=" + METRICS.completionTimeouts
			+ " rust_gal_asset_generation=" + assetGeneration
			+ " rust_gal_uploaded_asset_generation=" + uploadedAssetGeneration
			+ " rust_gal_asset_payload_count=" + lastAssetPayloadCount
			+ " rust_gal_asset_payload_bytes=" + lastAssetPayloadBytes
			+ " rust_gal_asset_update_failures=" + assetUpdateFailures
			+ " rust_gal_ffi_context_create_calls=" + METRICS.contextCreateCalls
			+ " rust_gal_ffi_capability_calls=" + METRICS.capabilityCalls
			+ " rust_gal_ffi_frame_configure_calls=" + METRICS.frameConfigureCalls
			+ " rust_gal_ffi_frame_acquire_calls=" + METRICS.frameAcquireCalls
			+ " rust_gal_ffi_frame_resize_calls=" + METRICS.frameResizeCalls
			+ " rust_gal_ffi_frame_present_calls=" + METRICS.framePresentCalls
			+ " rust_gal_ffi_resource_batch_calls=" + METRICS.resourceBatchCalls
			+ " rust_gal_ffi_submit_calls=" + METRICS.submitCalls
			+ " rust_gal_ffi_completion_query_calls=" + METRICS.completionQueryCalls
			+ " rust_gal_ffi_retire_calls=" + METRICS.retireCalls
			+ " rust_gal_ffi_asset_update_calls=" + METRICS.guiAssetUpdateCalls
			+ " rust_gal_ffi_world_border_asset_update_calls=" + METRICS.worldBorderAssetUpdateCalls
			+ " rust_gal_ffi_world_crack_asset_update_calls=" + METRICS.worldCrackAssetUpdateCalls
			+ " rust_gal_ffi_world_material_asset_update_calls=" + METRICS.worldMaterialAssetUpdateCalls
			+ " rust_gal_ffi_world_mesh_asset_update_calls=" + METRICS.worldMeshAssetUpdateCalls
			+ " rust_gal_ffi_world_lod_asset_update_calls=" + METRICS.worldLodAssetUpdateCalls
			+ " rust_gal_ffi_context_create_bytes=" + METRICS.contextCreateBytes
			+ " rust_gal_ffi_capability_bytes=" + METRICS.capabilityBytes
			+ " rust_gal_ffi_frame_configure_bytes=" + METRICS.frameConfigureBytes
			+ " rust_gal_ffi_frame_acquire_bytes=" + METRICS.frameAcquireBytes
			+ " rust_gal_ffi_frame_resize_bytes=" + METRICS.frameResizeBytes
			+ " rust_gal_ffi_frame_present_bytes=" + METRICS.framePresentBytes
			+ " rust_gal_ffi_resource_batch_bytes=" + METRICS.resourceBatchBytes
			+ " rust_gal_ffi_submit_bytes=" + METRICS.submitBytes
			+ " rust_gal_ffi_completion_query_bytes=" + METRICS.completionQueryBytes
			+ " rust_gal_ffi_retire_bytes=" + METRICS.retireBytes
			+ " rust_gal_ffi_asset_update_bytes=" + METRICS.guiAssetUpdateBytes
			+ " rust_gal_ffi_world_border_asset_update_bytes=" + METRICS.worldBorderAssetUpdateBytes
			+ " rust_gal_ffi_world_crack_asset_update_bytes=" + METRICS.worldCrackAssetUpdateBytes
			+ " rust_gal_ffi_world_material_asset_update_bytes=" + METRICS.worldMaterialAssetUpdateBytes
			+ " rust_gal_ffi_world_mesh_asset_update_bytes=" + METRICS.worldMeshAssetUpdateBytes
			+ " rust_gal_ffi_world_lod_asset_update_bytes=" + METRICS.worldLodAssetUpdateBytes
			+ " rust_gal_enqueue_nanos=" + METRICS.enqueueNanos
			+ " rust_gal_resource_lookup_nanos=" + METRICS.resourceLookupNanos
			+ " rust_gal_resource_create_nanos=" + METRICS.resourceCreateNanos
			+ " rust_gal_abi_packing_nanos=" + METRICS.abiPackingNanos
			+ " rust_gal_profile_ffi_decode_nanos=" + METRICS.profileFfiDecodeNanos
			+ " rust_gal_profile_gui_frontend_nanos=" + METRICS.profileGuiFrontendNanos
			+ " rust_gal_profile_world_frontend_nanos=" + METRICS.profileWorldFrontendNanos
			+ " rust_gal_profile_world_validate_frame_nanos=" + METRICS.profileWorldValidateFrameNanos
			+ " rust_gal_profile_world_batching_nanos=" + METRICS.profileWorldBatchingNanos
			+ " rust_gal_profile_world_resource_prepare_nanos=" + METRICS.profileWorldResourcePrepareNanos
			+ " rust_gal_profile_world_mesh_expand_group_nanos=" + METRICS.profileWorldMeshSectionExpandGroupNanos
			+ " rust_gal_profile_shader_plan_lookup_nanos=" + METRICS.profileShaderPlanLookupNanos
			+ " rust_gal_profile_gal_command_generation_nanos=" + METRICS.profileGalCommandGenerationNanos
			+ " rust_gal_profile_gal_submit_total_nanos=" + METRICS.profileGalSubmitTotalNanos
			+ " rust_gal_profile_gal_validate_ops_nanos=" + METRICS.profileGalValidateOpsNanos
			+ " rust_gal_profile_gal_validate_handles_nanos=" + METRICS.profileGalValidateHandlesNanos
			+ " rust_gal_profile_gal_hazard_analysis_nanos=" + METRICS.profileGalHazardAnalysisNanos
			+ " rust_gal_profile_gal_hazard_read_events=" + METRICS.profileGalHazardReadEvents
			+ " rust_gal_profile_gal_hazard_write_events=" + METRICS.profileGalHazardWriteEvents
			+ " rust_gal_profile_gal_hazard_candidates_examined=" + METRICS.profileGalHazardCandidatesExamined
			+ " rust_gal_profile_gal_hazard_conflicts=" + METRICS.profileGalHazardConflicts
			+ " rust_gal_profile_gal_hazard_barriers_applied=" + METRICS.profileGalHazardBarriersApplied
			+ " rust_gal_profile_gal_hazard_active_read_entries=" + METRICS.profileGalHazardActiveReadEntries
			+ " rust_gal_profile_gal_hazard_active_write_entries=" + METRICS.profileGalHazardActiveWriteEntries
			+ " rust_gal_profile_gal_command_ops_before_normalize=" + METRICS.profileGalCommandOpsBeforeNormalize
			+ " rust_gal_profile_gal_command_ops_after_normalize=" + METRICS.profileGalCommandOpsAfterNormalize
			+ " rust_gal_profile_gal_redundant_pipeline_binds_removed=" + METRICS.profileGalRedundantPipelineBindsRemoved
			+ " rust_gal_profile_gal_redundant_resource_set_binds_removed=" + METRICS.profileGalRedundantResourceSetBindsRemoved
			+ " rust_gal_profile_gal_redundant_vertex_buffer_binds_removed=" + METRICS.profileGalRedundantVertexBufferBindsRemoved
			+ " rust_gal_profile_gal_redundant_index_buffer_binds_removed=" + METRICS.profileGalRedundantIndexBufferBindsRemoved
			+ " rust_gal_profile_backend_encode_nanos=" + METRICS.profileBackendEncodeNanos
			+ " rust_gal_profile_backend_submit_nanos=" + METRICS.profileBackendSubmitNanos
			+ " rust_gal_profile_backend_retire_nanos=" + METRICS.profileBackendRetireNanos
			+ " rust_gal_profile_vulkan_command_buffer_alloc_nanos=" + METRICS.profileVulkanCommandBufferAllocNanos
			+ " rust_gal_profile_vulkan_command_buffer_begin_nanos=" + METRICS.profileVulkanCommandBufferBeginNanos
			+ " rust_gal_profile_vulkan_command_recording_nanos=" + METRICS.profileVulkanCommandRecordingNanos
			+ " rust_gal_profile_vulkan_command_buffer_end_nanos=" + METRICS.profileVulkanCommandBufferEndNanos
			+ " rust_gal_profile_vulkan_queue_submit_nanos=" + METRICS.profileVulkanQueueSubmitNanos
			+ " rust_gal_profile_vulkan_timeline_poll_nanos=" + METRICS.profileVulkanTimelinePollNanos
			+ " rust_gal_profile_vulkan_timeline_wait_nanos=" + METRICS.profileVulkanTimelineWaitNanos
			+ " rust_gal_profile_vulkan_device_wait_idle_nanos=" + METRICS.profileVulkanDeviceWaitIdleNanos
			+ " rust_gal_profile_vulkan_acquire_nanos=" + METRICS.profileVulkanAcquireNanos
			+ " rust_gal_profile_vulkan_present_nanos=" + METRICS.profileVulkanPresentNanos
			+ " rust_gal_profile_vulkan_present_wait_nanos=" + METRICS.profileVulkanPresentWaitNanos
			+ " rust_gal_profile_vulkan_present_mode=" + METRICS.profileVulkanPresentMode
			+ " rust_gal_profile_vulkan_last_acquired_image_index=" + METRICS.profileVulkanLastAcquiredImageIndex
			+ " rust_gal_profile_vulkan_last_swapchain_generation=" + METRICS.profileVulkanLastSwapchainGeneration
			+ " rust_gal_profile_vulkan_last_images_in_flight=" + METRICS.profileVulkanLastImagesInFlight
			+ " rust_gal_profile_vulkan_last_available_frame_slots=" + METRICS.profileVulkanLastAvailableFrameSlots
			+ " rust_gal_profile_vulkan_command_buffers_allocated=" + METRICS.profileVulkanCommandBuffersAllocated
			+ " rust_gal_profile_vulkan_command_buffers_freed=" + METRICS.profileVulkanCommandBuffersFreed
			+ " rust_gal_profile_vulkan_wait_count=" + METRICS.profileVulkanWaitCount
			+ " rust_gal_profile_vulkan_device_wait_idle_count=" + METRICS.profileVulkanDeviceWaitIdleCount
			+ " rust_gal_profile_resource_creates_delta=" + METRICS.profileResourceCreatesDelta
			+ " rust_gal_profile_resource_destroys_delta=" + METRICS.profileResourceDestroysDelta
			+ " rust_gal_profile_host_write_ops=" + METRICS.profileHostWriteOps
			+ " rust_gal_profile_host_write_bytes=" + METRICS.profileHostWriteBytes
			+ " rust_gal_profile_barrier_ops=" + METRICS.profileBarrierOps
			+ " rust_gal_profile_pass_count=" + METRICS.profilePassCount
			+ " rust_gal_profile_draw_ops=" + METRICS.profileDrawOps
			+ " rust_gal_profile_draw_indexed_ops=" + METRICS.profileDrawIndexedOps
			+ " rust_gal_profile_pipeline_binds=" + METRICS.profilePipelineBinds
				+ " rust_gal_profile_resource_set_binds=" + METRICS.profileResourceSetBinds
				+ " rust_gal_profile_gpu_timestamp_unavailable_frames=" + METRICS.profileGpuTimestampUnavailableFrames
				+ " rust_gal_profile_g_buffer_persistent_cache_hits=" + METRICS.profileGBufferPersistentCacheHits
				+ " rust_gal_profile_g_buffer_persistent_cache_misses=" + METRICS.profileGBufferPersistentCacheMisses
				+ " rust_gal_profile_g_buffer_final_binding_cache_hits=" + METRICS.profileGBufferFinalBindingCacheHits
				+ " rust_gal_profile_g_buffer_final_binding_cache_misses=" + METRICS.profileGBufferFinalBindingCacheMisses
				+ " rust_gal_profile_g_buffer_attachment_creates=" + METRICS.profileGBufferAttachmentCreates
				+ " rust_gal_profile_g_buffer_pipeline_creates=" + METRICS.profileGBufferPipelineCreates
				+ " rust_gal_profile_g_buffer_shader_module_creates=" + METRICS.profileGBufferShaderModuleCreates
				+ " rust_gal_profile_g_buffer_descriptor_creates=" + METRICS.profileGBufferDescriptorCreates
				+ " rust_gal_profile_g_buffer_render_target_creates=" + METRICS.profileGBufferRenderTargetCreates
				+ " rust_gal_profile_g_buffer_resources_retired=" + METRICS.profileGBufferResourcesRetired
				+ " rust_gal_profile_g_buffer_final_pass_creates=" + METRICS.profileGBufferFinalPassCreates
				+ " rust_gal_frame_acquire_nanos=" + METRICS.frameAcquireNanos
				+ " rust_gal_submit_nanos=" + METRICS.submitNanos
				+ " rust_gal_frame_present_nanos=" + METRICS.framePresentNanos
			+ " rust_gal_retire_nanos=" + METRICS.retireNanos
			+ " rust_gal_completion_query_nanos=" + METRICS.completionQueryNanos
			+ " rust_gal_execute_nanos=" + METRICS.executeNanos
			+ " rust_gal_command_lists=" + METRICS.commandLists
			+ " rust_gal_command_ops=" + METRICS.commandOps
			+ " rust_gal_backend_submissions=" + METRICS.backendSubmissions
			+ " rust_gal_backend_waits=" + METRICS.backendWaits
			+ " rust_gal_gl_calls=" + METRICS.glCalls
			+ " rust_gal_gl_flushes=" + METRICS.glFlushes
			+ " rust_gal_gl_finishes=" + METRICS.glFinishes
			+ " rust_gal_gl_fences_inserted=" + METRICS.glFencesInserted
			+ " rust_gal_gl_fences_polled=" + METRICS.glFencesPolled
			+ " rust_gal_gl_fences_waited=" + METRICS.glFencesWaited
			+ " rust_gal_gl_fences_deleted=" + METRICS.glFencesDeleted
			+ " ffi_call_count=" + METRICS.ffiCalls
			+ " ffi_bytes=" + METRICS.ffiBytes;
	}

	private static String auditBackendPrefix(boolean wholeFrameVulkan) {
		return wholeFrameVulkan
			? "Rust VulkanicGAL"
			: "Rust OpenGL VulkanicGAL";
	}

	private static String metricValue(String value) {
		return value == null || value.isBlank() ? "unset" : value.replaceAll("\\s+", "_");
	}

	private static String clearColorString(int colorArgb) {
		float alpha = ((colorArgb >>> 24) & 0xFF) / 255.0F;
		float red = ((colorArgb >>> 16) & 0xFF) / 255.0F;
		float green = ((colorArgb >>> 8) & 0xFF) / 255.0F;
		float blue = (colorArgb & 0xFF) / 255.0F;
		return String.format(java.util.Locale.ROOT, "%.3f,%.3f,%.3f,%.3f", red, green, blue, alpha);
	}

	private static long elapsedSince(long started) {
		return Math.max(0L, System.nanoTime() - started);
	}

	private enum Operation {
		CONTEXT_CREATE,
		CAPABILITY_QUERY,
		FRAME_CONFIGURE,
		FRAME_ACQUIRE,
		FRAME_RESIZE,
		FRAME_PRESENT,
		FRAME_CANCEL,
		RESOURCE_BATCH,
		SUBMIT,
		COMPLETION_QUERY,
		RETIRE,
		GUI_ASSET_UPDATE,
		WORLD_BORDER_ASSET_UPDATE,
		WORLD_CRACK_ASSET_UPDATE,
		WORLD_MATERIAL_ASSET_UPDATE,
		WORLD_TEXT_ASSET_UPDATE,
		WORLD_MESH_ASSET_UPDATE,
		WORLD_LOD_ASSET_UPDATE,
		SHADER_PACK_SOURCE_UPDATE,
		SHADER_PACK_ASSET_UPDATE
	}

	private static final class Metrics {
		long frames;
		long submissions;
		long cacheHits;
		long cacheMisses;
		long resourceCreates;
		long resourceDestroys;
		long ffiCalls;
		long ffiBytes;
		long lastContextFfiCalls;
		long lastContextFfiBytes;
		long cancellations;
		long reloadInvalidations;
		long completionPolls;
		long completionTimeouts;
		long batchesExecuted;
		long spriteBatchesExecuted;
		long packedSpritesExecuted;
		long worldPrimitiveBatchesExecuted;
		long worldLineSegmentsExecuted;
		long worldLineVerticesExecuted;
		long worldPrimitiveDrawsExecuted;
		long worldCrackQuadsExecuted;
		long worldCrackBatchesExecuted;
		long worldCrackDrawsExecuted;
		long worldBorderQuadsExecuted;
		long worldBorderBatchesExecuted;
		long worldBorderDrawsExecuted;
		long worldMaterialQuadsExecuted;
		long worldMaterialBatchesExecuted;
		long worldMaterialDrawsExecuted;
		long worldMeshInstancesExecuted;
		long worldMeshBatchesExecuted;
		long worldMeshDrawsExecuted;
		long worldBackgroundClearsExecuted;
		long worldBackgroundDiagnosticFallbacks;
		long lastWorldBackgroundSkyType;
		long lastWorldBackgroundColorArgb;
		long worldDepthAttachmentCreates;
		long worldDepthAttachmentReuses;
		long worldDepthAttachmentRetires;
		long worldOutlineCacheHits;
		long worldOutlineCacheMisses;
		long worldCrackCacheHits;
		long worldCrackCacheMisses;
		long worldBorderCacheHits;
		long worldBorderCacheMisses;
		long worldMaterialCacheHits;
		long worldMaterialCacheMisses;
		long worldMeshCacheHits;
		long worldMeshCacheMisses;
		long frameTargetGenerations;
		long frameTargetIdentityChanges;
		long lastFrameTargetGeneration;
		long lastFrameTargetIdentity;
		long batchesCancelled;
		long contextCreateCalls;
		long capabilityCalls;
		long frameConfigureCalls;
		long frameAcquireCalls;
		long frameResizeCalls;
		long framePresentCalls;
		long resourceBatchCalls;
		long submitCalls;
		long completionQueryCalls;
		long retireCalls;
		long guiAssetUpdateCalls;
		long worldBorderAssetUpdateCalls;
		long worldCrackAssetUpdateCalls;
		long worldMaterialAssetUpdateCalls;
		long worldTextAssetUpdateCalls;
		long worldMeshAssetUpdateCalls;
		long worldLodAssetUpdateCalls;
		long worldLodSelectedFrames;
		long worldLodInstancesSubmitted;
		long worldLodFramesExecuted;
		long contextCreateBytes;
		long capabilityBytes;
		long frameConfigureBytes;
		long frameAcquireBytes;
		long frameResizeBytes;
		long framePresentBytes;
		long resourceBatchBytes;
		long submitBytes;
		long completionQueryBytes;
		long retireBytes;
		long guiAssetUpdateBytes;
		long worldBorderAssetUpdateBytes;
		long worldCrackAssetUpdateBytes;
		long worldMaterialAssetUpdateBytes;
		long worldTextAssetUpdateBytes;
		long worldMeshAssetUpdateBytes;
		long worldLodAssetUpdateBytes;
		long enqueueNanos;
		long resourceLookupNanos;
		long resourceCreateNanos;
		long abiPackingNanos;
		long profileFfiDecodeNanos;
		long profileGuiFrontendNanos;
		long profileWorldFrontendNanos;
		long profileWorldValidateFrameNanos;
		long profileWorldBatchingNanos;
		long profileWorldResourcePrepareNanos;
		long profileWorldMeshSectionExpandGroupNanos;
		long profileShaderPlanLookupNanos;
		long profileGalCommandGenerationNanos;
		long profileGalSubmitTotalNanos;
		long profileGalValidateOpsNanos;
		long profileGalValidateHandlesNanos;
		long profileGalHazardAnalysisNanos;
		long profileGalHazardReadEvents;
		long profileGalHazardWriteEvents;
		long profileGalHazardCandidatesExamined;
		long profileGalHazardConflicts;
		long profileGalHazardBarriersApplied;
		long profileGalHazardActiveReadEntries;
		long profileGalHazardActiveWriteEntries;
		long profileGalCommandOpsBeforeNormalize;
		long profileGalCommandOpsAfterNormalize;
		long profileGalRedundantPipelineBindsRemoved;
		long profileGalRedundantResourceSetBindsRemoved;
		long profileGalRedundantVertexBufferBindsRemoved;
		long profileGalRedundantIndexBufferBindsRemoved;
		long profileBackendEncodeNanos;
		long profileBackendSubmitNanos;
		long profileBackendRetireNanos;
		long profileVulkanCommandBufferAllocNanos;
		long profileVulkanCommandBufferBeginNanos;
		long profileVulkanCommandRecordingNanos;
		long profileVulkanCommandBufferEndNanos;
		long profileVulkanQueueSubmitNanos;
		long profileVulkanTimelinePollNanos;
		long profileVulkanTimelineWaitNanos;
		long profileVulkanDeviceWaitIdleNanos;
		long profileVulkanAcquireNanos;
		long profileVulkanPresentNanos;
		long profileVulkanPresentWaitNanos;
		long profileVulkanPresentMode;
		long profileVulkanLastAcquiredImageIndex;
		long profileVulkanLastSwapchainGeneration;
		long profileVulkanLastImagesInFlight;
		long profileVulkanLastAvailableFrameSlots;
		long profileVulkanCommandBuffersAllocated;
		long profileVulkanCommandBuffersFreed;
		long profileVulkanWaitCount;
		long profileVulkanDeviceWaitIdleCount;
		long profileResourceCreatesDelta;
		long profileResourceDestroysDelta;
		long profileHostWriteOps;
		long profileHostWriteBytes;
		long profileBarrierOps;
		long profilePassCount;
		long profileDrawOps;
		long profileDrawIndexedOps;
		long profilePipelineBinds;
		long profileResourceSetBinds;
		long profileGpuTimestampUnavailableFrames;
		long profileGBufferPersistentCacheHits;
		long profileGBufferPersistentCacheMisses;
		long profileGBufferFinalBindingCacheHits;
		long profileGBufferFinalBindingCacheMisses;
		long profileGBufferAttachmentCreates;
		long profileGBufferPipelineCreates;
		long profileGBufferShaderModuleCreates;
		long profileGBufferDescriptorCreates;
		long profileGBufferRenderTargetCreates;
		long profileGBufferResourcesRetired;
		long profileGBufferFinalPassCreates;
		long frameAcquireNanos;
		long submitNanos;
		long framePresentNanos;
		long retireNanos;
		long completionQueryNanos;
		long executeNanos;
		long commandLists;
		long commandOps;
		long backendSubmissions;
		long backendWaits;
		long glCalls;
		long glFlushes;
		long glFinishes;
		long glFencesInserted;
		long glFencesPolled;
		long glFencesWaited;
		long glFencesDeleted;
	}

	public record MetricsSnapshot(
		long frames,
		long submissions,
		long cacheHits,
		long cacheMisses,
		long resourceCreates,
		long resourceDestroys,
		long ffiCalls,
		long ffiBytes,
		long cancellations,
		long reloadInvalidations,
		long completionPolls,
		long completionTimeouts,
		long pendingBatches,
		long batchesExecuted,
		long spriteBatchesExecuted,
		long packedSpritesExecuted,
		long batchesCancelled,
		long contextCreateCalls,
		long capabilityCalls,
		long frameConfigureCalls,
		long frameAcquireCalls,
		long frameResizeCalls,
		long framePresentCalls,
		long resourceBatchCalls,
		long submitCalls,
		long completionQueryCalls,
		long retireCalls,
		long contextCreateBytes,
		long capabilityBytes,
		long frameConfigureBytes,
		long frameAcquireBytes,
		long frameResizeBytes,
		long framePresentBytes,
		long resourceBatchBytes,
		long submitBytes,
		long completionQueryBytes,
		long retireBytes,
		long enqueueNanos,
		long resourceLookupNanos,
		long resourceCreateNanos,
		long abiPackingNanos,
		long frameAcquireNanos,
		long submitNanos,
		long framePresentNanos,
		long retireNanos,
		long completionQueryNanos,
		long executeNanos,
		long commandLists,
		long commandOps,
		long backendSubmissions,
		long backendWaits,
		long glCalls,
		long glFlushes,
		long glFinishes,
		long glFencesInserted,
		long glFencesPolled,
		long glFencesWaited,
		long glFencesDeleted
	) {
	}
}
