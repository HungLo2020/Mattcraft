package net.minecraft.client.particle;

import com.google.gson.JsonArray;
import com.google.gson.JsonObject;
import net.minecraft.client.Minecraft;
import net.minecraft.client.multiplayer.ClientLevel;
import net.minecraft.data.AtlasIds;
import net.minecraft.resources.ResourceLocation;

/** Opt-in copied-world inputs for the ordinary two-quad shriek particle. */
public final class GraphicsAuditShriekParticleFixture {
    private static ClientLevel installedLevel;
    private static ShriekParticle particle;
    private GraphicsAuditShriekParticleFixture() {}

    public static boolean requested() {
        return Boolean.getBoolean("mattmc.dev.graphicsAuditShriekParticle");
    }

    static void configure(ShriekParticle value) {
        // At this age/lifetime magnitude bounded runs cannot change the float
        // size/alpha ratio. No renderer or extraction method is overridden.
        value.age = 1_000_000_000;
        value.lifetime = 2_000_000_000;
        value.xd = value.yd = value.zd = 0;
        value.gravity = 0;
        value.friction = 1;
        value.hasPhysics = false;
    }

    public static void install(Minecraft minecraft) {
        if (!requested() || minecraft.level == null || minecraft.player == null) return;
        if (installedLevel == minecraft.level && particle != null) return;
        var origin = GraphicsAuditTerrainParticleFixture.position(
            minecraft.player.getEyePosition(), minecraft.player.getLookAngle());
        var sprite = minecraft.getAtlasManager().getAtlasOrThrow(AtlasIds.PARTICLES)
            .getSprite(ResourceLocation.withDefaultNamespace("shriek"));
        particle = new ShriekParticle(minecraft.level, origin.x, origin.y, origin.z,
            Boolean.getBoolean("mattmc.dev.graphicsAuditShriekDelayed") ? Integer.MAX_VALUE : 0, sprite);
        configure(particle);
        installedLevel = minecraft.level;
        minecraft.particleEngine.installGraphicsAuditParticle(particle);
    }

    public static String receipt(Minecraft minecraft) {
        if (!requested()) return "null";
        var result = new JsonObject();
        boolean present = installedLevel == minecraft.level && particle != null;
        result.addProperty("fixture", "ordinary-shriek-two-quad-v1");
        result.addProperty("delayed", Boolean.getBoolean("mattmc.dev.graphicsAuditShriekDelayed"));
        result.addProperty("complete", present && particle.isAlive()
            && minecraft.particleEngine.containsGraphicsAuditParticle(particle));
        if (!present) return result.toString();
        result.addProperty("sprite", particle.sprite.contents().name().toString());
        var position = new JsonArray();
        position.add(particle.x); position.add(particle.y); position.add(particle.z);
        result.add("position", position);
        result.addProperty("size", particle.getQuadSize(1));
        result.addProperty("light", particle.getLightColor(1));
        result.addProperty("age", particle.age);
        result.addProperty("lifetime", particle.lifetime);
        return result.toString();
    }
}
