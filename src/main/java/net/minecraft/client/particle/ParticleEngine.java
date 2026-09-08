package net.minecraft.client.particle;

import com.google.common.collect.Lists;
import com.google.common.collect.Maps;
import com.google.common.collect.Queues;
import it.unimi.dsi.fastutil.objects.Object2IntOpenHashMap;
import java.util.List;
import java.util.Map;
import java.util.Optional;
import java.util.Queue;
import net.minecraft.api.EnvType;
import net.minecraft.api.Environment;
import net.minecraft.client.Camera;
import net.minecraft.client.multiplayer.ClientLevel;
import net.minecraft.client.renderer.SubmitNodeStorage;
import net.minecraft.client.renderer.culling.Frustum;
import net.minecraft.client.renderer.state.CameraRenderState;
import net.minecraft.client.renderer.state.ParticleGroupRenderState;
import net.minecraft.client.renderer.state.ParticlesRenderState;
import net.minecraft.core.particles.ParticleLimit;
import net.minecraft.core.particles.ParticleOptions;
import net.minecraft.core.registries.BuiltInRegistries;
import net.minecraft.util.RandomSource;
import net.minecraft.util.profiling.Profiler;
import net.minecraft.world.entity.Entity;
import org.jetbrains.annotations.Nullable;

@Environment(EnvType.CLIENT)
public class ParticleEngine {
	private static int rustGalTerrainParticleDiagnostics;
	private static final int MAX_RUST_MODEL_PARTICLE_GROUPS = 1_024;
	private static final int MAX_RUST_MODEL_PARTICLE_INSTANCES = 4_096;
	private static final List<ParticleRenderType> RENDER_ORDER = List.of(
		ParticleRenderType.SINGLE_QUADS, ParticleRenderType.ITEM_PICKUP, ParticleRenderType.ELDER_GUARDIANS
	);
	protected ClientLevel level;
	private final Map<ParticleRenderType, ParticleGroup<?>> particles = Maps.<ParticleRenderType, ParticleGroup<?>>newIdentityHashMap();
	private final Queue<TrackingEmitter> trackingEmitters = Queues.<TrackingEmitter>newArrayDeque();
	private final Queue<Particle> particlesToAdd = Queues.<Particle>newArrayDeque();
	private final Object2IntOpenHashMap<ParticleLimit> trackedParticleCounts = new Object2IntOpenHashMap<>();
	private final ParticleResources resourceManager;
	private final RandomSource random = RandomSource.create();

	public ParticleEngine(ClientLevel clientLevel, ParticleResources particleResources) {
		this.level = clientLevel;
		this.resourceManager = particleResources;
	}

	public void createTrackingEmitter(Entity entity, ParticleOptions particleOptions) {
		this.trackingEmitters.add(new TrackingEmitter(this.level, entity, particleOptions));
	}

	public void createTrackingEmitter(Entity entity, ParticleOptions particleOptions, int i) {
		this.trackingEmitters.add(new TrackingEmitter(this.level, entity, particleOptions, i));
	}

	@Nullable
	public Particle createParticle(ParticleOptions particleOptions, double d, double e, double f, double g, double h, double i) {
		Particle particle = this.makeParticle(particleOptions, d, e, f, g, h, i);
		if (particle != null) {
			this.add(particle);
			return particle;
		} else {
			return null;
		}
	}

	@Nullable
	private <T extends ParticleOptions> Particle makeParticle(T particleOptions, double d, double e, double f, double g, double h, double i) {
		ParticleProvider<T> particleProvider = (ParticleProvider<T>)this.resourceManager
			.getProviders()
			.get(BuiltInRegistries.PARTICLE_TYPE.getId(particleOptions.getType()));
		return particleProvider == null ? null : particleProvider.createParticle(particleOptions, this.level, d, e, f, g, h, i, this.random);
	}

	/** Installs one capture fixture while simulation ticks are frozen; never called by gameplay. */
	void installGraphicsAuditParticle(Particle particle) {
		if ((!GraphicsAuditTerrainParticleFixture.requested() && !GraphicsAuditAtlasParticleFixture.requested())
			|| particle.getParticleLimit().isPresent()) {
			throw new IllegalStateException("Unrequested or limited graphics audit particle");
		}
		this.particles.computeIfAbsent(particle.getGroup(), this::createParticleGroup).add(particle);
	}

	boolean containsGraphicsAuditParticle(Particle particle) {
		var group = this.particles.get(particle.getGroup());
		return group != null && group.particles.contains(particle);
	}

	public void add(Particle particle) {
		Optional<ParticleLimit> optional = particle.getParticleLimit();
		if (optional.isPresent()) {
			if (this.hasSpaceInParticleLimit((ParticleLimit)optional.get())) {
				this.particlesToAdd.add(particle);
				this.updateCount((ParticleLimit)optional.get(), 1);
			}
		} else {
			this.particlesToAdd.add(particle);
		}
	}

	public void tick() {
		this.particles.forEach((particleRenderType, particleGroup) -> {
			Profiler.get().push(particleRenderType.name());
			particleGroup.tickParticles();
			Profiler.get().pop();
		});
		if (!this.trackingEmitters.isEmpty()) {
			List<TrackingEmitter> list = Lists.<TrackingEmitter>newArrayList();

			for (TrackingEmitter trackingEmitter : this.trackingEmitters) {
				trackingEmitter.tick();
				if (!trackingEmitter.isAlive()) {
					list.add(trackingEmitter);
				}
			}

			this.trackingEmitters.removeAll(list);
		}

		Particle particle;
		if (!this.particlesToAdd.isEmpty()) {
			while ((particle = (Particle)this.particlesToAdd.poll()) != null) {
				((ParticleGroup)this.particles.computeIfAbsent(particle.getGroup(), this::createParticleGroup)).add(particle);
			}
		}
	}

	private ParticleGroup<?> createParticleGroup(ParticleRenderType particleRenderType) {
		if (particleRenderType == ParticleRenderType.ITEM_PICKUP) {
			return new ItemPickupParticleGroup(this);
		} else if (particleRenderType == ParticleRenderType.ELDER_GUARDIANS) {
			return new ElderGuardianParticleGroup(this);
		} else {
			return (ParticleGroup<?>)(particleRenderType == ParticleRenderType.NO_RENDER
				? new NoRenderParticleGroup(this)
				: new QuadParticleGroup(this, particleRenderType));
		}
	}

	/** Drains capture-seeded particles through the normal render-group path. */
	public void flushPendingParticlesForCapture() {
		if (!Boolean.getBoolean("mattmc.dev.deterministicCameraCapture")) {
			return;
		}
		Particle particle;
		while ((particle = this.particlesToAdd.poll()) != null) {
			((ParticleGroup)this.particles.computeIfAbsent(particle.getGroup(), this::createParticleGroup)).add(particle);
		}
	}

	protected void updateCount(ParticleLimit particleLimit, int i) {
		this.trackedParticleCounts.addTo(particleLimit, i);
	}

	public void extract(ParticlesRenderState particlesRenderState, Frustum frustum, Camera camera, float f) {
		net.minecraft.client.dev.GraphicsAuditLavaFixture.beginParticles();
		for (ParticleRenderType particleRenderType : RENDER_ORDER) {
			ParticleGroup<?> particleGroup = (ParticleGroup<?>)this.particles.get(particleRenderType);
			if (particleGroup != null && !particleGroup.isEmpty()) {
				if (!(particleGroup instanceof QuadParticleGroup) && !(particleGroup instanceof NoRenderParticleGroup))
					net.minecraft.client.dev.GraphicsAuditLavaFixture.unknownParticleGroup();
				particlesRenderState.add(particleGroup.extractRenderState(frustum, camera, f));
			}
		}
		net.minecraft.client.dev.GraphicsAuditLavaFixture.endParticles();
	}

	public int enqueueRustGalBlockMarkers(Camera camera, float f) {
		if (!net.vulkanic.world.WorldRenderRoutePolicy.currentMaterialRoute().usesRustWholeFrameVulkan()) return 0;
		int enqueued = 0;
		for (ParticleGroup<?> particleGroup : this.particles.values()) {
			if (particleGroup instanceof QuadParticleGroup quadParticleGroup) {
				enqueued += quadParticleGroup.enqueueRustGalBlockMarkers(camera, f);
			}
		}
		return enqueued;
	}

	public int enqueueRustGalTerrainParticles(Camera camera, float f) {
		if (!net.vulkanic.world.WorldRenderRoutePolicy.currentMaterialRoute().usesRustWholeFrameVulkan()) return 0;
		int enqueued = 0;
		int terrainParticles = 0;
		for (ParticleGroup<?> particleGroup : this.particles.values()) {
			if (particleGroup instanceof QuadParticleGroup quadParticleGroup) {
				for (Object particle : quadParticleGroup.getAll()) {
					if (particle instanceof TerrainParticle) terrainParticles++;
				}
				enqueued += quadParticleGroup.enqueueRustGalTerrainParticles(camera, f);
			}
		}
		if (terrainParticles > 0 && Boolean.getBoolean("mattmc.dev.graphicsAuditSliceMetrics") && rustGalTerrainParticleDiagnostics++ < 8) {
			System.out.println("[MattMC graphics audit] TerrainParticle group drain particles=" + terrainParticles + " enqueued=" + enqueued);
		}
		return enqueued;
	}

	public int enqueueRustGalParticles() {
		if (!net.vulkanic.world.WorldRenderRoutePolicy.currentMaterialRoute().usesRustWholeFrameVulkan()) return 0;
		int enqueued = 0;
		for (ParticleGroup<?> particleGroup : this.particles.values()) {
			if (particleGroup instanceof QuadParticleGroup quadParticleGroup) {
				enqueued += quadParticleGroup.enqueueRustGalParticles();
			}
		}
		return enqueued;
	}

	/**
	 * Whole-frame extraction entrypoint. Unlike the compatibility renderer, the
	 * Rust route does not run {@link #extract}; rebuild each quad group's copied
	 * state from the current camera/frustum before handing it to Rust.
	 */
	public int enqueueRustGalParticles(Frustum frustum, Camera camera, float partialTick) {
		if (!net.vulkanic.world.WorldRenderRoutePolicy.currentMaterialRoute().usesRustWholeFrameVulkan()) return 0;
		net.minecraft.client.dev.GraphicsAuditLavaFixture.beginParticles();
		int enqueued = 0;
		int terrainParticles = 0;
		for (ParticleGroup<?> particleGroup : this.particles.values()) {
			if (!particleGroup.isEmpty() && !(particleGroup instanceof QuadParticleGroup)
				&& !(particleGroup instanceof NoRenderParticleGroup))
				net.minecraft.client.dev.GraphicsAuditLavaFixture.unknownParticleGroup();
			if (particleGroup instanceof QuadParticleGroup quadParticleGroup) {
				if (!System.getProperty("mattmc.dev.rustGalWorldMaterial.terrainParticleScenario", "").isBlank()) {
					for (Object particle : quadParticleGroup.getAll()) {
						if (particle instanceof TerrainParticle) terrainParticles++;
					}
					if (!net.vulkanic.world.RustGalWorldPrimitiveRenderer.ensureParticleAtlasAvailable(
						net.minecraft.client.renderer.texture.TextureAtlas.LOCATION_BLOCKS)) {
						throw new IllegalStateException("Rust whole-frame terrain particle atlas preflight rejected the block atlas");
					}
					enqueued += quadParticleGroup.enqueueRustGalTerrainParticles(camera, partialTick);
				}
				enqueued += quadParticleGroup.enqueueRustGalParticles(frustum, camera, partialTick);
			}
		}
		if (terrainParticles > 0 && Boolean.getBoolean("mattmc.dev.graphicsAuditSliceMetrics")) {
			System.out.println("[MattMC graphics audit] TerrainParticle whole-frame collector particles=" + terrainParticles + " enqueued=" + enqueued);
		}
		// The optional dedicated terrain-particle fixture bypasses quad-state
		// extraction. It cannot count as an observed clear lava capture.
		if (terrainParticles > 0) net.minecraft.client.dev.GraphicsAuditLavaFixture.unknownParticleGroup();
		net.minecraft.client.dev.GraphicsAuditLavaFixture.endParticles();
		return enqueued;
	}

	/**
	 * Extracts the non-quad particle families whose vanilla states already
	 * submit through the semantic model collector. This is used only by the
	 * Rust whole-frame route; the legacy renderer continues to use
	 * {@link #extract(ParticlesRenderState, Frustum, Camera, float)}.
	 */
	public int enqueueRustGalModelParticles(
		Camera camera, float partialTick, SubmitNodeStorage submitNodeStorage, CameraRenderState cameraRenderState
	) {
		if (!net.vulkanic.world.WorldRenderRoutePolicy.currentMaterialRoute().usesRustWholeFrameVulkan()) return 0;
		int enqueued = 0;
		net.vulkanic.world.RustGalWorldPrimitiveRenderer.ModelMeshBatchCheckpoint checkpoint =
			net.vulkanic.world.RustGalWorldPrimitiveRenderer.markModelMeshBatch();
		try {
			for (ParticleGroup<?> particleGroup : this.particles.values()) {
				if (particleGroup.isEmpty() || particleGroup instanceof NoRenderParticleGroup
					|| particleGroup instanceof QuadParticleGroup) {
					continue;
				}
				if (particleGroup instanceof ItemPickupParticleGroup || particleGroup instanceof ElderGuardianParticleGroup) {
					if (enqueued >= MAX_RUST_MODEL_PARTICLE_GROUPS) {
						throw new IllegalStateException(
							"Rust whole-frame model-particle group bound exceeded " + MAX_RUST_MODEL_PARTICLE_GROUPS
						);
					}
					// These two render states do not read the frustum during extraction;
					// they retain only copied gameplay/model data and immediately submit
					// into the collector. No Java particle renderer crosses the boundary.
					ParticleGroupRenderState state = particleGroup.extractRenderState(null, camera, partialTick);
					int instanceCount = state instanceof ItemPickupParticleGroup.State itemState
						? itemState.instances().size()
						: state instanceof ElderGuardianParticleGroup.State elderState
							? elderState.states().size()
							: 0;
					if (instanceCount > MAX_RUST_MODEL_PARTICLE_INSTANCES) {
						throw new IllegalStateException(
							"Rust whole-frame model-particle instance bound exceeded " + MAX_RUST_MODEL_PARTICLE_INSTANCES
						);
					}
					state.submitSemantic(submitNodeStorage, cameraRenderState);
					enqueued++;
					continue;
				}
				// This method runs outside the ordinary QuadParticleRenderState path.
				// Do not let a newly introduced or mod-provided model particle family
				// disappear merely because it has no explicit Rust semantic collector.
				// The selected Vulkan presenter will fail closed before submission, with
				// no Java callback retained as a hidden fallback.
				net.vulkanic.world.RustGalWorldPrimitiveRenderer.recordUnsupportedParticleGroup();
				throw new IllegalStateException(
					"Rust whole-frame particle route has no semantic collector for "
						+ particleGroup.getClass().getName()
				);
			}
		} catch (RuntimeException failure) {
			net.vulkanic.world.RustGalWorldPrimitiveRenderer.rollbackModelMeshBatch(checkpoint);
			throw failure;
		}
		return enqueued;
	}

	public void setLevel(@Nullable ClientLevel clientLevel) {
		this.level = clientLevel;
		this.clearParticles();
		this.trackingEmitters.clear();
	}

	public String countParticles() {
		return String.valueOf(this.particles.values().stream().mapToInt(ParticleGroup::size).sum());
	}

	private boolean hasSpaceInParticleLimit(ParticleLimit particleLimit) {
		return this.trackedParticleCounts.getInt(particleLimit) < particleLimit.limit();
	}

	public void clearParticles() {
		this.particles.clear();
		this.particlesToAdd.clear();
		this.trackingEmitters.clear();
		this.trackedParticleCounts.clear();
	}
}
