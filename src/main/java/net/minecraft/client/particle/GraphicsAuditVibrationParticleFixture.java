package net.minecraft.client.particle;

import com.google.gson.JsonArray;
import com.google.gson.JsonObject;
import net.minecraft.client.Minecraft;
import net.minecraft.client.multiplayer.ClientLevel;
import net.minecraft.client.renderer.texture.TextureAtlasSprite;
import net.minecraft.core.BlockPos;
import net.minecraft.data.AtlasIds;
import net.minecraft.resources.ResourceLocation;
import net.minecraft.world.level.gameevent.BlockPositionSource;
import net.minecraft.world.phys.Vec3;

/** Opt-in fixed simulation inputs; ordinary vibration extraction and rendering. */
public final class GraphicsAuditVibrationParticleFixture {
    private static ClientLevel installedLevel;
    private static VibrationSignalParticle particle;
    private static float extractionPartialTick = Float.NaN;
    public static final ResourceLocation SPRITE = ResourceLocation.withDefaultNamespace("vibration");
    private GraphicsAuditVibrationParticleFixture() {}
    public static boolean requested() { return Boolean.getBoolean("mattmc.dev.graphicsAuditVibrationParticle"); }
    public static boolean hidden() { return Boolean.getBoolean("mattmc.dev.graphicsAuditVibrationHidden"); }
    public static int requestedElevation() {
        int elevation = Integer.parseInt(System.getProperty("mattmc.dev.graphicsAuditVibrationElevation", "0"));
        if (elevation != 0 && elevation != 1) throw new IllegalArgumentException("unsupported vibration fixture elevation");
        return elevation;
    }
    public static int requestedSteps() {
        int steps = Integer.parseInt(System.getProperty("mattmc.dev.graphicsAuditVibrationSteps", "0"));
        if (steps != 0 && steps != 6) throw new IllegalArgumentException("unsupported vibration fixture step count");
        return steps;
    }

    static VibrationSignalParticle create(ClientLevel level, Vec3 origin, TextureAtlasSprite sprite, boolean hidden) {
        return create(level, origin, sprite, hidden, 0);
    }

    static VibrationSignalParticle create(ClientLevel level, Vec3 origin, TextureAtlasSprite sprite, boolean hidden, int steps) {
        return create(level, origin, sprite, hidden, steps, 0);
    }

    static VibrationSignalParticle create(ClientLevel level, Vec3 origin, TextureAtlasSprite sprite, boolean hidden, int steps, int elevation) {
        if (steps != 0 && steps != 6) throw new IllegalArgumentException("unsupported vibration fixture step count");
        if ((elevation != 0 && elevation != 1) || (elevation != 0 && steps != 0))
            throw new IllegalArgumentException("elevated vibration fixture requires the initial simulation pose");
        // Translate source and destination together by an integer height.
        // Orientation/size/simulation are ordinary; the full diagnostic quad is
        // now above the horizon instead of overlapping animated world water.
        origin = origin.add(0, elevation, 0);
        // Hold the complete simulation clock, including its sub-tick input.
        // Delegate geometry to the ordinary implementation with identical
        // fixture inputs; no production particle or renderer is changed.
        var value = new VibrationSignalParticle(level, origin.x, origin.y, origin.z,
            new BlockPositionSource(BlockPos.containing(origin.add(2, 1, -3))), 40, sprite) {
                private int remainingSteps = steps;
                @Override public void tick() {
                    if (remainingSteps > 0) {
                        super.tick();
                        remainingSteps--;
                    }
                }
                @Override public void extract(net.minecraft.client.renderer.state.QuadParticleRenderState state,
                    net.minecraft.client.Camera camera, float enginePartialTick) {
                    extractionPartialTick = 1.0F;
                    super.extract(state, camera, extractionPartialTick);
                }
            };
        value.age = 12;
        value.alpha = hidden ? 0 : 1;
        for (int i = 0; i < steps; i++) value.tick();
        return value;
    }

    public static void install(Minecraft minecraft) {
        if (!requested() || minecraft.level == null || minecraft.player == null) return;
        var sprite = minecraft.getAtlasManager().getAtlasOrThrow(AtlasIds.PARTICLES).getSprite(SPRITE);
        if (installedLevel == minecraft.level && particle != null && particle.sprite == sprite) return;
        if (particle != null) particle.remove();
        var origin = GraphicsAuditTerrainParticleFixture.position(minecraft.player.getEyePosition(), minecraft.player.getLookAngle());
        particle = create(minecraft.level, origin, sprite, hidden(), requestedSteps(), requestedElevation());
        installedLevel = minecraft.level;
        minecraft.particleEngine.installGraphicsAuditParticle(particle);
    }

    public static String receipt(Minecraft minecraft) {
        if (!requested()) return "null";
        var result = new JsonObject();
        boolean present = installedLevel == minecraft.level && particle != null;
        result.addProperty("fixture", "ordinary-vibration-fixed-simulation-v1");
        result.addProperty("hidden", hidden());
        result.addProperty("complete", present && particle.isAlive()
            && minecraft.particleEngine.containsGraphicsAuditParticle(particle));
        if (!present) return result.toString();
        result.addProperty("sprite", particle.sprite.contents().name().toString());
        var position = new JsonArray();
        position.add(particle.x); position.add(particle.y); position.add(particle.z);
        result.add("position", position);
        result.addProperty("age", particle.age);
        result.addProperty("simulationSteps", requestedSteps());
        result.addProperty("elevation", requestedElevation());
        result.addProperty("lifetime", particle.lifetime);
        result.addProperty("alpha", particle.alpha);
        result.addProperty("size", particle.getQuadSize(1));
        result.addProperty("light", particle.getLightColor(1));
        result.addProperty("extractionPartialTick", Float.isFinite(extractionPartialTick) ? extractionPartialTick : -1);
        return result.toString();
    }
}
