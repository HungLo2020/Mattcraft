package net.sodium.client.perf.real;

import com.google.gson.Gson;
import com.google.gson.GsonBuilder;
import com.google.gson.JsonArray;
import com.google.gson.JsonObject;
import net.minecraft.client.Minecraft;
import net.minecraft.client.renderer.BiomeColors;
import net.minecraft.client.renderer.ItemBlockRenderTypes;
import net.minecraft.client.renderer.LevelRenderer;
import net.minecraft.client.renderer.LightTexture;
import net.minecraft.client.renderer.block.model.BakedQuad;
import net.minecraft.client.renderer.block.model.BlockModelPart;
import net.minecraft.core.BlockPos;
import net.minecraft.core.Direction;
import net.minecraft.core.SectionPos;
import net.minecraft.core.registries.BuiltInRegistries;
import net.minecraft.network.chat.Component;
import net.minecraft.util.RandomSource;
import net.minecraft.world.Difficulty;
import net.minecraft.world.level.GameRules;
import net.minecraft.world.level.GameType;
import net.minecraft.world.level.LevelSettings;
import net.minecraft.world.level.LightLayer;
import net.minecraft.world.level.WorldDataConfiguration;
import net.minecraft.world.level.block.Block;
import net.minecraft.world.level.block.Blocks;
import net.minecraft.world.level.block.FenceBlock;
import net.minecraft.world.level.block.LiquidBlock;
import net.minecraft.world.level.block.LightBlock;
import net.minecraft.world.level.block.RedStoneWireBlock;
import net.minecraft.world.level.block.RenderShape;
import net.minecraft.world.level.block.SlabBlock;
import net.minecraft.world.level.block.StairBlock;
import net.minecraft.world.level.block.TrapDoorBlock;
import net.minecraft.world.level.block.state.BlockState;
import net.minecraft.world.level.block.state.properties.BlockStateProperties;
import net.minecraft.world.level.block.state.properties.Half;
import net.minecraft.world.level.block.state.properties.Property;
import net.minecraft.world.level.block.state.properties.SlabType;
import net.minecraft.world.level.block.state.properties.StairsShape;
import net.minecraft.world.level.chunk.LevelChunk;
import net.minecraft.world.level.material.FluidState;
import net.minecraft.world.level.material.Fluids;
import net.minecraft.world.level.levelgen.WorldOptions;
import net.minecraft.world.level.levelgen.presets.WorldPresets;
import net.minecraft.hooks.GameHooks;
import net.sodium.client.render.chunk.terrain.material.DefaultMaterials;
import net.sodium.client.render.chunk.RenderSection;
import net.sodium.client.render.chunk.compile.ChunkBuildContext;
import net.sodium.client.render.chunk.compile.ChunkBuildOutput;
import net.sodium.client.render.chunk.compile.pipeline.NativeStaticBlockModelRegistry;
import net.sodium.client.render.chunk.compile.tasks.ChunkBuilderMeshingTask;
import net.sodium.client.render.chunk.data.BuiltSectionMeshParts;
import net.sodium.client.model.quad.properties.ModelQuadFacing;
import net.sodium.client.render.chunk.region.RenderRegion;
import net.sodium.client.render.chunk.terrain.DefaultTerrainRenderPasses;
import net.sodium.client.render.chunk.terrain.TerrainRenderPass;
import net.sodium.client.render.chunk.translucent_sorting.SortBehavior;
import net.sodium.client.render.chunk.vertex.format.ChunkMeshFormats;
import net.sodium.client.render.chunk.vertex.format.NativeChunkMeshEncoder;
import net.sodium.client.render.chunk.vertex.format.NativeSectionMeshBuilder;
import net.sodium.client.services.PlatformBlockAccess;
import net.sodium.client.util.task.CancellationToken;
import net.sodium.client.world.LevelSlice;
import net.sodium.client.world.cloned.ChunkRenderContext;
import net.sodium.client.world.cloned.ClonedChunkSectionCache;
import org.joml.Vector3d;

import java.io.IOException;
import java.lang.reflect.Field;
import java.lang.management.GarbageCollectorMXBean;
import java.lang.management.ManagementFactory;
import java.nio.ByteBuffer;
import java.nio.ByteOrder;
import java.nio.file.Files;
import java.nio.file.Path;
import java.time.Instant;
import java.util.ArrayList;
import java.util.Comparator;
import java.util.LinkedHashMap;
import java.util.List;
import java.util.Locale;
import java.util.Map;

public final class RealChunkMeshingReplayRunner implements GameHooks {
    private static final Gson GSON = new GsonBuilder().disableHtmlEscaping().create();
    private static final String ENABLE_PROPERTY = "mattmc.realMeshingReplay";
    private static final String OUTPUT_PROPERTY = "mattmc.realMeshingReplay.output";
    private static final String WARMUP_PROPERTY = "mattmc.realMeshingReplay.warmup";
    private static final String WARMUP_SECONDS_PROPERTY = "mattmc.realMeshingReplay.warmupSeconds";
    private static final String MEASURE_PROPERTY = "mattmc.realMeshingReplay.measure";
    private static final String MEASURE_SECONDS_PROPERTY = "mattmc.realMeshingReplay.measureSeconds";
    private static final String REBUILDS_PER_SAMPLE_PROPERTY = "mattmc.realMeshingReplay.rebuildsPerSample";
    private static final String VALIDATE_EACH_SAMPLE_PROPERTY = "mattmc.realMeshingReplay.validateEachSample";
    private static final String RUST_PROFILE_PROPERTY = "mattmc.realMeshingReplay.rustProfile";
    private static final String FIXTURE_PROPERTY = "mattmc.realMeshingReplay.fixture";
    private static final String CAPTURE_INPUTS_PROPERTY = "mattmc.realMeshingReplay.captureInputs";
    private static final String WORLD_NAME = "MattMC Real Chunk Meshing Replay";
    private static final long WORLD_SEED = 0x4d6174744d435265L;
    private static final int SECTION_X = 0;
    private static final int SECTION_Y = 4;
    private static final int SECTION_Z = 0;
    private static final int SETTLE_TICKS_AFTER_LEVEL = 1;
    private static final int SETTLE_TICKS_AFTER_POPULATE = 1;

    private enum Phase {
        WAITING_FOR_LEVEL,
        POPULATING,
        WAITING_FOR_LIGHT,
        RUNNING,
        DONE
    }

    private final Path outputPath;
    private final int warmupIterations;
    private final int measurementIterations;
    private final long warmupNanos;
    private final long measurementNanos;
    private final int rebuildsPerSample;
    private final boolean validateEachSample;
    private final boolean captureInputs;
    private final List<Fixture> fixtures;
    private Phase phase = Phase.WAITING_FOR_LEVEL;
    private int ticksInPhase;

    private RealChunkMeshingReplayRunner(Path outputPath, int warmupIterations, int measurementIterations,
            long warmupNanos, long measurementNanos, int rebuildsPerSample, boolean validateEachSample,
            String selectedFixture) {
        this.outputPath = outputPath;
        this.warmupIterations = warmupIterations;
        this.measurementIterations = measurementIterations;
        this.warmupNanos = warmupNanos;
        this.measurementNanos = measurementNanos;
        this.rebuildsPerSample = Math.max(1, rebuildsPerSample);
        this.validateEachSample = validateEachSample;
        this.captureInputs = booleanProperty(CAPTURE_INPUTS_PROPERTY, false);
        List<Fixture> allFixtures = List.of(
                new Fixture("ordinary_terrain_m1"),
                new Fixture("empty"),
                new Fixture("dense_cube"),
                new Fixture("normal_surface"),
                new Fixture("foliage_tinted"),
                new Fixture("weighted_multipart"),
                new Fixture("fluid_heavy"),
                new Fixture("waterlogged"),
                new Fixture("translucent_heavy"),
                new Fixture("complex_static")
        );
        if (selectedFixture == null || selectedFixture.isBlank()) {
            this.fixtures = allFixtures;
        } else {
            this.fixtures = allFixtures.stream()
                    .filter(fixture -> fixture.name.equals(selectedFixture))
                    .toList();
            if (this.fixtures.isEmpty()) {
                throw new IllegalArgumentException("Unknown real chunk meshing replay fixture: " + selectedFixture);
            }
        }
    }

    public static void installIfRequested() {
        if (!Boolean.getBoolean(ENABLE_PROPERTY)) {
            return;
        }

        String output = System.getProperty(OUTPUT_PROPERTY);
        if (output == null || output.isBlank()) {
            throw new IllegalStateException("Missing required system property: " + OUTPUT_PROPERTY);
        }

        int warmup = Integer.getInteger(WARMUP_PROPERTY, 30);
        int measure = Integer.getInteger(MEASURE_PROPERTY, 80);
        long warmupNanos = secondsPropertyToNanos(WARMUP_SECONDS_PROPERTY);
        long measurementNanos = secondsPropertyToNanos(MEASURE_SECONDS_PROPERTY);
        int rebuildsPerSample = Integer.getInteger(REBUILDS_PER_SAMPLE_PROPERTY, 1);
        boolean validateEachSample = booleanProperty(VALIDATE_EACH_SAMPLE_PROPERTY, true);
        net.minecraft.hooks.HookRegistry.registerGameHook(new RealChunkMeshingReplayRunner(Path.of(output), warmup,
                measure, warmupNanos, measurementNanos, rebuildsPerSample, validateEachSample,
                System.getProperty(FIXTURE_PROPERTY)));
    }

    private static boolean booleanProperty(String property, boolean defaultValue) {
        String value = System.getProperty(property);
        if (value == null || value.isBlank()) {
            return defaultValue;
        }

        return switch (value.trim().toLowerCase(Locale.ROOT)) {
            case "1", "true", "yes", "on" -> true;
            case "0", "false", "no", "off" -> false;
            default -> throw new IllegalArgumentException("Invalid boolean value for " + property + ": " + value);
        };
    }

    private static long secondsPropertyToNanos(String property) {
        String value = System.getProperty(property, "0");
        try {
            double seconds = Double.parseDouble(value);
            return seconds <= 0.0 ? 0L : (long) (seconds * 1_000_000_000.0);
        } catch (NumberFormatException exception) {
            throw new IllegalArgumentException("Invalid seconds value for " + property + ": " + value, exception);
        }
    }

    @Override
    public void onGameInitialized(Minecraft minecraft) {
        LevelSettings settings = new LevelSettings(
                WORLD_NAME,
                GameType.CREATIVE,
                false,
                Difficulty.PEACEFUL,
                true,
                new GameRules(WorldDataConfiguration.DEFAULT.enabledFeatures()),
                WorldDataConfiguration.DEFAULT
        );

        minecraft.createWorldOpenFlows().createFreshLevel(
                uniqueWorldId(),
                settings,
                new WorldOptions(WORLD_SEED, false, false),
                WorldPresets::createFlatWorldDimensions,
                null
        );
    }

    @Override
    public void afterRunTick(Minecraft minecraft, boolean tick) {
        if (!tick || this.phase == Phase.DONE) {
            return;
        }

        try {
            this.tick(minecraft);
        } catch (Throwable throwable) {
            this.phase = Phase.DONE;
            try {
                this.writeFailure(throwable);
            } catch (IOException ignored) {
            }
            throwable.printStackTrace();
            minecraft.stop();
        }
    }

    private void tick(Minecraft minecraft) throws Exception {
        this.ticksInPhase++;

        if (this.phase == Phase.WAITING_FOR_LEVEL) {
            if (minecraft.level == null || minecraft.player == null || this.ticksInPhase < SETTLE_TICKS_AFTER_LEVEL) {
                return;
            }

            this.phase = Phase.POPULATING;
            this.ticksInPhase = 0;
            return;
        }

        if (this.phase == Phase.POPULATING) {
            if (!this.fixtureChunksLoaded(minecraft)) {
                return;
            }

            for (Fixture fixture : this.fixtures) {
                FixtureSection section = sectionForFixture(fixture.name);
                clearFixtureVolume(minecraft, section.sectionX, section.sectionZ);
            }
            this.phase = Phase.WAITING_FOR_LIGHT;
            this.ticksInPhase = 0;
            return;
        }

        if (this.phase == Phase.WAITING_FOR_LIGHT) {
            if (this.ticksInPhase < SETTLE_TICKS_AFTER_POPULATE) {
                return;
            }

            this.phase = Phase.RUNNING;
            this.ticksInPhase = 0;
            return;
        }

        if (this.phase == Phase.RUNNING) {
            ResultDocument document = this.runBenchmarks(minecraft);
            this.writeResults(document);
            this.phase = Phase.DONE;
            minecraft.stop();
        }
    }

    private boolean fixtureChunksLoaded(Minecraft minecraft) {
        for (Fixture fixture : this.fixtures) {
            FixtureSection section = sectionForFixture(fixture.name);
            if (!this.fixtureChunksLoaded(minecraft, section.sectionX, section.sectionZ)) {
                return false;
            }
        }
        return true;
    }

    private boolean fixtureChunksLoaded(Minecraft minecraft, int sectionX, int sectionZ) {
        for (int x = sectionX - 1; x <= sectionX + 1; x++) {
            for (int z = sectionZ - 1; z <= sectionZ + 1; z++) {
                try {
                    LevelChunk ignored = minecraft.level.getChunk(x, z);
                } catch (RuntimeException exception) {
                    return false;
                }
            }
        }
        return true;
    }

    private static void clearFixtureVolume(Minecraft minecraft, int sectionX, int sectionZ) {
        int minX = SectionPos.sectionToBlockCoord(sectionX) - 16;
        int minY = SectionPos.sectionToBlockCoord(SECTION_Y) - 2;
        int minZ = SectionPos.sectionToBlockCoord(sectionZ) - 16;
        int maxX = SectionPos.sectionToBlockCoord(sectionX) + 31;
        int maxY = minecraft.level.getMaxY();
        int maxZ = SectionPos.sectionToBlockCoord(sectionZ) + 31;
        BlockState air = Blocks.AIR.defaultBlockState();

        for (int y = minY; y <= maxY; y++) {
            for (int z = minZ; z <= maxZ; z++) {
                for (int x = minX; x <= maxX; x++) {
                    minecraft.level.setBlock(new BlockPos(x, y, z), air, 3);
                }
            }
        }
    }

    private static void populateFixture(Minecraft minecraft, String fixture, int sectionX, int sectionZ) {
        switch (fixture) {
            case "empty" -> {
            }
            case "dense_cube" -> fill(minecraft, sectionX, sectionZ, (x, y, z) -> Blocks.STONE.defaultBlockState());
            case "ordinary_terrain_m1" -> fill(minecraft, sectionX, sectionZ, (x, y, z) -> {
                if (y < 7) return Blocks.STONE.defaultBlockState();
                if (y <= 8) return Blocks.DIRT.defaultBlockState();
                return Blocks.AIR.defaultBlockState();
            });
            case "normal_surface" -> fill(minecraft, sectionX, sectionZ, (x, y, z) -> {
                if (y < 7) return Blocks.STONE.defaultBlockState();
                if (y == 7) return Blocks.DIRT.defaultBlockState();
                if (y == 8) return Blocks.GRASS_BLOCK.defaultBlockState();
                if (y == 9 && ((x + z) & 7) == 0) return Blocks.SHORT_GRASS.defaultBlockState();
                return Blocks.AIR.defaultBlockState();
            });
            case "foliage_tinted" -> fill(minecraft, sectionX, sectionZ, (x, y, z) -> {
                if (y < 4) return Blocks.DIRT.defaultBlockState();
                if (y == 4) return Blocks.GRASS_BLOCK.defaultBlockState();
                if (y >= 7 && y <= 12 && ((x * 31 + z * 17 + y) & 3) != 0) return Blocks.OAK_LEAVES.defaultBlockState();
                if (y == 5 && ((x + z) & 2) == 0) return Blocks.FERN.defaultBlockState();
                return Blocks.AIR.defaultBlockState();
            });
            case "weighted_multipart" -> fill(minecraft, sectionX, sectionZ, (x, y, z) -> {
                if (y < 3) return Blocks.STONE.defaultBlockState();
                if (y == 3 && ((x + z) & 1) == 0) return fenceState(x, z);
                if (y == 4 && ((x * 13 + z) & 3) == 0) return Blocks.REDSTONE_WIRE.defaultBlockState()
                        .setValue(RedStoneWireBlock.POWER, (x + z) & 15);
                if (y == 5 && ((x + z) % 5) == 0) return Blocks.OAK_STAIRS.defaultBlockState()
                        .setValue(StairBlock.FACING, switch ((x + z) & 3) {
                            case 0 -> net.minecraft.core.Direction.NORTH;
                            case 1 -> net.minecraft.core.Direction.SOUTH;
                            case 2 -> net.minecraft.core.Direction.WEST;
                            default -> net.minecraft.core.Direction.EAST;
                        })
                        .setValue(StairBlock.HALF, ((x ^ z) & 1) == 0 ? Half.BOTTOM : Half.TOP)
                        .setValue(StairBlock.SHAPE, StairsShape.STRAIGHT);
                return Blocks.AIR.defaultBlockState();
            });
            case "fluid_heavy" -> fill(minecraft, sectionX, sectionZ, (x, y, z) -> {
                if (y < 3) return Blocks.STONE.defaultBlockState();
                if (y >= 3 && y <= 11) {
                    int level = (x + z + y) & 7;
                    return Blocks.WATER.defaultBlockState().setValue(LiquidBlock.LEVEL, level);
                }
                return Blocks.AIR.defaultBlockState();
            });
            case "waterlogged" -> fill(minecraft, sectionX, sectionZ, (x, y, z) -> {
                if (y < 4) return Blocks.STONE.defaultBlockState();
                if (y == 4) return Blocks.WATER.defaultBlockState();
                if (y == 5 && ((x + z) & 1) == 0) return Blocks.OAK_SLAB.defaultBlockState()
                        .setValue(SlabBlock.TYPE, SlabType.BOTTOM)
                        .setValue(SlabBlock.WATERLOGGED, true);
                if (y == 5) return Blocks.SEAGRASS.defaultBlockState();
                return Blocks.AIR.defaultBlockState();
            });
            case "translucent_heavy" -> fill(minecraft, sectionX, sectionZ, (x, y, z) -> {
                if (y < 3) return Blocks.STONE.defaultBlockState();
                if (y >= 4 && y <= 12 && ((x + y + z) & 1) == 0) return Blocks.GLASS.defaultBlockState();
                if (y >= 4 && y <= 12) return Blocks.WATER.defaultBlockState();
                return Blocks.AIR.defaultBlockState();
            });
            case "complex_static" -> fill(minecraft, sectionX, sectionZ, (x, y, z) -> {
                if (y < 3) return Blocks.STONE_BRICKS.defaultBlockState();
                if (y == 3) return Blocks.GRASS_BLOCK.defaultBlockState();
                if (y == 4 && ((x + z) & 3) == 0) return Blocks.OAK_TRAPDOOR.defaultBlockState()
                        .setValue(TrapDoorBlock.HALF, Half.BOTTOM)
                        .setValue(TrapDoorBlock.OPEN, ((x ^ z) & 1) == 0)
                        .setValue(TrapDoorBlock.WATERLOGGED, false);
                if (y == 5 && ((x * 7 + z) & 7) == 0) return Blocks.COBWEB.defaultBlockState();
                if (y == 6 && ((x + z) & 3) == 1) return Blocks.OAK_LEAVES.defaultBlockState();
                if (y == 7 && ((x + z) & 7) == 2) return Blocks.LANTERN.defaultBlockState();
                if (y == 8 && ((x + z) & 7) == 3) return Blocks.GLASS_PANE.defaultBlockState();
                return Blocks.AIR.defaultBlockState();
            });
            default -> throw new IllegalArgumentException("Unknown fixture: " + fixture);
        }
    }

    private static BlockState fenceState(int x, int z) {
        return Blocks.OAK_FENCE.defaultBlockState()
                .setValue(FenceBlock.NORTH, (z & 1) == 0)
                .setValue(FenceBlock.SOUTH, (z & 2) == 0)
                .setValue(FenceBlock.WEST, (x & 1) == 0)
                .setValue(FenceBlock.EAST, (x & 2) == 0)
                .setValue(BlockStateProperties.WATERLOGGED, false);
    }

    private static void fill(Minecraft minecraft, int sectionX, int sectionZ, StateFactory factory) {
        int originX = SectionPos.sectionToBlockCoord(sectionX);
        int originY = SectionPos.sectionToBlockCoord(SECTION_Y);
        int originZ = SectionPos.sectionToBlockCoord(sectionZ);
        for (int y = 0; y < 16; y++) {
            for (int z = 0; z < 16; z++) {
                for (int x = 0; x < 16; x++) {
                    minecraft.level.setBlock(new BlockPos(originX + x, originY + y, originZ + z), factory.state(x, y, z), 3);
                }
            }
        }
    }

    private ResultDocument runBenchmarks(Minecraft minecraft) throws IOException {
        List<FixtureResult> results = new ArrayList<>();

        try (CloseableChunkBuildContext context = new CloseableChunkBuildContext(minecraft.level)) {
            for (Fixture fixture : this.fixtures) {
                FixtureSection section = sectionForFixture(fixture.name);
                System.out.println("[RealReplay] running fixture " + fixture.name);
                if (!this.fixtureChunksLoaded(minecraft, section.sectionX, section.sectionZ)) {
                    throw new IllegalStateException("Fixture chunks failed to load for " + fixture.name);
                }
                long setupStart = System.nanoTime();
                clearFixtureVolume(minecraft, section.sectionX, section.sectionZ);
                populateFixture(minecraft, fixture.name, section.sectionX, section.sectionZ);
                settleFixtureLighting(minecraft);
                long fixtureSetupNanos = System.nanoTime() - setupStart;
                results.add(this.runFixture(minecraft, context, fixture, section, fixtureSetupNanos));
                this.writeResults(new ResultDocument(results));
                System.out.println("[RealReplay] finished fixture " + fixture.name);
            }
        }

        return new ResultDocument(results);
    }

    private static void settleFixtureLighting(Minecraft minecraft) {
        for (int attempt = 0; attempt < 64; attempt++) {
            minecraft.level.pollLightUpdates();
            minecraft.level.getChunkSource().getLightEngine().runLightUpdates();

            if (!minecraft.level.getChunkSource().getLightEngine().hasLightWork()) {
                minecraft.level.pollLightUpdates();
                minecraft.level.getChunkSource().getLightEngine().runLightUpdates();

                if (!minecraft.level.getChunkSource().getLightEngine().hasLightWork()) {
                    return;
                }
            }
        }

        throw new IllegalStateException("Real chunk meshing replay fixture lighting did not settle");
    }

    private FixtureResult runFixture(Minecraft minecraft, ChunkBuildContext context, Fixture fixture,
                                     FixtureSection section, long fixtureSetupNanos) {
        SectionPos sectionPos = SectionPos.of(section.sectionX, SECTION_Y, section.sectionZ);
        long contextStart = System.nanoTime();
        ClonedChunkSectionCache cache = new ClonedChunkSectionCache(minecraft.level);
        ChunkRenderContext renderContext = LevelSlice.prepare(minecraft.level, sectionPos, cache);
        long renderContextNanos = System.nanoTime() - contextStart;

        if (renderContext == null) {
            return FixtureResult.empty(fixture.name, FixtureFingerprint.forFixture(minecraft, fixture.name, section),
                    new FixtureTiming(fixtureSetupNanos, renderContextNanos, 0L, 0L, 0L),
                    GcDelta.empty(), GcDelta.empty());
        }

        long fingerprintStart = System.nanoTime();
        FixtureFingerprint fingerprint = FixtureFingerprint.forFixture(minecraft, fixture.name, section);
        long fingerprintNanos = System.nanoTime() - fingerprintStart;
        JsonObject corpusInput = this.captureInputs
                ? captureCorpusInput(minecraft, context, renderContext, sectionPos, fixture.name)
                : null;

        List<Long> warmupTimes = new ArrayList<>();
        long warmupStart = System.nanoTime();
        GcSnapshot beforeWarmupGc = GcSnapshot.capture();
        while (warmupTimes.size() < this.warmupIterations || System.nanoTime() - warmupStart < this.warmupNanos) {
            int i = warmupTimes.size();
            System.out.println("[RealReplay] warmup " + fixture.name + " " + (i + 1));
            long start = System.nanoTime();
            ChunkBuildOutput output = executeTask(sectionPos, renderContext, context);
            warmupTimes.add(System.nanoTime() - start);
            if (output != null) {
                output.destroy();
            }
        }
        GcSnapshot afterWarmupGc = GcSnapshot.capture();

        List<Long> rebuildTimes = new ArrayList<>();
        List<Long> sampleBatchTimes = new ArrayList<>();
        List<Long> canonicalHashList = new ArrayList<>();
        ChunkSummary lastSummary = null;
        long validationNanos = 0L;
        GcSnapshot beforeMeasureGc = GcSnapshot.capture();
        long measureStart = System.nanoTime();
        while (sampleBatchTimes.size() < this.measurementIterations || System.nanoTime() - measureStart < this.measurementNanos) {
            int i = sampleBatchTimes.size();
            System.out.println("[RealReplay] measure execute " + fixture.name + " " + (i + 1));
            ChunkBuildOutput[] outputs = new ChunkBuildOutput[this.rebuildsPerSample];
            long start = System.nanoTime();
            for (int rebuild = 0; rebuild < this.rebuildsPerSample; rebuild++) {
                outputs[rebuild] = executeTask(sectionPos, renderContext, context);
            }
            long elapsed = System.nanoTime() - start;
            sampleBatchTimes.add(elapsed);
            rebuildTimes.add(elapsed / this.rebuildsPerSample);
            if (this.validateEachSample) {
                System.out.println("[RealReplay] measure summarize " + fixture.name + " " + (i + 1));
                long validationStart = System.nanoTime();
                lastSummary = summarize(fixture.name, outputs[outputs.length - 1]);
                canonicalHashList.add(lastSummary.canonicalHash());
                validationNanos += System.nanoTime() - validationStart;
            }
            for (ChunkBuildOutput output : outputs) {
                if (output != null) {
                    output.destroy();
                }
            }
        }
        GcSnapshot afterMeasureGc = GcSnapshot.capture();
        if (!this.validateEachSample) {
            long validationStart = System.nanoTime();
            ChunkBuildOutput output = executeTask(sectionPos, renderContext, context);
            lastSummary = summarize(fixture.name, output);
            canonicalHashList.add(lastSummary.canonicalHash());
            if (output != null) {
                output.destroy();
            }
            validationNanos += System.nanoTime() - validationStart;
        }
        long[] canonicalHashes = toLongArray(canonicalHashList);
        verifyStableCanonicalHashes(fixture.name, canonicalHashes);

        return new FixtureResult(fixture.name, false, toLongArray(rebuildTimes), toLongArray(sampleBatchTimes),
                toLongArray(warmupTimes), this.rebuildsPerSample, fingerprint, canonicalHashes,
                new FixtureTiming(fixtureSetupNanos, renderContextNanos, fingerprintNanos, validationNanos,
                        sampleBatchTimes.size() * (long) this.rebuildsPerSample),
                afterWarmupGc.deltaSince(beforeWarmupGc), afterMeasureGc.deltaSince(beforeMeasureGc), lastSummary,
                corpusInput);
    }

    private static JsonObject captureCorpusInput(Minecraft minecraft, ChunkBuildContext context,
            ChunkRenderContext renderContext, SectionPos sectionPos, String fixture) {
        context.cache.init(renderContext);
        LevelSlice slice = context.cache.getWorldSlice();
        JsonObject root = new JsonObject();
        root.addProperty("schema", "mattmc-real-meshing-input-v2");
        root.addProperty("fixture", fixture);
        root.addProperty("capture_hook", "after LevelSlice.prepare and BlockRenderCache.init, before ChunkBuilderMeshingTask.execute");
        root.addProperty("section_x", sectionPos.getX());
        root.addProperty("section_y", sectionPos.getY());
        root.addProperty("section_z", sectionPos.getZ());
        root.addProperty("origin_x", sectionPos.minBlockX());
        root.addProperty("origin_y", sectionPos.minBlockY());
        root.addProperty("origin_z", sectionPos.minBlockZ());
        root.addProperty("compact_snapshot_version", NativeChunkMeshEncoder.COMPACT_SECTION_SNAPSHOT_VERSION);
        root.addProperty("compact_padded_length", NativeChunkMeshEncoder.COMPACT_SECTION_PADDED_LENGTH);
        root.addProperty("compact_padded_radius", 1);
        root.add("level_slice_volume", volumeJson(renderContext));

        CaptureTables tables = new CaptureTables();
        root.add("level_slice_blocks", levelSliceBlocksJson(slice, renderContext, tables));
        JsonArray padded = new JsonArray(NativeChunkMeshEncoder.COMPACT_SECTION_PADDED_BLOCK_COUNT);
        int minX = sectionPos.minBlockX();
        int minY = sectionPos.minBlockY();
        int minZ = sectionPos.minBlockZ();
        for (int py = 0; py < NativeChunkMeshEncoder.COMPACT_SECTION_PADDED_LENGTH; py++) {
            int y = minY + py - 1;
            for (int pz = 0; pz < NativeChunkMeshEncoder.COMPACT_SECTION_PADDED_LENGTH; pz++) {
                int z = minZ + pz - 1;
                for (int px = 0; px < NativeChunkMeshEncoder.COMPACT_SECTION_PADDED_LENGTH; px++) {
                    int x = minX + px - 1;
                    BlockState state = slice.getBlockState(x, y, z);
                    JsonObject entry = new JsonObject();
                    entry.addProperty("x", px);
                    entry.addProperty("y", py);
                    entry.addProperty("z", pz);
                    entry.addProperty("state_table", tables.stateIndex(state));
                    entry.addProperty("native_state_id", NativeStaticBlockModelRegistry.getStateId(state));
                    entry.addProperty("light_word", computeLightWord(slice, state, x, y, z));
                    padded.add(entry);
                }
            }
        }
        root.add("padded_compact_grid", padded);

        JsonArray active = new JsonArray();
        BlockPos.MutableBlockPos blockPos = new BlockPos.MutableBlockPos();
        for (int y = 0; y < 16; y++) {
            for (int z = 0; z < 16; z++) {
                for (int x = 0; x < 16; x++) {
                    int wx = minX + x;
                    int wy = minY + y;
                    int wz = minZ + z;
                    blockPos.set(wx, wy, wz);
                    BlockState state = slice.getBlockState(wx, wy, wz);
                    if (state.isAir() && !state.hasBlockEntity()) {
                        continue;
                    }
                    FluidState fluid = state.getFluidState();
                    JsonObject entry = new JsonObject();
                    int localIndex = (y << 8) | (z << 4) | x;
                    long seed = state.getSeed(blockPos);
                    entry.addProperty("local_index", localIndex);
                    entry.addProperty("x", x);
                    entry.addProperty("y", y);
                    entry.addProperty("z", z);
                    entry.addProperty("state_table", tables.stateIndex(state));
                    entry.addProperty("fluid_table", tables.fluidIndex(fluid));
                    entry.addProperty("native_state_id", NativeStaticBlockModelRegistry.getStateId(state));
                    entry.addProperty("native_model", NativeStaticBlockModelRegistry.hasNativeModel(state));
                    entry.addProperty("block_id", irisBlockId(state));
                    entry.addProperty("seed_lo", (int) seed);
                    entry.addProperty("seed_hi", (int) (seed >>> 32));
                    var modelOffset = state.getOffset(blockPos);
                    entry.addProperty("model_offset_x", (float) modelOffset.x);
                    entry.addProperty("model_offset_y", (float) modelOffset.y);
                    entry.addProperty("model_offset_z", (float) modelOffset.z);
                    entry.addProperty("tint", blockTint(slice, state, blockPos));
                    entry.addProperty("fluid_tint", fluidTint(slice, fluid, blockPos));
                    var flow = fluid.isEmpty() ? net.minecraft.world.phys.Vec3.ZERO : fluid.getFlow(slice, blockPos);
                    entry.addProperty("fluid_flow_x", (float) flow.x);
                    entry.addProperty("fluid_flow_z", (float) flow.z);
                    entry.addProperty("fluid_block_id", fluid.isEmpty() ? -1 : irisFluidBlockId(fluid));
                    entry.addProperty("flags", 0);
                    active.add(entry);
                    captureModel(minecraft, tables, state, blockPos);
                }
            }
        }
        root.add("active_blocks", active);
        root.add("state_table", tables.stateTableJson());
        root.add("fluid_table", tables.fluidTableJson());
        root.add("model_bundle", tables.modelBundleJson());
        root.add("unsupported", tables.unsupportedJson());
        context.cleanup();
        return root;
    }

    private static JsonArray levelSliceBlocksJson(LevelSlice slice, ChunkRenderContext renderContext,
            CaptureTables tables) {
        var volume = renderContext.getVolume();
        JsonArray blocks = new JsonArray((volume.getXSpan() * volume.getYSpan()) * volume.getZSpan());
        BlockPos.MutableBlockPos pos = new BlockPos.MutableBlockPos();
        for (int y = volume.minY(); y <= volume.maxY(); y++) {
            for (int z = volume.minZ(); z <= volume.maxZ(); z++) {
                for (int x = volume.minX(); x <= volume.maxX(); x++) {
                    pos.set(x, y, z);
                    BlockState state = slice.getBlockState(x, y, z);
                    FluidState fluid = state.getFluidState();
                    JsonObject entry = new JsonObject();
                    entry.addProperty("x", x);
                    entry.addProperty("y", y);
                    entry.addProperty("z", z);
                    entry.addProperty("state_table", tables.stateIndex(state));
                    entry.addProperty("fluid_table", tables.fluidIndex(fluid));
                    entry.addProperty("block_light", slice.getBrightness(LightLayer.BLOCK, pos));
                    entry.addProperty("sky_light", slice.getBrightness(LightLayer.SKY, pos));
                    entry.addProperty("light_word", computeLightWord(slice, state, x, y, z));
                    entry.addProperty("has_block_entity", state.hasBlockEntity());
                    blocks.add(entry);
                }
            }
        }
        return blocks;
    }

    private static JsonObject volumeJson(ChunkRenderContext renderContext) {
        var volume = renderContext.getVolume();
        JsonObject object = new JsonObject();
        object.addProperty("min_x", volume.minX());
        object.addProperty("min_y", volume.minY());
        object.addProperty("min_z", volume.minZ());
        object.addProperty("max_x", volume.maxX());
        object.addProperty("max_y", volume.maxY());
        object.addProperty("max_z", volume.maxZ());
        object.addProperty("note", "LevelSlice copies this 2-block-radius production volume; compact Rust consumes the captured 1-block padded grid.");
        return object;
    }

    private static void captureModel(Minecraft minecraft, CaptureTables tables, BlockState state,
            BlockPos blockPos) {
        if (state.isAir() || state.getRenderShape() != RenderShape.MODEL || tables.hasModel(state)) {
            return;
        }

        JsonObject model = new JsonObject();
        int stateId = NativeStaticBlockModelRegistry.getStateId(state);
        int modelId = tables.nextModelId();
        int selectorId = tables.nextSelectorId();
        var material = DefaultMaterials.forBlockState(state);
        model.addProperty("state_table", tables.stateIndex(state));
        model.addProperty("state_key", stableStateKey(state));
        model.addProperty("native_state_id", stateId);
        model.addProperty("model_id", modelId);
        model.addProperty("selector_id", selectorId);
        model.addProperty("selector_kind", "direct-selected");
        model.addProperty("selector_weight", 1);
        model.addProperty("material_bits", material.bits());
        model.addProperty("pass", passName(material.pass));
        model.addProperty("pass_id", passId(material.pass));
        model.addProperty("state_flags", nativeStateFlags(state));
        model.addProperty("block_emission", state.getLightEmission());
        model.addProperty("block_id", irisBlockId(state));
        model.addProperty("tint_type", nativeTintType(state));
        var offsetType = state.sodium$getOffsetType();
        model.addProperty("offset_type", offsetType.ordinal());
        model.addProperty("offset_type_name", offsetType.name());
        model.addProperty("max_horizontal_offset", state.sodium$getMaxHorizontalOffset());
        model.addProperty("max_vertical_offset", state.sodium$getMaxVerticalOffset());

        JsonArray quads = new JsonArray();
        try {
            var blockModel = minecraft.getBlockRenderer().getBlockModel(state);
            List<BlockModelPart> parts = new ArrayList<>();
            blockModel.collectParts(RandomSource.create(state.getSeed(blockPos)), parts);
            model.addProperty("model_class", blockModel.getClass().getName());
            model.addProperty("part_count", parts.size());
            for (BlockModelPart part : parts) {
                boolean hasAo = part.useAmbientOcclusion() && state.getLightEmission() == 0;
                for (int faceIndex = -1; faceIndex < Direction.values().length; faceIndex++) {
                    Direction cullFace = faceIndex < 0 ? null : Direction.from3DDataValue(faceIndex);
                    for (BakedQuad quad : part.getQuads(cullFace)) {
                        JsonObject q = new JsonObject();
                        q.addProperty("material_bits", material.bits());
                        q.addProperty("pass_id", passId(material.pass));
                        q.addProperty("cull_face", cullFace == null ? -1 : cullFace.get3DDataValue());
                        q.addProperty("normal_face", quad.getNormalFace().ordinal());
                        q.addProperty("packed_normal", quad.getFaceNormal());
                        q.addProperty("block_emission", state.getLightEmission());
                        q.addProperty("render_type", 0);
                        q.addProperty("shade", quad.hasShade());
                        q.addProperty("flags", quad.getFlags());
                        q.addProperty("light_face", quad.getLightFace().get3DDataValue());
                        q.addProperty("tint_index", quad.getTintIndex());
                        q.addProperty("has_ao", hasAo);
                        q.addProperty("sprite", spriteName(quad.sprite()));
                        JsonArray vertices = new JsonArray();
                        for (int word : quad.vertices()) {
                            vertices.add(word);
                        }
                        q.add("vertices", vertices);
                        quads.add(q);
                    }
                }
            }
        } catch (RuntimeException exception) {
            tables.unsupported("model-capture-error", stableStateKey(state), exception.getClass().getName());
        }
        model.add("quads", quads);
        tables.model(state, model);
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
        return -1;
    }

    private static int nativeStateFlags(BlockState state) {
        int flags = NativeStaticBlockModelRegistry.fluidOverlayFlags(state);
        if (state.isAir()) flags |= NativeStaticBlockModelRegistry.STATE_FLAG_AIR;
        if (state.getRenderShape() == RenderShape.MODEL && NativeStaticBlockModelRegistry.hasNativeModel(state)) {
            flags |= NativeStaticBlockModelRegistry.STATE_FLAG_MODEL;
        }
        if (!state.getFluidState().isEmpty()) flags |= NativeStaticBlockModelRegistry.STATE_FLAG_FLUID;
        if (state.isSolidRender()) flags |= NativeStaticBlockModelRegistry.STATE_FLAG_SOLID_RENDER
                | NativeStaticBlockModelRegistry.STATE_FLAG_FULL_OCCLUSION;
        if (state.getBlock() instanceof LightBlock) flags |= NativeStaticBlockModelRegistry.STATE_FLAG_LIGHT_BLOCK;
        if (state.hasBlockEntity()) flags |= NativeStaticBlockModelRegistry.STATE_FLAG_BLOCK_ENTITY;
        if (state.canOcclude()) flags |= NativeStaticBlockModelRegistry.STATE_FLAG_CAN_OCCLUDE;
        if (state.blocksMotion()) flags |= NativeStaticBlockModelRegistry.STATE_FLAG_BLOCKS_MOTION;
        return flags;
    }

    private static int nativeTintType(BlockState state) {
        Block block = state.getBlock();
        if (block == Blocks.GRASS_BLOCK) return 1;
        if (block == Blocks.FERN || block == Blocks.SHORT_GRASS || block == Blocks.POTTED_FERN
                || block == Blocks.BUSH || block == Blocks.SUGAR_CANE || block == Blocks.PINK_PETALS
                || block == Blocks.WILDFLOWERS) return 10;
        if (block == Blocks.LARGE_FERN || block == Blocks.TALL_GRASS) return 7;
        if (block == Blocks.OAK_LEAVES || block == Blocks.JUNGLE_LEAVES || block == Blocks.ACACIA_LEAVES
                || block == Blocks.DARK_OAK_LEAVES || block == Blocks.VINE || block == Blocks.MANGROVE_LEAVES
                || block == Blocks.AZALEA_LEAVES
                || block == Blocks.FLOWERING_AZALEA_LEAVES) return 2;
        if (block == Blocks.WATER || block == Blocks.BUBBLE_COLUMN || block == Blocks.WATER_CAULDRON) return 3;
        if (block == Blocks.REDSTONE_WIRE) return 4;
        if (block == Blocks.MELON_STEM || block == Blocks.PUMPKIN_STEM) return 6;
        if (block == Blocks.SPRUCE_LEAVES) return 8;
        if (block == Blocks.BIRCH_LEAVES) return 9;
        if (block == Blocks.ATTACHED_MELON_STEM || block == Blocks.ATTACHED_PUMPKIN_STEM
                || block == Blocks.LILY_PAD || block == Blocks.BAMBOO || block == Blocks.POTTED_BAMBOO
                || block == Blocks.LEAF_LITTER) return 5;
        return 0;
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
        return (blockLight & 0xF)
                | ((skyLight & 0xF) << 4)
                | ((luminance & 0xF) << 8)
                | ((aoi & 0xFFFF) << 12)
                | ((emissive ? 1 : 0) << 28)
                | ((opaque ? 1 : 0) << 29)
                | ((fullOpaque ? 1 : 0) << 30)
                | ((fullCube ? 1 : 0) << 31);
    }

    private static int blockTint(LevelSlice slice, BlockState state, BlockPos pos) {
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
        return -1;
    }

    private static int fluidTint(LevelSlice slice, FluidState state, BlockPos pos) {
        if (state.is(Fluids.WATER) || state.is(Fluids.FLOWING_WATER)) {
            return BiomeColors.getAverageWaterColor(slice, pos) | 0xFF000000;
        }
        return -1;
    }

    private static int irisBlockId(BlockState state) {
        var ids = net.irisshaders.iris.shaderpack.materialmap.WorldRenderingSettings.INSTANCE.getBlockStateIds();
        return ids == null ? -1 : ids.getOrDefault(state, -1);
    }

    private static int irisFluidBlockId(FluidState state) {
        var ids = net.irisshaders.iris.shaderpack.materialmap.WorldRenderingSettings.INSTANCE.getBlockStateIds();
        return ids == null ? -1 : ids.getInt(state.createLegacyBlock());
    }

    private static final class CaptureTables {
        private final Map<String, Integer> states = new LinkedHashMap<>();
        private final Map<String, Integer> fluids = new LinkedHashMap<>();
        private final Map<String, JsonObject> models = new LinkedHashMap<>();
        private final JsonArray unsupported = new JsonArray();
        private int nextModelId;
        private int nextSelectorId;

        private int stateIndex(BlockState state) {
            return this.states.computeIfAbsent(stableStateKey(state), ignored -> this.states.size());
        }

        private int fluidIndex(FluidState state) {
            return this.fluids.computeIfAbsent(stableFluidKey(state), ignored -> this.fluids.size());
        }

        private boolean hasModel(BlockState state) {
            return this.models.containsKey(stableStateKey(state));
        }

        private int nextModelId() {
            return this.nextModelId++;
        }

        private int nextSelectorId() {
            return this.nextSelectorId++;
        }

        private void model(BlockState state, JsonObject model) {
            this.models.put(stableStateKey(state), model);
        }

        private void unsupported(String reason, String subject, String detail) {
            JsonObject object = new JsonObject();
            object.addProperty("reason", reason);
            object.addProperty("subject", subject);
            object.addProperty("detail", detail);
            this.unsupported.add(object);
        }

        private JsonArray stateTableJson() {
            JsonArray array = new JsonArray();
            for (Map.Entry<String, Integer> entry : this.states.entrySet()) {
                JsonObject object = new JsonObject();
                object.addProperty("index", entry.getValue());
                object.addProperty("key", entry.getKey());
                array.add(object);
            }
            return array;
        }

        private JsonArray fluidTableJson() {
            JsonArray array = new JsonArray();
            for (Map.Entry<String, Integer> entry : this.fluids.entrySet()) {
                JsonObject object = new JsonObject();
                object.addProperty("index", entry.getValue());
                object.addProperty("key", entry.getKey());
                array.add(object);
            }
            return array;
        }

        private JsonArray modelBundleJson() {
            JsonArray array = new JsonArray();
            for (JsonObject model : this.models.values()) {
                array.add(model);
            }
            return array;
        }

        private JsonArray unsupportedJson() {
            return this.unsupported;
        }
    }

    private static long[] toLongArray(List<Long> values) {
        long[] array = new long[values.size()];
        for (int i = 0; i < values.size(); i++) {
            array[i] = values.get(i);
        }
        return array;
    }

    private static ChunkBuildOutput executeTask(SectionPos sectionPos, ChunkRenderContext renderContext,
                                                ChunkBuildContext context) {
        RenderRegion region = new RenderRegion(sectionPos.getX() >> 3, sectionPos.getY() >> 2, sectionPos.getZ() >> 3, null);
        RenderSection renderSection = new RenderSection(region, sectionPos.getX(), sectionPos.getY(), sectionPos.getZ());
        ChunkBuilderMeshingTask task = new ChunkBuilderMeshingTask(renderSection, 0,
                new Vector3d(sectionPos.minBlockX() + 8.0, sectionPos.minBlockY() + 8.0, sectionPos.minBlockZ() + 8.0),
                renderContext, SortBehavior.STATIC, true);
        return task.execute(context, NonCancellingToken.INSTANCE);
    }

    private static ChunkSummary summarize(String fixture, ChunkBuildOutput output) {
        if (output == null) {
            return ChunkSummary.empty();
        }

        int passCount = 0;
        int totalVertexBytes = 0;
        int totalVertices = 0;
        long checksum = 1469598103934665603L;
        long canonicalHash = 1469598103934665603L;
        List<PassSummary> passes = new ArrayList<>();

        for (TerrainRenderPass pass : DefaultTerrainRenderPasses.ALL) {
            BuiltSectionMeshParts mesh = output.meshes.get(pass);
            if (mesh == null) {
                continue;
            }

            passCount++;
            ByteBuffer data = mesh.getVertexData().getDirectBuffer().duplicate();
            data.order(ByteOrder.nativeOrder());
            int bytes = data.remaining();
            int vertices = 0;
            for (int i = 0; i < mesh.getVertexSegments().length; i += 2) {
                vertices += mesh.getVertexSegments()[i];
            }
            totalVertexBytes += bytes;
            totalVertices += vertices;
            checksum = checksum(checksum, data.duplicate());
            PassSummary passSummary = PassSummary.fromMesh(fixture, passName(pass), output.render.getSectionIndex(),
                    mesh, data.duplicate(), vertices, bytes);
            canonicalHash = checksum(canonicalHash, passSummary.canonicalKey());
            passes.add(passSummary);
        }

        int blockEntities = count(output.info.globalBlockEntities) + count(output.info.culledBlockEntities);
        List<String> animatedSpriteNames = animatedSpriteNames(output.info.animatedSprites);
        return new ChunkSummary(passCount, totalVertices, totalVertexBytes, checksum, blockEntities,
                animatedSpriteNames.size(), animatedSpriteNames, fallbackBlocks(output), fallbackQuads(output),
                nativeProfile(output), canonicalHash, passes);
    }

    private static long checksum(long seed, ByteBuffer data) {
        long hash = seed;
        while (data.hasRemaining()) {
            hash ^= data.get() & 0xffL;
            hash *= 1099511628211L;
        }
        return hash;
    }

    private static long checksum(ByteBuffer data) {
        return checksum(1469598103934665603L, data);
    }

    private static long checksum(long seed, String value) {
        long hash = seed;
        for (int i = 0; i < value.length(); i++) {
            hash ^= value.charAt(i);
            hash *= 1099511628211L;
        }
        return hash;
    }

    private static long checksum(long seed, long value) {
        long hash = seed;
        for (int i = 0; i < Long.BYTES; i++) {
            hash ^= (value >>> (i * 8)) & 0xffL;
            hash *= 1099511628211L;
        }
        return hash;
    }

    private static int count(Object[] array) {
        return array == null ? 0 : array.length;
    }

    private static List<String> animatedSpriteNames(net.minecraft.client.renderer.texture.TextureAtlasSprite[] sprites) {
        if (sprites == null || sprites.length == 0) {
            return List.of();
        }

        List<String> names = new ArrayList<>(sprites.length);
        for (var sprite : sprites) {
            if (sprite != null && sprite.contents() != null) {
                names.add(sprite.contents().name().toString());
            }
        }
        names.sort(String::compareTo);
        return names;
    }

    private static int fallbackBlocks(ChunkBuildOutput output) {
        return intField(output.info, "nativeMeshingFallbackBlocks");
    }

    private static int fallbackQuads(ChunkBuildOutput output) {
        return intField(output.info, "nativeMeshingFallbackQuads");
    }

    private static long[] nativeProfile(ChunkBuildOutput output) {
        try {
            Field field = output.info.getClass().getField("nativeMeshingProfile");
            Object value = field.get(output.info);
            if (value instanceof long[] profile) {
                return profile.clone();
            }
        } catch (ReflectiveOperationException ignored) {
        }
        return new long[0];
    }

    private static int intField(Object target, String fieldName) {
        try {
            Field field = target.getClass().getField(fieldName);
            return field.getInt(target);
        } catch (ReflectiveOperationException ignored) {
            return 0;
        }
    }

    private static String passName(TerrainRenderPass pass) {
        if (pass == DefaultTerrainRenderPasses.SOLID) return "solid";
        if (pass == DefaultTerrainRenderPasses.CUTOUT) return "cutout";
        if (pass == DefaultTerrainRenderPasses.TRANSLUCENT) return "translucent";
        return "unknown";
    }

    private void writeResults(ResultDocument document) throws IOException {
        Files.createDirectories(this.outputPath.getParent());
        Files.writeString(this.outputPath, document.toJson());
    }

    private void writeFailure(Throwable throwable) throws IOException {
        Files.createDirectories(this.outputPath.getParent());
        Files.writeString(this.outputPath, "{\n" +
                "  \"status\": \"failed\",\n" +
                "  \"error\": " + quote(throwable.toString()) + "\n" +
                "}\n");
    }

    private static String uniqueWorldId() {
        return "mattmc-real-meshing-replay";
    }

    private static FixtureSection sectionForFixture(String fixture) {
        long seed = checksum(checksum(1469598103934665603L, WORLD_SEED), fixture);
        return new FixtureSection(SECTION_X, SECTION_Y, SECTION_Z, seed);
    }

    private static void verifyStableCanonicalHashes(String fixture, long[] hashes) {
        if (hashes.length < 2) {
            return;
        }
        long expected = hashes[0];
        for (int i = 1; i < hashes.length; i++) {
            if (hashes[i] != expected) {
                throw new IllegalStateException("Fixture " + fixture + " produced nondeterministic canonical mesh hash in one process: iteration 1="
                        + Long.toUnsignedString(expected) + ", iteration " + (i + 1) + "=" + Long.toUnsignedString(hashes[i]));
            }
        }
    }

    private interface StateFactory {
        BlockState state(int x, int y, int z);
    }

    private enum NonCancellingToken implements CancellationToken {
        INSTANCE;

        @Override
        public boolean isCancelled() {
            return false;
        }

        @Override
        public void setCancelled() {
        }
    }

    private static final class CloseableChunkBuildContext extends ChunkBuildContext implements AutoCloseable {
        private CloseableChunkBuildContext(net.minecraft.client.multiplayer.ClientLevel level) {
            super(level, ChunkMeshFormats.COMPACT);
        }

        @Override
        public void close() {
            this.cleanup();
        }
    }

    private record Fixture(String name) {
    }

    private record FixtureSection(int sectionX, int sectionY, int sectionZ, long fixtureSeed) {
    }

    private record FixtureFingerprint(long worldSeed, int sectionX, int sectionY, int sectionZ,
                                      long fixtureSeed, long blockFluidStateHash, long modelRegistryHash,
                                      long spriteMaterialMappingHash, long selectedWeightedMultipartChildHash,
                                      long selectedModelGeometryHash, long lightHash) {
        private static FixtureFingerprint forFixture(Minecraft minecraft, String fixture, FixtureSection section) {
            long blockFluid = 1469598103934665603L;
            long models = 1469598103934665603L;
            long spriteMaterials = 1469598103934665603L;
            long weightedSeeds = 1469598103934665603L;
            long selectedModelGeometry = 1469598103934665603L;
            long light = 1469598103934665603L;
            int originX = SectionPos.sectionToBlockCoord(section.sectionX);
            int originY = SectionPos.sectionToBlockCoord(section.sectionY);
            int originZ = SectionPos.sectionToBlockCoord(section.sectionZ);
            BlockPos.MutableBlockPos blockPos = new BlockPos.MutableBlockPos();

            for (int y = 0; y < 16; y++) {
                for (int z = 0; z < 16; z++) {
                    for (int x = 0; x < 16; x++) {
                        BlockState state = fixtureState(fixture, x, y, z);
                        FluidState fluidState = state.getFluidState();
                        String stateKey = stableStateKey(state);
                        String fluidKey = stableFluidKey(fluidState);
                        blockPos.set(originX + x, originY + y, originZ + z);

                        blockFluid = checksum(checksum(checksum(blockFluid, x), y), z);
                        blockFluid = checksum(blockFluid, stateKey);
                        blockFluid = checksum(blockFluid, fluidKey);

                        if (!state.isAir()) {
                            try {
                                var model = minecraft.getBlockRenderer().getBlockModel(state);
                                models = checksum(models, model.getClass().getName());
                                if (model.particleIcon() != null && model.particleIcon().contents() != null) {
                                    models = checksum(models, model.particleIcon().contents().name().toString());
                                }
                            } catch (RuntimeException exception) {
                                models = checksum(models, "model-error:" + exception.getClass().getName());
                            }

                            spriteMaterials = checksum(spriteMaterials, stateKey);
                            spriteMaterials = checksum(spriteMaterials, DefaultMaterials.forBlockState(state).bits());
                            spriteMaterials = checksum(spriteMaterials, passName(DefaultMaterials.forBlockState(state).pass));
                        }

                        if (!fluidState.isEmpty()) {
                            spriteMaterials = checksum(spriteMaterials, fluidKey);
                            spriteMaterials = checksum(spriteMaterials, DefaultMaterials.forFluidState(fluidState).bits());
                            spriteMaterials = checksum(spriteMaterials, passName(DefaultMaterials.forFluidState(fluidState).pass));
                        }

                        long modelSeed = state.getSeed(blockPos);
                        weightedSeeds = checksum(weightedSeeds, section.fixtureSeed);
                        weightedSeeds = checksum(weightedSeeds, modelSeed);
                        weightedSeeds = checksum(weightedSeeds, stateKey);

                        light = checksum(light, LevelRenderer.getLightColor(minecraft.level, blockPos));
                        selectedModelGeometry = checksumSelectedModelGeometry(selectedModelGeometry, minecraft, state, blockPos, modelSeed);
                    }
                }
            }

            return new FixtureFingerprint(WORLD_SEED, section.sectionX, section.sectionY, section.sectionZ,
                    section.fixtureSeed, blockFluid, models, spriteMaterials, weightedSeeds, selectedModelGeometry, light);
        }

        private void appendJson(StringBuilder builder, String indent) {
            builder.append("{\n");
            builder.append(indent).append("  \"world_seed\": ").append(quote(Long.toUnsignedString(this.worldSeed))).append(",\n");
            builder.append(indent).append("  \"section\": [").append(this.sectionX).append(", ")
                    .append(this.sectionY).append(", ").append(this.sectionZ).append("],\n");
            builder.append(indent).append("  \"fixture_seed\": ").append(quote(Long.toUnsignedString(this.fixtureSeed))).append(",\n");
            builder.append(indent).append("  \"block_fluid_state_hash\": ").append(quote(Long.toUnsignedString(this.blockFluidStateHash))).append(",\n");
            builder.append(indent).append("  \"model_selector_registry_hash\": ").append(quote(Long.toUnsignedString(this.modelRegistryHash))).append(",\n");
            builder.append(indent).append("  \"sprite_material_mapping_hash\": ").append(quote(Long.toUnsignedString(this.spriteMaterialMappingHash))).append(",\n");
            builder.append(indent).append("  \"selected_weighted_multipart_child_hash\": ")
                    .append(quote(Long.toUnsignedString(this.selectedWeightedMultipartChildHash))).append(",\n");
            builder.append(indent).append("  \"selected_model_geometry_hash\": ")
                    .append(quote(Long.toUnsignedString(this.selectedModelGeometryHash))).append(",\n");
            builder.append(indent).append("  \"light_hash\": ").append(quote(Long.toUnsignedString(this.lightHash))).append("\n");
            builder.append(indent).append("}");
        }
    }

    private static long checksumSelectedModelGeometry(long seed, Minecraft minecraft, BlockState state,
                                                      BlockPos blockPos, long modelSeed) {
        if (state.isAir() || state.getRenderShape() != net.minecraft.world.level.block.RenderShape.MODEL) {
            return checksum(seed, "no-model");
        }

        long hash = checksum(seed, stableStateKey(state));
        try {
            List<BlockModelPart> parts = new ArrayList<>();
            minecraft.getBlockRenderer().getBlockModel(state).collectParts(RandomSource.create(modelSeed), parts);
            hash = checksum(hash, parts.size());
            for (BlockModelPart part : parts) {
                hash = checksum(hash, part.getClass().getName());
                hash = checksum(hash, part.useAmbientOcclusion() ? 1L : 0L);
                hash = checksum(hash, spriteName(part.particleIcon()));
                for (int faceIndex = -1; faceIndex < Direction.values().length; faceIndex++) {
                    Direction direction = faceIndex < 0 ? null : Direction.from3DDataValue(faceIndex);
                    List<BakedQuad> quads = part.getQuads(direction);
                    hash = checksum(hash, faceIndex);
                    hash = checksum(hash, quads.size());
                    for (BakedQuad quad : quads) {
                        hash = checksum(hash, quad.direction().ordinal());
                        hash = checksum(hash, quad.tintIndex());
                        hash = checksum(hash, quad.shade() ? 1L : 0L);
                        hash = checksum(hash, quad.lightEmission());
                        hash = checksum(hash, spriteName(quad.sprite()));
                        for (int word : quad.vertices()) {
                            hash = checksum(hash, word);
                        }
                    }
                }
            }
        } catch (RuntimeException exception) {
            hash = checksum(hash, "model-geometry-error:" + exception.getClass().getName());
        }
        return hash;
    }

    private static String spriteName(net.minecraft.client.renderer.texture.TextureAtlasSprite sprite) {
        if (sprite == null || sprite.contents() == null) {
            return "";
        }
        return sprite.contents().name().toString();
    }

    private static String stableStateKey(BlockState state) {
        StringBuilder builder = new StringBuilder();
        builder.append(BuiltInRegistries.BLOCK.getKey(state.getBlock()));
        appendSortedProperties(builder, state.getValues());
        return builder.toString();
    }

    private static String stableFluidKey(FluidState state) {
        if (state.isEmpty()) {
            return "minecraft:empty";
        }
        StringBuilder builder = new StringBuilder();
        builder.append(BuiltInRegistries.FLUID.getKey(state.getType()));
        appendSortedProperties(builder, state.getValues());
        return builder.toString();
    }

    private static <T extends Comparable<T>> void appendSortedProperties(StringBuilder builder,
                                                                        Map<Property<?>, Comparable<?>> values) {
        values.entrySet().stream()
                .sorted(Comparator.comparing(entry -> entry.getKey().getName()))
                .forEach(entry -> appendProperty(builder, entry.getKey(), entry.getValue()));
    }

    @SuppressWarnings("unchecked")
    private static <T extends Comparable<T>> void appendProperty(StringBuilder builder, Property<?> property,
                                                                 Comparable<?> value) {
        Property<T> typed = (Property<T>) property;
        builder.append('[')
                .append(property.getName())
                .append('=')
                .append(typed.getName((T) value))
                .append(']');
    }

    private BenchmarkProtocol protocol() {
        return BenchmarkProtocol.current(this.warmupIterations, this.warmupNanos, this.measurementIterations,
                this.measurementNanos, this.rebuildsPerSample, this.validateEachSample);
    }

    private record ResultDocument(List<FixtureResult> fixtures) {
        private String toJson() {
            StringBuilder builder = new StringBuilder(16384);
            builder.append("{\n");
            builder.append("  \"status\": \"ok\",\n");
            builder.append("  \"timestamp\": ").append(quote(Instant.now().toString())).append(",\n");
            builder.append("  \"runner\": \"real-production-chunk-meshing\",\n");
            builder.append("  \"protocol\": ");
            BenchmarkProtocol.currentFromSystem().appendJson(builder, "  ");
            builder.append(",\n");
            builder.append("  \"fixtures\": [\n");
            for (int i = 0; i < this.fixtures.size(); i++) {
                if (i > 0) builder.append(",\n");
                this.fixtures.get(i).appendJson(builder, "    ");
            }
            builder.append("\n  ]\n");
            builder.append("}\n");
            return builder.toString();
        }
    }

    private record BenchmarkProtocol(int warmupIterations, long warmupNanos, int measurementIterations,
                                     long measurementNanos, int rebuildsPerSample, boolean validateEachSample,
                                     String rustProfile, String nativesDir, List<String> jvmArgs,
                                     Map<String, String> profileEnvironment) {
        private static BenchmarkProtocol current(int warmupIterations, long warmupNanos, int measurementIterations,
                                                 long measurementNanos, int rebuildsPerSample,
                                                 boolean validateEachSample) {
            return new BenchmarkProtocol(warmupIterations, warmupNanos, measurementIterations, measurementNanos,
                    rebuildsPerSample, validateEachSample, System.getProperty(RUST_PROFILE_PROPERTY, "unknown"),
                    System.getProperty("mattmc.rust.natives.dir", ""),
                    ManagementFactory.getRuntimeMXBean().getInputArguments(),
                    Map.of(
                            "MATTMC_PROFILE_STATIC_MODEL_SUBSTAGES", System.getenv().getOrDefault("MATTMC_PROFILE_STATIC_MODEL_SUBSTAGES", ""),
                            "MATTMC_PROFILE_SCAN_SUBSTAGES", System.getenv().getOrDefault("MATTMC_PROFILE_SCAN_SUBSTAGES", ""),
                            "MATTMC_PROFILE_STAGING_SUBSTAGES", System.getenv().getOrDefault("MATTMC_PROFILE_STAGING_SUBSTAGES", ""),
                            "MATTMC_PROFILE_FLUID_SUBSTAGES", System.getenv().getOrDefault("MATTMC_PROFILE_FLUID_SUBSTAGES", "")
                    ));
        }

        private static BenchmarkProtocol currentFromSystem() {
            int warmup = Integer.getInteger(WARMUP_PROPERTY, 30);
            int measure = Integer.getInteger(MEASURE_PROPERTY, 80);
            return current(warmup, secondsPropertyToNanos(WARMUP_SECONDS_PROPERTY), measure,
                    secondsPropertyToNanos(MEASURE_SECONDS_PROPERTY),
                    Math.max(1, Integer.getInteger(REBUILDS_PER_SAMPLE_PROPERTY, 1)),
                    booleanProperty(VALIDATE_EACH_SAMPLE_PROPERTY, true));
        }

        private void appendJson(StringBuilder builder, String indent) {
            builder.append("{\n");
            builder.append(indent).append("  \"warmup_iterations_min\": ").append(this.warmupIterations).append(",\n");
            builder.append(indent).append("  \"warmup_seconds_min\": ").append(this.warmupNanos / 1_000_000_000.0).append(",\n");
            builder.append(indent).append("  \"measurement_samples_min\": ").append(this.measurementIterations).append(",\n");
            builder.append(indent).append("  \"measurement_seconds_min\": ").append(this.measurementNanos / 1_000_000_000.0).append(",\n");
            builder.append(indent).append("  \"rebuilds_per_sample\": ").append(this.rebuildsPerSample).append(",\n");
            builder.append(indent).append("  \"reported_sample_unit\": \"nanoseconds per section rebuild, normalized from the timed batch\",\n");
            builder.append(indent).append("  \"timed_scope\": \"ChunkBuilderMeshingTask.execute only; fixture reset, LevelSlice.prepare, fingerprinting, canonicalization, JSON, logging, and output destroy are outside the timer\",\n");
            builder.append(indent).append("  \"validate_each_sample\": ").append(this.validateEachSample).append(",\n");
            builder.append(indent).append("  \"rust_profile\": ").append(quote(this.rustProfile)).append(",\n");
            builder.append(indent).append("  \"native_library_dir\": ").append(quote(this.nativesDir)).append(",\n");
            builder.append(indent).append("  \"jvm_args\": ");
            appendStringList(builder, this.jvmArgs);
            builder.append(",\n");
            builder.append(indent).append("  \"profile_environment\": {");
            int index = 0;
            for (Map.Entry<String, String> entry : this.profileEnvironment.entrySet()) {
                if (index++ > 0) builder.append(", ");
                builder.append(quote(entry.getKey())).append(": ").append(quote(entry.getValue()));
            }
            builder.append("}\n");
            builder.append(indent).append("}");
        }
    }

    private record GcSnapshot(long collections, long collectionTimeMillis) {
        private static GcSnapshot capture() {
            long collections = 0L;
            long time = 0L;
            for (GarbageCollectorMXBean bean : ManagementFactory.getGarbageCollectorMXBeans()) {
                long count = bean.getCollectionCount();
                long millis = bean.getCollectionTime();
                if (count > 0) collections += count;
                if (millis > 0) time += millis;
            }
            return new GcSnapshot(collections, time);
        }

        private GcDelta deltaSince(GcSnapshot before) {
            return new GcDelta(this.collections - before.collections,
                    this.collectionTimeMillis - before.collectionTimeMillis);
        }
    }

    private record GcDelta(long collections, long collectionTimeMillis) {
        private static GcDelta empty() {
            return new GcDelta(0L, 0L);
        }

        private void appendJson(StringBuilder builder) {
            builder.append("{\"collections\": ").append(this.collections)
                    .append(", \"collection_time_ms\": ").append(this.collectionTimeMillis)
                    .append("}");
        }
    }

    private record FixtureTiming(long fixtureResetAndPopulationNanos, long renderContextCreationNanos,
                                 long fingerprintNanos, long validationNanos, long executeInvocations) {
        private void appendJson(StringBuilder builder, String indent) {
            builder.append("{\n");
            builder.append(indent).append("  \"fixture_reset_and_population_ns\": ").append(this.fixtureResetAndPopulationNanos).append(",\n");
            builder.append(indent).append("  \"render_context_creation_ns\": ").append(this.renderContextCreationNanos).append(",\n");
            builder.append(indent).append("  \"fingerprinting_ns\": ").append(this.fingerprintNanos).append(",\n");
            builder.append(indent).append("  \"canonicalization_and_validation_ns\": ").append(this.validationNanos).append(",\n");
            builder.append(indent).append("  \"chunk_builder_execute_invocations\": ").append(this.executeInvocations).append(",\n");
            builder.append(indent).append("  \"sections_meshed\": ").append(this.executeInvocations).append("\n");
            builder.append(indent).append("}");
        }
    }

    private record FixtureResult(String name, boolean skippedByProductionEmptySection, long[] times,
                                 long[] sampleBatchTimes, long[] warmupTimes, int rebuildsPerSample,
                                 FixtureFingerprint fingerprint, long[] canonicalHashes,
                                 FixtureTiming timing, GcDelta warmupGc, GcDelta measurementGc,
                                 ChunkSummary summary, JsonObject corpusInput) {
        private static FixtureResult empty(String name, FixtureFingerprint fingerprint, FixtureTiming timing,
                                           GcDelta warmupGc, GcDelta measurementGc) {
            return new FixtureResult(name, true, new long[0], new long[0], new long[0], 1,
                    fingerprint, new long[0], timing, warmupGc, measurementGc, ChunkSummary.empty(), null);
        }

        private void appendJson(StringBuilder builder, String indent) {
            builder.append(indent).append("{\n");
            builder.append(indent).append("  \"name\": ").append(quote(this.name)).append(",\n");
            builder.append(indent).append("  \"skipped_by_production_empty_section\": ").append(this.skippedByProductionEmptySection).append(",\n");
            builder.append(indent).append("  \"iterations\": ").append(this.times.length).append(",\n");
            builder.append(indent).append("  \"mean_ns\": ").append(mean(this.times)).append(",\n");
            builder.append(indent).append("  \"median_ns\": ").append(median(this.times)).append(",\n");
            builder.append(indent).append("  \"best_ns\": ").append(best(this.times)).append(",\n");
            builder.append(indent).append("  \"rebuilds_per_sample\": ").append(this.rebuildsPerSample).append(",\n");
            builder.append(indent).append("  \"raw_rebuild_times_ns\": ");
            appendLongArray(builder, this.times);
            builder.append(",\n");
            builder.append(indent).append("  \"raw_sample_batch_times_ns\": ");
            appendLongArray(builder, this.sampleBatchTimes);
            builder.append(",\n");
            builder.append(indent).append("  \"warmup_execute_times_ns\": ");
            appendLongArray(builder, this.warmupTimes);
            builder.append(",\n");
            builder.append(indent).append("  \"timing\": ");
            this.timing.appendJson(builder, indent + "  ");
            builder.append(",\n");
            builder.append(indent).append("  \"warmup_gc\": ");
            this.warmupGc.appendJson(builder);
            builder.append(",\n");
            builder.append(indent).append("  \"measurement_gc\": ");
            this.measurementGc.appendJson(builder);
            builder.append(",\n");
            builder.append(indent).append("  \"fingerprint\": ");
            this.fingerprint.appendJson(builder, indent + "  ");
            builder.append(",\n");
            builder.append(indent).append("  \"iteration_canonical_hashes\": [");
            for (int i = 0; i < this.canonicalHashes.length; i++) {
                if (i > 0) builder.append(", ");
                builder.append(quote(Long.toUnsignedString(this.canonicalHashes[i])));
            }
            builder.append("],\n");
            builder.append(indent).append("  \"summary\": ");
            this.summary.appendJson(builder, indent + "  ");
            if (this.corpusInput != null) {
                builder.append(",\n");
                builder.append(indent).append("  \"corpus_input\": ");
                builder.append(GSON.toJson(this.corpusInput));
            }
            builder.append("\n").append(indent).append("}");
        }

        private static long mean(long[] values) {
            if (values.length == 0) return 0L;
            long total = 0L;
            for (long value : values) total += value;
            return total / values.length;
        }

        private static long median(long[] values) {
            if (values.length == 0) return 0L;
            long[] copy = values.clone();
            java.util.Arrays.sort(copy);
            return copy[copy.length / 2];
        }

        private static long best(long[] values) {
            if (values.length == 0) return 0L;
            long best = Long.MAX_VALUE;
            for (long value : values) best = Math.min(best, value);
            return best;
        }
    }

    private record ChunkSummary(int passCount, int totalVertices, int totalVertexBytes, long checksum,
                                int blockEntities, int animatedSprites, List<String> animatedSpriteNames,
                                int fallbackBlocks, int fallbackQuads,
                                long[] nativeProfile,
                                long canonicalHash,
                                List<PassSummary> passes) {
        private static ChunkSummary empty() {
            return new ChunkSummary(0, 0, 0, 1469598103934665603L, 0, 0, List.of(), 0, 0,
                    new long[0], 1469598103934665603L, List.of());
        }

        private void appendJson(StringBuilder builder, String indent) {
            builder.append("{\n");
            builder.append(indent).append("  \"pass_count\": ").append(this.passCount).append(",\n");
            builder.append(indent).append("  \"total_vertices\": ").append(this.totalVertices).append(",\n");
            builder.append(indent).append("  \"total_vertex_bytes\": ").append(this.totalVertexBytes).append(",\n");
            builder.append(indent).append("  \"checksum\": ").append(Long.toUnsignedString(this.checksum)).append(",\n");
            builder.append(indent).append("  \"canonical_hash\": ").append(quote(Long.toUnsignedString(this.canonicalHash))).append(",\n");
            builder.append(indent).append("  \"block_entities\": ").append(this.blockEntities).append(",\n");
            builder.append(indent).append("  \"animated_sprites\": ").append(this.animatedSprites).append(",\n");
            builder.append(indent).append("  \"animated_sprite_names\": [");
            for (int i = 0; i < this.animatedSpriteNames.size(); i++) {
                if (i > 0) builder.append(", ");
                builder.append(quote(this.animatedSpriteNames.get(i)));
            }
            builder.append("],\n");
            builder.append(indent).append("  \"fallback_blocks\": ").append(this.fallbackBlocks).append(",\n");
            builder.append(indent).append("  \"fallback_quads\": ").append(this.fallbackQuads).append(",\n");
            appendProfileJson(builder, indent, this.nativeProfile);
            builder.append(indent).append("  \"passes\": [");
            for (int i = 0; i < this.passes.size(); i++) {
                if (i > 0) builder.append(", ");
                this.passes.get(i).appendJson(builder);
            }
            builder.append("]\n");
            builder.append(indent).append("}");
        }
    }

    private static void appendProfileJson(StringBuilder builder, String indent, long[] nativeProfile) {
        builder.append(indent).append("  \"native_profile\": {");
        if (nativeProfile.length == NativeSectionMeshBuilder.Profile.METRIC_COUNT) {
            builder.append("\n");
            builder.append(indent).append("    \"stages_nanos\": {");
            for (int i = 0; i < NativeSectionMeshBuilder.Profile.STAGE_NAMES.length; i++) {
                if (i > 0) builder.append(", ");
                builder.append(quote(NativeSectionMeshBuilder.Profile.STAGE_NAMES[i]))
                        .append(": ").append(nativeProfile[i]);
            }
            builder.append("},\n");
            builder.append(indent).append("    \"counts\": {");
            int countOffset = NativeSectionMeshBuilder.Profile.STAGE_COUNT;
            for (int i = 0; i < NativeSectionMeshBuilder.Profile.COUNT_NAMES.length; i++) {
                if (i > 0) builder.append(", ");
                builder.append(quote(NativeSectionMeshBuilder.Profile.COUNT_NAMES[i]))
                        .append(": ").append(nativeProfile[countOffset + i]);
            }
            builder.append("}\n");
            builder.append(indent).append("  },\n");
        } else {
            builder.append("},\n");
        }
    }

    private record PassSummary(String name, int vertices, int bytes, long checksum, int[] vertexSegments,
                               List<CanonicalQuad> quads) {
        private static PassSummary fromMesh(String fixture, String name, int sectionIndex, BuiltSectionMeshParts mesh,
                                            ByteBuffer data, int vertices, int bytes) {
            int[] segments = mesh.getVertexSegments().clone();
            List<CanonicalQuad> quads = canonicalizeQuads(fixture, name, sectionIndex, data, segments);
            return new PassSummary(name, vertices, bytes, RealChunkMeshingReplayRunner.checksum(data.duplicate()), segments, quads);
        }

        private void appendJson(StringBuilder builder) {
            builder.append("{\"name\": ").append(quote(this.name))
                    .append(", \"vertices\": ").append(this.vertices)
                    .append(", \"bytes\": ").append(this.bytes)
                    .append(", \"checksum\": ").append(Long.toUnsignedString(this.checksum))
                    .append(", \"vertex_segments\": [");
            for (int i = 0; i < this.vertexSegments.length; i++) {
                if (i > 0) builder.append(", ");
                builder.append(this.vertexSegments[i]);
            }
            builder.append("], \"canonical_quads\": [");
            for (int i = 0; i < this.quads.size(); i++) {
                if (i > 0) builder.append(", ");
                this.quads.get(i).appendJson(builder);
            }
            builder.append("]}");
        }

        private String canonicalKey() {
            StringBuilder builder = new StringBuilder(this.quads.size() * 96);
            builder.append(this.name).append('|');
            for (CanonicalQuad quad : this.quads) {
                builder.append(quad.sortKey()).append('#').append(quad.rawKey()).append('\n');
            }
            return builder.toString();
        }
    }

    private static List<CanonicalQuad> canonicalizeQuads(String fixture, String pass, int sectionIndex, ByteBuffer data,
                                                         int[] vertexSegments) {
        data.order(ByteOrder.nativeOrder());
        List<CanonicalQuad> quads = new ArrayList<>();
        int vertexCursor = 0;
        int stride = 20;

        for (int segmentIndex = 0; segmentIndex < vertexSegments.length; segmentIndex += 2) {
            int vertexCount = vertexSegments[segmentIndex];
            if (vertexCount == 0) {
                continue;
            }

            int facingIndex = vertexSegments[segmentIndex + 1];
            String facing = facingName(facingIndex);
            for (int localVertex = 0; localVertex < vertexCount; localVertex += 4) {
                CanonicalVertex[] vertices = new CanonicalVertex[4];
                for (int vertex = 0; vertex < 4; vertex++) {
                    int offset = (vertexCursor + localVertex + vertex) * stride;
                    int positionHi = data.getInt(offset);
                    int positionLo = data.getInt(offset + 4);
                    int color = data.getInt(offset + 8);
                    int texture = data.getInt(offset + 12);
                    int lightMaterialSection = data.getInt(offset + 16);
                    vertices[vertex] = new CanonicalVertex(positionHi, positionLo, color, texture,
                            PackedVertexMetadata.decode(lightMaterialSection, sectionIndex));
                }
                quads.add(CanonicalQuad.from(fixture, pass, facing, facingIndex, vertices));
            }
            vertexCursor += vertexCount;
        }

        quads.sort(Comparator
                .comparing(CanonicalQuad::sortKey)
                .thenComparing(CanonicalQuad::rawKey));
        return quads;
    }

    private static String facingName(int facingIndex) {
        if (facingIndex >= 0 && facingIndex < ModelQuadFacing.VALUES.length) {
            return ModelQuadFacing.VALUES[facingIndex].name();
        }
        return "UNKNOWN_" + facingIndex;
    }

    private record CanonicalQuad(String pass, String facing, int facingIndex, int blockX, int blockY, int blockZ,
                                 String blockState, String producer, CanonicalVertex[] vertices, String sortKey,
                                 String rawKey) {
        private static CanonicalQuad from(String fixture, String pass, String facing, int facingIndex,
                                          CanonicalVertex[] vertices) {
            int minX = Integer.MAX_VALUE;
            int minY = Integer.MAX_VALUE;
            int minZ = Integer.MAX_VALUE;
            int maxX = Integer.MIN_VALUE;
            int maxY = Integer.MIN_VALUE;
            int maxZ = Integer.MIN_VALUE;

            StringBuilder raw = new StringBuilder(256);
            for (CanonicalVertex vertex : vertices) {
                int x = vertex.x20();
                int y = vertex.y20();
                int z = vertex.z20();
                minX = Math.min(minX, x);
                minY = Math.min(minY, y);
                minZ = Math.min(minZ, z);
                maxX = Math.max(maxX, x);
                maxY = Math.max(maxY, y);
                maxZ = Math.max(maxZ, z);
                raw.append(vertex.rawKey()).append('|');
            }

            int blockX = inferredBlockCoordinate(facing, 0, minX, maxX);
            int blockY = inferredBlockCoordinate(facing, 1, minY, maxY);
            int blockZ = inferredBlockCoordinate(facing, 2, minZ, maxZ);
            BlockState state = fixtureState(fixture, blockX, blockY, blockZ);
            String stateName = state.toString();
            String producer = inferProducer(pass, state);
            String sort = pass + '/' + producer + '/' + stateName + '/' + facing + '/' + blockX + '/' + blockY + '/' + blockZ + '/' + raw;
            return new CanonicalQuad(pass, facing, facingIndex, blockX, blockY, blockZ, stateName, producer,
                    vertices, sort, raw.toString());
        }

        private void appendJson(StringBuilder builder) {
            builder.append("{\"pass\": ").append(quote(this.pass))
                    .append(", \"producer\": ").append(quote(this.producer))
                    .append(", \"block_state\": ").append(quote(this.blockState))
                    .append(", \"facing\": ").append(quote(this.facing))
                    .append(", \"block\": [").append(this.blockX).append(", ").append(this.blockY).append(", ").append(this.blockZ).append("]")
                    .append(", \"vertices\": [");
            for (int i = 0; i < this.vertices.length; i++) {
                if (i > 0) builder.append(", ");
                this.vertices[i].appendJson(builder);
            }
            builder.append("]}");
        }
    }

    private static String inferProducer(String pass, BlockState state) {
        if (!state.getFluidState().isEmpty() && "translucent".equals(pass)) {
            return "fluid";
        }
        return "model";
    }

    private static BlockState fixtureState(String fixture, int x, int y, int z) {
        return switch (fixture) {
            case "ordinary_terrain_m1" -> {
                if (y < 7) yield Blocks.STONE.defaultBlockState();
                if (y <= 8) yield Blocks.DIRT.defaultBlockState();
                yield Blocks.AIR.defaultBlockState();
            }
            case "dense_cube" -> Blocks.STONE.defaultBlockState();
            case "normal_surface" -> {
                if (y < 7) yield Blocks.STONE.defaultBlockState();
                if (y == 7) yield Blocks.DIRT.defaultBlockState();
                if (y == 8) yield Blocks.GRASS_BLOCK.defaultBlockState();
                if (y == 9 && ((x + z) & 7) == 0) yield Blocks.SHORT_GRASS.defaultBlockState();
                yield Blocks.AIR.defaultBlockState();
            }
            case "foliage_tinted" -> {
                if (y < 4) yield Blocks.DIRT.defaultBlockState();
                if (y == 4) yield Blocks.GRASS_BLOCK.defaultBlockState();
                if (y >= 7 && y <= 12 && ((x * 31 + z * 17 + y) & 3) != 0) yield Blocks.OAK_LEAVES.defaultBlockState();
                if (y == 5 && ((x + z) & 2) == 0) yield Blocks.FERN.defaultBlockState();
                yield Blocks.AIR.defaultBlockState();
            }
            case "weighted_multipart" -> {
                if (y < 3) yield Blocks.STONE.defaultBlockState();
                if (y == 3 && ((x + z) & 1) == 0) yield fenceState(x, z);
                if (y == 4 && ((x * 13 + z) & 3) == 0) yield Blocks.REDSTONE_WIRE.defaultBlockState()
                        .setValue(RedStoneWireBlock.POWER, (x + z) & 15);
                if (y == 5 && ((x + z) % 5) == 0) yield Blocks.OAK_STAIRS.defaultBlockState()
                        .setValue(StairBlock.FACING, switch ((x + z) & 3) {
                            case 0 -> net.minecraft.core.Direction.NORTH;
                            case 1 -> net.minecraft.core.Direction.SOUTH;
                            case 2 -> net.minecraft.core.Direction.WEST;
                            default -> net.minecraft.core.Direction.EAST;
                        })
                        .setValue(StairBlock.HALF, ((x ^ z) & 1) == 0 ? Half.BOTTOM : Half.TOP)
                        .setValue(StairBlock.SHAPE, StairsShape.STRAIGHT);
                yield Blocks.AIR.defaultBlockState();
            }
            case "fluid_heavy" -> {
                if (y < 3) yield Blocks.STONE.defaultBlockState();
                if (y >= 3 && y <= 11) yield Blocks.WATER.defaultBlockState().setValue(LiquidBlock.LEVEL, (x + z + y) & 7);
                yield Blocks.AIR.defaultBlockState();
            }
            case "waterlogged" -> {
                if (y < 4) yield Blocks.STONE.defaultBlockState();
                if (y == 4) yield Blocks.WATER.defaultBlockState();
                if (y == 5 && ((x + z) & 1) == 0) yield Blocks.OAK_SLAB.defaultBlockState()
                        .setValue(SlabBlock.TYPE, SlabType.BOTTOM)
                        .setValue(SlabBlock.WATERLOGGED, true);
                if (y == 5) yield Blocks.SEAGRASS.defaultBlockState();
                yield Blocks.AIR.defaultBlockState();
            }
            case "translucent_heavy" -> {
                if (y < 3) yield Blocks.STONE.defaultBlockState();
                if (y >= 4 && y <= 12 && ((x + y + z) & 1) == 0) yield Blocks.GLASS.defaultBlockState();
                if (y >= 4 && y <= 12) yield Blocks.WATER.defaultBlockState();
                yield Blocks.AIR.defaultBlockState();
            }
            case "complex_static" -> {
                if (y < 3) yield Blocks.STONE_BRICKS.defaultBlockState();
                if (y == 3) yield Blocks.GRASS_BLOCK.defaultBlockState();
                if (y == 4 && ((x + z) & 3) == 0) yield Blocks.OAK_TRAPDOOR.defaultBlockState()
                        .setValue(TrapDoorBlock.HALF, Half.BOTTOM)
                        .setValue(TrapDoorBlock.OPEN, ((x ^ z) & 1) == 0)
                        .setValue(TrapDoorBlock.WATERLOGGED, false);
                if (y == 5 && ((x * 7 + z) & 7) == 0) yield Blocks.COBWEB.defaultBlockState();
                if (y == 6 && ((x + z) & 3) == 1) yield Blocks.OAK_LEAVES.defaultBlockState();
                if (y == 7 && ((x + z) & 7) == 2) yield Blocks.LANTERN.defaultBlockState();
                if (y == 8 && ((x + z) & 7) == 3) yield Blocks.GLASS_PANE.defaultBlockState();
                yield Blocks.AIR.defaultBlockState();
            }
            default -> Blocks.AIR.defaultBlockState();
        };
    }

    private static int inferredBlockCoordinate(String facing, int axis, int min, int max) {
        int minBlock = quantizedToBlock(min);
        int maxBlock = quantizedToBlock(max);
        if ((axis == 0 && "POS_X".equals(facing)) || (axis == 1 && "POS_Y".equals(facing)) ||
                (axis == 2 && "POS_Z".equals(facing))) {
            return clampBlock(maxBlock - 1);
        }
        return clampBlock(minBlock);
    }

    private static int quantizedToBlock(int quantized) {
        return (int) Math.floor(((quantized / (double) (1 << 20)) * 32.0) - 8.0 + 1.0e-7);
    }

    private static int clampBlock(int value) {
        return Math.max(0, Math.min(15, value));
    }

    private record PackedVertexMetadata(int rawWord, int blockLight, int skyLight, int materialBits,
                                        boolean mipmapped, int alphaCutoffOrdinal, int unusedMaterialBits,
                                        int sectionIndex, boolean sectionMatches) {
        private static PackedVertexMetadata decode(int rawWord, int expectedSectionIndex) {
            int blockLight = rawWord & 0xff;
            int skyLight = (rawWord >>> 8) & 0xff;
            int materialBits = (rawWord >>> 16) & 0xff;
            int sectionIndex = (rawWord >>> 24) & 0xff;
            return new PackedVertexMetadata(rawWord, blockLight, skyLight, materialBits,
                    (materialBits & 1) != 0, (materialBits >>> 1) & 0x3, materialBits & ~0x7,
                    sectionIndex, sectionIndex == (expectedSectionIndex & 0xff));
        }

        private String semanticKey() {
            return this.blockLight + "," + this.skyLight + "," + this.materialBits + "," + this.sectionIndex;
        }

        private void appendJson(StringBuilder builder) {
            builder.append("{\"block_light\": ").append(this.blockLight)
                    .append(", \"sky_light\": ").append(this.skyLight)
                    .append(", \"material_bits\": ").append(this.materialBits)
                    .append(", \"mipmapped\": ").append(this.mipmapped)
                    .append(", \"alpha_cutoff_ordinal\": ").append(this.alphaCutoffOrdinal)
                    .append(", \"unused_material_bits\": ").append(this.unusedMaterialBits)
                    .append(", \"section_index\": ").append(this.sectionIndex)
                    .append(", \"section_matches_output\": ").append(this.sectionMatches)
                    .append(", \"raw_word\": ").append(Integer.toUnsignedString(this.rawWord))
                    .append("}");
        }
    }

    private record CanonicalVertex(int positionHi, int positionLo, int color, int texture,
                                   PackedVertexMetadata metadata) {
        private int x20() {
            return ((this.positionHi & 0x3ff) << 10) | (this.positionLo & 0x3ff);
        }

        private int y20() {
            return (((this.positionHi >>> 10) & 0x3ff) << 10) | ((this.positionLo >>> 10) & 0x3ff);
        }

        private int z20() {
            return (((this.positionHi >>> 20) & 0x3ff) << 10) | ((this.positionLo >>> 20) & 0x3ff);
        }

        private String rawKey() {
            return Integer.toUnsignedString(this.positionHi) + ',' +
                    Integer.toUnsignedString(this.positionLo) + ',' +
                    Integer.toUnsignedString(this.color) + ',' +
                    Integer.toUnsignedString(this.texture) + ',' +
                    this.metadata.semanticKey();
        }

        private void appendJson(StringBuilder builder) {
            builder.append("{\"pos\": [").append(this.x20()).append(", ").append(this.y20()).append(", ").append(this.z20())
                    .append("], \"color\": ").append(Integer.toUnsignedString(this.color))
                    .append(", \"texture\": ").append(Integer.toUnsignedString(this.texture))
                    .append(", \"metadata\": ");
            this.metadata.appendJson(builder);
            builder.append("}");
        }
    }

    private static void appendLongArray(StringBuilder builder, long[] values) {
        builder.append("[");
        for (int i = 0; i < values.length; i++) {
            if (i > 0) builder.append(", ");
            builder.append(values[i]);
        }
        builder.append("]");
    }

    private static void appendStringList(StringBuilder builder, List<String> values) {
        builder.append("[");
        for (int i = 0; i < values.size(); i++) {
            if (i > 0) builder.append(", ");
            builder.append(quote(values.get(i)));
        }
        builder.append("]");
    }

    private static String quote(String value) {
        return "\"" + value.replace("\\", "\\\\").replace("\"", "\\\"") + "\"";
    }
}
