package net.sodium.client.render.chunk.compile.pipeline;

import it.unimi.dsi.fastutil.objects.Object2IntOpenHashMap;
import it.unimi.dsi.fastutil.objects.Reference2IntOpenHashMap;
import it.unimi.dsi.fastutil.objects.Reference2ReferenceOpenHashMap;
import net.sodium.api.util.ColorARGB;
import net.sodium.client.compatibility.workarounds.Workarounds;
import net.minecraft.client.Minecraft;
import net.minecraft.client.renderer.ItemBlockRenderTypes;
import net.minecraft.client.renderer.block.model.BakedQuad;
import net.minecraft.client.renderer.block.model.BlockModelPart;
import net.minecraft.client.renderer.block.model.BlockStateModel;
import net.minecraft.client.renderer.block.model.SingleVariant;
import net.minecraft.client.renderer.block.model.multipart.MultiPartModel;
import net.minecraft.client.renderer.chunk.ChunkSectionLayer;
import net.minecraft.client.renderer.texture.TextureAtlasSprite;
import net.minecraft.data.AtlasIds;
import net.minecraft.client.resources.model.WeightedVariants;
import net.minecraft.core.Direction;
import net.minecraft.resources.ResourceLocation;
import net.minecraft.core.registries.BuiltInRegistries;
import net.minecraft.util.random.Weighted;
import net.minecraft.world.level.block.Block;
import net.minecraft.world.level.block.Blocks;
import net.minecraft.world.level.block.IronBarsBlock;
import net.minecraft.world.level.block.LightBlock;
import net.minecraft.world.level.block.RenderShape;
import net.minecraft.world.level.block.StemBlock;
import net.minecraft.world.level.block.RedStoneWireBlock;
import net.minecraft.world.level.block.state.BlockBehaviour;
import net.minecraft.world.level.block.state.BlockState;
import net.minecraft.world.level.material.FluidState;
import net.minecraft.world.level.material.Fluids;
import net.sodium.client.render.chunk.terrain.DefaultTerrainRenderPasses;
import net.sodium.client.render.chunk.terrain.TerrainRenderPass;
import net.sodium.client.render.chunk.terrain.material.DefaultMaterials;
import net.sodium.client.render.chunk.terrain.material.Material;
import net.sodium.client.render.chunk.vertex.format.NativeChunkMeshEncoder;
import net.sodium.client.render.chunk.vertex.format.NativeStaticBlockModelCache;
import net.fabricmc.fabric.api.client.render.fluid.v1.FluidRenderHandler;
import net.fabricmc.fabric.api.client.render.fluid.v1.FluidRenderHandlerRegistry;

import java.util.ArrayList;
import java.util.List;
import java.util.Map;

public final class NativeStaticBlockModelRegistry {
    public static final int STATE_FLAG_AIR = 1;
    public static final int STATE_FLAG_MODEL = 1 << 1;
    public static final int STATE_FLAG_FLUID = 1 << 2;
    public static final int STATE_FLAG_SOLID_RENDER = 1 << 3;
    public static final int STATE_FLAG_FULL_OCCLUSION = 1 << 4;
    public static final int STATE_FLAG_LIGHT_BLOCK = 1 << 5;
    public static final int STATE_FLAG_BLOCK_ENTITY = 1 << 6;
    public static final int STATE_FLAG_CAN_OCCLUDE = 1 << 7;
    public static final int STATE_FLAG_BLOCKS_MOTION = 1 << 8;
    public static final int STATE_FLAG_MODEL_FACE_CULLABLE = 1 << 9;
    public static final int STATE_FLAG_FLUID_OVERLAY_TRANSPARENT = 1 << 10;

    public static int fluidOverlayFlags(BlockState state) {
        // Copy the declared resource property used by the ordinary fluid
        // producer. Occlusion is independent and cannot stand in for it.
        return FluidRenderHandlerRegistry.INSTANCE.isBlockTransparent(state.getBlock())
            ? STATE_FLAG_FLUID_OVERLAY_TRANSPARENT : 0;
    }

    private static final int TINT_NONE = 0;
    private static final int TINT_GRASS = 1;
    private static final int TINT_FOLIAGE = 2;
    private static final int TINT_WATER = 3;
    private static final int TINT_REDSTONE = 4;
    private static final int TINT_CONSTANT = 5;
    private static final int TINT_STEM = 6;
    private static final int TINT_DOUBLE_PLANT_GRASS = 7;
    private static final int TINT_SPRUCE = 8;
    private static final int TINT_BIRCH = 9;
    private static final int TINT_FORCE_GRASS = 10;

    private static final int FLUID_NONE = 0;
    private static final int FLUID_WATER = 1;
    private static final int FLUID_LAVA = 2;

    private static final int MISSING_ID = -1;
    private static final int SELECTOR_DIRECT = 0;
    private static final int SELECTOR_WEIGHTED = 1;
    private static final int SELECTOR_GROUP = 2;

    private static final boolean FORCE_JAVA_PRODUCERS = Boolean.getBoolean("mattmc.nativeMeshing.forceJavaProducers");
    private static final boolean FORCE_JAVA_MODELS = FORCE_JAVA_PRODUCERS
            || Boolean.getBoolean("mattmc.nativeMeshing.forceJavaModels");
    private static final boolean FORCE_JAVA_FLUIDS = FORCE_JAVA_PRODUCERS
            || Boolean.getBoolean("mattmc.nativeMeshing.forceJavaFluids");
    private static final boolean FORCE_DEFAULT_FLUID_SPRITE = Boolean.getBoolean("mattmc.nativeMeshing.forceDefaultFluidSprite");

    private static final Reference2ReferenceOpenHashMap<BlockState, BlockStateModel> MODELS = new Reference2ReferenceOpenHashMap<>();
    private static final Reference2IntOpenHashMap<BlockState> STATE_IDS = new Reference2IntOpenHashMap<>();
    private static final Reference2IntOpenHashMap<BlockState> STATE_SELECTORS = new Reference2IntOpenHashMap<>();
    private static final Map<SelectorKey, Integer> SELECTOR_IDS = new java.util.HashMap<>();
    private static final Map<PartModelKey, Integer> PART_MODEL_IDS = new java.util.HashMap<>();
    private static final Object2IntOpenHashMap<Block> SKIP_GROUPS = new Object2IntOpenHashMap<>();
    private static final int FLUID_SPRITE_WATER_STILL = 1;
    private static final int FLUID_SPRITE_WATER_FLOW = 1 << 1;
    private static final int FLUID_SPRITE_WATER_OVERLAY = 1 << 2;
    private static final int FLUID_SPRITE_LAVA_STILL = 1 << 8;
    private static final int FLUID_SPRITE_LAVA_FLOW = 1 << 9;

    private static final Map<Integer, List<TextureAtlasSprite>> MODEL_SPRITES = new java.util.HashMap<>();
    private static final Map<Integer, List<TextureAtlasSprite>> SELECTOR_SPRITES = new java.util.HashMap<>();

    private static int nextSelectorId;
    private static int nextModelId;
    private static int nextSkipGroup = 1;
    private static int reloadGeneration;

    static {
        STATE_IDS.defaultReturnValue(MISSING_ID);
        STATE_SELECTORS.defaultReturnValue(MISSING_ID);
        SKIP_GROUPS.defaultReturnValue(0);
    }

    private NativeStaticBlockModelRegistry() {
    }

    /**
     * Rebuilds the Java-owned model/state registry and the Rust-owned native cache as one generation.
     * Native section snapshots capture {@link #reloadGeneration()} when allocated and must be flushed
     * before the next reload. After this method clears {@link NativeStaticBlockModelCache}, old native
     * state, selector, and model ids are invalid and must not be used by a later compact snapshot.
     */
    public static synchronized void reload(Map<BlockState, BlockStateModel> models) {
        reloadGeneration++;
        MODELS.clear();
        STATE_IDS.clear();
        STATE_SELECTORS.clear();
        SELECTOR_IDS.clear();
        PART_MODEL_IDS.clear();
        MODEL_SPRITES.clear();
        SELECTOR_SPRITES.clear();
        SKIP_GROUPS.clear();
        NativeStaticBlockModelCache.clear();
        nextSelectorId = 0;
        nextModelId = 0;
        nextSkipGroup = 1;

        ItemBlockRenderTypes.setFancy(Minecraft.useFancyGraphics());

        for (Map.Entry<BlockState, BlockStateModel> entry : models.entrySet()) {
            MODELS.put(entry.getKey(), entry.getValue());
            registerState(entry.getKey(), entry.getValue());
        }
    }

    public static synchronized int reloadGeneration() {
        return reloadGeneration;
    }

    public static synchronized int getStateId(BlockState state) {
        int id = STATE_IDS.getInt(state);
        if (id != MISSING_ID) {
            return id;
        }

        return registerState(state, MODELS.get(state));
    }

    public static synchronized boolean hasNativeModel(BlockState state) {
        getStateId(state);
        return STATE_SELECTORS.getInt(state) >= 0;
    }

    public static List<TextureAtlasSprite> getSprites(int selectorId) {
        return SELECTOR_SPRITES.get(selectorId);
    }

    public static List<TextureAtlasSprite> getSprites(BlockState state) {
        int selectorId = STATE_SELECTORS.getInt(state);
        if (selectorId < 0) {
            return List.of();
        }

        List<TextureAtlasSprite> sprites = SELECTOR_SPRITES.get(selectorId);
        return sprites == null ? List.of() : sprites;
    }

    public static List<TextureAtlasSprite> getNativeFluidSprites(int emittedSpriteMask) {
        if (emittedSpriteMask == 0) return List.of();
        ArrayList<TextureAtlasSprite> tracked = new ArrayList<>(5);
        for (String name : nativeFluidSpriteNames(emittedSpriteMask)) {
            TextureAtlasSprite sprite = blockSprite(name);
            if (sprite != null && !tracked.contains(sprite)) tracked.add(sprite);
        }
        return tracked;
    }

    /** Immutable usage semantics; renderer capability must not hide emitted resources. */
    static List<String> nativeFluidSpriteNames(int emittedSpriteMask) {
        if (emittedSpriteMask == 0) return List.of();
        ArrayList<String> names = new ArrayList<>(5);
        addFluidSpriteName(names, emittedSpriteMask, FLUID_SPRITE_WATER_STILL, "minecraft:block/water_still");
        addFluidSpriteName(names, emittedSpriteMask, FLUID_SPRITE_WATER_FLOW, "minecraft:block/water_flow");
        addFluidSpriteName(names, emittedSpriteMask, FLUID_SPRITE_WATER_OVERLAY, "minecraft:block/water_overlay");
        addFluidSpriteName(names, emittedSpriteMask, FLUID_SPRITE_LAVA_STILL, "minecraft:block/lava_still");
        addFluidSpriteName(names, emittedSpriteMask, FLUID_SPRITE_LAVA_FLOW, "minecraft:block/lava_flow");
        return List.copyOf(names);
    }

    private static void addFluidSpriteName(List<String> tracked, int emittedSpriteMask, int flag,
            String spriteId) {
        if ((emittedSpriteMask & flag) != 0) tracked.add(spriteId);
    }

    private static int registerState(BlockState state, BlockStateModel model) {
        int existing = STATE_IDS.getInt(state);
        if (existing != MISSING_ID) {
            return existing;
        }

        int stateId = Block.getId(state);
        STATE_IDS.put(state, stateId);

        int tintType = tintType(state);
        int selectorId = MISSING_ID;
        if (!FORCE_JAVA_MODELS && model != null && state.getRenderShape() == RenderShape.MODEL) {
            selectorId = registerSelector(state, model, stateId);
            if (selectorId != MISSING_ID && tintType == TINT_NONE && modelHasTintedQuads(model)) {
                selectorId = MISSING_ID;
            }
        }
        STATE_SELECTORS.put(state, selectorId);

        Material material = DefaultMaterials.forBlockState(state);
        int modelPassId = selectorId >= 0 ? MISSING_ID : passId(material.pass);
        FluidState fluidState = state.getFluidState();
        int fluidMaterialBits = 0;
        int fluidPassId = MISSING_ID;
        int fluidBlockId = MISSING_ID;
        if (!FORCE_JAVA_FLUIDS && !fluidState.isEmpty()) {
            Material fluidMaterial = DefaultMaterials.forFluidState(fluidState);
            fluidMaterialBits = fluidMaterial.bits();
            fluidPassId = passId(fluidMaterial.pass);
			fluidBlockId = semanticFluidStateId(fluidState);
        }

        int flags = fluidOverlayFlags(state);
        if (state.isAir()) {
            flags |= STATE_FLAG_AIR;
        }
        if (selectorId >= 0) {
            flags |= STATE_FLAG_MODEL;
            if (modelHasOnlyCullFaceQuads(model)) {
                flags |= STATE_FLAG_MODEL_FACE_CULLABLE;
            }
        }
        if (!FORCE_JAVA_FLUIDS && !fluidState.isEmpty()) {
            flags |= STATE_FLAG_FLUID;
        }
        if (state.isSolidRender()) {
            flags |= STATE_FLAG_SOLID_RENDER | STATE_FLAG_FULL_OCCLUSION;
        }
        if (state.getBlock() instanceof LightBlock) {
            flags |= STATE_FLAG_LIGHT_BLOCK;
        }
        if (state.hasBlockEntity()) {
            flags |= STATE_FLAG_BLOCK_ENTITY;
        }
        if (state.canOcclude()) {
            flags |= STATE_FLAG_CAN_OCCLUDE;
        }
        if (state.blocksMotion()) {
            flags |= STATE_FLAG_BLOCKS_MOTION;
        }

        int fluidType = FORCE_JAVA_FLUIDS ? FLUID_NONE : fluidType(fluidState);
        BlockBehaviour.OffsetType offsetType = state.sodium$getOffsetType();
        FluidSpriteMetadata fluidSprites = fluidSpriteMetadata(fluidState);
        int sameBlockSkipMask = sameBlockSkipMask(state);
        NativeStaticBlockModelCache.registerState(stateId, selectorId, flags, material.bits(), modelPassId,
				state.getLightEmission(), 0, semanticBlockStateId(state), fluidMaterialBits, fluidPassId, fluidBlockId,
                skipGroup(state, sameBlockSkipMask), sameBlockSkipMask, fluidType,
                fluidState.isEmpty() ? 0.0F : fluidState.getOwnHeight(),
                fluidState.hasProperty(net.minecraft.world.level.material.FlowingFluid.FALLING) && fluidState.getValue(net.minecraft.world.level.material.FlowingFluid.FALLING) ? 1 : 0,
                offsetType.ordinal(), state.sodium$getMaxHorizontalOffset(), state.sodium$getMaxVerticalOffset(),
                tintType,
                fluidSprites.still.u0, fluidSprites.still.u1, fluidSprites.still.v0, fluidSprites.still.v1,
                fluidSprites.still.shrink,
                fluidSprites.flow.u0, fluidSprites.flow.u1, fluidSprites.flow.v0, fluidSprites.flow.v1,
                fluidSprites.flow.shrink,
                fluidSprites.overlay.u0, fluidSprites.overlay.u1, fluidSprites.overlay.v0, fluidSprites.overlay.v1,
                fluidSprites.overlay.shrink, fluidSprites.overlayValid ? 1 : 0);
        return stateId;
    }

    private static int registerSelector(BlockState state, BlockStateModel model, int stateId) {
        Material material = DefaultMaterials.forBlockState(state);
        SelectorKey key = new SelectorKey(model, stateId, material.bits(), passId(material.pass),
                state.getLightEmission() == 0);
        Integer existing = SELECTOR_IDS.get(key);
        if (existing != null) {
            return existing;
        }

        int selectorId = nextSelectorId++;
        SELECTOR_IDS.put(key, selectorId);

        if (model instanceof SingleVariant single) {
            int modelId = registerPartModel(state, stateId, single.sodium$getModelPart());
            NativeStaticBlockModelCache.registerSelector(selectorId, SELECTOR_DIRECT,
                    (recordAddress, index) -> NativeChunkMeshEncoder.writeNativeModelSelectorEntry(recordAddress,
                            modelId, 1), 1);
            copySprites(selectorId, List.of(modelId));
            return selectorId;
        }

        if (model instanceof WeightedVariants weighted) {
            List<Weighted<BlockStateModel>> entries = weighted.sodium$getWeightedModels().unwrap();
            int[] childSelectors = new int[entries.size()];
            int[] weights = new int[entries.size()];
            for (int index = 0; index < entries.size(); index++) {
                Weighted<BlockStateModel> entry = entries.get(index);
                childSelectors[index] = registerSelector(state, entry.value(), stateId);
                weights[index] = entry.weight();
            }
            NativeStaticBlockModelCache.registerSelector(selectorId, SELECTOR_WEIGHTED,
                    (recordAddress, index) -> NativeChunkMeshEncoder.writeNativeModelSelectorEntry(recordAddress,
                            childSelectors[index], weights[index]), entries.size());
            copySelectorSprites(selectorId, childSelectors);
            return selectorId;
        }

        if (model instanceof MultiPartModel multipart) {
            List<BlockStateModel> children = multipart.sodium$getSelectedModels();
            int[] childSelectors = new int[children.size()];
            for (int index = 0; index < children.size(); index++) {
                childSelectors[index] = registerSelector(state, children.get(index), stateId);
            }
            NativeStaticBlockModelCache.registerSelector(selectorId, SELECTOR_GROUP,
                    (recordAddress, index) -> NativeChunkMeshEncoder.writeNativeModelSelectorEntry(recordAddress,
                            childSelectors[index], 1), children.size());
            copySelectorSprites(selectorId, childSelectors);
            return selectorId;
        }

        SELECTOR_IDS.remove(key);
        return MISSING_ID;
    }

    private static boolean modelHasTintedQuads(BlockStateModel model) {
        if (model instanceof SingleVariant single) {
            return partHasTintedQuads(single.sodium$getModelPart());
        }

        if (model instanceof WeightedVariants weighted) {
            for (Weighted<BlockStateModel> entry : weighted.sodium$getWeightedModels().unwrap()) {
                if (modelHasTintedQuads(entry.value())) {
                    return true;
                }
            }
            return false;
        }

        if (model instanceof MultiPartModel multipart) {
            for (BlockStateModel child : multipart.sodium$getSelectedModels()) {
                if (modelHasTintedQuads(child)) {
                    return true;
                }
            }
        }

        return false;
    }

    private static boolean partHasTintedQuads(BlockModelPart part) {
        for (int faceIndex = -1; faceIndex < net.minecraft.core.Direction.values().length; faceIndex++) {
            net.minecraft.core.Direction cullFace = faceIndex < 0 ? null : net.minecraft.core.Direction.from3DDataValue(faceIndex);
            for (BakedQuad quad : part.getQuads(cullFace)) {
                if (quad.getTintIndex() != -1) {
                    return true;
                }
            }
        }
        return false;
    }

    private static boolean modelHasOnlyCullFaceQuads(BlockStateModel model) {
        if (model instanceof SingleVariant single) {
            return partHasOnlyCullFaceQuads(single.sodium$getModelPart());
        }

        if (model instanceof WeightedVariants weighted) {
            for (Weighted<BlockStateModel> entry : weighted.sodium$getWeightedModels().unwrap()) {
                if (!modelHasOnlyCullFaceQuads(entry.value())) {
                    return false;
                }
            }
            return true;
        }

        if (model instanceof MultiPartModel multipart) {
            for (BlockStateModel child : multipart.sodium$getSelectedModels()) {
                if (!modelHasOnlyCullFaceQuads(child)) {
                    return false;
                }
            }
            return true;
        }

        return false;
    }

    private static boolean partHasOnlyCullFaceQuads(BlockModelPart part) {
        for (BakedQuad quad : part.getQuads(null)) {
            return false;
        }
        return true;
    }

    private static int registerPartModel(BlockState state, int stateId, BlockModelPart part) {
        Material material = DefaultMaterials.forBlockState(state);
        boolean hasAo = part.useAmbientOcclusion() && state.getLightEmission() == 0;
        ChunkSectionLayer defaultRenderType = ItemBlockRenderTypes.getChunkRenderType(state);
        PartModelKey key = new PartModelKey(part, stateId, material.bits(), passId(material.pass), hasAo);
        Integer existing = PART_MODEL_IDS.get(key);
        if (existing != null) {
            return existing;
        }

        int modelId = nextModelId++;
        PART_MODEL_IDS.put(key, modelId);

        CachedModel cached = buildCachedModel(part, defaultRenderType, hasAo);
        MODEL_SPRITES.put(modelId, cached.sprites);
        NativeStaticBlockModelCache.register(modelId, (recordAddress, index) -> {
            CachedQuad quad = cached.quads.get(index);
            NativeChunkMeshEncoder.writeStaticModelQuadRecord(recordAddress, quad.materialBits, quad.passId,
                    quad.cullFace, quad.normalFace, quad.packedNormal, (byte) 0, (byte) 0, quad.shade,
                    quad.flags, quad.lightFace, quad.tintIndex, quad.hasAo,
                    quad.x0, quad.y0, quad.z0, quad.color0, quad.u0, quad.v0, quad.light0,
                    quad.x1, quad.y1, quad.z1, quad.color1, quad.u1, quad.v1, quad.light1,
                    quad.x2, quad.y2, quad.z2, quad.color2, quad.u2, quad.v2, quad.light2,
                    quad.x3, quad.y3, quad.z3, quad.color3, quad.u3, quad.v3, quad.light3);
        }, cached.quads.size());
        return modelId;
    }

    private static CachedModel buildCachedModel(BlockModelPart part, ChunkSectionLayer defaultRenderType, boolean hasAo) {
        List<CachedQuad> quads = new ArrayList<>();
        List<TextureAtlasSprite> sprites = new ArrayList<>();

        for (int faceIndex = -1; faceIndex < net.minecraft.core.Direction.values().length; faceIndex++) {
            net.minecraft.core.Direction cullFace = faceIndex < 0 ? null : net.minecraft.core.Direction.from3DDataValue(faceIndex);
            for (BakedQuad quad : part.getQuads(cullFace)) {
                Material material = DefaultMaterials.forChunkLayer(defaultRenderType);
                quads.add(CachedQuad.from(quad, cullFace, material, downgradedPassId(quad, material),
                        hasAo));
                if (quad.sprite() != null && !sprites.contains(quad.sprite())) {
                    sprites.add(quad.sprite());
                }
            }
        }

        return new CachedModel(quads, sprites);
    }

    private static void copySprites(int selectorId, List<Integer> modelIds) {
        List<TextureAtlasSprite> sprites = new ArrayList<>();
        for (int modelId : modelIds) {
            List<TextureAtlasSprite> modelSprites = MODEL_SPRITES.get(modelId);
            if (modelSprites != null) {
                for (TextureAtlasSprite sprite : modelSprites) {
                    if (!sprites.contains(sprite)) {
                        sprites.add(sprite);
                    }
                }
            }
        }
        SELECTOR_SPRITES.put(selectorId, sprites);
    }

    private static void copySelectorSprites(int selectorId, int[] childSelectors) {
        List<TextureAtlasSprite> sprites = new ArrayList<>();
        for (int childSelector : childSelectors) {
            List<TextureAtlasSprite> childSprites = SELECTOR_SPRITES.get(childSelector);
            if (childSprites != null) {
                for (TextureAtlasSprite sprite : childSprites) {
                    if (!sprites.contains(sprite)) {
                        sprites.add(sprite);
                    }
                }
            }
        }
        SELECTOR_SPRITES.put(selectorId, sprites);
    }

    private static int passId(TerrainRenderPass pass) {
        if (pass == DefaultTerrainRenderPasses.SOLID) {
            return 0;
        }
        if (pass == DefaultTerrainRenderPasses.CUTOUT) {
            return 1;
        }
        if (pass == DefaultTerrainRenderPasses.TRANSLUCENT) {
            return 2;
        }
        return MISSING_ID;
    }

    static int materialBitsForPass(Material material, int selectedPassId) {
        // The immutable native snapshot must carry the same alpha predicate
        // as Frozen's selected pass. Binary-alpha translucent sprites become
        // cutout; carrying ZERO here would turn transparent texels opaque.
        if (material == DefaultMaterials.TRANSLUCENT && selectedPassId == passId(DefaultTerrainRenderPasses.CUTOUT)) {
            return net.sodium.client.render.chunk.terrain.material.parameters.MaterialParameters.pack(
                net.sodium.client.render.chunk.terrain.material.parameters.AlphaCutoffParameter.ONE_TENTH,
                material.mipped);
        }
        return material.bits();
    }

    private static int downgradedPassId(BakedQuad quad, Material material) {
        TerrainRenderPass pass = material.pass;
        TextureAtlasSprite sprite = quad.sprite();
        if (sprite == null || Workarounds.isWorkaroundEnabled(Workarounds.Reference.INTEL_DEPTH_BUFFER_COMPARISON_UNRELIABLE)) {
            return passId(pass);
        }

        boolean hasNonOpaqueVertex = false;
        for (int vertex = 0; vertex < 4; vertex++) {
            hasNonOpaqueVertex |= ColorARGB.unpackAlpha(quad.getColor(vertex)) != 0xFF;
        }
        if (pass.isTranslucent() && hasNonOpaqueVertex) {
            return passId(pass);
        }
        if (!validateQuadUVs(sprite, quad)) {
            return passId(pass);
        }
        if (sprite instanceof TextureAtlasSpriteExtension spriteExt && spriteExt.sodium$hasUnknownImageContents()) {
            return passId(pass);
        }
        if (sprite.contents() instanceof SpriteContentsExtension contentsExt) {
            if (pass == DefaultTerrainRenderPasses.TRANSLUCENT && !contentsExt.sodium$hasTranslucentPixels()) {
                pass = DefaultTerrainRenderPasses.CUTOUT;
            }
            if (pass == DefaultTerrainRenderPasses.CUTOUT && !contentsExt.sodium$hasTransparentPixels()) {
                pass = DefaultTerrainRenderPasses.SOLID;
            }
        }
        return passId(pass);
    }

    private static boolean validateQuadUVs(TextureAtlasSprite sprite, BakedQuad quad) {
        float spriteUMin = sprite.getU0();
        float spriteUMax = sprite.getU1();
        float spriteVMin = sprite.getV0();
        float spriteVMax = sprite.getV1();

        for (int vertex = 0; vertex < 4; vertex++) {
            float u = quad.getTexU(vertex);
            float v = quad.getTexV(vertex);
            if (u < spriteUMin || u > spriteUMax || v < spriteVMin || v > spriteVMax) {
                return false;
            }
        }

        return true;
    }

	private static int semanticBlockStateId(BlockState state) {
		return Block.getId(state);
	}

	private static int semanticFluidStateId(FluidState state) {
		return Block.getId(state.createLegacyBlock());
	}

    private static int skipGroup(BlockState state, int sameBlockSkipMask) {
        if (sameBlockSkipMask == 0) {
            return 0;
        }

        Block block = state.getBlock();
        int id = SKIP_GROUPS.getInt(block);
        if (id != 0) {
            return id;
        }

        id = nextSkipGroup++;
        SKIP_GROUPS.put(block, id);
        return id;
    }

    static boolean skipsRenderingAgainstSameBlock(BlockState state) {
        return sameBlockSkipMask(state) == ((1 << Direction.values().length) - 1);
    }

    static int sameBlockSkipMask(BlockState state) {
        if (state.getBlock() instanceof IronBarsBlock) {
            return ironBarsSkipMask(state);
        }

        int mask = 0;
        for (Direction direction : Direction.values()) {
            if (state.skipRendering(state, direction)) {
                mask |= 1 << direction.get3DDataValue();
            }
        }
        return mask;
    }

    private static int ironBarsSkipMask(BlockState state) {
        int mask = 0;
        for (Direction direction : Direction.values()) {
            if (!direction.getAxis().isHorizontal()) {
                if (state.skipRendering(state, direction)) {
                    mask |= 1 << direction.get3DDataValue();
                }
                continue;
            }

            if (Boolean.TRUE.equals(state.getValue(IronBarsBlock.PROPERTY_BY_DIRECTION.get(direction)))) {
                mask |= 1 << direction.get3DDataValue();
            }
        }
        return mask;
    }

    private record CachedModel(List<CachedQuad> quads, List<TextureAtlasSprite> sprites) {
    }

    private static int fluidType(FluidState fluidState) {
        if (fluidState.isEmpty()) {
            return FLUID_NONE;
        }
        if (isNativeFluidSupported(fluidState) && (fluidState.is(Fluids.WATER) || fluidState.is(Fluids.FLOWING_WATER))) {
            return FLUID_WATER;
        }
        if (isNativeFluidSupported(fluidState) && (fluidState.is(Fluids.LAVA) || fluidState.is(Fluids.FLOWING_LAVA))) {
            return FLUID_LAVA;
        }
        return FLUID_NONE;
    }

    public static boolean isNativeFluidSupported(FluidState fluidState) {
        if (FORCE_JAVA_FLUIDS) {
            return false;
        }
        if (fluidState.isEmpty()) {
            return false;
        }
        boolean builtIn = fluidState.is(Fluids.WATER) || fluidState.is(Fluids.FLOWING_WATER)
                || fluidState.is(Fluids.LAVA) || fluidState.is(Fluids.FLOWING_LAVA);
        return builtIn && FluidRenderHandlerRegistry.INSTANCE.getOverride(fluidState.getType()) == null;
    }

    private static FluidSpriteMetadata fluidSpriteMetadata(FluidState fluidState) {
        if (!isNativeFluidSupported(fluidState)) {
            return FluidSpriteMetadata.DEFAULT;
        }
        if (FORCE_DEFAULT_FLUID_SPRITE) {
            return FluidSpriteMetadata.DEFAULT;
        }

        TextureAtlasSprite[] builtInSprites = builtInFluidSprites(fluidState);
        if (builtInSprites.length != 0) {
            TextureAtlasSprite still = builtInSprites.length > 0 ? builtInSprites[0] : null;
            TextureAtlasSprite flow = builtInSprites.length > 1 ? builtInSprites[1] : still;
            TextureAtlasSprite overlay = builtInSprites.length > 2 ? builtInSprites[2] : null;
            return new FluidSpriteMetadata(FluidSprite.from(still), FluidSprite.from(flow),
                    FluidSprite.from(overlay), overlay != null).withFlowFallback();
        }

        FluidRenderHandler handler = FluidRenderHandlerRegistry.INSTANCE.get(fluidState.getType());
        if (handler == null) {
            handler = FluidRenderHandlerRegistry.INSTANCE.get(fluidState.is(Fluids.LAVA) ? Fluids.LAVA : Fluids.WATER);
        }
        if (handler == null) {
            return FluidSpriteMetadata.DEFAULT;
        }

        TextureAtlasSprite[] sprites = handler.getFluidSprites(null, null, fluidState);
        if (sprites == null) {
            return FluidSpriteMetadata.DEFAULT;
        }
        TextureAtlasSprite still = sprites.length > 0 ? sprites[0] : null;
        TextureAtlasSprite flow = sprites.length > 1 ? sprites[1] : still;
        TextureAtlasSprite overlay = sprites.length > 2 ? sprites[2] : null;
        FluidSpriteMetadata base = new FluidSpriteMetadata(FluidSprite.from(still), FluidSprite.from(flow),
                FluidSprite.from(overlay), overlay != null);
        return base.withFlowFallback();
    }

    private static TextureAtlasSprite[] builtInFluidSprites(FluidState fluidState) {
        if (fluidState.is(Fluids.WATER) || fluidState.is(Fluids.FLOWING_WATER)) {
            return new TextureAtlasSprite[] {
                    blockSprite("minecraft:block/water_still"),
                    blockSprite("minecraft:block/water_flow"),
                    blockSprite("minecraft:block/water_overlay")
            };
        }
        if (fluidState.is(Fluids.LAVA) || fluidState.is(Fluids.FLOWING_LAVA)) {
            return new TextureAtlasSprite[] {
                    blockSprite("minecraft:block/lava_still"),
                    blockSprite("minecraft:block/lava_flow")
            };
        }
        return new TextureAtlasSprite[0];
    }

    private static TextureAtlasSprite blockSprite(String id) {
        return Minecraft.getInstance().getAtlasManager().getAtlasOrThrow(AtlasIds.BLOCKS)
                .getSprite(ResourceLocation.parse(id));
    }

    private static int tintType(BlockState state) {
        Block block = state.getBlock();
        if (block == Blocks.GRASS_BLOCK) {
            return TINT_GRASS;
        }
        if (block == Blocks.FERN || block == Blocks.SHORT_GRASS || block == Blocks.POTTED_FERN
                || block == Blocks.BUSH || block == Blocks.SUGAR_CANE || block == Blocks.PINK_PETALS
                || block == Blocks.WILDFLOWERS) {
            return TINT_FORCE_GRASS;
        }
        if (block == Blocks.LARGE_FERN || block == Blocks.TALL_GRASS) {
            return TINT_DOUBLE_PLANT_GRASS;
        }
        if (block == Blocks.OAK_LEAVES || block == Blocks.JUNGLE_LEAVES || block == Blocks.ACACIA_LEAVES
                || block == Blocks.DARK_OAK_LEAVES || block == Blocks.VINE || block == Blocks.MANGROVE_LEAVES
                || block == Blocks.AZALEA_LEAVES
                || block == Blocks.FLOWERING_AZALEA_LEAVES || block == Blocks.CHERRY_LEAVES
                || block == Blocks.PALE_OAK_LEAVES) {
            return TINT_FOLIAGE;
        }
        if (block == Blocks.WATER || block == Blocks.BUBBLE_COLUMN || block == Blocks.WATER_CAULDRON) {
            return TINT_WATER;
        }
        if (block == Blocks.REDSTONE_WIRE) {
            return TINT_REDSTONE;
        }
        if (block == Blocks.MELON_STEM || block == Blocks.PUMPKIN_STEM) {
            return TINT_STEM;
        }
        if (block == Blocks.SPRUCE_LEAVES) {
            return TINT_SPRUCE;
        }
        if (block == Blocks.BIRCH_LEAVES) {
            return TINT_BIRCH;
        }
        // Bamboo's leaf models carry tintindex=0 for resource-pack compatibility,
        // but vanilla deliberately has no BlockColors provider for bamboo.  Its
        // semantic tint is therefore the unmodified texture color, represented by
        // the explicit constant/no-tint state rather than a Java model fallback.
        if (block == Blocks.ATTACHED_MELON_STEM || block == Blocks.ATTACHED_PUMPKIN_STEM
                || block == Blocks.LILY_PAD || block == Blocks.BAMBOO || block == Blocks.POTTED_BAMBOO
                || block == Blocks.LEAF_LITTER) {
            // Leaf litter uses the authored per-block dry-foliage sample.
            // Frozen's VanillaAdapter repeats it at all vertices; it does not
            // use the neighbouring-vertex blend of the green foliage provider.
            return TINT_CONSTANT;
        }
        return TINT_NONE;
    }

    private static final class PartModelKey {
        private final BlockModelPart part;
        private final int stateId;
        private final int materialBits;
        private final int passId;
        private final boolean hasAo;

        private PartModelKey(BlockModelPart part, int stateId, int materialBits, int passId, boolean hasAo) {
            this.part = part;
            this.stateId = stateId;
            this.materialBits = materialBits;
            this.passId = passId;
            this.hasAo = hasAo;
        }

        @Override
        public boolean equals(Object obj) {
            if (this == obj) {
                return true;
            }
            if (!(obj instanceof PartModelKey other)) {
                return false;
            }
            return this.part == other.part
                    && this.stateId == other.stateId
                    && this.materialBits == other.materialBits
                    && this.passId == other.passId
                    && this.hasAo == other.hasAo;
        }

        @Override
        public int hashCode() {
            int result = System.identityHashCode(this.part);
            result = 31 * result + this.stateId;
            result = 31 * result + this.materialBits;
            result = 31 * result + this.passId;
            result = 31 * result + (this.hasAo ? 1 : 0);
            return result;
        }
    }

    private static final class SelectorKey {
        private final BlockStateModel model;
        private final int stateId;
        private final int materialBits;
        private final int passId;
        private final boolean canUseAo;

        private SelectorKey(BlockStateModel model, int stateId, int materialBits, int passId, boolean canUseAo) {
            this.model = model;
            this.stateId = stateId;
            this.materialBits = materialBits;
            this.passId = passId;
            this.canUseAo = canUseAo;
        }

        @Override
        public boolean equals(Object obj) {
            if (this == obj) {
                return true;
            }
            if (!(obj instanceof SelectorKey other)) {
                return false;
            }
            return this.model == other.model
                    && this.stateId == other.stateId
                    && this.materialBits == other.materialBits
                    && this.passId == other.passId
                    && this.canUseAo == other.canUseAo;
        }

        @Override
        public int hashCode() {
            int result = System.identityHashCode(this.model);
            result = 31 * result + this.stateId;
            result = 31 * result + this.materialBits;
            result = 31 * result + this.passId;
            result = 31 * result + (this.canUseAo ? 1 : 0);
            return result;
        }
    }

    private record CachedQuad(int materialBits, int passId, int cullFace, int normalFace, int packedNormal, boolean shade,
            int flags, int lightFace, int tintIndex, boolean hasAo,
            float x0, float y0, float z0, int color0, float u0, float v0, int light0,
            float x1, float y1, float z1, int color1, float u1, float v1, int light1,
            float x2, float y2, float z2, int color2, float u2, float v2, int light2,
            float x3, float y3, float z3, int color3, float u3, float v3, int light3) {
        static CachedQuad from(BakedQuad quad, net.minecraft.core.Direction cullFace, Material material, int passId,
                boolean hasAo) {
            return new CachedQuad(materialBitsForPass(material, passId), passId, cullFace == null ? -1 : cullFace.get3DDataValue(),
                    quad.getNormalFace().ordinal(), quad.getFaceNormal(), quad.hasShade(),
                    quad.getFlags(), quad.getLightFace().get3DDataValue(), quad.getTintIndex(), hasAo,
                    quad.getX(0), quad.getY(0), quad.getZ(0), quad.getColor(0), quad.getTexU(0), quad.getTexV(0), quad.getLight(0),
                    quad.getX(1), quad.getY(1), quad.getZ(1), quad.getColor(1), quad.getTexU(1), quad.getTexV(1), quad.getLight(1),
                    quad.getX(2), quad.getY(2), quad.getZ(2), quad.getColor(2), quad.getTexU(2), quad.getTexV(2), quad.getLight(2),
                    quad.getX(3), quad.getY(3), quad.getZ(3), quad.getColor(3), quad.getTexU(3), quad.getTexV(3), quad.getLight(3));
        }
    }

    private record FluidSprite(float u0, float u1, float v0, float v1, float shrink) {
        private static final FluidSprite DEFAULT = new FluidSprite(0.0F, 1.0F, 0.0F, 1.0F, 0.0F);

        private static FluidSprite from(TextureAtlasSprite sprite) {
            if (sprite == null) {
                return DEFAULT;
            }
            return new FluidSprite(sprite.getU0(), sprite.getU1(), sprite.getV0(), sprite.getV1(),
                    sprite.uvShrinkRatio());
        }
    }

    private record FluidSpriteMetadata(FluidSprite still, FluidSprite flow, FluidSprite overlay,
            boolean overlayValid) {
        private static final FluidSpriteMetadata DEFAULT = new FluidSpriteMetadata(FluidSprite.DEFAULT,
                FluidSprite.DEFAULT, FluidSprite.DEFAULT, false);

        private FluidSpriteMetadata withFlowFallback() {
            FluidSprite resolvedFlow = this.flow == null ? this.still : this.flow;
            return new FluidSpriteMetadata(this.still, resolvedFlow, this.overlay, this.overlayValid);
        }
    }
}
