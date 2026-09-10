package net.vulkanic.world;

import net.minecraft.client.Camera;
import net.minecraft.client.Minecraft;
import net.minecraft.core.BlockPos;
import net.minecraft.client.renderer.chunk.ChunkSectionLayer;
import net.minecraft.client.renderer.texture.SpriteContents;
import net.minecraft.client.renderer.texture.TextureAtlas;
import net.minecraft.client.renderer.texture.TextureAtlasSprite;
import net.minecraft.data.AtlasIds;
import net.minecraft.resources.ResourceLocation;
import net.minecraft.server.packs.resources.Resource;
import net.sodium.client.render.chunk.RenderSection;
import net.sodium.client.render.chunk.compile.ChunkBuildOutput;
import net.sodium.client.render.chunk.compile.ChunkSortOutput;
import net.sodium.client.render.chunk.data.BuiltSectionInfo;
import net.sodium.client.render.chunk.data.BuiltSectionMeshParts;
import net.sodium.client.render.chunk.lists.ChunkRenderList;
import net.sodium.client.render.chunk.lists.SortedRenderLists;
import net.sodium.client.render.chunk.terrain.DefaultTerrainRenderPasses;
import net.sodium.client.render.chunk.terrain.TerrainRenderPass;
import net.sodium.client.render.chunk.translucent_sorting.data.SharedIndexSorter;
import net.sodium.client.render.chunk.translucent_sorting.data.Sorter;
import net.sodium.client.render.chunk.vertex.format.NativeSectionMeshBuilder;
import net.sodium.client.util.iterator.ByteIterator;
import net.sodium.client.util.NativeBuffer;
import net.irisshaders.iris.shaderpack.materialmap.WorldRenderingSettings;
import net.vulkanic.bridge.VulkanicGalBridge;
import org.slf4j.Logger;
import net.logging.LogUtils;

import javax.imageio.ImageIO;
import java.awt.image.BufferedImage;
import java.io.ByteArrayOutputStream;
import java.io.IOException;
import java.io.InputStream;
import java.nio.ByteBuffer;
import java.nio.ByteOrder;
import java.util.ArrayDeque;
import java.util.ArrayList;
import java.util.Arrays;
import java.util.Collections;
import java.util.Comparator;
import java.util.HashSet;
import java.util.Iterator;
import java.util.List;
import java.util.Locale;
import java.util.Map;
import java.util.Optional;
import java.util.Set;
import java.util.concurrent.ConcurrentHashMap;
import java.util.concurrent.atomic.AtomicLong;

public final class RustGalTerrainRenderer {
	private static final Logger LOGGER = LogUtils.getLogger();
	private static final int INDEX_TYPE_U16 = 1;
	private static final int INDEX_TYPE_U32 = 2;
	private static final int POSITION_MAX_VALUE = 1 << 20;
	private static final int TEXTURE_MAX_VALUE = 1 << 15;
	/** Frozen Sodium's non-macOS compact-vertex sampling precision. */
	private static final int COMPACT_TEXTURE_SUB_TEXEL_PRECISION = 1 << 8;
	private static final int COMPACT_PREFIX_STRIDE = 20;
	private static final int POSITION_OFFSET = 0;
	private static final int COLOR_OFFSET = 8;
	private static final int TEXTURE_OFFSET = 12;
	private static final int LIGHT_MATERIAL_OFFSET = 16;
	private static final float SECTION_LOCAL_MIN = -8.01F;
	private static final float SECTION_LOCAL_MAX = 24.01F;
	private static final int MAX_RECENT_EVENTS = 8192;
	private static final int MAX_LIFECYCLE_EVENTS = 256;
	// Keep enough bounded history for a deterministic camera sequence to retain
	// the initial mesh/accounting records while later frames append visibility
	// and sort receipts.  This is diagnostics-only state; it does not increase
	// semantic mesh residency or GPU resource bounds.
	private static final int MAX_TRANSLUCENT_EVENTS = 16384;
	/** Bounds each copied atlas before base/normal/specular expansion. */
	private static final long MAX_RUST_ATLAS_PIXELS = 16_777_216L;
	private static final String STATIC_TERRAIN_SCENARIO_PROPERTY = "mattmc.dev.rustGalStaticTerrain.scenario";
	private static final String FAULT_PROPERTY = "mattmc.dev.rustGalStaticTerrain.fault";
	private static final Map<LayerKey, TerrainSectionAsset> SECTION_ASSETS = new ConcurrentHashMap<>();
	/** Bounded semantic-only probe samples retained after Rust accepts a mesh. */
	private static final Map<BlockPos, List<VulkanicGalBridge.WorldMeshVertexRecord>> TEXTURE_PROBE_QUADS = new ConcurrentHashMap<>();
	private static volatile List<TerrainTextureProbe> activeTextureProbes = List.of();
	/** Must match the Rust whole-frame static-terrain residency bound. */
	private static final int MAX_SEMANTIC_TERRAIN_SECTIONS = 4096;
	private static final ArrayDeque<TerrainDiagnosticEvent> RECENT_EVENTS = new ArrayDeque<>(MAX_RECENT_EVENTS);
	private static final ArrayDeque<TerrainDiagnosticEvent> LIFECYCLE_EVENTS = new ArrayDeque<>(MAX_LIFECYCLE_EVENTS);
	private static final ArrayDeque<TerrainDiagnosticEvent> TRANSLUCENT_EVENTS = new ArrayDeque<>(MAX_TRANSLUCENT_EVENTS);
	/**
	 * Sort metadata is part of the semantic submission, not mutable mesh-cache
	 * state.  Rust execution can occur after a later camera sort has replaced the
	 * cache entry, so retain the exact receipt by gameplay frame and mesh key.
	 */
	private static final Map<Long, ArrayDeque<TranslucentExecutionMetadata>> TRANSLUCENT_EXECUTION_METADATA = new ConcurrentHashMap<>();
	private static volatile long atlasGeneration;
	private static volatile long registeredAtlasGeneration;
	private static volatile long publishedWorldMeshAtlasGeneration;
	private static VulkanicGalBridge.WorldMeshTextureAssetRecord publishedWorldMeshAtlasPayload;
	/** Semantic atlas generation last copied into the Rust-owned PNG payload. */
	private static volatile long copiedAtlasSemanticGeneration;
	/** Animation-frame key last copied into the Rust-owned PNG payload. */
	private static volatile long copiedAtlasSemanticFrameKey = Long.MIN_VALUE;
	private static volatile byte[] atlasPayload;
	/** Exact Frozen-compatible sprite-isolated atlas levels, excluding mip zero. */
	private static volatile List<byte[]> atlasMipPayloads = List.of();
	private static volatile byte[] normalAtlasPayload;
	private static volatile byte[] specularAtlasPayload;
	private static volatile FluidSpriteAsset waterStillAsset;
	private static volatile AtlasAnimationResource atlasAnimationSource;
	private static volatile FluidSpriteAsset waterFlowAsset;
	private static volatile FluidSpriteAsset waterOverlayAsset;
	private static final AtomicLong acceptedBuildOutputs = new AtomicLong();
	private static final AtomicLong skippedRouteBuildOutputs = new AtomicLong();
	private static final AtomicLong skippedUnsupportedAnimatedSections = new AtomicLong();
	private static final AtomicLong skippedUnsupportedFluidTranslucentSections = new AtomicLong();
	private static final AtomicLong acceptedWaterAnimatedSections = new AtomicLong();
	private static final AtomicLong unsupportedFluidRejectedSections = new AtomicLong();
	private static final AtomicLong skippedEmptyLayers = new AtomicLong();
	private static final AtomicLong registeredMeshes = new AtomicLong();
	private static final AtomicLong registeredTranslucentSorts = new AtomicLong();
	private static final AtomicLong registeredTranslucentSortBytes = new AtomicLong();
	private static final AtomicLong texturePayloadUpdates = new AtomicLong();
	private static final AtomicLong texturePayloadUpdateBytes = new AtomicLong();
	private static final AtomicLong atlasTextureOnlyUpdates = new AtomicLong();
	private static final AtomicLong atlasMissingPayloadUpdates = new AtomicLong();
	private static final AtomicLong atlasMalformedPayloadUpdates = new AtomicLong();
	private static final AtomicLong atlasPartialPayloadUpdates = new AtomicLong();
	private static final AtomicLong removedLayers = new AtomicLong();
	private static final AtomicLong visibleLayerProbes = new AtomicLong();
	private static final AtomicLong visibleLayerSubmissions = new AtomicLong();
	private static final AtomicLong failedLayerSubmissions = new AtomicLong();
	private static final AtomicLong lastVisibleSubmissionFrameId = new AtomicLong(-1L);
	private static final AtomicLong currentFrameVisibleLayerSubmissions = new AtomicLong();
	private static final AtomicLong lastExecutedStaticTerrainFrameId = new AtomicLong(-1L);
	private static final AtomicLong lastExecutedStaticTerrainSubmissionId = new AtomicLong(-1L);
	private static final AtomicLong lastExecutedStaticTerrainInstances = new AtomicLong();
	private static final AtomicLong invalidations = new AtomicLong();
	private static final AtomicLong terrainExtractionFrames = new AtomicLong();
	private static final AtomicLong rustEnqueueFrames = new AtomicLong();
	private static final AtomicLong translucentSortGenerations = new AtomicLong();
	private static volatile TerrainSectionAsset lastWorldUnloadAsset;
	private static volatile long lastWorldUnloadSectionPos;
	private static volatile ChunkSectionLayer lastWorldUnloadLayer = ChunkSectionLayer.SOLID;

	private record TranslucentExecutionMetadata(
		long sortGeneration,
		long sortedIndexHash,
		double cameraX,
		double cameraY,
		double cameraZ,
		int drawOrder
	) {}


	private static void retainTranslucentExecutionMetadata(long meshKey, TranslucentExecutionMetadata metadata) {
		ArrayDeque<TranslucentExecutionMetadata> queue = TRANSLUCENT_EXECUTION_METADATA.computeIfAbsent(
			meshKey, ignored -> new ArrayDeque<>()
		);
		synchronized (queue) {
			if (queue.size() >= 32) {
				queue.removeFirst();
			}
			queue.addLast(metadata);
		}
	}

	private static TranslucentExecutionMetadata takeTranslucentExecutionMetadata(long meshKey) {
		ArrayDeque<TranslucentExecutionMetadata> queue = TRANSLUCENT_EXECUTION_METADATA.get(meshKey);
		if (queue == null) {
			return null;
		}
		synchronized (queue) {
			TranslucentExecutionMetadata metadata = queue.pollFirst();
			if (queue.isEmpty()) {
				TRANSLUCENT_EXECUTION_METADATA.remove(meshKey, queue);
			}
			return metadata;
		}
	}

	private record FluidSpriteAsset(
		int textureId,
		ResourceLocation location,
		float u0,
		float u1,
		float v0,
		float v1,
		int frameWidth,
		int frameHeight,
		int frameCount,
		int frameTicks,
		int animationFlags,
		int frameRowSize,
		int interpolationPolicy,
		List<VulkanicGalBridge.WorldMeshAnimationFrameRecord> animationFrames,
		byte[] pngBytes,
		int mipLevels
	) {
		long animationHash() {
			long hash = 0xcbf29ce484222325L;
			hash = fnv64Int(hash, textureId);
			hash = fnv64Int(hash, frameWidth);
			hash = fnv64Int(hash, frameHeight);
			hash = fnv64Int(hash, frameCount);
			hash = fnv64Int(hash, frameTicks);
			hash = fnv64Int(hash, frameRowSize);
			hash = fnv64Int(hash, interpolationPolicy);
			for (VulkanicGalBridge.WorldMeshAnimationFrameRecord frame : animationFrames) {
				hash = fnv64Int(hash, frame.frameIndex());
				hash = fnv64Int(hash, frame.durationTicks());
			}
			return hash;
		}

		String animationSummary() {
			StringBuilder summary = new StringBuilder();
			summary.append(textureId)
				.append('/')
				.append(location.toString().replace(':', '~'))
				.append('/')
				.append(frameWidth)
				.append('x')
				.append(frameHeight)
				.append('/')
				.append(frameCount)
				.append('/')
				.append(frameTicks)
				.append('/')
				.append(frameRowSize)
				.append('/')
				.append(interpolationPolicy)
				.append('/');
			int limit = Math.min(animationFrames.size(), 64);
			for (int index = 0; index < limit; index++) {
				if (index > 0) {
					summary.append(',');
				}
				VulkanicGalBridge.WorldMeshAnimationFrameRecord frame = animationFrames.get(index);
				summary.append(frame.frameIndex()).append('@').append(frame.durationTicks());
			}
			if (animationFrames.size() > limit) {
				summary.append(",...");
			}
			return summary.toString();
		}

		VulkanicGalBridge.WorldMeshTextureAssetRecord textureRecord() {
			var record = new VulkanicGalBridge.WorldMeshTextureAssetRecord(
				textureId,
				pngBytes,
				frameWidth,
				frameHeight,
				frameCount,
				frameTicks,
				animationFlags,
				frameRowSize,
				interpolationPolicy,
				animationFrames
			);
			return Boolean.getBoolean("mattmc.dev.rustGalFluidMipLevels") ? record.withMipLevels(mipLevels) : record;
		}

		boolean contains(float u, float v) {
			float minU = Math.min(u0, u1) - 0.00001F;
			float maxU = Math.max(u0, u1) + 0.00001F;
			float minV = Math.min(v0, v1) - 0.00001F;
			float maxV = Math.max(v0, v1) + 0.00001F;
			return u >= minU && u <= maxU && v >= minV && v <= maxV;
		}

		float localU(float u) {
			return (u - u0) / (u1 - u0);
		}

		float localV(float v) {
			return (v - v0) / (v1 - v0);
		}
	}

	static void installTestingFluidSpriteAssetsForUnitTests() {
		waterStillAsset = new FluidSpriteAsset(
			RustGalWorldPrimitiveRenderer.MATERIAL_TEXTURE_WATER_STILL,
			ResourceLocation.fromNamespaceAndPath("minecraft", "block/water_still"),
			0.0F,
			0.25F,
			0.0F,
			0.25F,
			16,
			16,
			1,
			1,
			0,
			0,
			0,
			List.of(),
			new byte[] { 1 }, 1
		);
		waterFlowAsset = new FluidSpriteAsset(
			RustGalWorldPrimitiveRenderer.MATERIAL_TEXTURE_WATER_FLOW,
			ResourceLocation.fromNamespaceAndPath("minecraft", "block/water_flow"),
			0.25F,
			0.5F,
			0.0F,
			0.25F,
			16,
			16,
			1,
			1,
			0,
			0,
			0,
			List.of(),
			new byte[] { 2 }, 1
		);
		waterOverlayAsset = new FluidSpriteAsset(
			RustGalWorldPrimitiveRenderer.MATERIAL_TEXTURE_WATER_OVERLAY,
			ResourceLocation.fromNamespaceAndPath("minecraft", "block/water_overlay"),
			0.5F,
			0.75F,
			0.0F,
			0.25F,
			16,
			16,
			1,
			1,
			0,
			0,
			0,
			List.of(),
			new byte[] { 3 }, 1
		);
	}

	private RustGalTerrainRenderer() {
	}

	public static void acceptChunkBuildOutput(ChunkBuildOutput output) {
		// Select the owner before constructing any layout.  The whole-frame
		// Vulkan producer is compact and must not even query Iris's live vertex
		// format; Iris-derived layout metadata is private to the OpenGL route.
		if (net.vulkanic.VulkanicAPI.isVulkanBackendSelected()) {
			if (!WorldRenderRoutePolicy.currentStaticTerrainRoute().usesRustWholeFrameVulkan()) {
				skippedRouteBuildOutputs.incrementAndGet();
				return;
			}
			acceptChunkBuildOutput(output, TerrainMeshLayout.compact());
			return;
		}
		acceptChunkBuildOutput(output, TerrainMeshLayout.activeIrisCompatible());
	}

	/**
	 * Admits a CPU-built section directly into the explicit Rust terrain route.
	 * The layout is carried by the producer rather than recovered from Iris at
	 * decode time, so selecting Vulkan cannot borrow shader-pack runtime state.
	 */
	public static void acceptWholeFrameChunkBuildOutput(ChunkBuildOutput output) {
		acceptChunkBuildOutput(output, TerrainMeshLayout.compact());
	}

	private static void acceptChunkBuildOutput(ChunkBuildOutput output, TerrainMeshLayout layout) {
		if (!WorldRenderRoutePolicy.currentStaticTerrainRoute().usesRustWholeFrameVulkan()) {
			skippedRouteBuildOutputs.incrementAndGet();
			return;
		}
		if (output == null || output.info == null) {
			return;
		}
		net.sodium.client.render.StaticTerrainParityDiagnostics.recordRustStaticTerrainAdmission(
			output,
			"accepted"
		);
		if (output.info.animatedSprites != null) {
			acceptedWaterAnimatedSections.incrementAndGet();
		}
		acceptedBuildOutputs.incrementAndGet();
		long extractionFrameId = terrainExtractionFrames.incrementAndGet();
		ensureAtlasPayload();
		acceptLayer(output, DefaultTerrainRenderPasses.SOLID, ChunkSectionLayer.SOLID, extractionFrameId, layout);
		acceptLayer(output, DefaultTerrainRenderPasses.CUTOUT, ChunkSectionLayer.CUTOUT_MIPPED, extractionFrameId, layout);
		acceptLayer(output, DefaultTerrainRenderPasses.TRANSLUCENT, ChunkSectionLayer.TRANSLUCENT, extractionFrameId, layout);
		recordTranslucentSortData(output);
	}

	private static void recordTranslucentSortData(ChunkBuildOutput output) {
		if (output == null || output.translucentData == null || !isStaticTerrainTranslucentScenario()) {
			return;
		}
		TerrainSectionAsset asset = SECTION_ASSETS.get(new LayerKey(output.render.getPosition().asLong(), ChunkSectionLayer.TRANSLUCENT));
		if (asset == null) {
			return;
		}
		String reason = "translucent-sort-data:"
			+ output.translucentData.getSortType().name().toLowerCase(Locale.ROOT)
			+ ":"
			+ output.translucentData.getClass().getSimpleName();
		recordEvent(
			output.render.getPosition().asLong(),
			ChunkSectionLayer.TRANSLUCENT,
			output.submitTime,
			asset.meshGeneration(),
			asset.meshGeneration(),
			atlasGeneration,
			asset,
			output.render.getOriginX(),
			output.render.getOriginY(),
			output.render.getOriginZ(),
			0.0F,
			0.0F,
			0.0F,
			reason,
			0L,
			0L,
			0L,
			0L,
			asset.meshGeneration(),
			currentCameraX(),
			currentCameraY(),
			currentCameraZ(),
			output.render.getOriginX() + 8.0D,
			output.render.getOriginY() + 8.0D,
			output.render.getOriginZ() + 8.0D,
			Math.max(0, asset.indexCount() / 6),
			asset.contentHash(),
			asset.meshGeneration(),
			0
		);
	}

	private static void recordInitialTranslucentSort(ChunkBuildOutput output, TerrainSectionAsset asset) {
		if (output == null || asset == null || asset.initialSortGeneration() <= 0L) {
			return;
		}
		byte[] indexBytes = asset.asset().indexBytes();
		if (indexBytes.length == 0) {
			return;
		}
		long sortedIndexHash = sortedIndexHash(indexBytes);
		long sortedIndexSampleHash = sortedIndexSampleHash(indexBytes);
		String sortedIndexSample = sortedIndexSample(indexBytes, asset.indexType(), 12);
		String sorterType = initialTranslucentSorterType(output);
		long sectionPos = output.render.getPosition().asLong();
		recordTranslucentSortPayloadEvent(
			sectionPos,
			output.submitTime,
			asset,
			output.render.getOriginX(),
			output.render.getOriginY(),
			output.render.getOriginZ(),
			"translucent-source-sort",
			asset.initialSortGeneration(),
			currentCameraX(),
			currentCameraY(),
			currentCameraZ(),
			output.render.getOriginX() + 8.0D,
			output.render.getOriginY() + 8.0D,
			output.render.getOriginZ() + 8.0D,
			Math.max(0, asset.indexCount() / 6),
			sortedIndexHash,
			asset.initialSortGeneration(),
			sorterType,
			sortedIndexHash,
			0L,
			sortedIndexSampleHash,
			0L,
			sortedIndexSample
		);
		recordTranslucentSortPayloadEvent(
			sectionPos,
			output.submitTime,
			asset,
			output.render.getOriginX(),
			output.render.getOriginY(),
			output.render.getOriginZ(),
			"translucent-rust-sort-copy",
			asset.initialSortGeneration(),
			currentCameraX(),
			currentCameraY(),
			currentCameraZ(),
			output.render.getOriginX() + 8.0D,
			output.render.getOriginY() + 8.0D,
			output.render.getOriginZ() + 8.0D,
			Math.max(0, asset.indexCount() / 6),
			sortedIndexHash,
			asset.initialSortGeneration(),
			"rust-route-cache-initial",
			0L,
			sortedIndexHash,
			0L,
			sortedIndexSampleHash,
			sortedIndexSample
		);
		recordTranslucentSortPayloadEvent(
			sectionPos,
			output.submitTime,
			asset,
			output.render.getOriginX(),
			output.render.getOriginY(),
			output.render.getOriginZ(),
			"translucent-sort-registered:authority=sodium-build-index-payload"
				+ ":sourceHash=" + sortedIndexHash
				+ ":copiedHash=" + sortedIndexHash
				+ ":sourceSampleHash=" + sortedIndexSampleHash
				+ ":copiedSampleHash=" + sortedIndexSampleHash
				+ ":sample=" + sortedIndexSample
				+ ":sorterType=" + sorterType,
			asset.initialSortGeneration(),
			currentCameraX(),
			currentCameraY(),
			currentCameraZ(),
			output.render.getOriginX() + 8.0D,
			output.render.getOriginY() + 8.0D,
			output.render.getOriginZ() + 8.0D,
			Math.max(0, asset.indexCount() / 6),
			sortedIndexHash,
			asset.initialSortGeneration(),
			sorterType,
			sortedIndexHash,
			sortedIndexHash,
			sortedIndexSampleHash,
			sortedIndexSampleHash,
			sortedIndexSample
		);
	}

	public static void acceptChunkSortOutput(ChunkSortOutput output) {
		if (!WorldRenderRoutePolicy.currentStaticTerrainRoute().usesRustWholeFrameVulkan()) {
			return;
		}
		if (output == null || output.isReusingUploadedIndexData()) {
			return;
		}
		TerrainSectionAsset asset = SECTION_ASSETS.get(new LayerKey(output.render.getPosition().asLong(), ChunkSectionLayer.TRANSLUCENT));
		if (asset == null) {
			return;
		}
		byte[] indexBytes = normalizeTranslucentSortedIndexBytes(
			copySorterIndexBytes(output.getSorter()),
			asset.vertexCount(),
			asset.translucentSourceSegmentQuadCounts()
		);
		if (indexBytes.length == 0) {
			recordEvent(
				output.render.getPosition().asLong(),
				ChunkSectionLayer.TRANSLUCENT,
				output.submitTime,
				asset.meshGeneration(),
				asset.meshGeneration(),
				atlasGeneration,
				asset,
				output.render.getOriginX(),
				output.render.getOriginY(),
				output.render.getOriginZ(),
				0.0F,
				0.0F,
				0.0F,
				"translucent-sort-missing",
				0L,
				0L,
				0L,
				0L,
				0L,
				currentCameraX(),
				currentCameraY(),
				currentCameraZ(),
				output.render.getOriginX() + 8.0D,
				output.render.getOriginY() + 8.0D,
				output.render.getOriginZ() + 8.0D,
				Math.max(0, asset.indexCount() / 6),
				0L,
				0L,
				0
			);
			return;
		}
		long indexGeneration = translucentSortGenerations.incrementAndGet();
		long sortedIndexHash = sortedIndexHash(indexBytes);
		long sortedIndexSampleHash = sortedIndexSampleHash(indexBytes);
		String sortedIndexSample = sortedIndexSample(indexBytes, INDEX_TYPE_U32, 12);
		String sorterType = translucentSorterType(output.getSorter());
		recordTranslucentSortPayloadEvent(
			output.render.getPosition().asLong(),
			output.submitTime,
			asset,
			output.render.getOriginX(),
			output.render.getOriginY(),
			output.render.getOriginZ(),
			"translucent-source-sort",
			indexGeneration,
			currentCameraX(),
			currentCameraY(),
			currentCameraZ(),
			output.render.getOriginX() + 8.0D,
			output.render.getOriginY() + 8.0D,
			output.render.getOriginZ() + 8.0D,
			Math.max(0, indexBytes.length / (Integer.BYTES * 6)),
			sortedIndexHash,
			indexGeneration,
			sorterType,
			sortedIndexHash,
			0L,
			sortedIndexSampleHash,
			0L,
			sortedIndexSample
		);
		RustGalWorldPrimitiveRenderer.registerStaticTerrainSortedIndex(new VulkanicGalBridge.WorldMeshSortedIndexRecord(
			asset.meshKey(),
			asset.meshGeneration(),
			indexGeneration,
			INDEX_TYPE_U32,
			indexBytes
		));
		registeredTranslucentSorts.incrementAndGet();
		registeredTranslucentSortBytes.addAndGet(indexBytes.length);
		recordEvent(
			output.render.getPosition().asLong(),
			ChunkSectionLayer.TRANSLUCENT,
			output.submitTime,
			asset.meshGeneration(),
			indexGeneration,
			atlasGeneration,
			asset,
			output.render.getOriginX(),
			output.render.getOriginY(),
			output.render.getOriginZ(),
			0.0F,
			0.0F,
			0.0F,
			"translucent-sort-registered"
				+ ":authority=sodium-source-payload"
				+ ":sourceHash=" + sortedIndexHash
				+ ":copiedHash=" + sortedIndexHash
				+ ":sourceSampleHash=" + sortedIndexSampleHash
				+ ":copiedSampleHash=" + sortedIndexSampleHash
				+ ":sample=" + sortedIndexSample
				+ ":sorterType=" + sorterType,
			0L,
			0L,
			0L,
			0L,
			indexGeneration,
			currentCameraX(),
			currentCameraY(),
			currentCameraZ(),
			output.render.getOriginX() + 8.0D,
			output.render.getOriginY() + 8.0D,
			output.render.getOriginZ() + 8.0D,
			Math.max(0, indexBytes.length / (Integer.BYTES * 6)),
			sortedIndexHash,
			indexGeneration,
			0,
			sorterType,
			sortedIndexHash,
			sortedIndexHash,
			sortedIndexSampleHash,
			sortedIndexSampleHash,
			sortedIndexSample
		);
	}

	static void recordTranslucentSortCopyRegistered(VulkanicGalBridge.WorldMeshSortedIndexRecord sortedIndex) {
		if (sortedIndex == null) {
			return;
		}
		LayerKey layerKey = null;
		TerrainSectionAsset asset = null;
		for (Map.Entry<LayerKey, TerrainSectionAsset> entry : SECTION_ASSETS.entrySet()) {
			if (entry.getKey().layer() == ChunkSectionLayer.TRANSLUCENT && entry.getValue().meshKey() == sortedIndex.meshKey()) {
				layerKey = entry.getKey();
				asset = entry.getValue();
				break;
			}
		}
		if (asset == null || layerKey == null) {
			return;
		}
		long hash = sortedIndexHash(sortedIndex.indexBytes());
		long sampleHash = sortedIndexSampleHash(sortedIndex.indexBytes());
		recordTranslucentSortPayloadEvent(
			layerKey.sectionPos(),
			0L,
			asset,
			asset.sectionOriginX(),
			asset.sectionOriginY(),
			asset.sectionOriginZ(),
			"translucent-rust-sort-copy",
			sortedIndex.indexGeneration(),
			currentCameraX(),
			currentCameraY(),
			currentCameraZ(),
			asset.sectionOriginX() + 8.0D,
			asset.sectionOriginY() + 8.0D,
			asset.sectionOriginZ() + 8.0D,
			Math.max(0, sortedIndex.indexBytes().length / (Integer.BYTES * 6)),
			hash,
			sortedIndex.indexGeneration(),
			"rust-route-cache",
			0L,
			hash,
			0L,
			sampleHash,
			sortedIndexSample(sortedIndex.indexBytes(), sortedIndex.indexType(), 12)
		);
	}

	private static void recordTranslucentSortPayloadEvent(
		long sectionPos,
		long sourceGeneration,
		TerrainSectionAsset asset,
		int sectionOriginX,
		int sectionOriginY,
		int sectionOriginZ,
		String reason,
		long sortGeneration,
		double cameraX,
		double cameraY,
		double cameraZ,
		double sortOriginX,
		double sortOriginY,
		double sortOriginZ,
		int primitiveCount,
		long sortedIndexHash,
		long indexUploadGeneration,
		String sorterType,
		long sourceSortedIndexHash,
		long rustCopiedSortedIndexHash,
		long sourceSortedIndexSampleHash,
		long rustCopiedSortedIndexSampleHash,
		String sortedIndexSample
	) {
		recordEvent(
			sectionPos,
			ChunkSectionLayer.TRANSLUCENT,
			sourceGeneration,
			asset.meshGeneration(),
			asset.meshGeneration(),
			atlasGeneration,
			asset,
			sectionOriginX,
			sectionOriginY,
			sectionOriginZ,
			0.0F,
			0.0F,
			0.0F,
			reason,
			0L,
			0L,
			0L,
			0L,
			sortGeneration,
			cameraX,
			cameraY,
			cameraZ,
			sortOriginX,
			sortOriginY,
			sortOriginZ,
			primitiveCount,
			sortedIndexHash,
			indexUploadGeneration,
			0,
			sorterType,
			sourceSortedIndexHash,
			rustCopiedSortedIndexHash,
			sourceSortedIndexSampleHash,
			rustCopiedSortedIndexSampleHash,
			sortedIndexSample
		);
	}

	public static void enqueueVisibleTerrain(SortedRenderLists renderLists, Camera camera, int viewportWidth, int viewportHeight) {
		if (!WorldRenderRoutePolicy.currentStaticTerrainRoute().usesRustWholeFrameVulkan() || renderLists == null || camera == null) {
			return;
		}
		Set<VisibleSubmitKey> visibleSubmissions = new HashSet<>();
		Iterator<ChunkRenderList> iterator = renderLists.iterator();
		while (iterator.hasNext()) {
			ChunkRenderList renderList = iterator.next();
			ByteIterator sectionIterator = renderList.sectionsWithGeometryIterator(false);
			if (sectionIterator == null) {
				continue;
			}
			while (sectionIterator.hasNext()) {
				RenderSection section = renderList.getRegion().getSection(sectionIterator.nextByteAsInt());
				if (section == null) {
					continue;
				}
				enqueueSectionLayer(section, ChunkSectionLayer.SOLID, camera, viewportWidth, viewportHeight, 0, visibleSubmissions);
				enqueueSectionLayer(section, ChunkSectionLayer.CUTOUT_MIPPED, camera, viewportWidth, viewportHeight, 0, visibleSubmissions);
			}
		}
		int translucentDrawOrder = 0;
		Iterator<ChunkRenderList> translucentIterator = renderLists.iterator();
		while (translucentIterator.hasNext()) {
			ChunkRenderList renderList = translucentIterator.next();
			ByteIterator sectionIterator = renderList.sectionsWithGeometryIterator(true);
			if (sectionIterator == null) {
				continue;
			}
			while (sectionIterator.hasNext()) {
				RenderSection section = renderList.getRegion().getSection(sectionIterator.nextByteAsInt());
				if (section == null) {
					continue;
				}
				enqueueSectionLayer(section, ChunkSectionLayer.TRANSLUCENT, camera, viewportWidth, viewportHeight, translucentDrawOrder++, visibleSubmissions);
			}
		}
	}

	/**
	 * Submits the explicitly owned whole-frame source's CPU-built sections.  It
	 * intentionally accepts sections rather than Sodium render lists: those
	 * lists are coupled to Java GL region storage and must never exist on this
	 * route.
	 */
	public static void enqueueWholeFrameTerrainSections(Iterable<RenderSection> sections, Camera camera,
			int viewportWidth, int viewportHeight) {
		if (!WorldRenderRoutePolicy.currentStaticTerrainRoute().usesRustWholeFrameVulkan()
			|| sections == null || camera == null) {
			return;
		}
		List<RenderSection> sectionSnapshot = snapshotBuiltTerrainSections(sections);
		// Mesh residency is intentionally longer lived than visibility. Publish
		// the complete semantic visibility replacement below so the primitive
		// frame cannot replay terrain that this frame's CPU source did not select.
		// This is derived only from the copied section/layer assets, never from a
		// Sodium render list or a backend handle.
		Set<Long> visibleMeshKeys = visibleWholeFrameMeshKeys(sectionSnapshot);
		Set<VisibleSubmitKey> visibleSubmissions = new HashSet<>();
		int translucentDrawOrder = 0;
		for (RenderSection section : sectionSnapshot) {
			enqueueSectionLayer(section, ChunkSectionLayer.SOLID, camera, viewportWidth, viewportHeight, 0, visibleSubmissions);
			enqueueSectionLayer(section, ChunkSectionLayer.CUTOUT_MIPPED, camera, viewportWidth, viewportHeight, 0, visibleSubmissions);
		}
		// Translucent sections are a single camera-sorted semantic stream. The
		// legacy Sodium render list is unavailable on the Rust whole-frame route,
		// so retain no list/GL state and order copied section centers explicitly.
		sectionSnapshot.sort(Comparator.comparingDouble((RenderSection section) -> {
			double dx = section.getOriginX() + 8.0D - camera.getPosition().x();
			double dy = section.getOriginY() + 8.0D - camera.getPosition().y();
			double dz = section.getOriginZ() + 8.0D - camera.getPosition().z();
			return dx * dx + dy * dy + dz * dz;
		}).reversed());
		for (RenderSection section : sectionSnapshot) {
			enqueueSectionLayer(section, ChunkSectionLayer.TRANSLUCENT, camera, viewportWidth, viewportHeight,
				translucentDrawOrder++, visibleSubmissions);
		}
		RustGalWorldPrimitiveRenderer.reconcileStaticTerrainVisibility(visibleMeshKeys);
		// Report the semantic terrain workload at the producer boundary. These
		// names are parity-family labels only: the work above is Rust-owned CPU
		// extraction and explicit mesh submission, never a Java Sodium draw.
		if (!sectionSnapshot.isEmpty()) {
			net.minecraft.client.dev.GraphicsFrameBenchmark.recordPhaseSample("sodium.terrain.setup", 1L);
			// The explicit producer owns three layer passes (solid, cutout, and
			// translucent) even when one pass has zero geometry. Record each semantic
			// pass separately, matching Frozen's producer-phase accounting without
			// pretending that an empty pass issued a backend draw.
			for (int layerPass = 0; layerPass < 3; layerPass++) {
				net.minecraft.client.dev.GraphicsFrameBenchmark.recordPhaseSample("sodium.terrain.draw", 1L);
			}
		}
		// The companion receipt is built from the same CPU-owned RenderSection
		// values that this semantic callsite submitted. It deliberately contains
		// no Sodium render list, GL object, or backend handle.
		net.sodium.client.render.StaticTerrainParityDiagnostics.recordRustWholeFrameEnqueueCoverage(
			sectionSnapshot,
			currentGameplayFrameId(),
			RustGalWholeFrameTerrainSource.isWholeFrameTerrainQueueDrained(),
			camera.getPosition().x(),
			camera.getPosition().y(),
			camera.getPosition().z(),
			viewportWidth,
			viewportHeight
		);
	}

	/**
	 * Resolves the exact asset identities addressed by a whole-frame semantic
	 * visibility set. Empty layers deliberately contribute nothing: they have
	 * no mesh instance to retain or execute.
	 */
	private static Set<Long> visibleWholeFrameMeshKeys(Iterable<RenderSection> sections) {
		Set<Long> visibleMeshKeys = new HashSet<>();
		for (RenderSection section : sections) {
			if (section == null) {
				continue;
			}
			long sectionPos = section.getPosition().asLong();
			for (ChunkSectionLayer layer : ChunkSectionLayer.values()) {
				TerrainSectionAsset asset = SECTION_ASSETS.get(new LayerKey(sectionPos, layer));
				if (asset != null) {
					visibleMeshKeys.add(asset.meshKey());
				}
			}
		}
		return visibleMeshKeys;
	}

	/**
	 * Copies the unique built section identities for the Rust semantic route.
	 * Upstream visibility iterables may contain repeated entries; canonical
	 * packed positions are the stable section identity and prevent duplicate
	 * mesh instances without retaining Sodium render-list state.
	 */
	static List<RenderSection> snapshotBuiltTerrainSections(Iterable<RenderSection> sections) {
		if (sections == null) {
			return List.of();
		}
		List<RenderSection> snapshot = new ArrayList<>();
		Set<Long> seenPositions = new HashSet<>();
		for (RenderSection section : sections) {
			if (section != null && section.isBuilt()
				&& seenPositions.add(section.getPosition().asLong())) {
				if (snapshot.size() >= MAX_SEMANTIC_TERRAIN_SECTIONS) {
					throw new IllegalStateException(
						"Rust whole-frame semantic terrain section bound exceeded "
							+ MAX_SEMANTIC_TERRAIN_SECTIONS
					);
				}
				snapshot.add(section);
			}
		}
		return snapshot;
	}

	public static void invalidateForResourceReload() {
		// Native upload acknowledgement permits Java to release opaque/cutout mesh
		// payloads. A reload therefore explicitly rebuilds the semantic source
		// rather than pretending an absent Java payload can recreate a Rust mesh.
		// Until the rebuild is acknowledged, this terrain family remains unavailable
		// instead of presenting stale resource-pack materials.
		RustGalWholeFrameTerrainSource.requestResourceReload();
		// The source rebuild is consumed lazily by the next whole-frame enqueue.
		// Retire the currently drawable identities here, not when that later enqueue
		// happens: their atlas UVs name the pre-reload atlas layout and can otherwise
		// sample unrelated sprites from the newly copied atlas for one or more frames.
		// `removeLayer` also retires the matching Rust-owned mesh resource, so this
		// remains an explicit semantic rebuild rather than retaining or borrowing a
		// Java renderer/GPU object across the resource generation boundary.
		for (LayerKey key : List.copyOf(SECTION_ASSETS.keySet())) {
			removeLayer(key.sectionPos(), key.layer(), "resource-reload");
		}
		TRANSLUCENT_EXECUTION_METADATA.clear();
		TEXTURE_PROBE_QUADS.clear();
		DistantHorizonsFaceMaterialResolver.clearCachedStateResolutions();
			synchronized (RustGalTerrainRenderer.class) {
				atlasPayload = null;
				atlasMipPayloads = List.of();
				atlasAnimationSource = null;
			copiedAtlasSemanticGeneration = 0L;
			copiedAtlasSemanticFrameKey = Long.MIN_VALUE;
			normalAtlasPayload = null;
			specularAtlasPayload = null;
			atlasGeneration++;
			registeredAtlasGeneration = 0L;
			publishedWorldMeshAtlasGeneration = 0L;
			publishedWorldMeshAtlasPayload = null;
		}
		invalidations.incrementAndGet();
			recordEvent(0L, ChunkSectionLayer.SOLID, 0L, 0L, 0L, atlasGeneration, null, 0, 0, 0, 0.0F, 0.0F, 0.0F, "resource-reload");
	}

	public static void invalidateForWorldUnload() {
		// DH snapshots are CPU-only semantic copies, but they are world-scoped.
		// Discard them with the existing terrain world epoch so no later Rust
		// route can accidentally observe data from a disconnected level.
		DistantHorizonsSemanticCollector.clear();
		TRANSLUCENT_EXECUTION_METADATA.clear();
		TEXTURE_PROBE_QUADS.clear();
		for (LayerKey key : List.copyOf(SECTION_ASSETS.keySet())) {
			removeLayer(key.sectionPos(), key.layer(), "world-unload");
		}
		invalidations.incrementAndGet();
			recordEvent(0L, ChunkSectionLayer.SOLID, 0L, 0L, 0L, atlasGeneration, null, 0, 0, 0, 0.0F, 0.0F, 0.0F, "world-unload");
	}

	public static void removeSection(int x, int y, int z, String reason) {
		long sectionPos = net.minecraft.core.SectionPos.asLong(x, y, z);
		removeLayer(sectionPos, ChunkSectionLayer.SOLID, reason);
		removeLayer(sectionPos, ChunkSectionLayer.CUTOUT_MIPPED, reason);
		removeLayer(sectionPos, ChunkSectionLayer.TRANSLUCENT, reason);
	}

	/**
	 * Rust owns opaque/cutout geometry after its explicit asset update is
	 * accepted. Keep the section's generation and draw metadata for the visible
	 * semantic callsite, but discard Java's duplicate vertex/index payload. A
	 * translucent section is deliberately excluded because its copied CPU index
	 * stream is required for the explicit camera-relative sort producer.
	 */
	public static void releaseUploadedStaticTerrainPayload(long meshKey, long meshGeneration) {
		for (Map.Entry<LayerKey, TerrainSectionAsset> entry : SECTION_ASSETS.entrySet()) {
			TerrainSectionAsset asset = entry.getValue();
			if (asset.meshKey() != meshKey || asset.meshGeneration() != meshGeneration
				|| entry.getKey().layer() == ChunkSectionLayer.TRANSLUCENT || asset.asset() == null) {
				continue;
			}
			SECTION_ASSETS.replace(entry.getKey(), asset, asset.releaseCpuPayload());
			return;
		}
	}

	public static TerrainDiagnostics diagnosticsSnapshot() {
		synchronized (RECENT_EVENTS) {
			long currentFrameId = currentGameplayFrameId();
			long currentVisibleSubmissions =
				lastVisibleSubmissionFrameId.get() == currentFrameId ? currentFrameVisibleLayerSubmissions.get() : 0L;
			return new TerrainDiagnostics(
				SECTION_ASSETS.size(),
				SECTION_ASSETS.size(),
				SECTION_ASSETS.size(),
				currentVisibleSubmissions,
				atlasGeneration,
				registeredAtlasGeneration,
				activeTerrainVertexStride(),
				activeTerrainVertexStride(),
				acceptedBuildOutputs.get(),
				skippedRouteBuildOutputs.get(),
				skippedUnsupportedAnimatedSections.get(),
				skippedUnsupportedFluidTranslucentSections.get(),
				acceptedWaterAnimatedSections.get(),
				unsupportedFluidRejectedSections.get(),
				skippedEmptyLayers.get(),
				registeredMeshes.get(),
				registeredTranslucentSorts.get(),
				registeredTranslucentSortBytes.get(),
				translucentSortGenerations.get(),
				texturePayloadUpdates.get(),
				texturePayloadUpdateBytes.get(),
				atlasTextureOnlyUpdates.get(),
				atlasMissingPayloadUpdates.get(),
				atlasMalformedPayloadUpdates.get(),
				atlasPartialPayloadUpdates.get(),
				removedLayers.get(),
				visibleLayerProbes.get(),
				visibleLayerSubmissions.get(),
				failedLayerSubmissions.get(),
				invalidations.get(),
				terrainExtractionFrames.get(),
				rustEnqueueFrames.get(),
				List.copyOf(LIFECYCLE_EVENTS),
				List.copyOf(TRANSLUCENT_EVENTS),
				List.copyOf(RECENT_EVENTS)
			);
		}
	}

	/**
	 * Completed whole-frame evidence for capture gates. This intentionally tracks
	 * static terrain separately from Distant Horizons so a distant-only frame
	 * cannot validate a static-terrain screenshot.
	 */
	public static StaticTerrainExecutionSnapshot staticTerrainExecutionSnapshot() {
		return new StaticTerrainExecutionSnapshot(
			lastExecutedStaticTerrainFrameId.get(),
			lastExecutedStaticTerrainSubmissionId.get(),
			lastExecutedStaticTerrainInstances.get()
		);
	}

	/**
	 * Capture-only route evidence for a specific client section. Client chunk
	 * residency outlives normal render-radius visibility, so deterministic DH
	 * scenarios must consult the actual completed static-terrain submission
	 * rather than {@code ClientLevel#isLoaded} when excluding the near route.
	 */
	public static boolean staticTerrainSectionExecutedInLastCompletedFrame(BlockPos blockPos) {
		if (blockPos == null) {
			return false;
		}
		long executionFrame = lastExecutedStaticTerrainFrameId.get();
		if (executionFrame <= 0L) {
			return false;
		}
		long sectionPos = net.minecraft.core.SectionPos.asLong(
			net.minecraft.core.SectionPos.blockToSectionCoord(blockPos.getX()),
			net.minecraft.core.SectionPos.blockToSectionCoord(blockPos.getY()),
			net.minecraft.core.SectionPos.blockToSectionCoord(blockPos.getZ())
		);
		synchronized (RECENT_EVENTS) {
			return RECENT_EVENTS.stream().anyMatch(event -> event.sectionPos() == sectionPos
				&& event.executionFrameId() == executionFrame
				&& "executed-submit".equals(event.reason()));
		}
	}

	/**
	 * Test-only semantic receipt for the copied terrain atlas. It deliberately
	 * compares source sprite pixels with the corresponding copied atlas rectangle
	 * instead of observing a GL/Vulkan texture or backend binding.
	 */
	public static TerrainAtlasReceipt terrainAtlasReceipt() {
		try {
			ensureAtlasPayload();
			byte[] payload = atlasPayload;
			if (payload == null) {
				return TerrainAtlasReceipt.unavailable("atlas payload is unavailable");
			}
			BufferedImage copiedAtlas = ImageIO.read(new java.io.ByteArrayInputStream(payload));
			if (copiedAtlas == null) {
				return TerrainAtlasReceipt.unavailable("atlas payload did not decode as an image");
			}
			TextureAtlas atlas = Minecraft.getInstance().getAtlasManager().getAtlasOrThrow(AtlasIds.BLOCKS);
			List<TerrainAtlasSpriteReceipt> sprites = new ArrayList<>();
			for (String path : List.of(
				"block/grass_block_top",
				"block/grass_block_side",
				"block/redstone_ore",
				"block/yellow_terracotta",
				"block/oak_leaves"
			)) {
				ResourceLocation location = ResourceLocation.fromNamespaceAndPath("minecraft", path);
				TextureAtlasSprite sprite = atlas.getSprite(location);
				if (sprite == null || sprite.contents() == null) {
					sprites.add(TerrainAtlasSpriteReceipt.missing(location.toString()));
					continue;
				}
				int width = sprite.contents().width();
				int height = sprite.contents().height();
				long sourceHash = rgbaHash(sprite.contents().originalImage, 0, 0, width, height);
				long copiedHash = rgbaHash(copiedAtlas, sprite.getX(), sprite.getY(), width, height);
				int sampleX = sprite.getX() + width / 2;
				int sampleY = sprite.getY() + height / 2;
				int mirroredSampleY = copiedAtlas.getHeight() - 1 - sampleY;
				sprites.add(new TerrainAtlasSpriteReceipt(
					location.toString(),
					sprite.getX(),
					sprite.getY(),
					width,
					height,
					sourceHash,
					copiedHash,
					atlasSpriteIdentityAt(atlas, sampleX, sampleY),
					atlasSpriteIdentityAt(atlas, sampleX, mirroredSampleY),
					sampleX,
					sampleY,
					mirroredSampleY,
					sourceHash == copiedHash,
					"ok"
				));
			}
			return new TerrainAtlasReceipt(
				true,
				"ok",
				copiedAtlas.getWidth(),
				copiedAtlas.getHeight(),
				rgbaHash(copiedAtlas, 0, 0, copiedAtlas.getWidth(), copiedAtlas.getHeight()),
				List.copyOf(sprites)
			);
		} catch (RuntimeException | IOException error) {
			return TerrainAtlasReceipt.unavailable(error.getMessage());
		}
	}

	private static void cacheTextureProbeQuads(ChunkBuildOutput output, TerrainSectionAsset asset) {
		if (activeTextureProbes.isEmpty() || asset.asset() == null) {
			return;
		}
		List<VulkanicGalBridge.WorldMeshVertexRecord> vertices = asset.asset().vertices();
		for (TerrainTextureProbe probe : activeTextureProbes) {
			if (TEXTURE_PROBE_QUADS.containsKey(probe.position())) continue;
			for (int first = 0; first + 3 < vertices.size(); first += 4) {
				if (quadBelongsToBlock(vertices, first, asset, probe.position())) {
					TEXTURE_PROBE_QUADS.putIfAbsent(probe.position(), List.copyOf(vertices.subList(first, first + 4)));
					break;
				}
			}
		}
	}

	private static String atlasSpriteIdentityAt(TextureAtlas atlas, int sampleX, int sampleY) {
		for (Map.Entry<ResourceLocation, TextureAtlasSprite> entry : atlas.texturesByName.entrySet()) {
			TextureAtlasSprite candidate = entry.getValue();
			if (candidate == null || candidate.contents() == null) {
				continue;
			}
			int minX = candidate.getX();
			int minY = candidate.getY();
			int maxX = minX + candidate.contents().width();
			int maxY = minY + candidate.contents().height();
			if (sampleX >= minX && sampleX < maxX && sampleY >= minY && sampleY < maxY) {
				return entry.getKey().toString();
			}
		}
		return "<atlas-padding-or-unassigned>";
	}

	/**
	 * Capture-only proof that a copied static-terrain quad still addresses the
	 * atlas region of the block that owns it. This observes CPU semantic mesh
	 * records before FFI; it never reads a backend texture or changes rendering.
	 */
	public static TerrainTextureProbeReceipt terrainTextureProbeReceipt(List<TerrainTextureProbe> probes) {
		if (probes == null || probes.isEmpty()) {
			return new TerrainTextureProbeReceipt(false, "no texture probes", List.of());
		}
		try {
			ensureAtlasPayload();
			TextureAtlas atlas = Minecraft.getInstance().getAtlasManager().getAtlasOrThrow(AtlasIds.BLOCKS);
			List<TerrainTextureProbeResult> results = new ArrayList<>(probes.size());
			for (TerrainTextureProbe probe : probes) {
				if (probe == null || probe.position() == null || probe.allowedSprites().isEmpty()) {
					return new TerrainTextureProbeReceipt(false, "invalid texture probe", List.copyOf(results));
				}
				List<AtlasUvRegion> allowedRegions = new ArrayList<>(probe.allowedSprites().size());
				for (ResourceLocation identity : probe.allowedSprites()) {
					TextureAtlasSprite sprite = atlas.getSprite(identity);
					if (sprite == null || sprite.contents() == null) {
						results.add(new TerrainTextureProbeResult(
							probe.position(), probe.allowedSprites(), 0, 0, false,
							"missing expected sprite " + identity, List.of()
						));
						allowedRegions.clear();
						break;
					}
					allowedRegions.add(new AtlasUvRegion(sprite.getU0(), sprite.getV0(), sprite.getU1(), sprite.getV1()));
				}
				if (allowedRegions.isEmpty()) {
					continue;
				}
				int matchingQuads = 0;
				int mismatchedQuads = 0;
				List<TerrainTextureProbeObservation> observations = new ArrayList<>();
				for (Map.Entry<LayerKey, TerrainSectionAsset> entry : SECTION_ASSETS.entrySet()) {
					if (entry.getKey().layer() != ChunkSectionLayer.SOLID
						&& entry.getKey().layer() != ChunkSectionLayer.CUTOUT
						&& entry.getKey().layer() != ChunkSectionLayer.CUTOUT_MIPPED) {
						continue;
					}
					TerrainSectionAsset asset = entry.getValue();
					if (asset.asset() == null) {
						// The explicit Rust upload owns this opaque/cutout payload now.
						// Probe diagnostics must report only source bytes that remain
						// resident rather than silently recreating a Java mesh copy.
						continue;
					}
					List<VulkanicGalBridge.WorldMeshVertexRecord> vertices = asset.asset().vertices();
					for (int firstVertex = 0; firstVertex + 3 < vertices.size(); firstVertex += 4) {
						if (!quadBelongsToBlock(vertices, firstVertex, asset, probe.position())) {
							continue;
						}
						matchingQuads++;
						boolean uvMatches = quadUsesAnyAtlasRegion(vertices, firstVertex, allowedRegions);
						if (!uvMatches) {
							mismatchedQuads++;
						}
						if (observations.size() < 12) {
							observations.add(new TerrainTextureProbeObservation(
								entry.getKey().sectionPos(),
								entry.getKey().layer().name(),
								firstVertex / 4,
								uvMatches,
								atlasIdentityForQuad(atlas, vertices, firstVertex),
								vertices.get(firstVertex).atlasU(),
								vertices.get(firstVertex).atlasV()
							));
						}
					}
				}
				// Opaque/cutout payloads are released after explicit Rust upload. Use
				// the bounded semantic samples captured at admission for diagnostics.
				if (matchingQuads == 0) {
					List<VulkanicGalBridge.WorldMeshVertexRecord> cached = TEXTURE_PROBE_QUADS.get(probe.position());
					if (cached != null && cached.size() >= 4) {
						matchingQuads = 1;
						if (!quadUsesAnyAtlasRegion(cached, 0, allowedRegions)) {
							mismatchedQuads = 1;
						}
						observations.add(new TerrainTextureProbeObservation(
							0L, "semantic-probe-cache", 0, mismatchedQuads == 0,
							atlasIdentityForQuad(atlas, cached, 0), cached.get(0).atlasU(), cached.get(0).atlasV()
						));
					}
				}
				boolean matched = matchingQuads > 0 && mismatchedQuads == 0;
				results.add(new TerrainTextureProbeResult(
					probe.position(), probe.allowedSprites(), matchingQuads, mismatchedQuads, matched,
					matched ? "ok" : matchingQuads == 0 ? "no copied terrain quad for probe" : "atlas UV outside expected sprite",
					List.copyOf(observations)
				));
			}
			boolean matched = results.stream().allMatch(TerrainTextureProbeResult::matched);
			String status = matched
				? "ok"
				: results.stream()
					.filter(result -> !result.matched())
					.map(result -> result.position().toShortString() + ":" + result.status())
					.reduce((left, right) -> left + ";" + right)
					.orElse("texture probe mismatch");
			return new TerrainTextureProbeReceipt(matched, status, List.copyOf(results));
		} catch (RuntimeException error) {
			return new TerrainTextureProbeReceipt(false, error.getMessage(), List.of());
		}
	}

	public static void setActiveTextureProbes(List<TerrainTextureProbe> probes) {
		activeTextureProbes = probes == null ? List.of() : List.copyOf(probes);
		TEXTURE_PROBE_QUADS.clear();
	}

	private static String atlasIdentityForQuad(
		TextureAtlas atlas,
		List<VulkanicGalBridge.WorldMeshVertexRecord> vertices,
		int firstVertex
	) {
		for (Map.Entry<ResourceLocation, TextureAtlasSprite> entry : atlas.texturesByName.entrySet()) {
			TextureAtlasSprite sprite = entry.getValue();
			if (sprite != null && quadUsesAnyAtlasRegion(vertices, firstVertex, List.of(
				new AtlasUvRegion(sprite.getU0(), sprite.getV0(), sprite.getU1(), sprite.getV1())
			))) {
				return entry.getKey().toString();
			}
		}
		return "unresolved";
	}

	private static boolean quadBelongsToBlock(
		List<VulkanicGalBridge.WorldMeshVertexRecord> vertices,
		int firstVertex,
		TerrainSectionAsset asset,
		BlockPos position
	) {
		final float epsilon = 0.0001F;
		float minX = position.getX() - epsilon;
		float minY = position.getY() - epsilon;
		float minZ = position.getZ() - epsilon;
		float maxX = position.getX() + 1.0F + epsilon;
		float maxY = position.getY() + 1.0F + epsilon;
		float maxZ = position.getZ() + 1.0F + epsilon;
		boolean touchesBoundary = false;
		boolean allAtMinX = true;
		boolean allAtMaxX = true;
		boolean allAtMinY = true;
		boolean allAtMaxY = true;
		boolean allAtMinZ = true;
		boolean allAtMaxZ = true;
		for (int index = firstVertex; index < firstVertex + 4; index++) {
			VulkanicGalBridge.WorldMeshVertexRecord vertex = vertices.get(index);
			float x = asset.sectionOriginX() + vertex.x();
			float y = asset.sectionOriginY() + vertex.y();
			float z = asset.sectionOriginZ() + vertex.z();
			if (x < minX || x > maxX || y < minY || y > maxY || z < minZ || z > maxZ) {
				return false;
			}
			touchesBoundary |= Math.abs(x - position.getX()) <= epsilon || Math.abs(x - (position.getX() + 1.0F)) <= epsilon
				|| Math.abs(y - position.getY()) <= epsilon || Math.abs(y - (position.getY() + 1.0F)) <= epsilon
				|| Math.abs(z - position.getZ()) <= epsilon || Math.abs(z - (position.getZ() + 1.0F)) <= epsilon;
			allAtMinX &= Math.abs(x - position.getX()) <= epsilon;
			allAtMaxX &= Math.abs(x - (position.getX() + 1.0F)) <= epsilon;
			allAtMinY &= Math.abs(y - position.getY()) <= epsilon;
			allAtMaxY &= Math.abs(y - (position.getY() + 1.0F)) <= epsilon;
			allAtMinZ &= Math.abs(z - position.getZ()) <= epsilon;
			allAtMaxZ &= Math.abs(z - (position.getZ() + 1.0F)) <= epsilon;
		}
		if (!touchesBoundary) {
			return false;
		}
		VulkanicGalBridge.WorldMeshVertexRecord firstVertexRecord = vertices.get(firstVertex);
		float normalX = unpackPackedNormalComponent(firstVertexRecord.normalPacked(), 0);
		float normalY = unpackPackedNormalComponent(firstVertexRecord.normalPacked(), 8);
		float normalZ = unpackPackedNormalComponent(firstVertexRecord.normalPacked(), 16);
		return !(allAtMinX && normalX > epsilon)
			&& !(allAtMaxX && normalX < -epsilon)
			&& !(allAtMinY && normalY > epsilon)
			&& !(allAtMaxY && normalY < -epsilon)
			&& !(allAtMinZ && normalZ > epsilon)
			&& !(allAtMaxZ && normalZ < -epsilon);
	}

	private static boolean quadUsesAnyAtlasRegion(
		List<VulkanicGalBridge.WorldMeshVertexRecord> vertices,
		int firstVertex,
		List<AtlasUvRegion> allowedRegions
	) {
		for (AtlasUvRegion region : allowedRegions) {
			boolean everyVertexMatches = true;
			for (int index = firstVertex; index < firstVertex + 4; index++) {
				VulkanicGalBridge.WorldMeshVertexRecord vertex = vertices.get(index);
				if (!region.contains(vertex.atlasU(), vertex.atlasV())) {
					everyVertexMatches = false;
					break;
				}
			}
			if (everyVertexMatches) {
				return true;
			}
		}
		return false;
	}

	public static BlockPos chooseLifecycleEditTarget(String scenario) {
		String normalized = scenario == null ? "" : scenario.trim().toLowerCase(Locale.ROOT);
		synchronized (RECENT_EVENTS) {
			BlockPos fallback = null;
			Iterator<TerrainDiagnosticEvent> iterator = RECENT_EVENTS.descendingIterator();
			while (iterator.hasNext()) {
				TerrainDiagnosticEvent event = iterator.next();
				if ("visible-submit".equals(event.reason())
					&& "SOLID".equals(event.layer())
					&& event.sectionOriginValid()
					&& event.vertexCount() > 0
					&& event.localBoundsValid()) {
					int localX = chooseLifecycleLocalCoordinate(event.localMinX(), event.localMaxX());
					int localY = chooseLifecycleLocalCoordinate(event.localMinY(), event.localMaxY());
					int localZ = chooseLifecycleLocalCoordinate(event.localMinZ(), event.localMaxZ());
					BlockPos candidate = new BlockPos(
						event.sectionOriginX() + localX,
						event.sectionOriginY() + localY,
						event.sectionOriginZ() + localZ
					);
					if (fallback == null) {
						fallback = candidate;
					}
					if (switch (normalized) {
						case "boundary-x-edit" -> localX == 15;
						case "boundary-y-edit" -> localY == 15;
						case "boundary-z-edit" -> localZ == 15;
						default -> true;
					}) {
						return candidate;
					}
				}
			}
			return fallback;
		}
	}

	private static int chooseLifecycleLocalCoordinate(float minInclusive, float maxInclusive) {
		if (!Float.isFinite(minInclusive) || !Float.isFinite(maxInclusive)) {
			return 8;
		}
		int lower = Math.max(0, Math.min(15, (int)Math.floor(Math.min(minInclusive, maxInclusive))));
		int upperExclusive = Math.max(1, Math.min(16, (int)Math.ceil(Math.max(minInclusive, maxInclusive))));
		if (upperExclusive <= lower) {
			lower = Math.max(0, Math.min(15, upperExclusive - 1));
			upperExclusive = Math.max(lower + 1, upperExclusive);
		}
		return lower + ((upperExclusive - lower - 1) / 2);
	}

	public static TerrainLayerSnapshot snapshotLayer(BlockPos blockPos, ChunkSectionLayer layer) {
		if (blockPos == null || layer == null) {
			return null;
		}
		long sectionPos = net.minecraft.core.SectionPos.asLong(
			net.minecraft.core.SectionPos.blockToSectionCoord(blockPos.getX()),
			net.minecraft.core.SectionPos.blockToSectionCoord(blockPos.getY()),
			net.minecraft.core.SectionPos.blockToSectionCoord(blockPos.getZ())
		);
		TerrainSectionAsset asset = SECTION_ASSETS.get(new LayerKey(sectionPos, layer));
		if (asset == null) {
			return new TerrainLayerSnapshot(sectionPos, layer.name(), 0L, 0L, 0, 0, 0);
		}
		return new TerrainLayerSnapshot(
			sectionPos,
			layer.name(),
			asset.meshKey(),
			asset.meshGeneration(),
			asset.vertexCount(),
			asset.indexCount() * 2,
			asset.sectionCount()
		);
	}

	public static void recordLifecycleMarker(String reason, BlockPos blockPos, ChunkSectionLayer layer, String detail) {
		if (blockPos == null || layer == null) {
			return;
		}
		long sectionPos = net.minecraft.core.SectionPos.asLong(
			net.minecraft.core.SectionPos.blockToSectionCoord(blockPos.getX()),
			net.minecraft.core.SectionPos.blockToSectionCoord(blockPos.getY()),
			net.minecraft.core.SectionPos.blockToSectionCoord(blockPos.getZ())
		);
		TerrainSectionAsset asset = SECTION_ASSETS.get(new LayerKey(sectionPos, layer));
		int originX = net.minecraft.core.SectionPos.sectionToBlockCoord(net.minecraft.core.SectionPos.blockToSectionCoord(blockPos.getX()));
		int originY = net.minecraft.core.SectionPos.sectionToBlockCoord(net.minecraft.core.SectionPos.blockToSectionCoord(blockPos.getY()));
		int originZ = net.minecraft.core.SectionPos.sectionToBlockCoord(net.minecraft.core.SectionPos.blockToSectionCoord(blockPos.getZ()));
		recordEvent(
			sectionPos,
			layer,
			0L,
			asset == null ? 0L : asset.meshGeneration(),
			asset == null ? 0L : asset.meshGeneration(),
			atlasGeneration,
			asset,
			asset == null ? originX : asset.sectionOriginX(),
			asset == null ? originY : asset.sectionOriginY(),
			asset == null ? originZ : asset.sectionOriginZ(),
			0.0F,
			0.0F,
			0.0F,
			(detail == null || detail.isBlank()) ? reason : reason + ":" + detail
		);
	}

	public static void recordTranslucentFaultMarker(String fault, BlockPos blockPos, String detail) {
		if (fault == null || fault.isBlank() || blockPos == null) {
			return;
		}
		long sectionPos = net.minecraft.core.SectionPos.asLong(
			net.minecraft.core.SectionPos.blockToSectionCoord(blockPos.getX()),
			net.minecraft.core.SectionPos.blockToSectionCoord(blockPos.getY()),
			net.minecraft.core.SectionPos.blockToSectionCoord(blockPos.getZ())
		);
		TerrainSectionAsset asset = SECTION_ASSETS.get(new LayerKey(sectionPos, ChunkSectionLayer.TRANSLUCENT));
		int originX = net.minecraft.core.SectionPos.sectionToBlockCoord(net.minecraft.core.SectionPos.blockToSectionCoord(blockPos.getX()));
		int originY = net.minecraft.core.SectionPos.sectionToBlockCoord(net.minecraft.core.SectionPos.blockToSectionCoord(blockPos.getY()));
		int originZ = net.minecraft.core.SectionPos.sectionToBlockCoord(net.minecraft.core.SectionPos.blockToSectionCoord(blockPos.getZ()));
		String reason = "translucent-fault-" + fault.trim().toLowerCase(Locale.ROOT);
		if (detail != null && !detail.isBlank()) {
			reason += ":" + detail;
		}
		recordEvent(
			sectionPos,
			ChunkSectionLayer.TRANSLUCENT,
			0L,
			asset == null ? 0L : asset.meshGeneration(),
			asset == null ? 0L : asset.meshGeneration(),
			atlasGeneration,
			asset,
			asset == null ? originX : asset.sectionOriginX(),
			asset == null ? originY : asset.sectionOriginY(),
			asset == null ? originZ : asset.sectionOriginZ(),
			0.0F,
			0.0F,
			0.0F,
			reason,
			0L,
			0L,
			0L,
			0L,
			asset == null ? 0L : asset.meshGeneration(),
			currentCameraX(),
			currentCameraY(),
			currentCameraZ(),
			asset == null ? originX + 8.0D : asset.sectionOriginX() + 8.0D,
			asset == null ? originY + 8.0D : asset.sectionOriginY() + 8.0D,
			asset == null ? originZ + 8.0D : asset.sectionOriginZ() + 8.0D,
			asset == null ? 0 : Math.max(0, asset.indexCount() / 6),
			asset == null ? 0L : asset.contentHash(),
			asset == null ? 0L : asset.meshGeneration(),
			0
		);
	}

	public static void injectAtlasTexturePayloadForDiagnostics(byte[] payload, String reason) {
		if (!WorldRenderRoutePolicy.currentStaticTerrainRoute().usesRustWholeFrameVulkan()) {
			return;
		}
		byte[] copied = payload == null ? new byte[0] : payload.clone();
		long generation;
		synchronized (RustGalTerrainRenderer.class) {
			atlasPayload = copied;
			atlasMipPayloads = List.of();
			atlasAnimationSource = null;
			atlasGeneration++;
			registeredAtlasGeneration = atlasGeneration;
			generation = atlasGeneration;
		}
		atlasTextureOnlyUpdates.incrementAndGet();
		if (copied.length == 0) {
			atlasMissingPayloadUpdates.incrementAndGet();
		} else if (!isPngPayload(copied)) {
			atlasMalformedPayloadUpdates.incrementAndGet();
		} else if (copied.length < 256) {
			atlasPartialPayloadUpdates.incrementAndGet();
		}
		texturePayloadUpdates.incrementAndGet();
		texturePayloadUpdateBytes.addAndGet(copied.length);
		RustGalWorldPrimitiveRenderer.registerStaticTerrainAtlasTexture(
			new VulkanicGalBridge.WorldMeshTextureAssetRecord(
				RustGalWorldPrimitiveRenderer.MATERIAL_TEXTURE_TERRAIN_BLOCK_ATLAS,
				copied
			)
		);
		recordEvent(
			0L,
			ChunkSectionLayer.SOLID,
			0L,
			0L,
			0L,
			generation,
			null,
			0,
			0,
			0,
			0.0F,
			0.0F,
			0.0F,
			(reason == null || reason.isBlank()) ? "atlas-texture-only-update" : reason
		);
	}

	/**
	 * Publishes the copied Minecraft block atlas as a Rust-owned world-mesh
	 * resource even when the first visible consumer is Distant Horizons. DH
	 * exact-atlas draws must not depend on an unrelated near-terrain section
	 * happening to build first.
	 */
	public static void ensureTerrainAtlasAssetForWorldMesh() {
		// Semantic-only tests deliberately exercise routing without a live client.
		// Asset publication remains deferred until the frame coordinator flushes a
		// real client-owned atlas before native submission.
		if (Minecraft.getInstance() == null) {
			return;
		}
		ensureAtlasPayload();
		byte[] payload;
		List<byte[]> mipPayloads;
		AtlasAnimationResource animation;
		long generation;
		synchronized (RustGalTerrainRenderer.class) {
			if (atlasPayload == null || publishedWorldMeshAtlasGeneration == atlasGeneration) {
				return;
			}
			payload = atlasPayload;
			mipPayloads = atlasMipPayloads;
			animation = atlasAnimationSource;
			generation = atlasGeneration;
		}
		texturePayloadUpdates.incrementAndGet();
		texturePayloadUpdateBytes.addAndGet(atlasPayloadByteCount(payload, mipPayloads));
		var texture = new VulkanicGalBridge.WorldMeshTextureAssetRecord(
				RustGalWorldPrimitiveRenderer.MATERIAL_TEXTURE_TERRAIN_BLOCK_ATLAS,
				payload,
				mipPayloads
			);
		if (animation != null) {
			RustGalWorldPrimitiveRenderer.registerTerrainAtlasAnimation(texture, animation);
		} else {
			RustGalWorldPrimitiveRenderer.registerWorldMeshTexture(texture, "terrain-atlas");
		}
		var registeredTexture = RustGalWorldPrimitiveRenderer.requireRegisteredWorldMeshTexturePayload(texture);
		synchronized (RustGalTerrainRenderer.class) {
			// Publication failure must leave this incarnation retryable.
			if (atlasGeneration == generation) {
				publishedWorldMeshAtlasGeneration = generation;
				publishedWorldMeshAtlasPayload = registeredTexture;
			}
		}
	}

	/** Semantic region in the same copied atlas used by terrain; no per-item image copy. */
	public static AtlasSpritePayload requireGuiAtlasSpritePayload(TextureAtlasSprite sprite) {
		TextureAtlas atlas = Minecraft.getInstance().getAtlasManager().getAtlasOrThrow(AtlasIds.BLOCKS);
		if (sprite == null || atlas.getSprite(sprite.contents().name()) != sprite) {
			throw new IllegalStateException("GUI sprite does not belong to the current block atlas");
		}
		ensureTerrainAtlasAssetForWorldMesh();
		synchronized (RustGalTerrainRenderer.class) {
			if (publishedWorldMeshAtlasPayload == null || publishedWorldMeshAtlasGeneration != atlasGeneration
				|| copiedAtlasSemanticGeneration != atlas.semanticReloadGeneration()
				|| copiedAtlasSemanticFrameKey != atlas.semanticSnapshotFrameKey()
				|| atlas.getSprite(sprite.contents().name()) != sprite) {
				throw new IllegalStateException("GUI atlas changed during semantic collection");
			}
			return new AtlasSpritePayload(publishedWorldMeshAtlasPayload, atlas.width, atlas.height,
				sprite.getX(), sprite.getY(), sprite.contents().width(), sprite.contents().height());
		}
	}

	/**
	 * The direct CPU source may finish meshing before its copied assets have
	 * crossed the explicit VulkanicGAL upload boundary. A settled whole-frame
	 * capture is valid only once that bounded queue has drained; otherwise the
	 * backend would correctly omit unknown mesh resources from its draw plan.
	 */
	public static boolean areWholeFrameAssetsUploaded() {
		RustGalWorldPrimitiveRenderer.WorldMeshAssetMetrics metrics =
			RustGalWorldPrimitiveRenderer.worldMeshAssetMetrics();
		return metrics.failures() == 0L
			&& metrics.dirtyMeshes() == 0
			&& metrics.dirtyTextures() == 0
			&& metrics.pendingInstances() == 0
			&& metrics.uploadedMeshes() >= metrics.cachedMeshes()
			&& metrics.uploadedTextures() >= metrics.cachedTextures();
	}

	/** Capture diagnostic companion to {@link #areWholeFrameAssetsUploaded()}. */
	public static String wholeFrameAssetUploadSummary() {
		RustGalWorldPrimitiveRenderer.WorldMeshAssetMetrics metrics =
			RustGalWorldPrimitiveRenderer.worldMeshAssetMetrics();
		return "cachedMeshes=" + metrics.cachedMeshes()
			+ ",uploadedMeshes=" + metrics.uploadedMeshes()
			+ ",dirtyMeshes=" + metrics.dirtyMeshes()
			+ ",cachedTextures=" + metrics.cachedTextures()
			+ ",uploadedTextures=" + metrics.uploadedTextures()
			+ ",dirtyTextures=" + metrics.dirtyTextures()
			+ ",pendingInstances=" + metrics.pendingInstances()
			+ ",failures=" + metrics.failures()
			+ ",ready=" + areWholeFrameAssetsUploaded();
	}

	public static void recordExecutedStaticTerrainInstances(
		List<VulkanicGalBridge.WorldMeshInstanceRecord> instances,
		long frameId,
		long submissionId
	) {
		if (instances == null || instances.isEmpty()) {
			return;
		}
		long executedStaticTerrainInstances = 0L;
		List<net.sodium.client.render.StaticTerrainParityDiagnostics.RustExecutionIdentity> executionReceipt =
			new ArrayList<>();
		for (VulkanicGalBridge.WorldMeshInstanceRecord instance : instances) {
			TerrainSectionAsset asset = null;
			LayerKey layerKey = null;
			for (Map.Entry<LayerKey, TerrainSectionAsset> entry : SECTION_ASSETS.entrySet()) {
				TerrainSectionAsset candidate = entry.getValue();
				if (candidate.meshKey() == instance.meshKey()) {
					asset = candidate;
					layerKey = entry.getKey();
					break;
				}
			}
			if (asset == null || layerKey == null) {
				continue;
			}
			executedStaticTerrainInstances++;
		TranslucentExecutionMetadata submittedMetadata =
				layerKey.layer() == ChunkSectionLayer.TRANSLUCENT
					? takeTranslucentExecutionMetadata(instance.meshKey())
					: null;
			TranslucentSortSnapshot sortedIndex =
				layerKey.layer() == ChunkSectionLayer.TRANSLUCENT ? currentTranslucentSortSnapshot(asset) : null;
		// Execution is recorded after command generation. Report the sorted
		// payload currently owned by Rust at that point; an older queued receipt
		// must not masquerade as the active draw after a camera resort.
		long sortGeneration = sortedIndex == null ? 0L : sortedIndex.sortGeneration();
		long sortedIndexHash = sortedIndex == null ? 0L : sortedIndex.indexHash();
		double cameraX = layerKey.layer() == ChunkSectionLayer.TRANSLUCENT ? currentCameraX() : 0.0D;
		double cameraY = layerKey.layer() == ChunkSectionLayer.TRANSLUCENT ? currentCameraY() : 0.0D;
		double cameraZ = layerKey.layer() == ChunkSectionLayer.TRANSLUCENT ? currentCameraZ() : 0.0D;
			int drawOrder = submittedMetadata != null ? submittedMetadata.drawOrder() : 0;
			net.sodium.client.render.StaticTerrainParityDiagnostics.recordRustStaticTerrainExecution(
				"rust-vulkan-executed",
				layerKey.sectionPos(),
				layerKey.layer().name(),
				asset.meshKey(),
				asset.meshGeneration(),
				instance.meshGeneration(),
				asset.contentHash(),
				asset.vertexCount(),
				asset.indexCount(),
				asset.indexType(),
				Math.max(0, asset.indexCount() / 6),
				asset.sectionCount(),
				asset.sectionOriginX(),
				asset.sectionOriginY(),
				asset.sectionOriginZ(),
				asset.localMinX(),
				asset.localMinY(),
				asset.localMinZ(),
				asset.localMaxX(),
				asset.localMaxY(),
				asset.localMaxZ(),
				asset.uvMinU(),
				asset.uvMinV(),
				asset.uvMaxU(),
				asset.uvMaxV(),
				frameId,
				0L
			);
			executionReceipt.add(new net.sodium.client.render.StaticTerrainParityDiagnostics.RustExecutionIdentity(
				layerKey.sectionPos(), layerKey.layer().name(), instance.meshGeneration()
			));
			recordEvent(
				layerKey.sectionPos(),
				layerKey.layer(),
				0L,
				asset.meshGeneration(),
				instance.meshGeneration(),
				atlasGeneration,
				asset,
				asset.sectionOriginX(),
				asset.sectionOriginY(),
				asset.sectionOriginZ(),
				instance.transform()[12],
				instance.transform()[13],
				instance.transform()[14],
				"executed-submit",
				0L,
				0L,
				frameId,
				submissionId,
				layerKey.layer() == ChunkSectionLayer.TRANSLUCENT ? sortGeneration : 0L,
				cameraX,
				cameraY,
				cameraZ,
				asset.sectionOriginX() + 8.0D,
				asset.sectionOriginY() + 8.0D,
				asset.sectionOriginZ() + 8.0D,
				layerKey.layer() == ChunkSectionLayer.TRANSLUCENT ? Math.max(0, asset.indexCount() / 6) : 0,
				sortedIndexHash,
				sortGeneration,
				drawOrder
			);
		}
		if (executedStaticTerrainInstances > 0L) {
			net.sodium.client.render.StaticTerrainParityDiagnostics.recordRustWholeFrameExecutionCoverage(
				executionReceipt, frameId
			);
			lastExecutedStaticTerrainFrameId.set(frameId);
			lastExecutedStaticTerrainSubmissionId.set(submissionId);
			lastExecutedStaticTerrainInstances.set(executedStaticTerrainInstances);
		}
	}

	private static void acceptLayer(ChunkBuildOutput output, TerrainRenderPass pass, ChunkSectionLayer layer, long extractionFrameId,
			TerrainMeshLayout layout) {
		BuiltSectionMeshParts mesh = output.getMesh(pass);
		if (mesh == null || mesh.getVertexData().getLength() == 0) {
			skippedEmptyLayers.incrementAndGet();
			net.sodium.client.render.StaticTerrainParityDiagnostics.recordRustStaticTerrainAdmission(
				output, layer.name(), "source-layer-empty"
			);
			removeLayer(output.render.getPosition().asLong(), layer, "empty-layer");
			return;
		}
			try {
				TerrainSectionAsset asset = decodeMesh(output, mesh, layer, layout);
				if (asset == null) {
					skippedEmptyLayers.incrementAndGet();
					net.sodium.client.render.StaticTerrainParityDiagnostics.recordRustStaticTerrainAdmission(
						output, layer.name(), "source-layer-fully-filtered"
					);
					removeLayer(output.render.getPosition().asLong(), layer, "fully-filtered-layer");
					return;
				}
				cacheTextureProbeQuads(output, asset);
				net.sodium.client.render.StaticTerrainParityDiagnostics.recordRustStaticTerrainAppearanceCopy(
					output,
					layer.name(),
					asset.meshKey(),
					asset.meshGeneration(),
					asset.asset().vertices()
				);
				if (layer == ChunkSectionLayer.TRANSLUCENT && asset.unsupportedPrimitiveCount() > 0) {
					unsupportedFluidRejectedSections.incrementAndGet();
					throw new IllegalStateException("Rust whole-frame Vulkan encountered translucent fluid metadata without a semantic material route");
				}
				// Publish the section index only after the Rust-owned mesh registry has
				// accepted the complete asset transaction. If registry admission rejects
				// the copied mesh, SECTION_ASSETS must not advertise a drawable section
				// that the Rust frontend cannot consume.
				long atlasGenerationForRegistration = atlasGeneration;
				RustGalWorldPrimitiveRenderer.registerStaticTerrainMeshAsset(
					asset.asset(), atlasTextureUpdatePayload(waterTextureBinding(WorldRenderRoutePolicy.currentStaticTerrainRoute())), layer == ChunkSectionLayer.TRANSLUCENT
				);
				confirmAtlasPayloadRegistered(atlasGenerationForRegistration);
				SECTION_ASSETS.put(new LayerKey(output.render.getPosition().asLong(), layer), asset);
				net.sodium.client.render.StaticTerrainParityDiagnostics.recordRustStaticTerrainAdmission(
					output, layer.name(), "asset-registered"
				);
				registeredMeshes.incrementAndGet();
				recordEvent(
					output.render.getPosition().asLong(),
					layer,
					output.submitTime,
					asset.meshGeneration(),
					0L,
					atlasGeneration,
					asset,
					output.render.getOriginX(),
					output.render.getOriginY(),
					output.render.getOriginZ(),
					0.0F,
					0.0F,
					0.0F,
					"mesh-registered",
					extractionFrameId,
					0L,
					0L,
					0L,
						layer == ChunkSectionLayer.TRANSLUCENT ? asset.initialSortGeneration() : 0L,
					currentCameraX(),
					currentCameraY(),
					currentCameraZ(),
					output.render.getOriginX() + 8.0D,
					output.render.getOriginY() + 8.0D,
					output.render.getOriginZ() + 8.0D,
					layer == ChunkSectionLayer.TRANSLUCENT ? Math.max(0, asset.indexCount() / 6) : 0,
						layer == ChunkSectionLayer.TRANSLUCENT ? sortedIndexHash(asset.asset().indexBytes()) : 0L,
						layer == ChunkSectionLayer.TRANSLUCENT ? asset.initialSortGeneration() : 0L,
					0
					);
					if (layer == ChunkSectionLayer.TRANSLUCENT) {
						recordInitialTranslucentSort(output, asset);
					}
			} catch (RuntimeException error) {
				LOGGER.warn("Failed to copy Rust static terrain section {} layer {}", output.render.getPosition(), layer, error);
				net.sodium.client.render.StaticTerrainParityDiagnostics.recordRustStaticTerrainAdmission(
					output, layer.name(), "asset-decode-failed"
				);
				removeLayer(output.render.getPosition().asLong(), layer, "decode-failed");
		}
	}

	private static TerrainSectionAsset decodeMesh(ChunkBuildOutput output, BuiltSectionMeshParts mesh, ChunkSectionLayer layer,
			TerrainMeshLayout layout) {
		ByteBuffer buffer = mesh.getVertexData().getDirectBuffer().duplicate().order(ByteOrder.nativeOrder());
		int vertexStride = layout.vertexStride();
		boolean separateAo = layout.separateAo();
		TextureAtlas copiedAtlas = Minecraft.getInstance().getAtlasManager().getAtlasOrThrow(AtlasIds.BLOCKS);
		int copiedAtlasWidth = copiedAtlas.width;
		int copiedAtlasHeight = copiedAtlas.height;
		if (copiedAtlasWidth <= 0 || copiedAtlasHeight <= 0) {
			throw new IllegalStateException("copied terrain atlas has invalid dimensions " + copiedAtlasWidth + "x" + copiedAtlasHeight);
		}
		String fault = activeFault();
		if (vertexStride < COMPACT_PREFIX_STRIDE) {
			throw new IllegalArgumentException("static terrain vertex stride " + vertexStride + " is smaller than compact prefix " + COMPACT_PREFIX_STRIDE);
		}
		if (buffer.remaining() % vertexStride != 0) {
			throw new IllegalArgumentException("static terrain vertex buffer length is not aligned to stride " + vertexStride);
		}
		int bufferVertexCapacity = buffer.remaining() / vertexStride;
		int shaderBlockIdOffset = layout.shaderBlockIdOffset();
		int midBlockOffset = layout.midBlockOffset();
		int[] vertexSegments = mesh.getVertexSegments();
		if (vertexSegments.length % 2 != 0) {
			throw new IllegalArgumentException("static terrain vertex segment array length is odd");
		}
		int vertexCount = 0;
		for (int i = 0; i < vertexSegments.length; i += 2) {
			int segmentVertexCount = vertexSegments[i];
			if (segmentVertexCount <= 0) {
				continue;
			}
			if (segmentVertexCount % 4 != 0) {
				throw new IllegalArgumentException("static terrain vertex segment is not quad-aligned: " + segmentVertexCount);
			}
			vertexCount += segmentVertexCount;
		}
		if (vertexCount <= 0 || vertexCount > 0xffff) {
			throw new IllegalArgumentException("unsupported static terrain vertex count " + vertexCount);
		}
		if (vertexCount > bufferVertexCapacity) {
			throw new IllegalArgumentException("static terrain vertex segments require " + vertexCount + " vertices but buffer holds " + bufferVertexCapacity);
		}
		List<VulkanicGalBridge.WorldMeshVertexRecord> vertices = new ArrayList<>(vertexCount);
		float minX = Float.POSITIVE_INFINITY;
		float minY = Float.POSITIVE_INFINITY;
		float minZ = Float.POSITIVE_INFINITY;
		float maxX = Float.NEGATIVE_INFINITY;
		float maxY = Float.NEGATIVE_INFINITY;
		float maxZ = Float.NEGATIVE_INFINITY;
		float minU = Float.POSITIVE_INFINITY;
		float minV = Float.POSITIVE_INFINITY;
		float maxU = Float.NEGATIVE_INFINITY;
		float maxV = Float.NEGATIVE_INFINITY;
		int separateAoVertexCount = 0;
		float minAo = 1.0F;
		float maxAo = 0.0F;
		boolean aoContractValid = true;
		boolean blockSkyLightContractValid = true;
		for (int vertexIndex = 0; vertexIndex < vertexCount; vertexIndex++) {
			int offset = vertexIndex * vertexStride;
			int positionHi = buffer.getInt(offset + POSITION_OFFSET);
			int positionLo = buffer.getInt(offset + POSITION_OFFSET + 4);
			int compactColor = buffer.getInt(offset + COLOR_OFFSET);
			int color = decodeCompactTerrainColorForRust(compactColor, separateAo, "inverted-ao".equals(fault), "doubled-face-shade".equals(fault));
			int texture = buffer.getInt(offset + TEXTURE_OFFSET);
			int lightMaterial = buffer.getInt(offset + LIGHT_MATERIAL_OFFSET);
			float ao = separateAo ? ((compactColor >>> 24) & 0xff) / 255.0F : 1.0F;
			if ("inverted-ao".equals(fault)) {
				ao = 1.0F - ao;
			}
			if (separateAo) {
				separateAoVertexCount++;
				minAo = Math.min(minAo, ao);
				maxAo = Math.max(maxAo, ao);
				if (((color >>> 24) & 0xff) != 0xff) {
					aoContractValid = false;
				}
			}
			if ("inverted-ao".equals(fault) || "doubled-face-shade".equals(fault)) {
				aoContractValid = false;
			}
			if ("swapped-block-sky-light".equals(fault)) {
				blockSkyLightContractValid = false;
			}
			float x = decodePosition(positionHi, positionLo, 0);
			float y = decodePosition(positionHi, positionLo, 1);
			float z = decodePosition(positionHi, positionLo, 2);
			float u = decodeTextureForCopiedAtlas(texture & 0xffff, copiedAtlasWidth);
			float v = decodeTextureForCopiedAtlas((texture >>> 16) & 0xffff, copiedAtlasHeight);
			minX = Math.min(minX, x);
			minY = Math.min(minY, y);
			minZ = Math.min(minZ, z);
			maxX = Math.max(maxX, x);
			maxY = Math.max(maxY, y);
			maxZ = Math.max(maxZ, z);
			minU = Math.min(minU, u);
			minV = Math.min(minV, v);
			maxU = Math.max(maxU, u);
			maxV = Math.max(maxV, v);
			int packedShaderBlock = shaderBlockIdOffset == 0 ? 0 : buffer.getInt(offset + shaderBlockIdOffset);
			int shaderBlockId = shaderBlockIdOffset == 0 ? -1 : decodeIrisShaderBlockId(packedShaderBlock);
			int shaderMaterialType = shaderBlockIdOffset == 0 ? -1 : decodeIrisShaderRenderType(packedShaderBlock);
			int midBlockPacked = midBlockOffset == 0 ? 0 : buffer.getInt(offset + midBlockOffset);
			vertices.add(new VulkanicGalBridge.WorldMeshVertexRecord(
				x,
				y,
				z,
				u,
				v,
				u,
				v,
				shaderBlockId,
				shaderMaterialType,
				decodeTerrainMaterialBits(lightMaterial),
				color,
				0,
				decodeLight(lightMaterial, "swapped-block-sky-light".equals(fault)),
				midBlockPacked
			));
		}
		applyPrimitiveSemanticFallback(
			mesh.getPrimitiveMetadata(),
			vertices,
			vertexCount,
			shaderBlockIdOffset != 0,
			midBlockOffset != 0
		);
		List<Integer> indices = new ArrayList<>(Math.max(6, vertexCount / 4 * 6));
		List<VulkanicGalBridge.WorldMeshSectionRecord> sections = new ArrayList<>();
		int cursor = 0;
		int maxIndex = -1;
		int positiveYNormalSections = 0;
		int negativeYNormalSections = 0;
		int horizontalNormalSections = 0;
		boolean normalContractValid = true;
		boolean topFaceShadeContractValid = true;
		for (int i = 0; i < vertexSegments.length; i += 2) {
			int segmentVertexCount = vertexSegments[i];
			if (segmentVertexCount <= 0) {
				continue;
			}
			if (cursor + segmentVertexCount > vertexCount) {
				throw new IllegalArgumentException("static terrain vertex segments exceed vertex payload");
			}
			int firstIndex = indices.size();
			int facing = vertexSegments[i + 1];
			int normalPacked = normalForSegment(vertices, cursor, segmentVertexCount, facing);
			if ("inverted-normal".equals(fault)) {
				normalPacked = invertPackedNormal(normalPacked);
				normalContractValid = false;
			}
			switch (facing) {
				case 1 -> positiveYNormalSections++;
				case 4 -> negativeYNormalSections++;
				case 0, 2, 3, 5 -> horizontalNormalSections++;
				default -> {
				}
			}
			if ("wrong-top-face-shade".equals(fault) && facing == 1) {
				topFaceShadeContractValid = false;
			}
			for (int vertex = cursor; vertex < cursor + segmentVertexCount; vertex++) {
				VulkanicGalBridge.WorldMeshVertexRecord original = vertices.get(vertex);
				int color = original.colorArgb();
				if ("wrong-top-face-shade".equals(fault) && facing == 1) {
					color = multiplyArgbRgb(color, 0x80);
				}
				vertices.set(vertex, new VulkanicGalBridge.WorldMeshVertexRecord(
					original.x(), original.y(), original.z(), original.u(), original.v(), original.atlasU(), original.atlasV(),
					original.shaderBlockId(), original.shaderMaterialType(), original.terrainMaterialBits(), color, normalPacked, original.light(), original.midBlockPacked()
				));
			}
			for (int quadBase = cursor; quadBase + 3 < cursor + segmentVertexCount; quadBase += 4) {
				indices.add(quadBase);
				indices.add(quadBase + 1);
				indices.add(quadBase + 2);
				indices.add(quadBase + 2);
				indices.add(quadBase + 3);
				indices.add(quadBase);
				maxIndex = Math.max(maxIndex, quadBase + 3);
			}
			if (layer != ChunkSectionLayer.TRANSLUCENT) {
				// A segment is one facing/material run.  Emit one range after all
				// of its quads have been appended; emitting a growing range per quad
				// duplicates prior quads and advances later segment offsets wrongly.
				sections.add(new VulkanicGalBridge.WorldMeshSectionRecord(
					layer == ChunkSectionLayer.SOLID ? RustGalWorldPrimitiveRenderer.MATERIAL_ID_OPAQUE_TEXTURED : RustGalWorldPrimitiveRenderer.MATERIAL_ID_CUTOUT_TEXTURED,
					RustGalWorldPrimitiveRenderer.MATERIAL_TEXTURE_TERRAIN_BLOCK_ATLAS,
					layer == ChunkSectionLayer.SOLID ? RustGalWorldPrimitiveRenderer.MATERIAL_MODE_OPAQUE : RustGalWorldPrimitiveRenderer.MATERIAL_MODE_CUTOUT,
					// Frozen's Sodium terrain pipelines inherit RenderPipeline's
					// default back-face culling, including the translucent pass.
					// The copied mesh ABI preserves each baked quad's CCW winding, so
					// carry that semantic raster contract into GAL rather than
					// accumulating both sides of every glass/fluid face.
					RustGalWorldPrimitiveRenderer.CULL_BACK,
					RustGalWorldPrimitiveRenderer.WORLD_WINDING_CCW,
					firstIndex * 2,
					indices.size() - firstIndex
				));
			}
			cursor += segmentVertexCount;
		}
		if (cursor != vertexCount) {
			throw new IllegalArgumentException("static terrain vertex segments cover " + cursor + " of " + vertexCount + " vertices");
		}
		byte[] indexBytes;
		OrderedTranslucentMesh orderedTranslucentMesh = null;
		int[] translucentSourceSegmentQuadCounts = layer == ChunkSectionLayer.TRANSLUCENT
			? translucentSourceSegmentQuadCounts(vertexSegments)
			: new int[0];
		if (layer == ChunkSectionLayer.TRANSLUCENT) {
			byte[] sourceSortedIndexBytes = normalizeTranslucentSortedIndexBytes(
				copySorterIndexBytes(output.getSorter()),
				vertexCount,
				translucentSourceSegmentQuadCounts
			);
			if (sourceSortedIndexBytes.length == 0) {
				sourceSortedIndexBytes = packU32(indices);
			}
			orderedTranslucentMesh = buildOrderedTranslucentMesh(
				sourceSortedIndexBytes,
				mesh.getPrimitiveMetadata(),
				vertices,
				vertexCount,
				waterTextureBinding(WorldRenderRoutePolicy.currentStaticTerrainRoute())
			);
			if (orderedTranslucentMesh.retainedIndexCount() == 0) {
				return null;
			}
			indexBytes = orderedTranslucentMesh.indexBytes();
			sections.addAll(orderedTranslucentMesh.sections());
		} else {
			indexBytes = packU16(indices);
		}
		if (sections.isEmpty()) {
			throw new IllegalArgumentException("static terrain mesh has no drawable sections");
		}
		long sectionPos = output.render.getPosition().asLong();
		long meshKey = meshKey(sectionPos, layer);
		long generation = meshGeneration(sectionPos, layer, vertices, indexBytes, sections);
		int diagnosticVertexCount = vertexCount;
		int diagnosticVertexStride = vertexStride;
		int diagnosticMaxIndex = maxIndex;
		int diagnosticIndexType = layer == ChunkSectionLayer.TRANSLUCENT ? INDEX_TYPE_U32 : INDEX_TYPE_U16;
		float diagnosticMaxX = maxX;
		boolean vertexPositionsFinite = finiteBounds(minX, minY, minZ, maxX, maxY, maxZ);
		boolean localBoundsValid = vertexPositionsFinite && minX >= SECTION_LOCAL_MIN && minY >= SECTION_LOCAL_MIN && minZ >= SECTION_LOCAL_MIN && maxX <= SECTION_LOCAL_MAX && maxY <= SECTION_LOCAL_MAX && maxZ <= SECTION_LOCAL_MAX;
		boolean indexRangeValid = maxIndex >= 0 && maxIndex < vertexCount;
		boolean sectionOriginValid = true;
		boolean indexOffsetAlignmentValid = true;
		switch (activeFault()) {
			case "old-stride", "incorrect-vertex-stride" -> diagnosticVertexStride = COMPACT_PREFIX_STRIDE;
			case "vertex-count-exceeds-capacity" -> diagnosticVertexCount = bufferVertexCapacity + 1;
			case "out-of-range-index" -> {
				diagnosticMaxIndex = vertexCount + 7;
				indexRangeValid = false;
			}
			case "index-type-invalid", "incorrect-index-type" -> diagnosticIndexType = 99;
			case "index-alignment-invalid", "incorrect-index-alignment" -> indexOffsetAlignmentValid = false;
			case "non-finite-position" -> {
				vertexPositionsFinite = false;
				localBoundsValid = false;
			}
			case "section-origin-offset", "replacement-previous-origin" -> sectionOriginValid = false;
			case "mesh-key-collision" -> meshKey = 0x5a17_5e77_a14c_0111L;
			case "bounds-out-of-range" -> {
				diagnosticMaxX = 4096.0F;
				localBoundsValid = false;
			}
			default -> {
			}
		}
			long initialSortGeneration = layer == ChunkSectionLayer.TRANSLUCENT ? translucentSortGenerations.incrementAndGet() : 0L;
		return new TerrainSectionAsset(
				meshKey,
				generation,
				generation,
				initialSortGeneration,
				diagnosticVertexCount,
			bufferVertexCapacity,
			diagnosticVertexStride,
			layer == ChunkSectionLayer.TRANSLUCENT ? indexBytes.length / Integer.BYTES : indices.size(),
			diagnosticMaxIndex,
			diagnosticIndexType,
			sections.size(),
			minX,
			minY,
			minZ,
			diagnosticMaxX,
			maxY,
			maxZ,
			minU,
			minV,
			maxU,
			maxV,
			vertexPositionsFinite,
			localBoundsValid,
			finiteBounds(minU, minV, 0.0F, maxU, maxV, 0.0F) && minU >= -0.01F && minV >= -0.01F && maxU <= 1.01F && maxV <= 1.01F,
			indexRangeValid,
			true,
			sectionOriginValid,
			indexOffsetAlignmentValid,
			normalContractValid,
			aoContractValid,
			blockSkyLightContractValid,
			topFaceShadeContractValid,
			separateAo,
			separateAoVertexCount,
			minAo,
			maxAo,
			positiveYNormalSections,
			negativeYNormalSections,
			horizontalNormalSections,
			output.render.getOriginX(),
			output.render.getOriginY(),
			output.render.getOriginZ(),
			orderedTranslucentMesh == null ? "" : orderedTranslucentMesh.accountingReason(),
			orderedTranslucentMesh == null ? 0 : orderedTranslucentMesh.unsupportedPrimitiveCount(),
			translucentSourceSegmentQuadCounts,
			new VulkanicGalBridge.WorldMeshAssetRecord(
				meshKey,
				generation,
				RustGalWorldPrimitiveRenderer.MESH_VERTEX_LAYOUT_V3,
				layer == ChunkSectionLayer.TRANSLUCENT ? INDEX_TYPE_U32 : INDEX_TYPE_U16,
				vertices,
				indexBytes,
				sections
			)
		);
	}

	/**
	 * Restores terrain shader semantics from the native builder's per-quad
	 * metadata when the ordinary compact vertex format deliberately omits Iris
	 * extension fields. Metadata is copied in the same assembled quad order as
	 * vertex data, so this is a semantic mesh boundary rather than a read of
	 * renderer or GPU state.
	 */
	static void applyPrimitiveSemanticFallback(
		int[] primitiveMetadata,
		List<VulkanicGalBridge.WorldMeshVertexRecord> vertices,
		int vertexCount,
		boolean hasPackedShaderBlock,
		boolean hasPackedMidBlock
	) {
		if (vertexCount < 0 || vertexCount % 4 != 0 || vertices.size() != vertexCount) {
			throw new IllegalArgumentException("static terrain semantic vertices are not quad-aligned");
		}
		int metadataStride = NativeSectionMeshBuilder.PRIMITIVE_METADATA_RECORD_INTS;
		int primitiveCount = vertexCount / 4;
		if (primitiveMetadata.length != primitiveCount * metadataStride) {
			throw new IllegalArgumentException("static terrain primitive metadata count " + primitiveMetadata.length
				+ " does not match assembled primitive count " + primitiveCount);
		}
		for (int primitive = 0; primitive < primitiveCount; primitive++) {
			int metadataOffset = primitive * metadataStride;
			int blockId = primitiveMetadata[metadataOffset + 2];
			int localX = primitiveMetadata[metadataOffset + 3];
			int localY = primitiveMetadata[metadataOffset + 4];
			int localZ = primitiveMetadata[metadataOffset + 5];
			int renderType = primitiveMetadata[metadataOffset + 6];
			int blockEmission = primitiveMetadata[metadataOffset + 9];
			// Native render-pass IDs have more states than the single material bit
			// that the semantic terrain ABI carries. This must be derived from the
			// copied primitive metadata, never from an optional Iris extension in
			// the source vertex stream: that packed value is renderer-private and
			// can encode additional non-semantic render types.
			int shaderMaterialType = renderType & 1;
			if (blockEmission < 0 || blockEmission > 0xff) {
				throw new IllegalArgumentException("static terrain primitive " + primitive
					+ " has invalid semantic block emission " + blockEmission);
			}
			for (int vertexOffset = 0; vertexOffset < 4; vertexOffset++) {
				int vertexIndex = primitive * 4 + vertexOffset;
				VulkanicGalBridge.WorldMeshVertexRecord original = vertices.get(vertexIndex);
				if (blockId < 0) {
					throw new IllegalArgumentException("static terrain primitive " + primitive
						+ " lacks a canonical native block-state identity");
				}
				// A packed extension field may be present when Iris chose a wider
				// vertex layout, but that value is an Iris-private shader mapping.
				// The native primitive metadata carries the canonical raw block-state
				// ID that Rust-owned shader-pack resources resolve semantically.
				int shaderBlockId = blockId;
				int resolvedShaderMaterialType = shaderMaterialType;
				int midBlockPacked = hasPackedMidBlock
					? original.midBlockPacked()
					: semanticMidBlockPacked(original.x(), original.y(), original.z(), localX, localY, localZ, blockEmission);
				vertices.set(vertexIndex, new VulkanicGalBridge.WorldMeshVertexRecord(
					original.x(), original.y(), original.z(), original.u(), original.v(), original.atlasU(), original.atlasV(),
					shaderBlockId, resolvedShaderMaterialType, original.terrainMaterialBits(), original.colorArgb(), original.normalPacked(), original.light(), midBlockPacked
				));
			}
		}
	}

	private static int semanticMidBlockPacked(float vertexX, float vertexY, float vertexZ,
		int localX, int localY, int localZ, int blockEmission) {
		int x = ((int)((localX + 0.5F - vertexX) * 64.0F)) & 0xff;
		int y = ((int)((localY + 0.5F - vertexY) * 64.0F)) & 0xff;
		int z = ((int)((localZ + 0.5F - vertexZ) * 64.0F)) & 0xff;
		return x | (y << 8) | (z << 16) | (blockEmission << 24);
	}

	record OrderedTranslucentMesh(
		byte[] indexBytes,
		List<VulkanicGalBridge.WorldMeshSectionRecord> sections,
		int sourcePrimitiveCount,
		int nonFluidPrimitiveCount,
		int waterPrimitiveCount,
		int unsupportedPrimitiveCount,
		int retainedPrimitiveCount,
		int omittedPrimitiveCount,
		int sourceIndexCount,
		int retainedIndexCount,
		int omittedIndexCount,
		long sourceIndexHash,
		long retainedIndexHash,
		long omittedIndexHash,
		int materialSwitchCount,
		int waterStillPrimitiveCount,
		int waterFlowPrimitiveCount,
		int waterOverlayPrimitiveCount,
		int waterTextureSwitchCount,
		long waterAnimationHash,
		String waterAnimationSummary,
		String rangeSummary,
		String primitiveSample
	) {
		String accountingReason() {
			return "translucent-primitive-accounting"
				+ ":sourcePrimitives=" + sourcePrimitiveCount
				+ ":nonFluidPrimitives=" + nonFluidPrimitiveCount
				+ ":waterPrimitives=" + waterPrimitiveCount
				+ ":unsupportedPrimitives=" + unsupportedPrimitiveCount
				+ ":retainedPrimitives=" + retainedPrimitiveCount
				+ ":omittedPrimitives=" + omittedPrimitiveCount
				+ ":executedPrimitives=" + retainedPrimitiveCount
				+ ":sourceIndices=" + sourceIndexCount
				+ ":retainedIndices=" + retainedIndexCount
				+ ":omittedIndices=" + omittedIndexCount
				+ ":sourceHash=" + Long.toUnsignedString(sourceIndexHash)
				+ ":retainedHash=" + Long.toUnsignedString(retainedIndexHash)
				+ ":omittedHash=" + Long.toUnsignedString(omittedIndexHash)
				+ ":rangeCount=" + sections.size()
				+ ":materialSwitches=" + materialSwitchCount
				+ ":waterStillPrimitives=" + waterStillPrimitiveCount
				+ ":waterFlowPrimitives=" + waterFlowPrimitiveCount
				+ ":waterOverlayPrimitives=" + waterOverlayPrimitiveCount
				+ ":waterTextureSwitches=" + waterTextureSwitchCount
				+ ":waterAnimationHash=" + Long.toUnsignedString(waterAnimationHash)
				+ ":waterAnimationEntries=" + waterAnimationSummary
				+ ":ranges=" + rangeSummary
				+ ":sample=" + primitiveSample;
		}
	}

	/** Resource binding only: animation clocks and uploads remain owned by Rust. */
	enum WaterTextureBinding { BLOCK_ATLAS, SEPARATE_SHEETS }

	static WaterTextureBinding waterTextureBinding(WorldRenderRoutePolicy.Route route) {
		// The whole-frame route publishes the owned block atlas unconditionally.
		// Water must consume that same resource, with its declared mip chain and
		// visibility-driven animation, rather than an independently animated sheet.
		return route.usesRustWholeFrameVulkan()
			? WaterTextureBinding.BLOCK_ATLAS : WaterTextureBinding.SEPARATE_SHEETS;
	}

	static OrderedTranslucentMesh buildOrderedTranslucentMesh(byte[] sourceSortedIndexBytes,
			int[] primitiveMetadata, List<VulkanicGalBridge.WorldMeshVertexRecord> vertices, int vertexCount) {
		return buildOrderedTranslucentMesh(sourceSortedIndexBytes, primitiveMetadata, vertices, vertexCount,
			WaterTextureBinding.SEPARATE_SHEETS);
	}

	static OrderedTranslucentMesh buildOrderedTranslucentMesh(byte[] sourceSortedIndexBytes,
			int[] primitiveMetadata, List<VulkanicGalBridge.WorldMeshVertexRecord> vertices, int vertexCount,
			WaterTextureBinding waterBinding) {
		java.util.Objects.requireNonNull(waterBinding);
		if (sourceSortedIndexBytes.length == 0 || sourceSortedIndexBytes.length % (Integer.BYTES * 6) != 0) {
			throw new IllegalArgumentException("translucent sorted index payload must contain whole u32 quads");
		}
		int primitiveCount = vertexCount / 4;
		int metadataStride = NativeSectionMeshBuilder.PRIMITIVE_METADATA_RECORD_INTS;
		if (primitiveMetadata.length != primitiveCount * metadataStride) {
			throw new IllegalArgumentException("translucent primitive metadata count " + primitiveMetadata.length
				+ " does not match primitive count " + primitiveCount);
		}
		ByteArrayOutputStream retainedIndices = new ByteArrayOutputStream(sourceSortedIndexBytes.length);
		ByteArrayOutputStream omittedIndices = new ByteArrayOutputStream(sourceSortedIndexBytes.length);
		List<VulkanicGalBridge.WorldMeshSectionRecord> sections = new ArrayList<>();
		int openMaterialId = 0;
		int openTextureId = 0;
		int openIndexStart = 0;
		int retainedIndexCount = 0;
		int retainedPrimitiveCount = 0;
		int omittedPrimitiveCount = 0;
		int nonFluidPrimitiveCount = 0;
		int waterPrimitiveCount = 0;
		int unsupportedPrimitiveCount = 0;
		int previousRetainedMaterialId = 0;
		int previousRetainedTextureId = 0;
		int materialSwitchCount = 0;
		int waterStillPrimitiveCount = 0;
		int waterFlowPrimitiveCount = 0;
		int waterOverlayPrimitiveCount = 0;
		int waterTextureSwitchCount = 0;
		StringBuilder primitiveSample = new StringBuilder();
		boolean[] seen = new boolean[primitiveCount];
		for (int sourceOffset = 0; sourceOffset < sourceSortedIndexBytes.length; sourceOffset += Integer.BYTES * 6) {
			int primitiveId = primitiveIdFromSortedQuad(sourceSortedIndexBytes, sourceOffset, vertexCount);
			if (primitiveId < 0 || primitiveId >= primitiveCount) {
				throw new IllegalArgumentException("translucent sorted payload references primitive " + primitiveId
					+ " outside 0.." + (primitiveCount - 1));
			}
			if (seen[primitiveId]) {
				throw new IllegalArgumentException("translucent sorted payload references primitive " + primitiveId + " more than once");
			}
			seen[primitiveId] = true;
			int metadataOffset = primitiveId * metadataStride;
			int primitiveKind = primitiveMetadata[metadataOffset];
			// Flat translucent quads do not carry a fluid record.  The native
			// metadata ABI leaves their kind at UNKNOWN (0) on the ordinary
			// compact path; the modified-translucent path already normalizes this
			// value.  Treating it as a non-fluid translucent quad here preserves
			// the semantic atlas material instead of rejecting the whole section.
			if (primitiveKind == NativeSectionMeshBuilder.PRIMITIVE_KIND_UNKNOWN) {
				primitiveKind = NativeSectionMeshBuilder.PRIMITIVE_KIND_NON_FLUID_TRANSLUCENT;
			}
			switch (primitiveKind) {
				case NativeSectionMeshBuilder.PRIMITIVE_KIND_NON_FLUID_TRANSLUCENT -> nonFluidPrimitiveCount++;
				case NativeSectionMeshBuilder.PRIMITIVE_KIND_GENERIC_FLUID -> nonFluidPrimitiveCount++;
				case NativeSectionMeshBuilder.PRIMITIVE_KIND_BUILTIN_WATER -> waterPrimitiveCount++;
				case NativeSectionMeshBuilder.PRIMITIVE_KIND_UNSUPPORTED_FLUID -> unsupportedPrimitiveCount++;
				default -> {
				}
			}
			int materialId = translucentMaterialForPrimitiveKind(primitiveKind);
			if (materialId == 0) {
				closeTranslucentRange(sections, openMaterialId, openTextureId, openIndexStart, retainedIndexCount);
				openMaterialId = 0;
				openTextureId = 0;
				openIndexStart = retainedIndexCount;
				omittedIndices.write(sourceSortedIndexBytes, sourceOffset, Integer.BYTES * 6);
				omittedPrimitiveCount++;
				appendPrimitiveSample(primitiveSample, primitiveId, primitiveKind, "omitted", 0, 0, retainedIndexCount);
				continue;
			}
			int textureId = translucentTextureForPrimitive(primitiveKind, primitiveId, vertices, waterBinding);
			if (primitiveKind == NativeSectionMeshBuilder.PRIMITIVE_KIND_BUILTIN_WATER) {
				int waterType = vertices.get(primitiveId * 4).shaderMaterialType();
				if (waterType == 1) {
					waterStillPrimitiveCount++;
				} else if (waterType == 2) {
					waterFlowPrimitiveCount++;
				} else if (waterType == 3) {
					waterOverlayPrimitiveCount++;
				}
				if (previousRetainedTextureId != 0 && previousRetainedTextureId != textureId) {
					waterTextureSwitchCount++;
				}
			}
			if (openMaterialId != materialId || openTextureId != textureId) {
				closeTranslucentRange(sections, openMaterialId, openTextureId, openIndexStart, retainedIndexCount);
				if (previousRetainedMaterialId != 0 && previousRetainedMaterialId != materialId) {
					materialSwitchCount++;
				}
				previousRetainedMaterialId = materialId;
				previousRetainedTextureId = textureId;
				openMaterialId = materialId;
				openTextureId = textureId;
				openIndexStart = retainedIndexCount;
			}
			retainedIndices.write(sourceSortedIndexBytes, sourceOffset, Integer.BYTES * 6);
			retainedIndexCount += 6;
			retainedPrimitiveCount++;
			appendPrimitiveSample(primitiveSample, primitiveId, primitiveKind, "retained", materialId, textureId, retainedIndexCount - 6);
		}
		closeTranslucentRange(sections, openMaterialId, openTextureId, openIndexStart, retainedIndexCount);
		for (int primitiveId = 0; primitiveId < primitiveCount; primitiveId++) {
			if (!seen[primitiveId]) {
				throw new IllegalArgumentException("translucent sorted payload omitted primitive " + primitiveId);
			}
		}
		byte[] retainedIndexBytes = retainedIndices.toByteArray();
		byte[] omittedIndexBytes = omittedIndices.toByteArray();
		return new OrderedTranslucentMesh(
			retainedIndexBytes,
			sections,
			primitiveCount,
			nonFluidPrimitiveCount,
			waterPrimitiveCount,
			unsupportedPrimitiveCount,
			retainedPrimitiveCount,
			omittedPrimitiveCount,
			sourceSortedIndexBytes.length / Integer.BYTES,
			retainedIndexCount,
			omittedIndexBytes.length / Integer.BYTES,
			sortedIndexHash(sourceSortedIndexBytes),
			sortedIndexHash(retainedIndexBytes),
			sortedIndexHash(omittedIndexBytes),
			materialSwitchCount,
			waterStillPrimitiveCount,
			waterFlowPrimitiveCount,
			waterOverlayPrimitiveCount,
			waterTextureSwitchCount,
			waterAnimationHash(),
			waterAnimationSummary(),
			translucentRangeSummary(sections),
			primitiveSample.toString()
		);
	}

	private static void appendPrimitiveSample(StringBuilder sample, int primitiveId, int primitiveKind, String fate,
			int materialId, int textureId, int retainedIndexStart) {
		if (sample.length() > 512) {
			return;
		}
		if (sample.length() > 0) {
			sample.append(',');
		}
		sample.append(primitiveId)
			.append('/')
			.append(primitiveKind)
			.append('/')
			.append(fate)
			.append('/')
			.append(materialId)
			.append('/')
			.append(textureId)
			.append('/')
			.append(retainedIndexStart);
	}

	private static long waterAnimationHash() {
		long hash = 0xcbf29ce484222325L;
		FluidSpriteAsset still = waterStillAsset;
		FluidSpriteAsset flow = waterFlowAsset;
		FluidSpriteAsset overlay = waterOverlayAsset;
		if (still != null) {
			hash = fnv64Long(hash, still.animationHash());
		}
		if (flow != null) {
			hash = fnv64Long(hash, flow.animationHash());
		}
		if (overlay != null) {
			hash = fnv64Long(hash, overlay.animationHash());
		}
		return hash;
	}

	public static long waterAnimationHashForDiagnostics() {
		return waterAnimationHash();
	}

	private static String waterAnimationSummary() {
		FluidSpriteAsset still = waterStillAsset;
		FluidSpriteAsset flow = waterFlowAsset;
		FluidSpriteAsset overlay = waterOverlayAsset;
		if (still == null || flow == null || overlay == null) {
			return "missing";
		}
		return still.animationSummary() + "|" + flow.animationSummary() + "|" + overlay.animationSummary();
	}

	public static String waterAnimationSummaryForDiagnostics() {
		return waterAnimationSummary();
	}

	public static String waterAnimationFrameStateForDiagnostics(long frameTick) {
		return waterAnimationState("still", waterStillAsset, frameTick)
			+ "|" + waterAnimationState("flow", waterFlowAsset, frameTick)
			+ "|" + waterAnimationState("overlay", waterOverlayAsset, frameTick);
	}

	private static String waterAnimationState(String name, FluidSpriteAsset asset, long frameTick) {
		if (asset == null) {
			return name + "=missing";
		}
		List<VulkanicGalBridge.WorldMeshAnimationFrameRecord> frames = asset.animationFrames();
		if (frames.isEmpty()) {
			return name
				+ "=tex:" + asset.textureId()
				+ ",loc:" + asset.location().toString().replace(':', '~')
				+ ",generation:" + atlasGeneration
				+ ",current:0,next:0,elapsed:0,duration:1,fraction:0.000000,interpolation:" + asset.interpolationPolicy();
		}
		long totalDuration = 0L;
		for (VulkanicGalBridge.WorldMeshAnimationFrameRecord frame : frames) {
			totalDuration += Math.max(1, frame.durationTicks());
		}
		long cursor = Math.floorMod(frameTick, Math.max(1L, totalDuration));
		long elapsed = 0L;
		int frameListIndex = 0;
		for (int index = 0; index < frames.size(); index++) {
			long duration = Math.max(1, frames.get(index).durationTicks());
			if (cursor < elapsed + duration) {
				frameListIndex = index;
				break;
			}
			elapsed += duration;
		}
		VulkanicGalBridge.WorldMeshAnimationFrameRecord current = frames.get(frameListIndex);
		VulkanicGalBridge.WorldMeshAnimationFrameRecord next = frames.get((frameListIndex + 1) % frames.size());
		long duration = Math.max(1L, current.durationTicks());
		double fraction = (double)(cursor - elapsed) / (double)duration;
		return name
			+ "=tex:" + asset.textureId()
			+ ",loc:" + asset.location().toString().replace(':', '~')
			+ ",generation:" + atlasGeneration
			+ ",current:" + current.frameIndex()
			+ ",next:" + next.frameIndex()
			+ ",elapsed:" + (cursor - elapsed)
			+ ",duration:" + duration
			+ ",fraction:" + String.format(Locale.ROOT, "%.6f", fraction)
			+ ",interpolation:" + asset.interpolationPolicy();
	}

	private static String translucentRangeSummary(List<VulkanicGalBridge.WorldMeshSectionRecord> sections) {
		StringBuilder summary = new StringBuilder();
		for (int i = 0; i < sections.size() && i < 16; i++) {
			if (i > 0) {
				summary.append(',');
			}
			VulkanicGalBridge.WorldMeshSectionRecord section = sections.get(i);
			summary.append(section.materialId())
				.append('@')
				.append(section.indexOffset())
				.append('+')
				.append(section.indexCount());
		}
		if (sections.size() > 16) {
			summary.append(",...");
		}
		return summary.toString();
	}

	private static void closeTranslucentRange(List<VulkanicGalBridge.WorldMeshSectionRecord> sections,
			int materialId, int textureId, int startIndex, int currentIndexCount) {
		if (materialId == 0 || currentIndexCount <= startIndex) {
			return;
		}
		sections.add(new VulkanicGalBridge.WorldMeshSectionRecord(
			materialId,
			textureId == 0 ? RustGalWorldPrimitiveRenderer.MATERIAL_TEXTURE_TERRAIN_BLOCK_ATLAS : textureId,
			RustGalWorldPrimitiveRenderer.MATERIAL_MODE_TRANSLUCENT,
			// Frozen's translucent terrain inherits the terrain pipeline's
			// back-face culling. The pass differs by blend/depth-write semantics,
			// not by silently rasterizing both sides of one baked face.
			RustGalWorldPrimitiveRenderer.CULL_BACK,
			RustGalWorldPrimitiveRenderer.WORLD_WINDING_CCW,
			startIndex * Integer.BYTES,
			currentIndexCount - startIndex
		));
	}

	private static int translucentMaterialForPrimitiveKind(int primitiveKind) {
		return switch (primitiveKind) {
			case NativeSectionMeshBuilder.PRIMITIVE_KIND_NON_FLUID_TRANSLUCENT ->
				RustGalWorldPrimitiveRenderer.MATERIAL_ID_TRANSLUCENT_TEXTURED;
			case NativeSectionMeshBuilder.PRIMITIVE_KIND_GENERIC_FLUID ->
				RustGalWorldPrimitiveRenderer.MATERIAL_ID_TRANSLUCENT_TEXTURED;
			case NativeSectionMeshBuilder.PRIMITIVE_KIND_BUILTIN_WATER ->
				RustGalWorldPrimitiveRenderer.MATERIAL_ID_WATER_TRANSLUCENT;
			case NativeSectionMeshBuilder.PRIMITIVE_KIND_UNSUPPORTED_FLUID -> 0;
			default -> throw new IllegalArgumentException("unsupported translucent primitive kind " + primitiveKind);
		};
	}

	private static int translucentTextureForPrimitive(int primitiveKind, int primitiveId,
			List<VulkanicGalBridge.WorldMeshVertexRecord> vertices, WaterTextureBinding waterBinding) {
		if (primitiveKind != NativeSectionMeshBuilder.PRIMITIVE_KIND_BUILTIN_WATER) {
			return RustGalWorldPrimitiveRenderer.MATERIAL_TEXTURE_TERRAIN_BLOCK_ATLAS;
		}
		int base = primitiveId * 4;
		if (base < 0 || base + 3 >= vertices.size()) {
			throw new IllegalArgumentException("water primitive " + primitiveId + " exceeds vertex payload");
		}
		FluidSpriteAsset asset = waterTextureForPrimitive(vertices.subList(base, base + 4));
		for (int index = base; index < base + 4; index++) {
			VulkanicGalBridge.WorldMeshVertexRecord original = vertices.get(index);
			vertices.set(index, new VulkanicGalBridge.WorldMeshVertexRecord(
				original.x(),
				original.y(),
				original.z(),
				waterBinding == WaterTextureBinding.BLOCK_ATLAS ? original.u() : clamp01(asset.localU(original.u())),
				waterBinding == WaterTextureBinding.BLOCK_ATLAS ? original.v() : clamp01(asset.localV(original.v())),
				original.atlasU(),
				original.atlasV(),
				original.shaderBlockId(),
				waterShaderMaterialType(asset.textureId()),
				original.terrainMaterialBits(),
				original.colorArgb(),
				original.normalPacked(),
				original.light(),
				original.midBlockPacked()
			));
		}
		return waterBinding == WaterTextureBinding.BLOCK_ATLAS
			? RustGalWorldPrimitiveRenderer.MATERIAL_TEXTURE_TERRAIN_BLOCK_ATLAS : asset.textureId();
	}

	private static FluidSpriteAsset waterTextureForPrimitive(List<VulkanicGalBridge.WorldMeshVertexRecord> vertices) {
		FluidSpriteAsset still = waterStillAsset;
		FluidSpriteAsset flow = waterFlowAsset;
		FluidSpriteAsset overlay = waterOverlayAsset;
		if (still == null || flow == null || overlay == null) {
			throw new IllegalStateException("water animation texture assets have not been initialized");
		}
		if (allVerticesWithin(vertices, still)) {
			return still;
		}
		if (allVerticesWithin(vertices, overlay)) {
			return overlay;
		}
		if (allVerticesWithin(vertices, flow)) {
			return flow;
		}
		throw new IllegalArgumentException("built-in water primitive UVs do not match still, flow, or overlay sprites");
	}

	private static boolean allVerticesWithin(List<VulkanicGalBridge.WorldMeshVertexRecord> vertices, FluidSpriteAsset asset) {
		for (VulkanicGalBridge.WorldMeshVertexRecord vertex : vertices) {
			if (!asset.contains(vertex.u(), vertex.v())) {
				return false;
			}
		}
		return true;
	}

	private static float clamp01(float value) {
		return Math.max(0.0F, Math.min(1.0F, value));
	}

	private static int waterShaderMaterialType(int textureId) {
		if (textureId == RustGalWorldPrimitiveRenderer.MATERIAL_TEXTURE_WATER_STILL) {
			return 1;
		}
		if (textureId == RustGalWorldPrimitiveRenderer.MATERIAL_TEXTURE_WATER_FLOW) {
			return 2;
		}
		if (textureId == RustGalWorldPrimitiveRenderer.MATERIAL_TEXTURE_WATER_OVERLAY) {
			return 3;
		}
		return 0;
	}

	private static int primitiveIdFromSortedQuad(byte[] bytes, int offset, int vertexCount) {
		int i0 = readU32Index(bytes, offset);
		int i1 = readU32Index(bytes, offset + 4);
		int i2 = readU32Index(bytes, offset + 8);
		int i3 = readU32Index(bytes, offset + 12);
		int i4 = readU32Index(bytes, offset + 16);
		int i5 = readU32Index(bytes, offset + 20);
		if ((i0 & 3) != 0 || i1 != i0 + 1 || i2 != i0 + 2 || i3 != i0 + 2 || i4 != i0 + 3 || i5 != i0) {
			throw new IllegalArgumentException("translucent sorted payload contains an interleaved or malformed primitive at byte " + offset);
		}
		if (i4 < 0 || i4 >= vertexCount) {
			throw new IllegalArgumentException("translucent sorted payload references vertex " + i4 + " but vertex count is " + vertexCount);
		}
		return i0 / 4;
	}

	/**
	 * Sodium's STATIC_NORMAL_RELATIVE sorter keeps each facing's vertex indices
	 * local. Our copied semantic terrain stream is flattened, so the same local
	 * quad index legitimately occurs once per facing. Translate that established
	 * producer format without changing its within-facing sort order. Global
	 * sorter payloads, including dynamic/topological orders, are already unique
	 * and pass through untouched.
	 */
	static byte[] normalizeTranslucentSortedIndexBytes(byte[] source, int vertexCount, int[] segmentQuadCounts) {
		if (source.length == 0) {
			return source;
		}
		if (source.length % (Integer.BYTES * 6) != 0 || vertexCount < 0 || vertexCount % 4 != 0) {
			throw new IllegalArgumentException("translucent sorted index payload has an invalid quad layout");
		}
		int primitiveCount = vertexCount / 4;
		if (source.length / (Integer.BYTES * 6) != primitiveCount) {
			throw new IllegalArgumentException("translucent sorted index payload count does not match copied terrain vertices");
		}
		if (sortedIndexPayloadHasUniqueGlobalPrimitives(source, vertexCount)) {
			return source;
		}
		int totalSegmentPrimitives = 0;
		for (int count : segmentQuadCounts) {
			if (count < 0) {
				throw new IllegalArgumentException("translucent source segment has a negative primitive count");
			}
			totalSegmentPrimitives = Math.addExact(totalSegmentPrimitives, count);
		}
		if (totalSegmentPrimitives != primitiveCount) {
			throw new IllegalArgumentException("translucent source segments do not cover copied terrain primitives");
		}
		byte[] normalized = new byte[source.length];
		int sourceOffset = 0;
		int globalVertexBase = 0;
		for (int segmentQuadCount : segmentQuadCounts) {
			boolean[] seenLocal = new boolean[segmentQuadCount];
			int segmentVertexCount = Math.multiplyExact(segmentQuadCount, 4);
			for (int quad = 0; quad < segmentQuadCount; quad++) {
				int localPrimitive = primitiveIdFromSortedQuad(source, sourceOffset, segmentVertexCount);
				if (seenLocal[localPrimitive]) {
					throw new IllegalArgumentException("translucent facing-local payload references primitive "
						+ localPrimitive + " more than once");
				}
				seenLocal[localPrimitive] = true;
				for (int index = 0; index < 6; index++) {
					writeU32Index(normalized, sourceOffset + index * Integer.BYTES,
						Math.addExact(globalVertexBase, readU32Index(source, sourceOffset + index * Integer.BYTES)));
				}
				sourceOffset += Integer.BYTES * 6;
			}
			globalVertexBase = Math.addExact(globalVertexBase, segmentVertexCount);
		}
		if (!sortedIndexPayloadHasUniqueGlobalPrimitives(normalized, vertexCount)) {
			throw new IllegalArgumentException("normalized translucent sorted payload is not globally complete");
		}
		return normalized;
	}

	private static boolean sortedIndexPayloadHasUniqueGlobalPrimitives(byte[] bytes, int vertexCount) {
		int primitiveCount = vertexCount / 4;
		boolean[] seen = new boolean[primitiveCount];
		try {
			for (int offset = 0; offset < bytes.length; offset += Integer.BYTES * 6) {
				int primitive = primitiveIdFromSortedQuad(bytes, offset, vertexCount);
				if (seen[primitive]) {
					return false;
				}
				seen[primitive] = true;
			}
		} catch (IllegalArgumentException ignored) {
			return false;
		}
		for (boolean present : seen) {
			if (!present) {
				return false;
			}
		}
		return true;
	}

	private static int[] translucentSourceSegmentQuadCounts(int[] vertexSegments) {
		int[] counts = new int[vertexSegments.length / 2];
		int count = 0;
		for (int index = 0; index < vertexSegments.length; index += 2) {
			int vertices = vertexSegments[index];
			if (vertices < 0 || vertices % 4 != 0) {
				throw new IllegalArgumentException("translucent source vertex segment is not quad-aligned");
			}
			if (vertices > 0) {
				counts[count++] = vertices / 4;
			}
		}
		return Arrays.copyOf(counts, count);
	}

	private static void writeU32Index(byte[] bytes, int offset, int value) {
		bytes[offset] = (byte)(value & 0xff);
		bytes[offset + 1] = (byte)((value >>> 8) & 0xff);
		bytes[offset + 2] = (byte)((value >>> 16) & 0xff);
		bytes[offset + 3] = (byte)((value >>> 24) & 0xff);
	}

	private static int readU32Index(byte[] bytes, int offset) {
		return (bytes[offset] & 0xff)
			| ((bytes[offset + 1] & 0xff) << 8)
			| ((bytes[offset + 2] & 0xff) << 16)
			| ((bytes[offset + 3] & 0xff) << 24);
	}

	private static boolean enqueueSectionLayer(RenderSection section, ChunkSectionLayer layer, Camera camera, int viewportWidth, int viewportHeight, int drawOrder,
			Set<VisibleSubmitKey> visibleSubmissions) {
		visibleLayerProbes.incrementAndGet();
		TerrainSectionAsset asset = SECTION_ASSETS.get(new LayerKey(section.getPosition().asLong(), layer));
		if (asset == null) {
			net.sodium.client.render.StaticTerrainParityDiagnostics.recordRustStaticTerrainNonExecution(
				section.getPosition().asLong(),
				layer.name(),
				"asset-missing:" + net.sodium.client.render.StaticTerrainParityDiagnostics
					.rustStaticTerrainAdmissionReason(section.getPosition().asLong(), layer.name()),
				0L,
				0L
			);
			return false;
		}
		float[] transform = cameraRelativeTranslationTransform(
			(float)(section.getOriginX() - camera.getPosition().x()),
			(float)(section.getOriginY() - camera.getPosition().y()),
			(float)(section.getOriginZ() - camera.getPosition().z())
		);
		long visibleGeneration = "stale-generation".equals(activeFault()) ? asset.meshGeneration() + 1L : asset.meshGeneration();
			TranslucentSortSnapshot sortedIndex =
				layer == ChunkSectionLayer.TRANSLUCENT ? currentTranslucentSortSnapshot(asset) : null;
			long sortGeneration = sortedIndex == null ? 0L : sortedIndex.sortGeneration();
			long sortedIndexHash = sortedIndex == null ? 0L : sortedIndex.indexHash();
		// A visible section can be discovered in the same frame that its copied
		// asset is registered, before the combined Rust upload is acknowledged.
		// Keep that ordinary admission latency out of the stale-generation
		// counter; an explicitly requested stale-generation fault still reaches
		// the strict rejection path below.
		if (!"stale-generation".equals(activeFault())
				&& !RustGalWorldPrimitiveRenderer.isStaticTerrainMeshGenerationUploaded(asset.meshKey(), asset.meshGeneration())) {
			net.sodium.client.render.StaticTerrainParityDiagnostics.recordRustStaticTerrainNonExecution(
				section.getPosition().asLong(), layer.name(), "asset-upload-pending", asset.meshGeneration(), 0L
			);
			return false;
		}
		if (visibleSubmissions != null && !"duplicate-visible-section".equals(activeFault())) {
			VisibleSubmitKey submitKey = new VisibleSubmitKey(section.getPosition().asLong(), layer, visibleGeneration);
			if (!visibleSubmissions.add(submitKey)) {
				net.sodium.client.render.StaticTerrainParityDiagnostics.recordRustStaticTerrainNonExecution(
					section.getPosition().asLong(), layer.name(), "duplicate-visible-suppressed", visibleGeneration, 0L
				);
				recordEvent(
					section.getPosition().asLong(),
					layer,
					0L,
					asset.meshGeneration(),
					visibleGeneration,
					atlasGeneration,
					asset,
					section.getOriginX(),
					section.getOriginY(),
					section.getOriginZ(),
					(float)(section.getOriginX() - camera.getPosition().x()),
					(float)(section.getOriginY() - camera.getPosition().y()),
					(float)(section.getOriginZ() - camera.getPosition().z()),
					"duplicate-visible-suppressed",
					0L,
					0L,
					0L,
					0L,
					sortGeneration,
					camera.getPosition().x(),
					camera.getPosition().y(),
					camera.getPosition().z(),
					section.getOriginX() + 8.0D,
					section.getOriginY() + 8.0D,
					section.getOriginZ() + 8.0D,
					Math.max(0, asset.indexCount() / 6),
					sortedIndexHash,
					sortGeneration,
					drawOrder
				);
				return false;
			}
		}
		boolean submitted = RustGalWorldPrimitiveRenderer.enqueueStaticTerrainMeshInstance(
			asset.meshKey(),
			visibleGeneration,
			transform,
			viewportWidth,
			viewportHeight,
			terrainDepthPolicy(layer),
			// Match Sodium's terrain pipeline contract for every layer. The
			// copied mesh preserves its authored winding, so this remains an
			// explicit semantic request rather than an OpenGL-state fallback.
			RustGalWorldPrimitiveRenderer.CULL_BACK,
			terrainCameraSortRequested(layer),
			Boolean.getBoolean("mattmc.dev.rustGalTerrainPlacement")
				? new VulkanicGalBridge.TerrainSectionPlacement(section.getOriginX(),section.getOriginY(),section.getOriginZ(),
					camera.getPosition().x(),camera.getPosition().y(),camera.getPosition().z()) : null
		);
		long enqueueFrameId = rustEnqueueFrames.incrementAndGet();
		if (submitted) {
			visibleLayerSubmissions.incrementAndGet();
			// This section's accepted semantic draw supplies CPU resource names,
			// never Sodium render-list storage or its mutable sprite-active flags.
			var animatedSprites = section.getAnimatedSprites();
			if (animatedSprites != null) {
				for (var sprite : animatedSprites) {
					RustGalWorldPrimitiveRenderer.recordAtlasSpriteUse(
						sprite.semanticAnimationResource(), sprite.atlasLocation(), sprite.contents().name());
				}
			}
			recordCurrentFrameVisibleSubmission(enqueueFrameId);
			recordVisibleSubmissionIdentity(section.getPosition().asLong(), layer, visibleGeneration);
			if (layer == ChunkSectionLayer.TRANSLUCENT) {
				retainTranslucentExecutionMetadata(asset.meshKey(), new TranslucentExecutionMetadata(
						sortGeneration,
						sortedIndexHash,
						camera.getPosition().x(),
						camera.getPosition().y(),
						camera.getPosition().z(),
						drawOrder
				));
			}
			net.sodium.client.render.StaticTerrainParityDiagnostics.recordRustStaticTerrainExecution(
				"rust-vulkan-enqueued",
				section.getPosition().asLong(),
				layer.name(),
				asset.meshKey(),
				asset.meshGeneration(),
				visibleGeneration,
				asset.contentHash(),
				asset.vertexCount(),
				asset.indexCount(),
				asset.indexType(),
				Math.max(0, asset.indexCount() / 6),
				asset.sectionCount(),
				asset.sectionOriginX(),
				asset.sectionOriginY(),
				asset.sectionOriginZ(),
				asset.localMinX(),
				asset.localMinY(),
				asset.localMinZ(),
				asset.localMaxX(),
				asset.localMaxY(),
				asset.localMaxZ(),
				asset.uvMinU(),
				asset.uvMinV(),
				asset.uvMaxU(),
				asset.uvMaxV(),
				currentGameplayFrameId(),
				enqueueFrameId
			);
			if (detailedTerrainDiagnosticsEnabled()) {
			recordEvent(
				section.getPosition().asLong(),
				layer,
				0L,
				asset.meshGeneration(),
				visibleGeneration,
				atlasGeneration,
				asset,
				section.getOriginX(),
				section.getOriginY(),
				section.getOriginZ(),
				(float)(section.getOriginX() - camera.getPosition().x()),
				(float)(section.getOriginY() - camera.getPosition().y()),
				(float)(section.getOriginZ() - camera.getPosition().z()),
				"visible-submit",
				0L,
				enqueueFrameId,
				0L,
				0L,
				layer == ChunkSectionLayer.TRANSLUCENT ? sortGeneration : 0L,
				camera.getPosition().x(),
				camera.getPosition().y(),
				camera.getPosition().z(),
				section.getOriginX() + 8.0D,
				section.getOriginY() + 8.0D,
				section.getOriginZ() + 8.0D,
				Math.max(0, asset.indexCount() / 6),
				sortedIndexHash,
				sortGeneration,
				drawOrder
			);
			}
			if (detailedTerrainDiagnosticsEnabled() && "duplicate-visible-section".equals(activeFault())) {
				recordEvent(
					section.getPosition().asLong(),
					layer,
					0L,
					asset.meshGeneration(),
					visibleGeneration,
					atlasGeneration,
					asset,
					section.getOriginX(),
					section.getOriginY(),
					section.getOriginZ(),
					(float)(section.getOriginX() - camera.getPosition().x()),
					(float)(section.getOriginY() - camera.getPosition().y()),
					(float)(section.getOriginZ() - camera.getPosition().z()),
					"visible-submit",
					0L,
					enqueueFrameId,
					0L,
					0L
				);
			}
		} else {
			// Registration can be retired concurrently with the visible-list walk
			// when Sodium publishes a replacement section. Re-check the explicit
			// Rust residency state before classifying the failed enqueue as a stale
			// generation; a missing generation here is ordinary upload/retirement
			// latency, not evidence of a bad semantic submission.
			if (!"stale-generation".equals(activeFault())
					&& !RustGalWorldPrimitiveRenderer.isStaticTerrainMeshGenerationUploaded(asset.meshKey(), asset.meshGeneration())) {
				net.sodium.client.render.StaticTerrainParityDiagnostics.recordRustStaticTerrainNonExecution(
					section.getPosition().asLong(), layer.name(), "asset-upload-pending", asset.meshGeneration(), 0L
				);
				return false;
			}
			failedLayerSubmissions.incrementAndGet();
			net.sodium.client.render.StaticTerrainParityDiagnostics.recordRustStaticTerrainNonExecution(
				section.getPosition().asLong(), layer.name(), "stale-or-unregistered-submit", visibleGeneration, enqueueFrameId
			);
			recordEvent(section.getPosition().asLong(), layer, 0L, asset.meshGeneration(), visibleGeneration, atlasGeneration, asset, section.getOriginX(), section.getOriginY(), section.getOriginZ(), 0.0F, 0.0F, 0.0F, "stale-or-unregistered-submit", 0L, enqueueFrameId, 0L, 0L);
		}
		return submitted;
	}

	/**
	 * Builds the column-major affine translation used by the semantic terrain
	 * instance ABI without constructing a temporary JOML matrix. The returned
	 * array is intentionally fresh because the queued semantic record retains
	 * it until the frame coordinator consumes the exact frame snapshot.
	 */
	private static float[] cameraRelativeTranslationTransform(float x, float y, float z) {
		return new float[] {
			1.0F, 0.0F, 0.0F, 0.0F,
			0.0F, 1.0F, 0.0F, 0.0F,
			0.0F, 0.0F, 1.0F, 0.0F,
			x, y, z, 1.0F
		};
	}

	/**
	 * The deterministic capture gate settles on the exact semantic work that
	 * reached the Rust route. A global mesh-cache counter is not a valid proxy:
	 * chunks outside the current camera can continue rebuilding while the
	 * visible terrain is already stable.
	 */
	private static void recordVisibleSubmissionIdentity(long sectionPos, ChunkSectionLayer layer, long generation) {
		String identity = "rust-vulkan-whole-frame:section=" + sectionPos
			+ ":layer=" + layer.name()
			+ ":generation=" + generation;
		// The direct CPU producer replaces Sodium's GL-region renderer on this
		// route, but it is still the semantic sodium-terrain family. Recording
		// that identity lets the shared parity capture wait for real submitted
		// terrain instead of accepting an empty first Rust frame.
		net.minecraft.client.dev.GraphicsFrameBenchmark.recordSubmittedWorkIdentity("sodium-terrain", identity);
		net.minecraft.client.dev.DeterministicCameraCapture.recordSubmittedWorkIdentity("sodium-terrain", identity);
		net.minecraft.client.dev.GraphicsFrameBenchmark.recordSubmittedWorkIdentity("static-terrain", identity);
		net.minecraft.client.dev.DeterministicCameraCapture.recordSubmittedWorkIdentity("static-terrain", identity);
	}

	public static boolean injectCrossWorldStaleSubmissionForDiagnostics(int viewportWidth, int viewportHeight) {
		if (!"cross-world-stale-submission".equals(activeFault())) {
			return false;
		}
		TerrainSectionAsset stale = lastWorldUnloadAsset;
		if (stale == null) {
			return false;
		}
		long enqueueFrameId = rustEnqueueFrames.incrementAndGet();
		boolean submitted = RustGalWorldPrimitiveRenderer.enqueueStaticTerrainMeshInstance(
			stale.meshKey(),
			stale.meshGeneration(),
			cameraRelativeTranslationTransform(0.0F, 0.0F, 0.0F),
			viewportWidth,
			viewportHeight
		);
		if (submitted) {
			visibleLayerSubmissions.incrementAndGet();
			recordCurrentFrameVisibleSubmission(enqueueFrameId);
			recordEvent(
				lastWorldUnloadSectionPos,
				lastWorldUnloadLayer,
				0L,
				stale.meshGeneration(),
				stale.meshGeneration(),
				atlasGeneration,
				stale,
				stale.sectionOriginX(),
				stale.sectionOriginY(),
				stale.sectionOriginZ(),
				0.0F,
				0.0F,
				0.0F,
				"cross_world_stale_submission_unexpected_success",
				0L,
				enqueueFrameId,
				0L,
				0L
			);
			return true;
		}
		failedLayerSubmissions.incrementAndGet();
		recordEvent(
			lastWorldUnloadSectionPos,
			lastWorldUnloadLayer,
			0L,
			stale.meshGeneration(),
			stale.meshGeneration(),
			atlasGeneration,
			stale,
			stale.sectionOriginX(),
			stale.sectionOriginY(),
			stale.sectionOriginZ(),
			0.0F,
			0.0F,
			0.0F,
			"cross_world_stale_submission",
			0L,
			enqueueFrameId,
			0L,
			0L
		);
		return false;
	}

	private static void recordCurrentFrameVisibleSubmission(long fallbackFrameId) {
		long frameId = currentGameplayFrameId();
		if (frameId <= 0L) {
			frameId = fallbackFrameId;
		}
		long previousFrameId = lastVisibleSubmissionFrameId.getAndSet(frameId);
		if (previousFrameId == frameId) {
			currentFrameVisibleLayerSubmissions.incrementAndGet();
		} else {
			currentFrameVisibleLayerSubmissions.set(1L);
		}
	}

	private static boolean finiteBounds(float minX, float minY, float minZ, float maxX, float maxY, float maxZ) {
		return Float.isFinite(minX) && Float.isFinite(minY) && Float.isFinite(minZ)
			&& Float.isFinite(maxX) && Float.isFinite(maxY) && Float.isFinite(maxZ)
			&& minX <= maxX && minY <= maxY && minZ <= maxZ;
	}

	private static int activeTerrainVertexStride() {
		if (net.vulkanic.bridge.RustGalVulkanWholeFrameMode.enabled()) {
			return COMPACT_PREFIX_STRIDE;
		}
		try {
			return WorldRenderingSettings.INSTANCE.getVertexFormat().getNativeFormat().stride();
		} catch (RuntimeException error) {
			return COMPACT_PREFIX_STRIDE;
		}
	}

	private static int activeTerrainShaderBlockIdOffset(int vertexStride) {
		if (net.vulkanic.bridge.RustGalVulkanWholeFrameMode.enabled()) {
			return 0;
		}
		try {
			int offset = WorldRenderingSettings.INSTANCE.getVertexFormat().getNativeFormat().blockIdOffset();
			if (offset <= 0 || offset + Integer.BYTES > vertexStride) {
				return 0;
			}
			return offset;
		} catch (RuntimeException error) {
			return 0;
		}
	}

	private static int activeTerrainMidBlockOffset(int vertexStride) {
		if (net.vulkanic.bridge.RustGalVulkanWholeFrameMode.enabled()) {
			return 0;
		}
		try {
			int offset = WorldRenderingSettings.INSTANCE.getVertexFormat().getNativeFormat().midBlockOffset();
			if (offset <= 0 || offset + Integer.BYTES > vertexStride) {
				return 0;
			}
			return offset;
		} catch (RuntimeException error) {
			return 0;
		}
	}

	private record TerrainMeshLayout(int vertexStride, boolean separateAo, int shaderBlockIdOffset, int midBlockOffset) {
		private static TerrainMeshLayout compact() {
			// The Rust-owned vanilla producer uses Sodium's compact baked-color
			// convention: AO and directional face shade are already in RGB.  This
			// preserves the complete semantic color contract without consulting
			// Iris or requiring a hidden shader-side lighting stage.
			return new TerrainMeshLayout(COMPACT_PREFIX_STRIDE, false, 0, 0);
		}

		private static TerrainMeshLayout activeIrisCompatible() {
			int stride = activeTerrainVertexStride();
			boolean separateAo = false;
			try {
				separateAo = WorldRenderingSettings.INSTANCE.shouldUseSeparateAo();
			} catch (RuntimeException ignored) {
			}
			return new TerrainMeshLayout(stride, separateAo, activeTerrainShaderBlockIdOffset(stride),
				activeTerrainMidBlockOffset(stride));
		}
	}

	// Iris stores (blockId + 1) << 1 with the low bit reserved for render type.
	static int decodeIrisShaderBlockId(int packedBlockId) {
		return (packedBlockId >>> 1) - 1;
	}

	static int decodeIrisShaderRenderType(int packedBlockId) {
		return packedBlockId & 1;
	}

	private static String activeFault() {
		String scenario = System.getProperty(STATIC_TERRAIN_SCENARIO_PROPERTY, "").trim();
		if (scenario.isBlank() || "hidden".equalsIgnoreCase(scenario)) {
			return "";
		}
		return System.getProperty(FAULT_PROPERTY, "").trim().toLowerCase(Locale.ROOT);
	}

	/**
	 * Per-visible-layer diagnostic records are intentionally capture/fault-only.
	 * Ordinary gameplay still records semantic submission counts and Rust
	 * execution receipts, but constructing a large object plus locking the recent
	 * event deque for every visible layer would turn the producer into the frame
	 * bottleneck. Deterministic captures and fault fixtures retain the complete
	 * event stream needed by their validation contracts.
	 */
	private static boolean detailedTerrainDiagnosticsEnabled() {
		return Boolean.getBoolean("mattmc.dev.deterministicCameraCapture")
			|| Boolean.getBoolean("mattmc.dev.staticTerrainParityDiagnostics")
			|| !activeFault().isEmpty();
	}

	private static boolean isStaticTerrainTranslucentScenario() {
		String scenario = System.getProperty(STATIC_TERRAIN_SCENARIO_PROPERTY, "").trim();
		return "translucent-glass".equalsIgnoreCase(scenario) || "translucent-overlap".equalsIgnoreCase(scenario);
	}

	private static float decodePosition(int hi, int lo, int component) {
		int shift = component * 10;
		int value = ((hi >>> shift) & 0x3ff) << 10 | ((lo >>> shift) & 0x3ff);
		return value / (float)POSITION_MAX_VALUE * 32.0F - 8.0F;
	}

	/**
	 * Decodes Sodium's compact terrain coordinate into the copied block-atlas
	 * coordinate system. The copied atlas is top-origin, so this is deliberately
	 * not an OpenGL-style {@code 1 - v} conversion.
	 */
	static float decodeTexture(int value) {
		return (value & 0x7fff) / (float)TEXTURE_MAX_VALUE;
	}

	/**
	 * Resolves Sodium's compact UV and its high-bit direction into the exact
	 * copied-atlas coordinate consumed by Rust. Frozen's block-layer vertex
	 * stage performs this as {@code base + direction * u_TexCoordShrink}; the
	 * result is semantic vertex data, never a borrowed Java texture or sampler.
	 */
	static float decodeTextureForCopiedAtlas(int packedValue, int atlasExtent) {
		if (atlasExtent <= 0) {
			throw new IllegalArgumentException("copied atlas extent must be positive");
		}
		float base = decodeTexture(packedValue);
		float direction = (packedValue & 0x8000) == 0 ? -1.0F : 1.0F;
		float shrink = (1.0F / TEXTURE_MAX_VALUE)
			- (1.0F / (atlasExtent * (float)COMPACT_TEXTURE_SUB_TEXEL_PRECISION));
		return base + direction * shrink;
	}


	static int decodeLight(int lightMaterial, boolean swapBlockAndSky) {
		int block = lightMaterial & 0xff;
		int sky = (lightMaterial >>> 8) & 0xff;
		if (swapBlockAndSky) {
			int swapped = block;
			block = sky;
			sky = swapped;
		}
		// These are UV2 byte coordinates, not integer light levels. Smooth
		// lighting may use every low bit; quantizing changes texture sampling.
		return block | (sky << 16);
	}

	/** Sodium's compact terrain material byte: mip policy and alpha cutoff. */
	static int decodeTerrainMaterialBits(int lightMaterial) {
		return (lightMaterial >>> 16) & 0xff;
	}

	static int decodeCompactTerrainColorForRust(int compactAbgr, boolean separateAo) {
		return decodeCompactTerrainColorForRust(compactAbgr, separateAo, false, false);
	}

	private static int decodeCompactTerrainColorForRust(int compactAbgr, boolean separateAo, boolean invertAo, boolean doubleShade) {
		int alphaOrAo = (compactAbgr >>> 24) & 0xff;
		int blue = (compactAbgr >>> 16) & 0xff;
		int green = (compactAbgr >>> 8) & 0xff;
		int red = compactAbgr & 0xff;
		int alpha = alphaOrAo;
		if (separateAo) {
			if (invertAo) {
				alphaOrAo = 255 - alphaOrAo;
			}
			red = multiplyColorByte(red, alphaOrAo);
			green = multiplyColorByte(green, alphaOrAo);
			blue = multiplyColorByte(blue, alphaOrAo);
			if (doubleShade) {
				red = multiplyColorByte(red, alphaOrAo);
				green = multiplyColorByte(green, alphaOrAo);
				blue = multiplyColorByte(blue, alphaOrAo);
			}
			alpha = 0xff;
		}
		return (alpha << 24) | (red << 16) | (green << 8) | blue;
	}

	private static int multiplyColorByte(int color, int factor) {
		return Math.max(0, Math.min(255, (color * factor + 255) >>> 8));
	}

	private static int multiplyArgbRgb(int argb, int factor) {
		int alpha = argb & 0xff000000;
		int red = multiplyColorByte((argb >>> 16) & 0xff, factor);
		int green = multiplyColorByte((argb >>> 8) & 0xff, factor);
		int blue = multiplyColorByte(argb & 0xff, factor);
		return alpha | (red << 16) | (green << 8) | blue;
	}

	private static int invertPackedNormal(int normalPacked) {
		float x = -unpackPackedNormalComponent(normalPacked, 0);
		float y = -unpackPackedNormalComponent(normalPacked, 8);
		float z = -unpackPackedNormalComponent(normalPacked, 16);
		return packNormal(x, y, z);
	}

	private static float unpackPackedNormalComponent(int normalPacked, int shift) {
		return (byte)((normalPacked >>> shift) & 0xff) / 127.0F;
	}

	private static int normalForSegment(List<VulkanicGalBridge.WorldMeshVertexRecord> vertices, int start, int count, int facing) {
		return switch (facing) {
			case 0 -> packNormal(1, 0, 0);
			case 1 -> packNormal(0, 1, 0);
			case 2 -> packNormal(0, 0, 1);
			case 3 -> packNormal(-1, 0, 0);
			case 4 -> packNormal(0, -1, 0);
			case 5 -> packNormal(0, 0, -1);
			default -> count >= 3 ? computedNormal(vertices.get(start), vertices.get(start + 1), vertices.get(start + 2)) : packNormal(0, 1, 0);
		};
	}

	private static int computedNormal(
		VulkanicGalBridge.WorldMeshVertexRecord a,
		VulkanicGalBridge.WorldMeshVertexRecord b,
		VulkanicGalBridge.WorldMeshVertexRecord c
	) {
		float ax = b.x() - a.x();
		float ay = b.y() - a.y();
		float az = b.z() - a.z();
		float bx = c.x() - a.x();
		float by = c.y() - a.y();
		float bz = c.z() - a.z();
		float nx = ay * bz - az * by;
		float ny = az * bx - ax * bz;
		float nz = ax * by - ay * bx;
		float length = (float)Math.sqrt(nx * nx + ny * ny + nz * nz);
		if (length <= 0.00001F) {
			return packNormal(0, 1, 0);
		}
		return packNormal(nx / length, ny / length, nz / length);
	}

	private static int packNormal(float x, float y, float z) {
		int ix = Math.max(-127, Math.min(127, Math.round(x * 127.0F))) & 0xff;
		int iy = Math.max(-127, Math.min(127, Math.round(y * 127.0F))) & 0xff;
		int iz = Math.max(-127, Math.min(127, Math.round(z * 127.0F))) & 0xff;
		return ix | (iy << 8) | (iz << 16);
	}

	private static byte[] packU16(List<Integer> indices) {
		byte[] bytes = new byte[indices.size() * 2];
		for (int i = 0; i < indices.size(); i++) {
			int value = indices.get(i);
			bytes[i * 2] = (byte)(value & 0xff);
			bytes[i * 2 + 1] = (byte)((value >>> 8) & 0xff);
		}
		return bytes;
	}

	private static byte[] packU32(List<Integer> indices) {
		byte[] bytes = new byte[indices.size() * 4];
		for (int i = 0; i < indices.size(); i++) {
			int value = indices.get(i);
			int offset = i * 4;
			bytes[offset] = (byte)(value & 0xff);
			bytes[offset + 1] = (byte)((value >>> 8) & 0xff);
			bytes[offset + 2] = (byte)((value >>> 16) & 0xff);
			bytes[offset + 3] = (byte)((value >>> 24) & 0xff);
		}
		return bytes;
	}

	private static byte[] copySorterIndexBytes(Sorter sorter) {
		if (sorter == null) {
			return new byte[0];
		}
		NativeBuffer indexBuffer = sorter.getIndexBuffer();
		if (indexBuffer == null || indexBuffer.getLength() <= 0) {
			return new byte[0];
		}
		ByteBuffer src = indexBuffer.getDirectBuffer().duplicate();
		src.clear();
		src.limit(indexBuffer.getLength());
		byte[] bytes = new byte[indexBuffer.getLength()];
		src.get(bytes);
		return bytes;
	}

		private static RustGalWorldPrimitiveRenderer.StaticTerrainSortedIndexSnapshot sortedIndexSnapshot(long meshKey) {
			return RustGalWorldPrimitiveRenderer.staticTerrainSortedIndexSnapshot(meshKey);
		}

		private static TranslucentSortSnapshot currentTranslucentSortSnapshot(TerrainSectionAsset asset) {
			if (asset == null) {
				return null;
			}
			RustGalWorldPrimitiveRenderer.StaticTerrainSortedIndexSnapshot sortedIndex = sortedIndexSnapshot(asset.meshKey());
			if (sortedIndex != null) {
				return new TranslucentSortSnapshot(
					sortedIndex.meshGeneration(),
					sortedIndex.indexGeneration(),
					sortedIndex.indexType(),
					sortedIndex.indexBytes(),
					sortedIndex.indexHash()
				);
			}
			byte[] indexBytes = asset.asset().indexBytes();
			if (asset.initialSortGeneration() <= 0L || indexBytes.length == 0) {
				return null;
			}
			return new TranslucentSortSnapshot(
				asset.meshGeneration(),
				asset.initialSortGeneration(),
				asset.indexType(),
				indexBytes.length,
				sortedIndexHash(indexBytes)
			);
		}


	static long sortedIndexHash(byte[] indexBytes) {
		return fnv64Bytes(fnv64("static-terrain-translucent-sort-bytes-v1"), indexBytes == null ? new byte[0] : indexBytes);
	}

	private static long sortedIndexSampleHash(byte[] indexBytes) {
		if (indexBytes == null || indexBytes.length == 0) {
			return 0L;
		}
		int sampleBytes = Math.min(indexBytes.length, Integer.BYTES * 12);
		long hash = fnv64("static-terrain-translucent-sort-sample-v1");
		hash = fnv64Int(hash, sampleBytes);
		for (int i = 0; i < sampleBytes; i++) {
			hash ^= indexBytes[i] & 0xffL;
			hash *= 0x100000001b3L;
		}
		return hash == 0L ? 1L : hash;
	}

	private static String sortedIndexSample(byte[] indexBytes, int indexType, int maxIndices) {
		if (indexBytes == null || indexBytes.length == 0 || maxIndices <= 0) {
			return "";
		}
		int stride = indexType == INDEX_TYPE_U16 ? Short.BYTES : Integer.BYTES;
		int count = Math.min(maxIndices, indexBytes.length / stride);
		ByteBuffer buffer = ByteBuffer.wrap(indexBytes).order(ByteOrder.LITTLE_ENDIAN);
		StringBuilder sample = new StringBuilder();
		for (int index = 0; index < count; index++) {
			if (index > 0) {
				sample.append(',');
			}
			int value = indexType == INDEX_TYPE_U16
				? buffer.getShort(index * stride) & 0xffff
				: buffer.getInt(index * stride);
			sample.append(value);
		}
		return sample.toString();
	}

		private static String translucentSorterType(Sorter sorter) {
			if (sorter == null) {
				return "none";
			}
			if (sorter instanceof SharedIndexSorter) {
				return "shared-index";
			}
			return sorter.getClass().getName();
		}

		private static String initialTranslucentSorterType(ChunkBuildOutput output) {
			if (output == null || output.translucentData == null) {
				return "initial-build-index";
			}
			return "initial-build-index:"
				+ output.translucentData.getSortType().name().toLowerCase(Locale.ROOT)
				+ ":"
				+ output.translucentData.getClass().getSimpleName();
		}

	private static long sortedIndexGeneration(long meshKey, long meshGeneration, byte[] indexBytes) {
		long hash = fnv64("static-terrain-translucent-sort-v1");
		hash = fnv64Long(hash, meshKey);
		hash = fnv64Long(hash, meshGeneration);
		hash = fnv64Bytes(hash, indexBytes);
		return hash == 0L ? 1L : hash;
	}

	private static long meshKey(long sectionPos, ChunkSectionLayer layer) {
		long hash = fnv64("static-terrain-section-v1");
		hash = fnv64Long(hash, sectionPos);
		hash = fnv64Int(hash, layer.ordinal());
		return hash == 0L ? 1L : hash;
	}

	private static long meshGeneration(
		long sectionPos,
		ChunkSectionLayer layer,
		List<VulkanicGalBridge.WorldMeshVertexRecord> vertices,
		byte[] indexBytes,
		List<VulkanicGalBridge.WorldMeshSectionRecord> sections
	) {
		long hash = fnv64("static-terrain-generation-v1");
		hash = fnv64Long(hash, sectionPos);
		hash = fnv64Int(hash, layer.ordinal());
		hash = fnv64Int(hash, vertices.size());
		for (VulkanicGalBridge.WorldMeshVertexRecord vertex : vertices) {
			hash = fnv64Float(hash, vertex.x());
			hash = fnv64Float(hash, vertex.y());
			hash = fnv64Float(hash, vertex.z());
			hash = fnv64Float(hash, vertex.u());
			hash = fnv64Float(hash, vertex.v());
			hash = fnv64Float(hash, vertex.atlasU());
			hash = fnv64Float(hash, vertex.atlasV());
			hash = fnv64Int(hash, vertex.shaderBlockId());
			hash = fnv64Int(hash, vertex.shaderMaterialType());
				hash = fnv64Int(hash, vertex.colorArgb());
				hash = fnv64Int(hash, vertex.normalPacked());
				hash = fnv64Int(hash, vertex.light());
				// The compact Rust-owned voxel source retains this terrain semantic
				// alongside positions and material IDs. A rebuild that changes it
				// must therefore advance the shared mesh generation as well.
				hash = fnv64Int(hash, vertex.midBlockPacked());
			}
		hash = fnv64Int(hash, sections.size());
		for (VulkanicGalBridge.WorldMeshSectionRecord section : sections) {
			hash = fnv64Int(hash, section.materialId());
			hash = fnv64Int(hash, section.textureId());
			hash = fnv64Int(hash, section.materialMode());
			hash = fnv64Int(hash, section.cullPolicy());
			hash = fnv64Int(hash, section.winding());
			hash = fnv64Int(hash, section.indexOffset());
			hash = fnv64Int(hash, section.indexCount());
		}
		hash = fnv64Bytes(hash, indexBytes);
		return hash == 0L ? 1L : hash;
	}

	static List<VulkanicGalBridge.WorldMeshTextureAssetRecord> atlasTextureUpdatePayload(WaterTextureBinding waterBinding) {
		byte[] payload = atlasPayload;
		long generation = atlasGeneration;
		if (payload == null || registeredAtlasGeneration == generation) {
			return List.of();
		}
		synchronized (RustGalTerrainRenderer.class) {
			if (atlasPayload == null || registeredAtlasGeneration == atlasGeneration) {
				return List.of();
			}
			texturePayloadUpdates.incrementAndGet();
			texturePayloadUpdateBytes.addAndGet(atlasPayloadByteCount(atlasPayload, atlasMipPayloads));
			ArrayList<VulkanicGalBridge.WorldMeshTextureAssetRecord> records = new ArrayList<>(4);
			// This encoder emits the already-normalized row order consumed by the
			// Rust terrain UV contract.  It must agree with the eager whole-frame
			// publisher above: the same semantic atlas cannot acquire a different
			// origin merely because a section build happens first.
			records.add(new VulkanicGalBridge.WorldMeshTextureAssetRecord(
				RustGalWorldPrimitiveRenderer.MATERIAL_TEXTURE_TERRAIN_BLOCK_ATLAS,
				atlasPayload,
				atlasMipPayloads
			));
			if (normalAtlasPayload != null) {
				records.add(new VulkanicGalBridge.WorldMeshTextureAssetRecord(
					RustGalWorldPrimitiveRenderer.MATERIAL_TEXTURE_TERRAIN_BLOCK_NORMAL_ATLAS,
					normalAtlasPayload
				));
			}
			if (specularAtlasPayload != null) {
				records.add(new VulkanicGalBridge.WorldMeshTextureAssetRecord(
					RustGalWorldPrimitiveRenderer.MATERIAL_TEXTURE_TERRAIN_BLOCK_SPECULAR_ATLAS,
					specularAtlasPayload
				));
			}
			// Atlas-bound water shares the explicit terrain resource. Do not
			// publish unused separate sheets into the Rust GPU resource registry.
			if (waterBinding == WaterTextureBinding.SEPARATE_SHEETS && waterStillAsset != null) {
				records.add(waterStillAsset.textureRecord());
			}
			if (waterBinding == WaterTextureBinding.SEPARATE_SHEETS && waterFlowAsset != null) {
				records.add(waterFlowAsset.textureRecord());
			}
			if (waterBinding == WaterTextureBinding.SEPARATE_SHEETS && waterOverlayAsset != null) {
				records.add(waterOverlayAsset.textureRecord());
			}
			return records;
		}
	}

	/** Commits atlas-generation residency only after Rust accepted a mesh batch. */
	private static void confirmAtlasPayloadRegistered(long generation) {
		synchronized (RustGalTerrainRenderer.class) {
			if (atlasGeneration == generation) {
				registeredAtlasGeneration = generation;
			}
		}
	}

	private static boolean isPngPayload(byte[] payload) {
		return payload != null
			&& payload.length >= 8
			&& (payload[0] & 0xff) == 0x89
			&& payload[1] == 0x50
			&& payload[2] == 0x4e
			&& payload[3] == 0x47
			&& payload[4] == 0x0d
			&& payload[5] == 0x0a
			&& payload[6] == 0x1a
			&& payload[7] == 0x0a;
	}

	private static void removeLayer(long sectionPos, ChunkSectionLayer layer, String reason) {
		TerrainSectionAsset removed = SECTION_ASSETS.remove(new LayerKey(sectionPos, layer));
		if (removed != null) {
			if ("world-unload".equals(reason)) {
				lastWorldUnloadAsset = removed;
				lastWorldUnloadSectionPos = sectionPos;
				lastWorldUnloadLayer = layer;
			}
			removedLayers.incrementAndGet();
			TRANSLUCENT_EXECUTION_METADATA.remove(removed.meshKey());
			RustGalWorldPrimitiveRenderer.removeStaticTerrainMeshAsset(removed.meshKey());
				recordEvent(sectionPos, layer, 0L, removed.meshGeneration(), 0L, atlasGeneration, removed, 0, 0, 0, 0.0F, 0.0F, 0.0F, reason);
		}
	}

	private static void ensureAtlasPayload() {
		TextureAtlas atlas = Minecraft.getInstance().getAtlasManager().getAtlasOrThrow(AtlasIds.BLOCKS);
		long atlasPixels = (long) atlas.width * atlas.height;
		if (atlas.width <= 0 || atlas.height <= 0) {
			throw new IllegalStateException("Rust terrain atlas is not uploaded: " + atlas.width + "x" + atlas.height);
		}
		if (atlasPixels > MAX_RUST_ATLAS_PIXELS) {
			throw new IllegalStateException("Rust terrain atlas pixel bound exceeded " + MAX_RUST_ATLAS_PIXELS
				+ " extent=" + atlas.width + "x" + atlas.height);
		}
		long semanticGeneration = atlas.semanticReloadGeneration();
		if (semanticGeneration <= 0L) return;
		long semanticFrameKey = atlas.semanticSnapshotFrameKey();
		if (atlasPayload != null && copiedAtlasSemanticGeneration == semanticGeneration
			&& copiedAtlasSemanticFrameKey == semanticFrameKey) return;
		synchronized (RustGalTerrainRenderer.class) {
			semanticFrameKey = atlas.semanticSnapshotFrameKey();
			if (atlasPayload != null && copiedAtlasSemanticGeneration == semanticGeneration
				&& copiedAtlasSemanticFrameKey == semanticFrameKey) return;
				try {
					// Keep the large base image's lifetime confined to this helper.  The
					// derived PBR atlases are independently bounded images; retaining the
					// base BufferedImage across their allocations can exhaust a small
					// capture heap even when each individual atlas is within its limit.
					TextureAtlas.SemanticRawSnapshot semanticRawSnapshot = atlas.semanticRawSnapshot();
					if (semanticRawSnapshot == null
						|| semanticRawSnapshot.generation() != atlas.semanticSnapshotGeneration()
						|| semanticRawSnapshot.width() != atlas.width
						|| semanticRawSnapshot.height() != atlas.height) {
						throw new IllegalStateException("Rust terrain atlas semantic snapshot is unavailable or incoherent");
					}
					// Re-read after snapshot construction. Animation can advance on the
					// client tick while this bounded CPU copy is being built; never tag a
					// payload with a key that describes a different selected frame.
					long snapshotFrameKey = atlas.semanticSnapshotFrameKey();
					if (snapshotFrameKey != semanticFrameKey) {
						semanticRawSnapshot = atlas.semanticRawSnapshot();
						snapshotFrameKey = atlas.semanticSnapshotFrameKey();
						if (semanticRawSnapshot == null
							|| semanticRawSnapshot.generation() != atlas.semanticSnapshotGeneration()
							|| snapshotFrameKey != atlas.semanticSnapshotFrameKey()) {
							throw new IllegalStateException("Rust terrain atlas semantic animation snapshot changed during copy");
						}
					}
					byte[] nextAtlasPayload = encodeSemanticAtlasPayload(semanticRawSnapshot);
					List<byte[]> nextAtlasMipPayloads = encodeSemanticAtlasMipPayloads(semanticRawSnapshot);
					var nextAnimationSource = WorldRenderRoutePolicy.currentStaticTerrainRoute().usesRustWholeFrameVulkan()
						? atlas.semanticAnimationResource() : null;
					if (nextAnimationSource != null && (nextAnimationSource.source().generation() != semanticRawSnapshot.generation()
						|| nextAnimationSource.source().width() != semanticRawSnapshot.width()
						|| nextAnimationSource.source().height() != semanticRawSnapshot.height())) {
						throw new IllegalStateException("Terrain atlas changed while copying animation declarations");
					}
					// Do not allocate full atlas-sized PBR images unless the active
					// resource manager actually exposes that semantic map.  A missing
					// map is a valid Rust resource state; constructing a default image
					// for every atlas needlessly duplicates tens of megabytes and can
					// exhaust the bounded capture heap during first world entry.
					byte[] nextNormalAtlasPayload = hasPbrResources(atlas, "_n")
						? buildPbrAtlasPayload(atlas, "_n", 0x7F7FFFFF) : null;
					byte[] nextSpecularAtlasPayload = hasPbrResources(atlas, "_s")
						? buildPbrAtlasPayload(atlas, "_s", 0x00000000) : null;
					FluidSpriteAsset nextWaterStillAsset = buildFluidSpriteAsset(
						atlas,
						"block/water_still",
						RustGalWorldPrimitiveRenderer.MATERIAL_TEXTURE_WATER_STILL
					);
					FluidSpriteAsset nextWaterFlowAsset = buildFluidSpriteAsset(
						atlas,
						"block/water_flow",
						RustGalWorldPrimitiveRenderer.MATERIAL_TEXTURE_WATER_FLOW
					);
					FluidSpriteAsset nextWaterOverlayAsset = buildFluidSpriteAsset(
						atlas,
						"block/water_overlay",
						RustGalWorldPrimitiveRenderer.MATERIAL_TEXTURE_WATER_OVERLAY
					);
					atlasPayload = nextAtlasPayload;
					atlasMipPayloads = nextAtlasMipPayloads;
					atlasAnimationSource = nextAnimationSource;
					normalAtlasPayload = nextNormalAtlasPayload;
					specularAtlasPayload = nextSpecularAtlasPayload;
					waterStillAsset = nextWaterStillAsset;
					waterFlowAsset = nextWaterFlowAsset;
					waterOverlayAsset = nextWaterOverlayAsset;
					copiedAtlasSemanticGeneration = semanticGeneration;
					copiedAtlasSemanticFrameKey = snapshotFrameKey;
					atlasGeneration++;
			} catch (RuntimeException | IOException error) {
				throw new IllegalStateException("Failed to build Rust-owned block atlas payload for static terrain", error);
			}
		}
	}

	private static byte[] buildBaseAtlasPayload(TextureAtlas atlas) throws IOException {
		BufferedImage image = new BufferedImage(atlas.width, atlas.height, BufferedImage.TYPE_INT_ARGB);
		for (TextureAtlasSprite sprite : atlas.texturesByName.values()) copySprite(image, sprite);
		try (ByteArrayOutputStream output = new ByteArrayOutputStream()) {
			ImageIO.write(image, "png", output);
			return output.toByteArray();
		}
	}

	private static byte[] encodeSemanticAtlasPayload(TextureAtlas.SemanticRawSnapshot snapshot) throws IOException {
		return encodeSemanticRgbaPng(snapshot.width(), snapshot.height(), snapshot.pixels());
	}

	private static List<byte[]> encodeSemanticAtlasMipPayloads(TextureAtlas.SemanticRawSnapshot snapshot) throws IOException {
		List<byte[]> rawLevels = snapshot.mipPixels();
		if (rawLevels.isEmpty()) {
			throw new IOException("semantic atlas mip chain is incomplete");
		}
		ArrayList<byte[]> encoded = new ArrayList<>(Math.max(0, rawLevels.size() - 1));
		for (int mip = 1; mip < rawLevels.size(); mip++) {
			encoded.add(encodeSemanticRgbaPng(
				Math.max(1, snapshot.width() >> mip), Math.max(1, snapshot.height() >> mip), rawLevels.get(mip)
			));
		}
		return List.copyOf(encoded);
	}

	private static long atlasPayloadByteCount(byte[] baseLevel, List<byte[]> mipLevels) {
		long total = baseLevel.length;
		for (byte[] mipLevel : mipLevels) {
			total = Math.addExact(total, mipLevel.length);
		}
		return total;
	}

	private static byte[] encodeSemanticRgbaPng(int width, int height, byte[] pixels) throws IOException {
		if (pixels.length != Math.multiplyExact(Math.multiplyExact(width, height), 4)) {
			throw new IOException("semantic RGBA extent does not match pixel payload");
		}
		BufferedImage image = new BufferedImage(width, height, BufferedImage.TYPE_INT_ARGB);
		for (int y = 0; y < height; y++) {
			for (int x = 0; x < width; x++) {
				int offset = (y * width + x) * 4;
				int argb = net.minecraft.util.ARGB.color(
					pixels[offset + 3] & 0xff,
					pixels[offset] & 0xff,
					pixels[offset + 1] & 0xff,
					pixels[offset + 2] & 0xff
				);
				image.setRGB(x, y, argb);
			}
		}
		try (ByteArrayOutputStream output = new ByteArrayOutputStream()) {
			ImageIO.write(image, "png", output);
			return output.toByteArray();
		}
	}

	private static FluidSpriteAsset buildFluidSpriteAsset(TextureAtlas atlas, String spritePath, int textureId) throws IOException {
		TextureAtlasSprite sprite = atlas.getSprite(ResourceLocation.fromNamespaceAndPath("minecraft", spritePath));
		if (sprite == null || sprite.contents() == null) {
			throw new IllegalStateException("Missing built-in water sprite " + spritePath);
		}
		SpriteContents contents = sprite.contents();
		SpriteContents.AnimatedTexture animation = contents.animatedTexture;
		int frameCount = 1;
		int frameTicks = 1;
		int animationFlags = 0;
		int frameRowSize = 0;
		int interpolationPolicy = 0;
		List<VulkanicGalBridge.WorldMeshAnimationFrameRecord> animationFrames = List.of();
		if (animation != null && !animation.frames.isEmpty()) {
			frameCount = animation.frames.size();
			frameTicks = animation.frames.get(0).time();
			frameRowSize = animation.frameRowSize;
			interpolationPolicy = animation.interpolateFrames() ? 1 : 0;
			ArrayList<VulkanicGalBridge.WorldMeshAnimationFrameRecord> frames = new ArrayList<>(animation.frames.size());
			for (SpriteContents.FrameInfo frame : animation.frames) {
				frames.add(new VulkanicGalBridge.WorldMeshAnimationFrameRecord(frame.index(), frame.time()));
			}
			animationFrames = List.copyOf(frames);
		}
		byte[] png = encodeNativeSpriteImage(contents.originalImage);
		return new FluidSpriteAsset(
			textureId,
			contents.name(),
			sprite.getU0(),
			sprite.getU1(),
			sprite.getV0(),
			sprite.getV1(),
			contents.width(),
			contents.height(),
			Math.max(1, frameCount),
			Math.max(1, frameTicks),
			animationFlags,
			Math.max(0, frameRowSize),
			interpolationPolicy,
			animationFrames,
			png,
			atlas.mipLevel > 1 ? atlas.mipLevel + 1 : 1
		);
	}

	private static byte[] encodeNativeSpriteImage(net.blaze3d.platform.NativeImage image) throws IOException {
		BufferedImage copy = new BufferedImage(image.getWidth(), image.getHeight(), BufferedImage.TYPE_INT_ARGB);
		for (int y = 0; y < image.getHeight(); y++) {
			for (int x = 0; x < image.getWidth(); x++) {
				copy.setRGB(x, y, image.getPixel(x, y));
			}
		}
		try (ByteArrayOutputStream output = new ByteArrayOutputStream()) {
			ImageIO.write(copy, "png", output);
			return output.toByteArray();
		}
	}

	private static void copySprite(BufferedImage atlasImage, TextureAtlasSprite sprite) {
		SpriteContents contents = sprite.contents();
		int sourceX = 0;
		int sourceY = 0;
		if (contents.animatedTexture != null) {
			int frame = contents.semanticFrameIndex();
			sourceX = contents.animatedTexture.getFrameX(frame) * contents.width();
			sourceY = contents.animatedTexture.getFrameY(frame) * contents.height();
		}
		for (int y = 0; y < sprite.contents().height(); y++) {
			for (int x = 0; x < sprite.contents().width(); x++) {
				atlasImage.setRGB(sprite.getX() + x, sprite.getY() + y,
					contents.originalImage.getPixel(sourceX + x, sourceY + y));
			}
		}
	}

	private static long rgbaHash(net.blaze3d.platform.NativeImage image, int originX, int originY, int width, int height) {
		long hash = 0xcbf29ce484222325L;
		for (int y = 0; y < height; y++) {
			for (int x = 0; x < width; x++) {
				hash = fnv64Rgba(hash, image.getPixel(originX + x, originY + y));
			}
		}
		return hash;
	}

	private static long rgbaHash(BufferedImage image, int originX, int originY, int width, int height) {
		if (originX < 0 || originY < 0 || width < 0 || height < 0
			|| originX + width > image.getWidth() || originY + height > image.getHeight()) {
			throw new IllegalArgumentException("atlas receipt region is outside the decoded atlas");
		}
		long hash = 0xcbf29ce484222325L;
		for (int y = 0; y < height; y++) {
			for (int x = 0; x < width; x++) {
				hash = fnv64Rgba(hash, image.getRGB(originX + x, originY + y));
			}
		}
		return hash;
	}

	/**
	 * Builds an atlas-aligned semantic PBR payload directly from resolved
	 * resource-pack files. This intentionally does not query Iris PBR atlas
	 * objects or their GL textures. Missing sprites retain the source-defined
	 * default value for the requested semantic map.
	 */
	private static byte[] buildPbrAtlasPayload(TextureAtlas atlas, String suffix, int defaultArgb) throws IOException {
		long pixels = (long) atlas.width * atlas.height;
		if (atlas.width <= 0 || atlas.height <= 0 || pixels > MAX_RUST_ATLAS_PIXELS) {
			throw new IOException("Rust PBR atlas pixel bound exceeded " + MAX_RUST_ATLAS_PIXELS);
		}
		BufferedImage image = new BufferedImage(atlas.width, atlas.height, BufferedImage.TYPE_INT_ARGB);
		if (defaultArgb != 0) {
			java.awt.Graphics2D graphics = image.createGraphics();
			try {
				graphics.setColor(new java.awt.Color(defaultArgb, true));
				graphics.fillRect(0, 0, atlas.width, atlas.height);
			} finally {
				graphics.dispose();
			}
		}
		for (TextureAtlasSprite sprite : atlas.texturesByName.values()) {
			copyPbrSprite(image, sprite, suffix);
		}
		try (ByteArrayOutputStream output = new ByteArrayOutputStream()) {
			ImageIO.write(image, "png", output);
			return output.toByteArray();
		}
	}

	private static boolean hasPbrResources(TextureAtlas atlas, String suffix) {
		for (TextureAtlasSprite sprite : atlas.texturesByName.values()) {
			ResourceLocation spriteName = sprite.contents().name();
			String pbrPath = appendPbrSuffix(spriteName.getPath(), suffix);
			ResourceLocation pbrLocation = pbrPath.startsWith("optifine/cit/")
				? ResourceLocation.fromNamespaceAndPath(spriteName.getNamespace(), pbrPath + ".png")
				: ResourceLocation.fromNamespaceAndPath(spriteName.getNamespace(), "textures/" + pbrPath + ".png");
			if (Minecraft.getInstance().getResourceManager().getResource(pbrLocation).isPresent()) return true;
		}
		return false;
	}

	private static void copyPbrSprite(BufferedImage atlasImage, TextureAtlasSprite sprite, String suffix) throws IOException {
		ResourceLocation spriteName = sprite.contents().name();
		String pbrPath = appendPbrSuffix(spriteName.getPath(), suffix);
		ResourceLocation pbrLocation = pbrPath.startsWith("optifine/cit/")
			? ResourceLocation.fromNamespaceAndPath(spriteName.getNamespace(), pbrPath + ".png")
			: ResourceLocation.fromNamespaceAndPath(spriteName.getNamespace(), "textures/" + pbrPath + ".png");
		Optional<Resource> resource = Minecraft.getInstance().getResourceManager().getResource(pbrLocation);
		if (resource.isEmpty()) {
			return;
		}
		try (InputStream input = resource.get().open()) {
			BufferedImage source = ImageIO.read(input);
			if (source == null || source.getWidth() <= 0 || source.getHeight() <= 0) {
				throw new IOException("Unable to decode PBR sprite " + pbrLocation);
			}
			int targetWidth = sprite.contents().width();
			int targetHeight = sprite.contents().height();
			for (int y = 0; y < targetHeight; y++) {
				int sourceY = Math.min(source.getHeight() - 1, (int)((long)y * source.getHeight() / targetHeight));
				for (int x = 0; x < targetWidth; x++) {
					int sourceX = Math.min(source.getWidth() - 1, (int)((long)x * source.getWidth() / targetWidth));
					atlasImage.setRGB(sprite.getX() + x, sprite.getY() + y, source.getRGB(sourceX, sourceY));
				}
			}
		}
	}

	static int terrainDepthPolicy(ChunkSectionLayer layer) {
		return layer.pipeline().isWriteDepth()
			? RustGalWorldPrimitiveRenderer.DEPTH_POLICY_TEST_WRITE
			: RustGalWorldPrimitiveRenderer.DEPTH_POLICY_TEST_NO_WRITE;
	}

	static boolean terrainCameraSortRequested(ChunkSectionLayer layer) {
		// Blend ordering is independent of whether the layer writes depth.
		return layer == ChunkSectionLayer.TRANSLUCENT;
	}

	static String appendPbrSuffix(String path, String suffix) {
		int extension = path.lastIndexOf('.');
		return extension < 0 ? path + suffix : path.substring(0, extension) + suffix + path.substring(extension);
	}

	private static long fnv64(String value) {
		long hash = 0xcbf29ce484222325L;
		for (int i = 0; i < value.length(); i++) {
			hash ^= value.charAt(i);
			hash *= 0x100000001b3L;
		}
		return hash;
	}

	private static long fnv64Long(long hash, long value) {
		hash = fnv64Int(hash, (int)value);
		return fnv64Int(hash, (int)(value >>> 32));
	}

	private static long fnv64Int(long hash, int value) {
		for (int shift = 0; shift < 32; shift += 8) {
			hash ^= (value >>> shift) & 0xffL;
			hash *= 0x100000001b3L;
		}
		return hash;
	}

	private static long fnv64Rgba(long hash, int argb) {
		hash ^= (argb >>> 16) & 0xffL;
		hash *= 0x100000001b3L;
		hash ^= (argb >>> 8) & 0xffL;
		hash *= 0x100000001b3L;
		hash ^= argb & 0xffL;
		hash *= 0x100000001b3L;
		hash ^= (argb >>> 24) & 0xffL;
		hash *= 0x100000001b3L;
		return hash;
	}

	private static long fnv64Float(long hash, float value) {
		return fnv64Int(hash, Float.floatToIntBits(value));
	}

	private static long fnv64Bytes(long hash, byte[] bytes) {
		hash = fnv64Int(hash, bytes.length);
		for (byte value : bytes) {
			hash ^= value & 0xffL;
			hash *= 0x100000001b3L;
		}
		return hash;
	}

	private static void recordEvent(
		long sectionPos,
		ChunkSectionLayer layer,
		long sourceGeneration,
		long meshGeneration,
		long visibleGeneration,
		long uploadGeneration,
		TerrainSectionAsset asset,
		int sectionOriginX,
		int sectionOriginY,
		int sectionOriginZ,
		float transformX,
		float transformY,
		float transformZ,
		String reason
	) {
		recordEvent(sectionPos, layer, sourceGeneration, meshGeneration, visibleGeneration, uploadGeneration, asset,
			sectionOriginX, sectionOriginY, sectionOriginZ, transformX, transformY, transformZ, reason, 0L, 0L, 0L, 0L);
	}

	private static void recordEvent(
		long sectionPos,
		ChunkSectionLayer layer,
		long sourceGeneration,
		long meshGeneration,
		long visibleGeneration,
		long uploadGeneration,
		TerrainSectionAsset asset,
		int sectionOriginX,
		int sectionOriginY,
		int sectionOriginZ,
		float transformX,
		float transformY,
		float transformZ,
		String reason,
		long terrainExtractionFrameId,
		long rustEnqueueFrameId,
		long executionFrameId,
		long executionSubmissionId
	) {
		recordEvent(sectionPos, layer, sourceGeneration, meshGeneration, visibleGeneration, uploadGeneration, asset,
			sectionOriginX, sectionOriginY, sectionOriginZ, transformX, transformY, transformZ, reason,
			terrainExtractionFrameId, rustEnqueueFrameId, executionFrameId, executionSubmissionId,
			0L, 0.0D, 0.0D, 0.0D, 0.0D, 0.0D, 0.0D, 0, 0L, 0L, 0);
	}

	private static void recordEvent(
		long sectionPos,
		ChunkSectionLayer layer,
		long sourceGeneration,
		long meshGeneration,
		long visibleGeneration,
		long uploadGeneration,
		TerrainSectionAsset asset,
		int sectionOriginX,
		int sectionOriginY,
		int sectionOriginZ,
		float transformX,
		float transformY,
		float transformZ,
		String reason,
		long terrainExtractionFrameId,
		long rustEnqueueFrameId,
		long executionFrameId,
		long executionSubmissionId,
		long sortGeneration,
		double cameraX,
		double cameraY,
		double cameraZ,
		double sortOriginX,
		double sortOriginY,
		double sortOriginZ,
		int primitiveCount,
		long sortedIndexHash,
		long indexUploadGeneration,
		int translucentDrawOrder
	) {
		recordEvent(
			sectionPos,
			layer,
			sourceGeneration,
			meshGeneration,
			visibleGeneration,
			uploadGeneration,
			asset,
			sectionOriginX,
			sectionOriginY,
			sectionOriginZ,
			transformX,
			transformY,
			transformZ,
			reason,
			terrainExtractionFrameId,
			rustEnqueueFrameId,
			executionFrameId,
			executionSubmissionId,
			sortGeneration,
			cameraX,
			cameraY,
			cameraZ,
			sortOriginX,
			sortOriginY,
			sortOriginZ,
			primitiveCount,
			sortedIndexHash,
			indexUploadGeneration,
			translucentDrawOrder,
			"",
			0L,
			0L,
			0L,
			0L,
			""
		);
	}

	private static void recordEvent(
		long sectionPos,
		ChunkSectionLayer layer,
		long sourceGeneration,
		long meshGeneration,
		long visibleGeneration,
		long uploadGeneration,
		TerrainSectionAsset asset,
		int sectionOriginX,
		int sectionOriginY,
		int sectionOriginZ,
		float transformX,
		float transformY,
		float transformZ,
		String reason,
		long terrainExtractionFrameId,
		long rustEnqueueFrameId,
		long executionFrameId,
		long executionSubmissionId,
		long sortGeneration,
		double cameraX,
		double cameraY,
		double cameraZ,
		double sortOriginX,
		double sortOriginY,
		double sortOriginZ,
		int primitiveCount,
		long sortedIndexHash,
		long indexUploadGeneration,
		int translucentDrawOrder,
		String sorterType,
		long sourceSortedIndexHash,
		long rustCopiedSortedIndexHash,
		long sourceSortedIndexSampleHash,
		long rustCopiedSortedIndexSampleHash,
		String sortedIndexSample
	) {
		if (layer == ChunkSectionLayer.TRANSLUCENT
				&& "mesh-registered".equals(reason)
				&& asset != null
				&& !asset.translucentPrimitiveAccountingReason().isEmpty()) {
			reason = asset.translucentPrimitiveAccountingReason();
		}
		synchronized (RECENT_EVENTS) {
			TerrainDiagnosticEvent event = new TerrainDiagnosticEvent(
				diagnosticGameplayFrameId(terrainExtractionFrameId, rustEnqueueFrameId, executionFrameId),
				terrainExtractionFrameId,
				rustEnqueueFrameId,
				executionFrameId,
				executionSubmissionId,
				sectionPos,
				layer.name(),
				sourceGeneration,
				meshGeneration,
				visibleGeneration,
				uploadGeneration,
				asset == null ? 0L : asset.meshKey(),
				asset == null ? 0L : asset.contentHash(),
				asset == null ? 0 : asset.vertexCount(),
				asset == null ? 0 : asset.bufferVertexCapacity(),
				asset == null ? 0 : asset.vertexStride(),
				asset == null ? 0 : asset.indexCount(),
				asset == null ? -1 : asset.maxIndex(),
				asset == null ? 0 : asset.indexType(),
				asset == null ? 0 : asset.sectionCount(),
				sectionOriginX,
				sectionOriginY,
				sectionOriginZ,
				transformX,
				transformY,
				transformZ,
				asset == null ? 0.0F : asset.localMinX(),
				asset == null ? 0.0F : asset.localMinY(),
				asset == null ? 0.0F : asset.localMinZ(),
				asset == null ? 0.0F : asset.localMaxX(),
				asset == null ? 0.0F : asset.localMaxY(),
				asset == null ? 0.0F : asset.localMaxZ(),
				asset == null ? 0.0F : asset.uvMinU(),
				asset == null ? 0.0F : asset.uvMinV(),
				asset == null ? 0.0F : asset.uvMaxU(),
				asset == null ? 0.0F : asset.uvMaxV(),
				asset == null || asset.vertexPositionsFinite(),
				asset == null || asset.localBoundsValid(),
				asset == null || asset.uvBoundsValid(),
				asset == null || asset.indexRangeValid(),
				asset == null || asset.segmentLayoutValid(),
				asset == null || asset.sectionOriginValid(),
				asset == null || asset.indexOffsetAlignmentValid(),
				finiteBounds(
					(asset == null ? 0.0F : asset.localMinX()) + transformX,
					(asset == null ? 0.0F : asset.localMinY()) + transformY,
					(asset == null ? 0.0F : asset.localMinZ()) + transformZ,
					(asset == null ? 0.0F : asset.localMaxX()) + transformX,
					(asset == null ? 0.0F : asset.localMaxY()) + transformY,
					(asset == null ? 0.0F : asset.localMaxZ()) + transformZ
				),
				asset == null || asset.normalContractValid(),
				asset == null || asset.aoContractValid(),
				asset == null || asset.blockSkyLightContractValid(),
				asset == null || asset.topFaceShadeContractValid(),
				asset != null && asset.separateAoActive(),
				asset == null ? 0 : asset.separateAoVertexCount(),
				asset == null ? 1.0F : asset.minAo(),
				asset == null ? 1.0F : asset.maxAo(),
				asset == null ? 0 : asset.positiveYNormalSections(),
				asset == null ? 0 : asset.negativeYNormalSections(),
				asset == null ? 0 : asset.horizontalNormalSections(),
				sortGeneration,
				cameraX,
				cameraY,
				cameraZ,
				sortOriginX,
				sortOriginY,
				sortOriginZ,
				primitiveCount,
				sortedIndexHash,
				indexUploadGeneration,
				translucentDrawOrder,
				sorterType == null ? "" : sorterType,
				sourceSortedIndexHash,
				rustCopiedSortedIndexHash,
				sourceSortedIndexSampleHash,
				rustCopiedSortedIndexSampleHash,
				sortedIndexSample == null ? "" : sortedIndexSample,
				reason
			);
			if (RECENT_EVENTS.size() >= MAX_RECENT_EVENTS) {
				RECENT_EVENTS.removeFirst();
			}
			RECENT_EVENTS.addLast(event);
			if (reason.startsWith("lifecycle-")) {
				if (LIFECYCLE_EVENTS.size() >= MAX_LIFECYCLE_EVENTS) {
					LIFECYCLE_EVENTS.removeFirst();
				}
				LIFECYCLE_EVENTS.addLast(event);
			}
			if (layer == ChunkSectionLayer.TRANSLUCENT || reason.startsWith("translucent-")) {
				if (TRANSLUCENT_EVENTS.size() >= MAX_TRANSLUCENT_EVENTS) {
					TRANSLUCENT_EVENTS.removeFirst();
				}
				TRANSLUCENT_EVENTS.addLast(event);
			}
		}
	}

	private static long currentGameplayFrameId() {
		long semanticFrame = RustGalWorldPrimitiveRenderer.currentSemanticFrameSequence();
		if (semanticFrame > 0L) {
			return semanticFrame;
		}
		return Math.max(
			net.minecraft.client.dev.GraphicsFrameBenchmark.currentFrameIndex(),
			net.minecraft.client.dev.DeterministicCameraCapture.currentRenderedFrameIndex()
		);
	}

	/**
	 * The benchmark/capture clocks begin after ordinary world rendering has already
	 * submitted terrain. Keep those earlier diagnostic events frame-distinct so the
	 * geometry gate does not mistake consecutive normal frames for a duplicate draw.
	 */
	private static long diagnosticGameplayFrameId(long terrainExtractionFrameId, long rustEnqueueFrameId, long executionFrameId) {
		long frameId = currentGameplayFrameId();
		if (frameId > 0L) {
			return frameId;
		}
		if (executionFrameId > 0L) {
			return executionFrameId;
		}
		if (rustEnqueueFrameId > 0L) {
			return rustEnqueueFrameId;
		}
		return terrainExtractionFrameId;
	}

	private static double currentCameraX() {
		try {
			return Minecraft.getInstance().gameRenderer.getMainCamera().getPosition().x();
		} catch (RuntimeException error) {
			return 0.0D;
		}
	}

	private static double currentCameraY() {
		try {
			return Minecraft.getInstance().gameRenderer.getMainCamera().getPosition().y();
		} catch (RuntimeException error) {
			return 0.0D;
		}
	}

	private static double currentCameraZ() {
		try {
			return Minecraft.getInstance().gameRenderer.getMainCamera().getPosition().z();
		} catch (RuntimeException error) {
			return 0.0D;
		}
	}

	private record LayerKey(long sectionPos, ChunkSectionLayer layer) {
	}

	private record VisibleSubmitKey(long sectionPos, ChunkSectionLayer layer, long generation) {
	}

	private record TerrainSectionAsset(
			long meshKey,
			long meshGeneration,
			long contentHash,
			long initialSortGeneration,
			int vertexCount,
		int bufferVertexCapacity,
		int vertexStride,
		int indexCount,
		int maxIndex,
		int indexType,
		int sectionCount,
		float localMinX,
		float localMinY,
		float localMinZ,
		float localMaxX,
		float localMaxY,
		float localMaxZ,
		float uvMinU,
		float uvMinV,
		float uvMaxU,
		float uvMaxV,
		boolean vertexPositionsFinite,
		boolean localBoundsValid,
		boolean uvBoundsValid,
		boolean indexRangeValid,
		boolean segmentLayoutValid,
		boolean sectionOriginValid,
		boolean indexOffsetAlignmentValid,
		boolean normalContractValid,
		boolean aoContractValid,
		boolean blockSkyLightContractValid,
		boolean topFaceShadeContractValid,
		boolean separateAoActive,
		int separateAoVertexCount,
		float minAo,
		float maxAo,
		int positiveYNormalSections,
		int negativeYNormalSections,
		int horizontalNormalSections,
		int sectionOriginX,
		int sectionOriginY,
		int sectionOriginZ,
		String translucentPrimitiveAccountingReason,
		int unsupportedPrimitiveCount,
		int[] translucentSourceSegmentQuadCounts,
			VulkanicGalBridge.WorldMeshAssetRecord asset
		) {
		TerrainSectionAsset releaseCpuPayload() {
			return new TerrainSectionAsset(
				meshKey, meshGeneration, contentHash, initialSortGeneration, vertexCount,
				bufferVertexCapacity, vertexStride, indexCount, maxIndex, indexType, sectionCount,
				localMinX, localMinY, localMinZ, localMaxX, localMaxY, localMaxZ,
				uvMinU, uvMinV, uvMaxU, uvMaxV, vertexPositionsFinite, localBoundsValid,
				uvBoundsValid, indexRangeValid, segmentLayoutValid, sectionOriginValid,
				indexOffsetAlignmentValid, normalContractValid, aoContractValid,
				blockSkyLightContractValid, topFaceShadeContractValid, separateAoActive,
				separateAoVertexCount, minAo, maxAo, positiveYNormalSections,
				negativeYNormalSections, horizontalNormalSections, sectionOriginX,
				sectionOriginY, sectionOriginZ, translucentPrimitiveAccountingReason,
				unsupportedPrimitiveCount, translucentSourceSegmentQuadCounts, null
			);
		}
	}

		private record TranslucentSortSnapshot(
			long meshGeneration,
			long sortGeneration,
			int indexType,
			int indexBytes,
			long indexHash
		) {
		}

	public record TerrainDiagnosticEvent(
		long gameplayFrameId,
		long terrainExtractionFrameId,
		long rustEnqueueFrameId,
		long executionFrameId,
		long executionSubmissionId,
		long sectionPos,
		String layer,
		long sourceGeneration,
		long meshGeneration,
		long visibleGeneration,
		long uploadGeneration,
		long meshKey,
		long contentHash,
		int vertexCount,
		int bufferVertexCapacity,
		int vertexStride,
		int indexCount,
		int maxIndex,
		int indexType,
		int sectionCount,
		int sectionOriginX,
		int sectionOriginY,
		int sectionOriginZ,
		float transformX,
		float transformY,
		float transformZ,
		float localMinX,
		float localMinY,
		float localMinZ,
		float localMaxX,
		float localMaxY,
		float localMaxZ,
		float uvMinU,
		float uvMinV,
		float uvMaxU,
		float uvMaxV,
		boolean vertexPositionsFinite,
		boolean localBoundsValid,
		boolean uvBoundsValid,
		boolean indexRangeValid,
		boolean segmentLayoutValid,
		boolean sectionOriginValid,
		boolean indexOffsetAlignmentValid,
		boolean cameraBoundsFinite,
		boolean normalContractValid,
		boolean aoContractValid,
		boolean blockSkyLightContractValid,
		boolean topFaceShadeContractValid,
		boolean separateAoActive,
		int separateAoVertexCount,
		float minAo,
		float maxAo,
			int positiveYNormalSections,
			int negativeYNormalSections,
			int horizontalNormalSections,
			long sortGeneration,
			double cameraX,
			double cameraY,
			double cameraZ,
			double sortOriginX,
			double sortOriginY,
			double sortOriginZ,
			int primitiveCount,
			long sortedIndexHash,
			long indexUploadGeneration,
			int translucentDrawOrder,
			String sorterType,
			long sourceSortedIndexHash,
			long rustCopiedSortedIndexHash,
			long sourceSortedIndexSampleHash,
			long rustCopiedSortedIndexSampleHash,
			String sortedIndexSample,
			String reason
		) {
		}

	public record TerrainDiagnostics(
		int cachedLayerAssets,
		int activeTerrainLayers,
		int activeSectionAssets,
		long currentFrameVisibleLayerSubmissions,
		long atlasGeneration,
		long registeredAtlasGeneration,
		int activeNativeVertexStride,
		int expectedNativeVertexStride,
		long acceptedBuildOutputs,
		long skippedRouteBuildOutputs,
		long skippedUnsupportedAnimatedSections,
		long skippedUnsupportedFluidTranslucentSections,
		long acceptedWaterAnimatedSections,
		long unsupportedFluidRejectedSections,
		long skippedEmptyLayers,
		long registeredMeshes,
		long registeredTranslucentSorts,
		long registeredTranslucentSortBytes,
		long translucentSortGenerations,
		long texturePayloadUpdates,
		long texturePayloadUpdateBytes,
		long atlasTextureOnlyUpdates,
		long atlasMissingPayloadUpdates,
		long atlasMalformedPayloadUpdates,
		long atlasPartialPayloadUpdates,
		long removedLayers,
		long visibleLayerProbes,
		long visibleLayerSubmissions,
		long failedLayerSubmissions,
			long invalidations,
			long terrainExtractionFrames,
			long rustEnqueueFrames,
			List<TerrainDiagnosticEvent> lifecycleEvents,
			List<TerrainDiagnosticEvent> translucentEvents,
			List<TerrainDiagnosticEvent> recentEvents
		) {
			public TerrainDiagnostics {
				lifecycleEvents = Collections.unmodifiableList(new ArrayList<>(lifecycleEvents));
				translucentEvents = Collections.unmodifiableList(new ArrayList<>(translucentEvents));
				recentEvents = Collections.unmodifiableList(new ArrayList<>(recentEvents));
			}
		}

	public record TerrainLayerSnapshot(
		long sectionPos,
		String layer,
		long meshKey,
		long meshGeneration,
		int vertexCount,
		int indexBytes,
		int sectionCount
	) {
	}

	public record StaticTerrainExecutionSnapshot(
		long frameId,
		long submissionId,
		long instances
	) {
		public boolean executedAfter(long submissionBaseline) {
			return instances > 0L && submissionId > submissionBaseline;
		}
	}

	public record TerrainAtlasReceipt(
		boolean available,
		String status,
		int width,
		int height,
		long copiedAtlasHash,
		List<TerrainAtlasSpriteReceipt> sprites
	) {
		static TerrainAtlasReceipt unavailable(String reason) {
			return new TerrainAtlasReceipt(false, reason == null ? "unknown atlas receipt failure" : reason, 0, 0, 0L, List.of());
		}

		public boolean allSpritesMatch() {
			return available && !sprites.isEmpty() && sprites.stream().allMatch(TerrainAtlasSpriteReceipt::matchesSource);
		}
	}

	public record TerrainAtlasSpriteReceipt(
		String identity,
		int x,
		int y,
		int width,
		int height,
		long sourceHash,
		long copiedHash,
		String directSampleIdentity,
		String mirroredVSampleIdentity,
		int sampleX,
		int sampleY,
		int mirroredSampleY,
		boolean matchesSource,
		String status
	) {
		static TerrainAtlasSpriteReceipt missing(String identity) {
			return new TerrainAtlasSpriteReceipt(
				identity,
				0,
				0,
				0,
				0,
				0L,
				0L,
				"<missing>",
				"<missing>",
				0,
				0,
				0,
				false,
				"missing"
			);
		}
	}

	/** Test-only semantic probe; no atlas object or native handle crosses this boundary. */
	public record TerrainTextureProbe(BlockPos position, List<ResourceLocation> allowedSprites) {
		public TerrainTextureProbe {
			allowedSprites = List.copyOf(allowedSprites == null ? List.of() : allowedSprites);
		}
	}

	public record TerrainTextureProbeReceipt(
		boolean matched,
		String status,
		List<TerrainTextureProbeResult> probes
	) {
	}

	public record TerrainTextureProbeResult(
		BlockPos position,
		List<ResourceLocation> allowedSprites,
		int matchingQuads,
		int mismatchedQuads,
		boolean matched,
		String status,
		List<TerrainTextureProbeObservation> observations
	) {
		public TerrainTextureProbeResult {
			allowedSprites = List.copyOf(allowedSprites);
			observations = List.copyOf(observations);
		}
	}

	/** Bounded capture-only UV evidence for one copied terrain quad. */
	public record TerrainTextureProbeObservation(
		long sectionPos,
		String layer,
		int quadIndex,
		boolean expectedSprite,
		String atlasIdentity,
		float atlasU,
		float atlasV
	) {
	}

	private record AtlasUvRegion(float u0, float v0, float u1, float v1) {
		boolean contains(float u, float v) {
			final float epsilon = 0.0001F;
			return Float.isFinite(u) && Float.isFinite(v)
				&& u >= u0 - epsilon && u <= u1 + epsilon
				&& v >= v0 - epsilon && v <= v1 + epsilon;
		}
	}
}
