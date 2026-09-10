package net.minecraft.client.gui;

import com.google.common.collect.ImmutableMap;
import com.google.common.collect.Ordering;
import net.blaze3d.platform.Window;
import java.util.ArrayList;
import java.util.Collection;
import java.util.Comparator;
import java.util.List;
import java.util.Map;
import java.util.function.Supplier;
import net.minecraft.api.EnvType;
import net.minecraft.api.Environment;
import net.minecraft.ChatFormatting;
import net.minecraft.hooks.GraphicsConfigHooks;
import net.minecraft.hooks.HookRegistry;
import net.minecraft.Optionull;
import net.minecraft.Util;
import net.minecraft.client.AttackIndicatorStatus;
import net.minecraft.client.DeltaTracker;
import net.minecraft.client.Minecraft;
import net.minecraft.client.Options;
import net.minecraft.client.dev.DeterministicCameraCapture;
import net.minecraft.client.gui.components.BossHealthOverlay;
import net.minecraft.client.gui.components.ChatComponent;
import net.minecraft.client.gui.components.DebugScreenOverlay;
import net.minecraft.client.gui.components.PlayerTabOverlay;
import net.minecraft.client.gui.components.SubtitleOverlay;
import net.minecraft.client.gui.components.debug.DebugScreenEntries;
import net.minecraft.client.gui.components.spectator.SpectatorGui;
import net.minecraft.client.gui.contextualbar.ContextualBarRenderer;
import net.minecraft.client.gui.contextualbar.ExperienceBarRenderer;
import net.minecraft.client.gui.contextualbar.JumpableVehicleBarRenderer;
import net.minecraft.client.gui.contextualbar.LocatorBarRenderer;
import net.minecraft.client.gui.screens.LevelLoadingScreen;
import net.minecraft.client.player.LocalPlayer;
import net.minecraft.client.renderer.LightTexture;
import net.minecraft.client.renderer.RenderPipelines;
import net.minecraft.client.renderer.texture.MissingTextureAtlasSprite;
import net.minecraft.client.renderer.texture.TextureAtlasSprite;
import net.minecraft.core.BlockPos;
import net.minecraft.core.Holder;
import net.minecraft.core.component.DataComponents;
import net.minecraft.network.chat.Component;
import net.minecraft.network.chat.MutableComponent;
import net.minecraft.network.chat.numbers.NumberFormat;
import net.minecraft.network.chat.numbers.StyledFormat;
import net.minecraft.resources.ResourceKey;
import net.minecraft.resources.ResourceLocation;
import net.minecraft.server.MinecraftServer;
import net.minecraft.sounds.SoundEvents;
import net.minecraft.tags.FluidTags;
import net.minecraft.util.ARGB;
import net.minecraft.util.Mth;
import net.minecraft.util.RandomSource;
import net.minecraft.util.StringUtil;
import net.minecraft.util.profiling.Profiler;
import net.minecraft.world.MenuProvider;
import net.minecraft.world.effect.MobEffect;
import net.minecraft.world.effect.MobEffectInstance;
import net.minecraft.world.effect.MobEffects;
import net.minecraft.world.entity.Entity;
import net.minecraft.world.entity.EquipmentSlot;
import net.minecraft.world.entity.HumanoidArm;
import net.minecraft.world.entity.LivingEntity;
import net.minecraft.world.entity.PlayerRideableJumping;
import net.minecraft.world.entity.ai.attributes.Attributes;
import net.minecraft.world.entity.player.Player;
import net.minecraft.world.food.FoodData;
import net.minecraft.world.item.ItemStack;
import net.minecraft.world.item.TaczMvpGunItem;
import net.minecraft.world.item.equipment.Equippable;
import net.minecraft.world.level.GameType;
import net.minecraft.world.level.Level;
import net.minecraft.world.level.block.Blocks;
import net.minecraft.world.level.border.WorldBorder;
import net.minecraft.world.phys.BlockHitResult;
import net.minecraft.world.phys.EntityHitResult;
import net.minecraft.world.phys.HitResult;
import net.minecraft.world.phys.HitResult.Type;
import net.minecraft.world.scores.DisplaySlot;
import net.minecraft.world.scores.Objective;
import net.minecraft.world.scores.PlayerScoreEntry;
import net.minecraft.world.scores.PlayerTeam;
import net.minecraft.world.scores.Scoreboard;
import net.voxelmap.VoxelConstants;
import net.vulkanic.gui.AbsorptionHeartRequest;
import net.vulkanic.gui.AbsorptionHeartVariant;
import net.vulkanic.gui.AirBubbleRequest;
import net.vulkanic.gui.AirBubbleState;
import net.vulkanic.gui.GuiHeartState;
import net.vulkanic.gui.HungerIconRequest;
import net.vulkanic.gui.HungerIconState;
import net.vulkanic.gui.HungerIconVariant;
import net.vulkanic.gui.MountHeartRequest;
import net.vulkanic.gui.MountHeartState;
import net.vulkanic.gui.MountHeartVariant;
import net.vulkanic.gui.PlayerHeartRequest;
import net.vulkanic.gui.PlayerHeartVariant;
import net.vulkanic.gui.RustGalGuiRenderer;
import org.apache.commons.lang3.tuple.Pair;
import org.jetbrains.annotations.Nullable;

@Environment(EnvType.CLIENT)
public class Gui {
	private static final ResourceLocation VOXELMAP_MINIMAP_HUD_ELEMENT = ResourceLocation.parse("voxelmap:minimap");
	private static final ResourceLocation CROSSHAIR_SPRITE = ResourceLocation.withDefaultNamespace("hud/crosshair");
	private static final ResourceLocation CROSSHAIR_ATTACK_INDICATOR_FULL_SPRITE = ResourceLocation.withDefaultNamespace("hud/crosshair_attack_indicator_full");
	private static final ResourceLocation CROSSHAIR_ATTACK_INDICATOR_BACKGROUND_SPRITE = ResourceLocation.withDefaultNamespace(
		"hud/crosshair_attack_indicator_background"
	);
	private static final ResourceLocation CROSSHAIR_ATTACK_INDICATOR_PROGRESS_SPRITE = ResourceLocation.withDefaultNamespace(
		"hud/crosshair_attack_indicator_progress"
	);
	private static final ResourceLocation EFFECT_BACKGROUND_AMBIENT_SPRITE = ResourceLocation.withDefaultNamespace("hud/effect_background_ambient");
	private static final ResourceLocation EFFECT_BACKGROUND_SPRITE = ResourceLocation.withDefaultNamespace("hud/effect_background");
	private static final ResourceLocation HOTBAR_SPRITE = ResourceLocation.withDefaultNamespace("hud/hotbar");
	private static final ResourceLocation HOTBAR_SELECTION_SPRITE = ResourceLocation.withDefaultNamespace("hud/hotbar_selection");
	private static final ResourceLocation HOTBAR_OFFHAND_LEFT_SPRITE = ResourceLocation.withDefaultNamespace("hud/hotbar_offhand_left");
	private static final ResourceLocation HOTBAR_OFFHAND_RIGHT_SPRITE = ResourceLocation.withDefaultNamespace("hud/hotbar_offhand_right");
	private static final ResourceLocation HOTBAR_ATTACK_INDICATOR_BACKGROUND_SPRITE = ResourceLocation.withDefaultNamespace(
		"hud/hotbar_attack_indicator_background"
	);
	private static final ResourceLocation HOTBAR_ATTACK_INDICATOR_PROGRESS_SPRITE = ResourceLocation.withDefaultNamespace("hud/hotbar_attack_indicator_progress");
	private static final ResourceLocation ARMOR_EMPTY_SPRITE = ResourceLocation.withDefaultNamespace("hud/armor_empty");
	private static final ResourceLocation ARMOR_HALF_SPRITE = ResourceLocation.withDefaultNamespace("hud/armor_half");
	private static final ResourceLocation ARMOR_FULL_SPRITE = ResourceLocation.withDefaultNamespace("hud/armor_full");
	private static final ResourceLocation FOOD_EMPTY_HUNGER_SPRITE = ResourceLocation.withDefaultNamespace("hud/food_empty_hunger");
	private static final ResourceLocation FOOD_HALF_HUNGER_SPRITE = ResourceLocation.withDefaultNamespace("hud/food_half_hunger");
	private static final ResourceLocation FOOD_FULL_HUNGER_SPRITE = ResourceLocation.withDefaultNamespace("hud/food_full_hunger");
	private static final ResourceLocation FOOD_EMPTY_SPRITE = ResourceLocation.withDefaultNamespace("hud/food_empty");
	private static final ResourceLocation FOOD_HALF_SPRITE = ResourceLocation.withDefaultNamespace("hud/food_half");
	private static final ResourceLocation FOOD_FULL_SPRITE = ResourceLocation.withDefaultNamespace("hud/food_full");
	private static final ResourceLocation AIR_SPRITE = ResourceLocation.withDefaultNamespace("hud/air");
	private static final ResourceLocation AIR_POPPING_SPRITE = ResourceLocation.withDefaultNamespace("hud/air_bursting");
	private static final ResourceLocation AIR_EMPTY_SPRITE = ResourceLocation.withDefaultNamespace("hud/air_empty");
	private static final ResourceLocation HEART_VEHICLE_CONTAINER_SPRITE = ResourceLocation.withDefaultNamespace("hud/heart/vehicle_container");
	private static final ResourceLocation HEART_VEHICLE_FULL_SPRITE = ResourceLocation.withDefaultNamespace("hud/heart/vehicle_full");
	private static final ResourceLocation HEART_VEHICLE_HALF_SPRITE = ResourceLocation.withDefaultNamespace("hud/heart/vehicle_half");
	private static final ResourceLocation VIGNETTE_LOCATION = ResourceLocation.withDefaultNamespace("textures/misc/vignette.png");
	public static final ResourceLocation NAUSEA_LOCATION = ResourceLocation.withDefaultNamespace("textures/misc/nausea.png");
	private static final ResourceLocation SPYGLASS_SCOPE_LOCATION = ResourceLocation.withDefaultNamespace("textures/misc/spyglass_scope.png");
	private static final ResourceLocation POWDER_SNOW_OUTLINE_LOCATION = ResourceLocation.withDefaultNamespace("textures/misc/powder_snow_outline.png");
	private static final Comparator<PlayerScoreEntry> SCORE_DISPLAY_ORDER = Comparator.comparing(PlayerScoreEntry::value)
		.reversed()
		.thenComparing(PlayerScoreEntry::owner, String.CASE_INSENSITIVE_ORDER);
	private static final Component DEMO_EXPIRED_TEXT = Component.translatable("demo.demoExpired");
	private static final Component SAVING_TEXT = Component.translatable("menu.savingLevel");
	private static final float MIN_CROSSHAIR_ATTACK_SPEED = 5.0F;
	private static final int EXPERIENCE_BAR_DISPLAY_TICKS = 100;
	private static final int NUM_HEARTS_PER_ROW = 10;
	private static final int LINE_HEIGHT = 10;
	private static final String SPACER = ": ";
	private static final float PORTAL_OVERLAY_ALPHA_MIN = 0.2F;
	private static final int HEART_SIZE = 9;
	private static final int HEART_SEPARATION = 8;
	private static final int NUM_AIR_BUBBLES = 10;
	private static final int AIR_BUBBLE_SIZE = 9;
	private static final int AIR_BUBBLE_SEPERATION = 8;
	private static final int AIR_BUBBLE_POPPING_DURATION = 2;
	private static final int EMPTY_AIR_BUBBLE_DELAY_DURATION = 1;
	private static final float AIR_BUBBLE_POP_SOUND_VOLUME_BASE = 0.5F;
	private static final float AIR_BUBBLE_POP_SOUND_VOLUME_INCREMENT = 0.1F;
	private static final float AIR_BUBBLE_POP_SOUND_PITCH_BASE = 1.0F;
	private static final float AIR_BUBBLE_POP_SOUND_PITCH_INCREMENT = 0.1F;
	private static final int NUM_AIR_BUBBLE_POPPED_BEFORE_SOUND_VOLUME_INCREASE = 3;
	private static final int NUM_AIR_BUBBLE_POPPED_BEFORE_SOUND_PITCH_INCREASE = 5;
	private static final float AUTOSAVE_FADE_SPEED_FACTOR = 0.2F;
	private static final int SAVING_INDICATOR_WIDTH_PADDING_RIGHT = 5;
	private static final int SAVING_INDICATOR_HEIGHT_PADDING_BOTTOM = 5;
	private final RandomSource random = RandomSource.create();
	private final Minecraft minecraft;
	private final ChatComponent chat;
	private int tickCount;
	@Nullable
	private Component overlayMessageString;
	private int overlayMessageTime;
	private boolean animateOverlayMessageColor;
	private boolean chatDisabledByPlayerShown;
	public float vignetteBrightness = 1.0F;
	private int toolHighlightTimer;
	private ItemStack lastToolHighlight = ItemStack.EMPTY;
	private final DebugScreenOverlay debugOverlay;
	private final SubtitleOverlay subtitleOverlay;
	private final SpectatorGui spectatorGui;
	private final PlayerTabOverlay tabList;
	private final BossHealthOverlay bossOverlay;
	private int titleTime;
	@Nullable
	private Component title;
	@Nullable
	private Component subtitle;
	private int titleFadeInTime;
	private int titleStayTime;
	private int titleFadeOutTime;
	private int lastHealth;
	private int displayHealth;
	private long lastHealthTime;
	private long healthBlinkTime;
	private int lastBubblePopSoundPlayed;
	@Nullable
	private Runnable deferredSubtitles;
	private float autosaveIndicatorValue;
	private float lastAutosaveIndicatorValue;
	private Pair<Gui.ContextualInfo, ContextualBarRenderer> contextualInfoBar = Pair.of(Gui.ContextualInfo.EMPTY, ContextualBarRenderer.EMPTY);
	private final Map<Gui.ContextualInfo, Supplier<ContextualBarRenderer>> contextualInfoBarRenderers;
	private float scopeScale;

	public Gui(Minecraft minecraft) {
		this.minecraft = minecraft;
		this.debugOverlay = new DebugScreenOverlay(minecraft);
		this.spectatorGui = new SpectatorGui(minecraft);
		this.chat = new ChatComponent(minecraft);
		this.tabList = new PlayerTabOverlay(minecraft, this);
		this.bossOverlay = new BossHealthOverlay(minecraft);
		this.subtitleOverlay = new SubtitleOverlay(minecraft);
		this.contextualInfoBarRenderers = ImmutableMap.of(
			Gui.ContextualInfo.EMPTY,
			() -> ContextualBarRenderer.EMPTY,
			Gui.ContextualInfo.EXPERIENCE,
			() -> new ExperienceBarRenderer(minecraft),
			Gui.ContextualInfo.LOCATOR,
			() -> new LocatorBarRenderer(minecraft),
			Gui.ContextualInfo.JUMPABLE_VEHICLE,
			() -> new JumpableVehicleBarRenderer(minecraft)
		);
		this.resetTitleTimes();
	}

	public void resetTitleTimes() {
		this.titleFadeInTime = 10;
		this.titleStayTime = 70;
		this.titleFadeOutTime = 20;
	}

	public void render(GuiGraphics guiGraphics, DeltaTracker deltaTracker) {
		// Iris HUD visibility is compatibility-renderer state. Rust whole-frame
		// Vulkan receives semantic GUI elements directly and must not query Iris
		// screen/runtime internals while assembling that frame.
		if (!net.vulkanic.bridge.RustGalVulkanWholeFrameMode.enabled()
			&& !net.vulkanic.VulkanicAPI.isVulkanBackendSelected()
			&& this.minecraft.screen instanceof net.irisshaders.iris.gui.screen.HudHideable) {
			return;
		}
		
		// GL debug groups are legacy Iris renderer state. Whole-frame Vulkan
		// submits semantic HUD commands directly to Rust and must not touch it.
		boolean legacyIrisDebugGroup = !net.vulkanic.bridge.RustGalVulkanWholeFrameMode.enabled()
			&& !net.vulkanic.VulkanicAPI.isVulkanBackendSelected();
		if (legacyIrisDebugGroup) {
			net.irisshaders.iris.gl.GLDebug.pushGroup(1000, "GUI");
		}
		
		if (!(this.minecraft.screen instanceof LevelLoadingScreen)) {
			if (!this.minecraft.options.hideGui) {
				this.renderCameraOverlays(guiGraphics, deltaTracker);
				this.renderCrosshair(guiGraphics, deltaTracker);
				guiGraphics.nextStratum();
				this.renderHotbarAndDecorations(guiGraphics, deltaTracker);
				this.renderEffects(guiGraphics, deltaTracker);
				this.renderBossOverlay(guiGraphics, deltaTracker);
			}

			this.renderSleepOverlay(guiGraphics, deltaTracker);
			if (!this.minecraft.options.hideGui) {
				this.renderDemoOverlay(guiGraphics, deltaTracker);
				this.renderScoreboardSidebar(guiGraphics, deltaTracker);
				this.renderOverlayMessage(guiGraphics, deltaTracker);
				this.renderTitle(guiGraphics, deltaTracker);
				this.renderChat(guiGraphics, deltaTracker);
				this.renderTabList(guiGraphics, deltaTracker);
				this.renderSubtitleOverlay(guiGraphics, this.minecraft.screen == null || this.minecraft.screen.isInGameUi());
			} else if (this.minecraft.screen != null && this.minecraft.screen.isInGameUi()) {
				this.renderSubtitleOverlay(guiGraphics, true);
			}
		}
		
		// VoxelMap: Render minimap overlay (after boss bar, before debug overlay)
		if (!this.minecraft.options.hideGui) {
			if (net.vulkanic.bridge.RustGalVulkanWholeFrameMode.enabled()
				|| net.vulkanic.VulkanicAPI.isVulkanBackendSelected()) {
				// The semantic producer owns fail-closed admission on Rust Vulkan;
				// swallowing its boundary error would make an unavailable overlay
				// indistinguishable from a Java fallback.
				if (net.fabricmc.fabric.api.client.rendering.v1.hud.HudElementRegistry.isRegistered(VOXELMAP_MINIMAP_HUD_ELEMENT)) {
					net.fabricmc.fabric.api.client.rendering.v1.hud.HudElementRegistry.renderAll(guiGraphics, deltaTracker);
				} else {
					// Some stripped test/client boots do not load VoxelMap's Fabric
					// initializer. Keep the semantic callsite live without reopening
					// the Java GPU overlay path.
					net.voxelmap.VoxelConstants.renderOverlay(guiGraphics);
				}
			} else {
				try {
					net.fabricmc.fabric.api.client.rendering.v1.hud.HudElementRegistry.renderAll(guiGraphics, deltaTracker);
				} catch (Exception e) {
					// Silently ignore errors to avoid crashing the game
				}
			}
		}
		
		if (legacyIrisDebugGroup) {
			net.irisshaders.iris.gl.GLDebug.popGroup();
		}
	}

	private void renderBossOverlay(GuiGraphics guiGraphics, DeltaTracker deltaTracker) {
		this.bossOverlay.render(guiGraphics);
	}

	public void renderDebugOverlay(GuiGraphics guiGraphics) {
		this.debugOverlay.render(guiGraphics);
	}

	private void renderSubtitleOverlay(GuiGraphics guiGraphics, boolean bl) {
		if (bl) {
			this.deferredSubtitles = () -> this.subtitleOverlay.render(guiGraphics);
		} else {
			this.deferredSubtitles = null;
			this.subtitleOverlay.render(guiGraphics);
		}
	}

	public void renderDeferredSubtitles() {
		if (this.deferredSubtitles != null) {
			this.deferredSubtitles.run();
			this.deferredSubtitles = null;
		}
	}

	private void renderCameraOverlays(GuiGraphics guiGraphics, DeltaTracker deltaTracker) {
		boolean useFancy = Minecraft.useFancyGraphics();
		
		// HOOK: Allow mods to override vignette enable setting
		for (GraphicsConfigHooks hook : HookRegistry.getGraphicsConfigHooks()) {
			Boolean override = hook.shouldEnableVignette(useFancy);
			if (override != null) {
				useFancy = override;
				break;
			}
		}
		
		if (useFancy) {
			this.renderVignette(guiGraphics, this.minecraft.getCameraEntity());
		}

		LocalPlayer localPlayer = this.minecraft.player;
		float f = deltaTracker.getGameTimeDeltaTicks();
		this.scopeScale = Mth.lerp(0.5F * f, this.scopeScale, 1.125F);
		if (this.minecraft.options.getCameraType().isFirstPerson()) {
			if (localPlayer.isScoping()) {
				this.renderSpyglassOverlay(guiGraphics, this.scopeScale);
			} else {
				this.scopeScale = 0.5F;

				for (EquipmentSlot equipmentSlot : EquipmentSlot.values()) {
					ItemStack itemStack = localPlayer.getItemBySlot(equipmentSlot);
					Equippable equippable = (Equippable)itemStack.get(DataComponents.EQUIPPABLE);
					if (equippable != null && equippable.slot() == equipmentSlot && equippable.cameraOverlay().isPresent()) {
						this.renderTextureOverlay(guiGraphics, ((ResourceLocation)equippable.cameraOverlay().get()).withPath(string -> "textures/" + string + ".png"), 1.0F);
					}
				}
			}
		}

		if (localPlayer.getTicksFrozen() > 0) {
			this.renderTextureOverlay(guiGraphics, POWDER_SNOW_OUTLINE_LOCATION, localPlayer.getPercentFrozen());
		}

		float g = deltaTracker.getGameTimeDeltaPartialTick(false);
		float h = Mth.lerp(g, localPlayer.oPortalEffectIntensity, localPlayer.portalEffectIntensity);
		float i = localPlayer.getEffectBlendFactor(MobEffects.NAUSEA, g);
		if (h > 0.0F) {
			this.renderPortalOverlay(guiGraphics, h);
		} else if (i > 0.0F) {
			float j = this.minecraft.options.screenEffectScale().get().floatValue();
			if (j < 1.0F) {
				float k = i * (1.0F - j);
				this.renderConfusionOverlay(guiGraphics, k);
			}
		}
	}

	private void renderSleepOverlay(GuiGraphics guiGraphics, DeltaTracker deltaTracker) {
		if (this.minecraft.player.getSleepTimer() > 0) {
			Profiler.get().push("sleep");
			guiGraphics.nextStratum();
			float f = this.minecraft.player.getSleepTimer();
			float g = f / 100.0F;
			if (g > 1.0F) {
				g = 1.0F - (f - 100.0F) / 10.0F;
			}

			int i = (int)(220.0F * g) << 24 | 1052704;
			guiGraphics.fill(0, 0, guiGraphics.guiWidth(), guiGraphics.guiHeight(), i);
			Profiler.get().pop();
		}
	}

	private void renderOverlayMessage(GuiGraphics guiGraphics, DeltaTracker deltaTracker) {
		Font font = this.getFont();
		if (this.overlayMessageString != null && this.overlayMessageTime > 0) {
			Profiler.get().push("overlayMessage");
			float f = this.overlayMessageTime - deltaTracker.getGameTimeDeltaPartialTick(false);
			int i = (int)(f * 255.0F / 20.0F);
			if (i > 255) {
				i = 255;
			}

			if (i > 0) {
				guiGraphics.nextStratum();
				guiGraphics.pose().pushMatrix();
				guiGraphics.pose().translate(guiGraphics.guiWidth() / 2, guiGraphics.guiHeight() - 68);
				int j;
				if (this.animateOverlayMessageColor) {
					j = Mth.hsvToArgb(f / 50.0F, 0.7F, 0.6F, i);
				} else {
					j = ARGB.color(i, -1);
				}

				int k = font.width(this.overlayMessageString);
				guiGraphics.drawStringWithBackdrop(font, this.overlayMessageString, -k / 2, -4, k, j);
				guiGraphics.pose().popMatrix();
			}

			Profiler.get().pop();
		}
	}

	private void renderTitle(GuiGraphics guiGraphics, DeltaTracker deltaTracker) {
		if (this.title != null && this.titleTime > 0) {
			Font font = this.getFont();
			Profiler.get().push("titleAndSubtitle");
			float f = this.titleTime - deltaTracker.getGameTimeDeltaPartialTick(false);
			int i = 255;
			if (this.titleTime > this.titleFadeOutTime + this.titleStayTime) {
				float g = this.titleFadeInTime + this.titleStayTime + this.titleFadeOutTime - f;
				i = (int)(g * 255.0F / this.titleFadeInTime);
			}

			if (this.titleTime <= this.titleFadeOutTime) {
				i = (int)(f * 255.0F / this.titleFadeOutTime);
			}

			i = Mth.clamp(i, 0, 255);
			if (i > 0) {
				guiGraphics.nextStratum();
				guiGraphics.pose().pushMatrix();
				guiGraphics.pose().translate(guiGraphics.guiWidth() / 2, guiGraphics.guiHeight() / 2);
				guiGraphics.pose().pushMatrix();
				guiGraphics.pose().scale(4.0F, 4.0F);
				int j = font.width(this.title);
				int k = ARGB.color(i, -1);
				guiGraphics.drawStringWithBackdrop(font, this.title, -j / 2, -10, j, k);
				guiGraphics.pose().popMatrix();
				if (this.subtitle != null) {
					guiGraphics.pose().pushMatrix();
					guiGraphics.pose().scale(2.0F, 2.0F);
					int l = font.width(this.subtitle);
					guiGraphics.drawStringWithBackdrop(font, this.subtitle, -l / 2, 5, l, k);
					guiGraphics.pose().popMatrix();
				}

				guiGraphics.pose().popMatrix();
			}

			Profiler.get().pop();
		}
	}

	private void renderChat(GuiGraphics guiGraphics, DeltaTracker deltaTracker) {
		if (!this.chat.isChatFocused()) {
			Window window = this.minecraft.getWindow();
			int i = Mth.floor(this.minecraft.mouseHandler.getScaledXPos(window));
			int j = Mth.floor(this.minecraft.mouseHandler.getScaledYPos(window));
			guiGraphics.nextStratum();
			this.chat.render(guiGraphics, this.tickCount, i, j, false);
		}
	}

	private void renderScoreboardSidebar(GuiGraphics guiGraphics, DeltaTracker deltaTracker) {
		Scoreboard scoreboard = this.minecraft.level.getScoreboard();
		Objective objective = null;
		PlayerTeam playerTeam = scoreboard.getPlayersTeam(this.minecraft.player.getScoreboardName());
		if (playerTeam != null) {
			DisplaySlot displaySlot = DisplaySlot.teamColorToSlot(playerTeam.getColor());
			if (displaySlot != null) {
				objective = scoreboard.getDisplayObjective(displaySlot);
			}
		}

		Objective objective2 = objective != null ? objective : scoreboard.getDisplayObjective(DisplaySlot.SIDEBAR);
		if (objective2 != null) {
			guiGraphics.nextStratum();
			this.displayScoreboardSidebar(guiGraphics, objective2);
		}
	}

	private void renderTabList(GuiGraphics guiGraphics, DeltaTracker deltaTracker) {
		Scoreboard scoreboard = this.minecraft.level.getScoreboard();
		Objective objective = scoreboard.getDisplayObjective(DisplaySlot.LIST);
		if (!this.minecraft.options.keyPlayerList.isDown()
			|| this.minecraft.isLocalServer() && this.minecraft.player.connection.getListedOnlinePlayers().size() <= 1 && objective == null) {
			this.tabList.setVisible(false);
		} else {
			this.tabList.setVisible(true);
			guiGraphics.nextStratum();
			this.tabList.render(guiGraphics, guiGraphics.guiWidth(), scoreboard, objective);
		}
	}

	private void renderCrosshair(GuiGraphics guiGraphics, DeltaTracker deltaTracker) {
		Options options = this.minecraft.options;
		if (this.isHoldingTaczGun()) {
			return;
		}

		if (options.getCameraType().isFirstPerson()) {
			if (this.minecraft.gameMode.getPlayerMode() != GameType.SPECTATOR || this.canRenderCrosshairForSpectator(this.minecraft.hitResult)) {
				if (!this.minecraft.debugEntries.isCurrentlyEnabled(DebugScreenEntries.THREE_DIMENSIONAL_CROSSHAIR)) {
					guiGraphics.nextStratum();
					int i = 15;
					if (RustGalGuiRenderer.isMigratedGuiDisabledForDiagnostics()) {
						// Dev-only measurement control: no migrated or legacy crosshair draw.
					} else if (RustGalGuiRenderer.shouldDrawJavaCompatibilityGui()) {
						guiGraphics.blitSprite(RenderPipelines.CROSSHAIR, CROSSHAIR_SPRITE, (guiGraphics.guiWidth() - i) / 2, (guiGraphics.guiHeight() - i) / 2, i, i);
					} else {
						RustGalGuiRenderer.enqueueCrosshair(this.minecraft, guiGraphics, (guiGraphics.guiWidth() - i) / 2, (guiGraphics.guiHeight() - i) / 2, i, i);
					}
						if (this.minecraft.options.attackIndicator().get() == AttackIndicatorStatus.CROSSHAIR) {
							DeterministicCameraCapture.forceCrosshairAttackTargetForDiagnostics(this.minecraft);
							float f = this.minecraft.player.getAttackStrengthScale(0.0F);
							boolean bl = false;
						if (this.minecraft.crosshairPickEntity != null && this.minecraft.crosshairPickEntity instanceof LivingEntity && f >= 1.0F) {
							bl = this.minecraft.player.getCurrentItemAttackStrengthDelay() > 5.0F;
							bl &= this.minecraft.crosshairPickEntity.isAlive();
						}

							int j = guiGraphics.guiHeight() / 2 - 7 + 16;
							int k = guiGraphics.guiWidth() / 2 - 8;
							if (bl) {
								if (RustGalGuiRenderer.isMigratedGuiDisabledForDiagnostics()) {
									// Dev-only measurement control: no migrated or legacy attack indicator draw.
								} else if (RustGalGuiRenderer.shouldDrawJavaCompatibilityGui()) {
									guiGraphics.blitSprite(RenderPipelines.CROSSHAIR, CROSSHAIR_ATTACK_INDICATOR_FULL_SPRITE, k, j, 16, 16);
								} else {
									RustGalGuiRenderer.enqueueCrosshairAttackIndicator(this.minecraft, guiGraphics, k, j, f, 16, true);
								}
							} else if (f < 1.0F) {
								int l = crosshairAttackIndicatorFilledWidth(f);
								if (RustGalGuiRenderer.isMigratedGuiDisabledForDiagnostics()) {
									// Dev-only measurement control: no migrated or legacy attack indicator draw.
								} else if (RustGalGuiRenderer.shouldDrawJavaCompatibilityGui()) {
									guiGraphics.blitSprite(RenderPipelines.CROSSHAIR, CROSSHAIR_ATTACK_INDICATOR_BACKGROUND_SPRITE, k, j, 16, 4);
									guiGraphics.blitSprite(RenderPipelines.CROSSHAIR, CROSSHAIR_ATTACK_INDICATOR_PROGRESS_SPRITE, 16, 4, 0, 0, k, j, l, 4);
								} else {
									RustGalGuiRenderer.enqueueCrosshairAttackIndicator(this.minecraft, guiGraphics, k, j, f, l, false);
								}
							}
						}
				}
			}
		}
	}

	private boolean isHoldingTaczGun() {
		LocalPlayer player = this.minecraft.player;
		return player != null
			&& (player.getMainHandItem().getItem() instanceof TaczMvpGunItem || player.getOffhandItem().getItem() instanceof TaczMvpGunItem);
	}

	private boolean canRenderCrosshairForSpectator(@Nullable HitResult hitResult) {
		if (hitResult == null) {
			return false;
		} else if (hitResult.getType() == Type.ENTITY) {
			return ((EntityHitResult)hitResult).getEntity() instanceof MenuProvider;
		} else if (hitResult.getType() == Type.BLOCK) {
			BlockPos blockPos = ((BlockHitResult)hitResult).getBlockPos();
			Level level = this.minecraft.level;
			return level.getBlockState(blockPos).getMenuProvider(level, blockPos) != null;
		} else {
			return false;
		}
	}

	private void renderEffects(GuiGraphics guiGraphics, DeltaTracker deltaTracker) {
		Collection<MobEffectInstance> collection = this.minecraft.player.getActiveEffects();
		if (!collection.isEmpty() && (this.minecraft.screen == null || !this.minecraft.screen.showsActiveEffects())) {
			int i = 0;
			int j = 0;

			for (MobEffectInstance mobEffectInstance : Ordering.natural().reverse().sortedCopy(collection)) {
				Holder<MobEffect> holder = mobEffectInstance.getEffect();
				if (mobEffectInstance.showIcon()) {
					int k = guiGraphics.guiWidth();
					int l = 1;
					if (this.minecraft.isDemo()) {
						l += 15;
					}

					if (((MobEffect)holder.value()).isBeneficial()) {
						i++;
						k -= 25 * i;
					} else {
						j++;
						k -= 25 * j;
						l += 26;
					}

					float f = 1.0F;
					if (mobEffectInstance.isAmbient()) {
						guiGraphics.blitSprite(RenderPipelines.GUI_TEXTURED, EFFECT_BACKGROUND_AMBIENT_SPRITE, k, l, 24, 24);
					} else {
						guiGraphics.blitSprite(RenderPipelines.GUI_TEXTURED, EFFECT_BACKGROUND_SPRITE, k, l, 24, 24);
						if (mobEffectInstance.endsWithin(200)) {
							int m = mobEffectInstance.getDuration();
							int n = 10 - m / 20;
							f = Mth.clamp(m / 10.0F / 5.0F * 0.5F, 0.0F, 0.5F) + Mth.cos(m * (float) Math.PI / 5.0F) * Mth.clamp(n / 10.0F * 0.25F, 0.0F, 0.25F);
							f = Mth.clamp(f, 0.0F, 1.0F);
						}
					}

					guiGraphics.blitSprite(RenderPipelines.GUI_TEXTURED, getMobEffectSprite(holder), k + 3, l + 3, 18, 18, ARGB.white(f));
				}
			}
		}
	}

	public static ResourceLocation getMobEffectSprite(Holder<MobEffect> holder) {
		return (ResourceLocation)holder.unwrapKey()
			.map(ResourceKey::location)
			.map(resourceLocation -> resourceLocation.withPrefix("mob_effect/"))
			.orElseGet(MissingTextureAtlasSprite::getLocation);
	}

	private void renderHotbarAndDecorations(GuiGraphics guiGraphics, DeltaTracker deltaTracker) {
		if (this.minecraft.gameMode.getPlayerMode() == GameType.SPECTATOR) {
			this.spectatorGui.renderHotbar(guiGraphics);
		} else {
			this.renderItemHotbar(guiGraphics, deltaTracker);
		}

		if (this.minecraft.gameMode.canHurtPlayer()) {
			this.renderPlayerHealth(guiGraphics);
		}

		this.renderVehicleHealth(guiGraphics);
		Gui.ContextualInfo contextualInfo = this.nextContextualInfoState();
		if (contextualInfo != this.contextualInfoBar.getKey()) {
			this.contextualInfoBar = Pair.of(contextualInfo, (ContextualBarRenderer)((Supplier)this.contextualInfoBarRenderers.get(contextualInfo)).get());
		}

		this.contextualInfoBar.getValue().renderBackground(guiGraphics, deltaTracker);
		if (this.minecraft.gameMode.hasExperience() && this.minecraft.player.experienceLevel > 0) {
			ContextualBarRenderer.renderExperienceLevel(guiGraphics, this.minecraft.font, this.minecraft.player.experienceLevel);
		}

		this.contextualInfoBar.getValue().render(guiGraphics, deltaTracker);
		if (this.minecraft.gameMode.getPlayerMode() != GameType.SPECTATOR) {
			this.renderSelectedItemName(guiGraphics);
		} else if (this.minecraft.player.isSpectator()) {
			this.spectatorGui.renderAction(guiGraphics);
		}
	}

	private void renderItemHotbar(GuiGraphics guiGraphics, DeltaTracker deltaTracker) {
		Player player = this.getCameraPlayer();
		if (player != null) {
			ItemStack itemStack = player.getOffhandItem();
			HumanoidArm humanoidArm = player.getMainArm().getOpposite();
			int i = guiGraphics.guiWidth() / 2;
			int j = 182;
			int k = 91;
			if (RustGalGuiRenderer.isMigratedGuiDisabledForDiagnostics()) {
				// Dev-only measurement control: no migrated or legacy hotbar base draw.
			} else if (RustGalGuiRenderer.shouldDrawJavaCompatibilityGui()) {
				guiGraphics.blitSprite(RenderPipelines.GUI_TEXTURED, HOTBAR_SPRITE, i - 91, guiGraphics.guiHeight() - 22, 182, 22);
			} else {
				RustGalGuiRenderer.enqueueHotbarBase(this.minecraft, guiGraphics, i - 91, guiGraphics.guiHeight() - 22, 182, 22);
			}
			int selectedSlot = player.getInventory().getSelectedSlot();
			int selectedSlotX = selectedHotbarHighlightX(guiGraphics.guiWidth(), selectedSlot);
			int selectedSlotY = selectedHotbarHighlightY(guiGraphics.guiHeight());
			if (RustGalGuiRenderer.isMigratedGuiDisabledForDiagnostics()) {
				// Dev-only measurement control: no migrated or legacy selected-slot highlight draw.
			} else if (RustGalGuiRenderer.shouldDrawJavaCompatibilityGui()) {
				guiGraphics.blitSprite(RenderPipelines.GUI_TEXTURED, HOTBAR_SELECTION_SPRITE, selectedSlotX, selectedSlotY, 24, 23);
			} else {
				RustGalGuiRenderer.enqueueHotbarSelection(this.minecraft, guiGraphics, selectedSlot, selectedSlotX, selectedSlotY, 24, 23);
			}
			if (!itemStack.isEmpty()) {
				if (humanoidArm == HumanoidArm.LEFT) {
					guiGraphics.blitSprite(RenderPipelines.GUI_TEXTURED, HOTBAR_OFFHAND_LEFT_SPRITE, i - 91 - 29, guiGraphics.guiHeight() - 23, 29, 24);
				} else {
					guiGraphics.blitSprite(RenderPipelines.GUI_TEXTURED, HOTBAR_OFFHAND_RIGHT_SPRITE, i + 91, guiGraphics.guiHeight() - 23, 29, 24);
				}
			}

			int l = 1;

			for (int m = 0; m < 9; m++) {
				int n = i - 90 + m * 20 + 2;
				int o = guiGraphics.guiHeight() - 16 - 3;
				this.renderSlot(guiGraphics, n, o, deltaTracker, player, player.getInventory().getItem(m), l++);
			}

			if (!itemStack.isEmpty()) {
				int m = guiGraphics.guiHeight() - 16 - 3;
				if (humanoidArm == HumanoidArm.LEFT) {
					this.renderSlot(guiGraphics, i - 91 - 26, m, deltaTracker, player, itemStack, l++);
				} else {
					this.renderSlot(guiGraphics, i + 91 + 10, m, deltaTracker, player, itemStack, l++);
				}
			}

			if (this.minecraft.options.attackIndicator().get() == AttackIndicatorStatus.HOTBAR) {
				float f = this.minecraft.player.getAttackStrengthScale(0.0F);
				if (f < 1.0F) {
					int n = guiGraphics.guiHeight() - 20;
					int o = i + 91 + 6;
					if (humanoidArm == HumanoidArm.RIGHT) {
						o = i - 91 - 22;
					}

					int p = hotbarAttackIndicatorFilledHeight(f);
					if (RustGalGuiRenderer.isMigratedGuiDisabledForDiagnostics()) {
						// Dev-only measurement control: no migrated or legacy attack indicator draw.
					} else if (RustGalGuiRenderer.shouldDrawJavaCompatibilityGui()) {
						guiGraphics.blitSprite(RenderPipelines.GUI_TEXTURED, HOTBAR_ATTACK_INDICATOR_BACKGROUND_SPRITE, o, n, 18, 18);
						guiGraphics.blitSprite(RenderPipelines.GUI_TEXTURED, HOTBAR_ATTACK_INDICATOR_PROGRESS_SPRITE, 18, 18, 0, 18 - p, o, n + 18 - p, 18, p);
					} else {
						RustGalGuiRenderer.enqueueHotbarAttackIndicator(this.minecraft, guiGraphics, o, n, f, p);
					}
				}
			}
		}
	}

	public static int selectedHotbarHighlightX(int guiWidth, int selectedSlot) {
		return guiWidth / 2 - 91 - 1 + selectedSlot * 20;
	}

	public static int selectedHotbarHighlightY(int guiHeight) {
		return guiHeight - 22 - 1;
	}

	public static int crosshairAttackIndicatorFilledWidth(float progress) {
		return (int)(progress * 17.0F);
	}

	public static int hotbarAttackIndicatorFilledHeight(float progress) {
		return (int)(progress * 19.0F);
	}

	private void renderSelectedItemName(GuiGraphics guiGraphics) {
		Profiler.get().push("selectedItemName");
		if (this.toolHighlightTimer > 0 && !this.lastToolHighlight.isEmpty()) {
			MutableComponent mutableComponent = Component.empty().append(this.lastToolHighlight.getHoverName()).withStyle(this.lastToolHighlight.getRarity().color());
			if (this.lastToolHighlight.has(DataComponents.CUSTOM_NAME)) {
				mutableComponent.withStyle(ChatFormatting.ITALIC);
			}

			int i = this.getFont().width(mutableComponent);
			int j = (guiGraphics.guiWidth() - i) / 2;
			int k = guiGraphics.guiHeight() - 59;
			if (!this.minecraft.gameMode.canHurtPlayer()) {
				k += 14;
			}

			int l = (int)(this.toolHighlightTimer * 256.0F / 10.0F);
			if (l > 255) {
				l = 255;
			}

			if (l > 0) {
				guiGraphics.drawStringWithBackdrop(this.getFont(), mutableComponent, j, k, i, ARGB.color(l, -1));
			}
		}

		Profiler.get().pop();
	}

	private void renderDemoOverlay(GuiGraphics guiGraphics, DeltaTracker deltaTracker) {
		if (this.minecraft.isDemo()) {
			Profiler.get().push("demo");
			guiGraphics.nextStratum();
			Component component;
			if (this.minecraft.level.getGameTime() >= 120500L) {
				component = DEMO_EXPIRED_TEXT;
			} else {
				component = Component.translatable(
					"demo.remainingTime",
					new Object[]{StringUtil.formatTickDuration((int)(120500L - this.minecraft.level.getGameTime()), this.minecraft.level.tickRateManager().tickrate())}
				);
			}

			int i = this.getFont().width(component);
			int j = guiGraphics.guiWidth() - i - 10;
			int k = 5;
			guiGraphics.drawStringWithBackdrop(this.getFont(), component, j, 5, i, -1);
			Profiler.get().pop();
		}
	}

	private void displayScoreboardSidebar(GuiGraphics guiGraphics, Objective objective) {
		Scoreboard scoreboard = objective.getScoreboard();
		NumberFormat numberFormat = objective.numberFormatOrDefault(StyledFormat.SIDEBAR_DEFAULT);

		@Environment(EnvType.CLIENT)
		record DisplayEntry(Component name, Component score, int scoreWidth) {
		}

		DisplayEntry[] lvs = (DisplayEntry[])scoreboard.listPlayerScores(objective)
			.stream()
			.filter(playerScoreEntry -> !playerScoreEntry.isHidden())
			.sorted(SCORE_DISPLAY_ORDER)
			.limit(15L)
			.map(playerScoreEntry -> {
				PlayerTeam playerTeam = scoreboard.getPlayersTeam(playerScoreEntry.owner());
				Component componentx = playerScoreEntry.ownerName();
				Component component2 = PlayerTeam.formatNameForTeam(playerTeam, componentx);
				Component component3 = playerScoreEntry.formatValue(numberFormat);
				int ix = this.getFont().width(component3);
				return new DisplayEntry(component2, component3, ix);
			})
			.toArray(DisplayEntry[]::new);
		Component component = objective.getDisplayName();
		int i = this.getFont().width(component);
		int j = i;
		int k = this.getFont().width(": ");

		for (DisplayEntry lv : lvs) {
			j = Math.max(j, this.getFont().width(lv.name) + (lv.scoreWidth > 0 ? k + lv.scoreWidth : 0));
		}

		int m = lvs.length;
		int n = m * 9;
		int o = guiGraphics.guiHeight() / 2 + n / 3;
		
		// VoxelMap: Move scoreboard to make room for minimap
		o = VoxelConstants.moveScoreboard(o, n);
		
		int p = 3;
		int q = guiGraphics.guiWidth() - j - 3;
		int r = guiGraphics.guiWidth() - 3 + 2;
		int s = this.minecraft.options.getBackgroundColor(0.3F);
		int t = this.minecraft.options.getBackgroundColor(0.4F);
		int u = o - m * 9;
		guiGraphics.fill(q - 2, u - 9 - 1, r, u - 1, t);
		guiGraphics.fill(q - 2, u - 1, r, o, s);
		guiGraphics.drawString(this.getFont(), component, q + j / 2 - i / 2, u - 9, -1, false);

		for (int v = 0; v < m; v++) {
			DisplayEntry lv2 = lvs[v];
			int w = o - (m - v) * 9;
			guiGraphics.drawString(this.getFont(), lv2.name, q, w, -1, false);
			guiGraphics.drawString(this.getFont(), lv2.score, r - lv2.scoreWidth, w, -1, false);
		}
	}

	@Nullable
	private Player getCameraPlayer() {
		return this.minecraft.getCameraEntity() instanceof Player player ? player : null;
	}

	@Nullable
	private LivingEntity getPlayerVehicleWithHealth() {
		Player player = this.getCameraPlayer();
		if (player != null) {
			Entity entity = player.getVehicle();
			if (entity == null) {
				return null;
			}

			if (entity instanceof LivingEntity) {
				return (LivingEntity)entity;
			}
		}

		return null;
	}

	private int getVehicleMaxHearts(@Nullable LivingEntity livingEntity) {
		if (livingEntity != null && livingEntity.showVehicleHealth()) {
			float f = livingEntity.getMaxHealth();
			int i = (int)(f + 0.5F) / 2;
			if (i > 30) {
				i = 30;
			}

			return i;
		} else {
			return 0;
		}
	}

	private int getVisibleVehicleHeartRows(int i) {
		return (int)Math.ceil(i / 10.0);
	}

	private void renderPlayerHealth(GuiGraphics guiGraphics) {
		Player player = this.getCameraPlayer();
		if (player != null) {
			float renderedHealth = diagnosticPlayerHealth(player.getHealth());
			int i = Mth.ceil(renderedHealth);
			boolean bl = this.healthBlinkTime > this.tickCount && (this.healthBlinkTime - this.tickCount) / 3L % 2L == 1L;
			long l = Util.getMillis();
			if (i < this.lastHealth && player.invulnerableTime > 0) {
				this.lastHealthTime = l;
				this.healthBlinkTime = this.tickCount + 20;
			} else if (i > this.lastHealth && player.invulnerableTime > 0) {
				this.lastHealthTime = l;
				this.healthBlinkTime = this.tickCount + 10;
			}

			if (l - this.lastHealthTime > 1000L) {
				this.displayHealth = i;
				this.lastHealthTime = l;
			}

			this.lastHealth = i;
			int j = this.displayHealth;
			this.random.setSeed(this.tickCount * 312871);
			int k = guiGraphics.guiWidth() / 2 - 91;
			int m = guiGraphics.guiWidth() / 2 + 91;
			int n = guiGraphics.guiHeight() - 39;
			float f = Math.max(diagnosticPlayerMaxHealth((float)player.getAttributeValue(Attributes.MAX_HEALTH)), Math.max(j, i));
			int o = Mth.ceil(diagnosticPlayerAbsorption(player.getAbsorptionAmount()));
			int p = Mth.ceil((f + o) / 2.0F / 10.0F);
			int q = Math.max(10 - (p - 2), 3);
			int r = n - 10;
			int s = -1;
			if (player.hasEffect(MobEffects.REGENERATION)) {
				s = this.tickCount % Mth.ceil(f + 5.0F);
			}

			Profiler.get().push("armor");
			this.renderArmor(guiGraphics, player, n, p, q, k);
			Profiler.get().popPush("health");
			this.renderHearts(guiGraphics, player, k, n, q, s, f, i, j, o, bl);
			LivingEntity livingEntity = this.getPlayerVehicleWithHealth();
			int t = diagnosticMountPresent(livingEntity) ? diagnosticMountMaxHearts(livingEntity) : 0;
			if (t == 0) {
				Profiler.get().popPush("food");
				this.renderFood(guiGraphics, player, n, m);
				r -= 10;
			}

			Profiler.get().popPush("air");
			this.renderAirBubbles(guiGraphics, player, t, r, m);
			Profiler.get().pop();
		}
	}

	private static float diagnosticPlayerHealth(float fallback) {
		return Math.max(0.0F, diagnosticPlayerFloat("mattmc.dev.deterministicCameraCapture.playerHealth", "mattmc.dev.graphicsFrameBenchmark.playerHealth", fallback));
	}

	private static float diagnosticPlayerMaxHealth(float fallback) {
		return Math.max(1.0F, diagnosticPlayerFloat("mattmc.dev.deterministicCameraCapture.playerMaxHealth", "mattmc.dev.graphicsFrameBenchmark.playerMaxHealth", fallback));
	}

	private static float diagnosticPlayerAbsorption(float fallback) {
		return Math.max(0.0F, diagnosticPlayerFloat("mattmc.dev.deterministicCameraCapture.playerAbsorption", "mattmc.dev.graphicsFrameBenchmark.playerAbsorption", fallback));
	}

	private static int diagnosticFoodLevel(int fallback) {
		return Mth.clamp(diagnosticPlayerInt("mattmc.dev.deterministicCameraCapture.playerFoodLevel", "mattmc.dev.graphicsFrameBenchmark.playerFoodLevel", fallback), 0, 20);
	}

	private static float diagnosticFoodSaturation(float fallback) {
		return Math.max(0.0F, diagnosticPlayerFloat("mattmc.dev.deterministicCameraCapture.playerFoodSaturation", "mattmc.dev.graphicsFrameBenchmark.playerFoodSaturation", fallback));
	}

	private static boolean diagnosticFoodHungerEffect(Player player) {
		return player.hasEffect(MobEffects.HUNGER) || Boolean.getBoolean("mattmc.dev.deterministicCameraCapture.playerFoodHungerEffect");
	}

	private static boolean diagnosticFoodJitter() {
		return Boolean.getBoolean("mattmc.dev.deterministicCameraCapture.playerFoodJitter");
	}

	private static int diagnosticMaxAirSupply(int fallback) {
		return Math.max(1, diagnosticPlayerInt("mattmc.dev.deterministicCameraCapture.playerMaxAirSupply", "mattmc.dev.graphicsFrameBenchmark.playerMaxAirSupply", fallback));
	}

	private static int diagnosticAirSupply(int fallback, int maxAirSupply) {
		return Math.clamp(diagnosticPlayerInt("mattmc.dev.deterministicCameraCapture.playerAirSupply", "mattmc.dev.graphicsFrameBenchmark.playerAirSupply", fallback), 0, maxAirSupply);
	}

	private static boolean diagnosticAirUnderwater(Player player) {
		String value = System.getProperty("mattmc.dev.deterministicCameraCapture.playerUnderwater");
		if (value == null || value.isBlank()) {
			value = System.getProperty("mattmc.dev.graphicsFrameBenchmark.playerUnderwater");
		}
		return value == null || value.isBlank() ? player.isEyeInFluid(FluidTags.WATER) : Boolean.parseBoolean(value);
	}

	private static boolean diagnosticAirPopAnimation() {
		return Boolean.getBoolean("mattmc.dev.deterministicCameraCapture.playerAirPop")
			|| Boolean.getBoolean("mattmc.dev.graphicsFrameBenchmark.playerAirPop");
	}

	private static boolean diagnosticMountPresent(@Nullable LivingEntity livingEntity) {
		String value = System.getProperty("mattmc.dev.deterministicCameraCapture.mountPresent");
		if (value == null || value.isBlank()) {
			value = System.getProperty("mattmc.dev.graphicsFrameBenchmark.mountPresent");
		}
		return value == null || value.isBlank() ? livingEntity != null : Boolean.parseBoolean(value);
	}

	private static int diagnosticMountMaxHearts(@Nullable LivingEntity livingEntity) {
		int forcedRows = diagnosticPlayerInt("mattmc.dev.deterministicCameraCapture.mountHealthRows", "mattmc.dev.graphicsFrameBenchmark.mountHealthRows", -1);
		if (forcedRows >= 0) {
			return Math.clamp(forcedRows, 0, 3) * NUM_HEARTS_PER_ROW;
		}
		float fallback = livingEntity == null ? 0.0F : livingEntity.getMaxHealth();
		float maxHealth = Math.max(0.0F, diagnosticPlayerFloat("mattmc.dev.deterministicCameraCapture.mountMaxHealth", "mattmc.dev.graphicsFrameBenchmark.mountMaxHealth", fallback));
		int hearts = (int)(maxHealth + 0.5F) / 2;
		return Math.min(hearts, 30);
	}

	private static int diagnosticMountHealth(@Nullable LivingEntity livingEntity) {
		float fallback = livingEntity == null ? 0.0F : livingEntity.getHealth();
		float health = Math.max(0.0F, diagnosticPlayerFloat("mattmc.dev.deterministicCameraCapture.mountHealth", "mattmc.dev.graphicsFrameBenchmark.mountHealth", fallback));
		return Mth.ceil(health);
	}

	private static float diagnosticPlayerFloat(String primaryProperty, String secondaryProperty, float fallback) {
		String value = System.getProperty(primaryProperty);
		if (value == null || value.isBlank()) {
			value = System.getProperty(secondaryProperty);
		}
		if (value == null || value.isBlank()) {
			return fallback;
		}
		float parsed = Float.parseFloat(value);
		return Float.isFinite(parsed) ? parsed : fallback;
	}

	private static int diagnosticPlayerInt(String primaryProperty, String secondaryProperty, int fallback) {
		String value = System.getProperty(primaryProperty);
		if (value == null || value.isBlank()) {
			value = System.getProperty(secondaryProperty);
		}
		if (value == null || value.isBlank()) {
			return fallback;
		}
		return Integer.parseInt(value);
	}

	private void renderArmor(GuiGraphics guiGraphics, Player player, int i, int j, int k, int l) {
		int m = player.getArmorValue();
		if (m > 0) {
			int n = i - (j - 1) * k - 10;

			if (RustGalGuiRenderer.isMigratedGuiDisabledForDiagnostics() || RustGalGuiRenderer.isArmorDisabledForDiagnostics()) {
				// Dev-only measurement control: no migrated or legacy armor icon draw.
			} else if (RustGalGuiRenderer.shouldDrawJavaCompatibilityGui() || RustGalGuiRenderer.isArmorLegacyControl()) {
				for (int o = 0; o < 10; o++) {
					int p = l + o * 8;
					if (o * 2 + 1 < m) {
						guiGraphics.blitSprite(RenderPipelines.GUI_TEXTURED, ARMOR_FULL_SPRITE, p, n, 9, 9);
					}

					if (o * 2 + 1 == m) {
						guiGraphics.blitSprite(RenderPipelines.GUI_TEXTURED, ARMOR_HALF_SPRITE, p, n, 9, 9);
					}

					if (o * 2 + 1 > m) {
						guiGraphics.blitSprite(RenderPipelines.GUI_TEXTURED, ARMOR_EMPTY_SPRITE, p, n, 9, 9);
					}
				}
			} else {
				RustGalGuiRenderer.enqueueArmorIcons(this.minecraft, guiGraphics, m, l, n);
			}
		}
	}

	private void renderHearts(GuiGraphics guiGraphics, Player player, int i, int j, int k, int l, float f, int m, int n, int o, boolean bl) {
		Gui.HeartType heartType = Gui.HeartType.forPlayer(player);
		boolean bl2 = player.level().getLevelData().isHardcore()
			|| Boolean.getBoolean("mattmc.dev.deterministicCameraCapture.playerHealthHardcore");
		boolean heartFlash = bl || Boolean.getBoolean("mattmc.dev.deterministicCameraCapture.playerHealthFlash");
		int p = Mth.ceil(f / 2.0);
		int q = Mth.ceil(o / 2.0);
		int r = p * 2;
		boolean playerHeartsDisabled = RustGalGuiRenderer.isMigratedGuiDisabledForDiagnostics() || RustGalGuiRenderer.isPlayerHealthDisabledForDiagnostics();
		boolean playerHeartsLegacy = RustGalGuiRenderer.shouldDrawJavaCompatibilityGui() || RustGalGuiRenderer.isPlayerHealthLegacyControl();
		boolean absorptionHeartsDisabled = RustGalGuiRenderer.isMigratedGuiDisabledForDiagnostics() || RustGalGuiRenderer.isAbsorptionHealthDisabledForDiagnostics();
		boolean absorptionHeartsLegacy = RustGalGuiRenderer.shouldDrawJavaCompatibilityGui() || RustGalGuiRenderer.isAbsorptionHealthLegacyControl();
		boolean migratePlayerHearts = !playerHeartsDisabled && !playerHeartsLegacy;
		boolean migrateAbsorptionHearts = !absorptionHeartsDisabled && !absorptionHeartsLegacy;
		List<PlayerHeartRequest> rustPlayerHearts = migratePlayerHearts ? new ArrayList<>() : List.of();
		List<AbsorptionHeartRequest> rustAbsorptionHearts = migrateAbsorptionHearts ? new ArrayList<>() : List.of();
		int rustHeartOrder = 0;

		for (int s = p + q - 1; s >= 0; s--) {
			int t = s / 10;
			int u = s % 10;
			int v = i + u * 8;
			int w = j - t * k;
			if (m + o <= 4) {
				w += this.random.nextInt(2);
			}

			if (s < p && s == l) {
				w -= 2;
			}

			if (s < p) {
				if (migratePlayerHearts) {
					rustPlayerHearts.add(new PlayerHeartRequest(
						PlayerHeartVariant.CONTAINER,
						GuiHeartState.CONTAINER,
						bl2,
						heartFlash,
						rustHeartOrder++,
						v,
						w
					));
				} else if (!playerHeartsDisabled) {
					this.renderHeart(guiGraphics, Gui.HeartType.CONTAINER, v, w, bl2, heartFlash, false);
				}
			} else {
				if (migrateAbsorptionHearts) {
					rustAbsorptionHearts.add(new AbsorptionHeartRequest(
						AbsorptionHeartVariant.CONTAINER,
						GuiHeartState.CONTAINER,
						bl2,
						heartFlash,
						rustHeartOrder++,
						v,
						w
					));
				} else if (!absorptionHeartsDisabled) {
					this.renderHeart(guiGraphics, Gui.HeartType.CONTAINER, v, w, bl2, heartFlash, false);
				}
			}
			int x = s * 2;
			boolean bl3 = s >= p;
			if (bl3) {
				int y = x - r;
				if (y < o) {
					boolean bl4 = y + 1 == o;
					if (migrateAbsorptionHearts) {
						rustAbsorptionHearts.add(new AbsorptionHeartRequest(
							rustAbsorptionHeartVariant(heartType),
							bl4 ? GuiHeartState.HALF : GuiHeartState.FULL,
							bl2,
							false,
							rustHeartOrder++,
							v,
							w
						));
					} else if (!absorptionHeartsDisabled) {
						this.renderHeart(guiGraphics, heartType == Gui.HeartType.WITHERED ? heartType : Gui.HeartType.ABSORBING, v, w, bl2, false, bl4);
					}
				}
			}

			if (heartFlash && x < n) {
				boolean bl5 = x + 1 == n;
				if (migratePlayerHearts && s < p) {
					rustPlayerHearts.add(new PlayerHeartRequest(
						rustHeartVariant(heartType),
						bl5 ? GuiHeartState.HALF : GuiHeartState.FULL,
						bl2,
						true,
						rustHeartOrder++,
						v,
						w
					));
				} else if (!playerHeartsDisabled) {
					this.renderHeart(guiGraphics, heartType, v, w, bl2, true, bl5);
				}
			}

			if (x < m) {
				boolean bl5 = x + 1 == m;
				if (migratePlayerHearts && s < p) {
					rustPlayerHearts.add(new PlayerHeartRequest(
						rustHeartVariant(heartType),
						bl5 ? GuiHeartState.HALF : GuiHeartState.FULL,
						bl2,
						false,
						rustHeartOrder++,
						v,
						w
					));
				} else if (!playerHeartsDisabled) {
					this.renderHeart(guiGraphics, heartType, v, w, bl2, false, bl5);
				}
			}
		}

		if (migrateAbsorptionHearts) {
			RustGalGuiRenderer.enqueueAbsorptionHearts(this.minecraft, guiGraphics, rustAbsorptionHearts);
		}
		if (migratePlayerHearts) {
			RustGalGuiRenderer.enqueuePlayerHearts(this.minecraft, guiGraphics, rustPlayerHearts);
		}
	}

	private static PlayerHeartVariant rustHeartVariant(Gui.HeartType heartType) {
		return switch (heartType) {
			case NORMAL -> PlayerHeartVariant.NORMAL;
			case POISIONED -> PlayerHeartVariant.POISONED;
			case WITHERED -> PlayerHeartVariant.WITHERED;
			case FROZEN -> PlayerHeartVariant.FROZEN;
			case CONTAINER, ABSORBING -> throw new IllegalArgumentException("not a migrated player-health heart variant: " + heartType);
		};
	}

	private static AbsorptionHeartVariant rustAbsorptionHeartVariant(Gui.HeartType heartType) {
		return heartType == Gui.HeartType.WITHERED ? AbsorptionHeartVariant.WITHERED : AbsorptionHeartVariant.ABSORBING;
	}

	private void renderHeart(GuiGraphics guiGraphics, Gui.HeartType heartType, int i, int j, boolean bl, boolean bl2, boolean bl3) {
		guiGraphics.blitSprite(RenderPipelines.GUI_TEXTURED, heartType.getSprite(bl, bl3, bl2), i, j, 9, 9);
	}

	private void renderAirBubbles(GuiGraphics guiGraphics, Player player, int i, int j, int k) {
		int l = diagnosticMaxAirSupply(player.getMaxAirSupply());
		int m = diagnosticAirSupply(player.getAirSupply(), l);
		boolean bl = diagnosticAirUnderwater(player);
		if (bl || m < l) {
			j = this.getAirBubbleYLine(i, j);
			int n = getCurrentAirSupplyBubble(m, l, -2);
			int o = getCurrentAirSupplyBubble(m, l, 0);
			int p = 10 - getCurrentAirSupplyBubble(m, l, getEmptyBubbleDelayDuration(m, bl));
			boolean bl2 = n != o || diagnosticAirPopAnimation();
			if (!bl) {
				this.lastBubblePopSoundPlayed = 0;
			}

			boolean airDisabled = RustGalGuiRenderer.isMigratedGuiDisabledForDiagnostics() || RustGalGuiRenderer.isAirDisabledForDiagnostics();
			boolean airLegacy = RustGalGuiRenderer.shouldDrawJavaCompatibilityGui() || RustGalGuiRenderer.isAirLegacyControl();
			boolean migrateAir = !airDisabled && !airLegacy;
			List<AirBubbleRequest> rustAirBubbles = migrateAir ? new ArrayList<>() : List.of();
			int airOrder = 0;
			for (int q = 1; q <= 10; q++) {
				int r = k - (q - 1) * 8 - 9;
				if (q <= n) {
					if (migrateAir) {
						rustAirBubbles.add(new AirBubbleRequest(AirBubbleState.FULL, false, true, airOrder++, r, j));
					} else if (!airDisabled) {
						guiGraphics.blitSprite(RenderPipelines.GUI_TEXTURED, AIR_SPRITE, r, j, 9, 9);
					}
				} else if (bl2 && q == o && bl) {
					if (migrateAir) {
						rustAirBubbles.add(new AirBubbleRequest(AirBubbleState.PARTIAL, true, true, airOrder++, r, j));
					} else if (!airDisabled) {
						guiGraphics.blitSprite(RenderPipelines.GUI_TEXTURED, AIR_POPPING_SPRITE, r, j, 9, 9);
					}
					this.playAirBubblePoppedSound(q, player, p);
				} else if (q > 10 - p) {
					int s = p == 10 && this.tickCount % 2 == 0 ? this.random.nextInt(2) : 0;
					if (migrateAir) {
						rustAirBubbles.add(new AirBubbleRequest(AirBubbleState.EMPTY, false, true, airOrder++, r, j + s));
					} else if (!airDisabled) {
						guiGraphics.blitSprite(RenderPipelines.GUI_TEXTURED, AIR_EMPTY_SPRITE, r, j + s, 9, 9);
					}
				}
			}
			if (migrateAir) {
				RustGalGuiRenderer.enqueueAirBubbles(this.minecraft, guiGraphics, rustAirBubbles);
			}
		}
	}

	private int getAirBubbleYLine(int i, int j) {
		int k = this.getVisibleVehicleHeartRows(i) - 1;
		return j - k * 10;
	}

	private static int getCurrentAirSupplyBubble(int i, int j, int k) {
		return Mth.ceil((float)((i + k) * 10) / j);
	}

	private static int getEmptyBubbleDelayDuration(int i, boolean bl) {
		return i != 0 && bl ? 1 : 0;
	}

	private void playAirBubblePoppedSound(int i, Player player, int j) {
		if (this.lastBubblePopSoundPlayed != i) {
			float f = 0.5F + 0.1F * Math.max(0, j - 3 + 1);
			float g = 1.0F + 0.1F * Math.max(0, j - 5 + 1);
			player.playSound(SoundEvents.BUBBLE_POP, f, g);
			this.lastBubblePopSoundPlayed = i;
		}
	}

	private void renderFood(GuiGraphics guiGraphics, Player player, int i, int j) {
		FoodData foodData = player.getFoodData();
		int k = diagnosticFoodLevel(foodData.getFoodLevel());
		float saturation = diagnosticFoodSaturation(foodData.getSaturationLevel());
		boolean hungerEffect = diagnosticFoodHungerEffect(player);
		boolean forceJitter = diagnosticFoodJitter();
		boolean hungerDisabled = RustGalGuiRenderer.isMigratedGuiDisabledForDiagnostics() || RustGalGuiRenderer.isHungerDisabledForDiagnostics();
		boolean hungerLegacy = RustGalGuiRenderer.shouldDrawJavaCompatibilityGui() || RustGalGuiRenderer.isHungerLegacyControl();
		boolean migrateHunger = !hungerDisabled && !hungerLegacy;
		List<HungerIconRequest> rustHungerIcons = migrateHunger ? new ArrayList<>() : List.of();
		int rustHungerOrder = 0;

		for (int l = 0; l < 10; l++) {
			int m = i;
			ResourceLocation resourceLocation;
			ResourceLocation resourceLocation2;
			ResourceLocation resourceLocation3;
			if (hungerEffect) {
				resourceLocation = FOOD_EMPTY_HUNGER_SPRITE;
				resourceLocation2 = FOOD_HALF_HUNGER_SPRITE;
				resourceLocation3 = FOOD_FULL_HUNGER_SPRITE;
			} else {
				resourceLocation = FOOD_EMPTY_SPRITE;
				resourceLocation2 = FOOD_HALF_SPRITE;
				resourceLocation3 = FOOD_FULL_SPRITE;
			}

			boolean jitterActive = (saturation <= 0.0F && this.tickCount % (k * 3 + 1) == 0) || forceJitter;
			int jitterOffset = 0;
			if (jitterActive) {
				jitterOffset = this.random.nextInt(3) - 1;
				m = i + jitterOffset;
			}

			int n = j - l * 8 - 9;
			HungerIconVariant variant = hungerEffect ? HungerIconVariant.HUNGER_EFFECT : HungerIconVariant.NORMAL;
			if (migrateHunger) {
				rustHungerIcons.add(new HungerIconRequest(
					variant,
					HungerIconState.EMPTY,
					jitterActive,
					jitterOffset,
					rustHungerOrder++,
					n,
					m
				));
			} else if (!hungerDisabled) {
				guiGraphics.blitSprite(RenderPipelines.GUI_TEXTURED, resourceLocation, n, m, 9, 9);
			}
			if (l * 2 + 1 < k) {
				if (migrateHunger) {
					rustHungerIcons.add(new HungerIconRequest(
						variant,
						HungerIconState.FULL,
						jitterActive,
						jitterOffset,
						rustHungerOrder++,
						n,
						m
					));
				} else if (!hungerDisabled) {
					guiGraphics.blitSprite(RenderPipelines.GUI_TEXTURED, resourceLocation3, n, m, 9, 9);
				}
			}

			if (l * 2 + 1 == k) {
				if (migrateHunger) {
					rustHungerIcons.add(new HungerIconRequest(
						variant,
						HungerIconState.HALF,
						jitterActive,
						jitterOffset,
						rustHungerOrder++,
						n,
						m
					));
				} else if (!hungerDisabled) {
					guiGraphics.blitSprite(RenderPipelines.GUI_TEXTURED, resourceLocation2, n, m, 9, 9);
				}
			}
		}

		if (migrateHunger) {
			RustGalGuiRenderer.enqueueHungerIcons(this.minecraft, guiGraphics, rustHungerIcons);
		}
	}

	private void renderVehicleHealth(GuiGraphics guiGraphics) {
		LivingEntity livingEntity = this.getPlayerVehicleWithHealth();
		if (!diagnosticMountPresent(livingEntity)) {
			return;
		}
		int i = diagnosticMountMaxHearts(livingEntity);
		if (i != 0) {
			int j = diagnosticMountHealth(livingEntity);
			Profiler.get().popPush("mountHealth");
			int k = guiGraphics.guiHeight() - 39;
			int l = guiGraphics.guiWidth() / 2 + 91;
			int m = k;
			boolean mountHealthDisabled = RustGalGuiRenderer.isMigratedGuiDisabledForDiagnostics() || RustGalGuiRenderer.isMountHealthDisabledForDiagnostics();
			boolean mountHealthLegacy = RustGalGuiRenderer.shouldDrawJavaCompatibilityGui() || RustGalGuiRenderer.isMountHealthLegacyControl();
			boolean migrateMountHealth = !mountHealthDisabled && !mountHealthLegacy;
			List<MountHeartRequest> rustMountHearts = migrateMountHealth ? new ArrayList<>() : List.of();
			int rustMountHeartOrder = 0;

			for (int n = 0; i > 0; n += 20) {
				int row = n / 20;
				int o = Math.min(i, 10);
				i -= o;

				for (int p = 0; p < o; p++) {
					int q = l - p * 8 - 9;
					if (migrateMountHealth) {
						rustMountHearts.add(new MountHeartRequest(
							MountHeartVariant.VEHICLE,
							MountHeartState.EMPTY,
							true,
							row,
							rustMountHeartOrder++,
							q,
							m
						));
					} else if (!mountHealthDisabled) {
						guiGraphics.blitSprite(RenderPipelines.GUI_TEXTURED, HEART_VEHICLE_CONTAINER_SPRITE, q, m, 9, 9);
					}
					if (p * 2 + 1 + n < j) {
						if (migrateMountHealth) {
							rustMountHearts.add(new MountHeartRequest(
								MountHeartVariant.VEHICLE,
								MountHeartState.FULL,
								true,
								row,
								rustMountHeartOrder++,
								q,
								m
							));
						} else if (!mountHealthDisabled) {
							guiGraphics.blitSprite(RenderPipelines.GUI_TEXTURED, HEART_VEHICLE_FULL_SPRITE, q, m, 9, 9);
						}
					}

					if (p * 2 + 1 + n == j) {
						if (migrateMountHealth) {
							rustMountHearts.add(new MountHeartRequest(
								MountHeartVariant.VEHICLE,
								MountHeartState.HALF,
								true,
								row,
								rustMountHeartOrder++,
								q,
								m
							));
						} else if (!mountHealthDisabled) {
							guiGraphics.blitSprite(RenderPipelines.GUI_TEXTURED, HEART_VEHICLE_HALF_SPRITE, q, m, 9, 9);
						}
					}
				}

				m -= 10;
			}
			if (migrateMountHealth) {
				RustGalGuiRenderer.enqueueMountHearts(this.minecraft, guiGraphics, rustMountHearts);
			}
		}
	}

	private void renderTextureOverlay(GuiGraphics guiGraphics, ResourceLocation resourceLocation, float f) {
		int i = ARGB.white(f);
		guiGraphics.blit(
			RenderPipelines.GUI_TEXTURED,
			resourceLocation,
			0,
			0,
			0.0F,
			0.0F,
			guiGraphics.guiWidth(),
			guiGraphics.guiHeight(),
			guiGraphics.guiWidth(),
			guiGraphics.guiHeight(),
			i
		);
	}

	private void renderSpyglassOverlay(GuiGraphics guiGraphics, float f) {
		float g = Math.min(guiGraphics.guiWidth(), guiGraphics.guiHeight());
		float i = Math.min(guiGraphics.guiWidth() / g, guiGraphics.guiHeight() / g) * f;
		int j = Mth.floor(g * i);
		int k = Mth.floor(g * i);
		int l = (guiGraphics.guiWidth() - j) / 2;
		int m = (guiGraphics.guiHeight() - k) / 2;
		int n = l + j;
		int o = m + k;
		guiGraphics.blit(RenderPipelines.GUI_TEXTURED, SPYGLASS_SCOPE_LOCATION, l, m, 0.0F, 0.0F, j, k, j, k);
		guiGraphics.fill(RenderPipelines.GUI, 0, o, guiGraphics.guiWidth(), guiGraphics.guiHeight(), -16777216);
		guiGraphics.fill(RenderPipelines.GUI, 0, 0, guiGraphics.guiWidth(), m, -16777216);
		guiGraphics.fill(RenderPipelines.GUI, 0, m, l, o, -16777216);
		guiGraphics.fill(RenderPipelines.GUI, n, m, guiGraphics.guiWidth(), o, -16777216);
	}

	private void updateVignetteBrightness(Entity entity) {
		BlockPos blockPos = BlockPos.containing(entity.getX(), entity.getEyeY(), entity.getZ());
		float f = LightTexture.getBrightness(entity.level().dimensionType(), entity.level().getMaxLocalRawBrightness(blockPos));
		float g = Mth.clamp(1.0F - f, 0.0F, 1.0F);
		this.vignetteBrightness = this.vignetteBrightness + (g - this.vignetteBrightness) * 0.01F;
	}

	/**
	 * Capture-only readiness predicate for the shared cross-repository parity
	 * harness. It observes the same vanilla light target used by the ordinary
	 * tick method and never writes GUI state; a slow candidate renderer must not
	 * be compared against a baseline captured midway through this animation.
	 */
	public boolean vignetteBrightnessSettledForDeterministicCapture(@Nullable Entity entity) {
		if (entity == null || this.minecraft.level == null) return false;
		BlockPos blockPos = BlockPos.containing(entity.getX(), entity.getEyeY(), entity.getZ());
		float brightness = LightTexture.getBrightness(entity.level().dimensionType(), entity.level().getMaxLocalRawBrightness(blockPos));
		float target = Mth.clamp(1.0F - brightness, 0.0F, 1.0F);
		return Math.abs(this.vignetteBrightness - target) <= 0.02F;
	}

	/** Capture-only immutable observation of vanilla's current vignette fade. */
	public float vignetteBrightnessForDeterministicCapture() {
		return this.vignetteBrightness;
	}

	private void renderVignette(GuiGraphics guiGraphics, @Nullable Entity entity) {
		// Iris: Check if vignette should be rendered
		net.irisshaders.iris.pipeline.WorldRenderingPipeline pipeline =
			 net.vulkanic.bridge.RustGalVulkanWholeFrameMode.enabled()
				|| net.vulkanic.VulkanicAPI.isVulkanBackendSelected()
				? null
				: net.irisshaders.iris.Iris.getPipelineManager().getPipelineNullable();
		
		if (pipeline != null && !pipeline.shouldRenderVignette()) {
			return;
		}
		
		WorldBorder worldBorder = this.minecraft.level.getWorldBorder();
		float f = 0.0F;
		if (entity != null) {
			float g = (float)worldBorder.getDistanceToBorder(entity);
			double d = Math.min(worldBorder.getLerpSpeed() * worldBorder.getWarningTime() * 1000.0, Math.abs(worldBorder.getLerpTarget() - worldBorder.getSize()));
			double e = Math.max(worldBorder.getWarningBlocks(), d);
			if (g < e) {
				f = 1.0F - (float)(g / e);
			}
		}

		int i;
		if (f > 0.0F) {
			f = Mth.clamp(f, 0.0F, 1.0F);
			i = ARGB.colorFromFloat(1.0F, 0.0F, f, f);
		} else {
			float h = this.vignetteBrightness;
			h = Mth.clamp(h, 0.0F, 1.0F);
			i = ARGB.colorFromFloat(1.0F, h, h, h);
		}

		guiGraphics.blit(
			RenderPipelines.VIGNETTE,
			VIGNETTE_LOCATION,
			0,
			0,
			0.0F,
			0.0F,
			guiGraphics.guiWidth(),
			guiGraphics.guiHeight(),
			guiGraphics.guiWidth(),
			guiGraphics.guiHeight(),
			i
		);
	}

	private void renderPortalOverlay(GuiGraphics guiGraphics, float f) {
		if (f < 1.0F) {
			f *= f;
			f *= f;
			f = f * 0.8F + 0.2F;
		}

		int i = ARGB.white(f);
		TextureAtlasSprite textureAtlasSprite = this.minecraft.getBlockRenderer().getBlockModelShaper().getParticleIcon(Blocks.NETHER_PORTAL.defaultBlockState());
		guiGraphics.blitSprite(RenderPipelines.GUI_TEXTURED, textureAtlasSprite, 0, 0, guiGraphics.guiWidth(), guiGraphics.guiHeight(), i);
	}

	private void renderConfusionOverlay(GuiGraphics guiGraphics, float f) {
		int i = guiGraphics.guiWidth();
		int j = guiGraphics.guiHeight();
		guiGraphics.pose().pushMatrix();
		float g = Mth.lerp(f, 2.0F, 1.0F);
		guiGraphics.pose().translate(i / 2.0F, j / 2.0F);
		guiGraphics.pose().scale(g, g);
		guiGraphics.pose().translate(-i / 2.0F, -j / 2.0F);
		float h = 0.2F * f;
		float k = 0.4F * f;
		float l = 0.2F * f;
		guiGraphics.blit(RenderPipelines.GUI_NAUSEA_OVERLAY, NAUSEA_LOCATION, 0, 0, 0.0F, 0.0F, i, j, i, j, ARGB.colorFromFloat(1.0F, h, k, l));
		guiGraphics.pose().popMatrix();
	}

	private void renderSlot(GuiGraphics guiGraphics, int i, int j, DeltaTracker deltaTracker, Player player, ItemStack itemStack, int k) {
		if (!itemStack.isEmpty()) {
			float f = itemStack.getPopTime() - deltaTracker.getGameTimeDeltaPartialTick(false);
			if (f > 0.0F) {
				float g = 1.0F + f / 5.0F;
				guiGraphics.pose().pushMatrix();
				guiGraphics.pose().translate(i + 8, j + 12);
				guiGraphics.pose().scale(1.0F / g, (g + 1.0F) / 2.0F);
				guiGraphics.pose().translate(-(i + 8), -(j + 12));
			}

			if (!net.minecraft.client.dev.GraphicsAuditGuiItemPlacementFixture.render(guiGraphics, player, itemStack, i, j, k))
				guiGraphics.renderItem(player, itemStack, i, j, k);
			if (f > 0.0F) {
				guiGraphics.pose().popMatrix();
			}

			guiGraphics.renderItemDecorations(this.minecraft.font, itemStack, i, j);
		}
	}

	public void tick(boolean bl) {
		this.tickAutosaveIndicator();
		if (!bl) {
			this.tick();
		}
	}

	private void tick() {
		if (this.overlayMessageTime > 0) {
			this.overlayMessageTime--;
		}

		if (this.titleTime > 0) {
			this.titleTime--;
			if (this.titleTime <= 0) {
				this.title = null;
				this.subtitle = null;
			}
		}

		this.tickCount++;
		Entity entity = this.minecraft.getCameraEntity();
		if (entity != null) {
			this.updateVignetteBrightness(entity);
		}

		if (this.minecraft.player != null) {
			ItemStack itemStack = this.minecraft.player.getInventory().getSelectedItem();
			if (itemStack.isEmpty()) {
				this.toolHighlightTimer = 0;
			} else if (this.lastToolHighlight.isEmpty()
				|| !itemStack.is(this.lastToolHighlight.getItem())
				|| !itemStack.getHoverName().equals(this.lastToolHighlight.getHoverName())) {
				this.toolHighlightTimer = (int)(40.0 * this.minecraft.options.notificationDisplayTime().get());
			} else if (this.toolHighlightTimer > 0) {
				this.toolHighlightTimer--;
			}

			this.lastToolHighlight = itemStack;
		}

		this.chat.tick();
	}

	private void tickAutosaveIndicator() {
		MinecraftServer minecraftServer = this.minecraft.getSingleplayerServer();
		boolean bl = minecraftServer != null && minecraftServer.isCurrentlySaving();
		this.lastAutosaveIndicatorValue = this.autosaveIndicatorValue;
		this.autosaveIndicatorValue = Mth.lerp(0.2F, this.autosaveIndicatorValue, bl ? 1.0F : 0.0F);
	}

	public void setNowPlaying(Component component) {
		Component component2 = Component.translatable("record.nowPlaying", new Object[]{component});
		this.setOverlayMessage(component2, true);
	}

	public void setOverlayMessage(Component component, boolean bl) {
		this.setChatDisabledByPlayerShown(false);
		this.overlayMessageString = component;
		this.overlayMessageTime = 60;
		this.animateOverlayMessageColor = bl;
	}

	public void setChatDisabledByPlayerShown(boolean bl) {
		this.chatDisabledByPlayerShown = bl;
	}

	public boolean isShowingChatDisabledByPlayer() {
		return this.chatDisabledByPlayerShown && this.overlayMessageTime > 0;
	}

	public void setTimes(int i, int j, int k) {
		if (i >= 0) {
			this.titleFadeInTime = i;
		}

		if (j >= 0) {
			this.titleStayTime = j;
		}

		if (k >= 0) {
			this.titleFadeOutTime = k;
		}

		if (this.titleTime > 0) {
			this.titleTime = this.titleFadeInTime + this.titleStayTime + this.titleFadeOutTime;
		}
	}

	public void setSubtitle(Component component) {
		this.subtitle = component;
	}

	public void setTitle(Component component) {
		this.title = component;
		this.titleTime = this.titleFadeInTime + this.titleStayTime + this.titleFadeOutTime;
	}

	public void clearTitles() {
		this.title = null;
		this.subtitle = null;
		this.titleTime = 0;
	}

	public ChatComponent getChat() {
		return this.chat;
	}

	public int getGuiTicks() {
		return this.tickCount;
	}

	public Font getFont() {
		return this.minecraft.font;
	}

	public SpectatorGui getSpectatorGui() {
		return this.spectatorGui;
	}

	public PlayerTabOverlay getTabList() {
		return this.tabList;
	}

	public void onDisconnected() {
		this.tabList.reset();
		this.bossOverlay.reset();
		this.minecraft.getToastManager().clear();
		this.debugOverlay.reset();
		this.chat.clearMessages(true);
		this.clearTitles();
		this.resetTitleTimes();
	}

	public BossHealthOverlay getBossOverlay() {
		return this.bossOverlay;
	}

	public DebugScreenOverlay getDebugOverlay() {
		return this.debugOverlay;
	}

	public void clearCache() {
		this.debugOverlay.clearChunkCache();
	}

	public void renderSavingIndicator(GuiGraphics guiGraphics, DeltaTracker deltaTracker) {
		if (this.minecraft.options.showAutosaveIndicator().get() && (this.autosaveIndicatorValue > 0.0F || this.lastAutosaveIndicatorValue > 0.0F)) {
			int i = Mth.floor(
				255.0F * Mth.clamp(Mth.lerp(deltaTracker.getRealtimeDeltaTicks(), this.lastAutosaveIndicatorValue, this.autosaveIndicatorValue), 0.0F, 1.0F)
			);
			if (i > 0) {
				Font font = this.getFont();
				int j = font.width(SAVING_TEXT);
				int k = ARGB.color(i, -1);
				int l = guiGraphics.guiWidth() - j - 5;
				int m = guiGraphics.guiHeight() - 9 - 5;
				guiGraphics.nextStratum();
				guiGraphics.drawStringWithBackdrop(font, SAVING_TEXT, l, m, j, k);
			}
		}
	}

	private boolean willPrioritizeExperienceInfo() {
		return this.minecraft.player.experienceDisplayStartTick + 100 > this.minecraft.player.tickCount;
	}

	private boolean willPrioritizeJumpInfo() {
		return this.minecraft.player.getJumpRidingScale() > 0.0F
			|| (Integer)Optionull.mapOrDefault(this.minecraft.player.jumpableVehicle(), PlayerRideableJumping::getJumpCooldown, 0) > 0;
	}

	private Gui.ContextualInfo nextContextualInfoState() {
		boolean bl = this.minecraft.player.connection.getWaypointManager().hasWaypoints();
		boolean bl2 = this.minecraft.player.jumpableVehicle() != null;
		boolean bl3 = this.minecraft.gameMode.hasExperience();
		if (bl) {
			if (bl2 && this.willPrioritizeJumpInfo()) {
				return Gui.ContextualInfo.JUMPABLE_VEHICLE;
			} else {
				return bl3 && this.willPrioritizeExperienceInfo() ? Gui.ContextualInfo.EXPERIENCE : Gui.ContextualInfo.LOCATOR;
			}
		} else if (bl2) {
			return Gui.ContextualInfo.JUMPABLE_VEHICLE;
		} else {
			return bl3 ? Gui.ContextualInfo.EXPERIENCE : Gui.ContextualInfo.EMPTY;
		}
	}

	@Environment(EnvType.CLIENT)
	static enum ContextualInfo {
		EMPTY,
		EXPERIENCE,
		LOCATOR,
		JUMPABLE_VEHICLE;
	}

	@Environment(EnvType.CLIENT)
	static enum HeartType {
		CONTAINER(
			ResourceLocation.withDefaultNamespace("hud/heart/container"),
			ResourceLocation.withDefaultNamespace("hud/heart/container_blinking"),
			ResourceLocation.withDefaultNamespace("hud/heart/container"),
			ResourceLocation.withDefaultNamespace("hud/heart/container_blinking"),
			ResourceLocation.withDefaultNamespace("hud/heart/container_hardcore"),
			ResourceLocation.withDefaultNamespace("hud/heart/container_hardcore_blinking"),
			ResourceLocation.withDefaultNamespace("hud/heart/container_hardcore"),
			ResourceLocation.withDefaultNamespace("hud/heart/container_hardcore_blinking")
		),
		NORMAL(
			ResourceLocation.withDefaultNamespace("hud/heart/full"),
			ResourceLocation.withDefaultNamespace("hud/heart/full_blinking"),
			ResourceLocation.withDefaultNamespace("hud/heart/half"),
			ResourceLocation.withDefaultNamespace("hud/heart/half_blinking"),
			ResourceLocation.withDefaultNamespace("hud/heart/hardcore_full"),
			ResourceLocation.withDefaultNamespace("hud/heart/hardcore_full_blinking"),
			ResourceLocation.withDefaultNamespace("hud/heart/hardcore_half"),
			ResourceLocation.withDefaultNamespace("hud/heart/hardcore_half_blinking")
		),
		POISIONED(
			ResourceLocation.withDefaultNamespace("hud/heart/poisoned_full"),
			ResourceLocation.withDefaultNamespace("hud/heart/poisoned_full_blinking"),
			ResourceLocation.withDefaultNamespace("hud/heart/poisoned_half"),
			ResourceLocation.withDefaultNamespace("hud/heart/poisoned_half_blinking"),
			ResourceLocation.withDefaultNamespace("hud/heart/poisoned_hardcore_full"),
			ResourceLocation.withDefaultNamespace("hud/heart/poisoned_hardcore_full_blinking"),
			ResourceLocation.withDefaultNamespace("hud/heart/poisoned_hardcore_half"),
			ResourceLocation.withDefaultNamespace("hud/heart/poisoned_hardcore_half_blinking")
		),
		WITHERED(
			ResourceLocation.withDefaultNamespace("hud/heart/withered_full"),
			ResourceLocation.withDefaultNamespace("hud/heart/withered_full_blinking"),
			ResourceLocation.withDefaultNamespace("hud/heart/withered_half"),
			ResourceLocation.withDefaultNamespace("hud/heart/withered_half_blinking"),
			ResourceLocation.withDefaultNamespace("hud/heart/withered_hardcore_full"),
			ResourceLocation.withDefaultNamespace("hud/heart/withered_hardcore_full_blinking"),
			ResourceLocation.withDefaultNamespace("hud/heart/withered_hardcore_half"),
			ResourceLocation.withDefaultNamespace("hud/heart/withered_hardcore_half_blinking")
		),
		ABSORBING(
			ResourceLocation.withDefaultNamespace("hud/heart/absorbing_full"),
			ResourceLocation.withDefaultNamespace("hud/heart/absorbing_full_blinking"),
			ResourceLocation.withDefaultNamespace("hud/heart/absorbing_half"),
			ResourceLocation.withDefaultNamespace("hud/heart/absorbing_half_blinking"),
			ResourceLocation.withDefaultNamespace("hud/heart/absorbing_hardcore_full"),
			ResourceLocation.withDefaultNamespace("hud/heart/absorbing_hardcore_full_blinking"),
			ResourceLocation.withDefaultNamespace("hud/heart/absorbing_hardcore_half"),
			ResourceLocation.withDefaultNamespace("hud/heart/absorbing_hardcore_half_blinking")
		),
		FROZEN(
			ResourceLocation.withDefaultNamespace("hud/heart/frozen_full"),
			ResourceLocation.withDefaultNamespace("hud/heart/frozen_full_blinking"),
			ResourceLocation.withDefaultNamespace("hud/heart/frozen_half"),
			ResourceLocation.withDefaultNamespace("hud/heart/frozen_half_blinking"),
			ResourceLocation.withDefaultNamespace("hud/heart/frozen_hardcore_full"),
			ResourceLocation.withDefaultNamespace("hud/heart/frozen_hardcore_full_blinking"),
			ResourceLocation.withDefaultNamespace("hud/heart/frozen_hardcore_half"),
			ResourceLocation.withDefaultNamespace("hud/heart/frozen_hardcore_half_blinking")
		);

		private final ResourceLocation full;
		private final ResourceLocation fullBlinking;
		private final ResourceLocation half;
		private final ResourceLocation halfBlinking;
		private final ResourceLocation hardcoreFull;
		private final ResourceLocation hardcoreFullBlinking;
		private final ResourceLocation hardcoreHalf;
		private final ResourceLocation hardcoreHalfBlinking;

		private HeartType(
			final ResourceLocation resourceLocation,
			final ResourceLocation resourceLocation2,
			final ResourceLocation resourceLocation3,
			final ResourceLocation resourceLocation4,
			final ResourceLocation resourceLocation5,
			final ResourceLocation resourceLocation6,
			final ResourceLocation resourceLocation7,
			final ResourceLocation resourceLocation8
		) {
			this.full = resourceLocation;
			this.fullBlinking = resourceLocation2;
			this.half = resourceLocation3;
			this.halfBlinking = resourceLocation4;
			this.hardcoreFull = resourceLocation5;
			this.hardcoreFullBlinking = resourceLocation6;
			this.hardcoreHalf = resourceLocation7;
			this.hardcoreHalfBlinking = resourceLocation8;
		}

		public ResourceLocation getSprite(boolean bl, boolean bl2, boolean bl3) {
			if (!bl) {
				if (bl2) {
					return bl3 ? this.halfBlinking : this.half;
				} else {
					return bl3 ? this.fullBlinking : this.full;
				}
			} else if (bl2) {
				return bl3 ? this.hardcoreHalfBlinking : this.hardcoreHalf;
			} else {
				return bl3 ? this.hardcoreFullBlinking : this.hardcoreFull;
			}
		}

		static Gui.HeartType forPlayer(Player player) {
			Gui.HeartType heartType;
			if (player.hasEffect(MobEffects.POISON)) {
				heartType = POISIONED;
			} else if (player.hasEffect(MobEffects.WITHER)) {
				heartType = WITHERED;
			} else if (player.isFullyFrozen()) {
				heartType = FROZEN;
			} else {
				heartType = NORMAL;
			}

			return heartType;
		}
	}

	@Environment(EnvType.CLIENT)
	public interface RenderFunction {
		void render(GuiGraphics guiGraphics, DeltaTracker deltaTracker);
	}
}
