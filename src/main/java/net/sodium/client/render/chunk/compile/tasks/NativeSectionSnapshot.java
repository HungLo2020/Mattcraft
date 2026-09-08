package net.sodium.client.render.chunk.compile.tasks;

import net.sodium.client.render.chunk.compile.ChunkBuildBuffers;
import net.sodium.client.render.chunk.compile.pipeline.NativeStaticBlockModelRegistry;
import net.sodium.client.render.chunk.compile.pipeline.BlockOcclusionCache;
import net.sodium.client.render.StaticTerrainParityDiagnostics;
import net.sodium.client.render.chunk.terrain.DefaultTerrainRenderPasses;
import net.sodium.client.render.chunk.terrain.TerrainRenderPass;
import net.sodium.client.render.chunk.translucent_sorting.TranslucentGeometryCollector;
import net.sodium.client.render.chunk.vertex.format.NativeChunkMeshEncoder;
import net.sodium.client.services.PlatformBlockAccess;
import net.sodium.client.world.LevelSlice;
import net.minecraft.client.Minecraft;
import net.minecraft.client.renderer.BiomeColors;
import net.minecraft.client.renderer.LevelRenderer;
import net.minecraft.client.renderer.LightTexture;
import net.minecraft.core.BlockPos;
import net.minecraft.core.Direction;
import net.minecraft.world.level.block.RenderShape;
import net.minecraft.world.level.LightLayer;
import net.minecraft.world.level.block.Block;
import net.minecraft.world.level.block.Blocks;
import net.minecraft.world.level.block.RedStoneWireBlock;
import net.minecraft.world.level.block.state.BlockState;
import net.minecraft.world.level.material.FluidState;
import net.minecraft.world.level.material.Fluids;
import org.lwjgl.system.MemoryUtil;

final class NativeSectionSnapshot implements AutoCloseable {
    private static final int SECTION_BLOCK_COUNT = 16 * 16 * 16;
    private static final int PADDED_LENGTH = NativeChunkMeshEncoder.COMPACT_SECTION_PADDED_LENGTH;
    private static final int PADDED_BLOCK_COUNT = NativeChunkMeshEncoder.COMPACT_SECTION_PADDED_BLOCK_COUNT;
    private static final int HEADER_VERSION_OFFSET = 0;
    private static final int HEADER_ACTIVE_COUNT_OFFSET = 4;
    private static final int HEADER_MIN_X_OFFSET = 8;
    private static final int HEADER_MIN_Y_OFFSET = 12;
    private static final int HEADER_MIN_Z_OFFSET = 16;
    private static final int HEADER_PADDING_OFFSET = 20;
    private static final int HEADER_ACTIVE_INDICES_ADDRESS_OFFSET = 24;
    private static final int HEADER_PADDED_STATE_IDS_ADDRESS_OFFSET = 32;
    private static final int HEADER_PADDED_LIGHT_WORDS_ADDRESS_OFFSET = 40;
    private static final int HEADER_BLOCK_IDS_ADDRESS_OFFSET = 48;
    private static final int HEADER_SEED_LOS_ADDRESS_OFFSET = 56;
    private static final int HEADER_SEED_HIS_ADDRESS_OFFSET = 64;
    private static final int HEADER_TINTS_ADDRESS_OFFSET = 72;
    private static final int HEADER_FLUID_TINTS_ADDRESS_OFFSET = 80;
    private static final int HEADER_FLUID_FLOW_X_ADDRESS_OFFSET = 88;
    private static final int HEADER_FLUID_FLOW_Z_ADDRESS_OFFSET = 96;
    private static final int HEADER_FLUID_BLOCK_IDS_ADDRESS_OFFSET = 104;
    private static final int HEADER_FLAGS_ADDRESS_OFFSET = 112;
    private static final int HEADER_TINT_LATTICES_ADDRESS_OFFSET = 120;
    // Resource-pack models may legitimately extend beyond the unit block
    // (for example Stay True's leaf planes reach -7..25 model units). Java's
    // BlendedColorProvider floors vertex - 0.5 and therefore needs samples
    // through +2 as well as -1. This is immutable semantic input for Rust,
    // not a renderer-side color decision.
    private static final int TINT_LATTICE_MIN_OFFSET = -1;
    private static final int TINT_LATTICE_MAX_OFFSET = 2;
    private static final int TINT_LATTICE_DIMENSION = TINT_LATTICE_MAX_OFFSET - TINT_LATTICE_MIN_OFFSET + 1;
    private static final int TINT_LATTICE_SAMPLE_COUNT = TINT_LATTICE_DIMENSION * TINT_LATTICE_DIMENSION * TINT_LATTICE_DIMENSION;
    private static final int SEMANTIC_CULL_MASK_SHIFT = 8;

    private final ChunkBuildBuffers buffers;
    private final int sectionIndex;
    private final int minX;
    private final int minY;
    private final int minZ;
    private final int modelReloadGeneration;
    private final BlockOcclusionCache modelOcclusionCache = new BlockOcclusionCache();
    private final long totalBytes;
    private long address;
    private long activeIndicesAddress;
    private long paddedStateIdsAddress;
    private long paddedLightWordsAddress;
    private long blockIdsAddress;
    private long seedLosAddress;
    private long seedHisAddress;
    private long tintsAddress;
    private long fluidTintsAddress;
    private long fluidFlowXAddress;
    private long fluidFlowZAddress;
    private long fluidBlockIdsAddress;
    private long flagsAddress;
    private long tintLatticesAddress;
    private int activeRecordCount;

    NativeSectionSnapshot(ChunkBuildBuffers buffers, int sectionIndex, int minX, int minY, int minZ,
            LevelSlice slice) {
        this.buffers = buffers;
        this.sectionIndex = sectionIndex;
        this.minX = minX;
        this.minY = minY;
        this.minZ = minZ;
        this.modelReloadGeneration = NativeStaticBlockModelRegistry.reloadGeneration();

        long offset = NativeChunkMeshEncoder.COMPACT_SECTION_SNAPSHOT_HEADER_STRIDE;
        this.activeIndicesAddress = offset;
        offset += (long) SECTION_BLOCK_COUNT * NativeChunkMeshEncoder.COMPACT_SECTION_ACTIVE_INDEX_STRIDE;
        offset = align(offset, Integer.BYTES);
        this.paddedStateIdsAddress = offset;
        offset += (long) PADDED_BLOCK_COUNT * Integer.BYTES;
        this.paddedLightWordsAddress = offset;
        offset += (long) PADDED_BLOCK_COUNT * Integer.BYTES;
        this.blockIdsAddress = offset;
        offset += (long) SECTION_BLOCK_COUNT * Integer.BYTES;
        this.seedLosAddress = offset;
        offset += (long) SECTION_BLOCK_COUNT * Integer.BYTES;
        this.seedHisAddress = offset;
        offset += (long) SECTION_BLOCK_COUNT * Integer.BYTES;
        this.tintsAddress = offset;
        offset += (long) SECTION_BLOCK_COUNT * Integer.BYTES;
        this.fluidTintsAddress = offset;
        offset += (long) SECTION_BLOCK_COUNT * Integer.BYTES;
        this.fluidFlowXAddress = offset;
        offset += (long) SECTION_BLOCK_COUNT * Float.BYTES;
        this.fluidFlowZAddress = offset;
        offset += (long) SECTION_BLOCK_COUNT * Float.BYTES;
        this.fluidBlockIdsAddress = offset;
        offset += (long) SECTION_BLOCK_COUNT * Integer.BYTES;
        this.flagsAddress = offset;
        offset += (long) SECTION_BLOCK_COUNT * Integer.BYTES;
        this.tintLatticesAddress = offset;
        offset += (long) SECTION_BLOCK_COUNT * TINT_LATTICE_SAMPLE_COUNT * Integer.BYTES;
        this.totalBytes = align(offset, Long.BYTES);

        this.address = MemoryUtil.nmemCalloc(1L, this.totalBytes);
        if (this.address == 0L) {
            throw new OutOfMemoryError("Could not allocate native section snapshot");
        }
        this.rebaseAddresses();
        this.writeHeader();
        this.populatePaddedGrids(slice);
    }

    void appendBlock(int localBlockIndex, LevelSlice slice, BlockState blockState, BlockPos blockPos,
            int localX, int localY, int localZ, boolean suppressNativeFluid) {
        long seed = blockState.getSeed(blockPos);
        int activeIndex = this.activeRecordCount++;
        if (activeIndex < 0 || activeIndex >= SECTION_BLOCK_COUNT) {
            throw new IllegalArgumentException("Invalid active section block index: " + activeIndex);
        }
        if (localBlockIndex < 0 || localBlockIndex >= SECTION_BLOCK_COUNT) {
            throw new IllegalArgumentException("Invalid section block index: " + localBlockIndex);
        }

        FluidState fluidState = blockState.getFluidState();
        var flow = fluidState.isEmpty() ? net.minecraft.world.phys.Vec3.ZERO : fluidState.getFlow(slice, blockPos);
        int tint = blockTint(slice, blockState, blockPos);
        int fluidTint = fluidTint(slice, fluidState, blockPos);

        MemoryUtil.memPutShort(this.activeIndicesAddress
                + (long) activeIndex * NativeChunkMeshEncoder.COMPACT_SECTION_ACTIVE_INDEX_STRIDE,
                (short) localBlockIndex);
        MemoryUtil.memPutInt(this.blockIdsAddress + (long) localBlockIndex * Integer.BYTES, irisBlockId(blockState));
        MemoryUtil.memPutInt(this.seedLosAddress + (long) localBlockIndex * Integer.BYTES, (int) seed);
        MemoryUtil.memPutInt(this.seedHisAddress + (long) localBlockIndex * Integer.BYTES, (int) (seed >>> 32));
        MemoryUtil.memPutInt(this.tintsAddress + (long) localBlockIndex * Integer.BYTES, tint);
        this.writeTintLattice(localBlockIndex, slice, blockState, blockPos);
        this.recordTintSource(localBlockIndex, blockState, blockPos, tint);
        MemoryUtil.memPutInt(this.fluidTintsAddress + (long) localBlockIndex * Integer.BYTES, fluidTint);
        MemoryUtil.memPutFloat(this.fluidFlowXAddress + (long) localBlockIndex * Float.BYTES, (float) flow.x);
        MemoryUtil.memPutFloat(this.fluidFlowZAddress + (long) localBlockIndex * Float.BYTES, (float) flow.z);
        if (!fluidState.isEmpty()) {
            MemoryUtil.memPutInt(this.fluidBlockIdsAddress + (long) localBlockIndex * Integer.BYTES,
                    irisFluidBlockId(fluidState));
        }
        int flags = suppressNativeFluid
                ? NativeChunkMeshEncoder.NATIVE_SECTION_BLOCK_FLAG_SUPPRESS_FLUID
                : 0;
        flags |= NativeChunkMeshEncoder.NATIVE_SECTION_BLOCK_FLAG_TINT_LATTICE;
        if (blockState.getRenderShape() == RenderShape.MODEL
                && NativeStaticBlockModelRegistry.hasNativeModel(blockState)) {
            flags |= this.modelCullMask(slice, blockState, blockPos) << SEMANTIC_CULL_MASK_SHIFT;
        }
        MemoryUtil.memPutInt(this.flagsAddress + (long) localBlockIndex * Integer.BYTES, flags);
    }

    int[] flushAll(TranslucentGeometryCollector collector) {
        // The compact snapshot stores ids into Rust's model/state tables. A model reload clears those
        // native tables, so the snapshot must fail before Rust can interpret ids from an old generation.
        if (this.modelReloadGeneration != NativeStaticBlockModelRegistry.reloadGeneration()) {
            throw new IllegalStateException("Native section snapshot was built against stale native model metadata");
        }
        MemoryUtil.memPutInt(this.address + HEADER_ACTIVE_COUNT_OFFSET, this.activeRecordCount);
        int[] nativeQuads = this.buffers.appendCompactNativeSectionSnapshotAllPasses(this.address,
                this.sectionIndex, collector);
        this.addNativeFluidSprites(DefaultTerrainRenderPasses.SOLID);
        this.addNativeFluidSprites(DefaultTerrainRenderPasses.CUTOUT);
        this.addNativeFluidSprites(DefaultTerrainRenderPasses.TRANSLUCENT);
        return nativeQuads;
    }

    private void addNativeFluidSprites(TerrainRenderPass pass) {
        int emittedSpriteMask = this.buffers.nativeFluidSpriteMask(pass);
        for (var sprite : NativeStaticBlockModelRegistry.getNativeFluidSprites(
                emittedSpriteMask)) {
            this.buffers.get(pass).addSprite(sprite);
        }
    }

    private void rebaseAddresses() {
        this.activeIndicesAddress += this.address;
        this.paddedStateIdsAddress += this.address;
        this.paddedLightWordsAddress += this.address;
        this.blockIdsAddress += this.address;
        this.seedLosAddress += this.address;
        this.seedHisAddress += this.address;
        this.tintsAddress += this.address;
        this.fluidTintsAddress += this.address;
        this.fluidFlowXAddress += this.address;
        this.fluidFlowZAddress += this.address;
        this.fluidBlockIdsAddress += this.address;
        this.flagsAddress += this.address;
        this.tintLatticesAddress += this.address;
    }

    private void writeHeader() {
        MemoryUtil.memPutInt(this.address + HEADER_VERSION_OFFSET,
                NativeChunkMeshEncoder.COMPACT_SECTION_SNAPSHOT_VERSION);
        MemoryUtil.memPutInt(this.address + HEADER_ACTIVE_COUNT_OFFSET, 0);
        MemoryUtil.memPutInt(this.address + HEADER_MIN_X_OFFSET, this.minX);
        MemoryUtil.memPutInt(this.address + HEADER_MIN_Y_OFFSET, this.minY);
        MemoryUtil.memPutInt(this.address + HEADER_MIN_Z_OFFSET, this.minZ);
        MemoryUtil.memPutInt(this.address + HEADER_PADDING_OFFSET, 0);
        MemoryUtil.memPutLong(this.address + HEADER_ACTIVE_INDICES_ADDRESS_OFFSET, this.activeIndicesAddress);
        MemoryUtil.memPutLong(this.address + HEADER_PADDED_STATE_IDS_ADDRESS_OFFSET, this.paddedStateIdsAddress);
        MemoryUtil.memPutLong(this.address + HEADER_PADDED_LIGHT_WORDS_ADDRESS_OFFSET, this.paddedLightWordsAddress);
        MemoryUtil.memPutLong(this.address + HEADER_BLOCK_IDS_ADDRESS_OFFSET, this.blockIdsAddress);
        MemoryUtil.memPutLong(this.address + HEADER_SEED_LOS_ADDRESS_OFFSET, this.seedLosAddress);
        MemoryUtil.memPutLong(this.address + HEADER_SEED_HIS_ADDRESS_OFFSET, this.seedHisAddress);
        MemoryUtil.memPutLong(this.address + HEADER_TINTS_ADDRESS_OFFSET, this.tintsAddress);
        MemoryUtil.memPutLong(this.address + HEADER_FLUID_TINTS_ADDRESS_OFFSET, this.fluidTintsAddress);
        MemoryUtil.memPutLong(this.address + HEADER_FLUID_FLOW_X_ADDRESS_OFFSET, this.fluidFlowXAddress);
        MemoryUtil.memPutLong(this.address + HEADER_FLUID_FLOW_Z_ADDRESS_OFFSET, this.fluidFlowZAddress);
        MemoryUtil.memPutLong(this.address + HEADER_FLUID_BLOCK_IDS_ADDRESS_OFFSET, this.fluidBlockIdsAddress);
        MemoryUtil.memPutLong(this.address + HEADER_FLAGS_ADDRESS_OFFSET, this.flagsAddress);
        MemoryUtil.memPutLong(this.address + HEADER_TINT_LATTICES_ADDRESS_OFFSET, this.tintLatticesAddress);
    }

    /**
     * Semantic biome/color-provider samples for the 2x2 blend at every model
     * vertex. Java extracts values only; Rust owns interpolation and encoding.
     */
    private void writeTintLattice(int localBlockIndex, LevelSlice slice, BlockState state, BlockPos pos) {
        long base = this.tintLatticesAddress + (long) localBlockIndex * TINT_LATTICE_SAMPLE_COUNT * Integer.BYTES;
        int sample = 0;
        for (int y = TINT_LATTICE_MIN_OFFSET; y <= TINT_LATTICE_MAX_OFFSET; y++) {
            for (int z = TINT_LATTICE_MIN_OFFSET; z <= TINT_LATTICE_MAX_OFFSET; z++) {
                for (int x = TINT_LATTICE_MIN_OFFSET; x <= TINT_LATTICE_MAX_OFFSET; x++) {
                    MemoryUtil.memPutInt(base + (long) sample++ * Integer.BYTES,
                            blockTint(slice, state, pos.offset(x, y, z)));
                }
            }
        }
    }

    private void recordTintSource(int localBlockIndex, BlockState state, BlockPos pos, int tint) {
        if (tint == -1) {
            return;
        }
        long sectionKey = ((long) (this.minX >> 4) & 0x3F_FFFFL) << 42
                | ((long) (this.minZ >> 4) & 0x3F_FFFFL) << 20
                | ((long) (this.minY >> 4) & 0xF_FFFFL);
        if (!StaticTerrainParityDiagnostics.tracesNativeTintSource(sectionKey)) {
            return;
        }
        long base = this.tintLatticesAddress + (long) localBlockIndex * TINT_LATTICE_SAMPLE_COUNT * Integer.BYTES;
        int[] lattice = new int[TINT_LATTICE_SAMPLE_COUNT];
        for (int sample = 0; sample < lattice.length; sample++) {
            lattice[sample] = MemoryUtil.memGetInt(base + (long) sample * Integer.BYTES);
        }
        StaticTerrainParityDiagnostics.recordNativeTintSource(
                sectionKey, pos.getX(), pos.getY(), pos.getZ(), String.valueOf(state.getBlock()), tint, lattice);
    }

    private void populatePaddedGrids(LevelSlice slice) {
        for (int py = 0; py < PADDED_LENGTH; py++) {
            int y = this.minY + py - 1;
            for (int pz = 0; pz < PADDED_LENGTH; pz++) {
                int z = this.minZ + pz - 1;
                for (int px = 0; px < PADDED_LENGTH; px++) {
                    int x = this.minX + px - 1;
                    int index = paddedIndex(px, py, pz);
                    BlockState state = slice.getBlockState(x, y, z);
                    MemoryUtil.memPutInt(this.paddedStateIdsAddress + (long) index * Integer.BYTES,
                            NativeStaticBlockModelRegistry.getStateId(state));
                    MemoryUtil.memPutInt(this.paddedLightWordsAddress + (long) index * Integer.BYTES,
                            computeLightWord(slice, state, x, y, z));
                }
            }
        }
    }

    private static int paddedIndex(int x, int y, int z) {
        return (y * PADDED_LENGTH + z) * PADDED_LENGTH + x;
    }

    private int modelCullMask(LevelSlice slice, BlockState state, BlockPos position) {
        int mask = 0;
        for (Direction direction : Direction.values()) {
            if (!this.modelOcclusionCache.shouldDrawSide(state, slice, position, direction)) {
                mask |= 1 << direction.get3DDataValue();
            }
        }
        return mask;
    }

    private static long align(long offset, int alignment) {
        return (offset + alignment - 1L) & -alignment;
    }

    @Override
    public void close() {
        if (this.address != 0L) {
            MemoryUtil.nmemFree(this.address);
            this.address = 0L;
        }
    }

    static boolean isNativeFluidSupported(FluidState fluidState) {
        return NativeStaticBlockModelRegistry.isNativeFluidSupported(fluidState);
    }

    private static int computeLightWord(LevelSlice slice, BlockState state, int x, int y, int z) {
        BlockPos pos = new BlockPos(x, y, z);
        boolean emissive = state.emissiveRendering(slice, pos);
        boolean opaque = state.isViewBlocking(slice, pos) && state.getLightBlock() != 0;
        boolean fullOpaque = state.isSolidRender();
        boolean fullCube = state.isCollisionShapeFullBlock(slice, pos);
        int luminance = PlatformBlockAccess.getInstance().getLightEmission(state, slice, pos);
        int blockLight;
        int skyLight;
        if (fullOpaque && luminance == 0) {
            blockLight = 0;
            skyLight = 0;
        } else {
            if (emissive) {
                blockLight = slice.getBrightness(LightLayer.BLOCK, pos);
                skyLight = slice.getBrightness(LightLayer.SKY, pos);
            } else {
                int light = LevelRenderer.getLightColor(LevelRenderer.BrightnessGetter.DEFAULT, slice, state, pos);
                blockLight = LightTexture.block(light);
                skyLight = LightTexture.sky(light);
            }
        }
        float ao = luminance == 0 ? state.getShadeBrightness(slice, pos) : 1.0F;
        int aoi = (int) (ao * 4096.0F);
        int lightWord = (blockLight & 0xF)
                | ((skyLight & 0xF) << 4)
                | ((luminance & 0xF) << 8)
                | ((aoi & 0xFFFF) << 12)
                | ((emissive ? 1 : 0) << 28)
                | ((opaque ? 1 : 0) << 29)
                | ((fullOpaque ? 1 : 0) << 30)
                | ((fullCube ? 1 : 0) << 31);
        StaticTerrainParityDiagnostics.recordAppearanceLightInput(
                "native-section-snapshot",
                x,
                y,
                z,
                String.valueOf(state.getBlock()),
                lightWord
        );
        return lightWord;
    }

    static int blockTint(net.minecraft.world.level.BlockAndTintGetter slice, BlockState state, BlockPos pos) {
        if (NativeMeshingDiagnostics.forceWhiteTint()) {
            return 0xFFFFFFFF;
        }
        Block block = state.getBlock();
        if (block == Blocks.GRASS_BLOCK || block == Blocks.FERN || block == Blocks.SHORT_GRASS
                || block == Blocks.POTTED_FERN || block == Blocks.BUSH || block == Blocks.SUGAR_CANE
                || block == Blocks.PINK_PETALS || block == Blocks.WILDFLOWERS
                || block == Blocks.LARGE_FERN || block == Blocks.TALL_GRASS) {
            return BiomeColors.getAverageGrassColor(slice, pos) | 0xFF000000;
        }
        if (block == Blocks.OAK_LEAVES || block == Blocks.JUNGLE_LEAVES || block == Blocks.ACACIA_LEAVES
                || block == Blocks.DARK_OAK_LEAVES || block == Blocks.VINE || block == Blocks.MANGROVE_LEAVES) {
            return BiomeColors.getAverageFoliageColor(slice, pos) | 0xFF000000;
        }
        if (block == Blocks.LEAF_LITTER) {
            return BiomeColors.getAverageDryFoliageColor(slice, pos) | 0xFF000000;
        }
        if (block == Blocks.REDSTONE_WIRE) {
            return RedStoneWireBlock.getColorForPower(state.getValue(RedStoneWireBlock.POWER)) | 0xFF000000;
        }
        int color = Minecraft.getInstance().getBlockColors().getColor(state, slice, pos, 0);
        return normalizeBlockTintColor(color);
    }

    static int normalizeBlockTintColor(int color) {
        return color == -1 ? -1 : color | 0xFF000000;
    }

    private static int fluidTint(LevelSlice slice, FluidState state, BlockPos pos) {
        if (NativeMeshingDiagnostics.forceWhiteTint()) {
            return 0xFFFFFFFF;
        }
        if (state.is(Fluids.WATER) || state.is(Fluids.FLOWING_WATER)) {
            return BiomeColors.getAverageWaterColor(slice, pos) | 0xFF000000;
        }
        return -1;
    }

    private static int irisBlockId(BlockState state) {
		if (net.vulkanic.VulkanicAPI.isVulkanBackendSelected()
			|| net.vulkanic.bridge.RustGalVulkanWholeFrameMode.enabled()) {
			return -1;
		}
        var ids = net.irisshaders.iris.shaderpack.materialmap.WorldRenderingSettings.INSTANCE.getBlockStateIds();
        return ids == null ? -1 : ids.getOrDefault(state, -1);
    }

    private static int irisFluidBlockId(FluidState state) {
		if (net.vulkanic.VulkanicAPI.isVulkanBackendSelected()
			|| net.vulkanic.bridge.RustGalVulkanWholeFrameMode.enabled()) {
			return -1;
		}
        var ids = net.irisshaders.iris.shaderpack.materialmap.WorldRenderingSettings.INSTANCE.getBlockStateIds();
        return ids == null ? -1 : ids.getInt(state.createLegacyBlock());
    }
}
