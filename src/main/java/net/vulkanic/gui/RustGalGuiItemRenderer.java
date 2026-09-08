package net.vulkanic.gui;

import java.util.ArrayList;
import java.util.HashMap;
import java.util.List;
import java.util.Map;
import net.minecraft.Util;
import net.minecraft.client.Minecraft;
import net.minecraft.client.gui.navigation.ScreenRectangle;
import net.minecraft.client.gui.render.state.GuiItemRenderState;
import net.minecraft.client.gui.render.state.GuiRenderState;
import net.minecraft.client.model.TridentModel;
import net.minecraft.client.model.Model;
import net.minecraft.client.model.ChestModel;
import net.minecraft.client.renderer.RenderType;
import net.minecraft.client.renderer.SubmitNodeCollection;
import net.minecraft.client.renderer.SubmitNodeCollector;
import net.minecraft.client.renderer.OrderedSubmitNodeCollector;
import net.blaze3d.vertex.PoseStack;
import net.minecraft.client.renderer.block.model.BakedQuad;
import net.minecraft.client.renderer.entity.ItemRenderer;
import net.minecraft.client.renderer.item.TrackingItemStackRenderState;
import net.minecraft.client.renderer.texture.TextureAtlasSprite;
import net.minecraft.client.renderer.texture.DynamicTexture;
import net.minecraft.resources.ResourceLocation;
import net.minecraft.util.ARGB;
import net.minecraft.util.Mth;
import net.minecraft.world.item.ItemDisplayContext;
import net.minecraft.client.renderer.item.ItemStackRenderState;
import net.minecraft.client.renderer.special.TridentSpecialRenderer;
import net.minecraft.client.renderer.special.ChestSpecialRenderer;
import net.minecraft.client.renderer.special.HangingSignSpecialRenderer;
import net.minecraft.client.renderer.special.StandingSignSpecialRenderer;
import net.minecraft.client.renderer.special.CopperGolemStatueSpecialRenderer;
import net.minecraft.client.renderer.special.BedSpecialRenderer;
import net.minecraft.client.renderer.special.ShieldSpecialRenderer;
import net.minecraft.client.renderer.special.BannerSpecialRenderer;
import net.minecraft.client.renderer.special.SkullSpecialRenderer;
import net.minecraft.client.renderer.special.PlayerHeadSpecialRenderer;
import net.minecraft.client.renderer.special.ShulkerBoxSpecialRenderer;
import net.minecraft.client.renderer.blockentity.ShulkerBoxRenderer;
import net.minecraft.client.renderer.special.ConduitSpecialRenderer;
import net.minecraft.client.renderer.blockentity.ConduitRenderer;
import net.minecraft.client.renderer.special.DecoratedPotSpecialRenderer;
import net.minecraft.client.renderer.special.TaczGlock17SpecialRenderer;
import net.minecraft.client.renderer.blockentity.DecoratedPotRenderer;
import net.minecraft.world.level.block.entity.PotDecorations;
import net.minecraft.client.model.SkullModelBase;
import net.minecraft.client.renderer.PlayerSkinRenderCache;
import net.minecraft.client.renderer.blockentity.SkullBlockRenderer;
import net.minecraft.client.resources.DefaultPlayerSkin;
import net.minecraft.client.renderer.blockentity.BannerRenderer;
import net.minecraft.client.model.BannerModel;
import net.minecraft.client.model.BannerFlagModel;
import net.minecraft.client.resources.model.ModelBakery;
import net.minecraft.core.component.DataComponentMap;
import net.minecraft.core.component.DataComponents;
import net.minecraft.world.level.block.entity.BannerPatternLayers;
import net.minecraft.world.item.DyeColor;
import net.math.Axis;
import net.minecraft.client.renderer.blockentity.HangingSignRenderer;
import net.minecraft.client.renderer.blockentity.SignRenderer;
import net.minecraft.world.item.ItemStack;
import net.minecraft.world.level.block.Blocks;
import net.vulkanic.bridge.VulkanicGalBridge;
import net.vulkanic.bridge.RustGalFrameScheduler;
import net.sodium.client.model.quad.BakedQuadView;
import org.jetbrains.annotations.Nullable;
import org.joml.Matrix3x2f;
import org.joml.Matrix4f;
import org.joml.Vector3f;

/**
 * Java-side semantic extraction for the bounded flat GUI-item subset. It
 * copies sprite pixels and screen-space quads immediately; no item renderer,
 * atlas object, or backend state crosses into Rust.
 */
public final class RustGalGuiItemRenderer {
	private static final String PRODUCER = "minecraft.gui.item.flat";
	private static final boolean STANDARD_3D_ROUTE_DISABLED = Boolean.getBoolean("mattmc.rustGal.gui.standard3d.disabled");
	private static final boolean DEBUG_STANDARD_3D_ITEM_ENABLED = Boolean.getBoolean("mattmc.rustGal.gui.standard3d.debugItem");
	private static final int MAX_DIAGNOSTIC_ENTRIES = 256;
	private static final Map<String, Boolean> DIAGNOSTICS = new HashMap<>();

	private RustGalGuiItemRenderer() {
	}

	/**
	 * Copies the bounded vanilla trident special model into the same Rust GUI
	 * mesh family used by picture-in-picture models.  The special renderer is
	 * inspected only as a Java semantic producer; no renderer, model, or GPU
	 * object is retained after this call.
	 */
	public static List<RustGalGuiElementRenderState> tryEnqueueSpecialItem(
		GuiItemRenderState item, int guiWidth, int guiHeight, @Nullable Integer dynamicLayerOrder
	) {
		if (RustGalGuiRenderer.currentExecutionRoute() != RustGalGuiRenderer.GuiExecutionRoute.RUST_VULKAN_WHOLE_FRAME
			|| !item.itemStackRenderState().hasSpecialRenderer()) return List.of();
		final ItemStackRenderState.SpecialRender[] selected = new ItemStackRenderState.SpecialRender[1];
		final boolean[] foil = new boolean[1];
		item.itemStackRenderState().forEachSemanticLayer(layer -> {
			if (layer.foilType() != ItemStackRenderState.FoilType.NONE) foil[0] = true;
		});
		item.itemStackRenderState().forEachSpecialRenderer(render -> {
			if (selected[0] == null) selected[0] = render;
		});
		Object selectedRenderer = selected[0] == null ? null : (Object)selected[0].renderer();
		if (selected[0] == null || (foil[0]
			&& !(selectedRenderer instanceof TaczGlock17SpecialRenderer)
			&& !(selectedRenderer instanceof TridentSpecialRenderer)
			&& !(selectedRenderer instanceof ShieldSpecialRenderer)
			&& !(selectedRenderer instanceof BannerSpecialRenderer)
			&& !(selectedRenderer instanceof SkullSpecialRenderer)
			&& !(selectedRenderer instanceof PlayerHeadSpecialRenderer)
			&& !(selectedRenderer instanceof ShulkerBoxSpecialRenderer)
			&& !(selectedRenderer instanceof BedSpecialRenderer)
			&& !(selectedRenderer instanceof ChestSpecialRenderer)
			&& !(selectedRenderer instanceof HangingSignSpecialRenderer)
			&& !(selectedRenderer instanceof StandingSignSpecialRenderer)
			&& !(selectedRenderer instanceof CopperGolemStatueSpecialRenderer)
			&& !(selectedRenderer instanceof ConduitSpecialRenderer)
			&& !(selectedRenderer instanceof DecoratedPotSpecialRenderer))) {
			recordDiagnostic(selected[0] == null ? "special-renderer-empty" : foil[0] ? "special-renderer-foil" : "special-renderer-unavailable");
			return List.of();
		}
		ScreenRectangle bounds = item.bounds();
		int left = bounds == null ? item.x() : bounds.left();
		int top = bounds == null ? item.y() : bounds.top();
		int right = bounds == null ? item.x() + 16 : bounds.right();
		int bottom = bounds == null ? item.y() + 16 : bounds.bottom();
		List<RustGalGuiElementRenderState> result;
		String rejectionName;
		if (selectedRenderer instanceof TridentSpecialRenderer trident) {
			List<RustGalGuiElementRenderState> tridentBase = RustGalGuiRenderer.tryEnqueueModelPip(
				trident.model(), TridentModel.TEXTURE, left, top, right, bottom, 1.0F,
				item.pose(), item.scissorArea(), dynamicLayerOrder,
				pose -> {
					pose.mulPose(selected[0].transform().pose());
					pose.scale(1.0F, -1.0F, -1.0F);
				}
			);
			if (foil[0]) {
				List<RustGalGuiElementRenderState> tridentGlint = RustGalGuiRenderer.tryEnqueueModelPip(
					trident.model(), ItemRenderer.ENCHANTED_GLINT_ITEM, left, top, right, bottom, 1.0F,
					item.pose(), item.scissorArea(), dynamicLayerOrder,
					pose -> {
						pose.mulPose(selected[0].transform().pose());
						pose.scale(1.0F, -1.0F, -1.0F);
					}, 0xffffffff, 4
				);
				List<RustGalGuiElementRenderState> combined = new ArrayList<>();
				if (tridentBase != null) combined.addAll(tridentBase);
				if (tridentGlint != null) combined.addAll(tridentGlint);
				result = combined;
			} else {
				result = tridentBase;
			}
			rejectionName = "special-renderer-trident-rejected";
		} else if (selectedRenderer instanceof ChestSpecialRenderer chest) {
			ChestModel model = chest.model();
			model.setupAnim(chest.openness());
			result = RustGalGuiRenderer.tryEnqueueModelPip(
				model, chest.material().texture(), left, top, right, bottom, 1.0F,
				item.pose(), item.scissorArea(), dynamicLayerOrder,
				pose -> pose.mulPose(selected[0].transform().pose())
			);
			if (foil[0]) {
				List<RustGalGuiElementRenderState> glint = RustGalGuiRenderer.tryEnqueueModelPip(
					model, ItemRenderer.ENCHANTED_GLINT_ITEM, left, top, right, bottom, 1.0F,
					item.pose(), item.scissorArea(), dynamicLayerOrder,
					pose -> {
						model.setupAnim(chest.openness());
						pose.mulPose(selected[0].transform().pose());
					}, 0xffffffff, 4
				);
				if (glint != null) {
					List<RustGalGuiElementRenderState> combined = new ArrayList<>(result == null ? List.of() : result);
					combined.addAll(glint);
					result = combined;
				}
			}
			rejectionName = "special-renderer-chest-rejected";
		} else if (selectedRenderer instanceof HangingSignSpecialRenderer hangingSign) {
			result = RustGalGuiRenderer.tryEnqueueModelPip(
				hangingSign.model(), hangingSign.material().texture(), left, top, right, bottom, 1.0F,
				item.pose(), item.scissorArea(), dynamicLayerOrder,
				pose -> {
					HangingSignRenderer.translateBase(pose, 0.0F);
					pose.scale(1.0F, -1.0F, -1.0F);
					pose.mulPose(selected[0].transform().pose());
				}
			);
			if (foil[0]) {
				List<RustGalGuiElementRenderState> glint = RustGalGuiRenderer.tryEnqueueModelPip(
					hangingSign.model(), ItemRenderer.ENCHANTED_GLINT_ITEM, left, top, right, bottom, 1.0F,
					item.pose(), item.scissorArea(), dynamicLayerOrder,
					pose -> {
						HangingSignRenderer.translateBase(pose, 0.0F);
						pose.scale(1.0F, -1.0F, -1.0F);
						pose.mulPose(selected[0].transform().pose());
					}, 0xffffffff, 4
				);
				if (glint != null) {
					List<RustGalGuiElementRenderState> combined = new ArrayList<>(result == null ? List.of() : result);
					combined.addAll(glint);
					result = combined;
				}
			}
			rejectionName = "special-renderer-hanging-sign-rejected";
		} else if (selectedRenderer instanceof StandingSignSpecialRenderer standingSign) {
			result = RustGalGuiRenderer.tryEnqueueModelPip(
				standingSign.model(), standingSign.material().texture(), left, top, right, bottom, 1.0F,
				item.pose(), item.scissorArea(), dynamicLayerOrder,
				pose -> {
					SignRenderer.applyInHandTransforms(pose);
					pose.mulPose(selected[0].transform().pose());
				}
			);
			if (foil[0]) {
				List<RustGalGuiElementRenderState> glint = RustGalGuiRenderer.tryEnqueueModelPip(
					standingSign.model(), ItemRenderer.ENCHANTED_GLINT_ITEM, left, top, right, bottom, 1.0F,
					item.pose(), item.scissorArea(), dynamicLayerOrder,
					pose -> {
						SignRenderer.applyInHandTransforms(pose);
						pose.mulPose(selected[0].transform().pose());
					}, 0xffffffff, 4
				);
				if (glint != null) {
					List<RustGalGuiElementRenderState> combined = new ArrayList<>(result == null ? List.of() : result);
					combined.addAll(glint);
					result = combined;
				}
			}
			rejectionName = "special-renderer-standing-sign-rejected";
		} else if (selectedRenderer instanceof CopperGolemStatueSpecialRenderer copperGolem) {
			result = RustGalGuiRenderer.tryEnqueueModelPip(
				copperGolem.model(), copperGolem.texture(), left, top, right, bottom, 1.0F,
				item.pose(), item.scissorArea(), dynamicLayerOrder,
				pose -> {
					pose.translate(0.5F, 1.5F, 0.5F);
					pose.scale(-1.0F, -1.0F, 1.0F);
					pose.mulPose(selected[0].transform().pose());
				}
			);
			if (foil[0]) {
				List<RustGalGuiElementRenderState> glint = RustGalGuiRenderer.tryEnqueueModelPip(
					copperGolem.model(), ItemRenderer.ENCHANTED_GLINT_ITEM, left, top, right, bottom, 1.0F,
					item.pose(), item.scissorArea(), dynamicLayerOrder,
					pose -> {
						pose.translate(0.5F, 1.5F, 0.5F);
						pose.scale(-1.0F, -1.0F, 1.0F);
						pose.mulPose(selected[0].transform().pose());
					}, 0xffffffff, 4
				);
				if (glint != null) {
					List<RustGalGuiElementRenderState> combined = new ArrayList<>(result == null ? List.of() : result);
					combined.addAll(glint);
					result = combined;
				}
			}
			rejectionName = "special-renderer-copper-golem-rejected";
		} else if (selectedRenderer instanceof BedSpecialRenderer bed) {
			List<RustGalGuiElementRenderState> pieces = new ArrayList<>(2);
			for (int piece = 0; piece < 2; piece++) {
				boolean foot = piece == 1;
				List<RustGalGuiElementRenderState> pieceElements = RustGalGuiRenderer.tryEnqueueModelPip(
					foot ? bed.footModel() : bed.headModel(), bed.material().texture(), left, top, right, bottom, 1.0F,
					item.pose(), item.scissorArea(), dynamicLayerOrder,
					pose -> {
						pose.translate(0.0F, 0.5625F, foot ? -1.0F : 0.0F);
						pose.mulPose(Axis.XP.rotationDegrees(90.0F));
						pose.translate(0.5F, 0.5F, 0.5F);
						pose.mulPose(Axis.ZP.rotationDegrees(180.0F));
						pose.translate(-0.5F, -0.5F, -0.5F);
						pose.mulPose(selected[0].transform().pose());
					}
				);
				if (pieceElements != null) pieces.addAll(pieceElements);
			}
			if (foil[0]) {
				for (int piece = 0; piece < 2; piece++) {
					boolean foot = piece == 1;
					List<RustGalGuiElementRenderState> glint = RustGalGuiRenderer.tryEnqueueModelPip(
						foot ? bed.footModel() : bed.headModel(), ItemRenderer.ENCHANTED_GLINT_ITEM,
						left, top, right, bottom, 1.0F, item.pose(), item.scissorArea(), dynamicLayerOrder,
						pose -> {
							pose.translate(0.0F, 0.5625F, foot ? -1.0F : 0.0F);
							pose.mulPose(Axis.XP.rotationDegrees(90.0F));
							pose.translate(0.5F, 0.5F, 0.5F);
							pose.mulPose(Axis.ZP.rotationDegrees(180.0F));
							pose.translate(-0.5F, -0.5F, -0.5F);
							pose.mulPose(selected[0].transform().pose());
						}, 0xffffffff, 4
					);
					if (glint != null) pieces.addAll(glint);
				}
			}
			result = pieces;
			rejectionName = "special-renderer-bed-rejected";
		} else if (selectedRenderer instanceof ShieldSpecialRenderer shield) {
			DataComponentMap components = selected[0].argument() instanceof DataComponentMap map ? map : null;
			BannerPatternLayers patterns = components == null
				? BannerPatternLayers.EMPTY
				: components.getOrDefault(DataComponents.BANNER_PATTERNS, BannerPatternLayers.EMPTY);
			DyeColor baseColor = components == null ? null : components.get(DataComponents.BASE_COLOR);
			List<RustGalGuiElementRenderState> shieldLayers = new ArrayList<>();
			int baseTint = baseColor == null ? 0xffffffff : baseColor.getTextureDiffuseColor();
			List<RustGalGuiElementRenderState> baseLayer = RustGalGuiRenderer.tryEnqueueModelPip(
				shield.model(), (patterns.layers().isEmpty() && baseColor == null
					? ModelBakery.NO_PATTERN_SHIELD : ModelBakery.SHIELD_BASE).texture(),
				left, top, right, bottom, 1.0F, item.pose(), item.scissorArea(), dynamicLayerOrder,
				pose -> {
					pose.scale(1.0F, -1.0F, -1.0F);
					pose.mulPose(selected[0].transform().pose());
				}, 0xffffffff);
			if (baseLayer != null) shieldLayers.addAll(baseLayer);
			if (!patterns.layers().isEmpty() || baseColor != null) {
				List<RustGalGuiElementRenderState> dyedBase = RustGalGuiRenderer.tryEnqueueModelPip(
					shield.model(), ModelBakery.SHIELD_BASE.texture(), left, top, right, bottom, 1.0F,
					item.pose(), item.scissorArea(), dynamicLayerOrder,
					pose -> {
						pose.scale(1.0F, -1.0F, -1.0F);
						pose.mulPose(selected[0].transform().pose());
					}, baseTint);
				if (dyedBase != null) shieldLayers.addAll(dyedBase);
				for (BannerPatternLayers.Layer layer : patterns.layers().subList(0, Math.min(16, patterns.layers().size()))) {
					List<RustGalGuiElementRenderState> patternLayer = RustGalGuiRenderer.tryEnqueueModelPip(
						shield.model(), net.minecraft.client.renderer.Sheets.getShieldMaterial(layer.pattern()).texture(),
						left, top, right, bottom, 1.0F, item.pose(), item.scissorArea(), dynamicLayerOrder,
						pose -> {
							pose.scale(1.0F, -1.0F, -1.0F);
							pose.mulPose(selected[0].transform().pose());
						}, layer.color().getTextureDiffuseColor());
					if (patternLayer != null) shieldLayers.addAll(patternLayer);
				}
			}
			if (foil[0]) {
				List<RustGalGuiElementRenderState> glint = RustGalGuiRenderer.tryEnqueueModelPip(
					shield.model(), ItemRenderer.ENCHANTED_GLINT_ITEM, left, top, right, bottom, 1.0F,
					item.pose(), item.scissorArea(), dynamicLayerOrder,
					pose -> {
						pose.scale(1.0F, -1.0F, -1.0F);
						pose.mulPose(selected[0].transform().pose());
					}, 0xffffffff, 4
				);
				if (glint != null) shieldLayers.addAll(glint);
			}
			result = shieldLayers;
			rejectionName = "special-renderer-shield-rejected";
		} else if (selectedRenderer instanceof BannerSpecialRenderer banner) {
			BannerRenderer renderer = banner.bannerRenderer();
			BannerModel model = renderer.standingModel();
			BannerFlagModel flagModel = renderer.standingFlagModel();
			BannerPatternLayers patterns = selected[0].argument() instanceof BannerPatternLayers layers
				? layers : BannerPatternLayers.EMPTY;
			List<RustGalGuiElementRenderState> bannerLayers = new ArrayList<>();
			List<RustGalGuiElementRenderState> base = RustGalGuiRenderer.tryEnqueueModelPip(
				model, ModelBakery.BANNER_BASE.texture(), left, top, right, bottom, 1.0F,
				item.pose(), item.scissorArea(), dynamicLayerOrder,
				pose -> {
					pose.translate(0.5F, 0.0F, 0.5F);
					pose.scale(0.6666667F, -0.6666667F, -0.6666667F);
					pose.mulPose(selected[0].transform().pose());
				}
			);
			if (base != null) bannerLayers.addAll(base);
			int baseTint = banner.baseColor().getTextureDiffuseColor();
			List<RustGalGuiElementRenderState> flagBase = RustGalGuiRenderer.tryEnqueueModelPip(
				flagModel, ModelBakery.BANNER_BASE.texture(), left, top, right, bottom, 1.0F,
				item.pose(), item.scissorArea(), dynamicLayerOrder,
				pose -> {
					pose.translate(0.5F, 0.0F, 0.5F);
					pose.scale(0.6666667F, -0.6666667F, -0.6666667F);
					pose.mulPose(selected[0].transform().pose());
				}, baseTint
			);
			if (flagBase != null) bannerLayers.addAll(flagBase);
			for (BannerPatternLayers.Layer layer : patterns.layers().subList(0, Math.min(16, patterns.layers().size()))) {
				List<RustGalGuiElementRenderState> pattern = RustGalGuiRenderer.tryEnqueueModelPip(
					flagModel, net.minecraft.client.renderer.Sheets.getBannerMaterial(layer.pattern()).texture(),
					left, top, right, bottom, 1.0F, item.pose(), item.scissorArea(), dynamicLayerOrder,
					pose -> {
						pose.translate(0.5F, 0.0F, 0.5F);
						pose.scale(0.6666667F, -0.6666667F, -0.6666667F);
						pose.mulPose(selected[0].transform().pose());
					}, layer.color().getTextureDiffuseColor()
				);
				if (pattern != null) bannerLayers.addAll(pattern);
			}
			if (foil[0]) {
				List<RustGalGuiElementRenderState> glint = RustGalGuiRenderer.tryEnqueueModelPip(
					flagModel, ItemRenderer.ENCHANTED_GLINT_ITEM, left, top, right, bottom, 1.0F,
					item.pose(), item.scissorArea(), dynamicLayerOrder,
					pose -> {
						pose.translate(0.5F, 0.0F, 0.5F);
						pose.scale(0.6666667F, -0.6666667F, -0.6666667F);
						pose.mulPose(selected[0].transform().pose());
					}, 0xffffffff, 4
				);
				if (glint != null) bannerLayers.addAll(glint);
			}
			result = bannerLayers;
			rejectionName = "special-renderer-banner-rejected";
		} else if (selectedRenderer instanceof SkullSpecialRenderer skull) {
			ResourceLocation texture = skull.texture();
			if (texture == null) {
				recordDiagnostic("special-renderer-skull-texture-unavailable");
				return List.of();
			}
			result = RustGalGuiRenderer.tryEnqueueModelPip(
				skull.model(), texture, left, top, right, bottom, 1.0F,
				item.pose(), item.scissorArea(), dynamicLayerOrder,
				pose -> {
					pose.translate(0.5F, 0.0F, 0.5F);
					pose.scale(-1.0F, -1.0F, 1.0F);
					SkullModelBase.State state = new SkullModelBase.State();
					state.animationPos = skull.animation();
					state.yRot = 180.0F;
					 skull.model().setupAnim(state);
					pose.mulPose(selected[0].transform().pose());
				}
			);
			if (foil[0]) {
				List<RustGalGuiElementRenderState> glint = RustGalGuiRenderer.tryEnqueueModelPip(
					skull.model(), ItemRenderer.ENCHANTED_GLINT_ITEM, left, top, right, bottom, 1.0F,
					item.pose(), item.scissorArea(), dynamicLayerOrder,
					pose -> {
						pose.translate(0.5F, 0.0F, 0.5F);
						pose.scale(-1.0F, -1.0F, 1.0F);
						SkullModelBase.State state = new SkullModelBase.State();
						state.animationPos = skull.animation();
						state.yRot = 180.0F;
						skull.model().setupAnim(state);
						pose.mulPose(selected[0].transform().pose());
					}, 0xffffffff, 4
				);
				if (glint != null) {
					List<RustGalGuiElementRenderState> combined = new ArrayList<>(result == null ? List.of() : result);
					combined.addAll(glint);
					result = combined;
				}
			}
			rejectionName = "special-renderer-skull-rejected";
		} else if (selectedRenderer instanceof PlayerHeadSpecialRenderer playerHead) {
			ResourceLocation texture = DefaultPlayerSkin.getDefaultTexture();
			if (selected[0].argument() instanceof PlayerSkinRenderCache.RenderInfo info) {
				texture = info.playerSkin().body().texturePath();
			}
			result = RustGalGuiRenderer.tryEnqueueModelPip(
				playerHead.model(), texture, left, top, right, bottom, 1.0F,
				item.pose(), item.scissorArea(), dynamicLayerOrder,
				pose -> {
					pose.translate(0.5F, 0.0F, 0.5F);
					pose.scale(-1.0F, -1.0F, 1.0F);
					SkullModelBase.State state = new SkullModelBase.State();
					state.yRot = 180.0F;
					playerHead.model().setupAnim(state);
					pose.mulPose(selected[0].transform().pose());
				}
			);
			if (foil[0]) {
				List<RustGalGuiElementRenderState> glint = RustGalGuiRenderer.tryEnqueueModelPip(
					playerHead.model(), ItemRenderer.ENCHANTED_GLINT_ITEM, left, top, right, bottom, 1.0F,
					item.pose(), item.scissorArea(), dynamicLayerOrder,
					pose -> {
						pose.translate(0.5F, 0.0F, 0.5F);
						pose.scale(-1.0F, -1.0F, 1.0F);
						SkullModelBase.State state = new SkullModelBase.State();
						state.yRot = 180.0F;
						playerHead.model().setupAnim(state);
						pose.mulPose(selected[0].transform().pose());
					}, 0xffffffff, 4
				);
				if (glint != null) {
					List<RustGalGuiElementRenderState> combined = new ArrayList<>(result == null ? List.of() : result);
					combined.addAll(glint);
					result = combined;
				}
			}
			rejectionName = "special-renderer-player-head-rejected";
		} else if (selectedRenderer instanceof ShulkerBoxSpecialRenderer shulker) {
			ShulkerBoxRenderer renderer = shulker.renderer();
			ShulkerBoxRenderer.ShulkerBoxModel model = renderer.model();
			model.setupAnim(shulker.openness());
			result = RustGalGuiRenderer.tryEnqueueModelPip(
				model, shulker.material().texture(), left, top, right, bottom, 1.0F,
				item.pose(), item.scissorArea(), dynamicLayerOrder,
				pose -> {
					pose.translate(0.5F, 0.5F, 0.5F);
					pose.scale(0.9995F, 0.9995F, 0.9995F);
					pose.mulPose(shulker.orientation().getRotation());
					pose.scale(1.0F, -1.0F, -1.0F);
					pose.translate(0.0F, -1.0F, 0.0F);
					pose.mulPose(selected[0].transform().pose());
				}
			);
			if (foil[0]) {
				List<RustGalGuiElementRenderState> glint = RustGalGuiRenderer.tryEnqueueModelPip(
					model, ItemRenderer.ENCHANTED_GLINT_ITEM, left, top, right, bottom, 1.0F,
					item.pose(), item.scissorArea(), dynamicLayerOrder,
					pose -> {
						pose.translate(0.5F, 0.5F, 0.5F);
						pose.scale(0.9995F, 0.9995F, 0.9995F);
						pose.mulPose(shulker.orientation().getRotation());
						pose.scale(1.0F, -1.0F, -1.0F);
						pose.translate(0.0F, -1.0F, 0.0F);
						pose.mulPose(selected[0].transform().pose());
					}, 0xffffffff, 4
				);
				if (glint != null) {
					List<RustGalGuiElementRenderState> combined = new ArrayList<>(result == null ? List.of() : result);
					combined.addAll(glint);
					result = combined;
				}
			}
			rejectionName = "special-renderer-shulker-rejected";
		} else if (selectedRenderer instanceof ConduitSpecialRenderer conduit) {
			result = RustGalGuiRenderer.tryEnqueueModelPartPip(
				conduit.model(), ConduitRenderer.SHELL_TEXTURE.texture(), left, top, right, bottom, 1.0F,
				item.pose(), item.scissorArea(), dynamicLayerOrder,
				pose -> {
					pose.translate(0.5F, 0.5F, 0.5F);
					pose.mulPose(selected[0].transform().pose());
				}
			);
			if (foil[0]) {
				List<RustGalGuiElementRenderState> glint = RustGalGuiRenderer.tryEnqueueModelPip(
					new Model.Simple(conduit.model(), RenderType::entitySolid), ItemRenderer.ENCHANTED_GLINT_ITEM,
					left, top, right, bottom, 1.0F, item.pose(), item.scissorArea(), dynamicLayerOrder,
					pose -> {
						pose.translate(0.5F, 0.5F, 0.5F);
						pose.mulPose(selected[0].transform().pose());
					}, 0xffffffff, 4
				);
				if (glint != null) {
					List<RustGalGuiElementRenderState> combined = new ArrayList<>(result == null ? List.of() : result);
					combined.addAll(glint);
					result = combined;
				}
			}
			rejectionName = "special-renderer-conduit-rejected";
		} else if (selectedRenderer instanceof DecoratedPotSpecialRenderer pot) {
			DecoratedPotRenderer renderer = pot.renderer();
			PotDecorations decorations = selected[0].argument() instanceof PotDecorations value
				? value : PotDecorations.EMPTY;
			List<RustGalGuiElementRenderState> potParts = new ArrayList<>(7);
			List<RustGalGuiElementRenderState> baseParts = enqueueModelParts(
				List.of(renderer.neckPart(), renderer.topPart(), renderer.bottomPart()),
				net.minecraft.client.renderer.Sheets.DECORATED_POT_BASE.texture(), left, top, right, bottom, item, dynamicLayerOrder, selected[0]
			);
			if (baseParts != null) potParts.addAll(baseParts);
			List<RustGalGuiElementRenderState> front = enqueueModelPart(
				renderer.frontSidePart(), renderer.sideMaterial(decorations.front()).texture(), left, top, right, bottom,
				item, dynamicLayerOrder, selected[0]
			);
			if (front != null) potParts.addAll(front);
			List<RustGalGuiElementRenderState> back = enqueueModelPart(
				renderer.backSidePart(), renderer.sideMaterial(decorations.back()).texture(), left, top, right, bottom,
				item, dynamicLayerOrder, selected[0]
			);
			if (back != null) potParts.addAll(back);
			List<RustGalGuiElementRenderState> leftSide = enqueueModelPart(
				renderer.leftSidePart(), renderer.sideMaterial(decorations.left()).texture(), left, top, right, bottom,
				item, dynamicLayerOrder, selected[0]
			);
			if (leftSide != null) potParts.addAll(leftSide);
			List<RustGalGuiElementRenderState> rightSide = enqueueModelPart(
				renderer.rightSidePart(), renderer.sideMaterial(decorations.right()).texture(), left, top, right, bottom,
				item, dynamicLayerOrder, selected[0]
			);
			if (rightSide != null) potParts.addAll(rightSide);
			if (foil[0]) {
				List<net.minecraft.client.model.geom.ModelPart> glintParts = List.of(
					renderer.neckPart(), renderer.topPart(), renderer.bottomPart(), renderer.frontSidePart(),
					renderer.backSidePart(), renderer.leftSidePart(), renderer.rightSidePart());
				for (net.minecraft.client.model.geom.ModelPart part : glintParts) {
					List<RustGalGuiElementRenderState> glint = RustGalGuiRenderer.tryEnqueueModelPip(
						new Model.Simple(part, RenderType::entitySolid), ItemRenderer.ENCHANTED_GLINT_ITEM,
						left, top, right, bottom, 1.0F, item.pose(), item.scissorArea(), dynamicLayerOrder,
						pose -> pose.mulPose(selected[0].transform().pose()), 0xffffffff, 4
					);
					if (glint != null) potParts.addAll(glint);
				}
			}
			result = potParts;
			rejectionName = "special-renderer-decorated-pot-rejected";
		} else if (selectedRenderer instanceof TaczGlock17SpecialRenderer tacz) {
			result = tryEnqueueTaczItem(tacz, selected[0], item, foil[0], left, top, right, bottom, dynamicLayerOrder);
			rejectionName = "special-renderer-tacz-rejected";
		} else {
			recordDiagnostic("special-renderer-unavailable");
			return List.of();
		}
		if (result == null || result.isEmpty()) recordDiagnostic(rejectionName);
		return result == null ? List.of() : result;
	}

	/**
	 * Captures TACZ's already-semantic Bedrock quad stream into a GUI mesh. The
	 * collector accepts only explicit textured quad batches; arbitrary callbacks
	 * remain rejected by the existing whole-frame boundary.
	 */
	private static List<RustGalGuiElementRenderState> tryEnqueueTaczItem(
		TaczGlock17SpecialRenderer renderer,
		ItemStackRenderState.SpecialRender selected,
		GuiItemRenderState item,
		boolean foil,
		int left,
		int top,
		int right,
		int bottom,
		@Nullable Integer dynamicLayerOrder
	) {
		TaczGuiQuadCapture capture = new TaczGuiQuadCapture();
		TaczGuiSubmitCollector collector = new TaczGuiSubmitCollector(capture);
		PoseStack poseStack = new PoseStack();
		poseStack.last().pose().set(selected.transform().pose());
		try {
			renderer.submit(ItemDisplayContext.GUI, poseStack, collector, 15728880, 0, false, 0);
		} catch (RuntimeException error) {
			recordDiagnostic("special-renderer-tacz-rejected=" + error.getClass().getSimpleName());
			return List.of();
		}
		return capture.enqueue(item, foil, left, top, right, bottom, dynamicLayerOrder);
	}

	private static final class TaczGuiQuadCapture {
		private static final int MAX_QUADS = 4096;
		private final List<Batch> batches = new ArrayList<>();

		private boolean add(ResourceLocation texture, float[] vertices, float[] uvs, int[] colors) {
			if (texture == null || vertices == null || uvs == null || colors == null) {
				recordDiagnostic("tacz-quad-missing-array");
				return false;
			}
			int quadCount = vertices.length / 12;
			if (vertices.length == 0 || vertices.length % 12 != 0 || uvs.length != vertices.length / 3 * 2
				|| (colors.length != quadCount && colors.length != quadCount * 4)
				|| quadCount + totalQuads() > MAX_QUADS) {
				recordDiagnostic("tacz-quad-shape=" + vertices.length + "/" + uvs.length + "/" + colors.length);
				return false;
			}
			RustGalGuiRawImageAssets.Asset asset = RustGalGuiRawImageAssets.resolve(texture);
			if (asset == null && "minecraft".equals(texture.getNamespace())) {
				asset = RustGalGuiRawImageAssets.loadVanillaResource(texture,
					Minecraft.getInstance().getVanillaPackResources().asProvider());
			}
			if (asset == null) {
				recordDiagnostic("tacz-asset-missing=" + texture);
				return false;
			}
			int[] perVertexColors = colors.length == quadCount * 4 ? colors.clone() : new int[quadCount * 4];
			if (colors.length == quadCount) {
				for (int quad = 0; quad < quadCount; quad++) {
					for (int vertex = 0; vertex < 4; vertex++) perVertexColors[quad * 4 + vertex] = colors[quad];
				}
			}
			batches.add(new Batch(asset, vertices.clone(), uvs.clone(), perVertexColors));
			return true;
		}

		private int totalQuads() { return batches.stream().mapToInt(batch -> batch.vertices.length / 12).sum(); }

		private List<RustGalGuiElementRenderState> enqueue(
			GuiItemRenderState item, boolean foil, int left, int top, int right, int bottom, @Nullable Integer dynamicLayerOrder
		) {
			if (batches.isEmpty()) return List.of();
			int guiWidth = Minecraft.getInstance().getWindow().getGuiScaledWidth();
			int guiHeight = Minecraft.getInstance().getWindow().getGuiScaledHeight();
			int guiScale = Math.max(1, Minecraft.getInstance().getWindow().getGuiScale());
			int width = Math.max(2, (right - left) * guiScale + 2);
			int height = Math.max(2, (bottom - top) * guiScale + 2);
			Matrix4f transform = new Matrix4f().translate(width / 2.0F, height / 2.0F, 0.0F)
				.scale(guiScale * 16.0F, guiScale * 16.0F, -guiScale * 16.0F);
			RustGalGuiRawImageAssets.Asset glintAsset = null;
			int glintColor = 0xffffffff;
			if (foil) {
				glintAsset = RustGalGuiRawImageAssets.resolve(ItemRenderer.ENCHANTED_GLINT_ITEM);
				if (glintAsset == null) return List.of();
				int strength = Mth.clamp((int)Math.round(Minecraft.getInstance().options.glintStrength().get() * 255.0F), 0, 255);
				glintColor = ARGB.color(strength, 255, 255, 255);
			}
			List<VulkanicGalBridge.GuiMeshBatchRecord> records = new ArrayList<>(batches.size());
			for (Batch batch : batches) {
				List<VulkanicGalBridge.GuiMeshVertexRecord> copied = new ArrayList<>(batch.vertices.length / 3);
				Vector3f first = transform.transformPosition(batch.vertices[0], batch.vertices[1], batch.vertices[2], new Vector3f());
				Vector3f second = transform.transformPosition(batch.vertices[3], batch.vertices[4], batch.vertices[5], new Vector3f());
				Vector3f third = transform.transformPosition(batch.vertices[6], batch.vertices[7], batch.vertices[8], new Vector3f());
				Vector3f edgeA = new Vector3f(second).sub(first);
				Vector3f edgeB = new Vector3f(third).sub(first);
				Vector3f normal = edgeA.cross(edgeB);
				float normalLength = normal.length();
				if (!Float.isFinite(normalLength) || normalLength <= 1.0e-6F) return List.of();
				normal.mul(1.0F / normalLength);
				int normalPacked = packGuiNormal(normal.x(), normal.y(), normal.z());
				for (int vertex = 0; vertex < batch.vertices.length / 3; vertex++) {
					Vector3f position = transform.transformPosition(batch.vertices[vertex * 3], batch.vertices[vertex * 3 + 1], batch.vertices[vertex * 3 + 2], new Vector3f());
					copied.add(new VulkanicGalBridge.GuiMeshVertexRecord(
						new float[] {position.x, position.y, position.z},
						new float[] {batch.uvs[vertex * 2], batch.uvs[vertex * 2 + 1]},
						new float[] {batch.uvs[vertex * 2], batch.uvs[vertex * 2 + 1]},
						batch.colors[vertex], normalPacked));
				}
				List<Integer> indices = new ArrayList<>(batch.vertices.length / 2);
				for (int vertex = 0; vertex < copied.size(); vertex += 4) {
					indices.add(vertex); indices.add(vertex + 1); indices.add(vertex + 2);
					indices.add(vertex + 2); indices.add(vertex + 3); indices.add(vertex);
				}
				int layerOrder = dynamicLayerOrder == null ? GuiRenderStratum.GUI_ITEM.order() : dynamicLayerOrder;
				records.add(new VulkanicGalBridge.GuiMeshBatchRecord(layerOrder, records.size(), 1, 2,
					batch.asset.assetId(), 0L, 0.0F, identity(), new float[] {
						item.pose().m00(), item.pose().m01(), item.pose().m10(), item.pose().m11(), item.pose().m20(), item.pose().m21()
					},
					left, top, right, bottom, guiWidth, guiHeight, width, height, 1, 0, 0, 0, 0, 0,
					copied, indices));
				if (glintAsset != null) {
					List<VulkanicGalBridge.GuiMeshVertexRecord> glintVertices = new ArrayList<>(copied.size());
					for (int vertex = 0; vertex < copied.size(); vertex++) {
						float x = batch.vertices[vertex * 3];
						float y = batch.vertices[vertex * 3 + 1];
						float u = -x * ItemRenderer.SPECIAL_FOIL_TEXTURE_SCALE;
						float v = -y * ItemRenderer.SPECIAL_FOIL_TEXTURE_SCALE;
						VulkanicGalBridge.GuiMeshVertexRecord source = copied.get(vertex);
						glintVertices.add(new VulkanicGalBridge.GuiMeshVertexRecord(
							source.position(), new float[] {u, v}, new float[] {u, v}, glintColor, source.normalPacked()
						));
					}
					records.add(new VulkanicGalBridge.GuiMeshBatchRecord(layerOrder, records.size(), 4, 1,
						glintAsset.assetId(), 0L, 0.1F, identity(), new float[] {
							item.pose().m00(), item.pose().m01(), item.pose().m10(), item.pose().m11(), item.pose().m20(), item.pose().m21()
						}, left, top, right, bottom, guiWidth, guiHeight, width, height, 1, 0, 0, 0, 0, 0,
						glintVertices, indices));
				}
			}
			RustGalFrameScheduler.Token token = RustGalFrameCoordinator.enqueueGuiMeshItemRequest(records,
				GuiRenderStratum.GUI_ITEM.id(), dynamicLayerOrder == null ? GuiRenderStratum.GUI_ITEM.order() : dynamicLayerOrder, System.nanoTime());
			// Commit copied item/glint images only after the complete special-item
			// mesh has entered the scheduler. Rejected requests must not retain
			// texture assets without corresponding Rust GUI work.
			for (Batch batch : batches) RustGalGuiRawImageAssets.stage(batch.asset());
			if (glintAsset != null) RustGalGuiRawImageAssets.stage(glintAsset);
			return List.of(new RustGalGuiElementRenderState(token, GuiRenderStratum.GUI_ITEM, "minecraft.gui.tacz-bedrock",
				-1, -1.0F, GuiFillDirection.NONE, left, top, right - left, bottom - top, guiWidth, guiHeight));
		}

		private static int packGuiNormal(float x, float y, float z) {
			int px = Math.round(Mth.clamp(x, -1.0F, 1.0F) * 127.0F) & 0xff;
			int py = Math.round(Mth.clamp(y, -1.0F, 1.0F) * 127.0F) & 0xff;
			int pz = Math.round(Mth.clamp(z, -1.0F, 1.0F) * 127.0F) & 0xff;
			return px | (py << 8) | (pz << 16);
		}

		private static float[] identity() { return new float[] {1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1}; }
		private record Batch(RustGalGuiRawImageAssets.Asset asset, float[] vertices, float[] uvs, int[] colors) {}
	}

	private static final class TaczGuiSubmitCollector extends SubmitNodeCollection implements SubmitNodeCollector {
		private final TaczGuiQuadCapture capture;

		private TaczGuiSubmitCollector(TaczGuiQuadCapture capture) {
			super(null);
			this.capture = capture;
		}

		@Override
		public OrderedSubmitNodeCollector order(int ignored) { return this; }

		@Override
		public boolean submitTexturedQuads(PoseStack poseStack, RenderType renderType, ResourceLocation texture,
			float[] vertices, float[] uvs, int[] colors, int lightCoords) {
			return capture.add(texture, vertices, uvs, colors);
		}

		@Override
		public boolean submitTexturedQuad(PoseStack poseStack, RenderType renderType, ResourceLocation texture,
			float[] vertices, float[] uvs, int color, int lightCoords) {
			return capture.add(texture, vertices, uvs, new int[] {color, color, color, color});
		}


		@Override
		public void submitCustomGeometry(PoseStack poseStack, RenderType renderType, SubmitNodeCollector.CustomGeometryRenderer customGeometryRenderer) {
			throw new IllegalStateException("TACZ GUI semantic capture encountered arbitrary custom geometry");
		}
	}

	private static List<RustGalGuiElementRenderState> enqueueModelPart(
		net.minecraft.client.model.geom.ModelPart part, ResourceLocation texture, int left, int top, int right, int bottom,
		GuiItemRenderState item, @Nullable Integer dynamicLayerOrder, ItemStackRenderState.SpecialRender selected
	) {
		return RustGalGuiRenderer.tryEnqueueModelPartPip(
			part, texture, left, top, right, bottom, 1.0F, item.pose(), item.scissorArea(), dynamicLayerOrder,
			pose -> pose.mulPose(selected.transform().pose())
		);
	}

	private static List<RustGalGuiElementRenderState> enqueueModelParts(
		List<net.minecraft.client.model.geom.ModelPart> parts, ResourceLocation texture, int left, int top, int right, int bottom,
		GuiItemRenderState item, @Nullable Integer dynamicLayerOrder, ItemStackRenderState.SpecialRender selected
	) {
		List<RustGalGuiElementRenderState> result = new ArrayList<>();
		for (net.minecraft.client.model.geom.ModelPart part : parts) {
			List<RustGalGuiElementRenderState> elements = enqueueModelPart(part, texture, left, top, right, bottom, item, dynamicLayerOrder, selected);
			if (elements != null) result.addAll(elements);
		}
		return result;
	}

	public static boolean standard3dRouteEnabled() {
		// This is a normal producer route for Rust's exclusive Vulkan frame. An
		// unsupported item remains Java-owned before selection; selected items
		// never enter Java's PIP renderer in the same frame.
		return !STANDARD_3D_ROUTE_DISABLED
			&& RustGalGuiRenderer.currentExecutionRoute() == RustGalGuiRenderer.GuiExecutionRoute.RUST_VULKAN_WHOLE_FRAME;
	}

	/**
	 * Adds one ordinary vanilla GUI item state before whole-frame semantic
	 * collection. This is capture-only infrastructure: it exercises the same
	 * ItemModelResolver and GUI item records as production items, but never
	 * invokes Java's PIP renderer or any Java draw path.
	 */
	public static void enqueueDebugStandard3dItem(GuiRenderState guiRenderState) {
		if (!DEBUG_STANDARD_3D_ITEM_ENABLED || !standard3dRouteEnabled()) {
			return;
		}
		Minecraft minecraft = Minecraft.getInstance();
		if (!minecraft.getModelManager().hasLoadedModels() || minecraft.level == null || minecraft.player == null) {
			return;
		}
		TrackingItemStackRenderState itemState = new TrackingItemStackRenderState();
		minecraft.getItemModelResolver().updateForTopItem(
			itemState, new ItemStack(Blocks.GRASS_BLOCK), ItemDisplayContext.GUI, minecraft.level, minecraft.player, 0
		);
		guiRenderState.submitItem(new GuiItemRenderState(
			"rust_gal_debug_standard_3d_grass_block", new Matrix3x2f(), itemState, 16, 16, null
		));
	}

	/**
	 * Extracts a flat, untransformed GUI item into the existing coarse affine
	 * GUI request family. 3D-lit, transformed, foil, special, and
	 * otherwise non-planar items are explicitly left absent until a Rust-owned
	 * GUI mesh pass exists.
	 */
	public static List<RustGalGuiElementRenderState> tryEnqueueFlatItem(
		GuiItemRenderState item,
		int guiWidth,
		int guiHeight
	) {
		return tryEnqueueFlatItem(item, guiWidth, guiHeight, null);
	}

	public static List<RustGalGuiElementRenderState> tryEnqueueFlatItem(
		GuiItemRenderState item,
		int guiWidth,
		int guiHeight,
		@Nullable Integer dynamicLayerOrder
	) {
		if (RustGalGuiRenderer.currentExecutionRoute() != RustGalGuiRenderer.GuiExecutionRoute.RUST_VULKAN_WHOLE_FRAME) {
			return List.of();
		}
		if (item.itemStackRenderState().displayContext() != ItemDisplayContext.GUI) {
			recordDiagnostic("display-context");
			return List.of();
		}
		List<FlatQuad> quads = new ArrayList<>();
		if (Boolean.getBoolean("mattmc.dev.rustGalGuiItemFoil")
			&& Boolean.getBoolean("mattmc.dev.rustGalGuiAtlasItems")) {
			boolean[] foil = {false};
			item.itemStackRenderState().forEachSemanticLayer(layer -> {
				if (layer.foilType() != ItemStackRenderState.FoilType.NONE) foil[0] = true;
			});
			if (foil[0]) return tryEnqueueNativeFlatFoil(item, guiWidth, guiHeight, dynamicLayerOrder);
		}
		String[] rejected = new String[1];
		List<VulkanicGalBridge.GuiItemRasterLayerRecord> rasterLayers = new ArrayList<>();
		float[] rasterPlane = {Float.NaN};
		item.itemStackRenderState().forEachSemanticLayer(layer -> {
			if (rejected[0] != null) {
				return;
			}
			int first = quads.size();
			rejected[0] = appendSupportedLayer(item, layer, quads);
			if (rejected[0] == null) {
				for (FlatQuad quad : quads.subList(first, quads.size())) {
					if (!(quad.source() instanceof GuiItemTextureSource.Atlas)) continue;
					var geometry = boundedItemRasterGeometry(quad.localX0(), quad.localY0(),
						quad.localX1(), quad.localY1(), quad.localX3(), quad.localY3());
					try {
						if (geometry == null) throw new IllegalArgumentException("unbounded geometry");
						rasterLayers.add(new VulkanicGalBridge.GuiItemRasterLayerRecord(quad.source().assetId(),
							quad.colorArgb(), flatItemMaterial(layer.renderType().pipeline()), geometry,
							quad.u0(), quad.v0(), quad.u1(), quad.v1(), layer.modelTransform()));
					} catch (IllegalArgumentException invalid) {
						rejected[0] = "private-item-raster-invalid-layer";
						return;
					}
				}
				if (rasterLayers.size() > 64) { rejected[0] = "private-item-raster-layer-bound"; return; }
				if (quads.size() > first && quads.get(first).source() instanceof GuiItemTextureSource.Atlas) {
					for (BakedQuad face : layer.quads()) {
						if (face.direction() != net.minecraft.core.Direction.SOUTH) continue;
						BakedQuadView view = (BakedQuadView)(Object)face;
						for (int vertex = 0; vertex < 4; vertex++) {
							float z = view.getZ(vertex);
							if (!Float.isFinite(z) || (!Float.isNaN(rasterPlane[0]) && z != rasterPlane[0])) {
								rejected[0] = "private-item-raster-noncoplanar-layers";
								return;
							}
							rasterPlane[0] = z;
						}
					}
				}
			}
		});
		if (rejected[0] != null || quads.isEmpty()) {
			recordDiagnostic(rejected[0] == null ? "empty-flat-geometry" : rejected[0]);
			return List.of();
		}

		boolean atlasItem = !rasterLayers.isEmpty();
		if (atlasItem) {
			if (rasterLayers.size() != quads.size()) {
				recordDiagnostic("private-item-raster-mixed-source-layers");
				return List.of();
			}
			// Only admitted item draws publish use. Rust selects and uploads the
			// animation frame; this callsite supplies no Java ticker/GPU state.
			item.itemStackRenderState().forEachSemanticLayer(layer -> {
				for (BakedQuad face : layer.quads()) {
					if (face.direction() == net.minecraft.core.Direction.SOUTH) {
						TextureAtlasSprite sprite = ((BakedQuadView)(Object)face).getSprite();
						net.vulkanic.world.RustGalWorldPrimitiveRenderer.recordAtlasSpriteUse(
							sprite.semanticAnimationResource(), sprite.atlasLocation(), sprite.contents().name());
					}
				}
			});
		}
		List<RustGalGuiElementRenderState> elements = new ArrayList<>(quads.size());
		long startedNanos = System.nanoTime();
		int requestLayerOrder = dynamicLayerOrder == null ? GuiRenderStratum.GUI_ITEM.order()
			: RustGalGuiRenderer.dynamicLayerOrder(dynamicLayerOrder);
		for (FlatQuad quad : atlasItem ? List.of(quads.getFirst()) : quads) {
			// Composite the whole item cell; the separate authored local corners
			// determine which part of that cell Rust rasterizes.
			float[] p0 = atlasItem ? guiPoint(item, 0, 1) : new float[] {quad.x0(),quad.y0()};
			float[] p1 = atlasItem ? guiPoint(item, 1, 1) : new float[] {quad.x1(),quad.y1()};
			float[] p3 = atlasItem ? guiPoint(item, 0, 0) : new float[] {quad.x3(),quad.y3()};
			VulkanicGalBridge.GuiAffineQuadRecord request = new VulkanicGalBridge.GuiAffineQuadRecord(
				requestLayerOrder,
				quad.source().assetId(),
				p0[0], p0[1], p1[0], p1[1], p3[0], p3[1],
				0.0F,
				quad.u0(), quad.v0(), quad.u1(), quad.v1(),
				quad.colorArgb(), guiWidth, guiHeight
			);
			if (quad.source() instanceof GuiItemTextureSource.Atlas) {
				request = request.withMaterialMode(rasterLayers.getFirst().materialMode())
					.withItemRasterScale(Minecraft.getInstance().getWindow().getGuiScale())
					.withItemRasterGeometry(boundedItemRasterGeometry(quad.localX0(), quad.localY0(),
						quad.localX1(), quad.localY1(), quad.localX3(), quad.localY3()));
				request = request.withItemRasterLayers(rasterLayers);
			}
			if (quad.clipWidth() != 0 || quad.clipHeight() != 0) {
				request = request.withClip(quad.clipLeft(), quad.clipTop(), quad.clipWidth(), quad.clipHeight());
			}
			var token = dynamicLayerOrder == null
				? RustGalFrameCoordinator.enqueueGuiAffineQuadRequest(request, GuiRenderStratum.GUI_ITEM, startedNanos)
				: RustGalFrameCoordinator.enqueueGuiAffineQuadRequest(
					request, RustGalGuiRenderer.dynamicLayerId(dynamicLayerOrder),
					requestLayerOrder, startedNanos);
			if (atlasItem) quads.forEach(layer -> layer.source().stage());
			else quad.source().stage();
			int left = Math.max(0, (int)Math.floor(Math.min(request.x0(), Math.min(request.x1(), request.x3()))));
			int top = Math.max(0, (int)Math.floor(Math.min(request.y0(), Math.min(request.y1(), request.y3()))));
			int right = Math.min(guiWidth, (int)Math.ceil(Math.max(request.x0(), Math.max(request.x1(), request.x3()))));
			int bottom = Math.min(guiHeight, (int)Math.ceil(Math.max(request.y0(), Math.max(request.y1(), request.y3()))));
			elements.add(new RustGalGuiElementRenderState(
				token, GuiRenderStratum.GUI_ITEM, PRODUCER, -1, -1.0F, GuiFillDirection.NONE,
				left, top, Math.max(1, right - left), Math.max(1, bottom - top), guiWidth, guiHeight
			));
		}
		recordDiagnostic("accepted-quads=" + quads.size());
		return List.copyOf(elements);
	}

	/**
	 * Converts one ordinary block-lit GUI item into a single ordered semantic
	 * mesh item. Its material layers remain nested under one scheduler token so
	 * Rust owns the offscreen raster and final composition ordering.
	 */
	public static List<RustGalGuiElementRenderState> tryEnqueueStandard3dItem(
		GuiItemRenderState item,
		int guiWidth,
		int guiHeight
	) {
		return tryEnqueueStandard3dItem(item, guiWidth, guiHeight, null);
	}

	public static List<RustGalGuiElementRenderState> tryEnqueueStandard3dItem(
		GuiItemRenderState item,
		int guiWidth,
		int guiHeight,
		@Nullable Integer dynamicLayerOrder
	) {
		if (!standard3dRouteEnabled()) {
			return List.of();
		}
		GuiItemMeshSemanticCollector.CollectionResult collected = GuiItemMeshSemanticCollector.collectStandard3d(
			item, Math.max(1, Minecraft.getInstance().getWindow().getGuiScale()));
		if (!collected.accepted()) {
			recordDiagnostic("mesh-" + collected.rejection());
			return List.of();
		}
		GuiItemMeshSemanticCollector.GuiItemMesh mesh = collected.mesh();
		if (mesh.right() > guiWidth || mesh.bottom() > guiHeight || mesh.left() < 0 || mesh.top() < 0) {
			recordDiagnostic("mesh-bounds:" + mesh.left() + "," + mesh.top() + "," + mesh.right() + "," + mesh.bottom()
				+ "/" + guiWidth + "," + guiHeight);
			return List.of();
		}
		List<VulkanicGalBridge.GuiMeshBatchRecord> batches = new ArrayList<>();
		int requestLayerOrder = dynamicLayerOrder == null ? GuiRenderStratum.GUI_ITEM.order()
			: RustGalGuiRenderer.dynamicLayerOrder(dynamicLayerOrder);
		int batchLayerIndex = 0;
		for (int layerIndex = 0; layerIndex < mesh.layers().size(); layerIndex++) {
			GuiItemMeshSemanticCollector.GuiItemMeshLayer layer = mesh.layers().get(layerIndex);
			for (GuiItemMeshSemanticCollector.GuiItemMeshQuad quad : layer.quads()) {
				List<VulkanicGalBridge.GuiMeshVertexRecord> vertices = new ArrayList<>(4);
				for (int vertex = 0; vertex < 4; vertex++) {
					int position = vertex * 3;
					int uv = vertex * 2;
					vertices.add(new VulkanicGalBridge.GuiMeshVertexRecord(
						new float[] {quad.positions()[position], quad.positions()[position + 1], quad.positions()[position + 2]},
						new float[] {quad.atlasUvs()[uv], quad.atlasUvs()[uv + 1]},
						new float[] {quad.localUvs()[uv], quad.localUvs()[uv + 1]},
						quad.colorsArgb()[vertex], quad.packedNormals()[vertex]
					));
				}
				batches.add(new VulkanicGalBridge.GuiMeshBatchRecord(
					requestLayerOrder, batchLayerIndex++,
					guiMaterialMode(layer.materialMode()),
					// Ordinary inventory light space; Rust owns the light vectors.
					layer.blockLight() ? VulkanicGalBridge.GUI_MESH_LIGHTING_INVENTORY_BLOCK : 1, quad.assetId(), 0L,
					(layer.materialMode() == GuiItemMeshSemanticCollector.MaterialMode.CUTOUT
						|| layer.materialMode() == GuiItemMeshSemanticCollector.MaterialMode.GLINT) ? 0.1F : 0.0F,
					layer.modelTransform(), mesh.guiPose(), mesh.left(), mesh.top(), mesh.right(), mesh.bottom(),
					guiWidth, guiHeight, mesh.renderWidth(), mesh.renderHeight(), mesh.guardPixels(),
					vertices, List.of(0, 1, 2, 2, 3, 0)
				).withItemFoil(layer.itemFoil()));
			}
		}
		if (batches.isEmpty()) return List.of();
		long startedNanos = System.nanoTime();
		var token = dynamicLayerOrder == null
			? RustGalFrameCoordinator.enqueueGuiMeshItemRequest(batches, GuiRenderStratum.GUI_ITEM, startedNanos)
			: RustGalFrameCoordinator.enqueueGuiMeshItemRequest(
				batches, RustGalGuiRenderer.dynamicLayerId(dynamicLayerOrder),
				requestLayerOrder, startedNanos);
		for (RustGalGuiRawImageAssets.Asset asset : mesh.assets()) RustGalGuiRawImageAssets.stage(asset);
		recordDiagnostic("mesh-accepted-layers=" + batches.size());
		return List.of(new RustGalGuiElementRenderState(
			token, GuiRenderStratum.GUI_ITEM, "minecraft.gui.item.standard3d", -1, -1.0F, GuiFillDirection.NONE,
			mesh.left(), mesh.top(), mesh.right() - mesh.left(), mesh.bottom() - mesh.top(), guiWidth, guiHeight
		));
	}

	private static List<RustGalGuiElementRenderState> tryEnqueueNativeFlatFoil(
		GuiItemRenderState item, int guiWidth, int guiHeight, @Nullable Integer dynamicLayerOrder) {
		int order = dynamicLayerOrder == null ? GuiRenderStratum.GUI_ITEM.order() : RustGalGuiRenderer.dynamicLayerOrder(dynamicLayerOrder);
		GuiFlatItemMeshCollector.Snapshot snapshot;
		try {
			var options = Minecraft.getInstance().options;
			snapshot = GuiFlatItemMeshCollector.collect(item, guiWidth, guiHeight,
				Minecraft.getInstance().getWindow().getGuiScale(), order,
				new VulkanicGalBridge.StandardItemFoilRecord(net.minecraft.Util.getMillis(), options.glintSpeed().get(), options.glintStrength().get().floatValue()));
		} catch (IllegalArgumentException | IllegalStateException rejection) {
			recordDiagnostic("flat-mesh-rejected:" + rejection.getMessage());
			return List.of();
		}
		long startedNanos = System.nanoTime();
		var token = dynamicLayerOrder == null
			? RustGalFrameCoordinator.enqueueGuiMeshItemRequest(snapshot.batches(), GuiRenderStratum.GUI_ITEM, startedNanos)
			: RustGalFrameCoordinator.enqueueGuiMeshItemRequest(snapshot.batches(), RustGalGuiRenderer.dynamicLayerId(dynamicLayerOrder), order, startedNanos);
		snapshot.sources().forEach(GuiItemTextureSource::stage);
		for (var batch : snapshot.batches()) {
			var foil = batch.itemFoil();
			if (foil != null) {
				net.minecraft.client.dev.GraphicsAuditGuiFoilTiming.observeSemanticClock(
					item.x(), item.y(), foil.clockMillis(), foil.speed(), foil.strength());
				// Every face in this immutable item snapshot uses the same
				// supplied foil payload; observe the item once, not each face.
				break;
			}
		}
		item.itemStackRenderState().forEachSemanticLayer(layer -> {
			for (BakedQuad face : layer.quads()) {
				net.minecraft.client.dev.GraphicsAuditGuiFoilSource.record(face);
				var sprite = ((BakedQuadView)(Object)face).getSprite();
				net.vulkanic.world.RustGalWorldPrimitiveRenderer.recordAtlasSpriteUse(
					sprite.semanticAnimationResource(), sprite.atlasLocation(), sprite.contents().name());
			}
		});
		recordDiagnostic("flat-mesh-accepted-layers=" + snapshot.batches().size());
		return List.of(new RustGalGuiElementRenderState(token, GuiRenderStratum.GUI_ITEM, "minecraft.gui.item.flat-foil",
			-1, -1.0F, GuiFillDirection.NONE, item.x(), item.y(), 16, 16, guiWidth, guiHeight));
	}

	private static int guiMaterialMode(GuiItemMeshSemanticCollector.MaterialMode mode) {
		return switch (mode) {
			case OPAQUE -> 1;
			case CUTOUT -> 2;
			case TRANSLUCENT -> 3;
			case GLINT -> 4;
		};
	}

	public static void invalidateAssets() {
		RustGalGuiRawImageAssets.invalidate();
	}

	private static String appendSupportedLayer(
		GuiItemRenderState item,
		ItemStackRenderState.SemanticLayer layer,
		List<FlatQuad> output
	) {
		if (layer.hasSpecialRenderer()) return "special-renderer";
		if (layer.usesBlockLight()) return "block-light";
		boolean atlasItems = Boolean.getBoolean("mattmc.dev.rustGalGuiAtlasItems")
			&& net.vulkanic.world.WorldRenderRoutePolicy.currentMaterialRoute().usesRustWholeFrameVulkan();
		if (!layer.identityTransform() && !(atlasItems && Boolean.getBoolean("mattmc.dev.rustGalGuiItemTransforms")))
			return "non-identity-transform";
		if (layer.foilType() == ItemStackRenderState.FoilType.SPECIAL) {
			RustGalGuiRawImageAssets.Asset glint = RustGalGuiRawImageAssets.resolve(ItemRenderer.ENCHANTED_GLINT_ITEM);
			if (glint == null) return "glint-texture-unavailable";
		}
		if (layer.renderType() == null || layer.quads().isEmpty()) return "empty-or-missing-render-type";
		if (!supportedGuiRenderType(layer.renderType())) return "render-type";

		if (atlasItems && layer.foilType() != ItemStackRenderState.FoilType.NONE) return "private-item-raster-foil-unavailable";
		List<BakedQuad> selected = new ArrayList<>();
		for (BakedQuad candidate : layer.quads()) {
			if (candidate.direction() == net.minecraft.core.Direction.SOUTH) {
				if (!atlasItems && !selected.isEmpty()) return "multiple-front-quads";
				if (selected.size() == 64) return "private-item-raster-layer-bound";
				selected.add(candidate);
			}
		}
		if (selected.isEmpty()) return "missing-front-quad";
		for (BakedQuad face : selected) {
		FlatQuad quad = copyFlatQuad(item, face, layer.tintLayers());
		if (quad == null) return "non-planar-or-nonuniform-quad";
		output.add(quad);
		if (layer.foilType() == ItemStackRenderState.FoilType.STANDARD) {
			RustGalGuiRawImageAssets.Asset glint = RustGalGuiRawImageAssets.resolve(ItemRenderer.ENCHANTED_GLINT_ITEM);
			if (glint == null) return "glint-texture-unavailable";
			output.add(glintQuad(quad, glint));
		} else if (layer.foilType() == ItemStackRenderState.FoilType.SPECIAL) {
			RustGalGuiRawImageAssets.Asset glint = RustGalGuiRawImageAssets.resolve(ItemRenderer.ENCHANTED_GLINT_ITEM);
			output.add(specialFoilQuad(quad, glint));
		}
		}
		return null;
	}

	private static FlatQuad specialFoilQuad(FlatQuad source, RustGalGuiRawImageAssets.Asset glint) {
		float scale = ItemRenderer.SPECIAL_FOIL_TEXTURE_SCALE;
		float u0 = -source.localX0() * scale;
		float v0 = -source.localY0() * scale;
		float u1 = -source.localX1() * scale;
		float v1 = -source.localY1() * scale;
		float u3 = -source.localX3() * scale;
		float v3 = -source.localY3() * scale;
		int strength = Mth.clamp((int)Math.round(Minecraft.getInstance().options.glintStrength().get() * 255.0F), 0, 255);
		return new FlatQuad(new GuiItemTextureSource.Raw(glint), source.x0(), source.y0(), source.x1(), source.y1(), source.x3(), source.y3(),
			u0, v0, u1, v1, ARGB.color(strength, 255, 255, 255), source.clipLeft(), source.clipTop(),
			source.clipWidth(), source.clipHeight(), u0, v0, u1, v1,
			source.localX0(), source.localY0(), source.localX1(), source.localY1(), source.localX3(), source.localY3());
	}

	private static FlatQuad glintQuad(FlatQuad source, RustGalGuiRawImageAssets.Asset glint) {
		long ticks = (long)(Util.getMillis() * Minecraft.getInstance().options.glintSpeed().get() * 8.0);
		float g = (float)(ticks % 110000L) / 110000.0F;
		float h = (float)(ticks % 30000L) / 30000.0F;
		float angle = (float)(Math.PI / 18.0);
		float scale = 8.0F;
		float cos = (float)Math.cos(angle) * scale;
		float sin = (float)Math.sin(angle) * scale;
		float glintU0 = cos * source.atlasU0() - sin * source.atlasV0() - g;
		float glintV0 = sin * source.atlasU0() + cos * source.atlasV0() + h;
		float glintU1 = cos * source.atlasU1() - sin * source.atlasV1() - g;
		float glintV1 = sin * source.atlasU1() + cos * source.atlasV1() + h;
		int strength = Mth.clamp((int)Math.round(Minecraft.getInstance().options.glintStrength().get() * 255.0F), 0, 255);
		return new FlatQuad(new GuiItemTextureSource.Raw(glint), source.x0(), source.y0(), source.x1(), source.y1(), source.x3(), source.y3(),
			glintU0, glintV0, glintU1, glintV1, ARGB.color(strength, 255, 255, 255),
			source.clipLeft(), source.clipTop(), source.clipWidth(), source.clipHeight(),
			glintU0, glintV0, glintU1, glintV1,
			source.localX0(), source.localY0(), source.localX1(), source.localY1(), source.localX3(), source.localY3());
	}

	static boolean supportedGuiRenderType(RenderType renderType) {
		return renderType != null && flatItemMaterial(renderType.pipeline()) != 0;
	}

	static int flatItemMaterial(net.blaze3d.pipeline.RenderPipeline pipeline) {
		if (pipeline == net.minecraft.client.renderer.RenderPipelines.ITEM_ENTITY_TRANSLUCENT_CULL) return 1;
		if (pipeline == net.minecraft.client.renderer.RenderPipelines.ENTITY_CUTOUT) return 2;
		return 0;
	}

	private static FlatQuad copyFlatQuad(GuiItemRenderState item, BakedQuad bakedQuad, int[] tintLayers) {
		if (!(bakedQuad instanceof BakedQuadView quad)) return null;
		TextureAtlasSprite sprite = quad.getSprite();
		if (sprite == null || sprite.contents().name() == null) return null;
		GuiItemTextureSource source;
		if (Boolean.getBoolean("mattmc.dev.rustGalGuiAtlasItems")
			&& net.vulkanic.world.WorldRenderRoutePolicy.currentMaterialRoute().usesRustWholeFrameVulkan()) {
			var region = net.vulkanic.world.RustGalTerrainRenderer.requireGuiAtlasSpritePayload(sprite);
			long assetId = RustGalGuiRawImageAssets.assetId("gui-atlas-region:" + sprite.atlasLocation()
				+ ":" + sprite.contents().name());
			source = new GuiItemTextureSource.Atlas(new GuiAtlasRegion(assetId, region.texture(),
				region.atlasWidth(), region.atlasHeight(), region.x(), region.y(), region.width(), region.height()));
		} else {
			RustGalGuiRawImageAssets.Asset asset = sprite.contents().isAnimated()
				? RustGalGuiRawImageAssets.resolveAnimatedSprite(sprite)
				: RustGalGuiRawImageAssets.resolve(sprite.contents().name());
			if (asset == null) return null;
			source = new GuiItemTextureSource.Raw(asset);
		}
		int tint = itemTint(bakedQuad, tintLayers);
		int color = shadedColor(quad.getColor(0), tint);
		for (int index = 1; index < 4; index++) {
			if (shadedColor(quad.getColor(index), tint) != color) return null;
		}

		int origin = findUvVertex(quad, sprite, true, true);
		int axisU = findUvVertex(quad, sprite, false, true);
		int axisV = findUvVertex(quad, sprite, true, false);
		if (origin < 0 || axisU < 0 || axisV < 0 || origin == axisU || origin == axisV || axisU == axisV) return null;
		int diagonal = 6 - origin - axisU - axisV;
		// The explicit affine primitive reconstructs its fourth corner. Never
		// silently change a baked trapezoid or skewed UV mapping into that shape.
		if (!affineFourthCorner(quad.getX(origin),quad.getY(origin),quad.getX(axisU),quad.getY(axisU),
			quad.getX(diagonal),quad.getY(diagonal),quad.getX(axisV),quad.getY(axisV))
			|| quad.getTexV(axisU) != quad.getTexV(origin)
			|| quad.getTexU(axisV) != quad.getTexU(origin)
			|| quad.getTexU(diagonal) != quad.getTexU(axisU)
			|| quad.getTexV(diagonal) != quad.getTexV(axisV)) return null;
		float u0 = localU(sprite, quad.getTexU(origin));
		float v0 = localV(sprite, quad.getTexV(origin));
		float u1 = localU(sprite, quad.getTexU(axisU));
		float v1 = localV(sprite, quad.getTexV(axisV));
		if (source instanceof GuiItemTextureSource.Atlas) {
			u0 = itemLocalUv(quad.getTexU(origin),sprite.getU0(),sprite.getU1());
			v0 = itemLocalUv(quad.getTexV(origin),sprite.getV0(),sprite.getV1());
			u1 = itemLocalUv(quad.getTexU(axisU),sprite.getU0(),sprite.getU1());
			v1 = itemLocalUv(quad.getTexV(axisV),sprite.getV0(),sprite.getV1());
			if (!Float.isFinite(u0) || !Float.isFinite(v0) || !Float.isFinite(u1) || !Float.isFinite(v1)
				|| u0 < 0 || v0 < 0 || u1 > 1 || v1 > 1) return null;
		}
		float atlasU0 = quad.getTexU(origin);
		float atlasV0 = quad.getTexV(origin);
		float atlasU1 = quad.getTexU(axisU);
		float atlasV1 = quad.getTexV(axisV);
		if (u1 <= u0 || v1 <= v0) return null;

		float[] p0 = guiPoint(item, quad.getX(origin), quad.getY(origin));
		float[] p1 = guiPoint(item, quad.getX(axisU), quad.getY(axisU));
		float[] p3 = guiPoint(item, quad.getX(axisV), quad.getY(axisV));
		if (!finite(p0) || !finite(p1) || !finite(p3) || Math.abs(area(p0, p1, p3)) < 0.01F) return null;
		if (Boolean.getBoolean("mattmc.dev.guiItemRasterTrace")) {
			recordDiagnostic("affine-source sprite=" + sprite.contents().name()
				+ " atlas=" + sprite.atlasLocation() + " region=" + sprite.getX() + "," + sprite.getY()
				+ "," + sprite.contents().width() + "," + sprite.contents().height()
				+ " sourceUv=" + Float.toHexString(atlasU0) + "," + Float.toHexString(atlasV0)
				+ "," + Float.toHexString(atlasU1) + "," + Float.toHexString(atlasV1)
				+ " localUv=" + Float.toHexString(u0) + "," + Float.toHexString(v0)
				+ "," + Float.toHexString(u1) + "," + Float.toHexString(v1)
				+ " points=" + java.util.Arrays.toString(p0) + java.util.Arrays.toString(p1) + java.util.Arrays.toString(p3));
		}
		ScreenRectangle scissor = item.scissorArea();
		return new FlatQuad(
			source, p0[0], p0[1], p1[0], p1[1], p3[0], p3[1], u0, v0, u1, v1, color,
			scissor == null ? 0 : scissor.left(),
			scissor == null ? 0 : scissor.top(),
			 scissor == null ? 0 : scissor.width(),
			 scissor == null ? 0 : scissor.height(), atlasU0, atlasV0, atlasU1, atlasV1,
			quad.getX(origin), quad.getY(origin), quad.getX(axisU), quad.getY(axisU), quad.getX(axisV), quad.getY(axisV)
		);
	}

	/** Copies model-local corners; no geometry is inferred from screen state. */
	static boolean affineFourthCorner(float x0, float y0, float x1, float y1,
		float x2, float y2, float x3, float y3) {
		for (float value : new float[] {x0,y0,x1,y1,x2,y2,x3,y3})
			if (!Float.isFinite(value)) return false;
		float expectedX = x1+x3-x0, expectedY = y1+y3-y0;
		// Baked rotations and reconstruction round in different orders. Allow
		// only float arithmetic error, not a geometric approximation tolerance.
		float magnitude = Math.max(1.0F, Math.max(Math.abs(expectedX),Math.abs(expectedY)));
		return Float.isFinite(expectedX) && Float.isFinite(expectedY)
			&& Math.abs(x2-expectedX) <= 8*Math.ulp(magnitude)
			&& Math.abs(y2-expectedY) <= 8*Math.ulp(magnitude);
	}

	static VulkanicGalBridge.GuiItemRasterGeometryRecord boundedItemRasterGeometry(
		float x0, float y0, float x1, float y1, float x3, float y3) {
		try {
			return new VulkanicGalBridge.GuiItemRasterGeometryRecord(x0*16, (1-y0)*16,
				x1*16, (1-y1)*16, x3*16, (1-y3)*16);
		} catch (IllegalArgumentException unsupportedGeometry) {
			return null;
		}
	}

	private static int findUvVertex(BakedQuadView quad, TextureAtlasSprite sprite, boolean minU, boolean minV) {
		int best = -1;
		float bestDistance = Float.POSITIVE_INFINITY;
		float targetU = minU ? Float.POSITIVE_INFINITY : Float.NEGATIVE_INFINITY;
		float targetV = minV ? Float.POSITIVE_INFINITY : Float.NEGATIVE_INFINITY;
		for (int index = 0; index < 4; index++) {
			float u = localU(sprite, quad.getTexU(index));
			float v = localV(sprite, quad.getTexV(index));
			targetU = minU ? Math.min(targetU, u) : Math.max(targetU, u);
			targetV = minV ? Math.min(targetV, v) : Math.max(targetV, v);
		}
		for (int index = 0; index < 4; index++) {
			float du = localU(sprite, quad.getTexU(index)) - targetU;
			float dv = localV(sprite, quad.getTexV(index)) - targetV;
			float distance = du * du + dv * dv;
			if (distance < bestDistance) {
				bestDistance = distance;
				best = index;
			}
		}
		return bestDistance <= 0.0001F ? best : -1;
	}

	private static float[] guiPoint(GuiItemRenderState item, float modelX, float modelY) {
		float x = item.x() + modelX * 16.0F;
		float y = item.y() + (1.0F - modelY) * 16.0F;
		Matrix3x2f pose = item.pose();
		return new float[] {pose.m00() * x + pose.m10() * y + pose.m20(), pose.m01() * x + pose.m11() * y + pose.m21()};
	}

	/**
	 * Resolves and stages a copied semantic image for the Rust-owned mesh
	 * layer. The returned key is stable across a resource generation; neither a
	 * sprite nor an atlas object escapes this Java boundary.
	 */
	public static long stageSemanticImage(ResourceLocation sprite) {
		long assetId = resolveSemanticImage(sprite);
		if (assetId == 0L) return 0L;
		RustGalGuiRawImageAssets.Asset asset = RustGalGuiRawImageAssets.resolve(sprite);
		if (asset != null) RustGalGuiRawImageAssets.stage(asset);
		return assetId;
	}

	/** Resolves a semantic image identity without publishing it to the Rust frame. */
	public static long resolveSemanticImage(ResourceLocation sprite) {
		RustGalGuiRawImageAssets.Asset asset = RustGalGuiRawImageAssets.resolve(sprite);
		if (asset == null) {
			var texture = Minecraft.getInstance().getTextureManager().getTexture(sprite);
			if (texture instanceof DynamicTexture dynamic) {
				RustGalGuiRawImageAssets.registerDynamicTextureUnstaged(sprite, dynamic);
				RustGalGuiRawImageAssets.prepareDynamicTexture(dynamic);
				asset = RustGalGuiRawImageAssets.resolve(sprite);
			}
		}
		if (asset == null) {
			return 0L;
		}
		return asset.assetId();
	}

	/** Stages one current animated-sprite frame under a stable semantic identity. */
	public static long stageSemanticImage(TextureAtlasSprite sprite) {
		if (sprite == null || sprite.contents() == null) return 0L;
		RustGalGuiRawImageAssets.Asset asset = sprite.contents().isAnimated()
			? RustGalGuiRawImageAssets.resolveAnimatedSprite(sprite)
			: RustGalGuiRawImageAssets.resolve(sprite.contents().name());
		if (asset == null) return 0L;
		RustGalGuiRawImageAssets.stage(asset);
		return asset.assetId();
	}

	private static int itemTint(BakedQuad quad, int[] tintLayers) {
		if (!quad.isTinted() || tintLayers.length == 0 || quad.tintIndex() < 0 || quad.tintIndex() >= tintLayers.length) return 0xffffffff;
		int tint = tintLayers[quad.tintIndex()];
		return tint == -1 ? 0xffffffff : tint;
	}

	private static int shadedColor(int bakedColor, int tint) {
		int alpha = Math.round(((bakedColor >>> 24 & 0xff) / 255.0F) * ((ARGB.alpha(tint)) / 255.0F) * 255.0F);
		int red = Math.round(((bakedColor & 0xff) / 255.0F) * ((ARGB.red(tint)) / 255.0F) * 255.0F);
		int green = Math.round(((bakedColor >>> 8 & 0xff) / 255.0F) * ((ARGB.green(tint)) / 255.0F) * 255.0F);
		int blue = Math.round(((bakedColor >>> 16 & 0xff) / 255.0F) * ((ARGB.blue(tint)) / 255.0F) * 255.0F);
		return ARGB.color(Mth.clamp(alpha, 0, 255), Mth.clamp(red, 0, 255), Mth.clamp(green, 0, 255), Mth.clamp(blue, 0, 255));
	}

	/** Exact semantic conversion; callers reject unsupported ranges, never clamp them. */
	static float itemLocalUv(float coordinate, float start, float end) {
		return (coordinate-start)/(end-start);
	}

	private static float localU(TextureAtlasSprite sprite, float atlasU) {
		float width = sprite.getU1() - sprite.getU0();
		return width == 0.0F ? 0.0F : Mth.clamp((atlasU - sprite.getU0()) / width, 0.0F, 1.0F);
	}

	private static float localV(TextureAtlasSprite sprite, float atlasV) {
		float height = sprite.getV1() - sprite.getV0();
		return height == 0.0F ? 0.0F : Mth.clamp((atlasV - sprite.getV0()) / height, 0.0F, 1.0F);
	}

	private static boolean finite(float[] point) {
		return Float.isFinite(point[0]) && Float.isFinite(point[1]);
	}

	private static float area(float[] p0, float[] p1, float[] p3) {
		return (p1[0] - p0[0]) * (p3[1] - p0[1]) - (p1[1] - p0[1]) * (p3[0] - p0[0]);
	}

	private static synchronized void recordDiagnostic(String detail) {
		if (DIAGNOSTICS.containsKey(detail) || DIAGNOSTICS.size() >= MAX_DIAGNOSTIC_ENTRIES) return;
		DIAGNOSTICS.put(detail, Boolean.TRUE);
		RustGalFrameCoordinator.auditMessage("gui.item.route " + detail);
	}

	private record FlatQuad(
		GuiItemTextureSource source,
		float x0, float y0, float x1, float y1, float x3, float y3,
		float u0, float v0, float u1, float v1,
		int colorArgb,
		int clipLeft, int clipTop, int clipWidth, int clipHeight,
		float atlasU0, float atlasV0, float atlasU1, float atlasV1
		, float localX0, float localY0, float localX1, float localY1, float localX3, float localY3
	) {
	}
}
