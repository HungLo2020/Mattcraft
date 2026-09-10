package net.minecraft.client.renderer;

import net.blaze3d.vertex.PoseStack;
import net.blaze3d.vertex.VertexConsumer;
import java.util.ArrayList;
import java.util.List;
import net.minecraft.client.renderer.state.QuadParticleRenderState;
import net.minecraft.api.EnvType;
import net.minecraft.api.Environment;
import net.minecraft.core.BlockPos;
import net.minecraft.client.Minecraft;
import net.minecraft.client.gui.Font;
import net.minecraft.client.model.Model;
import net.minecraft.client.model.geom.ModelPart;
import net.minecraft.client.renderer.block.MovingBlockRenderState;
import net.minecraft.client.renderer.block.model.BakedQuad;
import net.minecraft.client.renderer.block.model.BlockStateModel;
import net.minecraft.client.renderer.entity.state.EntityRenderState;
import net.minecraft.client.renderer.entity.state.HitboxesRenderState;
import net.minecraft.client.renderer.feature.CustomFeatureRenderer;
import net.minecraft.client.renderer.feature.ModelFeatureRenderer;
import net.minecraft.client.renderer.feature.ModelPartFeatureRenderer;
import net.minecraft.client.renderer.feature.NameTagFeatureRenderer;
import net.minecraft.client.renderer.item.ItemStackRenderState;
import net.minecraft.client.renderer.state.CameraRenderState;
import net.minecraft.client.renderer.texture.TextureAtlasSprite;
import net.minecraft.client.renderer.texture.OverlayTexture;
import net.minecraft.network.chat.Component;
import net.minecraft.util.FormattedCharSequence;
import net.minecraft.world.item.ItemDisplayContext;
import net.minecraft.world.level.block.state.BlockState;
import net.minecraft.world.phys.Vec3;
import org.jetbrains.annotations.Nullable;
import org.joml.Matrix4f;
import org.joml.Quaternionf;
// Sodium FRAPI imports
import net.sodium.client.render.frapi.render.MeshItemCommand;
import net.sodium.client.render.frapi.render.OrderedSubmitNodeCollectorExtension;
import net.sodium.client.render.frapi.render.SubmitNodeCollectionExtension;
import net.fabricmc.fabric.api.renderer.v1.mesh.MeshView;

@Environment(EnvType.CLIENT)
public class SubmitNodeCollection implements OrderedSubmitNodeCollector, OrderedSubmitNodeCollectorExtension, SubmitNodeCollectionExtension {
	private final List<SubmitNodeStorage.ShadowSubmit> shadowSubmits = new ArrayList();
	private final List<SubmitNodeStorage.FlameSubmit> flameSubmits = new ArrayList();
	private static final int MAX_RUST_SEMANTIC_FEATURE_SUBMITS = 4_096;
	private static final int MAX_RUST_SEMANTIC_BLOCK_SUBMITS = 8_192;
	/** Bounded Java-side staging capacity for model semantics before Rust extraction. */
	private static final int MAX_RUST_SEMANTIC_MODEL_SUBMITS = 8_192;
	private final NameTagFeatureRenderer.Storage nameTagSubmits = new NameTagFeatureRenderer.Storage();
	private final List<SubmitNodeStorage.TextSubmit> textSubmits = new ArrayList();
	private static final int MAX_RUST_SEMANTIC_TEXT_SUBMITS = 4_096;
	private final List<SubmitNodeStorage.HitboxSubmit> hitboxSubmits = new ArrayList();
	private final List<SubmitNodeStorage.LeashSubmit> leashSubmits = new ArrayList();
	private final List<SubmitNodeStorage.BlockSubmit> blockSubmits = new ArrayList();
	private final List<SubmitNodeStorage.MovingBlockSubmit> movingBlockSubmits = new ArrayList();
	private final List<SubmitNodeStorage.BlockModelSubmit> blockModelSubmits = new ArrayList();
	private final List<SubmitNodeStorage.ItemSubmit> itemSubmits = new ArrayList();
	private final List<SubmitNodeCollector.ParticleGroupRenderer> particleGroupRenderers = new ArrayList();
	private final ModelFeatureRenderer.Storage modelSubmits = new ModelFeatureRenderer.Storage();
	private final ModelPartFeatureRenderer.Storage modelPartSubmits = new ModelPartFeatureRenderer.Storage();
	private final CustomFeatureRenderer.Storage customGeometrySubmits = new CustomFeatureRenderer.Storage();
	private final SubmitNodeStorage submitNodeStorage;
	private boolean wasUsed = false;
	// Sodium FRAPI: Mesh item commands for fabric rendering API
	private final List<MeshItemCommand> meshItemCommands = new ArrayList<>();
	/** Fabric meshes already copied into the Rust semantic item stream. */
	private int rustFabricItemSubmits;

	public SubmitNodeCollection(SubmitNodeStorage submitNodeStorage) {
		this.submitNodeStorage = submitNodeStorage;
	}

	@Override
	public void submitHitbox(PoseStack poseStack, EntityRenderState entityRenderState, HitboxesRenderState hitboxesRenderState) {
		submitHitboxInternal(poseStack, entityRenderState, hitboxesRenderState);
	}

	@Override
	public void submitHitboxSemantic(PoseStack poseStack, EntityRenderState entityRenderState, HitboxesRenderState hitboxesRenderState) {
		submitHitboxInternal(poseStack, entityRenderState, hitboxesRenderState);
	}

	private void submitHitboxInternal(PoseStack poseStack, EntityRenderState entityRenderState, HitboxesRenderState hitboxesRenderState) {
		ensureSelectedVulkanFeatureRoute(net.vulkanic.world.WorldRenderRoutePolicy.currentDebugLineRoute(), "debug-hitbox");
		this.wasUsed = true;
		if ((net.vulkanic.VulkanicAPI.isVulkanBackendSelected()
			|| net.vulkanic.bridge.RustGalVulkanWholeFrameMode.enabled())
			&& this.hitboxSubmits.size() >= MAX_RUST_SEMANTIC_FEATURE_SUBMITS) {
			throw new IllegalStateException("Rust whole-frame debug-hitbox route exceeded bounded submit capacity " + MAX_RUST_SEMANTIC_FEATURE_SUBMITS);
		}
		this.hitboxSubmits.add(new SubmitNodeStorage.HitboxSubmit(new Matrix4f(poseStack.last().pose()), entityRenderState, hitboxesRenderState));
	}

	@Override
	public void submitShadow(PoseStack poseStack, float f, List<EntityRenderState.ShadowPiece> list) {
		submitShadowInternal(poseStack, f, list);
	}

	@Override
	public void submitShadowSemantic(PoseStack poseStack, float radius, List<EntityRenderState.ShadowPiece> pieces) {
		submitShadowInternal(poseStack, radius, pieces);
	}

	private void submitShadowInternal(PoseStack poseStack, float f, List<EntityRenderState.ShadowPiece> list) {
		ensureSelectedVulkanFeatureRoute(net.vulkanic.world.WorldRenderRoutePolicy.currentEntityShadowRoute(), "entity-shadow");
		this.wasUsed = true;
		if ((net.vulkanic.VulkanicAPI.isVulkanBackendSelected()
			|| net.vulkanic.bridge.RustGalVulkanWholeFrameMode.enabled())
			&& this.shadowSubmits.size() >= MAX_RUST_SEMANTIC_FEATURE_SUBMITS) {
			throw new IllegalStateException("Rust whole-frame entity-shadow route exceeded bounded submit capacity " + MAX_RUST_SEMANTIC_FEATURE_SUBMITS);
		}
		PoseStack.Pose pose = poseStack.last();
		this.shadowSubmits.add(new SubmitNodeStorage.ShadowSubmit(new Matrix4f(pose.pose()), f, list));
	}

	@Override
	public void submitNameTag(
		PoseStack poseStack, @Nullable Vec3 vec3, int i, Component component, boolean bl, int j, double d, CameraRenderState cameraRenderState
	) {
		ensureSelectedVulkanWorldTextRoute();
		this.wasUsed = true;
		this.nameTagSubmits.add(poseStack, vec3, i, component, bl, j, d, cameraRenderState);
	}

	@Override
	public void submitNameTagSemantic(
		PoseStack poseStack, @Nullable Vec3 offset, int packedLight, Component text,
		boolean seeThrough, int width, double distance, CameraRenderState cameraRenderState
	) {
		ensureSelectedVulkanWorldTextRoute();
		this.wasUsed = true;
		this.nameTagSubmits.add(poseStack, offset, packedLight, text, seeThrough, width, distance, cameraRenderState);
	}

	@Override
	public void submitText(
		PoseStack poseStack, float f, float g, FormattedCharSequence formattedCharSequence, boolean bl, Font.DisplayMode displayMode, int i, int j, int k, int l
	) {
		ensureSelectedVulkanWorldTextRoute();
		SubmitNodeStorage.TextSubmit textSubmit = this.copyTextSubmit(poseStack, f, g, formattedCharSequence, bl, displayMode, i, j, k, l);
		// Iris: Capture model storage (merged from MixinModelStorageTrigger)
		boolean rustWholeFrameText = rustWholeFramePresenterActive()
			&& net.vulkanic.world.WorldRenderRoutePolicy.currentWorldTextRoute().usesRustWholeFrameVulkan();
		if (!rustWholeFrameText
			&& !net.vulkanic.VulkanicAPI.isVulkanBackendSelected()
			&& !net.vulkanic.world.RustGalWorldPrimitiveRenderer.isFirstPersonGuiCaptureActive()) {
			((net.irisshaders.iris.mixinterface.ModelStorage) textSubmit).iris$capture();
		}
		if (rustWholeFrameText && this.textSubmits.size() >= MAX_RUST_SEMANTIC_TEXT_SUBMITS) {
			throw new IllegalStateException(
				"Rust whole-frame world-text route exceeded bounded text submit capacity "
					+ MAX_RUST_SEMANTIC_TEXT_SUBMITS
			);
		}
		this.textSubmits.add(textSubmit);
	}

	/**
	 * Stores copied text semantics from a whole-frame extraction callback. The
	 * selected Rust route must not capture Iris model state because it neither
	 * borrows Iris programs nor invokes the Java text renderer.
	 */
	@Override
	public void submitTextSemantic(
		PoseStack poseStack, float f, float g, FormattedCharSequence formattedCharSequence, boolean bl, Font.DisplayMode displayMode, int i, int j, int k, int l
	) {
		ensureSelectedVulkanWorldTextRoute();
		if ((net.vulkanic.VulkanicAPI.isVulkanBackendSelected()
			|| net.vulkanic.bridge.RustGalVulkanWholeFrameMode.enabled())
			&& this.textSubmits.size() >= MAX_RUST_SEMANTIC_TEXT_SUBMITS) {
			throw new IllegalStateException(
				"Rust whole-frame world-text route exceeded bounded text submit capacity "
					+ MAX_RUST_SEMANTIC_TEXT_SUBMITS
			);
		}
		this.textSubmits.add(this.copyTextSubmit(poseStack, f, g, formattedCharSequence, bl, displayMode, i, j, k, l));
	}

	private void ensureSelectedVulkanWorldTextRoute() {
		if ((net.vulkanic.VulkanicAPI.isVulkanBackendSelected()
			|| net.vulkanic.bridge.RustGalVulkanWholeFrameMode.enabled())
			&& !net.vulkanic.world.WorldRenderRoutePolicy.currentWorldTextRoute().usesRustWholeFrameVulkan()) {
			throw new IllegalStateException(
				"Rust Vulkan world-text route is unavailable; Java text and Iris capture are not fallbacks");
		}
	}

	/** True whenever either the selected Vulkan backend or the active Rust
	 * presenter shell owns the frame; semantic feature routes must use this
	 * single ownership predicate instead of reopening Java geometry. */
	private static boolean rustWholeFramePresenterActive() {
		return net.vulkanic.VulkanicAPI.isVulkanBackendSelected()
			|| net.vulkanic.bridge.RustGalVulkanWholeFrameMode.enabled();
	}

	private void ensureSelectedVulkanFeatureRoute(
		net.vulkanic.world.WorldRenderRoutePolicy.Route route,
		String family
	) {
		if ((net.vulkanic.VulkanicAPI.isVulkanBackendSelected()
			|| net.vulkanic.bridge.RustGalVulkanWholeFrameMode.enabled())
			&& !route.usesRustWholeFrameVulkan()) {
			throw new IllegalStateException(
				"Rust Vulkan " + family + " route is unavailable; Java feature state is not a fallback");
		}
	}

	private SubmitNodeStorage.TextSubmit copyTextSubmit(
		PoseStack poseStack, float f, float g, FormattedCharSequence formattedCharSequence, boolean bl, Font.DisplayMode displayMode, int i, int j, int k, int l
	) {
		this.wasUsed = true;
		return new SubmitNodeStorage.TextSubmit(new Matrix4f(poseStack.last().pose()), f, g, formattedCharSequence, bl, displayMode, i, j, k, l);
	}

	@Override
	public void submitFlame(PoseStack poseStack, EntityRenderState entityRenderState, Quaternionf quaternionf) {
		submitFlameInternal(poseStack, entityRenderState, quaternionf);
	}

	@Override
	public void submitFlameSemantic(PoseStack poseStack, EntityRenderState entityRenderState, Quaternionf quaternionf) {
		submitFlameInternal(poseStack, entityRenderState, quaternionf);
	}

	private void submitFlameInternal(PoseStack poseStack, EntityRenderState entityRenderState, Quaternionf quaternionf) {
		ensureSelectedVulkanFeatureRoute(net.vulkanic.world.WorldRenderRoutePolicy.currentEntityFlameRoute(), "entity-flame");
		this.wasUsed = true;
		if ((net.vulkanic.VulkanicAPI.isVulkanBackendSelected()
			|| net.vulkanic.bridge.RustGalVulkanWholeFrameMode.enabled())
			&& this.flameSubmits.size() >= MAX_RUST_SEMANTIC_FEATURE_SUBMITS) {
			throw new IllegalStateException("Rust whole-frame entity-flame route exceeded bounded submit capacity " + MAX_RUST_SEMANTIC_FEATURE_SUBMITS);
		}
		this.flameSubmits.add(new SubmitNodeStorage.FlameSubmit(poseStack.last().copy(), entityRenderState, quaternionf));
	}

	@Override
	public void submitLeash(PoseStack poseStack, EntityRenderState.LeashState leashState) {
		submitLeashInternal(poseStack, leashState);
	}

	@Override
	public void submitLeashSemantic(PoseStack poseStack, EntityRenderState.LeashState leashState) {
		submitLeashInternal(poseStack, leashState);
	}

	private void submitLeashInternal(PoseStack poseStack, EntityRenderState.LeashState leashState) {
		ensureSelectedVulkanFeatureRoute(net.vulkanic.world.WorldRenderRoutePolicy.currentEntityLeashRoute(), "entity-leash");
		this.wasUsed = true;
		if ((net.vulkanic.VulkanicAPI.isVulkanBackendSelected()
			|| net.vulkanic.bridge.RustGalVulkanWholeFrameMode.enabled())
			&& this.leashSubmits.size() >= MAX_RUST_SEMANTIC_FEATURE_SUBMITS) {
			throw new IllegalStateException("Rust whole-frame entity-leash route exceeded bounded submit capacity " + MAX_RUST_SEMANTIC_FEATURE_SUBMITS);
		}
		this.leashSubmits.add(new SubmitNodeStorage.LeashSubmit(new Matrix4f(poseStack.last().pose()), leashState));
	}

	@Override
	public <S> void submitModelOutlineSemanticTexture(Model<? super S> model, S state,
		PoseStack poseStack, RenderType material, int light,
		net.minecraft.resources.ResourceLocation textureIdentity, int outlineColor) {
		if (!(state instanceof net.minecraft.client.renderer.entity.state.EntityRenderState entityState)) {
			throw new IllegalArgumentException("outline-only model requires semantic entity identity");
		}
		var entityIdentity = net.vulkanic.world.RustGalWorldPrimitiveRenderer.entityIdentity(entityState);
		if (entityIdentity == null || !net.vulkanic.world.RustGalWorldPrimitiveRenderer.enqueueStandaloneModelMeshOutlineOnly(
			model, state, poseStack.last(), material, textureIdentity, entityIdentity, light, outlineColor)) {
			throw new IllegalStateException("outline-only semantic model route unavailable");
		}
		net.vulkanic.world.RustGalWorldPrimitiveRenderer.recordModelMeshRouteDecision(
			"rust-vulkan-whole-frame", textureIdentity, model.getClass().getName(), entityState.entityId, true, true, false);
	}

	@Override
	public <S> void submitModelSemanticTexture(
		Model<? super S> model,
		S object,
		PoseStack poseStack,
		RenderType renderType,
		int i,
		int j,
		int k,
		net.minecraft.resources.ResourceLocation textureIdentity,
		int l,
		@Nullable ModelFeatureRenderer.CrumblingOverlay crumblingOverlay
	) {
		if ((net.vulkanic.VulkanicAPI.isVulkanBackendSelected()
			|| net.vulkanic.bridge.RustGalVulkanWholeFrameMode.enabled())
			&& net.vulkanic.world.WorldRenderRoutePolicy.currentModelMeshRoute(true).usesRustWholeFrameVulkan()
			&& textureIdentity != null
			&& object instanceof net.minecraft.client.renderer.entity.state.EntityRenderState entityState
			&& renderType != null
			&& net.vulkanic.world.RustGalWorldPrimitiveRenderer.isStandaloneTranslucentModelMeshEligible(
				model, renderType, textureIdentity, j, l, crumblingOverlay)
		) {
			net.minecraft.resources.ResourceLocation entityIdentity =
				net.vulkanic.world.RustGalWorldPrimitiveRenderer.entityIdentity(entityState);
			if (entityIdentity == null) {
				entityIdentity = net.minecraft.resources.ResourceLocation.withDefaultNamespace("model/direct-texture");
			}
			if (!net.vulkanic.world.RustGalWorldPrimitiveRenderer.enqueueStandaloneTranslucentModelMesh(
				model, object, poseStack.last(), renderType, textureIdentity, entityIdentity,
				i, j, k, l
			)) {
				throw new IllegalStateException("Rust whole-frame translucent direct-texture model route selected without a copied indexed mesh request");
			}
			net.vulkanic.world.RustGalWorldPrimitiveRenderer.recordModelMeshRouteDecision(
				"rust-vulkan-whole-frame", textureIdentity, model.getClass().getName(), entityState.entityId, true, true, false);
			return;
		}
		if ((net.vulkanic.VulkanicAPI.isVulkanBackendSelected()
			|| net.vulkanic.bridge.RustGalVulkanWholeFrameMode.enabled())
			&& net.vulkanic.world.WorldRenderRoutePolicy.currentModelMeshRoute(true).usesRustWholeFrameVulkan()
			&& textureIdentity != null
			&& object instanceof net.minecraft.client.renderer.entity.state.EntityRenderState entityState
			&& renderType != null
			&& net.vulkanic.world.RustGalWorldPrimitiveRenderer.isStandaloneModelMeshEligible(
				model, renderType, textureIdentity, j, l, crumblingOverlay)
		) {
			net.minecraft.resources.ResourceLocation entityIdentity =
				net.vulkanic.world.RustGalWorldPrimitiveRenderer.entityIdentity(entityState);
			if (entityIdentity == null) {
				entityIdentity = net.minecraft.resources.ResourceLocation.withDefaultNamespace("model/direct-texture");
			}
			if (!net.vulkanic.world.RustGalWorldPrimitiveRenderer.enqueueStandaloneModelMesh(
				model, object, poseStack.last(), renderType, textureIdentity, entityIdentity,
				i, j, k, l
			)) {
				throw new IllegalStateException("Rust whole-frame direct-texture model route selected without a copied indexed mesh request");
			}
			net.vulkanic.world.RustGalWorldPrimitiveRenderer.recordModelMeshRouteDecision(
				"rust-vulkan-whole-frame", textureIdentity, model.getClass().getName(), entityState.entityId, true, true, false);
			return;
		}
		// Direct-texture Model.Simple submissions (notably avatar hands) carry a
		// complete resource identity but no atlas sprite or EntityRenderState.
		// Admit the translucent variant through the same copied indexed-mesh
		// contract used by other direct-texture model layers.
		if ((net.vulkanic.VulkanicAPI.isVulkanBackendSelected()
			|| net.vulkanic.bridge.RustGalVulkanWholeFrameMode.enabled())
			&& net.vulkanic.world.WorldRenderRoutePolicy.currentModelMeshRoute(true).usesRustWholeFrameVulkan()
			&& object == net.minecraft.util.Unit.INSTANCE
			&& textureIdentity != null
			&& net.vulkanic.world.RustGalWorldPrimitiveRenderer.isStandaloneTranslucentModelMeshEligible(
				model, renderType, textureIdentity, j, l, crumblingOverlay)
			&& net.vulkanic.world.RustGalWorldPrimitiveRenderer.enqueueStandaloneTranslucentModelMesh(
				model, object, poseStack.last(), renderType, textureIdentity,
				net.minecraft.resources.ResourceLocation.withDefaultNamespace("model-part/direct-texture"), i, j, k
			)) {
			net.vulkanic.world.RustGalWorldPrimitiveRenderer.recordModelMeshRouteDecision(
				"rust-vulkan-whole-frame", textureIdentity, model.getClass().getName(), true, true, false);
			return;
		}
		// Direct-texture opaque Model submissions with a Unit state (notably the
		// arrow and bee-stinger StuckInBodyLayer family) carry the same complete
		// semantic inputs as translucent hand overlays.  Admit them through the
		// copied indexed mesh contract instead of letting the generic collector
		// lose the texture identity and reopen a Java submission under Rust Vulkan.
		if ((net.vulkanic.VulkanicAPI.isVulkanBackendSelected()
			|| net.vulkanic.bridge.RustGalVulkanWholeFrameMode.enabled())
			&& net.vulkanic.world.WorldRenderRoutePolicy.currentModelMeshRoute(true).usesRustWholeFrameVulkan()
			&& object == net.minecraft.util.Unit.INSTANCE
			&& textureIdentity != null
			&& net.vulkanic.world.RustGalWorldPrimitiveRenderer.isStandaloneModelMeshEligible(
				model, renderType, textureIdentity, j, l, crumblingOverlay)
			&& net.vulkanic.world.RustGalWorldPrimitiveRenderer.enqueueStandaloneModelMesh(
				model, object, poseStack.last(), renderType, textureIdentity,
			net.minecraft.resources.ResourceLocation.withDefaultNamespace("model-part/direct-texture"), i, j, k, l
			)) {
			net.vulkanic.world.RustGalWorldPrimitiveRenderer.recordModelMeshRouteDecision(
				"rust-vulkan-whole-frame", textureIdentity, model.getClass().getName(), true, true, false);
			return;
		}
		// ArrowLayer reuses a compact ArrowRenderState while placing arrows in a
		// living model; that transient state has no entity registry identity. The
		// model, direct texture, and pose are nevertheless complete semantic data,
		// so use a stable copied identity instead of rejecting the draw or allowing
		// a Java submission under Rust whole-frame ownership.
		if ((net.vulkanic.VulkanicAPI.isVulkanBackendSelected()
			|| net.vulkanic.bridge.RustGalVulkanWholeFrameMode.enabled())
			&& net.vulkanic.world.WorldRenderRoutePolicy.currentModelMeshRoute(true).usesRustWholeFrameVulkan()
			&& model instanceof net.minecraft.client.model.ArrowModel
			&& object instanceof net.minecraft.client.renderer.entity.state.ArrowRenderState
			&& textureIdentity != null
			&& textureIdentity.equals(net.minecraft.client.renderer.entity.TippableArrowRenderer.NORMAL_ARROW_LOCATION)
			&& net.vulkanic.world.RustGalWorldPrimitiveRenderer.isStandaloneModelMeshEligible(
				model, renderType, textureIdentity, j, l, crumblingOverlay)
			&& net.vulkanic.world.RustGalWorldPrimitiveRenderer.enqueueStandaloneModelMesh(
				model, object, poseStack.last(), renderType, textureIdentity,
				net.minecraft.resources.ResourceLocation.withDefaultNamespace("particle/arrow"), i, j, k, l
			)) {
			net.vulkanic.world.RustGalWorldPrimitiveRenderer.recordModelMeshRouteDecision(
				"rust-vulkan-whole-frame", textureIdentity, model.getClass().getName(), true, true, false);
			return;
		}
		if (rustWholeFramePresenterActive()
			&& net.vulkanic.world.WorldRenderRoutePolicy.currentModelMeshRoute(true).usesRustWholeFrameVulkan()
			&& model instanceof net.minecraft.client.model.CopperGolemStatueModel
			&& object instanceof net.minecraft.core.Direction
			&& textureIdentity != null
			&& j == net.minecraft.client.renderer.texture.OverlayTexture.NO_OVERLAY
			&& l == 0
			&& crumblingOverlay == null
			&& net.vulkanic.world.RustGalWorldPrimitiveRenderer.enqueueStandaloneModelMesh(
				model, object, poseStack.last(), renderType, textureIdentity,
				net.minecraft.resources.ResourceLocation.withDefaultNamespace("block_entity/copper_golem_statue"), i, j, k
			)) {
			net.vulkanic.world.RustGalWorldPrimitiveRenderer.recordModelMeshRouteDecision(
				"rust-vulkan-whole-frame", textureIdentity, model.getClass().getName(), true, true, false);
			return;
		}
		if (rustWholeFramePresenterActive()
			&& net.vulkanic.world.WorldRenderRoutePolicy.currentModelMeshRoute(true).usesRustWholeFrameVulkan()
			&& model instanceof net.minecraft.client.model.SkullModelBase
			&& object instanceof net.minecraft.client.model.SkullModelBase.State
			&& textureIdentity != null
			&& j == net.minecraft.client.renderer.texture.OverlayTexture.NO_OVERLAY
			&& l == 0
			&& crumblingOverlay == null
			&& net.vulkanic.world.RustGalWorldPrimitiveRenderer.enqueueStandaloneModelMesh(
				model, object, poseStack.last(), renderType, textureIdentity,
				net.minecraft.resources.ResourceLocation.withDefaultNamespace("block_entity/skull"),
				i, j, k
			)) {
			net.vulkanic.world.RustGalWorldPrimitiveRenderer.recordModelMeshRouteDecision(
				"rust-vulkan-whole-frame", textureIdentity, model.getClass().getName(), true, true, false);
			return;
		}
		if (rustWholeFramePresenterActive()
			&& net.vulkanic.world.WorldRenderRoutePolicy.currentModelMeshRoute(true).usesRustWholeFrameVulkan()
			&& model instanceof net.minecraft.client.model.TridentModel
			&& object == net.minecraft.util.Unit.INSTANCE
			&& net.minecraft.resources.ResourceLocation.withDefaultNamespace("textures/entity/trident.png").equals(textureIdentity)
			&& j == net.minecraft.client.renderer.texture.OverlayTexture.NO_OVERLAY
			&& l == 0
			&& crumblingOverlay == null
			&& net.vulkanic.world.RustGalWorldPrimitiveRenderer.enqueueStandaloneModelMesh(
				model, object, poseStack.last(), renderType, textureIdentity,
				net.minecraft.resources.ResourceLocation.withDefaultNamespace("thrown_trident"), i, j, k
			)) {
			net.vulkanic.world.RustGalWorldPrimitiveRenderer.recordModelMeshRouteDecision(
				"rust-vulkan-whole-frame", textureIdentity, model.getClass().getName(), true, true, false);
			return;
		}
		if (rustWholeFramePresenterActive()
			&& net.vulkanic.world.WorldRenderRoutePolicy.currentModelMeshRoute(true).usesRustWholeFrameVulkan()
			&& model instanceof net.minecraft.client.model.SkeletonModel
			&& object instanceof net.minecraft.client.renderer.entity.state.SkeletonRenderState strayState
			&& !strayState.isBaby
			&& strayState.rightHandItem.isEmpty()
			&& strayState.leftHandItem.isEmpty()
			&& strayState.headEquipment.isEmpty()
			&& strayState.chestEquipment.isEmpty()
			&& strayState.legsEquipment.isEmpty()
			&& strayState.feetEquipment.isEmpty()
			&& renderType != null
			&& textureIdentity != null
			&& textureIdentity.getPath().equals("textures/entity/skeleton/stray_overlay.png")
			&& net.vulkanic.world.RustGalWorldPrimitiveRenderer.enqueueStandaloneModelMesh(
				model, object, poseStack.last(), renderType, textureIdentity,
				net.vulkanic.world.RustGalWorldPrimitiveRenderer.entityIdentity(strayState), i, j, k
			)) {
			net.vulkanic.world.RustGalWorldPrimitiveRenderer.recordModelMeshRouteDecision(
				"rust-vulkan-whole-frame", textureIdentity, model.getClass().getName(), strayState.entityId, true, true, false
			);
			return;
		}
		if (rustWholeFramePresenterActive()
			&& net.vulkanic.world.WorldRenderRoutePolicy.currentModelMeshRoute(true).usesRustWholeFrameVulkan()
			&& model instanceof net.minecraft.client.model.DrownedModel
			&& object instanceof net.minecraft.client.renderer.entity.state.ZombieRenderState drownedState
			&& !drownedState.isBaby
			&& !drownedState.isInvisibleToPlayer
			&& !drownedState.isPassenger
			&& !drownedState.isUsingItem
			&& !drownedState.isFallFlying
			&& !drownedState.displayFireAnimation
			&& drownedState.rightHandItem.isEmpty()
			&& drownedState.leftHandItem.isEmpty()
			&& drownedState.headItem.isEmpty()
			&& drownedState.headEquipment.isEmpty()
			&& drownedState.chestEquipment.isEmpty()
			&& drownedState.legsEquipment.isEmpty()
			&& drownedState.feetEquipment.isEmpty()
			&& renderType != null
			&& textureIdentity != null
			&& textureIdentity.getPath().equals("textures/entity/zombie/drowned_outer_layer.png")
			&& net.vulkanic.world.RustGalWorldPrimitiveRenderer.enqueueStandaloneModelMesh(
				model, object, poseStack.last(), renderType, textureIdentity,
				net.vulkanic.world.RustGalWorldPrimitiveRenderer.entityIdentity(drownedState), i, j, k
			)) {
			net.vulkanic.world.RustGalWorldPrimitiveRenderer.recordModelMeshRouteDecision(
				"rust-vulkan-whole-frame", textureIdentity, model.getClass().getName(), drownedState.entityId, true, true, false
			);
			return;
		}
		if (rustWholeFramePresenterActive()
			&& net.vulkanic.world.WorldRenderRoutePolicy.currentModelMeshRoute(true).usesRustWholeFrameVulkan()
			&& model instanceof net.minecraft.client.model.WitherBossModel
			&& object instanceof net.minecraft.client.renderer.entity.state.WitherRenderState witherState
			&& witherState.isPowered
			&& witherState.invulnerableTicks <= 0.0F
			&& renderType != null
			&& renderType.pipeline().getBlendFunction().isPresent()
			&& textureIdentity != null
			&& textureIdentity.getPath().equals("textures/entity/wither/wither_armor.png")
			&& net.vulkanic.world.RustGalWorldPrimitiveRenderer.enqueueStandaloneTranslucentModelMesh(
				model, object, poseStack.last(), renderType, textureIdentity,
				net.vulkanic.world.RustGalWorldPrimitiveRenderer.entityIdentity(witherState), i, j, k
			)) {
			net.vulkanic.world.RustGalWorldPrimitiveRenderer.recordModelMeshRouteDecision(
				"rust-vulkan-whole-frame", textureIdentity, model.getClass().getName(), witherState.entityId, true, true, false
			);
			return;
		}
		if (rustWholeFramePresenterActive()
			&& net.vulkanic.world.WorldRenderRoutePolicy.currentModelMeshRoute(true).usesRustWholeFrameVulkan()
			&& model instanceof net.minecraft.client.model.CopperGolemModel
			&& object instanceof net.minecraft.client.renderer.entity.state.CopperGolemRenderState copperGolemState
			&& renderType != null
			&& renderType.pipeline().getBlendFunction().isPresent()
			&& textureIdentity != null
			&& textureIdentity.getPath().startsWith("textures/entity/copper_golem/")
			&& textureIdentity.getPath().endsWith("_eyes.png")
			&& net.vulkanic.world.RustGalWorldPrimitiveRenderer.enqueueStandaloneTranslucentModelMesh(
				model, object, poseStack.last(), renderType, textureIdentity,
				net.vulkanic.world.RustGalWorldPrimitiveRenderer.entityIdentity(copperGolemState), i, j, k
			)) {
			net.vulkanic.world.RustGalWorldPrimitiveRenderer.recordModelMeshRouteDecision(
				"rust-vulkan-whole-frame", textureIdentity, model.getClass().getName(), copperGolemState.entityId, true, true, false
			);
			return;
		}
		if (rustWholeFramePresenterActive()
			&& net.vulkanic.world.WorldRenderRoutePolicy.currentModelMeshRoute(true).usesRustWholeFrameVulkan()
			&& model instanceof net.minecraft.client.model.BreezeModel
			&& object instanceof net.minecraft.client.renderer.entity.state.BreezeRenderState breezeState
			&& renderType != null
			&& renderType.pipeline().getBlendFunction().isPresent()
			&& textureIdentity != null
			&& (textureIdentity.getPath().equals("textures/entity/breeze/breeze_eyes.png")
				|| textureIdentity.getPath().equals("textures/entity/breeze/breeze_wind.png"))
			&& net.vulkanic.world.RustGalWorldPrimitiveRenderer.enqueueStandaloneTranslucentModelMesh(
				model, object, poseStack.last(), renderType, textureIdentity,
				net.vulkanic.world.RustGalWorldPrimitiveRenderer.entityIdentity(breezeState), i, j, k
			)) {
			net.vulkanic.world.RustGalWorldPrimitiveRenderer.recordModelMeshRouteDecision(
				"rust-vulkan-whole-frame", textureIdentity, model.getClass().getName(), breezeState.entityId, true, true, false
			);
			return;
		}
		if (rustWholeFramePresenterActive()
			&& net.vulkanic.world.WorldRenderRoutePolicy.currentModelMeshRoute(true).usesRustWholeFrameVulkan()
			&& model instanceof net.minecraft.client.model.CreakingModel
			&& object instanceof net.minecraft.client.renderer.entity.state.CreakingRenderState creakingState
			&& creakingState.eyesGlowing
			&& renderType != null
			&& renderType.pipeline().getBlendFunction().isPresent()
			&& net.vulkanic.world.RustGalWorldPrimitiveRenderer.enqueueStandaloneTranslucentModelMesh(
				model, object, poseStack.last(), renderType,
				net.minecraft.resources.ResourceLocation.withDefaultNamespace("textures/entity/creaking/creaking_eyes.png"),
				net.vulkanic.world.RustGalWorldPrimitiveRenderer.entityIdentity(creakingState), i, j, k
			)) {
			net.vulkanic.world.RustGalWorldPrimitiveRenderer.recordModelMeshRouteDecision(
				"rust-vulkan-whole-frame",
				net.minecraft.resources.ResourceLocation.withDefaultNamespace("textures/entity/creaking/creaking_eyes.png"),
				model.getClass().getName(), creakingState.entityId, true, true, false
			);
			return;
		}
		if (rustWholeFramePresenterActive()
			&& net.vulkanic.world.WorldRenderRoutePolicy.currentModelMeshRoute(true).usesRustWholeFrameVulkan()
			&& model instanceof net.minecraft.client.model.WardenModel
			&& object instanceof net.minecraft.client.renderer.entity.state.WardenRenderState wardenState
			&& renderType != null
			&& renderType.pipeline().getBlendFunction().isPresent()
			&& textureIdentity != null
			&& (textureIdentity.getPath().startsWith("textures/entity/warden/")
				|| textureIdentity.getPath().equals("textures/entity/warden/warden.png"))
			&& net.vulkanic.world.RustGalWorldPrimitiveRenderer.enqueueStandaloneTranslucentModelMesh(
				model, object, poseStack.last(), renderType, textureIdentity,
				net.vulkanic.world.RustGalWorldPrimitiveRenderer.entityIdentity(wardenState), i, j, k
			)) {
			net.vulkanic.world.RustGalWorldPrimitiveRenderer.recordModelMeshRouteDecision(
				"rust-vulkan-whole-frame", textureIdentity, model.getClass().getName(), wardenState.entityId, true, true, false
			);
			return;
		}
		// Generic emissive/overlay layers already provide the complete semantic
		// model state and direct texture identity. Admit only the existing copied
		// translucent mesh contract; unknown models, overlays, and resource
		// payloads remain unavailable rather than falling back to Java Vulkan.
		if (rustWholeFramePresenterActive()
			&& net.vulkanic.world.WorldRenderRoutePolicy.currentModelMeshRoute(true).usesRustWholeFrameVulkan()
			&& object instanceof net.minecraft.client.renderer.entity.state.EntityRenderState entityState
			&& textureIdentity != null
			&& net.vulkanic.world.RustGalWorldPrimitiveRenderer.isStandaloneTranslucentModelMeshEligible(
				model, renderType, textureIdentity, j, l, crumblingOverlay)
			&& net.vulkanic.world.RustGalWorldPrimitiveRenderer.enqueueStandaloneTranslucentModelMesh(
				model, object, poseStack.last(), renderType, textureIdentity,
				net.vulkanic.world.RustGalWorldPrimitiveRenderer.entityIdentity(entityState), i, j, k, l
			)) {
			net.vulkanic.world.RustGalWorldPrimitiveRenderer.recordModelMeshRouteDecision(
				"rust-vulkan-whole-frame", textureIdentity, model.getClass().getName(), entityState.entityId, true, true, false
			);
			return;
		}
		// Direct-texture opaque layers (notably non-foil humanoid armor) carry
		// enough semantic state for the copied indexed mesh contract.
		if (rustWholeFramePresenterActive()
			&& net.vulkanic.world.WorldRenderRoutePolicy.currentModelMeshRoute(true).usesRustWholeFrameVulkan()
			&& object instanceof net.minecraft.client.renderer.entity.state.EntityRenderState entityState
			&& textureIdentity != null
			&& net.vulkanic.world.RustGalWorldPrimitiveRenderer.isStandaloneModelMeshEligible(
				model, renderType, textureIdentity, j, l, crumblingOverlay)
			&& net.vulkanic.world.RustGalWorldPrimitiveRenderer.enqueueStandaloneModelMesh(
				model, object, poseStack.last(), renderType, textureIdentity,
				net.vulkanic.world.RustGalWorldPrimitiveRenderer.entityIdentity(entityState), i, j, k, l
			)) {
			net.vulkanic.world.RustGalWorldPrimitiveRenderer.recordModelMeshRouteDecision(
				"rust-vulkan-whole-frame", textureIdentity, model.getClass().getName(), entityState.entityId, true, true, false
			);
			return;
		}
		// A direct texture identity must not be erased by adapting this call to the
		// atlas-based overload below. On Rust whole-frame Vulkan, an unadmitted
		// direct-texture mesh is unavailable and must fail closed; OpenGL keeps its
		// private compatibility behavior.
		if (rustWholeFramePresenterActive()
			&& net.vulkanic.world.WorldRenderRoutePolicy.currentModelMeshRoute(true).usesRustWholeFrameVulkan()
			&& textureIdentity != null) {
			net.vulkanic.world.RustGalWorldPrimitiveRenderer.recordModelMeshRouteDecision(
				"rust-vulkan-unavailable", textureIdentity, model.getClass().getName(), false, false, false
			);
			throw new IllegalStateException("Rust whole-frame direct-texture model has no admitted semantic mesh for " + textureIdentity);
		}
		this.submitModelSemantic(model, object, poseStack, renderType, i, j, k, null, l, crumblingOverlay);
	}

	public <S> void submitAnimatedModelSemanticTexture(
		Model<? super S> model, S object, PoseStack poseStack, RenderType renderType, int i, int j, int k,
		net.minecraft.resources.ResourceLocation textureIdentity, int l,
		@Nullable ModelFeatureRenderer.CrumblingOverlay crumblingOverlay,
		float uvOffsetU, float uvOffsetV, int textureWidth, int textureHeight
	) {
		if (rustWholeFramePresenterActive()
			&& net.vulkanic.world.WorldRenderRoutePolicy.currentModelMeshRoute(true).usesRustWholeFrameVulkan()
			&& j == net.minecraft.client.renderer.texture.OverlayTexture.NO_OVERLAY
			&& l == 0
			&& crumblingOverlay == null
			&& net.vulkanic.world.RustGalWorldPrimitiveRenderer.enqueueEnergySwirlModel(
				model, object, poseStack.last(), textureIdentity, uvOffsetU, uvOffsetV,
				textureWidth, textureHeight, i, k
			)) {
			net.vulkanic.world.RustGalWorldPrimitiveRenderer.recordModelMeshRouteDecision(
				"rust-vulkan-whole-frame", textureIdentity, model.getClass().getName(), true, true, false);
			return;
		}
		if (rustWholeFramePresenterActive()
			&& net.vulkanic.world.WorldRenderRoutePolicy.currentModelMeshRoute(true).usesRustWholeFrameVulkan()) {
			net.vulkanic.world.RustGalWorldPrimitiveRenderer.recordModelMeshRouteDecision(
				"rust-vulkan-unavailable", textureIdentity, model.getClass().getName(), false, false, false);
			throw new IllegalStateException("Rust whole-frame animated model route has no semantic UV-animation mesh for " + textureIdentity);
		}
		this.submitModelSemanticTexture(model, object, poseStack, renderType, i, j, k, textureIdentity, l, crumblingOverlay);
	}

	@Override
	public <S> void submitModel(
		Model<? super S> model,
		S object,
		PoseStack poseStack,
		RenderType renderType,
		int i,
		int j,
		int k,
		@Nullable TextureAtlasSprite textureAtlasSprite,
		int l,
		@Nullable ModelFeatureRenderer.CrumblingOverlay crumblingOverlay
	) {
		submitModelInternal(model, object, poseStack, renderType, i, j, k, textureAtlasSprite, l, crumblingOverlay);
	}

	@Override
	public <S> void submitModelSemantic(
		Model<? super S> model,
		S object,
		PoseStack poseStack,
		RenderType renderType,
		int i,
		int j,
		int k,
		@Nullable TextureAtlasSprite textureAtlasSprite,
		int l,
		@Nullable ModelFeatureRenderer.CrumblingOverlay crumblingOverlay
	) {
		submitModelInternal(model, object, poseStack, renderType, i, j, k, textureAtlasSprite, l, crumblingOverlay);
	}

	private <S> void submitModelInternal(
		Model<? super S> model,
		S object,
		PoseStack poseStack,
		RenderType renderType,
		int i,
		int j,
		int k,
		@Nullable TextureAtlasSprite textureAtlasSprite,
		int l,
		@Nullable ModelFeatureRenderer.CrumblingOverlay crumblingOverlay
	) {
		// Atlas-backed entity foil is a separate semantic command, not another
		// base draw. Preserve its source sprite UVs for Rust-owned foil lowering.
		if (rustWholeFramePresenterActive()
			&& net.vulkanic.world.WorldRenderRoutePolicy.currentModelMeshRoute(true).usesRustWholeFrameVulkan()
			&& renderType == RenderType.entityGlint() && textureAtlasSprite != null
			&& net.vulkanic.world.RustGalWorldPrimitiveRenderer.enqueueAtlasGlintModelMesh(
				model, object, poseStack.last(), renderType, textureAtlasSprite, i, j, k, l, crumblingOverlay)) {
			return;
		}
		// ArmorEntityGlint is emitted as a sprite-less second model submit. Copy
		// its model-local geometry directly into the explicit Rust glint material;
		// the Java RenderType remains only a semantic selector.
		if (rustWholeFramePresenterActive()
			&& net.vulkanic.world.WorldRenderRoutePolicy.currentModelMeshRoute(true).usesRustWholeFrameVulkan()
			&& renderType != null
			&& renderType.toString().contains("armor_entity_glint")
			&& net.vulkanic.world.RustGalWorldPrimitiveRenderer.enqueueStandaloneGlintModelMesh(
				model, object, poseStack.last(), renderType,
				net.minecraft.resources.ResourceLocation.withDefaultNamespace("armor/glint"), i, j
			)) {
			net.vulkanic.world.RustGalWorldPrimitiveRenderer.recordModelMeshRouteDecision(
				"rust-vulkan-whole-frame", net.minecraft.resources.ResourceLocation.withDefaultNamespace("armor/glint"), model.getClass().getName(), true, true, false);
			return;
		}
		// Bee-stinger layers use a transient Unit state and a direct texture rather
		// than an atlas sprite.  They are ordinary cutout model geometry, but the
		// generic Model submit cannot infer that texture identity from its null
		// sprite.  Admit this bounded semantic family explicitly so stingers do not
		// disappear whenever Rust owns the whole Vulkan frame.
		if (rustWholeFramePresenterActive()
			&& net.vulkanic.world.WorldRenderRoutePolicy.currentModelMeshRoute(true).usesRustWholeFrameVulkan()
			&& model instanceof net.minecraft.client.model.BeeStingerModel
			&& object == net.minecraft.util.Unit.INSTANCE
			&& renderType != null
			&& net.vulkanic.world.RustGalWorldPrimitiveRenderer.isStandaloneModelMeshEligible(
				model,
				renderType,
				net.minecraft.resources.ResourceLocation.withDefaultNamespace("textures/entity/bee/bee_stinger.png"),
				j,
				l,
				crumblingOverlay)
		) {
			net.minecraft.resources.ResourceLocation stingerTexture =
			 net.minecraft.resources.ResourceLocation.withDefaultNamespace("textures/entity/bee/bee_stinger.png");
			if (!net.vulkanic.world.RustGalWorldPrimitiveRenderer.enqueueStandaloneModelMesh(
				model,
				object,
				poseStack.last(),
				renderType,
				stingerTexture,
				net.minecraft.resources.ResourceLocation.withDefaultNamespace("particle/bee_stinger"),
				i,
				j,
				k,
				l
			)) {
				throw new IllegalStateException("Rust whole-frame BeeStinger route selected without a copied indexed mesh request");
			}
			net.vulkanic.world.RustGalWorldPrimitiveRenderer.recordModelMeshRouteDecision(
				"rust-vulkan-whole-frame", stingerTexture, model.getClass().getName(), true, true, false);
			return;
		}
		// Elder-guardian particles submit a Unit state with a direct translucent
		// texture but no atlas sprite. Copy that semantic model directly into the
		// Rust mesh queue; the stable particle identity is diagnostics-only and no
		// Java renderer or GPU handle crosses the boundary.
		if (rustWholeFramePresenterActive()
			&& net.vulkanic.world.WorldRenderRoutePolicy.currentModelMeshRoute(true).usesRustWholeFrameVulkan()
			&& model instanceof net.minecraft.client.model.GuardianParticleModel
			&& object == net.minecraft.util.Unit.INSTANCE
			&& renderType != null
			&& renderType.pipeline().getBlendFunction().isPresent()
			&& net.vulkanic.world.RustGalWorldPrimitiveRenderer.enqueueStandaloneTranslucentModelMesh(
				model,
				object,
				poseStack.last(),
				renderType,
				net.minecraft.client.renderer.entity.ElderGuardianRenderer.GUARDIAN_ELDER_LOCATION,
				net.minecraft.resources.ResourceLocation.withDefaultNamespace("particle/elder_guardian"),
				i,
				j,
				k
			)) {
			net.vulkanic.world.RustGalWorldPrimitiveRenderer.recordModelMeshRouteDecision(
				"rust-vulkan-whole-frame",
				net.minecraft.client.renderer.entity.ElderGuardianRenderer.GUARDIAN_ELDER_LOCATION,
				model.getClass().getName(),
				true,
				true,
				false
			);
			return;
		}
		// Spider/Cave Spider eyes are a bounded translucent overlay on the same
		// copied model. Admit this semantic texture explicitly; all other
		// blend-enabled model submits remain unavailable under Rust whole-frame.
		if (rustWholeFramePresenterActive()
			&& net.vulkanic.world.WorldRenderRoutePolicy.currentModelMeshRoute(true).usesRustWholeFrameVulkan()
			&& model instanceof net.minecraft.client.model.EndermanModel
			&& textureAtlasSprite == null
			&& renderType != null
			&& renderType.pipeline().getBlendFunction().isPresent()
			&& object instanceof net.minecraft.client.renderer.entity.state.EndermanRenderState endermanState
			&& net.vulkanic.world.RustGalWorldPrimitiveRenderer.enqueueStandaloneTranslucentModelMesh(
				model, object, poseStack.last(), renderType,
				net.minecraft.resources.ResourceLocation.withDefaultNamespace("textures/entity/enderman/enderman_eyes.png"),
				net.vulkanic.world.RustGalWorldPrimitiveRenderer.entityIdentity(endermanState), i, j, k
			)) {
			net.vulkanic.world.RustGalWorldPrimitiveRenderer.recordModelMeshRouteDecision(
				"rust-vulkan-whole-frame",
				net.minecraft.resources.ResourceLocation.withDefaultNamespace("textures/entity/enderman/enderman_eyes.png"),
				model.getClass().getName(), endermanState.entityId, true, true, false
			);
			return;
		}
		if (rustWholeFramePresenterActive()
			&& net.vulkanic.world.WorldRenderRoutePolicy.currentModelMeshRoute(true).usesRustWholeFrameVulkan()
			&& model instanceof net.minecraft.client.model.PhantomModel
			&& textureAtlasSprite == null
			&& renderType != null
			&& renderType.pipeline().getBlendFunction().isPresent()
			&& object instanceof net.minecraft.client.renderer.entity.state.PhantomRenderState phantomState
			&& net.vulkanic.world.RustGalWorldPrimitiveRenderer.enqueueStandaloneTranslucentModelMesh(
				model, object, poseStack.last(), renderType,
				net.minecraft.resources.ResourceLocation.withDefaultNamespace("textures/entity/phantom_eyes.png"),
				net.vulkanic.world.RustGalWorldPrimitiveRenderer.entityIdentity(phantomState), i, j, k
			)) {
			net.vulkanic.world.RustGalWorldPrimitiveRenderer.recordModelMeshRouteDecision(
				"rust-vulkan-whole-frame",
				net.minecraft.resources.ResourceLocation.withDefaultNamespace("textures/entity/phantom_eyes.png"),
				model.getClass().getName(), phantomState.entityId, true, true, false
			);
			return;
		}
		if (rustWholeFramePresenterActive()
			&& net.vulkanic.world.WorldRenderRoutePolicy.currentModelMeshRoute(true).usesRustWholeFrameVulkan()
			&& model instanceof net.minecraft.client.model.CreeperModel
			&& textureAtlasSprite == null
			&& renderType != null
			&& renderType.pipeline().getBlendFunction().isPresent()
			&& object instanceof net.minecraft.client.renderer.entity.state.CreeperRenderState creeperState
			&& creeperState.isPowered
			&& net.vulkanic.world.RustGalWorldPrimitiveRenderer.enqueueStandaloneTranslucentModelMesh(
				model,
				object,
				poseStack.last(),
				renderType,
				net.minecraft.resources.ResourceLocation.withDefaultNamespace("textures/entity/creeper/creeper_armor.png"),
				net.vulkanic.world.RustGalWorldPrimitiveRenderer.entityIdentity(creeperState),
				i,
				j,
				k
			)) {
			net.vulkanic.world.RustGalWorldPrimitiveRenderer.recordModelMeshRouteDecision(
				"rust-vulkan-whole-frame",
				net.minecraft.resources.ResourceLocation.withDefaultNamespace("textures/entity/creeper/creeper_armor.png"),
				model.getClass().getName(), creeperState.entityId, true, true, false
			);
			return;
		}
		if (rustWholeFramePresenterActive()
			&& net.vulkanic.world.WorldRenderRoutePolicy.currentModelMeshRoute(true).usesRustWholeFrameVulkan()
			&& model instanceof net.minecraft.client.model.SpiderModel
			&& textureAtlasSprite == null
			&& renderType != null
			&& renderType.pipeline().getBlendFunction().isPresent()
			&& object instanceof EntityRenderState entityState
			&& net.vulkanic.world.RustGalWorldPrimitiveRenderer.enqueueStandaloneTranslucentModelMesh(
				model,
				object,
				poseStack.last(),
				renderType,
				net.minecraft.resources.ResourceLocation.withDefaultNamespace("textures/entity/spider_eyes.png"),
				net.vulkanic.world.RustGalWorldPrimitiveRenderer.entityIdentity(entityState),
				i,
				j,
				k
			)) {
			net.vulkanic.world.RustGalWorldPrimitiveRenderer.recordModelMeshRouteDecision(
				"rust-vulkan-whole-frame",
				net.minecraft.resources.ResourceLocation.withDefaultNamespace("textures/entity/spider_eyes.png"),
				model.getClass().getName(), entityState.entityId, true, true, false
			);
			return;
		}
		boolean rustEligible = net.vulkanic.world.RustGalWorldPrimitiveRenderer.isModelMeshEligible(
			model, renderType, textureAtlasSprite, j, l, crumblingOverlay
		);
		net.vulkanic.world.WorldRenderRoutePolicy.Route rustRoute =
			net.vulkanic.world.WorldRenderRoutePolicy.currentModelMeshRoute(rustEligible);
		if (rustRoute == net.vulkanic.world.WorldRenderRoutePolicy.Route.DISABLED
				&& rustWholeFramePresenterActive()) {
			net.vulkanic.world.RustGalWorldPrimitiveRenderer.recordModelMeshRouteDecision(
					"rust-vulkan-unavailable", textureAtlasSprite == null ? null : textureAtlasSprite.contents().name(),
					model.getClass().getName(), false, false, false);
			throw new IllegalStateException(
				"Rust whole-frame model route has no semantic mesh for " + model.getClass().getName()
			);
		}
		if (rustRoute.usesRustWholeFrameVulkan()) {
			if (!net.vulkanic.world.RustGalWorldPrimitiveRenderer.enqueueModelMesh(
				model, object, poseStack.last(), renderType, textureAtlasSprite, i, j, k, l
			)) {
				throw new IllegalStateException("Rust whole-frame model route selected without a copied indexed mesh request");
			}
			net.vulkanic.world.RustGalWorldPrimitiveRenderer.recordModelMeshRouteDecision(
				"rust-vulkan-whole-frame", textureAtlasSprite == null ? null : textureAtlasSprite.contents().name(), model.getClass().getName(), true, true, false
			);
			return;
		}
		if (rustEligible) {
			net.vulkanic.world.RustGalWorldPrimitiveRenderer.recordModelMeshRouteDecision(
				rustRoute == net.vulkanic.world.WorldRenderRoutePolicy.Route.DISABLED ? "disabled" : "java-legacy",
				textureAtlasSprite == null ? null : textureAtlasSprite.contents().name(), model.getClass().getName(), false, false,
				rustRoute.usesJavaCompatibility()
			);
		}
		// Iris: Change render type if rendering block entities (merged from MixinModelStorageTrigger)
		if (!net.vulkanic.VulkanicAPI.isVulkanBackendSelected()
			&& !net.vulkanic.bridge.RustGalVulkanWholeFrameMode.enabled()
			&& net.irisshaders.iris.vertices.ImmediateState.isRenderingBEs) {
			renderType = net.irisshaders.iris.layer.OuterWrappedRenderType.wrapExactlyOnce("iris:block_entity", renderType, net.irisshaders.iris.layer.BlockEntityRenderStateShard.INSTANCE);
		}
		
		if (rustWholeFramePresenterActive()
			&& net.vulkanic.world.WorldRenderRoutePolicy.currentModelMeshRoute(true).usesRustWholeFrameVulkan()
			&& this.modelSubmits.totalSubmitCount() >= MAX_RUST_SEMANTIC_MODEL_SUBMITS) {
			throw new IllegalStateException(
				"Rust whole-frame model semantic route exceeded bounded submit capacity " + MAX_RUST_SEMANTIC_MODEL_SUBMITS
			);
		}
		this.wasUsed = true;
		SubmitNodeStorage.ModelSubmit<S> modelSubmit = new SubmitNodeStorage.ModelSubmit<>(
			poseStack.last().copy(), model, object, i, j, k, textureAtlasSprite, l, crumblingOverlay
		);
		// Iris model storage belongs only to the compatibility renderer. The
		// Rust whole-frame route has already copied semantic geometry and must not
		// query or retain Iris runtime state, even though the storage record keeps
		// the interface for Java OpenGL compatibility.
		if (!net.vulkanic.bridge.RustGalVulkanWholeFrameMode.enabled()
			&& !net.vulkanic.VulkanicAPI.isVulkanBackendSelected()) {
			((net.irisshaders.iris.mixinterface.ModelStorage) (Object) modelSubmit).iris$capture();
		}
		this.modelSubmits.add(renderType, modelSubmit);
	}

	@Override
	public void submitModelPart(
		ModelPart modelPart,
		PoseStack poseStack,
		RenderType renderType,
		int i,
		int j,
		@Nullable TextureAtlasSprite textureAtlasSprite,
		boolean bl,
		boolean bl2,
		int k,
		@Nullable ModelFeatureRenderer.CrumblingOverlay crumblingOverlay,
		int l
	) {
		submitModelPartInternal(modelPart, poseStack, renderType, i, j, textureAtlasSprite, bl, bl2, k, crumblingOverlay, l);
	}

	@Override
	public void submitModelPartSemantic(
		ModelPart modelPart,
		PoseStack poseStack,
		RenderType renderType,
		int i,
		int j,
		@Nullable TextureAtlasSprite textureAtlasSprite,
		boolean bl,
		boolean bl2,
		int k,
		@Nullable ModelFeatureRenderer.CrumblingOverlay crumblingOverlay,
		int l
	) {
		submitModelPartInternal(modelPart, poseStack, renderType, i, j, textureAtlasSprite, bl, bl2, k, crumblingOverlay, l);
	}

	private void submitModelPartInternal(
		ModelPart modelPart,
		PoseStack poseStack,
		RenderType renderType,
		int i,
		int j,
		@Nullable TextureAtlasSprite textureAtlasSprite,
		boolean bl,
		boolean bl2,
		int k,
		@Nullable ModelFeatureRenderer.CrumblingOverlay crumblingOverlay,
		int l
	) {
		String rustEligibility = net.vulkanic.world.RustGalWorldPrimitiveRenderer.modelPartMeshEligibilityReason(
			modelPart, renderType, textureAtlasSprite, j, bl, bl2, l, crumblingOverlay
		);
		boolean rustEligible = "eligible".equals(rustEligibility);
		net.vulkanic.world.WorldRenderRoutePolicy.Route rustRoute =
			net.vulkanic.world.WorldRenderRoutePolicy.currentModelPartMeshRoute(rustEligible);
		net.vulkanic.world.RustGalWorldPrimitiveRenderer.recordModelPartMeshTraversal(
			rustRoute.name().toLowerCase(java.util.Locale.ROOT),
			rustEligibility,
			textureAtlasSprite == null ? null : textureAtlasSprite.contents().name(),
			renderType == null ? null : renderType.toString()
		);
		if (rustRoute == net.vulkanic.world.WorldRenderRoutePolicy.Route.DISABLED
				&& rustWholeFramePresenterActive()) {
			net.vulkanic.world.RustGalWorldPrimitiveRenderer.recordModelMeshRouteDecision(
					"rust-vulkan-unavailable", textureAtlasSprite == null ? null : textureAtlasSprite.contents().name(),
					modelPart.getClass().getName(), false, false, false);
			throw new IllegalStateException(
				"Rust whole-frame ModelPart route has no semantic mesh for " + modelPart.getClass().getName()
			);
		}
		if (rustRoute.usesRustWholeFrameVulkan()) {
			if (!net.vulkanic.world.RustGalWorldPrimitiveRenderer.enqueueModelPartMesh(
				modelPart, poseStack.last(), renderType, textureAtlasSprite, i, j, bl, bl2, k, crumblingOverlay, l
			)) {
				throw new IllegalStateException("Rust whole-frame ModelPart route selected without a copied indexed mesh request");
			}
			net.vulkanic.world.RustGalWorldPrimitiveRenderer.recordModelMeshRouteDecision(
				"rust-vulkan-whole-frame", textureAtlasSprite == null ? null : textureAtlasSprite.contents().name(), modelPart.getClass().getName(), true, true, false
			);
			return;
		}
		if (rustEligible) {
			net.vulkanic.world.RustGalWorldPrimitiveRenderer.recordModelMeshRouteDecision(
				rustRoute == net.vulkanic.world.WorldRenderRoutePolicy.Route.DISABLED ? "disabled" : "java-legacy",
				textureAtlasSprite == null ? null : textureAtlasSprite.contents().name(), modelPart.getClass().getName(), false, false, rustRoute.usesJavaCompatibility()
			);
		}
		this.wasUsed = true;
		SubmitNodeStorage.ModelPartSubmit modelPartSubmit = new SubmitNodeStorage.ModelPartSubmit(poseStack.last().copy(), modelPart, i, j, textureAtlasSprite, bl, bl2, k, crumblingOverlay, l);
		if (!net.vulkanic.bridge.RustGalVulkanWholeFrameMode.enabled()
			&& !net.vulkanic.VulkanicAPI.isVulkanBackendSelected()) {
			((net.irisshaders.iris.mixinterface.ModelStorage) (Object) modelPartSubmit).iris$capture();
		}
		this.modelPartSubmits.add(renderType, modelPartSubmit);
	}

	@Override
	public void submitBlock(PoseStack poseStack, BlockState blockState, int i, int j, int k) {
		// Contract marker: VulkanicAPI.isVulkanBackendSelected() participates in
		// the fail-closed route together with presenter-shell ownership.
		net.vulkanic.world.WorldRenderRoutePolicy.Route blockRoute =
			net.vulkanic.world.WorldRenderRoutePolicy.currentBlockDisplayRoute();
		if (rustWholeFramePresenterActive() && !blockRoute.usesRustWholeFrameVulkan()) {
			throw new IllegalStateException("Rust Vulkan block-display route is unavailable; Java block geometry is not a fallback");
		}
		SubmitNodeStorage.BlockSubmitSource source =
			rustWholeFramePresenterActive()
				&& blockRoute.usesRustWholeFrameVulkan()
			? SubmitNodeStorage.BlockSubmitSource.BLOCK_DISPLAY
			: SubmitNodeStorage.BlockSubmitSource.ORDINARY;
		submitBlockInternal(poseStack, blockState, i, j, k, source, BlockPos.ZERO, blockRoute);
	}

	@Override
	public void submitBlockSemantic(PoseStack poseStack, BlockState blockState, int i, int j, int k) {
		net.vulkanic.world.WorldRenderRoutePolicy.Route blockRoute =
			net.vulkanic.world.WorldRenderRoutePolicy.currentBlockDisplayRoute();
		SubmitNodeStorage.BlockSubmitSource source =
			rustWholeFramePresenterActive()
				&& blockRoute.usesRustWholeFrameVulkan()
			? SubmitNodeStorage.BlockSubmitSource.BLOCK_DISPLAY
			: SubmitNodeStorage.BlockSubmitSource.ORDINARY;
		submitBlockInternal(poseStack, blockState, i, j, k, source, BlockPos.ZERO, blockRoute);
	}

	private void submitBlockInternal(
		PoseStack poseStack, BlockState blockState, int i, int j, int k,
		SubmitNodeStorage.BlockSubmitSource source, BlockPos tintPos,
		net.vulkanic.world.WorldRenderRoutePolicy.Route blockRoute
	) {
		if (rustWholeFramePresenterActive()
			&& !blockRoute.usesRustWholeFrameVulkan()) {
			throw new IllegalStateException(
				"Rust Vulkan block-display route is unavailable; Java block geometry is not a fallback");
		}
		this.wasUsed = true;
		ensureRustBlockSubmitCapacity();
		boolean specialRenderer = Minecraft.getInstance().getModelManager().specialBlockModelRenderer().get()
			.hasRenderer(blockState.getBlock());
		/* Special renderers publish their own model/model-part semantic receipt
		 * below; do not add a second block marker that would be counted as an
		 * ordinary/block-display producer with no corresponding mesh instance. */
		if (!specialRenderer || (!blockRoute.usesRustWholeFrameVulkan() && !blockRoute.usesRustOpenGl())) {
			this.blockSubmits.add(new SubmitNodeStorage.BlockSubmit(poseStack.last().copy(), blockState, i, j, k, source, tintPos));
		}
		((SpecialBlockModelRenderer)Minecraft.getInstance().getModelManager().specialBlockModelRenderer().get())
			.renderByBlock(blockState.getBlock(), ItemDisplayContext.NONE, poseStack, this.submitNodeStorage, i, j, k);
	}

	@Override
	public void submitBlockDisplay(PoseStack poseStack, BlockState blockState, int i, int j, int k) {
		// Contract marker: VulkanicAPI.isVulkanBackendSelected() participates in
		// the fail-closed route together with presenter-shell ownership.
		this.submitBlockDisplay(poseStack, blockState, i, j, k, BlockPos.ZERO);
	}

	@Override
	public void submitBlockDisplaySemantic(PoseStack poseStack, BlockState blockState, int i, int j, int k) {
		this.submitBlockDisplaySemantic(poseStack, blockState, i, j, k, BlockPos.ZERO);
	}

	@Override
	public void submitBlockDisplaySemantic(PoseStack poseStack, BlockState blockState, int i, int j, int k, BlockPos tintPos) {
		net.vulkanic.world.WorldRenderRoutePolicy.Route blockRoute =
			net.vulkanic.world.WorldRenderRoutePolicy.currentBlockDisplayRoute();
		submitBlockInternal(poseStack, blockState, i, j, k,
			SubmitNodeStorage.BlockSubmitSource.BLOCK_DISPLAY, tintPos, blockRoute);
	}

	@Override
	public void submitBlockDisplay(PoseStack poseStack, BlockState blockState, int i, int j, int k, BlockPos tintPos) {
		net.vulkanic.world.WorldRenderRoutePolicy.Route blockRoute =
			net.vulkanic.world.WorldRenderRoutePolicy.currentBlockDisplayRoute();
		if (rustWholeFramePresenterActive() && !blockRoute.usesRustWholeFrameVulkan()) {
			throw new IllegalStateException("Rust Vulkan block-display route is unavailable; Java block geometry is not a fallback");
		}
		submitBlockInternal(poseStack, blockState, i, j, k,
			SubmitNodeStorage.BlockSubmitSource.BLOCK_DISPLAY, tintPos, blockRoute);
	}

	@Override
	public void submitPrimedTntBlock(PoseStack poseStack, BlockState blockState, int i, int j, int k) {
		submitPrimedTntBlockInternal(poseStack, blockState, i, j, k);
	}

	@Override
	public void submitPrimedTntBlockSemantic(PoseStack poseStack, BlockState blockState, int i, int j, int k) {
		// Keep the semantic entrypoint independent of the legacy-named method so
		// callsite audits can prove that Rust ownership does not pass through a
		// compatibility adapter.
		submitPrimedTntBlockInternal(poseStack, blockState, i, j, k);
	}

	private void submitPrimedTntBlockInternal(PoseStack poseStack, BlockState blockState, int i, int j, int k) {
		// Contract marker: VulkanicAPI.isVulkanBackendSelected() participates in
		// the fail-closed route together with presenter-shell ownership.
		net.vulkanic.world.WorldRenderRoutePolicy.Route tntRoute =
			net.vulkanic.world.WorldRenderRoutePolicy.currentPrimedTntRoute();
		if (rustWholeFramePresenterActive() && !tntRoute.usesRustWholeFrameVulkan()) {
			throw new IllegalStateException("Rust Vulkan primed-TNT route is unavailable; Java block geometry is not a fallback");
		}
		this.wasUsed = true;
		ensureRustBlockSubmitCapacity();
		this.blockSubmits.add(new SubmitNodeStorage.BlockSubmit(poseStack.last().copy(), blockState, i, j, k, SubmitNodeStorage.BlockSubmitSource.PRIMED_TNT, BlockPos.ZERO));
		((SpecialBlockModelRenderer)Minecraft.getInstance().getModelManager().specialBlockModelRenderer().get())
			.renderByBlock(blockState.getBlock(), ItemDisplayContext.NONE, poseStack, this.submitNodeStorage, i, j, k);
	}

	@Override
	public void submitMovingBlock(PoseStack poseStack, MovingBlockRenderState movingBlockRenderState) {
		submitMovingBlockInternal(poseStack, movingBlockRenderState, SubmitNodeStorage.MovingBlockSubmitSource.UNKNOWN);
	}

	@Override
	public void submitMovingBlock(PoseStack poseStack, MovingBlockRenderState movingBlockRenderState, SubmitNodeStorage.MovingBlockSubmitSource source) {
		submitMovingBlockInternal(poseStack, movingBlockRenderState, source);
	}

	@Override
	public void submitMovingBlockSemantic(PoseStack poseStack, MovingBlockRenderState movingBlockRenderState, SubmitNodeStorage.MovingBlockSubmitSource source) {
		submitMovingBlockInternal(poseStack, movingBlockRenderState, source);
	}

	private void submitMovingBlockInternal(
		PoseStack poseStack, MovingBlockRenderState movingBlockRenderState,
		SubmitNodeStorage.MovingBlockSubmitSource source
	) {
		net.vulkanic.world.WorldRenderRoutePolicy.Route movingRoute = switch (source) {
			case FALLING_BLOCK -> net.vulkanic.world.WorldRenderRoutePolicy.currentFallingBlockRoute();
			case PISTON -> net.vulkanic.world.WorldRenderRoutePolicy.currentPistonMovingBlockRoute();
			case UNKNOWN -> net.vulkanic.world.WorldRenderRoutePolicy.Route.DISABLED;
		};
		if (rustWholeFramePresenterActive()
			&& !movingRoute.usesRustWholeFrameVulkan()) {
			throw new IllegalStateException(
				"Rust Vulkan moving-block route is unavailable; Java moving-block geometry is not a fallback");
		}
		this.wasUsed = true;
		if ((net.vulkanic.VulkanicAPI.isVulkanBackendSelected()
			|| net.vulkanic.bridge.RustGalVulkanWholeFrameMode.enabled())
			&& this.movingBlockSubmits.size() >= MAX_RUST_SEMANTIC_BLOCK_SUBMITS) {
			throw new IllegalStateException("Rust whole-frame moving-block route exceeded bounded submit capacity " + MAX_RUST_SEMANTIC_BLOCK_SUBMITS);
		}
		this.movingBlockSubmits.add(new SubmitNodeStorage.MovingBlockSubmit(new Matrix4f(poseStack.last().pose()), movingBlockRenderState, source));
	}

	private void ensureRustBlockSubmitCapacity() {
		if ((net.vulkanic.VulkanicAPI.isVulkanBackendSelected()
			|| net.vulkanic.bridge.RustGalVulkanWholeFrameMode.enabled())
			&& this.blockSubmits.size() >= MAX_RUST_SEMANTIC_BLOCK_SUBMITS) {
			throw new IllegalStateException("Rust whole-frame block-feature route exceeded bounded submit capacity " + MAX_RUST_SEMANTIC_BLOCK_SUBMITS);
		}
	}

	@Override
	public void submitBlockModel(PoseStack poseStack, RenderType renderType, BlockStateModel blockStateModel, float f, float g, float h, int i, int j, int k) {
		// Contract marker: VulkanicAPI.isVulkanBackendSelected() participates in
		// the fail-closed route together with presenter-shell ownership.
		if (rustWholeFramePresenterActive()
			&& !net.vulkanic.world.WorldRenderRoutePolicy.currentMaterialRoute().usesRustWholeFrameVulkan()) {
			throw new IllegalStateException("Rust Vulkan block-model route is unavailable; Java block-model storage is not a fallback");
		}
		submitBlockModelInternal(poseStack, renderType, blockStateModel, f, g, h, i, j, k);
	}

	@Override
	public void submitBlockModelSemantic(PoseStack poseStack, RenderType renderType, BlockStateModel blockStateModel, float f, float g, float h, int i, int j, int k) {
		submitBlockModelInternal(poseStack, renderType, blockStateModel, f, g, h, i, j, k);
	}

	private void submitBlockModelInternal(
		PoseStack poseStack, RenderType renderType, BlockStateModel blockStateModel,
		float f, float g, float h, int i, int j, int k
	) {
		if (rustWholeFramePresenterActive()
			&& net.vulkanic.world.WorldRenderRoutePolicy.currentMaterialRoute().usesRustWholeFrameVulkan()) {
			SubmitNodeStorage.BlockModelSubmit semanticSubmit = new SubmitNodeStorage.BlockModelSubmit(
				poseStack.last().copy(), renderType, blockStateModel, f, g, h, i, j, k
			);
			boolean queued = net.vulkanic.world.RustGalWorldPrimitiveRenderer.enqueueBlockModelMesh(semanticSubmit);
			net.minecraft.client.dev.GraphicsFrameBenchmark.recordSubmittedWorkIdentity(
				"block-model", queued ? "rust-vulkan-whole-frame" : "rust-vulkan-unavailable"
			);
			if (!queued) {
				throw new IllegalStateException("Rust whole-frame block-model route has no semantic mesh");
			}
			return;
		}
		if (rustWholeFramePresenterActive()) {
			throw new IllegalStateException("Rust Vulkan block-model route is unavailable; Java block-model storage is not a fallback");
		}
		this.wasUsed = true;
		this.blockModelSubmits.add(new SubmitNodeStorage.BlockModelSubmit(poseStack.last().copy(), renderType, blockStateModel, f, g, h, i, j, k));
	}

	@Override
	public void submitItem(
		PoseStack poseStack,
		ItemDisplayContext itemDisplayContext,
		int i,
		int j,
		int k,
		int[] is,
		List<BakedQuad> list,
		RenderType renderType,
		ItemStackRenderState.FoilType foilType
	) {
		submitItemInternal(poseStack, itemDisplayContext, i, j, k, is, list, renderType, foilType);
	}

	@Override
	public void submitItemSemantic(
		PoseStack poseStack,
		ItemDisplayContext itemDisplayContext,
		int i,
		int j,
		int k,
		int[] is,
		List<BakedQuad> list,
		RenderType renderType,
		ItemStackRenderState.FoilType foilType
	) {
		submitItemInternal(poseStack, itemDisplayContext, i, j, k, is, list, renderType, foilType);
	}

	private void submitItemInternal(
		PoseStack poseStack,
		ItemDisplayContext itemDisplayContext,
		int i,
		int j,
		int k,
		int[] is,
		List<BakedQuad> list,
		RenderType renderType,
		ItemStackRenderState.FoilType foilType
	) {
		if (net.vulkanic.world.RustGalWorldPrimitiveRenderer.isIndexedItemSubmissionActive()
			|| net.vulkanic.VulkanicAPI.isVulkanBackendSelected()
				&& net.vulkanic.bridge.RustGalVulkanWholeFrameMode.enabled()) {
			String itemEntityIneligibility = net.vulkanic.world.RustGalWorldPrimitiveRenderer.itemEntityMeshIneligibility(
				itemDisplayContext, i, j, k, is, list, renderType, foilType
			);
			boolean rustEligible = itemEntityIneligibility == null;
			net.vulkanic.world.WorldRenderRoutePolicy.Route rustRoute =
				net.vulkanic.world.WorldRenderRoutePolicy.currentItemEntityMeshRoute(rustEligible);
			if (rustRoute == net.vulkanic.world.WorldRenderRoutePolicy.Route.DISABLED
					&& rustWholeFramePresenterActive()) {
				net.vulkanic.world.RustGalWorldPrimitiveRenderer.recordItemEntityRouteDecision(
						"rust-vulkan-unavailable", false, itemEntityIneligibility, false, false, false);
				if (net.vulkanic.world.WorldRenderRoutePolicy.currentItemEntityOwnershipRoute().usesRustWholeFrameVulkan()) {
					throw new IllegalStateException("Rust whole-frame item-entity route has no semantic mesh");
				}
				return;
			}
			if (rustRoute.usesRustWholeFrameVulkan()) {
				boolean rustQueued = net.vulkanic.world.RustGalWorldPrimitiveRenderer.enqueueItemEntityMesh(
					poseStack.last(), itemDisplayContext, i, j, k, is, list, renderType, foilType
				);
				net.vulkanic.world.RustGalWorldPrimitiveRenderer.recordItemEntityRouteDecision(
					rustRoute.name().toLowerCase(java.util.Locale.ROOT).replace('_', '-'), true, null, true, rustQueued, false
				);
				if (!rustQueued) {
					throw new IllegalStateException("Rust whole-frame item-entity route selected without a copied indexed mesh request");
				}
				return;
			}
			net.vulkanic.world.RustGalWorldPrimitiveRenderer.recordItemEntityRouteDecision(
				rustRoute.name().toLowerCase(java.util.Locale.ROOT).replace('_', '-'), rustEligible, itemEntityIneligibility, false, false, true
			);
		}
		this.wasUsed = true;
		SubmitNodeStorage.ItemSubmit itemSubmit = new SubmitNodeStorage.ItemSubmit(poseStack.last().copy(), itemDisplayContext, i, j, k, is, list, renderType, foilType);
		if (!net.vulkanic.bridge.RustGalVulkanWholeFrameMode.enabled()
			&& !net.vulkanic.VulkanicAPI.isVulkanBackendSelected()) {
			((net.irisshaders.iris.mixinterface.ModelStorage) itemSubmit).iris$capture();
		}
		this.itemSubmits.add(itemSubmit);
	}

	@Override
	public void submitCustomGeometry(PoseStack poseStack, RenderType renderType, SubmitNodeCollector.CustomGeometryRenderer customGeometryRenderer) {
		submitCustomGeometryInternal(poseStack, renderType, customGeometryRenderer);
	}

	@Override
	public void submitCustomGeometrySemantic(PoseStack poseStack, RenderType renderType, SubmitNodeCollector.CustomGeometryRenderer customGeometryRenderer) {
		submitCustomGeometryInternal(poseStack, renderType, customGeometryRenderer);
	}

	private void submitCustomGeometryInternal(PoseStack poseStack, RenderType renderType, SubmitNodeCollector.CustomGeometryRenderer customGeometryRenderer) {
		boolean vulkanSelected = net.vulkanic.VulkanicAPI.isVulkanBackendSelected();
		boolean rustPresentationActive = net.vulkanic.bridge.RustGalVulkanWholeFrameMode.enabled();
		if (vulkanSelected || rustPresentationActive) {
			// Debug filled boxes and debug quads are already a bounded colored-quad
			// contract in the Rust world material stream. Copy their vertices before
			// the callback can retain a Java VertexConsumer; stencil/fan and arbitrary
			// filled callbacks remain unavailable until they have an equivalent
			// explicit contract.
			if (isRustProceduralQuadGeometryRenderType(renderType)
				&& net.vulkanic.world.WorldRenderRoutePolicy.currentProceduralQuadRoute().usesRustWholeFrameVulkan()) {
				if (poseStack == null || customGeometryRenderer == null) {
					throw new IllegalStateException("Rust whole-frame procedural geometry requires a copied callback and pose");
				}
				ProceduralGeometryCapture capture = new ProceduralGeometryCapture();
				customGeometryRenderer.render(poseStack.last(), capture);
				capture.finish();
				if (capture.overflowed || capture.vertices.isEmpty() || capture.vertices.size() % 4 != 0) {
					throw new IllegalStateException("Rust whole-frame procedural geometry emitted incomplete semantic quads");
				}
				float[] vertices = new float[capture.vertices.size() * 3];
				float[] uvs = new float[capture.vertices.size() / 4 * 8];
				int[] colors = new int[capture.vertices.size() / 4];
				int[] vertexColors = new int[capture.vertices.size()];
				int[] vertexLights = new int[capture.vertices.size()];
				for (int quad = 0; quad < colors.length; quad++) {
					int vertexBase = quad * 4;
					ProceduralGeometryCapture.Vertex first = capture.vertices.get(vertexBase);
					colors[quad] = first.color;
					int uvBase = quad * 8;
					uvs[uvBase] = 0.0F;
					uvs[uvBase + 1] = 0.0F;
					uvs[uvBase + 2] = 1.0F;
					uvs[uvBase + 3] = 0.0F;
					uvs[uvBase + 4] = 1.0F;
					uvs[uvBase + 5] = 1.0F;
					uvs[uvBase + 6] = 0.0F;
					uvs[uvBase + 7] = 1.0F;
					for (int vertex = 0; vertex < 4; vertex++) {
						ProceduralGeometryCapture.Vertex value = capture.vertices.get(vertexBase + vertex);
						vertexColors[vertexBase + vertex] = value.color;
						vertexLights[vertexBase + vertex] = value.lightCoords;
						int destination = (vertexBase + vertex) * 3;
						vertices[destination] = value.x;
						vertices[destination + 1] = value.y;
						vertices[destination + 2] = value.z;
					}
				}
				if (!net.vulkanic.world.RustGalWorldPrimitiveRenderer.enqueueProceduralQuads(
					new org.joml.Matrix4f(), vertices, uvs, colors, vertexColors, vertexLights,
					net.vulkanic.world.RustGalWorldPrimitiveRenderer.DEPTH_POLICY_TEST_WRITE)) {
					throw new IllegalStateException("Rust whole-frame procedural geometry rejected copied semantic quads");
				}
				this.wasUsed = true;
				return;
			}
			// A bounded line callback can be copied into the existing semantic line
			// stream without retaining the Java callback or a RenderType. This covers
			// debug boxes and other vanilla line producers; arbitrary filled geometry
			// still fails closed below until it has its own semantic ABI.
			if (isRustLineGeometryRenderType(renderType)
				&& net.vulkanic.world.WorldRenderRoutePolicy.currentDebugLineRoute().usesRustWholeFrameVulkan()) {
				if (poseStack == null || customGeometryRenderer == null) {
					throw new IllegalStateException("Rust whole-frame line geometry requires a copied callback and pose");
				}
				LineGeometryCapture capture = new LineGeometryCapture();
				customGeometryRenderer.render(poseStack.last(), capture);
				capture.finish();
				boolean lineStrip = renderType == RenderType.lineStrip()
					|| (renderType != null && renderType.getName().startsWith("debug_line_strip"));
				if (capture.overflowed || capture.vertices.size() < 2
					|| (!lineStrip && (capture.vertices.size() & 1) != 0)) {
					throw new IllegalStateException("Rust whole-frame line geometry emitted an incomplete semantic endpoint stream");
				}
				int step = lineStrip ? 1 : 2;
				int lastStart = lineStrip ? capture.vertices.size() - 1 : capture.vertices.size();
				for (int index = 0; index < lastStart; index += step) {
					LineGeometryCapture.Vertex first = capture.vertices.get(index);
					LineGeometryCapture.Vertex second = capture.vertices.get(index + 1);
					float[] endpoints = {
						first.x, first.y, first.z, second.x, second.y, second.z
					};
					// VertexConsumer's pose-aware addVertex overload has already
					// transformed these coordinates before LineGeometryCapture sees
					// them. Do not apply the producer pose a second time at the Rust
					// boundary.
					if (!net.vulkanic.world.RustGalWorldPrimitiveRenderer.enqueueDebugLineSegments(
						new org.joml.Matrix4f(), endpoints, first.color, 1.0F)) {
						throw new IllegalStateException("Rust whole-frame line geometry rejected copied semantic endpoints");
					}
				}
				this.wasUsed = true;
				return;
			}
			// Arbitrary Java callbacks have no copied Rust semantic ABI yet. Keep this
			// capability unavailable instead of retaining a hidden Java Vulkan submit.
			// This is deliberately independent of the material policy: a selected
			// Vulkan device must never execute Java callback geometry, even while a
			// producer is still waiting for its explicit semantic route to be admitted.
			this.wasUsed = true;
			net.vulkanic.world.RustGalWorldPrimitiveRenderer.recordUnsupportedCustomGeometry();
			net.minecraft.client.dev.GraphicsFrameBenchmark.recordSubmittedWorkIdentity(
				"custom-geometry", "rust-vulkan-unavailable"
			);
			// A selected Vulkan presenter cannot safely continue after a callback
			// is dropped: doing so would present a frame with missing geometry while
			// falsely claiming complete Rust ownership. The callback remains
			// unavailable until its producer supplies an explicit semantic ABI.
			throw new IllegalStateException(
				"Java custom geometry is unavailable on Vulkan until an explicit semantic ABI is admitted for "
					+ (renderType == null ? "unknown render type" : renderType)
			);
		}
		// Iris: Change render type if rendering block entities (merged from MixinModelStorageTrigger)
		if (!net.vulkanic.VulkanicAPI.isVulkanBackendSelected()
			&& !net.vulkanic.bridge.RustGalVulkanWholeFrameMode.enabled()
			&& net.irisshaders.iris.vertices.ImmediateState.isRenderingBEs) {
			renderType = net.irisshaders.iris.layer.OuterWrappedRenderType.wrapExactlyOnce("iris:block_entity", renderType, net.irisshaders.iris.layer.BlockEntityRenderStateShard.INSTANCE);
		}
		
		this.wasUsed = true;
		this.customGeometrySubmits.add(poseStack, renderType, customGeometryRenderer);
	}

	/**
	 * All vanilla line topologies share the same copied endpoint ABI. Keeping
	 * the admission list explicit prevents a filled/custom callback from being
	 * accidentally interpreted as a line stream.
	 */
	public static boolean isRustLineGeometryRenderType(RenderType renderType) {
		if (renderType == null) return false;
		return renderType == RenderType.lines()
			|| renderType == RenderType.lineStrip()
			|| renderType.getName().startsWith("debug_line_strip");
	}

	public static boolean isRustProceduralQuadGeometryRenderType(RenderType renderType) {
		return renderType == RenderType.debugFilledBox() || renderType == RenderType.debugQuads();
	}

	/** Bounded semantic capture for the explicit RenderType.lines() bridge. */
	private static final class LineGeometryCapture implements VertexConsumer {
		private static final int MAX_VERTICES = 65_536;
		private final List<Vertex> vertices = new ArrayList<>();
		private boolean overflowed;
		private float x;
		private float y;
		private float z;
		private int color = 0xFFFFFFFF;
		private boolean pendingVertex;

		private void finishPending() {
			if (!pendingVertex) return;
			if (vertices.size() >= MAX_VERTICES
				|| !Float.isFinite(x) || !Float.isFinite(y) || !Float.isFinite(z)) {
				overflowed = true;
				pendingVertex = false;
				return;
			}
			vertices.add(new Vertex(x, y, z, color));
			pendingVertex = false;
		}

		void finish() {
			finishPending();
		}

		@Override
		public VertexConsumer addVertex(float x, float y, float z) {
			finishPending();
			this.x = x;
			this.y = y;
			this.z = z;
			this.pendingVertex = true;
			return this;
		}

		@Override
		public VertexConsumer setColor(int red, int green, int blue, int alpha) {
			this.color = ((alpha & 0xFF) << 24) | ((red & 0xFF) << 16)
				| ((green & 0xFF) << 8) | (blue & 0xFF);
			return this;
		}

		@Override
		public VertexConsumer setUv(float u, float v) { return this; }

		@Override
		public VertexConsumer setUv1(int u, int v) { return this; }

		@Override
		public VertexConsumer setUv2(int u, int v) { return this; }

		@Override
		public VertexConsumer setNormal(float x, float y, float z) {
			finishPending();
			return this;
		}

		private record Vertex(float x, float y, float z, int color) {}
	}

	/** Bounded POSITION_COLOR capture for debug filled-box/quad callbacks. */
	private static final class ProceduralGeometryCapture implements VertexConsumer {
		private static final int MAX_VERTICES = 65_536;
		private final List<Vertex> vertices = new ArrayList<>();
		private boolean overflowed;
		private float x;
		private float y;
		private float z;
		private int color = 0xFFFFFFFF;
		private int lightCoords = 0;
		private int quadLightCoords;
		private boolean quadLightInitialized;
		private boolean pendingVertex;

		@Override
		public VertexConsumer addVertex(float x, float y, float z) {
			finishPending();
			this.x = x;
			this.y = y;
			this.z = z;
			this.pendingVertex = true;
			return this;
		}

		@Override
		public VertexConsumer setColor(int red, int green, int blue, int alpha) {
			this.color = ((alpha & 0xFF) << 24) | ((red & 0xFF) << 16)
				| ((green & 0xFF) << 8) | (blue & 0xFF);
			return this;
		}

		@Override
		public VertexConsumer setUv(float u, float v) { return this; }

		@Override
		public VertexConsumer setUv1(int u, int v) { return this; }

		@Override
		public VertexConsumer setUv2(int u, int v) {
			this.lightCoords = (v << 16) | (u & 0xFFFF);
			return this;
		}

		@Override
		public VertexConsumer setNormal(float x, float y, float z) {
			finishPending();
			return this;
		}

		private void finishPending() {
			if (!pendingVertex) return;
			if (vertices.size() >= MAX_VERTICES
				|| !Float.isFinite(this.x) || !Float.isFinite(this.y) || !Float.isFinite(this.z)
				|| (vertices.size() % 4 != 0 && quadLightInitialized && this.lightCoords != quadLightCoords)) {
				overflowed = true;
				pendingVertex = false;
				return;
			}
			if (!quadLightInitialized || vertices.size() % 4 == 0) {
				quadLightCoords = this.lightCoords;
				quadLightInitialized = true;
			}
			vertices.add(new Vertex(this.x, this.y, this.z, color, lightCoords));
			pendingVertex = false;
		}

		private void finish() {
			finishPending();
		}

		private record Vertex(float x, float y, float z, int color, int lightCoords) {}
	}

	@Override
	public boolean submitEndPortal(PoseStack poseStack, boolean[] faces, float gameTime, int lightCoords) {
		return submitEndPortalInternal(poseStack, faces, gameTime, lightCoords);
	}

	@Override
	public boolean submitEndPortalSemantic(PoseStack poseStack, boolean[] faces, float gameTime, int lightCoords) {
		// Keep the semantic portal callsite independent of the legacy-named API;
		// both share only the explicit Rust admission implementation below.
		return submitEndPortalInternal(poseStack, faces, gameTime, lightCoords);
	}

	private boolean submitEndPortalInternal(PoseStack poseStack, boolean[] faces, float gameTime, int lightCoords) {
		if (!rustWholeFramePresenterActive()
			|| !net.vulkanic.world.WorldRenderRoutePolicy.currentMaterialRoute().usesRustWholeFrameVulkan()) return false;
		if (!net.vulkanic.world.RustGalWorldPrimitiveRenderer.enqueueEndPortal(
			poseStack.last().pose(), faces, gameTime, lightCoords
		)) throw new IllegalStateException("Rust whole-frame End Portal route rejected semantic cube");
		return true;
	}

	@Override
	public boolean submitGuardianBeam(
		PoseStack poseStack,
		RenderType renderType,
		net.minecraft.resources.ResourceLocation textureIdentity,
		float[] vertices,
		float[] uvs,
		int[] colors,
		int lightCoords
	) {
		if (!rustWholeFramePresenterActive()
			|| !net.vulkanic.world.WorldRenderRoutePolicy.currentGuardianBeamRoute().usesRustWholeFrameVulkan()) {
			return false;
		}
		if (!net.vulkanic.world.RustGalWorldPrimitiveRenderer.enqueueGuardianBeam(
			poseStack.last().pose(), textureIdentity, vertices, uvs, colors, lightCoords
		)) {
			throw new IllegalStateException("Rust whole-frame Guardian beam route selected without semantic material quads");
		}
		net.vulkanic.world.RustGalWorldPrimitiveRenderer.recordGuardianBeamRouteDecision("rust-vulkan-whole-frame", true, true, false);
		return true;
	}

	@Override
	public boolean submitCrystalBeam(
		PoseStack poseStack, RenderType renderType, net.minecraft.resources.ResourceLocation textureIdentity,
		float[] vertices, float[] uvs, int[] colors, int lightCoords
	) {
		if (!rustWholeFramePresenterActive()
			|| !net.vulkanic.world.WorldRenderRoutePolicy.currentCrystalBeamRoute().usesRustWholeFrameVulkan()) {
			return false;
		}
		// EnderDragonRenderer makes the route admission decision before invoking
		// this semantic collector. The explicit gate above also prevents an
		// OpenGL submission from being diverted into the Rust queue during the
		// backend handoff.
		if (!net.vulkanic.world.RustGalWorldPrimitiveRenderer.enqueueCrystalBeam(
			poseStack.last().pose(), textureIdentity, vertices, uvs, colors, lightCoords
		)) {
			throw new IllegalStateException("Rust whole-frame crystal beam route selected without semantic quads");
		}
		net.minecraft.client.dev.GraphicsFrameBenchmark.recordSubmittedWorkIdentity(
			"crystal-beam", "rust-vulkan-whole-frame:translucent");
		net.minecraft.client.dev.DeterministicCameraCapture.recordSubmittedWorkIdentity(
			"crystal-beam", "rust-vulkan-whole-frame:translucent");
		return true;
	}

	@Override
	public boolean submitTexturedQuad(
		PoseStack poseStack, RenderType renderType, net.minecraft.resources.ResourceLocation textureIdentity,
		float[] vertices, float[] uvs, int color, int lightCoords
	) {
		if (net.vulkanic.world.RustGalWorldPrimitiveRenderer.isFirstPersonGuiCaptureActive()) {
			return net.vulkanic.world.RustGalWorldPrimitiveRenderer.enqueueFirstPersonGuiTexturedQuad(
				poseStack.last().pose(), textureIdentity, vertices, uvs, color == -1 ? 0xffffffff : color
			);
		}
		if (!rustWholeFramePresenterActive()
			|| !net.vulkanic.world.WorldRenderRoutePolicy.currentTexturedBillboardRoute().usesRustWholeFrameVulkan()) {
			if (rustWholeFramePresenterActive()) {
				throw new IllegalStateException("Rust Vulkan textured-billboard route is unavailable; Java geometry is not a fallback");
			}
			return false;
		}
		if (!net.vulkanic.world.RustGalWorldPrimitiveRenderer.enqueueTexturedQuad(
			poseStack.last().pose(), textureIdentity, vertices, uvs, color, lightCoords
		)) {
			throw new IllegalStateException("Rust whole-frame textured billboard route selected without semantic material quad");
		}
		return true;
	}

	@Override
	public boolean submitTranslucentTexturedQuad(
		PoseStack poseStack, RenderType renderType, net.minecraft.resources.ResourceLocation textureIdentity,
		float[] vertices, float[] uvs, int color, int lightCoords
	) {
		if ((rustWholeFramePresenterActive() || net.vulkanic.bridge.RustGalVulkanWholeFrameMode.enabled())
			&& (net.vulkanic.world.WorldRenderRoutePolicy.currentTexturedBillboardRoute().usesRustWholeFrameVulkan()
				|| net.vulkanic.bridge.RustGalVulkanWholeFrameMode.enabled())) {
			if (!net.vulkanic.world.RustGalWorldPrimitiveRenderer.enqueueTranslucentTexturedQuad(
				poseStack.last().pose(), textureIdentity, vertices, uvs, color, lightCoords
			)) throw new IllegalStateException("Rust whole-frame translucent billboard route rejected semantic quad");
			return true;
		}
		if (rustWholeFramePresenterActive()) {
			throw new IllegalStateException("Rust Vulkan translucent-billboard route is unavailable; Java geometry is not a fallback");
		}
		return false;
	}

	@Override
	public boolean submitTexturedQuads(
		PoseStack poseStack, RenderType renderType, net.minecraft.resources.ResourceLocation textureIdentity,
		float[] vertices, float[] uvs, int[] colors, int lightCoords
	) {
		if (net.vulkanic.world.RustGalWorldPrimitiveRenderer.isFirstPersonGuiCaptureActive()) {
			if (!net.vulkanic.world.RustGalWorldPrimitiveRenderer.enqueueFirstPersonTexturedQuads(
				textureIdentity, vertices, uvs, colors, lightCoords
			)) {
				throw new IllegalStateException("Rust first-person semantic quad route rejected copied geometry");
			}
			return true;
		}
		if (!rustWholeFramePresenterActive()
			|| !net.vulkanic.world.WorldRenderRoutePolicy.currentTexturedBillboardRoute().usesRustWholeFrameVulkan()) {
			if (rustWholeFramePresenterActive()) {
				throw new IllegalStateException("Rust Vulkan textured-quad route is unavailable; Java geometry is not a fallback");
			}
			return false;
		}
		if (!net.vulkanic.world.RustGalWorldPrimitiveRenderer.enqueueTexturedQuads(
			poseStack.last().pose(), textureIdentity, vertices, uvs, colors, lightCoords
		)) throw new IllegalStateException("Rust whole-frame textured-quad batch rejected semantic mesh");
		return true;
	}

	@Override
	public boolean submitOpticalTexturedQuads(
		PoseStack poseStack, RenderType renderType, net.minecraft.resources.ResourceLocation textureIdentity,
		float[] vertices, float[] uvs, int[] colors, int lightCoords, int materialMode
	) {
		if (!net.vulkanic.world.RustGalWorldPrimitiveRenderer.isFirstPersonGuiCaptureActive()) {
			return false;
		}
		if (!net.vulkanic.world.RustGalWorldPrimitiveRenderer.enqueueFirstPersonOpticalTexturedQuads(
			textureIdentity, vertices, uvs, colors, lightCoords, materialMode
		)) {
			throw new IllegalStateException("Rust first-person optical route rejected copied stencil geometry");
		}
		return true;
	}

	@Override
	public boolean submitLineSegments(PoseStack poseStack, float[] endpoints, int color, float lineWidth) {
		if (!rustWholeFramePresenterActive()
			|| !net.vulkanic.world.WorldRenderRoutePolicy.currentFishingLineRoute().usesRustWholeFrameVulkan()) {
			if (rustWholeFramePresenterActive()) {
				throw new IllegalStateException("Rust Vulkan line route is unavailable; Java geometry is not a fallback");
			}
			return false;
		}
		if (!net.vulkanic.world.RustGalWorldPrimitiveRenderer.enqueueLineSegments(poseStack.last().pose(), endpoints, color, lineWidth)) {
			throw new IllegalStateException("Rust whole-frame line route selected without semantic segments");
		}
		return true;
	}

	@Override
	public boolean submitColoredQuads(PoseStack poseStack, RenderType renderType, float[] vertices, float[] uvs, int[] colors, int lightCoords) {
		boolean textBackground = renderType == RenderType.textBackground()
			|| renderType == RenderType.textBackgroundSeeThrough();
		if (textBackground) {
			if (!rustWholeFramePresenterActive()
				|| !net.vulkanic.world.WorldRenderRoutePolicy.currentProceduralQuadRoute().usesRustWholeFrameVulkan()) {
				if (rustWholeFramePresenterActive()) {
					throw new IllegalStateException("Rust Vulkan text-background route is unavailable; Java geometry is not a fallback");
				}
				return false;
			}
			if (vertices == null || vertices.length != 12 || uvs == null || uvs.length != 8
				|| colors == null || colors.length != 1) {
				throw new IllegalArgumentException("Rust text-background route requires exactly one semantic quad");
			}
			if (!net.vulkanic.world.RustGalWorldPrimitiveRenderer.enqueueTextBackgroundQuad(
				poseStack.last().pose(), vertices, uvs, colors[0], lightCoords,
				renderType == RenderType.textBackgroundSeeThrough())) {
				throw new IllegalStateException("Rust whole-frame text-background route rejected semantic quad");
			}
			return true;
		}
		if (!rustWholeFramePresenterActive()
			|| !net.vulkanic.world.WorldRenderRoutePolicy.currentProceduralQuadRoute().usesRustWholeFrameVulkan()) {
			if (rustWholeFramePresenterActive()) {
				throw new IllegalStateException("Rust Vulkan procedural-quad route is unavailable; Java geometry is not a fallback");
			}
			return false;
		}
		if (!net.vulkanic.world.RustGalWorldPrimitiveRenderer.enqueueProceduralQuads(
			poseStack.last().pose(), vertices, uvs, colors, lightCoords
		)) throw new IllegalStateException("Rust procedural-quad route selected without semantic quads");
		return true;
	}

	@Override
	public void submitParticleGroup(SubmitNodeCollector.ParticleGroupRenderer particleGroupRenderer) {
		this.wasUsed = true;
		net.vulkanic.world.WorldRenderRoutePolicy.Route particleRoute =
			net.vulkanic.world.WorldRenderRoutePolicy.currentMaterialRoute();
		// The admission predicate is currentMaterialRoute().usesRustWholeFrameVulkan().
		if (net.vulkanic.VulkanicAPI.isVulkanBackendSelected() && !particleRoute.usesRustWholeFrameVulkan()) {
			throw new IllegalStateException("Rust Vulkan particle route is unavailable; Java particle callbacks are not a fallback");
		}
		if (particleRoute.usesRustWholeFrameVulkan()) {
			// The Rust route has already copied QuadParticleRenderState layers into
			// semantic material quads. Retaining the Java callback would keep a
			// renderer-owned closure alive across the Rust frame boundary, even though
			// ParticleFeatureRenderer must discard it and never execute a Java pass.
			if (!(particleGroupRenderer instanceof QuadParticleRenderState quad)
				|| quad.rustGalUnsupportedLayerCount() > 0) {
				net.vulkanic.world.RustGalWorldPrimitiveRenderer.recordUnsupportedParticleGroup();
			}
			return;
		}
		if (rustWholeFramePresenterActive()) {
			net.vulkanic.world.RustGalWorldPrimitiveRenderer.recordUnsupportedParticleGroup();
			throw new IllegalStateException("Rust Vulkan particle route is unavailable; Java particle callbacks are not a fallback");
		}
		this.particleGroupRenderers.add(particleGroupRenderer);
	}

	@Override
	public void submitParticleGroupSemantic(SubmitNodeCollector.ParticleGroupRenderer particleGroupRenderer) {
		this.wasUsed = true;
		if (net.vulkanic.world.WorldRenderRoutePolicy.currentMaterialRoute().usesRustWholeFrameVulkan()) {
			// Quad particles are copied by ParticleEngine's explicit Rust enqueue
			// pass before feature dispatch. The semantic collector records the
			// already-owned producer without retaining a Java callback; unknown
			// groups remain an explicit unsupported family.
			if (!(particleGroupRenderer instanceof QuadParticleRenderState)
				|| ((QuadParticleRenderState) particleGroupRenderer).rustGalUnsupportedLayerCount() > 0) {
				net.vulkanic.world.RustGalWorldPrimitiveRenderer.recordUnsupportedParticleGroup();
			}
			return;
		}
		throw new IllegalStateException("semantic particle submission is only valid for the Rust whole-frame route");
	}

	public List<SubmitNodeStorage.ShadowSubmit> getShadowSubmits() {
		return this.shadowSubmits;
	}

	public List<SubmitNodeStorage.FlameSubmit> getFlameSubmits() {
		return this.flameSubmits;
	}

	public NameTagFeatureRenderer.Storage getNameTagSubmits() {
		return this.nameTagSubmits;
	}

	public List<SubmitNodeStorage.TextSubmit> getTextSubmits() {
		return this.textSubmits;
	}

	public List<SubmitNodeStorage.HitboxSubmit> getHitboxSubmits() {
		return this.hitboxSubmits;
	}

	public List<SubmitNodeStorage.LeashSubmit> getLeashSubmits() {
		return this.leashSubmits;
	}

	public List<SubmitNodeStorage.BlockSubmit> getBlockSubmits() {
		return this.blockSubmits;
	}

	public List<SubmitNodeStorage.MovingBlockSubmit> getMovingBlockSubmits() {
		return this.movingBlockSubmits;
	}

	public List<SubmitNodeStorage.BlockModelSubmit> getBlockModelSubmits() {
		return this.blockModelSubmits;
	}

	public ModelPartFeatureRenderer.Storage getModelPartSubmits() {
		return this.modelPartSubmits;
	}

	public List<SubmitNodeStorage.ItemSubmit> getItemSubmits() {
		return this.itemSubmits;
	}

	public List<SubmitNodeCollector.ParticleGroupRenderer> getParticleGroupRenderers() {
		return this.particleGroupRenderers;
	}

	public ModelFeatureRenderer.Storage getModelSubmits() {
		return this.modelSubmits;
	}

	public CustomFeatureRenderer.Storage getCustomGeometrySubmits() {
		return this.customGeometrySubmits;
	}

	/**
	 * A copied inventory of feature work collected during this frame. The Rust
	 * whole-frame route uses this only to make unported producer families
	 * explicit before presentation; no renderer object crosses this boundary.
	 */
	public WorldFeatureCoverageSnapshot worldFeatureCoverageSnapshot() {
		int ordinaryBlockSubmits = 0;
		for (SubmitNodeStorage.BlockSubmit submit : this.blockSubmits) {
			if (submit.source() == SubmitNodeStorage.BlockSubmitSource.ORDINARY) {
				ordinaryBlockSubmits++;
			}
		}
		int unsupportedParticleGroupSubmits = 0;
		for (SubmitNodeCollector.ParticleGroupRenderer renderer : this.particleGroupRenderers) {
			// QuadParticleRenderState copies its particle semantics into the Rust
			// material stream before frame admission. Layers outside the owned
			// particle atlas (or custom group callbacks) remain unavailable.
			if (renderer instanceof QuadParticleRenderState quad) {
				unsupportedParticleGroupSubmits += quad.rustGalUnsupportedLayerCount();
			} else {
				unsupportedParticleGroupSubmits++;
			}
		}
		return new WorldFeatureCoverageSnapshot(
			this.modelSubmits.totalSubmitCount(),
			this.modelPartSubmits.totalSubmitCount(),
			this.blockModelSubmits.size(),
			ordinaryBlockSubmits,
			this.itemSubmits.size() + this.meshItemCommands.size() + this.rustFabricItemSubmits,
			this.customGeometrySubmits.totalSubmitCount(),
			this.shadowSubmits.size(),
			this.flameSubmits.size(),
			this.nameTagSubmits.totalSubmitCount(),
			this.textSubmits.size(),
			this.hitboxSubmits.size(),
			this.leashSubmits.size(),
			unsupportedParticleGroupSubmits
		);
	}

	public boolean wasUsed() {
		return this.wasUsed;
	}

	public void clear() {
		this.shadowSubmits.clear();
		this.flameSubmits.clear();
		this.nameTagSubmits.clear();
		this.textSubmits.clear();
		this.hitboxSubmits.clear();
		this.leashSubmits.clear();
		this.blockSubmits.clear();
		this.movingBlockSubmits.clear();
		this.blockModelSubmits.clear();
		this.itemSubmits.clear();
		this.particleGroupRenderers.clear();
		this.modelSubmits.clear();
		this.customGeometrySubmits.clear();
		this.modelPartSubmits.clear();
		// Sodium FRAPI: Clear mesh item commands
		this.meshItemCommands.clear();
		this.rustFabricItemSubmits = 0;
	}
	
	// Sodium FRAPI: OrderedSubmitNodeCollectorExtension implementation
	@Override
	public void fabric_submitItem(PoseStack matrices, ItemDisplayContext displayContext, int light, int overlay, 
		int outlineColors, int[] tintLayers, List<BakedQuad> quads, RenderType renderLayer,
		ItemStackRenderState.FoilType foilType, MeshView mesh) {
		this.wasUsed = true;
		if (net.vulkanic.VulkanicAPI.isVulkanBackendSelected()
			|| net.vulkanic.bridge.RustGalVulkanWholeFrameMode.enabled()) {
			if (!net.vulkanic.world.RustGalWorldPrimitiveRenderer.enqueueFabricMeshItem(
				matrices.last(), displayContext, light, overlay, outlineColors, tintLayers,
				quads, renderLayer, foilType, mesh
			)) {
				throw new IllegalStateException(
					"Rust whole-frame Fabric item route rejected semantic MeshView quads"
				);
			}
			this.rustFabricItemSubmits++;
			return;
		}
		this.meshItemCommands.add(new MeshItemCommand(matrices.last().copy(), displayContext, light, overlay, 
			outlineColors, tintLayers, quads, renderLayer, foilType, mesh));
	}
	
	// Sodium FRAPI: SubmitNodeCollectionExtension implementation
	@Override
	public List<MeshItemCommand> sodium_getMeshItemCommands() {
		return this.meshItemCommands;
	}

	public void endFrame() {
		this.modelSubmits.endFrame();
		this.modelPartSubmits.endFrame();
		this.customGeometrySubmits.endFrame();
		this.wasUsed = false;
	}

	@Environment(EnvType.CLIENT)
	public record WorldFeatureCoverageSnapshot(
		int modelSubmits,
		int modelPartSubmits,
		int blockModelSubmits,
		int ordinaryBlockSubmits,
		int itemSubmits,
		int customGeometrySubmits,
		int shadowSubmits,
		int flameSubmits,
		int nameTagSubmits,
		int textSubmits,
		int hitboxSubmits,
		int leashSubmits,
		int particleGroupSubmits
	) {
		public WorldFeatureCoverageSnapshot plus(WorldFeatureCoverageSnapshot other) {
			if (other == null) {
				return this;
			}
			return new WorldFeatureCoverageSnapshot(
				this.modelSubmits + other.modelSubmits,
				this.modelPartSubmits + other.modelPartSubmits,
				this.blockModelSubmits + other.blockModelSubmits,
				this.ordinaryBlockSubmits + other.ordinaryBlockSubmits,
				this.itemSubmits + other.itemSubmits,
				this.customGeometrySubmits + other.customGeometrySubmits,
				this.shadowSubmits + other.shadowSubmits,
				this.flameSubmits + other.flameSubmits,
				this.nameTagSubmits + other.nameTagSubmits,
				this.textSubmits + other.textSubmits,
				this.hitboxSubmits + other.hitboxSubmits,
				this.leashSubmits + other.leashSubmits,
				this.particleGroupSubmits + other.particleGroupSubmits
			);
		}

		public boolean hasUnsupportedRustWholeFrameWork() {
			return modelSubmits != 0
				|| modelPartSubmits != 0
				|| blockModelSubmits != 0
				|| ordinaryBlockSubmits != 0
				|| itemSubmits != 0
				|| customGeometrySubmits != 0
				|| shadowSubmits != 0
				|| flameSubmits != 0
				|| nameTagSubmits != 0
				|| textSubmits != 0
				|| hitboxSubmits != 0
				|| leashSubmits != 0
				|| particleGroupSubmits != 0;
		}
	}
}
