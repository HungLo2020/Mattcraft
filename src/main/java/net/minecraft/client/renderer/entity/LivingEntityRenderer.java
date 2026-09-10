package net.minecraft.client.renderer.entity;

import com.google.common.collect.Lists;
import com.google.common.collect.ImmutableList.Builder;
import net.blaze3d.vertex.PoseStack;
import net.math.Axis;
import java.util.List;
import net.minecraft.api.EnvType;
import net.minecraft.api.Environment;
import net.minecraft.client.Minecraft;
import net.minecraft.client.model.EntityModel;
import net.minecraft.client.model.DrownedModel;
import net.minecraft.client.model.BatModel;
import net.minecraft.client.model.BeeModel;
import net.minecraft.client.model.AxolotlModel;
import net.minecraft.client.model.FrogModel;
import net.minecraft.client.model.SquidModel;
import net.minecraft.client.model.GuardianModel;
import net.minecraft.client.model.SpiderModel;
import net.minecraft.client.model.SnowGolemModel;
import net.minecraft.client.model.IronGolemModel;
import net.minecraft.client.model.RavagerModel;
import net.minecraft.client.model.VexModel;
import net.minecraft.client.model.AllayModel;
import net.minecraft.client.model.WitchModel;
import net.minecraft.client.model.FoxModel;
import net.minecraft.client.model.IllagerModel;
import net.minecraft.client.model.CodModel;
import net.minecraft.client.model.PufferfishBigModel;
import net.minecraft.client.model.PufferfishMidModel;
import net.minecraft.client.model.PufferfishSmallModel;
import net.minecraft.client.model.OcelotModel;
import net.minecraft.client.model.CatModel;
import net.minecraft.client.model.WolfModel;
import net.minecraft.client.model.ParrotModel;
import net.minecraft.client.model.GhastModel;
import net.minecraft.client.model.BlazeModel;
import net.minecraft.client.model.LavaSlimeModel;
import net.minecraft.client.model.HorseModel;
import net.minecraft.client.model.DonkeyModel;
import net.minecraft.client.model.LlamaModel;
import net.minecraft.client.model.StriderModel;
import net.minecraft.client.model.HoglinModel;
import net.minecraft.client.model.CamelModel;
import net.minecraft.client.model.PiglinModel;
import net.minecraft.client.model.ZombifiedPiglinModel;
import net.minecraft.client.model.SkeletonModel;
import net.minecraft.client.model.BoggedModel;
import net.minecraft.client.model.GiantZombieModel;
import net.minecraft.client.model.ArmadilloModel;
import net.minecraft.client.model.SnifferModel;
import net.minecraft.client.model.GoatModel;
import net.minecraft.client.model.TropicalFishModelA;
import net.minecraft.client.model.TropicalFishModelB;
import net.minecraft.client.model.PolarBearModel;
import net.minecraft.client.model.DolphinModel;
import net.minecraft.client.model.TurtleModel;
import net.minecraft.client.model.PandaModel;
import net.minecraft.client.model.ChickenModel;
import net.minecraft.client.model.CowModel;
import net.minecraft.client.model.ModelLobster;
import net.minecraft.client.model.CreeperModel;
import net.minecraft.client.model.EndermiteModel;
import net.minecraft.client.model.PigModel;
import net.minecraft.client.model.RabbitModel;
import net.minecraft.client.model.SheepModel;
import net.minecraft.client.model.SlimeModel;
import net.minecraft.client.model.SilverfishModel;
import net.minecraft.client.model.SalmonModel;
import net.minecraft.client.model.TadpoleModel;
import net.minecraft.client.model.ZombieModel;
import net.minecraft.client.player.LocalPlayer;
import net.minecraft.client.renderer.RenderType;
import net.minecraft.client.renderer.SubmitNodeCollector;
import net.minecraft.client.renderer.entity.layers.HumanoidArmorLayer;
import net.minecraft.client.renderer.entity.layers.RenderLayer;
import net.minecraft.client.renderer.entity.state.HitboxRenderState;
import net.minecraft.client.renderer.entity.state.BatRenderState;
import net.minecraft.client.renderer.entity.state.BeeRenderState;
import net.minecraft.client.renderer.entity.state.AxolotlRenderState;
import net.minecraft.client.renderer.entity.state.FrogRenderState;
import net.minecraft.client.renderer.entity.state.SquidRenderState;
import net.minecraft.client.renderer.entity.state.GuardianRenderState;
import net.minecraft.client.renderer.entity.state.SnowGolemRenderState;
import net.minecraft.client.renderer.entity.state.IronGolemRenderState;
import net.minecraft.client.renderer.entity.state.RavagerRenderState;
import net.minecraft.client.renderer.entity.state.VexRenderState;
import net.minecraft.client.renderer.entity.state.AllayRenderState;
import net.minecraft.client.renderer.entity.state.WitchRenderState;
import net.minecraft.client.renderer.entity.state.FoxRenderState;
import net.minecraft.client.renderer.entity.state.EvokerRenderState;
import net.minecraft.client.renderer.entity.state.IllagerRenderState;
import net.minecraft.client.renderer.entity.state.SalmonRenderState;
import net.minecraft.client.renderer.entity.state.PufferfishRenderState;
import net.minecraft.client.renderer.entity.state.FelineRenderState;
import net.minecraft.client.renderer.entity.state.CatRenderState;
import net.minecraft.client.renderer.entity.state.WolfRenderState;
import net.minecraft.client.renderer.entity.state.ParrotRenderState;
import net.minecraft.client.renderer.entity.state.GhastRenderState;
import net.minecraft.client.renderer.entity.state.HorseRenderState;
import net.minecraft.client.renderer.entity.state.DonkeyRenderState;
import net.minecraft.client.renderer.entity.state.LlamaRenderState;
import net.minecraft.client.renderer.entity.state.StriderRenderState;
import net.minecraft.client.renderer.entity.state.HoglinRenderState;
import net.minecraft.client.renderer.entity.state.CamelRenderState;
import net.minecraft.client.renderer.entity.state.HumanoidRenderState;
import net.minecraft.client.renderer.entity.state.SkeletonRenderState;
import net.minecraft.client.renderer.entity.state.BoggedRenderState;
import net.minecraft.client.renderer.entity.state.ArmadilloRenderState;
import net.minecraft.client.renderer.entity.state.SnifferRenderState;
import net.minecraft.client.renderer.entity.state.NautilusRenderState;
import net.minecraft.client.renderer.entity.state.PhantomRenderState;
import net.minecraft.client.renderer.entity.state.WardenRenderState;
import net.minecraft.client.renderer.entity.state.CreakingRenderState;
import net.minecraft.client.renderer.entity.state.BreezeRenderState;
import net.minecraft.client.renderer.entity.state.EndermanRenderState;
import net.minecraft.client.renderer.entity.state.CopperGolemRenderState;
import net.minecraft.client.renderer.entity.state.WitherRenderState;
import net.minecraft.client.renderer.entity.state.GoatRenderState;
import net.minecraft.client.renderer.entity.state.TropicalFishRenderState;
import net.minecraft.client.renderer.entity.state.PolarBearRenderState;
import net.minecraft.client.renderer.entity.state.DolphinRenderState;
import net.minecraft.client.renderer.entity.state.TurtleRenderState;
import net.minecraft.client.renderer.entity.state.PandaRenderState;
import net.minecraft.client.renderer.entity.state.ChickenRenderState;
import net.minecraft.client.renderer.entity.state.CowRenderState;
import net.minecraft.client.renderer.entity.state.MushroomCowRenderState;
import net.minecraft.client.renderer.entity.state.LobsterRenderState;
import net.minecraft.client.renderer.entity.state.CreeperRenderState;
import net.minecraft.client.renderer.entity.state.PigRenderState;
import net.minecraft.client.renderer.entity.state.RabbitRenderState;
import net.minecraft.client.renderer.entity.state.SheepRenderState;
import net.minecraft.client.renderer.entity.state.SlimeRenderState;
import net.minecraft.client.renderer.entity.state.ZombieRenderState;
import net.minecraft.client.renderer.entity.state.LivingEntityRenderState;
import net.minecraft.client.renderer.item.ItemModelResolver;
import net.minecraft.client.renderer.state.CameraRenderState;
import net.minecraft.client.renderer.texture.OverlayTexture;
import net.minecraft.core.Direction;
import net.minecraft.core.component.DataComponents;
import net.minecraft.network.chat.Component;
import net.minecraft.resources.ResourceLocation;
import net.minecraft.util.ARGB;
import net.minecraft.util.Mth;
import net.minecraft.world.entity.EquipmentSlot;
import net.minecraft.world.entity.LivingEntity;
import net.minecraft.world.entity.Pose;
import net.minecraft.world.item.BlockItem;
import net.minecraft.world.item.ItemDisplayContext;
import net.minecraft.world.item.ItemStack;
import net.minecraft.world.item.Items;
import net.minecraft.world.item.component.ResolvableProfile;
import net.minecraft.world.level.block.AbstractSkullBlock;
import net.minecraft.world.phys.AABB;
import net.minecraft.world.scores.Team;
import net.minecraft.world.scores.Team.Visibility;
import org.jetbrains.annotations.Nullable;

@Environment(EnvType.CLIENT)
public abstract class LivingEntityRenderer<T extends LivingEntity, S extends LivingEntityRenderState, M extends EntityModel<? super S>>
	extends EntityRenderer<T, S>
	implements RenderLayerParent<S, M> {
	private static final float EYE_BED_OFFSET = 0.1F;
	protected M model;
	protected final ItemModelResolver itemModelResolver;
	// VoxelMap: Made accessible
	public final List<RenderLayer<S, M>> layers = Lists.<RenderLayer<S, M>>newArrayList();

	public LivingEntityRenderer(EntityRendererProvider.Context context, M entityModel, float f) {
		super(context);
		this.itemModelResolver = context.getItemModelResolver();
		this.model = entityModel;
		this.shadowRadius = f;
	}

	protected final boolean addLayer(RenderLayer<S, M> renderLayer) {
		return this.layers.add(renderLayer);
	}

	@Override
	public M getModel() {
		return this.model;
	}

	/** Applies the copied semantic model pose used by Rust GUI entity PIPs. */
	public void applySemanticModelPose(S state, PoseStack poseStack) {
		float scale = state.scale;
		poseStack.scale(scale, scale, scale);
		this.setupRotations(state, poseStack, state.bodyRot, scale);
		poseStack.scale(-1.0F, -1.0F, 1.0F);
		this.scale(state, poseStack);
		poseStack.translate(0.0F, -1.501F, 0.0F);
		this.model.setupAnim(state);
	}

	public AABB getBoundingBoxForCulling(T livingEntity) {
		AABB aABB = super.getBoundingBoxForCulling(livingEntity);
		if (livingEntity.getItemBySlot(EquipmentSlot.HEAD).is(Items.DRAGON_HEAD)) {
			float f = 0.5F;
			return aABB.inflate(0.5, 0.5, 0.5);
		} else {
			return aABB;
		}
	}

	public void submit(S livingEntityRenderState, PoseStack poseStack, SubmitNodeCollector submitNodeCollector, CameraRenderState cameraRenderState) {
		poseStack.pushPose();
		if (livingEntityRenderState.hasPose(Pose.SLEEPING)) {
			Direction direction = livingEntityRenderState.bedOrientation;
			if (direction != null) {
				float f = livingEntityRenderState.eyeHeight - 0.1F;
				poseStack.translate(-direction.getStepX() * f, 0.0F, -direction.getStepZ() * f);
			}
		}

		float g = livingEntityRenderState.scale;
		poseStack.scale(g, g, g);
		this.setupRotations(livingEntityRenderState, poseStack, livingEntityRenderState.bodyRot, g);
		poseStack.scale(-1.0F, -1.0F, 1.0F);
		this.scale(livingEntityRenderState, poseStack);
		poseStack.translate(0.0F, -1.501F, 0.0F);
		boolean bl = this.isBodyVisible(livingEntityRenderState);
		boolean bl2 = !bl && !livingEntityRenderState.isInvisibleToPlayer;
		RenderType renderType = this.getRenderType(livingEntityRenderState, bl, bl2, livingEntityRenderState.appearsGlowing());
		if (renderType != null) {
			int i = getOverlayCoords(livingEntityRenderState, this.getWhiteOverlayProgress(livingEntityRenderState));
		int j = bl2 ? 654311423 : -1;
		int k = ARGB.multiply(j, this.getModelTint(livingEntityRenderState));
		ResourceLocation textureIdentity = this.getTextureLocation(livingEntityRenderState);
		boolean rustOutlineOnlyLivingBody = !bl && !bl2 && livingEntityRenderState.appearsGlowing();
		// Rust's outline post-effect consumes outlineColor from the copied mesh
		// instance. Keep the selected Vulkan route on the model's ordinary
		// semantic material instead of passing Java's outline-only RenderType,
		// which has no texture/material contract at the Rust boundary.
		if ((net.vulkanic.VulkanicAPI.isVulkanBackendSelected()
				|| net.vulkanic.bridge.RustGalVulkanWholeFrameMode.enabled())
			&& renderType.isOutline() && textureIdentity != null) {
			RenderType semanticOutlineMaterial = this.model.renderType(textureIdentity);
			if (semanticOutlineMaterial != null && !semanticOutlineMaterial.isOutline()) {
				renderType = semanticOutlineMaterial;
			}
		}
		ResourceLocation entityIdentity = net.vulkanic.world.RustGalWorldPrimitiveRenderer.entityIdentity(livingEntityRenderState);
			boolean rustLivingModelFamily = (this.model.getClass() == ChickenModel.class || this.model instanceof ChickenModel)
				&& livingEntityRenderState instanceof ChickenRenderState
				|| this.model != null
					&& this.model.getClass() == net.minecraft.client.model.ArmorStandModel.class
					&& livingEntityRenderState instanceof net.minecraft.client.renderer.entity.state.ArmorStandRenderState
				|| this.model != null
					&& this.model.getClass() == net.minecraft.client.model.VillagerModel.class
					&& livingEntityRenderState instanceof net.minecraft.client.renderer.entity.state.VillagerRenderState
				|| this.model != null
					&& this.model.getClass() == net.minecraft.client.model.ZombieVillagerModel.class
					&& livingEntityRenderState instanceof net.minecraft.client.renderer.entity.state.ZombieVillagerRenderState
				|| this.model instanceof CowModel
					&& livingEntityRenderState instanceof CowRenderState
				|| this.model instanceof CowModel
					&& livingEntityRenderState instanceof MushroomCowRenderState
				|| this.model.getClass() == ModelLobster.class
					&& livingEntityRenderState instanceof LobsterRenderState
				|| (this.model instanceof net.alexsmobs.client.model.ModelBison
					|| this.model instanceof net.alexsmobs.client.model.ModelBisonBaby)
					&& livingEntityRenderState instanceof net.alexsmobs.client.render.BisonRenderState
				|| this.model != null
					&& this.model instanceof PigModel
					&& livingEntityRenderState instanceof PigRenderState
				|| this.model != null
					&& this.model.getClass() == ZombieModel.class
					&& livingEntityRenderState instanceof ZombieRenderState
				|| this.model != null
					&& this.model.getClass() == RabbitModel.class
					&& livingEntityRenderState instanceof RabbitRenderState
				|| this.model != null
					&& this.model.getClass() == SheepModel.class
					&& livingEntityRenderState instanceof SheepRenderState
				|| this.model != null
					&& this.model.getClass() == CreeperModel.class
					&& livingEntityRenderState instanceof CreeperRenderState
				|| this.model != null
					&& this.model.getClass() == SlimeModel.class
					&& livingEntityRenderState instanceof SlimeRenderState
				|| this.model != null
					&& this.model.getClass() == LavaSlimeModel.class
					&& livingEntityRenderState instanceof SlimeRenderState
				|| this.model != null
					&& this.model.getClass() == HorseModel.class
					&& livingEntityRenderState instanceof HorseRenderState
				|| this.model != null
					&& this.model.getClass() == DonkeyModel.class
					&& livingEntityRenderState instanceof DonkeyRenderState
				|| this.model != null
					&& this.model.getClass() == LlamaModel.class
					&& livingEntityRenderState instanceof LlamaRenderState
				|| this.model != null
					&& this.model.getClass() == StriderModel.class
					&& livingEntityRenderState instanceof StriderRenderState
				|| this.model != null
					&& this.model.getClass() == HoglinModel.class
					&& livingEntityRenderState instanceof HoglinRenderState
				|| this.model != null
					&& this.model.getClass() == CamelModel.class
					&& livingEntityRenderState instanceof CamelRenderState
				|| this.model != null
					&& (this.model.getClass() == PiglinModel.class || this.model.getClass() == ZombifiedPiglinModel.class)
					&& livingEntityRenderState instanceof HumanoidRenderState
				|| this.model != null
					&& this.model.getClass() == SkeletonModel.class
					&& livingEntityRenderState instanceof SkeletonRenderState
				|| this.model != null
					&& this.model.getClass() == BoggedModel.class
					&& livingEntityRenderState instanceof BoggedRenderState
				|| this.model != null
					&& this.model.getClass() == GiantZombieModel.class
					&& livingEntityRenderState instanceof ZombieRenderState
				|| this.model != null
					&& this.model.getClass() == ArmadilloModel.class
					&& livingEntityRenderState instanceof ArmadilloRenderState
				|| this.model != null
					&& this.model.getClass() == SnifferModel.class
					&& livingEntityRenderState instanceof SnifferRenderState
				|| this.model != null
					&& (this.model.getClass() == net.minecraft.client.model.animal.nautilus.NautilusModel.class
						|| this.model instanceof net.minecraft.client.model.animal.nautilus.NautilusModel)
					&& livingEntityRenderState instanceof NautilusRenderState
				|| this.model != null
					&& this.model.getClass() == net.minecraft.client.model.PhantomModel.class
					&& livingEntityRenderState instanceof PhantomRenderState
				|| this.model != null
					&& this.model.getClass() == net.minecraft.client.model.WardenModel.class
					&& livingEntityRenderState instanceof WardenRenderState
				|| this.model != null
					&& this.model.getClass() == net.minecraft.client.model.CreakingModel.class
					&& livingEntityRenderState instanceof CreakingRenderState
				|| this.model != null
					&& this.model.getClass() == net.minecraft.client.model.BreezeModel.class
					&& livingEntityRenderState instanceof BreezeRenderState
				|| this.model != null
					&& this.model.getClass() == net.minecraft.client.model.EndermanModel.class
					&& livingEntityRenderState instanceof EndermanRenderState
				|| this.model != null
					&& this.model.getClass() == net.minecraft.client.model.CopperGolemModel.class
					&& livingEntityRenderState instanceof CopperGolemRenderState
				|| this.model != null
					&& this.model.getClass() == net.minecraft.client.model.WitherBossModel.class
					&& livingEntityRenderState instanceof WitherRenderState
				|| this.model != null
					&& this.model.getClass() == DrownedModel.class
					&& livingEntityRenderState instanceof ZombieRenderState
				|| this.model != null
					&& this.model.getClass() == EndermiteModel.class
				|| this.model != null
					&& this.model.getClass() == SilverfishModel.class
				|| this.model != null
					&& this.model.getClass() == BatModel.class
					&& livingEntityRenderState instanceof BatRenderState
				|| this.model != null
					&& this.model.getClass() == CodModel.class
				|| this.model != null
					&& this.model.getClass() == SalmonModel.class
					&& livingEntityRenderState instanceof SalmonRenderState
				|| this.model != null
					&& (this.model.getClass() == PufferfishBigModel.class
						|| this.model.getClass() == PufferfishMidModel.class
						|| this.model.getClass() == PufferfishSmallModel.class)
					&& livingEntityRenderState instanceof PufferfishRenderState
				|| this.model != null
					&& this.model.getClass() == TadpoleModel.class
				|| this.model != null
					&& this.model.getClass() == OcelotModel.class
					&& livingEntityRenderState instanceof FelineRenderState
				|| this.model != null
					&& this.model.getClass() == CatModel.class
					&& livingEntityRenderState instanceof CatRenderState
				|| this.model != null
					&& this.model.getClass() == WolfModel.class
					&& livingEntityRenderState instanceof WolfRenderState
				|| this.model != null
					&& this.model.getClass() == ParrotModel.class
					&& livingEntityRenderState instanceof ParrotRenderState
				|| this.model != null
					&& this.model.getClass() == GhastModel.class
					&& livingEntityRenderState instanceof GhastRenderState
				|| this.model != null
					&& this.model.getClass() == net.minecraft.client.model.HappyGhastModel.class
					&& livingEntityRenderState instanceof net.minecraft.client.renderer.entity.state.HappyGhastRenderState
				|| this.model != null
					&& this.model.getClass() == BlazeModel.class
					&& livingEntityRenderState instanceof LivingEntityRenderState
				|| this.model != null
					&& this.model.getClass() == GoatModel.class
					&& livingEntityRenderState instanceof GoatRenderState
				|| this.model != null
					&& (this.model.getClass() == TropicalFishModelA.class || this.model.getClass() == TropicalFishModelB.class)
					&& livingEntityRenderState instanceof TropicalFishRenderState
				|| this.model != null
					&& this.model.getClass() == PolarBearModel.class
					&& livingEntityRenderState instanceof PolarBearRenderState
				|| this.model != null
					&& this.model.getClass() == DolphinModel.class
					&& livingEntityRenderState instanceof DolphinRenderState
				|| this.model != null
					&& this.model.getClass() == TurtleModel.class
					&& livingEntityRenderState instanceof TurtleRenderState
				|| this.model != null
					&& this.model.getClass() == PandaModel.class
					&& livingEntityRenderState instanceof PandaRenderState
				|| this.model != null
					&& this.model.getClass() == BeeModel.class
					&& livingEntityRenderState instanceof BeeRenderState
				|| this.model != null
					&& this.model.getClass() == AxolotlModel.class
					&& livingEntityRenderState instanceof AxolotlRenderState
				|| this.model != null
					&& this.model.getClass() == FrogModel.class
					&& livingEntityRenderState instanceof FrogRenderState
				|| this.model != null
					&& this.model.getClass() == SquidModel.class
					&& livingEntityRenderState instanceof SquidRenderState
				|| this.model != null
					&& this.model.getClass() == GuardianModel.class
					&& livingEntityRenderState instanceof GuardianRenderState
				|| this.model != null
					&& this.model.getClass() == SpiderModel.class
					&& livingEntityRenderState instanceof LivingEntityRenderState
				|| this.model != null
					&& this.model.getClass() == SnowGolemModel.class
					&& livingEntityRenderState instanceof SnowGolemRenderState
				|| this.model != null
					&& this.model.getClass() == IronGolemModel.class
					&& livingEntityRenderState instanceof IronGolemRenderState
				|| this.model != null
					&& this.model.getClass() == RavagerModel.class
					&& livingEntityRenderState instanceof RavagerRenderState
				|| this.model != null
					&& this.model.getClass() == VexModel.class
					&& livingEntityRenderState instanceof VexRenderState
				|| this.model != null
					&& this.model.getClass() == AllayModel.class
					&& livingEntityRenderState instanceof AllayRenderState
				|| this.model != null
					&& this.model.getClass() == WitchModel.class
					&& livingEntityRenderState instanceof WitchRenderState
				|| this.model != null
					&& this.model.getClass() == FoxModel.class
					&& livingEntityRenderState instanceof FoxRenderState
				|| this.model instanceof IllagerModel
					&& livingEntityRenderState instanceof EvokerRenderState
				|| this.model instanceof IllagerModel
					&& livingEntityRenderState instanceof IllagerRenderState;
			boolean rustLivingModelEligible = entityIdentity != null && (this.model.getClass() == net.minecraft.client.model.ArmorStandModel.class
				&& livingEntityRenderState instanceof net.minecraft.client.renderer.entity.state.ArmorStandRenderState armorStandRenderState
				&& net.vulkanic.world.RustGalWorldPrimitiveRenderer.isVanillaArmorStandModelMeshEligible(
					this.model,
					armorStandRenderState,
					renderType,
					textureIdentity,
					i,
					livingEntityRenderState.outlineColor,
					bl,
					bl2,
					livingEntityRenderState.appearsGlowing()
				)
				|| this.model.getClass() == net.minecraft.client.model.VillagerModel.class
				&& livingEntityRenderState instanceof net.minecraft.client.renderer.entity.state.VillagerRenderState villagerRenderState
				&& net.vulkanic.world.RustGalWorldPrimitiveRenderer.isVanillaVillagerModelMeshEligible(
					this.model, villagerRenderState, renderType, textureIdentity, i,
					livingEntityRenderState.outlineColor, bl, bl2, livingEntityRenderState.appearsGlowing()
				)
				|| this.model.getClass() == net.minecraft.client.model.ZombieVillagerModel.class
				&& livingEntityRenderState instanceof net.minecraft.client.renderer.entity.state.ZombieVillagerRenderState zombieVillagerRenderState
				&& net.vulkanic.world.RustGalWorldPrimitiveRenderer.isVanillaZombieVillagerModelMeshEligible(
					this.model, zombieVillagerRenderState, renderType, textureIdentity, i,
					livingEntityRenderState.outlineColor, bl, bl2, livingEntityRenderState.appearsGlowing()
				)
				|| this.model instanceof ChickenModel
				&& livingEntityRenderState instanceof ChickenRenderState chickenRenderState
				&& net.vulkanic.world.RustGalWorldPrimitiveRenderer.isVanillaChickenModelMeshEligible(
					this.model,
					chickenRenderState,
					renderType,
					textureIdentity,
					i,
					livingEntityRenderState.outlineColor,
					bl,
					bl2,
					livingEntityRenderState.appearsGlowing()
				)
				|| this.model instanceof CowModel
				&& livingEntityRenderState instanceof CowRenderState cowRenderState
				&& net.vulkanic.world.RustGalWorldPrimitiveRenderer.isVanillaCowModelMeshEligible(
					this.model,
					cowRenderState,
					renderType,
					textureIdentity,
					i,
					livingEntityRenderState.outlineColor,
					bl,
					bl2,
					livingEntityRenderState.appearsGlowing()
				)
				|| this.model instanceof CowModel
				&& livingEntityRenderState instanceof MushroomCowRenderState mushroomCowRenderState
				&& net.vulkanic.world.RustGalWorldPrimitiveRenderer.isVanillaMushroomCowModelMeshEligible(
					this.model, mushroomCowRenderState, renderType, textureIdentity, i,
					livingEntityRenderState.outlineColor, bl, bl2, livingEntityRenderState.appearsGlowing()
				)
				|| this.model.getClass() == ModelLobster.class
				&& livingEntityRenderState instanceof LobsterRenderState lobsterRenderState
				&& net.vulkanic.world.RustGalWorldPrimitiveRenderer.isLobsterModelMeshEligible(
					this.model, lobsterRenderState, renderType, textureIdentity, i,
					livingEntityRenderState.outlineColor, bl, bl2, livingEntityRenderState.appearsGlowing()
				)
				|| (this.model instanceof net.alexsmobs.client.model.ModelBison
					|| this.model instanceof net.alexsmobs.client.model.ModelBisonBaby)
				&& livingEntityRenderState instanceof net.alexsmobs.client.render.BisonRenderState bisonRenderState
				&& net.vulkanic.world.RustGalWorldPrimitiveRenderer.isBisonModelMeshEligible(
					this.model, bisonRenderState, renderType, textureIdentity, i,
					livingEntityRenderState.outlineColor, bl, bl2, livingEntityRenderState.appearsGlowing()
				)
				|| this.model != null
					&& this.model instanceof PigModel
					&& livingEntityRenderState instanceof PigRenderState pigRenderState
					&& net.vulkanic.world.RustGalWorldPrimitiveRenderer.isVanillaPigModelMeshEligible(
						this.model,
						pigRenderState,
						renderType,
						textureIdentity,
						i,
						livingEntityRenderState.outlineColor,
						bl,
						bl2,
						livingEntityRenderState.appearsGlowing()
					)
				|| this.model != null
					&& this.model.getClass() == CatModel.class
					&& livingEntityRenderState instanceof CatRenderState catRenderState
					&& net.vulkanic.world.RustGalWorldPrimitiveRenderer.isVanillaCatModelMeshEligible(
						this.model, catRenderState, renderType, textureIdentity, i,
						livingEntityRenderState.outlineColor, bl, bl2, livingEntityRenderState.appearsGlowing()
					)
				|| this.model != null
					&& this.model.getClass() == WolfModel.class
					&& livingEntityRenderState instanceof WolfRenderState wolfRenderState
					&& net.vulkanic.world.RustGalWorldPrimitiveRenderer.isVanillaWolfModelMeshEligible(
						this.model, wolfRenderState, renderType, textureIdentity, i,
						livingEntityRenderState.outlineColor, bl, bl2, livingEntityRenderState.appearsGlowing()
					)
				|| this.model != null
					&& this.model.getClass() == ParrotModel.class
					&& livingEntityRenderState instanceof ParrotRenderState parrotRenderState
					&& net.vulkanic.world.RustGalWorldPrimitiveRenderer.isVanillaParrotModelMeshEligible(
						this.model, parrotRenderState, renderType, textureIdentity, i,
						livingEntityRenderState.outlineColor, bl, bl2, livingEntityRenderState.appearsGlowing()
					)
				|| this.model != null
					&& this.model.getClass() == GhastModel.class
					&& livingEntityRenderState instanceof GhastRenderState ghastRenderState
					&& net.vulkanic.world.RustGalWorldPrimitiveRenderer.isVanillaGhastModelMeshEligible(
						this.model, ghastRenderState, renderType, textureIdentity, i,
						livingEntityRenderState.outlineColor, bl, bl2, livingEntityRenderState.appearsGlowing()
					)
				|| this.model != null
					&& this.model.getClass() == net.minecraft.client.model.HappyGhastModel.class
					&& livingEntityRenderState instanceof net.minecraft.client.renderer.entity.state.HappyGhastRenderState happyGhastRenderState
					&& net.vulkanic.world.RustGalWorldPrimitiveRenderer.isVanillaHappyGhastModelMeshEligible(
						this.model, happyGhastRenderState, renderType, textureIdentity, i,
						livingEntityRenderState.outlineColor, bl, bl2, livingEntityRenderState.appearsGlowing()
					)
				|| this.model != null
					&& this.model.getClass() == BlazeModel.class
					&& livingEntityRenderState instanceof LivingEntityRenderState blazeRenderState
					&& net.vulkanic.world.RustGalWorldPrimitiveRenderer.isVanillaBlazeModelMeshEligible(
						this.model, blazeRenderState, renderType, textureIdentity, i,
						livingEntityRenderState.outlineColor, bl, bl2, livingEntityRenderState.appearsGlowing()
					)
				|| this.model != null
					&& this.model.getClass() == FoxModel.class
					&& livingEntityRenderState instanceof FoxRenderState foxRenderState
					&& net.vulkanic.world.RustGalWorldPrimitiveRenderer.isVanillaFoxModelMeshEligible(
						this.model, foxRenderState, renderType, textureIdentity, i,
						livingEntityRenderState.outlineColor, bl, bl2, livingEntityRenderState.appearsGlowing()
					)
				|| this.model != null
					&& this.model.getClass() == ZombieModel.class
					&& livingEntityRenderState instanceof ZombieRenderState zombieRenderState
					&& net.vulkanic.world.RustGalWorldPrimitiveRenderer.isVanillaZombieModelMeshEligible(
						this.model,
						zombieRenderState,
						renderType,
						textureIdentity,
						i,
						livingEntityRenderState.outlineColor,
						bl,
						bl2,
						livingEntityRenderState.appearsGlowing()
					)
				|| this.model != null
					&& this.model.getClass() == RabbitModel.class
					&& livingEntityRenderState instanceof RabbitRenderState rabbitRenderState
					&& net.vulkanic.world.RustGalWorldPrimitiveRenderer.isVanillaRabbitModelMeshEligible(
						this.model,
						rabbitRenderState,
						renderType,
						textureIdentity,
						i,
						livingEntityRenderState.outlineColor,
						bl,
						bl2,
						livingEntityRenderState.appearsGlowing()
					)
				|| this.model != null
					&& this.model.getClass() == SheepModel.class
					&& livingEntityRenderState instanceof SheepRenderState sheepRenderState
					&& net.vulkanic.world.RustGalWorldPrimitiveRenderer.isVanillaSheepModelMeshEligible(
						this.model, sheepRenderState, renderType, textureIdentity, i,
						livingEntityRenderState.outlineColor, bl, bl2, livingEntityRenderState.appearsGlowing()
					)
				|| this.model != null
					&& this.model.getClass() == CreeperModel.class
					&& livingEntityRenderState instanceof CreeperRenderState creeperRenderState
					&& net.vulkanic.world.RustGalWorldPrimitiveRenderer.isVanillaCreeperModelMeshEligible(
						this.model, creeperRenderState, renderType, textureIdentity, i,
						livingEntityRenderState.outlineColor, bl, bl2, livingEntityRenderState.appearsGlowing()
					)
				|| this.model != null
					&& this.model.getClass() == SlimeModel.class
					&& livingEntityRenderState instanceof SlimeRenderState slimeRenderState
					&& net.vulkanic.world.RustGalWorldPrimitiveRenderer.isVanillaSlimeModelMeshEligible(
						this.model, slimeRenderState, renderType, textureIdentity, i,
						livingEntityRenderState.outlineColor, bl, bl2, livingEntityRenderState.appearsGlowing()
					)
				|| this.model != null
					&& this.model.getClass() == LavaSlimeModel.class
					&& livingEntityRenderState instanceof SlimeRenderState magmaCubeRenderState
					&& net.vulkanic.world.RustGalWorldPrimitiveRenderer.isVanillaMagmaCubeModelMeshEligible(
						this.model, magmaCubeRenderState, renderType, textureIdentity, i,
						livingEntityRenderState.outlineColor, bl, bl2, livingEntityRenderState.appearsGlowing()
					)
				|| this.model != null
					&& this.model.getClass() == HorseModel.class
					&& livingEntityRenderState instanceof HorseRenderState horseRenderState
					&& net.vulkanic.world.RustGalWorldPrimitiveRenderer.isVanillaHorseModelMeshEligible(
						this.model, horseRenderState, renderType, textureIdentity, i,
						livingEntityRenderState.outlineColor, bl, bl2, livingEntityRenderState.appearsGlowing()
					)
				|| this.model != null
					&& this.model.getClass() == DonkeyModel.class
					&& livingEntityRenderState instanceof DonkeyRenderState donkeyRenderState
					&& net.vulkanic.world.RustGalWorldPrimitiveRenderer.isVanillaDonkeyModelMeshEligible(
						this.model, donkeyRenderState, renderType, textureIdentity, i,
						livingEntityRenderState.outlineColor, bl, bl2, livingEntityRenderState.appearsGlowing()
					)
				|| this.model != null
					&& this.model.getClass() == LlamaModel.class
					&& livingEntityRenderState instanceof LlamaRenderState llamaRenderState
					&& net.vulkanic.world.RustGalWorldPrimitiveRenderer.isVanillaLlamaModelMeshEligible(
						this.model, llamaRenderState, renderType, textureIdentity, i,
						livingEntityRenderState.outlineColor, bl, bl2, livingEntityRenderState.appearsGlowing()
					)
				|| this.model != null
					&& this.model.getClass() == StriderModel.class
					&& livingEntityRenderState instanceof StriderRenderState striderRenderState
					&& net.vulkanic.world.RustGalWorldPrimitiveRenderer.isVanillaStriderModelMeshEligible(
						this.model, striderRenderState, renderType, textureIdentity, i,
						livingEntityRenderState.outlineColor, bl, bl2, livingEntityRenderState.appearsGlowing()
					)
				|| this.model != null
					&& this.model.getClass() == HoglinModel.class
					&& livingEntityRenderState instanceof HoglinRenderState hoglinRenderState
					&& net.vulkanic.world.RustGalWorldPrimitiveRenderer.isVanillaHoglinModelMeshEligible(
						this.model, hoglinRenderState, renderType, textureIdentity, i,
						livingEntityRenderState.outlineColor, bl, bl2, livingEntityRenderState.appearsGlowing()
					)
				|| this.model != null
					&& this.model.getClass() == CamelModel.class
					&& livingEntityRenderState instanceof CamelRenderState camelRenderState
					&& net.vulkanic.world.RustGalWorldPrimitiveRenderer.isVanillaCamelModelMeshEligible(
						this.model, camelRenderState, renderType, textureIdentity, i,
						livingEntityRenderState.outlineColor, bl, bl2, livingEntityRenderState.appearsGlowing()
					)
				|| this.model != null
					&& (this.model.getClass() == PiglinModel.class || this.model.getClass() == ZombifiedPiglinModel.class)
					&& livingEntityRenderState instanceof HumanoidRenderState piglinRenderState
					&& net.vulkanic.world.RustGalWorldPrimitiveRenderer.isVanillaPiglinModelMeshEligible(
						this.model, piglinRenderState, renderType, textureIdentity, i,
						livingEntityRenderState.outlineColor, bl, bl2, livingEntityRenderState.appearsGlowing()
					)
				|| this.model != null
					&& this.model.getClass() == SkeletonModel.class
					&& livingEntityRenderState instanceof SkeletonRenderState skeletonRenderState
					&& net.vulkanic.world.RustGalWorldPrimitiveRenderer.isVanillaSkeletonModelMeshEligible(
						this.model, skeletonRenderState, renderType, textureIdentity, i,
						livingEntityRenderState.outlineColor, bl, bl2, livingEntityRenderState.appearsGlowing()
					)
				|| this.model != null
					&& this.model.getClass() == SkeletonModel.class
					&& livingEntityRenderState instanceof SkeletonRenderState strayRenderState
					&& net.vulkanic.world.RustGalWorldPrimitiveRenderer.isVanillaStrayModelMeshEligible(
						this.model, strayRenderState, renderType, textureIdentity, i,
						livingEntityRenderState.outlineColor, bl, bl2, livingEntityRenderState.appearsGlowing()
					)
				|| this.model != null
					&& this.model.getClass() == BoggedModel.class
					&& livingEntityRenderState instanceof BoggedRenderState boggedRenderState
					&& net.vulkanic.world.RustGalWorldPrimitiveRenderer.isVanillaBoggedModelMeshEligible(
						this.model, boggedRenderState, renderType, textureIdentity, i,
						livingEntityRenderState.outlineColor, bl, bl2, livingEntityRenderState.appearsGlowing()
					)
				|| this.model != null
					&& this.model.getClass() == GiantZombieModel.class
					&& livingEntityRenderState instanceof ZombieRenderState giantRenderState
					&& net.vulkanic.world.RustGalWorldPrimitiveRenderer.isVanillaGiantModelMeshEligible(
						this.model, giantRenderState, renderType, textureIdentity, i,
						livingEntityRenderState.outlineColor, bl, bl2, livingEntityRenderState.appearsGlowing()
					)
				|| this.model != null
					&& this.model.getClass() == ArmadilloModel.class
					&& livingEntityRenderState instanceof ArmadilloRenderState armadilloRenderState
					&& net.vulkanic.world.RustGalWorldPrimitiveRenderer.isVanillaArmadilloModelMeshEligible(
						this.model, armadilloRenderState, renderType, textureIdentity, i,
						livingEntityRenderState.outlineColor, bl, bl2, livingEntityRenderState.appearsGlowing()
					)
				|| this.model != null
					&& this.model.getClass() == SnifferModel.class
					&& livingEntityRenderState instanceof SnifferRenderState snifferRenderState
					&& net.vulkanic.world.RustGalWorldPrimitiveRenderer.isVanillaSnifferModelMeshEligible(
						this.model, snifferRenderState, renderType, textureIdentity, i,
						livingEntityRenderState.outlineColor, bl, bl2, livingEntityRenderState.appearsGlowing()
					)
				|| this.model != null
					&& (this.model.getClass() == net.minecraft.client.model.animal.nautilus.NautilusModel.class
						|| this.model instanceof net.minecraft.client.model.animal.nautilus.NautilusModel)
					&& livingEntityRenderState instanceof NautilusRenderState nautilusRenderState
					&& net.vulkanic.world.RustGalWorldPrimitiveRenderer.isVanillaNautilusModelMeshEligible(
						this.model, nautilusRenderState, renderType, textureIdentity, i,
						livingEntityRenderState.outlineColor, bl, bl2, livingEntityRenderState.appearsGlowing()
					)
				|| this.model != null
					&& this.model.getClass() == net.minecraft.client.model.PhantomModel.class
					&& livingEntityRenderState instanceof PhantomRenderState phantomRenderState
					&& net.vulkanic.world.RustGalWorldPrimitiveRenderer.isVanillaPhantomModelMeshEligible(
						this.model, phantomRenderState, renderType, textureIdentity, i,
						livingEntityRenderState.outlineColor, bl, bl2, livingEntityRenderState.appearsGlowing()
					)
				|| this.model != null
					&& this.model.getClass() == net.minecraft.client.model.WardenModel.class
					&& livingEntityRenderState instanceof WardenRenderState wardenRenderState
					&& net.vulkanic.world.RustGalWorldPrimitiveRenderer.isVanillaWardenModelMeshEligible(
						this.model, wardenRenderState, renderType, textureIdentity, i,
						livingEntityRenderState.outlineColor, bl, bl2, livingEntityRenderState.appearsGlowing()
					)
				|| this.model != null
					&& this.model.getClass() == net.minecraft.client.model.CreakingModel.class
					&& livingEntityRenderState instanceof CreakingRenderState creakingRenderState
					&& net.vulkanic.world.RustGalWorldPrimitiveRenderer.isVanillaCreakingModelMeshEligible(
						this.model, creakingRenderState, renderType, textureIdentity, i,
						livingEntityRenderState.outlineColor, bl, bl2, livingEntityRenderState.appearsGlowing()
					)
				|| this.model != null
					&& this.model.getClass() == net.minecraft.client.model.BreezeModel.class
					&& livingEntityRenderState instanceof BreezeRenderState breezeRenderState
					&& net.vulkanic.world.RustGalWorldPrimitiveRenderer.isVanillaBreezeModelMeshEligible(
						this.model, breezeRenderState, renderType, textureIdentity, i,
						livingEntityRenderState.outlineColor, bl, bl2, livingEntityRenderState.appearsGlowing()
					)
				|| this.model != null
					&& this.model.getClass() == net.minecraft.client.model.EndermanModel.class
					&& livingEntityRenderState instanceof EndermanRenderState endermanRenderState
					&& net.vulkanic.world.RustGalWorldPrimitiveRenderer.isVanillaEndermanModelMeshEligible(
						this.model, endermanRenderState, renderType, textureIdentity, i,
						livingEntityRenderState.outlineColor, bl, bl2, livingEntityRenderState.appearsGlowing()
					)
				|| this.model != null
					&& this.model.getClass() == net.minecraft.client.model.CopperGolemModel.class
					&& livingEntityRenderState instanceof CopperGolemRenderState copperGolemRenderState
					&& net.vulkanic.world.RustGalWorldPrimitiveRenderer.isVanillaCopperGolemModelMeshEligible(
						this.model, copperGolemRenderState, renderType, textureIdentity, i,
						livingEntityRenderState.outlineColor, bl, bl2, livingEntityRenderState.appearsGlowing()
					)
				|| this.model != null
					&& this.model.getClass() == net.minecraft.client.model.WitherBossModel.class
					&& livingEntityRenderState instanceof WitherRenderState witherRenderState
					&& net.vulkanic.world.RustGalWorldPrimitiveRenderer.isVanillaWitherModelMeshEligible(
						this.model, witherRenderState, renderType, textureIdentity, i,
						livingEntityRenderState.outlineColor, bl, bl2, livingEntityRenderState.appearsGlowing()
					)
				|| this.model != null
					&& this.model.getClass() == DrownedModel.class
					&& livingEntityRenderState instanceof ZombieRenderState drownedRenderState
					&& net.vulkanic.world.RustGalWorldPrimitiveRenderer.isVanillaDrownedModelMeshEligible(
						this.model, drownedRenderState, renderType, textureIdentity, i,
						livingEntityRenderState.outlineColor, bl, bl2, livingEntityRenderState.appearsGlowing()
					)
				|| this.model != null
					&& this.model.getClass() == EndermiteModel.class
					&& net.vulkanic.world.RustGalWorldPrimitiveRenderer.isVanillaEndermiteModelMeshEligible(
						this.model, livingEntityRenderState, renderType, textureIdentity, i,
						livingEntityRenderState.outlineColor, bl, bl2, livingEntityRenderState.appearsGlowing()
					)
				|| this.model != null
					&& this.model.getClass() == SilverfishModel.class
					&& net.vulkanic.world.RustGalWorldPrimitiveRenderer.isVanillaSilverfishModelMeshEligible(
						this.model, livingEntityRenderState, renderType, textureIdentity, i,
						livingEntityRenderState.outlineColor, bl, bl2, livingEntityRenderState.appearsGlowing()
					)
				|| this.model != null
					&& this.model.getClass() == BatModel.class
					&& livingEntityRenderState instanceof BatRenderState batRenderState
					&& net.vulkanic.world.RustGalWorldPrimitiveRenderer.isVanillaBatModelMeshEligible(
						this.model, batRenderState, renderType, textureIdentity, i,
						livingEntityRenderState.outlineColor, bl, bl2, livingEntityRenderState.appearsGlowing()
					)
				|| this.model != null
					&& this.model.getClass() == CodModel.class
					&& net.vulkanic.world.RustGalWorldPrimitiveRenderer.isVanillaCodModelMeshEligible(
						this.model, livingEntityRenderState, renderType, textureIdentity, i,
						livingEntityRenderState.outlineColor, bl, bl2, livingEntityRenderState.appearsGlowing()
					)
				|| this.model != null
					&& this.model.getClass() == SalmonModel.class
					&& livingEntityRenderState instanceof SalmonRenderState salmonRenderState
					&& net.vulkanic.world.RustGalWorldPrimitiveRenderer.isVanillaSalmonModelMeshEligible(
						this.model, salmonRenderState, renderType, textureIdentity, i,
						livingEntityRenderState.outlineColor, bl, bl2, livingEntityRenderState.appearsGlowing()
					)
				|| this.model != null
					&& (this.model.getClass() == PufferfishBigModel.class
						|| this.model.getClass() == PufferfishMidModel.class
						|| this.model.getClass() == PufferfishSmallModel.class)
					&& livingEntityRenderState instanceof PufferfishRenderState pufferfishRenderState
					&& net.vulkanic.world.RustGalWorldPrimitiveRenderer.isVanillaPufferfishModelMeshEligible(
						this.model, pufferfishRenderState, renderType, textureIdentity, i,
						livingEntityRenderState.outlineColor, bl, bl2, livingEntityRenderState.appearsGlowing()
					)
				|| this.model != null
					&& this.model.getClass() == TadpoleModel.class
					&& net.vulkanic.world.RustGalWorldPrimitiveRenderer.isVanillaTadpoleModelMeshEligible(
						this.model, livingEntityRenderState, renderType, textureIdentity, i,
						livingEntityRenderState.outlineColor, bl, bl2, livingEntityRenderState.appearsGlowing()
					)
				|| this.model != null
					&& this.model.getClass() == OcelotModel.class
					&& livingEntityRenderState instanceof FelineRenderState felineRenderState
					&& net.vulkanic.world.RustGalWorldPrimitiveRenderer.isVanillaOcelotModelMeshEligible(
						this.model, felineRenderState, renderType, textureIdentity, i,
						livingEntityRenderState.outlineColor, bl, bl2, livingEntityRenderState.appearsGlowing()
					)
				|| this.model != null
					&& this.model.getClass() == GoatModel.class
					&& livingEntityRenderState instanceof GoatRenderState goatRenderState
					&& net.vulkanic.world.RustGalWorldPrimitiveRenderer.isVanillaGoatModelMeshEligible(
						this.model, goatRenderState, renderType, textureIdentity, i,
						livingEntityRenderState.outlineColor, bl, bl2, livingEntityRenderState.appearsGlowing()
					)
				|| this.model != null
					&& (this.model.getClass() == TropicalFishModelA.class || this.model.getClass() == TropicalFishModelB.class)
					&& livingEntityRenderState instanceof TropicalFishRenderState tropicalFishRenderState
					&& net.vulkanic.world.RustGalWorldPrimitiveRenderer.isVanillaTropicalFishModelMeshEligible(
						this.model, tropicalFishRenderState, renderType, textureIdentity, i,
						livingEntityRenderState.outlineColor, bl, bl2, livingEntityRenderState.appearsGlowing()
					)
				|| this.model != null
					&& this.model.getClass() == PolarBearModel.class
					&& livingEntityRenderState instanceof PolarBearRenderState polarBearRenderState
					&& net.vulkanic.world.RustGalWorldPrimitiveRenderer.isVanillaPolarBearModelMeshEligible(
						this.model, polarBearRenderState, renderType, textureIdentity, i,
						livingEntityRenderState.outlineColor, bl, bl2, livingEntityRenderState.appearsGlowing()
					)
				|| this.model != null
					&& this.model.getClass() == DolphinModel.class
					&& livingEntityRenderState instanceof DolphinRenderState dolphinRenderState
					&& net.vulkanic.world.RustGalWorldPrimitiveRenderer.isVanillaDolphinModelMeshEligible(
						this.model, dolphinRenderState, renderType, textureIdentity, i,
						livingEntityRenderState.outlineColor, bl, bl2, livingEntityRenderState.appearsGlowing()
					)
				|| this.model != null
					&& this.model.getClass() == TurtleModel.class
					&& livingEntityRenderState instanceof TurtleRenderState turtleRenderState
					&& net.vulkanic.world.RustGalWorldPrimitiveRenderer.isVanillaTurtleModelMeshEligible(
						this.model, turtleRenderState, renderType, textureIdentity, i,
						livingEntityRenderState.outlineColor, bl, bl2, livingEntityRenderState.appearsGlowing()
					)
				|| this.model != null
					&& this.model.getClass() == PandaModel.class
					&& livingEntityRenderState instanceof PandaRenderState pandaRenderState
					&& net.vulkanic.world.RustGalWorldPrimitiveRenderer.isVanillaPandaModelMeshEligible(
						this.model, pandaRenderState, renderType, textureIdentity, i,
						livingEntityRenderState.outlineColor, bl, bl2, livingEntityRenderState.appearsGlowing()
					)
				|| this.model != null
					&& this.model.getClass() == BeeModel.class
					&& livingEntityRenderState instanceof BeeRenderState beeRenderState
					&& net.vulkanic.world.RustGalWorldPrimitiveRenderer.isVanillaBeeModelMeshEligible(
						this.model, beeRenderState, renderType, textureIdentity, i,
						livingEntityRenderState.outlineColor, bl, bl2, livingEntityRenderState.appearsGlowing()
					)
				|| this.model != null
					&& this.model.getClass() == AxolotlModel.class
					&& livingEntityRenderState instanceof AxolotlRenderState axolotlRenderState
					&& net.vulkanic.world.RustGalWorldPrimitiveRenderer.isVanillaAxolotlModelMeshEligible(
						this.model, axolotlRenderState, renderType, textureIdentity, i,
						livingEntityRenderState.outlineColor, bl, bl2, livingEntityRenderState.appearsGlowing()
					)
				|| this.model != null
					&& this.model.getClass() == FrogModel.class
					&& livingEntityRenderState instanceof FrogRenderState frogRenderState
					&& net.vulkanic.world.RustGalWorldPrimitiveRenderer.isVanillaFrogModelMeshEligible(
						this.model, frogRenderState, renderType, textureIdentity, i,
						livingEntityRenderState.outlineColor, bl, bl2, livingEntityRenderState.appearsGlowing()
					)
				|| this.model != null
					&& this.model.getClass() == SquidModel.class
					&& livingEntityRenderState instanceof SquidRenderState squidRenderState
					&& net.vulkanic.world.RustGalWorldPrimitiveRenderer.isVanillaSquidModelMeshEligible(
						this.model, squidRenderState, renderType, textureIdentity, i,
						livingEntityRenderState.outlineColor, bl, bl2, livingEntityRenderState.appearsGlowing()
					)
				|| this.model != null
					&& this.model.getClass() == GuardianModel.class
					&& livingEntityRenderState instanceof GuardianRenderState guardianRenderState
					&& net.vulkanic.world.RustGalWorldPrimitiveRenderer.isVanillaGuardianModelMeshEligible(
						this.model, guardianRenderState, renderType, textureIdentity, i,
						livingEntityRenderState.outlineColor, bl, bl2, livingEntityRenderState.appearsGlowing()
					)
				|| this.model != null
					&& this.model.getClass() == SpiderModel.class
					&& livingEntityRenderState instanceof LivingEntityRenderState spiderRenderState
					&& net.vulkanic.world.RustGalWorldPrimitiveRenderer.isVanillaSpiderModelMeshEligible(
						this.model, spiderRenderState, renderType, textureIdentity, i,
						livingEntityRenderState.outlineColor, bl, bl2, livingEntityRenderState.appearsGlowing()
					)
				|| this.model != null
					&& this.model.getClass() == SnowGolemModel.class
					&& livingEntityRenderState instanceof SnowGolemRenderState snowGolemRenderState
					&& net.vulkanic.world.RustGalWorldPrimitiveRenderer.isVanillaSnowGolemModelMeshEligible(
						this.model, snowGolemRenderState, renderType, textureIdentity, i,
						livingEntityRenderState.outlineColor, bl, bl2, livingEntityRenderState.appearsGlowing()
					)
				|| this.model != null
					&& this.model.getClass() == IronGolemModel.class
					&& livingEntityRenderState instanceof IronGolemRenderState ironGolemRenderState
					&& net.vulkanic.world.RustGalWorldPrimitiveRenderer.isVanillaIronGolemModelMeshEligible(
						this.model, ironGolemRenderState, renderType, textureIdentity, i,
						livingEntityRenderState.outlineColor, bl, bl2, livingEntityRenderState.appearsGlowing()
					)
				|| this.model != null
					&& this.model.getClass() == RavagerModel.class
					&& livingEntityRenderState instanceof RavagerRenderState ravagerRenderState
					&& net.vulkanic.world.RustGalWorldPrimitiveRenderer.isVanillaRavagerModelMeshEligible(
						this.model, ravagerRenderState, renderType, textureIdentity, i,
						livingEntityRenderState.outlineColor, bl, bl2, livingEntityRenderState.appearsGlowing()
					)
				|| this.model != null
					&& this.model.getClass() == VexModel.class
					&& livingEntityRenderState instanceof VexRenderState vexRenderState
					&& net.vulkanic.world.RustGalWorldPrimitiveRenderer.isVanillaVexModelMeshEligible(
						this.model, vexRenderState, renderType, textureIdentity, i,
						livingEntityRenderState.outlineColor, bl, bl2, livingEntityRenderState.appearsGlowing()
					)
				|| this.model != null
					&& this.model.getClass() == AllayModel.class
					&& livingEntityRenderState instanceof AllayRenderState allayRenderState
					&& net.vulkanic.world.RustGalWorldPrimitiveRenderer.isVanillaAllayModelMeshEligible(
						this.model, allayRenderState, renderType, textureIdentity, i,
						livingEntityRenderState.outlineColor, bl, bl2, livingEntityRenderState.appearsGlowing()
					)
				|| this.model != null
					&& this.model.getClass() == WitchModel.class
					&& livingEntityRenderState instanceof WitchRenderState witchRenderState
					&& net.vulkanic.world.RustGalWorldPrimitiveRenderer.isVanillaWitchModelMeshEligible(
						this.model, witchRenderState, renderType, textureIdentity, i,
						livingEntityRenderState.outlineColor, bl, bl2, livingEntityRenderState.appearsGlowing()
					)
				|| this.model instanceof IllagerModel
					&& livingEntityRenderState instanceof EvokerRenderState evokerRenderState
					&& net.vulkanic.world.RustGalWorldPrimitiveRenderer.isVanillaEvokerModelMeshEligible(
						this.model, evokerRenderState, renderType, textureIdentity, i,
						livingEntityRenderState.outlineColor, bl, bl2, livingEntityRenderState.appearsGlowing()
					)
				|| this.model instanceof IllagerModel
					&& livingEntityRenderState instanceof IllagerRenderState illagerRenderState
					&& net.vulkanic.world.RustGalWorldPrimitiveRenderer.isVanillaVindicatorOrPillagerModelMeshEligible(
						this.model, illagerRenderState, renderType, textureIdentity, i,
						livingEntityRenderState.outlineColor, bl, bl2, livingEntityRenderState.appearsGlowing()
					));
			var rustLivingModelOwnership = net.vulkanic.world.LivingEntityBaseModelOwnershipPolicy.currentOwnershipRoute(rustLivingModelFamily);
			boolean semanticSubmission = EntityRenderDispatcher.isSemanticSubmission();
			boolean rustLivingOutlineOnlySubmitted = false;
			if (!semanticSubmission && rustOutlineOnlyLivingBody && rustLivingModelFamily && entityIdentity != null
				&& net.vulkanic.world.WorldRenderRoutePolicy.currentModelMeshRoute(true).usesRustWholeFrameVulkan()) {
				RenderType outlineMaterial = this.model.renderType(textureIdentity);
				if (outlineMaterial == null || outlineMaterial.isOutline()
					|| !net.vulkanic.world.RustGalWorldPrimitiveRenderer.enqueueStandaloneModelMeshOutlineOnly(
						this.model, livingEntityRenderState, poseStack.last(), outlineMaterial, textureIdentity,
						entityIdentity, livingEntityRenderState.lightCoords, livingEntityRenderState.outlineColor)) {
					throw new IllegalStateException("Rust whole-frame invisible-glowing living model has no semantic outline mesh");
				}
				rustLivingOutlineOnlySubmitted = true;
			}
			var rustLivingModelDisposition = net.vulkanic.world.LivingEntityBaseModelOwnershipPolicy.classify(
				semanticSubmission, rustLivingModelFamily, rustLivingModelEligible || rustLivingOutlineOnlySubmitted, rustLivingModelOwnership
			);
			if (semanticSubmission && rustOutlineOnlyLivingBody) {
				submitNodeCollector.submitModelOutlineSemanticTexture(this.model, livingEntityRenderState,
					poseStack, this.model.renderType(textureIdentity), livingEntityRenderState.lightCoords,
					textureIdentity, livingEntityRenderState.outlineColor);
			} else if (rustLivingModelDisposition == net.vulkanic.world.LivingEntityBaseModelOwnershipPolicy.Disposition.RUST_AVAILABLE) {
				if (!rustLivingOutlineOnlySubmitted && !net.vulkanic.world.RustGalWorldPrimitiveRenderer.enqueueStandaloneModelMesh(
					this.model, livingEntityRenderState, poseStack.last(), renderType, textureIdentity, entityIdentity,
					livingEntityRenderState.lightCoords, i, k, livingEntityRenderState.outlineColor
				)) {
					throw new IllegalStateException("Rust whole-frame living-model route selected without a copied indexed mesh request");
				}
				net.vulkanic.world.RustGalWorldPrimitiveRenderer.recordModelMeshRouteDecision(
					"rust-vulkan-whole-frame", textureIdentity, this.model.getClass().getName(), livingEntityRenderState.entityId, true, true, false
				);
			} else if (rustLivingModelDisposition == net.vulkanic.world.LivingEntityBaseModelOwnershipPolicy.Disposition.RUST_UNAVAILABLE) {
				net.vulkanic.world.RustGalWorldPrimitiveRenderer.recordModelMeshRouteDecision(
					"rust-vulkan-unavailable", textureIdentity, this.model.getClass().getName(), livingEntityRenderState.entityId, false, false, false
				);
				// Keep the explicit route fail-closed when a copied semantic asset is
				// temporarily unavailable (for example during a resource reload). Do
				// not fall back to Java rendering or leak a backend texture handle; the
				// entity is simply absent from this frame until the Rust asset becomes
				// available again.
				return;
			} else {
				if (rustLivingModelFamily && !semanticSubmission) {
					net.vulkanic.world.RustGalWorldPrimitiveRenderer.recordModelMeshRouteDecision(
						rustLivingModelOwnership == net.vulkanic.world.WorldRenderRoutePolicy.Route.DISABLED ? "disabled" : "java-legacy",
						textureIdentity,
						this.model.getClass().getName(),
						livingEntityRenderState.entityId,
						false,
						false,
						rustLivingModelOwnership.usesJavaCompatibility()
					);
				}
				// Every living-entity base callsite already has an immutable direct
				// texture identity. Preserve that semantic input for the generic Rust
				// mesh route instead of discarding it into the atlas-only submit path;
				// the collector still performs bounded copied-asset admission and the
				// legacy route remains unchanged when Rust whole-frame ownership is off.
				if (textureIdentity != null) {
					submitNodeCollector.submitModelSemanticTexture(
						this.model, livingEntityRenderState, poseStack, renderType,
						livingEntityRenderState.lightCoords, i, k, textureIdentity,
						livingEntityRenderState.outlineColor, null
					);
				} else {
					submitNodeCollector.submitModelSemantic(
						this.model, livingEntityRenderState, poseStack, renderType,
						livingEntityRenderState.lightCoords, i, k, null,
						livingEntityRenderState.outlineColor, null
					);
				}
			}
		}

		if (this.shouldRenderLayers(livingEntityRenderState) && !this.layers.isEmpty()) {
			this.model.setupAnim(livingEntityRenderState);
			boolean rustItemScope = net.vulkanic.VulkanicAPI.isVulkanBackendSelected()
				|| net.vulkanic.bridge.RustGalVulkanWholeFrameMode.enabled();
			if (rustItemScope) net.vulkanic.world.RustGalWorldPrimitiveRenderer.beginItemEntitySubmission();
			try {
				for (RenderLayer<S, M> renderLayer : this.layers) {
					renderLayer.submit(
						poseStack, submitNodeCollector, livingEntityRenderState.lightCoords, livingEntityRenderState, livingEntityRenderState.yRot, livingEntityRenderState.xRot
					);
				}
			} finally {
				if (rustItemScope) net.vulkanic.world.RustGalWorldPrimitiveRenderer.endItemEntitySubmission();
			}
		}

		poseStack.popPose();
		super.submit(livingEntityRenderState, poseStack, submitNodeCollector, cameraRenderState);
	}

	protected boolean shouldRenderLayers(S livingEntityRenderState) {
		return true;
	}

	protected int getModelTint(S livingEntityRenderState) {
		return -1;
	}

	public abstract ResourceLocation getTextureLocation(S livingEntityRenderState);

	@Nullable
	protected RenderType getRenderType(S livingEntityRenderState, boolean bl, boolean bl2, boolean bl3) {
		ResourceLocation resourceLocation = this.getTextureLocation(livingEntityRenderState);
		if (bl2) {
			return RenderType.itemEntityTranslucentCull(resourceLocation);
		} else if (bl) {
			return this.model.renderType(resourceLocation);
		} else {
			return bl3 ? RenderType.outline(resourceLocation) : null;
		}
	}

	public static int getOverlayCoords(LivingEntityRenderState livingEntityRenderState, float f) {
		return OverlayTexture.pack(OverlayTexture.u(f), OverlayTexture.v(livingEntityRenderState.hasRedOverlay));
	}

	protected boolean isBodyVisible(S livingEntityRenderState) {
		return !livingEntityRenderState.isInvisible;
	}

	private static float sleepDirectionToRotation(Direction direction) {
		switch (direction) {
			case SOUTH:
				return 90.0F;
			case WEST:
				return 0.0F;
			case NORTH:
				return 270.0F;
			case EAST:
				return 180.0F;
			default:
				return 0.0F;
		}
	}

	protected boolean isShaking(S livingEntityRenderState) {
		return livingEntityRenderState.isFullyFrozen;
	}

	protected void setupRotations(S livingEntityRenderState, PoseStack poseStack, float f, float g) {
		if (this.isShaking(livingEntityRenderState)) {
			f += (float)(Math.cos(Mth.floor(livingEntityRenderState.ageInTicks) * 3.25F) * Math.PI * 0.4F);
		}

		if (!livingEntityRenderState.hasPose(Pose.SLEEPING)) {
			poseStack.mulPose(Axis.YP.rotationDegrees(180.0F - f));
		}

		if (livingEntityRenderState.deathTime > 0.0F) {
			float h = (livingEntityRenderState.deathTime - 1.0F) / 20.0F * 1.6F;
			h = Mth.sqrt(h);
			if (h > 1.0F) {
				h = 1.0F;
			}

			poseStack.mulPose(Axis.ZP.rotationDegrees(h * this.getFlipDegrees()));
		} else if (livingEntityRenderState.isAutoSpinAttack) {
			poseStack.mulPose(Axis.XP.rotationDegrees(-90.0F - livingEntityRenderState.xRot));
			poseStack.mulPose(Axis.YP.rotationDegrees(livingEntityRenderState.ageInTicks * -75.0F));
		} else if (livingEntityRenderState.hasPose(Pose.SLEEPING)) {
			Direction direction = livingEntityRenderState.bedOrientation;
			float i = direction != null ? sleepDirectionToRotation(direction) : f;
			poseStack.mulPose(Axis.YP.rotationDegrees(i));
			poseStack.mulPose(Axis.ZP.rotationDegrees(this.getFlipDegrees()));
			poseStack.mulPose(Axis.YP.rotationDegrees(270.0F));
		} else if (livingEntityRenderState.isUpsideDown) {
			poseStack.translate(0.0F, (livingEntityRenderState.boundingBoxHeight + 0.1F) / g, 0.0F);
			poseStack.mulPose(Axis.ZP.rotationDegrees(180.0F));
		}
	}

	protected float getFlipDegrees() {
		return 90.0F;
	}

	protected float getWhiteOverlayProgress(S livingEntityRenderState) {
		return 0.0F;
	}

	protected void scale(S livingEntityRenderState, PoseStack poseStack) {
	}

	protected boolean shouldShowName(T livingEntity, double d) {
		if (livingEntity.isDiscrete()) {
			float f = 32.0F;
			if (d >= 1024.0) {
				return false;
			}
		}

		Minecraft minecraft = Minecraft.getInstance();
		LocalPlayer localPlayer = minecraft.player;
		boolean bl = !livingEntity.isInvisibleTo(localPlayer);
		if (livingEntity != localPlayer) {
			Team team = livingEntity.getTeam();
			Team team2 = localPlayer.getTeam();
			if (team != null) {
				Visibility visibility = team.getNameTagVisibility();
				switch (visibility) {
					case ALWAYS:
						return bl;
					case NEVER:
						return false;
					case HIDE_FOR_OTHER_TEAMS:
						return team2 == null ? bl : team.isAlliedTo(team2) && (team.canSeeFriendlyInvisibles() || bl);
					case HIDE_FOR_OWN_TEAM:
						return team2 == null ? bl : !team.isAlliedTo(team2) && bl;
					default:
						return true;
				}
			}
		}

		return Minecraft.renderNames() && livingEntity != minecraft.getCameraEntity() && bl && !livingEntity.isVehicle();
	}

	public boolean isEntityUpsideDown(T livingEntity) {
		Component component = livingEntity.getCustomName();
		return component != null && isUpsideDownName(component.getString());
	}

	protected static boolean isUpsideDownName(String string) {
		return "Dinnerbone".equals(string) || "Grumm".equals(string);
	}

	protected float getShadowRadius(S livingEntityRenderState) {
		return super.getShadowRadius(livingEntityRenderState) * livingEntityRenderState.scale;
	}

	public void extractRenderState(T livingEntity, S livingEntityRenderState, float f) {
		super.extractRenderState(livingEntity, livingEntityRenderState, f);
		float g = Mth.rotLerp(f, livingEntity.yHeadRotO, livingEntity.yHeadRot);
		livingEntityRenderState.bodyRot = solveBodyRot(livingEntity, g, f);
		livingEntityRenderState.yRot = Mth.wrapDegrees(g - livingEntityRenderState.bodyRot);
		livingEntityRenderState.xRot = livingEntity.getXRot(f);
		livingEntityRenderState.isUpsideDown = this.isEntityUpsideDown(livingEntity);
		if (livingEntityRenderState.isUpsideDown) {
			livingEntityRenderState.xRot *= -1.0F;
			livingEntityRenderState.yRot *= -1.0F;
		}

		if (!livingEntity.isPassenger() && livingEntity.isAlive()) {
			livingEntityRenderState.walkAnimationPos = livingEntity.walkAnimation.position(f);
			livingEntityRenderState.walkAnimationSpeed = livingEntity.walkAnimation.speed(f);
		} else {
			livingEntityRenderState.walkAnimationPos = 0.0F;
			livingEntityRenderState.walkAnimationSpeed = 0.0F;
		}

		if (livingEntity.getVehicle() instanceof LivingEntity livingEntity2) {
			livingEntityRenderState.wornHeadAnimationPos = livingEntity2.walkAnimation.position(f);
		} else {
			livingEntityRenderState.wornHeadAnimationPos = livingEntityRenderState.walkAnimationPos;
		}

		livingEntityRenderState.scale = livingEntity.getScale();
		livingEntityRenderState.ageScale = livingEntity.getAgeScale();
		livingEntityRenderState.pose = livingEntity.getPose();
		livingEntityRenderState.bedOrientation = livingEntity.getBedOrientation();
		if (livingEntityRenderState.bedOrientation != null) {
			livingEntityRenderState.eyeHeight = livingEntity.getEyeHeight(Pose.STANDING);
		}

		livingEntityRenderState.isFullyFrozen = livingEntity.isFullyFrozen();
		livingEntityRenderState.isBaby = livingEntity.isBaby();
		livingEntityRenderState.isInWater = livingEntity.isInWater();
		livingEntityRenderState.isAutoSpinAttack = livingEntity.isAutoSpinAttack();
		livingEntityRenderState.hasRedOverlay = livingEntity.hurtTime > 0 || livingEntity.deathTime > 0;
		ItemStack itemStack = livingEntity.getItemBySlot(EquipmentSlot.HEAD);
		if (itemStack.getItem() instanceof BlockItem blockItem && blockItem.getBlock() instanceof AbstractSkullBlock abstractSkullBlock) {
			livingEntityRenderState.wornHeadType = abstractSkullBlock.getType();
			livingEntityRenderState.wornHeadProfile = (ResolvableProfile)itemStack.get(DataComponents.PROFILE);
			livingEntityRenderState.headItem.clear();
		} else {
			livingEntityRenderState.wornHeadType = null;
			livingEntityRenderState.wornHeadProfile = null;
			if (!HumanoidArmorLayer.shouldRender(itemStack, EquipmentSlot.HEAD)) {
				this.itemModelResolver.updateForLiving(livingEntityRenderState.headItem, itemStack, ItemDisplayContext.HEAD, livingEntity);
			} else {
				livingEntityRenderState.headItem.clear();
			}
		}

		livingEntityRenderState.deathTime = livingEntity.deathTime > 0 ? livingEntity.deathTime + f : 0.0F;
		Minecraft minecraft = Minecraft.getInstance();
		livingEntityRenderState.isInvisibleToPlayer = livingEntityRenderState.isInvisible && livingEntity.isInvisibleTo(minecraft.player);
	}

	protected void extractAdditionalHitboxes(T livingEntity, Builder<HitboxRenderState> builder, float f) {
		AABB aABB = livingEntity.getBoundingBox();
		float g = 0.01F;
		HitboxRenderState hitboxRenderState = new HitboxRenderState(
			aABB.minX - livingEntity.getX(),
			livingEntity.getEyeHeight() - 0.01F,
			aABB.minZ - livingEntity.getZ(),
			aABB.maxX - livingEntity.getX(),
			livingEntity.getEyeHeight() + 0.01F,
			aABB.maxZ - livingEntity.getZ(),
			1.0F,
			0.0F,
			0.0F
		);
		builder.add(hitboxRenderState);
	}

	private static float solveBodyRot(LivingEntity livingEntity, float f, float g) {
		if (livingEntity.getVehicle() instanceof LivingEntity livingEntity2) {
			float h = Mth.rotLerp(g, livingEntity2.yBodyRotO, livingEntity2.yBodyRot);
			float i = 85.0F;
			float j = Mth.clamp(Mth.wrapDegrees(f - h), -85.0F, 85.0F);
			h = f - j;
			if (Math.abs(j) > 50.0F) {
				h += j * 0.2F;
			}

			return h;
		} else {
			return Mth.rotLerp(g, livingEntity.yBodyRotO, livingEntity.yBodyRot);
		}
	}
}
