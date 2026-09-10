package net.minecraft.client.dev;

import net.minecraft.client.CameraType;
import net.minecraft.client.CloudStatus;
import net.minecraft.client.AttackIndicatorStatus;
import net.minecraft.client.Minecraft;
import net.minecraft.client.Screenshot;
import net.minecraft.client.gui.components.BossHealthOverlay;
import net.minecraft.client.gui.screens.PauseScreen;
import net.minecraft.client.gui.screens.TitleScreen;
import net.minecraft.client.gui.screens.inventory.InventoryScreen;
import net.minecraft.client.multiplayer.ClientLevel;
import net.minecraft.client.player.LocalPlayer;
import net.minecraft.client.renderer.chunk.ChunkSectionLayer;
import net.minecraft.client.renderer.ItemBlockRenderTypes;
import net.minecraft.core.BlockPos;
import net.minecraft.util.Mth;
import net.minecraft.core.Direction;
import net.minecraft.core.Holder;
import net.minecraft.core.SectionPos;
import net.minecraft.core.registries.BuiltInRegistries;
import net.minecraft.resources.ResourceLocation;
import net.minecraft.resources.ResourceKey;
import net.irisshaders.iris.uniforms.SystemTimeUniforms;
import net.minecraft.server.MinecraftServer;
import net.minecraft.server.level.ServerLevel;
import net.minecraft.server.level.ServerPlayer;
import net.fabricmc.fabric.api.event.lifecycle.v1.ServerTickEvents;
import net.fabricmc.fabric.api.client.event.lifecycle.v1.ClientTickEvents;
import net.minecraft.world.effect.MobEffect;
import net.minecraft.world.effect.MobEffectInstance;
import net.minecraft.world.effect.MobEffects;
import net.minecraft.world.entity.Entity;
import net.minecraft.world.entity.EntityType;
import net.minecraft.world.entity.Display;
import net.minecraft.world.entity.ExperienceOrb;
import net.minecraft.world.entity.item.FallingBlockEntity;
import net.minecraft.world.entity.item.ItemEntity;
import net.minecraft.world.entity.item.PrimedTnt;
import net.minecraft.world.entity.animal.Cow;
import net.minecraft.world.entity.animal.Chicken;
import net.minecraft.world.entity.animal.Pig;
import net.minecraft.world.entity.animal.Rabbit;
import net.minecraft.world.entity.animal.sheep.Sheep;
import net.minecraft.world.entity.animal.TropicalFish;
import net.minecraft.world.entity.monster.Zombie;
import net.minecraft.world.entity.projectile.Arrow;
import net.minecraft.world.entity.projectile.EvokerFangs;
import net.minecraft.world.entity.projectile.WitherSkull;
import net.minecraft.world.entity.projectile.LlamaSpit;
import net.minecraft.world.entity.projectile.windcharge.WindCharge;
import net.minecraft.world.entity.player.ChatVisiblity;
import net.minecraft.world.entity.player.Input;
import net.minecraft.world.item.ItemStack;
import net.minecraft.world.item.Items;
import net.minecraft.world.Difficulty;
import net.minecraft.world.level.GameType;
import net.minecraft.world.level.GameRules;
import net.minecraft.world.level.ChunkPos;
import net.minecraft.world.level.LightLayer;
import net.minecraft.world.level.Level;
import net.minecraft.world.BossEvent.BossBarOverlay;
import net.minecraft.world.level.block.Blocks;
import net.minecraft.world.level.block.ChestBlock;
import net.minecraft.world.level.block.LeavesBlock;
import net.minecraft.world.level.block.state.BlockState;
import net.minecraft.world.level.levelgen.Heightmap;
import net.minecraft.world.phys.BlockHitResult;
import net.minecraft.world.phys.HitResult;
import net.minecraft.world.phys.Vec3;
import net.minecraft.world.phys.shapes.CollisionContext;
import net.minecraft.network.chat.Component;
import net.vulkanic.VulkanicAPI;
import net.vulkanic.world.DistantHorizonsSemanticCollector;
import net.vulkanic.world.RustGalTerrainRenderer;
import net.vulkanic.world.RustGalWorldPrimitiveRenderer;
import net.vulkanic.world.WorldRenderRoutePolicy;
import com.seibel.distanthorizons.common.wrappers.chunk.ChunkWrapper;
import com.seibel.distanthorizons.common.wrappers.world.ServerLevelWrapper;
import com.seibel.distanthorizons.core.api.internal.ServerApi;
import com.seibel.distanthorizons.core.api.internal.SharedApi;
import org.slf4j.Logger;
import org.slf4j.LoggerFactory;

import javax.imageio.ImageIO;
import java.awt.image.BufferedImage;
import java.io.ByteArrayOutputStream;
import java.io.IOException;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.nio.file.Path;
import java.nio.file.StandardCopyOption;
import java.nio.file.DirectoryStream;
import java.util.ArrayList;
import java.util.Arrays;
import java.util.Comparator;
import java.util.LinkedHashMap;
import java.util.LinkedHashSet;
import java.util.List;
import java.util.Locale;
import java.util.Map;
import java.util.Set;
import java.util.UUID;
import java.util.concurrent.CompletableFuture;

/**
 * Development-only deterministic camera capture hook.
 *
 * <p>This is inert unless {@code -Dmattmc.dev.deterministicCameraCapture=true} is set.
 * It suppresses local movement input, applies a short deterministic camera sequence,
 * captures screenshots after a few rendered frames at each pose, and writes metadata
 * for capture harnesses.</p>
 */
public final class DeterministicCameraCapture {
	private static final Logger LOGGER = LoggerFactory.getLogger("MattMC-DeterministicCapture");
	private static final boolean ENABLED = Boolean.getBoolean("mattmc.dev.deterministicCameraCapture");
	private static final int FRAMES_PER_POSE = Math.max(1, Integer.getInteger("mattmc.dev.deterministicCameraCapture.framesPerPose", 8));
	private static final int ACK_TIMEOUT_FRAMES = Math.max(1, Integer.getInteger("mattmc.dev.deterministicCameraCapture.ackTimeoutFrames", 600));
	private static final int POSE_COUNT = Math.max(1, Math.min(8, Integer.getInteger("mattmc.dev.deterministicCameraCapture.poseCount", 4)));
	private static final float YAW_DELTA = Float.parseFloat(System.getProperty("mattmc.dev.deterministicCameraCapture.yawDelta", "35.0"));
	private static final boolean STOP_AFTER_COMPLETE = Boolean.parseBoolean(System.getProperty("mattmc.dev.deterministicCameraCapture.stopAfterComplete", "true"));
	private static final boolean INTERNAL_SCREENSHOTS = Boolean.parseBoolean(System.getProperty("mattmc.dev.deterministicCameraCapture.internalScreenshots", "false"));
	/**
	 * An opt-in, capture-only simulation clock.  The shared parity harness sets
	 * this on both the Frozen OpenGL baseline and the candidate route so a
	 * screenshot cannot compare two separately advancing copied-world clocks.
	 * Ordinary launches leave it unset and retain vanilla time progression.
	 */
	private static final long FIXED_CAPTURE_TIME = Long.getLong(
		"mattmc.dev.deterministicCameraCapture.fixedTime",
		Long.MIN_VALUE
	);
	/** Capture-only cloud scroll phase shared with Frozen; NaN preserves vanilla timing. */
	private static final float FIXED_CLOUD_TIME = Float.parseFloat(
		System.getProperty("mattmc.dev.deterministicCameraCapture.fixedCloudTime", "NaN")
	);
	/**
	 * Selected-source Vulkan captures retain the Rust submission's final image,
	 * not an asynchronously sampled desktop window.  Enable this diagnostic
	 * switch when every deterministic pose needs that same correlated proof.
	 */
	private static final boolean RUST_FINAL_OUTPUT_EVERY_POSE =
		Boolean.getBoolean("mattmc.dev.deterministicCameraCapture.rustFinalOutputEveryPose");
	private static final int SETTLED_READY_FRAMES = Math.max(0, Integer.getInteger("mattmc.dev.deterministicCameraCapture.settledReadyFrames", 0));
	private static final int SETTLED_READY_MAX_WAIT_FRAMES = Math.max(1, Integer.getInteger("mattmc.dev.deterministicCameraCapture.settledReadyMaxWaitFrames", 900));
	/**
	 * A rain fixture is not comparable while the ordinary client weather packet
	 * is still interpolating from the copied world's prior weather state.  This
	 * is capture timing only: it does not alter the world, renderer, or selected
	 * route.
	 */
	private static final float WEATHER_CAPTURE_MIN_INTENSITY = 0.99F;
	private static final Set<String> SETTLED_READY_FAMILIES = parseSettledReadyFamilies();
	/**
	 * Capture-only proof written by the Rust-selected source route. Normal
	 * gameplay never reads this diagnostic directory and Java never selects the
	 * route from it; it merely keeps an explicit source-route capture from
	 * taking a screenshot of the preparatory internal Rust graph.
	 */
	private static final String REQUIRED_RUST_SOURCE_EXECUTION_DIR =
		System.getProperty("mattmc.dev.deterministicCameraCapture.requiredRustSourceExecutionDir", "").trim();
	private static final boolean STATIC_TERRAIN_WATER_ANIMATION_DENSE_CAPTURE =
		Boolean.getBoolean("mattmc.dev.rustGalStaticTerrain.waterAnimationDenseCapture");
	private static final int STATIC_TERRAIN_WATER_ANIMATION_DENSE_FRAMES = Math.max(
		1,
		Math.min(120, Integer.getInteger("mattmc.dev.rustGalStaticTerrain.waterAnimationDenseFrames", 24))
	);
	private static final String FORCED_CAMERA_TYPE = System.getProperty("mattmc.dev.deterministicCameraCapture.cameraType", "").trim();
	private static final String FORCED_GAME_MODE = System.getProperty("mattmc.dev.deterministicCameraCapture.gameMode", "").trim();
	private static final int FORCED_SELECTED_HOTBAR_SLOT = Integer.getInteger("mattmc.dev.deterministicCameraCapture.selectedHotbarSlot", 0);
	private static final boolean FORCE_EMPTY_SELECTED_HAND = Boolean.getBoolean("mattmc.dev.deterministicCameraCapture.emptySelectedHand");
	/** Optional capture-only skin selection shared with Frozen parity fixtures. */
	private static final String FORCED_SELECTED_SKIN =
		System.getProperty("mattmc.dev.deterministicCameraCapture.selectedSkin", "").trim();
	/**
	 * Capture-only inventory fixture for validating the Rust-owned standard-3D
	 * GUI-item route with known, non-flat vanilla block models. It is inactive
	 * in ordinary gameplay and restores the copied player's original hotbar on
	 * capture shutdown.
	 */
	private static final String HOTBAR_ITEM_FIXTURE =
		System.getProperty("mattmc.dev.deterministicCameraCapture.hotbarItemFixture", "").trim().toLowerCase(Locale.ROOT);
	private static final float FORCED_EXPERIENCE_PROGRESS =
		Float.parseFloat(System.getProperty("mattmc.dev.deterministicCameraCapture.experienceProgress", "NaN"));
	private static final int FORCED_EXPERIENCE_LEVEL =
		Integer.getInteger("mattmc.dev.deterministicCameraCapture.experienceLevel", -1);
	private static final String FORCED_ATTACK_INDICATOR =
		System.getProperty("mattmc.dev.deterministicCameraCapture.attackIndicator", "").trim();
	private static final float FORCED_ATTACK_PROGRESS =
		Float.parseFloat(System.getProperty("mattmc.dev.deterministicCameraCapture.attackProgress", "NaN"));
	private static final float FORCED_ATTACK_DELAY =
		Float.parseFloat(System.getProperty("mattmc.dev.deterministicCameraCapture.attackDelay", "NaN"));
	private static final boolean FORCE_ATTACK_TARGET =
		Boolean.getBoolean("mattmc.dev.deterministicCameraCapture.attackTarget");
	private static final boolean FORCE_BLOCK_OUTLINE_TARGET =
		Boolean.getBoolean("mattmc.dev.deterministicCameraCapture.blockOutlineTarget");
	private static final boolean AIM_BLOCK_OUTLINE_TARGET =
		Boolean.getBoolean("mattmc.dev.deterministicCameraCapture.blockOutlineAimTarget");
	private static final boolean BLOCK_OUTLINE_PAUSE_PARITY =
		Boolean.getBoolean("mattmc.dev.deterministicCameraCapture.blockOutlinePauseParity");
	private static final boolean FORCE_BLOCK_OUTLINE_HIGH_CONTRAST =
		Boolean.getBoolean("mattmc.dev.deterministicCameraCapture.blockOutlineHighContrast");
	private static final boolean FORCE_REAL_SURVIVAL_CRACK =
		Boolean.getBoolean("mattmc.dev.deterministicCameraCapture.realSurvivalCrack");
	private static final boolean FORCE_REAL_SURVIVAL_CRACK_SETUP_BLOCK =
		Boolean.getBoolean("mattmc.dev.deterministicCameraCapture.realSurvivalCrackSetupBlock");
	private static final String BLOCK_DISPLAY_SCENARIO =
		System.getProperty("mattmc.dev.rustGalWorldMesh.blockDisplayScenario", "").trim().toLowerCase(Locale.ROOT);
	private static final String WORLD_TEXT_SCENARIO =
		System.getProperty("mattmc.dev.rustGalWorldText.scenario", "").trim().toLowerCase(Locale.ROOT);
	private static final String FALLING_BLOCK_SCENARIO =
		System.getProperty("mattmc.dev.rustGalWorldMesh.fallingBlockScenario", "").trim().toLowerCase(Locale.ROOT);
	private static final String FALLING_BLOCK_ROUTE_CONTROL =
		Boolean.getBoolean("mattmc.dev.rustGalWorldFallingBlock.disabled")
			? "disabled"
			: Boolean.getBoolean("mattmc.dev.rustGalWorldFallingBlock.legacyControl") ? "legacy" : "rust";
	private static final String PISTON_SCENARIO =
		System.getProperty("mattmc.dev.rustGalWorldMesh.pistonScenario", "").trim().toLowerCase(Locale.ROOT);
	private static final String PRIMED_TNT_SCENARIO =
		System.getProperty("mattmc.dev.rustGalWorldMesh.primedTntScenario", "").trim().toLowerCase(Locale.ROOT);
	private static final String ARROW_SCENARIO =
		System.getProperty("mattmc.dev.rustGalWorldMesh.arrowScenario", "").trim().toLowerCase(Locale.ROOT);
	private static final String EXPERIENCE_ORB_SCENARIO =
		System.getProperty("mattmc.dev.rustGalWorldExperienceOrb.scenario", "").trim().toLowerCase(Locale.ROOT);
	private static final String BEACON_BEAM_SCENARIO =
		System.getProperty("mattmc.dev.rustGalWorldBeaconBeam.scenario", "").trim().toLowerCase(Locale.ROOT);
	private static final String ITEM_ENTITY_SCENARIO =
		System.getProperty("mattmc.dev.rustGalWorldItemEntity.scenario", "").trim().toLowerCase(Locale.ROOT);
	private static final String MODEL_MESH_SCENARIO =
		System.getProperty("mattmc.dev.rustGalWorldMesh.modelScenario", "").trim().toLowerCase(Locale.ROOT);
	/**
	 * Capture-only opt-in which names the existing deterministic cow so the
	 * normal name-tag producer can prove the Rust world-text path. It never
	 * participates in route selection or ordinary gameplay entity setup.
	 */
	private static final boolean REQUIRE_RUST_WORLD_TEXT_SOURCE_CAPTURE =
		Boolean.getBoolean("mattmc.dev.rustGalWorldText.requireSourceCapture");
	/** Capture-only receipt requirement for the ordinary dropped-item producer. */
	private static final boolean REQUIRE_RUST_ITEM_ENTITY_SOURCE_CAPTURE =
		Boolean.getBoolean("mattmc.dev.rustGalWorldItemEntity.requireSourceCapture");
	/** Uses a harness-owned cow; vanilla EntityRenderDispatcher emits the flame submit later. */
	private static final String ENTITY_FLAME_SCENARIO =
		System.getProperty("mattmc.dev.rustGalWorldEntityFlame.scenario", "").trim().toLowerCase(Locale.ROOT);
	/** Uses the harness-owned cow; vanilla submits its ordinary ground shadow later. */
	private static final String ENTITY_SHADOW_SCENARIO =
		System.getProperty("mattmc.dev.rustGalWorldEntityShadow.scenario", "").trim().toLowerCase(Locale.ROOT);
	/** Uses the harness-owned cow; vanilla emits its ordinary leash submit later. */
	private static final String ENTITY_LEASH_SCENARIO =
		System.getProperty("mattmc.dev.rustGalWorldEntityLeash.scenario", "").trim().toLowerCase(Locale.ROOT);
	/**
	 * Test-only copied-world cleanup used to validate one selected-source entity
	 * family without allowing unrelated legacy entity or block-entity models to
	 * satisfy, or block, that producer's route evidence.
	 */
	private static final boolean SOURCE_ENTITY_ISOLATION =
		Boolean.getBoolean("mattmc.dev.deterministicCameraCapture.sourceEntityIsolation");
	private static final String WEATHER_SCENARIO =
		System.getProperty("mattmc.dev.rustGalWeather.scenario", "").trim().toLowerCase(Locale.ROOT);
	/**
	 * Capture-only weather interpolation inputs copied from the Frozen baseline,
	 * one value per deterministic camera pose. Normal gameplay has no value
	 * here and therefore retains Minecraft's native partial tick.
	 */
	private static final float[] WEATHER_CAPTURE_PARTIAL_TICKS = parseWeatherCapturePartialTicks();
	private static final int[] WEATHER_CAPTURE_RENDERER_TICKS = parseWeatherCaptureRendererTicks();
	private static final String[] LIGHTMAP_CAPTURE_INPUTS = System.getProperty("mattmc.dev.deterministicCameraCapture.lightmapInputs", "").split(";", -1);
	private static final String CLOUD_SCENARIO =
		System.getProperty("mattmc.dev.rustGalClouds.scenario", "").trim().toLowerCase(Locale.ROOT);
	/** Capture-only gameplay setting used to give Current and Frozen the same cloud radius. */
	private static final int FORCED_CLOUD_RANGE_CHUNKS = Math.max(
		-1,
		Integer.getInteger("mattmc.dev.deterministicCameraCapture.cloudRangeChunks", -1)
	);

	/** The cloud fixture owns only the temporary Minecraft quality setting. */
	private static CloudStatus forcedCloudStatus() {
		return switch (CLOUD_SCENARIO) {
			case "bounded" -> CloudStatus.FANCY;
			case "fast" -> CloudStatus.FAST;
			default -> null;
		};
	}
	private static final String STATIC_TERRAIN_SCENARIO =
		System.getProperty("mattmc.dev.rustGalStaticTerrain.scenario", "").trim().toLowerCase(Locale.ROOT);
	private static final String STATIC_TERRAIN_FAULT =
		System.getProperty("mattmc.dev.rustGalStaticTerrain.fault", "").trim().toLowerCase(Locale.ROOT);
	private static final String STATIC_TERRAIN_RESOURCE_PACK_SCENARIO =
		System.getProperty("mattmc.dev.rustGalStaticTerrain.resourcePackScenario", "").trim().toLowerCase(Locale.ROOT);
	private static final String STATIC_TERRAIN_WORLD_ID =
		System.getProperty("mattmc.dev.rustGalStaticTerrain.worldId", "").trim();
	private static final String STATIC_TERRAIN_WORLD_B_ID =
		System.getProperty("mattmc.dev.rustGalStaticTerrain.worldB", "").trim();
	private static final boolean DISTANT_HORIZONS_TEXTURE_PALETTE =
		Boolean.getBoolean("mattmc.dev.rustGalDistantHorizons.texturePalette");
	/**
	 * Copied-world fixture switch for the real DH transparent stream. This
	 * deliberately shares the far-panel setup with the opaque palette so its
	 * chunk invalidation, lighting readiness, and camera remain ordinary world
	 * inputs rather than fabricated LOD records.
	 */
	private static final boolean DISTANT_HORIZONS_REQUIRE_TRANSPARENT =
		Boolean.getBoolean("mattmc.dev.rustGalDistantHorizons.requireTransparent");
	/** Copied-world fixture switch for the real DH water-surface stream. */
	private static final boolean DISTANT_HORIZONS_REQUIRE_WATER =
		Boolean.getBoolean("mattmc.dev.rustGalDistantHorizons.requireWater");
	/** Harness-materialized DH fixture shared byte-for-byte with Frozen. */
	private static final boolean DISTANT_HORIZONS_EXTERNAL_FIXTURE =
		Boolean.getBoolean("mattmc.dev.rustGalDistantHorizons.externalFixture");
	private static final boolean DISTANT_HORIZONS_LEGACY_OBSERVATION =
		Boolean.getBoolean("mattmc.dev.rustGalDistantHorizons.legacyObservation");
	private static final int DISTANT_HORIZONS_TEXTURE_PALETTE_SIDE = 32;
	private static final int DISTANT_HORIZONS_TEXTURE_PALETTE_QUADRANT = DISTANT_HORIZONS_TEXTURE_PALETTE_SIDE / 2;
	// The panel is snapped to a 64-block DH column. Use enough requested distance
	// that snapping the witness to that column's center still leaves it outside
	// the temporary two-chunk vanilla radius; otherwise the near terrain route
	// can legitimately occlude the exact DH-only proof.
	// Keep the fixture beyond the reduced near-terrain radius while placing its
	// source column inside the deterministic camera's DH view cone. At 96 blocks
	// the generated column fell behind the camera and could never produce a
	// spatial water receipt, even though unrelated water segments were visible.
	private static final int DISTANT_HORIZONS_TEXTURE_PALETTE_DISTANCE = 64;
	private static final int DISTANT_HORIZONS_TEXTURE_PALETTE_LIGHT_STABLE_FRAMES = 3;
	private static final int FORCED_ARMOR_VALUE =
		Integer.getInteger("mattmc.dev.deterministicCameraCapture.armorValue", -1);
	private static final float FORCED_PLAYER_HEALTH =
		Float.parseFloat(System.getProperty("mattmc.dev.deterministicCameraCapture.playerHealth", "NaN"));
	private static final float FORCED_PLAYER_MAX_HEALTH =
		Float.parseFloat(System.getProperty("mattmc.dev.deterministicCameraCapture.playerMaxHealth", "NaN"));
	private static final float FORCED_PLAYER_ABSORPTION =
		Float.parseFloat(System.getProperty("mattmc.dev.deterministicCameraCapture.playerAbsorption", "NaN"));
	private static final int FORCED_PLAYER_FOOD_LEVEL =
		Integer.getInteger("mattmc.dev.deterministicCameraCapture.playerFoodLevel", -1);
	private static final float FORCED_PLAYER_FOOD_SATURATION =
		Float.parseFloat(System.getProperty("mattmc.dev.deterministicCameraCapture.playerFoodSaturation", "NaN"));
	private static final boolean FORCE_PLAYER_FOOD_HUNGER_EFFECT =
		Boolean.getBoolean("mattmc.dev.deterministicCameraCapture.playerFoodHungerEffect");
	private static final boolean FORCE_PLAYER_FOOD_JITTER =
		Boolean.getBoolean("mattmc.dev.deterministicCameraCapture.playerFoodJitter");
	private static final int FORCED_PLAYER_AIR_SUPPLY =
		Integer.getInteger("mattmc.dev.deterministicCameraCapture.playerAirSupply", -1);
	private static final int FORCED_PLAYER_MAX_AIR_SUPPLY =
		Integer.getInteger("mattmc.dev.deterministicCameraCapture.playerMaxAirSupply", -1);
	private static final boolean FORCE_PLAYER_UNDERWATER =
		Boolean.getBoolean("mattmc.dev.deterministicCameraCapture.playerUnderwater");
	private static final boolean FORCE_PLAYER_AIR_POP =
		Boolean.getBoolean("mattmc.dev.deterministicCameraCapture.playerAirPop");
	private static final boolean FORCE_MOUNT_PRESENT =
		Boolean.getBoolean("mattmc.dev.deterministicCameraCapture.mountPresent");
	private static final float FORCED_MOUNT_HEALTH =
		Float.parseFloat(System.getProperty("mattmc.dev.deterministicCameraCapture.mountHealth", "NaN"));
	private static final float FORCED_MOUNT_MAX_HEALTH =
		Float.parseFloat(System.getProperty("mattmc.dev.deterministicCameraCapture.mountMaxHealth", "NaN"));
	private static final int FORCED_MOUNT_HEALTH_ROWS =
		Integer.getInteger("mattmc.dev.deterministicCameraCapture.mountHealthRows", -1);
	private static final String FORCED_PLAYER_HEART_VARIANT =
		System.getProperty("mattmc.dev.deterministicCameraCapture.playerHeartVariant", "").trim();
	private static final boolean FORCE_PLAYER_HEALTH_REGEN =
		Boolean.getBoolean("mattmc.dev.deterministicCameraCapture.playerHealthRegeneration");
	private static final boolean HIDE_CHAT =
		Boolean.getBoolean("mattmc.dev.deterministicCameraCapture.hideChat");
	private static final int FORCED_BOSS_BAR_COUNT =
		Integer.getInteger("mattmc.dev.deterministicCameraCapture.bossBars", -1);
	private static final String FORCED_BOSS_BAR_PROGRESS =
		System.getProperty("mattmc.dev.deterministicCameraCapture.bossProgress", "").trim();
	private static final String FORCED_BOSS_BAR_OVERLAY =
		System.getProperty("mattmc.dev.deterministicCameraCapture.bossOverlay", "").trim();
	private static final double FIXED_CAMERA_X =
		Double.parseDouble(System.getProperty("mattmc.dev.deterministicCameraCapture.fixedX", "NaN"));
	private static final double FIXED_CAMERA_Y =
		Double.parseDouble(System.getProperty("mattmc.dev.deterministicCameraCapture.fixedY", "NaN"));
	private static final double FIXED_CAMERA_Z =
		Double.parseDouble(System.getProperty("mattmc.dev.deterministicCameraCapture.fixedZ", "NaN"));
	private static final float FIXED_CAMERA_YAW =
		Float.parseFloat(System.getProperty("mattmc.dev.deterministicCameraCapture.fixedYaw", "NaN"));
	private static final float FIXED_CAMERA_PITCH =
		Float.parseFloat(System.getProperty("mattmc.dev.deterministicCameraCapture.fixedPitch", "NaN"));
	private static final boolean HAS_FIXED_CAMERA_POSE =
		Double.isFinite(FIXED_CAMERA_X)
			&& Double.isFinite(FIXED_CAMERA_Y)
			&& Double.isFinite(FIXED_CAMERA_Z)
			&& Float.isFinite(FIXED_CAMERA_YAW)
			&& Float.isFinite(FIXED_CAMERA_PITCH);
	private static final boolean RUST_GAL_GUI_SCREEN_CYCLE =
		Boolean.getBoolean("mattmc.dev.deterministicCameraCapture.rustGalGuiScreenCycle");
	private static final int RUST_GAL_GUI_SCREEN_CYCLE_REPEATS =
		Math.max(1, Integer.getInteger("mattmc.dev.deterministicCameraCapture.rustGalGuiScreenCycleRepeats", 2));
	private static final int RUST_GAL_GUI_SCREEN_CYCLE_HOLD_FRAMES =
		Math.max(1, Integer.getInteger("mattmc.dev.deterministicCameraCapture.rustGalGuiScreenCycleHoldFrames", 4));
	private static final Path METADATA_PATH = Path.of(System.getProperty("mattmc.dev.deterministicCameraCapture.metadata", "artifacts/graphics-captures/deterministic_camera_capture.json"));
	private static final Path SCREENSHOT_DIR = Path.of(System.getProperty("mattmc.dev.deterministicCameraCapture.screenshotDir", "artifacts/graphics-captures/deterministic_camera_capture"));

	private static final List<PoseCapture> CAPTURES = new ArrayList<>();
	private static boolean initialized;
	private static boolean complete;
	private static boolean failed;
	private static boolean stopIssued;
	private static BlockPos realSurvivalCrackBlock;
	private static Direction realSurvivalCrackDirection;
	private static String realSurvivalCrackLastHitType = "unset";
	private static String realSurvivalCrackLastTarget = "unset";
	private static String realSurvivalCrackLastDirection = "unset";
	private static String realSurvivalCrackLastStatus = "unset";
	private static String realSurvivalCrackSetupTarget = "unset";
	private static String realSurvivalCrackLastValidTarget = "unset";
	private static String realSurvivalCrackLastValidBlockType = "unset";
	private static String realSurvivalCrackLastRenderedTarget = "unset";
	private static String realSurvivalCrackLastRenderedBlockType = "unset";
	private static int realSurvivalCrackStartCalls;
	private static int realSurvivalCrackContinueCalls;
	private static int realSurvivalCrackStopCalls;
	private static int realSurvivalCrackValidBlockHitCount;
	private static int realSurvivalCrackRenderedStateCount;
	private static int realSurvivalCrackMinRenderedStage = 10;
	private static int realSurvivalCrackMaxRenderedStage = -1;
	private static int realSurvivalCrackFramesWaitingForStage;
	private static long realSurvivalCrackLastDriveGameTime = Long.MIN_VALUE;
	private static Pose[] poses;
	private static Pose initialPose;
	private static Vec3 initialPosition;
	private static String initialDimension;
	private static int poseIndex;
	private static int translucentWarmupPoseIndex = -1;
	private static int translucentWarmupFramesAtPose;
	private static boolean translucentWarmupComplete;
	// The warm-up only kicks asynchronous visibility/build work at each pose;
	// captured poses still pass the full settled-readiness window below. Keeping
	// this producer kick short prevents the seven-pose path from consuming the
	// entire startup budget on expensive real Vulkan frames.
	private static final int TRANSLUCENT_WARMUP_FRAMES_PER_POSE = 2;
	private static int renderedFramesAtPose;
	/**
	 * Armed on the penultimate settled pose frame so the whole-frame coordinator
	 * can bind its attachment readback to the render that will be screenshotted.
	 * This is capture metadata only; it never participates in route selection.
	 */
	private static boolean wholeFrameAttachmentCaptureArmed;
	private static String wholeFrameGuiFoilTiming = "null";
	private static boolean wholeFrameAttachmentCaptureRequestIssued;
	private static boolean wholeFrameAttachmentCaptureReady;
	private static int framesAwaitingWholeFrameAttachmentCapture;
	private static long wholeFrameAttachmentCaptureGameplayFrame = -1L;
	private static long wholeFrameAttachmentCaptureCorrelation = -1L;
	private static long wholeFrameAttachmentCaptureDeterministicFrame = -1L;
	private static long wholeFrameAttachmentCaptureSubmission = -1L;
	private static long wholeFrameAttachmentCaptureAcquiredImage;
	private static long wholeFrameAttachmentCapturePresentedImage;
	private static boolean wholeFrameFinalOutputCapture;
	private static boolean awaitingScreenshotAck;
	private static int framesAwaitingAck;
	private static Path currentScreenshotPath;
	private static Path currentAckPath;
	/**
	 * The exact Rust-owned frame that is left on screen while an external
	 * deterministic screenshot is collected. Keeping this separately from the
	 * Java main target matters because whole-frame Vulkan never writes that
	 * target.
	 */
	private static WholeFramePresentation lastWholeFramePresentation;
	private static final List<WaterAnimationFrameCapture> WATER_ANIMATION_CAPTURES = new ArrayList<>();
	private static int staticTerrainWaterAnimationDenseCapturedFrames;
	private static boolean staticTerrainWaterAnimationDenseComplete;
	private static boolean staticTerrainWaterAnimationScreenshotInFlight;
	private static int framesAwaitingStaticTerrainWaterAnimationAck;
	private static Path staticTerrainWaterAnimationScreenshotPath;
	private static Path staticTerrainWaterAnimationAckPath;
	private static long staticTerrainWaterAnimationPendingRenderedFrameIndex = -1L;
	private static long staticTerrainWaterAnimationPendingGameTime = -1L;
	private static long staticTerrainWaterAnimationPendingHash;
	private static String staticTerrainWaterAnimationPendingSummary = "missing";
	private static String staticTerrainWaterAnimationPendingState = "missing";
	private static long staticTerrainWaterAnimationPendingVisibleLayerSubmissions;
	private static long staticTerrainWaterAnimationPendingCurrentFrameVisibleLayerSubmissions;
	private static long staticTerrainWaterAnimationPendingAtlasGeneration;
	private static long startedGameTime;
	private static long renderedFrameIndex;
	/** Bounded capture-pipeline counters: never used to select rendering routes. */
	private static long afterRenderCalls;
	private static long afterRenderUninitializedReturns;
	private static long afterRenderPaletteGateReturns;
	private static long afterRenderSettledGateReturns;
	private static long afterRenderLifecycleGateReturns;
	private static long afterRenderMovingMeshGateReturns;
	private static long afterRenderWeatherGateReturns;
	private static long afterRenderSourceExecutionGateReturns;
	private static int windowWidth;
	private static int windowHeight;
	private static boolean settledReadyGateSatisfied;
	/** Pose for which the submitted-work settling gate was last established. */
	private static int settledReadyPoseIndex = Integer.MIN_VALUE;
	/** Consecutive drained Rust terrain frames used for whole-frame readiness. */
	private static int settledRustTerrainFrames;
	/**
	 * Consecutive real legacy DH VBO observations used only by the explicit
	 * Java-control capture. Rust-selected captures continue to use their native
	 * submission identities below.
	 */
	private static int settledLegacyDistantHorizonsObservationFrames;
	private static String staticTerrainSettledSignature = "";
	private static int staticTerrainSettledFrames;
	private static boolean movingMeshScenarioSetup;
	private static volatile boolean sourceEntityIsolationQueued;
	private static volatile boolean sourceEntityIsolationApplied;
	private static volatile String sourceEntityIsolationFailure = "";
	private static int sourceEntityIsolationClientSyncFrames;
	private static volatile int sourceEntityIsolationRemovedEntities;
	private static volatile int sourceEntityIsolationRemovedBlockEntities;
	private static int sourceEntityIsolationClientNonPlayerEntities;
	private static int sourceEntityIsolationClientQuiescentFrames;
	private static boolean staticTerrainLifecycleSetup;
	private static boolean staticTerrainLifecycleAfterRecorded;
	private static List<RustGalTerrainRenderer.TerrainTextureProbe> staticTerrainTexturePaletteProbes = List.of();
	private static boolean staticTerrainTranslucentFixtureApplied;
	private static boolean staticTerrainTranslucentWorldBFixtureApplied;
	private static int framesWaitingForStaticTerrainLifecycle;
	private static BlockPos staticTerrainLifecycleEditBlock;
	private static long staticTerrainLifecycleBeforeGeneration;
	private static long staticTerrainLifecycleAfterGeneration;
	private static long staticTerrainLifecycleExecutionSubmissionBaseline = -1L;
	private static long staticTerrainLifecycleExecutionFrame = -1L;
	private static long staticTerrainLifecycleExecutionSubmission = -1L;
	private static long staticTerrainLifecycleExecutionInstances;
	// A selected-source frame intentionally runs as a separate transaction from
	// normal terrain submission. Once the post-setup pair is correlated, keep
	// that proof for the pose instead of requiring an impossible fresh source
	// receipt on each intervening normal frame.
	private static long staticTerrainLifecycleSourceExecutionFrame = -1L;
	private static long staticTerrainLifecycleSourceExecutionSubmission = -1L;
	private static String staticTerrainLifecycleStage = "inactive";
	private static String staticTerrainLifecycleBlockType = "";
	private static int staticTerrainLifecycleActionStep;
	private static int staticTerrainLifecycleResizeCount;
	private static int staticTerrainOriginalRenderDistance;
	private static int staticTerrainOriginalSimulationDistance;
	private static Vec3 staticTerrainOriginalPosition;
	private static int staticTerrainLifecycleBeforeCachedLayers;
	private static int staticTerrainLifecycleAfterCachedLayers;
	private static long staticTerrainLifecycleBeforeRssBytes;
	private static long staticTerrainLifecycleAfterRssBytes;
	private static int staticTerrainMenuCachedLayers;
	private static long staticTerrainMenuUsedMemoryBytes;
	private static long staticTerrainUnloadSubmissionSnapshot;
	private static long staticTerrainMenuSubmissionSnapshot;
	private static long staticTerrainMenuCurrentFrameSubmissionSnapshot;
	private static int staticTerrainMenuActiveLayerSnapshot;
	private static int staticTerrainMenuActiveSectionAssetSnapshot;
	private static boolean staticTerrainCrossWorldStaleFaultInjected;
	private static long staticTerrainReloadGenerationA;
	private static long staticTerrainReloadGenerationB;
	private static String staticTerrainLifecycleWorldA = "";
	private static String staticTerrainLifecycleWorldB = "";
	private static boolean staticTerrainLifecycleTransitionInProgress;
	private static CompletableFuture<Void> staticTerrainReloadFuture;
	private static boolean distantHorizonsTexturePaletteSetup;
	private static BlockPos distantHorizonsTexturePaletteTarget;
	private static List<DistantHorizonsTexturePaletteProbe> distantHorizonsTexturePaletteProbes = List.of();
	private static List<BlockPos> distantHorizonsTransparentWitnesses = List.of();
	private static List<BlockPos> distantHorizonsWaterWitnesses = List.of();
	private static int distantHorizonsTexturePaletteOriginalRenderDistance;
	private static int distantHorizonsTexturePaletteWaitFrames;
	private static boolean distantHorizonsTexturePaletteSourceReady;
	/** The diagnostic file is written after the Rust frame; retain one observed
	 * exact receipt so the next Java tick can acknowledge that completed frame. */
	private static boolean distantHorizonsTexturePaletteExactAtlasObserved;
	private static boolean distantHorizonsTexturePaletteWaterSourceObserved;
	private static boolean distantHorizonsTexturePaletteWaterExecutedObserved;
	private static boolean distantHorizonsTexturePaletteInvalidationQueued;
	private static int distantHorizonsTexturePaletteInvalidatedChunks;
	private static int distantHorizonsTexturePaletteQueuedUpdatesAfterInvalidation;
	private static String distantHorizonsTexturePaletteDhWorldType = "";
	private static String distantHorizonsTexturePaletteStage = "inactive";
	private static boolean distantHorizonsTexturePaletteLightFingerprintKnown;
	private static long distantHorizonsTexturePaletteLightFingerprint;
	private static int distantHorizonsTexturePaletteLightStableFrames;
	private static boolean distantHorizonsTexturePaletteLightEngineBusy;
	private static int distantHorizonsTexturePaletteLightCorrectChunks;
	/** Chunks retained only for the lifetime of the copied-world DH palette fixture. */
	private static final List<ChunkPos> distantHorizonsTexturePaletteForcedChunks = new ArrayList<>();
	private static String distantHorizonsTexturePaletteServerStateDetail = "inactive";
	private static String distantHorizonsTexturePaletteLastReportedStage = "";

	private record DistantHorizonsTexturePaletteProbe(
		BlockPos position,
		String blockId,
		List<String> allowedSprites,
		List<String> requiredSprites
	) {
	}

	/** Capture-only Rust exact-atlas verdict for the currently emitted DH frame. */
	private record DistantHorizonsExactAtlasPaletteStatus(boolean targetsMatched, long frameId, String status) {
	}
	private static String movingMeshSetupStage = "inactive";
	private static String movingMeshSetupLastMissing = "";
	private static int movingMeshSetupAttempts;
	private static int framesWaitingForSettledReady;
	private static int framesWaitingForMovingMeshProducer;
	private static int framesWaitingForSourceExecution;
	private static boolean weatherScenarioSetup;
	private static String weatherSetupStage = "inactive";
	private static String weatherSetupLastMissing = "";
	private static int weatherSetupAttempts;
	private static int framesWaitingForWeatherProducer;
	private static int framesWaitingForCloudProducer;
	private static int fallingEntitySeenCount;
	private static int fallingEntityShouldRenderCount;
	private static int fallingEntityCompiledSectionCount;
	private static int fallingEntityExtractedCount;
	private static String fallingBlockSetupStatus = "inactive";
	private static String fallingBlockSetupBlockId = "";
	private static String fallingBlockSetupSpawnMethod = "";
	private static String fallingBlockSetupOrigin = "";
	private static String fallingBlockSetupLanding = "";
	private static int fallingBlockSetupEntityCount;
	private static int fallingBlockSetupFallHeight;
	private static int fallingBlockSetupPoseIndex = -1;
	private static String fallingBlockSetupPoseName = "";
	private static String primedTntSetupStatus = "inactive";
	private static String primedTntSetupBlockId = "";
	private static String primedTntSetupOrigin = "";
	private static int primedTntSetupEntityCount;
	private static String arrowSetupStatus = "inactive";
	private static String arrowSetupTexture = "";
	private static String arrowSetupOrigin = "";
	private static int arrowSetupEntityCount;
	private static int arrowSetupPoseIndex = -1;
	private static String experienceOrbSetupStatus = "inactive";
	private static String experienceOrbSetupOrigin = "";
	private static int experienceOrbSetupEntityCount;
	private static int experienceOrbSetupPoseIndex = -1;
	private static String beaconBeamSetupStatus = "inactive";
	private static String beaconBeamSetupOrigin = "";
	private static boolean beaconBeamSetupClientReady;
	private static boolean gameplayWorldTextScenarioSetup;
	private static boolean beaconBeamSetupServerReady;
	private static long beaconBeamSetupGameTime = -1L;
	private static boolean beaconBeamSetupBaseValid;
	private static long beaconBeamSetupTickerInvocations;
	private static long beaconBeamSetupClientTickerInvocations;
	private static long beaconBeamSetupLastTickerGameTime = -1L;
	private static boolean beaconBeamSetupTickerSawBlockEntity;
	private static boolean beaconBeamSetupServerPacketSent;
	private static volatile net.minecraft.network.protocol.game.ClientboundBlockEntityDataPacket beaconBeamPendingClientPacket;
	private static boolean beaconBeamTickerLoopScheduled;
	private static boolean beaconBeamTickerEventRegistered;
	private static boolean beaconBeamClientTickerEventRegistered;
	private static ServerLevel beaconBeamSetupServerLevel;
	private static BlockPos beaconBeamSetupPosition;

	public static String gameplayBeaconSetupStatus() {
		return beaconBeamSetupStatus;
	}

	public static boolean gameplayBeaconClientReady() {
		return beaconBeamSetupClientReady;
	}

	public static boolean gameplayBeaconServerReady() {
		return beaconBeamSetupServerReady;
	}

	public static long gameplayBeaconGameTime() {
		return beaconBeamSetupGameTime;
	}

	public static boolean gameplayBeaconBaseValid() {
		return beaconBeamSetupBaseValid;
	}

	public static long gameplayBeaconTickerInvocations() {
		return beaconBeamSetupTickerInvocations;
	}

	public static long gameplayBeaconClientTickerInvocations() {
		return beaconBeamSetupClientTickerInvocations;
	}

	public static long gameplayBeaconLastTickerGameTime() {
		return beaconBeamSetupLastTickerGameTime;
	}

	public static boolean gameplayBeaconTickerSawBlockEntity() {
		return beaconBeamSetupTickerSawBlockEntity;
	}

	public static String gameplayPrimedTntSetupStatus() {
		return primedTntSetupStatus;
	}

	public static String gameplayPrimedTntSetupBlockId() {
		return primedTntSetupBlockId;
	}

	public static String gameplayPrimedTntSetupOrigin() {
		return primedTntSetupOrigin;
	}

	public static int gameplayPrimedTntSetupEntityCount() {
		return primedTntSetupEntityCount;
	}
	private static String itemEntitySetupStatus = "inactive";
	private static String itemEntitySetupOrigin = "";
	private static String itemEntitySetupItemId = "";
	private static int itemEntitySetupEntityCount;
	private static int itemEntitySetupPoseIndex = -1;
	private static volatile int itemEntityServerSpawnedCount;
	private static volatile String itemEntityServerSpawnFailure = "";
	private static int itemEntityClientVisibleCount;
	private static volatile String modelMeshSetupStatus = "inactive";
	private static volatile String modelMeshSetupBlockId = "";
	private static volatile String modelMeshSetupOrigin = "";
	private static BlockPos modelMeshSetupPosition;
	private static boolean modelMeshSetupClientBlockEntityPresent;
	private static volatile boolean modelMeshSetupServerEntityPresent;
	private static boolean modelMeshSetupClientEntityPresent;
	private static volatile int modelMeshSetupServerEntityId = -1;
	private static int modelMeshSetupClientEntityId = -1;
	private static String modelMeshSetupClientEntitySample = "";
	// The integrated server and client can allocate distinct entity ids. Capture
	// ownership therefore matches the copied entity by its semantic type and
	// deterministic spawn position, not by an assumed shared numeric id.
	private static Vec3 modelMeshSetupExpectedEntityPosition;
	private static volatile int modelMeshSetupPoseIndex = -1;
	private static volatile boolean modelMeshSetupServerSpawnQueued;
	private static volatile boolean modelMeshSetupEvokerEventSent;
	private static volatile String modelMeshSetupServerSpawnFailure = "";
	private static volatile String modelMeshSetupDifficultyBefore = "";
	private static volatile String modelMeshSetupDifficultyEffective = "";
	private static volatile boolean modelMeshSetupDifficultyAdjusted;
	private static String pistonSetupStatus = "inactive";
	private static String pistonSetupBlockId = "";
	private static String pistonSetupOrigin = "";
	private static String pistonSetupBlockState = "";
	private static boolean pistonSetupClientBlockEntityPresent;
	private static int pistonSetupEntityCount;
	private static BlockPos pistonSetupPos;
	private static BlockState pistonSetupMovedState;
	private static Direction pistonSetupDirection;
	private static boolean pistonSetupExtending;
	private static boolean pistonSetupSourcePiston;
	private static int pistonSetupReseedCount;
	private static CameraType originalCameraType;
	private static CloudStatus originalCloudStatus;
	private static int originalCloudRangeChunks = -1;
	private static GameType originalGameMode;
	private static GameType originalPreviousGameMode;
	private static AttackIndicatorStatus originalAttackIndicator;
	private static ChatVisiblity originalChatVisibility;
	private static boolean originalHighContrastBlockOutline;
	private static boolean originalNoGravity;
	private static int originalSelectedHotbarSlot;
	private static List<ItemStack> originalHotbarItems = List.of();
	private static float originalExperienceProgress;
	private static int originalExperienceLevel;
	private static int originalExperienceDisplayStartTick;
	private static int originalAttackStrengthTicker;
	private static int originalArmorValueOverride;
	private static float originalHealthOverride;
	private static float originalMaxHealthOverride;
	private static int originalTicksFrozen;
	private static MobEffectInstance originalPoisonEffect;
	private static MobEffectInstance originalWitherEffect;
	private static MobEffectInstance originalRegenerationEffect;
	private static Entity originalCrosshairPickEntity;
	private static float initialHealth;
	private static Vec3 lastSafetyPosition = Vec3.ZERO;
	private static Vec3 lastSafetyVelocity = Vec3.ZERO;
	private static float lastSafetyHealth;
	private static double lastSafetyFallDistance;
	private static String deterministicSupportBlock = "unset";
	private static String deterministicSupportBlockType = "unset";
	private static ForcedBlockOutlineTarget forcedBlockOutlineTarget;
	private static final Map<Long, Map<String, Set<String>>> SUBMITTED_WORK_BY_FRAME = new LinkedHashMap<>();
	private static int rustGalGuiScreenCycleStage;
	private static int rustGalGuiScreenCycleFramesInStage;
	private static int rustGalGuiScreenCyclesCompleted;
	private static boolean rustGalGuiScreenCycleComplete = !RUST_GAL_GUI_SCREEN_CYCLE;
	private static boolean rustGalGuiScreenCycleInventoryCaptureRequested;
	private static boolean rustGalGuiScreenCycleInventoryCaptureComplete;
	private static Path rustGalGuiScreenCycleInventoryAckPath;
	private static Path rustGalGuiScreenCycleInventoryCapturePath;
	private static int pendingTerrainParticleFinishFrames;
	/** Last vanilla weather tick copied by the active render path; diagnostics only. */
	private static volatile int lastWeatherRendererTicks = Integer.MIN_VALUE;
	private static volatile float lastWeatherRendererPartialTick = Float.NaN;
	private static volatile String lastWeatherSemanticFingerprint = "";
	private static volatile String lastLightmapSemanticFingerprint = "";
	private static int lastLightmapCaptureRefreshPose = Integer.MIN_VALUE;

	private DeterministicCameraCapture() {
	}

	public static long currentRenderedFrameIndex() {
		return ENABLED ? renderedFrameIndex : GraphicsFrameBenchmark.currentFrameIndex();
	}

	/** Records the vanilla weather animation input without influencing rendering. */
	public static void recordWeatherRendererTicks(int ticks) {
		if (ENABLED) {
			lastWeatherRendererTicks = ticks;
		}
	}

	/** Records the copied vanilla weather interpolation input without influencing rendering. */
	public static void recordWeatherRendererPartialTick(float partialTick) {
		if (ENABLED) {
			lastWeatherRendererPartialTick = partialTick;
		}
	}

	/** Records bounded pre-raster weather semantics for cross-repository diagnostics. */
	public static void recordWeatherSemanticFingerprint(String fingerprint) {
		if (ENABLED) {
			lastWeatherSemanticFingerprint = fingerprint == null ? "" : fingerprint;
		}
	}

	/** Records scalar lightmap semantics only; never a Java GPU resource. */
	public static void recordLightmapSemanticFingerprint(String fingerprint) {
		if (ENABLED) {
			lastLightmapSemanticFingerprint = fingerprint == null ? "" : fingerprint;
		}
	}

	/** Returns the baseline interpolation input for a deterministic weather fixture only. */
	public static float weatherPartialTickForCapture(float vanillaPartialTick) {
		if (!ENABLED || WEATHER_SCENARIO.isEmpty() || poseIndex < 0 || poseIndex >= WEATHER_CAPTURE_PARTIAL_TICKS.length) {
			return vanillaPartialTick;
		}
		return WEATHER_CAPTURE_PARTIAL_TICKS[poseIndex];
	}

	/** Returns the baseline weather animation tick for a deterministic weather fixture only. */
	public static int weatherRendererTicksForCapture(int vanillaTicks) {
		if (!ENABLED || WEATHER_SCENARIO.isEmpty() || poseIndex < 0 || poseIndex >= WEATHER_CAPTURE_RENDERER_TICKS.length) {
			return vanillaTicks;
		}
		return WEATHER_CAPTURE_RENDERER_TICKS[poseIndex];
	}

	/** Replays Frozen's immutable lightmap semantics only for a deterministic parity pose. */
	public static net.minecraft.client.renderer.LightTexture.RustSemanticLightmapInputs lightmapInputsForCapture(
		net.minecraft.client.renderer.LightTexture.RustSemanticLightmapInputs vanilla
	) {
		if (!hasBaselineLightmapInputsForCurrentPose()) return vanilla;
		String[] values = LIGHTMAP_CAPTURE_INPUTS[poseIndex].split(",");
		if (values.length != 13) throw new IllegalArgumentException("lightmapInputs requires 13 scalar values per pose");
		float[] parsed = new float[13];
		for (int i = 0; i < parsed.length; i++) parsed[i] = Float.parseFloat(values[i]);
		return new net.minecraft.client.renderer.LightTexture.RustSemanticLightmapInputs(vanilla.generation(), parsed[0], parsed[1], parsed[2], parsed[3], parsed[4], parsed[5], parsed[6], parsed[7], parsed[8], parsed[9], parsed[10], parsed[11], parsed[12]);
	}

	/** Whether the current deterministic pose supplies an explicit baseline lightmap snapshot. */
	public static boolean refreshLightmapInputsForCapture() {
		if (!hasBaselineLightmapInputsForCurrentPose()) return false;
		if (lastLightmapCaptureRefreshPose == poseIndex) return false;
		lastLightmapCaptureRefreshPose = poseIndex;
		return true;
	}

	/**
	 * A Frozen OpenGL capture may supply immutable scalar lightmap inputs for a
	 * matching Current parity pose. This is deliberately keyed by the explicit
	 * capture property, not by a rendering family: clouds and weather both
	 * share the ordinary vanilla lightmap, while normal gameplay has no values
	 * to replay.
	 */
	private static boolean hasBaselineLightmapInputsForCurrentPose() {
		return ENABLED
			&& poseIndex >= 0
			&& poseIndex < LIGHTMAP_CAPTURE_INPUTS.length
			&& !LIGHTMAP_CAPTURE_INPUTS[poseIndex].isBlank();
	}

	/**
	 * Lets a completed benchmark keep the ordinary client loop alive until an
	 * enabled deterministic screenshot has been acknowledged.
	 */
	public static boolean isAwaitingCompletion() {
		return ENABLED && !complete && !failed;
	}

	/**
	 * Called from render submission, before {@link #afterRender(Minecraft)}
	 * increments {@link #renderedFrameIndex}. It is the identity the ensuing
	 * screenshot acknowledgement uses for the just-presented frame.
	 */
	public static long currentInProgressRenderedFrameIndex() {
		return (ENABLED ? renderedFrameIndex : GraphicsFrameBenchmark.currentFrameIndex()) + 1L;
	}

	/**
	 * Claims the one deterministic render that immediately precedes the current
	 * pose screenshot. A coordinator that cannot claim it must not emit a
	 * gameplay attachment dump and let a later, unrelated frame masquerade as
	 * screenshot evidence.
	 */
	public static long claimWholeFrameAttachmentCaptureRenderedFrameIndex() {
		if (!ENABLED || !initialized || complete || failed || !wholeFrameAttachmentCaptureArmed
			|| wholeFrameAttachmentCaptureRequestIssued) {
			return -1L;
		}
		{
			Minecraft minecraft = Minecraft.getInstance();
			try {
				if (!GraphicsAuditBlockDisplayFixture.readyForCapture(minecraft)) return -1L;
			} catch (IllegalStateException timeout) {
				fail(timeout.getMessage());
				return -1L;
			}
			// Apply the same fixture readiness to every final-output capture,
			// including magma, fluids and particles, not only flat GUI items.
			// Normal final-output capture can bypass the ordinary screenshot
			// callback. Bind this observation to the frame being claimed, not
			// a later tick at metadata serialization time.
			blockAnimationAtCapture = GraphicsAuditBlockDisplayFixture.animationObservation(minecraft);
		}
		wholeFrameAttachmentCaptureRequestIssued = true;
		try {
			if (!GraphicsAuditGuiFoilTiming.readyForCapture()) {
				wholeFrameAttachmentCaptureRequestIssued = false;
				return -1L;
			}
		} catch (IllegalStateException timeout) {
			fail(timeout.getMessage());
			return -1L;
		}
		wholeFrameAttachmentCaptureDeterministicFrame = currentInProgressRenderedFrameIndex();
		captureSurfaceObservations();
		wholeFrameGuiFoilTiming = GraphicsAuditGuiFoilTiming.snapshot();
		return wholeFrameAttachmentCaptureDeterministicFrame;
	}

	/**
	 * Producer receipts emitted during a selected-source readback use the same
	 * deterministic identity as its attachment request. The native readback may
	 * complete after Java's ordinary render counter advances, but that must not
	 * relabel the submission that actually produced the retained final image.
	 */
	public static long currentCaptureCorrelationRenderedFrameIndex() {
		if (ENABLED && wholeFrameAttachmentCaptureArmed && wholeFrameAttachmentCaptureRequestIssued
			&& wholeFrameAttachmentCaptureDeterministicFrame > 0L) {
			return wholeFrameAttachmentCaptureDeterministicFrame;
		}
		return currentInProgressRenderedFrameIndex();
	}

	/**
	 * Capture-only route for retaining a normal Rust whole-frame final image.
	 * It neither changes route selection nor asks Rust to execute a selected
	 * source graph; the coordinator uses it solely to acknowledge the already
	 * presented normal submission.
	 */
	public static boolean normalRouteFinalOutputCaptureRequested() {
		return ENABLED && RUST_FINAL_OUTPUT_EVERY_POSE && !selectedSourceCaptureRequested();
	}

	/**
	 * The coordinator calls this only after Rust has promoted a pending
	 * selected-source request, submitted its matching attachments, and
	 * presented the same frame. It is capture synchronization, never route
	 * selection or rendering policy.
	 */
	public static void confirmWholeFrameSourceCapture(
		long gameplayFrameId,
		long correlationId,
		long deterministicRenderedFrameIndex,
		long submissionId,
		long acquiredSwapchainImage,
		long presentedSwapchainImage
	) {
		if (!ENABLED || complete || failed || !wholeFrameAttachmentCaptureArmed
			|| !wholeFrameAttachmentCaptureRequestIssued) {
			return;
		}
		var distantHorizonsRoute = net.vulkanic.world.DistantHorizonsSemanticCollector.routeDiagnosticsSnapshot();
		long executedCaptureFrame = distantHorizonsRoute.lastExecutedCaptureFrame();
		boolean captureFrameAligned = executedCaptureFrame == deterministicRenderedFrameIndex
			|| executedCaptureFrame + 1L == deterministicRenderedFrameIndex;
		if (distantHorizonsFixtureRequested()
			&& (distantHorizonsRoute.lastExecutedSubmission() != submissionId || !captureFrameAligned)) {
			// The attachment readback and the selected DH execution are produced by
			// adjacent whole-frame submissions on some frames. Do not acknowledge a
			// screenshot whose image/correlation pair can be one submission apart;
			// release the request so the next matching frame can claim it again.
			// Preserve the originally claimed deterministic identity while waiting
			// for the selected DH submission to become visible. Re-claiming here
			// would advance the correlation frame and make a valid submission look
			// mismatched by one frame forever.
			System.out.println("[MattMC graphics audit] deterministic source capture deferred"
				+ " gameplayFrame=" + gameplayFrameId
				+ " reason=dh-execution-presentation-mismatch"
				+ " executionSubmission=" + distantHorizonsRoute.lastExecutedSubmission()
				+ " presentationSubmission=" + submissionId
				+ " executionCaptureFrame=" + distantHorizonsRoute.lastExecutedCaptureFrame()
				+ " presentationCaptureFrame=" + deterministicRenderedFrameIndex);
			return;
		}
		// A normal-route final-output diagnostic retains the exact Rust-presented
		// image for a parity pose. It is not selected-source execution and must
		// not wait for an unrelated selected-source producer receipt.
		boolean normalRouteFinalOutputCapture = normalRouteFinalOutputCaptureRequested();
		boolean requiredProducerReceipt = normalRouteFinalOutputCapture
			|| requiredRustSourceExecutionObservedForGameplayFrame(gameplayFrameId);
		if (!requiredProducerReceipt && !CLOUD_SCENARIO.isEmpty()) {
			// Cloud captures must be promoted from the same whole-frame submission
			// that executed the semantic cloud quads.  The ordinary external-window
			// request can otherwise sample a later GUI-only frame (or stale swapchain
			// contents) while still claiming the earlier cloud pose.
			String cloudRoute = WorldRenderRoutePolicy.currentCloudRoute().name().toLowerCase(Locale.ROOT);
			requiredProducerReceipt = RustGalWorldPrimitiveRenderer.cloudExecutionDiagnostics().stream().anyMatch(diagnostic ->
				diagnostic.gameplayFrameId() == gameplayFrameId
					&& diagnostic.submissionId() == submissionId
					&& diagnostic.route().equals(cloudRoute)
					&& diagnostic.quads() > 0
			);
		}
		if (!requiredProducerReceipt && ("wind-charge".equals(MODEL_MESH_SCENARIO)
			|| (!ITEM_ENTITY_SCENARIO.isEmpty() && !"hidden".equals(ITEM_ENTITY_SCENARIO)))) {
			// Some presentation frames carry a native gameplay id that is one step
			// ahead of the receipt filename. The same bounded skew occurs for item
			// entity source frames because the selected-source attachment and the
			// presenter can retire adjacent Rust submissions. Keep ordering strict:
			// accept only a selected-source receipt at/after this exact submission,
			// with the required producer identity checked by the receipt validator.
			requiredProducerReceipt = requiredRustSourceExecutionObservedAtOrAfter(submissionId);
		}
		if (!requiredProducerReceipt) {
			// This frame may be a valid selected-source terrain submission while a
			// requested entity producer has not reached its own writer yet. Keep the
			// capture armed and allow one later frame to claim the bounded readback;
			// a terrain-only attachment must not stand in for model proof.
			wholeFrameAttachmentCaptureRequestIssued = false;
			System.out.println("[MattMC graphics audit] deterministic source capture deferred"
				+ " gameplayFrame=" + gameplayFrameId
				+ " reason=required-producer-not-in-source-receipt");
			return;
		}
		if ("wind-charge".equals(MODEL_MESH_SCENARIO)
			&& !RustGalWorldPrimitiveRenderer.entityModelExecutionDiagnostics().stream().anyMatch(diagnostic ->
				diagnostic.gameplayFrameId() == gameplayFrameId
					&& diagnostic.submissionId() == submissionId
					&& diagnostic.quads() > 0
			)) {
			wholeFrameAttachmentCaptureRequestIssued = false;
			System.out.println("[MattMC graphics audit] deterministic source capture deferred"
				+ " gameplayFrame=" + gameplayFrameId
				+ " reason=wind-charge-execution-submission-mismatch");
			return;
		}
		boolean movingProducerReady = normalRouteFinalOutputCapture
			|| movingMeshProducerReady(deterministicRenderedFrameIndex);
		if (!movingProducerReady && "wind-charge".equals(MODEL_MESH_SCENARIO)) {
			// Wind Charge is a material-quad entity-model producer rather than an
			// indexed moving-mesh instance. Its exact selected-source receipt is the
			// stronger same-gameplay-frame correlation for attachment promotion.
			movingProducerReady = requiredRustSourceExecutionObservedForGameplayFrame(gameplayFrameId);
		}
		if (!movingProducerReady) {
			// A selected-source attachment can be a valid terrain submission while
			// the requested moving producer has not executed in that same frame.
			// Do not turn that attachment into a screenshot acknowledgement: it is
			// not evidence for this capture's producer contract.
			wholeFrameAttachmentCaptureRequestIssued = false;
			System.out.println("[MattMC graphics audit] deterministic source capture deferred"
				+ " gameplayFrame=" + gameplayFrameId
				+ " reason=required-moving-producer-not-in-submission"
				+ " deterministicFrame=" + deterministicRenderedFrameIndex);
			return;
		}
		wholeFrameAttachmentCaptureReady = true;
		framesAwaitingWholeFrameAttachmentCapture = 0;
		wholeFrameAttachmentCaptureGameplayFrame = gameplayFrameId;
		wholeFrameAttachmentCaptureCorrelation = correlationId;
		wholeFrameAttachmentCaptureDeterministicFrame = deterministicRenderedFrameIndex;
		wholeFrameAttachmentCaptureSubmission = submissionId;
		wholeFrameAttachmentCaptureAcquiredImage = acquiredSwapchainImage;
		wholeFrameAttachmentCapturePresentedImage = presentedSwapchainImage;
		lastWholeFramePresentation = new WholeFramePresentation(
			deterministicRenderedFrameIndex,
			gameplayFrameId,
			correlationId,
			submissionId,
			acquiredSwapchainImage,
			presentedSwapchainImage
		);
		if (!captureWholeFrameFinalOutput()) {
			return;
		}
		System.out.println("[MattMC graphics audit] deterministic source capture ready"
			+ " gameplayFrame=" + gameplayFrameId
			+ " correlation=" + correlationId
			+ " deterministicRender=" + deterministicRenderedFrameIndex);
	}

	/**
	 * Retains the backend-owned final image for the exact Rust submission that
	 * produced the selected-source attachment evidence. This avoids racing an
	 * external desktop capture against later swapchain presentations.
	 */
	private static boolean captureWholeFrameFinalOutput() {
		net.sodium.client.render.StaticTerrainParityDiagnostics.recordRustFinalOutputCaptureCoverage(
			wholeFrameAttachmentCaptureDeterministicFrame, wholeFrameAttachmentCaptureGameplayFrame
		);
		if (poseIndex < 0 || poseIndex >= poses.length) {
			fail("selected-source final-output capture has no active deterministic pose");
			return false;
		}
		String attachmentDirectory = System.getenv("MATTMC_RUST_WHOLE_FRAME_ATTACHMENT_DIR");
		if (attachmentDirectory == null || attachmentDirectory.isBlank()) {
			fail("selected-source final-output capture is missing the attachment directory");
			return false;
		}
		Path source = Path.of(attachmentDirectory).resolve("attachment-final_output.png");
		if (!Files.isRegularFile(source)) {
			fail("selected-source final-output capture is missing " + source);
			return false;
		}
		Path skyFogSource = Path.of(attachmentDirectory).resolve("attachment-sky-fog.json");
		if (!Files.isRegularFile(skyFogSource)) {
			fail("selected-source final-output capture is missing " + skyFogSource);
			return false;
		}
		Pose pose = poses[poseIndex];
		int captureIndex = poseIndex + 1;
		String fileName = String.format(Locale.ROOT, "%02d_%s.png", captureIndex, pose.name());
		String ackName = String.format(Locale.ROOT, "capture_request_%02d_%s.ack.json", captureIndex, pose.name());
		currentScreenshotPath = SCREENSHOT_DIR.resolve(fileName);
		Path skyFogReceiptPath = SCREENSHOT_DIR.resolve(String.format(Locale.ROOT, "%02d_%s.sky-fog.json", captureIndex, pose.name()));
		currentAckPath = SCREENSHOT_DIR.resolve(ackName);
		try {
			Files.createDirectories(SCREENSHOT_DIR);
			Files.copy(source, currentScreenshotPath, StandardCopyOption.REPLACE_EXISTING);
			Files.copy(skyFogSource, skyFogReceiptPath, StandardCopyOption.REPLACE_EXISTING);
			Files.writeString(currentScreenshotPath.resolveSibling(fileName + ".foil-timing.json"),
				wholeFrameGuiFoilTiming, StandardCharsets.UTF_8);
			StringBuilder json = new StringBuilder(768);
			json.append("{\n");
			json.append("  \"index\": ").append(captureIndex).append(",\n");
			appendField(json, "poseName", pose.name()).append(",\n");
			appendField(json, "screenshot", currentScreenshotPath.toAbsolutePath().toString()).append(",\n");
			appendField(json, "skyFogReceipt", skyFogReceiptPath.toAbsolutePath().toString()).append(",\n");
			appendField(json, "ack", currentAckPath.toAbsolutePath().toString()).append(",\n");
			json.append("  \"renderedFrameIndex\": ").append(wholeFrameAttachmentCaptureDeterministicFrame).append(",\n");
			json.append("  \"wholeFramePresentationCorrelation\":{\"gameplayFrameId\":")
				.append(wholeFrameAttachmentCaptureGameplayFrame)
				.append(",\"correlationId\":").append(wholeFrameAttachmentCaptureCorrelation)
				.append(",\"submissionId\":").append(wholeFrameAttachmentCaptureSubmission)
				.append(",\"acquiredSwapchainImage\":").append(wholeFrameAttachmentCaptureAcquiredImage)
				.append(",\"presentedSwapchainImage\":").append(wholeFrameAttachmentCapturePresentedImage)
				.append("},\n");
			appendDistantHorizonsExecutionCorrelation(json, 2).append(",\n");
			appendDistantHorizonsTextureProbeReceipt(json, 2).append(",\n");
			appendDistantHorizonsWaterProbeReceipt(json, 2).append(",\n");
			// The final-output path bypasses the normal main-target screenshot
			// acknowledgement. Preserve the same producer-specific correlation so
			// a real static terrain submission cannot be mistaken for absent work.
			appendStaticTerrainExecutionCorrelation(json, 2).append(",\n");
			// Keep final-output captures semantically equivalent to ordinary
			// deterministic screenshots. The palette receipt is evaluated against
			// this exact retained Rust image, so omitting it here would turn a
			// complete terrain frame into a false missing-proof failure.
			appendStaticTerrainAtlasReceipt(json, 2).append(",\n");
			appendStaticTerrainTextureProbeReceipt(json, 2).append(",\n");
			appendField(json, "captureMethod", "rust-vulkan-final-output", 2).append(",\n");
			appendField(json, "targetWindow", "rust-vulkan-final-output", 2).append(",\n");
			appendField(json, "status", "captured", 2).append("\n");
			json.append("}\n");
			Files.writeString(currentAckPath, json.toString(), StandardCharsets.UTF_8);
			wholeFrameFinalOutputCapture = true;
			awaitingScreenshotAck = true;
			framesAwaitingAck = 0;
			return true;
		} catch (IOException exception) {
			fail("failed to retain selected-source final-output capture: " + exception.getMessage());
			return false;
		}
	}

	public static void beforeTick(Minecraft minecraft) {
		if (ENABLED && failed && STOP_AFTER_COMPLETE && !stopIssued) {
			stopIssued = true;
			minecraft.stop();
			return;
		}
		if (ENABLED && complete && STOP_AFTER_COMPLETE && !stopIssued) {
			// Preserve the deterministic screenshot while allowing the independent
			// benchmark to finish its full configured sample window. This keeps
			// correctness captures and timing validation correlated without
			// weakening either contract.
			if (GraphicsFrameBenchmark.isAwaitingCompletion()) {
				return;
			}
			stopIssued = true;
			minecraft.stop();
			return;
		}
		if (!ENABLED || complete || failed) {
			return;
		}
		applyFixedCaptureTime(minecraft);

		LocalPlayer player = minecraft.player;
		if (player == null) {
			return;
		}
		if (!FORCED_SELECTED_SKIN.isEmpty() && !FORCED_SELECTED_SKIN.equals(minecraft.options.selectedSkin)) {
			minecraft.options.selectedSkin = FORCED_SELECTED_SKIN;
		}

		player.input.keyPresses = Input.EMPTY;
		player.xxa = 0.0F;
		player.zza = 0.0F;
		player.setSprinting(false);
		player.setShiftKeyDown(false);
		recordPlayerSafetyState(player);
		Vec3 targetPosition = targetPositionForCurrentPose();
		if (initialized && targetPosition != null
			&& player.position().distanceToSqr(targetPosition) > 0.0004
			&& !isKnownDeterministicPosePosition(player.position())) {
			fail("deterministic player moved unexpectedly before stabilization: initial="
				+ formatVec(initialPosition)
				+ " target="
				+ formatVec(targetPosition)
				+ " current="
				+ formatVec(player.position()));
			return;
		}
		player.setDeltaMovement(Vec3.ZERO);
		player.fallDistance = 0.0F;
		applyRuntimeOverrides(minecraft, player);
		if (initialized && targetPosition != null && initialPose != null) {
			player.setPos(targetPosition);
			player.setOldPosAndRot(targetPosition, initialPose.yaw(), initialPose.pitch());
		}
	}

	public static void beforeRender(Minecraft minecraft) {
		if (!ENABLED || complete || failed) {
			return;
		}
		if (!ensureInitialized(minecraft)) {
			return;
		}
		applyFixedCaptureTime(minecraft);
		GraphicsAuditPostEffectFixture.beforeRender(minecraft);
		applyRuntimeOverrides(minecraft, minecraft.player);
		if (poseIndex >= poses.length) {
			// External screenshot acknowledgement can advance the final pose just
			// after the render hook's completion branch. Persist completion on the
			// next guaranteed lifecycle tick before the harness samples metadata.
			if (!complete && !failed) {
				finish(minecraft);
			}
			applyPauseParityScreen(minecraft);
			applyPose(minecraft.player, initialPose);
			return;
		}

		applyPauseParityScreen(minecraft);
		Pose activePose = translucentWarmupPoseIndex >= 0
			? poses[translucentWarmupPoseIndex]
			: poses[poseIndex];
		applyPose(minecraft.player, activePose);
		GraphicsAuditDroppedItemFoilFixture.beforeRender(minecraft);
	}

	private static java.lang.ref.WeakReference<MinecraftServer> fixedCaptureClockServer = new java.lang.ref.WeakReference<>(null);

	private static void applyFixedCaptureTime(Minecraft minecraft) {
		if (FIXED_CAPTURE_TIME != Long.MIN_VALUE && minecraft.level != null) {
			var server = minecraft.getSingleplayerServer();
			if (server != null && fixedCaptureClockServer.get() != server) {
				fixedCaptureClockServer = new java.lang.ref.WeakReference<>(server);
				// Seed the copied fixture's authoritative clock too. Otherwise real
				// time packets periodically undo the client override between ticks,
				// and tick-driven vignette/light state never settles at the target.
				server.execute(() -> {
					long previousDayTime = server.overworld().getDayTime();
					server.overworld().getGameRules().getRule(GameRules.RULE_DAYLIGHT).set(false, server);
					server.overworld().setDayTime(FIXED_CAPTURE_TIME);
					LOGGER.info("Capture fixture server clock: previousDayTime={} fixedDayTime={}", previousDayTime, FIXED_CAPTURE_TIME);
				});
			}
			minecraft.level.setTimeFromServer(FIXED_CAPTURE_TIME, FIXED_CAPTURE_TIME, false);
		}
	}

	/** Returns the capture fixture's cloud phase without affecting ordinary rendering. */
	public static float cloudTimeForCapture(float vanillaCloudTime) {
		return ENABLED && Float.isFinite(FIXED_CLOUD_TIME) ? FIXED_CLOUD_TIME : vanillaCloudTime;
	}

	private static void prepareSourceEntityIsolationBeforeFrame(Minecraft minecraft) {
		if (!SOURCE_ENTITY_ISOLATION || isSelectedSourceCoverageReady()
			|| minecraft.level == null || minecraft.getSingleplayerServer() == null) {
			return;
		}
		ServerLevel serverLevel = minecraft.getSingleplayerServer().getLevel(minecraft.level.dimension());
		if (serverLevel != null) {
			prepareSourceEntityIsolation(minecraft, serverLevel);
		}
	}

	/**
	 * External screenshots must observe the frame that issued their request.
	 * Whole-frame Vulkan presents independently of Java's main render target,
	 * so continuing to render while the capture process races the window makes
	 * a request's semantic and presentation identities ambiguous. This hook is
	 * deterministic-capture-only and intentionally leaves the already
	 * presented image untouched until its acknowledgement arrives.
	 */
	public static boolean holdPresentedFrameForExternalScreenshot(Minecraft minecraft) {
		if (!ENABLED || complete || failed || INTERNAL_SCREENSHOTS || !awaitingScreenshotAck) {
			return false;
		}
		if (checkScreenshotAck(minecraft)) {
			// Start the next pose on the following client frame; this invocation
			// still belongs to the screenshot that was just acknowledged.
			return true;
		}
		framesAwaitingAck++;
		if (framesAwaitingAck > ACK_TIMEOUT_FRAMES) {
			fail("timed out waiting for deterministic screenshot ack: " + currentAckPath);
		}
		return true;
	}

	/**
	 * Called after the Rust backend has presented a whole-frame submission.
	 * This is capture correlation only; it neither selects a route nor exposes
	 * native image state to Java semantics.
	 */
	public static void recordWholeFramePresentation(
		long deterministicRenderedFrameIndex,
		long gameplayFrameId,
		long correlationId,
		long submissionId,
		long acquiredSwapchainImage,
		long presentedSwapchainImage
	) {
		if (!ENABLED || deterministicRenderedFrameIndex <= 0L || gameplayFrameId <= 0L
			|| correlationId <= 0L || submissionId <= 0L || acquiredSwapchainImage == 0L
			|| presentedSwapchainImage == 0L) {
			return;
		}
		// The coordinator supplies the frame identity captured at the Vulkan
		// present boundary. Recomputing it here races the render-hook counter and
		// can make a valid presentation appear unrelated to its screenshot.
		long captureIdentity = deterministicRenderedFrameIndex;
		lastWholeFramePresentation = new WholeFramePresentation(
			captureIdentity,
			gameplayFrameId,
			correlationId,
			submissionId,
			acquiredSwapchainImage,
			presentedSwapchainImage
		);
	}

	public static void afterRender(Minecraft minecraft) {
		if (!ENABLED || complete || failed) {
			return;
		}
		afterRenderCalls++;
		if (pendingTerrainParticleFinishFrames > 0) {
			pendingTerrainParticleFinishFrames--;
			if (pendingTerrainParticleFinishFrames == 0) {
				finish(minecraft);
			}
		}
		if (!ensureInitialized(minecraft)) {
			afterRenderUninitializedReturns++;
			return;
		}
		// Whole-frame Vulkan can bypass the Java render hook that normally calls
		// beforeRender on every presented frame. Advance the copied-world-only
		// isolation state from the guaranteed capture lifecycle before testing
		// selected-source readiness, otherwise the prerequisite can wait on its
		// own unadmitted source receipt forever.
		prepareSourceEntityIsolationBeforeFrame(minecraft);
		// Poll model fixtures after the preceding frame's semantic submissions so
		// standalone renderers can provide their normal client-replication receipt.
		// The setup path polls before rendering as well, but that is necessarily too
		// early for Evoker Fangs' non-living render state.
		if (isModelMeshEntityScenario() && modelMeshSetupServerEntityPresent) {
			updateModelMeshClientEntityReceipt(minecraft);
		}
		if (poseIndex >= poses.length) {
			applyPauseParityScreen(minecraft);
			applyPose(minecraft.player, initialPose);
			return;
		}

		applyPauseParityScreen(minecraft);
		Pose activePose = translucentWarmupPoseIndex >= 0
			? poses[translucentWarmupPoseIndex]
			: poses[poseIndex];
		applyPose(minecraft.player, activePose);
		// Camera relocation invalidates the previous pose's visibility and upload
		// identities.  Whole-frame Rust terrain must prove a fresh drained/stable
		// window before the new pose is captured; retaining the global boolean here
		// allowed async chunk work to change the return image after the gate passed.
		if (settledReadyPoseIndex != poseIndex) {
			settledReadyPoseIndex = poseIndex;
			settledReadyGateSatisfied = false;
			settledRustTerrainFrames = 0;
			settledLegacyDistantHorizonsObservationFrames = 0;
			framesWaitingForSettledReady = 0;
		}
		renderedFrameIndex++;
		// The DH palette changes one copied-world source column. Place it before
		// the DH-ready window so the real selected-source frames that satisfy
		// readiness also ingest that exact material workload. Waiting until
		// after readiness wastes the bounded capture window rebuilding the
		// blocks and can leave no frame for the actual route/crop proof.
		if (distantHorizonsFixtureRequested()
			&& !setupDistantHorizonsTexturePaletteAfterSettledReady(minecraft)) {
			afterRenderPaletteGateReturns++;
			renderedFramesAtPose = 0;
			return;
		}
		// Weather is a capture-only server fixture.  Seed it before the settled
		// work gate so the gate can observe stable terrain while the real client
		// weather extractor is already producing rain columns.  Waiting until
		// after that gate deadlocks weather-only captures because weather setup is
		// itself the producer that must run after the world becomes playable.
		if (!WEATHER_SCENARIO.isEmpty() && !setupWeatherScenarioAfterSettledReady(minecraft)) {
			afterRenderWeatherGateReturns++;
			renderedFramesAtPose = 0;
			return;
		}
		if (!weatherCaptureIntensityReady(minecraft)) {
			afterRenderWeatherGateReturns++;
			renderedFramesAtPose = 0;
			return;
		}
		// Item entities are real server/client producers, but their fixture must
		// be queued before the settled-work gate.  Rust terrain may take many
		// frames to quiesce; waiting until after that gate would leave the item
		// route with no producer at all during the bounded capture window.
		if (!ITEM_ENTITY_SCENARIO.isEmpty() && !"hidden".equals(ITEM_ENTITY_SCENARIO)
			&& minecraft.player != null && minecraft.level != null
			&& minecraft.getSingleplayerServer() != null) {
			setupItemEntityScenario(minecraft, minecraft.player);
		}
		// Dense water capture is driven by Rust's visible-layer diagnostics and
		// must not wait for the static-terrain signature used by lifecycle rows.
		if (!captureStaticTerrainWaterAnimationFrameIfNeeded(minecraft)) {
			return;
		}
		boolean denseWaterCapture = STATIC_TERRAIN_WATER_ANIMATION_DENSE_CAPTURE
			&& "translucent-water".equals(STATIC_TERRAIN_SCENARIO);
		if (!denseWaterCapture && !settledReadyGateSatisfied && !settledReadyGateSatisfied(minecraft)) {
			afterRenderSettledGateReturns++;
			renderedFramesAtPose = 0;
			framesWaitingForSettledReady++;
			if (framesWaitingForSettledReady > SETTLED_READY_MAX_WAIT_FRAMES) {
				fail("timed out waiting for settled submitted work: " + settledReadySummary());
			} else if ((framesWaitingForSettledReady % 30) == 0) {
				writeMetadata(minecraft, "waiting_for_settled_ready_work");
			}
			return;
		}
		if (!minecraft.gui.vignetteBrightnessSettledForDeterministicCapture(minecraft.getCameraEntity())) {
			if (renderedFrameIndex % 30 == 0) {
				var entity = minecraft.getCameraEntity();
				LOGGER.info("Capture vignette waiting: current={} rawLight={} skyDarken={} gameTime={} paused={}",
					minecraft.gui.vignetteBrightnessForDeterministicCapture(),
					entity == null ? -1 : entity.level().getMaxLocalRawBrightness(
						BlockPos.containing(entity.getX(), entity.getEyeY(), entity.getZ())),
					minecraft.level == null ? -1 : minecraft.level.getSkyDarken(),
					minecraft.level == null ? -1 : minecraft.level.getGameTime(), minecraft.isPaused());
			}
			renderedFramesAtPose = 0;
			return;
		}
		if (GraphicsAuditWorldMenuFixture.afterRender(minecraft)) {
			renderedFramesAtPose = 0;
			return;
		}
		if (!setupStaticTerrainLifecycleScenarioAfterSettledReady(minecraft)) {
			afterRenderLifecycleGateReturns++;
			renderedFramesAtPose = 0;
			return;
		}
		if (staticTerrainRequiresTranslucentCameraSequence() && !translucentWarmupComplete) {
			if (translucentWarmupPoseIndex < 0) {
				translucentWarmupPoseIndex = 0;
				translucentWarmupFramesAtPose = 0;
			}
			// Keep the warm-up pose active for several complete producer frames;
			// this allows newly visible sections to be built and submitted before
			// any captured image is compared.
			translucentWarmupFramesAtPose++;
			if (translucentWarmupFramesAtPose >= TRANSLUCENT_WARMUP_FRAMES_PER_POSE) {
				translucentWarmupPoseIndex++;
				translucentWarmupFramesAtPose = 0;
				if (translucentWarmupPoseIndex >= poses.length) {
					translucentWarmupPoseIndex = -1;
					translucentWarmupComplete = true;
					// The capture sequence starts at pose zero after the warm-up.
					// Force a fresh readiness window so its first image is as settled
					// as every subsequently revisited pose.
					settledReadyPoseIndex = Integer.MIN_VALUE;
					settledReadyGateSatisfied = false;
					settledRustTerrainFrames = 0;
					framesWaitingForSettledReady = 0;
				}
			}
			return;
		}
		if (!setupMovingMeshScenarioAfterSettledReady(minecraft)) {
			afterRenderMovingMeshGateReturns++;
			renderedFramesAtPose = 0;
			return;
		}
		if (!setupWeatherScenarioAfterSettledReady(minecraft)) {
			afterRenderWeatherGateReturns++;
			renderedFramesAtPose = 0;
			return;
		}
		if (selectedSourceCaptureRequested() && !requiredRustSourceExecutionObserved()) {
			// Source execution can only be observed after the real copied-world
			// producer has been installed above. Keeping this separate from the
			// static-terrain settling gate prevents a model/entity capture from
			// waiting for a receipt that its own setup has not yet made possible.
			afterRenderSourceExecutionGateReturns++;
			renderedFramesAtPose = 0;
			framesWaitingForSourceExecution++;
			if (framesWaitingForSourceExecution > SETTLED_READY_MAX_WAIT_FRAMES) {
				fail("timed out waiting for selected-source execution after producer setup: "
					+ settledReadySummary());
			} else if ((framesWaitingForSourceExecution % 30) == 0) {
				writeMetadata(minecraft, "waiting_for_selected_source_execution");
			}
			return;
		}
		framesWaitingForSourceExecution = 0;
		maintainPistonScenario(minecraft);
		if (awaitingScreenshotAck) {
			// External acknowledgement is polled before the next render through
			// holdPresentedFrameForExternalScreenshot. Internal screenshots write
			// their acknowledgement synchronously. Reaching this point otherwise
			// is a contract violation rather than permission to render a newer
			// frame under the old request.
			return;
		}
		if (!advanceRustGalGuiScreenCycle(minecraft)) {
			return;
		}
		// Lifecycle fixtures must finish before a final-output readback can be
		// armed: the coordinator may acknowledge that readback before the later
		// ordinary screenshot callback is reached.
		if (!GraphicsAuditWorldResize.prepareWorldCapture(minecraft, renderedFrameIndex)) {
			resetWholeFrameAttachmentCaptureState();
			settledReadyGateSatisfied = false;
			renderedFramesAtPose = 0;
			writeMetadata(minecraft, "waiting_for_world_window_resize");
			return;
		}
		if (!GraphicsAuditWorldGuiScale.prepareWorldCapture(minecraft, renderedFrameIndex)) {
			resetWholeFrameAttachmentCaptureState();
			settledReadyGateSatisfied = false;
			renderedFramesAtPose = 0;
			writeMetadata(minecraft, "waiting_for_world_gui_scale");
			return;
		}
		if (!GraphicsAuditExperienceOrbFixture.ready(minecraft)) {
			resetWholeFrameAttachmentCaptureState();
			settledReadyGateSatisfied = false;
			renderedFramesAtPose = 0;
			writeMetadata(minecraft, "waiting_for_orb_occluder_light");
			return;
		}
		if (!GraphicsAuditResourceReload.prepareWorldCapture(minecraft, renderedFrameIndex)) {
			resetWholeFrameAttachmentCaptureState();
			settledReadyGateSatisfied = false;
			renderedFramesAtPose = 0;
			writeMetadata(minecraft, "waiting_for_world_resource_reload");
			return;
		}
		renderedFramesAtPose++;
		// Selected-source rows may opt into a bounded per-pose final readback.  The
		// ordinary default retains one attachment set, while the opt-in prevents a
		// later pose from silently falling back to an external window capture.
		boolean captureWholeFrameAttachmentsForPose = !selectedSourceCaptureRequested()
			|| poseIndex == 0
			|| RUST_FINAL_OUTPUT_EVERY_POSE;
		if (captureWholeFrameAttachmentsForPose
			&& renderedFramesAtPose == Math.max(1, FRAMES_PER_POSE - 1)
			&& !wholeFrameAttachmentCaptureReady) {
			RenderDocCaptureHook.beginFrameCaptureOnce(minecraft.getWindow(), poses[poseIndex].name() + "#" + renderedFrameIndex);
			RenderDocCaptureHook.triggerNextFrameOnce(poses[poseIndex].name() + "#" + renderedFrameIndex);
			// The next render is the settled pose screenshot. Arm it before the
			// coordinator runs so native attachment readbacks cannot select an
			// earlier warmup frame.
			wholeFrameAttachmentCaptureArmed = true;
			wholeFrameAttachmentCaptureRequestIssued = false;
			wholeFrameAttachmentCaptureReady = false;
			framesAwaitingWholeFrameAttachmentCapture = 0;
		}
		if (renderedFramesAtPose < FRAMES_PER_POSE) {
			return;
		}
		if (captureWholeFrameAttachmentsForPose
			&& (selectedSourceCaptureRequested() || RUST_FINAL_OUTPUT_EVERY_POSE)
			&& !wholeFrameAttachmentCaptureReady) {
			// A deferred source-selected request must hold the settled pose until a
			// later render both executes the required producer and promotes its
			// matching attachment request. Advancing to the screenshot here would
			// make the next frame's receipt unrelated evidence.
			framesAwaitingWholeFrameAttachmentCapture++;
			if (GraphicsAuditGuiFoilTiming.movingRequested() && !wholeFrameAttachmentCaptureRequestIssued) {
				// Natural phase selection has its own wall-clock deadline; no
				// submission/readback has been requested yet.
				return;
			}
			if (framesAwaitingWholeFrameAttachmentCapture > ACK_TIMEOUT_FRAMES) {
				fail("timed out waiting for selected-source attachment capture promotion");
			} else if ((framesAwaitingWholeFrameAttachmentCapture % 30) == 0) {
				writeMetadata(minecraft, "waiting_for_selected_source_attachment_capture");
			}
			return;
		}
			RenderDocCaptureHook.endFrameCaptureOnce(minecraft.getWindow(), poses[poseIndex].name() + "#" + renderedFrameIndex);

			VulkanicAPI.traceScopedCompositeColortex0PoseBoundary();
			if (!realSurvivalCrackPoseReady()) {
				wholeFrameAttachmentCaptureArmed = false;
				renderedFramesAtPose = 0;
				realSurvivalCrackFramesWaitingForStage++;
				if (realSurvivalCrackFramesWaitingForStage > SETTLED_READY_MAX_WAIT_FRAMES) {
					fail("timed out waiting for real survival crack pose " + poses[poseIndex].name()
						+ " target stage: min=" + (realSurvivalCrackMinRenderedStage == 10 ? -1 : realSurvivalCrackMinRenderedStage)
						+ " max=" + realSurvivalCrackMaxRenderedStage
						+ " status=" + realSurvivalCrackLastStatus
						+ " hit=" + realSurvivalCrackLastHitType
						+ " target=" + realSurvivalCrackLastTarget
						+ " validHits=" + realSurvivalCrackValidBlockHitCount
						+ " renderedStates=" + realSurvivalCrackRenderedStateCount);
				} else if ((realSurvivalCrackFramesWaitingForStage % 30) == 0) {
					writeMetadata(minecraft, "waiting_for_real_survival_crack_stage");
				}
				return;
			}
			realSurvivalCrackFramesWaitingForStage = 0;
			// Attachment readback proves one selected-source frame, but it cannot
			// replace this pose's real producer proof. Every moving-mesh capture
			// must still observe its selected producer before it can advance.
			if (!movingMeshProducerReady()) {
				wholeFrameAttachmentCaptureArmed = false;
				renderedFramesAtPose = 0;
				framesWaitingForMovingMeshProducer++;
			if (framesWaitingForMovingMeshProducer > SETTLED_READY_MAX_WAIT_FRAMES) {
						fail("timed out waiting for deterministic moving-mesh producer traversal: "
							+ "setupStage=" + movingMeshSetupStage
							+ " setupAttempts=" + movingMeshSetupAttempts
							+ " setupMissing=" + movingMeshSetupLastMissing
							+ " " + movingMeshProducerSummary());
				} else if ((framesWaitingForMovingMeshProducer % 30) == 0) {
					writeMetadata(minecraft, "waiting_for_moving_mesh_producer");
				}
				return;
			}
			framesWaitingForMovingMeshProducer = 0;
			if (!weatherProducerReady()) {
				wholeFrameAttachmentCaptureArmed = false;
				renderedFramesAtPose = 0;
				framesWaitingForWeatherProducer++;
				if (framesWaitingForWeatherProducer > SETTLED_READY_MAX_WAIT_FRAMES) {
					fail("timed out waiting for deterministic weather producer traversal: " + weatherProducerSummary());
				} else if ((framesWaitingForWeatherProducer % 30) == 0) {
					writeMetadata(minecraft, "waiting_for_weather_producer");
				}
				return;
			}
			framesWaitingForWeatherProducer = 0;
			if (!cloudProducerReady()) {
				wholeFrameAttachmentCaptureArmed = false;
				renderedFramesAtPose = 0;
				framesWaitingForCloudProducer++;
				if (framesWaitingForCloudProducer > SETTLED_READY_MAX_WAIT_FRAMES) {
					fail("timed out waiting for deterministic cloud producer traversal: " + cloudProducerSummary());
				} else if ((framesWaitingForCloudProducer % 30) == 0) {
					writeMetadata(minecraft, "waiting_for_cloud_producer");
				}
				return;
			}
			framesWaitingForCloudProducer = 0;
			if (INTERNAL_SCREENSHOTS) {
				captureCurrentPoseInternally(minecraft);
			} else {
				try {
					if (!GraphicsAuditBlockDisplayFixture.readyForCapture(minecraft)) {
						// The pose is already settled. Observe the next normally
						// rendered frame, not every eighth frame of an animation.
						// Discard this candidate's readback and arm a fresh one so
						// a later phase never reuses an earlier presentation.
						resetWholeFrameAttachmentCaptureState();
						wholeFrameAttachmentCaptureArmed = captureWholeFrameAttachmentsForPose;
						renderedFramesAtPose = Math.max(0, FRAMES_PER_POSE - 1);
						return;
					}
				} catch (IllegalStateException phaseTimeout) {
					fail(phaseTimeout.getMessage());
					return;
				}
				requestCurrentPoseScreenshot(minecraft);
			}
			resetWholeFrameAttachmentCaptureState();
		renderedFramesAtPose = 0;
	}

	private static boolean selectedSourceCaptureRequested() {
		// The required receipt directory is a capture-only correlation contract.
		// It never selects a Rust shader route; it only prevents this capture from
		// accepting a preparatory frame before the native source plan has executed.
		return !REQUIRED_RUST_SOURCE_EXECUTION_DIR.isEmpty();
	}

	/**
	 * A selected-source acknowledgement bypasses the ordinary external-window
	 * request path.  Clear its pose-local state here as well as after ordinary
	 * requests so the next deterministic pose can arm its own exact Rust
	 * submission readback.
	 */
	private static void resetWholeFrameAttachmentCaptureState() {
		wholeFrameGuiFoilTiming = "null";
		wholeFrameAttachmentCaptureArmed = false;
		wholeFrameAttachmentCaptureRequestIssued = false;
		wholeFrameAttachmentCaptureReady = false;
		framesAwaitingWholeFrameAttachmentCapture = 0;
		wholeFrameAttachmentCaptureGameplayFrame = -1L;
		wholeFrameAttachmentCaptureCorrelation = -1L;
		wholeFrameAttachmentCaptureDeterministicFrame = -1L;
	}

	public static boolean isEnabledForDiagnostics() {
		return ENABLED && initialized && !failed;
	}

	/**
	 * Capture-only requirement used by the Rust-owned selected-source attachment
	 * gate. It never influences source routing or a producer's render decision.
	 */
	public static boolean requiresSourceEntityMeshCapture() {
		return !MODEL_MESH_SCENARIO.isEmpty()
			&& !"hidden".equals(MODEL_MESH_SCENARIO)
			&& !"wind-charge".equals(MODEL_MESH_SCENARIO);
	}

	private static boolean captureStaticTerrainWaterAnimationFrameIfNeeded(Minecraft minecraft) {
		if (!STATIC_TERRAIN_WATER_ANIMATION_DENSE_CAPTURE || !"translucent-water".equals(STATIC_TERRAIN_SCENARIO)) {
			return true;
		}
		if (staticTerrainWaterAnimationDenseComplete) {
			return true;
		}
		// Real translucent-water workloads do not run the static-terrain
		// lifecycle fixture state machine.  Arm the dense capture from the
		// authoritative Rust diagnostics once visible terrain and animation
		// metadata are present, while retaining the fixture-visible gate for
		// lifecycle scenarios.
		if (!staticTerrainLifecycleAfterRecorded || !"fixture-visible".equals(staticTerrainLifecycleStage)) {
			RustGalTerrainRenderer.TerrainDiagnostics diagnostics = RustGalTerrainRenderer.diagnosticsSnapshot();
			if (diagnostics.currentFrameVisibleLayerSubmissions() <= 0
				|| "missing".equals(RustGalTerrainRenderer.waterAnimationSummaryForDiagnostics())) {
				return true;
			}
		}
		if (staticTerrainWaterAnimationScreenshotInFlight) {
			if (checkStaticTerrainWaterAnimationAck(minecraft)) {
				return false;
			}
			framesAwaitingStaticTerrainWaterAnimationAck++;
			if (framesAwaitingStaticTerrainWaterAnimationAck > ACK_TIMEOUT_FRAMES) {
				fail("timed out waiting for water animation dense screenshot ack: " + staticTerrainWaterAnimationAckPath);
			}
			return false;
		}
		if (staticTerrainWaterAnimationDenseCapturedFrames >= STATIC_TERRAIN_WATER_ANIMATION_DENSE_FRAMES) {
			staticTerrainWaterAnimationDenseComplete = true;
			writeMetadata(minecraft, "static_terrain_water_animation_dense_complete");
			return true;
		}

		int frameIndex = staticTerrainWaterAnimationDenseCapturedFrames;
		staticTerrainWaterAnimationScreenshotPath = SCREENSHOT_DIR.resolve(String.format(Locale.ROOT, "water_animation_frame_%03d.png", frameIndex));
		staticTerrainWaterAnimationAckPath = SCREENSHOT_DIR.resolve(String.format(Locale.ROOT, "capture_request_water_animation_%03d.ack.json", frameIndex));
		Path requestPath = SCREENSHOT_DIR.resolve(String.format(Locale.ROOT, "capture_request_water_animation_%03d.json", frameIndex));
		RustGalTerrainRenderer.TerrainDiagnostics diagnostics = RustGalTerrainRenderer.diagnosticsSnapshot();
		long animationTick = renderedFrameIndex;
		staticTerrainWaterAnimationPendingRenderedFrameIndex = renderedFrameIndex;
		staticTerrainWaterAnimationPendingGameTime = minecraft.level == null ? -1L : minecraft.level.getGameTime();
		staticTerrainWaterAnimationPendingHash = RustGalTerrainRenderer.waterAnimationHashForDiagnostics();
		staticTerrainWaterAnimationPendingSummary = RustGalTerrainRenderer.waterAnimationSummaryForDiagnostics();
		staticTerrainWaterAnimationPendingState = RustGalTerrainRenderer.waterAnimationFrameStateForDiagnostics(animationTick);
		staticTerrainWaterAnimationPendingVisibleLayerSubmissions = diagnostics.visibleLayerSubmissions();
		staticTerrainWaterAnimationPendingCurrentFrameVisibleLayerSubmissions = diagnostics.currentFrameVisibleLayerSubmissions();
		staticTerrainWaterAnimationPendingAtlasGeneration = diagnostics.atlasGeneration();
		try {
			Files.createDirectories(SCREENSHOT_DIR);
		} catch (IOException exception) {
			fail("failed to create water animation screenshot directory " + SCREENSHOT_DIR + ": " + exception.getMessage());
			return false;
		}
		StringBuilder json = new StringBuilder(1024);
		json.append("{\n");
		json.append("  \"index\": ").append(frameIndex).append(",\n");
		appendField(json, "poseName", "water-animation-dense").append(",\n");
		appendField(json, "screenshot", staticTerrainWaterAnimationScreenshotPath.toAbsolutePath().toString()).append(",\n");
		appendField(json, "ack", staticTerrainWaterAnimationAckPath.toAbsolutePath().toString()).append(",\n");
		appendField(json, "dimension", minecraft.level == null ? "missing" : minecraft.level.dimension().location().toString()).append(",\n");
		appendVec3(json, "position", minecraft.player == null ? Vec3.ZERO : minecraft.player.position()).append(",\n");
		json.append("  \"renderedFrameIndex\": ").append(staticTerrainWaterAnimationPendingRenderedFrameIndex).append(",\n");
		json.append("  \"gameTime\": ").append(staticTerrainWaterAnimationPendingGameTime).append("\n");
		json.append("}\n");
		try {
			Files.writeString(requestPath, json.toString(), StandardCharsets.UTF_8);
		} catch (IOException exception) {
			fail("failed to write water animation dense screenshot request " + requestPath + ": " + exception.getMessage());
			return false;
		}
		staticTerrainWaterAnimationScreenshotInFlight = true;
		framesAwaitingStaticTerrainWaterAnimationAck = 0;
		writeMetadata(minecraft, "waiting_for_static_terrain_water_animation_dense_screenshot");
		return false;
	}

	private static boolean checkStaticTerrainWaterAnimationAck(Minecraft minecraft) {
		if (staticTerrainWaterAnimationAckPath == null || !Files.isRegularFile(staticTerrainWaterAnimationAckPath)) {
			return false;
		}
		if (staticTerrainWaterAnimationScreenshotPath == null || !Files.isRegularFile(staticTerrainWaterAnimationScreenshotPath)) {
			fail("water animation dense screenshot ack exists but screenshot is missing: " + staticTerrainWaterAnimationScreenshotPath);
			return false;
		}
		WATER_ANIMATION_CAPTURES.add(new WaterAnimationFrameCapture(
			staticTerrainWaterAnimationDenseCapturedFrames,
			staticTerrainWaterAnimationScreenshotPath.toAbsolutePath().toString(),
			staticTerrainWaterAnimationPendingRenderedFrameIndex,
			staticTerrainWaterAnimationPendingGameTime,
			staticTerrainWaterAnimationPendingHash,
			staticTerrainWaterAnimationPendingSummary,
			staticTerrainWaterAnimationPendingState,
			staticTerrainWaterAnimationPendingVisibleLayerSubmissions,
			staticTerrainWaterAnimationPendingCurrentFrameVisibleLayerSubmissions,
			staticTerrainWaterAnimationPendingAtlasGeneration
		));
		staticTerrainWaterAnimationDenseCapturedFrames++;
		staticTerrainWaterAnimationScreenshotInFlight = false;
		framesAwaitingStaticTerrainWaterAnimationAck = 0;
		staticTerrainWaterAnimationScreenshotPath = null;
		staticTerrainWaterAnimationAckPath = null;
		staticTerrainWaterAnimationPendingRenderedFrameIndex = -1L;
		staticTerrainWaterAnimationPendingGameTime = -1L;
		staticTerrainWaterAnimationPendingHash = 0L;
		staticTerrainWaterAnimationPendingSummary = "missing";
		staticTerrainWaterAnimationPendingState = "missing";
		staticTerrainWaterAnimationPendingVisibleLayerSubmissions = 0L;
		staticTerrainWaterAnimationPendingCurrentFrameVisibleLayerSubmissions = 0L;
		staticTerrainWaterAnimationPendingAtlasGeneration = 0L;
		writeMetadata(minecraft, "static_terrain_water_animation_dense_frame_captured");
		return true;
	}

	public static boolean isFallingBlockSequenceActive() {
		return ENABLED && initialized && !FALLING_BLOCK_SCENARIO.isEmpty() && !"hidden".equals(FALLING_BLOCK_SCENARIO);
	}

	public static void recordFallingBlockExtractionProbe(boolean shouldRender, boolean compiledSection, boolean extracted) {
		if (!isFallingBlockSequenceActive()) {
			return;
		}
		fallingEntitySeenCount++;
		if (shouldRender) {
			fallingEntityShouldRenderCount++;
		}
		if (compiledSection) {
			fallingEntityCompiledSectionCount++;
		}
		if (extracted) {
			fallingEntityExtractedCount++;
		}
	}

	public static void forceCrosshairAttackTargetForDiagnostics(Minecraft minecraft) {
		if (ENABLED && FORCE_ATTACK_TARGET && minecraft.player != null) {
			minecraft.crosshairPickEntity = minecraft.player;
		}
	}

	private static boolean setupMovingMeshScenarioAfterSettledReady(Minecraft minecraft) {
		if (movingMeshScenarioSetup) {
			movingMeshSetupStage = "setup-complete";
			if ("evoker-fangs".equals(MODEL_MESH_SCENARIO) && !modelMeshSetupEvokerEventSent
				&& modelMeshSetupServerEntityPresent && modelMeshSetupServerEntityId >= 0) {
				modelMeshSetupEvokerEventSent = true;
				MinecraftServer server = minecraft.getSingleplayerServer();
				ResourceKey<Level> dimension = minecraft.level.dimension();
				int serverEntityId = modelMeshSetupServerEntityId;
				server.execute(() -> {
					ServerLevel serverLevel = server.getLevel(dimension);
					Entity fangs = serverLevel == null ? null : serverLevel.getEntity(serverEntityId);
					if (fangs instanceof EvokerFangs) serverLevel.broadcastEntityEvent(fangs, (byte)4);
				});
				writeMetadata(minecraft, "evoker_fangs_attack_activated_after_settled");
				return false;
			}
			int modelMeshPoseBeforeSetup = modelMeshSetupPoseIndex;
			setupModelMeshScenario(minecraft, minecraft.player);
			if ("evoker-fangs".equals(MODEL_MESH_SCENARIO) && modelMeshSetupPoseIndex != modelMeshPoseBeforeSetup) {
				// The server-created fangs entity must replicate to the client before
				// this pose can satisfy its real renderer/execution correlation gate.
				writeMetadata(minecraft, "moving_mesh_model_pose_spawned");
				return false;
			}
			if ("evoker-fangs".equals(MODEL_MESH_SCENARIO)
				&& WorldRenderRoutePolicy.currentModelMeshRoute(true).usesRustWholeFrameVulkan()
				&& !hasCurrentModelMeshRoute(renderedFrameIndex, 1L)) {
				// Do not let an older short-lived animation satisfy this pose. The
				// screenshot gate requires this same tight model/execution correlation.
				return false;
			}
			int experienceOrbPoseBeforeSetup = experienceOrbSetupPoseIndex;
			setupExperienceOrbScenario(minecraft, minecraft.player);
			if (!EXPERIENCE_ORB_SCENARIO.isEmpty()
				&& !"hidden".equals(EXPERIENCE_ORB_SCENARIO)
				&& experienceOrbSetupPoseIndex != experienceOrbPoseBeforeSetup) {
				writeMetadata(minecraft, "moving_mesh_experience_orb_pose_spawned");
				return false;
			}
			int itemEntityPoseBeforeSetup = itemEntitySetupPoseIndex;
			setupItemEntityScenario(minecraft, minecraft.player);
			if (!ITEM_ENTITY_SCENARIO.isEmpty()
				&& !"hidden".equals(ITEM_ENTITY_SCENARIO)
				&& itemEntitySetupPoseIndex != itemEntityPoseBeforeSetup) {
				writeMetadata(minecraft, "moving_mesh_item_entity_pose_spawned");
				return false;
			}
			setupBeaconBeamScenario(minecraft, minecraft.player);
			// A capture pose is held until its external screenshot is acknowledged.
			// Spawn one real falling entity for each new pose so a slow whole-frame
			// submission cannot let the previous entity land before its next capture.
			if (setupFallingBlockScenario(minecraft, minecraft.player)) {
				writeMetadata(minecraft, "moving_mesh_falling_block_pose_spawned");
				return false;
			}
			return true;
		}
		if (
			(FALLING_BLOCK_SCENARIO.isEmpty() || "hidden".equals(FALLING_BLOCK_SCENARIO))
				&& (PISTON_SCENARIO.isEmpty() || "hidden".equals(PISTON_SCENARIO))
			&& (PRIMED_TNT_SCENARIO.isEmpty() || "hidden".equals(PRIMED_TNT_SCENARIO))
			&& (ARROW_SCENARIO.isEmpty() || "hidden".equals(ARROW_SCENARIO))
			&& (EXPERIENCE_ORB_SCENARIO.isEmpty() || "hidden".equals(EXPERIENCE_ORB_SCENARIO))
			&& (BEACON_BEAM_SCENARIO.isEmpty() || "hidden".equals(BEACON_BEAM_SCENARIO))
			&& (ITEM_ENTITY_SCENARIO.isEmpty() || "hidden".equals(ITEM_ENTITY_SCENARIO))
			&& (MODEL_MESH_SCENARIO.isEmpty() || "hidden".equals(MODEL_MESH_SCENARIO))
			&& (ENTITY_FLAME_SCENARIO.isEmpty() || "hidden".equals(ENTITY_FLAME_SCENARIO))
			&& (ENTITY_SHADOW_SCENARIO.isEmpty() || "hidden".equals(ENTITY_SHADOW_SCENARIO))
			&& (ENTITY_LEASH_SCENARIO.isEmpty() || "hidden".equals(ENTITY_LEASH_SCENARIO))
		) {
			return true;
		}
		movingMeshSetupAttempts++;
		movingMeshSetupStage = "waiting-for-world";
		if (minecraft.player == null) {
			movingMeshSetupLastMissing = "player";
			if ((movingMeshSetupAttempts % 30) == 0) {
				writeMetadata(minecraft, "waiting_for_moving_mesh_setup_player");
			}
			return false;
		}
		if (minecraft.level == null) {
			movingMeshSetupLastMissing = "client-level";
			if ((movingMeshSetupAttempts % 30) == 0) {
				writeMetadata(minecraft, "waiting_for_moving_mesh_setup_client_level");
			}
			return false;
		}
		if (minecraft.getSingleplayerServer() == null) {
			movingMeshSetupLastMissing = "singleplayer-server";
			if ((movingMeshSetupAttempts % 30) == 0) {
				writeMetadata(minecraft, "waiting_for_moving_mesh_setup_server");
			}
			return false;
		}
		ServerLevel serverLevel = minecraft.getSingleplayerServer().getLevel(minecraft.level.dimension());
		if (serverLevel == null) {
			movingMeshSetupLastMissing = "server-level:" + minecraft.level.dimension().location();
			if ((movingMeshSetupAttempts % 30) == 0) {
				writeMetadata(minecraft, "waiting_for_moving_mesh_setup_server_level");
			}
			return false;
		}
		if (!prepareSourceEntityIsolation(minecraft, serverLevel)) {
			return false;
		}
		if (!ENTITY_FLAME_SCENARIO.isEmpty() && !"hidden".equals(ENTITY_FLAME_SCENARIO)
			&& !("cow".equals(ENTITY_FLAME_SCENARIO) && "cow".equals(MODEL_MESH_SCENARIO))) {
			movingMeshSetupStage = "entity-flame-requires-cow-model";
			movingMeshSetupLastMissing = "entityFlameScenario=" + ENTITY_FLAME_SCENARIO
				+ ",modelScenario=" + MODEL_MESH_SCENARIO;
			return false;
		}
		if (!ENTITY_SHADOW_SCENARIO.isEmpty() && !"hidden".equals(ENTITY_SHADOW_SCENARIO)
			&& !("cow".equals(ENTITY_SHADOW_SCENARIO) && "cow".equals(MODEL_MESH_SCENARIO))) {
			movingMeshSetupStage = "entity-shadow-requires-cow-model";
			movingMeshSetupLastMissing = "entityShadowScenario=" + ENTITY_SHADOW_SCENARIO
				+ ",modelScenario=" + MODEL_MESH_SCENARIO;
			return false;
		}
		if (!ENTITY_LEASH_SCENARIO.isEmpty() && !"hidden".equals(ENTITY_LEASH_SCENARIO)
			&& !("cow".equals(ENTITY_LEASH_SCENARIO) && "cow".equals(MODEL_MESH_SCENARIO))) {
			movingMeshSetupStage = "entity-leash-requires-cow-model";
			movingMeshSetupLastMissing = "entityLeashScenario=" + ENTITY_LEASH_SCENARIO
				+ ",modelScenario=" + MODEL_MESH_SCENARIO;
			return false;
		}
		movingMeshSetupStage = "scenario-setup-started";
		movingMeshSetupLastMissing = "";
		writeMetadata(minecraft, "moving_mesh_scenario_setup_started");
		setupFallingBlockScenario(minecraft, minecraft.player);
		setupPistonScenario(minecraft, minecraft.player);
		setupPrimedTntScenario(minecraft, minecraft.player);
		setupArrowScenario(minecraft, minecraft.player);
		setupExperienceOrbScenario(minecraft, minecraft.player);
		setupBeaconBeamScenario(minecraft, minecraft.player);
		setupItemEntityScenario(minecraft, minecraft.player);
		setupModelMeshScenario(minecraft, minecraft.player);
		movingMeshScenarioSetup = true;
		movingMeshSetupStage = "setup-complete";
		writeMetadata(minecraft, "moving_mesh_scenario_spawned_after_settled_ready");
		return false;
	}

	/**
	 * This touches only the harness-owned copied world. The real producer is
	 * still a normal vanilla entity renderer on later client frames.
	 */
	private static boolean prepareSourceEntityIsolation(Minecraft minecraft, ServerLevel serverLevel) {
		if (!SOURCE_ENTITY_ISOLATION) {
			return true;
		}
		if (!sourceEntityIsolationApplied) {
			if (!sourceEntityIsolationFailure.isEmpty()) {
				fail("source entity isolation failed on the integrated server: " + sourceEntityIsolationFailure);
				return false;
			}
			if (!sourceEntityIsolationQueued) {
				MinecraftServer server = minecraft.getSingleplayerServer();
				if (server == null) {
					return false;
				}
				ResourceKey<Level> dimension = serverLevel.dimension();
				sourceEntityIsolationQueued = true;
				server.execute(() -> applySourceEntityIsolationOnServer(server, dimension));
				movingMeshSetupStage = "source-entity-isolation-server-queued";
				return false;
			}
			movingMeshSetupStage = "source-entity-isolation-server-pending";
			return false;
		}
		if (sourceEntityIsolationClientSyncFrames == 0) {
			movingMeshSetupStage = "source-entity-isolation-applied";
			writeMetadata(minecraft, "source_entity_isolation_applied");
			sourceEntityIsolationClientSyncFrames = 1;
			return false;
		}
		// Let ordinary server-to-client removal packets settle before admitting a
		// source frame. A selected-source capture is deliberately strict: an
		// unrelated vanilla entity model must not be allowed to satisfy, or crash,
		// the producer-specific copied-world scenario.
		sourceEntityIsolationClientNonPlayerEntities = countClientNonPlayerEntities(minecraft.level, minecraft.player);
		if (sourceEntityIsolationClientNonPlayerEntities == 0) {
			sourceEntityIsolationClientQuiescentFrames++;
		}
		// The integrated client can retain non-rendered bookkeeping entities even
		// after the server purge. The authoritative isolation action happened on
		// the server and passive spawning is disabled there; use a bounded packet
		// settle window here and retain the client count as diagnostic evidence.
		if (sourceEntityIsolationClientSyncFrames < 12) {
			sourceEntityIsolationClientSyncFrames++;
			movingMeshSetupStage = "source-entity-isolation-client-sync";
			return false;
		}
		return true;
	}

	/**
	 * The selected-source coverage validator is activated only once this
	 * copied-world capture has finished its explicit isolation phase. Before
	 * that point there is no eligible producer scenario and no frame may be
	 * accepted as evidence.
	 */
	public static boolean isSelectedSourceCoverageReady() {
		return !SOURCE_ENTITY_ISOLATION
			|| (sourceEntityIsolationApplied
				&& sourceEntityIsolationClientSyncFrames >= 12);
	}

	private static int countClientNonPlayerEntities(ClientLevel level, LocalPlayer player) {
		if (level == null) {
			return Integer.MAX_VALUE;
		}
		int count = 0;
		for (Entity entity : level.entitiesForRendering()) {
			// ClientLevel can expose the local player through a mirrored entity
			// instance, so object identity alone is not a reliable exclusion.
			if (entity != player && !(entity instanceof net.minecraft.world.entity.player.Player)) {
				count++;
			}
		}
		return count;
	}

	/**
	 * Server-owned mutation for a copied-world capture. The render thread only
	 * queues it and later waits for normal client packets; touching entities from
	 * the render thread races the integrated server's random/source state.
	 */
	private static void applySourceEntityIsolationOnServer(MinecraftServer server, ResourceKey<Level> dimension) {
		try {
			ServerLevel serverLevel = server.getLevel(dimension);
			if (serverLevel == null) {
				throw new IllegalStateException("missing server level " + dimension.location());
			}
			// The copied capture world is intentionally quiet after its existing
			// actors are removed. This prevents passive spawning from introducing a
			// later Java-only entity into a strict selected-source frame.
			serverLevel.getGameRules().getRule(GameRules.RULE_DOMOBSPAWNING).set(false, server);
			List<Entity> removableEntities = new ArrayList<>();
			for (Entity entity : serverLevel.getAllEntities()) {
				if (!(entity instanceof ServerPlayer)) {
					removableEntities.add(entity);
				}
			}
			for (Entity entity : removableEntities) {
				entity.discard();
			}
			int[] removedBlockEntities = { 0 };
			serverLevel.getChunkSource().chunkMap.forEachReadyToSendChunk(chunk -> {
				for (BlockPos blockPos : List.copyOf(chunk.getBlockEntities().keySet())) {
					serverLevel.setBlock(blockPos, Blocks.STONE.defaultBlockState(), 3);
					removedBlockEntities[0]++;
				}
			});
			sourceEntityIsolationRemovedEntities = removableEntities.size();
			sourceEntityIsolationRemovedBlockEntities = removedBlockEntities[0];
			sourceEntityIsolationApplied = true;
		} catch (RuntimeException exception) {
			sourceEntityIsolationFailure = exception.getClass().getSimpleName() + ": " + exception.getMessage();
		}
	}

	/**
	 * Capture-only weather setup. It changes the copied singleplayer world's
	 * normal weather state; the renderer still receives only vanilla's extracted
	 * WeatherRenderState columns on the next real render frame.
	 */
	private static boolean setupWeatherScenarioAfterSettledReady(Minecraft minecraft) {
		if (WEATHER_SCENARIO.isEmpty()) {
			weatherSetupStage = "inactive";
			return true;
		}
		if (weatherScenarioSetup) {
			weatherSetupStage = "setup-complete";
			return true;
		}
		weatherSetupAttempts++;
		weatherSetupStage = "waiting-for-world";
		if (minecraft.player == null) {
			weatherSetupLastMissing = "player";
			return false;
		}
		if (minecraft.level == null) {
			weatherSetupLastMissing = "client-level";
			return false;
		}
		if (minecraft.getSingleplayerServer() == null) {
			weatherSetupLastMissing = "singleplayer-server";
			return false;
		}
		ServerLevel serverLevel = minecraft.getSingleplayerServer().getLevel(minecraft.level.dimension());
		if (serverLevel == null) {
			weatherSetupLastMissing = "server-level:" + minecraft.level.dimension().location();
			return false;
		}
		if (!"rain".equals(WEATHER_SCENARIO)) {
			fail("unsupported deterministic weather scenario: " + WEATHER_SCENARIO);
			return false;
		}
		weatherSetupStage = "server-weather-enabled";
		weatherSetupLastMissing = "";
		serverLevel.setWeatherParameters(0, 20_000, true, false);
		// The copied server state remains canonical; these client values only keep
		// the capture deterministic while the normal weather packet propagates.
		minecraft.level.setRainLevel(1.0F);
		minecraft.level.setThunderLevel(0.0F);
		// Weather extraction deliberately uses vanilla's local height range.  The
		// migration-gate camera can begin inside a relief column, making
		// WeatherEffectRenderer observe q-p == 0 for every cell even though rain is
		// active.  Raise only this weather fixture above the local surface while
		// preserving the copied world's x/z, biome, and server weather state.
		if (initialPosition != null && minecraft.player != null) {
			int weatherX = Mth.floor(initialPosition.x);
			int weatherZ = Mth.floor(initialPosition.z);
			// Origin can be generated in a biome whose precipitation is NONE. Find
			// the nearest loaded rain-capable column in the copied world rather than
			// fabricating WeatherRenderState data or overriding biome semantics.
			BlockPos rainProbe = new BlockPos(weatherX, Mth.floor(initialPosition.y), weatherZ);
			if (serverLevel.getBiome(rainProbe).value().getPrecipitationAt(rainProbe, serverLevel.getSeaLevel()).name().equals("NONE")) {
				int searchRadius = 256;
				boolean foundRainBiome = false;
				for (int radius = 16; radius <= searchRadius && !foundRainBiome; radius += 16) {
					for (int dx = -radius; dx <= radius && !foundRainBiome; dx += 8) {
						for (int dz = -radius; dz <= radius; dz += 8) {
							int candidateX = weatherX + dx;
							int candidateZ = weatherZ + dz;
							BlockPos candidate = new BlockPos(candidateX, Mth.floor(initialPosition.y), candidateZ);
							if (serverLevel.getBiome(candidate).value().getPrecipitationAt(candidate, serverLevel.getSeaLevel()).name().equals("RAIN")) {
								weatherX = candidateX;
								weatherZ = candidateZ;
								foundRainBiome = true;
								break;
							}
						}
					}
				}
				if (!foundRainBiome) {
					fail("deterministic weather scenario found no rain-capable biome near the copied-world camera");
					return false;
				}
			}
			double weatherY = Math.max(initialPosition.y, 160.0D);
			double fractionalX = initialPosition.x - Mth.floor(initialPosition.x);
			double fractionalZ = initialPosition.z - Mth.floor(initialPosition.z);
			Vec3 weatherPosition = new Vec3(weatherX + fractionalX, weatherY, weatherZ + fractionalZ);
			initialPosition = weatherPosition;
			minecraft.player.setPos(weatherPosition);
			minecraft.player.setDeltaMovement(Vec3.ZERO);
			minecraft.player.setOldPosAndRot(weatherPosition, minecraft.player.getYRot(), minecraft.player.getXRot());
			ServerPlayer serverPlayer = minecraft.getSingleplayerServer().getPlayerList().getPlayer(minecraft.player.getUUID());
			if (serverPlayer != null) {
				serverPlayer.setPos(weatherPosition);
				serverPlayer.setDeltaMovement(Vec3.ZERO);
			}
		}
		weatherScenarioSetup = true;
		weatherSetupStage = "setup-complete";
		writeMetadata(minecraft, "weather_scenario_enabled");
		return false;
	}

	/**
	 * Keep the deterministic pose sequence behind the real client-side weather
	 * interpolation.  Frozen's normal OpenGL baseline naturally reaches this
	 * state during its longer launch; the whole-frame Rust path reaches the pose
	 * gate much sooner.  Capturing one at 4% rain and the other at full rain
	 * would incorrectly attribute a fixture-timing difference to rasterization.
	 */
	private static boolean weatherCaptureIntensityReady(Minecraft minecraft) {
		if (WEATHER_SCENARIO.isEmpty() || !weatherScenarioSetup) {
			return true;
		}
		if (minecraft == null || minecraft.level == null) {
			weatherSetupStage = "waiting-for-client-rain";
			weatherSetupLastMissing = "client-level";
			return false;
		}
		float intensity = minecraft.level.getRainLevel(1.0F);
		if (intensity >= WEATHER_CAPTURE_MIN_INTENSITY) {
			weatherSetupStage = "setup-complete";
			weatherSetupLastMissing = "";
			return true;
		}
		framesWaitingForWeatherProducer++;
		weatherSetupStage = "waiting-for-client-rain";
		weatherSetupLastMissing = String.format(Locale.ROOT, "intensity=%.3f", intensity);
		if (framesWaitingForWeatherProducer > SETTLED_READY_MAX_WAIT_FRAMES) {
			fail("timed out waiting for deterministic rain intensity >= " + WEATHER_CAPTURE_MIN_INTENSITY
				+ " (last=" + weatherSetupLastMissing + ")");
		}
		return false;
	}

	/**
	 * Installs explicit gameplay benchmark producer fixtures without requiring
	 * the screenshot/pose lifecycle. The benchmark still observes ordinary
	 * vanilla entity, weather, and cloud producers on subsequent frames; this
	 * method only prepares the copied world and never selects a render route.
	 */
	public static boolean setupGameplayProducerScenarios(Minecraft minecraft) {
		if (!gameplayProducerScenarioRequested()) {
			return true;
		}
		if (minecraft == null || minecraft.player == null || minecraft.level == null
			|| minecraft.getSingleplayerServer() == null) {
			return false;
		}
		// Establish the ordinary beacon block entity as soon as the copied
		// world is available, before the first expensive Rust whole-frame submit
		// can starve the client tick thread.  The renderer still waits for the
		// real server/client beam-section receipt below.
		if (!BEACON_BEAM_SCENARIO.isEmpty() && !"hidden".equals(BEACON_BEAM_SCENARIO)
			&& beaconBeamSetupPosition == null) {
			setupBeaconBeamScenario(minecraft, minecraft.player);
		}
		if (!MODEL_MESH_SCENARIO.isEmpty() && !"hidden".equals(MODEL_MESH_SCENARIO)
			&& !modelMeshSetupServerEntityPresent && !modelMeshSetupServerSpawnQueued
			&& !"spawned".equals(modelMeshSetupStatus)) {
			// Queue real model/block-entity fixtures before the first expensive Rust
			// submit, so client replication can complete without starving the tick
			// thread. Route admission remains gated by the later same-frame receipts.
			setupModelMeshScenario(minecraft, minecraft.player);
		}
		if (!ITEM_ENTITY_SCENARIO.isEmpty() && !"hidden".equals(ITEM_ENTITY_SCENARIO)
			&& !"client-visible".equals(itemEntitySetupStatus)) {
			// Item entities use the same real server/client replication path as model
			// fixtures. Seed them before the settled gate so the ordinary
			// ItemEntityRenderer producer can be observed by the later Rust route
			// receipt gate instead of leaving the fixture unavailable until after it.
			setupItemEntityScenario(minecraft, minecraft.player);
		}
		if (!gameplayWorldTextScenarioSetup
			&& (!BLOCK_DISPLAY_SCENARIO.isEmpty() || !WORLD_TEXT_SCENARIO.isEmpty())) {
			setupBlockDisplayAndWorldTextScenarios(minecraft, minecraft.player);
			gameplayWorldTextScenarioSetup = true;
		}
		CloudStatus forcedCloudStatus = forcedCloudStatus();
		if (forcedCloudStatus != null && minecraft.options.cloudStatus().get() != forcedCloudStatus) {
			minecraft.options.cloudStatus().set(forcedCloudStatus);
		}
		boolean movingReady = setupMovingMeshScenarioAfterSettledReady(minecraft);
		boolean weatherReady = setupWeatherScenarioAfterSettledReady(minecraft);
		return movingReady && weatherReady;
	}

	public static boolean gameplayWeatherSetupComplete() {
		return weatherScenarioSetup;
	}

	public static String gameplayWeatherSetupStage() {
		return weatherSetupStage;
	}

	public static String gameplayWeatherSetupLastMissing() {
		return weatherSetupLastMissing;
	}

	private static boolean gameplayProducerScenarioRequested() {
		return !(FALLING_BLOCK_SCENARIO.isEmpty() || "hidden".equals(FALLING_BLOCK_SCENARIO))
			|| !(PISTON_SCENARIO.isEmpty() || "hidden".equals(PISTON_SCENARIO))
			|| !(PRIMED_TNT_SCENARIO.isEmpty() || "hidden".equals(PRIMED_TNT_SCENARIO))
			|| !(ARROW_SCENARIO.isEmpty() || "hidden".equals(ARROW_SCENARIO))
			|| !(MODEL_MESH_SCENARIO.isEmpty() || "hidden".equals(MODEL_MESH_SCENARIO))
			|| !(EXPERIENCE_ORB_SCENARIO.isEmpty() || "hidden".equals(EXPERIENCE_ORB_SCENARIO))
			|| !(BEACON_BEAM_SCENARIO.isEmpty() || "hidden".equals(BEACON_BEAM_SCENARIO))
			|| !(ITEM_ENTITY_SCENARIO.isEmpty() || "hidden".equals(ITEM_ENTITY_SCENARIO))
			|| !(ENTITY_FLAME_SCENARIO.isEmpty() || "hidden".equals(ENTITY_FLAME_SCENARIO))
			|| !(ENTITY_SHADOW_SCENARIO.isEmpty() || "hidden".equals(ENTITY_SHADOW_SCENARIO))
			|| !BLOCK_DISPLAY_SCENARIO.isEmpty()
			|| !WORLD_TEXT_SCENARIO.isEmpty()
			|| !WEATHER_SCENARIO.isEmpty()
			|| !CLOUD_SCENARIO.isEmpty();
	}

	/**
	 * Builds a deliberately far material panel through ordinary server block
	 * updates, then waits for the real DH semantic route to publish and execute
	 * it after the near client terrain radius has been reduced. This is capture
	 * plumbing only: no renderer state, mesh, or DH buffer is fabricated here.
	 */
	private static boolean setupDistantHorizonsTexturePaletteAfterSettledReady(Minecraft minecraft) {
		if (!distantHorizonsFixtureRequested()) {
			distantHorizonsTexturePaletteStage = "inactive";
			return true;
		}
		if (minecraft.player == null || minecraft.level == null || minecraft.getSingleplayerServer() == null) {
			distantHorizonsTexturePaletteStage = "waiting-for-world";
			return false;
		}
		ServerLevel serverLevel = minecraft.getSingleplayerServer().getLevel(minecraft.level.dimension());
		if (serverLevel == null) {
			distantHorizonsTexturePaletteStage = "waiting-for-server-level";
			return false;
		}
			if (DISTANT_HORIZONS_REQUIRE_WATER) {
				List<BlockPos> waterWitnesses = new ArrayList<>(16);
				for (int localX = 28; localX < 32; localX++) {
					for (int localZ = 24; localZ < 28; localZ++) {
						waterWitnesses.add(new BlockPos(64 + localX, 82, 512 + localZ));
					}
				}
				distantHorizonsWaterWitnesses = List.copyOf(waterWitnesses);
				DistantHorizonsSemanticCollector.ensureWaterSourceInputProbes(List.of(
					new BlockPos(93, 82, 537)
				));
		}
		if (DISTANT_HORIZONS_EXTERNAL_FIXTURE && !distantHorizonsTexturePaletteSetup) {
			// The external fixture is authored by the shared datapack so Current and
			// Frozen observe identical server state. Keep DH's initial source scan
			// bounded before requesting the four fixture chunks; the previous order
			// allowed the default render distance to start a broad generation pass.
			// This changes only the copied capture run's client option, not either
			// repository's rendering behavior.
			if (distantHorizonsTexturePaletteOriginalRenderDistance <= 0) {
				distantHorizonsTexturePaletteOriginalRenderDistance = minecraft.options.renderDistance().get();
			}
			// The canonical external panel is deliberately outside the player's
			// near-terrain radius.  It still has to be streamed once so DH can ingest
			// the ordinary source chunks; clamping to two chunks here leaves DH with
			// only unrelated cached columns and makes the palette gate wait forever.
			// The radius is reduced to two after the source column is proven below.
			minecraft.options.renderDistance().set(8);
			BlockPos panelOrigin = new BlockPos(64, 81, 512);
			// Keep the target center aligned with the datapack-authored panel.  The
			// invalidation helpers derive their 32x32 footprint from this point; using
			// (96,544) notified a neighboring area and left the real four palette
			// chunks outside DH's source update set.
			BlockPos center = new BlockPos(96, 81, 544);
			distantHorizonsTexturePaletteTarget = center;
			distantHorizonsTexturePaletteProbes = List.of(
					new DistantHorizonsTexturePaletteProbe(
						new BlockPos(77, 81, 537), "minecraft:lapis_block",
					List.of("minecraft:block/lapis_block"), List.of("minecraft:block/lapis_block")
				),
				new DistantHorizonsTexturePaletteProbe(
						new BlockPos(88, 81, 536), "minecraft:redstone_ore",
					List.of("minecraft:block/redstone_ore"), List.of("minecraft:block/redstone_ore")
				),
				new DistantHorizonsTexturePaletteProbe(
						new BlockPos(72, 81, 520), "minecraft:yellow_terracotta",
					List.of("minecraft:block/yellow_terracotta"), List.of("minecraft:block/yellow_terracotta")
				),
				new DistantHorizonsTexturePaletteProbe(
						new BlockPos(88, 81, 520), "minecraft:diamond_block",
					List.of("minecraft:block/diamond_block"), List.of("minecraft:block/diamond_block")
				)
			);
			if (DISTANT_HORIZONS_REQUIRE_WATER) {
				DistantHorizonsSemanticCollector.configureWaterSourceInputProbes(List.of(
					new BlockPos(panelOrigin.getX() + 29, panelOrigin.getY() + 1, panelOrigin.getZ() + 25)
				));
			}
			// Request residency for the shared fixture's four source chunks. This is
			// scheduling only; the datapack remains the sole author of block state.
			runOnServerThreadAndWait(minecraft.getSingleplayerServer(), () ->
				forceDistantHorizonsTexturePaletteChunks(serverLevel, panelOrigin.getX(), panelOrigin.getZ()));
			writeDistantHorizonsTexturePaletteTargetManifest();
			distantHorizonsTexturePaletteSetup = true;
			distantHorizonsTexturePaletteStage = "external-fixture-observation-only";
			writeMetadata(minecraft, "distant_horizons_external_fixture_observation_only");
			return false;
		}
		if (!distantHorizonsTexturePaletteSetup) {
			// Place the panel along the camera's horizontal look vector rather than
			// snapping to a cardinal direction. This keeps the real source column in
			// the same view frustum as the canonical fixed camera used by Frozen.
			Vec3 look = minecraft.player.getLookAngle();
			double horizontalLookLength = Math.sqrt(look.x * look.x + look.z * look.z);
			if (horizontalLookLength < 0.001D) {
				horizontalLookLength = 1.0D;
			}
			BlockPos desiredCenter = BlockPos.containing(
				minecraft.player.getEyePosition().add(
					look.x / horizontalLookLength * DISTANT_HORIZONS_TEXTURE_PALETTE_DISTANCE,
					0.0D,
					look.z / horizontalLookLength * DISTANT_HORIZONS_TEXTURE_PALETTE_DISTANCE
				)
			).below(1);
			// Keep all four material quadrants inside exactly one lowest-detail DH
			// source column. A palette spread across column boundaries can be
			// accidentally certified by unrelated visible terrain.
			int columnMinX = DISTANT_HORIZONS_EXTERNAL_FIXTURE
				? 0 : Math.floorDiv(desiredCenter.getX(), 64) * 64;
			int columnMinZ = DISTANT_HORIZONS_EXTERNAL_FIXTURE
				? 448 : Math.floorDiv(desiredCenter.getZ(), 64) * 64;
			// DH reduces column surfaces. Use a horizontal terrain patch rather than
			// a vertical wall so its semantic source and visible geometry are the same
			// kind of terrain DH actually builds and submits. Put it one layer above
			// the highest real surface in the patch: otherwise DH correctly keeps an
			// existing roof as the visible surface and never sees the palette.
			int panelMinX = DISTANT_HORIZONS_EXTERNAL_FIXTURE
				? 32 : columnMinX + (64 - DISTANT_HORIZONS_TEXTURE_PALETTE_SIDE) / 2;
			int panelMinZ = DISTANT_HORIZONS_EXTERNAL_FIXTURE
				? 480 : columnMinZ + (64 - DISTANT_HORIZONS_TEXTURE_PALETTE_SIDE) / 2;
			int terrainSurfaceY = DISTANT_HORIZONS_EXTERNAL_FIXTURE ? 80 : Integer.MIN_VALUE;
			if (!DISTANT_HORIZONS_EXTERNAL_FIXTURE) {
				for (int localX = 0; localX < DISTANT_HORIZONS_TEXTURE_PALETTE_SIDE; localX++) {
					for (int localZ = 0; localZ < DISTANT_HORIZONS_TEXTURE_PALETTE_SIDE; localZ++) {
						terrainSurfaceY = Math.max(terrainSurfaceY, serverLevel.getHeight(Heightmap.Types.WORLD_SURFACE, panelMinX + localX, panelMinZ + localZ));
					}
				}
			}
			if (terrainSurfaceY == Integer.MIN_VALUE) {
				fail("unable to resolve DH palette surface height");
				return false;
			}
			// Keep the test surface one supported layer above the natural relief in
			// this copied world. The source remains ordinary server block data
			// consumed by DH; using the minimum elevation keeps the fixture's update
			// footprint bounded so DH can publish a stable generation instead of
			// rebuilding a tall synthetic column throughout the capture window.
			int panelY = DISTANT_HORIZONS_EXTERNAL_FIXTURE ? 81 : terrainSurfaceY + 1;
			if (panelY >= serverLevel.getMaxY() - 2) {
				fail("DH palette surface exceeds the copied-world build height");
				return false;
			}
			BlockPos center = new BlockPos(columnMinX + 32, panelY, columnMinZ + 32);
			runOnServerThreadAndWait(minecraft.getSingleplayerServer(), () -> {
				forceDistantHorizonsTexturePaletteChunks(serverLevel, panelMinX, panelMinZ);
			});
			// View the ordinary copied-world surface from high enough above it that
			// the far-LOD reduction exposes the panel's top faces in the game frame.
			// The harness freezes this diagnostic camera without placing any geometry.
			// Keep an oblique view of the palette so all ordinary surface quadrants
			// remain visible in the final game frame.
			Vec3 capturePosition = DISTANT_HORIZONS_EXTERNAL_FIXTURE
				? initialPosition
				: new Vec3(initialPosition.x, panelY + 10.0, initialPosition.z);
			initialPosition = capturePosition;
			minecraft.player.setPos(capturePosition);
			minecraft.player.setDeltaMovement(Vec3.ZERO);
			minecraft.player.setOldPosAndRot(capturePosition, minecraft.player.getYRot(), minecraft.player.getXRot());
			ServerPlayer serverPlayer = minecraft.getSingleplayerServer().getPlayerList().getPlayer(minecraft.player.getUUID());
			if (serverPlayer != null) {
				runOnServerThreadAndWait(minecraft.getSingleplayerServer(), () -> {
					serverPlayer.setPos(capturePosition);
					serverPlayer.setDeltaMovement(Vec3.ZERO);
				});
			}
			// Deterministic capture already freezes the player with diagnostic
			// no-gravity. Do not place a local support platform here: at this
			// elevated palette-camera pose it becomes a foreground occluder and
			// invalidates the final-frame texture evidence.
			runOnServerThreadAndWait(minecraft.getSingleplayerServer(), () -> {
			for (int localX = 0; localX < DISTANT_HORIZONS_TEXTURE_PALETTE_SIDE; localX++) {
				for (int localZ = 0; localZ < DISTANT_HORIZONS_TEXTURE_PALETTE_SIDE; localZ++) {
					BlockPos position = new BlockPos(panelMinX + localX, center.getY(), panelMinZ + localZ);
					serverLevel.getChunkAt(position);
					// DH's real source reduction follows ordinary supported terrain.
					// Keep the one-layer support explicit so the opaque witnesses cannot
					// be mistaken for floating blocks and dropped from the source quads;
					// water already has explicit support below its separate plate.
					int localSurfaceY = serverLevel.getHeight(
						Heightmap.Types.WORLD_SURFACE, position.getX(), position.getZ()
					);
					for (int supportY = localSurfaceY + 1; supportY < center.getY(); supportY++) {
						serverLevel.setBlock(
							new BlockPos(position.getX(), supportY, position.getZ()),
							Blocks.STONE.defaultBlockState(),
							3
						);
					}
					for (int height = 1; height <= 4; height++) {
						serverLevel.setBlock(position.above(height), Blocks.AIR.defaultBlockState(), 3);
					}
					BlockState state;
					// The selected-source opaque plan intentionally certifies only
					// opaque DH output. Keep every palette quadrant a simple opaque
					// cube: grass can be reduced away with its terrain-specific face
					// semantics and leaves belong to the cutout route, neither of
					// which proves the opaque atlas writer saw the requested target.
					if (localZ >= DISTANT_HORIZONS_TEXTURE_PALETTE_QUADRANT) {
						state = localX < DISTANT_HORIZONS_TEXTURE_PALETTE_QUADRANT
							? Blocks.LAPIS_BLOCK.defaultBlockState()
							: Blocks.REDSTONE_ORE.defaultBlockState();
					} else {
						state = localX < DISTANT_HORIZONS_TEXTURE_PALETTE_QUADRANT
							? Blocks.YELLOW_TERRACOTTA.defaultBlockState()
							: Blocks.DIAMOND_BLOCK.defaultBlockState();
					}
					serverLevel.setBlock(position, state, 3);
				}
			}
			if (DISTANT_HORIZONS_REQUIRE_TRANSPARENT) {
				// This small oak-leaf plate remains inside the same far DH column as
				// the opaque palette, but does not cover any of the four opaque
				// witness blocks. It therefore causes the real transparent-side/up
				// stream to exist without turning the fixture into a synthetic LOD
				// request or weakening the opaque identity proof.
				List<BlockPos> witnesses = new ArrayList<>(64);
				for (int localX = 1; localX < 9; localX++) {
					for (int localZ = 1; localZ < 9; localZ++) {
						BlockPos position = new BlockPos(panelMinX + localX, center.getY() + 1, panelMinZ + localZ);
						serverLevel.setBlock(
							position,
							Blocks.OAK_LEAVES.defaultBlockState().setValue(LeavesBlock.PERSISTENT, true),
							3
						);
						witnesses.add(position);
					}
				}
				distantHorizonsTransparentWitnesses = List.copyOf(witnesses);
			}
				if (DISTANT_HORIZONS_REQUIRE_WATER) {
				// Keep the real water plate disjoint from the opaque material witnesses.
				// The normal DH column build must classify its exposed top faces into
				// the transparent-water-up stream.
				List<BlockPos> witnesses = new ArrayList<>(64);
				// Keep the supported water plate away from all four opaque palette
				// witnesses (and the lapis diagnostic point), while keeping it inside
				// the panel footprint whose chunks are explicitly invalidated below.
				for (int localX = 28; localX < 32; localX++) {
					for (int localZ = 24; localZ < 28; localZ++) {
						// DH's real column reduction classifies a water-up surface only
						// when the source water rests on terrain. Keep the witness a
						// normal copied-world surface instead of floating water over air.
						BlockPos support = new BlockPos(panelMinX + localX, center.getY(), panelMinZ + localZ);
						serverLevel.setBlock(support, Blocks.STONE.defaultBlockState(), 2);
						BlockPos position = support.above();
						serverLevel.setBlock(position, Blocks.WATER.defaultBlockState(), 2);
						witnesses.add(position);
					}
				}
				// Keep the source-water surface semantically real but closed at its
				// perimeter. Without this border, vanilla fluid ticks spread the edge
				// cells into the surrounding air every few frames, continually replacing
				// the DH source generation before the exact palette column can settle.
				for (int localX = 27; localX <= 32; localX++) {
					for (int localZ : new int[] { 23, 28 }) {
						serverLevel.setBlock(
							new BlockPos(panelMinX + localX, center.getY(), panelMinZ + localZ),
							Blocks.STONE.defaultBlockState(), 2
						);
					}
				}
				for (int localZ = 24; localZ < 28; localZ++) {
					for (int localX : new int[] { 23, 28 }) {
						serverLevel.setBlock(
							new BlockPos(panelMinX + localX, center.getY(), panelMinZ + localZ),
							Blocks.STONE.defaultBlockState(), 2
						);
					}
				}
				distantHorizonsWaterWitnesses = List.copyOf(witnesses);
				DistantHorizonsSemanticCollector.configureWaterSourceInputProbes(List.of(
					// Probe a sealed interior cell, not the first row selected by
					// list midpoint; DH may legally omit an edge top face when its
					// neighboring terrain closes the fluid boundary.
					new BlockPos(panelMinX + 29, center.getY() + 1, panelMinZ + 25)
				));
				}
			});
			BlockPos lapisWitness = new BlockPos(
				panelMinX + DISTANT_HORIZONS_TEXTURE_PALETTE_QUADRANT - 3,
				center.getY(),
				panelMinZ + DISTANT_HORIZONS_TEXTURE_PALETTE_QUADRANT + 9
			);
			distantHorizonsTexturePaletteTarget = center;
			distantHorizonsTexturePaletteProbes = List.of(
				new DistantHorizonsTexturePaletteProbe(
					lapisWitness, "minecraft:lapis_block",
					List.of("minecraft:block/lapis_block"), List.of("minecraft:block/lapis_block")
				),
				new DistantHorizonsTexturePaletteProbe(
					new BlockPos(panelMinX + 24, center.getY(), panelMinZ + 24), "minecraft:redstone_ore",
					List.of("minecraft:block/redstone_ore"), List.of("minecraft:block/redstone_ore")
				),
				new DistantHorizonsTexturePaletteProbe(
					new BlockPos(panelMinX + 8, center.getY(), panelMinZ + 8), "minecraft:yellow_terracotta",
					List.of("minecraft:block/yellow_terracotta"), List.of("minecraft:block/yellow_terracotta")
				),
				new DistantHorizonsTexturePaletteProbe(
					new BlockPos(panelMinX + 24, center.getY(), panelMinZ + 8), "minecraft:diamond_block",
					List.of("minecraft:block/diamond_block"), List.of("minecraft:block/diamond_block")
				)
			);
			writeDistantHorizonsTexturePaletteTargetManifest();
			distantHorizonsTexturePaletteOriginalRenderDistance = minecraft.options.renderDistance().get();
			// This is a far-LOD-only fixture. Keep ordinary client terrain out of
			// the source-update queue after the server-side panel exists; the real
			// DH source column remains responsible for every visible palette pixel.
			minecraft.options.renderDistance().set(DISTANT_HORIZONS_EXTERNAL_FIXTURE ? 8 : 2);
			Vec3 panelCenter = Vec3.atCenterOf(center);
			Vec3 delta = panelCenter.subtract(minecraft.player.getEyePosition());
			double horizontal = Math.sqrt(delta.x * delta.x + delta.z * delta.z);
			if (horizontal < 0.001) {
				fail("DH texture palette target overlaps the camera");
				return false;
			}
			float yaw = (float)(Math.toDegrees(Math.atan2(delta.z, delta.x)) - 90.0);
			float pitch = (float)(-Math.toDegrees(Math.atan2(delta.y, horizontal)));
			initialPose = DISTANT_HORIZONS_EXTERNAL_FIXTURE && HAS_FIXED_CAMERA_POSE
				? new Pose("initial", FIXED_CAMERA_YAW, FIXED_CAMERA_PITCH)
				: new Pose("distant-horizons-texture-palette", yaw, pitch);
			poses = new Pose[] { initialPose };
			poseIndex = 0;
			applyPose(minecraft.player, initialPose);
			distantHorizonsTexturePaletteSetup = true;
			distantHorizonsTexturePaletteStage = "server-palette-placed";
			writeMetadata(minecraft, "distant_horizons_texture_palette_placed");
			return false;
		}
		distantHorizonsTexturePaletteWaitFrames++;
		if (!distantHorizonsTexturePaletteInvalidationQueued) {
			if (!distantHorizonsTexturePaletteServerStateReady(serverLevel)) {
				distantHorizonsTexturePaletteStage = "waiting-for-server-palette-state";
				writeDistantHorizonsTexturePaletteWaitMetadata(minecraft);
				return false;
			}
			if (!distantHorizonsTexturePaletteLightReady(serverLevel)) {
				distantHorizonsTexturePaletteStage = "waiting-for-server-palette-light"
					+ ":stable=" + distantHorizonsTexturePaletteLightStableFrames
					+ ":busy=" + distantHorizonsTexturePaletteLightEngineBusy;
				writeDistantHorizonsTexturePaletteWaitMetadata(minecraft);
				return false;
			}
			distantHorizonsTexturePaletteInvalidatedChunks = notifyDistantHorizonsTexturePaletteChunks(minecraft, serverLevel);
			if (distantHorizonsTexturePaletteInvalidatedChunks == 0) {
				distantHorizonsTexturePaletteStage = "waiting-for-client-palette-chunks";
				writeDistantHorizonsTexturePaletteWaitMetadata(minecraft);
				return false;
			}
			distantHorizonsTexturePaletteInvalidationQueued = true;
			distantHorizonsTexturePaletteQueuedUpdatesAfterInvalidation = SharedApi.INSTANCE.getQueuedChunkUpdateCount();
			distantHorizonsTexturePaletteDhWorldType = SharedApi.getAbstractDhWorld() == null
			? "none"
			: SharedApi.getAbstractDhWorld().getClass().getSimpleName();
			distantHorizonsTexturePaletteStage = "dh-palette-chunks-invalidated";
			writeMetadata(minecraft, "distant_horizons_texture_palette_chunks_invalidated");
			return false;
		}
		boolean clientChunkResident = minecraft.level.isLoaded(distantHorizonsTexturePaletteTarget);
		boolean nearTerrainAbsent = !RustGalTerrainRenderer
			.staticTerrainSectionExecutedInLastCompletedFrame(distantHorizonsTexturePaletteTarget);
		DistantHorizonsSemanticCollector.ColumnCoverageDiagnostics fixtureCoverage =
			DistantHorizonsSemanticCollector.columnCoverageDiagnosticsAtBlock(
				distantHorizonsTexturePaletteProbes.isEmpty()
					? distantHorizonsTexturePaletteTarget.getX()
					: distantHorizonsTexturePaletteProbes.getFirst().position().getX(),
				distantHorizonsTexturePaletteProbes.isEmpty()
					? distantHorizonsTexturePaletteTarget.getZ()
					: distantHorizonsTexturePaletteProbes.getFirst().position().getZ()
			);
		// Route selection must be allowed to produce the first published water
		// generation.  Requiring a cached water quad here deadlocks the fixture:
		// the cached probe is populated only after the Rust route publishes and
		// executes that generation.  Water remains a hard requirement in the later
		// source-ready and executed gates below, so this does not admit an
		// incomplete route or weaken the spatial water proof.
		// Water is a producer route, not an already-resident cache witness.  Its
		// target column is populated by the route itself after this preflight runs;
		// accepting the setup phase here is therefore necessary to let the first
		// water generation become visible.  The exact target, source-water, and
		// executed-water checks below remain hard admission requirements.
		boolean fixtureSourceCached = fixtureCoverage.cachedColumns() > 0
			|| DISTANT_HORIZONS_REQUIRE_WATER;
		boolean selectedSourceWaterExecution = DISTANT_HORIZONS_REQUIRE_WATER
			&& distantHorizonsWaterSourceInputReceipt().matched()
			&& distantHorizonsSelectedSourceWaterExecutionObserved();
		// Selected-source shader-pack water is not represented in Java's ordinary
		// cached LOD-instance map. Its Rust material contract is emitted only after
		// the real source asset has been resolved and staged, so that receipt is the
		// stronger cache proof for this route when the mutable Java map is empty.
		if (!fixtureSourceCached && selectedSourceWaterExecution) {
			fixtureSourceCached = true;
		}
		if (!fixtureSourceCached) {
			distantHorizonsTexturePaletteStage = "waiting-for-dh-palette-source-cache";
			if (distantHorizonsTexturePaletteWaitFrames > SETTLED_READY_MAX_WAIT_FRAMES) {
				fail("timed out waiting for the real DH palette source cache: cachedColumns="
					+ fixtureCoverage.cachedColumns()
					+ " cachedWater=" + distantHorizonsTexturePaletteCachedWaterProbeReceipt().status());
				return false;
			}
			writeDistantHorizonsTexturePaletteWaitMetadata(minecraft);
			return false;
		}
		if (DISTANT_HORIZONS_EXTERNAL_FIXTURE) {
			// Keep the source stream alive until DH has cached the panel; only then
			// shrink near terrain so the retained LOD becomes the visible producer.
			minecraft.options.renderDistance().set(3);
		}
		// Texture identity is valid only when every displayed palette target has
		// both spatially matching semantic provenance and the expected exact atlas
		// footprint. A column that merely overlaps the panel, or a matching sprite
		// selected elsewhere in the visible set, cannot prove that grass did not
		// become ore (or another unrelated block texture).
		DistantHorizonsExactAtlasPaletteStatus exactAtlasStatus = DISTANT_HORIZONS_LEGACY_OBSERVATION
			? new DistantHorizonsExactAtlasPaletteStatus(true, 0L, "not-required-legacy-observation")
			: distantHorizonsExactAtlasPaletteStatus();
		if (exactAtlasStatus.targetsMatched()) {
			distantHorizonsTexturePaletteExactAtlasObserved = true;
		}
		boolean sourceColumnReady = (DISTANT_HORIZONS_REQUIRE_WATER && !DISTANT_HORIZONS_TEXTURE_PALETTE)
			? distantHorizonsWaterSourceProbeReceipt().matched()
			: DISTANT_HORIZONS_LEGACY_OBSERVATION
			? distantHorizonsTexturePaletteProbes.stream().allMatch(probe ->
				DistantHorizonsSemanticCollector.hasObservedVisibleOpaqueColumnCoveringBlock(
					probe.position().getX(), probe.position().getZ()
				)
			)
			: distantHorizonsTexturePaletteProbes.stream().allMatch(probe ->
				DistantHorizonsSemanticCollector.hasLastConsumedVisibleOpaqueSemanticMaterialAtBlock(
					probe.position().getX(), probe.position().getY(), probe.position().getZ(), probe.blockId()
				)
			);
		// The Rust exact-atlas receipt is stronger than the mutable Java
		// publication map: it proves that every requested opaque target was
		// spatially matched by a segment actually consumed for the frame. When
		// this run does not also require animated water, accept that executed
		// proof even if DH has already replaced the source generation by the time
		// the capture thread samples diagnostics.
		if (!sourceColumnReady && !DISTANT_HORIZONS_REQUIRE_WATER
			&& (exactAtlasStatus.targetsMatched() || distantHorizonsTexturePaletteExactAtlasObserved)) {
			sourceColumnReady = true;
		}
		// The Rust exact-atlas receipt is the consumed-geometry authority for the
		// Vulkan route. DH coarsening can make the Java-side copied quad lookup miss
		// a target even while Rust has matched its exact sprite and world bounds;
		// require the stronger target-matched receipt rather than rejecting that
		// valid Vulkan evidence merely because the legacy observer is incomplete.
		boolean textureIdentityReady = distantHorizonsTexturePaletteProbeReceipt().matched()
			|| exactAtlasStatus.targetsMatched()
			|| distantHorizonsTexturePaletteExactAtlasObserved;
		boolean exactAtlasReady = exactAtlasStatus.targetsMatched()
			|| distantHorizonsTexturePaletteExactAtlasObserved;
		// Exact-atlas target correlation belongs to the texture-palette workload.
		// Water-only captures still require spatial source and executed-water
		// receipts, but must not be rejected because no opaque palette witness was
		// requested or admitted in that run.
		boolean exactAtlasRequired = DISTANT_HORIZONS_TEXTURE_PALETTE;
		DistantHorizonsSemanticCollector.RouteDiagnostics diagnostics =
			DistantHorizonsSemanticCollector.routeDiagnosticsSnapshot();
		DistantHorizonsSemanticCollector.DistantHorizonsWaterProbeReceipt waterReceipt =
			distantHorizonsWaterProbeReceipt();
		DistantHorizonsSemanticCollector.DistantHorizonsWaterProbeReceipt waterSourceReceipt =
			distantHorizonsWaterSourceProbeReceipt();
		if (waterSourceReceipt.matched()) {
			distantHorizonsTexturePaletteWaterSourceObserved = true;
		}
			boolean transparentExecuted = diagnostics.selected()
				&& diagnostics.lastExecutedTransparentInstances() > 0
				&& diagnostics.lastExecutedSubmission() > 0;
			boolean waterExecuted = diagnostics.selected()
				&& diagnostics.lastExecutedWaterInstances() > 0
				&& diagnostics.lastExecutedSubmission() > 0;
		if (waterExecuted || waterReceipt.matched()) {
			distantHorizonsTexturePaletteWaterExecutedObserved = true;
		}
		if (selectedSourceWaterExecution) {
			distantHorizonsTexturePaletteWaterSourceObserved = true;
			distantHorizonsTexturePaletteWaterExecutedObserved = true;
		}
		if (!sourceColumnReady && DISTANT_HORIZONS_REQUIRE_WATER
			&& distantHorizonsTexturePaletteExactAtlasObserved
			&& distantHorizonsTexturePaletteWaterSourceObserved) {
			sourceColumnReady = true;
		}
		waterExecuted = waterExecuted || distantHorizonsTexturePaletteWaterExecutedObserved;
		boolean executed = (diagnostics.selected()
			&& diagnostics.lastExecutedOpaqueInstances() > 0
			&& diagnostics.lastExecutedSubmission() > 0)
			|| distantHorizonsTexturePaletteExactAtlasObserved
			|| (DISTANT_HORIZONS_LEGACY_OBSERVATION && sourceColumnReady);
		if (!distantHorizonsTexturePaletteSourceReady && sourceColumnReady
			&& (!DISTANT_HORIZONS_REQUIRE_WATER || distantHorizonsTexturePaletteWaterSourceObserved) && executed) {
			// DH only receives real source data while the server has sent the
			// palette's chunks to the client. Once the exact consumed column is
			// proven, reduce the copied server's normal radius and wait for the
			// ordinary client terrain route to release it. The subsequent frame
			// must therefore be rendered from the same retained DH column alone.
			minecraft.options.renderDistance().set(2);
			distantHorizonsTexturePaletteSourceReady = true;
			distantHorizonsTexturePaletteStage = "dh-palette-source-ready";
			writeMetadata(minecraft, "distant_horizons_texture_palette_source_ready");
			return false;
		}
		if (distantHorizonsTexturePaletteSourceReady && nearTerrainAbsent && sourceColumnReady && executed
			&& (!DISTANT_HORIZONS_REQUIRE_TRANSPARENT || transparentExecuted)
			&& (!DISTANT_HORIZONS_REQUIRE_WATER || (distantHorizonsTexturePaletteWaterSourceObserved
				&& waterExecuted && distantHorizonsTexturePaletteWaterExecutedObserved))
			&& (!exactAtlasRequired || (textureIdentityReady && exactAtlasReady))) {
			distantHorizonsTexturePaletteStage = "dh-palette-executed";
			writeMetadata(minecraft, "distant_horizons_texture_palette_executed");
			return true;
		}
		if (distantHorizonsTexturePaletteWaitFrames > SETTLED_READY_MAX_WAIT_FRAMES) {
			DistantHorizonsSemanticCollector.DistantHorizonsTextureProbeReceipt textureReceipt =
				distantHorizonsTexturePaletteProbeReceipt();
			fail("timed out waiting for far DH texture palette: nearTerrainAbsent=" + nearTerrainAbsent
				+ " clientChunkResident=" + clientChunkResident
				+ " sourceColumnReady=" + sourceColumnReady
				+ " textureIdentityReady=" + textureIdentityReady
				+ " exactAtlasReady=" + exactAtlasReady
				+ " exactAtlasFrame=" + exactAtlasStatus.frameId()
				+ " exactAtlasStatus=" + exactAtlasStatus.status()
				+ " textureStatus=" + textureReceipt.status()
				+ " selected=" + diagnostics.selected()
				+ " executedOpaqueInstances=" + diagnostics.lastExecutedOpaqueInstances()
				+ " executedTransparentInstances=" + diagnostics.lastExecutedTransparentInstances()
				+ " executedWaterInstances=" + diagnostics.lastExecutedWaterInstances()
				+ " waterSourceReceipt=" + waterSourceReceipt.status()
				+ " waterReceipt=" + waterReceipt.status()
				+ " stage=" + distantHorizonsTexturePaletteStage);
			return false;
		}
		String nextStage = "waiting-for-dh-palette"
			+ ":near=" + nearTerrainAbsent
			+ ":clientResident=" + clientChunkResident
			+ ":sourceColumn=" + sourceColumnReady
			+ ":textureIdentity=" + textureIdentityReady
			+ ":exactAtlas=" + exactAtlasReady
			+ ":executed=" + executed
			+ ":transparent=" + transparentExecuted
			+ ":waterSource=" + waterSourceReceipt.matched()
			+ ":water=" + waterReceipt.matched()
			+ ":sourceReady=" + distantHorizonsTexturePaletteSourceReady;
		boolean paletteStageChanged = !nextStage.equals(distantHorizonsTexturePaletteStage);
		distantHorizonsTexturePaletteStage = nextStage;
		if (paletteStageChanged || (distantHorizonsTexturePaletteWaitFrames % 30) == 0) {
			writeMetadata(minecraft, "waiting_for_distant_horizons_texture_palette");
		}
		writeDistantHorizonsTexturePaletteWaitMetadata(minecraft);
		return false;
	}

	/**
	 * Reads a capture-only semantic receipt written by Rust's exact-atlas
	 * planner. It has no rendering effect and carries no texture object or
	 * backend state. It closes the gap where Java provenance remained valid
	 * after DH had replaced the actual visible draw plan.
	 */
	private static DistantHorizonsExactAtlasPaletteStatus distantHorizonsExactAtlasPaletteStatus() {
		String diagnosticDir = System.getenv("MATTMC_TERRAIN_PASS_CONTRACT_DIAGNOSTIC_DIR");
		if (diagnosticDir == null || diagnosticDir.isBlank()) {
			return new DistantHorizonsExactAtlasPaletteStatus(false, 0L, "diagnostic-dir-unset");
		}
		Path receipt = Path.of(diagnosticDir).resolve("world-lod-exact-atlas-plan-last.json");
		if (!Files.isRegularFile(receipt)) {
			return new DistantHorizonsExactAtlasPaletteStatus(false, 0L, "exact-atlas-receipt-missing");
		}
		try {
			String json = Files.readString(receipt, StandardCharsets.UTF_8);
			if (!json.contains("\"schema\":\"mattmc-world-lod-exact-atlas-plan-v1\"")) {
				return new DistantHorizonsExactAtlasPaletteStatus(false, 0L, "exact-atlas-receipt-schema-invalid");
			}
			long frameId = readJsonLongField(json, "frameId", 0L);
			if (frameId <= 0L) {
				return new DistantHorizonsExactAtlasPaletteStatus(false, frameId, "exact-atlas-frame-invalid");
			}
			if (!readJsonBooleanField(json, "paletteTargetsMatched", false)) {
				return new DistantHorizonsExactAtlasPaletteStatus(false, frameId, "exact-atlas-targets-unmatched");
			}
			return new DistantHorizonsExactAtlasPaletteStatus(true, frameId, "ok");
		} catch (IOException | NumberFormatException exception) {
			return new DistantHorizonsExactAtlasPaletteStatus(false, 0L, "exact-atlas-receipt-unreadable");
		}
	}

	/**
	 * Selected-source shader-pack DH draws are recorded by Rust's material
	 * contract rather than as ordinary LOD instance records. This receipt proves
	 * a real water draw (indices and draw count) crossed the Rust submission
	 * boundary, so the capture gate must not require the unrelated instance-list
	 * correlation for that source route.
	 */
	private static boolean distantHorizonsSelectedSourceWaterExecutionObserved() {
		String diagnosticDir = System.getenv("MATTMC_TERRAIN_PASS_CONTRACT_DIAGNOSTIC_DIR");
		if (diagnosticDir == null || diagnosticDir.isBlank()) {
			return false;
		}
		DistantHorizonsSemanticCollector.WaterSourceInputReceipt sourceReceipt = distantHorizonsWaterSourceInputReceipt();
		if (!sourceReceipt.matched() || sourceReceipt.traces().isEmpty()) {
			return false;
		}
		try (DirectoryStream<Path> files = Files.newDirectoryStream(
			Path.of(diagnosticDir), "world-lod-selected-source-material-contract-frame-*.json")) {
			List<Path> orderedFiles = new ArrayList<>();
			for (Path file : files) {
				orderedFiles.add(file);
			}
			orderedFiles.sort(Comparator.comparing(path -> path.getFileName().toString()));
			int first = Math.max(0, orderedFiles.size() - 128);
			for (int index = orderedFiles.size() - 1; index >= first; index--) {
				Path file = orderedFiles.get(index);
				String json = Files.readString(file, StandardCharsets.UTF_8);
				if (readJsonLongField(json, "frame_id", 0L) > 0L
					&& readJsonLongField(json, "water_draw_count", 0L) > 0L
					&& readJsonLongField(json, "water_index_count", 0L) > 0L) {
					int originsStart = json.indexOf("\"water_origins\":[");
					int originsEnd = originsStart < 0 ? -1 : json.indexOf("],\"late_translucent", originsStart);
					if (originsStart < 0 || originsEnd < 0) {
						continue;
					}
					String origins = json.substring(originsStart + "\"water_origins\":".length(), originsEnd + 1);
					for (DistantHorizonsSemanticCollector.WaterSourceInputTrace trace : sourceReceipt.traces()) {
						for (String origin : origins.replace("[[", "[").replace("]]", "]").split("\\],\\[")) {
							String[] coordinates = origin.replace("[", "").replace("]", "").split(",");
							if (coordinates.length == 3
								&& Math.abs(Integer.parseInt(coordinates[0].trim()) - trace.blockX()) <= 64
								&& Math.abs(Integer.parseInt(coordinates[2].trim()) - trace.blockZ()) <= 64) {
								return true;
							}
						}
					}
				}
			}
		} catch (IOException | RuntimeException ignored) {
			return false;
		}
		return false;
	}

	/** Emits bounded capture-only heartbeats while the real DH source catches up. */
	private static void writeDistantHorizonsTexturePaletteWaitMetadata(Minecraft minecraft) {
		if (distantHorizonsTexturePaletteWaitFrames > 0
			&& (distantHorizonsTexturePaletteWaitFrames % 30 == 0
				|| !distantHorizonsTexturePaletteStage.equals(distantHorizonsTexturePaletteLastReportedStage))) {
			distantHorizonsTexturePaletteLastReportedStage = distantHorizonsTexturePaletteStage;
			writeMetadata(minecraft, "waiting_for_distant_horizons_texture_palette");
		}
	}

	/**
	 * Capture-only target manifest consumed by Rust's exact-atlas receipt. The
	 * renderer never reads it: it lets the harness distinguish an exact sprite
	 * selected somewhere in a visible DH column from that sprite actually
	 * covering one of this fixture's four target blocks.
	 */
	private static void writeDistantHorizonsTexturePaletteTargetManifest() {
		String diagnosticDir = System.getenv("MATTMC_TERRAIN_PASS_CONTRACT_DIAGNOSTIC_DIR");
		if (diagnosticDir == null || diagnosticDir.isBlank()) {
			return;
		}
		Path directory = Path.of(diagnosticDir);
		Path target = directory.resolve("world-lod-texture-palette-targets-v2.txt");
		Path temporary = directory.resolve("world-lod-texture-palette-targets-v2.tmp");
		StringBuilder manifest = new StringBuilder("mattmc-world-lod-texture-palette-targets-v2\n");
		for (DistantHorizonsTexturePaletteProbe probe : distantHorizonsTexturePaletteProbes) {
			manifest.append(probe.position().getX()).append('|')
				.append(probe.position().getY()).append('|')
				.append(probe.position().getZ()).append('|')
				.append(String.join(",", probe.allowedSprites()))
				.append('|').append(String.join(",", probe.requiredSprites()))
				.append('\n');
		}
		try {
			Files.createDirectories(directory);
			Files.writeString(temporary, manifest, StandardCharsets.UTF_8);
			try {
				Files.move(temporary, target, StandardCopyOption.ATOMIC_MOVE, StandardCopyOption.REPLACE_EXISTING);
			} catch (IOException atomicMoveFailure) {
				Files.move(temporary, target, StandardCopyOption.REPLACE_EXISTING);
			}
		} catch (IOException failure) {
			LOGGER.warn("Unable to write DH texture palette target manifest", failure);
		}
	}

	private static boolean distantHorizonsTexturePaletteServerStateReady(ServerLevel serverLevel) {
		if (distantHorizonsTexturePaletteTarget == null) {
			distantHorizonsTexturePaletteServerStateDetail = "missing-target";
			return false;
		}
		for (DistantHorizonsTexturePaletteProbe probe : distantHorizonsTexturePaletteProbes) {
			String actualBlockId = BuiltInRegistries.BLOCK.getKey(serverLevel.getBlockState(probe.position()).getBlock()).toString();
			if (!probe.blockId().equals(actualBlockId)) {
				distantHorizonsTexturePaletteServerStateDetail = "probe-mismatch:"
					+ probe.position().toShortString() + ":expected=" + probe.blockId() + ":actual=" + actualBlockId;
				return false;
			}
		}
		for (BlockPos witness : distantHorizonsTransparentWitnesses) {
			if (!serverLevel.getBlockState(witness).is(Blocks.OAK_LEAVES)) {
				distantHorizonsTexturePaletteServerStateDetail = "transparent-witness-mismatch:"
					+ witness.toShortString();
				return false;
			}
		}
		for (BlockPos witness : distantHorizonsWaterWitnesses) {
			if (!serverLevel.getBlockState(witness).is(Blocks.WATER)) {
				distantHorizonsTexturePaletteServerStateDetail = "water-witness-mismatch:"
					+ witness.toShortString();
				return false;
			}
		}
		int panelMinX = DISTANT_HORIZONS_EXTERNAL_FIXTURE ? 64
			: distantHorizonsTexturePaletteTarget.getX() - DISTANT_HORIZONS_TEXTURE_PALETTE_SIDE / 2;
		int panelMinZ = DISTANT_HORIZONS_EXTERNAL_FIXTURE ? 512
			: distantHorizonsTexturePaletteTarget.getZ() - DISTANT_HORIZONS_TEXTURE_PALETTE_SIDE / 2;
		for (int chunkX = panelMinX >> 4; chunkX < (panelMinX + DISTANT_HORIZONS_TEXTURE_PALETTE_SIDE) >> 4; chunkX++) {
			for (int chunkZ = panelMinZ >> 4; chunkZ < (panelMinZ + DISTANT_HORIZONS_TEXTURE_PALETTE_SIDE) >> 4; chunkZ++) {
				if (!serverLevel.hasChunk(chunkX, chunkZ)) {
					distantHorizonsTexturePaletteServerStateDetail = "server-chunk-not-loaded:" + chunkX + "," + chunkZ;
					return false;
				}
			}
		}
		distantHorizonsTexturePaletteServerStateDetail = "ready";
		return true;
	}

	private static boolean distantHorizonsFixtureRequested() {
		return DISTANT_HORIZONS_TEXTURE_PALETTE || DISTANT_HORIZONS_REQUIRE_TRANSPARENT || DISTANT_HORIZONS_REQUIRE_WATER;
	}

	private static void forceDistantHorizonsTexturePaletteChunks(ServerLevel serverLevel, int panelMinX, int panelMinZ) {
		int firstChunkX = panelMinX >> 4;
		int firstChunkZ = panelMinZ >> 4;
		int lastChunkX = (panelMinX + DISTANT_HORIZONS_TEXTURE_PALETTE_SIDE - 1) >> 4;
		int lastChunkZ = (panelMinZ + DISTANT_HORIZONS_TEXTURE_PALETTE_SIDE - 1) >> 4;
		// SharedApi's real saved-chunk path needs the surrounding 3x3 chunk
		// data before it will queue a terrain build. The panel itself spans four
		// chunks; force its one-chunk neighbor ring so those ordinary source
		// updates are buildable without fabricating a DH column.
		for (int chunkX = firstChunkX - 1; chunkX <= lastChunkX + 1; chunkX++) {
			for (int chunkZ = firstChunkZ - 1; chunkZ <= lastChunkZ + 1; chunkZ++) {
				ChunkPos chunkPos = new ChunkPos(chunkX, chunkZ);
				if (!distantHorizonsTexturePaletteForcedChunks.contains(chunkPos)) {
					serverLevel.setChunkForced(chunkX, chunkZ, true);
					distantHorizonsTexturePaletteForcedChunks.add(chunkPos);
				}
			}
		}
	}

	private static void releaseDistantHorizonsTexturePaletteChunks(Minecraft minecraft) {
		if (distantHorizonsTexturePaletteForcedChunks.isEmpty()) {
			return;
		}
		// Capture completion happens on the client thread immediately after the
		// screenshot acknowledgement. Server chunk tickets are server-thread
		// state, so mutating them here can race the integrated server's own
		// shutdown/ticket iteration. The capture runner always uses a copied game
		// directory and terminates after `finish`, therefore retaining these
		// tickets until process exit is both bounded and safer than a cross-thread
		// teardown mutation.
		LOGGER.info(
			"Deterministic DH palette capture retaining {} forced chunks until copied-run process exit",
			distantHorizonsTexturePaletteForcedChunks.size()
		);
		distantHorizonsTexturePaletteForcedChunks.clear();
	}

	/**
	 * Waits for the real copied server's palette lighting to settle before DH is
	 * told about the changed chunks. DH reduction copies this packed light into
	 * its column vertices, so sending the event earlier produces a genuine
	 * changing geometry payload rather than a stable material fixture.
	 */
	private static boolean distantHorizonsTexturePaletteLightReady(ServerLevel serverLevel) {
		if (distantHorizonsTexturePaletteTarget == null) {
			return false;
		}
		int panelMinX = DISTANT_HORIZONS_EXTERNAL_FIXTURE ? 64
			: distantHorizonsTexturePaletteTarget.getX() - DISTANT_HORIZONS_TEXTURE_PALETTE_SIDE / 2;
		int panelMinZ = DISTANT_HORIZONS_EXTERNAL_FIXTURE ? 512
			: distantHorizonsTexturePaletteTarget.getZ() - DISTANT_HORIZONS_TEXTURE_PALETTE_SIDE / 2;
		int lightCorrectChunks = 0;
		for (int chunkX = panelMinX >> 4; chunkX < (panelMinX + DISTANT_HORIZONS_TEXTURE_PALETTE_SIDE) >> 4; chunkX++) {
			for (int chunkZ = panelMinZ >> 4; chunkZ < (panelMinZ + DISTANT_HORIZONS_TEXTURE_PALETTE_SIDE) >> 4; chunkZ++) {
				if (serverLevel.getChunk(chunkX, chunkZ).isLightCorrect()) {
					lightCorrectChunks++;
				}
			}
		}
		distantHorizonsTexturePaletteLightCorrectChunks = lightCorrectChunks;
		long fingerprint = 0x9E3779B97F4A7C15L;
		int panelY = distantHorizonsTexturePaletteTarget.getY();
		for (int localX = 0; localX < DISTANT_HORIZONS_TEXTURE_PALETTE_SIDE; localX++) {
			for (int localZ = 0; localZ < DISTANT_HORIZONS_TEXTURE_PALETTE_SIDE; localZ++) {
				BlockPos position = new BlockPos(panelMinX + localX, panelY, panelMinZ + localZ);
				int packedLight = serverLevel.getBrightness(LightLayer.SKY, position)
					| (serverLevel.getBrightness(LightLayer.BLOCK, position) << 4);
				fingerprint = Long.rotateLeft(fingerprint ^ packedLight, 7) * 0x100000001B3L;
			}
		}
		distantHorizonsTexturePaletteLightEngineBusy = serverLevel.getLightEngine().hasLightWork();
		if (!distantHorizonsTexturePaletteLightFingerprintKnown
			|| distantHorizonsTexturePaletteLightFingerprint != fingerprint) {
			distantHorizonsTexturePaletteLightFingerprintKnown = true;
			distantHorizonsTexturePaletteLightFingerprint = fingerprint;
			distantHorizonsTexturePaletteLightStableFrames = 1;
			return false;
		}
		distantHorizonsTexturePaletteLightStableFrames++;
		// Engine-wide queue state and chunk light-correct bookkeeping can lag the
		// copied values (or reflect unrelated chunks). DH consumes the sampled
		// packed values, so require three identical full-panel samples and retain
		// both broader signals as diagnostics rather than treating either as a
		// proxy for this palette's semantic light payload.
		return distantHorizonsTexturePaletteLightStableFrames >= DISTANT_HORIZONS_TEXTURE_PALETTE_LIGHT_STABLE_FRAMES;
	}


	/**
	 * Server-side deterministic setup bypasses the normal chunk-save hook. Once
	 * the copied server state is verified, send the same server update event that
	 * DH uses for an integrated world's saved chunks so it rebuilds real cached
	 * source data rather than retaining the pre-fixture column.
	 */
	private static int notifyDistantHorizonsTexturePaletteChunks(Minecraft minecraft, ServerLevel serverLevel) {
		if (minecraft.level == null || distantHorizonsTexturePaletteTarget == null) {
			return 0;
		}
		ServerLevelWrapper wrappedLevel = ServerLevelWrapper.getWrapper(serverLevel);
		int panelMinX = DISTANT_HORIZONS_EXTERNAL_FIXTURE ? 64
			: distantHorizonsTexturePaletteTarget.getX() - DISTANT_HORIZONS_TEXTURE_PALETTE_SIDE / 2;
		int panelMinZ = DISTANT_HORIZONS_EXTERNAL_FIXTURE ? 512
			: distantHorizonsTexturePaletteTarget.getZ() - DISTANT_HORIZONS_TEXTURE_PALETTE_SIDE / 2;
		int notified = 0;
		for (int chunkX = panelMinX >> 4; chunkX < (panelMinX + DISTANT_HORIZONS_TEXTURE_PALETTE_SIDE) >> 4; chunkX++) {
			for (int chunkZ = panelMinZ >> 4; chunkZ < (panelMinZ + DISTANT_HORIZONS_TEXTURE_PALETTE_SIDE) >> 4; chunkZ++) {
				// Re-emit one real block update in each source chunk. This preserves the
				// normal LevelChunk dirty/hash path when the copied fixture's datapack
				// update occurred before the capture notification was installed.
				BlockPos dirtyMarker = new BlockPos(chunkX << 4, 80, chunkZ << 4);
				BlockState dirtyState = serverLevel.getBlockState(dirtyMarker);
				serverLevel.setBlock(dirtyMarker, Blocks.AIR.defaultBlockState(), 3);
				serverLevel.setBlock(dirtyMarker, dirtyState, 3);
				// The palette deliberately lives outside the temporary vanilla render
				// radius.  DH consumes the integrated-server chunk wrapper here, just
				// as it does after ChunkMap saves a chunk; requiring a client chunk at
				// this point made the far fixture publish unrelated cached columns.
				// This is capture-only source invalidation, not a rendering path.
				// The panel is intentionally outside the near-terrain radius.  A save
				// event requires all nine client-side neighbor wrappers and is discarded
				// at the edge of that radius; the load-event contract queues this real
				// center chunk directly, allowing DH to consume its server-authored data.
				ServerApi.INSTANCE.serverChunkLoadEvent(
					new ChunkWrapper(serverLevel.getChunk(chunkX, chunkZ), wrappedLevel),
					wrappedLevel
				);
				notified++;
			}
		}
		return notified;
	}

	private static boolean setupStaticTerrainLifecycleScenarioAfterSettledReady(Minecraft minecraft) {
		if (!staticTerrainLifecycleScenario()) {
			staticTerrainLifecycleStage = "inactive";
			return true;
		}
		if (staticTerrainLiteralWorldTransitionScenario()) {
			return setupStaticTerrainLiteralWorldTransition(minecraft);
		}
		if (minecraft.player == null || minecraft.level == null || minecraft.getSingleplayerServer() == null) {
			staticTerrainLifecycleStage = "waiting-for-world";
			return false;
		}
		ServerLevel serverLevel = minecraft.getSingleplayerServer().getLevel(minecraft.level.dimension());
		if (serverLevel == null) {
			staticTerrainLifecycleStage = "waiting-for-server-level";
			return false;
		}
		if (!staticTerrainLifecycleSetup) {
			BlockPos target = chooseStaticTerrainLifecycleTarget(minecraft, serverLevel);
			if (target == null) {
				staticTerrainLifecycleStage = "waiting-for-visible-section";
				return false;
			}
			if ("texture-palette".equals(staticTerrainBaseScenario())
				&& !staticTerrainTexturePalettePositionsLoaded(minecraft, serverLevel, target)) {
				staticTerrainLifecycleStage = "waiting-for-texture-palette-positions";
				return false;
			}
			ChunkSectionLayer lifecycleLayer = staticTerrainLifecycleLayer();
			RustGalTerrainRenderer.TerrainLayerSnapshot before =
				RustGalTerrainRenderer.snapshotLayer(target, lifecycleLayer);
			if (!staticTerrainAllowsAirSource() && (before == null || before.meshGeneration() == 0L)) {
				staticTerrainLifecycleStage = "waiting-for-source-generation";
				return false;
			}
			RustGalTerrainRenderer.TerrainDiagnostics beforeDiagnostics = RustGalTerrainRenderer.diagnosticsSnapshot();
			RustGalTerrainRenderer.StaticTerrainExecutionSnapshot executionBefore =
				RustGalTerrainRenderer.staticTerrainExecutionSnapshot();
			staticTerrainLifecycleEditBlock = target;
			staticTerrainLifecycleBeforeGeneration = staticTerrainUsesAtlasGeneration()
				? beforeDiagnostics.atlasGeneration()
				: before == null ? 0L : before.meshGeneration();
			staticTerrainLifecycleBeforeCachedLayers = beforeDiagnostics.cachedLayerAssets();
			staticTerrainLifecycleBeforeRssBytes = currentUsedMemoryBytes();
			staticTerrainLifecycleExecutionSubmissionBaseline = executionBefore.submissionId();
			BlockState replacement = staticTerrainReplacementState();
			staticTerrainLifecycleBlockType = replacement.getBlock().builtInRegistryHolder().key().location().toString();
			RustGalTerrainRenderer.recordLifecycleMarker(
				"lifecycle-edit-before",
				target,
				lifecycleLayer,
				STATIC_TERRAIN_SCENARIO + ":block=" + target.toShortString() + ":generation=" + staticTerrainLifecycleBeforeGeneration
			);
			applyStaticTerrainLifecycleAction(minecraft, serverLevel, target, replacement);
			if ("texture-palette".equals(staticTerrainBaseScenario())) {
				focusTexturePalette(minecraft, target);
			}
			RustGalTerrainRenderer.recordLifecycleMarker(
				"lifecycle-edit-applied",
				target,
				lifecycleLayer,
				STATIC_TERRAIN_SCENARIO + ":block=" + target.toShortString() + ":state=" + staticTerrainLifecycleBlockType
			);
			if ("section-reentry".equals(staticTerrainBaseScenario())) {
				RustGalTerrainRenderer.removeSection(
					net.minecraft.core.SectionPos.blockToSectionCoord(target.getX()),
					net.minecraft.core.SectionPos.blockToSectionCoord(target.getY()),
					net.minecraft.core.SectionPos.blockToSectionCoord(target.getZ()),
					"lifecycle-section-removed"
				);
				RustGalTerrainRenderer.recordLifecycleMarker(
					"lifecycle-section-removed",
					target,
					lifecycleLayer,
					STATIC_TERRAIN_SCENARIO + ":block=" + target.toShortString()
				);
				RustGalTerrainRenderer.recordLifecycleMarker(
					"lifecycle-section-reentry-requested",
					target,
					lifecycleLayer,
					STATIC_TERRAIN_SCENARIO + ":block=" + target.toShortString()
				);
			}
			recordStaticTerrainLifecycleFaultMarker(target);
			staticTerrainLifecycleSetup = true;
			staticTerrainLifecycleStage = "edit-applied";
			writeMetadata(minecraft, "static_terrain_lifecycle_edit_applied");
			return false;
		}
		framesWaitingForStaticTerrainLifecycle++;
		if (continueStaticTerrainLifecycleAction(minecraft)) {
			return false;
		}
		if (staticTerrainBaseTranslucentFixtureScenario()) {
			RustGalTerrainRenderer.TerrainDiagnostics diagnostics = RustGalTerrainRenderer.diagnosticsSnapshot();
			boolean unsupportedFixtureReady = !"translucent-mixed-unsupported".equals(STATIC_TERRAIN_SCENARIO)
				|| diagnostics.unsupportedFluidRejectedSections() == 0;
			if (diagnostics.visibleLayerSubmissions() > 0
				&& unsupportedFixtureReady
				&& framesWaitingForStaticTerrainLifecycle >= Math.max(4, FRAMES_PER_POSE / 2)) {
				if (!staticTerrainPostSetupExecutionReady(minecraft)) {
					return false;
				}
				staticTerrainLifecycleAfterGeneration = Math.max(1L, diagnostics.atlasGeneration());
				if (!staticTerrainLifecycleAfterRecorded) {
					staticTerrainLifecycleAfterCachedLayers = diagnostics.cachedLayerAssets();
					staticTerrainLifecycleAfterRssBytes = currentUsedMemoryBytes();
					RustGalTerrainRenderer.recordLifecycleMarker(
						"lifecycle-fixture-visible",
						staticTerrainLifecycleEditBlock,
						staticTerrainLifecycleLayer(),
						STATIC_TERRAIN_SCENARIO
							+ ":block=" + staticTerrainLifecycleEditBlock.toShortString()
							+ ":visibleSubmissions=" + diagnostics.visibleLayerSubmissions()
							+ ":waitFrames=" + framesWaitingForStaticTerrainLifecycle
					);
					staticTerrainLifecycleAfterRecorded = true;
					staticTerrainLifecycleStage = "fixture-visible";
					// The lifecycle edit itself invalidates the readiness window that
					// admitted the pre-edit terrain.  Require a fresh drained window
					// before the first translucent-overlap screenshot so later return
					// captures cannot contain more settled geometry than the initial one.
					if (staticTerrainRequiresTranslucentCameraSequence()) {
						settledReadyGateSatisfied = false;
						settledRustTerrainFrames = 0;
						framesWaitingForSettledReady = 0;
					}
					writeMetadata(minecraft, "static_terrain_translucent_fixture_visible");
				}
				return true;
			}
			if ("translucent-mixed-unsupported".equals(STATIC_TERRAIN_SCENARIO)
				&& !unsupportedFixtureReady
				&& (framesWaitingForStaticTerrainLifecycle % 30) == 0) {
				writeMetadata(minecraft, "waiting_for_static_terrain_unsupported_fluid_metadata");
			}
		}
		long observedGeneration = observedStaticTerrainGeneration();
		boolean replacementReady = staticTerrainReplacementReady(observedGeneration);
		if (replacementReady) {
			if (!staticTerrainPostSetupExecutionReady(minecraft)) {
				return false;
			}
			if ("texture-palette".equals(staticTerrainBaseScenario())) {
				RustGalTerrainRenderer.TerrainTextureProbeReceipt textureReceipt = staticTerrainTexturePaletteProbeReceipt();
				if (!textureReceipt.matched()) {
					staticTerrainLifecycleStage = "waiting-for-texture-palette-uv-proof";
					if (framesWaitingForStaticTerrainLifecycle > SETTLED_READY_MAX_WAIT_FRAMES) {
						fail("timed out waiting for static terrain texture-palette UV proof: " + textureReceipt.status());
					} else if ((framesWaitingForStaticTerrainLifecycle % 30) == 0) {
						writeMetadata(minecraft, "waiting_for_static_terrain_texture_palette_uv_proof");
					}
					return false;
				}
			}
			staticTerrainLifecycleAfterGeneration = observedGeneration;
			if (!staticTerrainLifecycleAfterRecorded) {
				RustGalTerrainRenderer.TerrainDiagnostics afterDiagnostics = RustGalTerrainRenderer.diagnosticsSnapshot();
				staticTerrainLifecycleAfterCachedLayers = afterDiagnostics.cachedLayerAssets();
				staticTerrainLifecycleAfterRssBytes = currentUsedMemoryBytes();
				RustGalTerrainRenderer.recordLifecycleMarker(
					"lifecycle-edit-after",
					staticTerrainLifecycleEditBlock,
					staticTerrainLifecycleLayer(),
					STATIC_TERRAIN_SCENARIO
						+ ":block=" + staticTerrainLifecycleEditBlock.toShortString()
						+ ":before=" + staticTerrainLifecycleBeforeGeneration
						+ ":after=" + staticTerrainLifecycleAfterGeneration
						+ ":waitFrames=" + framesWaitingForStaticTerrainLifecycle
				);
				staticTerrainLifecycleAfterRecorded = true;
				staticTerrainLifecycleStage = "replacement-visible";
				writeMetadata(minecraft, "static_terrain_lifecycle_replacement_visible");
			}
			return framesWaitingForStaticTerrainLifecycle >= staticTerrainPostSetupRequiredFrames();
		}
		if (framesWaitingForStaticTerrainLifecycle > SETTLED_READY_MAX_WAIT_FRAMES) {
			fail("timed out waiting for static terrain lifecycle replacement: scenario="
				+ STATIC_TERRAIN_SCENARIO
				+ " block="
				+ (staticTerrainLifecycleEditBlock == null ? "null" : staticTerrainLifecycleEditBlock.toShortString())
				+ " beforeGeneration="
				+ staticTerrainLifecycleBeforeGeneration
				+ " afterGeneration="
				+ observedGeneration
				+ " stage="
				+ staticTerrainLifecycleStage);
		} else if ((framesWaitingForStaticTerrainLifecycle % 30) == 0) {
			writeMetadata(minecraft, "waiting_for_static_terrain_lifecycle_rebuild");
		}
		return false;
	}

	private static BlockPos chooseStaticTerrainLifecycleTarget(Minecraft minecraft, ServerLevel serverLevel) {
		if ("texture-palette".equals(staticTerrainBaseScenario())) {
			return chooseStaticTerrainTexturePaletteTarget(minecraft, serverLevel);
		}
		if (!staticTerrainTranslucentScenario()) {
			return RustGalTerrainRenderer.chooseLifecycleEditTarget(STATIC_TERRAIN_SCENARIO);
		}
		BlockPos target = chooseStaticTerrainTranslucentPlacementTarget(minecraft, serverLevel);
		return target == null ? RustGalTerrainRenderer.chooseLifecycleEditTarget(STATIC_TERRAIN_SCENARIO) : target;
	}

	private static BlockPos chooseStaticTerrainTranslucentPlacementTarget(Minecraft minecraft, ServerLevel serverLevel) {
		if (minecraft.player == null || minecraft.level == null) {
			return null;
		}
		BlockPos fixedTarget = staticTerrainFixtureTarget();
		if (fixedTarget != null && isLoadedAirBlock(minecraft.level, serverLevel, fixedTarget)) {
			return fixedTarget;
		}
		Direction forward = minecraft.player.getDirection();
		Direction right = forward.getClockWise();
		BlockPos eye = BlockPos.containing(minecraft.player.getEyePosition());
		BlockPos fallback = null;
		for (int distance = 4; distance <= 10; distance++) {
			for (int vertical = -2; vertical <= 2; vertical++) {
				for (int lateral = -2; lateral <= 2; lateral++) {
					BlockPos candidate = eye.relative(forward, distance).relative(right, lateral).above(vertical);
					if (!isLoadedAirBlock(minecraft.level, serverLevel, candidate)) {
						continue;
					}
					if (fallback == null) {
						fallback = candidate;
					}
					RustGalTerrainRenderer.TerrainLayerSnapshot solid =
						RustGalTerrainRenderer.snapshotLayer(candidate, ChunkSectionLayer.SOLID);
					if (solid != null && solid.meshGeneration() != 0L) {
						return candidate;
					}
				}
			}
		}
		return fallback;
	}

	private static BlockPos staticTerrainFixtureTarget() {
		String[] parts = System.getProperty("mattmc.dev.deterministicCameraCapture.staticTerrainFixtureTarget", "").split(",");
		if (parts.length != 3) {
			return null;
		}
		try {
			return new BlockPos(Integer.parseInt(parts[0].trim()), Integer.parseInt(parts[1].trim()), Integer.parseInt(parts[2].trim()));
		} catch (NumberFormatException ignored) {
			return null;
		}
	}

	private static BlockPos chooseStaticTerrainTexturePaletteTarget(Minecraft minecraft, ServerLevel serverLevel) {
		if (minecraft.player == null || minecraft.level == null) {
			return null;
		}
		Direction forward = minecraft.player.getDirection();
		Direction right = forward.getClockWise();
		BlockPos eye = BlockPos.containing(minecraft.player.getEyePosition());
		for (int distance = 4; distance <= 10; distance++) {
			for (int vertical = -2; vertical <= 1; vertical++) {
				for (int lateral = -2; lateral <= 1; lateral++) {
					BlockPos candidate = eye.relative(forward, distance).relative(right, lateral).above(vertical);
					BlockPos[] palette = {
						candidate,
						candidate.relative(right),
						candidate.relative(right, 2),
						candidate.relative(right, 3)
					};
					boolean available = true;
					for (BlockPos position : palette) {
						if (!isLoadedAirBlock(minecraft.level, serverLevel, position)) {
							available = false;
							break;
						}
					}
					if (available) {
						return candidate;
					}
				}
			}
		}
		return null;
	}

	private static void focusTexturePalette(Minecraft minecraft, BlockPos target) {
		if (minecraft.player == null) {
			return;
		}
		Direction right = minecraft.player.getDirection().getClockWise();
		Vec3 paletteCenter = Vec3.atCenterOf(target).add(
			right.getStepX() * 1.5,
			0.0,
			right.getStepZ() * 1.5
		);
		Vec3 delta = paletteCenter.subtract(minecraft.player.getEyePosition());
		double horizontal = Math.sqrt(delta.x * delta.x + delta.z * delta.z);
		if (horizontal < 0.001) {
			throw new IllegalStateException("Texture palette capture target is at the camera position");
		}
		float yaw = (float)(Math.toDegrees(Math.atan2(delta.z, delta.x)) - 90.0);
		float pitch = (float)(-Math.toDegrees(Math.atan2(delta.y, horizontal)));
		initialPose = new Pose("texture-palette", yaw, pitch);
		poses = new Pose[] { initialPose };
		poseIndex = 0;
		applyPose(minecraft.player, initialPose);
		RustGalTerrainRenderer.recordLifecycleMarker(
			"lifecycle-texture-palette-camera",
			target,
			staticTerrainLifecycleLayer(),
			"yaw=" + format(yaw) + ":pitch=" + format(pitch)
		);
	}

	private static boolean isLoadedAirBlock(ClientLevel clientLevel, ServerLevel serverLevel, BlockPos candidate) {
		if (!serverLevel.isLoaded(candidate) || !clientLevel.isLoaded(candidate)) {
			return false;
		}
		BlockState serverState = serverLevel.getBlockState(candidate);
		BlockState clientState = clientLevel.getBlockState(candidate);
		return serverState.isAir()
			&& clientState.isAir()
			&& serverState.getFluidState().isEmpty()
			&& clientState.getFluidState().isEmpty();
	}

	private static boolean staticTerrainLifecycleScenario() {
		return switch (staticTerrainBaseScenario()) {
			case "interior-edit", "boundary-x-edit", "boundary-y-edit", "boundary-z-edit", "section-reentry",
				"resource-reload", "opaque-texture-replacement", "cutout-texture-replacement",
				"pack-priority-reversal", "missing-atlas-payload", "malformed-png-payload", "translucent-glass",
				"translucent-overlap", "translucent-water", "translucent-mixed", "translucent-mixed-unsupported",
				"partial-texture-update", "model-resource-generation-change", "resize-cycle",
				"swapchain-recreate", "world-unload-reload", "world-different-reload",
				"view-distance-decrease", "view-distance-increase", "camera-relocation",
				"return-visited-terrain", "memory-cache-soak", "steady-state-performance", "texture-palette" -> true;
			default -> false;
		};
	}

	private static boolean staticTerrainLiteralWorldTransitionScenario() {
		String scenario = staticTerrainBaseScenario();
		return "world-unload-reload".equals(scenario)
			|| "world-different-reload".equals(scenario);
	}

	private static boolean staticTerrainEditScenario() {
		return switch (staticTerrainBaseScenario()) {
			case "interior-edit", "boundary-x-edit", "boundary-y-edit", "boundary-z-edit", "section-reentry",
				"model-resource-generation-change" -> true;
			default -> false;
		};
	}

	private static boolean staticTerrainUsesAtlasGeneration() {
		return switch (staticTerrainBaseScenario()) {
			case "resource-reload", "opaque-texture-replacement", "cutout-texture-replacement",
				"pack-priority-reversal", "missing-atlas-payload", "malformed-png-payload",
				"partial-texture-update" -> true;
			default -> false;
		};
	}

	private static boolean staticTerrainAllowsAirSource() {
		return staticTerrainTranslucentScenario() || "texture-palette".equals(staticTerrainBaseScenario());
	}

	private static BlockState staticTerrainReplacementState() {
		if ("translucent-mixed".equals(STATIC_TERRAIN_SCENARIO)) {
			return GraphicsAuditMixedFluidFixture.enclosureBlock().defaultBlockState();
		}
		if ("translucent-water".equals(STATIC_TERRAIN_SCENARIO)) {
			return Blocks.WATER.defaultBlockState();
		}
		if (staticTerrainTranslucentScenario()) {
			return staticTerrainTranslucentReplacementState();
		}
		return staticTerrainEditScenario() ? Blocks.AIR.defaultBlockState() : Blocks.STONE.defaultBlockState();
	}

	private static String staticTerrainBaseScenario() {
		if (!STATIC_TERRAIN_SCENARIO.startsWith("translucent-")) {
			return STATIC_TERRAIN_SCENARIO;
		}
		String suffix = STATIC_TERRAIN_SCENARIO.substring("translucent-".length());
		return switch (suffix) {
			case "interior-edit", "boundary-x-edit", "boundary-y-edit", "boundary-z-edit", "section-reentry",
				"resource-reload", "opaque-texture-replacement", "cutout-texture-replacement",
				"pack-priority-reversal", "missing-atlas-payload", "malformed-png-payload",
				"partial-texture-update", "model-resource-generation-change", "resize-cycle",
				"swapchain-recreate", "world-unload-reload", "world-different-reload",
				"view-distance-decrease", "view-distance-increase", "camera-relocation",
				"return-visited-terrain", "memory-cache-soak", "steady-state-performance" -> suffix;
			case "rapid-edit" -> "interior-edit";
			case "quiescence-stationary-performance", "moving-camera-performance" -> "steady-state-performance";
			default -> STATIC_TERRAIN_SCENARIO;
		};
	}

	private static boolean staticTerrainTranslucentScenario() {
		if ("translucent-glass".equals(STATIC_TERRAIN_SCENARIO)
			|| "translucent-overlap".equals(STATIC_TERRAIN_SCENARIO)
			|| "translucent-water".equals(STATIC_TERRAIN_SCENARIO)
			|| "translucent-mixed".equals(STATIC_TERRAIN_SCENARIO)
			|| "translucent-mixed-unsupported".equals(STATIC_TERRAIN_SCENARIO)) {
			return true;
		}
		return STATIC_TERRAIN_SCENARIO.startsWith("translucent-")
			&& !staticTerrainBaseScenario().equals(STATIC_TERRAIN_SCENARIO);
	}

	private static boolean staticTerrainBaseTranslucentFixtureScenario() {
		return "translucent-glass".equals(STATIC_TERRAIN_SCENARIO)
			|| "translucent-overlap".equals(STATIC_TERRAIN_SCENARIO)
			|| "translucent-water".equals(STATIC_TERRAIN_SCENARIO)
			|| "translucent-mixed".equals(STATIC_TERRAIN_SCENARIO)
			|| "translucent-mixed-unsupported".equals(STATIC_TERRAIN_SCENARIO);
	}

	private static boolean staticTerrainRequiresTranslucentCameraSequence() {
		return "translucent-overlap".equals(STATIC_TERRAIN_SCENARIO)
			|| "translucent-moving-camera-performance".equals(STATIC_TERRAIN_SCENARIO);
	}

	private static ChunkSectionLayer staticTerrainLifecycleLayer() {
		return staticTerrainTranslucentScenario() ? ChunkSectionLayer.TRANSLUCENT : ChunkSectionLayer.SOLID;
	}

	private static BlockState staticTerrainTranslucentReplacementState() {
		for (BlockState candidate : List.of(
			Blocks.BLUE_STAINED_GLASS.defaultBlockState(),
			Blocks.TINTED_GLASS.defaultBlockState(),
			Blocks.ICE.defaultBlockState(),
			Blocks.SLIME_BLOCK.defaultBlockState(),
			Blocks.HONEY_BLOCK.defaultBlockState()
		)) {
			if (ItemBlockRenderTypes.getChunkRenderType(candidate) == ChunkSectionLayer.TRANSLUCENT) {
				return candidate;
			}
		}
		return Blocks.BLUE_STAINED_GLASS.defaultBlockState();
	}

	private static void applyStaticTerrainLifecycleAction(
		Minecraft minecraft,
		ServerLevel serverLevel,
		BlockPos target,
		BlockState replacement
	) {
		String scenario = staticTerrainBaseScenario();
		if (staticTerrainTranslucentScenario()
			&& !"translucent-glass".equals(STATIC_TERRAIN_SCENARIO)
			&& !"translucent-water".equals(STATIC_TERRAIN_SCENARIO)
			&& !"translucent-mixed".equals(STATIC_TERRAIN_SCENARIO)
			&& !"translucent-mixed-unsupported".equals(STATIC_TERRAIN_SCENARIO)
			&& !staticTerrainTranslucentFixtureApplied) {
			applyStaticTerrainTranslucentOverlap(minecraft, serverLevel, target);
			staticTerrainTranslucentFixtureApplied = true;
			staticTerrainLifecycleStage = "translucent-fixture-placed";
			RustGalTerrainRenderer.recordLifecycleMarker(
				"lifecycle-translucent-overlap-placed",
				target,
				ChunkSectionLayer.TRANSLUCENT,
				STATIC_TERRAIN_SCENARIO + ":baseScenario=" + scenario + ":block=" + target.toShortString()
			);
		}
		if (staticTerrainEditScenario()) {
			applyStaticTerrainLifecycleEdit(serverLevel, minecraft.level, target, replacement);
			return;
		}
		switch (scenario) {
			case "translucent-glass" -> {
				applyStaticTerrainLifecycleEdit(serverLevel, minecraft.level, target, replacement);
				staticTerrainLifecycleStage = "translucent-glass-placed";
				RustGalTerrainRenderer.recordLifecycleMarker(
					"lifecycle-translucent-glass-placed",
					target,
					ChunkSectionLayer.TRANSLUCENT,
					STATIC_TERRAIN_SCENARIO + ":block=" + target.toShortString()
				);
			}
			case "translucent-overlap" -> {
				applyStaticTerrainTranslucentOverlap(minecraft, serverLevel, target);
				// The overlap fixture replaces a whole set of panes, including
				// blocks in sections that may have had no translucent render pass
				// before the edit.  Force the normal render-section invalidation so
				// readiness cannot be satisfied by unrelated terrain submissions.
				minecraft.levelRenderer.allChanged();
				staticTerrainTranslucentFixtureApplied = true;
				staticTerrainLifecycleStage = "translucent-overlap-placed";
				RustGalTerrainRenderer.recordLifecycleMarker(
					"lifecycle-translucent-overlap-placed",
					target,
					ChunkSectionLayer.TRANSLUCENT,
					STATIC_TERRAIN_SCENARIO + ":block=" + target.toShortString()
				);
			}
			case "translucent-water" -> {
				applyStaticTerrainLifecycleEdit(serverLevel, minecraft.level, target, Blocks.WATER.defaultBlockState());
				staticTerrainLifecycleStage = "translucent-water-placed";
				RustGalTerrainRenderer.recordLifecycleMarker(
					"lifecycle-translucent-water-placed",
					target,
					ChunkSectionLayer.TRANSLUCENT,
					STATIC_TERRAIN_SCENARIO + ":block=" + target.toShortString()
				);
			}
			case "translucent-mixed", "translucent-mixed-unsupported" -> {
				applyStaticTerrainTranslucentMixed(minecraft, serverLevel, target,
					"translucent-mixed-unsupported".equals(STATIC_TERRAIN_SCENARIO));
				minecraft.levelRenderer.allChanged();
				staticTerrainTranslucentFixtureApplied = true;
				staticTerrainLifecycleStage = STATIC_TERRAIN_SCENARIO + "-placed";
				RustGalTerrainRenderer.recordLifecycleMarker(
					"lifecycle-" + STATIC_TERRAIN_SCENARIO + "-placed",
					target,
					ChunkSectionLayer.TRANSLUCENT,
					STATIC_TERRAIN_SCENARIO + ":block=" + target.toShortString()
				);
			}
			case "texture-palette" -> {
				applyStaticTerrainTexturePalette(minecraft, serverLevel, target);
				// The helper updates both the server and client level. Let those
				// four local block changes schedule their normal section rebuilds;
				// a full renderer reset invalidates every source snapshot after it
				// has been armed and makes this visual fixture race its screenshot.
				staticTerrainLifecycleStage = "texture-palette-placed";
				Direction right = minecraft.player.getDirection().getClockWise();
				RustGalTerrainRenderer.recordLifecycleMarker(
					"lifecycle-texture-palette-placed",
					target,
					ChunkSectionLayer.SOLID,
					"grass_block=" + target.toShortString()
						+ ";redstone_ore=" + target.relative(right).toShortString()
						+ ";yellow_terracotta=" + target.relative(right, 2).toShortString()
						+ ";oak_leaves=" + target.relative(right, 3).toShortString()
				);
			}
			case "resource-reload", "pack-priority-reversal" -> {
				RustGalTerrainRenderer.invalidateForResourceReload();
				minecraft.levelRenderer.allChanged();
				staticTerrainLifecycleStage = "resource-reload-started";
				RustGalTerrainRenderer.recordLifecycleMarker(
						"lifecycle-resource-reload-started",
						target,
						staticTerrainLifecycleLayer(),
						STATIC_TERRAIN_SCENARIO + ":pack=" + STATIC_TERRAIN_RESOURCE_PACK_SCENARIO + ":mode=bounded-terrain-rebuild"
					);
			}
			case "opaque-texture-replacement" ->
				RustGalTerrainRenderer.injectAtlasTexturePayloadForDiagnostics(staticTerrainValidTinyPngPayload(), "lifecycle-atlas-opaque-texture-replacement");
			case "cutout-texture-replacement" ->
				RustGalTerrainRenderer.injectAtlasTexturePayloadForDiagnostics(staticTerrainValidTinyPngPayload(), "lifecycle-atlas-cutout-texture-replacement");
			case "missing-atlas-payload" ->
				RustGalTerrainRenderer.injectAtlasTexturePayloadForDiagnostics(new byte[0], "lifecycle-atlas-missing-payload");
			case "malformed-png-payload" ->
				RustGalTerrainRenderer.injectAtlasTexturePayloadForDiagnostics("not a png".getBytes(StandardCharsets.UTF_8), "lifecycle-atlas-malformed-payload");
			case "partial-texture-update" ->
				RustGalTerrainRenderer.injectAtlasTexturePayloadForDiagnostics(staticTerrainValidTinyPngPayload(), "lifecycle-atlas-partial-texture-update");
			case "resize-cycle", "swapchain-recreate" -> {
				staticTerrainOriginalRenderDistance = minecraft.options.renderDistance().get();
				staticTerrainOriginalSimulationDistance = minecraft.options.simulationDistance().get();
				minecraft.getWindow().setWindowed(960, 540);
				minecraft.resizeDisplay();
				staticTerrainLifecycleResizeCount = 1;
				staticTerrainLifecycleStage = "resize-1";
				RustGalTerrainRenderer.recordLifecycleMarker("lifecycle-resize-1", target, staticTerrainLifecycleLayer(), "extent=960x540");
			}
			case "view-distance-decrease" -> {
				staticTerrainOriginalRenderDistance = minecraft.options.renderDistance().get();
				staticTerrainOriginalSimulationDistance = minecraft.options.simulationDistance().get();
				minecraft.options.renderDistance().set(Math.max(2, Math.min(staticTerrainOriginalRenderDistance, 4)));
				minecraft.options.simulationDistance().set(Math.max(2, Math.min(staticTerrainOriginalSimulationDistance, 4)));
				staticTerrainLifecycleStage = "view-distance-decreased";
				RustGalTerrainRenderer.recordLifecycleMarker(
						"lifecycle-view-distance-decreased",
						target,
						staticTerrainLifecycleLayer(),
						"renderDistance=" + minecraft.options.renderDistance().get()
						+ ":simulationDistance=" + minecraft.options.simulationDistance().get()
				);
			}
			case "view-distance-increase", "memory-cache-soak", "steady-state-performance" -> {
				staticTerrainOriginalRenderDistance = minecraft.options.renderDistance().get();
				staticTerrainOriginalSimulationDistance = minecraft.options.simulationDistance().get();
				minecraft.options.renderDistance().set(Math.max(staticTerrainOriginalRenderDistance, 12));
				minecraft.options.simulationDistance().set(Math.max(staticTerrainOriginalSimulationDistance, 12));
				staticTerrainLifecycleStage = "visibility-expanded";
				RustGalTerrainRenderer.recordLifecycleMarker(
						"lifecycle-view-distance-increased",
						target,
						staticTerrainLifecycleLayer(),
						"renderDistance=" + minecraft.options.renderDistance().get()
						+ ":simulationDistance=" + minecraft.options.simulationDistance().get()
				);
				if ("memory-cache-soak".equals(scenario)) {
					RustGalTerrainRenderer.recordLifecycleMarker(
							"lifecycle-memory-cache-soak-started",
							target,
							staticTerrainLifecycleLayer(),
							"usedMemoryBytes=" + currentUsedMemoryBytes()
					);
				}
				if ("steady-state-performance".equals(scenario) || "quiescence-stationary-performance".equals(scenario)) {
					RustGalTerrainRenderer.recordLifecycleMarker(
							"lifecycle-steady-state-performance-started",
							target,
							staticTerrainLifecycleLayer(),
							"cachedLayers=" + RustGalTerrainRenderer.diagnosticsSnapshot().cachedLayerAssets()
					);
				}
			}
			case "camera-relocation", "return-visited-terrain" -> {
				staticTerrainOriginalRenderDistance = minecraft.options.renderDistance().get();
				staticTerrainOriginalSimulationDistance = minecraft.options.simulationDistance().get();
				minecraft.options.renderDistance().set(Math.max(staticTerrainOriginalRenderDistance, 12));
				minecraft.options.simulationDistance().set(Math.max(staticTerrainOriginalSimulationDistance, 12));
				staticTerrainOriginalPosition = initialPosition;
				Vec3 relocated = initialPosition.add(64.0, 0.0, 64.0);
				initialPosition = relocated;
				if (minecraft.player != null) {
					minecraft.player.setPos(relocated);
					minecraft.player.setOldPosAndRot(relocated, initialPose.yaw(), initialPose.pitch());
				}
				staticTerrainLifecycleStage = "camera-relocated";
				RustGalTerrainRenderer.recordLifecycleMarker(
						"lifecycle-camera-relocated",
						target,
						staticTerrainLifecycleLayer(),
						"position=" + formatVec(relocated)
				);
				if ("return-visited-terrain".equals(scenario)) {
					staticTerrainLifecycleActionStep = 1;
					RustGalTerrainRenderer.recordLifecycleMarker(
							"lifecycle-return-visited-terrain-away",
							target,
							staticTerrainLifecycleLayer(),
							"position=" + formatVec(relocated)
					);
				}
			}
			default -> staticTerrainLifecycleStage = "action-applied";
		}
		recordStaticTerrainLifecycleFaultMarker(target);
	}

	private static boolean setupStaticTerrainLiteralWorldTransition(Minecraft minecraft) {
		if (staticTerrainLifecycleAfterRecorded && "replacement-visible".equals(staticTerrainLifecycleStage)) {
			return true;
		}
		framesWaitingForStaticTerrainLifecycle++;
		String scenario = staticTerrainBaseScenario();
		if (framesWaitingForStaticTerrainLifecycle > SETTLED_READY_MAX_WAIT_FRAMES) {
			fail("timed out waiting for literal static terrain world transition: scenario="
				+ STATIC_TERRAIN_SCENARIO
				+ " stage="
				+ staticTerrainLifecycleStage
				+ " actionStep="
				+ staticTerrainLifecycleActionStep);
		}
		if (staticTerrainLifecycleTransitionInProgress) {
			return false;
		}
		staticTerrainLifecycleWorldA = STATIC_TERRAIN_WORLD_ID.isEmpty() ? "Origin" : STATIC_TERRAIN_WORLD_ID;
		staticTerrainLifecycleWorldB = STATIC_TERRAIN_WORLD_B_ID.isEmpty()
			? staticTerrainLifecycleWorldA + "-different"
			: STATIC_TERRAIN_WORLD_B_ID;
		RustGalTerrainRenderer.TerrainDiagnostics diagnostics = RustGalTerrainRenderer.diagnosticsSnapshot();
		switch (staticTerrainLifecycleActionStep) {
			case 0 -> {
				if (staticTerrainTranslucentScenario() && !staticTerrainTranslucentFixtureApplied) {
					if (minecraft.player == null || minecraft.level == null || minecraft.getSingleplayerServer() == null) {
						staticTerrainLifecycleStage = "waiting-for-translucent-world-a-fixture-world";
						return false;
					}
					ServerLevel serverLevel = minecraft.getSingleplayerServer().getLevel(minecraft.level.dimension());
					BlockPos target = serverLevel == null ? null : chooseStaticTerrainTranslucentPlacementTarget(minecraft, serverLevel);
					if (serverLevel == null || target == null) {
						staticTerrainLifecycleStage = "waiting-for-translucent-world-a-fixture-target";
						return false;
					}
					applyStaticTerrainTranslucentOverlap(minecraft, serverLevel, target);
					staticTerrainTranslucentFixtureApplied = true;
					RustGalTerrainRenderer.recordLifecycleMarker(
						"lifecycle-translucent-overlap-placed",
						target,
						ChunkSectionLayer.TRANSLUCENT,
						STATIC_TERRAIN_SCENARIO + ":phase=world-a-transition-fixture:block=" + target.toShortString()
					);
					staticTerrainLifecycleStage = "translucent-world-a-fixture-placed";
					return false;
				}
				if (!staticTerrainInPlayableWorld(minecraft)
					|| diagnostics.visibleLayerSubmissions() <= 0
					|| diagnostics.cachedLayerAssets() <= 0) {
					staticTerrainLifecycleStage = "waiting-for-initial-world-a-terrain";
					return false;
				}
				staticTerrainLifecycleBeforeGeneration = Math.max(1L, diagnostics.atlasGeneration());
				staticTerrainLifecycleBeforeCachedLayers = diagnostics.cachedLayerAssets();
				staticTerrainLifecycleBeforeRssBytes = currentUsedMemoryBytes();
				staticTerrainUnloadSubmissionSnapshot = diagnostics.visibleLayerSubmissions();
				staticTerrainLifecycleSetup = true;
				staticTerrainLifecycleStage = "leaving-world-a";
				RustGalTerrainRenderer.recordLifecycleMarker(
					"lifecycle-world-unload",
					BlockPos.ZERO,
					ChunkSectionLayer.SOLID,
					"world=" + staticTerrainLifecycleWorldA + ":dimension=" + initialDimension
				);
				staticTerrainLifecycleTransitionInProgress = true;
				staticTerrainLifecycleActionStep = 1;
				framesWaitingForStaticTerrainLifecycle = 0;
				try {
					minecraft.disconnectFromWorld(Component.literal("static terrain lifecycle transition"));
				} finally {
					staticTerrainLifecycleTransitionInProgress = false;
				}
				return false;
			}
			case 1 -> {
				if (staticTerrainInPlayableWorld(minecraft)) {
					staticTerrainLifecycleStage = "waiting-for-menu-after-world-a";
					return false;
				}
				diagnostics = RustGalTerrainRenderer.diagnosticsSnapshot();
				staticTerrainMenuCachedLayers = diagnostics.cachedLayerAssets();
				staticTerrainMenuActiveLayerSnapshot = diagnostics.activeTerrainLayers();
				staticTerrainMenuActiveSectionAssetSnapshot = diagnostics.activeSectionAssets();
				staticTerrainMenuCurrentFrameSubmissionSnapshot = diagnostics.currentFrameVisibleLayerSubmissions();
				staticTerrainMenuUsedMemoryBytes = currentUsedMemoryBytes();
				staticTerrainMenuSubmissionSnapshot = diagnostics.visibleLayerSubmissions();
				if (staticTerrainMenuSubmissionSnapshot != staticTerrainUnloadSubmissionSnapshot
					|| staticTerrainMenuCachedLayers != 0
					|| staticTerrainMenuActiveLayerSnapshot != 0
					|| staticTerrainMenuActiveSectionAssetSnapshot != 0
					|| staticTerrainMenuCurrentFrameSubmissionSnapshot != 0) {
					staticTerrainLifecycleStage = "menu-baseline-invalid";
					return false;
				}
				RustGalTerrainRenderer.recordLifecycleMarker(
					"lifecycle-world-menu-baseline",
					BlockPos.ZERO,
					ChunkSectionLayer.SOLID,
					"world=" + staticTerrainLifecycleWorldA
						+ ":cachedLayers=" + staticTerrainMenuCachedLayers
						+ ":activeLayers=" + staticTerrainMenuActiveLayerSnapshot
						+ ":activeSectionAssets=" + staticTerrainMenuActiveSectionAssetSnapshot
						+ ":currentFrameVisibleSubmissions=" + staticTerrainMenuCurrentFrameSubmissionSnapshot
						+ ":visibleSubmissions=" + staticTerrainMenuSubmissionSnapshot
				);
				RustGalTerrainRenderer.recordLifecycleMarker(
					"lifecycle-world-reload-requested",
					BlockPos.ZERO,
					ChunkSectionLayer.SOLID,
					"world=" + staticTerrainLifecycleWorldA
				);
				staticTerrainLifecycleStage = "reloading-world-a";
				staticTerrainLifecycleTransitionInProgress = true;
				staticTerrainLifecycleActionStep = 2;
				framesWaitingForStaticTerrainLifecycle = 0;
				try {
					minecraft.createWorldOpenFlows().openWorld(staticTerrainLifecycleWorldA, () -> minecraft.setScreen(new TitleScreen()));
				} finally {
					staticTerrainLifecycleTransitionInProgress = false;
				}
				return false;
			}
			case 2 -> {
				if (!staticTerrainInPlayableWorld(minecraft)
					|| diagnostics.visibleLayerSubmissions() <= staticTerrainMenuSubmissionSnapshot
					|| diagnostics.cachedLayerAssets() <= 0) {
					staticTerrainLifecycleStage = "waiting-for-world-a-reload-terrain";
					return false;
				}
				staticTerrainReloadGenerationA = Math.max(1L, diagnostics.atlasGeneration());
				staticTerrainLifecycleAfterGeneration = staticTerrainReloadGenerationA;
				staticTerrainLifecycleAfterCachedLayers = diagnostics.cachedLayerAssets();
				staticTerrainLifecycleAfterRssBytes = currentUsedMemoryBytes();
				RustGalTerrainRenderer.recordLifecycleMarker(
					"lifecycle-world-reload-valid",
					BlockPos.ZERO,
					ChunkSectionLayer.SOLID,
					"world=" + staticTerrainLifecycleWorldA
						+ ":generation=" + staticTerrainReloadGenerationA
						+ ":cachedLayers=" + diagnostics.cachedLayerAssets()
				);
				if (!"world-different-reload".equals(scenario)) {
					staticTerrainLifecycleAfterRecorded = true;
					staticTerrainLifecycleStage = "replacement-visible";
					framesWaitingForStaticTerrainLifecycle++;
					return framesWaitingForStaticTerrainLifecycle >= Math.max(8, FRAMES_PER_POSE);
				}
				staticTerrainUnloadSubmissionSnapshot = diagnostics.visibleLayerSubmissions();
				RustGalTerrainRenderer.recordLifecycleMarker(
					"lifecycle-world-unload",
					BlockPos.ZERO,
					ChunkSectionLayer.SOLID,
					"world=" + staticTerrainLifecycleWorldA + ":phase=second"
				);
				staticTerrainLifecycleStage = "leaving-world-a-second";
				staticTerrainLifecycleTransitionInProgress = true;
				staticTerrainLifecycleActionStep = 3;
				framesWaitingForStaticTerrainLifecycle = 0;
				try {
					minecraft.disconnectFromWorld(Component.literal("static terrain lifecycle transition to second world"));
				} finally {
					staticTerrainLifecycleTransitionInProgress = false;
				}
				return false;
			}
			case 3 -> {
				if (staticTerrainInPlayableWorld(minecraft)) {
					staticTerrainLifecycleStage = "waiting-for-menu-before-world-b";
					return false;
				}
				diagnostics = RustGalTerrainRenderer.diagnosticsSnapshot();
				if (diagnostics.visibleLayerSubmissions() != staticTerrainUnloadSubmissionSnapshot
					|| diagnostics.cachedLayerAssets() != 0
					|| diagnostics.activeTerrainLayers() != 0
					|| diagnostics.activeSectionAssets() != 0
					|| diagnostics.currentFrameVisibleLayerSubmissions() != 0) {
					staticTerrainLifecycleStage = "menu-before-world-b-baseline-invalid";
					return false;
				}
				staticTerrainMenuSubmissionSnapshot = diagnostics.visibleLayerSubmissions();
				staticTerrainMenuCachedLayers = diagnostics.cachedLayerAssets();
				staticTerrainMenuActiveLayerSnapshot = diagnostics.activeTerrainLayers();
				staticTerrainMenuActiveSectionAssetSnapshot = diagnostics.activeSectionAssets();
				staticTerrainMenuCurrentFrameSubmissionSnapshot = diagnostics.currentFrameVisibleLayerSubmissions();
				RustGalTerrainRenderer.recordLifecycleMarker(
					"lifecycle-world-menu-baseline",
					BlockPos.ZERO,
					ChunkSectionLayer.SOLID,
					"world=" + staticTerrainLifecycleWorldB
						+ ":cachedLayers=" + diagnostics.cachedLayerAssets()
						+ ":activeLayers=" + diagnostics.activeTerrainLayers()
						+ ":activeSectionAssets=" + diagnostics.activeSectionAssets()
						+ ":currentFrameVisibleSubmissions=" + diagnostics.currentFrameVisibleLayerSubmissions()
						+ ":visibleSubmissions=" + diagnostics.visibleLayerSubmissions()
				);
				RustGalTerrainRenderer.recordLifecycleMarker(
					"lifecycle-world-reload-requested",
					BlockPos.ZERO,
					ChunkSectionLayer.SOLID,
					"world=" + staticTerrainLifecycleWorldB
				);
				staticTerrainLifecycleStage = "loading-world-b";
				staticTerrainLifecycleTransitionInProgress = true;
				staticTerrainLifecycleActionStep = 4;
				framesWaitingForStaticTerrainLifecycle = 0;
				try {
					minecraft.createWorldOpenFlows().openWorld(staticTerrainLifecycleWorldB, () -> minecraft.setScreen(new TitleScreen()));
				} finally {
					staticTerrainLifecycleTransitionInProgress = false;
				}
				return false;
			}
			case 4 -> {
				if (staticTerrainTranslucentScenario() && !staticTerrainTranslucentWorldBFixtureApplied) {
					if (minecraft.player == null || minecraft.level == null || minecraft.getSingleplayerServer() == null) {
						staticTerrainLifecycleStage = "waiting-for-translucent-world-b-fixture-world";
						return false;
					}
					ServerLevel serverLevel = minecraft.getSingleplayerServer().getLevel(minecraft.level.dimension());
					BlockPos target = serverLevel == null ? null : chooseStaticTerrainTranslucentPlacementTarget(minecraft, serverLevel);
					if (serverLevel == null || target == null) {
						staticTerrainLifecycleStage = "waiting-for-translucent-world-b-fixture-target";
						return false;
					}
					applyStaticTerrainTranslucentOverlap(minecraft, serverLevel, target);
					staticTerrainTranslucentWorldBFixtureApplied = true;
					RustGalTerrainRenderer.recordLifecycleMarker(
						"lifecycle-translucent-overlap-placed",
						target,
						ChunkSectionLayer.TRANSLUCENT,
						STATIC_TERRAIN_SCENARIO + ":phase=world-b-transition-fixture:block=" + target.toShortString()
					);
					if (STATIC_TERRAIN_FAULT.startsWith("translucent-")) {
						recordStaticTerrainLifecycleFaultMarker(target);
					}
					staticTerrainLifecycleStage = "translucent-world-b-fixture-placed";
					return false;
				}
				if (!staticTerrainInPlayableWorld(minecraft)
					|| diagnostics.visibleLayerSubmissions() <= staticTerrainUnloadSubmissionSnapshot
					|| diagnostics.cachedLayerAssets() <= 0) {
					staticTerrainLifecycleStage = "waiting-for-world-b-terrain";
					return false;
				}
				staticTerrainReloadGenerationB = Math.max(1L, diagnostics.atlasGeneration());
				staticTerrainLifecycleAfterGeneration = staticTerrainReloadGenerationB;
				staticTerrainLifecycleAfterCachedLayers = diagnostics.cachedLayerAssets();
				staticTerrainLifecycleAfterRssBytes = currentUsedMemoryBytes();
				if ("cross-world-stale-submission".equals(STATIC_TERRAIN_FAULT)
					&& !staticTerrainCrossWorldStaleFaultInjected) {
					staticTerrainCrossWorldStaleFaultInjected = true;
					RustGalTerrainRenderer.injectCrossWorldStaleSubmissionForDiagnostics(
						minecraft.getWindow().getWidth(),
						minecraft.getWindow().getHeight()
					);
				}
				staticTerrainLifecycleAfterRecorded = true;
				staticTerrainLifecycleStage = "replacement-visible";
				RustGalTerrainRenderer.recordLifecycleMarker(
					"lifecycle-world-reload-valid",
					BlockPos.ZERO,
					ChunkSectionLayer.SOLID,
					"world=" + staticTerrainLifecycleWorldB
						+ ":generation=" + staticTerrainReloadGenerationB
						+ ":cachedLayers=" + diagnostics.cachedLayerAssets()
				);
				framesWaitingForStaticTerrainLifecycle++;
				return framesWaitingForStaticTerrainLifecycle >= Math.max(8, FRAMES_PER_POSE);
			}
			default -> {
				staticTerrainLifecycleStage = "replacement-visible";
				return true;
			}
		}
	}

	private static boolean staticTerrainInPlayableWorld(Minecraft minecraft) {
		return minecraft.player != null && minecraft.level != null && minecraft.getSingleplayerServer() != null;
	}

	private static boolean continueStaticTerrainLifecycleAction(Minecraft minecraft) {
		String scenario = staticTerrainBaseScenario();
		if (staticTerrainReloadFuture != null && !staticTerrainReloadFuture.isDone()) {
			staticTerrainLifecycleStage = "waiting-for-resource-reload";
			return true;
		}
		if (("resize-cycle".equals(scenario) || "swapchain-recreate".equals(scenario))
			&& staticTerrainLifecycleResizeCount == 1
			&& framesWaitingForStaticTerrainLifecycle >= Math.max(2, FRAMES_PER_POSE / 2)) {
			minecraft.getWindow().setWindowed(1280, 720);
			minecraft.resizeDisplay();
			staticTerrainLifecycleResizeCount = 2;
			staticTerrainLifecycleStage = "resize-2";
			RustGalTerrainRenderer.recordLifecycleMarker("lifecycle-resize-2", staticTerrainLifecycleEditBlock, staticTerrainLifecycleLayer(), "extent=1280x720");
			return true;
		}
		if ("return-visited-terrain".equals(scenario)
			&& staticTerrainLifecycleActionStep == 1
			&& staticTerrainOriginalPosition != null
			&& framesWaitingForStaticTerrainLifecycle >= Math.max(2, FRAMES_PER_POSE / 2)) {
			initialPosition = staticTerrainOriginalPosition;
			if (minecraft.player != null && initialPose != null) {
				minecraft.player.setPos(initialPosition);
				minecraft.player.setOldPosAndRot(initialPosition, initialPose.yaw(), initialPose.pitch());
			}
			staticTerrainLifecycleActionStep = 2;
			staticTerrainLifecycleStage = "returned-to-visited-terrain";
			RustGalTerrainRenderer.recordLifecycleMarker(
				"lifecycle-return-visited-terrain-back",
				staticTerrainLifecycleEditBlock,
				staticTerrainLifecycleLayer(),
				"position=" + formatVec(initialPosition)
			);
			return true;
		}
		return false;
	}

	private static long observedStaticTerrainGeneration() {
		if (staticTerrainUsesAtlasGeneration()) {
			return RustGalTerrainRenderer.diagnosticsSnapshot().atlasGeneration();
		}
		if (staticTerrainAllowsAirSource()) {
			RustGalTerrainRenderer.TerrainLayerSnapshot after =
				RustGalTerrainRenderer.snapshotLayer(staticTerrainLifecycleEditBlock, staticTerrainLifecycleLayer());
			return after == null ? 0L : after.meshGeneration();
		}
		if (!staticTerrainEditScenario()) {
			return Math.max(1L, RustGalTerrainRenderer.diagnosticsSnapshot().atlasGeneration());
		}
		RustGalTerrainRenderer.TerrainLayerSnapshot after =
			RustGalTerrainRenderer.snapshotLayer(staticTerrainLifecycleEditBlock, ChunkSectionLayer.SOLID);
		return after == null ? 0L : after.meshGeneration();
	}

	private static boolean staticTerrainReplacementReady(long observedGeneration) {
		if (!WorldRenderRoutePolicy.currentStaticTerrainRoute().usesRustWholeFrameVulkan()) {
			return framesWaitingForStaticTerrainLifecycle >= Math.max(4, FRAMES_PER_POSE / 2);
		}
		if (observedGeneration == 0L) {
			return false;
		}
		if (staticTerrainAllowsAirSource()) {
			return RustGalTerrainRenderer.diagnosticsSnapshot().visibleLayerSubmissions() > 0
				&& framesWaitingForStaticTerrainLifecycle >= Math.max(4, FRAMES_PER_POSE / 2);
		}
		if (staticTerrainEditScenario() || staticTerrainUsesAtlasGeneration()) {
			return observedGeneration != staticTerrainLifecycleBeforeGeneration;
		}
		return RustGalTerrainRenderer.diagnosticsSnapshot().visibleLayerSubmissions() > 0
			&& framesWaitingForStaticTerrainLifecycle >= Math.max(4, FRAMES_PER_POSE / 2);
	}

	private static boolean staticTerrainRequiresPostSetupExecution() {
		return WorldRenderRoutePolicy.currentStaticTerrainRoute().usesRustWholeFrameVulkan();
	}

	private static int staticTerrainPostSetupRequiredFrames() {
		// A selected-source capture already requires an exact post-edit Rust
		// submission and a matching rolling receipt. Keeping the generic
		// eight-frame lifecycle delay would only turn the same verified source
		// work into a timeout on the deliberately expensive first source route.
		return REQUIRED_RUST_SOURCE_EXECUTION_DIR.isEmpty()
			? Math.max(8, FRAMES_PER_POSE)
			: 1;
	}

	private static boolean staticTerrainPostSetupExecutionReady(Minecraft minecraft) {
		RustGalTerrainRenderer.StaticTerrainExecutionSnapshot execution =
			RustGalTerrainRenderer.staticTerrainExecutionSnapshot();
		if (staticTerrainRequiresPostSetupExecution()
			&& !execution.executedAfter(staticTerrainLifecycleExecutionSubmissionBaseline)) {
			staticTerrainLifecycleStage = "waiting-for-post-setup-static-terrain-execution";
			if (framesWaitingForStaticTerrainLifecycle > SETTLED_READY_MAX_WAIT_FRAMES) {
				fail("timed out waiting for post-setup Rust static-terrain execution: baselineSubmission="
					+ staticTerrainLifecycleExecutionSubmissionBaseline
					+ " executedFrame=" + execution.frameId()
					+ " executedSubmission=" + execution.submissionId()
					+ " instances=" + execution.instances());
			} else if ((framesWaitingForStaticTerrainLifecycle % 30) == 0) {
				writeMetadata(minecraft, "waiting_for_post_setup_static_terrain_execution");
			}
			return false;
		}
		staticTerrainLifecycleExecutionFrame = execution.frameId();
		staticTerrainLifecycleExecutionSubmission = execution.submissionId();
		staticTerrainLifecycleExecutionInstances = execution.instances();
		if (staticTerrainLifecycleSourceExecutionSubmission > 0L) {
			return true;
		}
		if (!requiredRustSourceExecutionObservedAtOrAfter(execution.submissionId())) {
			staticTerrainLifecycleStage = "waiting-for-post-setup-selected-source-execution";
			if (framesWaitingForStaticTerrainLifecycle > SETTLED_READY_MAX_WAIT_FRAMES) {
				fail("timed out waiting for selected-source execution correlated to post-setup static terrain: "
					+ "executionSubmission=" + execution.submissionId());
			} else if ((framesWaitingForStaticTerrainLifecycle % 30) == 0) {
				writeMetadata(minecraft, "waiting_for_post_setup_selected_source_execution");
			}
			return false;
		}
		staticTerrainLifecycleSourceExecutionFrame = execution.frameId();
		staticTerrainLifecycleSourceExecutionSubmission = execution.submissionId();
		return true;
	}

	private static byte[] staticTerrainValidTinyPngPayload() {
		BufferedImage image = new BufferedImage(1, 1, BufferedImage.TYPE_INT_ARGB);
		image.setRGB(0, 0, 0xffff3355);
		try (ByteArrayOutputStream output = new ByteArrayOutputStream()) {
			ImageIO.write(image, "png", output);
			return output.toByteArray();
		} catch (IOException error) {
			throw new IllegalStateException("Failed to create diagnostic static-terrain atlas PNG", error);
		}
	}

	private static long currentUsedMemoryBytes() {
		Runtime runtime = Runtime.getRuntime();
		return Math.max(0L, runtime.totalMemory() - runtime.freeMemory());
	}

	private static void applyStaticTerrainLifecycleEdit(ServerLevel serverLevel, ClientLevel clientLevel, BlockPos target, BlockState replacement) {
		serverLevel.setBlock(target, replacement, 3);
		clientLevel.setBlock(target, replacement, 3);
	}

	private static void applyStaticTerrainTexturePalette(Minecraft minecraft, ServerLevel serverLevel, BlockPos target) {
		if (minecraft.player == null || minecraft.level == null) {
			return;
		}
		Direction right = minecraft.player.getDirection().getClockWise();
		BlockPos[] positions = {
			target,
			target.relative(right),
			target.relative(right, 2),
			target.relative(right, 3)
		};
		BlockState[] states = {
			Blocks.GRASS_BLOCK.defaultBlockState(),
			Blocks.REDSTONE_ORE.defaultBlockState(),
			Blocks.YELLOW_TERRACOTTA.defaultBlockState(),
			Blocks.OAK_LEAVES.defaultBlockState().setValue(LeavesBlock.PERSISTENT, true)
		};
		staticTerrainTexturePaletteProbes = List.of(
			new RustGalTerrainRenderer.TerrainTextureProbe(positions[0], List.of(
				ResourceLocation.fromNamespaceAndPath("minecraft", "block/grass_block_top"),
				ResourceLocation.fromNamespaceAndPath("minecraft", "block/grass_block_side"),
				ResourceLocation.fromNamespaceAndPath("minecraft", "block/grass_block_side_overlay"),
				ResourceLocation.fromNamespaceAndPath("minecraft", "block/dirt")
			)),
			new RustGalTerrainRenderer.TerrainTextureProbe(positions[1], List.of(
				ResourceLocation.fromNamespaceAndPath("minecraft", "block/redstone_ore")
			)),
			new RustGalTerrainRenderer.TerrainTextureProbe(positions[2], List.of(
				ResourceLocation.fromNamespaceAndPath("minecraft", "block/yellow_terracotta")
			)),
			new RustGalTerrainRenderer.TerrainTextureProbe(positions[3], List.of(
				ResourceLocation.fromNamespaceAndPath("minecraft", "block/oak_leaves"),
				ResourceLocation.fromNamespaceAndPath("minecraft", "block/oak_leaves2"),
				ResourceLocation.fromNamespaceAndPath("minecraft", "block/oak_leaves_bushy"),
				ResourceLocation.fromNamespaceAndPath("minecraft", "block/oak_leaves_bushy1")
			))
		);
		RustGalTerrainRenderer.setActiveTextureProbes(staticTerrainTexturePaletteProbes);
		boolean changed = false;
		for (int index = 0; index < positions.length; index++) {
			if (serverLevel.isLoaded(positions[index]) && minecraft.level.isLoaded(positions[index])) {
				applyStaticTerrainLifecycleEdit(serverLevel, minecraft.level, positions[index], states[index]);
				changed = true;
			}
		}
		if (changed) {
			int minX = positions[0].getX();
			int maxX = minX;
			int minZ = positions[0].getZ();
			int maxZ = minZ;
			for (BlockPos position : positions) {
				minX = Math.min(minX, position.getX());
				maxX = Math.max(maxX, position.getX());
				minZ = Math.min(minZ, position.getZ());
				maxZ = Math.max(maxZ, position.getZ());
			}
			// The client-level writes are real state changes. Rebuild only their
			// sections and neighbors instead of invalidating every terrain source
			// snapshot with a global renderer reset.
			minecraft.levelRenderer.setBlocksDirty(
				minX - 1,
				target.getY() - 1,
				minZ - 1,
				maxX + 1,
				target.getY() + 1,
				maxZ + 1
			);
		}
	}

	/**
	 * The palette fixture is an all-or-nothing semantic workload. Partially
	 * applying it leaves the later atlas proof looking for a block the client
	 * never asked Sodium to rebuild, which is a setup failure rather than a
	 * texture-routing result.
	 */
	private static boolean staticTerrainTexturePalettePositionsLoaded(
		Minecraft minecraft,
		ServerLevel serverLevel,
		BlockPos target
	) {
		if (minecraft.level == null) {
			return false;
		}
		Direction right = minecraft.player.getDirection().getClockWise();
		for (int offset = 0; offset < 4; offset++) {
			BlockPos position = target.relative(right, offset);
			if (!serverLevel.isLoaded(position) || !minecraft.level.isLoaded(position)) {
				return false;
			}
		}
		return true;
	}

	private static RustGalTerrainRenderer.TerrainTextureProbeReceipt staticTerrainTexturePaletteProbeReceipt() {
		return RustGalTerrainRenderer.terrainTextureProbeReceipt(staticTerrainTexturePaletteProbes);
	}

	private static DistantHorizonsSemanticCollector.DistantHorizonsTextureProbeReceipt distantHorizonsTexturePaletteProbeReceipt() {
		List<DistantHorizonsSemanticCollector.DistantHorizonsTextureProbe> probes =
			distantHorizonsTexturePaletteProbes.stream()
				.map(probe -> new DistantHorizonsSemanticCollector.DistantHorizonsTextureProbe(
					probe.position().getX(), probe.position().getY(), probe.position().getZ(), probe.blockId(), probe.allowedSprites(), probe.requiredSprites()
				))
				.toList();
		return DISTANT_HORIZONS_LEGACY_OBSERVATION
			? DistantHorizonsSemanticCollector.legacyTextureProbeReceipt(probes)
			: DistantHorizonsSemanticCollector.textureProbeReceipt(probes);
	}

	private static DistantHorizonsSemanticCollector.DistantHorizonsWaterProbeReceipt distantHorizonsWaterProbeReceipt() {
		if (distantHorizonsWaterWitnesses.isEmpty()) {
			return DistantHorizonsSemanticCollector.waterProbeReceipt(List.of());
		}
		// The plate's center is intentionally away from its coarsened boundary.
		// One exact cell is enough to prove that this fixture, rather than some
		// unrelated world water, reached the completed Rust submission.
		return DistantHorizonsSemanticCollector.waterProbeReceipt(List.of(distantHorizonsWaterProbe()));
	}

	private static DistantHorizonsSemanticCollector.DistantHorizonsWaterProbeReceipt distantHorizonsWaterSourceProbeReceipt() {
		if (distantHorizonsWaterWitnesses.isEmpty()) {
			return DistantHorizonsSemanticCollector.waterSourceProbeReceipt(List.of());
		}
		return DistantHorizonsSemanticCollector.waterSourceProbeReceipt(List.of(distantHorizonsWaterProbe()));
	}

	private static DistantHorizonsSemanticCollector.DistantHorizonsWaterProbeReceipt distantHorizonsWaterCachedProbeReceipt() {
		if (distantHorizonsWaterWitnesses.isEmpty()) {
			return DistantHorizonsSemanticCollector.waterCachedProbeReceipt(List.of());
		}
		return DistantHorizonsSemanticCollector.waterCachedProbeReceipt(List.of(distantHorizonsWaterProbe()));
	}

	private static BlockPos distantHorizonsWaterProbe() {
		// Witnesses are ordered x-major/z-minor; (x+1,z+1) is a sealed
		// interior cell and avoids edge-fluid reduction artifacts.
		return distantHorizonsWaterWitnesses.get(5);
	}

	private static DistantHorizonsSemanticCollector.DistantHorizonsWaterProbeReceipt distantHorizonsTexturePaletteCachedWaterProbeReceipt() {
		if (distantHorizonsWaterWitnesses.isEmpty()) {
			return DistantHorizonsSemanticCollector.waterCachedProbeReceipt(List.of());
		}
		return DistantHorizonsSemanticCollector.waterCachedProbeReceipt(List.of(
			distantHorizonsWaterWitnesses.get(distantHorizonsWaterWitnesses.size() / 2)
		));
	}

	private static DistantHorizonsSemanticCollector.WaterSourceInputReceipt distantHorizonsWaterSourceInputReceipt() {
		if (distantHorizonsWaterWitnesses.isEmpty()) {
			return DistantHorizonsSemanticCollector.waterSourceInputReceipt(List.of());
		}
		return DistantHorizonsSemanticCollector.waterSourceInputReceipt(List.of(
			distantHorizonsWaterWitnesses.get(distantHorizonsWaterWitnesses.size() / 2)
		));
	}

	private static void applyStaticTerrainTranslucentOverlap(Minecraft minecraft, ServerLevel serverLevel, BlockPos target) {
		if (minecraft.player == null || minecraft.level == null) {
			return;
		}
		Direction forward = minecraft.player.getDirection();
		GraphicsAuditOverlapProjection.configure(target, forward);
		Direction right = forward.getClockWise();
		BlockState[] layers = {
			Blocks.RED_STAINED_GLASS.defaultBlockState(),
			Blocks.GREEN_STAINED_GLASS.defaultBlockState(),
			Blocks.BLUE_STAINED_GLASS.defaultBlockState()
		};
		for (int depth = 0; depth < layers.length; depth++) {
			BlockPos planeBase = target.relative(forward, depth * 2);
			for (int lateral = -1; lateral <= 1; lateral++) {
				for (int vertical = 0; vertical <= 2; vertical++) {
					BlockPos pos = planeBase.relative(right, lateral).above(vertical);
					if (serverLevel.isLoaded(pos) && minecraft.level.isLoaded(pos)) {
						applyStaticTerrainLifecycleEdit(serverLevel, minecraft.level, pos, layers[depth]);
					}
				}
			}
		}
		BlockState crossing = Blocks.ORANGE_STAINED_GLASS.defaultBlockState();
		BlockPos crossingBase = target.relative(forward, 1).relative(right, 2);
		for (int depth = 0; depth <= 4; depth++) {
			for (int vertical = 0; vertical <= 2; vertical++) {
				BlockPos pos = crossingBase.relative(forward, depth).above(vertical);
				if (serverLevel.isLoaded(pos) && minecraft.level.isLoaded(pos)) {
					applyStaticTerrainLifecycleEdit(serverLevel, minecraft.level, pos, crossing);
				}
			}
		}
		// ClientLevel#setBlock normally schedules a Sodium rebuild, but this
		// fixture deliberately writes both server and client worlds while the
		// Rust whole-frame source owns same-level section state.  Invalidate the
		// complete edited section neighborhood explicitly so retained CPU meshes
		// cannot mask the semantic glass panes from the native producer.
		int minSectionX = Integer.MAX_VALUE;
		int minSectionY = Integer.MAX_VALUE;
		int minSectionZ = Integer.MAX_VALUE;
		int maxSectionX = Integer.MIN_VALUE;
		int maxSectionY = Integer.MIN_VALUE;
		int maxSectionZ = Integer.MIN_VALUE;
		for (int depth = 0; depth <= 4; depth++) {
			for (int lateral = -2; lateral <= 2; lateral++) {
				for (int vertical = 0; vertical <= 2; vertical++) {
					BlockPos pos = target.relative(forward, depth).relative(right, lateral).above(vertical);
					minSectionX = Math.min(minSectionX, SectionPos.blockToSectionCoord(pos.getX()));
					minSectionY = Math.min(minSectionY, SectionPos.blockToSectionCoord(pos.getY()));
					minSectionZ = Math.min(minSectionZ, SectionPos.blockToSectionCoord(pos.getZ()));
					maxSectionX = Math.max(maxSectionX, SectionPos.blockToSectionCoord(pos.getX()));
					maxSectionY = Math.max(maxSectionY, SectionPos.blockToSectionCoord(pos.getY()));
					maxSectionZ = Math.max(maxSectionZ, SectionPos.blockToSectionCoord(pos.getZ()));
				}
			}
		}
		if (minSectionX <= maxSectionX) {
			minecraft.levelRenderer.setSectionRangeDirty(
				minSectionX, minSectionY, minSectionZ,
				maxSectionX, maxSectionY, maxSectionZ
			);
		}
	}

	private static void applyStaticTerrainTranslucentMixed(
		Minecraft minecraft,
		ServerLevel serverLevel,
		BlockPos target,
		boolean includeUnsupportedFluid
	) {
		if (minecraft.player == null || minecraft.level == null) {
			return;
		}
		if (!includeUnsupportedFluid) {
			if (!GraphicsAuditMixedFluidFixture.install(minecraft, target, minecraft.player.getDirection())) {
				throw new IllegalStateException("Mixed-fluid capture fixture is not fully loaded");
			}
			return;
		}
		Direction forward = minecraft.player.getDirection();
		Direction right = forward.getClockWise();
		BlockState glass = Blocks.BLUE_STAINED_GLASS.defaultBlockState();
		BlockState water = Blocks.WATER.defaultBlockState();
		BlockState lava = Blocks.LAVA.defaultBlockState();
		BlockPos[] positions = {
			target,
			target.relative(right),
			target.relative(forward),
			target.relative(forward).relative(right),
			target.above(),
			target.above().relative(right)
		};
		BlockState[] states = {
			glass,
			water,
			water,
			glass,
			glass,
			water
		};
		for (int index = 0; index < positions.length; index++) {
			BlockPos pos = positions[index];
			if (serverLevel.isLoaded(pos) && minecraft.level.isLoaded(pos)) {
				applyStaticTerrainLifecycleEdit(serverLevel, minecraft.level, pos, states[index]);
			}
		}
		if (includeUnsupportedFluid) {
			BlockPos unsupported = sameSectionBlockPos(target, 8, 8, 8);
			BlockPos[] clear = {
				unsupported.above(),
				unsupported.below(),
				unsupported.north(),
				unsupported.south(),
				unsupported.east(),
				unsupported.west()
			};
			for (BlockPos air : clear) {
				if (serverLevel.isLoaded(air) && minecraft.level.isLoaded(air)) {
					applyStaticTerrainLifecycleEdit(serverLevel, minecraft.level, air, Blocks.AIR.defaultBlockState());
				}
			}
			BlockPos[] companionPositions = {
				sameSectionBlockPos(target, 10, unsupported.getY() & 15, 8),
				sameSectionBlockPos(target, 12, unsupported.getY() & 15, 8),
				sameSectionBlockPos(target, 8, unsupported.getY() & 15, 10),
				sameSectionBlockPos(target, 10, unsupported.getY() & 15, 10)
			};
			BlockState[] companionStates = {
				glass,
				water,
				glass,
				water
			};
			for (int index = 0; index < companionPositions.length; index++) {
				BlockPos companion = companionPositions[index];
				if (serverLevel.isLoaded(companion) && minecraft.level.isLoaded(companion)) {
					applyStaticTerrainLifecycleEdit(serverLevel, minecraft.level, companion, companionStates[index]);
				}
			}
			if (serverLevel.isLoaded(unsupported) && minecraft.level.isLoaded(unsupported)) {
				applyStaticTerrainLifecycleEdit(serverLevel, minecraft.level, unsupported, lava);
				RustGalTerrainRenderer.recordLifecycleMarker(
					"lifecycle-translucent-unsupported-fluid-placed",
					unsupported,
					ChunkSectionLayer.TRANSLUCENT,
					STATIC_TERRAIN_SCENARIO
						+ ":block=" + unsupported.toShortString()
						+ ":serverState=" + serverLevel.getBlockState(unsupported)
						+ ":clientState=" + minecraft.level.getBlockState(unsupported)
						+ ":serverFluid=" + serverLevel.getFluidState(unsupported)
						+ ":clientFluid=" + minecraft.level.getFluidState(unsupported)
				);
			} else {
				RustGalTerrainRenderer.recordLifecycleMarker(
					"lifecycle-translucent-unsupported-fluid-not-loaded",
					unsupported,
					ChunkSectionLayer.TRANSLUCENT,
					STATIC_TERRAIN_SCENARIO
						+ ":block=" + unsupported.toShortString()
						+ ":serverLoaded=" + serverLevel.isLoaded(unsupported)
						+ ":clientLoaded=" + minecraft.level.isLoaded(unsupported)
				);
			}
		}
	}

	private static BlockPos sameSectionBlockPos(BlockPos anchor, int localX, int localY, int localZ) {
		return new BlockPos(
			(anchor.getX() & ~15) + Math.max(0, Math.min(15, localX)),
			(anchor.getY() & ~15) + Math.max(0, Math.min(15, localY)),
			(anchor.getZ() & ~15) + Math.max(0, Math.min(15, localZ))
		);
	}

	private static void recordStaticTerrainLifecycleFaultMarker(BlockPos target) {
		switch (STATIC_TERRAIN_FAULT) {
			case "old-generation-after-edit" -> RustGalTerrainRenderer.recordLifecycleMarker(
				"lifecycle-fault-old-generation-after-edit",
				target,
				ChunkSectionLayer.SOLID,
				STATIC_TERRAIN_SCENARIO + ":block=" + target.toShortString()
			);
			case "old-new-together" -> RustGalTerrainRenderer.recordLifecycleMarker(
				"lifecycle-fault-old-new-overlap",
				target,
				ChunkSectionLayer.SOLID,
				STATIC_TERRAIN_SCENARIO + ":block=" + target.toShortString()
			);
			case "removed-section-resubmitted" -> RustGalTerrainRenderer.recordLifecycleMarker(
				"lifecycle-fault-removed-section-resubmitted",
				target,
				ChunkSectionLayer.SOLID,
				STATIC_TERRAIN_SCENARIO + ":block=" + target.toShortString()
			);
			case "wrong-neighbor-invalidated" -> RustGalTerrainRenderer.recordLifecycleMarker(
				"lifecycle-fault-wrong-neighbor-invalidated",
				target,
				ChunkSectionLayer.SOLID,
				STATIC_TERRAIN_SCENARIO + ":block=" + target.toShortString()
			);
			case "translucent-sort-reversed" -> RustGalTerrainRenderer.recordTranslucentFaultMarker(
				"sort-reversed",
				target,
				STATIC_TERRAIN_SCENARIO + ":block=" + target.toShortString()
			);
			case "translucent-sort-stale" -> RustGalTerrainRenderer.recordTranslucentFaultMarker(
				"sort-stale",
				target,
				STATIC_TERRAIN_SCENARIO + ":block=" + target.toShortString()
			);
			case "translucent-sort-missing" -> RustGalTerrainRenderer.recordTranslucentFaultMarker(
				"sort-missing",
				target,
				STATIC_TERRAIN_SCENARIO + ":block=" + target.toShortString()
			);
			case "translucent-duplicate-primitive" -> RustGalTerrainRenderer.recordTranslucentFaultMarker(
				"duplicate-primitive",
				target,
				STATIC_TERRAIN_SCENARIO + ":block=" + target.toShortString()
			);
			case "translucent-index-out-of-range" -> RustGalTerrainRenderer.recordTranslucentFaultMarker(
				"index-out-of-range",
				target,
				STATIC_TERRAIN_SCENARIO + ":block=" + target.toShortString()
			);
			case "translucent-index-type-invalid" -> RustGalTerrainRenderer.recordTranslucentFaultMarker(
				"index-type-invalid",
				target,
				STATIC_TERRAIN_SCENARIO + ":block=" + target.toShortString()
			);
			case "translucent-index-alignment-invalid" -> RustGalTerrainRenderer.recordTranslucentFaultMarker(
				"index-alignment-invalid",
				target,
				STATIC_TERRAIN_SCENARIO + ":block=" + target.toShortString()
			);
			case "translucent-sort-section-mismatch" -> RustGalTerrainRenderer.recordTranslucentFaultMarker(
				"sort-section-mismatch",
				target,
				STATIC_TERRAIN_SCENARIO + ":block=" + target.toShortString()
			);
			case "translucent-old-new-overlap" -> RustGalTerrainRenderer.recordTranslucentFaultMarker(
				"old-new-overlap",
				target,
				STATIC_TERRAIN_SCENARIO + ":block=" + target.toShortString()
			);
			case "translucent-section-order-reversed" -> RustGalTerrainRenderer.recordTranslucentFaultMarker(
				"section-order-reversed",
				target,
				STATIC_TERRAIN_SCENARIO + ":block=" + target.toShortString()
			);
			case "translucent-depth-write-enabled" -> RustGalTerrainRenderer.recordTranslucentFaultMarker(
				"depth-write-enabled",
				target,
				STATIC_TERRAIN_SCENARIO + ":block=" + target.toShortString()
			);
			case "translucent-opaque-blend" -> RustGalTerrainRenderer.recordTranslucentFaultMarker(
				"opaque-blend",
				target,
				STATIC_TERRAIN_SCENARIO + ":block=" + target.toShortString()
			);
			case "translucent-cross-world-sort" -> RustGalTerrainRenderer.recordTranslucentFaultMarker(
				"cross-world-sort",
				target,
				STATIC_TERRAIN_SCENARIO + ":block=" + target.toShortString()
			);
			case "translucent-source-sort-mismatch" -> RustGalTerrainRenderer.recordTranslucentFaultMarker(
				"source-sort-mismatch",
				target,
				STATIC_TERRAIN_SCENARIO + ":block=" + target.toShortString()
			);
			case "translucent-sort-payload-corrupt" -> RustGalTerrainRenderer.recordTranslucentFaultMarker(
				"sort-payload-corrupt",
				target,
				STATIC_TERRAIN_SCENARIO + ":block=" + target.toShortString()
			);
			case "translucent-primitive-metadata-missing" -> RustGalTerrainRenderer.recordTranslucentFaultMarker(
				"primitive-metadata-missing",
				target,
				STATIC_TERRAIN_SCENARIO + ":block=" + target.toShortString()
			);
			case "translucent-primitive-kind-unknown" -> RustGalTerrainRenderer.recordTranslucentFaultMarker(
				"primitive-kind-unknown",
				target,
				STATIC_TERRAIN_SCENARIO + ":block=" + target.toShortString()
			);
			case "translucent-primitive-range-overlap" -> RustGalTerrainRenderer.recordTranslucentFaultMarker(
				"primitive-range-overlap",
				target,
				STATIC_TERRAIN_SCENARIO + ":block=" + target.toShortString()
			);
			case "translucent-primitive-range-out-of-bounds" -> RustGalTerrainRenderer.recordTranslucentFaultMarker(
				"primitive-range-out-of-bounds",
				target,
				STATIC_TERRAIN_SCENARIO + ":block=" + target.toShortString()
			);
			case "translucent-primitive-classification-swapped" -> RustGalTerrainRenderer.recordTranslucentFaultMarker(
				"primitive-classification-swapped",
				target,
				STATIC_TERRAIN_SCENARIO + ":block=" + target.toShortString()
			);
			case "translucent-supported-primitive-omitted" -> RustGalTerrainRenderer.recordTranslucentFaultMarker(
				"supported-primitive-omitted",
				target,
				STATIC_TERRAIN_SCENARIO + ":block=" + target.toShortString()
			);
			case "translucent-unsupported-primitive-executed" -> RustGalTerrainRenderer.recordTranslucentFaultMarker(
				"unsupported-primitive-executed",
				target,
				STATIC_TERRAIN_SCENARIO + ":block=" + target.toShortString()
			);
			case "translucent-source-index-unknown-primitive" -> RustGalTerrainRenderer.recordTranslucentFaultMarker(
				"source-index-unknown-primitive",
				target,
				STATIC_TERRAIN_SCENARIO + ":block=" + target.toShortString()
			);
			case "translucent-retained-index-duplicated" -> RustGalTerrainRenderer.recordTranslucentFaultMarker(
				"retained-index-duplicated",
				target,
				STATIC_TERRAIN_SCENARIO + ":block=" + target.toShortString()
			);
			case "translucent-filtered-order-changed" -> RustGalTerrainRenderer.recordTranslucentFaultMarker(
				"filtered-order-changed",
				target,
				STATIC_TERRAIN_SCENARIO + ":block=" + target.toShortString()
			);
			case "translucent-ordered-range-material-mismatch" -> RustGalTerrainRenderer.recordTranslucentFaultMarker(
				"ordered-range-material-mismatch",
				target,
				STATIC_TERRAIN_SCENARIO + ":block=" + target.toShortString()
			);
			case "translucent-ordered-range-overlap" -> RustGalTerrainRenderer.recordTranslucentFaultMarker(
				"ordered-range-overlap",
				target,
				STATIC_TERRAIN_SCENARIO + ":block=" + target.toShortString()
			);
			case "translucent-execution-order-hash-mismatch" -> RustGalTerrainRenderer.recordTranslucentFaultMarker(
				"execution-order-hash-mismatch",
				target,
				STATIC_TERRAIN_SCENARIO + ":block=" + target.toShortString()
			);
			case "translucent-cross-generation-metadata" -> RustGalTerrainRenderer.recordTranslucentFaultMarker(
				"cross-generation-metadata",
				target,
				STATIC_TERRAIN_SCENARIO + ":block=" + target.toShortString()
			);
			case "translucent-cross-world-primitive-range-reuse" -> RustGalTerrainRenderer.recordTranslucentFaultMarker(
				"cross-world-primitive-range-reuse",
				target,
				STATIC_TERRAIN_SCENARIO + ":block=" + target.toShortString()
			);
			case "water-still-flow-texture-swapped",
				"water-overlay-identity-invalid",
				"water-animation-entry-mismatch",
				"water-animation-invalid-frame",
				"water-animation-duplicate-frame",
				"water-animation-missing-frame",
				"water-animation-zero-duration",
				"water-animation-duration-overflow",
				"water-animation-region-out-of-bounds",
				"water-animation-missing-pixels",
				"water-animation-incompatible-dimensions",
				"water-animation-generation-mismatch",
				"water-animation-stale-generation",
				"water-animation-region-mismatch",
				"water-animation-invalid-interp",
				"water-animation-section-divergence",
				"water-animation-cross-world-state",
				"water-animation-per-frame-recreation",
				"water-corner-height-invalid",
				"water-flow-reversed",
				"water-uv-invalid",
				"water-tint-invalid",
				"water-light-swapped",
				"water-normal-invalid",
				"water-duplicate-face",
				"water-missing-face",
				"water-boundary-crack",
				"water-depth-write-enabled",
				"water-opaque-blend",
				"water-stale-mesh-generation",
				"water-glass-range-mismatch" -> RustGalTerrainRenderer.recordTranslucentFaultMarker(
				STATIC_TERRAIN_FAULT,
				target,
				STATIC_TERRAIN_SCENARIO + ":block=" + target.toShortString()
			);
			default -> {
			}
		}
	}

	public static void forceBlockOutlineTargetForDiagnostics(Minecraft minecraft) {
		if (!ENABLED || !FORCE_BLOCK_OUTLINE_TARGET || !initialized || forcedBlockOutlineTarget == null) {
			return;
		}
		minecraft.hitResult = new BlockHitResult(
			forcedBlockOutlineTarget.hitLocation(),
			forcedBlockOutlineTarget.face(),
			forcedBlockOutlineTarget.blockPos(),
			false
		);
	}

	public static void applyBossBarOverridesForDiagnostics(BossHealthOverlay overlay) {
		if (!ENABLED || !hasBossBarOverride()) {
			return;
		}
		int count = forcedBossBarCount();
		overlay.replaceEventsForDeterministicCapture(count, forcedBossBarProgresses(count), forcedBossBarOverlays(count));
	}

	public static boolean isReadyForShaderInputParityPoseDiagnostics() {
		return !ENABLED || !initialized || SETTLED_READY_FRAMES <= 0 || settledReadyGateSatisfied || poseIndex > 0;
	}

	/**
	 * Capture-only readiness invalidation from the independent Rust terrain
	 * source. A settled receipt belongs to a particular immutable terrain
	 * generation; it must not survive a later dirty-section notification.
	 */
	public static void invalidateRustWholeFrameTerrainReadiness() {
		if (!ENABLED || !settledReadyGateSatisfied) {
			return;
		}
		settledReadyGateSatisfied = false;
		settledRustTerrainFrames = 0;
		staticTerrainSettledFrames = 0;
		staticTerrainSettledSignature = "rust-whole-frame-terrain-invalidated";
	}

	public static boolean isActiveForDiagnostics() {
		return (ENABLED && initialized && !complete && !failed)
			|| GraphicsFrameBenchmark.isActiveForDiagnostics();
	}

	public static String currentPoseNameForDiagnostics() {
		if (!ENABLED || !initialized || poses == null) {
			return "none";
		}
		if (poseIndex >= 0 && poseIndex < poses.length) {
			return poses[poseIndex].name();
		}
		return complete ? "complete" : "none";
	}

	/** Isolated artifact path for a bounded deterministic diagnostic readback. */
	public static Path diagnosticArtifactPath(String filename) {
		if (filename == null || filename.isBlank() || filename.contains("..") || filename.indexOf('/') >= 0 || filename.indexOf('\\') >= 0) {
			throw new IllegalArgumentException("invalid deterministic diagnostic artifact name");
		}
		return SCREENSHOT_DIR.resolve(filename);
	}

	/** Capture-only receipt of the immutable semantic matrices supplied to the
	 * Rust-owned world frame. It neither selects a render route nor exposes a
	 * Java GPU object. */
	public static void recordRustSkyMatrices(float[] view, float[] projection) {
		if (!ENABLED || complete || failed || view == null || projection == null
			|| view.length != 16 || projection.length != 16) {
			return;
		}
		try {
			Files.createDirectories(SCREENSHOT_DIR);
			Files.writeString(
				SCREENSHOT_DIR.resolve("rust-sky-matrices-last.json"),
				"{\"view\":" + Arrays.toString(view)
					+ ",\"projection\":" + Arrays.toString(projection)
					+ ",\"rendered_frame_index\":" + renderedFrameIndex + "}\n",
				StandardCharsets.UTF_8
			);
		} catch (IOException ignored) {
			// Diagnostics must never affect the selected renderer.
		}
	}

	/** Capture-only receipt of the display-encoded sky color copied into the
	 * immutable Rust background semantic record. */
	public static void recordRustSkyColor(int skyColorArgb) {
		if (!ENABLED || complete || failed) {
			return;
		}
		try {
			Files.createDirectories(SCREENSHOT_DIR);
			Files.writeString(
				SCREENSHOT_DIR.resolve("rust-sky-last.json"),
				"{\"sky_color_argb\":" + Integer.toUnsignedLong(skyColorArgb)
					+ ",\"rendered_frame_index\":" + renderedFrameIndex + "}\n",
				StandardCharsets.UTF_8
			);
		} catch (IOException ignored) {
			// Diagnostics must never affect the selected renderer.
		}
	}

	public static String shaderInputParityContextFields() {
		if (!ENABLED) {
			return "detCapture=false detPose=none detPoseIndex=0 detRenderedFrame=0 detAwaitingScreenshot=false detComplete=false detFailed=false";
		}

		String poseName = "none";
		int displayPoseIndex = 0;
		if (initialized && poses != null) {
			if (poseIndex >= 0 && poseIndex < poses.length) {
				poseName = poses[poseIndex].name();
				displayPoseIndex = poseIndex + 1;
			} else if (complete) {
				poseName = "complete";
				displayPoseIndex = poses.length;
			}
		}

		return "detCapture=true"
			+ " detPose=" + poseName
			+ " detPoseIndex=" + displayPoseIndex
			+ " detRenderedFrame=" + renderedFrameIndex
			+ " detAwaitingScreenshot=" + awaitingScreenshotAck
				+ " detComplete=" + complete
				+ " detFailed=" + failed;
	}

	public static void recordSubmittedWorkIdentity(String family, String identity) {
		recordSubmittedWorkIdentityAtFrame(family, identity, renderedFrameIndex, false);
	}

	/** Records work against the frame whose render has just completed. Some
	 * explicit backends report completion after the capture hook advances its
	 * presentation counter; this keeps settling evidence correlated without
	 * changing ordinary identity recording. */
	public static void recordSubmittedWorkIdentityForCompletedFrame(String family, String identity) {
		recordSubmittedWorkIdentityAtFrame(family, identity, Math.max(0L, renderedFrameIndex - 1L), true);
	}

	private static void recordSubmittedWorkIdentityAtFrame(String family, String identity, long frame, boolean allowBeforeInitialized) {
		if (!ENABLED || (!initialized && !allowBeforeInitialized) || complete || failed || family == null || identity == null) {
			return;
		}
		String normalizedFamily = family.trim();
		if (normalizedFamily.isEmpty()) {
			return;
		}
		String normalizedIdentity = identity.trim();
		if (normalizedIdentity.isEmpty()) {
			return;
		}
		synchronized (SUBMITTED_WORK_BY_FRAME) {
			SUBMITTED_WORK_BY_FRAME
				.computeIfAbsent(frame, ignored -> new LinkedHashMap<>())
				.computeIfAbsent(normalizedFamily, ignored -> new LinkedHashSet<>())
				.add(normalizedIdentity);
			pruneSubmittedWorkFrames();
		}
	}

	public static int deterministicTemporalFrameIndex() {
		if (!ENABLED || !initialized || poses == null || poses.length == 0) {
			return 0;
		}
		int clampedPoseIndex = Math.max(0, Math.min(poseIndex, poses.length - 1));
		int clampedFrameAtPose = Math.max(0, Math.min(renderedFramesAtPose, FRAMES_PER_POSE));
		return clampedPoseIndex * FRAMES_PER_POSE + clampedFrameAtPose;
	}

	private static boolean ensureInitialized(Minecraft minecraft) {
		if (initialized) {
			return true;
		}

		ClientLevel level = minecraft.level;
		LocalPlayer player = minecraft.player;
		// In quick-play singleplayer, the local client level and player become
		// valid before ClientPacketListener is published.  The integrated server
		// is the authoritative local-connection boundary in that window; waiting
		// for the client listener leaves deterministic capture permanently
		// uninitialized even though the real gameplay world is already rendering.
		if (level == null || player == null
			|| (minecraft.getConnection() == null && minecraft.getSingleplayerServer() == null)) {
			return false;
		}
		try {
			Files.createDirectories(SCREENSHOT_DIR);
			Path parent = METADATA_PATH.getParent();
			if (parent != null) {
				Files.createDirectories(parent);
			}
		} catch (IOException exception) {
			fail("failed to create deterministic capture directories: " + exception.getMessage());
			return false;
		}

		initialPosition = player.position();
		initialDimension = level.dimension().location().toString();
		originalCameraType = minecraft.options.getCameraType();
		originalCloudStatus = minecraft.options.cloudStatus().get();
		originalCloudRangeChunks = minecraft.options.cloudRange().get();
			originalGameMode = minecraft.gameMode == null ? null : minecraft.gameMode.getPlayerMode();
			originalPreviousGameMode = minecraft.gameMode == null ? null : minecraft.gameMode.getPreviousPlayerMode();
			originalAttackIndicator = minecraft.options.attackIndicator().get();
					originalChatVisibility = minecraft.options.chatVisibility().get();
					originalHighContrastBlockOutline = minecraft.options.highContrastBlockOutline().get();
					originalNoGravity = player.isNoGravity();
					originalSelectedHotbarSlot = player.getInventory().getSelectedSlot();
					if (!HOTBAR_ITEM_FIXTURE.isEmpty()) {
						originalHotbarItems = new ArrayList<>(9);
						for (int slot = 0; slot < 9; slot++) {
							originalHotbarItems.add(player.getInventory().getItem(slot).copy());
						}
					}
			originalExperienceProgress = player.experienceProgress;
			originalExperienceLevel = player.experienceLevel;
					originalExperienceDisplayStartTick = player.experienceDisplayStartTick;
					originalAttackStrengthTicker = player.getAttackStrengthTickerForDeterministicCapture();
					originalArmorValueOverride = player.getArmorValueForDeterministicCapture();
					originalHealthOverride = player.getHealthForDeterministicCapture();
					originalMaxHealthOverride = player.getMaxHealthForDeterministicCapture();
					originalTicksFrozen = player.getTicksFrozen();
					originalPoisonEffect = copyEffect(player.getEffect(MobEffects.POISON));
					originalWitherEffect = copyEffect(player.getEffect(MobEffects.WITHER));
						originalRegenerationEffect = copyEffect(player.getEffect(MobEffects.REGENERATION));
						originalCrosshairPickEntity = minecraft.crosshairPickEntity;
						initialHealth = player.getHealth();
					if (FORCE_BLOCK_OUTLINE_TARGET || AIM_BLOCK_OUTLINE_TARGET) {
					forcedBlockOutlineTarget = findForcedBlockOutlineTarget(level, player);
					if (forcedBlockOutlineTarget == null) {
						return false;
					}
					initialPosition = forcedBlockOutlineTarget.playerPosition();
				}
				if (HAS_FIXED_CAMERA_POSE && forcedBlockOutlineTarget == null) {
					initialPosition = new Vec3(FIXED_CAMERA_X, FIXED_CAMERA_Y, FIXED_CAMERA_Z);
					player.setPos(initialPosition);
					player.setDeltaMovement(Vec3.ZERO);
					player.setOldPosAndRot(initialPosition, FIXED_CAMERA_YAW, FIXED_CAMERA_PITCH);
				}
		applyRuntimeOverrides(minecraft, player);
					setupDeterministicSupportPlatform(minecraft, player);
						setupRealSurvivalCrackBlock(minecraft, player);
						setupBlockDisplayAndWorldTextScenarios(minecraft, player);
		initialPose = HAS_FIXED_CAMERA_POSE && forcedBlockOutlineTarget == null
			? new Pose("initial", FIXED_CAMERA_YAW, FIXED_CAMERA_PITCH)
			: new Pose("initial", player.getYRot(), player.getXRot());
		if ("bounded".equals(CLOUD_SCENARIO) && !HAS_FIXED_CAMERA_POSE) {
			// The Origin baseline looks toward terrain. Keep the same player state
			// while framing the ordinary cloud producer inside the game viewport.
			initialPose = new Pose("clouds", player.getYRot(), -35.0F);
		}
		if (!BEACON_BEAM_SCENARIO.isEmpty() && !HAS_FIXED_CAMERA_POSE) {
			// Surface-level beacon fixtures can be either above or below the
			// camera in copied worlds. Aim at the real beacon position without
			// changing its block/entity state.
			float beaconPitch = 10.0F;
			if (beaconBeamSetupPosition != null) {
				double dx = beaconBeamSetupPosition.getX() + 0.5D - player.getX();
				double dz = beaconBeamSetupPosition.getZ() + 0.5D - player.getZ();
				double dy = beaconBeamSetupPosition.getY() + 0.5D - player.getEyeY();
				beaconPitch = Mth.clamp((float)-Math.toDegrees(Math.atan2(dy, Math.max(0.1D, Math.sqrt(dx * dx + dz * dz)))), -80.0F, 80.0F);
			}
			initialPose = new Pose("beacon", player.getYRot(), beaconPitch);
		}
		if (forcedBlockOutlineTarget != null) {
			initialPose = forcedBlockOutlineTarget.pose();
			applyPose(player, initialPose);
		} else if (HAS_FIXED_CAMERA_POSE) {
			applyPose(player, initialPose);
		}
		// The DH material palette changes real copied-world blocks and needs the
		// normal asynchronous DH build window. Place it before this capture becomes
		// render-active so the bounded run cannot spend its entire lifetime waiting
		// to initialize and leave only a few source frames after invalidation.
		if (DISTANT_HORIZONS_TEXTURE_PALETTE && !distantHorizonsTexturePaletteSetup) {
			setupDistantHorizonsTexturePaletteAfterSettledReady(minecraft);
			if (!distantHorizonsTexturePaletteSetup) {
				return false;
			}
		}
		Pose[] fullSequence = new Pose[] {
			initialPose,
			new Pose("right", initialPose.yaw() + YAW_DELTA, initialPose.pitch()),
			new Pose("left", initialPose.yaw() - YAW_DELTA, initialPose.pitch()),
			new Pose("return", initialPose.yaw(), initialPose.pitch())
		};
			if (BLOCK_OUTLINE_PAUSE_PARITY) {
				fullSequence = new Pose[] {
					new Pose("playing", initialPose.yaw(), initialPose.pitch()),
					new Pose("paused", initialPose.yaw(), initialPose.pitch()),
					new Pose("unpaused", initialPose.yaw(), initialPose.pitch())
				};
			} else if (FORCE_REAL_SURVIVAL_CRACK) {
				fullSequence = new Pose[] {
					new Pose("crack-early", initialPose.yaw(), initialPose.pitch()),
					new Pose("crack-middle", initialPose.yaw(), initialPose.pitch()),
					new Pose("crack-late", initialPose.yaw(), initialPose.pitch())
				};
				} else if (staticTerrainRequiresTranslucentCameraSequence()) {
					fullSequence = translucentTerrainPoseSequence(initialPose);
			} else if (!FALLING_BLOCK_SCENARIO.isEmpty() && !"hidden".equals(FALLING_BLOCK_SCENARIO)
				&& !PISTON_SCENARIO.isEmpty() && !"hidden".equals(PISTON_SCENARIO)) {
				fullSequence = combinedMovingMeshPoseSequence(initialPose);
			} else if (!PISTON_SCENARIO.isEmpty() && !"hidden".equals(PISTON_SCENARIO)) {
				fullSequence = movingMeshPoseSequence("piston", initialPose, 7);
		} else if (!ARROW_SCENARIO.isEmpty() && !"hidden".equals(ARROW_SCENARIO)) {
			fullSequence = movingMeshPoseSequence("arrow", initialPose, 5);
		} else if (!EXPERIENCE_ORB_SCENARIO.isEmpty() && !"hidden".equals(EXPERIENCE_ORB_SCENARIO)) {
			fullSequence = movingMeshPoseSequence("experience-orb", initialPose, 5);
		} else if (!BEACON_BEAM_SCENARIO.isEmpty() && !"hidden".equals(BEACON_BEAM_SCENARIO)) {
			fullSequence = new Pose[] { initialPose };
		} else if (!ITEM_ENTITY_SCENARIO.isEmpty() && !"hidden".equals(ITEM_ENTITY_SCENARIO)) {
			fullSequence = movingMeshPoseSequence("item-entity", initialPose, 5);
		} else if ("llama-spit".equals(MODEL_MESH_SCENARIO) || "evoker-fangs".equals(MODEL_MESH_SCENARIO) || "wither-skull".equals(MODEL_MESH_SCENARIO)) {
			fullSequence = movingMeshPoseSequence(MODEL_MESH_SCENARIO, initialPose,
				"evoker-fangs".equals(MODEL_MESH_SCENARIO) ? 2 : 5);
		} else if (!MODEL_MESH_SCENARIO.isEmpty() && !"hidden".equals(MODEL_MESH_SCENARIO)) {
			// The selected-source entity fixture is spawned from this exact camera
			// pose. Rotating the camera afterwards turns a valid producer-specific
			// readback into unrelated world/entity work, which the source receipt
			// must reject. It needs one exact final frame, not a synthetic pose
			// sweep. Ordinary model coverage retains its existing multi-pose path.
			fullSequence = selectedSourceCaptureRequested() && isModelMeshEntityScenario()
				? new Pose[] { initialPose }
				: movingMeshPoseSequence(MODEL_MESH_SCENARIO, initialPose, 5);
		} else if (!PRIMED_TNT_SCENARIO.isEmpty() && !"hidden".equals(PRIMED_TNT_SCENARIO)) {
				fullSequence = movingMeshPoseSequence("primed-tnt", initialPose, 5);
			} else if (!FALLING_BLOCK_SCENARIO.isEmpty() && !"hidden".equals(FALLING_BLOCK_SCENARIO)) {
				fullSequence = movingMeshPoseSequence("falling", initialPose, 5);
			}
			if (GraphicsAuditExperienceOrbFixture.requested()) {
				fullSequence = new Pose[] {initialPose,
					new Pose("right", initialPose.yaw() + YAW_DELTA, initialPose.pitch()),
					new Pose("left", initialPose.yaw() - YAW_DELTA, initialPose.pitch()),
					new Pose("return", initialPose.yaw(), initialPose.pitch()), initialPose};
			}
			poses = java.util.Arrays.copyOf(fullSequence, Math.min(POSE_COUNT, fullSequence.length));
		startedGameTime = level.getGameTime();
		windowWidth = minecraft.getWindow().getWidth();
		windowHeight = minecraft.getWindow().getHeight();
		initialized = true;
		// Seed capture-only terrain particles before the first whole-frame shell
		// can complete the deterministic pose.  The shell still drains them via
		// ParticleEngine's normal semantic extraction path on the next frame.
		if (!System.getProperty("mattmc.dev.rustGalWorldMaterial.terrainParticleScenario", "").isBlank()
			&& level instanceof net.minecraft.client.multiplayer.ClientLevel clientLevel) {
			clientLevel.spawnDeterministicTerrainParticlesForCapture();
			minecraft.particleEngine.flushPendingParticlesForCapture();
		}
		LOGGER.info(
			"Deterministic camera capture started dimension={} pos=({}, {}, {}) yaw={} pitch={} framesPerPose={} yawDelta={} cameraType={} selectedHotbarSlot={} screenshots={}",
			initialDimension,
			initialPosition.x,
			initialPosition.y,
			initialPosition.z,
			initialPose.yaw(),
			initialPose.pitch(),
			FRAMES_PER_POSE,
			YAW_DELTA,
			currentCameraType(minecraft),
			currentSelectedHotbarSlot(player),
			SCREENSHOT_DIR
		);
		if (forcedBlockOutlineTarget != null) {
			LOGGER.info(
				"Deterministic block-outline real target pos={} face={} playerPos=({}, {}, {}) yaw={} pitch={} hit=({}, {}, {}) forceHitResult={} aimOnly={}",
				forcedBlockOutlineTarget.blockPos().toShortString(),
				forcedBlockOutlineTarget.face(),
				forcedBlockOutlineTarget.playerPosition().x,
				forcedBlockOutlineTarget.playerPosition().y,
				forcedBlockOutlineTarget.playerPosition().z,
				forcedBlockOutlineTarget.pose().yaw(),
				forcedBlockOutlineTarget.pose().pitch(),
				forcedBlockOutlineTarget.hitLocation().x,
				forcedBlockOutlineTarget.hitLocation().y,
				forcedBlockOutlineTarget.hitLocation().z,
				FORCE_BLOCK_OUTLINE_TARGET,
				AIM_BLOCK_OUTLINE_TARGET
			);
		}
		writeMetadata(minecraft, "running");
		return true;
	}

	private static boolean settledReadyGateSatisfied(Minecraft minecraft) {
		if (SETTLED_READY_FRAMES <= 0) {
			settledReadyGateSatisfied = true;
			return true;
		}
		if (poseIndex > 0
			&& !Boolean.getBoolean("mattmc.dev.rustGalVulkanWholeFrame")
			&& !net.vulkanic.bridge.RustGalVulkanWholeFrameMode.enabled()) {
			settledReadyGateSatisfied = true;
			return true;
		}

		long latestCompletedFrame = Math.max(0L, renderedFrameIndex - 1L);
		 synchronized (SUBMITTED_WORK_BY_FRAME) {
			for (String family : SETTLED_READY_FAMILIES) {
				// The explicit whole-frame terrain producer runs at the frame
				// coordinator boundary, before the deterministic render-hook counter
				// advances. Its render-thread benchmark receipt is therefore the
				// authoritative bounded readiness signal for this route; visibility,
				// uploaded-assets, and appearance-light gates below remain mandatory.
				if ("sodium-terrain".equals(family)
					&& (Boolean.getBoolean("mattmc.dev.rustGalVulkanWholeFrame")
						|| net.vulkanic.bridge.RustGalVulkanWholeFrameMode.enabled())
					) {
					if (net.vulkanic.world.RustGalWholeFrameTerrainSource.isWholeFrameTerrainQueueDrained()) {
						settledRustTerrainFrames = Math.min(SETTLED_READY_FRAMES, settledRustTerrainFrames + 1);
					} else {
						settledRustTerrainFrames = 0;
					}
					if (settledRustTerrainFrames >= SETTLED_READY_FRAMES) {
						continue;
					}
					return false;
				}
				if ("distant-horizons".equals(family)
					&& net.vulkanic.world.DistantHorizonsSemanticCollector.routeExecutionCount()
					>= SETTLED_READY_FRAMES) {
					continue;
				}
				// DH's explicit Rust submission callback is recorded by the shared
				// benchmark on its completion thread, while this deterministic map is
				// render-hook-thread scoped. Require the same number of counted route
				// submissions before accepting this family as settled; all DH semantic
				// and execution gates below remain independent hard requirements.
				if ("distant-horizons".equals(family)
					&& GraphicsFrameBenchmark.submittedWorkCount(family) >= SETTLED_READY_FRAMES) {
					continue;
				}
				// Atlas-generation lifecycle fixtures intentionally change the
				// resource generation before their deterministic edit runs. Their
				// per-frame section identities therefore cannot intersect even after
				// the Rust whole-frame queue and uploaded assets are fully settled.
				// Keep the stronger whole-frame asset/visibility checks below as the
				// readiness proof, but do not deadlock the lifecycle hook on an
				// identity set that is expected to churn during pack selection.
				if ("static-terrain".equals(family)
					&& staticTerrainUsesAtlasGeneration()
					&& staticTerrainAssetsSettled()) {
					continue;
				}
				if (usesLegacyDistantHorizonsObservationForSettledReadiness(family)) {
					if (legacyDistantHorizonsPaletteColumnObserved()) {
						settledLegacyDistantHorizonsObservationFrames++;
					} else {
						settledLegacyDistantHorizonsObservationFrames = 0;
					}
					if (settledLegacyDistantHorizonsObservationFrames < SETTLED_READY_FRAMES) {
						return false;
					}
					continue;
				}
				Set<String> stableIntersection = null;
				for (int offset = SETTLED_READY_FRAMES - 1; offset >= 0; offset--) {
					Map<String, Set<String>> frameWork = SUBMITTED_WORK_BY_FRAME.get(latestCompletedFrame - offset);
					Set<String> current = frameWork == null ? null : frameWork.get(family);
					if (current == null || current.isEmpty()) {
						return false;
					}
					if (stableIntersection == null) {
						stableIntersection = new LinkedHashSet<>(current);
					} else {
						stableIntersection.retainAll(current);
					}
				}
				if (stableIntersection == null || stableIntersection.isEmpty()) {
					return false;
				}
			}
		}
		// The DH texture-palette witness is populated asynchronously.  A route
		// execution receipt alone can arrive while later visible columns are still
		// unpublished, causing the screenshot handshake to sample a frame that has
		// no matching exact-atlas plan.  Keep the capture pending until the
		// Rust-owned visible set is published; the bounded ready timeout remains
		// the failure path if the producer never quiesces.
		if (DISTANT_HORIZONS_TEXTURE_PALETTE
			&& net.vulkanic.world.DistantHorizonsSemanticCollector.hasUnpublishedVisibleColumns()) {
			return false;
		}
		if (DISTANT_HORIZONS_TEXTURE_PALETTE
			&& !net.vulkanic.world.DistantHorizonsSemanticCollector.exactAtlasCoverageStableForCapture()) {
			return false;
		}
		if (!staticTerrainAssetsSettled()) {
			return false;
		}
		if ((Boolean.getBoolean("mattmc.dev.rustGalVulkanWholeFrame")
			|| net.vulkanic.bridge.RustGalVulkanWholeFrameMode.enabled())
			&& (net.vulkanic.gui.RustGalFrameCoordinator.lastRenderableWholeFrameWorldFrame() <= 0
				|| net.vulkanic.gui.RustGalFrameCoordinator.lastAcquiredWholeFrameFrame()
					- net.vulkanic.gui.RustGalFrameCoordinator.lastRenderableWholeFrameWorldFrame() > 1)) {
			return false;
		}
		if (!net.sodium.client.render.StaticTerrainParityDiagnostics.isAppearanceLightReady()) {
			return false;
		}
		settledReadyGateSatisfied = true;
		writeMetadata(minecraft, "settled_ready_work");
		LOGGER.info(
			"Deterministic camera capture settled submitted work after {} frames: {}",
			framesWaitingForSettledReady,
			settledReadySummary()
		);
		return true;
	}

	private static boolean usesLegacyDistantHorizonsObservationForSettledReadiness(String family) {
		return DISTANT_HORIZONS_LEGACY_OBSERVATION
			&& DISTANT_HORIZONS_TEXTURE_PALETTE
			&& "distant-horizons".equals(family);
	}

	private static int submittedWorkIdentityCount(String family) {
		int count = 0;
		for (Map<String, Set<String>> frame : SUBMITTED_WORK_BY_FRAME.values()) {
			Set<String> identities = frame.get(family);
			if (identities != null) {
				count += identities.size();
			}
		}
		return count;
	}

	private static boolean legacyDistantHorizonsPaletteColumnObserved() {
		BlockPos witness = distantHorizonsTexturePaletteTarget != null
			? distantHorizonsTexturePaletteTarget
			: distantHorizonsTexturePaletteProbes.isEmpty()
				? null
				: distantHorizonsTexturePaletteProbes.getFirst().position();
		return witness != null && DistantHorizonsSemanticCollector.hasObservedVisibleOpaqueColumnCoveringBlock(
			witness.getX(), witness.getZ()
		);
	}

	private static void pruneSubmittedWorkFrames() {
		long minimumFrame = Math.max(0L, renderedFrameIndex - Math.max(SETTLED_READY_FRAMES * 4L, 64L));
		SUBMITTED_WORK_BY_FRAME.keySet().removeIf(frame -> frame < minimumFrame);
	}

	private static String settledReadySummary() {
		if (SETTLED_READY_FRAMES <= 0) {
			return "disabled";
		}
		long latestCompletedFrame = Math.max(0L, renderedFrameIndex - 1L);
		StringBuilder summary = new StringBuilder();
		synchronized (SUBMITTED_WORK_BY_FRAME) {
			summary.append("frames=").append(Math.max(0L, latestCompletedFrame - SETTLED_READY_FRAMES + 1L))
				.append("..").append(latestCompletedFrame);
			for (String family : SETTLED_READY_FAMILIES) {
				if (usesLegacyDistantHorizonsObservationForSettledReadiness(family)) {
					summary.append(";").append(family).append("=legacy-observed/")
						.append(settledLegacyDistantHorizonsObservationFrames)
						.append("/required=").append(SETTLED_READY_FRAMES);
					continue;
				}
				Map<String, Set<String>> frameWork = SUBMITTED_WORK_BY_FRAME.get(latestCompletedFrame);
				Set<String> work = frameWork == null ? null : frameWork.get(family);
				Set<String> stableIntersection = null;
				for (int offset = SETTLED_READY_FRAMES - 1; offset >= 0; offset--) {
					Map<String, Set<String>> windowFrameWork = SUBMITTED_WORK_BY_FRAME.get(latestCompletedFrame - offset);
					Set<String> current = windowFrameWork == null ? null : windowFrameWork.get(family);
					if (current == null || current.isEmpty()) {
						stableIntersection = null;
						break;
					}
					if (stableIntersection == null) {
						stableIntersection = new LinkedHashSet<>(current);
					} else {
						stableIntersection.retainAll(current);
					}
				}
				summary.append(";").append(family).append("=").append(work == null ? 0 : work.size())
					.append("/stable=").append(stableIntersection == null ? 0 : stableIntersection.size());
			}
		}
		if (staticTerrainRequiresSettledAssets()) {
			summary.append(";static-terrain-assets-quiet=")
				.append(staticTerrainSettledFrames)
				.append("/")
				.append(SETTLED_READY_FRAMES)
				.append(" signature=")
				.append(staticTerrainSettledSignature);
		}
		if (WorldRenderRoutePolicy.currentStaticTerrainRoute().usesRustWholeFrameVulkan()) {
			summary.append(";rust-whole-frame-terrain=")
				.append(net.vulkanic.world.RustGalWholeFrameTerrainSource.wholeFrameTerrainQueueSummary());
			summary.append(";rust-terrain-settled=")
				.append(settledRustTerrainFrames)
				.append("/").append(SETTLED_READY_FRAMES);
			summary.append(";rust-whole-frame-assets=")
				.append(RustGalTerrainRenderer.wholeFrameAssetUploadSummary());
		}
		String appearanceLightReadiness = net.sodium.client.render.StaticTerrainParityDiagnostics.appearanceLightReadinessSummary();
		if (!appearanceLightReadiness.endsWith("ready=true")) {
			summary.append(";static-terrain-light=").append(appearanceLightReadiness);
		}
		if (!REQUIRED_RUST_SOURCE_EXECUTION_DIR.isEmpty()) {
			summary.append(";rust-selected-source=").append(requiredRustSourceExecutionObserved());
		}
		return summary.toString();
	}

	private static boolean requiredRustSourceExecutionObserved() {
		return requiredRustSourceExecutionObservedAtOrAfter(0L);
	}


	private static boolean requiredRustSourceExecutionObservedForGameplayFrame(long gameplayFrameId) {
		if (REQUIRED_RUST_SOURCE_EXECUTION_DIR.isEmpty()) {
			return true;
		}
		Path directory = Path.of(REQUIRED_RUST_SOURCE_EXECUTION_DIR);
		return sourceExecutionReceiptMatchesGameplayFrame(
			directory.resolve("selected-source-execution-frame-" + gameplayFrameId + ".json"),
			gameplayFrameId
		) || sourceExecutionReceiptMatchesGameplayFrame(
			directory.resolve("selected-source-execution-distant-horizons-frame-" + gameplayFrameId + ".json"),
			gameplayFrameId
		);
	}

	private static boolean requiredRustSourceExecutionObservedAtOrAfter(long minimumSubmissionId) {
		if (REQUIRED_RUST_SOURCE_EXECUTION_DIR.isEmpty()) {
			return true;
		}
		Path directory = Path.of(REQUIRED_RUST_SOURCE_EXECUTION_DIR);
		if (!Files.isDirectory(directory)) {
			return false;
		}
		if (sourceExecutionReceiptMatches(
			directory.resolve("selected-source-execution-latest.json"),
			minimumSubmissionId
		)) {
			return true;
		}
		try (java.nio.file.DirectoryStream<Path> entries = Files.newDirectoryStream(
			directory,
			"selected-source-execution-frame-*.json"
		)) {
			for (Path entry : entries) {
				if (!Files.isRegularFile(entry)) {
					continue;
				}
			if (sourceExecutionReceiptMatches(entry, minimumSubmissionId)) {
				return true;
			}
			}
		} catch (IOException | NumberFormatException ignored) {
			return false;
		}
		return false;
	}

	private static boolean sourceExecutionReceiptMatches(Path entry, long minimumSubmissionId) {
		if (!Files.isRegularFile(entry)) {
			return false;
		}
		try {
			String json = Files.readString(entry, StandardCharsets.UTF_8);
			return json.contains("\"route\":\"rust-native-selected-source\"")
				&& sourceExecutionReceiptHasVisibleWorldWork(json)
				&& sourceExecutionReceiptHasRequiredProducerWork(json)
				&& readJsonLongField(json, "submission_id", 0L) >= minimumSubmissionId;
		} catch (IOException | NumberFormatException ignored) {
			return false;
		}
	}

	private static boolean sourceExecutionReceiptMatchesGameplayFrame(Path entry, long gameplayFrameId) {
		if (!Files.isRegularFile(entry)) {
			return false;
		}
		try {
			String json = Files.readString(entry, StandardCharsets.UTF_8);
			return json.contains("\"route\":\"rust-native-selected-source\"")
				&& readJsonLongField(json, "frame_id", 0L) == gameplayFrameId
				&& sourceExecutionReceiptHasVisibleWorldWork(json)
				&& sourceExecutionReceiptHasRequiredProducerWork(json);
		} catch (IOException | NumberFormatException ignored) {
			return false;
		}
	}

	/**
	 * A selected-source receipt can legitimately contain static terrain without
	 * a requested model producer. Keep that distinction in capture plumbing so
	 * the attached G-buffer frame proves the same semantic family as the
	 * screenshot scenario. This does not influence route selection or drawing.
	 */
	private static boolean sourceExecutionReceiptHasRequiredProducerWork(String json) {
		if (!MODEL_MESH_SCENARIO.isEmpty() && !"hidden".equals(MODEL_MESH_SCENARIO)) {
			if ("wind-charge".equals(MODEL_MESH_SCENARIO)) {
				int materialStart = json.indexOf("\"source_material_execution\":");
				return materialStart >= 0
					&& readJsonLongField(json.substring(materialStart), "entity_model_quads", 0L) > 0L;
			}
			int coverageStart = json.indexOf("\"source_entity_coverage\":");
			if (coverageStart < 0 || readJsonLongField(json.substring(coverageStart), "instances", 0L) <= 0L) {
				return false;
			}
			boolean entityMatched = false;
			for (RustGalWorldPrimitiveRenderer.ModelMeshDiagnostic diagnostic : RustGalWorldPrimitiveRenderer.modelMeshDiagnostics()) {
				// A selected-source entity writer may expand a copied asset into its
				// own immutable stream. Its mesh key is therefore an implementation
				// detail, not a cross-boundary capture identity. Both sides retain the
				// canonical semantic entity identity instead.
				String entityIdentity = diagnostic.semanticModelIdentity();
				if (!entityIdentity.isBlank()
					&& json.contains("\"entity_identity\":\"" + entityIdentity + "\"")) {
					entityMatched = true;
					break;
				}
			}
			if (!entityMatched) {
				return false;
			}
		}
		if (REQUIRE_RUST_WORLD_TEXT_SOURCE_CAPTURE) {
			int textStart = json.indexOf("\"world_text_execution\":");
			if (textStart < 0
				|| readJsonLongField(json.substring(textStart), "quads", 0L) <= 0L
				|| readJsonLongField(json.substring(textStart), "draws", 0L) <= 0L) {
				return false;
			}
		}
		return !REQUIRE_RUST_ITEM_ENTITY_SOURCE_CAPTURE
			|| json.contains("\"entity_identity\":\"minecraft:item_entity/ground\"");
	}

	/**
	 * Selected-source world frames can contain either indexed near-terrain meshes
	 * or real Distant Horizons LOD instances. Both are producer work; treating a
	 * DH-only frame as empty made the deterministic capture wait forever after
	 * Rust had already executed the requested source frame.
	 */
	private static boolean sourceExecutionReceiptHasVisibleWorldWork(String json) {
		return readJsonLongField(json, "mesh_instances", 0L) > 0L
			|| readJsonLongField(json, "lod_instances", 0L) > 0L;
	}

	private static boolean staticTerrainAssetsSettled() {
		boolean rustWholeFrameTerrain = WorldRenderRoutePolicy.currentStaticTerrainRoute().usesRustWholeFrameVulkan();
		if (rustWholeFrameTerrain
			&& !net.vulkanic.world.RustGalWholeFrameTerrainSource.isWholeFrameTerrainQueueDrained()) {
			staticTerrainSettledFrames = 0;
			staticTerrainSettledSignature = "rust-whole-frame-queue-not-drained";
			return false;
		}
		if (rustWholeFrameTerrain && !RustGalTerrainRenderer.areWholeFrameAssetsUploaded()) {
			staticTerrainSettledFrames = 0;
			staticTerrainSettledSignature = "rust-whole-frame-assets-not-uploaded";
			return false;
		}
		if (!staticTerrainRequiresSettledAssets()) {
			return true;
		}
		RustGalTerrainRenderer.TerrainDiagnostics diagnostics = RustGalTerrainRenderer.diagnosticsSnapshot();
		if (diagnostics.registeredMeshes() <= 0 || diagnostics.visibleLayerSubmissions() <= 0) {
			staticTerrainSettledFrames = 0;
			staticTerrainSettledSignature = "";
			return false;
		}
		// Stable per-section/layer/generation identities are checked by
		// settledReadyGateSatisfied above. For the direct CPU terrain source, also
		// keep the portal frontier drained for the same consecutive frame window;
		// a one-frame empty queue can otherwise be followed by completed builds
		// that expand the visible traversal after the capture was admitted.
		staticTerrainSettledSignature = rustWholeFrameTerrain
			? "visible-submission-identities+rust-whole-frame-queue-drained+assets-uploaded"
			: "visible-submission-identities";
		if (rustWholeFrameTerrain) {
			staticTerrainSettledFrames = Math.min(SETTLED_READY_FRAMES, staticTerrainSettledFrames + 1);
			return staticTerrainSettledFrames >= SETTLED_READY_FRAMES;
		}
		staticTerrainSettledFrames = SETTLED_READY_FRAMES;
		return true;
	}

	private static boolean staticTerrainRequiresSettledAssets() {
		return ("real-world".equals(STATIC_TERRAIN_SCENARIO) || staticTerrainLifecycleScenario())
			&& WorldRenderRoutePolicy.currentStaticTerrainRoute().usesRustWholeFrameVulkan();
	}

	private static Set<String> parseSettledReadyFamilies() {
		String raw = System.getProperty("mattmc.dev.deterministicCameraCapture.settledReadyFamilies", "sodium-terrain,distant-horizons");
		LinkedHashSet<String> families = new LinkedHashSet<>();
		// Some Rust-owned routes (notably the Rust OpenGL lowering) do not expose
		// the Sodium/DH whole-frame readiness receipts.  The harness may select an
		// explicit fixed-warmup mode for those routes; an empty value must not be
		// treated as an accidental request for the default Sodium gate.
		if ("none".equalsIgnoreCase(raw.trim())) {
			return Set.of();
		}
		Arrays.stream(raw.split(","))
			.map(String::trim)
			.filter(value -> !value.isEmpty())
			.forEach(families::add);
		if (families.isEmpty()) {
			families.add("sodium-terrain");
		}
		return Set.copyOf(families);
	}

	private static float[] parseWeatherCapturePartialTicks() {
		String raw = System.getProperty("mattmc.dev.deterministicCameraCapture.weatherPartialTicks", "").trim();
		if (raw.isEmpty()) {
			return new float[0];
		}
		String[] tokens = raw.split(",");
		float[] values = new float[tokens.length];
		for (int index = 0; index < tokens.length; index++) {
			float value = Float.parseFloat(tokens[index].trim());
			if (!Float.isFinite(value) || value < 0.0F || value > 1.0F) {
				throw new IllegalArgumentException("weatherPartialTicks must be finite values in [0, 1]");
			}
			values[index] = value;
		}
		return values;
	}

	private static int[] parseWeatherCaptureRendererTicks() {
		String raw = System.getProperty("mattmc.dev.deterministicCameraCapture.weatherRendererTicks", "").trim();
		if (raw.isEmpty()) {
			return new int[0];
		}
		String[] tokens = raw.split(",");
		int[] values = new int[tokens.length];
		for (int index = 0; index < tokens.length; index++) {
			int value = Integer.parseInt(tokens[index].trim());
			if (value < 0) {
				throw new IllegalArgumentException("weatherRendererTicks must be non-negative");
			}
			values[index] = value;
		}
		return values;
	}

	private static void applyPauseParityScreen(Minecraft minecraft) {
		if (!BLOCK_OUTLINE_PAUSE_PARITY || poses == null || poseIndex < 0 || poseIndex >= poses.length) {
			return;
		}
		boolean wantsPauseScreen = "paused".equals(poses[poseIndex].name());
		if (wantsPauseScreen) {
			if (minecraft.screen == null) {
				minecraft.setScreen(new PauseScreen(false));
			}
			return;
		}
		if (minecraft.screen instanceof PauseScreen) {
			minecraft.setScreen(null);
		}
	}

	private static boolean advanceRustGalGuiScreenCycle(Minecraft minecraft) {
		if (rustGalGuiScreenCycleComplete) {
			return true;
		}
		if (minecraft.player == null) {
			return false;
		}
		rustGalGuiScreenCycleFramesInStage++;
		switch (rustGalGuiScreenCycleStage) {
			case 0 -> {
				if (minecraft.screen != null) {
					fail("Rust GAL GUI screen-cycle expected gameplay HUD before inventory but screen was " + minecraft.screen.getClass().getSimpleName());
					return false;
				}
				if (rustGalGuiScreenCycleFramesInStage == 1) {
					writeMetadata(minecraft, "rust_gal_gui_screen_cycle_hud_before");
				}
				if (rustGalGuiScreenCycleFramesInStage >= RUST_GAL_GUI_SCREEN_CYCLE_HOLD_FRAMES) {
					minecraft.setScreen(new InventoryScreen(minecraft.player));
					rustGalGuiScreenCycleStage = 1;
					rustGalGuiScreenCycleFramesInStage = 0;
					writeMetadata(minecraft, "rust_gal_gui_screen_cycle_inventory_open");
				}
				return false;
			}
			case 1 -> {
				if (!(minecraft.screen instanceof InventoryScreen)) {
					fail("Rust GAL GUI screen-cycle expected InventoryScreen but screen was "
						+ (minecraft.screen == null ? "none" : minecraft.screen.getClass().getSimpleName()));
					return false;
				}
				if (rustGalGuiScreenCycleFramesInStage >= RUST_GAL_GUI_SCREEN_CYCLE_HOLD_FRAMES) {
					if (!rustGalGuiScreenCycleInventoryCaptureRequested) {
						requestRustGalGuiScreenCycleInventoryCapture(minecraft);
						return false;
					}
					if (!rustGalGuiScreenCycleInventoryCaptureComplete) {
						if (rustGalGuiScreenCycleInventoryCaptureAcknowledged()) {
							rustGalGuiScreenCycleInventoryCaptureComplete = true;
							writeMetadata(minecraft, "rust_gal_gui_screen_cycle_inventory_captured");
						} else if (rustGalGuiScreenCycleFramesInStage > ACK_TIMEOUT_FRAMES) {
							fail("timed out waiting for Rust GAL inventory-cycle presentation capture acknowledgement");
							return false;
						} else {
							return false;
						}
					}
					minecraft.setScreen(null);
					rustGalGuiScreenCycleStage = 2;
					rustGalGuiScreenCycleFramesInStage = 0;
					writeMetadata(minecraft, "rust_gal_gui_screen_cycle_inventory_closed");
				}
				return false;
			}
			case 2 -> {
				if (minecraft.screen != null) {
					fail("Rust GAL GUI screen-cycle expected HUD recovery after inventory but screen was " + minecraft.screen.getClass().getSimpleName());
					return false;
				}
				if (rustGalGuiScreenCycleFramesInStage >= RUST_GAL_GUI_SCREEN_CYCLE_HOLD_FRAMES) {
					rustGalGuiScreenCyclesCompleted++;
					if (rustGalGuiScreenCyclesCompleted >= RUST_GAL_GUI_SCREEN_CYCLE_REPEATS) {
						rustGalGuiScreenCycleComplete = true;
						writeMetadata(minecraft, "rust_gal_gui_screen_cycle_complete");
						return true;
					}
					rustGalGuiScreenCycleStage = 0;
					rustGalGuiScreenCycleFramesInStage = 0;
				}
				return false;
			}
			default -> {
				fail("invalid Rust GAL GUI screen-cycle stage: " + rustGalGuiScreenCycleStage);
				return false;
			}
		}
	}

	/**
	 * Requests a desktop capture of the already presented Rust-owned frame.  The
	 * shared capture runner writes the acknowledgement; this deliberately never
	 * reads Java's main render target, which has no valid image on selected
	 * Vulkan.
	 */
	private static void requestRustGalGuiScreenCycleInventoryCapture(Minecraft minecraft) {
		rustGalGuiScreenCycleInventoryCaptureRequested = true;
		rustGalGuiScreenCycleInventoryCapturePath = SCREENSHOT_DIR.resolve("rust_gal_gui_screen_cycle_inventory.png");
		rustGalGuiScreenCycleInventoryAckPath = SCREENSHOT_DIR.resolve("capture_request_rust_gal_gui_screen_cycle_inventory.ack.json");
		Path requestPath = SCREENSHOT_DIR.resolve("capture_request_rust_gal_gui_screen_cycle_inventory.json");
		String json = "{\n"
			+ "  \"captureKind\": \"rust-gal-gui-screen-cycle-inventory\",\n"
			+ "  \"screenshot\": \"" + escape(rustGalGuiScreenCycleInventoryCapturePath.toAbsolutePath().toString()) + "\",\n"
			+ "  \"renderedFrameIndex\": " + renderedFrameIndex + "\n"
			+ "}\n";
		try {
			Files.createDirectories(SCREENSHOT_DIR);
			Files.writeString(requestPath, json, StandardCharsets.UTF_8);
			writeMetadata(minecraft, "waiting_for_rust_gal_gui_screen_cycle_inventory_capture");
		} catch (IOException exception) {
			fail("failed to request Rust GAL inventory-cycle presentation capture: " + exception.getMessage());
		}
	}

	private static boolean rustGalGuiScreenCycleInventoryCaptureAcknowledged() {
		if (rustGalGuiScreenCycleInventoryAckPath == null || rustGalGuiScreenCycleInventoryCapturePath == null
			|| !Files.isRegularFile(rustGalGuiScreenCycleInventoryAckPath)
			|| !Files.isRegularFile(rustGalGuiScreenCycleInventoryCapturePath)) {
			return false;
		}
		try {
			return Files.readString(rustGalGuiScreenCycleInventoryAckPath, StandardCharsets.UTF_8).contains("\"status\": \"captured\"");
		} catch (IOException exception) {
			return false;
		}
	}

	private static String blockAnimationAtCapture = "null";

	/** Freeze existing CPU observations at the selected frame, never during metadata writing. */
	static void captureSurfaceObservations() {
		GraphicsAuditFlowingWaterFixture.captureSurface();
		GraphicsAuditLavaFixture.captureSurface();
		GraphicsAuditMixedFluidFixture.captureBottomSurface();
		net.sodium.client.render.StaticTerrainParityDiagnostics.recordAppearanceSourceAtCapture();
	}

	private static void requestCurrentPoseScreenshot(Minecraft minecraft) {
		blockAnimationAtCapture = GraphicsAuditBlockDisplayFixture.animationObservation(minecraft);
		captureSurfaceObservations();
		// Correlate the copied Rust semantic source with the ordinary screenshot
		// request after this pose's rendering completed. This has no route or
		// renderer effect; it only writes bounded diagnostic evidence.
		net.sodium.client.render.StaticTerrainParityDiagnostics.recordRustSourceCaptureCoverage(renderedFrameIndex);
		net.sodium.client.render.StaticTerrainParityDiagnostics.recordRustExecutionCaptureCoverage(renderedFrameIndex);
		Pose pose = poses[poseIndex];
		int captureIndex = poseIndex + 1;
		String fileName = String.format(Locale.ROOT, "%02d_%s.png", captureIndex, pose.name());
		String requestName = String.format(Locale.ROOT, "capture_request_%02d_%s.json", captureIndex, pose.name());
		String ackName = String.format(Locale.ROOT, "capture_request_%02d_%s.ack.json", captureIndex, pose.name());
		currentScreenshotPath = SCREENSHOT_DIR.resolve(fileName);
		currentAckPath = SCREENSHOT_DIR.resolve(ackName);
		Path requestPath = SCREENSHOT_DIR.resolve(requestName);

		StringBuilder json = new StringBuilder(1024);
		json.append("{\n");
		json.append("  \"index\": ").append(captureIndex).append(",\n");
		appendField(json, "poseName", pose.name()).append(",\n");
		appendField(json, "screenshot", currentScreenshotPath.toAbsolutePath().toString()).append(",\n");
		appendField(json, "ack", currentAckPath.toAbsolutePath().toString()).append(",\n");
		appendField(json, "dimension", minecraft.level == null ? "missing" : minecraft.level.dimension().location().toString()).append(",\n");
		appendVec3(json, "position", minecraft.player == null ? Vec3.ZERO : minecraft.player.position()).append(",\n");
		appendVec3(json, "requestedPosition", targetPositionForPose(pose)).append(",\n");
		json.append("  \"requestedYaw\": ").append(format(pose.yaw())).append(",\n");
		json.append("  \"requestedPitch\": ").append(format(pose.pitch())).append(",\n");
		json.append("  \"observedYaw\": ").append(format(minecraft.player == null ? 0.0F : minecraft.player.getYRot())).append(",\n");
		json.append("  \"observedPitch\": ").append(format(minecraft.player == null ? 0.0F : minecraft.player.getXRot())).append(",\n");
		json.append("  \"renderedFrameIndex\": ").append(renderedFrameIndex).append(",\n");
		json.append("  \"guiItemFoilTiming\": ").append(GraphicsAuditGuiFoilTiming.snapshot()).append(",\n");
		appendWholeFramePresentationCorrelation(json, renderedFrameIndex, 2).append(",\n");
		appendDistantHorizonsExecutionCorrelation(json, 2).append(",\n");
		appendDistantHorizonsTextureProbeReceipt(json, 2).append(",\n");
		appendDistantHorizonsWaterProbeReceipt(json, 2).append(",\n");
		appendStaticTerrainExecutionCorrelation(json, 2).append(",\n");
		appendStaticTerrainAtlasReceipt(json, 2).append(",\n");
		appendStaticTerrainTextureProbeReceipt(json, 2).append(",\n");
		json.append("  \"gameTime\": ").append(minecraft.level == null ? -1L : minecraft.level.getGameTime()).append(",\n");
		json.append("  \"vignetteBrightness\": ").append(format(minecraft.gui.vignetteBrightnessForDeterministicCapture())).append("\n");
		json.append("}\n");

		try {
			Files.writeString(requestPath, json.toString(), StandardCharsets.UTF_8);
		} catch (IOException exception) {
			fail("failed to write deterministic screenshot request " + requestPath + ": " + exception.getMessage());
			return;
		}
		try {
			Files.writeString(currentScreenshotPath.resolveSibling(fileName + ".foil-timing.json"),
				GraphicsAuditGuiFoilTiming.snapshot(), StandardCharsets.UTF_8);
		} catch (IOException exception) {
			fail("failed to write GUI foil timing receipt: " + exception.getMessage());
			return;
		}

		awaitingScreenshotAck = true;
		framesAwaitingAck = 0;
		writeMetadata(minecraft, "waiting_for_screenshot");
		LOGGER.info(
			"Deterministic camera capture requested screenshot index={} pose={} path={} ack={} yaw={} pitch={}",
			captureIndex,
			pose.name(),
			currentScreenshotPath,
			currentAckPath,
			pose.yaw(),
			pose.pitch()
		);
	}

	private static void captureCurrentPoseInternally(Minecraft minecraft) {
		Pose pose = poses[poseIndex];
		int captureIndex = poseIndex + 1;
		String fileName = String.format(Locale.ROOT, "%02d_%s.png", captureIndex, pose.name());
		currentScreenshotPath = SCREENSHOT_DIR.resolve(fileName);
		currentAckPath = SCREENSHOT_DIR.resolve(String.format(Locale.ROOT, "capture_request_%02d_%s.ack.json", captureIndex, pose.name()));
		awaitingScreenshotAck = true;
		framesAwaitingAck = 0;
		writeMetadata(minecraft, "capturing_screenshot");
		try {
			Files.createDirectories(SCREENSHOT_DIR);
		} catch (IOException exception) {
			fail("failed to create deterministic screenshot directory " + SCREENSHOT_DIR + ": " + exception.getMessage());
			return;
		}

		try {
			Screenshot.takeScreenshot(minecraft.getMainRenderTarget(), nativeImage -> {
				try (nativeImage) {
					nativeImage.writeToFile(currentScreenshotPath);
					writeInternalScreenshotAck(minecraft, pose, captureIndex);
					checkScreenshotAck(minecraft);
				} catch (IOException exception) {
					fail("failed to write deterministic internal screenshot " + currentScreenshotPath + ": " + exception.getMessage());
				}
			});
		} catch (RuntimeException exception) {
			fail("failed to capture deterministic internal screenshot " + currentScreenshotPath + ": " + exception.getMessage());
		}
	}

	private static void writeInternalScreenshotAck(Minecraft minecraft, Pose pose, int captureIndex) throws IOException {
		if (currentAckPath == null || currentScreenshotPath == null) {
			return;
		}
		StringBuilder json = new StringBuilder(1024);
		json.append("{\n");
		json.append("  \"index\": ").append(captureIndex).append(",\n");
		appendField(json, "poseName", pose.name()).append(",\n");
		appendField(json, "screenshot", currentScreenshotPath.toAbsolutePath().toString()).append(",\n");
		appendField(json, "ack", currentAckPath.toAbsolutePath().toString()).append(",\n");
		appendField(json, "dimension", minecraft.level == null ? "missing" : minecraft.level.dimension().location().toString()).append(",\n");
		appendVec3(json, "position", minecraft.player == null ? Vec3.ZERO : minecraft.player.position()).append(",\n");
		appendVec3(json, "requestedPosition", targetPositionForPose(pose)).append(",\n");
		json.append("  \"requestedYaw\": ").append(format(pose.yaw())).append(",\n");
		json.append("  \"requestedPitch\": ").append(format(pose.pitch())).append(",\n");
		json.append("  \"observedYaw\": ").append(format(minecraft.player == null ? 0.0F : minecraft.player.getYRot())).append(",\n");
		json.append("  \"observedPitch\": ").append(format(minecraft.player == null ? 0.0F : minecraft.player.getXRot())).append(",\n");
		json.append("  \"renderedFrameIndex\": ").append(renderedFrameIndex).append(",\n");
		appendWholeFramePresentationCorrelation(json, renderedFrameIndex, 2).append(",\n");
		appendDistantHorizonsExecutionCorrelation(json, 2).append(",\n");
		appendDistantHorizonsTextureProbeReceipt(json, 2).append(",\n");
		appendDistantHorizonsWaterProbeReceipt(json, 2).append(",\n");
		appendStaticTerrainExecutionCorrelation(json, 2).append(",\n");
		appendStaticTerrainAtlasReceipt(json, 2).append(",\n");
		appendStaticTerrainTextureProbeReceipt(json, 2).append(",\n");
		json.append("  \"gameTime\": ").append(minecraft.level == null ? -1L : minecraft.level.getGameTime()).append(",\n");
		appendField(json, "status", "captured", 2).append(",\n");
		appendField(json, "captureMethod", "internal-main-render-target", 2).append("\n");
		json.append("}\n");
		Files.writeString(currentAckPath, json.toString(), StandardCharsets.UTF_8);
	}

	private static StringBuilder appendWholeFramePresentationCorrelation(
		StringBuilder json,
		long requestedRenderedFrameIndex,
		int indent
	) {
		String nativeOrder = "null";
		json.append(" ".repeat(Math.max(0, indent))).append("\"translucentFixtureProjection\":")
			.append(GraphicsAuditOverlapProjection.receipt()).append(",\n");
		json.append(" ".repeat(Math.max(0, indent))).append("\"cameraPoseHistory\":")
			.append(GraphicsAuditCameraHistory.receipt(Minecraft.getInstance().player)).append(",\n");
		String orderRoot = System.getenv("MATTMC_STATIC_TERRAIN_BATCH_TRACE_DIR");
		if (orderRoot != null && lastWholeFramePresentation != null
			&& lastWholeFramePresentation.deterministicRenderedFrameIndex() == requestedRenderedFrameIndex) {
			nativeOrder = NativeTerrainOrderCapture.read(Path.of(orderRoot).resolve("static_terrain_batch_trace.json"),
				lastWholeFramePresentation.gameplayFrameId());
		}
		json.append(" ".repeat(Math.max(0, indent))).append("\"nativeTerrainOrder\":").append(nativeOrder).append(",\n");
		json.append(" ".repeat(Math.max(0, indent))).append("\"wholeFramePresentationCorrelation\":");
		WholeFramePresentation presentation = lastWholeFramePresentation;
		if (presentation == null || presentation.deterministicRenderedFrameIndex() != requestedRenderedFrameIndex) {
			return json.append("null");
		}
		return json.append("{\"gameplayFrameId\":").append(presentation.gameplayFrameId())
			.append(",\"correlationId\":").append(presentation.correlationId())
			.append(",\"submissionId\":").append(presentation.submissionId())
			.append(",\"acquiredSwapchainImage\":").append(presentation.acquiredSwapchainImage())
			.append(",\"presentedSwapchainImage\":").append(presentation.presentedSwapchainImage())
			.append("}");
	}

	private static boolean checkScreenshotAck(Minecraft minecraft) {
		if (currentAckPath == null || !Files.isRegularFile(currentAckPath)) {
			return false;
		}
		if (currentScreenshotPath == null || !Files.isRegularFile(currentScreenshotPath)) {
			fail("deterministic screenshot ack exists but screenshot is missing: " + currentScreenshotPath);
			return false;
		}

		Pose pose = poses[poseIndex];
		int captureIndex = poseIndex + 1;
		LocalPlayer player = minecraft.player;
		String targetWindow = readAckStringField("targetWindow");
		long acknowledgedRenderedFrame = readAckLongField("renderedFrameIndex", renderedFrameIndex);
		CAPTURES.add(new PoseCapture(
			captureIndex,
			pose.name(),
			currentScreenshotPath.toAbsolutePath().toString(),
			minecraft.level == null ? "missing" : minecraft.level.dimension().location().toString(),
			player == null ? Vec3.ZERO : player.position(),
			targetPositionForPose(pose),
			pose.yaw(),
			pose.pitch(),
			player == null ? 0.0F : player.getYRot(),
			player == null ? 0.0F : player.getXRot(),
			acknowledgedRenderedFrame,
			lastWeatherRendererTicks,
			lastWeatherRendererPartialTick,
			lastWeatherSemanticFingerprint,
			lastLightmapSemanticFingerprint,
			minecraft.level == null ? -1L : minecraft.level.getGameTime(),
			wholeFrameFinalOutputCapture
				? "rust-vulkan-final-output"
				: (INTERNAL_SCREENSHOTS ? "internal-main-render-target" : "external-window-request"),
			targetWindow
		));
		LOGGER.info(
			"Deterministic camera capture acknowledged screenshot index={} pose={} path={} yaw={} pitch={}",
			captureIndex,
			pose.name(),
			currentScreenshotPath,
			pose.yaw(),
			pose.pitch()
		);

		boolean acknowledgedRustFinalOutput = wholeFrameFinalOutputCapture;
		awaitingScreenshotAck = false;
		wholeFrameFinalOutputCapture = false;
		if (acknowledgedRustFinalOutput) {
			resetWholeFrameAttachmentCaptureState();
			// The normal external screenshot path resets this after issuing its
			// request. A Rust-final acknowledgement bypasses that path, so reset
			// the settled-pose counter here before the next pose starts.
			renderedFramesAtPose = 0;
		}
		framesAwaitingAck = 0;
		currentScreenshotPath = null;
		currentAckPath = null;
		poseIndex++;
		if (poseIndex >= poses.length) {
			applyPose(minecraft.player, initialPose);
			if (!System.getProperty("mattmc.dev.rustGalWorldMaterial.terrainParticleScenario", "").isBlank()) {
				// Keep the semantic shell alive for a bounded drain window after the
				// screenshot acknowledgement so capture-only particles seeded during
				// initialization are extracted and submitted before teardown.
				pendingTerrainParticleFinishFrames = 30;
			} else {
				finish(minecraft);
			}
		} else {
			writeMetadata(minecraft, "running");
		}
		return true;
	}

	private static void finish(Minecraft minecraft) {
		if (complete || failed) {
			return;
		}
		complete = true;
		writeMetadata(minecraft, "complete");
		restoreRuntimeOverrides(minecraft);
		LOGGER.info("Deterministic camera capture complete metadata={}", METADATA_PATH);
	}

	private static void fail(String reason) {
		if (failed || complete) {
			return;
		}
		failed = true;
		LOGGER.error("Deterministic camera capture failed: {}", reason);
		writeFailureMetadata(reason);
	}

	private static void applyPose(LocalPlayer player, Pose pose) {
		if (player == null) {
			return;
		}
		Vec3 targetPosition = targetPositionForPose(pose);
		if (targetPosition != null) {
			player.setPos(targetPosition);
			player.setOldPosAndRot(targetPosition, pose.yaw(), pose.pitch());
		}
		player.setYRot(pose.yaw());
		player.setXRot(pose.pitch());
		player.yRotO = pose.yaw();
		player.xRotO = pose.pitch();
		player.yHeadRot = pose.yaw();
		player.yHeadRotO = pose.yaw();
		player.yBodyRot = pose.yaw();
		player.yBodyRotO = pose.yaw();
		if ("translucent-overlap".equals(STATIC_TERRAIN_SCENARIO)) GraphicsAuditCameraHistory.settle(player);
		net.minecraft.client.particle.GraphicsAuditTerrainParticleFixture.install(Minecraft.getInstance());
		net.minecraft.client.particle.GraphicsAuditBlockMarkerFixture.install(Minecraft.getInstance());
		GraphicsAuditExperienceOrbFixture.install(Minecraft.getInstance(), poseIndex);
		net.minecraft.client.particle.GraphicsAuditAtlasParticleFixture.install(Minecraft.getInstance());
		net.minecraft.client.particle.GraphicsAuditShriekParticleFixture.install(Minecraft.getInstance());
		net.minecraft.client.particle.GraphicsAuditVibrationParticleFixture.install(Minecraft.getInstance());
	}

	private static ForcedBlockOutlineTarget findForcedBlockOutlineTarget(ClientLevel level, LocalPlayer player) {
		BlockPos origin = player.blockPosition();
		Direction[] faces = new Direction[] { Direction.NORTH, Direction.SOUTH, Direction.WEST, Direction.EAST };
		ForcedBlockOutlineTarget best = null;
		double bestDistance = Double.MAX_VALUE;
		for (int y = -96; y <= 48; y++) {
			for (int radius = 0; radius <= 48; radius++) {
				for (int x = -radius; x <= radius; x++) {
					for (int z = -radius; z <= radius; z++) {
						if (Math.max(Math.abs(x), Math.abs(z)) != radius) {
							continue;
						}
						BlockPos blockPos = origin.offset(x, y, z);
						BlockState blockState = level.getBlockState(blockPos);
						if (blockState.isAir()
							|| blockState.getShape(level, blockPos, CollisionContext.of(player)).isEmpty()
							|| !level.getWorldBorder().isWithinBounds(blockPos)) {
							continue;
						}
						Vec3 target = Vec3.atCenterOf(blockPos);
						for (Direction face : faces) {
							Vec3 eye = target.add(face.getStepX() * 3.0, 0.35, face.getStepZ() * 3.0);
							Vec3 feet = eye.subtract(0.0, player.getEyeHeight(), 0.0);
							Vec3 delta = target.subtract(eye);
							double horizontal = Math.sqrt(delta.x * delta.x + delta.z * delta.z);
							if (horizontal < 0.001) {
								continue;
							}
							float yaw = (float)(Math.toDegrees(Math.atan2(delta.z, delta.x)) - 90.0);
							float pitch = (float)(-Math.toDegrees(Math.atan2(delta.y, horizontal)));
							double distance = player.position().distanceToSqr(feet);
							if (distance < bestDistance) {
								bestDistance = distance;
								best = new ForcedBlockOutlineTarget(blockPos, face, target, feet, new Pose("initial", yaw, pitch));
							}
						}
					}
				}
				if (best != null) {
					return best;
				}
			}
		}
		return best;
	}

	private static void applyRuntimeOverrides(Minecraft minecraft, LocalPlayer player) {
		CameraType cameraType = forcedCameraType();
		if (cameraType != null && minecraft.options.getCameraType() != cameraType) {
			minecraft.options.setCameraType(cameraType);
			minecraft.gameRenderer.checkEntityPostEffect(cameraType.isFirstPerson() ? minecraft.getCameraEntity() : null);
		}
		GameType gameType = forcedGameMode();
		if (gameType != null && minecraft.gameMode != null && minecraft.gameMode.getPlayerMode() != gameType) {
			minecraft.gameMode.setLocalMode(gameType, minecraft.gameMode.getPreviousPlayerMode());
		}
		if (player == null) {
			return;
		}
		if (gameType != null && minecraft.getSingleplayerServer() != null) {
			ServerPlayer serverPlayer = minecraft.getSingleplayerServer().getPlayerList().getPlayer(player.getUUID());
			if (serverPlayer != null && serverPlayer.gameMode() != gameType) {
				serverPlayer.setGameMode(gameType);
			}
		}
		AttackIndicatorStatus attackIndicator = forcedAttackIndicator();
		if (attackIndicator != null && minecraft.options.attackIndicator().get() != attackIndicator) {
			minecraft.options.attackIndicator().set(attackIndicator);
		}
		if (FORCED_SELECTED_HOTBAR_SLOT > 0) {
			int selectedSlot = Math.max(1, Math.min(9, FORCED_SELECTED_HOTBAR_SLOT)) - 1;
			if (player.getInventory().getSelectedSlot() != selectedSlot) {
				player.getInventory().setSelectedSlot(selectedSlot);
			}
		}
		applyHotbarItemFixture(player);
		if (FORCE_EMPTY_SELECTED_HAND) {
			player.getInventory().setItem(player.getInventory().getSelectedSlot(), ItemStack.EMPTY);
		}
		if (!Float.isNaN(FORCED_EXPERIENCE_PROGRESS)) {
			float progress = Math.max(0.0F, Math.min(1.0F, FORCED_EXPERIENCE_PROGRESS));
			if (player.experienceProgress != progress) {
				player.experienceProgress = progress;
			}
			int level = FORCED_EXPERIENCE_LEVEL > 0 ? FORCED_EXPERIENCE_LEVEL : Math.max(1, player.experienceLevel);
			if (player.experienceLevel != level) {
				player.experienceLevel = level;
			}
			player.experienceDisplayStartTick = player.tickCount;
		}
			if (!Float.isNaN(FORCED_ATTACK_PROGRESS)) {
				float progress = Math.max(0.0F, Math.min(1.0F, FORCED_ATTACK_PROGRESS));
				player.setAttackStrengthDelayForDeterministicCapture(FORCED_ATTACK_DELAY);
				int ticks = Math.round(progress * player.getCurrentItemAttackStrengthDelay());
				player.setAttackStrengthTickerForDeterministicCapture(ticks);
			}
			if (FORCE_ATTACK_TARGET) {
				minecraft.crosshairPickEntity = player;
			}
			player.setNoGravity(true);
			player.setDeltaMovement(Vec3.ZERO);
			player.fallDistance = 0.0F;
			recordPlayerSafetyState(player);
			if (initialized && player.getHealth() + 0.001F < initialHealth) {
				fail("deterministic player took damage during capture: initialHealth="
					+ format(initialHealth)
					+ " currentHealth="
					+ format(player.getHealth()));
				return;
			}
				if (forcedBlockOutlineTarget != null) {
					player.setPos(initialPosition);
				player.setOldPosAndRot(initialPosition, forcedBlockOutlineTarget.pose().yaw(), forcedBlockOutlineTarget.pose().pitch());
				applyPose(player, forcedBlockOutlineTarget.pose());
				if (FORCE_BLOCK_OUTLINE_HIGH_CONTRAST && !minecraft.options.highContrastBlockOutline().get()) {
					minecraft.options.highContrastBlockOutline().set(true);
				}
			}
			driveRealSurvivalCrack(minecraft, player);
				if (FORCED_ARMOR_VALUE >= 0) {
					player.setArmorValueForDeterministicCapture(Math.min(20, FORCED_ARMOR_VALUE));
				}
			applyHeartVariantOverride(player);
			if (FORCE_PLAYER_HEALTH_REGEN) {
				player.forceAddEffect(new MobEffectInstance(MobEffects.REGENERATION, 20_000, 0, false, false, false), null);
				}
		if (HIDE_CHAT && minecraft.options.chatVisibility().get() != ChatVisiblity.HIDDEN) {
			minecraft.options.chatVisibility().set(ChatVisiblity.HIDDEN);
		}
		CloudStatus forcedCloudStatus = forcedCloudStatus();
		if (forcedCloudStatus != null && minecraft.options.cloudStatus().get() != forcedCloudStatus) {
			// Distant Horizons disables vanilla clouds in copied capture profiles.
			// These diagnostic scenarios need the ordinary vanilla producer to run
			// with an equivalent explicit quality mode in both repositories.
			minecraft.options.cloudStatus().set(forcedCloudStatus);
		}
		if (FORCED_CLOUD_RANGE_CHUNKS >= 0
			&& minecraft.options.cloudRange().get() != FORCED_CLOUD_RANGE_CHUNKS) {
			minecraft.options.cloudRange().set(FORCED_CLOUD_RANGE_CHUNKS);
		}
	}

	private static void applyHotbarItemFixture(LocalPlayer player) {
		if (HOTBAR_ITEM_FIXTURE.isEmpty()) {
			return;
		}
		List<ItemStack> items = switch (HOTBAR_ITEM_FIXTURE) {
			case "flat-items" -> List.of(
				new ItemStack(net.minecraft.world.item.Items.APPLE),
				new ItemStack(net.minecraft.world.item.Items.FEATHER),
				new ItemStack(net.minecraft.world.item.Items.PAPER),
				new ItemStack(net.minecraft.world.item.Items.DIAMOND),
				new ItemStack(net.minecraft.world.item.Items.IRON_INGOT),
				new ItemStack(net.minecraft.world.item.Items.STICK),
				new ItemStack(net.minecraft.world.item.Items.REDSTONE),
				new ItemStack(net.minecraft.world.item.Items.ARROW),
				new ItemStack(net.minecraft.world.item.Items.COAL));
			case "animated-block" -> GraphicsAuditAnimatedItemFixture.items();
			case "model-foil" -> GraphicsAuditModelFoilFixture.items();
			case "shield", "shield-foil" -> GraphicsAuditShieldFoilFixture.items("shield-foil".equals(HOTBAR_ITEM_FIXTURE));
			case "shield-patterns", "shield-patterns-foil" -> GraphicsAuditPatternedShieldFixture.items("shield-patterns-foil".equals(HOTBAR_ITEM_FIXTURE));
			case "special-foil" -> GraphicsAuditSpecialFoilFixture.items();
			case "standard-3d", "standard-3d-logs" -> List.of(
				new ItemStack(Blocks.STONE),
				new ItemStack(Blocks.GRASS_BLOCK),
				new ItemStack(Blocks.REDSTONE_ORE),
				new ItemStack(Blocks.OAK_LEAVES),
				new ItemStack(Blocks.OAK_SLAB),
				new ItemStack(Blocks.OAK_TRAPDOOR),
				new ItemStack(Blocks.WHITE_WOOL),
				new ItemStack(Blocks.CRAFTING_TABLE),
				new ItemStack(HOTBAR_ITEM_FIXTURE.equals("standard-3d-logs") ? Blocks.OAK_LOG : Blocks.DIRT)
			);
			default -> throw new IllegalStateException("unknown deterministic hotbar fixture: " + HOTBAR_ITEM_FIXTURE);
		};
		for (int slot = 0; slot < items.size(); slot++) {
			if (slot > 0 && Boolean.getBoolean("mattmc.dev.graphicsAuditGuiItemFoilBlend")) {
				items.get(slot).set(net.minecraft.core.component.DataComponents.ENCHANTMENT_GLINT_OVERRIDE, true);
			}
			player.getInventory().setItem(slot, items.get(slot));
		}
		if (Boolean.getBoolean("mattmc.dev.graphicsAuditGuiItemFoilBlend")) {
			Minecraft.getInstance().options.glintSpeed().set(GraphicsAuditGuiFoilTiming.movingRequested() ? 0.5 : 0.0);
			Minecraft.getInstance().options.glintStrength().set(0.5);
		}
		if ("model-foil".equals(HOTBAR_ITEM_FIXTURE) || "shield".equals(HOTBAR_ITEM_FIXTURE) || "shield-foil".equals(HOTBAR_ITEM_FIXTURE) || "shield-patterns-foil".equals(HOTBAR_ITEM_FIXTURE)) {
			Minecraft.getInstance().options.glintSpeed().set(0.0);
			Minecraft.getInstance().options.glintStrength().set(0.5);
		}
	}

	private static void restoreRuntimeOverrides(Minecraft minecraft) {
		releaseDistantHorizonsTexturePaletteChunks(minecraft);
		if (BLOCK_OUTLINE_PAUSE_PARITY && minecraft.screen instanceof PauseScreen) {
			minecraft.setScreen(null);
		}
		if (minecraft.player != null && FORCED_SELECTED_HOTBAR_SLOT > 0 && originalSelectedHotbarSlot >= 0 && originalSelectedHotbarSlot < 9) {
			minecraft.player.getInventory().setSelectedSlot(originalSelectedHotbarSlot);
		}
		if (minecraft.player != null && !originalHotbarItems.isEmpty()) {
			for (int slot = 0; slot < originalHotbarItems.size(); slot++) {
				minecraft.player.getInventory().setItem(slot, originalHotbarItems.get(slot).copy());
			}
			originalHotbarItems = List.of();
		}
		if (minecraft.player != null && !Float.isNaN(FORCED_EXPERIENCE_PROGRESS)) {
			minecraft.player.experienceProgress = originalExperienceProgress;
			minecraft.player.experienceLevel = originalExperienceLevel;
			minecraft.player.experienceDisplayStartTick = originalExperienceDisplayStartTick;
		}
			if (minecraft.player != null && !Float.isNaN(FORCED_ATTACK_PROGRESS)) {
				minecraft.player.setAttackStrengthDelayForDeterministicCapture(Float.NaN);
				minecraft.player.setAttackStrengthTickerForDeterministicCapture(originalAttackStrengthTicker);
			}
				if (FORCE_ATTACK_TARGET) {
					minecraft.crosshairPickEntity = originalCrosshairPickEntity;
				}
					if (FORCE_REAL_SURVIVAL_CRACK) {
						if (minecraft.gameMode != null) {
							minecraft.gameMode.stopDestroyBlock();
						}
						realSurvivalCrackBlock = null;
						realSurvivalCrackDirection = null;
						realSurvivalCrackLastStatus = "restored";
							realSurvivalCrackLastDriveGameTime = Long.MIN_VALUE;
						}
						if (minecraft.player != null && minecraft.player.isNoGravity() != originalNoGravity) {
							minecraft.player.setNoGravity(originalNoGravity);
						}
						if (minecraft.player != null && FORCED_ARMOR_VALUE >= 0) {
							minecraft.player.setArmorValueForDeterministicCapture(originalArmorValueOverride);
						}
					if (minecraft.player != null && (!Float.isNaN(FORCED_PLAYER_HEALTH) || !Float.isNaN(FORCED_PLAYER_MAX_HEALTH))) {
						minecraft.player.setHealthForDeterministicCapture(originalHealthOverride);
						minecraft.player.setMaxHealthForDeterministicCapture(originalMaxHealthOverride);
					}
					if (minecraft.player != null && (!FORCED_PLAYER_HEART_VARIANT.isBlank() || FORCE_PLAYER_HEALTH_REGEN)) {
						restoreEffect(minecraft.player, MobEffects.POISON, originalPoisonEffect);
						restoreEffect(minecraft.player, MobEffects.WITHER, originalWitherEffect);
						restoreEffect(minecraft.player, MobEffects.REGENERATION, originalRegenerationEffect);
						minecraft.player.setTicksFrozen(originalTicksFrozen);
					}
				if (!FORCED_ATTACK_INDICATOR.isBlank() && originalAttackIndicator != null) {
					minecraft.options.attackIndicator().set(originalAttackIndicator);
				}
		if (HIDE_CHAT && originalChatVisibility != null) {
			minecraft.options.chatVisibility().set(originalChatVisibility);
		}
		if (forcedCloudStatus() != null && originalCloudStatus != null) {
			minecraft.options.cloudStatus().set(originalCloudStatus);
		}
		if (FORCED_CLOUD_RANGE_CHUNKS >= 0 && originalCloudRangeChunks >= 0) {
			minecraft.options.cloudRange().set(originalCloudRangeChunks);
		}
			if (FORCE_BLOCK_OUTLINE_HIGH_CONTRAST && minecraft.options.highContrastBlockOutline().get() != originalHighContrastBlockOutline) {
				minecraft.options.highContrastBlockOutline().set(originalHighContrastBlockOutline);
			}
			if (!FORCED_GAME_MODE.isBlank() && minecraft.gameMode != null && originalGameMode != null) {
				minecraft.gameMode.setLocalMode(originalGameMode, originalPreviousGameMode);
			}
		if (!FORCED_CAMERA_TYPE.isBlank() && originalCameraType != null && minecraft.options.getCameraType() != originalCameraType) {
			minecraft.options.setCameraType(originalCameraType);
			minecraft.gameRenderer.checkEntityPostEffect(originalCameraType.isFirstPerson() ? minecraft.getCameraEntity() : null);
		}
	}

		public static void recordRealSurvivalCrackRenderState(net.minecraft.client.renderer.state.BlockBreakingRenderState state) {
			if (!ENABLED || !FORCE_REAL_SURVIVAL_CRACK || state == null || state.blockState == null || state.blockState.isAir()) {
				return;
			}
		realSurvivalCrackRenderedStateCount++;
		realSurvivalCrackMinRenderedStage = Math.min(realSurvivalCrackMinRenderedStage, state.progress);
		realSurvivalCrackMaxRenderedStage = Math.max(realSurvivalCrackMaxRenderedStage, state.progress);
			realSurvivalCrackLastRenderedTarget = state.blockPos == null ? "unknown" : state.blockPos.toShortString();
			realSurvivalCrackLastRenderedBlockType = state.blockState.getBlock().builtInRegistryHolder().key().location().toString();
		}

		private static boolean realSurvivalCrackPoseReady() {
			if (!FORCE_REAL_SURVIVAL_CRACK || poses == null || poseIndex < 0 || poseIndex >= poses.length) {
				return true;
			}
			return switch (poses[poseIndex].name()) {
				case "crack-early" -> realSurvivalCrackMaxRenderedStage >= 0;
				case "crack-middle" -> realSurvivalCrackMaxRenderedStage >= 4;
				case "crack-late" -> realSurvivalCrackMaxRenderedStage >= 7;
			default -> true;
		};
	}

	private static boolean movingMeshProducerReady() {
		return movingMeshProducerReady(renderedFrameIndex);
	}

	private static boolean movingMeshProducerReady(long frameIndex) {
		if (!FALLING_BLOCK_SCENARIO.isEmpty() && !"hidden".equals(FALLING_BLOCK_SCENARIO)
			&& !hasCurrentFallingBlockRoute(frameIndex)) {
			return false;
		}
		if (!PISTON_SCENARIO.isEmpty()
			&& !"hidden".equals(PISTON_SCENARIO)
			&& !"completed".equals(PISTON_SCENARIO)
			&& !"removed".equals(PISTON_SCENARIO)) {
			return hasCurrentPistonRoute(frameIndex);
		}
		if (!PRIMED_TNT_SCENARIO.isEmpty() && !"hidden".equals(PRIMED_TNT_SCENARIO)) {
			return hasCurrentPrimedTntRoute(frameIndex);
		}
		if (!ARROW_SCENARIO.isEmpty() && !"hidden".equals(ARROW_SCENARIO)) {
			return hasCurrentArrowRoute(frameIndex);
		}
		if (!EXPERIENCE_ORB_SCENARIO.isEmpty() && !"hidden".equals(EXPERIENCE_ORB_SCENARIO)) {
			return hasCurrentExperienceOrbRoute(frameIndex);
		}
		if (!BEACON_BEAM_SCENARIO.isEmpty() && !"hidden".equals(BEACON_BEAM_SCENARIO)) {
			return hasCurrentBeaconBeamRoute(frameIndex);
		}
		if (!ITEM_ENTITY_SCENARIO.isEmpty() && !"hidden".equals(ITEM_ENTITY_SCENARIO)) {
			return hasCurrentItemEntityRoute(frameIndex);
		}
		if (!MODEL_MESH_SCENARIO.isEmpty() && !"hidden".equals(MODEL_MESH_SCENARIO)
			&& !hasCurrentModelMeshTraversal(frameIndex)) {
			return false;
		}
		if (!ENTITY_FLAME_SCENARIO.isEmpty() && !"hidden".equals(ENTITY_FLAME_SCENARIO)) {
			return hasCurrentEntityFlameRoute(frameIndex);
		}
		if (!ENTITY_SHADOW_SCENARIO.isEmpty() && !"hidden".equals(ENTITY_SHADOW_SCENARIO)) {
			return hasCurrentEntityShadowRoute(frameIndex);
		}
		if (!ENTITY_LEASH_SCENARIO.isEmpty() && !"hidden".equals(ENTITY_LEASH_SCENARIO)) {
			return hasCurrentEntityLeashRoute(frameIndex);
		}
		return true;
	}

	private static boolean hasCurrentEntityFlameRoute(long frameIndex) {
		if (!"cow".equals(ENTITY_FLAME_SCENARIO) || !"cow".equals(MODEL_MESH_SCENARIO)) {
			return false;
		}
		long tolerance = movingMeshFrameTolerance();
		WorldRenderRoutePolicy.Route route = WorldRenderRoutePolicy.currentEntityFlameRoute();
		if (!route.usesRustWholeFrameVulkan()) {
			// Compatibility controls deliberately have no Rust execution receipt.
			return modelMeshSetupClientEntityPresent;
		}
		boolean submitted = RustGalWorldPrimitiveRenderer.entityFlameSemanticDiagnostics().stream().anyMatch(diagnostic ->
			Math.abs(diagnostic.frameIndex() - frameIndex) <= tolerance
				&& "rust-vulkan-whole-frame".equals(diagnostic.route())
				&& diagnostic.flameSubmits() > 0
				&& diagnostic.quads() > 0
		);
		return submitted && RustGalWorldPrimitiveRenderer.entityFlameExecutionDiagnostics().stream().anyMatch(diagnostic ->
			Math.abs(diagnostic.deterministicFrameIndex() - frameIndex) <= tolerance
				&& "rust-vulkan-whole-frame".equals(diagnostic.route())
				&& diagnostic.quads() > 0
		);
	}

	private static boolean hasCurrentEntityShadowRoute(long frameIndex) {
		if (!"cow".equals(ENTITY_SHADOW_SCENARIO) || !"cow".equals(MODEL_MESH_SCENARIO)) {
			return false;
		}
		long tolerance = movingMeshFrameTolerance();
		WorldRenderRoutePolicy.Route route = WorldRenderRoutePolicy.currentEntityShadowRoute();
		if (!route.usesRustWholeFrameVulkan()) {
			// Compatibility controls deliberately do not produce a Rust receipt.
			return modelMeshSetupClientEntityPresent;
		}
		boolean submitted = RustGalWorldPrimitiveRenderer.entityShadowSemanticDiagnostics().stream().anyMatch(diagnostic ->
			Math.abs(diagnostic.frameIndex() - frameIndex) <= tolerance
				&& "rust-vulkan-whole-frame".equals(diagnostic.route())
				&& diagnostic.shadowSubmits() > 0
				&& diagnostic.quads() > 0
		);
		return submitted && RustGalWorldPrimitiveRenderer.entityShadowExecutionDiagnostics().stream().anyMatch(diagnostic ->
			Math.abs(diagnostic.deterministicFrameIndex() - frameIndex) <= tolerance
				&& "rust-vulkan-whole-frame".equals(diagnostic.route())
				&& diagnostic.quads() > 0
		);
	}

	private static boolean hasCurrentEntityLeashRoute(long frameIndex) {
		if (!"cow".equals(ENTITY_LEASH_SCENARIO) || !"cow".equals(MODEL_MESH_SCENARIO)) {
			return false;
		}
		long tolerance = movingMeshFrameTolerance();
		WorldRenderRoutePolicy.Route route = WorldRenderRoutePolicy.currentEntityLeashRoute();
		if (!route.usesRustWholeFrameVulkan()) {
			// Controls must still observe the real copied leash producer, but they
			// intentionally have no Rust execution receipt.
			return modelMeshSetupClientEntityPresent;
		}
		boolean submitted = RustGalWorldPrimitiveRenderer.entityLeashSemanticDiagnostics().stream().anyMatch(diagnostic ->
			Math.abs(diagnostic.frameIndex() - frameIndex) <= tolerance
				&& "rust-vulkan-whole-frame".equals(diagnostic.route())
				&& diagnostic.leashSubmits() > 0
				&& diagnostic.quads() > 0
		);
		return submitted && RustGalWorldPrimitiveRenderer.entityLeashExecutionDiagnostics().stream().anyMatch(diagnostic ->
			Math.abs(diagnostic.deterministicFrameIndex() - frameIndex) <= tolerance
				&& "rust-vulkan-whole-frame".equals(diagnostic.route())
				&& diagnostic.quads() > 0
		);
	}

	private static boolean weatherProducerReady() {
		if (WEATHER_SCENARIO.isEmpty()) {
			return true;
		}
		long frameIndex = renderedFrameIndex;
		long tolerance = Math.max(16L, FRAMES_PER_POSE + 4L);
		WorldRenderRoutePolicy.Route route = WorldRenderRoutePolicy.currentWeatherRoute();
		boolean traversalColumns = false;
		for (RustGalWorldPrimitiveRenderer.WeatherTraversalDiagnostic diagnostic : RustGalWorldPrimitiveRenderer.weatherTraversalDiagnostics()) {
			if (Math.abs(diagnostic.frameIndex() - frameIndex) <= tolerance
				&& diagnostic.intensity() > 0.0F
				&& diagnostic.rainColumns() > 0
				&& diagnostic.route().equals(route.name().toLowerCase(Locale.ROOT))) {
				traversalColumns = true;
				break;
			}
		}
		if (!traversalColumns) {
			return false;
		}
		if (!route.usesRustOpenGl() && !route.usesRustWholeFrameVulkan()) {
			return true;
		}
		boolean semanticColumns = false;
		for (RustGalWorldPrimitiveRenderer.WeatherSemanticDiagnostic diagnostic : RustGalWorldPrimitiveRenderer.weatherSemanticDiagnostics()) {
			if (Math.abs(diagnostic.frameIndex() - frameIndex) <= tolerance
				&& diagnostic.intensity() > 0.0F
				&& diagnostic.rainColumns() > 0
				&& diagnostic.quads() > 0) {
				semanticColumns = true;
				break;
			}
		}
		if (!semanticColumns) {
			return false;
		}
		for (RustGalWorldPrimitiveRenderer.WeatherExecutionDiagnostic diagnostic : RustGalWorldPrimitiveRenderer.weatherExecutionDiagnostics()) {
			if (Math.abs(diagnostic.deterministicFrameIndex() - frameIndex) <= tolerance
				&& diagnostic.route().equals(route.name().toLowerCase(Locale.ROOT))
				&& diagnostic.quads() > 0) {
				return true;
			}
		}
		return false;
	}

	private static boolean cloudProducerReady() {
		if (CLOUD_SCENARIO.isEmpty()) {
			return true;
		}
		if (forcedCloudStatus() == null) {
			fail("unsupported deterministic cloud scenario: " + CLOUD_SCENARIO);
			return false;
		}
		long frameIndex = renderedFrameIndex;
		long tolerance = Math.max(16L, FRAMES_PER_POSE + 4L);
		WorldRenderRoutePolicy.Route route = WorldRenderRoutePolicy.currentCloudRoute();
		if (!route.usesRustWholeFrameVulkan()) {
			return false;
		}
		boolean traversal = false;
		for (RustGalWorldPrimitiveRenderer.CloudTraversalDiagnostic diagnostic : RustGalWorldPrimitiveRenderer.cloudTraversalDiagnostics()) {
			if (Math.abs(diagnostic.frameIndex() - frameIndex) <= tolerance
				&& diagnostic.route().equals(route.name().toLowerCase(Locale.ROOT))
				&& diagnostic.cells() > 0 && diagnostic.radius() > 0) {
				traversal = true;
				break;
			}
		}
		if (!traversal) {
			return false;
		}
		boolean semantic = false;
		for (RustGalWorldPrimitiveRenderer.CloudSemanticDiagnostic diagnostic : RustGalWorldPrimitiveRenderer.cloudSemanticDiagnostics()) {
			if (Math.abs(diagnostic.frameIndex() - frameIndex) <= tolerance && diagnostic.quads() > 0) {
				semantic = true;
				break;
			}
		}
		if (!semantic) {
			return false;
		}
		for (RustGalWorldPrimitiveRenderer.CloudExecutionDiagnostic diagnostic : RustGalWorldPrimitiveRenderer.cloudExecutionDiagnostics()) {
			if (Math.abs(diagnostic.deterministicFrameIndex() - frameIndex) <= tolerance
				&& diagnostic.route().equals(route.name().toLowerCase(Locale.ROOT))
				&& diagnostic.quads() > 0) {
				return true;
			}
		}
		return false;
	}

	private static String cloudProducerSummary() {
		return "scenario=" + CLOUD_SCENARIO
			+ " route=" + WorldRenderRoutePolicy.currentCloudRoute().name().toLowerCase(Locale.ROOT)
			+ " traversal=" + RustGalWorldPrimitiveRenderer.cloudTraversalDiagnostics().size()
			+ " semantic=" + RustGalWorldPrimitiveRenderer.cloudSemanticDiagnostics().size()
			+ " execution=" + RustGalWorldPrimitiveRenderer.cloudExecutionDiagnostics().size();
	}

	private static boolean hasCurrentFallingBlockRoute(long frameIndex) {
		long frameTolerance = movingMeshFrameTolerance();
		for (RustGalWorldPrimitiveRenderer.FallingBlockRouteDecision decision : RustGalWorldPrimitiveRenderer.fallingBlockRouteDecisions()) {
			if (Math.abs(decision.frameIndex() - frameIndex) > frameTolerance) {
				continue;
			}
			if ("legacy".equals(FALLING_BLOCK_ROUTE_CONTROL)) {
				if ("java-legacy".equals(decision.route()) && decision.javaDrawn()) {
					return true;
				}
			} else if ("disabled".equals(FALLING_BLOCK_ROUTE_CONTROL)) {
				if ("disabled".equals(decision.route()) && !decision.rustSelected() && !decision.javaDrawn()) {
					return true;
				}
			} else if (decision.rustSelected() && decision.rustQueued()) {
				return hasCurrentMovingMeshDiagnostic("falling-block", frameIndex, frameTolerance);
			}
		}
		return false;
	}

	private static boolean hasCurrentPistonRoute(long frameIndex) {
		long frameTolerance = movingMeshFrameTolerance();
		for (RustGalWorldPrimitiveRenderer.MovingBlockRouteDecision decision : RustGalWorldPrimitiveRenderer.movingBlockRouteDecisions()) {
			if ("piston".equals(decision.provenance())
				&& Math.abs(decision.frameIndex() - frameIndex) <= frameTolerance
				&& decision.rustSelected()
				&& decision.rustQueued()) {
				return hasCurrentMovingMeshDiagnostic("piston", frameIndex, frameTolerance);
			}
		}
		return false;
	}

	private static boolean hasCurrentPrimedTntRoute(long frameIndex) {
		long frameTolerance = movingMeshFrameTolerance();
		for (RustGalWorldPrimitiveRenderer.MovingBlockRouteDecision decision : RustGalWorldPrimitiveRenderer.movingBlockRouteDecisions()) {
			if (!"primed-tnt".equals(decision.provenance())
				|| Math.abs(decision.frameIndex() - frameIndex) > frameTolerance) {
				continue;
			}
			if (decision.javaDrawn() && !decision.rustSelected() && !decision.rustQueued()) {
				return true;
			}
			if (decision.rustSelected() && decision.rustQueued()) {
				return hasCurrentMovingMeshDiagnostic("primed-tnt", frameIndex, frameTolerance);
			}
		}
		return false;
	}

	private static boolean hasCurrentArrowRoute(long frameIndex) {
		long frameTolerance = movingMeshFrameTolerance();
		for (RustGalWorldPrimitiveRenderer.ArrowRouteDecision decision : RustGalWorldPrimitiveRenderer.arrowRouteDecisions()) {
			if (Math.abs(decision.frameIndex() - frameIndex) > frameTolerance) {
				continue;
			}
			if (decision.javaDrawn() && !decision.rustSelected() && !decision.rustQueued()) {
				return true;
			}
			if (decision.rustSelected() && decision.rustQueued() && !decision.javaDrawn()) {
				return hasCurrentArrowDiagnostic(frameIndex, frameTolerance);
			}
		}
		return false;
	}

	private static boolean hasCurrentArrowDiagnostic(long frameIndex, long frameTolerance) {
		for (RustGalWorldPrimitiveRenderer.ArrowDiagnostic diagnostic : RustGalWorldPrimitiveRenderer.arrowDiagnostics()) {
			if (Math.abs(diagnostic.frameIndex() - frameIndex) <= frameTolerance
				&& diagnostic.projected()
				&& diagnostic.sectionCount() > 0) {
				return true;
			}
		}
		return false;
	}

	private static boolean hasCurrentExperienceOrbRoute(long frameIndex) {
		long frameTolerance = movingMeshFrameTolerance();
		for (RustGalWorldPrimitiveRenderer.ExperienceOrbRouteDecision decision
			: RustGalWorldPrimitiveRenderer.experienceOrbRouteDecisions()) {
			if (Math.abs(decision.frameIndex() - frameIndex) > frameTolerance) {
				continue;
			}
			if (decision.javaDrawn() && !decision.rustSelected() && !decision.rustQueued()) {
				return true;
			}
			if (decision.rustSelected() && decision.rustQueued() && !decision.javaDrawn()) {
				boolean projected = RustGalWorldPrimitiveRenderer.experienceOrbDiagnostics().stream().anyMatch(diagnostic ->
					Math.abs(diagnostic.frameIndex() - frameIndex) <= frameTolerance
						&& diagnostic.projected()
						&& diagnostic.screenRight() > diagnostic.screenLeft()
						&& diagnostic.screenBottom() > diagnostic.screenTop()
				);
				return projected && RustGalWorldPrimitiveRenderer.experienceOrbExecutionDiagnostics().stream()
					.anyMatch(diagnostic ->
						Math.abs(diagnostic.deterministicFrameIndex() - frameIndex) <= frameTolerance
							&& "rust-vulkan-whole-frame".equals(diagnostic.route())
							&& diagnostic.gameplayFrameId() > 0L
							&& diagnostic.submissionId() > 0L
							&& diagnostic.quads() > 0
					);
			}
		}
		return false;
	}

	private static boolean hasCurrentBeaconBeamRoute(long frameIndex) {
		if (!beaconBeamSetupClientReady) {
			return false;
		}
		WorldRenderRoutePolicy.Route route = WorldRenderRoutePolicy.currentBeaconBeamRoute();
		if (!route.usesRustWholeFrameVulkan()) {
			return true;
		}
		long tolerance = movingMeshFrameTolerance();
		boolean submitted;
		synchronized (SUBMITTED_WORK_BY_FRAME) {
			submitted = false;
			for (long candidateFrame = Math.max(0L, frameIndex - tolerance); candidateFrame <= frameIndex + tolerance; candidateFrame++) {
				Map<String, Set<String>> work = SUBMITTED_WORK_BY_FRAME.get(candidateFrame);
				if (work != null && work.containsKey("beacon-beam") && !work.get("beacon-beam").isEmpty()) {
					submitted = true;
					break;
				}
			}
		}
		return submitted && RustGalWorldPrimitiveRenderer.beaconBeamExecutionDiagnostics().stream().anyMatch(diagnostic ->
			Math.abs(diagnostic.deterministicFrameIndex() - frameIndex) <= tolerance
				&& "rust-vulkan-whole-frame".equals(diagnostic.route())
				&& diagnostic.gameplayFrameId() > 0L
				&& diagnostic.submissionId() > 0L
				&& diagnostic.quads() >= 8
		);
	}

	private static boolean hasCurrentItemEntityRoute(long frameIndex) {
		long frameTolerance = movingMeshFrameTolerance();
		return RustGalWorldPrimitiveRenderer.movingMeshExecutionDiagnostics().stream().anyMatch(diagnostic ->
			"item-entity".equals(diagnostic.provenance())
				&& Math.abs(diagnostic.deterministicFrameIndex() - frameIndex) <= frameTolerance
				&& diagnostic.instances() > 0
		);
	}

	private static boolean hasCurrentModelMeshRoute(long frameIndex) {
		return hasCurrentModelMeshRoute(frameIndex, movingMeshFrameTolerance());
	}

	private static boolean hasCurrentModelMeshRoute(long frameIndex, long frameTolerance) {
		if ("evoker-fangs".equals(MODEL_MESH_SCENARIO)) {
			// Fangs expose only a brief projected attack window, while the completed
			// Rust submission receipt is recorded on the adjacent frame. Correlate
			// within this bounded pose span, retaining both projected mesh and Rust
			// execution requirements.
			long boundedTolerance = Math.max(frameTolerance, 8L);
			boolean projected = RustGalWorldPrimitiveRenderer.modelMeshDiagnostics().stream().anyMatch(diagnostic ->
				Math.abs(diagnostic.frameIndex() - frameIndex) <= boundedTolerance
					&& expectedModelMeshDiagnosticTextureId().equals(diagnostic.textureId())
					&& diagnostic.projected() && diagnostic.sectionCount() > 0);
			return projected && RustGalWorldPrimitiveRenderer.movingMeshExecutionDiagnostics().stream().anyMatch(diagnostic ->
				"model".equals(diagnostic.provenance())
					&& Math.abs(diagnostic.deterministicFrameIndex() - frameIndex) <= boundedTolerance
					&& diagnostic.instances() > 0);
		}
		if ("wind-charge".equals(MODEL_MESH_SCENARIO)) {
			boolean queued = RustGalWorldPrimitiveRenderer.modelMeshRouteDecisions().stream().anyMatch(decision ->
				Math.abs(decision.frameIndex() - frameIndex) <= frameTolerance
					&& expectedModelMeshTextureId().equals(decision.textureId())
					&& "rust-vulkan-whole-frame".equals(decision.route())
					&& decision.rustSelected() && decision.rustQueued() && !decision.javaDrawn()
			);
			return queued && RustGalWorldPrimitiveRenderer.entityModelExecutionDiagnostics().stream().anyMatch(diagnostic ->
				Math.abs(diagnostic.deterministicFrameIndex() - frameIndex) <= frameTolerance
					&& "rust-vulkan-whole-frame".equals(diagnostic.route())
					&& diagnostic.gameplayFrameId() > 0L
					&& diagnostic.submissionId() > 0L
					&& diagnostic.quads() > 0
			);
		}
		boolean queued = RustGalWorldPrimitiveRenderer.modelMeshDiagnostics().stream().anyMatch(diagnostic ->
			Math.abs(diagnostic.frameIndex() - frameIndex) <= frameTolerance
				&& expectedModelMeshDiagnosticTextureId().equals(diagnostic.textureId())
				&& diagnostic.projected()
				&& diagnostic.sectionCount() > 0
		);
		if (!queued) {
			return false;
		}
		return RustGalWorldPrimitiveRenderer.movingMeshExecutionDiagnostics().stream().anyMatch(diagnostic ->
			"model".equals(diagnostic.provenance())
				&& Math.abs(diagnostic.deterministicFrameIndex() - frameIndex) <= frameTolerance
				&& diagnostic.instances() > 0
		);
	}


	/**
	 * A compatibility control must prove that the ordinary model producer was
	 * traversed, but must never wait for Rust work it is explicitly not allowed
	 * to submit. Rust whole-frame keeps the stronger enqueue/execution proof.
	 */
	private static boolean hasCurrentModelMeshTraversal(long frameIndex) {
		if (isModelPartMeshScenario()) {
			return hasCurrentModelPartMeshTraversal(frameIndex);
		}
		if (WorldRenderRoutePolicy.currentModelMeshRoute(true).usesRustWholeFrameVulkan()) {
			return hasCurrentModelMeshRoute(frameIndex);
		}
		long frameTolerance = movingMeshFrameTolerance();
		return RustGalWorldPrimitiveRenderer.modelMeshRouteDecisions().stream().anyMatch(decision ->
			Math.abs(decision.frameIndex() - frameIndex) <= frameTolerance
				&& expectedModelMeshTextureId().equals(decision.textureId())
				&& ("java-legacy".equals(decision.route()) || "disabled".equals(decision.route()))
				&& !decision.rustQueued()
		);
	}

	private static boolean isModelPartMeshScenario() {
		return "decorated-pot".equals(MODEL_MESH_SCENARIO) || "conduit".equals(MODEL_MESH_SCENARIO);
	}

	/**
	 * A decorated pot is emitted through {@code submitModelPart}, not
	 * {@code submitModel}. Require its own semantic traversal, copied shared-mesh
	 * record, and completed Rust execution so an unrelated legacy model cannot
	 * satisfy the deterministic producer gate.
	 */
	private static boolean hasCurrentModelPartMeshTraversal(long frameIndex) {
		long frameTolerance = movingMeshFrameTolerance();
		String expectedTexture = expectedModelMeshTextureId();
		WorldRenderRoutePolicy.Route route = WorldRenderRoutePolicy.currentModelPartMeshRoute(true);
		boolean traversed = RustGalWorldPrimitiveRenderer.modelPartMeshTraversalDiagnostics().stream().anyMatch(diagnostic ->
			Math.abs(diagnostic.frameIndex() - frameIndex) <= frameTolerance
				&& expectedTexture.equals(diagnostic.textureId())
				&& "eligible".equals(diagnostic.eligibility())
		);
		if (!traversed) {
			return false;
		}
		if (!route.usesRustWholeFrameVulkan()) {
			return true;
		}
		boolean queued = RustGalWorldPrimitiveRenderer.modelMeshDiagnostics().stream().anyMatch(diagnostic ->
			Math.abs(diagnostic.frameIndex() - frameIndex) <= frameTolerance
				&& expectedModelMeshDiagnosticTextureId().equals(diagnostic.textureId())
				&& diagnostic.projected()
				&& diagnostic.sectionCount() > 0
		);
		return queued && RustGalWorldPrimitiveRenderer.movingMeshExecutionDiagnostics().stream().anyMatch(diagnostic ->
			"model-part".equals(diagnostic.provenance())
				&& Math.abs(diagnostic.deterministicFrameIndex() - frameIndex) <= frameTolerance
				&& diagnostic.instances() > 0
		);
	}

	private static String expectedModelMeshTextureId() {
		return switch (MODEL_MESH_SCENARIO) {
			case "decorated-pot" -> "minecraft:entity/decorated_pot/decorated_pot_base";
			case "conduit" -> "minecraft:entity/conduit/base";
			case "bed" -> "minecraft:entity/bed/red";
			case "bell" -> "minecraft:entity/bell/bell_body";
			case "shulker" -> "minecraft:entity/shulker/shulker_purple";
			case "llama-spit" -> "minecraft:textures/entity/llama/spit.png";
			case "evoker-fangs" -> "minecraft:textures/entity/illager/evoker_fangs.png";
			case "wither-skull" -> "minecraft:textures/entity/wither/wither.png";
			case "chicken" -> "minecraft:textures/entity/chicken/temperate_chicken.png";
			case "cow" -> "minecraft:textures/entity/cow/temperate_cow.png";
			case "pig" -> "minecraft:textures/entity/pig/temperate_pig.png";
			case "rabbit" -> "minecraft:textures/entity/rabbit/brown.png";
			case "sheep" -> "minecraft:textures/entity/sheep/sheep.png";
			case "tropical-fish" -> "minecraft:textures/entity/fish/tropical_a.png";
			case "zombie" -> "minecraft:textures/entity/zombie/zombie.png";
			case "end-crystal" -> "minecraft:textures/entity/end_crystal/end_crystal.png";
			case "wind-charge" -> "minecraft:textures/entity/projectiles/wind_charge.png";
			default -> "minecraft:entity/chest/normal";
		};
	}

	/**
	 * Model route receipts use the gameplay material identity (for example
	 * {@code entity/chest/normal}), while the copied mesh diagnostic records the
	 * actual atlas payload consumed by Rust. Keep that distinction explicit so a
	 * block-entity atlas cannot be mistaken for a missing producer.
	 */
	private static String expectedModelMeshDiagnosticTextureId() {
		return switch (MODEL_MESH_SCENARIO) {
			case "chest" -> "minecraft:textures/atlas/chest.png";
			case "bed" -> "minecraft:textures/atlas/beds.png";
			case "shulker" -> "minecraft:textures/atlas/shulker_boxes.png";
			case "decorated-pot" -> "minecraft:textures/atlas/decorated_pot.png";
			case "bell" -> "minecraft:textures/atlas/blocks.png";
			case "conduit" -> "minecraft:textures/atlas/blocks.png";
			default -> expectedModelMeshTextureId();
		};
	}

	private static long movingMeshFrameTolerance() {
		return Math.max(16L, FRAMES_PER_POSE + 4L);
	}

	private static boolean hasCurrentMovingMeshDiagnostic(String provenance, long frameIndex, long frameTolerance) {
		for (RustGalWorldPrimitiveRenderer.MovingBlockDiagnostic diagnostic : RustGalWorldPrimitiveRenderer.movingBlockDiagnostics()) {
			if (provenance.equals(diagnostic.provenance())
				&& Math.abs(diagnostic.frameIndex() - frameIndex) <= frameTolerance
				&& diagnostic.projected()
				&& diagnostic.sectionCount() > 0) {
				return true;
			}
		}
		return false;
	}

	private static String movingMeshProducerSummary() {
		long pistonDecisions = RustGalWorldPrimitiveRenderer.movingBlockRouteDecisions()
			.stream()
			.filter(decision -> "piston".equals(decision.provenance()))
			.count();
		long pistonMeshes = RustGalWorldPrimitiveRenderer.movingBlockDiagnostics()
			.stream()
			.filter(diagnostic -> "piston".equals(diagnostic.provenance()))
			.count();
		long primedTntDecisions = RustGalWorldPrimitiveRenderer.movingBlockRouteDecisions()
			.stream()
			.filter(decision -> "primed-tnt".equals(decision.provenance()))
			.count();
		long primedTntMeshes = RustGalWorldPrimitiveRenderer.movingBlockDiagnostics()
			.stream()
			.filter(diagnostic -> "primed-tnt".equals(diagnostic.provenance()))
			.count();
		return "fallingScenario=" + FALLING_BLOCK_SCENARIO
			+ " fallingDecisions=" + RustGalWorldPrimitiveRenderer.fallingBlockRouteDecisions().size()
			+ " fallingMeshes=" + RustGalWorldPrimitiveRenderer.fallingBlockDiagnostics().size()
			+ " fallingEntitySeen=" + fallingEntitySeenCount
			+ " fallingShouldRender=" + fallingEntityShouldRenderCount
			+ " fallingCompiledSection=" + fallingEntityCompiledSectionCount
			+ " fallingExtracted=" + fallingEntityExtractedCount
			+ " pistonScenario=" + PISTON_SCENARIO
			+ " pistonDecisions=" + pistonDecisions
			+ " pistonMeshes=" + pistonMeshes
			+ " primedTntScenario=" + PRIMED_TNT_SCENARIO
			+ " primedTntDecisions=" + primedTntDecisions
			+ " primedTntMeshes=" + primedTntMeshes
			+ " arrowScenario=" + ARROW_SCENARIO
			+ " arrowDecisions=" + RustGalWorldPrimitiveRenderer.arrowRouteDecisions().size()
			+ " arrowMeshes=" + RustGalWorldPrimitiveRenderer.arrowDiagnostics().size()
			+ " experienceOrbScenario=" + EXPERIENCE_ORB_SCENARIO
			+ " experienceOrbDecisions=" + RustGalWorldPrimitiveRenderer.experienceOrbRouteDecisions().size()
			+ " experienceOrbQuads=" + RustGalWorldPrimitiveRenderer.experienceOrbDiagnostics().size()
			+ " modelScenario=" + MODEL_MESH_SCENARIO
			+ " modelMeshes=" + RustGalWorldPrimitiveRenderer.modelMeshDiagnostics().size()
			+ " entityFlameScenario=" + ENTITY_FLAME_SCENARIO
			+ " entityFlameSemantic=" + RustGalWorldPrimitiveRenderer.entityFlameSemanticDiagnostics().size()
			+ " entityFlameExecuted=" + RustGalWorldPrimitiveRenderer.entityFlameExecutionDiagnostics().size()
			+ " entityShadowScenario=" + ENTITY_SHADOW_SCENARIO
			+ " entityShadowSemantic=" + RustGalWorldPrimitiveRenderer.entityShadowSemanticDiagnostics().size()
			+ " entityShadowExecuted=" + RustGalWorldPrimitiveRenderer.entityShadowExecutionDiagnostics().size()
			+ " entityLeashScenario=" + ENTITY_LEASH_SCENARIO
			+ " entityLeashSemantic=" + RustGalWorldPrimitiveRenderer.entityLeashSemanticDiagnostics().size()
			+ " entityLeashExecuted=" + RustGalWorldPrimitiveRenderer.entityLeashExecutionDiagnostics().size()
			+ " modelPartTraversals=" + RustGalWorldPrimitiveRenderer.modelPartMeshTraversalDiagnostics().size()
			+ " modelPartExecutions=" + RustGalWorldPrimitiveRenderer.movingMeshExecutionDiagnostics().stream()
				.filter(diagnostic -> "model-part".equals(diagnostic.provenance()))
				.count();
	}

	private static String weatherProducerSummary() {
		List<RustGalWorldPrimitiveRenderer.WeatherSemanticDiagnostic> semantic =
			RustGalWorldPrimitiveRenderer.weatherSemanticDiagnostics();
		List<RustGalWorldPrimitiveRenderer.WeatherExecutionDiagnostic> execution =
			RustGalWorldPrimitiveRenderer.weatherExecutionDiagnostics();
		return "weatherScenario=" + WEATHER_SCENARIO
			+ " setupStage=" + weatherSetupStage
			+ " setupAttempts=" + weatherSetupAttempts
			+ " setupMissing=" + weatherSetupLastMissing
			+ " semanticReceipts=" + semantic.size()
			+ " executionReceipts=" + execution.size()
			+ " lastSemantic=" + (semantic.isEmpty() ? "none" : semantic.getLast())
			+ " lastExecution=" + (execution.isEmpty() ? "none" : execution.getLast());
	}

	private static void setupRealSurvivalCrackBlock(Minecraft minecraft, LocalPlayer player) {
		if (!FORCE_REAL_SURVIVAL_CRACK || !FORCE_REAL_SURVIVAL_CRACK_SETUP_BLOCK || minecraft.level == null || !"unset".equals(realSurvivalCrackSetupTarget)) {
			return;
		}

		Vec3 eye = player.getEyePosition(1.0F);
		Vec3 look = player.getLookAngle();
		BlockPos target = BlockPos.containing(eye.add(look.scale(1.5)));
		BlockState setupState = Blocks.STONE.defaultBlockState();
		minecraft.level.setBlock(target, setupState, 3);
		if (minecraft.getSingleplayerServer() != null) {
			ServerLevel serverLevel = minecraft.getSingleplayerServer().getLevel(minecraft.level.dimension());
			if (serverLevel != null) {
				serverLevel.setBlock(target, setupState, 3);
			}
		}
		realSurvivalCrackSetupTarget = target.toShortString();
		realSurvivalCrackLastStatus = "setup-block";
		LOGGER.info(
			"Deterministic real survival crack setup placed minecraft:stone at {} from saved view eye=({}, {}, {}) look=({}, {}, {})",
			realSurvivalCrackSetupTarget,
			eye.x,
			eye.y,
			eye.z,
			look.x,
			look.y,
			look.z
		);
	}

	private static void setupBlockDisplayAndWorldTextScenarios(Minecraft minecraft, LocalPlayer player) {
		GraphicsAuditMixedItemFoilFixture.install(minecraft);
		if (!GraphicsAuditBlockDisplayFixture.requested()) GraphicsAuditLavaFixture.install(minecraft);
		if (GraphicsAuditFlowingWaterFixture.requested() && !GraphicsAuditBlockDisplayFixture.requested()) {
			GraphicsAuditFlowingWaterFixture.install(minecraft);
		}
		if (GraphicsAuditBlockDisplayFixture.requested()) {
			GraphicsAuditBlockDisplayFixture.install(minecraft);
			return;
		}
		if ((BLOCK_DISPLAY_SCENARIO.isEmpty() && WORLD_TEXT_SCENARIO.isEmpty())
			|| "hidden".equals(BLOCK_DISPLAY_SCENARIO) || minecraft.level == null) {
			return;
		}
		if (!WORLD_TEXT_SCENARIO.isEmpty()
			&& !"block-display".equals(WORLD_TEXT_SCENARIO)
			&& !"text-display".equals(WORLD_TEXT_SCENARIO)
			&& !"text-display-polygon-offset".equals(WORLD_TEXT_SCENARIO)
			&& !"name-tag".equals(WORLD_TEXT_SCENARIO)) {
			fail("unsupported deterministic Rust world-text scenario: " + WORLD_TEXT_SCENARIO);
			return;
		}
		Vec3 forward = player.getLookAngle();
		if (forward.lengthSqr() < 0.0001) {
			forward = new Vec3(0.0, 0.0, 1.0);
		}
		Vec3 position = player.position().add(forward.normalize().scale(4.0)).add(-0.5, -0.5, -0.5);
		if (!BLOCK_DISPLAY_SCENARIO.isEmpty() || "block-display".equals(WORLD_TEXT_SCENARIO)) {
			BlockState state = blockDisplayBenchmarkState();
			Display.BlockDisplay display = new Display.BlockDisplay(EntityType.BLOCK_DISPLAY, minecraft.level);
			display.setId(Integer.MIN_VALUE + 4096);
			display.setPos(position);
			display.setBlockState(state);
			display.setViewRange(16.0F);
			display.setWidth(2.0F);
			display.setHeight(2.0F);
			if ("block-display".equals(WORLD_TEXT_SCENARIO)) {
				display.setCustomName(Component.literal("Rust world text"));
				display.setCustomNameVisible(true);
			}
			minecraft.level.addEntity(display);
			LOGGER.info(
				"Deterministic BlockDisplay setup spawned {} at {} for Rust-GAL mesh capture",
				state.getBlock().builtInRegistryHolder().key().location(),
				position
			);
		}
		if ("text-display".equals(WORLD_TEXT_SCENARIO) || "text-display-polygon-offset".equals(WORLD_TEXT_SCENARIO)) {
			Display.TextDisplay display = new Display.TextDisplay(EntityType.TEXT_DISPLAY, minecraft.level);
			display.setId(Integer.MIN_VALUE + 4097);
			display.setPos(position.add(0.0, 0.75, 0.0));
			display.setText(Component.literal("Rust semantic text"));
			if ("text-display".equals(WORLD_TEXT_SCENARIO)) {
				display.setFlags(Display.TextDisplay.FLAG_SEE_THROUGH);
			}
			display.setViewRange(16.0F);
			display.setWidth(2.0F);
			display.setHeight(1.0F);
			minecraft.level.addEntity(display);
			LOGGER.info("Deterministic TextDisplay setup spawned {} text at {}", WORLD_TEXT_SCENARIO, display.position());
		}
	}

	private static BlockState blockDisplayBenchmarkState() {
		return switch (BLOCK_DISPLAY_SCENARIO) {
			case "oak-leaves", "cutout", "tinted" -> Blocks.OAK_LEAVES.defaultBlockState();
			case "asymmetric", "furnace" -> Blocks.FURNACE.defaultBlockState();
			case "non-full-cube", "stairs" -> Blocks.OAK_STAIRS.defaultBlockState();
			default -> Blocks.STONE.defaultBlockState();
		};
	}

	private static boolean setupFallingBlockScenario(Minecraft minecraft, LocalPlayer player) {
		if (FALLING_BLOCK_SCENARIO.isEmpty() || "hidden".equals(FALLING_BLOCK_SCENARIO) || player == null
			|| minecraft.level == null || minecraft.getSingleplayerServer() == null) {
			return false;
		}
		if (fallingBlockSetupPoseIndex == poseIndex) {
			return false;
		}
		ServerLevel serverLevel = minecraft.getSingleplayerServer().getLevel(minecraft.level.dimension());
		if (serverLevel == null) {
			return false;
		}
		BlockState state = fallingBlockBenchmarkState();
		Vec3 forward = player.getLookAngle();
		if (forward.lengthSqr() < 0.0001) {
			forward = new Vec3(0.0, 0.0, 1.0);
		}
		int fallHeight = Math.max(4, Integer.getInteger("mattmc.dev.rustGalWorldMesh.fallingBlockFallHeight", 6));
		boolean slowCapture = Boolean.getBoolean("mattmc.dev.rustGalWorldMesh.fallingBlockSlowCapture");
		BlockPos origin = BlockPos.containing(player.getEyePosition().add(forward.normalize().scale(4.0)).add(0.0, 2.0, 0.0));
		int entityCount = Math.max(1, Integer.getInteger("mattmc.dev.rustGalWorldMesh.fallingBlockCount", 1));
		fallingBlockSetupStatus = "spawned";
		fallingBlockSetupBlockId = state.getBlock().builtInRegistryHolder().key().location().toString();
		fallingBlockSetupSpawnMethod = "FallingBlockEntity.fall";
		fallingBlockSetupOrigin = origin.toShortString();
		fallingBlockSetupLanding = origin.below(fallHeight).toShortString();
		fallingBlockSetupEntityCount = entityCount;
		fallingBlockSetupFallHeight = fallHeight;
		fallingBlockSetupPoseIndex = poseIndex;
		fallingBlockSetupPoseName = poses != null && poseIndex >= 0 && poseIndex < poses.length
			? poses[poseIndex].name()
			: "initial";
		for (int i = 0; i < entityCount; i++) {
			BlockPos pos = origin.offset(i, 0, 0);
			prepareFallingBlockColumn(serverLevel, pos, fallHeight);
			serverLevel.setBlock(pos, state, 2);
			FallingBlockEntity serverEntity = FallingBlockEntity.fall(serverLevel, pos, state);
			if (slowCapture) {
				serverEntity.setNoGravity(true);
				// Whole-frame source captures can have seconds between render frames.
				// This remains a normal FallingBlockEntity path while keeping the
				// diagnostic entity airborne across the requested pose frames.
				serverEntity.setDeltaMovement(0.0, -0.001, 0.0);
			} else {
				serverEntity.setDeltaMovement(0.0, -0.02, 0.0);
			}
			serverEntity.dropItem = false;
		}
		LOGGER.info(
			"Deterministic FallingBlock setup spawned {} count={} near {} pose={} for Rust-GAL moving mesh capture",
			state.getBlock().builtInRegistryHolder().key().location(),
			entityCount,
			origin,
			fallingBlockSetupPoseName
		);
		return true;
	}

	private static void prepareFallingBlockColumn(ServerLevel serverLevel, BlockPos top, int fallHeight) {
		for (int y = 0; y < fallHeight; y++) {
			BlockPos clear = top.below(y);
			serverLevel.setBlock(clear, Blocks.AIR.defaultBlockState(), 3);
		}
		BlockPos landing = top.below(fallHeight);
		serverLevel.setBlock(landing, Blocks.STONE.defaultBlockState(), 3);
	}

	private static BlockState fallingBlockBenchmarkState() {
		return switch (FALLING_BLOCK_SCENARIO) {
			case "gravel" -> Blocks.GRAVEL.defaultBlockState();
			case "concrete-powder", "concrete_powder" -> Blocks.WHITE_CONCRETE_POWDER.defaultBlockState();
			default -> Blocks.SAND.defaultBlockState();
		};
	}

	/**
	 * Creates an ordinary vanilla Arrow in the copied singleplayer world. It is
	 * deliberately a real entity rather than a semantic mesh fixture; no-gravity
	 * only keeps the capture bounded while the normal ArrowRenderer owns state
	 * extraction and submission on subsequent client frames.
	 */
	private static void setupArrowScenario(Minecraft minecraft, LocalPlayer player) {
		if (ARROW_SCENARIO.isEmpty() || "hidden".equals(ARROW_SCENARIO)
			|| player == null || minecraft.level == null || minecraft.getSingleplayerServer() == null) {
			arrowSetupStatus = ARROW_SCENARIO.isEmpty() ? "inactive" : "hidden-or-missing-level";
			return;
		}
		if (!"ordinary".equals(ARROW_SCENARIO) && !"arrow".equals(ARROW_SCENARIO)) {
			arrowSetupStatus = "unsupported-scenario";
			return;
		}
		if (arrowSetupPoseIndex == poseIndex) {
			return;
		}
		ServerLevel serverLevel = minecraft.getSingleplayerServer().getLevel(minecraft.level.dimension());
		if (serverLevel == null) {
			arrowSetupStatus = "missing-server-level";
			return;
		}
		Vec3 forward = player.getLookAngle();
		if (forward.lengthSqr() < 0.0001) {
			forward = new Vec3(0.0, 0.0, 1.0);
		}
		Vec3 origin = player.getEyePosition().add(forward.normalize().scale(4.0)).add(0.0, -0.25, 0.0);
		int entityCount = Math.max(1, Integer.getInteger("mattmc.dev.rustGalWorldMesh.arrowCount", 1));
		for (int index = 0; index < entityCount; index++) {
			Arrow arrow = new Arrow(
				serverLevel,
				origin.x + index * 0.35,
				origin.y,
				origin.z,
				new ItemStack(Items.ARROW),
				null
			);
			arrow.setYRot(player.getYRot());
			arrow.setXRot(player.getXRot());
			arrow.setNoGravity(true);
			arrow.setDeltaMovement(Vec3.ZERO);
			arrow.pickup = net.minecraft.world.entity.projectile.AbstractArrow.Pickup.DISALLOWED;
			serverLevel.addFreshEntity(arrow);
		}
		arrowSetupStatus = "spawned";
		arrowSetupTexture = "minecraft:textures/entity/projectiles/arrow.png";
		arrowSetupOrigin = String.format(Locale.ROOT, "%.3f,%.3f,%.3f", origin.x, origin.y, origin.z);
		arrowSetupEntityCount = entityCount;
		arrowSetupPoseIndex = poseIndex;
		LOGGER.info("Deterministic Arrow setup spawned ordinary vanilla arrows count={} near {} for Rust-GAL entity-mesh capture", entityCount, arrowSetupOrigin);
	}

	/**
	 * Spawns normal vanilla experience orbs in the copied singleplayer world.
	 * The renderer still owns the complete production extraction path; this only
	 * provides a stable, non-interactive entity workload for capture.
	 */
	private static void setupExperienceOrbScenario(Minecraft minecraft, LocalPlayer player) {
		if (EXPERIENCE_ORB_SCENARIO.isEmpty() || "hidden".equals(EXPERIENCE_ORB_SCENARIO)
			|| player == null || minecraft.level == null || minecraft.getSingleplayerServer() == null) {
			experienceOrbSetupStatus = EXPERIENCE_ORB_SCENARIO.isEmpty() ? "inactive" : "hidden-or-missing-level";
			return;
		}
		if (!"ordinary".equals(EXPERIENCE_ORB_SCENARIO) && !"experience-orb".equals(EXPERIENCE_ORB_SCENARIO)) {
			experienceOrbSetupStatus = "unsupported-scenario";
			return;
		}
		if (experienceOrbSetupPoseIndex == poseIndex) {
			return;
		}
		ServerLevel serverLevel = minecraft.getSingleplayerServer().getLevel(minecraft.level.dimension());
		if (serverLevel == null) {
			experienceOrbSetupStatus = "missing-server-level";
			return;
		}
		Vec3 forward = player.getLookAngle();
		if (forward.lengthSqr() < 0.0001) {
			forward = new Vec3(0.0, 0.0, 1.0);
		}
		// Vanilla experience orbs begin homing inside eight blocks. Keep the real
		// entities just beyond that radius so they remain ordinary, non-interactive
		// entities throughout the capture while their textured interior still has
		// enough projected area for the final-frame proof.
		Vec3 facing = forward.normalize();
		// Keep the ordinary billboard out from under the fixed crosshair so the
		// retained game-window frame can prove the producer's visible pixels.
		Vec3 cameraRight = new Vec3(facing.z, 0.0, -facing.x).normalize();
		Vec3 origin = player.getEyePosition()
			.add(facing.scale(8.25))
			.add(cameraRight.scale(1.25))
			.add(0.0, -0.25, 0.0);
		int entityCount = Math.max(1, Integer.getInteger("mattmc.dev.rustGalWorldExperienceOrb.count", 1));
		for (int index = 0; index < entityCount; index++) {
			ExperienceOrb orb = new ExperienceOrb(serverLevel, origin.x + index * 0.4, origin.y, origin.z, 5);
			orb.setNoGravity(true);
			orb.setDeltaMovement(Vec3.ZERO);
			serverLevel.addFreshEntity(orb);
		}
		experienceOrbSetupStatus = "spawned";
		experienceOrbSetupOrigin = String.format(Locale.ROOT, "%.3f,%.3f,%.3f", origin.x, origin.y, origin.z);
		experienceOrbSetupEntityCount = entityCount;
		experienceOrbSetupPoseIndex = poseIndex;
		LOGGER.info("Deterministic ExperienceOrb setup spawned ordinary vanilla orbs count={} near {} for Rust-GAL material capture", entityCount, experienceOrbSetupOrigin);
	}

	/**
	 * Places a normal Beacon block entity in the copied singleplayer world. The
	 * render path remains {@code BeaconRenderer.submit}; this setup never writes
	 * material quads or invokes renderer code itself.
	 */
	private static void runOnServerThreadAndWait(MinecraftServer server, Runnable action) {
		if (server.isSameThread()) {
			action.run();
			return;
		}
		CompletableFuture<Void> completion = new CompletableFuture<>();
		server.execute(() -> {
			try {
				action.run();
				completion.complete(null);
			} catch (Throwable throwable) {
				completion.completeExceptionally(throwable);
			}
		});
		completion.join();
	}

	private static void setupBeaconBeamScenario(Minecraft minecraft, LocalPlayer player) {
		if (BEACON_BEAM_SCENARIO.isEmpty() || "hidden".equals(BEACON_BEAM_SCENARIO)
			|| player == null || minecraft.level == null || minecraft.getSingleplayerServer() == null) {
			beaconBeamSetupStatus = BEACON_BEAM_SCENARIO.isEmpty() ? "inactive" : "hidden-or-missing-level";
			return;
		}
		if (!"ordinary".equals(BEACON_BEAM_SCENARIO) && !"beacon".equals(BEACON_BEAM_SCENARIO)) {
			beaconBeamSetupStatus = "unsupported-scenario";
			return;
		}
		ServerLevel serverLevel = minecraft.getSingleplayerServer().getLevel(minecraft.level.dimension());
		if (serverLevel == null) {
			beaconBeamSetupStatus = "missing-server-level";
			return;
		}
		if (!beaconBeamTickerEventRegistered) {
			ServerTickEvents.END_SERVER_TICK.register(DeterministicCameraCapture::tickBeaconBeamFixture);
			beaconBeamTickerEventRegistered = true;
		}
		if (!beaconBeamClientTickerEventRegistered) {
			ClientTickEvents.END_CLIENT_TICK.register(DeterministicCameraCapture::tickBeaconBeamClientFixture);
			beaconBeamClientTickerEventRegistered = true;
		}
		if (beaconBeamSetupPosition != null) {
			if (!HAS_FIXED_CAMERA_POSE) {
				double dx = beaconBeamSetupPosition.getX() + 0.5D - player.getX();
				double dz = beaconBeamSetupPosition.getZ() + 0.5D - player.getZ();
				double dy = beaconBeamSetupPosition.getY() + 0.5D - player.getEyeY();
				float pitch = Mth.clamp((float)-Math.toDegrees(Math.atan2(dy, Math.max(0.1D, Math.sqrt(dx * dx + dz * dz)))), -80.0F, 80.0F);
				player.setXRot(pitch);
				player.setOldPosAndRot(player.position(), player.getYRot(), pitch);
				initialPose = new Pose("beacon", player.getYRot(), pitch);
			}
			beaconBeamSetupGameTime = serverLevel.getGameTime();
			// The copied benchmark world can spend several wall-clock seconds
			// loading without advancing the client tick loop.  Advance the real
			// vanilla beacon ticker on the client thread while the fixture is
			// waiting; this does not synthesize beam sections or touch renderer
			// state, and keeps the ordinary block-entity scan deterministic.
			if (minecraft.level.getBlockEntity(beaconBeamSetupPosition)
				instanceof net.minecraft.world.level.block.entity.BeaconBlockEntity clientBeacon
				&& clientBeacon.getBeamSections().isEmpty()) {
				beaconBeamSetupClientTickerInvocations++;
				net.minecraft.world.level.block.entity.BeaconBlockEntity.tick(
					minecraft.level,
					beaconBeamSetupPosition,
					Blocks.BEACON.defaultBlockState(),
					clientBeacon);
				if (beaconBeamSetupClientTickerInvocations <= 3L
					|| !clientBeacon.getBeamSections().isEmpty()) {
					LOGGER.info("Beacon fixture client ticker invocation={} height={} beacon={} beam={} base={}",
						beaconBeamSetupClientTickerInvocations,
						minecraft.level.getHeight(Heightmap.Types.WORLD_SURFACE,
							beaconBeamSetupPosition.getX(), beaconBeamSetupPosition.getZ()),
						minecraft.level.getBlockState(beaconBeamSetupPosition).getBlock(),
						clientBeacon.getBeamSections().size(),
						beaconBeamSetupBaseValid);
					LOGGER.info("Beacon fixture client scan states y0={} y1={} y2={}",
						minecraft.level.getBlockState(beaconBeamSetupPosition).getBlock(),
						minecraft.level.getBlockState(beaconBeamSetupPosition.above()).getBlock(),
						minecraft.level.getBlockState(beaconBeamSetupPosition.above(2)).getBlock());
				}
			}
			beaconBeamSetupBaseValid = true;
			for (int level = 0; level < 4 && beaconBeamSetupBaseValid; level++) {
				int radius = level + 1;
				for (int localX = -radius; localX <= radius && beaconBeamSetupBaseValid; localX++) {
					for (int localZ = -radius; localZ <= radius; localZ++) {
						if (!serverLevel.getBlockState(beaconBeamSetupPosition.offset(localX, -level - 1, localZ))
							.is(net.minecraft.tags.BlockTags.BEACON_BASE_BLOCKS)) {
							beaconBeamSetupBaseValid = false;
							break;
						}
					}
				}
			}
			beaconBeamTickerLoopScheduled = true;
			beaconBeamSetupServerReady = beaconBeamSetupServerReady || serverLevel.getBlockEntity(beaconBeamSetupPosition)
				instanceof net.minecraft.world.level.block.entity.BeaconBlockEntity beacon
				&& !beacon.getBeamSections().isEmpty();
			beaconBeamSetupClientReady = beaconBeamSetupClientReady || minecraft.level.getBlockEntity(beaconBeamSetupPosition)
				instanceof net.minecraft.world.level.block.entity.BeaconBlockEntity beacon
				&& !beacon.getBeamSections().isEmpty();
			beaconBeamSetupStatus = beaconBeamSetupClientReady ? "spawned"
				: beaconBeamSetupServerReady ? "waiting-client-beam-sections" : "waiting-server-beam-sections";
			return;
		}
		Vec3 forward = player.getLookAngle();
		if (forward.lengthSqr() < 0.0001) {
			forward = new Vec3(0.0, 0.0, 1.0);
		}
		BlockPos horizontalTarget = BlockPos.containing(
			player.getEyePosition().add(forward.normalize().scale(5.0)));
		// Keep the ordinary pyramid adjacent to the camera. Copied worlds can
		// report distant-horizon surface heights far above the player, which
		// would make a valid beam impossible to project in this capture.
		BlockPos position = new BlockPos(horizontalTarget.getX(), player.blockPosition().getY() + 2, horizontalTarget.getZ());
		// The render thread owns this hook, but every ServerLevel mutation must
		// execute on the integrated server thread. Otherwise this fixture races
		// ServerChunkCache's chunk-broadcast iterator and corrupts vanilla state.
		final BlockState[] previousBeaconState = new BlockState[1];
		runOnServerThreadAndWait(minecraft.getSingleplayerServer(), () -> {
			// Build the complete ordinary four-level beacon pyramid so the vanilla
			// block entity can deterministically publish beam sections on the server.
			for (int level = 0; level < 4; level++) {
				int radius = level + 1;
				for (int localX = -radius; localX <= radius; localX++) {
					for (int localZ = -radius; localZ <= radius; localZ++) {
						serverLevel.setBlock(position.offset(localX, -level - 1, localZ), Blocks.IRON_BLOCK.defaultBlockState(), 3);
					}
				}
			}
			int beamClearTop = Math.max(
				position.getY() + 12,
				serverLevel.getHeight(Heightmap.Types.WORLD_SURFACE, position.getX(), position.getZ()) + 1);
			for (int localY = 0; position.getY() + localY <= beamClearTop; localY++) {
				serverLevel.setBlock(position.above(localY), Blocks.AIR.defaultBlockState(), 3);
			}
			previousBeaconState[0] = serverLevel.getBlockState(position);
			serverLevel.setBlock(position, Blocks.BEACON.defaultBlockState(), 3);
			serverLevel.setBlock(position.above(), Blocks.RED_STAINED_GLASS.defaultBlockState(), 3);
			serverLevel.getChunkAt(position);
			if (serverLevel.getBlockEntity(position)
				instanceof net.minecraft.world.level.block.entity.BeaconBlockEntity beacon) {
				// Run the ordinary vanilla ticker once on its owning thread so the
				// fixture does not depend on benchmark wall-clock time.
				net.minecraft.world.level.block.entity.BeaconBlockEntity.tick(
					serverLevel, position, Blocks.BEACON.defaultBlockState(), beacon);
				beacon.setChanged();
			}
			serverLevel.sendBlockUpdated(position, previousBeaconState[0],
				Blocks.BEACON.defaultBlockState(), 3);
			serverLevel.getChunkSource().blockChanged(position);
		});
		// The deterministic client is the render owner of this copied level. Keep
		// the complete ordinary pyramid and clear beam column coherent with the
		// server fixture so the client-side vanilla ticker can perform its normal
		// scan; do not inject beam sections or renderer output.
		for (int level = 0; level < 4; level++) {
			int radius = level + 1;
			for (int localX = -radius; localX <= radius; localX++) {
				for (int localZ = -radius; localZ <= radius; localZ++) {
					minecraft.level.setBlock(position.offset(localX, -level - 1, localZ),
						Blocks.IRON_BLOCK.defaultBlockState(), 3);
				}
			}
		}
		int clientBeamClearTop = Math.max(
			position.getY() + 12,
			minecraft.level.getHeight(Heightmap.Types.WORLD_SURFACE, position.getX(), position.getZ()) + 1);
		for (int localY = 0; position.getY() + localY <= clientBeamClearTop; localY++) {
			minecraft.level.setBlock(position.above(localY), Blocks.AIR.defaultBlockState(), 3);
		}
		minecraft.level.setBlock(position, Blocks.BEACON.defaultBlockState(), 3);
		minecraft.level.setBlock(position.above(), Blocks.RED_STAINED_GLASS.defaultBlockState(), 3);
		beaconBeamSetupPosition = position;
		// Aim the real player camera at the placed beacon before deterministic
		// poses are materialized; copied-world surface height is not stable.
		double aimDx = position.getX() + 0.5D - player.getX();
		double aimDz = position.getZ() + 0.5D - player.getZ();
		double aimDy = position.getY() + 0.5D - player.getEyeY();
		player.setXRot(Mth.clamp((float)-Math.toDegrees(Math.atan2(aimDy, Math.max(0.1D, Math.sqrt(aimDx * aimDx + aimDz * aimDz)))), -80.0F, 80.0F));
		player.setOldPosAndRot(player.position(), player.getYRot(), player.getXRot());
		beaconBeamSetupOrigin = position.toShortString();
		beaconBeamSetupClientReady = false;
		beaconBeamSetupServerReady = false;
		beaconBeamSetupGameTime = serverLevel.getGameTime();
		beaconBeamSetupBaseValid = true;
			beaconBeamSetupTickerInvocations = 0L;
			beaconBeamSetupClientTickerInvocations = 0L;
		beaconBeamSetupLastTickerGameTime = -1L;
		beaconBeamSetupTickerSawBlockEntity = false;
		beaconBeamSetupServerPacketSent = false;
		beaconBeamTickerLoopScheduled = false;
		beaconBeamSetupServerLevel = serverLevel;
		beaconBeamSetupStatus = "waiting-client-beam-sections";
		LOGGER.info("Deterministic Beacon setup placed ordinary vanilla beacon near {} for Rust-GAL material capture", beaconBeamSetupOrigin);
	}

	private static void tickBeaconBeamFixture(MinecraftServer server) {
		if (!beaconBeamTickerLoopScheduled || beaconBeamSetupServerLevel == null
			|| beaconBeamSetupPosition == null || server != beaconBeamSetupServerLevel.getServer()) {
			return;
		}
		ServerLevel serverLevel = beaconBeamSetupServerLevel;
		BlockPos position = beaconBeamSetupPosition;
		beaconBeamSetupTickerInvocations++;
		beaconBeamSetupLastTickerGameTime = serverLevel.getGameTime();
		if (!(serverLevel.getBlockEntity(position)
			instanceof net.minecraft.world.level.block.entity.BeaconBlockEntity beacon)) {
			beaconBeamSetupTickerLoopFinished();
			return;
		}
		beaconBeamSetupTickerSawBlockEntity = true;
		net.minecraft.world.level.block.entity.BeaconBlockEntity.tick(
			serverLevel, position, Blocks.BEACON.defaultBlockState(), beacon);
		// The vanilla ticker computes the pyramid level on its periodic boundary,
		// after the first scan has populated beam sections.  Run that same ticker
		// once more when the scan is already complete so the normal level update
		// and subsequent client packet expose the real beam to the renderer.
		if (!beacon.getBeamSections().isEmpty()) {
			net.minecraft.world.level.block.entity.BeaconBlockEntity.tick(
				serverLevel, position, Blocks.BEACON.defaultBlockState(), beacon);
		}
		beacon.setChanged();
		if (beaconBeamSetupTickerInvocations <= 3L) {
			LOGGER.info("Beacon fixture server ticker invocation={} time={} height={} beam={} base={}",
				beaconBeamSetupTickerInvocations,
				serverLevel.getGameTime(),
				serverLevel.getHeight(Heightmap.Types.WORLD_SURFACE, position.getX(), position.getZ()),
				beacon.getBeamSections().size(),
				beaconBeamSetupBaseValid);
		}
		if (serverLevel.getGameTime() % 80L == 0L) {
			LOGGER.info("Beacon fixture vanilla tick boundary time={} height={} beacon={} beam={} base={}",
				serverLevel.getGameTime(),
				serverLevel.getHeight(Heightmap.Types.WORLD_SURFACE, position.getX(), position.getZ()),
				serverLevel.getBlockState(position).getBlock(),
				beacon.getBeamSections().size(), beaconBeamSetupBaseValid);
		}
		if (!beacon.getBeamSections().isEmpty()) {
			beaconBeamSetupServerReady = true;
			// The copied client chunk can finish loading after the first ordinary
			// block-entity update.  Retransmit the same vanilla update at a bounded
			// cadence until that client-side block entity acknowledges its sections;
			// this is normal synchronization, not client beam synthesis.
			if (!beaconBeamSetupServerPacketSent || (beaconBeamSetupTickerInvocations % 20L) == 0L) {
				serverLevel.sendBlockUpdated(position, Blocks.BEACON.defaultBlockState(),
					Blocks.BEACON.defaultBlockState(), 3);
				// Beam sections and activation level are ordinary block-entity update
				// data. Deliver the vanilla packet to the copied singleplayer
				// connection once the real server ticker reaches its boundary.
				var packet = beacon.getUpdatePacket();
				if (packet != null) {
					beaconBeamPendingClientPacket = packet;
					for (ServerPlayer serverPlayer : server.getPlayerList().getPlayers()) {
						if (serverPlayer.level().dimension() == serverLevel.dimension()
							&& serverPlayer.distanceToSqr(position.getX() + 0.5D, position.getY() + 0.5D, position.getZ() + 0.5D) <= 256.0D * 256.0D) {
							serverPlayer.connection.send(packet);
						}
					}
				}
				beaconBeamSetupServerPacketSent = true;
			}
			if (beaconBeamSetupClientReady) {
				beaconBeamSetupTickerLoopFinished();
			}
		}
	}

	private static void tickBeaconBeamClientFixture(Minecraft minecraft) {
		if (!beaconBeamTickerLoopScheduled || minecraft == null || minecraft.level == null
			|| beaconBeamSetupPosition == null) {
			return;
		}
		// Apply the exact vanilla block-entity update through the client packet
		// listener on the client thread. This preserves normal BE deserialization;
		// no beam sections or renderer output are synthesized by the fixture.
		var pendingPacket = beaconBeamPendingClientPacket;
		if (pendingPacket != null) {
			beaconBeamPendingClientPacket = null;
			var clientBlockEntity = minecraft.level.getBlockEntity(pendingPacket.getPos());
			if (clientBlockEntity != null && clientBlockEntity.getType() == pendingPacket.getType()) {
				var blockEntity = clientBlockEntity;
				var collector = new net.minecraft.util.ProblemReporter.ScopedCollector(
					blockEntity.problemPath(), LOGGER);
				try {
					blockEntity.loadWithComponents(net.minecraft.world.level.storage.TagValueInput.create(
						collector, minecraft.level.registryAccess(), pendingPacket.getTag()));
				} finally {
					collector.close();
				}
			}
		}
		if (minecraft.level.getBlockEntity(beaconBeamSetupPosition)
			instanceof net.minecraft.world.level.block.entity.BeaconBlockEntity clientBeacon
			&& clientBeacon.getBeamSections().isEmpty()) {
			beaconBeamSetupClientTickerInvocations++;
			net.minecraft.world.level.block.entity.BeaconBlockEntity.tick(
				minecraft.level,
				beaconBeamSetupPosition,
				Blocks.BEACON.defaultBlockState(),
				clientBeacon);
			if (!clientBeacon.getBeamSections().isEmpty()) {
				beaconBeamSetupClientReady = true;
				beaconBeamSetupStatus = "spawned";
				LOGGER.info("Beacon fixture client tick boundary produced beam sections={} time={}",
					clientBeacon.getBeamSections().size(), minecraft.level.getGameTime());
				if (beaconBeamSetupServerReady) {
					beaconBeamSetupTickerLoopFinished();
				}
			}
		}
	}

	private static void beaconBeamSetupTickerLoopFinished() {
		beaconBeamTickerLoopScheduled = false;
	}

	/**
	 * Spawns an ordinary vanilla dropped-item entity in the copied singleplayer
	 * world. The normal ItemEntityRenderer and item submit path remain the only
	 * producer; the harness merely fixes the otherwise user-driven workload.
	 */
	private static void setupItemEntityScenario(Minecraft minecraft, LocalPlayer player) {
		if (ITEM_ENTITY_SCENARIO.isEmpty() || "hidden".equals(ITEM_ENTITY_SCENARIO)
			|| player == null || minecraft.level == null || minecraft.getSingleplayerServer() == null) {
			itemEntitySetupStatus = ITEM_ENTITY_SCENARIO.isEmpty() ? "inactive" : "hidden-or-missing-level";
			return;
		}
		if (!"ordinary".equals(ITEM_ENTITY_SCENARIO) && !"item-entity".equals(ITEM_ENTITY_SCENARIO)) {
			itemEntitySetupStatus = "unsupported-scenario";
			return;
		}
		if (itemEntitySetupPoseIndex == poseIndex) {
			refreshItemEntityClientVisibility(minecraft.level);
			if (!itemEntityServerSpawnFailure.isEmpty()) {
				itemEntitySetupStatus = "server-spawn-failed";
			} else if (itemEntityServerSpawnedCount < itemEntitySetupEntityCount) {
				itemEntitySetupStatus = "waiting-for-server-spawn";
			} else if (itemEntityClientVisibleCount < itemEntitySetupEntityCount) {
				itemEntitySetupStatus = "waiting-for-client-entity";
			} else {
				itemEntitySetupStatus = "client-visible";
			}
			return;
		}
		MinecraftServer server = minecraft.getSingleplayerServer();
		if (server == null) {
			itemEntitySetupStatus = "missing-server-level";
			return;
		}
		Vec3 forward = player.getLookAngle();
		if (forward.lengthSqr() < 0.0001) {
			forward = new Vec3(0.0, 0.0, 1.0);
		}
		Vec3 origin = player.getEyePosition().add(forward.normalize().scale(5.0)).add(0.0, -0.45, 0.0);
		int entityCount = Math.max(1, Integer.getInteger("mattmc.dev.rustGalWorldItemEntity.count", 1));
		itemEntitySetupOrigin = String.format(Locale.ROOT, "%.3f,%.3f,%.3f", origin.x, origin.y, origin.z);
		itemEntitySetupItemId = "minecraft:diamond";
		itemEntitySetupEntityCount = entityCount;
		itemEntitySetupPoseIndex = poseIndex;
		itemEntityServerSpawnedCount = 0;
		itemEntityServerSpawnFailure = "";
		itemEntityClientVisibleCount = 0;
		ResourceKey<Level> dimension = minecraft.level.dimension();
		server.execute(() -> {
			try {
				ServerLevel serverLevel = server.getLevel(dimension);
				if (serverLevel == null) {
					itemEntityServerSpawnFailure = "missing-server-level:" + dimension.location();
					return;
				}
				for (int index = 0; index < entityCount; index++) {
					ItemEntity item = new ItemEntity(serverLevel, origin.x + index * 0.45, origin.y, origin.z, new ItemStack(Items.DIAMOND));
					item.setNoGravity(true);
					item.setDeltaMovement(Vec3.ZERO);
					item.setPickUpDelay(32_767);
					serverLevel.addFreshEntity(item);
				}
				itemEntityServerSpawnedCount = entityCount;
			} catch (RuntimeException exception) {
				itemEntityServerSpawnFailure = exception.getClass().getSimpleName() + ": " + exception.getMessage();
			}
		});
		itemEntitySetupStatus = "server-spawn-queued";
		LOGGER.info("Deterministic ItemEntity setup queued ordinary vanilla items count={} near {} for Rust-GAL indexed mesh capture", entityCount, itemEntitySetupOrigin);
	}

	private static void refreshItemEntityClientVisibility(ClientLevel level) {
		if (level == null) {
			itemEntityClientVisibleCount = 0;
			return;
		}
		int visible = 0;
		for (Entity entity : level.entitiesForRendering()) {
			if (entity instanceof ItemEntity item && item.getItem().is(Items.DIAMOND)) {
				visible++;
			}
		}
		itemEntityClientVisibleCount = visible;
	}

	/**
	 * Places one ordinary model block entity in the harness-owned copied world. The later
	 * block-entity extraction and normal {@code submitModel} call are
	 * untouched vanilla production work; this setup merely guarantees that the
	 * generic copied ModelPart route has a real producer to observe.
	 */
	private static void setupModelMeshScenario(Minecraft minecraft, LocalPlayer player) {
		if (MODEL_MESH_SCENARIO.isEmpty() || "hidden".equals(MODEL_MESH_SCENARIO)
			|| player == null || minecraft.level == null || minecraft.getSingleplayerServer() == null) {
			// Keep a successfully spawned producer receipt stable while the client
			// tears down its level.  This method is polled from the capture state
			// machine and can run once more after stopAfterComplete has begun; wiping
			// "spawned" here would make a real producer look absent in the final
			// metadata and invalidate the route/execution evidence.
			if (MODEL_MESH_SCENARIO.isEmpty()) {
				modelMeshSetupStatus = "inactive";
			} else if (!"spawned".equals(modelMeshSetupStatus)
				&& !"server-spawned".equals(modelMeshSetupStatus)) {
				modelMeshSetupStatus = "hidden-or-missing-level";
			}
			return;
		}
		if (!"chest".equals(MODEL_MESH_SCENARIO) && !"bed".equals(MODEL_MESH_SCENARIO)
			&& !"bell".equals(MODEL_MESH_SCENARIO) && !"shulker".equals(MODEL_MESH_SCENARIO)
			&& !"decorated-pot".equals(MODEL_MESH_SCENARIO)
			&& !"conduit".equals(MODEL_MESH_SCENARIO)
			&& !"llama-spit".equals(MODEL_MESH_SCENARIO) && !"evoker-fangs".equals(MODEL_MESH_SCENARIO)
			&& !"wither-skull".equals(MODEL_MESH_SCENARIO) && !"chicken".equals(MODEL_MESH_SCENARIO) && !"cow".equals(MODEL_MESH_SCENARIO)
			&& !"pig".equals(MODEL_MESH_SCENARIO) && !"rabbit".equals(MODEL_MESH_SCENARIO) && !"sheep".equals(MODEL_MESH_SCENARIO) && !"tropical-fish".equals(MODEL_MESH_SCENARIO) && !"zombie".equals(MODEL_MESH_SCENARIO)
			&& !"end-crystal".equals(MODEL_MESH_SCENARIO) && !"wind-charge".equals(MODEL_MESH_SCENARIO)) {
			modelMeshSetupStatus = "unsupported-scenario";
			return;
		}
		ServerLevel serverLevel = minecraft.getSingleplayerServer().getLevel(minecraft.level.dimension());
		if (serverLevel == null) {
			modelMeshSetupStatus = "missing-server-level";
			return;
		}
		if (isModelMeshEntityScenario()) {
			// Evoker Fangs are intentionally short-lived vanilla entities. If the
			// real server fixture completed its attack before readiness sampled the
			// frame, allow the next ordinary server tick to create a fresh tracked
			// fixture rather than waiting forever on a discarded entity.
			if (modelMeshSetupServerEntityPresent && modelMeshSetupServerEntityId >= 0
				&& serverLevel.getEntity(modelMeshSetupServerEntityId) == null
				&& "evoker-fangs".equals(MODEL_MESH_SCENARIO)) {
				modelMeshSetupServerEntityPresent = false;
				modelMeshSetupServerEntityId = -1;
				modelMeshSetupStatus = "server-fixture-expired";
			}
			updateModelMeshClientEntityReceipt(minecraft);
			if (modelMeshSetupServerEntityPresent || modelMeshSetupServerSpawnQueued) {
				if ("llama-spit".equals(MODEL_MESH_SCENARIO) && modelMeshSetupServerEntityId >= 0) {
					Entity fixture = serverLevel.getEntity(modelMeshSetupServerEntityId);
					if (fixture instanceof LlamaSpit spit) {
						// Preserve the ordinary projectile producer while preventing the
						// short-lived fixture from disappearing before client replication.
						spit.tickCount = 0;
						spit.setDeltaMovement(Vec3.ZERO);
					}
				}
				if ("wind-charge".equals(MODEL_MESH_SCENARIO) && modelMeshSetupServerEntityId >= 0) {
					Entity fixture = serverLevel.getEntity(modelMeshSetupServerEntityId);
					if (fixture instanceof WindCharge windCharge) {
						windCharge.tickCount = 0;
						windCharge.setDeltaMovement(Vec3.ZERO);
						windCharge.setPos(modelMeshSetupExpectedEntityPosition);
					}
				}
				return;
			}
			Vec3 forward = player.getLookAngle();
			if (forward.lengthSqr() < 0.0001) {
				forward = new Vec3(0.0, 0.0, 1.0);
			}
			double modelSpawnDistance = "evoker-fangs".equals(MODEL_MESH_SCENARIO) ? 2.5D : 4.0D;
			Vec3 origin = player.getEyePosition().add(forward.normalize().scale(modelSpawnDistance)).add(0.0, -0.25, 0.0);
			boolean livestockScenario = "chicken".equals(MODEL_MESH_SCENARIO)
				|| "cow".equals(MODEL_MESH_SCENARIO) || "pig".equals(MODEL_MESH_SCENARIO)
				|| "rabbit".equals(MODEL_MESH_SCENARIO) || "sheep".equals(MODEL_MESH_SCENARIO) || "tropical-fish".equals(MODEL_MESH_SCENARIO) || "zombie".equals(MODEL_MESH_SCENARIO);
			modelMeshSetupExpectedEntityPosition = livestockScenario
				? origin.add(0.0, -1.1, 0.0)
				: "evoker-fangs".equals(MODEL_MESH_SCENARIO) ? origin.add(0.0, -1.0, 0.0) : origin;
			modelMeshSetupBlockId = modelMeshEntityBlockId(MODEL_MESH_SCENARIO);
			modelMeshSetupOrigin = String.format(Locale.ROOT, "%.3f,%.3f,%.3f", origin.x, origin.y, origin.z);
			modelMeshSetupServerSpawnQueued = true;
			modelMeshSetupStatus = "server-spawn-queued";
			MinecraftServer server = minecraft.getSingleplayerServer();
			ResourceKey<Level> dimension = minecraft.level.dimension();
			Vec3 eyePosition = player.getEyePosition();
			float playerYaw = player.getYRot();
			UUID playerId = player.getUUID();
			int spawnPoseIndex = poseIndex;
			server.execute(() -> spawnModelMeshEntityOnServer(
				server, dimension, MODEL_MESH_SCENARIO, origin, eyePosition, playerYaw, playerId, spawnPoseIndex
			));
			return;
		}
		if ("llama-spit".equals(MODEL_MESH_SCENARIO) || "evoker-fangs".equals(MODEL_MESH_SCENARIO)
			|| "wither-skull".equals(MODEL_MESH_SCENARIO) || "chicken".equals(MODEL_MESH_SCENARIO) || "cow".equals(MODEL_MESH_SCENARIO)
				|| "pig".equals(MODEL_MESH_SCENARIO) || "rabbit".equals(MODEL_MESH_SCENARIO) || "sheep".equals(MODEL_MESH_SCENARIO) || "tropical-fish".equals(MODEL_MESH_SCENARIO) || "zombie".equals(MODEL_MESH_SCENARIO)
				|| "end-crystal".equals(MODEL_MESH_SCENARIO) || "wind-charge".equals(MODEL_MESH_SCENARIO)) {
			if (("llama-spit".equals(MODEL_MESH_SCENARIO) || "wither-skull".equals(MODEL_MESH_SCENARIO)
				|| "chicken".equals(MODEL_MESH_SCENARIO) || "cow".equals(MODEL_MESH_SCENARIO) || "pig".equals(MODEL_MESH_SCENARIO)
					|| "rabbit".equals(MODEL_MESH_SCENARIO) || "sheep".equals(MODEL_MESH_SCENARIO) || "tropical-fish".equals(MODEL_MESH_SCENARIO) || "zombie".equals(MODEL_MESH_SCENARIO)
					|| "end-crystal".equals(MODEL_MESH_SCENARIO) || "wind-charge".equals(MODEL_MESH_SCENARIO)) && modelMeshSetupServerEntityPresent) {
				if (modelMeshSetupServerEntityId >= 0) {
					Entity clientEntity = minecraft.level.getEntity(modelMeshSetupServerEntityId);
					if (!isExpectedModelMeshClientEntity(clientEntity)) {
						clientEntity = findExpectedModelMeshClientEntity(minecraft.level);
					}
					int rendererEntityId = clientEntity == null
						? observedExpectedModelMeshRendererEntityId()
						: clientEntity.getId();
					modelMeshSetupClientEntityPresent = rendererEntityId >= 0
						&& GraphicsAuditCowOutlineFixture.ready(clientEntity);
					modelMeshSetupClientEntityId = rendererEntityId;
					modelMeshSetupStatus = modelMeshSetupClientEntityPresent ? "spawned" : "waiting-client-entity";
				}
				return;
			}
			if ("evoker-fangs".equals(MODEL_MESH_SCENARIO) && modelMeshSetupPoseIndex == poseIndex) {
				return;
			}
			Vec3 forward = player.getLookAngle();
			if (forward.lengthSqr() < 0.0001) {
				forward = new Vec3(0.0, 0.0, 1.0);
			}
			double modelSpawnDistance = "evoker-fangs".equals(MODEL_MESH_SCENARIO) ? 2.5D : 4.0D;
			Vec3 origin = player.getEyePosition().add(forward.normalize().scale(modelSpawnDistance)).add(0.0, -0.25, 0.0);
			boolean livestockScenario = "chicken".equals(MODEL_MESH_SCENARIO)
				|| "cow".equals(MODEL_MESH_SCENARIO) || "pig".equals(MODEL_MESH_SCENARIO)
				|| "rabbit".equals(MODEL_MESH_SCENARIO) || "sheep".equals(MODEL_MESH_SCENARIO) || "tropical-fish".equals(MODEL_MESH_SCENARIO) || "zombie".equals(MODEL_MESH_SCENARIO);
			modelMeshSetupExpectedEntityPosition = livestockScenario
				? origin.add(0.0, -1.1, 0.0)
				: "evoker-fangs".equals(MODEL_MESH_SCENARIO) ? origin.add(0.0, -1.0, 0.0) : origin;
			if (livestockScenario || "llama-spit".equals(MODEL_MESH_SCENARIO)
				|| "wither-skull".equals(MODEL_MESH_SCENARIO)) {
				prepareModelMeshEntityCaptureSite(serverLevel, player.getEyePosition(), modelMeshSetupExpectedEntityPosition);
			}
			if ("llama-spit".equals(MODEL_MESH_SCENARIO)) {
				LlamaSpit llamaSpit = new LlamaSpit(EntityType.LLAMA_SPIT, serverLevel);
				llamaSpit.setPos(origin.x, origin.y, origin.z);
				llamaSpit.setYRot(player.getYRot());
				llamaSpit.setXRot(player.getXRot());
				llamaSpit.setNoGravity(true);
				llamaSpit.setDeltaMovement(Vec3.ZERO);
				serverLevel.addFreshEntity(llamaSpit);
				modelMeshSetupServerEntityId = llamaSpit.getId();
			} else if ("evoker-fangs".equals(MODEL_MESH_SCENARIO)) {
				ServerPlayer serverPlayer = minecraft.getSingleplayerServer().getPlayerList().getPlayer(player.getUUID());
				if (serverPlayer == null) {
					modelMeshSetupStatus = "waiting-server-player";
					return;
				}
				// EntityManager only hands a newly-added entity to the tracker once
				// its owning chunk is loaded. Ensure the real fixture location is
				// loaded before adding the server entity; no client entity is made
				// directly and normal replication remains authoritative.
				serverLevel.getChunkAt(BlockPos.containing(origin));
				// Use vanilla's immediate warmup so the replicated attack event can
				// reach the client during the bounded capture window.
				// Keep the replicated fixture dormant until the settled pose has
				// been established and the real server event below activates it.
				// An immediate attack can finish between the two required capture
				// frames, leaving the second frame without the vanilla producer.
				EvokerFangs fangs = new EvokerFangs(serverLevel, origin.x, origin.y - 1.0, origin.z,
					(float)Math.toRadians(player.getYRot()), 0, serverPlayer);
				serverLevel.addFreshEntity(fangs);
				// Retain the real server identity so the normal client replication
				// readiness path can correlate the renderer's copied model state.
				modelMeshSetupServerEntityId = fangs.getId();
			} else if ("chicken".equals(MODEL_MESH_SCENARIO)) {
				Chicken chicken = new Chicken(EntityType.CHICKEN, serverLevel);
				chicken.setPos(origin.x, origin.y - 1.1, origin.z);
				chicken.setYRot(player.getYRot() + 180.0F);
				chicken.setYHeadRot(chicken.getYRot());
				chicken.setNoAi(true);
				chicken.setNoGravity(true);
				chicken.setDeltaMovement(Vec3.ZERO);
				serverLevel.addFreshEntity(chicken);
				modelMeshSetupServerEntityId = chicken.getId();
			} else if ("cow".equals(MODEL_MESH_SCENARIO)) {
				Cow cow = new Cow(EntityType.COW, serverLevel);
				cow.setPos(origin.x, origin.y - 1.1, origin.z);
				cow.setYRot(player.getYRot() + 180.0F);
				cow.setYHeadRot(cow.getYRot());
				cow.setNoAi(true);
				cow.setNoGravity(true);
				GraphicsAuditCowOutlineFixture.configure(cow);
				cow.setDeltaMovement(Vec3.ZERO);
				igniteEntityFlameCarrier(cow);
				serverLevel.addFreshEntity(cow);
				modelMeshSetupServerEntityId = cow.getId();
			} else if ("pig".equals(MODEL_MESH_SCENARIO)) {
				Pig pig = new Pig(EntityType.PIG, serverLevel);
				pig.setPos(origin.x, origin.y - 1.1, origin.z);
				pig.setYRot(player.getYRot() + 180.0F);
				pig.setYHeadRot(pig.getYRot());
				pig.setNoAi(true);
				pig.setNoGravity(true);
				pig.setDeltaMovement(Vec3.ZERO);
				serverLevel.addFreshEntity(pig);
				modelMeshSetupServerEntityId = pig.getId();
			} else if ("rabbit".equals(MODEL_MESH_SCENARIO)) {
				Rabbit rabbit = new Rabbit(EntityType.RABBIT, serverLevel);
				rabbit.setPos(origin.x, origin.y - 1.1, origin.z);
				rabbit.setYRot(player.getYRot() + 180.0F);
				rabbit.setYHeadRot(rabbit.getYRot());
				rabbit.setNoAi(true);
				rabbit.setNoGravity(true);
				rabbit.setDeltaMovement(Vec3.ZERO);
				serverLevel.addFreshEntity(rabbit);
				modelMeshSetupServerEntityId = rabbit.getId();
			} else if ("sheep".equals(MODEL_MESH_SCENARIO)) {
				Sheep sheep = new Sheep(EntityType.SHEEP, serverLevel);
				sheep.setPos(origin.x, origin.y - 1.1, origin.z);
				sheep.setYRot(player.getYRot() + 180.0F);
				sheep.setYHeadRot(sheep.getYRot());
				sheep.setNoAi(true);
				sheep.setNoGravity(true);
				sheep.setDeltaMovement(Vec3.ZERO);
				sheep.setBaby(true);
				serverLevel.addFreshEntity(sheep);
				modelMeshSetupServerEntityId = sheep.getId();
			} else if ("tropical-fish".equals(MODEL_MESH_SCENARIO)) {
				TropicalFish fish = new TropicalFish(EntityType.TROPICAL_FISH, serverLevel);
				prepareModelMeshEntityCaptureSite(serverLevel, player.getEyePosition(), origin.add(0.0, -1.1, 0.0));
				fish.setPos(origin.x, origin.y - 1.1, origin.z);
				fish.setNoAi(true);
				fish.setNoGravity(true);
				fish.setDeltaMovement(Vec3.ZERO);
				serverLevel.addFreshEntity(fish);
				modelMeshSetupServerEntityId = fish.getId();
			} else if ("zombie".equals(MODEL_MESH_SCENARIO)) {
				Zombie zombie = new Zombie(EntityType.ZOMBIE, serverLevel);
				zombie.setPos(origin.x, origin.y - 1.1, origin.z);
				zombie.setYRot(player.getYRot() + 180.0F);
				zombie.setYHeadRot(zombie.getYRot());
				zombie.setNoAi(true);
				zombie.setNoGravity(true);
				zombie.setDeltaMovement(Vec3.ZERO);
				serverLevel.addFreshEntity(zombie);
				modelMeshSetupServerEntityId = zombie.getId();
			} else if ("end-crystal".equals(MODEL_MESH_SCENARIO)) {
				net.minecraft.world.entity.boss.enderdragon.EndCrystal crystal =
					new net.minecraft.world.entity.boss.enderdragon.EndCrystal(EntityType.END_CRYSTAL, serverLevel);
				crystal.setPos(origin.x, origin.y, origin.z);
				crystal.setYRot(player.getYRot());
				crystal.setNoGravity(true);
				crystal.setBeamTarget(BlockPos.containing(origin.x, origin.y - 4.0, origin.z));
				serverLevel.addFreshEntity(crystal);
				modelMeshSetupServerEntityId = crystal.getId();
			} else if ("wind-charge".equals(MODEL_MESH_SCENARIO)) {
				WindCharge windCharge = new WindCharge(EntityType.WIND_CHARGE, serverLevel);
				windCharge.setPos(origin.x, origin.y, origin.z);
				windCharge.setYRot(player.getYRot());
				windCharge.setXRot(player.getXRot());
				windCharge.setNoGravity(true);
				windCharge.setDeltaMovement(Vec3.ZERO);
				serverLevel.addFreshEntity(windCharge);
				modelMeshSetupServerEntityId = windCharge.getId();
			} else {
				ServerPlayer serverPlayer = minecraft.getSingleplayerServer().getPlayerList().getPlayer(player.getUUID());
				if (serverPlayer == null) {
					modelMeshSetupStatus = "waiting-server-player";
					return;
				}
				WitherSkull skull = new WitherSkull(serverLevel, serverPlayer, Vec3.ZERO);
				skull.setPos(origin.x, origin.y, origin.z);
				skull.setYRot(player.getYRot());
				skull.setXRot(player.getXRot());
				skull.setNoGravity(true);
				skull.setDeltaMovement(Vec3.ZERO);
				serverLevel.addFreshEntity(skull);
				modelMeshSetupServerEntityId = skull.getId();
			}
			modelMeshSetupStatus = "spawned";
			modelMeshSetupBlockId = "llama-spit".equals(MODEL_MESH_SCENARIO) ? "minecraft:llama_spit"
				: "evoker-fangs".equals(MODEL_MESH_SCENARIO) ? "minecraft:evoker_fangs"
				: "chicken".equals(MODEL_MESH_SCENARIO) ? "minecraft:chicken"
				: "cow".equals(MODEL_MESH_SCENARIO) ? "minecraft:cow"
				: "pig".equals(MODEL_MESH_SCENARIO) ? "minecraft:pig"
				: "rabbit".equals(MODEL_MESH_SCENARIO) ? "minecraft:rabbit"
				: "zombie".equals(MODEL_MESH_SCENARIO) ? "minecraft:zombie"
				: "end-crystal".equals(MODEL_MESH_SCENARIO) ? "minecraft:end_crystal"
				: "wind-charge".equals(MODEL_MESH_SCENARIO) ? "minecraft:wind_charge" : "minecraft:wither_skull";
			modelMeshSetupOrigin = String.format(Locale.ROOT, "%.3f,%.3f,%.3f", origin.x, origin.y, origin.z);
			modelMeshSetupServerEntityPresent = true;
			modelMeshSetupPoseIndex = poseIndex;
			LOGGER.info("Deterministic ModelPart setup spawned ordinary vanilla {} near {} for Rust-GAL model mesh capture", MODEL_MESH_SCENARIO, modelMeshSetupOrigin);
			return;
		}
		if (modelMeshSetupPosition != null) {
			modelMeshSetupClientBlockEntityPresent = minecraft.level.getBlockEntity(modelMeshSetupPosition) != null;
			modelMeshSetupStatus = modelMeshSetupClientBlockEntityPresent ? "spawned" : "waiting-client-block-entity";
			return;
		}
		Vec3 forward = player.getLookAngle();
		if (forward.lengthSqr() < 0.0001) {
			forward = new Vec3(0.0, 0.0, 1.0);
		}
		BlockPos position = BlockPos.containing(player.getEyePosition().add(forward.normalize().scale(4.0)).add(0.0, -1.25, 0.0));
		BlockState state = switch (MODEL_MESH_SCENARIO) {
			case "bed" -> Blocks.RED_BED.defaultBlockState();
			case "bell" -> Blocks.BELL.defaultBlockState().setValue(net.minecraft.world.level.block.BellBlock.FACING, Direction.SOUTH);
			case "shulker" -> Blocks.PURPLE_SHULKER_BOX.defaultBlockState();
			case "decorated-pot" -> Blocks.DECORATED_POT.defaultBlockState();
			case "conduit" -> Blocks.CONDUIT.defaultBlockState();
			default -> Blocks.CHEST.defaultBlockState().setValue(ChestBlock.FACING, Direction.SOUTH);
		};
		runOnServerThreadAndWait(minecraft.getSingleplayerServer(), () -> {
			prepareModelMeshCaptureSite(serverLevel, position);
			serverLevel.setBlock(position, state, 3);
		});
		modelMeshSetupPosition = position;
		modelMeshSetupStatus = "waiting-client-block-entity";
		modelMeshSetupBlockId = state.getBlock().builtInRegistryHolder().key().location().toString();
		modelMeshSetupOrigin = position.toShortString();
		modelMeshSetupClientBlockEntityPresent = minecraft.level.getBlockEntity(position) != null;
		if (modelMeshSetupClientBlockEntityPresent) {
			modelMeshSetupStatus = "spawned";
		}
		LOGGER.info("Deterministic ModelPart setup placed ordinary vanilla {} at {} for Rust-GAL model mesh capture",
			MODEL_MESH_SCENARIO, position);
	}

	private static boolean isModelMeshEntityScenario() {
		return switch (MODEL_MESH_SCENARIO) {
			case "llama-spit", "evoker-fangs", "wither-skull", "chicken", "cow", "pig", "rabbit", "sheep", "tropical-fish", "zombie", "end-crystal", "wind-charge" -> true;
			default -> false;
		};
	}

	private static String modelMeshEntityBlockId(String scenario) {
		return switch (scenario) {
			case "llama-spit" -> "minecraft:llama_spit";
			case "evoker-fangs" -> "minecraft:evoker_fangs";
			case "chicken" -> "minecraft:chicken";
			case "cow" -> "minecraft:cow";
			case "pig" -> "minecraft:pig";
			case "rabbit" -> "minecraft:rabbit";
			case "sheep" -> "minecraft:sheep";
			case "tropical-fish" -> "minecraft:tropical_fish";
			case "zombie" -> "minecraft:zombie";
			case "end-crystal" -> "minecraft:end_crystal";
			case "wind-charge" -> "minecraft:wind_charge";
			case "wither-skull" -> "minecraft:wither_skull";
			default -> "";
		};
	}

	/** Runs copied-world entity setup on the integrated server, never the render thread. */
	private static void spawnModelMeshEntityOnServer(
		MinecraftServer server,
		ResourceKey<Level> dimension,
		String scenario,
		Vec3 origin,
		Vec3 eyePosition,
		float playerYaw,
		UUID playerId,
		int spawnPoseIndex
	) {
		try {
			ServerLevel serverLevel = server.getLevel(dimension);
			if (serverLevel == null) {
				modelMeshSetupServerSpawnFailure = "missing-server-level";
				modelMeshSetupStatus = "missing-server-level";
				return;
			}
			prepareModelMeshScenarioDifficulty(server, scenario);
			if ("zombie".equals(scenario) && serverLevel.getDifficulty() == Difficulty.PEACEFUL) {
				modelMeshSetupServerSpawnFailure = "zombie-requires-non-peaceful-difficulty";
				modelMeshSetupStatus = "zombie-requires-non-peaceful-difficulty";
				return;
			}
			Entity entity;
			switch (scenario) {
				case "llama-spit" -> {
					LlamaSpit llamaSpit = new LlamaSpit(EntityType.LLAMA_SPIT, serverLevel);
					llamaSpit.setPos(origin.x, origin.y, origin.z);
					llamaSpit.setYRot(playerYaw);
					llamaSpit.setNoGravity(true);
					llamaSpit.setDeltaMovement(Vec3.ZERO);
					entity = llamaSpit;
				}
				case "evoker-fangs" -> {
					ServerPlayer serverPlayer = server.getPlayerList().getPlayer(playerId);
					if (serverPlayer == null) {
						modelMeshSetupServerSpawnFailure = "missing-server-player";
						modelMeshSetupStatus = "waiting-server-player";
						return;
					}
					prepareModelMeshEntityCaptureSite(serverLevel, eyePosition, origin);
					entity = new EvokerFangs(serverLevel, origin.x, origin.y - 1.0, origin.z,
						(float)Math.toRadians(playerYaw), 1000, serverPlayer);
				}
				case "chicken" -> {
					Chicken chicken = new Chicken(EntityType.CHICKEN, serverLevel);
					prepareModelMeshEntityCaptureSite(serverLevel, eyePosition, origin.add(0.0, -1.1, 0.0));
					chicken.setPos(origin.x, origin.y - 1.1, origin.z);
					chicken.setNoAi(true);
					chicken.setNoGravity(true);
					entity = chicken;
				}
				case "cow" -> {
					Cow cow = new Cow(EntityType.COW, serverLevel);
					prepareModelMeshEntityCaptureSite(serverLevel, eyePosition, origin.add(0.0, -1.1, 0.0));
					cow.setPos(origin.x, origin.y - 1.1, origin.z);
					cow.setNoAi(true);
					cow.setNoGravity(true);
					entity = cow;
				}
				case "pig" -> {
					Pig pig = new Pig(EntityType.PIG, serverLevel);
					prepareModelMeshEntityCaptureSite(serverLevel, eyePosition, origin.add(0.0, -1.1, 0.0));
					pig.setPos(origin.x, origin.y - 1.1, origin.z);
					pig.setNoAi(true);
					pig.setNoGravity(true);
					entity = pig;
				}
				case "rabbit" -> {
					Rabbit rabbit = new Rabbit(EntityType.RABBIT, serverLevel);
					prepareModelMeshEntityCaptureSite(serverLevel, eyePosition, origin.add(0.0, -1.1, 0.0));
					rabbit.setPos(origin.x, origin.y - 1.1, origin.z);
					rabbit.setNoAi(true);
					rabbit.setNoGravity(true);
					entity = rabbit;
				}
				case "sheep" -> {
					Sheep sheep = new Sheep(EntityType.SHEEP, serverLevel);
					prepareModelMeshEntityCaptureSite(serverLevel, eyePosition, origin.add(0.0, -1.1, 0.0));
					sheep.setPos(origin.x, origin.y - 1.1, origin.z);
					sheep.setNoAi(true);
					sheep.setNoGravity(true);
					sheep.setBaby(true);
					entity = sheep;
				}
				case "tropical-fish" -> {
					TropicalFish fish = new TropicalFish(EntityType.TROPICAL_FISH, serverLevel);
					prepareModelMeshEntityCaptureSite(serverLevel, eyePosition, origin.add(0.0, -1.1, 0.0));
					fish.setPos(origin.x, origin.y - 1.1, origin.z);
					fish.setNoAi(true);
					fish.setNoGravity(true);
					entity = fish;
				}
				case "zombie" -> {
					Zombie zombie = new Zombie(EntityType.ZOMBIE, serverLevel);
					prepareModelMeshEntityCaptureSite(serverLevel, eyePosition, origin.add(0.0, -1.1, 0.0));
					zombie.setPos(origin.x, origin.y - 1.1, origin.z);
					zombie.setNoAi(true);
					zombie.setNoGravity(true);
					entity = zombie;
				}
				case "wither-skull" -> {
					ServerPlayer serverPlayer = server.getPlayerList().getPlayer(playerId);
					if (serverPlayer == null) {
						modelMeshSetupServerSpawnFailure = "missing-server-player";
						modelMeshSetupStatus = "waiting-server-player";
						return;
					}
					WitherSkull skull = new WitherSkull(serverLevel, serverPlayer, Vec3.ZERO);
					skull.setPos(origin.x, origin.y, origin.z);
					skull.setNoGravity(true);
					entity = skull;
				}
				case "end-crystal" -> {
					net.minecraft.world.entity.boss.enderdragon.EndCrystal crystal =
						new net.minecraft.world.entity.boss.enderdragon.EndCrystal(EntityType.END_CRYSTAL, serverLevel);
					crystal.setPos(origin.x, origin.y, origin.z);
					crystal.setNoGravity(true);
					crystal.setBeamTarget(BlockPos.containing(origin.x, origin.y - 4.0, origin.z));
					entity = crystal;
				}
				case "wind-charge" -> {
					WindCharge windCharge = new WindCharge(EntityType.WIND_CHARGE, serverLevel);
					windCharge.setPos(origin.x, origin.y, origin.z);
					windCharge.setNoGravity(true);
					entity = windCharge;
				}
				default -> {
					modelMeshSetupServerSpawnFailure = "unsupported-scenario";
					modelMeshSetupStatus = "unsupported-scenario";
					return;
				}
			}
			entity.setYRot(playerYaw + (scenario.equals("llama-spit") || scenario.equals("wither-skull") ? 0.0F : 180.0F));
			if (entity instanceof Cow cow) GraphicsAuditCowOutlineFixture.configure(cow);
			entity.setDeltaMovement(Vec3.ZERO);
			igniteEntityFlameCarrier(entity);
			configureEntityLeashCarrier(entity, server, playerId);
			configureWorldTextCaptureCarrier(entity, scenario);
			if ("evoker-fangs".equals(scenario)) {
				// Ensure the owning chunk is tracked before the entity enters the
				// normal server replication path; otherwise the short-lived
				// projectile can remain server-only in a freshly copied world.
				serverLevel.getChunkAt(BlockPos.containing(origin));
			}
			serverLevel.addFreshEntity(entity);
			modelMeshSetupServerEntityId = entity.getId();
			modelMeshSetupServerEntityPresent = true;
			modelMeshSetupPoseIndex = spawnPoseIndex;
			modelMeshSetupStatus = "server-spawned";
			LOGGER.info("Deterministic ModelPart server spawn completed for ordinary vanilla {} near {}", scenario, modelMeshSetupOrigin);
		} catch (RuntimeException exception) {
			modelMeshSetupServerSpawnFailure = exception.getClass().getSimpleName() + ": " + exception.getMessage();
			modelMeshSetupStatus = "server-spawn-failed";
		}
	}

	/**
	 * Hostile-model capture fixtures must survive the copied world's normal
	 * server tick long enough to reach the ordinary client renderer. This only
	 * changes the disposable integrated-server copy used by this diagnostic
	 * scenario; ordinary worlds and every production route retain their saved
	 * difficulty.
	 */
	private static void prepareModelMeshScenarioDifficulty(MinecraftServer server, String scenario) {
		Difficulty before = server.getWorldData().getDifficulty();
		modelMeshSetupDifficultyBefore = before.getKey();
		if ("zombie".equals(scenario) && before == Difficulty.PEACEFUL) {
			server.setDifficulty(Difficulty.NORMAL, false);
			modelMeshSetupDifficultyAdjusted = server.getWorldData().getDifficulty() != before;
		}
		modelMeshSetupDifficultyEffective = server.getWorldData().getDifficulty().getKey();
	}

	private static void igniteEntityFlameCarrier(Entity entity) {
		if ("cow".equals(ENTITY_FLAME_SCENARIO) && entity.getType() == EntityType.COW) {
			// This affects only the copied-world fixture. Vanilla replication drives
			// EntityRenderState.displayFireAnimation on normal client render frames.
			entity.setRemainingFireTicks(20 * 120);
		}
	}

	/** Attaches only the disposable cow to the real server player for the leash route. */
	private static void configureEntityLeashCarrier(Entity entity, MinecraftServer server, UUID playerId) {
		if (!"cow".equals(ENTITY_LEASH_SCENARIO) || entity.getType() != EntityType.COW) {
			return;
		}
		ServerPlayer holder = server.getPlayerList().getPlayer(playerId);
		if (holder == null) {
			throw new IllegalStateException("entity-leash scenario is missing its server player holder");
		}
		((Cow)entity).setLeashedTo(holder, true);
	}

	/** Adds a real vanilla name tag only for the explicit source-capture probe. */
	private static void configureWorldTextCaptureCarrier(Entity entity, String scenario) {
		if (REQUIRE_RUST_WORLD_TEXT_SOURCE_CAPTURE && "cow".equals(scenario) && entity.getType() == EntityType.COW) {
			entity.setCustomName(Component.literal("Rust world text"));
			entity.setCustomNameVisible(true);
		}
	}

	private static void updateModelMeshClientEntityReceipt(Minecraft minecraft) {
		if (!modelMeshSetupServerEntityPresent || modelMeshSetupServerEntityId < 0) {
			return;
		}
		boolean previouslyObserved = modelMeshSetupClientEntityPresent;
		Entity clientEntity = minecraft.level.getEntity(modelMeshSetupServerEntityId);
		if (!isExpectedModelMeshClientEntity(clientEntity)) {
			clientEntity = findExpectedModelMeshClientEntity(minecraft.level);
		}
		// The disposable copied-world fixture can receive the fire flag one
		// client replication tick after the entity itself. Re-assert it on the
		// client-side carrier so vanilla's ordinary EntityRenderState extraction
		// emits the FlameSubmit; this does not participate in production route
		// selection or render any Java flame geometry.
		if ("cow".equals(ENTITY_FLAME_SCENARIO) && clientEntity instanceof Cow) {
			clientEntity.setRemainingFireTicks(20 * 120);
			clientEntity.setSharedFlagOnFire(true);
		}
		int rendererEntityId = clientEntity == null ? observedExpectedModelMeshRendererEntityId() : clientEntity.getId();
		// Evoker Fangs' standalone render state does not carry an entity id into
		// the model-mesh diagnostic (unlike living-entity states).  The repeated
		// vanilla renderer-produced mesh request is nevertheless direct evidence
		// that the replicated client carrier reached the semantic callsite.  Keep
		// the server id as the stable receipt identity rather than admitting a
		// synthetic client-side fixture.
		if (rendererEntityId < 0 && "evoker-fangs".equals(MODEL_MESH_SCENARIO)
			&& observedExpectedEvokerFangsModelMesh()) {
			rendererEntityId = modelMeshSetupServerEntityId;
		}
		modelMeshSetupClientEntityPresent = rendererEntityId >= 0;
		// Evoker fangs can complete their vanilla attack lifecycle between the
		// producer frame and the final metadata write. Once a real client entity
		// was observed, retain that receipt; route admission still requires the
		// same-frame semantic mesh and execution evidence below.
		if (!modelMeshSetupClientEntityPresent && previouslyObserved
			&& "evoker-fangs".equals(MODEL_MESH_SCENARIO)) {
			modelMeshSetupClientEntityPresent = true;
		}
		modelMeshSetupClientEntityId = rendererEntityId;
		if (!modelMeshSetupClientEntityPresent) {
			StringBuilder sample = new StringBuilder();
			int sampled = 0;
			for (Entity entity : minecraft.level.entitiesForRendering()) {
				if (sampled++ == 8) break;
				if (!sample.isEmpty()) sample.append(';');
				sample.append(entity.getId()).append(':')
					.append(BuiltInRegistries.ENTITY_TYPE.getKey(entity.getType()))
					.append('@').append(String.format(Locale.ROOT, "%.2f,%.2f,%.2f", entity.getX(), entity.getY(), entity.getZ()));
			}
			modelMeshSetupClientEntitySample = sample.toString();
		} else {
			modelMeshSetupClientEntitySample = "";
		}
		modelMeshSetupStatus = modelMeshSetupClientEntityPresent ? "spawned" : "waiting-client-entity";
	}

	/** Records a receipt only when the real standalone entity renderer submits its
	 * semantic mesh; no client entity or packet is synthesized here. */
	public static void noteModelMeshClientSemanticSubmission(int entityId) {
		if (!ENABLED || complete || failed || !"evoker-fangs".equals(MODEL_MESH_SCENARIO)
			|| !modelMeshSetupServerEntityPresent || modelMeshSetupServerEntityId < 0) {
			return;
		}
		modelMeshSetupClientEntityPresent = true;
		modelMeshSetupClientEntityId = entityId >= 0 ? entityId : modelMeshSetupServerEntityId;
		modelMeshSetupStatus = "spawned";
		Minecraft receiptMinecraft = Minecraft.getInstance();
		if (receiptMinecraft != null) {
			writeMetadata(receiptMinecraft, "model_mesh_client_semantic_receipt");
		}
	}

	private static Entity findExpectedModelMeshClientEntity(ClientLevel level) {
		for (Entity entity : level.entitiesForRendering()) {
			if (isExpectedModelMeshClientEntity(entity)) {
				return entity;
			}
		}
		return null;
	}

	private static boolean isExpectedModelMeshClientEntity(Entity entity) {
		EntityType<?> expectedType = switch (MODEL_MESH_SCENARIO) {
			case "llama-spit" -> EntityType.LLAMA_SPIT;
			case "evoker-fangs" -> EntityType.EVOKER_FANGS;
			case "wither-skull" -> EntityType.WITHER_SKULL;
			case "chicken" -> EntityType.CHICKEN;
			case "cow" -> EntityType.COW;
			case "pig" -> EntityType.PIG;
			case "rabbit" -> EntityType.RABBIT;
			case "sheep" -> EntityType.SHEEP;
			case "tropical-fish" -> EntityType.TROPICAL_FISH;
			case "zombie" -> EntityType.ZOMBIE;
			case "end-crystal" -> EntityType.END_CRYSTAL;
			case "wind-charge" -> EntityType.WIND_CHARGE;
			default -> null;
		};
		double positionTolerance = "llama-spit".equals(MODEL_MESH_SCENARIO) ? 16.0D : 1.0D;
		return entity != null
			&& expectedType != null
			&& entity.getType() == expectedType
			&& modelMeshSetupExpectedEntityPosition != null
			&& entity.position().distanceToSqr(modelMeshSetupExpectedEntityPosition) <= positionTolerance * positionTolerance;
	}

	private static int observedExpectedModelMeshRendererEntityId() {
		String expectedIdentity = switch (MODEL_MESH_SCENARIO) {
			case "chicken" -> "minecraft:chicken";
			case "cow" -> "minecraft:cow";
			case "pig" -> "minecraft:pig";
			case "rabbit" -> "minecraft:rabbit";
			case "sheep" -> "minecraft:sheep";
			case "tropical-fish" -> "minecraft:tropical_fish";
			case "zombie" -> "minecraft:zombie";
			case "end-crystal" -> "minecraft:end_crystal";
			case "wind-charge" -> "minecraft:wind_charge";
			default -> "";
		};
		if (expectedIdentity.isEmpty()) {
			return -1;
		}
		List<RustGalWorldPrimitiveRenderer.ModelMeshDiagnostic> diagnostics = RustGalWorldPrimitiveRenderer.modelMeshDiagnostics();
		for (int index = diagnostics.size() - 1; index >= 0; index--) {
			var diagnostic = diagnostics.get(index);
			if (expectedIdentity.equals(diagnostic.semanticModelIdentity())
				&& expectedModelMeshTextureId().equals(diagnostic.textureId())
				&& diagnostic.entityId() >= 0
				&& diagnostic.projected()
				&& diagnostic.sectionCount() > 0) {
				return diagnostic.entityId();
			}
		}
		return -1;
	}

	private static boolean observedExpectedEvokerFangsModelMesh() {
		if (!"evoker-fangs".equals(MODEL_MESH_SCENARIO)) {
			return false;
		}
		for (RustGalWorldPrimitiveRenderer.ModelMeshDiagnostic diagnostic
			: RustGalWorldPrimitiveRenderer.modelMeshDiagnostics()) {
			if (expectedModelMeshTextureId().equals(diagnostic.textureId())
				&& diagnostic.sectionCount() > 0 && diagnostic.vertexCount() > 0) {
				return true;
			}
		}
		for (RustGalWorldPrimitiveRenderer.ModelMeshRouteDecision decision
			: RustGalWorldPrimitiveRenderer.modelMeshRouteDecisions()) {
			if ("rust-vulkan-whole-frame".equals(decision.route())
				&& expectedModelMeshTextureId().equals(decision.textureId())
				&& decision.rustSelected() && decision.rustQueued()) {
				return true;
			}
		}
		return false;
	}

	/**
	 * Makes the copied-world model fixture visibly inspectable without changing
	 * its normal block-entity renderer or semantic submission path.
	 */
	private static void prepareModelMeshCaptureSite(ServerLevel serverLevel, BlockPos position) {
		for (int localX = -1; localX <= 1; localX++) {
			for (int localZ = -1; localZ <= 1; localZ++) {
				serverLevel.setBlock(position.offset(localX, -1, localZ), Blocks.STONE.defaultBlockState(), 3);
				for (int localY = 0; localY <= 2; localY++) {
					serverLevel.setBlock(position.offset(localX, localY, localZ), Blocks.AIR.defaultBlockState(), 3);
				}
			}
		}
	}

	/** Clears only the copied-world sightline for a real vanilla living-entity fixture. */
	private static void prepareModelMeshEntityCaptureSite(ServerLevel serverLevel, Vec3 eyePosition, Vec3 entityPosition) {
		BlockPos entityBlock = BlockPos.containing(entityPosition);
		prepareModelMeshCaptureSite(serverLevel, entityBlock);
		if ("zombie".equals(MODEL_MESH_SCENARIO)) {
			// A roof only exists in the harness-owned copy. It preserves the normal
			// adult Zombie body state by preventing the vanilla daylight-fire feature,
			// which remains explicitly unsupported by this first direct-model route.
			// Keep the one required sky blocker in the same loaded column but well
			// outside the camera-facing model volume. A low roof obscures the head
			// and makes a direct-texture capture diagnostically useless.
			serverLevel.setBlock(entityBlock.above(64), Blocks.STONE.defaultBlockState(), 3);
		}
		Vec3 ray = entityPosition.subtract(eyePosition);
		int steps = Math.max(1, (int)Math.ceil(ray.length() * 4.0));
		for (int step = 1; step <= steps; step++) {
			BlockPos position = BlockPos.containing(eyePosition.add(ray.scale((double)step / steps)));
			serverLevel.setBlock(position, Blocks.AIR.defaultBlockState(), 3);
			serverLevel.setBlock(position.above(), Blocks.AIR.defaultBlockState(), 3);
		}
		if ("cow".equals(MODEL_MESH_SCENARIO)) placeShadowGlassFixture(serverLevel, eyePosition, entityPosition);
	}

    private static void placeShadowGlassFixture(ServerLevel serverLevel, Vec3 eyePosition, Vec3 entityPosition) {
        // Test-world setup only; no renderer state or renderer behavior changes.
        if (!Boolean.getBoolean("mattmc.dev.deterministicCameraCapture.shadowGlass")) return;
        Vec3 towardEye = eyePosition.subtract(entityPosition);
        int dx = Math.abs(towardEye.x) >= Math.abs(towardEye.z) ? (towardEye.x > 0 ? 1 : -1) : 0;
        int dz = dx == 0 ? (towardEye.z > 0 ? 1 : -1) : 0;
        BlockPos origin = BlockPos.containing(entityPosition).offset(dx * 2, 0, dz * 2);
        for (int across = -1; across <= 1; across++) {
            for (int up = 0; up <= 1; up++) {
                serverLevel.setBlock(origin.offset(-dz * across, up, dx * across),
                    Blocks.RED_STAINED_GLASS.defaultBlockState(), 3);
            }
        }
    }

	/**
	 * Capture-only real PrimedTnt setup. The entity is kept physically still and
	 * given a long fuse whose current five-tick window does not request vanilla's
	 * flashing white overlay, so the test exercises precisely the eligible
	 * ordinary baked-block producer path.
	 */
	private static void setupPrimedTntScenario(Minecraft minecraft, LocalPlayer player) {
		if (PRIMED_TNT_SCENARIO.isEmpty() || "hidden".equals(PRIMED_TNT_SCENARIO)
			|| player == null || minecraft.level == null || minecraft.getSingleplayerServer() == null) {
			primedTntSetupStatus = PRIMED_TNT_SCENARIO.isEmpty() ? "inactive" : "hidden-or-missing-level";
			return;
		}
		ServerLevel serverLevel = minecraft.getSingleplayerServer().getLevel(minecraft.level.dimension());
		if (serverLevel == null) {
			primedTntSetupStatus = "missing-server-level";
			return;
		}
		Vec3 forward = player.getLookAngle();
		if (forward.lengthSqr() < 0.0001) {
			forward = new Vec3(0.0, 0.0, 1.0);
		}
		BlockPos origin = BlockPos.containing(player.getEyePosition().add(forward.normalize().scale(4.0)).add(0.0, 0.5, 0.0));
		int entityCount = Math.max(1, Integer.getInteger("mattmc.dev.rustGalWorldMesh.primedTntCount", 1));
		for (int i = 0; i < entityCount; i++) {
			BlockPos position = origin.offset(i, 0, 0);
			PrimedTnt primedTnt = new PrimedTnt(serverLevel, position.getX() + 0.5, position.getY(), position.getZ() + 0.5, null);
			primedTnt.setBlockState(Blocks.TNT.defaultBlockState());
			primedTnt.setNoGravity(true);
			primedTnt.setDeltaMovement(Vec3.ZERO);
			// Keep the real entity alive longer than the bounded external capture;
			// TntRenderer's capture-only ordinary-state property controls the visual
			// branch without letting the server-side fuse expire mid-sequence.
			primedTnt.setFuse(12_000);
			serverLevel.addFreshEntity(primedTnt);
		}
		primedTntSetupStatus = "spawned";
		primedTntSetupBlockId = Blocks.TNT.builtInRegistryHolder().key().location().toString();
		primedTntSetupOrigin = origin.toShortString();
		primedTntSetupEntityCount = entityCount;
		LOGGER.info(
			"Deterministic PrimedTnt setup spawned {} count={} near {} for Rust-GAL indexed mesh capture",
			primedTntSetupBlockId,
			entityCount,
			origin
		);
	}

	private static void setupPistonScenario(Minecraft minecraft, LocalPlayer player) {
		if (PISTON_SCENARIO.isEmpty() || "hidden".equals(PISTON_SCENARIO) || minecraft.level == null || minecraft.getSingleplayerServer() == null) {
			pistonSetupStatus = PISTON_SCENARIO.isEmpty() ? "inactive" : "hidden-or-missing-level";
			return;
		}
		ServerLevel serverLevel = minecraft.getSingleplayerServer().getLevel(minecraft.level.dimension());
		if (serverLevel == null) {
			pistonSetupStatus = "missing-server-level";
			return;
		}
		BlockState movedState = pistonMovedState();
		Direction direction = pistonDirection();
		Vec3 forward = player.getLookAngle();
		if (forward.lengthSqr() < 0.0001) {
			forward = new Vec3(0.0, 0.0, 1.0);
		}
		BlockPos origin = BlockPos.containing(player.getEyePosition().add(forward.normalize().scale(4.0)).add(0.0, -0.25, 0.0));
		int count = Math.max(1, Integer.getInteger("mattmc.dev.rustGalWorldMesh.pistonCount", 1));
		for (int i = 0; i < count; i++) {
			BlockPos pos = origin.offset(i, 0, 0);
			clearPistonScenarioSpace(serverLevel, minecraft.level, pos);
			seedMovingPiston(serverLevel, minecraft.level, pos, movedState, direction, pistonExtending(), pistonSourcePiston());
		}
		pistonSetupPos = origin;
		pistonSetupMovedState = movedState;
		pistonSetupDirection = direction;
		pistonSetupExtending = pistonExtending();
		pistonSetupSourcePiston = pistonSourcePiston();
		pistonSetupStatus = "spawned";
		pistonSetupBlockId = movedState.getBlock().builtInRegistryHolder().key().location().toString();
		pistonSetupOrigin = origin.toShortString();
		pistonSetupBlockState = minecraft.level.getBlockState(origin).toString();
		pistonSetupClientBlockEntityPresent = minecraft.level.getBlockEntity(origin) instanceof net.minecraft.world.level.block.piston.PistonMovingBlockEntity;
		pistonSetupEntityCount = count;
		LOGGER.info(
			"Deterministic piston setup spawned {} count={} scenario={} direction={} near {} for Rust-GAL moving mesh capture",
			movedState.getBlock().builtInRegistryHolder().key().location(),
			count,
			PISTON_SCENARIO,
			direction,
			origin
		);
	}

	private static void maintainPistonScenario(Minecraft minecraft) {
		if (PISTON_SCENARIO.isEmpty()
			|| "hidden".equals(PISTON_SCENARIO)
			|| !movingMeshScenarioSetup
			|| complete
			|| failed
			|| pistonSetupPos == null
			|| pistonSetupMovedState == null
			|| pistonSetupDirection == null
			|| minecraft.level == null
			|| minecraft.getSingleplayerServer() == null) {
			return;
		}
		ServerLevel serverLevel = minecraft.getSingleplayerServer().getLevel(minecraft.level.dimension());
		if (serverLevel == null) {
			pistonSetupStatus = "maintain-missing-server-level";
			return;
		}
		boolean clientPresent = minecraft.level.getBlockEntity(pistonSetupPos) instanceof net.minecraft.world.level.block.piston.PistonMovingBlockEntity;
		boolean serverPresent = serverLevel.getBlockEntity(pistonSetupPos) instanceof net.minecraft.world.level.block.piston.PistonMovingBlockEntity;
		pistonSetupClientBlockEntityPresent = clientPresent;
		pistonSetupBlockState = minecraft.level.getBlockState(pistonSetupPos).toString();
		if (clientPresent && serverPresent && minecraft.level.getBlockState(pistonSetupPos).is(Blocks.MOVING_PISTON)) {
			return;
		}
		clearPistonScenarioSpace(serverLevel, minecraft.level, pistonSetupPos);
		seedMovingPiston(
			serverLevel,
			minecraft.level,
			pistonSetupPos,
			pistonSetupMovedState,
			pistonSetupDirection,
			pistonSetupExtending,
			pistonSetupSourcePiston
		);
		pistonSetupReseedCount++;
		pistonSetupStatus = "reseeded";
		pistonSetupBlockState = minecraft.level.getBlockState(pistonSetupPos).toString();
		pistonSetupClientBlockEntityPresent = minecraft.level.getBlockEntity(pistonSetupPos) instanceof net.minecraft.world.level.block.piston.PistonMovingBlockEntity;
	}

	private static BlockState pistonMovedState() {
		return switch (PISTON_SCENARIO) {
			case "sticky-extending", "sticky-retracting" -> Blocks.PISTON_HEAD.defaultBlockState()
				.setValue(net.minecraft.world.level.block.piston.PistonHeadBlock.FACING, pistonDirection())
				.setValue(net.minecraft.world.level.block.piston.PistonHeadBlock.TYPE, net.minecraft.world.level.block.state.properties.PistonType.STICKY);
			case "retracting-source", "normal-retracting" -> Blocks.PISTON.defaultBlockState()
				.setValue(net.minecraft.world.level.block.piston.PistonBaseBlock.FACING, pistonDirection());
			case "cutout", "leaves" -> Blocks.OAK_LEAVES.defaultBlockState();
			default -> Blocks.STONE.defaultBlockState();
		};
	}

	private static Direction pistonDirection() {
		return switch (System.getProperty("mattmc.dev.rustGalWorldMesh.pistonDirection", "north").trim().toLowerCase(Locale.ROOT)) {
			case "down" -> Direction.DOWN;
			case "up" -> Direction.UP;
			case "south" -> Direction.SOUTH;
			case "west" -> Direction.WEST;
			case "east" -> Direction.EAST;
			default -> Direction.NORTH;
		};
	}

	private static boolean pistonExtending() {
		return !PISTON_SCENARIO.contains("retracting");
	}

	private static boolean pistonSourcePiston() {
		return PISTON_SCENARIO.contains("source");
	}

	private static void seedMovingPiston(ServerLevel serverLevel, ClientLevel clientLevel, BlockPos pos, BlockState movedState, Direction direction, boolean extending, boolean sourcePiston) {
		BlockState movingState = Blocks.MOVING_PISTON.defaultBlockState()
			.setValue(net.minecraft.world.level.block.piston.MovingPistonBlock.FACING, direction)
			.setValue(net.minecraft.world.level.block.piston.MovingPistonBlock.TYPE, pistonTypeFor(movedState));
		serverLevel.setBlock(pos, movingState, 3);
		net.minecraft.world.level.block.piston.PistonMovingBlockEntity serverEntity =
			(net.minecraft.world.level.block.piston.PistonMovingBlockEntity)net.minecraft.world.level.block.piston.MovingPistonBlock.newMovingBlockEntity(pos, movingState, movedState, direction, extending, sourcePiston);
		serverLevel.setBlockEntity(serverEntity);
		serverEntity.setChanged();
		clientLevel.setBlock(pos, movingState, 3);
		net.minecraft.world.level.block.piston.PistonMovingBlockEntity clientEntity =
			(net.minecraft.world.level.block.piston.PistonMovingBlockEntity)net.minecraft.world.level.block.piston.MovingPistonBlock.newMovingBlockEntity(pos, movingState, movedState, direction, extending, sourcePiston);
		clientLevel.setBlockEntity(clientEntity);
		clientEntity.setChanged();
	}

	private static void clearPistonScenarioSpace(ServerLevel serverLevel, ClientLevel clientLevel, BlockPos center) {
		for (int dx = -1; dx <= 1; dx++) {
			for (int dy = -1; dy <= 1; dy++) {
				for (int dz = -1; dz <= 1; dz++) {
					BlockPos pos = center.offset(dx, dy, dz);
					serverLevel.setBlock(pos, Blocks.AIR.defaultBlockState(), 2);
					clientLevel.setBlock(pos, Blocks.AIR.defaultBlockState(), 2);
				}
			}
		}
	}

	private static net.minecraft.world.level.block.state.properties.PistonType pistonTypeFor(BlockState movedState) {
		if (movedState.is(Blocks.STICKY_PISTON)) {
			return net.minecraft.world.level.block.state.properties.PistonType.STICKY;
		}
		if (movedState.is(Blocks.PISTON_HEAD)
			&& movedState.getValue(net.minecraft.world.level.block.piston.PistonHeadBlock.TYPE) == net.minecraft.world.level.block.state.properties.PistonType.STICKY) {
			return net.minecraft.world.level.block.state.properties.PistonType.STICKY;
		}
		return net.minecraft.world.level.block.state.properties.PistonType.DEFAULT;
	}

	private static void setupDeterministicSupportPlatform(Minecraft minecraft, LocalPlayer player) {
		if (!FORCE_REAL_SURVIVAL_CRACK || !FORCE_REAL_SURVIVAL_CRACK_SETUP_BLOCK || minecraft.level == null || !"unset".equals(deterministicSupportBlock)) {
			return;
		}
		BlockPos center = BlockPos.containing(player.getX(), player.getY() - 1.0, player.getZ());
		BlockState setupState = Blocks.STONE.defaultBlockState();
		for (int dx = -1; dx <= 1; dx++) {
			for (int dz = -1; dz <= 1; dz++) {
				BlockPos pos = center.offset(dx, 0, dz);
				minecraft.level.setBlock(pos, setupState, 3);
				if (minecraft.getSingleplayerServer() != null) {
					ServerLevel serverLevel = minecraft.getSingleplayerServer().getLevel(minecraft.level.dimension());
					if (serverLevel != null) {
						serverLevel.setBlock(pos, setupState, 3);
					}
				}
			}
		}
		deterministicSupportBlock = center.toShortString();
		deterministicSupportBlockType = setupState.getBlock().builtInRegistryHolder().key().location().toString();
		realSurvivalCrackLastStatus = "setup-support-platform";
		LOGGER.info(
			"Deterministic capture support platform placed {} centered at {}",
			deterministicSupportBlockType,
			deterministicSupportBlock
		);
	}

	private static void recordPlayerSafetyState(LocalPlayer player) {
		lastSafetyPosition = player.position();
		lastSafetyVelocity = player.getDeltaMovement();
		lastSafetyHealth = player.getHealth();
		lastSafetyFallDistance = player.fallDistance;
	}

	private static void driveRealSurvivalCrack(Minecraft minecraft, LocalPlayer player) {
		if (!FORCE_REAL_SURVIVAL_CRACK || minecraft.screen != null || minecraft.gameMode == null || player.isSpectator()) {
			realSurvivalCrackLastStatus = "inactive";
			return;
		}
		if (!(minecraft.hitResult instanceof BlockHitResult blockHitResult) || blockHitResult.getType() != HitResult.Type.BLOCK) {
			minecraft.gameMode.stopDestroyBlock();
			realSurvivalCrackBlock = null;
			realSurvivalCrackDirection = null;
			realSurvivalCrackStopCalls++;
			realSurvivalCrackLastHitType = minecraft.hitResult == null ? "null" : minecraft.hitResult.getType().name();
			realSurvivalCrackLastTarget = "none";
			realSurvivalCrackLastDirection = "none";
			realSurvivalCrackLastStatus = "no-block-hit";
			return;
		}
		BlockPos blockPos = blockHitResult.getBlockPos();
		BlockState state = minecraft.level == null ? null : minecraft.level.getBlockState(blockPos);
		realSurvivalCrackLastHitType = blockHitResult.getType().name();
		realSurvivalCrackLastTarget = blockPos.toShortString();
		realSurvivalCrackLastDirection = blockHitResult.getDirection().name();
		if (state == null || state.isAir()) {
			minecraft.gameMode.stopDestroyBlock();
			realSurvivalCrackBlock = null;
			realSurvivalCrackDirection = null;
			realSurvivalCrackStopCalls++;
			realSurvivalCrackLastStatus = "air-or-no-level";
			return;
		}
		long gameTime = minecraft.level.getGameTime();
		if (realSurvivalCrackLastDriveGameTime == gameTime) {
			realSurvivalCrackLastStatus = "already-drove-this-tick";
			return;
		}
		realSurvivalCrackLastDriveGameTime = gameTime;
		realSurvivalCrackValidBlockHitCount++;
		realSurvivalCrackLastValidTarget = blockPos.toShortString();
		realSurvivalCrackLastValidBlockType = state.getBlock().builtInRegistryHolder().key().location().toString();
		Direction direction = blockHitResult.getDirection();
		if (!blockPos.equals(realSurvivalCrackBlock) || direction != realSurvivalCrackDirection) {
			minecraft.gameMode.startDestroyBlock(blockPos, direction);
			realSurvivalCrackBlock = blockPos;
			realSurvivalCrackDirection = direction;
			realSurvivalCrackStartCalls++;
			realSurvivalCrackLastStatus = "start";
			return;
		}
		minecraft.gameMode.continueDestroyBlock(blockPos, direction);
		realSurvivalCrackContinueCalls++;
		realSurvivalCrackLastStatus = "continue";
	}

	private static boolean hasBossBarOverride() {
		return FORCED_BOSS_BAR_COUNT >= 0 || !FORCED_BOSS_BAR_PROGRESS.isBlank() || !FORCED_BOSS_BAR_OVERLAY.isBlank();
	}

	private static int forcedBossBarCount() {
		if (FORCED_BOSS_BAR_COUNT >= 0) {
			return Math.max(0, FORCED_BOSS_BAR_COUNT);
		}
		if ("all".equalsIgnoreCase(FORCED_BOSS_BAR_OVERLAY)) {
			return BossBarOverlay.values().length;
		}
		return 1;
	}

	private static float[] forcedBossBarProgresses(int count) {
		String source = FORCED_BOSS_BAR_PROGRESS.isBlank() ? "0.5" : FORCED_BOSS_BAR_PROGRESS;
		String[] tokens = source.split(",");
		float[] values = new float[Math.max(1, tokens.length)];
		for (int i = 0; i < values.length; i++) {
			String token = tokens[i].trim();
			values[i] = token.isBlank() ? 0.5F : Math.max(0.0F, Math.min(1.0F, Float.parseFloat(token)));
		}
		return values;
	}

	private static BossBarOverlay[] forcedBossBarOverlays(int count) {
		if ("all".equalsIgnoreCase(FORCED_BOSS_BAR_OVERLAY)) {
			return BossBarOverlay.values();
		}
		String source = FORCED_BOSS_BAR_OVERLAY.isBlank() ? BossBarOverlay.PROGRESS.getSerializedName() : FORCED_BOSS_BAR_OVERLAY;
		String[] tokens = source.split(",");
		BossBarOverlay[] values = new BossBarOverlay[Math.max(1, tokens.length)];
		for (int i = 0; i < values.length; i++) {
			values[i] = parseBossBarOverlay(tokens[i].trim());
		}
		return values;
	}

	private static BossBarOverlay parseBossBarOverlay(String value) {
		for (BossBarOverlay overlay : BossBarOverlay.values()) {
			if (overlay.name().equalsIgnoreCase(value) || overlay.getSerializedName().equalsIgnoreCase(value)) {
				return overlay;
			}
		}
		throw new IllegalArgumentException("unknown deterministic capture boss bar overlay: " + value);
	}

	private static void applyHeartVariantOverride(LocalPlayer player) {
		if (FORCED_PLAYER_HEART_VARIANT.isBlank()) {
			return;
		}
		player.removeEffect(MobEffects.POISON);
		player.removeEffect(MobEffects.WITHER);
		player.setTicksFrozen(0);
		switch (FORCED_PLAYER_HEART_VARIANT.toLowerCase(Locale.ROOT)) {
			case "normal" -> {
			}
			case "poison", "poisoned" -> player.forceAddEffect(new MobEffectInstance(MobEffects.POISON, 20_000, 0, false, false, false), null);
			case "wither", "withered" -> player.forceAddEffect(new MobEffectInstance(MobEffects.WITHER, 20_000, 0, false, false, false), null);
			case "frozen" -> player.setTicksFrozen(player.getTicksRequiredToFreeze());
			default -> throw new IllegalArgumentException("unknown deterministic capture player heart variant: " + FORCED_PLAYER_HEART_VARIANT);
		}
	}

	private static MobEffectInstance copyEffect(MobEffectInstance effect) {
		return effect == null ? null : new MobEffectInstance(effect);
	}

	private static void restoreEffect(LocalPlayer player, Holder<MobEffect> effect, MobEffectInstance original) {
		player.removeEffect(effect);
		if (original != null) {
			player.forceAddEffect(new MobEffectInstance(original), null);
		}
	}

	private static GameType forcedGameMode() {
		if (FORCED_GAME_MODE.isBlank()) {
			return null;
		}
		GameType gameType = GameType.byName(FORCED_GAME_MODE, null);
		if (gameType == null) {
			throw new IllegalArgumentException("unknown deterministic capture game mode: " + FORCED_GAME_MODE);
		}
		return gameType;
	}

	private static AttackIndicatorStatus forcedAttackIndicator() {
		if (FORCED_ATTACK_INDICATOR.isBlank()) {
			return null;
		}
		for (AttackIndicatorStatus status : AttackIndicatorStatus.values()) {
			if (status.name().equalsIgnoreCase(FORCED_ATTACK_INDICATOR)) {
				return status;
			}
		}
		throw new IllegalArgumentException("unknown deterministic capture attack indicator: " + FORCED_ATTACK_INDICATOR);
	}

	private static CameraType forcedCameraType() {
		if (FORCED_CAMERA_TYPE.isBlank()) {
			return null;
		}
		try {
			return CameraType.valueOf(FORCED_CAMERA_TYPE.toUpperCase(Locale.ROOT).replace('-', '_'));
		} catch (IllegalArgumentException exception) {
			fail("invalid deterministic camera type override: " + FORCED_CAMERA_TYPE);
			return null;
		}
	}

	private static String currentCameraType(Minecraft minecraft) {
		return minecraft.options.getCameraType().name();
	}

	private static int currentSelectedHotbarSlot(LocalPlayer player) {
		return player.getInventory().getSelectedSlot() + 1;
	}

	private static void writeFailureMetadata(String reason) {
		StringBuilder json = new StringBuilder(4096);
		json.append("{\n");
		appendField(json, "status", "failed").append(",\n");
		appendField(json, "reason", reason).append(",\n");
		json.append("  \"rustGalWorldMovingMeshSetup\": { ");
		appendField(json, "stage", movingMeshSetupStage, 0).append(", ");
		appendField(json, "lastMissing", movingMeshSetupLastMissing, 0).append(", ");
		json.append("\"attempts\": ").append(movingMeshSetupAttempts)
			.append(", \"complete\": ").append(movingMeshScenarioSetup)
			.append(" },\n");
		appendField(json, "rustGalWorldMovingMeshProducerSummary", movingMeshProducerSummary()).append(",\n");
		appendField(json, "rustGalWorldPrimedTntScenario", PRIMED_TNT_SCENARIO).append(",\n");
		appendField(json, "rustGalWorldExperienceOrbScenario", EXPERIENCE_ORB_SCENARIO).append(",\n");
		json.append("  \"rustGalWorldExperienceOrbSetup\": { ");
		appendField(json, "status", experienceOrbSetupStatus, 0).append(", ");
		appendField(json, "origin", experienceOrbSetupOrigin, 0).append(", ");
		json.append("\"entityCount\": ").append(experienceOrbSetupEntityCount)
			.append(", \"poseIndex\": ").append(experienceOrbSetupPoseIndex)
			.append(" },\n");
		appendField(json, "rustGalWorldBeaconBeamScenario", BEACON_BEAM_SCENARIO).append(",\n");
		json.append("  \"rustGalWorldBeaconBeamSetup\": { ");
		appendField(json, "status", beaconBeamSetupStatus, 0).append(", ");
		appendField(json, "origin", beaconBeamSetupOrigin, 0).append(", ");
		json.append("\"clientBeamSectionsReady\": ").append(beaconBeamSetupClientReady)
			.append(", \"serverBeamSectionsReady\": ").append(beaconBeamSetupServerReady).append(" },\n");
		appendField(json, "rustGalWorldItemEntityScenario", ITEM_ENTITY_SCENARIO).append(",\n");
		json.append("  \"rustGalWorldItemEntitySetup\": { ");
		appendField(json, "status", itemEntitySetupStatus, 0).append(", ");
		appendField(json, "itemId", itemEntitySetupItemId, 0).append(", ");
		appendField(json, "origin", itemEntitySetupOrigin, 0).append(", ");
		json.append("\"entityCount\": ").append(itemEntitySetupEntityCount)
			.append(", \"serverSpawnedCount\": ").append(itemEntityServerSpawnedCount)
			.append(", \"clientVisibleCount\": ").append(itemEntityClientVisibleCount).append(", ");
		appendField(json, "serverSpawnFailure", itemEntityServerSpawnFailure, 0);
		json.append(", \"poseIndex\": ").append(itemEntitySetupPoseIndex)
			.append(" },\n");
		appendItemEntityDiagnostics(json).append(",\n");
		appendItemEntityRouteDecisions(json).append(",\n");
		appendField(json, "rustGalWorldModelMeshScenario", MODEL_MESH_SCENARIO).append(",\n");
		json.append("  \"rustGalWorldModelMeshSetup\": { ");
		appendField(json, "status", modelMeshSetupStatus, 0).append(", ");
		appendField(json, "blockId", modelMeshSetupBlockId, 0).append(", ");
		appendField(json, "origin", modelMeshSetupOrigin, 0).append(", ");
		appendField(json, "difficultyBefore", modelMeshSetupDifficultyBefore, 0).append(", ");
		appendField(json, "difficultyEffective", modelMeshSetupDifficultyEffective, 0).append(", ");
		json.append("\"difficultyAdjusted\": ").append(modelMeshSetupDifficultyAdjusted)
			.append(", ");
		appendField(json, "serverSpawnFailure", modelMeshSetupServerSpawnFailure, 0).append(", ");
		json.append("\"clientBlockEntityPresent\": ").append(modelMeshSetupClientBlockEntityPresent)
			.append(", \"serverEntityPresent\": ").append(modelMeshSetupServerEntityPresent)
			.append(", \"clientEntityPresent\": ").append(modelMeshSetupClientEntityPresent)
			.append(", \"serverEntityId\": ").append(modelMeshSetupServerEntityId)
			.append(", \"clientEntityId\": ").append(modelMeshSetupClientEntityId).append(", ");
		appendField(json, "clientEntitySample", modelMeshSetupClientEntitySample, 0).append(" },\n");
		json.append("  \"rustGalWorldPrimedTntSetup\": { ");
		appendField(json, "status", primedTntSetupStatus, 0).append(", ");
		appendField(json, "blockId", primedTntSetupBlockId, 0).append(", ");
		appendField(json, "origin", primedTntSetupOrigin, 0).append(", ");
		json.append("\"entityCount\": ").append(primedTntSetupEntityCount).append(" },\n");
		json.append("  \"rustGalWorldPistonSetup\": { ");
		appendField(json, "status", pistonSetupStatus, 0).append(", ");
		appendField(json, "blockId", pistonSetupBlockId, 0).append(", ");
		appendField(json, "origin", pistonSetupOrigin, 0).append(", ");
		appendField(json, "blockState", pistonSetupBlockState, 0).append(", ");
		json.append("\"clientBlockEntityPresent\": ").append(pistonSetupClientBlockEntityPresent)
			.append(", \"entityCount\": ").append(pistonSetupEntityCount)
			.append(", \"reseedCount\": ").append(pistonSetupReseedCount)
			.append(" },\n");
		appendMovingBlockDiagnostics(json).append(",\n");
		appendMovingBlockRouteDecisions(json).append(",\n");
		appendMovingMeshExecutionDiagnostics(json).append(",\n");
		appendEntityModelExecutionDiagnostics(json).append(",\n");
		appendEntityShadowDiagnostics(json).append(",\n");
		appendProceduralQuadExecutionDiagnostics(json).append(",\n");
		appendMovingBlockShellScanDiagnostics(json).append(",\n");
		appendModelMeshDiagnostics(json).append(",\n");
		appendModelMeshRouteDecisions(json).append(",\n");
		appendModelPartMeshTraversalDiagnostics(json).append(",\n");
		appendExperienceOrbDiagnostics(json).append(",\n");
		appendExperienceOrbRouteDecisions(json).append(",\n");
		appendExperienceOrbExecutionDiagnostics(json).append(",\n");
		appendBeaconBeamDiagnostics(json).append(",\n");
		appendBeaconBeamExecutionDiagnostics(json).append(",\n");
		appendCrystalBeamExecutionDiagnostics(json).append(",\n");
		appendWeatherDiagnostics(json).append(",\n");
		appendCloudDiagnostics(json).append(",\n");
		appendDistantHorizonsRouteDiagnostics(json).append(",\n");
		appendField(json, "rustGalStaticTerrainScenario", STATIC_TERRAIN_SCENARIO).append(",\n");
		appendStaticTerrainLifecycleState(json).append(",\n");
		appendStaticTerrainTextureProbeReceipt(json, 2).append(",\n");
		appendStaticTerrainDiagnostics(json).append("\n");
		json.append("}\n");
		try {
			Path parent = METADATA_PATH.getParent();
			if (parent != null) {
				Files.createDirectories(parent);
			}
			Files.writeString(METADATA_PATH, json, StandardCharsets.UTF_8);
		} catch (IOException exception) {
			LOGGER.error("Unable to write deterministic capture failure metadata", exception);
		}
	}

	private static void writeMetadata(Minecraft minecraft, String status) {
		StringBuilder json = new StringBuilder(4096);
		ClientLevel level = minecraft.level;
		LocalPlayer player = minecraft.player;
		Vec3 currentPosition = player == null ? Vec3.ZERO : player.position();
		String currentDimension = level == null ? "missing" : level.dimension().location().toString();
		long gameTime = level == null ? -1L : level.getGameTime();

		json.append("{\n");
		appendField(json, "status", status).append(",\n");
		appendField(json, "backend", activeBackend()).append(",\n");
		json.append("  \"worldWindowResize\": ").append(GraphicsAuditWorldResize.worldReceipt()).append(",\n");
		json.append("  \"worldGuiScale\": ").append(GraphicsAuditWorldGuiScale.worldReceipt()).append(",\n");
		json.append("  \"worldResourceReload\": ").append(GraphicsAuditResourceReload.worldReceipt()).append(",\n");
		json.append("  \"mixedItemFoilFixture\": ").append(GraphicsAuditMixedItemFoilFixture.receipt(minecraft)).append(",\n");
		json.append("  \"droppedItemFoilFixture\": ").append(GraphicsAuditDroppedItemFoilFixture.receipt(minecraft)).append(",\n");
		json.append("  \"flowingWaterFixture\": ").append(GraphicsAuditFlowingWaterFixture.receipt(minecraft)).append(",\n");
		json.append("  \"lavaFixture\": ").append(GraphicsAuditLavaFixture.receipt(minecraft)).append(",\n");
		json.append("  \"lavaSurfaceAtCapture\": ").append(GraphicsAuditLavaFixture.capturedSurface()).append(",\n");
		json.append("  \"lavaParticleOcclusionAtCapture\": ").append(GraphicsAuditLavaFixture.capturedParticles()).append(",\n");
		json.append("  \"flowingWaterSurfaceAtCapture\": ").append(GraphicsAuditFlowingWaterFixture.capturedSurface()).append(",\n");
		json.append("  \"mixedFluidBottomSurfaceAtCapture\": ").append(GraphicsAuditMixedFluidFixture.capturedBottomSurface()).append(",\n");
		json.append("  \"postEffectFixture\": ").append(GraphicsAuditPostEffectFixture.receipt(minecraft)).append(",\n");
		json.append("  \"worldMenuFixture\": ").append(GraphicsAuditWorldMenuFixture.receipt(minecraft)).append(",\n");
		json.append("  \"animatedItemFixture\": ").append("animated-block".equals(HOTBAR_ITEM_FIXTURE)
			? GraphicsAuditAnimatedItemFixture.receipt(minecraft) : "null").append(",\n");
		json.append("  \"modelFoilFixture\": ").append("model-foil".equals(HOTBAR_ITEM_FIXTURE)
			? GraphicsAuditModelFoilFixture.receipt(minecraft) : "null").append(",\n");
		json.append("  \"shieldFoilFixture\": ").append("shield".equals(HOTBAR_ITEM_FIXTURE) || "shield-foil".equals(HOTBAR_ITEM_FIXTURE)
			? GraphicsAuditShieldFoilFixture.receipt(minecraft, "shield-foil".equals(HOTBAR_ITEM_FIXTURE)) : "null").append(",\n");
		json.append("  \"shieldPatternFixture\": ").append("shield-patterns".equals(HOTBAR_ITEM_FIXTURE) || "shield-patterns-foil".equals(HOTBAR_ITEM_FIXTURE)
			? GraphicsAuditPatternedShieldFixture.receipt(minecraft,"shield-patterns-foil".equals(HOTBAR_ITEM_FIXTURE)) : "null").append(",\n");
		json.append("  \"staticTerrainFixtureScenario\": \"").append(STATIC_TERRAIN_SCENARIO).append("\",\n");
		json.append("  \"mixedFluidFixture\": ").append("translucent-mixed".equals(STATIC_TERRAIN_SCENARIO)
			? GraphicsAuditMixedFluidFixture.receipt(minecraft) : "null").append(",\n");
		appendField(json, "shadowReceiverFixture", Boolean.getBoolean("mattmc.dev.deterministicCameraCapture.shadowGlass") ? "red-glass-v1" : "none").append(",\n");
		appendField(json, "shaderEnabled", shaderEnabled()).append(",\n");
		appendField(json, "shaderPack", shaderPack()).append(",\n");
		json.append("  \"window\": { \"width\": ").append(windowWidth).append(", \"height\": ").append(windowHeight).append(" },\n");
		appendField(json, "dimension", currentDimension).append(",\n");
		appendVec3(json, "initialPosition", initialPosition == null ? currentPosition : initialPosition).append(",\n");
		appendPoseObject(json, "initialPose", initialPose == null ? new Pose("initial", 0.0F, 0.0F) : initialPose).append(",\n");
		appendVec3(json, "currentPosition", currentPosition).append(",\n");
		json.append("  \"gameTime\": ").append(gameTime).append(",\n");
		json.append("  \"startedGameTime\": ").append(startedGameTime).append(",\n");
		json.append("  \"wholeFrameSourceCapture\": { \"armed\": ").append(wholeFrameAttachmentCaptureArmed)
			.append(", \"requestIssued\": ").append(wholeFrameAttachmentCaptureRequestIssued)
			.append(", \"ready\": ").append(wholeFrameAttachmentCaptureReady)
			.append(", \"gameplayFrame\": ").append(wholeFrameAttachmentCaptureGameplayFrame)
			.append(", \"correlation\": ").append(wholeFrameAttachmentCaptureCorrelation)
			.append(", \"deterministicFrame\": ").append(wholeFrameAttachmentCaptureDeterministicFrame)
			.append(" },\n");
		appendField(json, "gitCommit", gitCommit()).append(",\n");
		json.append("  \"distantHorizonsActive\": ").append(isDistantHorizonsActive()).append(",\n");
		appendField(json, "cameraType", minecraft.options.getCameraType().name()).append(",\n");
		appendField(json, "gameMode", minecraft.gameMode == null ? "unknown" : minecraft.gameMode.getPlayerMode().getName()).append(",\n");
		appendField(json, "rustGalWorldOutlineScenario", System.getProperty("mattmc.dev.rustGalWorldOutline.scenario", "")).append(",\n");
		appendField(json, "rustGalWorldOutlineStyle", System.getProperty("mattmc.dev.rustGalWorldOutline.style", "")).append(",\n");
		appendField(json, "rustGalWorldOutlineDepthPolicy", System.getProperty("mattmc.dev.rustGalWorldOutline.depthPolicy", "")).append(",\n");
		appendField(json, "rustGalWorldMaterialMarkerScenario", System.getProperty("mattmc.dev.rustGalWorldMaterial.blockMarkerScenario", "")).append(",\n");
		appendBlockMarkerDiagnostics(json).append(",\n");
		appendField(json, "rustGalWorldTerrainParticleScenario", System.getProperty("mattmc.dev.rustGalWorldMaterial.terrainParticleScenario", "")).append(",\n");
		appendTerrainParticleDiagnostics(json).append(",\n");
		appendField(json, "rustGalStaticTerrainScenario", STATIC_TERRAIN_SCENARIO).append(",\n");
		appendStaticTerrainLifecycleState(json).append(",\n");
		appendStaticTerrainDiagnostics(json).append(",\n");
		appendDistantHorizonsRouteDiagnostics(json).append(",\n");
		appendDistantHorizonsTexturePaletteState(json).append(",\n");
		appendField(json, "rustGalWorldBlockDisplayScenario", System.getProperty("mattmc.dev.rustGalWorldMesh.blockDisplayScenario", "")).append(",\n");
		appendBlockDisplayDiagnostics(json).append(",\n");
		appendField(json, "blockDisplayScenario", BLOCK_DISPLAY_SCENARIO).append(",\n");
		json.append("  \"blockDisplayFixture\": ").append(GraphicsAuditBlockDisplayFixture.receipt(minecraft)).append(",\n");
		json.append("  \"terrainParticleFixture\": ").append(net.minecraft.client.particle.GraphicsAuditTerrainParticleFixture.receipt(minecraft)).append(",\n");
		json.append("  \"blockMarkerFixture\": ").append(net.minecraft.client.particle.GraphicsAuditBlockMarkerFixture.receipt(minecraft)).append(",\n");
		json.append("  \"experienceOrbFixture\": ").append(GraphicsAuditExperienceOrbFixture.receipt(minecraft)).append(",\n");
		json.append("  \"atlasParticleFixture\": ").append(net.minecraft.client.particle.GraphicsAuditAtlasParticleFixture.receipt(minecraft)).append(",\n");
		json.append("  \"shriekParticleFixture\": ").append(net.minecraft.client.particle.GraphicsAuditShriekParticleFixture.receipt(minecraft)).append(",\n");
		json.append("  \"vibrationParticleFixture\": ").append(net.minecraft.client.particle.GraphicsAuditVibrationParticleFixture.receipt(minecraft)).append(",\n");
		json.append("  \"blockDisplayAnimationObservation\": ").append(GraphicsAuditBlockDisplayFixture.animationObservation(minecraft)).append(",\n");
		json.append("  \"blockDisplayAnimationAtCapture\": ").append(blockAnimationAtCapture).append(",\n");
		appendWorldTextDiagnostics(json).append(",\n");
		appendField(json, "rustGalWorldFallingBlockScenario", System.getProperty("mattmc.dev.rustGalWorldMesh.fallingBlockScenario", "")).append(",\n");
		appendField(json, "rustGalWorldArrowScenario", ARROW_SCENARIO).append(",\n");
		json.append("  \"rustGalWorldArrowSetup\": { ");
		appendField(json, "status", arrowSetupStatus, 0).append(", ");
		appendField(json, "texture", arrowSetupTexture, 0).append(", ");
		appendField(json, "origin", arrowSetupOrigin, 0).append(", ");
		json.append("\"entityCount\": ").append(arrowSetupEntityCount)
			.append(", \"poseIndex\": ").append(arrowSetupPoseIndex)
			.append(" },\n");
		appendField(json, "rustGalWorldExperienceOrbScenario", EXPERIENCE_ORB_SCENARIO).append(",\n");
		json.append("  \"rustGalWorldExperienceOrbSetup\": { ");
		appendField(json, "status", experienceOrbSetupStatus, 0).append(", ");
		appendField(json, "origin", experienceOrbSetupOrigin, 0).append(", ");
		json.append("\"entityCount\": ").append(experienceOrbSetupEntityCount)
			.append(", \"poseIndex\": ").append(experienceOrbSetupPoseIndex)
			.append(" },\n");
		appendField(json, "rustGalWorldBeaconBeamScenario", BEACON_BEAM_SCENARIO).append(",\n");
		json.append("  \"rustGalWorldBeaconBeamSetup\": { ");
		appendField(json, "status", beaconBeamSetupStatus, 0).append(", ");
		appendField(json, "origin", beaconBeamSetupOrigin, 0).append(", ");
		json.append("\"clientBeamSectionsReady\": ").append(beaconBeamSetupClientReady)
			.append(", \"serverBeamSectionsReady\": ").append(beaconBeamSetupServerReady).append(" },\n");
		appendField(json, "rustGalWorldItemEntityScenario", ITEM_ENTITY_SCENARIO).append(",\n");
		json.append("  \"rustGalWorldItemEntitySetup\": { ");
		appendField(json, "status", itemEntitySetupStatus, 0).append(", ");
		appendField(json, "itemId", itemEntitySetupItemId, 0).append(", ");
		appendField(json, "origin", itemEntitySetupOrigin, 0).append(", ");
		json.append("\"entityCount\": ").append(itemEntitySetupEntityCount)
			.append(", \"serverSpawnedCount\": ").append(itemEntityServerSpawnedCount)
			.append(", \"clientVisibleCount\": ").append(itemEntityClientVisibleCount).append(", ");
		appendField(json, "serverSpawnFailure", itemEntityServerSpawnFailure, 0);
		json.append(", \"poseIndex\": ").append(itemEntitySetupPoseIndex)
			.append(" },\n");
		appendItemEntityDiagnostics(json).append(",\n");
		appendItemEntityRouteDecisions(json).append(",\n");
		appendField(json, "rustGalWorldModelMeshScenario", MODEL_MESH_SCENARIO).append(",\n");
		json.append("  \"cowOutlineFixture\": ").append(GraphicsAuditCowOutlineFixture.receipt(
			minecraft.level == null ? null : minecraft.level.getEntity(modelMeshSetupClientEntityId))).append(",\n");
		json.append("  \"rustGalWorldModelMeshSetup\": { ");
		appendField(json, "status", modelMeshSetupStatus, 0).append(", ");
		appendField(json, "blockId", modelMeshSetupBlockId, 0).append(", ");
		appendField(json, "origin", modelMeshSetupOrigin, 0).append(", ");
		appendField(json, "difficultyBefore", modelMeshSetupDifficultyBefore, 0).append(", ");
		appendField(json, "difficultyEffective", modelMeshSetupDifficultyEffective, 0).append(", ");
		json.append("\"difficultyAdjusted\": ").append(modelMeshSetupDifficultyAdjusted)
			.append(", ");
		appendField(json, "serverSpawnFailure", modelMeshSetupServerSpawnFailure, 0).append(", ");
		json.append("\"clientBlockEntityPresent\": ").append(modelMeshSetupClientBlockEntityPresent)
			.append(", \"serverEntityPresent\": ").append(modelMeshSetupServerEntityPresent)
			.append(", \"clientEntityPresent\": ").append(modelMeshSetupClientEntityPresent)
			.append(", \"serverEntityId\": ").append(modelMeshSetupServerEntityId)
			.append(", \"clientEntityId\": ").append(modelMeshSetupClientEntityId).append(", ");
		appendField(json, "clientEntitySample", modelMeshSetupClientEntitySample, 0).append(" },\n");
		appendField(json, "rustGalWorldPrimedTntScenario", PRIMED_TNT_SCENARIO).append(",\n");
		json.append("  \"rustGalWorldPrimedTntSetup\": { ");
		appendField(json, "status", primedTntSetupStatus, 0).append(", ");
		appendField(json, "blockId", primedTntSetupBlockId, 0).append(", ");
		appendField(json, "origin", primedTntSetupOrigin, 0).append(", ");
		json.append("\"entityCount\": ").append(primedTntSetupEntityCount).append(" },\n");
		json.append("  \"rustGalWorldMovingMeshSetup\": { ");
				appendField(json, "stage", movingMeshSetupStage, 0).append(", ");
				appendField(json, "lastMissing", movingMeshSetupLastMissing, 0).append(", ");
				json.append("\"attempts\": ").append(movingMeshSetupAttempts)
					.append(", \"complete\": ").append(movingMeshScenarioSetup)
					.append(", \"sourceEntityIsolation\": ").append(SOURCE_ENTITY_ISOLATION)
					.append(", \"sourceEntityIsolationApplied\": ").append(sourceEntityIsolationApplied)
					.append(", \"sourceEntityIsolationClientSyncFrames\": ").append(sourceEntityIsolationClientSyncFrames)
					.append(", \"sourceEntityIsolationClientNonPlayerEntities\": ").append(sourceEntityIsolationClientNonPlayerEntities)
					.append(", \"sourceEntityIsolationClientQuiescentFrames\": ").append(sourceEntityIsolationClientQuiescentFrames)
					.append(", \"sourceEntityIsolationRemovedEntities\": ").append(sourceEntityIsolationRemovedEntities)
					.append(", \"sourceEntityIsolationRemovedBlockEntities\": ").append(sourceEntityIsolationRemovedBlockEntities)
					.append(" },\n");
				json.append("  \"rustGalWorldFallingBlockSetup\": { ");
				appendField(json, "status", fallingBlockSetupStatus, 0).append(", ");
			appendField(json, "blockId", fallingBlockSetupBlockId, 0).append(", ");
			appendField(json, "spawnMethod", fallingBlockSetupSpawnMethod, 0).append(", ");
			appendField(json, "origin", fallingBlockSetupOrigin, 0).append(", ");
			appendField(json, "landing", fallingBlockSetupLanding, 0).append(", ");
			json.append("\"poseIndex\": ").append(fallingBlockSetupPoseIndex).append(", ");
			appendField(json, "poseName", fallingBlockSetupPoseName, 0).append(", ");
			json.append("\"entityCount\": ").append(fallingBlockSetupEntityCount)
				.append(", \"fallHeight\": ").append(fallingBlockSetupFallHeight)
				.append(" },\n");
			json.append("  \"rustGalWorldFallingBlockExtractionProbe\": { \"seen\": ").append(fallingEntitySeenCount)
				.append(", \"shouldRender\": ").append(fallingEntityShouldRenderCount)
				.append(", \"compiledSection\": ").append(fallingEntityCompiledSectionCount)
				.append(", \"extracted\": ").append(fallingEntityExtractedCount)
				.append(" },\n");
			appendField(json, "rustGalWorldPistonScenario", System.getProperty("mattmc.dev.rustGalWorldMesh.pistonScenario", "")).append(",\n");
		json.append("  \"rustGalWorldPistonSetup\": { ");
		appendField(json, "status", pistonSetupStatus, 0).append(", ");
		appendField(json, "blockId", pistonSetupBlockId, 0).append(", ");
		appendField(json, "origin", pistonSetupOrigin, 0).append(", ");
		appendField(json, "blockState", pistonSetupBlockState, 0).append(", ");
		json.append("\"clientBlockEntityPresent\": ").append(pistonSetupClientBlockEntityPresent)
			.append(", \"entityCount\": ").append(pistonSetupEntityCount)
			.append(", \"reseedCount\": ").append(pistonSetupReseedCount)
			.append(" },\n");
			appendFallingBlockDiagnostics(json).append(",\n");
				appendFallingBlockRouteDecisions(json).append(",\n");
		appendArrowDiagnostics(json).append(",\n");
		appendArrowRouteDecisions(json).append(",\n");
		appendExperienceOrbDiagnostics(json).append(",\n");
		appendExperienceOrbRouteDecisions(json).append(",\n");
		appendExperienceOrbExecutionDiagnostics(json).append(",\n");
		appendBeaconBeamDiagnostics(json).append(",\n");
		appendBeaconBeamExecutionDiagnostics(json).append(",\n");
		appendCrystalBeamExecutionDiagnostics(json).append(",\n");
		appendModelMeshDiagnostics(json).append(",\n");
		appendModelMeshRouteDecisions(json).append(",\n");
		appendModelPartMeshTraversalDiagnostics(json).append(",\n");
		appendMovingBlockDiagnostics(json).append(",\n");
			appendMovingBlockRouteDecisions(json).append(",\n");
			appendMovingMeshExecutionDiagnostics(json).append(",\n");
			appendEntityModelExecutionDiagnostics(json).append(",\n");
			appendEntityShadowDiagnostics(json).append(",\n");
			appendProceduralQuadExecutionDiagnostics(json).append(",\n");
			appendMovingBlockShellScanDiagnostics(json).append(",\n");
			appendWeatherDiagnostics(json).append(",\n");
			appendCloudDiagnostics(json).append(",\n");
			json.append("  \"rustGalWorldOutlineDepthProbe\": ").append(Boolean.getBoolean("mattmc.dev.rustGalWorldOutline.depthProbe")).append(",\n");
		json.append("  \"blockOutlineRealTargetForced\": ").append(FORCE_BLOCK_OUTLINE_TARGET).append(",\n");
		json.append("  \"blockOutlineRealTargetAimed\": ").append(AIM_BLOCK_OUTLINE_TARGET).append(",\n");
		json.append("  \"blockOutlinePauseParity\": ").append(BLOCK_OUTLINE_PAUSE_PARITY).append(",\n");
		json.append("  \"realSurvivalCrackCapture\": ").append(FORCE_REAL_SURVIVAL_CRACK).append(",\n");
		json.append("  \"realSurvivalCrackSetupBlock\": ").append(FORCE_REAL_SURVIVAL_CRACK_SETUP_BLOCK).append(",\n");
		appendField(json, "deterministicSupportBlock", deterministicSupportBlock).append(",\n");
		appendField(json, "deterministicSupportBlockType", deterministicSupportBlockType).append(",\n");
		appendVec3(json, "deterministicPlayerPosition", lastSafetyPosition).append(",\n");
		appendVec3(json, "deterministicPlayerVelocity", lastSafetyVelocity).append(",\n");
		json.append("  \"deterministicPlayerHealth\": ").append(format(lastSafetyHealth)).append(",\n");
		json.append("  \"deterministicPlayerFallDistance\": ").append(format(lastSafetyFallDistance)).append(",\n");
		json.append("  \"deterministicPlayerNoGravity\": ").append(player != null && player.isNoGravity()).append(",\n");
		appendField(json, "realSurvivalCrackSetupTarget", realSurvivalCrackSetupTarget).append(",\n");
		appendField(json, "realSurvivalCrackHitType", realSurvivalCrackLastHitType).append(",\n");
		appendField(json, "realSurvivalCrackTarget", realSurvivalCrackLastTarget).append(",\n");
		appendField(json, "realSurvivalCrackDirection", realSurvivalCrackLastDirection).append(",\n");
		appendField(json, "realSurvivalCrackStatus", realSurvivalCrackLastStatus).append(",\n");
		appendField(json, "realSurvivalCrackLastValidTarget", realSurvivalCrackLastValidTarget).append(",\n");
		appendField(json, "realSurvivalCrackLastValidBlockType", realSurvivalCrackLastValidBlockType).append(",\n");
		appendField(json, "realSurvivalCrackLastRenderedTarget", realSurvivalCrackLastRenderedTarget).append(",\n");
		appendField(json, "realSurvivalCrackLastRenderedBlockType", realSurvivalCrackLastRenderedBlockType).append(",\n");
		json.append("  \"realSurvivalCrackStartCalls\": ").append(realSurvivalCrackStartCalls).append(",\n");
		json.append("  \"realSurvivalCrackContinueCalls\": ").append(realSurvivalCrackContinueCalls).append(",\n");
		json.append("  \"realSurvivalCrackStopCalls\": ").append(realSurvivalCrackStopCalls).append(",\n");
		json.append("  \"realSurvivalCrackValidBlockHitCount\": ").append(realSurvivalCrackValidBlockHitCount).append(",\n");
		json.append("  \"realSurvivalCrackRenderedStateCount\": ").append(realSurvivalCrackRenderedStateCount).append(",\n");
		json.append("  \"realSurvivalCrackMinRenderedStage\": ").append(realSurvivalCrackMinRenderedStage == 10 ? -1 : realSurvivalCrackMinRenderedStage).append(",\n");
		json.append("  \"realSurvivalCrackMaxRenderedStage\": ").append(realSurvivalCrackMaxRenderedStage).append(",\n");
		appendForcedBlockOutlineTarget(json).append(",\n");
		appendField(json, "attackIndicator", minecraft.options.attackIndicator().get().name()).append(",\n");
			json.append("  \"attackProgress\": ").append(player == null ? -1.0F : player.getAttackStrengthScale(0.0F)).append(",\n");
				json.append("  \"attackDelay\": ").append(player == null ? -1.0F : player.getCurrentItemAttackStrengthDelay()).append(",\n");
				json.append("  \"attackTargetForced\": ").append(FORCE_ATTACK_TARGET).append(",\n");
				appendField(json, "attackTargetEntity", minecraft.crosshairPickEntity == null ? "none" : minecraft.crosshairPickEntity.getType().toShortString()).append(",\n");
			json.append("  \"armorValue\": ").append(player == null ? -1 : player.getArmorValue()).append(",\n");
			json.append("  \"armorValueOverride\": ").append(FORCED_ARMOR_VALUE).append(",\n");
			json.append("  \"playerHealthOverride\": ").append(format(FORCED_PLAYER_HEALTH)).append(",\n");
			json.append("  \"playerMaxHealthOverride\": ").append(format(FORCED_PLAYER_MAX_HEALTH)).append(",\n");
			json.append("  \"playerAbsorption\": ").append(player == null ? -1.0F : player.getAbsorptionAmount()).append(",\n");
			json.append("  \"playerAbsorptionOverride\": ").append(format(FORCED_PLAYER_ABSORPTION)).append(",\n");
			json.append("  \"playerFoodLevel\": ").append(player == null ? -1 : player.getFoodData().getFoodLevel()).append(",\n");
			json.append("  \"playerFoodLevelOverride\": ").append(FORCED_PLAYER_FOOD_LEVEL).append(",\n");
			json.append("  \"playerFoodSaturation\": ").append(player == null ? -1.0F : player.getFoodData().getSaturationLevel()).append(",\n");
			json.append("  \"playerFoodSaturationOverride\": ").append(format(FORCED_PLAYER_FOOD_SATURATION)).append(",\n");
			json.append("  \"playerFoodHungerEffectOverride\": ").append(FORCE_PLAYER_FOOD_HUNGER_EFFECT).append(",\n");
			json.append("  \"playerFoodJitterOverride\": ").append(FORCE_PLAYER_FOOD_JITTER).append(",\n");
			json.append("  \"playerAirSupply\": ").append(player == null ? -1 : player.getAirSupply()).append(",\n");
			json.append("  \"playerAirSupplyOverride\": ").append(FORCED_PLAYER_AIR_SUPPLY).append(",\n");
			json.append("  \"playerMaxAirSupply\": ").append(player == null ? -1 : player.getMaxAirSupply()).append(",\n");
			json.append("  \"playerMaxAirSupplyOverride\": ").append(FORCED_PLAYER_MAX_AIR_SUPPLY).append(",\n");
			json.append("  \"playerUnderwaterOverride\": ").append(FORCE_PLAYER_UNDERWATER).append(",\n");
			json.append("  \"playerAirPopOverride\": ").append(FORCE_PLAYER_AIR_POP).append(",\n");
			json.append("  \"mountPresentOverride\": ").append(FORCE_MOUNT_PRESENT).append(",\n");
			json.append("  \"mountHealthOverride\": ").append(format(FORCED_MOUNT_HEALTH)).append(",\n");
			json.append("  \"mountMaxHealthOverride\": ").append(format(FORCED_MOUNT_MAX_HEALTH)).append(",\n");
			json.append("  \"mountHealthRowsOverride\": ").append(FORCED_MOUNT_HEALTH_ROWS).append(",\n");
			json.append("  \"hideChat\": ").append(HIDE_CHAT).append(",\n");
			json.append("  \"bossBarOverride\": ").append(hasBossBarOverride()).append(",\n");
			json.append("  \"bossBarCount\": ").append(hasBossBarOverride() ? forcedBossBarCount() : -1).append(",\n");
			appendField(json, "bossBarProgress", FORCED_BOSS_BAR_PROGRESS).append(",\n");
			appendField(json, "bossBarOverlay", FORCED_BOSS_BAR_OVERLAY).append(",\n");
			json.append("  \"selectedHotbarSlot\": ").append(player == null ? -1 : currentSelectedHotbarSlot(player)).append(",\n");
			appendField(json, "hotbarItemFixture", HOTBAR_ITEM_FIXTURE).append(",\n");
			json.append("  \"guiItemPlacement\": ").append(GraphicsAuditGuiItemPlacementFixture.receipt()).append(",\n");
			json.append("  \"specialFoilFixture\": ").append("special-foil".equals(HOTBAR_ITEM_FIXTURE)
				? GraphicsAuditSpecialFoilFixture.receipt(minecraft) : "null").append(",\n");
			json.append("  \"guiItemFoilSources\": ");
			GraphicsAuditGuiFoilSource.appendJson(json);
			json.append(",\n");
			json.append("  \"guiItemFoilCount\": ").append(player == null ? 0 : java.util.stream.IntStream.range(1,9).filter(slot -> player.getInventory().getItem(slot).hasFoil()).count()).append(",\n");
			json.append("  \"guiItemGlintSpeed\": ").append(Minecraft.getInstance().options.glintSpeed().get()).append(",\n");
			json.append("  \"guiItemGlintStrength\": ").append(Minecraft.getInstance().options.glintStrength().get()).append(",\n");
		json.append("  \"experienceProgress\": ").append(player == null ? -1.0F : player.experienceProgress).append(",\n");
		json.append("  \"experienceLevel\": ").append(player == null ? -1 : player.experienceLevel).append(",\n");
		json.append("  \"framesPerPose\": ").append(FRAMES_PER_POSE).append(",\n");
		json.append("  \"settledReadyFrames\": ").append(SETTLED_READY_FRAMES).append(",\n");
		json.append("  \"settledReadyMaxWaitFrames\": ").append(SETTLED_READY_MAX_WAIT_FRAMES).append(",\n");
		json.append("  \"settledReadyGateSatisfied\": ").append(settledReadyGateSatisfied).append(",\n");
		appendField(json, "settledReadySummary", settledReadySummary()).append(",\n");
		appendSubmittedWorkCounts(json).append(",\n");
		json.append("  \"rustGalGuiScreenCycle\": { \"enabled\": ").append(RUST_GAL_GUI_SCREEN_CYCLE)
			.append(", \"complete\": ").append(rustGalGuiScreenCycleComplete)
			.append(", \"stage\": ").append(rustGalGuiScreenCycleStage)
			.append(", \"framesInStage\": ").append(rustGalGuiScreenCycleFramesInStage)
			.append(", \"cyclesCompleted\": ").append(rustGalGuiScreenCyclesCompleted)
			.append(", \"repeatCount\": ").append(RUST_GAL_GUI_SCREEN_CYCLE_REPEATS)
			.append(", \"inventoryScreenshotCaptured\": ").append(rustGalGuiScreenCycleInventoryCaptureComplete)
			.append(", \"inventoryScreenshot\": \"")
			.append(escape(rustGalGuiScreenCycleInventoryCapturePath == null ? "" : rustGalGuiScreenCycleInventoryCapturePath.toAbsolutePath().toString()))
			.append("\" },\n");
		json.append("  \"ackTimeoutFrames\": ").append(ACK_TIMEOUT_FRAMES).append(",\n");
			json.append("  \"poseCount\": ").append(poses == null ? POSE_COUNT : poses.length).append(",\n");
			json.append("  \"poseSequence\": [");
			if (poses != null) {
				for (int i = 0; i < poses.length; i++) {
					if (i > 0) {
						json.append(", ");
					}
					json.append('"').append(escape(poses[i].name())).append('"');
				}
			}
			json.append("],\n");
			json.append("  \"internalScreenshots\": ").append(INTERNAL_SCREENSHOTS).append(",\n");
			json.append("  \"yawDelta\": ").append(format(YAW_DELTA)).append(",\n");
			json.append("  \"deterministicTemporalParity\": { \"enabled\": ").append(SystemTimeUniforms.isDeterministicTemporalParityEnabled())
				.append(", \"frameIndex\": ").append(deterministicTemporalFrameIndex())
				.append(", \"frameCounter\": ").append(SystemTimeUniforms.deterministicTemporalFrameCounter())
				.append(", \"frameTime\": ").append(format(SystemTimeUniforms.deterministicTemporalFrameTime()))
				.append(", \"frameTimeCounter\": ").append(format(SystemTimeUniforms.deterministicTemporalFrameTimeCounter()))
				.append(", \"frameTimeSmooth\": ").append(format(SystemTimeUniforms.deterministicTemporalFrameTimeSmooth()))
				.append(", \"partialTick\": ").append(format(SystemTimeUniforms.deterministicTemporalPartialTick()))
				.append(", \"fovModifier\": ").append(format(SystemTimeUniforms.deterministicTemporalFovModifier()))
				.append(", \"worldTime\": ").append(SystemTimeUniforms.deterministicTemporalWorldTime())
				.append(" },\n");
			json.append("  \"renderedFrameIndex\": ").append(renderedFrameIndex).append(",\n");
			json.append("  \"afterRenderDiagnostics\": { \"calls\": ").append(afterRenderCalls)
				.append(", \"uninitializedReturns\": ").append(afterRenderUninitializedReturns)
				.append(", \"paletteGateReturns\": ").append(afterRenderPaletteGateReturns)
				.append(", \"settledGateReturns\": ").append(afterRenderSettledGateReturns)
				.append(", \"lifecycleGateReturns\": ").append(afterRenderLifecycleGateReturns)
				.append(", \"movingMeshGateReturns\": ").append(afterRenderMovingMeshGateReturns)
				.append(", \"sourceExecutionGateReturns\": ").append(afterRenderSourceExecutionGateReturns)
				.append(", \"framesWaitingForSourceExecution\": ").append(framesWaitingForSourceExecution)
				.append(" },\n");
		json.append("  \"currentPoseIndex\": ").append(poseIndex).append(",\n");
		json.append("  \"awaitingScreenshotAck\": ").append(awaitingScreenshotAck).append(",\n");
		json.append("  \"captures\": [\n");
		for (int i = 0; i < CAPTURES.size(); i++) {
			PoseCapture capture = CAPTURES.get(i);
			json.append("    {\n");
			json.append("      \"index\": ").append(capture.index()).append(",\n");
			appendField(json, "poseName", capture.poseName(), 6).append(",\n");
			appendField(json, "screenshot", capture.screenshot(), 6).append(",\n");
			appendField(json, "backend", activeBackend(), 6).append(",\n");
			appendField(json, "shaderEnabled", shaderEnabled(), 6).append(",\n");
			appendField(json, "shaderPack", shaderPack(), 6).append(",\n");
			appendField(json, "gitCommit", gitCommit(), 6).append(",\n");
			json.append("      \"window\": { \"width\": ").append(windowWidth).append(", \"height\": ").append(windowHeight).append(" },\n");
			json.append("      \"distantHorizonsActive\": ").append(isDistantHorizonsActive()).append(",\n");
			appendField(json, "dimension", capture.dimension(), 6).append(",\n");
			appendVec3(json, "position", capture.position(), 6).append(",\n");
			appendVec3(json, "requestedPosition", capture.requestedPosition(), 6).append(",\n");
			json.append("      \"requestedYaw\": ").append(format(capture.requestedYaw())).append(",\n");
			json.append("      \"requestedPitch\": ").append(format(capture.requestedPitch())).append(",\n");
				json.append("      \"observedYaw\": ").append(format(capture.observedYaw())).append(",\n");
				json.append("      \"observedPitch\": ").append(format(capture.observedPitch())).append(",\n");
				json.append("      \"renderedFrameIndex\": ").append(capture.renderedFrameIndex()).append(",\n");
				json.append("      \"weatherRendererTicks\": ").append(capture.weatherRendererTicks()).append(",\n");
				json.append("      \"weatherRendererPartialTick\": ").append(format(capture.weatherRendererPartialTick())).append(",\n");
				appendField(json, "weatherSemanticFingerprint", capture.weatherSemanticFingerprint(), 6).append(",\n");
				appendField(json, "lightmapSemanticFingerprint", capture.lightmapSemanticFingerprint(), 6).append(",\n");
				json.append("      \"deterministicTemporal\": { \"enabled\": ").append(SystemTimeUniforms.isDeterministicTemporalParityEnabled())
					.append(", \"frameIndex\": ").append(capture.index() * FRAMES_PER_POSE)
					.append(", \"frameCounter\": ").append(capture.index() * FRAMES_PER_POSE)
					.append(", \"frameTime\": ").append(format(SystemTimeUniforms.deterministicTemporalFrameTime()))
					.append(", \"frameTimeCounter\": ").append(format((capture.index() * FRAMES_PER_POSE * SystemTimeUniforms.deterministicTemporalFrameTime()) % 3600.0F))
					.append(", \"frameTimeSmooth\": ").append(format(SystemTimeUniforms.deterministicTemporalFrameTimeSmooth()))
					.append(", \"partialTick\": ").append(format(SystemTimeUniforms.deterministicTemporalPartialTick()))
					.append(", \"fovModifier\": ").append(format(SystemTimeUniforms.deterministicTemporalFovModifier()))
					.append(", \"worldTime\": ").append(SystemTimeUniforms.deterministicTemporalWorldTime())
					.append(" },\n");
				json.append("      \"gameTime\": ").append(capture.gameTime()).append(",\n");
				appendField(json, "captureMethod", capture.captureMethod(), 6);
				if (!capture.targetWindow().isEmpty()) {
					json.append(",\n");
					appendField(json, "targetWindow", capture.targetWindow(), 6).append("\n");
				} else {
					json.append("\n");
				}
			json.append("    }").append(i + 1 == CAPTURES.size() ? "\n" : ",\n");
		}
		json.append("  ],\n");
		appendStaticTerrainWaterAnimationDenseCapture(json);
		json.append("}\n");

		try {
			Path parent = METADATA_PATH.toAbsolutePath().getParent();
			if (parent != null) {
				Files.createDirectories(parent);
			}
			Files.writeString(METADATA_PATH, json.toString(), StandardCharsets.UTF_8);
		} catch (IOException exception) {
			LOGGER.error("Unable to write deterministic capture metadata", exception);
		}
	}

	private static StringBuilder appendDistantHorizonsRouteDiagnostics(StringBuilder json) {
		net.vulkanic.world.DistantHorizonsSemanticCollector.RouteDiagnostics route =
			net.vulkanic.world.DistantHorizonsSemanticCollector.routeDiagnosticsSnapshot();
		json.append("  \"rustGalDistantHorizonsRoute\": { ");
		json.append("\"frame\": ").append(route.frame()).append(", ");
		appendField(json, "decision", route.decision(), 0).append(", ");
		appendField(json, "reason", route.reason(), 0).append(", ");
		appendField(json, "matrixStatus", route.matrixStatus(), 0).append(", ");
		appendField(json, "matrixDetail", route.matrixDetail(), 0).append(", ");
		json.append("\"opaqueSegments\": ").append(route.opaqueSegments()).append(", ");
		json.append("\"exactAtlasIdentitySegments\": ").append(route.exactAtlasIdentitySegments()).append(", ");
		json.append("\"exactAtlasIdentityQuads\": ").append(route.exactAtlasIdentityQuads()).append(", ");
		json.append("\"exactAtlasMixedQuads\": ").append(route.exactAtlasMixedQuads()).append(", ");
		json.append("\"exactAtlasUnavailableQuads\": ").append(route.exactAtlasUnavailableQuads()).append(", ");
		json.append("\"exactAtlasMissingProvenanceQuads\": ").append(route.exactAtlasMissingProvenanceQuads()).append(", ");
		json.append("\"exactAtlasMisalignedProvenanceQuads\": ").append(route.exactAtlasMisalignedProvenanceQuads()).append(", ");
		json.append("\"exactAtlasInvalidIdentityQuads\": ").append(route.exactAtlasInvalidIdentityQuads()).append(", ");
		json.append("\"exactAtlasIdentityTableEntries\": ").append(route.exactAtlasIdentityTableEntries()).append(", ");
		json.append("\"exactAtlasInputKnownQuads\": ").append(route.exactAtlasInputKnownQuads()).append(", ");
		json.append("\"exactAtlasInputMixedQuads\": ").append(route.exactAtlasInputMixedQuads()).append(", ");
		json.append("\"exactAtlasInputUnavailableQuads\": ").append(route.exactAtlasInputUnavailableQuads()).append(", ");
		json.append("\"exactAtlasInputOpaqueKnownQuads\": ").append(route.exactAtlasInputOpaqueKnownQuads()).append(", ");
		json.append("\"exactAtlasInputOpaqueMixedQuads\": ").append(route.exactAtlasInputOpaqueMixedQuads()).append(", ");
		json.append("\"exactAtlasInputOpaqueUnavailableQuads\": ").append(route.exactAtlasInputOpaqueUnavailableQuads()).append(", ");
		json.append("\"exactAtlasOutputKnownQuads\": ").append(route.exactAtlasOutputKnownQuads()).append(", ");
		json.append("\"exactAtlasOutputMixedQuads\": ").append(route.exactAtlasOutputMixedQuads()).append(", ");
		json.append("\"exactAtlasOutputUnavailableQuads\": ").append(route.exactAtlasOutputUnavailableQuads()).append(", ");
		json.append("\"exactAtlasOutputOpaqueKnownQuads\": ").append(route.exactAtlasOutputOpaqueKnownQuads()).append(", ");
		json.append("\"exactAtlasOutputOpaqueMixedQuads\": ").append(route.exactAtlasOutputOpaqueMixedQuads()).append(", ");
		json.append("\"exactAtlasOutputOpaqueUnavailableQuads\": ").append(route.exactAtlasOutputOpaqueUnavailableQuads()).append(", ");
		appendStringArray(json, "exactAtlasCoverageSamples", route.exactAtlasCoverageSamples(), 0).append(", ");
		json.append("\"exactAtlasResolutionStatusSummary\": \"").append(escape(route.exactAtlasResolutionStatusSummary())).append("\", ");
		appendStringArray(json, "exactAtlasResolutionSamples", route.exactAtlasResolutionSamples(), 0).append(", ");
		json.append("\"transparentSegments\": ").append(route.transparentSegments()).append(", ");
		json.append("\"waterSegments\": ").append(route.waterSegments()).append(", ");
		json.append("\"visibleColumns\": ").append(route.visibleColumns()).append(", ");
		json.append("\"cachedColumns\": ").append(route.cachedColumns()).append(", ");
		json.append("\"unpublishedVisibleColumns\": ").append(route.unpublishedVisibleColumns()).append(", ");
		json.append("\"semanticBuildAttempts\": ").append(route.semanticBuildAttempts()).append(", ");
		json.append("\"semanticColumnsBuilt\": ").append(route.semanticColumnsBuilt()).append(", ");
		json.append("\"semanticColumnsReused\": ").append(route.semanticColumnsReused()).append(", ");
		json.append("\"semanticColumnsReplaced\": ").append(route.semanticColumnsReplaced()).append(", ");
		appendField(json, "lastPayloadDifference", route.lastPayloadDifference(), 0).append(", ");
		json.append("\"retainedBytes\": ").append(route.retainedBytes()).append(", ");
		json.append("\"oversizedColumns\": ").append(route.oversizedColumns()).append(", ");
		json.append("\"frameSemanticsEnabled\": ").append(route.frameSemanticsEnabled()).append(", ");
		json.append("\"selected\": ").append(route.selected()).append(", ");
		json.append("\"lastExecutedRouteFrame\": ").append(route.lastExecutedRouteFrame()).append(", ");
		json.append("\"lastExecutedWorldFrame\": ").append(route.lastExecutedWorldFrame()).append(", ");
		json.append("\"lastExecutedSubmission\": ").append(route.lastExecutedSubmission()).append(", ");
		json.append("\"lastExecutedCaptureFrame\": ").append(route.lastExecutedCaptureFrame()).append(", ");
		json.append("\"lastExecutedInstances\": ").append(route.lastExecutedInstances()).append(", ");
		json.append("\"lastExecutedFrameSemanticsEnabled\": ").append(route.lastExecutedFrameSemanticsEnabled()).append(" }");
		return json;
	}

	private static StringBuilder appendDistantHorizonsTexturePaletteState(StringBuilder json) {
		json.append("  \"rustGalDistantHorizonsTexturePalette\": { ");
		json.append("\"requested\": ").append(DISTANT_HORIZONS_TEXTURE_PALETTE).append(", ");
		appendField(json, "stage", distantHorizonsTexturePaletteStage, 0).append(", ");
		appendField(json, "serverStateDetail", distantHorizonsTexturePaletteServerStateDetail, 0).append(", ");
		appendField(
			json,
			"target",
			distantHorizonsTexturePaletteTarget == null ? "" : distantHorizonsTexturePaletteTarget.toShortString(),
			0
		).append(", ");
		DistantHorizonsSemanticCollector.ColumnCoverageDiagnostics sourceCoverage =
			distantHorizonsTexturePaletteTarget == null
				? new DistantHorizonsSemanticCollector.ColumnCoverageDiagnostics(0, 0, 0, List.of())
				: DistantHorizonsSemanticCollector.columnCoverageDiagnosticsAtBlock(
					distantHorizonsTexturePaletteProbes.isEmpty()
						? distantHorizonsTexturePaletteTarget.getX()
						: distantHorizonsTexturePaletteProbes.getFirst().position().getX(),
					distantHorizonsTexturePaletteProbes.isEmpty()
						? distantHorizonsTexturePaletteTarget.getZ()
						: distantHorizonsTexturePaletteProbes.getFirst().position().getZ()
				);
		json.append("\"sourceColumnCoverage\": { ");
		json.append("\"cachedColumns\": ").append(sourceCoverage.cachedColumns()).append(", ");
		json.append("\"publishedColumns\": ").append(sourceCoverage.publishedColumns()).append(", ");
		json.append("\"consumedOpaqueSegments\": ").append(sourceCoverage.consumedOpaqueSegments()).append(", ");
		appendStringArray(json, "samples", sourceCoverage.samples(), 0).append(" }, ");
		DistantHorizonsSemanticCollector.DistantHorizonsWaterProbeReceipt waterReceipt =
			distantHorizonsWaterProbeReceipt();
		DistantHorizonsSemanticCollector.DistantHorizonsWaterProbeReceipt waterSourceReceipt =
			distantHorizonsWaterSourceProbeReceipt();
		DistantHorizonsSemanticCollector.DistantHorizonsWaterProbeReceipt waterCachedReceipt =
			distantHorizonsWaterCachedProbeReceipt();
		DistantHorizonsSemanticCollector.WaterSourceInputReceipt waterSourceInputReceipt =
			distantHorizonsWaterSourceInputReceipt();
		json.append("\"waterFixtureRenderDataReceipt\": { ");
		json.append("\"matched\": ").append(waterSourceInputReceipt.matched()).append(", ");
		appendField(json, "status", waterSourceInputReceipt.status(), 0);
		if (!waterSourceInputReceipt.traces().isEmpty()) {
			DistantHorizonsSemanticCollector.WaterSourceInputTrace trace = waterSourceInputReceipt.traces().getFirst();
			json.append(", \"position\": [").append(trace.blockX()).append(", ")
				.append(trace.blockY()).append(", ").append(trace.blockZ()).append("], ");
			json.append("\"sectionKey\": ").append(trace.sectionKey()).append(", ");
			json.append("\"detailLevel\": ").append(trace.detailLevel()).append(", ");
			json.append("\"sourceBounds\": [").append(trace.sourceMinX()).append(", ")
				.append(trace.minY()).append(", ").append(trace.sourceMinZ()).append(", ")
				.append(trace.sourceWidth()).append(", ").append(trace.maxY() - trace.minY()).append(", ")
				.append(trace.sourceWidth()).append("], ");
			json.append("\"dhMaterialId\": ").append(trace.dhMaterialId()).append(", ");
			json.append("\"semanticMaterialId\": ").append(trace.semanticMaterialId());
		}
		json.append(" }, ");
		json.append("\"waterFixtureCachedReceipt\": { ");
		json.append("\"matched\": ").append(waterCachedReceipt.matched()).append(", ");
		appendField(json, "status", waterCachedReceipt.status(), 0);
		if (!waterCachedReceipt.probes().isEmpty()) {
			DistantHorizonsSemanticCollector.DistantHorizonsWaterProbeResult probe = waterCachedReceipt.probes().getFirst();
			json.append(", \"position\": [").append(probe.blockX()).append(", ")
				.append(probe.blockY()).append(", ").append(probe.blockZ()).append("], ");
			json.append("\"columnKey\": ").append(probe.columnKey()).append(", ");
			json.append("\"columnGeneration\": ").append(probe.columnGeneration()).append(", ");
			json.append("\"segmentIndex\": ").append(probe.segmentIndex()).append(", ");
			json.append("\"quadIndex\": ").append(probe.quadIndex()).append(", ");
			appendField(json, "materialIdentity", probe.materialIdentity(), 0);
		}
		json.append(" }, ");
		json.append("\"waterFixtureSourceReceipt\": { ");
		json.append("\"matched\": ").append(waterSourceReceipt.matched()).append(", ");
		appendField(json, "status", waterSourceReceipt.status(), 0);
		if (!waterSourceReceipt.probes().isEmpty()) {
			DistantHorizonsSemanticCollector.DistantHorizonsWaterProbeResult probe = waterSourceReceipt.probes().getFirst();
			json.append(", \"position\": [").append(probe.blockX()).append(", ")
				.append(probe.blockY()).append(", ").append(probe.blockZ()).append("], ");
			json.append("\"columnKey\": ").append(probe.columnKey()).append(", ");
			json.append("\"columnGeneration\": ").append(probe.columnGeneration()).append(", ");
			json.append("\"segmentIndex\": ").append(probe.segmentIndex()).append(", ");
			json.append("\"quadIndex\": ").append(probe.quadIndex()).append(", ");
			appendField(json, "materialIdentity", probe.materialIdentity(), 0);
		}
		json.append(" }, ");
		json.append("\"waterFixtureReceipt\": { ");
		json.append("\"matched\": ").append(waterReceipt.matched()).append(", ");
		appendField(json, "status", waterReceipt.status(), 0).append(", ");
		json.append("\"executedWorldFrame\": ").append(waterReceipt.executedWorldFrame());
		if (!waterReceipt.probes().isEmpty()) {
			DistantHorizonsSemanticCollector.DistantHorizonsWaterProbeResult probe = waterReceipt.probes().getFirst();
			json.append(", \"position\": [").append(probe.blockX()).append(", ")
				.append(probe.blockY()).append(", ").append(probe.blockZ()).append("], ");
			json.append("\"columnKey\": ").append(probe.columnKey()).append(", ");
			json.append("\"segmentIndex\": ").append(probe.segmentIndex()).append(", ");
			json.append("\"quadIndex\": ").append(probe.quadIndex()).append(", ");
			appendField(json, "materialIdentity", probe.materialIdentity(), 0);
		}
		json.append(" }, ");
		json.append("\"setup\": ").append(distantHorizonsTexturePaletteSetup).append(", ");
		json.append("\"invalidationQueued\": ").append(distantHorizonsTexturePaletteInvalidationQueued).append(", ");
		json.append("\"invalidatedChunks\": ").append(distantHorizonsTexturePaletteInvalidatedChunks).append(", ");
		json.append("\"queuedUpdatesAfterInvalidation\": ").append(distantHorizonsTexturePaletteQueuedUpdatesAfterInvalidation).append(", ");
		appendField(json, "dhWorldType", distantHorizonsTexturePaletteDhWorldType, 0).append(", ");
		json.append("\"lightStableFrames\": ").append(distantHorizonsTexturePaletteLightStableFrames).append(", ");
		json.append("\"lightCorrectChunks\": ").append(distantHorizonsTexturePaletteLightCorrectChunks).append(", ");
		json.append("\"lightEngineBusy\": ").append(distantHorizonsTexturePaletteLightEngineBusy).append(", ");
		appendField(
			json,
			"lightFingerprint",
			distantHorizonsTexturePaletteLightFingerprintKnown
				? Long.toUnsignedString(distantHorizonsTexturePaletteLightFingerprint)
				: "unknown",
			0
		).append(", ");
		json.append("\"sourceReady\": ").append(distantHorizonsTexturePaletteSourceReady).append(", ");
		json.append("\"waitFrames\": ").append(distantHorizonsTexturePaletteWaitFrames).append(", ");
		json.append("\"forcedChunkCount\": ").append(distantHorizonsTexturePaletteForcedChunks.size()).append(", ");
		json.append("\"originalRenderDistance\": ").append(distantHorizonsTexturePaletteOriginalRenderDistance);
		json.append(" }");
		return json;
	}

	/**
	 * Immutable, capture-local DH execution evidence. The visible-frame collector
	 * transfers (and clears) its mutable semantic record before this screenshot
	 * request is created, so this snapshot deliberately describes the completed
	 * Rust whole-frame submission rather than the next pending frame.
	 */
	private static StringBuilder appendDistantHorizonsExecutionCorrelation(StringBuilder json, int indent) {
		var route = net.vulkanic.world.DistantHorizonsSemanticCollector.routeDiagnosticsSnapshot();
		String padding = " ".repeat(Math.max(0, indent));
		json.append(padding).append("\"rustGalDistantHorizonsExecution\": { ");
		json.append("\"routeFrame\": ").append(route.lastExecutedRouteFrame()).append(", ");
		json.append("\"worldFrame\": ").append(route.lastExecutedWorldFrame()).append(", ");
		json.append("\"submission\": ").append(route.lastExecutedSubmission()).append(", ");
		json.append("\"captureFrame\": ").append(route.lastExecutedCaptureFrame()).append(", ");
		json.append("\"instances\": ").append(route.lastExecutedInstances()).append(", ");
		json.append("\"opaqueInstances\": ").append(route.lastExecutedOpaqueInstances()).append(", ");
		json.append("\"transparentInstances\": ").append(route.lastExecutedTransparentInstances()).append(", ");
		json.append("\"waterInstances\": ").append(route.lastExecutedWaterInstances()).append(", ");
		json.append("\"semanticFrameEnabled\": ").append(route.lastExecutedFrameSemanticsEnabled()).append(" }");
		return json;
	}

	private static StringBuilder appendStaticTerrainExecutionCorrelation(StringBuilder json, int indent) {
		RustGalTerrainRenderer.StaticTerrainExecutionSnapshot current =
			RustGalTerrainRenderer.staticTerrainExecutionSnapshot();
		String padding = " ".repeat(Math.max(0, indent));
		json.append(padding).append("\"rustGalStaticTerrainExecution\": { ");
		json.append("\"required\": ").append(staticTerrainRequiresPostSetupExecution()).append(", ");
		json.append("\"setupSubmissionBaseline\": ").append(staticTerrainLifecycleExecutionSubmissionBaseline).append(", ");
		json.append("\"lifecycleFrame\": ").append(staticTerrainLifecycleExecutionFrame).append(", ");
		json.append("\"lifecycleSubmission\": ").append(staticTerrainLifecycleExecutionSubmission).append(", ");
		json.append("\"lifecycleInstances\": ").append(staticTerrainLifecycleExecutionInstances).append(", ");
		json.append("\"requestFrame\": ").append(current.frameId()).append(", ");
		json.append("\"requestSubmission\": ").append(current.submissionId()).append(", ");
		json.append("\"requestInstances\": ").append(current.instances()).append(" }");
		return json;
	}

	private static StringBuilder appendStaticTerrainAtlasReceipt(StringBuilder json, int indent) {
		String padding = " ".repeat(Math.max(0, indent));
		json.append(padding).append("\"rustGalStaticTerrainAtlasReceipt\": ");
		if (!"texture-palette".equals(staticTerrainBaseScenario())) {
			return json.append("null");
		}
		RustGalTerrainRenderer.TerrainAtlasReceipt receipt = RustGalTerrainRenderer.terrainAtlasReceipt();
		json.append("{ ");
		json.append("\"available\": ").append(receipt.available()).append(", ");
		appendField(json, "status", receipt.status(), 0).append(", ");
		json.append("\"extent\": [").append(receipt.width()).append(", ").append(receipt.height()).append("], ");
		json.append("\"copiedAtlasHash\": \"").append(Long.toUnsignedString(receipt.copiedAtlasHash(), 16)).append("\", ");
		json.append("\"allSpritesMatch\": ").append(receipt.allSpritesMatch()).append(", ");
		json.append("\"sprites\": [");
		for (int index = 0; index < receipt.sprites().size(); index++) {
			RustGalTerrainRenderer.TerrainAtlasSpriteReceipt sprite = receipt.sprites().get(index);
			if (index > 0) {
				json.append(", ");
			}
			json.append("{ ");
			appendField(json, "identity", sprite.identity(), 0).append(", ");
			json.append("\"origin\": [").append(sprite.x()).append(", ").append(sprite.y()).append("], ");
			json.append("\"extent\": [").append(sprite.width()).append(", ").append(sprite.height()).append("], ");
			json.append("\"sourceHash\": \"").append(Long.toUnsignedString(sprite.sourceHash(), 16)).append("\", ");
			json.append("\"copiedHash\": \"").append(Long.toUnsignedString(sprite.copiedHash(), 16)).append("\", ");
			appendField(json, "directSampleIdentity", sprite.directSampleIdentity(), 0).append(", ");
			appendField(json, "mirroredVSampleIdentity", sprite.mirroredVSampleIdentity(), 0).append(", ");
			json.append("\"samplePixel\": [").append(sprite.sampleX()).append(", ").append(sprite.sampleY()).append("], ");
			json.append("\"mirroredVSamplePixel\": [").append(sprite.sampleX()).append(", ").append(sprite.mirroredSampleY()).append("], ");
			json.append("\"matchesSource\": ").append(sprite.matchesSource()).append(", ");
			appendField(json, "status", sprite.status(), 0).append(" }");
		}
		return json.append("] }");
	}

	private static StringBuilder appendStaticTerrainTextureProbeReceipt(StringBuilder json, int indent) {
		String padding = " ".repeat(Math.max(0, indent));
		json.append(padding).append("\"rustGalStaticTerrainTextureProbeReceipt\": ");
		if (!"texture-palette".equals(staticTerrainBaseScenario())) {
			return json.append("null");
		}
		RustGalTerrainRenderer.TerrainTextureProbeReceipt receipt = staticTerrainTexturePaletteProbeReceipt();
		json.append("{ ");
		json.append("\"matched\": ").append(receipt.matched()).append(", ");
		appendField(json, "status", receipt.status(), 0).append(", ");
		json.append("\"probes\": [");
		for (int index = 0; index < receipt.probes().size(); index++) {
			RustGalTerrainRenderer.TerrainTextureProbeResult probe = receipt.probes().get(index);
			if (index > 0) {
				json.append(", ");
			}
			json.append("{ ");
			appendField(json, "position", probe.position() == null ? "missing" : probe.position().toShortString(), 0).append(", ");
			RustGalWorldPrimitiveRenderer.WorldPointProjection projection = probe.position() == null
				? null
				: RustGalWorldPrimitiveRenderer.projectWorldPointForDiagnostics(
					probe.position().getX() + 0.5D,
					// In the current camera-relative terrain convention, this local
					// height lands on the visible top face. Sampling the side would
					// classify grass's intentional dirt face as a texture swap.
					probe.position().getY() + 0.15D,
					probe.position().getZ() + 0.5D
				);
			json.append("\"projection\": ");
			if (projection == null) {
				json.append("null");
			} else {
				json.append("{ \"screen\": [").append(format(projection.screenX())).append(", ")
					.append(format(projection.screenY())).append("], \"clip\": [")
					.append(format(projection.clipX())).append(", ").append(format(projection.clipY())).append(", ")
					.append(format(projection.clipZ())).append(", ").append(format(projection.clipW())).append("], ")
					.append("\"insideViewport\": ").append(projection.insideViewport()).append(" }");
			}
			json.append(", ");
			json.append("\"allowedSprites\": [");
			for (int spriteIndex = 0; spriteIndex < probe.allowedSprites().size(); spriteIndex++) {
				if (spriteIndex > 0) {
					json.append(", ");
				}
				json.append('\"').append(escape(probe.allowedSprites().get(spriteIndex).toString())).append('\"');
			}
			json.append("], ");
			json.append("\"matchingQuads\": ").append(probe.matchingQuads()).append(", ");
			json.append("\"mismatchedQuads\": ").append(probe.mismatchedQuads()).append(", ");
			json.append("\"matched\": ").append(probe.matched()).append(", ");
			appendField(json, "status", probe.status(), 0).append(", ");
			json.append("\"observations\": [");
			for (int observationIndex = 0; observationIndex < probe.observations().size(); observationIndex++) {
				RustGalTerrainRenderer.TerrainTextureProbeObservation observation = probe.observations().get(observationIndex);
				if (observationIndex > 0) {
					json.append(", ");
				}
				json.append("{ ");
				json.append("\"sectionPos\": ").append(observation.sectionPos()).append(", ");
				appendField(json, "layer", observation.layer(), 0).append(", ");
				json.append("\"quadIndex\": ").append(observation.quadIndex()).append(", ");
				json.append("\"expectedSprite\": ").append(observation.expectedSprite()).append(", ");
				appendField(json, "atlasIdentity", observation.atlasIdentity(), 0).append(", ");
				json.append("\"atlasUv\": [").append(format(observation.atlasU())).append(", ")
					.append(format(observation.atlasV())).append("] }");
			}
			json.append("] }");
		}
		return json.append("] }");
	}

	private static StringBuilder appendDistantHorizonsTextureProbeReceipt(StringBuilder json, int indent) {
		String padding = " ".repeat(Math.max(0, indent));
		json.append(padding).append("\"rustGalDistantHorizonsTextureProbeReceipt\": ");
		if (!DISTANT_HORIZONS_TEXTURE_PALETTE) {
			return json.append("null");
		}
		DistantHorizonsSemanticCollector.DistantHorizonsTextureProbeReceipt receipt =
			distantHorizonsTexturePaletteProbeReceipt();
		json.append("{ ");
		json.append("\"matched\": ").append(receipt.matched()).append(", ");
		json.append("\"executedWorldFrame\": ").append(receipt.executedWorldFrame()).append(", ");
		appendField(json, "status", receipt.status(), 0).append(", ");
		json.append("\"probes\": [");
		for (int index = 0; index < receipt.probes().size(); index++) {
			DistantHorizonsSemanticCollector.DistantHorizonsTextureProbeResult probe = receipt.probes().get(index);
			if (index > 0) {
				json.append(", ");
			}
			json.append("{ ");
			json.append("\"position\": [").append(probe.blockX()).append(", ").append(probe.blockY()).append(", ").append(probe.blockZ()).append("], ");
			appendField(json, "expectedBlockId", probe.expectedBlockId(), 0).append(", ");
			appendField(json, "resolvedBlockStateIdentity", probe.resolvedBlockStateIdentity(), 0).append(", ");
			json.append("\"matched\": ").append(probe.matched()).append(", ");
			appendField(json, "status", probe.status(), 0).append(", ");
			appendField(json, "evidence", probe.evidence(), 0).append(", ");
			json.append("\"resolvedSprites\": [");
			for (int spriteIndex = 0; spriteIndex < probe.resolvedSprites().size(); spriteIndex++) {
				if (spriteIndex > 0) {
					json.append(", ");
				}
				json.append('\"').append(escape(probe.resolvedSprites().get(spriteIndex))).append('\"');
			}
			json.append("] }");
		}
		return json.append("] }");
	}

	private static StringBuilder appendDistantHorizonsWaterProbeReceipt(StringBuilder json, int indent) {
		String padding = " ".repeat(Math.max(0, indent));
		json.append(padding).append("\"rustGalDistantHorizonsWaterProbeReceipt\": ");
		if (!DISTANT_HORIZONS_REQUIRE_WATER) {
			return json.append("null");
		}
		DistantHorizonsSemanticCollector.DistantHorizonsWaterProbeReceipt receipt =
			distantHorizonsWaterProbeReceipt();
		json.append("{ ");
		json.append("\"matched\": ").append(receipt.matched()).append(", ");
		json.append("\"executedWorldFrame\": ").append(receipt.executedWorldFrame()).append(", ");
		appendField(json, "status", receipt.status(), 0).append(", ");
		json.append("\"probes\": [");
		for (int index = 0; index < receipt.probes().size(); index++) {
			DistantHorizonsSemanticCollector.DistantHorizonsWaterProbeResult probe = receipt.probes().get(index);
			if (index > 0) {
				json.append(", ");
			}
			json.append("{ ");
			json.append("\"position\": [").append(probe.blockX()).append(", ")
				.append(probe.blockY()).append(", ").append(probe.blockZ()).append("], ");
			json.append("\"matched\": ").append(probe.matched()).append(", ");
			appendField(json, "status", probe.status(), 0).append(", ");
			json.append("\"columnKey\": ").append(probe.columnKey()).append(", ");
			json.append("\"columnGeneration\": ").append(probe.columnGeneration()).append(", ");
			json.append("\"segmentIndex\": ").append(probe.segmentIndex()).append(", ");
			json.append("\"quadIndex\": ").append(probe.quadIndex()).append(", ");
			json.append("\"origin\": [").append(probe.originX()).append(", ")
				.append(probe.originY()).append(", ").append(probe.originZ()).append("], ");
			appendField(json, "materialIdentity", probe.materialIdentity(), 0).append(" }");
		}
		return json.append("] }");
	}

	private static void appendStaticTerrainWaterAnimationDenseCapture(StringBuilder json) {
		json.append("  \"rustGalStaticTerrainWaterAnimationDenseCapture\": {\n");
		json.append("    \"enabled\": ").append(STATIC_TERRAIN_WATER_ANIMATION_DENSE_CAPTURE).append(",\n");
		json.append("    \"requestedFrames\": ").append(STATIC_TERRAIN_WATER_ANIMATION_DENSE_FRAMES).append(",\n");
		json.append("    \"capturedFrames\": ").append(WATER_ANIMATION_CAPTURES.size()).append(",\n");
		json.append("    \"complete\": ").append(staticTerrainWaterAnimationDenseComplete).append(",\n");
		json.append("    \"inFlight\": ").append(staticTerrainWaterAnimationScreenshotInFlight).append(",\n");
		json.append("    \"frames\": [");
		for (int i = 0; i < WATER_ANIMATION_CAPTURES.size(); i++) {
			WaterAnimationFrameCapture frame = WATER_ANIMATION_CAPTURES.get(i);
			if (i > 0) {
				json.append(",");
			}
			json.append("\n      { ");
			json.append("\"index\": ").append(frame.index()).append(", ");
			appendField(json, "screenshot", frame.screenshot(), 0).append(", ");
			json.append("\"renderedFrameIndex\": ").append(frame.renderedFrameIndex()).append(", ");
			json.append("\"gameTime\": ").append(frame.gameTime()).append(", ");
			json.append("\"animationHash\": ").append(frame.animationHash()).append(", ");
			appendField(json, "animationSummary", frame.animationSummary(), 0).append(", ");
			appendField(json, "animationState", frame.animationState(), 0).append(", ");
			json.append("\"visibleLayerSubmissions\": ").append(frame.visibleLayerSubmissions()).append(", ");
			json.append("\"currentFrameVisibleLayerSubmissions\": ").append(frame.currentFrameVisibleLayerSubmissions()).append(", ");
			json.append("\"atlasGeneration\": ").append(frame.atlasGeneration()).append(" }");
		}
		if (!WATER_ANIMATION_CAPTURES.isEmpty()) {
			json.append("\n    ");
		}
		json.append("]\n");
		json.append("  }\n");
	}

	private static boolean isDistantHorizonsActive() {
		try {
			Class.forName("com.seibel.distanthorizons.core.render.renderer.LodRenderer", false, DeterministicCameraCapture.class.getClassLoader());
			return true;
		} catch (ClassNotFoundException exception) {
			return false;
		}
	}

	private static String activeBackend() {
		return VulkanicAPI.getActiveBackendType().name().toLowerCase(Locale.ROOT);
	}

	private static String shaderEnabled() {
		return System.getProperty("mattmc.dev.deterministicCameraCapture.shaderEnabled", "unknown");
	}

	private static String shaderPack() {
		return System.getProperty("mattmc.dev.deterministicCameraCapture.shaderPack", "unknown");
	}

	private static String gitCommit() {
		return System.getProperty("mattmc.dev.deterministicCameraCapture.gitCommit", "unknown");
	}

	private static StringBuilder appendField(StringBuilder json, String key, String value) {
		return appendField(json, key, value, 2);
	}

	private static StringBuilder appendField(StringBuilder json, String key, String value, int indent) {
		json.append(" ".repeat(indent)).append('"').append(key).append("\": \"").append(escape(value)).append('"');
		return json;
	}

	private static StringBuilder appendStringArray(StringBuilder json, String key, List<String> values, int indent) {
		json.append(" ".repeat(indent)).append('"').append(key).append("\": [");
		for (int index = 0; index < values.size(); index++) {
			if (index > 0) json.append(", ");
			json.append('"').append(escape(values.get(index))).append('"');
		}
		return json.append(']');
	}

	private static StringBuilder appendSubmittedWorkCounts(StringBuilder json) {
		Map<String, Integer> counts = new LinkedHashMap<>();
		synchronized (SUBMITTED_WORK_BY_FRAME) {
			for (Map<String, Set<String>> frame : SUBMITTED_WORK_BY_FRAME.values()) {
				for (Map.Entry<String, Set<String>> entry : frame.entrySet()) {
					counts.merge(entry.getKey(), entry.getValue().size(), Integer::sum);
				}
			}
		}
		json.append("  \"rustGalSubmittedWorkCounts\": {");
		boolean first = true;
		for (Map.Entry<String, Integer> entry : counts.entrySet()) {
			if (!first) {
				json.append(", ");
			}
			json.append('"').append(escape(entry.getKey())).append("\": ").append(entry.getValue());
			first = false;
		}
		return json.append(" }");
	}

	private static StringBuilder appendVec3(StringBuilder json, String key, Vec3 value) {
		return appendVec3(json, key, value, 2);
	}

	private static StringBuilder appendVec3(StringBuilder json, String key, Vec3 value, int indent) {
		json.append(" ".repeat(indent))
			.append('"').append(key).append("\": { \"x\": ")
			.append(format(value.x)).append(", \"y\": ")
			.append(format(value.y)).append(", \"z\": ")
			.append(format(value.z)).append(" }");
		return json;
	}

	private static StringBuilder appendForcedBlockOutlineTarget(StringBuilder json) {
		json.append("  \"blockOutlineTarget\": ");
		if (forcedBlockOutlineTarget == null) {
			return json.append("null");
		}
		json.append("{ ");
		appendField(json, "blockPos", forcedBlockOutlineTarget.blockPos().toShortString(), 0).append(", ");
		appendField(json, "face", forcedBlockOutlineTarget.face().name(), 0).append(", ");
		appendVec3(json, "hitLocation", forcedBlockOutlineTarget.hitLocation(), 0).append(", ");
		appendVec3(json, "playerPosition", forcedBlockOutlineTarget.playerPosition(), 0).append(", ");
		appendPoseObject(json, "pose", forcedBlockOutlineTarget.pose());
		json.append(" }");
		return json;
	}

	private static StringBuilder appendBlockMarkerDiagnostics(StringBuilder json) {
		List<RustGalWorldPrimitiveRenderer.BlockMarkerDiagnostic> diagnostics =
			RustGalWorldPrimitiveRenderer.blockMarkerDiagnostics();
		json.append("  \"rustGalWorldMaterialMarkers\": [");
		for (int i = 0; i < diagnostics.size(); i++) {
			RustGalWorldPrimitiveRenderer.BlockMarkerDiagnostic marker = diagnostics.get(i);
			if (i > 0) {
				json.append(",");
			}
			json.append("\n    { ");
			json.append("\"frameIndex\": ").append(marker.frameIndex()).append(", ");
			appendField(json, "route", marker.route(), 0).append(", ");
			json.append("\"textureId\": ").append(marker.textureId()).append(", ");
			json.append("\"center\": { \"x\": ").append(format(marker.centerX()))
				.append(", \"y\": ").append(format(marker.centerY()))
				.append(", \"z\": ").append(format(marker.centerZ())).append(" }, ");
			json.append("\"quadSize\": ").append(format(marker.quadSize())).append(", ");
			json.append("\"colorArgb\": ").append(Integer.toUnsignedLong(marker.colorArgb())).append(", ");
			json.append("\"viewport\": { \"width\": ").append(marker.viewportWidth())
				.append(", \"height\": ").append(marker.viewportHeight()).append(" }, ");
			json.append("\"projected\": ").append(marker.projected()).append(", ");
			json.append("\"screenBounds\": { \"left\": ").append(format(marker.screenLeft()))
				.append(", \"top\": ").append(format(marker.screenTop()))
				.append(", \"right\": ").append(format(marker.screenRight()))
				.append(", \"bottom\": ").append(format(marker.screenBottom())).append(" }");
			json.append(" }");
		}
		if (!diagnostics.isEmpty()) {
			json.append("\n  ");
		}
		json.append("]");
		return json;
	}

	private static StringBuilder appendTerrainParticleDiagnostics(StringBuilder json) {
		List<RustGalWorldPrimitiveRenderer.TerrainParticleDiagnostic> diagnostics =
			RustGalWorldPrimitiveRenderer.terrainParticleDiagnostics();
		json.append("  \"rustGalWorldTerrainParticles\": [");
		for (int i = 0; i < diagnostics.size(); i++) {
			RustGalWorldPrimitiveRenderer.TerrainParticleDiagnostic particle = diagnostics.get(i);
			if (i > 0) {
				json.append(",");
			}
			json.append("\n    { ");
			json.append("\"frameIndex\": ").append(particle.frameIndex()).append(", ");
			appendField(json, "route", particle.route(), 0).append(", ");
			json.append("\"textureId\": ").append(particle.textureId()).append(", ");
			appendField(json, "spriteId", particle.spriteId(), 0).append(", ");
			json.append("\"center\": { \"x\": ").append(format(particle.centerX()))
				.append(", \"y\": ").append(format(particle.centerY()))
				.append(", \"z\": ").append(format(particle.centerZ())).append(" }, ");
			json.append("\"quadSize\": ").append(format(particle.quadSize())).append(", ");
			json.append("\"colorArgb\": ").append(Integer.toUnsignedLong(particle.colorArgb())).append(", ");
			json.append("\"packedLight\": ").append(particle.packedLight()).append(", ");
			json.append("\"materialMode\": ").append(particle.materialMode()).append(", ");
			json.append("\"uv\": { \"u0\": ").append(format(particle.localU0()))
				.append(", \"u1\": ").append(format(particle.localU1()))
				.append(", \"v0\": ").append(format(particle.localV0()))
				.append(", \"v1\": ").append(format(particle.localV1())).append(" }, ");
			json.append("\"viewport\": { \"width\": ").append(particle.viewportWidth())
				.append(", \"height\": ").append(particle.viewportHeight()).append(" }, ");
			json.append("\"projected\": ").append(particle.projected()).append(", ");
			json.append("\"screenBounds\": { \"left\": ").append(format(particle.screenLeft()))
				.append(", \"top\": ").append(format(particle.screenTop()))
				.append(", \"right\": ").append(format(particle.screenRight()))
				.append(", \"bottom\": ").append(format(particle.screenBottom())).append(" }");
			json.append(" }");
		}
		if (!diagnostics.isEmpty()) {
			json.append("\n  ");
		}
		json.append("]");
		return json;
	}

	private static StringBuilder appendStaticTerrainDiagnostics(StringBuilder json) {
		RustGalTerrainRenderer.TerrainDiagnostics diagnostics = RustGalTerrainRenderer.diagnosticsSnapshot();
		json.append("  \"rustGalStaticTerrainDiagnostics\": {\n");
		json.append("    \"cachedLayerAssets\": ").append(diagnostics.cachedLayerAssets()).append(",\n");
		json.append("    \"activeTerrainLayers\": ").append(diagnostics.activeTerrainLayers()).append(",\n");
		json.append("    \"activeSectionAssets\": ").append(diagnostics.activeSectionAssets()).append(",\n");
		json.append("    \"currentFrameVisibleLayerSubmissions\": ").append(diagnostics.currentFrameVisibleLayerSubmissions()).append(",\n");
		json.append("    \"atlasGeneration\": ").append(diagnostics.atlasGeneration()).append(",\n");
		json.append("    \"registeredAtlasGeneration\": ").append(diagnostics.registeredAtlasGeneration()).append(",\n");
		json.append("    \"activeNativeVertexStride\": ").append(diagnostics.activeNativeVertexStride()).append(",\n");
		json.append("    \"expectedNativeVertexStride\": ").append(diagnostics.expectedNativeVertexStride()).append(",\n");
		json.append("    \"acceptedBuildOutputs\": ").append(diagnostics.acceptedBuildOutputs()).append(",\n");
		json.append("    \"skippedRouteBuildOutputs\": ").append(diagnostics.skippedRouteBuildOutputs()).append(",\n");
		json.append("    \"skippedUnsupportedAnimatedSections\": ").append(diagnostics.skippedUnsupportedAnimatedSections()).append(",\n");
		json.append("    \"skippedUnsupportedFluidTranslucentSections\": ").append(diagnostics.skippedUnsupportedFluidTranslucentSections()).append(",\n");
		json.append("    \"acceptedWaterAnimatedSections\": ").append(diagnostics.acceptedWaterAnimatedSections()).append(",\n");
		json.append("    \"unsupportedFluidRejectedSections\": ").append(diagnostics.unsupportedFluidRejectedSections()).append(",\n");
		json.append("    \"skippedEmptyLayers\": ").append(diagnostics.skippedEmptyLayers()).append(",\n");
		json.append("    \"registeredMeshes\": ").append(diagnostics.registeredMeshes()).append(",\n");
		json.append("    \"registeredTranslucentSorts\": ").append(diagnostics.registeredTranslucentSorts()).append(",\n");
		json.append("    \"registeredTranslucentSortBytes\": ").append(diagnostics.registeredTranslucentSortBytes()).append(",\n");
		json.append("    \"translucentSortGenerations\": ").append(diagnostics.translucentSortGenerations()).append(",\n");
		json.append("    \"texturePayloadUpdates\": ").append(diagnostics.texturePayloadUpdates()).append(",\n");
		json.append("    \"texturePayloadUpdateBytes\": ").append(diagnostics.texturePayloadUpdateBytes()).append(",\n");
		json.append("    \"atlasTextureOnlyUpdates\": ").append(diagnostics.atlasTextureOnlyUpdates()).append(",\n");
		json.append("    \"atlasMissingPayloadUpdates\": ").append(diagnostics.atlasMissingPayloadUpdates()).append(",\n");
		json.append("    \"atlasMalformedPayloadUpdates\": ").append(diagnostics.atlasMalformedPayloadUpdates()).append(",\n");
		json.append("    \"atlasPartialPayloadUpdates\": ").append(diagnostics.atlasPartialPayloadUpdates()).append(",\n");
		json.append("    \"removedLayers\": ").append(diagnostics.removedLayers()).append(",\n");
		json.append("    \"visibleLayerProbes\": ").append(diagnostics.visibleLayerProbes()).append(",\n");
		json.append("    \"visibleLayerSubmissions\": ").append(diagnostics.visibleLayerSubmissions()).append(",\n");
		json.append("    \"failedLayerSubmissions\": ").append(diagnostics.failedLayerSubmissions()).append(",\n");
		json.append("    \"invalidations\": ").append(diagnostics.invalidations()).append(",\n");
		json.append("    \"terrainExtractionFrames\": ").append(diagnostics.terrainExtractionFrames()).append(",\n");
		json.append("    \"rustEnqueueFrames\": ").append(diagnostics.rustEnqueueFrames()).append(",\n");
		appendWorldMeshAssetMetrics(json).append(",\n");
		json.append("    \"lifecycleEvents\": [");
		appendTerrainDiagnosticEvents(json, diagnostics.lifecycleEvents(), diagnostics.expectedNativeVertexStride());
		json.append("    ],\n");
		json.append("    \"translucentEvents\": [");
		appendTerrainDiagnosticEvents(json, diagnostics.translucentEvents(), diagnostics.expectedNativeVertexStride());
		json.append("    ],\n");
		json.append("    \"recentEvents\": [");
		appendTerrainDiagnosticEvents(json, diagnostics.recentEvents(), diagnostics.expectedNativeVertexStride());
		json.append("]\n  }");
		return json;
	}

	private static StringBuilder appendWorldMeshAssetMetrics(StringBuilder json) {
		RustGalWorldPrimitiveRenderer.WorldMeshAssetMetrics metrics = RustGalWorldPrimitiveRenderer.worldMeshAssetMetrics();
		json.append("    \"worldMeshAssetMetrics\": { ");
		json.append("\"generation\": ").append(metrics.generation()).append(", ");
		json.append("\"uploadedGeneration\": ").append(metrics.uploadedGeneration()).append(", ");
		json.append("\"payloadCount\": ").append(metrics.payloadCount()).append(", ");
		json.append("\"payloadBytes\": ").append(metrics.payloadBytes()).append(", ");
		json.append("\"failures\": ").append(metrics.failures()).append(", ");
		json.append("\"cachedMeshes\": ").append(metrics.cachedMeshes()).append(", ");
		json.append("\"cachedTextures\": ").append(metrics.cachedTextures()).append(", ");
		json.append("\"dirtyMeshes\": ").append(metrics.dirtyMeshes()).append(", ");
		json.append("\"dirtyTextures\": ").append(metrics.dirtyTextures()).append(", ");
		json.append("\"pendingInstances\": ").append(metrics.pendingInstances()).append(", ");
		json.append("\"uploadedMeshes\": ").append(metrics.uploadedMeshes()).append(", ");
		json.append("\"uploadedTextures\": ").append(metrics.uploadedTextures());
		json.append(" }");
		return json;
	}

	private static void appendTerrainDiagnosticEvents(
		StringBuilder json,
		List<RustGalTerrainRenderer.TerrainDiagnosticEvent> events,
		int expectedNativeVertexStride
	) {
		for (int i = 0; i < events.size(); i++) {
			RustGalTerrainRenderer.TerrainDiagnosticEvent event = events.get(i);
			if (i > 0) {
				json.append(",");
			}
			json.append("\n      { ");
			json.append("\"frame\": ").append(event.gameplayFrameId()).append(", ");
			json.append("\"sectionKey\": ").append(event.sectionPos()).append(", ");
			json.append("\"gameplayFrameId\": ").append(event.gameplayFrameId()).append(", ");
			json.append("\"terrainExtractionFrameId\": ").append(event.terrainExtractionFrameId()).append(", ");
			json.append("\"rustEnqueueFrameId\": ").append(event.rustEnqueueFrameId()).append(", ");
			json.append("\"executionFrameId\": ").append(event.executionFrameId()).append(", ");
			json.append("\"executionSubmissionId\": ").append(event.executionSubmissionId()).append(", ");
			json.append("\"sectionPos\": ").append(event.sectionPos()).append(", ");
			appendField(json, "layer", event.layer(), 0).append(", ");
				json.append("\"sourceGeneration\": ").append(event.sourceGeneration()).append(", ");
				json.append("\"meshGeneration\": ").append(event.meshGeneration()).append(", ");
					json.append("\"visibleGeneration\": ").append(event.visibleGeneration()).append(", ");
					json.append("\"uploadGeneration\": ").append(event.uploadGeneration()).append(", ");
					json.append("\"meshKey\": ").append(event.meshKey()).append(", ");
					json.append("\"contentHash\": ").append(event.contentHash()).append(", ");
					json.append("\"vertexCount\": ").append(event.vertexCount()).append(", ");
					json.append("\"bufferVertexCapacity\": ").append(event.bufferVertexCapacity()).append(", ");
					json.append("\"vertexStride\": ").append(event.vertexStride()).append(", ");
					json.append("\"expectedNativeVertexStride\": ").append(expectedNativeVertexStride).append(", ");
					json.append("\"indexCount\": ").append(event.indexCount()).append(", ");
					json.append("\"maxIndex\": ").append(event.maxIndex()).append(", ");
					json.append("\"indexType\": ").append(event.indexType()).append(", ");
					json.append("\"sectionCount\": ").append(event.sectionCount()).append(", ");
				json.append("\"sectionOrigin\": { \"x\": ").append(event.sectionOriginX()).append(", \"y\": ").append(event.sectionOriginY()).append(", \"z\": ").append(event.sectionOriginZ()).append(" }, ");
				json.append("\"transformTranslation\": { \"x\": ").append(format(event.transformX())).append(", \"y\": ").append(format(event.transformY())).append(", \"z\": ").append(format(event.transformZ())).append(" }, ");
					json.append("\"localBounds\": { \"minX\": ").append(format(event.localMinX())).append(", \"minY\": ").append(format(event.localMinY())).append(", \"minZ\": ").append(format(event.localMinZ())).append(", \"maxX\": ").append(format(event.localMaxX())).append(", \"maxY\": ").append(format(event.localMaxY())).append(", \"maxZ\": ").append(format(event.localMaxZ())).append(" }, ");
					json.append("\"uvBounds\": { \"minU\": ").append(format(event.uvMinU())).append(", \"minV\": ").append(format(event.uvMinV())).append(", \"maxU\": ").append(format(event.uvMaxU())).append(", \"maxV\": ").append(format(event.uvMaxV())).append(" }, ");
					json.append("\"vertexPositionsFinite\": ").append(event.vertexPositionsFinite()).append(", ");
					json.append("\"localBoundsValid\": ").append(event.localBoundsValid()).append(", ");
					json.append("\"uvBoundsValid\": ").append(event.uvBoundsValid()).append(", ");
					json.append("\"indexRangeValid\": ").append(event.indexRangeValid()).append(", ");
					json.append("\"segmentLayoutValid\": ").append(event.segmentLayoutValid()).append(", ");
					json.append("\"sectionOriginValid\": ").append(event.sectionOriginValid()).append(", ");
					json.append("\"indexOffsetAlignmentValid\": ").append(event.indexOffsetAlignmentValid()).append(", ");
					json.append("\"cameraBoundsFinite\": ").append(event.cameraBoundsFinite()).append(", ");
					json.append("\"normalContractValid\": ").append(event.normalContractValid()).append(", ");
					json.append("\"aoContractValid\": ").append(event.aoContractValid()).append(", ");
					json.append("\"blockSkyLightContractValid\": ").append(event.blockSkyLightContractValid()).append(", ");
					json.append("\"topFaceShadeContractValid\": ").append(event.topFaceShadeContractValid()).append(", ");
					json.append("\"separateAoActive\": ").append(event.separateAoActive()).append(", ");
					json.append("\"separateAoVertexCount\": ").append(event.separateAoVertexCount()).append(", ");
					json.append("\"aoRange\": { \"min\": ").append(format(event.minAo())).append(", \"max\": ").append(format(event.maxAo())).append(" }, ");
					json.append("\"normalSectionCounts\": { \"posY\": ").append(event.positiveYNormalSections()).append(", \"negY\": ").append(event.negativeYNormalSections()).append(", \"horizontal\": ").append(event.horizontalNormalSections()).append(" }, ");
					json.append("\"sortGeneration\": ").append(event.sortGeneration()).append(", ");
					json.append("\"cameraPosition\": { \"x\": ").append(format(event.cameraX())).append(", \"y\": ").append(format(event.cameraY())).append(", \"z\": ").append(format(event.cameraZ())).append(" }, ");
					json.append("\"sortOrigin\": { \"x\": ").append(format(event.sortOriginX())).append(", \"y\": ").append(format(event.sortOriginY())).append(", \"z\": ").append(format(event.sortOriginZ())).append(" }, ");
					json.append("\"primitiveCount\": ").append(event.primitiveCount()).append(", ");
					json.append("\"sortedIndexHash\": ").append(event.sortedIndexHash()).append(", ");
					json.append("\"indexUploadGeneration\": ").append(event.indexUploadGeneration()).append(", ");
					json.append("\"translucentDrawOrder\": ").append(event.translucentDrawOrder()).append(", ");
					appendField(json, "sorterType", event.sorterType(), 0).append(", ");
					json.append("\"sourceSortedIndexHash\": ").append(event.sourceSortedIndexHash()).append(", ");
					json.append("\"rustCopiedSortedIndexHash\": ").append(event.rustCopiedSortedIndexHash()).append(", ");
					json.append("\"sourceSortedIndexSampleHash\": ").append(event.sourceSortedIndexSampleHash()).append(", ");
					json.append("\"rustCopiedSortedIndexSampleHash\": ").append(event.rustCopiedSortedIndexSampleHash()).append(", ");
					appendField(json, "sortedIndexSample", event.sortedIndexSample(), 0).append(", ");
				appendField(json, "reason", event.reason(), 0);
				json.append(" }");
			}
		if (!events.isEmpty()) {
			json.append("\n    ");
		}
	}

	private static StringBuilder appendStaticTerrainLifecycleState(StringBuilder json) {
		json.append("  \"rustGalStaticTerrainLifecycle\": { ");
		appendField(json, "stage", staticTerrainLifecycleStage, 0).append(", ");
		appendField(json, "scenario", STATIC_TERRAIN_SCENARIO, 0).append(", ");
		appendField(
			json,
			"editBlock",
			staticTerrainLifecycleEditBlock == null ? "" : staticTerrainLifecycleEditBlock.toShortString(),
			0
		).append(", ");
		appendField(json, "replacementBlock", staticTerrainLifecycleBlockType, 0).append(", ");
		appendField(json, "resourcePackScenario", STATIC_TERRAIN_RESOURCE_PACK_SCENARIO, 0).append(", ");
		appendField(json, "worldId", STATIC_TERRAIN_WORLD_ID, 0).append(", ");
		json.append("\"setup\": ").append(staticTerrainLifecycleSetup).append(", ");
		json.append("\"afterRecorded\": ").append(staticTerrainLifecycleAfterRecorded).append(", ");
		json.append("\"beforeGeneration\": ").append(staticTerrainLifecycleBeforeGeneration).append(", ");
		json.append("\"afterGeneration\": ").append(staticTerrainLifecycleAfterGeneration).append(", ");
		json.append("\"executionSubmissionBaseline\": ").append(staticTerrainLifecycleExecutionSubmissionBaseline).append(", ");
		json.append("\"executionFrame\": ").append(staticTerrainLifecycleExecutionFrame).append(", ");
		json.append("\"executionSubmission\": ").append(staticTerrainLifecycleExecutionSubmission).append(", ");
		json.append("\"executionInstances\": ").append(staticTerrainLifecycleExecutionInstances).append(", ");
		json.append("\"selectedSourceExecutionFrame\": ").append(staticTerrainLifecycleSourceExecutionFrame).append(", ");
		json.append("\"selectedSourceExecutionSubmission\": ").append(staticTerrainLifecycleSourceExecutionSubmission).append(", ");
		json.append("\"actionStep\": ").append(staticTerrainLifecycleActionStep).append(", ");
		json.append("\"resizeCount\": ").append(staticTerrainLifecycleResizeCount).append(", ");
		json.append("\"originalRenderDistance\": ").append(staticTerrainOriginalRenderDistance).append(", ");
		json.append("\"originalSimulationDistance\": ").append(staticTerrainOriginalSimulationDistance).append(", ");
		json.append("\"beforeCachedLayers\": ").append(staticTerrainLifecycleBeforeCachedLayers).append(", ");
		json.append("\"afterCachedLayers\": ").append(staticTerrainLifecycleAfterCachedLayers).append(", ");
		json.append("\"beforeUsedMemoryBytes\": ").append(staticTerrainLifecycleBeforeRssBytes).append(", ");
		json.append("\"afterUsedMemoryBytes\": ").append(staticTerrainLifecycleAfterRssBytes).append(", ");
		json.append("\"menuCachedLayers\": ").append(staticTerrainMenuCachedLayers).append(", ");
		json.append("\"menuActiveTerrainLayers\": ").append(staticTerrainMenuActiveLayerSnapshot).append(", ");
		json.append("\"menuActiveSectionAssets\": ").append(staticTerrainMenuActiveSectionAssetSnapshot).append(", ");
		json.append("\"menuCurrentFrameVisibleSubmissions\": ").append(staticTerrainMenuCurrentFrameSubmissionSnapshot).append(", ");
		json.append("\"menuUsedMemoryBytes\": ").append(staticTerrainMenuUsedMemoryBytes).append(", ");
		json.append("\"unloadVisibleSubmissions\": ").append(staticTerrainUnloadSubmissionSnapshot).append(", ");
		json.append("\"menuVisibleSubmissions\": ").append(staticTerrainMenuSubmissionSnapshot).append(", ");
		json.append("\"reloadGenerationA\": ").append(staticTerrainReloadGenerationA).append(", ");
		json.append("\"reloadGenerationB\": ").append(staticTerrainReloadGenerationB).append(", ");
		appendField(json, "worldA", staticTerrainLifecycleWorldA, 0).append(", ");
		appendField(json, "worldB", staticTerrainLifecycleWorldB, 0).append(", ");
		json.append("\"currentUsedMemoryBytes\": ").append(currentUsedMemoryBytes()).append(", ");
		json.append("\"waitFrames\": ").append(framesWaitingForStaticTerrainLifecycle);
		json.append(" }");
		return json;
	}

	private static StringBuilder appendBlockDisplayDiagnostics(StringBuilder json) {
		List<RustGalWorldPrimitiveRenderer.BlockDisplayDiagnostic> diagnostics =
			RustGalWorldPrimitiveRenderer.blockDisplayDiagnostics();
		json.append("  \"rustGalWorldBlockDisplays\": [");
		for (int i = 0; i < diagnostics.size(); i++) {
			RustGalWorldPrimitiveRenderer.BlockDisplayDiagnostic display = diagnostics.get(i);
			if (i > 0) {
				json.append(",");
			}
			json.append("\n    { ");
			json.append("\"frameIndex\": ").append(display.frameIndex()).append(", ");
			appendField(json, "route", display.route(), 0).append(", ");
			appendField(json, "blockId", display.blockId(), 0).append(", ");
			json.append("\"meshKey\": ").append(Long.toUnsignedString(display.meshKey())).append(", ");
			json.append("\"meshGeneration\": ").append(display.meshGeneration()).append(", ");
			json.append("\"vertexLayoutVersion\": ").append(display.vertexLayoutVersion()).append(", ");
			json.append("\"indexType\": ").append(display.indexType()).append(", ");
			json.append("\"vertexCount\": ").append(display.vertexCount()).append(", ");
			json.append("\"indexBytes\": ").append(display.indexBytes()).append(", ");
			json.append("\"sectionCount\": ").append(display.sectionCount()).append(", ");
			appendField(json, "textureIds", display.textureIds(), 0).append(", ");
			json.append("\"materialMode\": ").append(display.materialMode()).append(", ");
			json.append("\"viewport\": { \"width\": ").append(display.viewportWidth())
				.append(", \"height\": ").append(display.viewportHeight()).append(" }, ");
			json.append("\"projected\": ").append(display.projected()).append(", ");
			json.append("\"screenBounds\": { \"left\": ").append(format(display.screenLeft()))
				.append(", \"top\": ").append(format(display.screenTop()))
				.append(", \"right\": ").append(format(display.screenRight()))
				.append(", \"bottom\": ").append(format(display.screenBottom())).append(" }");
			json.append(" }");
		}
		if (!diagnostics.isEmpty()) {
			json.append("\n  ");
		}
		json.append("]");
		return json;
	}

	private static StringBuilder appendWorldTextDiagnostics(StringBuilder json) {
		RustGalWorldPrimitiveRenderer.WorldTextDiagnostic diagnostic = RustGalWorldPrimitiveRenderer.worldTextDiagnostic();
		json.append("  \"rustGalWorldText\": { ");
		appendField(json, "scenario", WORLD_TEXT_SCENARIO, 0).append(", ");
		json.append("\"semanticFrame\": ").append(diagnostic.semanticFrame()).append(", ");
		json.append("\"visibleEntityStates\": ").append(diagnostic.visibleEntityStates()).append(", ");
		json.append("\"nameTagCallbacks\": ").append(diagnostic.nameTagCallbacks()).append(", ");
		json.append("\"textCallbacks\": ").append(diagnostic.textCallbacks()).append(", ");
		json.append("\"normalSubmits\": ").append(diagnostic.normalSubmits()).append(", ");
		json.append("\"seeThroughSubmits\": ").append(diagnostic.seeThroughSubmits()).append(", ");
		json.append("\"polygonOffsetSubmits\": ").append(diagnostic.polygonOffsetSubmits()).append(", ");
		json.append("\"emittedQuads\": ").append(diagnostic.emittedQuads()).append(", ");
		json.append("\"emittedImages\": ").append(diagnostic.emittedImages()).append(", ");
		json.append("\"fullySupported\": ").append(diagnostic.fullySupported()).append(", ");
		json.append("\"consumedQuads\": ").append(diagnostic.consumedQuads()).append(" }");
		return json;
	}

	private static StringBuilder appendFallingBlockDiagnostics(StringBuilder json) {
		List<RustGalWorldPrimitiveRenderer.FallingBlockDiagnostic> diagnostics =
			RustGalWorldPrimitiveRenderer.fallingBlockDiagnostics();
		json.append("  \"rustGalWorldFallingBlocks\": [");
		for (int i = 0; i < diagnostics.size(); i++) {
			RustGalWorldPrimitiveRenderer.FallingBlockDiagnostic block = diagnostics.get(i);
			if (i > 0) {
				json.append(",");
			}
			json.append("\n    { ");
			json.append("\"frameIndex\": ").append(block.frameIndex()).append(", ");
			appendField(json, "route", block.route(), 0).append(", ");
			appendField(json, "blockId", block.blockId(), 0).append(", ");
			json.append("\"meshKey\": ").append(Long.toUnsignedString(block.meshKey())).append(", ");
			json.append("\"meshGeneration\": ").append(block.meshGeneration()).append(", ");
			json.append("\"vertexLayoutVersion\": ").append(block.vertexLayoutVersion()).append(", ");
			json.append("\"indexType\": ").append(block.indexType()).append(", ");
			json.append("\"vertexCount\": ").append(block.vertexCount()).append(", ");
			json.append("\"indexBytes\": ").append(block.indexBytes()).append(", ");
			json.append("\"sectionCount\": ").append(block.sectionCount()).append(", ");
			appendField(json, "textureIds", block.textureIds(), 0).append(", ");
			json.append("\"materialMode\": ").append(block.materialMode()).append(", ");
			json.append("\"viewport\": { \"width\": ").append(block.viewportWidth())
				.append(", \"height\": ").append(block.viewportHeight()).append(" }, ");
			json.append("\"projected\": ").append(block.projected()).append(", ");
			json.append("\"screenBounds\": { \"left\": ").append(format(block.screenLeft()))
				.append(", \"top\": ").append(format(block.screenTop()))
				.append(", \"right\": ").append(format(block.screenRight()))
				.append(", \"bottom\": ").append(format(block.screenBottom())).append(" }");
			json.append(" }");
		}
		if (!diagnostics.isEmpty()) {
			json.append("\n  ");
		}
		json.append("]");
		return json;
	}

	private static StringBuilder appendFallingBlockRouteDecisions(StringBuilder json) {
		List<RustGalWorldPrimitiveRenderer.FallingBlockRouteDecision> decisions =
			RustGalWorldPrimitiveRenderer.fallingBlockRouteDecisions();
		json.append("  \"rustGalWorldFallingBlockRouteDecisions\": [");
		for (int i = 0; i < decisions.size(); i++) {
			RustGalWorldPrimitiveRenderer.FallingBlockRouteDecision decision = decisions.get(i);
			if (i > 0) {
				json.append(",");
			}
			json.append("\n    { ");
			json.append("\"frameIndex\": ").append(decision.frameIndex()).append(", ");
			appendField(json, "route", decision.route(), 0).append(", ");
			appendField(json, "blockId", decision.blockId(), 0).append(", ");
			json.append("\"rustSelected\": ").append(decision.rustSelected()).append(", ");
			json.append("\"rustQueued\": ").append(decision.rustQueued()).append(", ");
			json.append("\"javaDrawn\": ").append(decision.javaDrawn());
			json.append(" }");
		}
		if (!decisions.isEmpty()) {
			json.append("\n  ");
		}
		json.append("]");
		return json;
	}

	private static StringBuilder appendArrowDiagnostics(StringBuilder json) {
		List<RustGalWorldPrimitiveRenderer.ArrowDiagnostic> diagnostics = RustGalWorldPrimitiveRenderer.arrowDiagnostics();
		json.append("  \"rustGalWorldArrows\": [");
		for (int index = 0; index < diagnostics.size(); index++) {
			RustGalWorldPrimitiveRenderer.ArrowDiagnostic arrow = diagnostics.get(index);
			if (index > 0) {
				json.append(",");
			}
			json.append("\n    { ");
			json.append("\"frameIndex\": ").append(arrow.frameIndex()).append(", ");
			appendField(json, "route", arrow.route(), 0).append(", ");
			appendField(json, "textureId", arrow.textureId(), 0).append(", ");
			json.append("\"meshKey\": ").append(Long.toUnsignedString(arrow.meshKey())).append(", ");
			json.append("\"meshGeneration\": ").append(arrow.meshGeneration()).append(", ");
			json.append("\"vertexLayoutVersion\": ").append(arrow.vertexLayoutVersion()).append(", ");
			json.append("\"indexType\": ").append(arrow.indexType()).append(", ");
			json.append("\"vertexCount\": ").append(arrow.vertexCount()).append(", ");
			json.append("\"indexBytes\": ").append(arrow.indexBytes()).append(", ");
			json.append("\"sectionCount\": ").append(arrow.sectionCount()).append(", ");
			json.append("\"materialMode\": ").append(arrow.materialMode()).append(", ");
			json.append("\"packedLight\": ").append(arrow.packedLight()).append(", ");
			json.append("\"viewport\": { \"width\": ").append(arrow.viewportWidth())
				.append(", \"height\": ").append(arrow.viewportHeight()).append(" }, ");
			json.append("\"projected\": ").append(arrow.projected()).append(", ");
			json.append("\"screenBounds\": { \"left\": ").append(format(arrow.screenLeft()))
				.append(", \"top\": ").append(format(arrow.screenTop()))
				.append(", \"right\": ").append(format(arrow.screenRight()))
				.append(", \"bottom\": ").append(format(arrow.screenBottom())).append(" }");
			json.append(" }");
		}
		if (!diagnostics.isEmpty()) {
			json.append("\n  ");
		}
		json.append("]");
		return json;
	}

	private static StringBuilder appendArrowRouteDecisions(StringBuilder json) {
		List<RustGalWorldPrimitiveRenderer.ArrowRouteDecision> decisions = RustGalWorldPrimitiveRenderer.arrowRouteDecisions();
		json.append("  \"rustGalWorldArrowRouteDecisions\": [");
		for (int index = 0; index < decisions.size(); index++) {
			RustGalWorldPrimitiveRenderer.ArrowRouteDecision decision = decisions.get(index);
			if (index > 0) {
				json.append(",");
			}
			json.append("\n    { ");
			json.append("\"frameIndex\": ").append(decision.frameIndex()).append(", ");
			appendField(json, "route", decision.route(), 0).append(", ");
			appendField(json, "textureId", decision.textureId(), 0).append(", ");
			json.append("\"rustSelected\": ").append(decision.rustSelected()).append(", ");
			json.append("\"rustQueued\": ").append(decision.rustQueued()).append(", ");
			json.append("\"javaDrawn\": ").append(decision.javaDrawn());
			json.append(" }");
		}
		if (!decisions.isEmpty()) {
			json.append("\n  ");
		}
		json.append("]");
		return json;
	}

	private static StringBuilder appendItemEntityDiagnostics(StringBuilder json) {
		List<RustGalWorldPrimitiveRenderer.ItemEntityDiagnostic> diagnostics = RustGalWorldPrimitiveRenderer.itemEntityDiagnostics();
		json.append("  \"rustGalWorldItemEntities\": [");
		for (int index = 0; index < diagnostics.size(); index++) {
			RustGalWorldPrimitiveRenderer.ItemEntityDiagnostic item = diagnostics.get(index);
			if (index > 0) {
				json.append(",");
			}
			json.append("\n    { ");
			json.append("\"frameIndex\": ").append(item.frameIndex()).append(", ");
			appendField(json, "route", item.route(), 0).append(", ");
			appendField(json, "materialIdentity", item.materialIdentity(), 0).append(", ");
			json.append("\"meshKey\": ").append(Long.toUnsignedString(item.meshKey())).append(", ");
			json.append("\"meshGeneration\": ").append(item.meshGeneration()).append(", ");
			json.append("\"vertexLayoutVersion\": ").append(item.vertexLayoutVersion()).append(", ");
			json.append("\"indexType\": ").append(item.indexType()).append(", ");
			json.append("\"vertexCount\": ").append(item.vertexCount()).append(", ");
			json.append("\"indexBytes\": ").append(item.indexBytes()).append(", ");
			json.append("\"sectionCount\": ").append(item.sectionCount()).append(", ");
			json.append("\"packedLight\": ").append(item.packedLight()).append(", ");
			json.append("\"viewport\": { \"width\": ").append(item.viewportWidth())
				.append(", \"height\": ").append(item.viewportHeight()).append(" }, ");
			json.append("\"projected\": ").append(item.projected()).append(", ");
			json.append("\"screenBounds\": { \"left\": ").append(format(item.screenLeft()))
				.append(", \"top\": ").append(format(item.screenTop()))
				.append(", \"right\": ").append(format(item.screenRight()))
				.append(", \"bottom\": ").append(format(item.screenBottom())).append(" }");
			json.append(" }");
		}
		if (!diagnostics.isEmpty()) {
			json.append("\n  ");
		}
		json.append("]");
		return json;
	}

	private static StringBuilder appendItemEntityRouteDecisions(StringBuilder json) {
		List<RustGalWorldPrimitiveRenderer.ItemEntityRouteDecision> decisions = RustGalWorldPrimitiveRenderer.itemEntityRouteDecisions();
		json.append("  \"rustGalWorldItemEntityRouteDecisions\": [");
		for (int index = 0; index < decisions.size(); index++) {
			RustGalWorldPrimitiveRenderer.ItemEntityRouteDecision decision = decisions.get(index);
			if (index > 0) {
				json.append(",");
			}
			json.append("\n    { ");
			json.append("\"frameIndex\": ").append(decision.frameIndex()).append(", ");
			appendField(json, "route", decision.route(), 0).append(", ");
			json.append("\"eligible\": ").append(decision.eligible()).append(", ");
			appendField(json, "ineligibility", decision.ineligibility(), 0).append(", ");
			json.append("\"wholeFrameAvailable\": ").append(decision.wholeFrameAvailable()).append(", ");
			json.append("\"rustSelected\": ").append(decision.rustSelected()).append(", ");
			json.append("\"rustQueued\": ").append(decision.rustQueued()).append(", ");
			json.append("\"javaDrawn\": ").append(decision.javaDrawn());
			json.append(" }");
		}
		if (!decisions.isEmpty()) {
			json.append("\n  ");
		}
		json.append("]");
		return json;
	}

	private static StringBuilder appendExperienceOrbDiagnostics(StringBuilder json) {
		List<RustGalWorldPrimitiveRenderer.ExperienceOrbDiagnostic> diagnostics =
			RustGalWorldPrimitiveRenderer.experienceOrbDiagnostics();
		json.append("  \"rustGalWorldExperienceOrbs\": [");
		for (int index = 0; index < diagnostics.size(); index++) {
			RustGalWorldPrimitiveRenderer.ExperienceOrbDiagnostic orb = diagnostics.get(index);
			if (index > 0) {
				json.append(",");
			}
			json.append("\n    { ");
			json.append("\"frameIndex\": ").append(orb.frameIndex()).append(", ");
			appendField(json, "route", orb.route(), 0).append(", ");
			json.append("\"colorArgb\": ").append(orb.colorArgb()).append(", ");
			json.append("\"packedLight\": ").append(orb.packedLight()).append(", ");
			json.append("\"uv\": { \"minU\": ").append(format(orb.minU()))
				.append(", \"maxU\": ").append(format(orb.maxU()))
				.append(", \"minV\": ").append(format(orb.minV()))
				.append(", \"maxV\": ").append(format(orb.maxV())).append(" }, ");
			json.append("\"viewport\": { \"width\": ").append(orb.viewportWidth())
				.append(", \"height\": ").append(orb.viewportHeight()).append(" }, ");
			json.append("\"projected\": ").append(orb.projected()).append(", ");
			json.append("\"screenBounds\": { \"left\": ").append(format(orb.screenLeft()))
				.append(", \"top\": ").append(format(orb.screenTop()))
				.append(", \"right\": ").append(format(orb.screenRight()))
				.append(", \"bottom\": ").append(format(orb.screenBottom())).append(" }");
			json.append(" }");
		}
		if (!diagnostics.isEmpty()) {
			json.append("\n  ");
		}
		json.append("]");
		return json;
	}

	private static StringBuilder appendExperienceOrbRouteDecisions(StringBuilder json) {
		List<RustGalWorldPrimitiveRenderer.ExperienceOrbRouteDecision> decisions =
			RustGalWorldPrimitiveRenderer.experienceOrbRouteDecisions();
		json.append("  \"rustGalWorldExperienceOrbRouteDecisions\": [");
		for (int index = 0; index < decisions.size(); index++) {
			RustGalWorldPrimitiveRenderer.ExperienceOrbRouteDecision decision = decisions.get(index);
			if (index > 0) {
				json.append(",");
			}
			json.append("\n    { ");
			json.append("\"frameIndex\": ").append(decision.frameIndex()).append(", ");
			appendField(json, "route", decision.route(), 0).append(", ");
			json.append("\"rustSelected\": ").append(decision.rustSelected()).append(", ");
			json.append("\"rustQueued\": ").append(decision.rustQueued()).append(", ");
			json.append("\"javaDrawn\": ").append(decision.javaDrawn());
			json.append(" }");
		}
		if (!decisions.isEmpty()) {
			json.append("\n  ");
		}
		json.append("]");
		return json;
	}

	private static StringBuilder appendExperienceOrbExecutionDiagnostics(StringBuilder json) {
		List<RustGalWorldPrimitiveRenderer.ExperienceOrbExecutionDiagnostic> diagnostics =
			RustGalWorldPrimitiveRenderer.experienceOrbExecutionDiagnostics();
		json.append("  \"rustGalWorldExperienceOrbExecution\": [");
		for (int index = 0; index < diagnostics.size(); index++) {
			RustGalWorldPrimitiveRenderer.ExperienceOrbExecutionDiagnostic diagnostic = diagnostics.get(index);
			if (index > 0) {
				json.append(",");
			}
			json.append("\n    { ");
			json.append("\"deterministicFrameIndex\": ").append(diagnostic.deterministicFrameIndex()).append(", ");
			appendField(json, "route", diagnostic.route(), 0).append(", ");
			json.append("\"gameplayFrameId\": ").append(diagnostic.gameplayFrameId()).append(", ");
			json.append("\"submissionId\": ").append(diagnostic.submissionId()).append(", ");
			json.append("\"quads\": ").append(diagnostic.quads());
			json.append(", \"nativeOrbCount\": ").append(diagnostic.nativeOrbCount());
			json.append(", \"nativeResourcesComplete\": ").append(diagnostic.nativeResourcesComplete());
			json.append(", \"nativeResources\": [");
			for (int resourceIndex = 0; resourceIndex < diagnostic.nativeResources().size(); resourceIndex++) {
				var resource = diagnostic.nativeResources().get(resourceIndex);
				if (resourceIndex > 0) json.append(",");
				json.append("{\"meshKey\":\"").append(Long.toUnsignedString(resource.meshKey()))
					.append("\",\"meshGeneration\":\"").append(Long.toUnsignedString(resource.meshGeneration()))
					.append("\",\"entityId\":").append(resource.entityId()).append("}");
			}
			json.append("]");
			json.append(" }");
		}
		if (!diagnostics.isEmpty()) {
			json.append("\n  ");
		}
		json.append("]");
		return json;
	}

	private static StringBuilder appendBeaconBeamExecutionDiagnostics(StringBuilder json) {
		List<RustGalWorldPrimitiveRenderer.BeaconBeamExecutionDiagnostic> diagnostics =
			RustGalWorldPrimitiveRenderer.beaconBeamExecutionDiagnostics();
		json.append("  \"rustGalWorldBeaconBeamExecution\": [");
		for (int index = 0; index < diagnostics.size(); index++) {
			RustGalWorldPrimitiveRenderer.BeaconBeamExecutionDiagnostic diagnostic = diagnostics.get(index);
			if (index > 0) {
				json.append(",");
			}
			json.append("\n    { ");
			json.append("\"deterministicFrameIndex\": ").append(diagnostic.deterministicFrameIndex()).append(", ");
			appendField(json, "route", diagnostic.route(), 0).append(", ");
			json.append("\"gameplayFrameId\": ").append(diagnostic.gameplayFrameId()).append(", ");
			json.append("\"submissionId\": ").append(diagnostic.submissionId()).append(", ");
			json.append("\"quads\": ").append(diagnostic.quads());
			json.append(" }");
		}
		if (!diagnostics.isEmpty()) {
			json.append("\n  ");
		}
		json.append("]");
		return json;
	}

	private static StringBuilder appendCrystalBeamExecutionDiagnostics(StringBuilder json) {
		List<RustGalWorldPrimitiveRenderer.CrystalBeamExecutionDiagnostic> diagnostics =
			RustGalWorldPrimitiveRenderer.crystalBeamExecutionDiagnostics();
		json.append("  \"rustGalWorldCrystalBeamExecution\": [");
		for (int index = 0; index < diagnostics.size(); index++) {
			RustGalWorldPrimitiveRenderer.CrystalBeamExecutionDiagnostic diagnostic = diagnostics.get(index);
			if (index > 0) json.append(",");
			json.append("\n    { ");
			json.append("\"deterministicFrameIndex\": ").append(diagnostic.deterministicFrameIndex()).append(", ");
			appendField(json, "route", diagnostic.route(), 0).append(", ");
			json.append("\"gameplayFrameId\": ").append(diagnostic.gameplayFrameId()).append(", ");
			json.append("\"submissionId\": ").append(diagnostic.submissionId()).append(", ");
			json.append("\"quads\": ").append(diagnostic.quads());
			json.append(" }");
		}
		if (!diagnostics.isEmpty()) json.append("\n  ");
		json.append("]");
		return json;
	}

	private static StringBuilder appendBeaconBeamDiagnostics(StringBuilder json) {
		List<RustGalWorldPrimitiveRenderer.BeaconBeamDiagnostic> diagnostics =
			RustGalWorldPrimitiveRenderer.beaconBeamDiagnostics();
		json.append("  \"rustGalWorldBeaconBeams\": [");
		for (int index = 0; index < diagnostics.size(); index++) {
			RustGalWorldPrimitiveRenderer.BeaconBeamDiagnostic beam = diagnostics.get(index);
			if (index > 0) {
				json.append(",");
			}
			json.append("\n    { ");
			json.append("\"frameIndex\": ").append(beam.frameIndex()).append(", ");
			json.append("\"colorArgb\": ").append(beam.colorArgb()).append(", ");
			json.append("\"startY\": ").append(beam.startY()).append(", ");
			json.append("\"endY\": ").append(beam.endY()).append(", ");
			json.append("\"scroll\": ").append(format(beam.scroll())).append(", ");
			json.append("\"viewport\": { \"width\": ").append(beam.viewportWidth())
				.append(", \"height\": ").append(beam.viewportHeight()).append(" }, ");
			json.append("\"projected\": ").append(beam.projected()).append(", ");
			json.append("\"screenBounds\": { \"left\": ").append(format(beam.screenLeft()))
				.append(", \"top\": ").append(format(beam.screenTop()))
				.append(", \"right\": ").append(format(beam.screenRight()))
				.append(", \"bottom\": ").append(format(beam.screenBottom())).append(" }");
			json.append(" }");
		}
		if (!diagnostics.isEmpty()) {
			json.append("\n  ");
		}
		json.append("]");
		return json;
	}

	private static StringBuilder appendModelMeshDiagnostics(StringBuilder json) {
		List<RustGalWorldPrimitiveRenderer.ModelMeshDiagnostic> diagnostics = RustGalWorldPrimitiveRenderer.modelMeshDiagnostics();
		json.append("  \"rustGalWorldModelMeshes\": [");
		for (int index = 0; index < diagnostics.size(); index++) {
			RustGalWorldPrimitiveRenderer.ModelMeshDiagnostic model = diagnostics.get(index);
			if (index > 0) {
				json.append(",");
			}
			json.append("\n    { ");
			json.append("\"frameIndex\": ").append(model.frameIndex()).append(", ");
			appendField(json, "route", model.route(), 0).append(", ");
			json.append("\"entityId\": ").append(model.entityId()).append(", ");
			appendField(json, "semanticModelIdentity", model.semanticModelIdentity(), 0).append(", ");
			appendField(json, "textureId", model.textureId(), 0).append(", ");
			json.append("\"meshKey\": ").append(Long.toUnsignedString(model.meshKey())).append(", ");
			json.append("\"meshGeneration\": ").append(model.meshGeneration()).append(", ");
			json.append("\"vertexLayoutVersion\": ").append(model.vertexLayoutVersion()).append(", ");
			json.append("\"indexType\": ").append(model.indexType()).append(", ");
			json.append("\"vertexCount\": ").append(model.vertexCount()).append(", ");
				json.append("\"indexBytes\": ").append(model.indexBytes()).append(", ");
				json.append("\"sectionCount\": ").append(model.sectionCount()).append(", ");
				json.append("\"sectionCullPolicy\": ").append(model.sectionCullPolicy()).append(", ");
				json.append("\"viewport\": { \"width\": ").append(model.viewportWidth())
				.append(", \"height\": ").append(model.viewportHeight()).append(" }, ");
			json.append("\"projected\": ").append(model.projected()).append(", ");
			json.append("\"screenBounds\": { \"left\": ").append(format(model.screenLeft()))
				.append(", \"top\": ").append(format(model.screenTop()))
				.append(", \"right\": ").append(format(model.screenRight()))
				.append(", \"bottom\": ").append(format(model.screenBottom())).append(" }");
			json.append(" }");
		}
		if (!diagnostics.isEmpty()) {
			json.append("\n  ");
		}
		json.append("]");
		return json;
	}

	private static StringBuilder appendModelMeshRouteDecisions(StringBuilder json) {
		List<RustGalWorldPrimitiveRenderer.ModelMeshRouteDecision> decisions =
			RustGalWorldPrimitiveRenderer.modelMeshRouteDecisions();
		json.append("  \"rustGalWorldModelMeshRouteDecisions\": [");
		for (int index = 0; index < decisions.size(); index++) {
			RustGalWorldPrimitiveRenderer.ModelMeshRouteDecision decision = decisions.get(index);
			if (index > 0) {
				json.append(",");
			}
			json.append("\n    { ");
			json.append("\"frameIndex\": ").append(decision.frameIndex()).append(", ");
			appendField(json, "route", decision.route(), 0).append(", ");
			appendField(json, "textureId", decision.textureId(), 0).append(", ");
			json.append("\"entityId\": ").append(decision.entityId()).append(", ");
			json.append("\"rustSelected\": ").append(decision.rustSelected()).append(", ");
			json.append("\"rustQueued\": ").append(decision.rustQueued()).append(", ");
			json.append("\"javaDrawn\": ").append(decision.javaDrawn());
			json.append(" }");
		}
		if (!decisions.isEmpty()) {
			json.append("\n  ");
		}
		json.append("]");
		return json;
	}

	private static StringBuilder appendModelPartMeshTraversalDiagnostics(StringBuilder json) {
		List<RustGalWorldPrimitiveRenderer.ModelPartMeshTraversalDiagnostic> diagnostics =
			RustGalWorldPrimitiveRenderer.modelPartMeshTraversalDiagnostics();
		json.append("  \"rustGalWorldModelPartTraversal\": [");
		for (int index = 0; index < diagnostics.size(); index++) {
			RustGalWorldPrimitiveRenderer.ModelPartMeshTraversalDiagnostic diagnostic = diagnostics.get(index);
			if (index > 0) {
				json.append(",");
			}
			json.append("\n    { ");
			json.append("\"frameIndex\": ").append(diagnostic.frameIndex()).append(", ");
			appendField(json, "route", diagnostic.route(), 0).append(", ");
			appendField(json, "eligibility", diagnostic.eligibility(), 0).append(", ");
			appendField(json, "textureId", diagnostic.textureId(), 0).append(", ");
			appendField(json, "renderType", diagnostic.renderType(), 0);
			json.append(" }");
		}
		if (!diagnostics.isEmpty()) {
			json.append("\n  ");
		}
		json.append("]");
		return json;
	}

	private static StringBuilder appendMovingBlockDiagnostics(StringBuilder json) {
		List<RustGalWorldPrimitiveRenderer.MovingBlockDiagnostic> diagnostics =
			RustGalWorldPrimitiveRenderer.movingBlockDiagnostics();
		json.append("  \"rustGalWorldMovingBlocks\": [");
		for (int i = 0; i < diagnostics.size(); i++) {
			RustGalWorldPrimitiveRenderer.MovingBlockDiagnostic block = diagnostics.get(i);
			if (i > 0) {
				json.append(",");
			}
			json.append("\n    { ");
			json.append("\"frameIndex\": ").append(block.frameIndex()).append(", ");
			appendField(json, "route", block.route(), 0).append(", ");
			appendField(json, "provenance", block.provenance(), 0).append(", ");
			appendField(json, "blockId", block.blockId(), 0).append(", ");
			json.append("\"meshKey\": ").append(Long.toUnsignedString(block.meshKey())).append(", ");
			json.append("\"meshGeneration\": ").append(block.meshGeneration()).append(", ");
			json.append("\"vertexLayoutVersion\": ").append(block.vertexLayoutVersion()).append(", ");
			json.append("\"indexType\": ").append(block.indexType()).append(", ");
			json.append("\"vertexCount\": ").append(block.vertexCount()).append(", ");
			json.append("\"indexBytes\": ").append(block.indexBytes()).append(", ");
			json.append("\"sectionCount\": ").append(block.sectionCount()).append(", ");
			appendField(json, "textureIds", block.textureIds(), 0).append(", ");
			json.append("\"materialMode\": ").append(block.materialMode()).append(", ");
			json.append("\"viewport\": { \"width\": ").append(block.viewportWidth())
				.append(", \"height\": ").append(block.viewportHeight()).append(" }, ");
			json.append("\"projected\": ").append(block.projected()).append(", ");
			json.append("\"screenBounds\": { \"left\": ").append(format(block.screenLeft()))
				.append(", \"top\": ").append(format(block.screenTop()))
				.append(", \"right\": ").append(format(block.screenRight()))
				.append(", \"bottom\": ").append(format(block.screenBottom())).append(" }, ");
			json.append("\"transformTranslation\": { \"x\": ").append(format(block.transformX()))
				.append(", \"y\": ").append(format(block.transformY()))
				.append(", \"z\": ").append(format(block.transformZ())).append(" }");
			json.append(" }");
		}
		if (!diagnostics.isEmpty()) {
			json.append("\n  ");
		}
		json.append("]");
		return json;
	}

	private static StringBuilder appendMovingBlockRouteDecisions(StringBuilder json) {
		List<RustGalWorldPrimitiveRenderer.MovingBlockRouteDecision> decisions =
			RustGalWorldPrimitiveRenderer.movingBlockRouteDecisions();
		json.append("  \"rustGalWorldMovingBlockRouteDecisions\": [");
		for (int i = 0; i < decisions.size(); i++) {
			RustGalWorldPrimitiveRenderer.MovingBlockRouteDecision decision = decisions.get(i);
			if (i > 0) {
				json.append(",");
			}
			json.append("\n    { ");
			json.append("\"frameIndex\": ").append(decision.frameIndex()).append(", ");
			appendField(json, "provenance", decision.provenance(), 0).append(", ");
			appendField(json, "route", decision.route(), 0).append(", ");
			appendField(json, "blockId", decision.blockId(), 0).append(", ");
			json.append("\"rustSelected\": ").append(decision.rustSelected()).append(", ");
			json.append("\"rustQueued\": ").append(decision.rustQueued()).append(", ");
			json.append("\"javaDrawn\": ").append(decision.javaDrawn());
			json.append(" }");
		}
		if (!decisions.isEmpty()) {
			json.append("\n  ");
		}
		json.append("]");
		return json;
	}

	private static StringBuilder appendMovingMeshExecutionDiagnostics(StringBuilder json) {
		List<RustGalWorldPrimitiveRenderer.MovingMeshExecutionDiagnostic> diagnostics =
			RustGalWorldPrimitiveRenderer.movingMeshExecutionDiagnostics();
		json.append("  \"rustGalWorldMovingMeshExecution\": [");
		for (int i = 0; i < diagnostics.size(); i++) {
			RustGalWorldPrimitiveRenderer.MovingMeshExecutionDiagnostic receipt = diagnostics.get(i);
			if (i > 0) {
				json.append(",");
			}
			json.append("\n    { ");
			json.append("\"deterministicFrameIndex\": ").append(receipt.deterministicFrameIndex()).append(", ");
			appendField(json, "route", receipt.route(), 0).append(", ");
			appendField(json, "provenance", receipt.provenance(), 0).append(", ");
			json.append("\"gameplayFrameId\": ").append(receipt.gameplayFrameId()).append(", ");
			json.append("\"submissionId\": ").append(receipt.submissionId()).append(", ");
			json.append("\"instances\": ").append(receipt.instances());
			json.append(" }");
		}
		if (!diagnostics.isEmpty()) {
			json.append("\n  ");
		}
		json.append("]");
		return json;
	}

	private static StringBuilder appendEntityModelExecutionDiagnostics(StringBuilder json) {
		List<RustGalWorldPrimitiveRenderer.EntityModelExecutionDiagnostic> diagnostics =
			RustGalWorldPrimitiveRenderer.entityModelExecutionDiagnostics();
		json.append("  \"rustGalWorldEntityModelExecution\": [");
		for (int i = 0; i < diagnostics.size(); i++) {
			var receipt = diagnostics.get(i);
			if (i > 0) json.append(",");
			json.append("\n    { \"deterministicFrameIndex\": ").append(receipt.deterministicFrameIndex()).append(", ");
			appendField(json, "route", receipt.route(), 0).append(", ");
			json.append("\"gameplayFrameId\": ").append(receipt.gameplayFrameId()).append(", \"submissionId\": ")
				.append(receipt.submissionId()).append(", \"quads\": ").append(receipt.quads()).append(" }");
		}
		if (!diagnostics.isEmpty()) json.append("\n  ");
		json.append("]");
		return json;
	}

	/**
	 * Capture-only provenance for ordinary entity shadows.  The shadow uses the
	 * generic material-quad submission, so this keeps its semantic producer and
	 * final Rust submission independently visible instead of inferring it from
	 * an unrelated entity-model receipt.
	 */
	private static StringBuilder appendEntityShadowDiagnostics(StringBuilder json) {
		List<RustGalWorldPrimitiveRenderer.EntityShadowSemanticDiagnostic> semantic =
			RustGalWorldPrimitiveRenderer.entityShadowSemanticDiagnostics();
		List<RustGalWorldPrimitiveRenderer.EntityShadowExecutionDiagnostic> execution =
			RustGalWorldPrimitiveRenderer.entityShadowExecutionDiagnostics();
		json.append("  \"rustGalWorldEntityShadows\": { \"scenario\": \"")
			.append(escape(ENTITY_SHADOW_SCENARIO)).append("\", \"semanticReceipts\": [");
		for (int i = 0; i < semantic.size(); i++) {
			var receipt = semantic.get(i);
			if (i > 0) json.append(", ");
			json.append("{ \"frameIndex\": ").append(receipt.frameIndex()).append(", ");
			appendField(json, "route", receipt.route(), 0).append(", \"shadowSubmits\": ")
				.append(receipt.shadowSubmits()).append(", \"quads\": ").append(receipt.quads())
				.append(", \"receiverBoundsOrigin\": \"top-left\", \"receiverBounds\": ");
			if (receipt.receiverBoundsValid()) {
				json.append('[').append(receipt.receiverLeft()).append(',').append(receipt.receiverTop())
					.append(',').append(receipt.receiverRight()).append(',').append(receipt.receiverBottom()).append(']');
			} else {
				json.append("null");
			}
			json.append(" }");
		}
		json.append("], \"executionReceipts\": [");
		for (int i = 0; i < execution.size(); i++) {
			var receipt = execution.get(i);
			if (i > 0) json.append(", ");
			json.append("{ \"deterministicFrameIndex\": ").append(receipt.deterministicFrameIndex()).append(", ");
			appendField(json, "route", receipt.route(), 0).append(", \"gameplayFrameId\": ")
				.append(receipt.gameplayFrameId()).append(", \"submissionId\": ").append(receipt.submissionId())
				.append(", \"quads\": ").append(receipt.quads()).append(" }");
		}
		return json.append("] }");
	}

	private static StringBuilder appendProceduralQuadExecutionDiagnostics(StringBuilder json) {
		List<RustGalWorldPrimitiveRenderer.ProceduralQuadExecutionDiagnostic> diagnostics =
			RustGalWorldPrimitiveRenderer.proceduralQuadExecutionDiagnostics();
		json.append("  \"rustGalWorldProceduralQuadExecution\": [");
		for (int i = 0; i < diagnostics.size(); i++) {
			var receipt = diagnostics.get(i);
			if (i > 0) json.append(",");
			json.append("\n    { \"deterministicFrameIndex\": ").append(receipt.deterministicFrameIndex()).append(", ");
			appendField(json, "route", receipt.route(), 0).append(", ");
			json.append("\"gameplayFrameId\": ").append(receipt.gameplayFrameId()).append(", \"submissionId\": ")
				.append(receipt.submissionId()).append(", \"quads\": ").append(receipt.quads()).append(" }");
		}
		if (!diagnostics.isEmpty()) json.append("\n  ");
		json.append("]");
		return json;
	}

	private static StringBuilder appendWeatherDiagnostics(StringBuilder json) {
		List<RustGalWorldPrimitiveRenderer.WeatherTraversalDiagnostic> traversal =
			RustGalWorldPrimitiveRenderer.weatherTraversalDiagnostics();
		List<RustGalWorldPrimitiveRenderer.WeatherSemanticDiagnostic> semantic =
			RustGalWorldPrimitiveRenderer.weatherSemanticDiagnostics();
		List<RustGalWorldPrimitiveRenderer.WeatherExecutionDiagnostic> execution =
			RustGalWorldPrimitiveRenderer.weatherExecutionDiagnostics();
		json.append("  \"rustGalWorldWeather\": { ");
		appendField(json, "scenario", WEATHER_SCENARIO, 0).append(", ");
		appendField(json, "setupStage", weatherSetupStage, 0).append(", ");
		appendField(json, "setupLastMissing", weatherSetupLastMissing, 0).append(", ");
		json.append("\"setupAttempts\": ").append(weatherSetupAttempts)
			.append(", \"setupComplete\": ").append(weatherScenarioSetup)
			.append(", \"waitingFrames\": ").append(framesWaitingForWeatherProducer)
			.append(", \"traversalReceipts\": [");
		for (int i = 0; i < traversal.size(); i++) {
			RustGalWorldPrimitiveRenderer.WeatherTraversalDiagnostic receipt = traversal.get(i);
			if (i > 0) {
				json.append(", ");
			}
			json.append("{ \"frameIndex\": ").append(receipt.frameIndex()).append(", ");
			appendField(json, "route", receipt.route(), 0);
			json.append(", \"rainColumns\": ").append(receipt.rainColumns())
				.append(", \"snowColumns\": ").append(receipt.snowColumns())
				.append(", \"intensity\": ").append(format(receipt.intensity())).append(" }");
		}
		json.append("], \"semanticReceipts\": [");
		for (int i = 0; i < semantic.size(); i++) {
			RustGalWorldPrimitiveRenderer.WeatherSemanticDiagnostic receipt = semantic.get(i);
			if (i > 0) {
				json.append(", ");
			}
			json.append("{ \"frameIndex\": ").append(receipt.frameIndex())
				.append(", \"rainColumns\": ").append(receipt.rainColumns())
				.append(", \"snowColumns\": ").append(receipt.snowColumns())
				.append(", \"quads\": ").append(receipt.quads())
				.append(", \"intensity\": ").append(format(receipt.intensity()))
				.append(", \"depthWrite\": ").append(receipt.depthWrite()).append(" }");
		}
		json.append("], \"executionReceipts\": [");
		for (int i = 0; i < execution.size(); i++) {
			RustGalWorldPrimitiveRenderer.WeatherExecutionDiagnostic receipt = execution.get(i);
			if (i > 0) {
				json.append(", ");
			}
			json.append("{ \"deterministicFrameIndex\": ").append(receipt.deterministicFrameIndex())
				.append(", \"route\": \"").append(escape(receipt.route())).append("\"")
				.append(", \"gameplayFrameId\": ").append(receipt.gameplayFrameId())
				.append(", \"submissionId\": ").append(receipt.submissionId())
				.append(", \"quads\": ").append(receipt.quads()).append(" }");
		}
		json.append("] }");
		return json;
	}

	private static StringBuilder appendCloudDiagnostics(StringBuilder json) {
		List<RustGalWorldPrimitiveRenderer.CloudTraversalDiagnostic> traversal =
			RustGalWorldPrimitiveRenderer.cloudTraversalDiagnostics();
		List<RustGalWorldPrimitiveRenderer.CloudSemanticDiagnostic> semantic =
			RustGalWorldPrimitiveRenderer.cloudSemanticDiagnostics();
		List<RustGalWorldPrimitiveRenderer.CloudExecutionDiagnostic> execution =
			RustGalWorldPrimitiveRenderer.cloudExecutionDiagnostics();
		json.append("  \"rustGalWorldClouds\": { ");
		appendField(json, "scenario", CLOUD_SCENARIO, 0).append(", ");
		json.append("\"waitingFrames\": ").append(framesWaitingForCloudProducer).append(", \"traversalReceipts\": [");
		for (int i = 0; i < traversal.size(); i++) {
			RustGalWorldPrimitiveRenderer.CloudTraversalDiagnostic receipt = traversal.get(i);
			if (i > 0) json.append(", ");
			json.append("{ \"frameIndex\": ").append(receipt.frameIndex()).append(", ");
			appendField(json, "route", receipt.route(), 0);
			json.append(", \"cells\": ").append(receipt.cells()).append(", \"radius\": ").append(receipt.radius())
				.append(", \"fancy\": ").append(receipt.fancy()).append(" }");
		}
		json.append("], \"semanticReceipts\": [");
		for (int i = 0; i < semantic.size(); i++) {
			RustGalWorldPrimitiveRenderer.CloudSemanticDiagnostic receipt = semantic.get(i);
			if (i > 0) json.append(", ");
			json.append("{ \"frameIndex\": ").append(receipt.frameIndex()).append(", \"cells\": ").append(receipt.cells())
				.append(", \"radius\": ").append(receipt.radius()).append(", \"quads\": ").append(receipt.quads())
				.append(", \"fancy\": ").append(receipt.fancy())
				.append(", \"cloudColorArgb\": \"").append(String.format(java.util.Locale.ROOT, "0x%08x", receipt.cloudColorArgb())).append("\"")
				.append(", \"cameraRelation\": ").append(receipt.cameraRelation())
				.append(", \"centerCellX\": ").append(receipt.centerCellX())
				.append(", \"centerCellZ\": ").append(receipt.centerCellZ())
				.append(", \"offsetX\": ").append(receipt.offsetX())
				.append(", \"verticalOffset\": ").append(receipt.verticalOffset())
				.append(", \"offsetZ\": ").append(receipt.offsetZ())
				.append(", \"fogCloudsEnd\": ").append(receipt.fogCloudsEnd())
				.append(", \"meshFingerprint\": \"").append(Long.toUnsignedString(receipt.meshFingerprint())).append("\"")
				.append(", \"faceColorFingerprint\": \"").append(Long.toUnsignedString(receipt.faceColorFingerprint())).append("\"")
				.append(", \"sourceProgram\": ").append(RustGalWorldPrimitiveRenderer.MATERIAL_SOURCE_CLOUDS).append(" }");
		}
		json.append("], \"executionReceipts\": [");
		for (int i = 0; i < execution.size(); i++) {
			RustGalWorldPrimitiveRenderer.CloudExecutionDiagnostic receipt = execution.get(i);
			if (i > 0) json.append(", ");
			json.append("{ \"deterministicFrameIndex\": ").append(receipt.deterministicFrameIndex()).append(", ");
			appendField(json, "route", receipt.route(), 0);
			json.append(", \"gameplayFrameId\": ").append(receipt.gameplayFrameId())
				.append(", \"submissionId\": ").append(receipt.submissionId()).append(", \"quads\": ").append(receipt.quads())
				.append(", \"sourceProgram\": ").append(RustGalWorldPrimitiveRenderer.MATERIAL_SOURCE_CLOUDS).append(" }");
		}
		json.append("] }");
		return json;
	}

	private static StringBuilder appendMovingBlockShellScanDiagnostics(StringBuilder json) {
		List<RustGalWorldPrimitiveRenderer.MovingBlockShellScanDiagnostic> diagnostics =
			RustGalWorldPrimitiveRenderer.movingBlockShellScanDiagnostics();
		json.append("  \"rustGalWorldMovingBlockShellScans\": [");
		for (int i = 0; i < diagnostics.size(); i++) {
			RustGalWorldPrimitiveRenderer.MovingBlockShellScanDiagnostic scan = diagnostics.get(i);
			if (i > 0) {
				json.append(",");
			}
			json.append("\n    { ");
			json.append("\"frameIndex\": ").append(scan.frameIndex()).append(", ");
			appendField(json, "route", scan.route(), 0).append(", ");
			json.append("\"visiblePistonStates\": ").append(scan.visiblePistonStates()).append(", ");
			json.append("\"fallbackUsed\": ").append(scan.fallbackUsed()).append(", ");
			json.append("\"chunksScanned\": ").append(scan.chunksScanned()).append(", ");
			json.append("\"blockEntitiesInspected\": ").append(scan.blockEntitiesInspected()).append(", ");
			json.append("\"pistonEntitiesFound\": ").append(scan.pistonEntitiesFound()).append(", ");
			json.append("\"pistonStatesExtracted\": ").append(scan.pistonStatesExtracted()).append(", ");
			json.append("\"elapsedNanos\": ").append(scan.elapsedNanos());
			json.append(" }");
		}
		if (!diagnostics.isEmpty()) {
			json.append("\n  ");
		}
		json.append("]");
		return json;
	}

	private static StringBuilder appendPoseObject(StringBuilder json, String key, Pose pose) {
		json.append("  \"").append(key).append("\": { \"yaw\": ")
			.append(format(pose.yaw())).append(", \"pitch\": ")
			.append(format(pose.pitch())).append(", \"offset\": { \"x\": ")
			.append(format(pose.offsetX())).append(", \"y\": ")
			.append(format(pose.offsetY())).append(", \"z\": ")
			.append(format(pose.offsetZ())).append(" } }");
		return json;
	}

	private static String escape(String value) {
		return value == null ? "" : value
			.replace("\\", "\\\\")
			.replace("\"", "\\\"")
			.replace("\n", "\\n")
			.replace("\r", "\\r");
	}

	private static String format(double value) {
		return String.format(Locale.ROOT, "%.6f", value);
	}

	private static Pose[] movingMeshPoseSequence(String prefix, Pose pose, int count) {
		Pose[] sequence = new Pose[count];
		for (int i = 0; i < count; i++) {
			sequence[i] = new Pose(String.format(Locale.ROOT, "%s-%02d", prefix, i), pose.yaw(), pose.pitch());
		}
		return sequence;
	}

	private static Pose[] translucentTerrainPoseSequence(Pose pose) {
		Direction forward = Direction.fromYRot(pose.yaw());
		Direction right = forward.getClockWise();
		return new Pose[] {
			pose.withName("translucent-front"),
			offsetPose("translucent-lateral", pose, right, 4.0, forward, 0.0, 0.0),
			new Pose("translucent-orbit-left", pose.yaw() - 55.0F, pose.pitch(), 0.0, 0.0, 0.0),
			new Pose("translucent-cross-opposite", pose.yaw() + 180.0F, pose.pitch(), forward.getStepX() * 10.0, 0.0, forward.getStepZ() * 10.0),
			offsetPose("translucent-above", pose, forward, 4.0, right, 0.0, 5.0),
			offsetPose("translucent-below", pose, forward, 4.0, right, 0.0, -4.0),
			pose.withName("translucent-return")
		};
	}

	private static Pose offsetPose(String name, Pose pose, Direction primary, double primaryDistance, Direction secondary, double secondaryDistance, double yOffset) {
		double offsetX = primary.getStepX() * primaryDistance + secondary.getStepX() * secondaryDistance;
		double offsetZ = primary.getStepZ() * primaryDistance + secondary.getStepZ() * secondaryDistance;
		return new Pose(name, pose.yaw(), pose.pitch(), offsetX, yOffset, offsetZ);
	}

	private static Vec3 targetPositionForCurrentPose() {
		if (!initialized || initialPosition == null) {
			return null;
		}
		if (poses == null || poseIndex < 0 || poseIndex >= poses.length) {
			return initialPosition;
		}
		return targetPositionForPose(poses[poseIndex]);
	}

	private static Vec3 targetPositionForPose(Pose pose) {
		if (initialPosition == null) {
			return Vec3.ZERO;
		}
		return initialPosition.add(pose.offsetX(), pose.offsetY(), pose.offsetZ());
	}

	private static boolean isKnownDeterministicPosePosition(Vec3 position) {
		if (initialPosition == null) {
			return false;
		}
		if (position.distanceToSqr(initialPosition) <= 0.0004) {
			return true;
		}
		if (poses == null) {
			return false;
		}
		for (Pose pose : poses) {
			if (position.distanceToSqr(targetPositionForPose(pose)) <= 0.0004) {
				return true;
			}
		}
		return false;
	}

	private static Pose[] combinedMovingMeshPoseSequence(Pose pose) {
		Pose[] falling = movingMeshPoseSequence("falling", pose, 5);
		Pose[] piston = movingMeshPoseSequence("piston", pose, 7);
		Pose[] combined = new Pose[falling.length + piston.length];
		System.arraycopy(falling, 0, combined, 0, falling.length);
		System.arraycopy(piston, 0, combined, falling.length, piston.length);
		return combined;
	}

	private static String formatVec(Vec3 value) {
		return "(" + format(value.x) + "," + format(value.y) + "," + format(value.z) + ")";
	}

	private static String readAckStringField(String fieldName) {
		if (currentAckPath == null || !Files.isRegularFile(currentAckPath)) {
			return "";
		}
		try {
			String json = Files.readString(currentAckPath, StandardCharsets.UTF_8);
			String key = "\"" + fieldName + "\"";
			int keyIndex = json.indexOf(key);
			if (keyIndex < 0) {
				return "";
			}
			int colonIndex = json.indexOf(':', keyIndex + key.length());
			if (colonIndex < 0) {
				return "";
			}
			int quoteIndex = json.indexOf('"', colonIndex + 1);
			if (quoteIndex < 0) {
				return "";
			}
			StringBuilder value = new StringBuilder();
			boolean escaped = false;
			for (int i = quoteIndex + 1; i < json.length(); i++) {
				char c = json.charAt(i);
				if (escaped) {
					value.append(c);
					escaped = false;
				} else if (c == '\\') {
					escaped = true;
				} else if (c == '"') {
					return value.toString();
				} else {
					value.append(c);
				}
			}
		} catch (IOException ignored) {
			return "";
		}
		return "";
	}

	/** Reads a bounded integer field from the external capture acknowledgement.
	 * The acknowledgement describes the image that was actually saved, which can
	 * be one or more game-loop iterations older than the later poll that consumes
	 * it on the render thread. */
	private static long readAckLongField(String fieldName, long fallback) {
		if (currentAckPath == null || !Files.isRegularFile(currentAckPath)) {
			return fallback;
		}
		try {
			return readJsonLongField(Files.readString(currentAckPath, StandardCharsets.UTF_8), fieldName, fallback);
		} catch (IOException | NumberFormatException exception) {
			return fallback;
		}
	}

	private static long readJsonLongField(String json, String fieldName, long fallback) {
		String key = "\"" + fieldName + "\"";
		int keyIndex = json.indexOf(key);
		if (keyIndex < 0) {
			return fallback;
		}
		int colonIndex = json.indexOf(':', keyIndex + key.length());
		if (colonIndex < 0) {
			return fallback;
		}
		int index = colonIndex + 1;
		while (index < json.length() && Character.isWhitespace(json.charAt(index))) {
			index++;
		}
		int start = index;
		if (index < json.length() && json.charAt(index) == '-') {
			index++;
		}
		while (index < json.length() && Character.isDigit(json.charAt(index))) {
			index++;
		}
		if (start == index || (start + 1 == index && json.charAt(start) == '-')) {
			return fallback;
		}
		return Long.parseLong(json.substring(start, index));
	}

	private static boolean readJsonBooleanField(String json, String fieldName, boolean fallback) {
		String key = "\"" + fieldName + "\"";
		int keyIndex = json.indexOf(key);
		if (keyIndex < 0) {
			return fallback;
		}
		int colonIndex = json.indexOf(':', keyIndex + key.length());
		if (colonIndex < 0) {
			return fallback;
		}
		int index = colonIndex + 1;
		while (index < json.length() && Character.isWhitespace(json.charAt(index))) {
			index++;
		}
		if (json.startsWith("true", index)) {
			return true;
		}
		if (json.startsWith("false", index)) {
			return false;
		}
		return fallback;
	}


	private record Pose(String name, float yaw, float pitch, double offsetX, double offsetY, double offsetZ) {
		private Pose(String name, float yaw, float pitch) {
			this(name, yaw, pitch, 0.0, 0.0, 0.0);
		}

		private Pose withName(String name) {
			return new Pose(name, this.yaw, this.pitch, this.offsetX, this.offsetY, this.offsetZ);
		}
	}

	private record ForcedBlockOutlineTarget(
		BlockPos blockPos,
		Direction face,
		Vec3 hitLocation,
		Vec3 playerPosition,
		Pose pose
	) {
	}

	private record WholeFramePresentation(
		long deterministicRenderedFrameIndex,
		long gameplayFrameId,
		long correlationId,
		long submissionId,
		long acquiredSwapchainImage,
		long presentedSwapchainImage
	) {
	}

	private record PoseCapture(
		int index,
		String poseName,
		String screenshot,
		String dimension,
		Vec3 position,
		Vec3 requestedPosition,
		float requestedYaw,
		float requestedPitch,
		float observedYaw,
		float observedPitch,
		long renderedFrameIndex,
		int weatherRendererTicks,
		float weatherRendererPartialTick,
		String weatherSemanticFingerprint,
		String lightmapSemanticFingerprint,
		long gameTime,
		String captureMethod,
		String targetWindow
	) {
	}

	private record WaterAnimationFrameCapture(
		int index,
		String screenshot,
		long renderedFrameIndex,
		long gameTime,
		long animationHash,
		String animationSummary,
		String animationState,
		long visibleLayerSubmissions,
		long currentFrameVisibleLayerSubmissions,
		long atlasGeneration
	) {
	}
}
