package net.minecraft.client.particle;

import net.minecraft.api.EnvType;
import net.minecraft.api.Environment;
import net.minecraft.client.Minecraft;
import net.minecraft.client.Camera;
import net.minecraft.client.dev.GraphicsFrameBenchmark;
import net.minecraft.client.multiplayer.ClientLevel;
import net.minecraft.client.renderer.LevelRenderer;
import net.minecraft.client.renderer.state.QuadParticleRenderState;
import net.minecraft.core.BlockPos;
import net.minecraft.core.particles.BlockParticleOption;
import net.minecraft.util.ARGB;
import net.minecraft.util.RandomSource;
import net.minecraft.world.level.block.Blocks;
import net.minecraft.world.level.block.state.BlockState;
import net.vulkanic.world.RustGalWorldPrimitiveRenderer;
import org.jetbrains.annotations.Nullable;
import org.joml.Quaternionf;

@Environment(EnvType.CLIENT)
public class TerrainParticle extends SingleQuadParticle {
	private final BlockPos pos;
	private final BlockState blockState;
	private final float uo;
	private final float vo;
	// Iris: Track whether particle is opaque (from MixinTerrainParticle)
	private boolean isOpaque;
	private boolean alphaTested;

	/** Read-only fixture receipt; does not select rendering behavior. */
	boolean graphicsAuditAlphaTested() { return this.alphaTested; }

	public TerrainParticle(ClientLevel clientLevel, double d, double e, double f, double g, double h, double i, BlockState blockState) {
		this(clientLevel, d, e, f, g, h, i, blockState, BlockPos.containing(d, e, f));
	}

	public TerrainParticle(ClientLevel clientLevel, double d, double e, double f, double g, double h, double i, BlockState blockState, BlockPos blockPos) {
		super(clientLevel, d, e, f, g, h, i, Minecraft.getInstance().getBlockRenderer().getBlockModelShaper().getParticleIcon(blockState));
		this.pos = blockPos;
		this.blockState = blockState;
		this.gravity = 1.0F;
		if (!System.getProperty("mattmc.dev.rustGalWorldMaterial.terrainParticleScenario", "").isBlank()) {
			this.gravity = 0.0F;
			this.hasPhysics = false;
			this.xd = 0.0;
			this.yd = 0.0;
			this.zd = 0.0;
			this.lifetime = 20_000;
		}
		this.rCol = 0.6F;
		this.gCol = 0.6F;
		this.bCol = 0.6F;
		if (!blockState.is(Blocks.GRASS_BLOCK)) {
			int j = Minecraft.getInstance().getBlockColors().getColor(blockState, clientLevel, blockPos, 0);
			this.rCol *= (j >> 16 & 0xFF) / 255.0F;
			this.gCol *= (j >> 8 & 0xFF) / 255.0F;
			this.bCol *= (j & 0xFF) / 255.0F;
		}

		this.quadSize /= 2.0F;
		this.uo = GraphicsAuditTerrainParticleFixture.offset(this.random.nextFloat() * 3.0F);
		this.vo = GraphicsAuditTerrainParticleFixture.offset(this.random.nextFloat() * 3.0F);
		GraphicsAuditTerrainParticleFixture.configure(this);
		
		// Iris: Resolve translucency (from MixinTerrainParticle)
		net.minecraft.client.renderer.chunk.ChunkSectionLayer type = net.minecraft.client.renderer.ItemBlockRenderTypes.getChunkRenderType(blockState);
		if (type == net.minecraft.client.renderer.chunk.ChunkSectionLayer.SOLID
			|| type == net.minecraft.client.renderer.chunk.ChunkSectionLayer.CUTOUT
			|| type == net.minecraft.client.renderer.chunk.ChunkSectionLayer.CUTOUT_MIPPED) {
				isOpaque = true;
			}
		if (type == net.minecraft.client.renderer.chunk.ChunkSectionLayer.CUTOUT
			|| type == net.minecraft.client.renderer.chunk.ChunkSectionLayer.CUTOUT_MIPPED) {
			alphaTested = true;
		}
	}

	@Override
	public SingleQuadParticle.Layer getLayer() {
		// Iris: Override particle sheet for opaque particles (from MixinTerrainParticle)
		if (isOpaque) {
			return net.irisshaders.iris.fantastic.IrisParticleRenderTypes.TERRAIN_OPAQUE;
		}
		return SingleQuadParticle.Layer.TERRAIN;
	}

	@Override
	public void extract(QuadParticleRenderState quadParticleRenderState, Camera camera, float f) {
		long startNanos = System.nanoTime();
		GraphicsFrameBenchmark.beginPhase("game.particles.terrain.extract");
		if (Boolean.getBoolean("mattmc.dev.rustGalWorldMaterial.terrainParticle.disabled")) {
			if (net.vulkanic.VulkanicAPI.isVulkanBackendSelected()
				|| net.vulkanic.bridge.RustGalVulkanWholeFrameMode.enabled()) {
				throw new IllegalStateException("Rust whole-frame terrain particle route cannot be disabled");
			}
			GraphicsFrameBenchmark.endPhase("game.particles.terrain.extract");
			GraphicsFrameBenchmark.recordTerrainParticleExtraction("disabled", this.blockState, System.nanoTime() - startNanos);
			return;
		}
		if (Boolean.getBoolean("mattmc.dev.rustGalWorldMaterial.terrainParticle.legacyControl")) {
			if (net.vulkanic.VulkanicAPI.isVulkanBackendSelected()
				|| net.vulkanic.bridge.RustGalVulkanWholeFrameMode.enabled()) {
				throw new IllegalStateException("Rust whole-frame terrain particles cannot use the Java legacy control path");
			}
			super.extract(quadParticleRenderState, camera, f);
			GraphicsFrameBenchmark.endPhase("game.particles.terrain.extract");
			GraphicsFrameBenchmark.recordTerrainParticleExtraction("java-legacy", this.blockState, System.nanoTime() - startNanos);
			return;
		}
		if (this.enqueueRustGal(camera, f)) {
			GraphicsFrameBenchmark.endPhase("game.particles.terrain.extract");
			return;
		}
		if (net.vulkanic.VulkanicAPI.isVulkanBackendSelected()
			|| net.vulkanic.bridge.RustGalVulkanWholeFrameMode.enabled()) {
			throw new IllegalStateException("Rust whole-frame terrain particle semantics were rejected; Java particle extraction is not a fallback");
		}
		super.extract(quadParticleRenderState, camera, f);
		GraphicsFrameBenchmark.endPhase("game.particles.terrain.extract");
		GraphicsFrameBenchmark.recordTerrainParticleExtraction("java-compat", this.blockState, System.nanoTime() - startNanos);
	}

	boolean enqueueRustGal(Camera camera, float f) {
		long startNanos = System.nanoTime();
		if (!RustGalWorldPrimitiveRenderer.shouldRouteTerrainParticle(this.blockState)) {
			return false;
		}
		Quaternionf quaternionf = new Quaternionf();
		this.getFacingCameraMode().setRotation(quaternionf, camera, f);
		if (this.roll != 0.0F) {
			quaternionf.rotateZ(net.minecraft.util.Mth.lerp(f, this.oRoll, this.roll));
		}
		boolean queued = RustGalWorldPrimitiveRenderer.enqueueTerrainParticle(
			this.blockState,
			this.sprite.contents().name(),
			this.sprite.semanticAnimationResource(),
			camera,
			this.xo,
			this.x,
			this.yo,
			this.y,
			this.zo,
			this.z,
			quaternionf,
			f,
			this.getQuadSize(f),
				// The semantic material binds the full atlas, not a standalone sprite.
				this.getU0(),
				this.getU1(),
				this.getV0(),
				this.getV1(),
				ARGB.colorFromFloat(this.alpha, this.rCol, this.gCol, this.bCol),
			this.getLightColor(f),
			this.alphaTested ? net.vulkanic.bridge.VulkanicGalBridge.ParticleSurface.TERRAIN_CUTOUT
				: this.isOpaque ? net.vulkanic.bridge.VulkanicGalBridge.ParticleSurface.TERRAIN_OPAQUE
				: net.vulkanic.bridge.VulkanicGalBridge.ParticleSurface.TERRAIN_TRANSLUCENT
			);
		if (queued) {
			// Both the normal particle pass and the Rust whole-frame collector
			// call this semantic enqueue path. Record here so route evidence
			// describes the producer that actually reached Rust exactly once.
			GraphicsFrameBenchmark.recordTerrainParticleExtraction(
				"rust", this.blockState, System.nanoTime() - startNanos
			);
		}
		return queued;
	}

	@Override
	protected float getU0() {
		return this.sprite.getU((this.uo + 1.0F) / 4.0F);
	}

	@Override
	protected float getU1() {
		return this.sprite.getU(this.uo / 4.0F);
	}

	@Override
	protected float getV0() {
		return this.sprite.getV(this.vo / 4.0F);
	}

	@Override
	protected float getV1() {
		return this.sprite.getV((this.vo + 1.0F) / 4.0F);
	}

	@Override
	public int getLightColor(float f) {
		int i = super.getLightColor(f);
		return i == 0 && this.level.hasChunkAt(this.pos) ? LevelRenderer.getLightColor(this.level, this.pos) : i;
	}

	@Nullable
	static TerrainParticle createTerrainParticle(
		BlockParticleOption blockParticleOption, ClientLevel clientLevel, double d, double e, double f, double g, double h, double i
	) {
		BlockState blockState = blockParticleOption.getState();
		return !blockState.isAir() && !blockState.is(Blocks.MOVING_PISTON) && blockState.shouldSpawnTerrainParticles()
			? new TerrainParticle(clientLevel, d, e, f, g, h, i, blockState)
			: null;
	}

	@Environment(EnvType.CLIENT)
	public static class CrumblingProvider implements ParticleProvider<BlockParticleOption> {
		@Nullable
		public Particle createParticle(
			BlockParticleOption blockParticleOption, ClientLevel clientLevel, double d, double e, double f, double g, double h, double i, RandomSource randomSource
		) {
			Particle particle = TerrainParticle.createTerrainParticle(blockParticleOption, clientLevel, d, e, f, g, h, i);
			if (particle != null) {
				particle.setParticleSpeed(0.0, 0.0, 0.0);
				particle.setLifetime(randomSource.nextInt(10) + 1);
			}

			return particle;
		}
	}

	@Environment(EnvType.CLIENT)
	public static class DustPillarProvider implements ParticleProvider<BlockParticleOption> {
		@Nullable
		public Particle createParticle(
			BlockParticleOption blockParticleOption, ClientLevel clientLevel, double d, double e, double f, double g, double h, double i, RandomSource randomSource
		) {
			Particle particle = TerrainParticle.createTerrainParticle(blockParticleOption, clientLevel, d, e, f, g, h, i);
			if (particle != null) {
				particle.setParticleSpeed(randomSource.nextGaussian() / 30.0, h + randomSource.nextGaussian() / 2.0, randomSource.nextGaussian() / 30.0);
				particle.setLifetime(randomSource.nextInt(20) + 20);
			}

			return particle;
		}
	}

	@Environment(EnvType.CLIENT)
	public static class Provider implements ParticleProvider<BlockParticleOption> {
		@Nullable
		public Particle createParticle(
			BlockParticleOption blockParticleOption, ClientLevel clientLevel, double d, double e, double f, double g, double h, double i, RandomSource randomSource
		) {
			return TerrainParticle.createTerrainParticle(blockParticleOption, clientLevel, d, e, f, g, h, i);
		}
	}
}
